//! Stable terminal identities, split trees and versioned on-disk persistence.
use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use std::{
    collections::HashSet,
    fs,
    io::Write,
    path::{Path, PathBuf},
};
use uuid::Uuid;

use crate::settings::Settings;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkspaceKind {
    Local,
    Ssh,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SplitDirection {
    SideBySide,
    Stacked,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Pane {
    pub id: Uuid,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub working_directory: String,
    #[serde(default)]
    pub remote_working_directory: String,
    #[serde(default)]
    pub running_command: String,
}

impl Pane {
    pub fn new(cwd: impl Into<String>) -> Self {
        Self {
            id: Uuid::new_v4(),
            title: String::new(),
            working_directory: cwd.into(),
            remote_working_directory: String::new(),
            running_command: String::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum SplitNode {
    Pane(Pane),
    Split {
        id: Uuid,
        direction: SplitDirection,
        ratio: f32,
        first: Box<SplitNode>,
        second: Box<SplitNode>,
    },
}

impl SplitNode {
    pub fn id(&self) -> Uuid {
        match self {
            Self::Pane(p) => p.id,
            Self::Split { id, .. } => *id,
        }
    }

    pub fn panes(&self) -> Vec<&Pane> {
        let mut panes = Vec::new();
        self.collect_panes(&mut panes);
        panes
    }

    fn collect_panes<'a>(&'a self, panes: &mut Vec<&'a Pane>) {
        match self {
            Self::Pane(pane) => panes.push(pane),
            Self::Split { first, second, .. } => {
                first.collect_panes(panes);
                second.collect_panes(panes);
            }
        }
    }

    pub fn pane(&self, id: Uuid) -> Option<&Pane> {
        match self {
            Self::Pane(p) => (p.id == id).then_some(p),
            Self::Split { first, second, .. } => first.pane(id).or_else(|| second.pane(id)),
        }
    }

    pub fn pane_mut(&mut self, id: Uuid) -> Option<&mut Pane> {
        match self {
            Self::Pane(p) => (p.id == id).then_some(p),
            Self::Split { first, second, .. } => first.pane_mut(id).or_else(|| second.pane_mut(id)),
        }
    }

    /// Adds a sibling without changing any existing pane or split identity.
    pub fn split(&mut self, id: Uuid, direction: SplitDirection, new_pane: Pane) -> bool {
        match self {
            Self::Pane(p) if p.id == id => {
                *self = Self::Split {
                    id: Uuid::new_v4(),
                    direction,
                    ratio: 0.5,
                    first: Box::new(Self::Pane(p.clone())),
                    second: Box::new(Self::Pane(new_pane)),
                };
                true
            }
            Self::Pane(_) => false,
            Self::Split { first, second, .. } => {
                if first.pane(id).is_some() {
                    first.split(id, direction, new_pane)
                } else {
                    second.split(id, direction, new_pane)
                }
            }
        }
    }

    /// A root pane is retained; the caller closes its tab instead.
    pub fn close(&mut self, id: Uuid) -> bool {
        let Self::Split { first, second, .. } = self else {
            return false;
        };
        if matches!(first.as_ref(), Self::Pane(p) if p.id == id) {
            *self = *second.clone();
            return true;
        }
        if matches!(second.as_ref(), Self::Pane(p) if p.id == id) {
            *self = *first.clone();
            return true;
        }
        first.close(id) || second.close(id)
    }

    #[allow(dead_code)] // Safe identity-based API for persistence/import callers.
    pub fn set_ratio(&mut self, node: Uuid, ratio: f32) -> bool {
        if !ratio.is_finite() {
            return false;
        }
        match self {
            Self::Pane(_) => false,
            Self::Split {
                id,
                ratio: current,
                first,
                second,
                ..
            } => {
                if *id == node {
                    *current = ratio.clamp(0.05, 0.95);
                    true
                } else {
                    first.set_ratio(node, ratio) || second.set_ratio(node, ratio)
                }
            }
        }
    }

    fn validate(&mut self, ids: &mut HashSet<Uuid>, depth: usize) -> Result<()> {
        if depth > 64 {
            bail!("Split tree exceeds 64 levels");
        }
        if !ids.insert(self.id()) {
            bail!("Duplicate pane/split identity {}", self.id());
        }
        if let Self::Split {
            ratio,
            first,
            second,
            ..
        } = self
        {
            if !ratio.is_finite() {
                *ratio = 0.5;
            }
            *ratio = ratio.clamp(0.05, 0.95);
            first.validate(ids, depth + 1)?;
            second.validate(ids, depth + 1)?;
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Tab {
    pub id: Uuid,
    pub title: String,
    pub root: SplitNode,
    pub focused_pane: Uuid,
}

impl Tab {
    pub fn new(cwd: impl Into<String>) -> Self {
        let pane = Pane::new(cwd);
        Self {
            id: Uuid::new_v4(),
            title: "Terminal".into(),
            focused_pane: pane.id,
            root: SplitNode::Pane(pane),
        }
    }

    #[allow(dead_code)] // Shared model convenience API, also used by import tests.
    pub fn active_pane(&self) -> Option<&Pane> {
        self.root
            .pane(self.focused_pane)
            .or_else(|| self.root.panes().into_iter().next())
    }

    #[allow(dead_code)] // Shared model convenience API.
    pub fn active_pane_mut(&mut self) -> Option<&mut Pane> {
        let id = self.active_pane()?.id;
        self.root.pane_mut(id)
    }

    #[allow(dead_code)] // Alternative to splitting an explicitly selected pane.
    pub fn split(&mut self, direction: SplitDirection, pane: Pane) -> bool {
        let next = pane.id;
        if self.root.split(self.focused_pane, direction, pane) {
            self.focused_pane = next;
            true
        } else {
            false
        }
    }

    pub fn close_pane(&mut self, id: Uuid) -> bool {
        let panes: Vec<_> = self.root.panes().into_iter().map(|p| p.id).collect();
        if !self.root.close(id) {
            return false;
        }
        if self.root.pane(self.focused_pane).is_none() {
            let index = panes.iter().position(|p| *p == id).unwrap_or(0);
            let remaining = self.root.panes();
            self.focused_pane = remaining[index.saturating_sub(1).min(remaining.len() - 1)].id;
        }
        true
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Workspace {
    pub id: Uuid,
    pub name: String,
    pub kind: WorkspaceKind,
    pub working_directory: String,
    #[serde(default)]
    pub local_tmux: bool,
    #[serde(default)]
    pub ssh_command: String,
    #[serde(default)]
    pub ssh_tmux_session: String,
    #[serde(default = "terminal_icon")]
    pub icon_name: String,
    #[serde(default)]
    pub color_hex: String,
    pub tabs: Vec<Tab>,
    pub active_tab: Uuid,
}

fn terminal_icon() -> String {
    "terminal".into()
}
pub fn home_directory() -> String {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .to_string_lossy()
        .into_owned()
}

impl Workspace {
    pub fn local(name: impl Into<String>, cwd: impl Into<String>, tmux: bool) -> Self {
        let cwd = cwd.into();
        let tab = Tab::new(cwd.clone());
        Self {
            id: Uuid::new_v4(),
            name: name.into(),
            kind: WorkspaceKind::Local,
            working_directory: cwd,
            local_tmux: tmux,
            ssh_command: String::new(),
            ssh_tmux_session: String::new(),
            icon_name: terminal_icon(),
            color_hex: String::new(),
            active_tab: tab.id,
            tabs: vec![tab],
        }
    }

    pub fn ssh(
        name: impl Into<String>,
        command: impl Into<String>,
        session: impl Into<String>,
    ) -> Self {
        let mut workspace = Self::local(name, home_directory(), false);
        workspace.kind = WorkspaceKind::Ssh;
        workspace.ssh_command = command.into();
        workspace.ssh_tmux_session = session.into();
        workspace.icon_name = "server.rack".into();
        workspace
    }

    pub fn uses_tmux(&self) -> bool {
        self.kind == WorkspaceKind::Ssh || self.local_tmux
    }
    pub fn active_tab(&self) -> Option<&Tab> {
        self.tabs
            .iter()
            .find(|t| t.id == self.active_tab)
            .or_else(|| self.tabs.first())
    }
    pub fn active_tab_mut(&mut self) -> Option<&mut Tab> {
        let index = self
            .tabs
            .iter()
            .position(|t| t.id == self.active_tab)
            .unwrap_or(0);
        self.tabs.get_mut(index)
    }
    pub fn add_tab(&mut self) -> Uuid {
        let tab = Tab::new(self.working_directory.clone());
        self.active_tab = tab.id;
        self.tabs.push(tab);
        self.active_tab
    }
    pub fn close_tab(&mut self, id: Uuid) -> bool {
        let Some(index) = self.tabs.iter().position(|t| t.id == id) else {
            return false;
        };
        self.tabs.remove(index);
        if self.tabs.is_empty() {
            self.tabs.push(Tab::new(self.working_directory.clone()));
        }
        if self.active_tab == id {
            self.active_tab = self.tabs.first().map(|t| t.id).unwrap_or_default();
        }
        true
    }
    #[allow(dead_code)] // Convenience API for workspace-level operations.
    pub fn panes(&self) -> Vec<&Pane> {
        self.tabs.iter().flat_map(|t| t.root.panes()).collect()
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct State {
    pub workspaces: Vec<Workspace>,
    pub selected: Uuid,
    #[serde(default = "sidebar_width")]
    pub sidebar_width: f32,
    #[serde(default)]
    pub sidebar_visible: bool,
}

fn sidebar_width() -> f32 {
    240.0
}

impl Default for State {
    fn default() -> Self {
        let mut main = Workspace::local("Main", home_directory(), false);
        main.color_hex = "#4DA3FF".into();
        let mut development = Workspace::local("Development", home_directory(), false);
        development.color_hex = "#2DD4BF".into();
        development.icon_name = "hammer".into();
        let mut scratch =
            Workspace::local("Scratch", std::env::temp_dir().to_string_lossy(), false);
        scratch.color_hex = "#7ED957".into();
        scratch.icon_name = "pencil".into();
        Self {
            selected: main.id,
            workspaces: vec![main, development, scratch],
            sidebar_width: sidebar_width(),
            sidebar_visible: false,
        }
    }
}

impl State {
    pub fn active(&self) -> Option<&Workspace> {
        self.workspaces
            .iter()
            .find(|w| w.id == self.selected)
            .or_else(|| self.workspaces.first())
    }
    pub fn active_mut(&mut self) -> Option<&mut Workspace> {
        let index = self
            .workspaces
            .iter()
            .position(|w| w.id == self.selected)
            .unwrap_or(0);
        self.workspaces.get_mut(index)
    }
    pub fn active_tab(&self) -> Option<&Tab> {
        self.active().and_then(Workspace::active_tab)
    }
    pub fn active_tab_mut(&mut self) -> Option<&mut Tab> {
        self.active_mut().and_then(Workspace::active_tab_mut)
    }
    #[allow(dead_code)] // Convenience API for callers without a tab selection.
    pub fn active_pane(&self) -> Option<&Pane> {
        self.active_tab().and_then(Tab::active_pane)
    }
    #[allow(dead_code)] // Convenience API for callers without a tab selection.
    pub fn active_pane_mut(&mut self) -> Option<&mut Pane> {
        self.active_tab_mut().and_then(Tab::active_pane_mut)
    }
    #[allow(dead_code)] // Convenience API for importers and future integrations.
    pub fn add_workspace(&mut self, workspace: Workspace) {
        self.selected = workspace.id;
        self.workspaces.push(workspace);
    }
    pub fn close_workspace(&mut self, id: Uuid) -> bool {
        let Some(index) = self.workspaces.iter().position(|w| w.id == id) else {
            return false;
        };
        self.workspaces.remove(index);
        if self.selected == id {
            self.selected = self.workspaces.first().map(|w| w.id).unwrap_or_default();
        }
        true
    }

    /// Repair stale selection pointers, but reject duplicate identities instead of
    /// accidentally attaching two surfaces to the same running process.
    pub fn validate(&mut self) -> Result<()> {
        let mut ids = HashSet::new();
        for workspace in &mut self.workspaces {
            if !ids.insert(workspace.id) {
                bail!("Duplicate workspace identity {}", workspace.id);
            }
            for tab in &mut workspace.tabs {
                if !ids.insert(tab.id) {
                    bail!("Duplicate tab identity {}", tab.id);
                }
                tab.root.validate(&mut ids, 0)?;
                if tab.root.pane(tab.focused_pane).is_none() {
                    tab.focused_pane = tab.root.panes()[0].id;
                }
            }
            if !workspace
                .tabs
                .iter()
                .any(|tab| tab.id == workspace.active_tab)
            {
                workspace.active_tab = workspace.tabs.first().map(|t| t.id).unwrap_or_default();
            }
        }
        if !self.workspaces.iter().any(|w| w.id == self.selected) {
            self.selected = self.workspaces.first().map(|w| w.id).unwrap_or_default();
        }
        self.sidebar_width = if self.sidebar_width.is_finite() {
            self.sidebar_width.clamp(160.0, 600.0)
        } else {
            sidebar_width()
        };
        Ok(())
    }
}

#[derive(Debug, Clone)]
pub struct Store {
    directory: PathBuf,
}

impl Store {
    pub fn new() -> Result<Self> {
        Self::at(Self::default_directory())
    }
    pub fn default_directory() -> PathBuf {
        std::env::var_os("BMUX_DATA_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|| {
                dirs::data_local_dir()
                    .unwrap_or_else(|| PathBuf::from("."))
                    .join("Bmux")
            })
    }
    pub fn at(directory: impl Into<PathBuf>) -> Result<Self> {
        let store = Self {
            directory: directory.into(),
        };
        fs::create_dir_all(store.transcript_dir()).context("Create Bmux data directory")?;
        Ok(store)
    }
    pub fn data_dir(&self) -> &Path {
        &self.directory
    }
    pub fn transcript_dir(&self) -> PathBuf {
        self.directory.join("transcripts")
    }

    pub fn load_state(&self) -> Result<State> {
        let path = self.directory.join("workspaces.json");
        if !path.exists() {
            return Ok(State::default());
        }
        let bytes = fs::read(&path).with_context(|| format!("Read {}", path.display()))?;
        let parse = || -> Result<State> {
            let value: serde_json::Value = serde_json::from_slice(&bytes)?;
            let mut state = if value.get("version").and_then(|v| v.as_u64()) == Some(3) {
                serde_json::from_value(
                    value
                        .get("state")
                        .cloned()
                        .context("Missing state payload")?,
                )?
            } else if value.get("selected").is_some() {
                serde_json::from_value(value)?
            } else {
                migrate_swift_state(value)?
            };
            state.validate()?;
            Ok(state)
        };
        parse().map_err(|error| self.preserve_corrupt(&path, error))
    }

    pub fn save_state(&self, state: &State) -> Result<()> {
        let mut validated = state.clone();
        validated.validate()?;
        self.write_json(
            "workspaces.json",
            &serde_json::json!({"version":3,"state":validated}),
        )
    }
    pub fn load_settings(&self) -> Result<Settings> {
        let path = self.directory.join("settings.json");
        if !path.exists() {
            return Ok(Settings::default());
        }
        let bytes = fs::read(&path).with_context(|| format!("Read {}", path.display()))?;
        serde_json::from_slice::<Settings>(&bytes)
            .map(|mut settings| {
                settings.sanitize();
                settings
            })
            .map_err(|error| self.preserve_corrupt(&path, error.into()))
    }
    pub fn save_settings(&self, settings: &Settings) -> Result<()> {
        self.write_json("settings.json", settings)
    }

    fn preserve_corrupt(&self, path: &Path, error: anyhow::Error) -> anyhow::Error {
        let backup = path.with_extension(format!("corrupt-{}.json", Uuid::new_v4()));
        match fs::copy(path, &backup) {
            Ok(_) => error.context(format!(
                "Invalid {}. Original retained; backup: {}",
                path.display(),
                backup.display()
            )),
            Err(copy_error) => error.context(format!(
                "Invalid {}. Original retained; backup failed: {}",
                path.display(),
                copy_error
            )),
        }
    }

    /// A fully written, synced sibling is atomically renamed over the previous
    /// file. Failure before rename leaves the old save untouched.
    pub fn write_json<T: Serialize>(&self, name: &str, value: &T) -> Result<()> {
        let path = self.directory.join(name);
        let bytes = serde_json::to_vec_pretty(value)?;
        let temporary = self
            .directory
            .join(format!(".{name}.{}.tmp", Uuid::new_v4()));
        let save = || -> Result<()> {
            let mut options = fs::OpenOptions::new();
            options.create_new(true).write(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;
                options.mode(0o600);
            }
            let mut file = options.open(&temporary)?;
            file.write_all(&bytes)?;
            file.write_all(b"\n")?;
            file.sync_all()?;
            drop(file);
            fs::rename(&temporary, &path)?;
            #[cfg(unix)]
            {
                fs::File::open(&self.directory)?.sync_all()?;
            }
            Ok(())
        };
        let result = save().with_context(|| format!("Save {}", path.display()));
        if result.is_err() {
            let _ = fs::remove_file(&temporary);
        }
        result
    }
}

fn value_string(value: &serde_json::Value, key: &str) -> String {
    value
        .get(key)
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .into()
}
fn value_id(value: &serde_json::Value, key: &str) -> Result<Uuid> {
    Uuid::parse_str(
        value
            .get(key)
            .and_then(|v| v.as_str())
            .with_context(|| format!("Missing {key}"))?,
    )
    .with_context(|| format!("Invalid {key}"))
}

/// Swift's synthesized enum representation differs from serde's. Migration
/// reads v1, v2, and the legacy workspace array without changing UUIDs.
fn migrate_swift_state(value: serde_json::Value) -> Result<State> {
    if let Some(version) = value.get("version").and_then(|v| v.as_u64())
        && version != 1
        && version != 2
    {
        bail!("Unsupported workspace format version {version}");
    }
    let items = value
        .as_array()
        .or_else(|| value.get("workspaces").and_then(|v| v.as_array()))
        .context("Missing workspaces array")?;
    let mut state = State {
        workspaces: Vec::new(),
        selected: Uuid::nil(),
        sidebar_width: sidebar_width(),
        sidebar_visible: value
            .get("sidebarVisible")
            .and_then(|v| v.as_bool())
            .unwrap_or(false),
    };
    for item in items {
        let cwd = item
            .get("workingDirectory")
            .and_then(|v| v.as_str())
            .map(String::from)
            .unwrap_or_else(home_directory);
        let mut workspace = Workspace::local(
            value_string(item, "name"),
            cwd,
            item.get("localTmux")
                .and_then(|v| v.as_bool())
                .unwrap_or(false),
        );
        workspace.id = value_id(item, "id")?;
        workspace.kind = if value_string(item, "kind") == "ssh" {
            WorkspaceKind::Ssh
        } else {
            WorkspaceKind::Local
        };
        workspace.ssh_command = value_string(item, "sshCommand");
        workspace.ssh_tmux_session = value_string(item, "sshTmuxSession");
        workspace.color_hex = value_string(item, "colorHex");
        workspace.icon_name = item
            .get("iconName")
            .and_then(|v| v.as_str())
            .unwrap_or("terminal")
            .into();
        if let Some(detail) = value
            .get("details")
            .and_then(|v| v.as_array())
            .and_then(|details| {
                details
                    .iter()
                    .rev()
                    .find(|d| value_id(d, "id").ok() == Some(workspace.id))
            })
        {
            let tabs = detail
                .get("tabs")
                .and_then(|v| v.as_array())
                .context("Missing tabs")?;
            workspace.tabs.clear();
            for tab in tabs {
                let root = migrate_swift_node(tab.get("root").context("Missing split root")?, 0)?;
                let focused_pane = value_id(tab, "focusedPaneID").unwrap_or(root.panes()[0].id);
                workspace.tabs.push(Tab {
                    id: value_id(tab, "id")?,
                    title: value_string(tab, "title"),
                    root,
                    focused_pane,
                });
            }
            workspace.active_tab = value_id(detail, "activeTabID").unwrap_or_default();
        }
        state.workspaces.push(workspace);
    }
    state.selected = value_id(&value, "activeID").unwrap_or_default();
    Ok(state)
}

fn migrate_swift_node(value: &serde_json::Value, depth: usize) -> Result<SplitNode> {
    if depth > 64 {
        bail!("Split tree exceeds 64 levels");
    }
    if let Some(pane) = value.get("pane").and_then(|v| v.get("_0")) {
        return Ok(SplitNode::Pane(Pane {
            id: value_id(pane, "id")?,
            title: value_string(pane, "title"),
            working_directory: value_string(pane, "workingDirectory"),
            remote_working_directory: value_string(pane, "remoteWorkingDirectory"),
            running_command: value_string(pane, "runningCommand"),
        }));
    }
    let split = value.get("split").context("Unknown split node")?;
    Ok(SplitNode::Split {
        id: value_id(split, "id")?,
        direction: match value_string(split, "direction").as_str() {
            "sideBySide" => SplitDirection::SideBySide,
            "stacked" => SplitDirection::Stacked,
            other => bail!("Unknown split direction {other}"),
        },
        ratio: split.get("ratio").and_then(|v| v.as_f64()).unwrap_or(0.5) as f32,
        first: Box::new(migrate_swift_node(
            split.get("first").context("Missing first node")?,
            depth + 1,
        )?),
        second: Box::new(migrate_swift_node(
            split.get("second").context("Missing second node")?,
            depth + 1,
        )?),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    fn test_store() -> Store {
        Store::at(std::env::temp_dir().join(format!("bmux-test-{}", Uuid::new_v4()))).unwrap()
    }

    #[test]
    fn nested_close_collapses_only_target_branch_and_preserves_ids() {
        let a = Pane::new("/one");
        let b = Pane::new("/two");
        let c = Pane::new("/three");
        let mut root = SplitNode::Pane(a.clone());
        assert!(root.split(a.id, SplitDirection::SideBySide, b.clone()));
        let outer_id = root.id();
        assert!(root.split(b.id, SplitDirection::Stacked, c.clone()));
        assert!(root.close(b.id));
        assert_eq!(root.id(), outer_id);
        assert_eq!(root.panes(), vec![&a, &c]);
        assert!(root.close(a.id));
        assert_eq!(root, SplitNode::Pane(c.clone()));
        assert!(!root.close(c.id));
        assert!(!root.close(Uuid::new_v4()));
    }

    #[test]
    fn restore_roundtrip_keeps_remote_cwd_separate_and_preserves_empty_state() {
        let store = test_store();
        let mut state = State::default();
        let mut workspace = Workspace::ssh("Server", "ssh user@example.org", "prod");
        let tab = workspace.active_tab_mut().unwrap();
        let pane = tab.active_pane_mut().unwrap();
        pane.remote_working_directory = "/srv/production".into();
        let first = pane.id;
        tab.split(SplitDirection::Stacked, Pane::new(home_directory()));
        tab.root.set_ratio(tab.root.id(), 0.72);
        state.add_workspace(workspace);
        store.save_state(&state).unwrap();
        let loaded = store.load_state().unwrap();
        assert_eq!(loaded, state);
        let restored = loaded.active_tab().unwrap().root.pane(first).unwrap();
        assert_ne!(
            restored.working_directory,
            restored.remote_working_directory
        );
        state.workspaces.clear();
        state.selected = Uuid::nil();
        store.save_state(&state).unwrap();
        assert!(store.load_state().unwrap().workspaces.is_empty());
        fs::remove_dir_all(store.data_dir()).unwrap();
    }

    #[test]
    fn invalid_file_is_retained_and_backed_up() {
        let store = test_store();
        let path = store.data_dir().join("workspaces.json");
        fs::write(&path, b"{interrupted write").unwrap();
        assert!(store.load_state().is_err());
        assert_eq!(fs::read(&path).unwrap(), b"{interrupted write");
        let backups = fs::read_dir(store.data_dir())
            .unwrap()
            .filter_map(Result::ok)
            .filter(|entry| entry.file_name().to_string_lossy().contains(".corrupt-"))
            .count();
        assert_eq!(backups, 1);
        fs::remove_dir_all(store.data_dir()).unwrap();
    }

    #[test]
    fn swift_v2_migration_preserves_panes_and_ratio() {
        let workspace = Uuid::new_v4();
        let tab = Uuid::new_v4();
        let a = Uuid::new_v4();
        let b = Uuid::new_v4();
        let value = serde_json::json!({"version":2,"activeID":workspace,"sidebarVisible":true,
            "workspaces":[{"id":workspace,"name":"Remote","kind":"ssh","workingDirectory":"/home/local","sshCommand":"ssh server"}],
            "details":[{"id":workspace,"activeTabID":tab,"tabs":[{"id":tab,"title":"editor","focusedPaneID":b,
            "root":{"split":{"id":Uuid::new_v4(),"direction":"sideBySide","ratio":0.7,
            "first":{"pane":{"_0":{"id":a,"workingDirectory":"/home/local","remoteWorkingDirectory":"/srv/app"}}},
            "second":{"pane":{"_0":{"id":b,"workingDirectory":"/home/local"}}}}}}]}]});
        let mut state = migrate_swift_state(value).unwrap();
        state.validate().unwrap();
        assert_eq!(state.selected, workspace);
        assert!(state.sidebar_visible);
        assert_eq!(state.active_tab().unwrap().focused_pane, b);
        assert_eq!(state.active_tab().unwrap().root.panes()[0].id, a);
        assert!(
            matches!(state.active_tab().unwrap().root, SplitNode::Split { ratio, .. } if (ratio-0.7).abs()<0.001)
        );
    }

    #[test]
    fn duplicate_identity_is_rejected_and_stale_focus_is_repaired() {
        let mut state = State {
            selected: Uuid::nil(),
            ..State::default()
        };
        state.workspaces[0].tabs[0].focused_pane = Uuid::nil();
        state.validate().unwrap();
        assert_eq!(state.selected, state.workspaces[0].id);
        assert_ne!(state.active_pane().unwrap().id, Uuid::nil());
        state.workspaces.push(state.workspaces[0].clone());
        assert!(state.validate().is_err());
    }
}

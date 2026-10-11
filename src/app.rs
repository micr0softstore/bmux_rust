use std::{
    collections::HashMap,
    path::PathBuf,
    time::{Duration, Instant},
};

use eframe::egui::{
    self, Align2, Color32, FontId, Id, Key, Pos2, Rect, RichText, Sense, Stroke, Vec2,
};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{
    model::{Pane, SplitDirection, SplitNode, State, Store, Tab, Workspace, WorkspaceKind},
    settings::{Settings, ThemeColors},
    terminal::{CursorShape, SessionKind, SpawnSpec, Terminal, TerminalStyle, shell_quote},
};

#[derive(Serialize, Deserialize)]
pub struct WindowFrame {
    pub size: [f32; 2],
    pub position: Option<[f32; 2]>,
}

struct Host {
    terminal: Option<Terminal>,
    error: Option<String>,
    retry_at: Option<Instant>,
    attempts: u32,
}

#[derive(Clone, Debug, PartialEq)]
enum Action {
    NewWorkspace(bool),
    NewSsh,
    Settings,
    ToggleSidebar,
    NewTab,
    CloseTab(Uuid),
    ClosePane(Uuid),
    Split(Uuid, SplitDirection),
    Focus(Uuid),
    Reconnect(Uuid),
    History(Uuid),
    RenameWorkspace(Uuid),
    EditWorkspace(Uuid),
    DuplicateWorkspace(Uuid),
    DeleteWorkspace(Uuid),
    SelectWorkspace(Uuid),
    SelectTab(Uuid),
    MoveWorkspace(Uuid, usize),
    Color(Uuid, String),
    RenameTab(Uuid),
    Clear,
    Scroll(bool),
    Zoom(f32),
    ZoomReset,
}

/// Actions can be reported in any sizing pass. Commit their union once the
/// frame's visible pass is known, preserving repeated inputs within one pass.
#[derive(Clone, Default)]
struct FrameActions {
    frame: u64,
    actions: Vec<Action>,
    committed: usize,
}

impl FrameActions {
    fn merge_pass(&mut self, actions: Vec<Action>) {
        let previous = self.actions.clone();
        for (index, action) in actions.iter().enumerate() {
            let in_this_pass = actions[..=index]
                .iter()
                .filter(|other| *other == action)
                .count();
            let in_previous_passes = previous.iter().filter(|other| *other == action).count();
            if in_this_pass > in_previous_passes {
                self.actions.push(action.clone());
            }
        }
    }
}

#[derive(Default)]
struct WorkspaceForm {
    open: bool,
    id: Option<Uuid>,
    ssh: bool,
    tmux: bool,
    name: String,
    cwd: String,
    command: String,
    prefix: String,
    error: String,
}
struct RenameForm {
    id: Uuid,
    tab: bool,
    value: String,
}
struct HistoryView {
    id: Uuid,
    text: String,
    query: String,
}

pub struct BmuxApp {
    state: State,
    settings: Settings,
    store: Store,
    hosts: HashMap<Uuid, Host>,
    settings_open: bool,
    form: WorkspaceForm,
    rename: Option<RenameForm>,
    history: Option<HistoryView>,
    errors: Vec<String>,
    actions: Vec<Action>,
    last_save: Instant,
    last_state: String,
    last_settings: String,
    font_path: String,
    zoom: f32,
    dragging_workspace: Option<Uuid>,
    navigator_revealed: bool,
    navigator_band: bool,
    last_metadata: Instant,
    corner_mask: Option<std::sync::Arc<crate::window_shape::CornerMask>>,
}

impl BmuxApp {
    pub fn new(cc: &eframe::CreationContext<'_>, store: Store) -> Self {
        let mut errors = Vec::new();
        let state = store.load_state().unwrap_or_else(|e| {
            errors.push(format!("Could not restore workspaces: {e:#}"));
            State::default()
        });
        let settings = store.load_settings().unwrap_or_else(|e| {
            errors.push(format!("Could not restore settings: {e:#}"));
            Settings::default()
        });
        let corner_mask = crate::window_shape::CornerMask::install(cc)
            .map_err(|error| errors.push(format!("Could not round window corners: {error:#}")))
            .ok();
        let mut app = Self {
            state,
            settings,
            store,
            hosts: HashMap::new(),
            settings_open: false,
            form: WorkspaceForm::default(),
            rename: None,
            history: None,
            errors,
            actions: Vec::new(),
            last_save: Instant::now(),
            last_state: String::new(),
            last_settings: String::new(),
            font_path: String::new(),
            zoom: 0.0,
            dragging_workspace: None,
            navigator_revealed: false,
            navigator_band: false,
            last_metadata: Instant::now(),
            corner_mask,
        };
        app.load_fonts(&cc.egui_ctx);
        app
    }

    fn load_fonts(&mut self, ctx: &egui::Context) {
        let mut fonts = egui::FontDefinitions::default();
        fonts.font_data.insert(
            "Geist Mono".into(),
            egui::FontData::from_static(include_bytes!("../Assets/Fonts/GeistMono-Regular.ttf"))
                .into(),
        );
        fonts
            .families
            .get_mut(&egui::FontFamily::Monospace)
            .unwrap()
            .insert(0, "Geist Mono".into());
        fonts.font_data.insert(
            "Geist Mono Bold".into(),
            egui::FontData::from_static(include_bytes!("../Assets/Fonts/GeistMono-SemiBold.ttf"))
                .into(),
        );
        fonts.families.insert(
            egui::FontFamily::Name("terminal-bold".into()),
            vec!["Geist Mono Bold".into()],
        );
        let path = self.settings.font_family.trim();
        if !path.is_empty() {
            match crate::settings::load_font_file(std::path::Path::new(path)) {
                Ok(data) => {
                    fonts
                        .font_data
                        .insert("Custom".into(), egui::FontData::from_owned(data).into());
                    fonts
                        .families
                        .get_mut(&egui::FontFamily::Monospace)
                        .unwrap()
                        .insert(0, "Custom".into());
                }
                Err(e) => self.errors.push(format!("Could not load font {path}: {e}")),
            }
        }
        ctx.set_fonts(fonts);
        self.font_path = self.settings.font_family.clone();
    }

    fn workspace(&self) -> Option<&Workspace> {
        self.state.active()
    }
    fn workspace_mut(&mut self) -> Option<&mut Workspace> {
        self.state.active_mut()
    }
    fn tab(&self) -> Option<&Tab> {
        self.state.active_tab()
    }
    fn tab_mut(&mut self) -> Option<&mut Tab> {
        self.state.active_tab_mut()
    }
    fn focus(&self) -> Option<Uuid> {
        self.tab().map(|t| t.focused_pane)
    }
    fn modal(&self) -> bool {
        self.settings_open || self.form.open || self.rename.is_some() || self.history.is_some()
    }

    fn ensure_hosts(&mut self, ctx: &egui::Context) {
        crate::terminal::service_cleanup(&self.store.transcript_dir());
        for ws in &self.state.workspaces {
            // Mirror terminal_area's tab pick: only the visible tab paints.
            let visible = ws
                .tabs
                .iter()
                .find(|t| t.id == ws.active_tab)
                .or(ws.tabs.first())
                .map(|t| t.id);
            for tab in &ws.tabs {
                for pane in tab.root.panes() {
                    let id = pane.id;
                    let host = self.hosts.entry(id).or_insert(Host {
                        terminal: None,
                        error: None,
                        retry_at: None,
                        attempts: 0,
                    });
                    if let Some(term) = host.terminal.as_mut() {
                        term.poll();
                        if !term.is_alive() && host.retry_at.is_none() {
                            let persistent = ws.uses_tmux();
                            if persistent
                                || (self.settings.auto_relaunch
                                    && term.started_elapsed().as_secs_f64()
                                        >= self.settings.relaunch_after_seconds)
                            {
                                let delay = if persistent {
                                    2_u64.saturating_pow(host.attempts.min(5)).max(2)
                                } else {
                                    1
                                };
                                host.retry_at = Some(Instant::now() + Duration::from_secs(delay));
                            }
                        }
                    }
                    if host.retry_at.is_some_and(|at| at <= Instant::now()) {
                        host.terminal.take();
                        host.error.take();
                        host.retry_at = None;
                        host.attempts += 1;
                    }
                    if host.terminal.is_some() || host.error.is_some() {
                        continue;
                    }
                    // Spawn only for the visible tab: its first paint sizes
                    // the PTY in the same frame, so neither live output nor a
                    // tmux capture dump can land on the 24x80 spawn grid and
                    // wrap. Hidden tabs spawn on first view instead of
                    // recording garbage they can never reflow.
                    if Some(tab.id) != visible {
                        continue;
                    }
                    let kind = if ws.kind == WorkspaceKind::Ssh {
                        SessionKind::Ssh {
                            command: ws.ssh_command.clone(),
                            session: session_name(ws, id),
                        }
                    } else if ws.local_tmux {
                        SessionKind::LocalTmux {
                            session: session_name(ws, id),
                        }
                    } else {
                        SessionKind::Local
                    };
                    let cwd = if pane.working_directory.is_empty() {
                        &ws.working_directory
                    } else {
                        &pane.working_directory
                    };
                    let spec = SpawnSpec {
                        pane_id: id.to_string(),
                        cwd: PathBuf::from(cwd),
                        command: None,
                        shell: self.settings.shell_path.clone(),
                        kind,
                        transcript_dir: self.store.transcript_dir(),
                        transcript_max_mb: self.settings.transcript_max_mb as usize,
                        scrollback_lines: (self.settings.scrollback_mb.unwrap_or(10) as usize
                            * 1024)
                            .clamp(1024, 200_000),
                    };
                    match Terminal::spawn(spec, ctx.clone()) {
                        Ok(t) => host.terminal = Some(t),
                        Err(e) => host.error = Some(format!("{e:#}")),
                    }
                }
            }
        }
        if self.last_metadata.elapsed() > Duration::from_millis(700) {
            for ws in &mut self.state.workspaces {
                for tab in &mut ws.tabs {
                    let ids: Vec<_> = tab.root.panes().iter().map(|p| p.id).collect();
                    for id in ids {
                        if let Some(t) = self.hosts.get(&id).and_then(|h| h.terminal.as_ref())
                            && let Some(pane) = tab.root.pane_mut(id)
                        {
                            let path = t.cwd().to_string_lossy().into_owned();
                            if !path.is_empty() {
                                if ws.kind == WorkspaceKind::Ssh {
                                    pane.remote_working_directory = path;
                                } else {
                                    pane.working_directory = path;
                                }
                            }
                            let command = t.foreground_command();
                            pane.running_command = if [
                                "sh",
                                "bash",
                                "zsh",
                                "fish",
                                "dash",
                                "ksh",
                                "tcsh",
                                "csh",
                                "nu",
                                "xonsh",
                                "powershell",
                                "pwsh",
                                "cmd",
                            ]
                            .contains(&command.trim_start_matches('-').trim_end_matches(".exe"))
                            {
                                String::new()
                            } else {
                                command
                            };
                        }
                    }
                }
            }
            self.last_metadata = Instant::now();
        }
    }

    fn save(&mut self) {
        if self.last_save.elapsed() < Duration::from_millis(500) {
            return;
        }
        self.last_save = Instant::now();
        let state = serde_json::to_string(&self.state).unwrap_or_default();
        if state != self.last_state {
            match self.store.save_state(&self.state) {
                Ok(()) => self.last_state = state,
                Err(e) => self.push_error(format!("Could not save workspaces: {e:#}")),
            }
        }
        let settings = serde_json::to_string(&self.settings).unwrap_or_default();
        if settings != self.last_settings {
            match self.store.save_settings(&self.settings) {
                Ok(()) => self.last_settings = settings,
                Err(e) => self.push_error(format!("Could not save settings: {e:#}")),
            }
        }
    }
    fn push_error(&mut self, error: String) {
        if !self.errors.contains(&error) {
            self.errors.push(error);
        }
    }

    fn keyboard(&mut self, ctx: &egui::Context) {
        if self.modal() {
            return;
        }
        let events = ctx.input(|i| i.events.clone());
        for event in events {
            let egui::Event::Key {
                key,
                pressed: true,
                modifiers: m,
                ..
            } = event
            else {
                continue;
            };
            if !m.ctrl || !m.shift {
                continue;
            }
            let focus = self.focus();
            let action = match key {
                Key::S => Some(Action::ToggleSidebar),
                Key::N => Some(if m.alt {
                    Action::NewSsh
                } else {
                    Action::NewWorkspace(false)
                }),
                Key::T => Some(Action::NewTab),
                Key::W if m.alt => self.tab().map(|t| Action::CloseTab(t.id)),
                Key::W => focus.map(Action::ClosePane),
                Key::D => focus.map(|id| Action::Split(id, SplitDirection::SideBySide)),
                Key::E => focus.map(|id| Action::Split(id, SplitDirection::Stacked)),
                Key::H => focus.map(Action::History),
                Key::R => focus.map(Action::Reconnect),
                Key::K => Some(Action::Clear),
                Key::Home => Some(Action::Scroll(true)),
                Key::End => Some(Action::Scroll(false)),
                Key::Plus | Key::Equals => Some(Action::Zoom(1.0)),
                Key::Minus => Some(Action::Zoom(-1.0)),
                Key::Num0 => Some(Action::ZoomReset),
                Key::Comma => Some(Action::Settings),
                Key::OpenBracket | Key::CloseBracket => {
                    self.cycle_tab(if key == Key::OpenBracket { -1 } else { 1 })
                }
                Key::ArrowLeft | Key::ArrowRight if m.alt => {
                    self.cycle_pane(if key == Key::ArrowLeft { -1 } else { 1 })
                }
                Key::ArrowUp | Key::ArrowDown => {
                    self.cycle_workspace(if key == Key::ArrowUp { -1 } else { 1 })
                }
                _ => number_key(key).and_then(|n| {
                    if m.alt {
                        let n = if n == 9 {
                            self.state.workspaces.len().saturating_sub(1)
                        } else {
                            n - 1
                        };
                        self.state
                            .workspaces
                            .get(n)
                            .map(|w| Action::SelectWorkspace(w.id))
                    } else {
                        self.workspace().and_then(|w| {
                            let n = if n == 9 {
                                w.tabs.len().saturating_sub(1)
                            } else {
                                n - 1
                            };
                            w.tabs.get(n).map(|t| Action::SelectTab(t.id))
                        })
                    }
                }),
            };
            if let Some(a) = action {
                ctx.input_mut(|i| {
                    i.consume_key(m, key);
                    i.events.retain(|e| !matches!(e, egui::Event::Text(_)));
                });
                self.actions.push(a);
            }
        }
    }
    fn cycle_tab(&self, by: isize) -> Option<Action> {
        let w = self.workspace()?;
        let i = w
            .tabs
            .iter()
            .position(|t| t.id == w.active_tab)
            .unwrap_or(0);
        cycle_index(i, by, w.tabs.len()).map(|j| Action::SelectTab(w.tabs[j].id))
    }
    fn cycle_workspace(&self, by: isize) -> Option<Action> {
        let i = self
            .state
            .workspaces
            .iter()
            .position(|w| w.id == self.state.selected)
            .unwrap_or(0);
        cycle_index(i, by, self.state.workspaces.len())
            .map(|j| Action::SelectWorkspace(self.state.workspaces[j].id))
    }
    fn cycle_pane(&self, by: isize) -> Option<Action> {
        let tab = self.tab()?;
        let panes = tab.root.panes();
        let i = panes
            .iter()
            .position(|p| p.id == tab.focused_pane)
            .unwrap_or(0);
        cycle_index(i, by, panes.len()).map(|j| Action::Focus(panes[j].id))
    }

    fn toolbar(&mut self, ctx: &egui::Context, colors: &ThemeColors) {
        use egui::emath::GuiRounding as _;

        egui::TopBottomPanel::top("titlebar")
            .exact_height(46.0)
            .show_separator_line(false)
            .frame(egui::Frame::NONE)
            .show(ctx, |ui| {
                ui.visuals_mut().widgets.inactive.weak_bg_fill = Color32::TRANSPARENT;
                let rect = ui.max_rect();
                let scale = ui.painter().pixels_per_point();
                // Match egui's pixel-rounded panel scissor at fractional DPI.
                // Keep the logical layout at 46pt so controls and dragging stay put.
                let paint_rect = rect.round_to_pixels(scale);
                crate::glass::paint_surface(ui.painter(), paint_rect, colors, 0, false);
                let divider_height = (scale.round().max(1.0) / scale).min(paint_rect.height());
                let divider = Rect::from_min_max(
                    Pos2::new(paint_rect.left(), paint_rect.bottom() - divider_height),
                    paint_rect.max,
                );
                // A translucent separator still shows the desktop through the
                // titlebar/body boundary. Composite its tint into an opaque fill.
                let mut divider_mesh = egui::Mesh::default();
                divider_mesh.add_colored_rect(divider, split_separator_color(colors, false));
                ui.painter().add(divider_mesh);
                let drag = ui.interact(rect, Id::new("titlebar-drag"), Sense::click_and_drag());
                if drag.drag_started() {
                    ctx.send_viewport_cmd(egui::ViewportCommand::StartDrag);
                }
                if drag.double_clicked() {
                    let max = ctx.input(|i| i.viewport().maximized.unwrap_or(false));
                    ctx.send_viewport_cmd(egui::ViewportCommand::Maximized(!max));
                }
                let title = self
                    .tab()
                    .and_then(|t| t.root.panes().into_iter().find(|p| p.id == t.focused_pane))
                    .map(|p| {
                        if self
                            .workspace()
                            .is_some_and(|w| w.kind == WorkspaceKind::Ssh)
                        {
                            p.remote_working_directory.clone()
                        } else {
                            p.working_directory.clone()
                        }
                    })
                    .filter(|s| !s.is_empty())
                    .unwrap_or_default();
                let title_text = short_path(&title);
                let center = Pos2::new(
                    if self.state.sidebar_visible {
                        (rect.left() + self.state.sidebar_width + rect.right() - 105.0) * 0.5
                    } else {
                        rect.center().x
                    },
                    rect.center().y,
                );
                ui.painter().text(
                    center,
                    Align2::CENTER_CENTER,
                    title_text,
                    FontId::monospace(12.0),
                    colors.foreground.gamma_multiply(0.68),
                );
                let title_rect = Rect::from_center_size(center, Vec2::new(220.0, 32.0));
                ui.interact(title_rect, Id::new("path-status"), Sense::click())
                    .on_hover_text(&title)
                    .context_menu(|ui| {
                        if ui.button("Copy path").clicked() {
                            ctx.copy_text(title.clone());
                            ui.close();
                        }
                        if self
                            .workspace()
                            .is_some_and(|w| w.kind == WorkspaceKind::Local)
                            && ui.button("Open folder").clicked()
                        {
                            if let Err(e) = open::that(&title) {
                                self.errors.push(e.to_string());
                            }
                            ui.close();
                        }
                    });
                ui.scope_builder(
                    egui::UiBuilder::new().max_rect(Rect::from_min_size(
                        rect.left_top() + Vec2::new(8.0, 8.0),
                        Vec2::new(200.0, 30.0),
                    )),
                    |ui| {
                        ui.horizontal(|ui| {
                            if icon_button(ui, "◧", "Toggle sidebar · Ctrl+Shift+S").clicked() {
                                self.actions.push(Action::ToggleSidebar);
                            }
                            ui.menu_button(RichText::new("+").size(21.0), |ui| {
                                self.new_menu(ui);
                            });
                            ui.menu_button(RichText::new("[bmux]").monospace().size(13.0), |ui| {
                                ui.menu_button("Workspace", |ui| self.new_menu(ui));
                                ui.separator();
                                let focused = self.focus();
                                if menu_item(ui, "New terminal tab", "Ctrl+Shift+T") {
                                    self.actions.push(Action::NewTab);
                                }
                                if let Some(id) = focused {
                                    if menu_item(ui, "Split right", "Ctrl+Shift+D") {
                                        self.actions
                                            .push(Action::Split(id, SplitDirection::SideBySide));
                                    }
                                    if menu_item(ui, "Split down", "Ctrl+Shift+E") {
                                        self.actions
                                            .push(Action::Split(id, SplitDirection::Stacked));
                                    }
                                    if menu_item(ui, "View history", "Ctrl+Shift+H") {
                                        self.actions.push(Action::History(id));
                                    }
                                    if menu_item(ui, "Reconnect", "Ctrl+Shift+R") {
                                        self.actions.push(Action::Reconnect(id));
                                    }
                                }
                                ui.separator();
                                if menu_item(ui, "Settings…", "Ctrl+Shift+,") {
                                    self.actions.push(Action::Settings);
                                }
                                if ui.button("Open data folder").clicked() {
                                    if let Err(e) = open::that(self.store.data_dir()) {
                                        self.errors.push(e.to_string());
                                    }
                                    ui.close();
                                }
                                if ui.button("Quit").clicked() {
                                    ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                                }
                            });
                        });
                    },
                );
                ui.scope_builder(
                    egui::UiBuilder::new().max_rect(Rect::from_min_max(
                        Pos2::new(rect.right() - 108.0, rect.top() + 7.0),
                        rect.right_bottom(),
                    )),
                    |ui| {
                        ui.horizontal(|ui| {
                            if icon_button(ui, "−", "Minimize").clicked() {
                                ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(true));
                            }
                            if icon_button(ui, "□", "Maximize / restore").clicked() {
                                let max = ctx.input(|i| i.viewport().maximized.unwrap_or(false));
                                ctx.send_viewport_cmd(egui::ViewportCommand::Maximized(!max));
                            }
                            if icon_button(
                                ui,
                                "×",
                                "Close window; persistent sessions keep running",
                            )
                            .clicked()
                            {
                                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                            }
                        });
                    },
                );
                if self.state.sidebar_visible {
                    ui.painter().line_segment(
                        [
                            Pos2::new(self.state.sidebar_width, rect.top()),
                            Pos2::new(self.state.sidebar_width, rect.bottom()),
                        ],
                        Stroke::new(1.0_f32, colors.foreground.gamma_multiply(0.12)),
                    );
                }
            });
    }

    fn new_menu(&mut self, ui: &mut egui::Ui) {
        if menu_item(ui, "New local workspace", "Ctrl+Shift+N") {
            self.actions.push(Action::NewWorkspace(false));
        }
        if ui.button("New local tmux workspace").clicked() {
            self.actions.push(Action::NewWorkspace(true));
            ui.close();
        }
        if menu_item(ui, "New SSH workspace…", "Ctrl+Shift+Alt+N") {
            self.actions.push(Action::NewSsh);
        }
    }

    fn sidebar(&mut self, ctx: &egui::Context, colors: &ThemeColors) {
        if !self.state.sidebar_visible {
            return;
        }
        let response = egui::SidePanel::left("workspaces")
            .resizable(true)
            .default_width(self.state.sidebar_width)
            .width_range(210.0..=340.0)
            .frame(egui::Frame::new().inner_margin(egui::Margin::symmetric(8, 8)))
            .show(ctx, |ui| {
                crate::glass::paint_surface(
                    ui.painter(),
                    ui.max_rect().expand(8.0),
                    colors,
                    0,
                    false,
                );
                egui::ScrollArea::vertical().show(ui, |ui| {
                    let workspaces = self.state.workspaces.clone();
                    for (index, ws) in workspaces.iter().enumerate() {
                        let panes: Vec<_> = ws.tabs.iter().flat_map(|t| t.root.panes()).collect();
                        let height = (35.0 + panes.len() as f32 * 16.0).max(50.0);
                        let (rect, res) = ui.allocate_exact_size(
                            Vec2::new(ui.available_width(), height),
                            Sense::click_and_drag(),
                        );
                        let accent =
                            parse_hex(&ws.color_hex).unwrap_or(Color32::from_rgb(255, 135, 0));
                        if ws.id == self.state.selected {
                            crate::glass::paint_surface(ui.painter(), rect, colors, 9, true);
                            ui.painter()
                                .rect_filled(rect, 9, accent.gamma_multiply(0.09));
                        } else if res.hovered() {
                            ui.painter().rect_filled(
                                rect,
                                6,
                                colors.foreground.gamma_multiply(0.045),
                            );
                        }
                        if !ws.color_hex.is_empty() {
                            ui.painter().rect_filled(
                                Rect::from_min_max(
                                    rect.left_top() + Vec2::new(10.0, 8.0),
                                    Pos2::new(rect.left() + 13.0, rect.bottom() - 8.0),
                                ),
                                2,
                                accent,
                            );
                        }
                        let text_rect = rect.shrink2(Vec2::new(23.0, 8.0));
                        let paint = ui.painter().with_clip_rect(Rect::from_min_max(
                            text_rect.min,
                            Pos2::new(rect.right() - 55.0, rect.bottom()),
                        ));
                        paint.text(
                            text_rect.left_top(),
                            Align2::LEFT_TOP,
                            &ws.name,
                            FontId::proportional(13.0),
                            colors.foreground,
                        );
                        let badge = if ws.kind == WorkspaceKind::Ssh {
                            "SSH"
                        } else if ws.local_tmux {
                            "TMUX"
                        } else {
                            ""
                        };
                        ui.painter().text(
                            Pos2::new(rect.right() - 10.0, rect.top() + 8.0),
                            Align2::RIGHT_TOP,
                            format!("{} · {}", ws.tabs.len(), panes.len()),
                            FontId::proportional(11.0),
                            colors.foreground.gamma_multiply(0.48),
                        );
                        for (i, pane) in panes.iter().enumerate() {
                            let path = if ws.kind == WorkspaceKind::Ssh {
                                &pane.remote_working_directory
                            } else {
                                &pane.working_directory
                            };
                            let text = if !pane.running_command.is_empty() {
                                format!("[{}]", pane.running_command)
                            } else if path.is_empty() {
                                ws.ssh_command.clone()
                            } else {
                                short_path(path)
                            };
                            paint.text(
                                text_rect.left_top() + Vec2::new(0.0, 19.0 + i as f32 * 16.0),
                                Align2::LEFT_TOP,
                                text,
                                FontId::monospace(11.0),
                                colors.foreground.gamma_multiply(0.50),
                            );
                        }
                        if !badge.is_empty() {
                            ui.painter().text(
                                Pos2::new(rect.right() - 10.0, rect.bottom() - 9.0),
                                Align2::RIGHT_BOTTOM,
                                badge,
                                FontId::proportional(9.0),
                                colors.foreground.gamma_multiply(0.40),
                            );
                        }
                        if res.clicked() {
                            self.actions.push(Action::SelectWorkspace(ws.id));
                        }
                        if res.double_clicked() {
                            self.actions.push(Action::RenameWorkspace(ws.id));
                        }
                        if res.drag_started() {
                            self.dragging_workspace = Some(ws.id);
                        }
                        if res.hovered()
                            && ctx.input(|i| i.pointer.any_released())
                            && let Some(id) = self.dragging_workspace.take()
                        {
                            self.actions.push(Action::MoveWorkspace(id, index));
                        }
                        res.on_hover_text(format!(
                            "{}\n{} terminals, {} panes\n{}",
                            ws.name,
                            ws.tabs.len(),
                            panes.len(),
                            ws.working_directory
                        ))
                        .context_menu(|ui| {
                            if ui.button("Open").clicked() {
                                self.actions.push(Action::SelectWorkspace(ws.id));
                                ui.close();
                            }
                            ui.separator();
                            if ui.button("Rename…").clicked() {
                                self.actions.push(Action::RenameWorkspace(ws.id));
                                ui.close();
                            }
                            if ui.button("Edit workspace…").clicked() {
                                self.actions.push(Action::EditWorkspace(ws.id));
                                ui.close();
                            }
                            if ui.button("Duplicate").clicked() {
                                self.actions.push(Action::DuplicateWorkspace(ws.id));
                                ui.close();
                            }
                            ui.menu_button("Workspace color", |ui| {
                                for (name, color) in [
                                    ("Default", ""),
                                    ("Blue", "#4DA3FF"),
                                    ("Teal", "#2DD4BF"),
                                    ("Green", "#7ED957"),
                                    ("Yellow", "#F7C948"),
                                    ("Orange", "#FF9F43"),
                                    ("Pink", "#F472B6"),
                                    ("Purple", "#A78BFA"),
                                ] {
                                    if ui
                                        .button(
                                            RichText::new(name).color(
                                                parse_hex(color).unwrap_or(colors.foreground),
                                            ),
                                        )
                                        .clicked()
                                    {
                                        self.actions.push(Action::Color(ws.id, color.into()));
                                        ui.close();
                                    }
                                }
                            });
                            if index > 0 && ui.button("Move up").clicked() {
                                self.actions.push(Action::MoveWorkspace(ws.id, index - 1));
                                ui.close();
                            }
                            if index + 1 < workspaces.len() && ui.button("Move down").clicked() {
                                self.actions.push(Action::MoveWorkspace(ws.id, index + 1));
                                ui.close();
                            }
                            ui.separator();
                            if ui
                                .button(
                                    RichText::new("Delete workspace")
                                        .color(Color32::from_rgb(255, 85, 70)),
                                )
                                .clicked()
                            {
                                self.actions.push(Action::DeleteWorkspace(ws.id));
                                ui.close();
                            }
                        });
                        ui.add_space(2.0);
                    }
                });
            });
        self.state.sidebar_width = response.response.rect.width();
    }

    fn terminal_area(&mut self, ctx: &egui::Context, colors: &ThemeColors) {
        egui::CentralPanel::default()
            .frame(egui::Frame::NONE)
            .show(ctx, |ui| {
                let rect = ui.max_rect();
                let Some(ws) = self.workspace().cloned() else {
                    ui.painter().rect_filled(rect, 0, colors.background);
                    ui.centered_and_justified(|ui| {
                        ui.vertical_centered(|ui| {
                            ui.heading("No workspace");
                            ui.label("Create a workspace to open a terminal.");
                            if ui.button("New local workspace").clicked() {
                                self.actions.push(Action::NewWorkspace(false));
                            }
                        });
                    });
                    return;
                };
                let Some(tab) = ws
                    .tabs
                    .iter()
                    .find(|t| t.id == ws.active_tab)
                    .or(ws.tabs.first())
                    .cloned()
                else {
                    return;
                };
                let style = TerminalStyle {
                    background: colors.background,
                    foreground: colors.foreground,
                    cursor: colors.cursor,
                    selection: colors.selection,
                    palette: colors.palette,
                    font_size: (self.settings.font_size.unwrap_or(13.0) + self.zoom)
                        .clamp(6.0, 48.0),
                    padding: self.settings.padding_x as f32,
                    padding_y: self.settings.padding_y as f32,
                    cursor_blink: self.settings.cursor_blink,
                    cursor_opacity: self.settings.cursor_opacity,
                    cursor_shape: match format!("{:?}", self.settings.cursor_style)
                        .to_lowercase()
                        .as_str()
                    {
                        "bar" => CursorShape::Bar,
                        "underline" => CursorShape::Underline,
                        _ => CursorShape::Block,
                    },
                    resize_throttle_ms: self.settings.resize_throttle_ms.unwrap_or(0.0),
                    ..Default::default()
                };
                // Hover-reveal navigation: the strip stays hidden until the
                // pointer enters a thin band at the top edge, then floats over
                // the terminal as glass pills without pushing the grid around
                // or covering a full row with an opaque bar. Entering the band
                // with a button held (including the press and release of a
                // click) never reveals it; only a button-free entry does. An
                // open popup pins it visible so menus are never orphaned.
                let eligible = self.settings.show_single_tab
                    || ws.tabs.len() > 1
                    || tab.root.panes().len() > 1;
                let strip_height = 42.0;
                let reveal_band = 12.0;
                let hovered = ctx.input(|i| i.pointer.hover_pos());
                let in_band = hovered.is_some_and(|pos| {
                    pos.x >= rect.left()
                        && pos.x <= rect.right()
                        && pos.y >= rect.top()
                        && pos.y < rect.top() + reveal_band
                });
                let in_strip = hovered.is_some_and(|pos| {
                    pos.x >= rect.left()
                        && pos.x <= rect.right()
                        && pos.y >= rect.top()
                        && pos.y < rect.top() + strip_height
                });
                let pressed = ctx.input(|i| i.pointer.primary_down() || i.pointer.secondary_down());
                let entered = in_band && !self.navigator_band;
                let revealed = eligible
                    && ((in_strip && (self.navigator_revealed || (entered && !pressed)))
                        || egui::Popup::is_any_open(ctx));
                self.navigator_revealed = revealed;
                self.navigator_band = in_band;
                let mut root = tab.root.clone();
                self.split_ui(
                    ui,
                    &mut root,
                    rect,
                    tab.focused_pane,
                    &style,
                    colors,
                    !self.modal() && !egui::Popup::is_any_open(ctx),
                );
                if revealed {
                    let navigator = Rect::from_min_max(
                        rect.min,
                        Pos2::new(rect.right(), (rect.top() + strip_height).min(rect.bottom())),
                    );
                    self.tab_overlay(ui, navigator, &ws, &tab, colors);
                }
                if ctx.input(|i| !i.raw.hovered_files.is_empty()) {
                    ui.painter().text(
                        rect.center_top() + Vec2::new(0.0, 46.0),
                        Align2::CENTER_TOP,
                        "Drop files to paste their paths",
                        FontId::monospace(12.0),
                        colors.foreground.gamma_multiply(0.75),
                    );
                }
                if let Some(t) = self.tab_mut() {
                    t.root = root;
                }
            });
    }

    #[allow(clippy::too_many_arguments)] // Recursive layout carries one shared render context.
    fn split_ui(
        &mut self,
        ui: &mut egui::Ui,
        node: &mut SplitNode,
        rect: Rect,
        focus: Uuid,
        style: &TerminalStyle,
        colors: &ThemeColors,
        interactive: bool,
    ) {
        if !rect.is_positive() || !rect.is_finite() {
            return;
        }
        match node {
            SplitNode::Pane(pane) => {
                let focused = pane.id == focus;
                if self
                    .hosts
                    .get(&pane.id)
                    .is_none_or(|host| host.terminal.is_none())
                {
                    ui.painter().rect_filled(rect, 0, colors.background);
                }
                if let Some(host) = self.hosts.get_mut(&pane.id) {
                    if let Some(term) = host.terminal.as_mut() {
                        let response = term.show(ui, rect, focused && interactive, style);
                        if (response.clicked() || response.secondary_clicked()) && interactive {
                            self.actions.push(Action::Focus(pane.id));
                        }
                        if !term.is_alive() {
                            let message = if host.retry_at.is_some() {
                                "Disconnected · reconnecting…".to_owned()
                            } else {
                                format!("Shell exited ({:?})", term.exit_code())
                            };
                            ui.painter().rect_filled(
                                rect,
                                0,
                                colors.background.gamma_multiply(0.86),
                            );
                            ui.scope_builder(
                                egui::UiBuilder::new().max_rect(Rect::from_center_size(
                                    rect.center(),
                                    Vec2::new(320.0, 100.0),
                                )),
                                |ui| {
                                    ui.vertical_centered(|ui| {
                                        ui.heading(message);
                                        ui.add_space(10.0);
                                        ui.horizontal(|ui| {
                                            if ui.button("Reconnect").clicked() {
                                                self.actions.push(Action::Reconnect(pane.id));
                                            }
                                            if ui.button("View History").clicked() {
                                                self.actions.push(Action::History(pane.id));
                                            }
                                        });
                                    });
                                },
                            );
                        }
                    } else {
                        ui.scope_builder(
                            egui::UiBuilder::new().max_rect(rect.shrink(30.0)),
                            |ui| {
                                ui.vertical_centered(|ui| {
                                    ui.add_space((rect.height() / 2.0 - 80.0).max(0.0));
                                    ui.heading("Unable to start terminal");
                                    ui.add_space(8.0);
                                    ui.label(host.error.as_deref().unwrap_or("Starting…"));
                                    ui.add_space(12.0);
                                    if ui.button("Retry").clicked() {
                                        self.actions.push(Action::Reconnect(pane.id));
                                    }
                                });
                            },
                        );
                    }
                }
                if !focused {
                    ui.painter().rect_filled(
                        rect,
                        0,
                        colors
                            .background
                            .gamma_multiply(self.settings.unfocused_dim),
                    );
                }
                let hovered = ui
                    .ctx()
                    .input(|i| i.pointer.hover_pos())
                    .is_some_and(|p| rect.contains(p));
                let menu_key = Id::new(("pane-actions-open", pane.id));
                let menu_open = ui
                    .ctx()
                    .data(|data| data.get_temp::<bool>(menu_key).unwrap_or(false));
                if (hovered || menu_open) && !self.modal() {
                    // The glass and the button share one centered box so the
                    // glyph cannot drift out of its pill.
                    let actions_rect = Rect::from_min_size(
                        rect.right_bottom() - Vec2::new(48.0, 38.0),
                        Vec2::new(42.0, 32.0),
                    );
                    crate::glass::paint_surface(ui.painter(), actions_rect, colors, 10, true);
                    ui.scope_builder(
                        egui::UiBuilder::new()
                            .id_salt(("pane-actions", pane.id))
                            .max_rect(actions_rect),
                        |ui| {
                            ui.centered_and_justified(|ui| {
                                let menu = ui.menu_button("···", |ui| {
                                    if ui.button("Split right").clicked() {
                                        self.actions.push(Action::Split(
                                            pane.id,
                                            SplitDirection::SideBySide,
                                        ));
                                        ui.close();
                                    }
                                    if ui.button("Split down").clicked() {
                                        self.actions
                                            .push(Action::Split(pane.id, SplitDirection::Stacked));
                                        ui.close();
                                    }
                                    ui.separator();
                                    if ui.button("View history").clicked() {
                                        self.actions.push(Action::History(pane.id));
                                        ui.close();
                                    }
                                    if ui.button("Reconnect").clicked() {
                                        self.actions.push(Action::Reconnect(pane.id));
                                        ui.close();
                                    }
                                    if ui.button("Close pane").clicked() {
                                        self.actions.push(Action::ClosePane(pane.id));
                                        ui.close();
                                    }
                                });
                                ui.ctx().data_mut(|data| {
                                    data.insert_temp(menu_key, menu.inner.is_some())
                                });
                            });
                        },
                    );
                }
            }
            SplitNode::Split {
                id,
                direction,
                ratio,
                first,
                second,
            } => {
                let horizontal = *direction == SplitDirection::SideBySide;
                let length = if horizontal {
                    rect.width()
                } else {
                    rect.height()
                };
                let min_ratio = (80.0 / length).min(0.45);
                *ratio = ratio.clamp(min_ratio, 1.0 - min_ratio);
                let (a, b, separator, line) =
                    split_rects(rect, horizontal, *ratio, ui.painter().pixels_per_point());
                self.split_ui(ui, first, a, focus, style, colors, interactive);
                self.split_ui(ui, second, b, focus, style, colors, interactive);
                let res = ui
                    .interact(line, Id::new(("split", *id)), Sense::drag())
                    .on_hover_cursor(if horizontal {
                        egui::CursorIcon::ResizeHorizontal
                    } else {
                        egui::CursorIcon::ResizeVertical
                    });
                if res.dragged()
                    && let Some(p) = res.interact_pointer_pos()
                {
                    *ratio = (if horizontal {
                        (p.x - rect.left()) / length
                    } else {
                        (p.y - rect.top()) / length
                    })
                    .clamp(min_ratio, 1.0 - min_ratio);
                }
                // The gap must have its own opaque fill: a translucent stroke
                // over the transparent window exposes the desktop. A mesh also
                // avoids anti-aliasing a one-pixel fill back to transparency.
                let mut mesh = egui::Mesh::default();
                mesh.add_colored_rect(
                    separator,
                    split_separator_color(colors, res.hovered() || res.dragged()),
                );
                ui.painter().add(mesh);
            }
        }
    }

    fn tab_overlay(
        &mut self,
        ui: &mut egui::Ui,
        rect: Rect,
        ws: &Workspace,
        tab: &Tab,
        colors: &ThemeColors,
    ) {
        let panes = tab.root.panes();
        let compact = rect.width() < 620.0;
        let show_panes = panes.len() > 1 && rect.width() >= 360.0;
        let controls_width =
            (if compact { 64.0 } else { 160.0 }) + if show_panes { 92.0 } else { 0.0 };
        let controls_rect = Rect::from_min_max(
            Pos2::new(
                (rect.right() - controls_width - 8.0).max(rect.left()),
                rect.top() + 7.0,
            ),
            Pos2::new(rect.right() - 8.0, rect.bottom() - 7.0),
        );
        let track_space = Rect::from_min_max(
            Pos2::new(rect.left() + 8.0, rect.top() + 4.0),
            Pos2::new(
                (controls_rect.left() - 8.0).max(rect.left() + 8.0),
                rect.bottom() - 4.0,
            ),
        );
        // Split panes belong to a tab's tree, just as in the Swift app. Expose
        // their count and focus picker without turning them into unrelated tabs.
        let show_tabs = self.settings.show_single_tab || ws.tabs.len() > 1 || panes.len() > 1;
        if show_tabs && track_space.width() > 0.0 {
            let natural_width = ws
                .tabs
                .iter()
                .enumerate()
                .map(|(i, t)| tab_width(&navigator_tab_label(ws, t, i)))
                .sum::<f32>()
                + 4.0 * ws.tabs.len().saturating_sub(1) as f32
                + 6.0;
            let track_width = natural_width.min(track_space.width());
            // Center the strip on the terminal view, clamped so it never
            // slides out of its track or under the controls on narrow views.
            let center_x = rect.center().x.clamp(
                track_space.left() + track_width / 2.0,
                track_space.right() - track_width / 2.0,
            );
            let track_rect = Rect::from_center_size(
                Pos2::new(center_x, track_space.center().y),
                Vec2::new(track_width, track_space.height()),
            );
            crate::glass::paint_surface(ui.painter(), track_rect, colors, 13, true);
            let scroll_key = Id::new(("tab-scroll-selection", ws.id));
            let selection = (
                tab.id,
                ws.tabs.len(),
                panes.len(),
                track_rect.width().to_bits(),
            );
            let reveal = ui.ctx().data_mut(|data| {
                let changed =
                    data.get_temp::<(Uuid, usize, usize, u32)>(scroll_key) != Some(selection);
                data.insert_temp(scroll_key, selection);
                changed
            });
            ui.scope_builder(
                egui::UiBuilder::new()
                    .id_salt(("tab-track", ws.id))
                    .max_rect(track_rect),
                |ui| {
                    ui.set_clip_rect(track_rect.intersect(ui.clip_rect()));
                    if self.modal() {
                        ui.disable();
                    }
                    egui::Frame::new()
                        .corner_radius(13)
                        .inner_margin(3)
                        .show(ui, |ui| {
                            egui::ScrollArea::horizontal()
                                .id_salt(("tab-scroll", ws.id))
                                .max_width((track_rect.width() - 6.0).max(0.0))
                                .max_height(28.0)
                                .auto_shrink([false, false])
                                .scroll_bar_visibility(
                                    egui::scroll_area::ScrollBarVisibility::AlwaysHidden,
                                )
                                .animated(false)
                                .show(ui, |ui| {
                                    ui.spacing_mut().item_spacing.x = 4.0;
                                    ui.horizontal(|ui| {
                                        for (i, t) in ws.tabs.iter().enumerate() {
                                            let label = navigator_tab_label(ws, t, i);
                                            let selected = t.id == tab.id;
                                            let (tab_rect, _) = ui.allocate_exact_size(
                                                Vec2::new(tab_width(&label), 28.0),
                                                Sense::hover(),
                                            );
                                            let hover = ui.rect_contains_pointer(tab_rect);
                                            let close_visible = match self.settings.tab_close_mode {
                                                crate::settings::TabCloseMode::Always => true,
                                                crate::settings::TabCloseMode::Hover => hover,
                                                crate::settings::TabCloseMode::Never => false,
                                            };
                                            let button_rect = if close_visible {
                                                Rect::from_min_max(
                                                    tab_rect.min,
                                                    tab_rect.max - Vec2::new(23.0, 0.0),
                                                )
                                            } else {
                                                tab_rect
                                            };
                                            ui.painter().rect_filled(
                                                tab_rect,
                                                10,
                                                colors.foreground.gamma_multiply(if selected {
                                                    0.16
                                                } else {
                                                    0.05
                                                }),
                                            );
                                            ui.painter()
                                                .with_clip_rect(
                                                    button_rect
                                                        .shrink(4.0)
                                                        .intersect(ui.clip_rect()),
                                                )
                                                .text(
                                                    button_rect.center(),
                                                    Align2::CENTER_CENTER,
                                                    &label,
                                                    FontId::proportional(12.0),
                                                    colors.foreground,
                                                );
                                            let res = ui.interact(
                                                button_rect,
                                                Id::new(("tab", t.id)),
                                                Sense::click(),
                                            );
                                            res.widget_info(|| {
                                                egui::WidgetInfo::selected(
                                                    egui::WidgetType::SelectableLabel,
                                                    ui.is_enabled(),
                                                    selected,
                                                    &label,
                                                )
                                            });
                                            if selected && reveal {
                                                ui.scroll_to_rect(tab_rect, None);
                                            }
                                            if res.clicked() {
                                                self.actions.push(Action::SelectTab(t.id));
                                            }
                                            if res.double_clicked() {
                                                self.actions.push(Action::RenameTab(t.id));
                                            }
                                            if close_visible {
                                                let close_rect = Rect::from_min_max(
                                                    Pos2::new(
                                                        tab_rect.right() - 23.0,
                                                        tab_rect.top(),
                                                    ),
                                                    tab_rect.max,
                                                );
                                                ui.painter().text(
                                                    close_rect.center(),
                                                    Align2::CENTER_CENTER,
                                                    "×",
                                                    FontId::proportional(14.0),
                                                    colors.foreground,
                                                );
                                                if ui
                                                    .interact(
                                                        close_rect,
                                                        Id::new(("close-tab", t.id)),
                                                        Sense::click(),
                                                    )
                                                    .on_hover_text("Close terminal tab")
                                                    .clicked()
                                                {
                                                    self.actions.push(Action::CloseTab(t.id));
                                                }
                                            }
                                            res.on_hover_text(format!(
                                                "{} · {} pane{}",
                                                tab_label(ws, t, i),
                                                t.root.panes().len(),
                                                if t.root.panes().len() == 1 { "" } else { "s" }
                                            ))
                                            .context_menu(|ui| {
                                                if ui.button("Rename terminal…").clicked() {
                                                    self.actions.push(Action::RenameTab(t.id));
                                                    ui.close();
                                                }
                                                if ui.button("Close terminal").clicked() {
                                                    self.actions.push(Action::CloseTab(t.id));
                                                    ui.close();
                                                }
                                            });
                                        }
                                    });
                                });
                        });
                },
            );
        }
        crate::glass::paint_surface(ui.painter(), controls_rect.expand(3.0), colors, 12, true);
        ui.scope_builder(
            egui::UiBuilder::new()
                .id_salt(("terminal-controls", ws.id))
                .max_rect(controls_rect),
            |ui| {
                if self.modal() {
                    ui.disable();
                }
                ui.set_clip_rect(rect.intersect(ui.clip_rect()));
                ui.spacing_mut().item_spacing.x = 4.0;
                ui.horizontal(|ui| {
                    if show_panes {
                        ui.menu_button(format!("{} panes", panes.len()), |ui| {
                            self.pane_navigation(ui, tab);
                        });
                        ui.add_space(4.0);
                    }
                    if !compact {
                        if icon_button(ui, "×", "Close focused pane").clicked() {
                            self.actions.push(Action::ClosePane(tab.focused_pane));
                        }
                        if icon_button(ui, "◫", "Split right").clicked() {
                            self.actions
                                .push(Action::Split(tab.focused_pane, SplitDirection::SideBySide));
                        }
                        if icon_button(ui, "⊟", "Split down").clicked() {
                            self.actions
                                .push(Action::Split(tab.focused_pane, SplitDirection::Stacked));
                        }
                    }
                    if icon_button(ui, "＋", "New terminal tab").clicked() {
                        self.actions.push(Action::NewTab);
                    }
                    ui.menu_button("···", |ui| {
                        ui.menu_button("Terminals", |ui| {
                            for (i, t) in ws.tabs.iter().enumerate() {
                                if ui
                                    .selectable_label(t.id == tab.id, navigator_tab_label(ws, t, i))
                                    .clicked()
                                {
                                    self.actions.push(Action::SelectTab(t.id));
                                    ui.close();
                                }
                            }
                        });
                        if panes.len() > 1 {
                            ui.menu_button(format!("{} panes", panes.len()), |ui| {
                                self.pane_navigation(ui, tab);
                            });
                        }
                        ui.separator();
                        if ui.button("Split right").clicked() {
                            self.actions
                                .push(Action::Split(tab.focused_pane, SplitDirection::SideBySide));
                            ui.close();
                        }
                        if ui.button("Split down").clicked() {
                            self.actions
                                .push(Action::Split(tab.focused_pane, SplitDirection::Stacked));
                            ui.close();
                        }
                        if ui.button("Close focused pane").clicked() {
                            self.actions.push(Action::ClosePane(tab.focused_pane));
                            ui.close();
                        }
                    })
                    .response
                    .on_hover_text("Terminal navigation and pane actions");
                });
            },
        );
    }

    fn pane_navigation(&mut self, ui: &mut egui::Ui, tab: &Tab) {
        ui.set_max_width(420.0);
        for (index, pane) in tab.root.panes().iter().enumerate() {
            let label = pane_label(pane, index);
            let response = ui
                .push_id(("pane-navigation", pane.id), |ui| {
                    ui.add(egui::Button::selectable(pane.id == tab.focused_pane, &label).truncate())
                        .on_hover_text(&label)
                })
                .inner;
            if response.clicked() {
                self.actions.push(Action::Focus(pane.id));
                ui.close();
            }
        }
    }

    fn close_hosts(&mut self, ids: Vec<Uuid>) {
        for id in ids {
            let mut closed = false;
            if let Some(mut host) = self.hosts.remove(&id)
                && let Some(t) = host.terminal.take()
            {
                if let Err(error) = t.close() {
                    self.errors.push(format!("Closing pane: {error:#}"));
                }
                closed = true;
            }
            if !closed
                && let Some(ws) = self
                    .state
                    .workspaces
                    .iter()
                    .find(|w| w.panes().iter().any(|p| p.id == id))
            {
                let kind = if ws.kind == WorkspaceKind::Ssh {
                    SessionKind::Ssh {
                        command: ws.ssh_command.clone(),
                        session: session_name(ws, id),
                    }
                } else if ws.local_tmux {
                    SessionKind::LocalTmux {
                        session: session_name(ws, id),
                    }
                } else {
                    SessionKind::Local
                };
                if let Err(e) = crate::terminal::close_session(&kind, &self.store.transcript_dir())
                {
                    self.errors
                        .push(format!("Could not queue session cleanup: {e:#}"));
                }
            }
        }
    }
    fn commit_frame_actions(&mut self, ctx: &egui::Context) {
        let id = Id::new("bmux-frame-actions");
        let frame = ctx.cumulative_frame_nr();
        let mut pending = ctx
            .data_mut(|data| data.remove_temp::<FrameActions>(id))
            .unwrap_or_default();
        if pending.frame != frame {
            pending = FrameActions {
                frame,
                ..Default::default()
            };
        }
        pending.merge_pass(std::mem::take(&mut self.actions));
        if !ctx.will_discard() {
            self.actions = pending.actions[pending.committed..].to_vec();
            pending.committed = pending.actions.len();
            if !self.actions.is_empty() {
                self.process_actions();
                // Structural changes happened after drawing and need one fresh frame.
                ctx.request_repaint();
            }
        }
        ctx.data_mut(|data| data.insert_temp(id, pending));
    }

    fn process_actions(&mut self) {
        for action in std::mem::take(&mut self.actions) {
            match action {
                Action::ToggleSidebar => self.state.sidebar_visible = !self.state.sidebar_visible,
                Action::Settings => self.settings_open = true,
                Action::NewWorkspace(tmux) => {
                    let cwd = self
                        .workspace()
                        .map(|w| w.working_directory.clone())
                        .unwrap_or_else(|| {
                            dirs::home_dir()
                                .unwrap_or_else(|| PathBuf::from("."))
                                .to_string_lossy()
                                .into_owned()
                        });
                    let mut ws = Workspace::local("Untitled", &cwd, tmux);
                    let palette = [
                        "#4DA3FF", "#2DD4BF", "#7ED957", "#F7C948", "#FF9F43", "#F472B6", "#A78BFA",
                    ];
                    ws.color_hex = palette[self.state.workspaces.len() % palette.len()].into();
                    self.state.add_workspace(ws);
                }
                Action::NewSsh => {
                    self.form = WorkspaceForm {
                        open: true,
                        ssh: true,
                        prefix: self.settings.ssh_default_tmux.clone(),
                        ..Default::default()
                    };
                }
                Action::NewTab => {
                    if let Some(ws) = self.workspace_mut() {
                        ws.add_tab();
                    }
                }
                Action::CloseTab(id) => {
                    let ids = self
                        .workspace()
                        .and_then(|w| w.tabs.iter().find(|t| t.id == id))
                        .map(|t| t.root.panes().iter().map(|p| p.id).collect())
                        .unwrap_or_default();
                    self.close_hosts(ids);
                    if let Some(ws) = self.workspace_mut() {
                        ws.close_tab(id);
                    }
                }
                Action::ClosePane(id) => {
                    let single = self.tab().is_some_and(|t| t.root.panes().len() == 1);
                    if single {
                        if let Some(t) = self.tab() {
                            self.actions.push(Action::CloseTab(t.id));
                        }
                    } else {
                        self.close_hosts(vec![id]);
                        if let Some(t) = self.tab_mut() {
                            t.close_pane(id);
                        }
                    }
                }
                Action::Split(id, direction) => {
                    if let Some(t) = self.tab_mut() {
                        let cwd = t
                            .root
                            .panes()
                            .iter()
                            .find(|p| p.id == id)
                            .map(|p| p.working_directory.clone())
                            .unwrap_or_default();
                        let pane = Pane::new(cwd);
                        let new_id = pane.id;
                        if t.root.split(id, direction, pane) {
                            t.focused_pane = new_id;
                        }
                    }
                }
                Action::Focus(id) => {
                    if let Some(t) = self.tab_mut() {
                        t.focused_pane = id;
                    }
                }
                Action::Reconnect(id) => {
                    self.hosts.remove(&id);
                }
                Action::History(id) => {
                    let text = self
                        .hosts
                        .get(&id)
                        .and_then(|h| h.terminal.as_ref())
                        .map(|t| t.history())
                        .unwrap_or_else(|| {
                            crate::terminal::read_history(
                                &self.store.transcript_dir().join(format!("{id}.ts")),
                            )
                        });
                    self.history = Some(HistoryView {
                        id,
                        text,
                        query: String::new(),
                    });
                }
                Action::SelectWorkspace(id) => self.state.selected = id,
                Action::SelectTab(id) => {
                    if let Some(w) = self.workspace_mut() {
                        w.active_tab = id;
                    }
                }
                Action::RenameWorkspace(id) => {
                    if let Some(w) = self.state.workspaces.iter().find(|w| w.id == id) {
                        self.rename = Some(RenameForm {
                            id,
                            tab: false,
                            value: w.name.clone(),
                        });
                    }
                }
                Action::RenameTab(id) => {
                    if let Some(t) = self
                        .workspace()
                        .and_then(|w| w.tabs.iter().find(|t| t.id == id))
                    {
                        self.rename = Some(RenameForm {
                            id,
                            tab: true,
                            value: t.title.clone(),
                        });
                    }
                }
                Action::EditWorkspace(id) => {
                    if let Some(w) = self.state.workspaces.iter().find(|w| w.id == id) {
                        self.form = WorkspaceForm {
                            open: true,
                            id: Some(id),
                            ssh: w.kind == WorkspaceKind::Ssh,
                            tmux: w.local_tmux,
                            name: w.name.clone(),
                            cwd: w.working_directory.clone(),
                            command: w.ssh_command.clone(),
                            prefix: w.ssh_tmux_session.clone(),
                            error: String::new(),
                        };
                    }
                }
                Action::DuplicateWorkspace(id) => {
                    if let Some(w) = self.state.workspaces.iter().find(|w| w.id == id) {
                        let mut duplicate = if w.kind == WorkspaceKind::Ssh {
                            Workspace::ssh(
                                format!("{} copy", w.name),
                                &w.ssh_command,
                                &w.ssh_tmux_session,
                            )
                        } else {
                            Workspace::local(
                                format!("{} copy", w.name),
                                &w.working_directory,
                                w.local_tmux,
                            )
                        };
                        duplicate.color_hex = w.color_hex.clone();
                        duplicate.icon_name = w.icon_name.clone();
                        self.state.selected = duplicate.id;
                        self.state.workspaces.push(duplicate);
                    }
                }
                Action::DeleteWorkspace(id) => {
                    let ids = self
                        .state
                        .workspaces
                        .iter()
                        .find(|w| w.id == id)
                        .map(|w| {
                            w.tabs
                                .iter()
                                .flat_map(|t| {
                                    t.root.panes().iter().map(|p| p.id).collect::<Vec<_>>()
                                })
                                .collect()
                        })
                        .unwrap_or_default();
                    self.close_hosts(ids);
                    self.state.close_workspace(id);
                }
                Action::MoveWorkspace(id, to) => {
                    if let Some(from) = self.state.workspaces.iter().position(|w| w.id == id) {
                        let w = self.state.workspaces.remove(from);
                        let to = to.min(self.state.workspaces.len());
                        self.state.workspaces.insert(to, w);
                    }
                }
                Action::Color(id, color) => {
                    if let Some(w) = self.state.workspaces.iter_mut().find(|w| w.id == id) {
                        w.color_hex = color;
                    }
                }
                Action::Clear => {
                    if let Some(id) = self.focus()
                        && let Some(t) = self.hosts.get_mut(&id).and_then(|h| h.terminal.as_mut())
                    {
                        t.clear();
                    }
                }
                Action::Scroll(top) => {
                    if let Some(id) = self.focus()
                        && let Some(t) = self.hosts.get_mut(&id).and_then(|h| h.terminal.as_mut())
                    {
                        if top {
                            t.scroll_top();
                        } else {
                            t.scroll_bottom();
                        }
                    }
                }
                Action::Zoom(by) => self.zoom = (self.zoom + by).clamp(-7.0, 30.0),
                Action::ZoomReset => self.zoom = 0.0,
            }
        }
    }

    fn window_resize_handles(&self, ctx: &egui::Context) {
        if ctx.input(|i| {
            i.viewport().maximized.unwrap_or(false) || i.viewport().fullscreen.unwrap_or(false)
        }) {
            return;
        }
        let r = ctx.content_rect();
        let corner = 12.0;
        let edge = 4.0;
        use egui::ResizeDirection as D;
        let zones = [
            (
                D::NorthWest,
                Rect::from_min_size(r.min, Vec2::splat(corner)),
                egui::CursorIcon::ResizeNwSe,
            ),
            (
                D::NorthEast,
                Rect::from_min_size(Pos2::new(r.right() - corner, r.top()), Vec2::splat(corner)),
                egui::CursorIcon::ResizeNeSw,
            ),
            (
                D::SouthWest,
                Rect::from_min_size(
                    Pos2::new(r.left(), r.bottom() - corner),
                    Vec2::splat(corner),
                ),
                egui::CursorIcon::ResizeNeSw,
            ),
            (
                D::SouthEast,
                Rect::from_min_size(r.max - Vec2::splat(corner), Vec2::splat(corner)),
                egui::CursorIcon::ResizeNwSe,
            ),
            (
                D::North,
                Rect::from_min_max(
                    r.min + Vec2::new(corner, 0.0),
                    Pos2::new(r.right() - corner, r.top() + edge),
                ),
                egui::CursorIcon::ResizeVertical,
            ),
            (
                D::South,
                Rect::from_min_max(
                    Pos2::new(r.left() + corner, r.bottom() - edge),
                    r.max - Vec2::new(corner, 0.0),
                ),
                egui::CursorIcon::ResizeVertical,
            ),
            (
                D::West,
                Rect::from_min_max(
                    r.min + Vec2::new(0.0, corner),
                    Pos2::new(r.left() + edge, r.bottom() - corner),
                ),
                egui::CursorIcon::ResizeHorizontal,
            ),
            (
                D::East,
                Rect::from_min_max(
                    Pos2::new(r.right() - edge, r.top() + corner),
                    r.max - Vec2::new(0.0, corner),
                ),
                egui::CursorIcon::ResizeHorizontal,
            ),
        ];
        for (i, (direction, rect, cursor)) in zones.into_iter().enumerate() {
            egui::Area::new(Id::new(("window-resize", i)))
                .order(egui::Order::Foreground)
                .fixed_pos(rect.min)
                .movable(false)
                .show(ctx, |ui| {
                    let (_, res) = ui.allocate_exact_size(rect.size(), Sense::drag());
                    let res = res.on_hover_cursor(cursor);
                    if res.drag_started() {
                        ctx.send_viewport_cmd(egui::ViewportCommand::BeginResize(direction));
                    }
                });
        }
    }

    fn dialogs(&mut self, ctx: &egui::Context) {
        self.settings.show_with_workspaces(
            ctx,
            &mut self.settings_open,
            &mut self.state.workspaces,
        );
        if self.form.open {
            let mut open = true;
            let mut save = false;
            let replace_sessions = self
                .form
                .id
                .and_then(|id| self.state.workspaces.iter().find(|w| w.id == id))
                .is_some_and(|w| {
                    w.local_tmux != self.form.tmux
                        || w.ssh_command != self.form.command.trim()
                        || w.ssh_tmux_session != self.form.prefix
                });
            egui::Window::new(if self.form.id.is_some(){"Edit workspace"}else{"New SSH workspace"}).id(Id::new("workspace-form")).open(&mut open).collapsible(false).resizable(false).default_width(430.0).anchor(Align2::CENTER_CENTER,Vec2::ZERO).show(ctx,|ui|{
                ui.add_space(8.0);ui.label("Name");ui.text_edit_singleline(&mut self.form.name);
                if self.form.ssh {
                    ui.add_space(10.0);ui.label("SSH command");ui.add(egui::TextEdit::singleline(&mut self.form.command).hint_text("ssh user@hostname"));
                    ui.label(RichText::new("Uses your system SSH client, keys, agent, and SSH config.").small().weak());
                    ui.add_space(10.0);ui.label("Tmux session prefix (optional)");ui.add(egui::TextEdit::singleline(&mut self.form.prefix).hint_text("bmux"));
                    ui.add_space(10.0);ui.label(RichText::new("Requires tmux 3.2+ on the remote host. Sessions survive disconnects and app restarts.").small().weak());
                }else{
                    ui.add_space(10.0);ui.label("Working directory");ui.horizontal(|ui|{
                        ui.text_edit_singleline(&mut self.form.cwd);
                        if ui.button("Browse…").clicked()&& let Some(path)=rfd::FileDialog::new().pick_folder(){self.form.cwd=path.to_string_lossy().into_owned();}
                    });
                    ui.checkbox(&mut self.form.tmux,"Persistent local tmux session");
                    if cfg!(windows){ui.label(RichText::new("Local tmux workspaces use the default WSL distribution.").small().weak());}
                }
                if replace_sessions{ui.colored_label(Color32::from_rgb(255,159,67),"Saving these connection changes closes this workspace’s existing sessions and opens a fresh terminal.");}
                if !self.form.error.is_empty(){ui.colored_label(Color32::from_rgb(255,85,70),&self.form.error);}
                ui.add_space(14.0);ui.horizontal(|ui|{if ui.button("Cancel").clicked(){self.form.open=false;}save=ui.button(if replace_sessions{"Replace sessions"}else if self.form.id.is_some(){"Save"}else{"Connect"}).clicked();});
            });
            self.form.open &= open;
            if save {
                if self.form.name.trim().is_empty() {
                    self.form.error = "Give this workspace a name.".into();
                } else if self.form.ssh
                    && (crate::terminal::validate_ssh(self.form.command.trim()).is_err())
                {
                    self.form.error = "Enter an SSH command such as ssh user@hostname.".into();
                } else if self.form.ssh
                    && !self
                        .form
                        .prefix
                        .chars()
                        .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
                {
                    self.form.error =
                        "Session prefixes may contain letters, numbers, underscore and hyphen."
                            .into();
                } else if !self.form.ssh && !PathBuf::from(&self.form.cwd).is_dir() {
                    self.form.error = "Choose an existing local directory.".into();
                } else {
                    if let Some(id) = self.form.id {
                        if replace_sessions {
                            let ids = self
                                .state
                                .workspaces
                                .iter()
                                .find(|w| w.id == id)
                                .map(|w| w.panes().iter().map(|p| p.id).collect())
                                .unwrap_or_default();
                            self.close_hosts(ids);
                        }
                        if let Some(w) = self.state.workspaces.iter_mut().find(|w| w.id == id) {
                            w.name = self.form.name.trim().into();
                            w.working_directory = self.form.cwd.clone();
                            w.ssh_command = self.form.command.trim().into();
                            w.ssh_tmux_session = self.form.prefix.clone();
                            w.local_tmux = self.form.tmux;
                            if replace_sessions {
                                let t = Tab::new(&w.working_directory);
                                w.active_tab = t.id;
                                w.tabs = vec![t];
                            }
                        }
                    } else {
                        let ws = Workspace::ssh(
                            self.form.name.trim(),
                            self.form.command.trim(),
                            self.form.prefix.trim(),
                        );
                        self.state.selected = ws.id;
                        self.state.workspaces.push(ws);
                    }
                    self.form.open = false;
                }
            }
        }
        if let Some(rename) = &mut self.rename {
            let mut open = true;
            let mut save = false;
            egui::Window::new(if rename.tab {
                "Rename terminal"
            } else {
                "Rename workspace"
            })
            .collapsible(false)
            .resizable(false)
            .open(&mut open)
            .anchor(Align2::CENTER_CENTER, Vec2::ZERO)
            .show(ctx, |ui| {
                let input = ui.text_edit_singleline(&mut rename.value);
                if input.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter)) {
                    save = true;
                }
                ui.add_space(8.0);
                save |= ui
                    .add_enabled(!rename.value.trim().is_empty(), egui::Button::new("Save"))
                    .clicked();
            });
            if save && !rename.value.trim().is_empty() {
                let id = rename.id;
                let value = rename.value.trim().to_owned();
                let tab = rename.tab;
                if tab {
                    if let Some(t) = self
                        .workspace_mut()
                        .and_then(|w| w.tabs.iter_mut().find(|t| t.id == id))
                    {
                        t.title = value;
                    }
                } else if let Some(w) = self.state.workspaces.iter_mut().find(|w| w.id == id) {
                    w.name = value;
                }
                self.rename = None;
            } else if !open {
                self.rename = None;
            }
        }
        if let Some(history) = &mut self.history {
            let mut open = true;
            let mut replay = false;
            egui::Window::new(
                self.hosts
                    .get(&history.id)
                    .and_then(|h| h.terminal.as_ref())
                    .map(|t| format!("Pane history — {}", t.title()))
                    .unwrap_or_else(|| "Pane history".into()),
            )
            .id(Id::new("history"))
            .open(&mut open)
            .default_size([800.0, 520.0])
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.add(
                        egui::TextEdit::singleline(&mut history.query)
                            .hint_text("Filter history…")
                            .desired_width(260.0),
                    );
                    if ui.button("Refresh").clicked()
                        && let Some(t) = self
                            .hosts
                            .get(&history.id)
                            .and_then(|h| h.terminal.as_ref())
                    {
                        history.text = t.history();
                    }
                    if ui.button("Copy all").clicked() {
                        ctx.copy_text(history.text.clone());
                    }
                    if ui.button("Export…").clicked()
                        && let Some(path) = rfd::FileDialog::new()
                            .set_file_name("bmux-history.txt")
                            .save_file()
                        && let Err(e) = std::fs::write(path, &history.text)
                    {
                        self.errors.push(format!("Could not export history: {e}"));
                    }
                });
                ui.separator();
                let query = history.query.to_lowercase();
                let text = if query.is_empty() {
                    history.text.clone()
                } else {
                    history
                        .text
                        .lines()
                        .filter(|line| line.to_lowercase().contains(&query))
                        .collect::<Vec<_>>()
                        .join("\n")
                };
                egui::ScrollArea::both()
                    .max_height((ui.available_height() - 40.0).max(120.0))
                    .stick_to_bottom(true)
                    .show(ui, |ui| {
                        ui.add(
                            egui::Label::new(RichText::new(text).monospace().size(12.0))
                                .selectable(true)
                                .wrap_mode(egui::TextWrapMode::Extend),
                        );
                    });
                ui.separator();
                ui.horizontal(|ui| {
                    ui.label(format!("{} lines", history.text.lines().count()));
                    replay = ui
                        .button("Insert replay command")
                        .on_hover_text("Insert a command into the terminal without executing it")
                        .clicked();
                });
            });
            if replay {
                let id = history.id;
                let path = self
                    .hosts
                    .get(&id)
                    .and_then(|h| h.terminal.as_ref())
                    .map(|t| t.transcript_path().to_path_buf())
                    .unwrap_or_else(|| self.store.transcript_dir().join(format!("{id}.ts")));
                let workspace = self
                    .state
                    .workspaces
                    .iter()
                    .find(|w| w.panes().iter().any(|p| p.id == id));
                let command = if let Some(ws) = workspace.filter(|w| w.uses_tmux()) {
                    format!(
                        "tmux -L {} capture-pane -p -e -S - -t {}",
                        if ws.kind == WorkspaceKind::Ssh {
                            "bmux-v1"
                        } else {
                            "bmux-local-v1"
                        },
                        shell_words::quote(&format!("={}:", session_name(ws, id)))
                    )
                } else if cfg!(windows) {
                    let shell = if self.settings.shell_path.is_empty() {
                        crate::terminal::default_shell()
                    } else {
                        self.settings.shell_path.clone()
                    };
                    if shell.to_lowercase().ends_with("cmd.exe") {
                        format!("type \"{}\"", path.to_string_lossy())
                    } else {
                        format!(
                            "[Console]::Write([IO.File]::ReadAllText('{}'))",
                            path.to_string_lossy().replace('\'', "''")
                        )
                    }
                } else {
                    format!("cat -- {}", shell_words::quote(&path.to_string_lossy()))
                };
                if let Some(t) = self.hosts.get_mut(&id).and_then(|h| h.terminal.as_mut()) {
                    let _ = t.paste(&command);
                }
                open = false;
            }
            if !open {
                self.history = None;
            }
        }
        if !self.errors.is_empty() {
            let mut open = true;
            egui::Window::new("Bmux notice")
                .id(Id::new("error-notice"))
                .open(&mut open)
                .collapsible(false)
                .default_width(460.0)
                .show(ctx, |ui| {
                    for error in &self.errors {
                        ui.label(error);
                        ui.add_space(8.0);
                    }
                });
            if !open {
                self.errors.clear();
            }
        }
    }
}

impl eframe::App for BmuxApp {
    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        [0.0, 0.0, 0.0, 0.0]
    }
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        let dark = match format!("{:?}", self.settings.appearance_mode)
            .to_lowercase()
            .as_str()
        {
            "dark" => true,
            "light" => false,
            _ => ctx.system_theme().is_none_or(|t| t == egui::Theme::Dark),
        };
        let mut colors = self.settings.colors(dark);
        colors.background = colors
            .background
            .gamma_multiply(self.settings.background_opacity);
        let background_dark = crate::glass::is_dark(colors.background);
        let mut visuals = if background_dark {
            egui::Visuals::dark()
        } else {
            egui::Visuals::light()
        };
        visuals.panel_fill = colors.background;
        visuals.override_text_color = Some(colors.foreground);
        visuals.selection.bg_fill = Color32::from_rgb(255, 135, 0).gamma_multiply(0.35);
        crate::glass::style_visuals(&mut visuals, &colors);
        ctx.set_visuals(visuals);
        ctx.style_mut(|s| {
            s.spacing.button_padding = Vec2::new(8.0, 5.0);
            s.spacing.item_spacing = Vec2::new(5.0, 6.0);
            s.spacing.menu_margin = egui::Margin::same(8);
        });
        if self.font_path != self.settings.font_family && !self.settings_open {
            self.load_fonts(ctx);
        }
        // Consume shortcuts before terminals see them, but defer mutations until
        // all widgets have had the opportunity to request another sizing pass.
        self.keyboard(ctx);
        self.ensure_hosts(ctx);
        self.toolbar(ctx, &colors);
        self.sidebar(ctx, &colors);
        self.terminal_area(ctx, &colors);
        // Paste files dropped from the OS into the focused pane, relative
        // (`./…`) when under its directory, shell-quoted when needed.
        let dropped: Vec<PathBuf> = ctx.input(|i| {
            i.raw
                .dropped_files
                .iter()
                .filter_map(|file| file.path.clone())
                .collect()
        });
        if !dropped.is_empty()
            && let Some(id) = self.focus()
            && let Some(host) = self.hosts.get_mut(&id)
            && let Some(term) = host.terminal.as_mut()
        {
            let cwd = term.cwd();
            let text = dropped
                .iter()
                .map(|path| shell_quote(path, &cwd))
                .collect::<Vec<_>>()
                .join(" ");
            if let Err(e) = term.paste(&text) {
                self.errors
                    .push(format!("Could not paste dropped files: {e}"));
            }
        }
        self.window_resize_handles(ctx);
        self.dialogs(ctx);
        self.commit_frame_actions(ctx);
        if !ctx.will_discard() {
            self.save();
        }
        let title = self
            .workspace()
            .map(|w| format!("{} — [bmux]", w.name))
            .unwrap_or_else(|| "[bmux]".into());
        ctx.send_viewport_cmd(egui::ViewportCommand::Title(title));
        if ctx.input(|i| i.viewport().close_requested()) {
            self.last_save = Instant::now() - Duration::from_secs(1);
            self.save();
            if self.settings.remember_frame {
                ctx.input(|i| {
                    if let Some(rect) = i.viewport().inner_rect {
                        let position = i.viewport().outer_rect.map(|r| [r.min.x, r.min.y]);
                        let frame = WindowFrame {
                            size: [rect.width(), rect.height()],
                            position,
                        };
                        if let Ok(json) = serde_json::to_string_pretty(&frame) {
                            let _ = std::fs::write(self.store.data_dir().join("window.json"), json);
                        }
                    }
                });
            }
        }
        ctx.request_repaint_after(Duration::from_millis(100));
    }
    fn on_exit(&mut self, gl: Option<&eframe::glow::Context>) {
        self.last_save = Instant::now() - Duration::from_secs(1);
        self.save();
        if let (Some(mask), Some(gl)) = (self.corner_mask.take(), gl) {
            mask.destroy(gl);
        }
    }
}

/// Partition the full split rectangle without a hole, even at fractional DPI.
/// Only the interior boundaries snap; outer edges stay shared with their parent.
fn split_rects(rect: Rect, horizontal: bool, ratio: f32, scale: f32) -> (Rect, Rect, Rect, Rect) {
    let scale = if scale.is_finite() && scale > 0.0 {
        scale
    } else {
        1.0
    };
    let (start, end) = if horizontal {
        (rect.left(), rect.right())
    } else {
        (rect.top(), rect.bottom())
    };
    let aligned_start = (start * scale).ceil() / scale;
    let aligned_end = (end * scale).floor() / scale;
    let (first_end, second_start) = if aligned_end <= aligned_start {
        // Less than one physical pixel remains. Paint it all and give both
        // children empty bounds instead of producing inverted rectangles.
        (start, end)
    } else {
        let thickness = (scale.round().max(1.0) / scale).min(aligned_end - aligned_start);
        let first_end = ((start + (end - start - thickness) * ratio) * scale).round() / scale;
        let first_end =
            first_end.clamp(aligned_start, (aligned_end - thickness).max(aligned_start));
        (first_end, (first_end + thickness).min(aligned_end))
    };
    let (a, b, separator, hit) = if horizontal {
        let separator = Rect::from_min_max(
            Pos2::new(first_end, rect.top()),
            Pos2::new(second_start, rect.bottom()),
        );
        (
            Rect::from_min_max(rect.min, Pos2::new(first_end, rect.bottom())),
            Rect::from_min_max(Pos2::new(second_start, rect.top()), rect.max),
            separator,
            Rect::from_center_size(separator.center(), Vec2::new(7.0, rect.height())),
        )
    } else {
        let separator = Rect::from_min_max(
            Pos2::new(rect.left(), first_end),
            Pos2::new(rect.right(), second_start),
        );
        (
            Rect::from_min_max(rect.min, Pos2::new(rect.right(), first_end)),
            Rect::from_min_max(Pos2::new(rect.left(), second_start), rect.max),
            separator,
            Rect::from_center_size(separator.center(), Vec2::new(rect.width(), 7.0)),
        )
    };
    (a, b, separator, hit.intersect(rect))
}

fn split_separator_color(colors: &ThemeColors, highlighted: bool) -> Color32 {
    let background = colors.background.to_srgba_unmultiplied();
    let foreground = colors.foreground.to_srgba_unmultiplied();
    let amount = if highlighted { 0.32 } else { 0.12 };
    let channel = |index| {
        egui::lerp(background[index] as f32..=foreground[index] as f32, amount).round() as u8
    };
    Color32::from_rgb(channel(0), channel(1), channel(2))
}

fn tab_label(ws: &Workspace, t: &Tab, index: usize) -> String {
    if !t.title.is_empty() && t.title != "Terminal" {
        return t.title.clone();
    }
    if let Some(p) = t.active_pane().filter(|p| !p.running_command.is_empty()) {
        return format!("[{}]", p.running_command);
    }
    format!(
        "{} {}",
        if ws.kind == WorkspaceKind::Ssh {
            "SSH"
        } else {
            ">_"
        },
        index + 1
    )
}
fn tab_width(label: &str) -> f32 {
    (label.chars().count() as f32 * 7.0 + 45.0).clamp(66.0, 220.0)
}
fn navigator_tab_label(ws: &Workspace, tab: &Tab, index: usize) -> String {
    let label = tab_label(ws, tab, index);
    let count = tab.root.panes().len();
    if count > 1 {
        format!("{label} · {count} panes")
    } else {
        label
    }
}
fn pane_label(pane: &Pane, index: usize) -> String {
    let detail = if !pane.running_command.is_empty() {
        &pane.running_command
    } else if !pane.title.is_empty() {
        &pane.title
    } else {
        &pane.working_directory
    };
    if detail.is_empty() {
        format!("Pane {}", index + 1)
    } else {
        format!("Pane {} · {detail}", index + 1)
    }
}
fn icon_button(ui: &mut egui::Ui, text: &str, help: &str) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(Vec2::new(28.0, 28.0), Sense::click());
    let color = ui
        .visuals()
        .text_color()
        .gamma_multiply(if response.hovered() { 1.0 } else { 0.65 });
    if response.hovered() {
        ui.painter()
            .rect_filled(rect, 7, ui.visuals().text_color().gamma_multiply(0.09));
    }
    let stroke = Stroke::new(1.25_f32, color);
    let center = rect.center();
    let r = Rect::from_center_size(center, Vec2::new(14.0, 12.0));
    match text {
        "◧" | "◫" | "⊟" | "□" => {
            ui.painter()
                .rect_stroke(r, 2, stroke, egui::StrokeKind::Inside);
            if text == "◧" || text == "◫" {
                let x = if text == "◧" {
                    r.left() + 4.5
                } else {
                    center.x
                };
                ui.painter()
                    .line_segment([Pos2::new(x, r.top()), Pos2::new(x, r.bottom())], stroke);
            }
            if text == "⊟" {
                ui.painter().line_segment(
                    [
                        Pos2::new(r.left(), center.y),
                        Pos2::new(r.right(), center.y),
                    ],
                    stroke,
                );
            }
        }
        "＋" => {
            ui.painter().line_segment(
                [center - Vec2::new(5.0, 0.0), center + Vec2::new(5.0, 0.0)],
                stroke,
            );
            ui.painter().line_segment(
                [center - Vec2::new(0.0, 5.0), center + Vec2::new(0.0, 5.0)],
                stroke,
            );
        }
        "×" => {
            ui.painter().line_segment(
                [center - Vec2::splat(4.0), center + Vec2::splat(4.0)],
                stroke,
            );
            ui.painter().line_segment(
                [center + Vec2::new(-4.0, 4.0), center + Vec2::new(4.0, -4.0)],
                stroke,
            );
        }
        "−" => {
            ui.painter().line_segment(
                [center - Vec2::new(5.0, 0.0), center + Vec2::new(5.0, 0.0)],
                stroke,
            );
        }
        _ => {
            ui.painter().text(
                center,
                Align2::CENTER_CENTER,
                text,
                FontId::proportional(17.0),
                color,
            );
        }
    }
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, help));
    response.on_hover_text(help)
}
fn menu_item(ui: &mut egui::Ui, label: &str, shortcut: &str) -> bool {
    let clicked = ui
        .add(egui::Button::new(label).shortcut_text(shortcut))
        .clicked();
    if clicked {
        ui.close();
    }
    clicked
}
fn short_path(path: &str) -> String {
    if dirs::home_dir().is_some_and(|home| home == std::path::Path::new(path)) {
        return "~".into();
    }
    PathBuf::from(path)
        .file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.into())
}
fn cycle_index(index: usize, by: isize, len: usize) -> Option<usize> {
    if len == 0 {
        None
    } else {
        Some((index as isize + by).rem_euclid(len as isize) as usize)
    }
}
fn number_key(key: Key) -> Option<usize> {
    Some(match key {
        Key::Num1 => 1,
        Key::Num2 => 2,
        Key::Num3 => 3,
        Key::Num4 => 4,
        Key::Num5 => 5,
        Key::Num6 => 6,
        Key::Num7 => 7,
        Key::Num8 => 8,
        Key::Num9 => 9,
        _ => return None,
    })
}
fn parse_hex(value: &str) -> Option<Color32> {
    let s = value.strip_prefix('#').unwrap_or(value);
    if s.len() != 6 {
        return None;
    }
    let n = u32::from_str_radix(s, 16).ok()?;
    Some(Color32::from_rgb((n >> 16) as u8, (n >> 8) as u8, n as u8))
}
fn session_name(workspace: &Workspace, pane: Uuid) -> String {
    let prefix = if workspace.ssh_tmux_session.is_empty() {
        "bmux"
    } else {
        &workspace.ssh_tmux_session
    };
    if workspace.kind == WorkspaceKind::Local {
        format!("bmux-local-{pane}")
    } else {
        format!("{prefix}-{pane}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn navigation_wraps_without_dividing_by_zero() {
        assert_eq!(cycle_index(0, -1, 3), Some(2));
        assert_eq!(cycle_index(2, 1, 3), Some(0));
        assert_eq!(cycle_index(0, 1, 0), None);
    }
    #[test]
    fn persistent_session_identity_is_unique_and_stable() {
        let w = Workspace::ssh("Server", "ssh host", "project");
        let p = Uuid::new_v4();
        assert_eq!(session_name(&w, p), session_name(&w, p));
        assert_ne!(session_name(&w, p), session_name(&w, Uuid::new_v4()));
    }
}

#[cfg(test)]
#[path = "app_tests.rs"]
mod interaction_tests;

#[cfg(test)]
#[path = "action_pass_tests.rs"]
mod action_pass_tests;

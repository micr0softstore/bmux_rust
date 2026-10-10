//! Persistent preferences and the settings window shared by Linux and Windows.
use eframe::egui::{self, Color32, RichText};
use serde::{Deserialize, Serialize};
use std::sync::OnceLock;

use crate::model::Workspace;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CursorStyle {
    #[default]
    Block,
    Bar,
    Underline,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TabCloseMode {
    #[default]
    Hover,
    Always,
    Never,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AppearanceMode {
    #[default]
    System,
    Dark,
    Light,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub version: u32,
    pub appearance_mode: AppearanceMode,
    #[serde(alias = "darkThemeName")]
    pub dark_theme_name: String,
    #[serde(alias = "lightThemeName")]
    pub light_theme_name: String,
    #[serde(alias = "backgroundOpacity")]
    pub background_opacity: f32,
    #[serde(alias = "backgroundBlur")]
    pub background_blur: u32,
    #[serde(alias = "enforceContrast")]
    pub enforce_contrast: bool,
    #[serde(alias = "minimumContrast")]
    pub minimum_contrast: f32,
    #[serde(alias = "customBackgroundHex")]
    pub custom_background_hex: Option<String>,
    #[serde(alias = "customForegroundHex")]
    pub custom_foreground_hex: Option<String>,
    #[serde(alias = "customCursorHex")]
    pub custom_cursor_hex: Option<String>,
    #[serde(alias = "customSelectionHex")]
    pub custom_selection_hex: Option<String>,
    #[serde(alias = "fontFamily")]
    pub font_family: String,
    #[serde(alias = "fontSize")]
    pub font_size: Option<f32>,
    #[serde(alias = "fontThicken")]
    pub font_thicken: bool,
    #[serde(alias = "fontThickenStrength")]
    pub font_thicken_strength: Option<u8>,
    #[serde(alias = "cursorStyle")]
    pub cursor_style: CursorStyle,
    #[serde(alias = "cursorBlink")]
    pub cursor_blink: bool,
    #[serde(alias = "cursorOpacity")]
    pub cursor_opacity: f32,
    #[serde(alias = "scrollbackMB")]
    pub scrollback_mb: Option<u32>,
    #[serde(alias = "shellPath")]
    pub shell_path: String,
    #[serde(alias = "autoRelaunch")]
    pub auto_relaunch: bool,
    #[serde(alias = "relaunchAfterSeconds")]
    pub relaunch_after_seconds: f64,
    #[serde(alias = "transcriptMaxMB")]
    pub transcript_max_mb: u32,
    #[serde(alias = "restoreTailKB")]
    pub restore_tail_kb: u32,
    #[serde(alias = "extraConfig")]
    pub extra_config: String,
    #[serde(alias = "defaultWidth")]
    pub default_width: f32,
    #[serde(alias = "defaultHeight")]
    pub default_height: f32,
    #[serde(alias = "rememberFrame")]
    pub remember_frame: bool,
    #[serde(alias = "paddingX")]
    pub padding_x: u32,
    #[serde(alias = "paddingY")]
    pub padding_y: u32,
    #[serde(alias = "unfocusedDim")]
    pub unfocused_dim: f32,
    #[serde(alias = "resizeThrottleMs")]
    pub resize_throttle_ms: Option<f32>,
    #[serde(alias = "showSingleTab")]
    pub show_single_tab: bool,
    #[serde(alias = "tabCloseMode")]
    pub tab_close_mode: TabCloseMode,
    #[serde(alias = "sshSelfClear")]
    pub ssh_self_clear: bool,
    #[serde(alias = "sshDefaultTmux")]
    pub ssh_default_tmux: String,
    /// Last window dimensions are separate from the requested default size.
    pub remembered_width: Option<f32>,
    pub remembered_height: Option<f32>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            version: 5,
            appearance_mode: AppearanceMode::System,
            dark_theme_name: "Srcery".into(),
            light_theme_name: "Alabaster".into(),
            background_opacity: 1.0,
            background_blur: 0,
            enforce_contrast: false,
            minimum_contrast: 4.5,
            custom_background_hex: None,
            custom_foreground_hex: None,
            custom_cursor_hex: None,
            custom_selection_hex: None,
            font_family: String::new(),
            font_size: None,
            font_thicken: false,
            font_thicken_strength: None,
            cursor_style: CursorStyle::Block,
            cursor_blink: true,
            cursor_opacity: 1.0,
            scrollback_mb: None,
            shell_path: String::new(),
            auto_relaunch: true,
            relaunch_after_seconds: 5.0,
            transcript_max_mb: 8,
            restore_tail_kb: 128,
            extra_config: String::new(),
            default_width: 1280.0,
            default_height: 800.0,
            remember_frame: false,
            padding_x: 6,
            padding_y: 6,
            unfocused_dim: 0.28,
            resize_throttle_ms: None,
            show_single_tab: true,
            tab_close_mode: TabCloseMode::Hover,
            ssh_self_clear: true,
            ssh_default_tmux: String::new(),
            remembered_width: None,
            remembered_height: None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ThemeColors {
    pub background: Color32,
    pub foreground: Color32,
    pub cursor: Color32,
    pub selection: Color32,
    pub palette: [Color32; 16],
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Theme {
    pub name: String,
    pub background: String,
    pub foreground: String,
    pub cursor_color: String,
    pub selection_background: String,
    pub palette: [String; 16],
}

impl Theme {
    pub fn is_dark(&self) -> bool {
        luminance(parse_hex(&self.background).unwrap_or(Color32::BLACK)) < 0.45
    }
    pub fn colors(&self) -> ThemeColors {
        let background = parse_hex(&self.background).unwrap_or(Color32::BLACK);
        let foreground = parse_hex(&self.foreground).unwrap_or(Color32::WHITE);
        ThemeColors {
            background,
            foreground,
            cursor: parse_hex(&self.cursor_color).unwrap_or(foreground),
            selection: parse_hex(&self.selection_background).unwrap_or(Color32::DARK_GRAY),
            palette: std::array::from_fn(|index| {
                parse_hex(&self.palette[index]).unwrap_or(foreground)
            }),
        }
    }
}

pub fn themes() -> &'static [Theme] {
    static CATALOG: OnceLock<Vec<Theme>> = OnceLock::new();
    CATALOG.get_or_init(|| {
        serde_json::from_str(include_str!("../assets/themes.json"))
            .expect("Bundled theme catalog must be valid")
    })
}

pub fn parse_hex(hex: &str) -> Option<Color32> {
    let hex = hex.trim().trim_start_matches('#');
    if hex.len() != 6 || !hex.is_ascii() {
        return None;
    }
    Some(Color32::from_rgb(
        u8::from_str_radix(&hex[0..2], 16).ok()?,
        u8::from_str_radix(&hex[2..4], 16).ok()?,
        u8::from_str_radix(&hex[4..6], 16).ok()?,
    ))
}

pub fn to_hex(color: Color32) -> String {
    format!("#{:02X}{:02X}{:02X}", color.r(), color.g(), color.b())
}

/// Validate with the same parser that egui uses before installing a user font.
/// This turns invalid/unsupported files into a recoverable settings error.
pub fn load_font_file(path: &std::path::Path) -> anyhow::Result<Vec<u8>> {
    use anyhow::Context;
    let metadata =
        std::fs::metadata(path).with_context(|| format!("Read font file {}", path.display()))?;
    anyhow::ensure!(metadata.is_file(), "Font path is not a file");
    anyhow::ensure!(
        metadata.len() <= 64 * 1024 * 1024,
        "Font exceeds the 64 MB limit"
    );
    let bytes = std::fs::read(path)?;
    ab_glyph::FontRef::try_from_slice(&bytes).map_err(|_| {
        anyhow::anyhow!("Not a supported TrueType/OpenType font: {}", path.display())
    })?;
    Ok(bytes)
}

fn installed_monospace_fonts() -> &'static [(String, String)] {
    static FONTS: OnceLock<Vec<(String, String)>> = OnceLock::new();
    FONTS.get_or_init(|| {
        let mut fonts: Vec<(String, String)> = Vec::new();
        #[cfg(target_os = "linux")]
        if let Ok(output) = std::process::Command::new("fc-list")
            .args(["--format=%{family}\t%{file}\n", ":spacing=100"])
            .output()
            && output.status.success()
        {
            for line in String::from_utf8_lossy(&output.stdout).lines() {
                if let Some((family, file)) = line.split_once('\t') {
                    let name = family.split(',').next().unwrap_or(family).to_owned();
                    fonts.push((name, file.to_owned()));
                }
            }
        }
        #[cfg(target_os = "windows")]
        {
            let windows = std::env::var_os("WINDIR")
                .map(std::path::PathBuf::from)
                .unwrap_or_else(|| "C:\\Windows".into());
            for (family, file) in [
                ("Consolas", "consola.ttf"),
                ("Courier New", "cour.ttf"),
                ("Lucida Console", "lucon.ttf"),
                ("Cascadia Mono", "CascadiaMono.ttf"),
                ("Cascadia Code", "CascadiaCode.ttf"),
            ] {
                let path = windows.join("Fonts").join(file);
                if path.is_file() {
                    fonts.push((family.into(), path.to_string_lossy().into_owned()));
                }
            }
        }
        fonts.sort_by(|a, b| {
            a.0.to_lowercase()
                .cmp(&b.0.to_lowercase())
                .then(a.1.cmp(&b.1))
        });
        fonts.dedup_by(|a, b| a.0 == b.0);
        fonts
    })
}

fn luminance(color: Color32) -> f32 {
    fn linear(component: u8) -> f32 {
        let value = component as f32 / 255.0;
        if value <= 0.04045 {
            value / 12.92
        } else {
            ((value + 0.055) / 1.055).powf(2.4)
        }
    }
    0.2126 * linear(color.r()) + 0.7152 * linear(color.g()) + 0.0722 * linear(color.b())
}

fn contrast(a: Color32, b: Color32) -> f32 {
    let (a, b) = (luminance(a), luminance(b));
    (a.max(b) + 0.05) / (a.min(b) + 0.05)
}

fn ensure_contrast(foreground: Color32, background: Color32, minimum: f32) -> Color32 {
    if contrast(foreground, background) >= minimum {
        return foreground;
    }
    let target = if contrast(Color32::WHITE, background) > contrast(Color32::BLACK, background) {
        Color32::WHITE
    } else {
        Color32::BLACK
    };
    let blend = |amount: f32| {
        Color32::from_rgb(
            (foreground.r() as f32 + (target.r() as f32 - foreground.r() as f32) * amount).round()
                as u8,
            (foreground.g() as f32 + (target.g() as f32 - foreground.g() as f32) * amount).round()
                as u8,
            (foreground.b() as f32 + (target.b() as f32 - foreground.b() as f32) * amount).round()
                as u8,
        )
    };
    let (mut lower, mut upper) = (0.0, 1.0);
    for _ in 0..12 {
        let midpoint = (lower + upper) / 2.0;
        if contrast(blend(midpoint), background) >= minimum {
            upper = midpoint;
        } else {
            lower = midpoint;
        }
    }
    blend(upper)
}

impl Settings {
    pub fn sanitize(&mut self) {
        fn clamp(value: f32, range: std::ops::RangeInclusive<f32>, fallback: f32) -> f32 {
            if value.is_finite() {
                value.clamp(*range.start(), *range.end())
            } else {
                fallback
            }
        }
        self.version = 5;
        self.background_opacity = clamp(self.background_opacity, 0.3..=1.0, 1.0);
        self.background_blur = self.background_blur.min(60);
        self.minimum_contrast = clamp(self.minimum_contrast, 1.0..=21.0, 4.5);
        self.font_size = self.font_size.map(|v| clamp(v, 8.0..=48.0, 13.0));
        self.cursor_opacity = clamp(self.cursor_opacity, 0.0..=1.0, 1.0);
        self.scrollback_mb = self.scrollback_mb.map(|v| v.clamp(1, 512));
        self.relaunch_after_seconds = if self.relaunch_after_seconds.is_finite() {
            self.relaunch_after_seconds.clamp(0.0, 120.0)
        } else {
            5.0
        };
        self.transcript_max_mb = self.transcript_max_mb.clamp(1, 128);
        self.restore_tail_kb = self.restore_tail_kb.clamp(16, 2048);
        self.default_width = clamp(self.default_width, 640.0..=4096.0, 1280.0);
        self.default_height = clamp(self.default_height, 400.0..=2160.0, 800.0);
        self.padding_x = self.padding_x.min(32);
        self.padding_y = self.padding_y.min(32);
        self.unfocused_dim = clamp(self.unfocused_dim, 0.0..=0.6, 0.28);
        self.resize_throttle_ms = self.resize_throttle_ms.map(|v| clamp(v, 0.0..=200.0, 0.0));
        self.remembered_width = self
            .remembered_width
            .map(|v| clamp(v, 640.0..=8192.0, 1280.0));
        self.remembered_height = self
            .remembered_height
            .map(|v| clamp(v, 400.0..=8192.0, 800.0));
    }

    pub fn is_dark(&self, system_dark: bool) -> bool {
        match self.appearance_mode {
            AppearanceMode::System => system_dark,
            AppearanceMode::Dark => true,
            AppearanceMode::Light => false,
        }
    }

    pub fn colors(&self, dark: bool) -> ThemeColors {
        let name = if dark {
            &self.dark_theme_name
        } else {
            &self.light_theme_name
        };
        let fallback = if dark { "Srcery" } else { "Alabaster" };
        let theme = themes()
            .iter()
            .find(|t| t.name == *name)
            .or_else(|| themes().iter().find(|t| t.name == fallback))
            .expect("Bundled default themes must exist");
        let mut colors = theme.colors();
        for (value, target) in [
            (&self.custom_background_hex, &mut colors.background),
            (&self.custom_foreground_hex, &mut colors.foreground),
            (&self.custom_cursor_hex, &mut colors.cursor),
            (&self.custom_selection_hex, &mut colors.selection),
        ] {
            if let Some(color) = value.as_deref().and_then(parse_hex) {
                *target = color;
            }
        }
        if self.enforce_contrast {
            colors.foreground =
                ensure_contrast(colors.foreground, colors.background, self.minimum_contrast);
            for color in &mut colors.palette {
                *color = ensure_contrast(*color, colors.background, self.minimum_contrast);
            }
        }
        colors
    }

    pub fn effective_font_size(&self) -> f32 {
        self.font_size.unwrap_or(13.0)
    }
    pub fn effective_resize_throttle_ms(&self) -> f32 {
        self.resize_throttle_ms.unwrap_or_default().max(0.0)
    }
    pub fn initial_size(&self) -> [f32; 2] {
        if self.remember_frame {
            [
                self.remembered_width.unwrap_or(self.default_width),
                self.remembered_height.unwrap_or(self.default_height),
            ]
        } else {
            [self.default_width, self.default_height]
        }
    }

    #[allow(dead_code)] // Standalone embedding without the workspace color editor.
    pub fn show(&mut self, ctx: &egui::Context, open: &mut bool) -> bool {
        self.show_with_workspaces(ctx, open, &mut [])
    }

    /// The return value includes workspace color changes so the host can persist
    /// both preferences and workspace state immediately.
    pub fn show_with_workspaces(
        &mut self,
        ctx: &egui::Context,
        open: &mut bool,
        workspaces: &mut [Workspace],
    ) -> bool {
        let before = self.clone();
        let mut workspace_changed = false;
        egui::Window::new("Settings")
            .id(egui::Id::new("bmux_settings"))
            .open(open)
            .default_size([790.0, 600.0])
            .min_size([660.0, 520.0])
            .collapsible(false)
            .show(ctx, |ui| {
                let selection_id = egui::Id::new("settings_section");
                let mut selection = ctx
                    .data(|data| data.get_temp::<usize>(selection_id))
                    .unwrap_or(0);
                ui.horizontal_top(|ui| {
                    ui.vertical(|ui| {
                        ui.set_width(160.0);
                        ui.add_space(9.0);
                        for (index, title) in [
                            "Appearance",
                            "Font",
                            "Cursor",
                            "Terminal",
                            "Window",
                            "SSH",
                            "Workspaces",
                            "Advanced",
                        ]
                        .iter()
                        .enumerate()
                        {
                            if ui
                                .add_sized(
                                    [154.0, 33.0],
                                    egui::Button::new(*title).selected(selection == index),
                                )
                                .clicked()
                            {
                                selection = index;
                            }
                        }
                    });
                    ui.separator();
                    let detail_width = ui.available_width().max(400.0);
                    ui.vertical(|ui| {
                        ui.set_width(detail_width);
                        egui::ScrollArea::vertical()
                            .id_salt("settings_body")
                            .auto_shrink([false, false])
                            .show(ui, |ui| {
                                ui.set_width(detail_width - 16.0);
                                ui.add_space(8.0);
                                match selection {
                                    0 => self.appearance_ui(ui),
                                    1 => self.font_ui(ui),
                                    2 => self.cursor_ui(ui),
                                    3 => self.terminal_ui(ui),
                                    4 => self.window_ui(ui),
                                    5 => self.ssh_ui(ui),
                                    6 => workspace_changed = workspaces_ui(ui, workspaces),
                                    _ => self.advanced_ui(ui),
                                }
                            });
                    });
                });
                ctx.data_mut(|data| data.insert_temp(selection_id, selection));
            });
        *self != before || workspace_changed
    }

    fn appearance_ui(&mut self, ui: &mut egui::Ui) {
        ui.heading("Appearance");
        ui.add_space(12.0);
        group(ui, "Color scheme", |ui| {
            ui.horizontal(|ui| {
                ui.selectable_value(&mut self.appearance_mode, AppearanceMode::System, "System");
                ui.selectable_value(&mut self.appearance_mode, AppearanceMode::Dark, "Dark");
                ui.selectable_value(&mut self.appearance_mode, AppearanceMode::Light, "Light");
            });
        });
        group(ui, "Dark theme", |ui| {
            theme_picker(ui, &mut self.dark_theme_name, "dark_theme")
        });
        group(ui, "Light theme", |ui| {
            theme_picker(ui, &mut self.light_theme_name, "light_theme")
        });
        group(ui, "Transparency", |ui| {
            ui.add(
                egui::Slider::new(&mut self.background_opacity, 0.3..=1.0)
                    .text("Background opacity"),
            );
            ui.add_enabled(
                false,
                egui::Slider::new(&mut self.background_blur, 0..=60).text("Background blur"),
            );
            note(
                ui,
                "Background blur is unavailable in this renderer. Window transparency depends on your desktop compositor.",
            );
        });
        group(ui, "Contrast", |ui| {
            ui.checkbox(&mut self.enforce_contrast, "Enforce minimum contrast");
            ui.add_enabled(
                self.enforce_contrast,
                egui::Slider::new(&mut self.minimum_contrast, 1.0..=21.0)
                    .step_by(0.5)
                    .text("Minimum ratio"),
            );
        });
        let colors = self.colors(self.is_dark(ui.visuals().dark_mode));
        group(ui, "Custom colors", |ui| {
            color_row(
                ui,
                "Background",
                &mut self.custom_background_hex,
                colors.background,
            );
            color_row(
                ui,
                "Foreground",
                &mut self.custom_foreground_hex,
                colors.foreground,
            );
            color_row(ui, "Cursor", &mut self.custom_cursor_hex, colors.cursor);
            color_row(
                ui,
                "Selection",
                &mut self.custom_selection_hex,
                colors.selection,
            );
            note(
                ui,
                "Overrides apply to both schemes. Default returns to the selected theme.",
            );
        });
        group(ui, "Generated config", |ui| {
            let mut snapshot = serde_json::to_string_pretty(self).unwrap_or_default();
            egui::ScrollArea::vertical()
                .id_salt("settings_snapshot")
                .max_height(160.0)
                .show(ui, |ui| {
                    ui.add(
                        egui::TextEdit::multiline(&mut snapshot)
                            .code_editor()
                            .interactive(false)
                            .desired_width(f32::INFINITY),
                    );
                });
            if ui.button("Copy").clicked() {
                ui.ctx().copy_text(snapshot);
            }
            note(
                ui,
                "The current Rust terminal configuration; changes apply as you edit.",
            );
        });
    }

    fn font_ui(&mut self, ui: &mut egui::Ui) {
        ui.heading("Font");
        ui.add_space(12.0);
        group(ui, "Family", |ui| {
            let query_id = egui::Id::new("font_family_query");
            let mut query = ui
                .ctx()
                .data(|data| data.get_temp::<String>(query_id))
                .unwrap_or_default();
            ui.add(
                egui::TextEdit::singleline(&mut query)
                    .hint_text("Search fonts")
                    .desired_width(f32::INFINITY),
            );
            let query_lower = query.to_lowercase();
            egui::ScrollArea::vertical()
                .id_salt("installed_fonts")
                .max_height(180.0)
                .show(ui, |ui| {
                    if ui
                        .selectable_label(
                            self.font_family.is_empty(),
                            "Geist Mono · bundled default",
                        )
                        .clicked()
                    {
                        self.font_family.clear();
                    }
                    for (family, path) in installed_monospace_fonts()
                        .iter()
                        .filter(|(name, _)| name.to_lowercase().contains(&query_lower))
                    {
                        if ui
                            .selectable_label(self.font_family == *path, family)
                            .clicked()
                        {
                            self.font_family.clone_from(path);
                        }
                    }
                });
            ui.ctx().data_mut(|data| data.insert_temp(query_id, query));
            ui.label("Custom font file (.ttf or .otf)");
            ui.add(
                egui::TextEdit::singleline(&mut self.font_family)
                    .desired_width(f32::INFINITY)
                    .hint_text("Absolute path to a monospaced font"),
            );
            if ui.button("Choose font file…").clicked()
                && let Some(path) = rfd::FileDialog::new()
                    .add_filter("Fonts", &["ttf", "otf"])
                    .pick_file()
            {
                self.font_family = path.to_string_lossy().into_owned();
            }
            note(
                ui,
                "Installed monospaced fonts and custom TrueType/OpenType files. An imported macOS family name must be replaced with a local font.",
            );
            if !self.font_family.is_empty() && !std::path::Path::new(&self.font_family).is_file() {
                ui.colored_label(
                    Color32::from_rgb(230, 160, 80),
                    "Font file not found; using Geist Mono.",
                );
            }
        });
        group(ui, "Preview", |ui| {
            ui.label(
                RichText::new("Agile zebras vex $SHELL -l // 0123456789")
                    .monospace()
                    .size(self.effective_font_size()),
            );
        });
        group(ui, "Size", |ui| {
            let mut enabled = self.font_size.is_some();
            if ui.checkbox(&mut enabled, "Custom size").changed() {
                self.font_size = enabled.then_some(13.0);
            }
            let mut size = self.font_size.unwrap_or(13.0);
            if ui
                .add_enabled(
                    enabled,
                    egui::Slider::new(&mut size, 8.0..=32.0)
                        .step_by(0.5)
                        .text("Size"),
                )
                .changed()
            {
                self.font_size = Some(size);
            }
        });
        group(ui, "Weight", |ui| {
            ui.add_enabled_ui(false, |ui| {
                ui.checkbox(&mut self.font_thicken, "Thicken strokes (synthetic bold)");
                let mut strength = self.font_thicken_strength.unwrap_or(255);
                ui.add(egui::Slider::new(&mut strength, 0..=255).text("Strength"));
            });
            note(
                ui,
                "Ghostty synthetic stroke thickening is unavailable. ANSI bold text uses the bundled semibold font.",
            );
        });
    }

    fn cursor_ui(&mut self, ui: &mut egui::Ui) {
        ui.heading("Cursor");
        ui.add_space(12.0);
        group(ui, "Shape", |ui| {
            ui.horizontal(|ui| {
                ui.selectable_value(&mut self.cursor_style, CursorStyle::Block, "Block");
                ui.selectable_value(&mut self.cursor_style, CursorStyle::Bar, "Bar");
                ui.selectable_value(&mut self.cursor_style, CursorStyle::Underline, "Underline");
            });
            ui.checkbox(&mut self.cursor_blink, "Blink");
        });
        group(ui, "Opacity", |ui| {
            ui.add(egui::Slider::new(&mut self.cursor_opacity, 0.0..=1.0).text("Cursor opacity"));
            note(ui, "0% hides the cursor entirely.");
        });
    }

    fn terminal_ui(&mut self, ui: &mut egui::Ui) {
        ui.heading("Terminal");
        ui.add_space(12.0);
        group(ui, "Scrollback", |ui| {
            let mut enabled = self.scrollback_mb.is_some();
            if ui
                .checkbox(&mut enabled, "Custom scrollback limit")
                .changed()
            {
                self.scrollback_mb = enabled.then_some(10);
            }
            let mut limit = self.scrollback_mb.unwrap_or(10);
            if ui
                .add_enabled(
                    enabled,
                    egui::Slider::new(&mut limit, 1..=512).text("MB per pane"),
                )
                .changed()
            {
                self.scrollback_mb = Some(limit);
            }
            note(
                ui,
                "The memory limit determines scrollback capacity for new terminal sessions.",
            );
        });
        group(ui, "Shell", |ui| {
            ui.add(
                egui::TextEdit::singleline(&mut self.shell_path)
                    .desired_width(f32::INFINITY)
                    .hint_text(if cfg!(windows) {
                        "Auto (PowerShell or cmd.exe)"
                    } else {
                        "Auto ($SHELL)"
                    }),
            );
            note(
                ui,
                "Executable path only. Takes effect for new panes and reconnects.",
            );
        });
        group(ui, "Exited shells", |ui| {
            ui.checkbox(
                &mut self.auto_relaunch,
                "Relaunch shells that exit on their own",
            );
            ui.add_enabled(
                self.auto_relaunch,
                egui::Slider::new(&mut self.relaunch_after_seconds, 0.0..=120.0)
                    .step_by(1.0)
                    .text("Minimum lifetime (s)"),
            );
            note(
                ui,
                "Quick failures show the exited overlay to avoid a restart loop. SSH reconnects to the same remote tmux session.",
            );
        });
        group(ui, "History", |ui| {
            ui.add(
                egui::Slider::new(&mut self.transcript_max_mb, 1..=128).text("Transcript cap (MB)"),
            );
            ui.add(
                egui::Slider::new(&mut self.restore_tail_kb, 16..=2048)
                    .step_by(16.0)
                    .text("Restore tail (KB)"),
            );
            note(
                ui,
                "Transcripts persist per pane for the History viewer. Limits apply when a pane starts or reconnects.",
            );
        });
    }

    fn window_ui(&mut self, ui: &mut egui::Ui) {
        ui.heading("Window");
        ui.add_space(12.0);
        group(ui, "Default size", |ui| {
            ui.add(
                egui::Slider::new(&mut self.default_width, 640.0..=2560.0)
                    .step_by(10.0)
                    .text("Width (px)"),
            );
            ui.add(
                egui::Slider::new(&mut self.default_height, 400.0..=1600.0)
                    .step_by(10.0)
                    .text("Height (px)"),
            );
            if ui.button("Apply to window now").clicked() {
                ui.ctx()
                    .send_viewport_cmd(egui::ViewportCommand::InnerSize(egui::vec2(
                        self.default_width,
                        self.default_height,
                    )));
            }
            ui.checkbox(
                &mut self.remember_frame,
                "Remember window size across launches",
            );
        });
        group(ui, "Grid padding", |ui| {
            ui.add(egui::Slider::new(&mut self.padding_x, 0..=32).text("Horizontal"));
            ui.add(egui::Slider::new(&mut self.padding_y, 0..=32).text("Vertical"));
            note(
                ui,
                "Breathing room between the pane edge and the character grid.",
            );
        });
        group(ui, "Focus & tabs", |ui| {
            ui.add(
                egui::Slider::new(&mut self.unfocused_dim, 0.0..=0.6).text("Unfocused split dim"),
            );
            ui.checkbox(
                &mut self.show_single_tab,
                "Show the tab switcher with a single terminal",
            );
            ui.label("Tab close button");
            ui.horizontal(|ui| {
                ui.selectable_value(&mut self.tab_close_mode, TabCloseMode::Hover, "On hover");
                ui.selectable_value(&mut self.tab_close_mode, TabCloseMode::Always, "Always");
                ui.selectable_value(&mut self.tab_close_mode, TabCloseMode::Never, "Never");
            });
        });
        group(ui, "Resize", |ui| {
            let mut throttle = self.effective_resize_throttle_ms();
            if ui
                .add(
                    egui::Slider::new(&mut throttle, 0.0..=200.0)
                        .step_by(8.0)
                        .text("Coalesce (ms)"),
                )
                .changed()
            {
                self.resize_throttle_ms = (throttle > 0.0).then_some(throttle);
            }
            note(
                ui,
                "0 = every frame. Try 96 ms if a full-screen application flickers while resizing.",
            );
        });
    }

    fn ssh_ui(&mut self, ui: &mut egui::Ui) {
        ui.heading("SSH");
        ui.add_space(12.0);
        group(ui, "Persistent sessions", |ui| {
            note(
                ui,
                "SSH panes use tmux 3.2+ on the remote host. Disconnecting, sleeping or quitting leaves programs running. Reopening restores tabs and splits and reconnects to the same processes.",
            );
            ui.add_space(8.0);
            note(
                ui,
                "Closing a pane, tab or workspace ends its remote sessions. If the host is offline, the close request is saved and retried. Remote machine reboots end sessions.",
            );
        });
        group(ui, "New SSH workspaces", |ui| {
            ui.label("Session name prefix");
            ui.add(
                egui::TextEdit::singleline(&mut self.ssh_default_tmux)
                    .hint_text("bmux")
                    .desired_width(f32::INFINITY),
            );
            note(
                ui,
                "Optional prefix. Every pane gets a unique session. Letters, numbers, underscore and hyphen.",
            );
            if !self
                .ssh_default_tmux
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
            {
                ui.colored_label(
                    Color32::from_rgb(230, 160, 80),
                    "Other characters will be replaced when creating a session.",
                );
            }
        });
    }

    fn advanced_ui(&mut self, ui: &mut egui::Ui) {
        ui.heading("Advanced");
        ui.add_space(12.0);
        group(ui, "Extra Ghostty config", |ui| {
            ui.add_enabled(
                false,
                egui::TextEdit::multiline(&mut self.extra_config)
                    .code_editor()
                    .desired_rows(7)
                    .desired_width(f32::INFINITY),
            );
            note(
                ui,
                "Ghostty engine directives are preserved on import but are unavailable in the Rust renderer. Use the supported settings in the other sections.",
            );
        });
        group(ui, "Data", |ui| {
            let directory = crate::model::Store::default_directory();
            ui.label(
                RichText::new(directory.to_string_lossy())
                    .monospace()
                    .small(),
            );
            if ui.button("Reveal support folder").clicked() {
                let result = reveal_directory(&directory);
                let error_id = egui::Id::new("settings_reveal_error");
                ui.ctx().data_mut(|data| {
                    data.insert_temp(
                        error_id,
                        result.err().map(|e| e.to_string()).unwrap_or_default(),
                    )
                });
            }
            let error: String = ui
                .ctx()
                .data(|data| data.get_temp(egui::Id::new("settings_reveal_error")))
                .unwrap_or_default();
            if !error.is_empty() {
                ui.colored_label(Color32::from_rgb(230, 100, 80), error);
            }
            note(
                ui,
                "Workspaces, settings, transcripts, and pending remote cleanup requests are stored here.",
            );
        });
        group(ui, "Reset", |ui| {
            let id = egui::Id::new("settings_confirm_reset");
            let mut confirming = ui
                .ctx()
                .data(|data| data.get_temp::<bool>(id))
                .unwrap_or(false);
            if ui.button("Reset all settings to defaults").clicked() {
                confirming = true;
            }
            if confirming {
                ui.label("Reset appearance, font, terminal, window, and SSH preferences?");
                ui.horizontal(|ui| {
                    if ui.button("Reset").clicked() {
                        *self = Self::default();
                        confirming = false;
                    }
                    if ui.button("Cancel").clicked() {
                        confirming = false;
                    }
                });
            }
            ui.ctx().data_mut(|data| data.insert_temp(id, confirming));
        });
    }
}

fn group(ui: &mut egui::Ui, title: &str, body: impl FnOnce(&mut egui::Ui)) {
    ui.add_space(8.0);
    ui.label(RichText::new(title).strong());
    ui.add_space(4.0);
    egui::Frame::group(ui.style())
        .inner_margin(12)
        .corner_radius(8)
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            body(ui);
        });
    ui.add_space(5.0);
}

fn note(ui: &mut egui::Ui, text: &str) {
    ui.add(egui::Label::new(RichText::new(text).small().weak()).wrap());
}

fn color_row(ui: &mut egui::Ui, title: &str, value: &mut Option<String>, fallback: Color32) {
    ui.horizontal(|ui| {
        let color = value.as_deref().and_then(parse_hex).unwrap_or(fallback);
        let mut rgb = [color.r(), color.g(), color.b()];
        if ui.color_edit_button_srgb(&mut rgb).changed() {
            *value = Some(to_hex(Color32::from_rgb(rgb[0], rgb[1], rgb[2])));
        }
        ui.label(title);
        ui.label(
            RichText::new(value.clone().unwrap_or_else(|| to_hex(fallback)))
                .monospace()
                .small()
                .weak(),
        );
        if ui
            .add_enabled(value.is_some(), egui::Button::new("Default").small())
            .clicked()
        {
            *value = None;
        }
    });
}

fn theme_picker(ui: &mut egui::Ui, selected: &mut String, salt: &'static str) {
    ui.push_id(salt, |ui| {
        let query_id = ui.id().with("query");
        let filter_id = ui.id().with("filter");
        let mut query = ui
            .ctx()
            .data(|data| data.get_temp::<String>(query_id))
            .unwrap_or_default();
        let mut filter = ui
            .ctx()
            .data(|data| data.get_temp::<usize>(filter_id))
            .unwrap_or(0);
        ui.add(
            egui::TextEdit::singleline(&mut query)
                .hint_text("Search themes")
                .desired_width(f32::INFINITY),
        );
        ui.horizontal(|ui| {
            ui.selectable_value(&mut filter, 0, "All");
            ui.selectable_value(&mut filter, 1, "Dark");
            ui.selectable_value(&mut filter, 2, "Light");
        });
        ui.label(RichText::new(format!("Current: {selected}")).strong());
        let query_lower = query.trim().to_lowercase();
        let matches: Vec<_> = themes()
            .iter()
            .filter(|theme| theme.name.to_lowercase().contains(&query_lower))
            .filter(|theme| filter == 0 || (filter == 1) == theme.is_dark())
            .collect();
        egui::ScrollArea::vertical()
            .max_height(175.0)
            .id_salt("catalog")
            .show_rows(ui, 30.0, matches.len(), |ui, rows| {
                for index in rows {
                    let theme = matches[index];
                    ui.horizontal(|ui| {
                        let (swatches, _) =
                            ui.allocate_exact_size(egui::vec2(64.0, 24.0), egui::Sense::hover());
                        for (index, color) in theme.palette.iter().skip(1).take(5).enumerate() {
                            ui.painter().circle_filled(
                                egui::pos2(
                                    swatches.left() + 8.0 + index as f32 * 11.0,
                                    swatches.center().y,
                                ),
                                7.0,
                                parse_hex(color).unwrap_or(Color32::GRAY),
                            );
                        }
                        if ui
                            .selectable_label(*selected == theme.name, &theme.name)
                            .clicked()
                        {
                            *selected = theme.name.clone();
                        }
                        ui.label(
                            RichText::new(if theme.is_dark() { "Dark" } else { "Light" })
                                .small()
                                .weak(),
                        );
                    });
                }
            });
        if matches.is_empty() {
            note(ui, "No matching themes");
        }
        note(
            ui,
            &format!(
                "{} themes · Select a theme to preview it live",
                matches.len()
            ),
        );
        if !themes().iter().any(|theme| theme.name == *selected) {
            note(
                ui,
                "The saved theme is missing. The default theme is used until you choose another.",
            );
        }
        ui.ctx().data_mut(|data| {
            data.insert_temp(query_id, query);
            data.insert_temp(filter_id, filter);
        });
    });
}

fn workspaces_ui(ui: &mut egui::Ui, workspaces: &mut [Workspace]) -> bool {
    ui.heading("Workspaces");
    ui.add_space(12.0);
    let mut changed = false;
    for workspace in workspaces.iter_mut() {
        ui.push_id(workspace.id, |ui| {
            group(ui, &workspace.name, |ui| {
                ui.horizontal_wrapped(|ui| {
                    if ui
                        .selectable_label(workspace.color_hex.is_empty(), "Default")
                        .clicked()
                    {
                        workspace.color_hex.clear();
                        changed = true;
                    }
                    for hex in [
                        "#4DA3FF", "#2DD4BF", "#7ED957", "#F7C948", "#FF9F43", "#F472B6", "#A78BFA",
                    ] {
                        let (rect, response) =
                            ui.allocate_exact_size(egui::vec2(25.0, 28.0), egui::Sense::click());
                        ui.painter()
                            .circle_filled(rect.center(), 10.0, parse_hex(hex).unwrap());
                        if workspace.color_hex.eq_ignore_ascii_case(hex) {
                            ui.painter().circle_stroke(
                                rect.center(),
                                12.0,
                                egui::Stroke::new(1.5_f32, ui.visuals().text_color()),
                            );
                        }
                        if response.on_hover_text(hex).clicked() {
                            workspace.color_hex = hex.into();
                            changed = true;
                        }
                    }
                    let color =
                        parse_hex(&workspace.color_hex).unwrap_or(Color32::from_rgb(255, 135, 0));
                    let mut rgb = [color.r(), color.g(), color.b()];
                    if ui.color_edit_button_srgb(&mut rgb).changed() {
                        workspace.color_hex = to_hex(Color32::from_rgb(rgb[0], rgb[1], rgb[2]));
                        changed = true;
                    }
                });
            });
        });
    }
    if workspaces.is_empty() {
        note(ui, "No workspaces yet.");
    }
    note(
        ui,
        "The stripe shows on the workspace's sidebar row. Default removes it.",
    );
    changed
}

fn reveal_directory(path: &std::path::Path) -> std::io::Result<()> {
    std::fs::create_dir_all(path)?;
    #[cfg(target_os = "windows")]
    let mut command = std::process::Command::new("explorer.exe");
    #[cfg(target_os = "macos")]
    let mut command = std::process::Command::new("open");
    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    let mut command = std::process::Command::new("xdg-open");
    command.arg(path).spawn().map(|_| ())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_match_swift_and_missing_keys_keep_defaults() {
        let settings: Settings = serde_json::from_str("{}").unwrap();
        assert_eq!(settings, Settings::default());
        assert_eq!(settings.dark_theme_name, "Srcery");
        assert_eq!(settings.light_theme_name, "Alabaster");
        assert_eq!(settings.initial_size(), [1280.0, 800.0]);
        assert_eq!(settings.padding_x, 6);
        assert_eq!(settings.unfocused_dim, 0.28);
    }

    #[test]
    fn imports_swift_settings_names_without_losing_values() {
        let settings: Settings = serde_json::from_str(r##"{"darkThemeName":"Dracula","customBackgroundHex":"#101010","fontSize":16.5,"cursorStyle":"bar","cursorBlink":false,"shellPath":"/bin/bash","scrollbackMB":32,"resizeThrottleMs":96,"tabCloseMode":"always"}"##).unwrap();
        assert_eq!(settings.dark_theme_name, "Dracula");
        assert_eq!(settings.font_size, Some(16.5));
        assert_eq!(settings.cursor_style, CursorStyle::Bar);
        assert!(!settings.cursor_blink);
        assert_eq!(settings.scrollback_mb, Some(32));
        assert_eq!(settings.resize_throttle_ms, Some(96.0));
        assert_eq!(settings.tab_close_mode, TabCloseMode::Always);
        assert_eq!(
            settings.colors(true).background,
            Color32::from_rgb(16, 16, 16)
        );
    }

    #[test]
    fn theme_catalog_has_valid_defaults_and_complete_palettes() {
        assert!(themes().len() > 400);
        for theme in themes() {
            assert!(
                parse_hex(&theme.background).is_some(),
                "{} background",
                theme.name
            );
            assert!(
                parse_hex(&theme.foreground).is_some(),
                "{} foreground",
                theme.name
            );
            for color in &theme.palette {
                assert!(parse_hex(color).is_some(), "{} palette", theme.name);
            }
        }
        assert_eq!(
            Settings::default().colors(true).background,
            Color32::from_rgb(28, 27, 25)
        );
        assert_eq!(
            Settings::default().colors(false).background,
            Color32::from_rgb(247, 247, 247)
        );
    }

    #[test]
    fn contrast_enforcement_and_override_apply_to_both_schemes() {
        let settings = Settings {
            custom_background_hex: Some("#101010".into()),
            custom_foreground_hex: Some("#121212".into()),
            enforce_contrast: true,
            minimum_contrast: 4.5,
            ..Settings::default()
        };
        for dark in [true, false] {
            let colors = settings.colors(dark);
            assert!(contrast(colors.foreground, colors.background) >= 4.5);
            for color in colors.palette {
                assert!(contrast(color, colors.background) >= 4.5);
            }
        }
    }

    #[test]
    fn malformed_hex_and_out_of_range_preferences_are_handled() {
        assert!(parse_hex("#💥00").is_none());
        assert!(parse_hex("#ZZZZZZ").is_none());
        let mut settings = Settings {
            padding_x: 800,
            font_size: Some(-3.0),
            cursor_opacity: 12.0,
            ..Settings::default()
        };
        settings.sanitize();
        assert_eq!(settings.padding_x, 32);
        assert_eq!(settings.font_size, Some(8.0));
        assert_eq!(settings.cursor_opacity, 1.0);
    }

    #[test]
    fn invalid_custom_fonts_are_recoverable() {
        let temp = tempfile::tempdir().unwrap();
        let invalid = temp.path().join("not-a-font.ttf");
        std::fs::write(&invalid, "ordinary text").unwrap();
        assert!(load_font_file(&invalid).is_err());
        assert!(load_font_file(temp.path()).is_err());
        let font = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("Assets/Fonts/GeistMono-Regular.ttf");
        assert!(load_font_file(&font).is_ok());
    }

    #[test]
    fn every_settings_category_renders_without_modifying_preferences() {
        let ctx = egui::Context::default();
        let mut settings = Settings::default();
        let mut state = crate::model::State::default();
        for selection in 0..8_usize {
            ctx.data_mut(|data| data.insert_temp(egui::Id::new("settings_section"), selection));
            let mut open = true;
            let _ = ctx.run(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(1280.0, 800.0),
                    )),
                    ..Default::default()
                },
                |ctx| {
                    assert!(!settings.show_with_workspaces(ctx, &mut open, &mut state.workspaces));
                },
            );
            assert!(open);
            let rect = ctx
                .memory(|memory| memory.area_rect(egui::Id::new("bmux_settings")))
                .unwrap();
            assert!(
                rect.width() <= 850.0,
                "Category {selection} exceeded settings width: {rect:?}"
            );
            assert!(
                rect.height() <= 680.0,
                "Category {selection} exceeded settings height: {rect:?}"
            );
        }
        assert_eq!(settings, Settings::default());
    }
}

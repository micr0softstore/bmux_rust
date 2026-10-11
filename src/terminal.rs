//! Native PTY/ConPTY terminal, ANSI screen, input encoder and bounded transcript recorder.
//!
//! A persistent pane owns only an attached tmux client. Dropping that client detaches;
//! `close` is the explicit operation that destroys the associated tmux session.
use anyhow::{Context, Result, bail};
use eframe::egui::{
    self, Color32, FontId, Key, Modifiers, Pos2, Rect, Response, Sense, Stroke, Ui, Vec2,
};
use portable_pty::{Child, CommandBuilder, MasterPty, PtySize};
use std::{
    fs::{self, File, OpenOptions},
    io::{Read, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::mpsc::{self, Receiver},
    thread,
    time::{Duration, Instant},
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SessionKind {
    Local,
    LocalTmux { session: String },
    Ssh { command: String, session: String },
}

#[derive(Clone, Debug)]
pub struct SpawnSpec {
    pub pane_id: String,
    pub cwd: PathBuf,
    pub command: Option<String>,
    pub shell: String,
    pub kind: SessionKind,
    pub transcript_dir: PathBuf,
    pub transcript_max_mb: usize,
    pub restore_tail_kb: usize,
    pub scrollback_lines: usize,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum CursorShape {
    #[default]
    Block,
    Bar,
    Underline,
}

#[derive(Clone, Debug)]
pub struct TerminalStyle {
    pub foreground: Color32,
    pub background: Color32,
    pub cursor: Color32,
    pub selection: Color32,
    pub palette: [Color32; 16],
    pub font_size: f32,
    pub padding: f32,
    pub padding_y: f32,
    pub cursor_opacity: f32,
    pub resize_throttle_ms: f32,
    /// Multiplier applied to the actual font row height.
    pub line_height: f32,
    pub cursor_shape: CursorShape,
    pub cursor_blink: bool,
    pub scroll_multiplier: f32,
    pub copy_on_select: bool,
    pub bold_is_bright: bool,
}

impl Default for TerminalStyle {
    fn default() -> Self {
        Self {
            foreground: Color32::from_rgb(225, 225, 229),
            background: Color32::from_rgb(20, 20, 24),
            cursor: Color32::from_rgb(225, 225, 229),
            selection: Color32::from_rgba_unmultiplied(111, 133, 185, 100),
            palette: [
                (29, 31, 33),
                (204, 102, 102),
                (181, 189, 104),
                (240, 198, 116),
                (129, 162, 190),
                (178, 148, 187),
                (138, 190, 183),
                (197, 200, 198),
                (102, 102, 102),
                (213, 78, 83),
                (185, 202, 74),
                (231, 197, 71),
                (122, 166, 218),
                (195, 151, 216),
                (112, 192, 177),
                (234, 234, 234),
            ]
            .map(|(r, g, b)| Color32::from_rgb(r, g, b)),
            font_size: 14.0,
            padding: 12.0,
            padding_y: 12.0,
            cursor_opacity: 0.65,
            resize_throttle_ms: 0.0,
            line_height: 1.15,
            cursor_shape: CursorShape::Block,
            cursor_blink: true,
            scroll_multiplier: 1.0,
            copy_on_select: false,
            bold_is_bright: true,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct CellPos {
    row: u16,
    col: u16,
}

/// Hyperlink advertised by the application (OSC 8), attributed to the cells
/// between the opening and closing sequences via the cursor positions seen
/// when each boundary arrived. Positions go stale when the grid reflows, so
/// spans are dropped on resize, clear and explicit scrolling; plain-text URL
/// detection always re-scans the live cells instead.
#[derive(Clone, Debug)]
struct Hyperlink {
    uri: String,
    start_row: u16,
    start_col: u16,
    end_row: u16,
    end_col: u16,
}

impl Hyperlink {
    fn covers(&self, cell: CellPos) -> bool {
        if cell.row < self.start_row || cell.row > self.end_row {
            return false;
        }
        if self.start_row == self.end_row {
            return cell.col >= self.start_col && cell.col <= self.end_col;
        }
        if cell.row == self.start_row {
            return cell.col >= self.start_col;
        }
        if cell.row == self.end_row {
            return cell.col <= self.end_col;
        }
        true
    }
}

/// Characters allowed inside a detected URL. Wrapping punctuation such as
/// quotes or parentheses is stripped after the match, not during it.
fn is_url_char(ch: char) -> bool {
    ch.is_ascii_alphanumeric() || "-._~:/?#[]@!$&'()*+,;=%".contains(ch)
}

/// Punctuation that closes a sentence rather than a URL.
fn is_trailing_url_punct(ch: char) -> bool {
    matches!(
        ch,
        '.' | ',' | ';' | ':' | '!' | '?' | ')' | ']' | '}' | '\'' | '"'
    )
}

/// Plain-text URLs in `text` as (start, end-exclusive) char indices plus the
/// URL. `www.` matches gain an `https://` prefix at open time so the opener
/// does not treat them as file paths.
fn find_urls(text: &str) -> Vec<(usize, usize, String)> {
    const SCHEMES: [&[char]; 3] = [
        &['h', 't', 't', 'p', 's', ':', '/', '/'],
        &['h', 't', 't', 'p', ':', '/', '/'],
        &['f', 'i', 'l', 'e', ':', '/', '/'],
    ];
    const WWW: [char; 4] = ['w', 'w', 'w', '.'];
    let chars: Vec<char> = text.chars().collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        let mut prefix_len = 0;
        let mut bare_www = false;
        for scheme in SCHEMES {
            if chars[i..].starts_with(scheme) {
                prefix_len = scheme.len();
                break;
            }
        }
        if prefix_len == 0
            && chars[i..].starts_with(&WWW)
            && (i == 0 || !chars[i - 1].is_ascii_alphanumeric())
        {
            prefix_len = WWW.len();
            bare_www = true;
        }
        if prefix_len == 0 {
            i += 1;
            continue;
        }
        let mut end = i + prefix_len;
        while end < chars.len() && is_url_char(chars[end]) {
            end += 1;
        }
        let mut trimmed = end;
        while trimmed > i + prefix_len && is_trailing_url_punct(chars[trimmed - 1]) {
            trimmed -= 1;
        }
        if trimmed > i + prefix_len {
            let url: String = chars[i..trimmed].iter().collect();
            out.push((
                i,
                trimmed,
                if bare_www {
                    format!("https://{url}")
                } else {
                    url
                },
            ));
            i = end.max(i + 1);
        } else {
            i += 1;
        }
    }
    out
}

fn cell_at_pos(content: Rect, cw: f32, ch: f32, rows: u16, cols: u16, pos: Pos2) -> CellPos {
    CellPos {
        row: ((pos.y - content.top()) / ch)
            .floor()
            .clamp(0.0, rows.saturating_sub(1) as f32) as u16,
        col: ((pos.x - content.left()) / cw)
            .floor()
            .clamp(0.0, cols.saturating_sub(1) as f32) as u16,
    }
}

pub struct Terminal {
    spec: SpawnSpec,
    master: Box<dyn MasterPty + Send>,
    writer: Box<dyn Write + Send>,
    child: Option<Box<dyn Child + Send + Sync>>,
    output: Receiver<Vec<u8>>,
    parser: vt100::Parser,
    protocol: Protocol,
    started: Instant,
    exited: Option<u32>,
    size: (u16, u16),
    transcript: PathBuf,
    cwd: PathBuf,
    selection: Option<(CellPos, CellPos)>,
    last_mouse: Option<CellPos>,
    mouse_button: Option<u8>,
    context_cell: Option<CellPos>,
    last_input: Instant,
    had_focus: bool,
    ime: String,
    scroll_fraction: f32,
    last_cwd_check: Instant,
    cwd_receiver: Option<Receiver<(PathBuf, String)>>,
    foreground: String,
    last_resize: Instant,
    last_cleanup: Instant,
    pub last_error: Option<String>,
}

impl Terminal {
    pub fn spawn(spec: SpawnSpec, ctx: egui::Context) -> Result<Self> {
        if spec.pane_id.is_empty()
            || !spec
                .pane_id
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
        {
            bail!("Invalid pane identifier");
        }
        let (rows, cols) = (24, 80);
        let mut command = build_command(&spec)?;
        if spec.cwd.is_dir() {
            command.cwd(&spec.cwd);
        }
        command.env("TERM", "xterm-256color");
        command.env("COLORTERM", "truecolor");
        command.env("TERM_PROGRAM", "bmux");
        command.env("TERM_PROGRAM_VERSION", env!("CARGO_PKG_VERSION"));
        command.env_remove("TMUX");
        command.env_remove("TMUX_PANE");
        let detected_shell = default_shell();
        // Respected by bash installations that do not replace PROMPT_COMMAND.
        if matches!(spec.kind, SessionKind::Local)
            && shell_basename(if spec.shell.is_empty() {
                &detected_shell
            } else {
                &spec.shell
            }) == "bash"
        {
            let prior = std::env::var("PROMPT_COMMAND").unwrap_or_default();
            command.env(
                "PROMPT_COMMAND",
                format!("printf '\\033]7;file://localhost%s\\007' \"$PWD\"; {prior}"),
            );
        }
        fs::create_dir_all(&spec.transcript_dir).context("Create transcript directory")?;
        retry_cleanup(spec.transcript_dir.join("pending-close"));
        let transcript = spec.transcript_dir.join(format!("{}.ts", spec.pane_id));
        let max_bytes = spec.transcript_max_mb.max(1).saturating_mul(1024 * 1024) as u64;
        let mut parser =
            vt100::Parser::new(rows, cols, spec.scrollback_lines.clamp(100, 1_000_000));
        if matches!(spec.kind, SessionKind::Local) && spec.restore_tail_kb > 0 {
            let restored = read_tail(&transcript, spec.restore_tail_kb.min(16 * 1024) * 1024);
            if !restored.is_empty() {
                parser.process(&restored);
                // Recordings cannot carry an alternate-screen/input mode into a fresh shell.
                parser.process(
                    b"\x1b[?1049l\x1b[?1l\x1b[?2004l\x1b[?1000l\x1b[?1002l\x1b[?1003l\x1b[0m\r\n",
                );
            }
        }
        let mut recorder = Recorder::new(transcript.clone(), max_bytes)?;
        let pair = portable_pty::native_pty_system()
            .openpty(PtySize {
                rows,
                cols,
                pixel_width: 0,
                pixel_height: 0,
            })
            .context("Create native terminal (PTY/ConPTY)")?;
        let child = pair
            .slave
            .spawn_command(command)
            .context("Launch terminal process")?;
        drop(pair.slave);
        let mut reader = pair.master.try_clone_reader()?;
        let writer = pair.master.take_writer()?;
        // A slow/minimized renderer can retain at most one MiB of unread output.
        let (sender, output) = mpsc::sync_channel(64);
        thread::Builder::new()
            .name(format!("pty-{}", spec.pane_id))
            .spawn(move || {
                let mut buffer = [0; 16 * 1024];
                loop {
                    match reader.read(&mut buffer) {
                        Ok(0) => break,
                        Ok(n) => {
                            let _ = recorder.append(&buffer[..n]);
                            if sender.send(buffer[..n].to_vec()).is_err() {
                                break;
                            }
                            ctx.request_repaint();
                        }
                        Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
                        Err(_) => break,
                    }
                }
                ctx.request_repaint();
            })?;
        let cwd = spec.cwd.clone();
        Ok(Self {
            spec,
            master: pair.master,
            writer,
            child: Some(child),
            output,
            parser,
            protocol: Protocol::default(),
            started: Instant::now(),
            exited: None,
            size: (rows, cols),
            transcript,
            cwd,
            selection: None,
            last_mouse: None,
            mouse_button: None,
            context_cell: None,
            last_input: Instant::now(),
            had_focus: false,
            ime: String::new(),
            scroll_fraction: 0.0,
            last_cwd_check: Instant::now(),
            cwd_receiver: None,
            foreground: String::new(),
            last_resize: Instant::now(),
            last_cleanup: Instant::now(),
            last_error: None,
        })
    }

    /// Drain output regularly even for hidden tabs so their child process never stalls.
    pub fn poll(&mut self) {
        let deadline = Instant::now() + Duration::from_millis(6);
        while let Ok(bytes) = self.output.try_recv() {
            let replies = self.protocol.process(&bytes, &mut self.parser);
            if let Some(cwd) = self.protocol.cwd.take() {
                self.cwd = cwd;
            }
            for reply in replies {
                let _ = self.writer.write_all(&reply);
            }
            if Instant::now() >= deadline {
                break;
            }
        }
        if self.exited.is_none()
            && let Some(child) = self.child.as_mut()
        {
            match child.try_wait() {
                Ok(Some(status)) => self.exited = Some(status.exit_code()),
                Err(e) => self.last_error = Some(e.to_string()),
                _ => {}
            }
        }
        if let Some(receiver) = &self.cwd_receiver {
            match receiver.try_recv() {
                Ok((cwd, command)) => {
                    self.cwd = cwd;
                    self.foreground = command;
                    self.cwd_receiver = None;
                }
                Err(mpsc::TryRecvError::Disconnected) => self.cwd_receiver = None,
                _ => {}
            }
        }
        if self.last_cleanup.elapsed() > Duration::from_secs(30) {
            self.last_cleanup = Instant::now();
            retry_cleanup(self.spec.transcript_dir.join("pending-close"));
        }
        if self.is_alive() && self.last_cwd_check.elapsed() > Duration::from_secs(2) {
            self.last_cwd_check = Instant::now();
            self.refresh_cwd();
        }
    }

    pub fn is_alive(&self) -> bool {
        self.exited.is_none() && self.child.is_some()
    }
    pub fn exit_code(&self) -> Option<u32> {
        self.exited
    }
    pub fn started_elapsed(&self) -> Duration {
        self.started.elapsed()
    }
    pub fn title(&self) -> String {
        self.parser.screen().title().to_owned()
    }
    pub fn cwd(&self) -> PathBuf {
        self.cwd.clone()
    }
    pub fn foreground_command(&self) -> String {
        self.foreground.clone()
    }
    #[cfg(all(test, unix))]
    pub fn screen_text(&self) -> String {
        self.parser.screen().contents()
    }
    pub fn transcript_path(&self) -> &Path {
        &self.transcript
    }

    pub fn write(&mut self, bytes: &[u8]) -> Result<()> {
        if !self.is_alive() {
            bail!("Terminal process has exited");
        }
        self.writer.write_all(bytes).context("Write to terminal")?;
        self.writer.flush()?;
        self.last_input = Instant::now();
        self.parser.set_scrollback(0);
        Ok(())
    }
    pub fn paste(&mut self, text: &str) -> Result<()> {
        // Embedded ESC cannot end bracketed paste or inject terminal control commands.
        let clean = text
            .replace('\x1b', "")
            .replace("\r\n", "\n")
            .replace('\r', "\n");
        if self.parser.screen().bracketed_paste() {
            self.write(format!("\x1b[200~{clean}\x1b[201~").as_bytes())
        } else {
            self.write(clean.replace('\n', "\r").as_bytes())
        }
    }
    pub fn clear(&mut self) {
        self.parser.process(b"\x1b[3J\x1b[2J\x1b[H");
        self.selection = None;
        self.protocol.drop_links();
        self.parser.set_scrollback(0);
    }
    pub fn scroll_top(&mut self) {
        self.selection = None;
        self.protocol.drop_links();
        self.parser.set_scrollback(usize::MAX);
    }
    pub fn scroll_bottom(&mut self) {
        self.selection = None;
        self.parser.set_scrollback(0);
    }
    pub fn history(&self) -> String {
        plain_text(&read_tail(&self.transcript, 1024 * 1024))
    }
    pub fn resize(&mut self, rows: u16, cols: u16) -> Result<()> {
        let size = (rows.max(2), cols.max(2));
        if self.size != size {
            self.master.resize(PtySize {
                rows: size.0,
                cols: size.1,
                pixel_width: 0,
                pixel_height: 0,
            })?;
            self.parser.set_size(size.0, size.1);
            self.size = size;
            self.selection = None;
            // Recorded OSC 8 cells no longer line up after a reflow.
            self.protocol.drop_links();
        }
        Ok(())
    }

    /// Deliberate pane/workspace deletion. Regular application shutdown calls Drop only.
    pub fn close(mut self) -> Result<()> {
        if matches!(self.spec.kind, SessionKind::Local) {
            self.detach();
            return Ok(());
        }
        let directory = self.spec.transcript_dir.join("pending-close");
        queue_cleanup(&directory, &self.spec.pane_id, &self.spec.kind)
            .context("Save persistent-session close request")?;
        // The reserved tmux user key goes through the already authenticated transport.
        // This also closes password-only SSH sessions without a second authentication.
        let _ = self.write(TMUX_CLOSE_KEY.as_bytes());
        thread::spawn(move || {
            let deadline = Instant::now() + Duration::from_secs(3);
            while self.is_alive() && Instant::now() < deadline {
                self.poll();
                thread::sleep(Duration::from_millis(20));
            }
            retry_cleanup(directory);
            // Drop detaches only if the transport did not exit by itself.
        });
        Ok(())
    }
    fn detach(&mut self) {
        if let Some(mut child) = self.child.take() {
            let _ = child.kill();
            // Reap on a worker; a slow SSH/ConPTY teardown must not block the UI.
            thread::spawn(move || {
                let _ = child.wait();
            });
        }
    }

    fn refresh_cwd(&mut self) {
        match &self.spec.kind {
            SessionKind::Local => {
                #[cfg(target_os = "linux")]
                if let Some(pid) = self.child.as_ref().and_then(|c| c.process_id())
                    && let Ok(path) = fs::read_link(format!("/proc/{pid}/cwd"))
                {
                    self.cwd = path;
                    self.foreground = linux_foreground_command(pid);
                }
            }
            SessionKind::LocalTmux { .. } | SessionKind::Ssh { .. }
                if self.cwd_receiver.is_none() =>
            {
                let kind = self.spec.kind.clone();
                let (sender, receiver) = mpsc::channel();
                self.cwd_receiver = Some(receiver);
                thread::spawn(move || {
                    let Ok(mut command) = metadata_command(&kind) else {
                        return;
                    };
                    let Ok(output) = bounded_output(&mut command, Duration::from_secs(10)) else {
                        return;
                    };
                    if output.status.success() {
                        let text = String::from_utf8_lossy(&output.stdout);
                        // Login banners can precede the final metadata line.
                        if let Some(line) = text.lines().last()
                            && let Some((path, foreground)) = line.rsplit_once('\t')
                            && !path.is_empty()
                        {
                            let _ = sender.send((PathBuf::from(path), foreground.to_owned()));
                        }
                    }
                });
            }
            _ => {}
        }
    }

    pub fn selected_text(&self) -> String {
        let Some((a, b)) = self.selection else {
            return String::new();
        };
        let (start, end) = if a <= b { (a, b) } else { (b, a) };
        self.parser.screen().contents_between(
            start.row,
            start.col,
            end.row,
            end.col.saturating_add(1),
        )
    }
    pub fn select_all(&mut self) {
        self.selection = Some((
            CellPos { row: 0, col: 0 },
            CellPos {
                row: self.size.0 - 1,
                col: self.size.1 - 1,
            },
        ));
    }
    /// Raw visible text of one row plus the starting cell of every char, so
    /// URL spans map back to cells. Wide continuation cells add no chars.
    fn row_segment(&self, row: u16) -> (String, Vec<u16>) {
        let screen = self.parser.screen();
        let mut text = String::new();
        let mut cols = Vec::new();
        for col in 0..self.size.1 {
            let Some(cell) = screen.cell(row, col) else {
                continue;
            };
            if cell.is_wide_continuation() {
                continue;
            }
            for ch in cell.contents().chars() {
                text.push(ch);
                cols.push(col);
            }
        }
        (text, cols)
    }
    fn row_is_full(&self, row: u16) -> bool {
        self.size.1 > 0
            && self
                .parser
                .screen()
                .cell(row, self.size.1 - 1)
                .is_some_and(|cell| !cell.contents().is_empty())
    }
    /// Plain-text URLs covering `row`. A match that runs into the last cell
    /// of a full row may be wrapped onto the next rows (up to four total), so
    /// long links stay clickable in narrow windows. Rows are only joined when
    /// the boundary is glued (both sides URL characters) and the next row
    /// does not start a fresh URL, so adjacent links never merge.
    fn url_hits(&self, row: u16) -> Vec<Hyperlink> {
        const SCHEME_PREFIXES: [&str; 4] = ["https://", "http://", "file://", "www."];
        let mut text = String::new();
        let mut map: Vec<(u16, u16)> = Vec::new();
        let mut current = row;
        loop {
            let (segment, cols) = self.row_segment(current);
            text.push_str(&segment);
            map.extend(cols.iter().map(|col| (current, *col)));
            if current - row >= 3 || current + 1 >= self.size.0 {
                break;
            }
            if !self.row_is_full(current) {
                break;
            }
            let (next_text, _) = self.row_segment(current + 1);
            let glued = text.chars().last().is_some_and(is_url_char)
                && next_text.chars().next().is_some_and(is_url_char);
            let fresh_url = SCHEME_PREFIXES
                .iter()
                .any(|prefix| next_text.starts_with(prefix));
            if !glued || fresh_url {
                break;
            }
            current += 1;
        }
        find_urls(&text)
            .into_iter()
            .filter_map(|(start, end, uri)| {
                let (start_row, start_col) = map.get(start).copied()?;
                let (end_row, end_col) = map.get(end.saturating_sub(1)).copied()?;
                Some(Hyperlink {
                    uri,
                    start_row,
                    start_col,
                    end_row,
                    end_col,
                })
            })
            .collect()
    }
    /// Clickable link under a cell: OSC 8 spans first, then plain-text URLs
    /// re-scanned from the live grid.
    fn hit_link(&self, cell: CellPos) -> Option<Hyperlink> {
        if let Some(link) = self.protocol.links.iter().find(|link| link.covers(cell)) {
            return Some(link.clone());
        }
        self.url_hits(cell.row)
            .into_iter()
            .find(|hit| hit.covers(cell))
    }

    pub fn show(
        &mut self,
        ui: &mut Ui,
        rect: Rect,
        focused: bool,
        style: &TerminalStyle,
    ) -> Response {
        self.protocol.colors = [style.foreground, style.background, style.cursor];
        self.protocol.palette = style.palette;
        self.poll();
        let response = ui.interact(
            rect,
            ui.id().with(&self.spec.pane_id),
            Sense::click_and_drag(),
        );
        if response.clicked() || response.secondary_clicked() {
            response.request_focus();
        }
        if focused {
            ui.memory_mut(|m| {
                m.set_focus_lock_filter(
                    response.id,
                    egui::EventFilter {
                        tab: true,
                        horizontal_arrows: true,
                        vertical_arrows: true,
                        escape: true,
                    },
                )
            });
        }
        let font = FontId::monospace(style.font_size.clamp(6.0, 48.0));
        let bold_family = egui::FontFamily::Name("terminal-bold".into());
        let has_bold_font = ui.fonts(|f| f.families().contains(&bold_family));
        let cell_width = ui.fonts_mut(|f| f.glyph_width(&font, 'M')).max(1.0);
        let cell_height = ui.fonts_mut(|f| f.row_height(&font)) * style.line_height.clamp(0.8, 2.0);
        let content = rect.shrink2(Vec2::new(style.padding.max(0.0), style.padding_y.max(0.0)));
        let rows = (content.height() / cell_height).floor().clamp(2.0, 500.0) as u16;
        let cols = (content.width() / cell_width).floor().clamp(2.0, 1000.0) as u16;
        if self.last_resize.elapsed().as_secs_f32() * 1000.0
            >= style.resize_throttle_ms.clamp(0.0, 1000.0)
        {
            if let Err(e) = self.resize(rows, cols) {
                self.last_error = Some(e.to_string());
            }
            self.last_resize = Instant::now();
        } else if self.size != (rows, cols) {
            ui.ctx().request_repaint_after(Duration::from_millis(16));
        }
        let (rows, cols) = self.size;
        if self.protocol.focus_reporting && self.had_focus != focused {
            let _ = self.write(if focused { b"\x1b[I" } else { b"\x1b[O" });
        }
        self.had_focus = focused;
        self.handle_input(
            ui,
            &response,
            content,
            cell_width,
            cell_height,
            focused,
            style,
        );
        // Ctrl+hover underlines the link and offers it for opening. The scan
        // only runs while Ctrl is held and stays limited to the hovered rows.
        let hover_link = if ui.input(|i| i.modifiers.ctrl) {
            ui.input(|i| i.pointer.hover_pos())
                .filter(|pos| content.contains(*pos))
                .and_then(|pos| {
                    let cell = cell_at_pos(content, cell_width, cell_height, rows, cols, pos);
                    self.hit_link(cell)
                })
        } else {
            None
        };
        let painter = ui.painter().with_clip_rect(rect.intersect(ui.clip_rect()));
        painter.rect_filled(rect, 0.0, style.background);
        let screen = self.parser.screen();
        let selection = self
            .selection
            .map(|(a, b)| if a <= b { (a, b) } else { (b, a) });
        for row in 0..rows {
            for col in 0..cols {
                let Some(cell) = screen.cell(row, col) else {
                    continue;
                };
                if cell.is_wide_continuation() {
                    continue;
                }
                let pos = Pos2::new(
                    content.left() + col as f32 * cell_width,
                    content.top() + row as f32 * cell_height,
                );
                let width = cell_width * if cell.is_wide() { 2.0 } else { 1.0 };
                let cell_rect = Rect::from_min_size(pos, Vec2::new(width, cell_height));
                let mut fg = ansi_color(
                    cell.fgcolor(),
                    style.foreground,
                    &style.palette,
                    cell.bold() && style.bold_is_bright,
                );
                let mut bg = ansi_color(cell.bgcolor(), style.background, &style.palette, false);
                if cell.inverse() {
                    std::mem::swap(&mut fg, &mut bg);
                }
                if bg != style.background {
                    painter.rect_filled(cell_rect, 0.0, bg);
                }
                if let Some((a, b)) = selection
                    && (CellPos { row, col }) >= a
                    && (CellPos { row, col }) <= b
                {
                    painter.rect_filled(cell_rect, 0.0, style.selection);
                }
                let text = cell.contents();
                if !text.is_empty() {
                    let text_pos = pos + Vec2::new(0.0, (cell_height - font.size) * 0.25);
                    let cell_font = if cell.bold() && has_bold_font {
                        FontId::new(font.size, bold_family.clone())
                    } else {
                        font.clone()
                    };
                    let mut format = egui::TextFormat {
                        font_id: cell_font,
                        color: fg,
                        italics: cell.italic(),
                        ..Default::default()
                    };
                    if cell.underline() {
                        format.underline = Stroke::new(1.0_f32, fg);
                    }
                    if hover_link
                        .as_ref()
                        .is_some_and(|link| link.covers(CellPos { row, col }))
                    {
                        format.underline = Stroke::new(1.0_f32, fg);
                    }
                    let galley =
                        painter.layout_job(egui::text::LayoutJob::single_section(text, format));
                    painter.galley(text_pos, galley.clone(), fg);
                    if cell.bold() && !has_bold_font {
                        painter.galley(text_pos + Vec2::new(0.4, 0.0), galley, fg);
                    }
                }
            }
        }
        let (cursor_row, cursor_col) = screen.cursor_position();
        let cursor_rect = Rect::from_min_size(
            Pos2::new(
                content.left() + cursor_col as f32 * cell_width,
                content.top() + cursor_row as f32 * cell_height,
            ),
            Vec2::new(cell_width, cell_height),
        );
        if focused {
            ui.output_mut(|o| {
                o.ime = Some(egui::output::IMEOutput { rect, cursor_rect });
                o.mutable_text_under_cursor = true;
            });
        }
        let blink = !style.cursor_blink || self.last_input.elapsed().as_millis() % 1000 < 600;
        if !screen.hide_cursor()
            && screen.scrollback() == 0
            && cursor_row < rows
            && cursor_col < cols
        {
            if focused && blink {
                match style.cursor_shape {
                    CursorShape::Block => {
                        painter.rect_filled(
                            cursor_rect,
                            0.0,
                            style
                                .cursor
                                .gamma_multiply(style.cursor_opacity.clamp(0.0, 1.0)),
                        );
                    }
                    CursorShape::Bar => {
                        painter.rect_filled(
                            Rect::from_min_size(cursor_rect.min, Vec2::new(2.0, cell_height)),
                            0.0,
                            style.cursor,
                        );
                    }
                    CursorShape::Underline => {
                        painter.rect_filled(
                            Rect::from_min_size(
                                cursor_rect.left_bottom() - Vec2::new(0.0, 2.0),
                                Vec2::new(cell_width, 2.0),
                            ),
                            0.0,
                            style.cursor,
                        );
                    }
                }
            } else if !focused {
                painter.rect_stroke(
                    cursor_rect.shrink(0.5),
                    0.0,
                    Stroke::new(1.0_f32, style.cursor.gamma_multiply(0.45)),
                    egui::StrokeKind::Inside,
                );
            }
        }
        if !self.ime.is_empty() {
            painter.text(
                cursor_rect.min,
                egui::Align2::LEFT_TOP,
                &self.ime,
                font,
                style.foreground,
            );
        }
        if screen.scrollback() > 0 {
            painter.text(
                rect.right_top() + Vec2::new(-10.0, 5.0),
                egui::Align2::RIGHT_TOP,
                format!("↑ {} lines", screen.scrollback()),
                FontId::monospace(10.0),
                style.foreground.gamma_multiply(0.5),
            );
        }
        if style.cursor_blink && focused {
            ui.ctx().request_repaint_after(Duration::from_millis(100));
        }
        // Right-click menu with clipboard and link actions. It stays away
        // while the application owns the mouse (plain right-clicks go to
        // vim-style programs); Shift+right-click still opens it there. Once
        // open it stays open for the gesture even if modifiers change.
        let menu_allowed = self.parser.screen().mouse_protocol_mode()
            == vt100::MouseProtocolMode::None
            || ui.input(|i| i.modifiers.shift);
        if menu_allowed || response.context_menu_opened() {
            let menu_cell = self.context_cell;
            let menu_ctx = ui.ctx().clone();
            response.context_menu(|ui| {
                if let Some(cell) = menu_cell
                    && let Some(link) = self.hit_link(cell)
                {
                    if ui.button("Open Link").clicked() {
                        if let Err(e) = open::that(&link.uri) {
                            self.last_error = Some(format!("Could not open link: {e}"));
                        }
                        ui.close();
                    }
                    if ui.button("Copy Link Address").clicked() {
                        menu_ctx.copy_text(link.uri);
                        ui.close();
                    }
                    ui.separator();
                }
                if ui
                    .add_enabled(self.selection.is_some(), egui::Button::new("Copy"))
                    .clicked()
                {
                    menu_ctx.copy_text(self.selected_text());
                    ui.close();
                }
                if ui.button("Paste").clicked() {
                    // The backend delivers clipboard text as a Paste event on
                    // the next frame, which handle_input pastes when focused.
                    menu_ctx.send_viewport_cmd(egui::ViewportCommand::RequestPaste);
                    ui.close();
                }
                if ui.button("Select All").clicked() {
                    self.select_all();
                    ui.close();
                }
                ui.separator();
                if ui.button("Browse Files to Paste…").clicked() {
                    if let Some(paths) = rfd::FileDialog::new().pick_files() {
                        let cwd = self.cwd.clone();
                        let text = paths
                            .iter()
                            .map(|path| shell_quote(path, &cwd))
                            .collect::<Vec<_>>()
                            .join(" ");
                        if let Err(e) = self.paste(&text) {
                            self.last_error = Some(format!("Could not paste file paths: {e}"));
                        }
                    }
                    ui.close();
                }
                if ui.button("Clear Terminal").clicked() {
                    self.clear();
                    ui.close();
                }
            });
        }
        if let Some(link) = hover_link {
            response
                .on_hover_cursor(egui::CursorIcon::PointingHand)
                .on_hover_text(format!("Open link (Ctrl+Click):\n{}", link.uri))
        } else {
            response
        }
    }

    #[allow(clippy::too_many_arguments)] // The renderer supplies cell geometry for pointer encoding.
    fn handle_input(
        &mut self,
        ui: &mut Ui,
        response: &Response,
        content: Rect,
        cw: f32,
        ch: f32,
        focused: bool,
        style: &TerminalStyle,
    ) {
        let mouse_enabled =
            self.parser.screen().mouse_protocol_mode() != vt100::MouseProtocolMode::None;
        let events = ui.input(|i| i.events.clone());
        // egui can assign clicks to an overlaid button while retaining this
        // terminal as the drag target. Both responses then contain the pointer.
        let hovered = ui
            .ctx()
            .interaction_snapshot(|snapshot| snapshot.hovered.iter().copied().collect::<Vec<_>>());
        let covered_by_control = hovered.into_iter().any(|id| {
            id != response.id
                && ui
                    .ctx()
                    .read_response(id)
                    .is_some_and(|other| other.sense.senses_click())
        });
        let pointer_available = ui.is_enabled()
            && !egui::Popup::is_any_open(ui.ctx())
            && response.contains_pointer()
            && !covered_by_control;
        let copy_event = events
            .iter()
            .any(|event| matches!(event, egui::Event::Copy));
        let cut_event = events.iter().any(|event| matches!(event, egui::Event::Cut));
        let paste_event = events
            .iter()
            .any(|event| matches!(event, egui::Event::Paste(_)));
        // Copy the dimensions into the closure so input can mutably borrow the terminal.
        let cell_at = {
            let rows = self.size.0;
            let cols = self.size.1;
            move |pos: Pos2| cell_at_pos(content, cw, ch, rows, cols, pos)
        };
        for event in events {
            match event {
                egui::Event::Copy if focused => {
                    if ui.input(|i| i.modifiers.shift) {
                        ui.ctx().copy_text(self.selected_text());
                    } else {
                        self.send_input(b"\x03");
                    }
                }
                egui::Event::Cut if focused => self.send_input(b"\x18"),
                egui::Event::Paste(text) if focused => {
                    if let Err(e) = self.paste(&text) {
                        self.last_error = Some(e.to_string());
                    }
                }
                egui::Event::Text(text) if focused && self.ime.is_empty() => {
                    let modifiers = ui.input(|i| i.modifiers);
                    // AltGr text is emitted as Text even when Ctrl+Alt are held.
                    if modifiers.alt && !modifiers.ctrl {
                        self.send_input(b"\x1b");
                    }
                    self.send_input(text.as_bytes());
                }
                egui::Event::Ime(egui::ImeEvent::Preedit(text)) if focused => self.ime = text,
                egui::Event::Ime(egui::ImeEvent::Commit(text)) if focused => {
                    self.ime.clear();
                    self.send_input(text.as_bytes());
                }
                egui::Event::Ime(egui::ImeEvent::Disabled) => self.ime.clear(),
                egui::Event::Key {
                    key,
                    pressed: true,
                    modifiers,
                    ..
                } if focused => {
                    if modifiers.ctrl
                        && ((key == Key::C && copy_event)
                            || (key == Key::X && cut_event)
                            || (key == Key::V && paste_event))
                    {
                        continue;
                    }
                    if modifiers.ctrl && modifiers.shift && key == Key::C {
                        ui.ctx().copy_text(self.selected_text());
                    } else if modifiers.ctrl && modifiers.shift && key == Key::A {
                        self.select_all();
                    } else if modifiers.shift
                        && matches!(key, Key::PageUp | Key::PageDown | Key::Home | Key::End)
                    {
                        match key {
                            Key::Home => self.scroll_top(),
                            Key::End => self.scroll_bottom(),
                            Key::PageUp => self.scroll_by(self.size.0 as isize - 1),
                            _ => self.scroll_by(-(self.size.0 as isize - 1)),
                        }
                    } else if !(modifiers.ctrl && modifiers.shift && key == Key::V)
                        && let Some(bytes) =
                            encode_key(key, modifiers, self.parser.screen().application_cursor())
                    {
                        self.selection = None;
                        self.send_input(&bytes);
                    }
                }
                egui::Event::MouseWheel {
                    unit,
                    delta,
                    modifiers,
                } if pointer_available && response.hovered() => {
                    if modifiers.ctrl {
                        continue;
                    }
                    let lines = match unit {
                        egui::MouseWheelUnit::Point => delta.y / ch,
                        egui::MouseWheelUnit::Line => delta.y,
                        egui::MouseWheelUnit::Page => delta.y * self.size.0 as f32,
                    };
                    if mouse_enabled && !modifiers.shift {
                        if let Some(pos) = ui.input(|i| i.pointer.hover_pos()) {
                            for _ in 0..lines.abs().ceil().clamp(1.0, 20.0) as usize {
                                self.send_mouse(
                                    if lines > 0.0 { 64 } else { 65 },
                                    cell_at(pos),
                                    true,
                                    modifiers,
                                );
                            }
                        }
                    } else {
                        self.scroll_fraction += lines * style.scroll_multiplier.clamp(0.1, 10.0);
                        let whole = self.scroll_fraction.trunc() as isize;
                        self.scroll_fraction -= whole as f32;
                        self.scroll_by(whole);
                    }
                }
                egui::Event::PointerButton {
                    pos,
                    button,
                    pressed,
                    modifiers,
                } if (pointer_available && content.contains(pos))
                    || (!pressed && self.mouse_button.is_some()) =>
                {
                    let button_code = match button {
                        egui::PointerButton::Primary => 0,
                        egui::PointerButton::Middle => 1,
                        egui::PointerButton::Secondary => 2,
                        _ => continue,
                    };
                    if mouse_enabled && !modifiers.shift {
                        if pressed {
                            self.mouse_button = Some(button_code);
                        }
                        if pressed
                            || self.parser.screen().mouse_protocol_mode()
                                != vt100::MouseProtocolMode::Press
                        {
                            self.send_mouse(button_code, cell_at(pos), pressed, modifiers);
                        }
                        if !pressed {
                            self.mouse_button = None;
                        }
                    } else if button == egui::PointerButton::Secondary && pressed {
                        // Right-click opens the terminal menu (attached in
                        // show() under the same condition). The application
                        // owns the button while its mouse protocol is active.
                        if !mouse_enabled || modifiers.shift {
                            self.context_cell = Some(cell_at(pos));
                        }
                    } else if button == egui::PointerButton::Primary {
                        if pressed {
                            let cell = cell_at(pos);
                            // Ctrl+Click opens the link under the cursor in the
                            // host browser (which also works for remote panes).
                            // The application owns the click while its mouse
                            // protocol is active, so links yield to it there.
                            if modifiers.ctrl
                                && !modifiers.shift
                                && let Some(link) = self.hit_link(cell)
                            {
                                if let Err(e) = open::that(&link.uri) {
                                    self.last_error = Some(format!("Could not open link: {e}"));
                                }
                            } else {
                                self.selection = Some((cell, cell));
                            }
                        } else if style.copy_on_select {
                            ui.ctx().copy_text(self.selected_text());
                        }
                    }
                }
                egui::Event::PointerMoved(pos)
                    if content.contains(pos)
                        && (pointer_available || self.mouse_button.is_some()) =>
                {
                    let modifiers = ui.input(|i| i.modifiers);
                    let cell = cell_at(pos);
                    if mouse_enabled && !modifiers.shift {
                        let mode = self.parser.screen().mouse_protocol_mode();
                        if self.last_mouse != Some(cell)
                            && (mode == vt100::MouseProtocolMode::AnyMotion
                                || (mode == vt100::MouseProtocolMode::ButtonMotion
                                    && self.mouse_button.is_some()))
                        {
                            self.send_mouse(
                                32 + self.mouse_button.unwrap_or(3),
                                cell,
                                true,
                                modifiers,
                            );
                        }
                        self.last_mouse = Some(cell);
                    } else if ui.input(|i| i.pointer.primary_down())
                        && let Some((start, _)) = self.selection
                    {
                        self.selection = Some((start, cell));
                    }
                }
                _ => {}
            }
        }
        if response.double_clicked()
            && (!mouse_enabled || ui.input(|i| i.modifiers.shift))
            && let Some(pos) = response.interact_pointer_pos()
        {
            let point = cell_at(pos);
            let mut start = point.col;
            let mut end = point.col;
            let is_word = |col| {
                self.parser.screen().cell(point.row, col).is_some_and(|c| {
                    c.contents()
                        .chars()
                        .any(|ch| ch.is_alphanumeric() || "_./:~@-%+".contains(ch))
                })
            };
            while start > 0 && is_word(start - 1) {
                start -= 1;
            }
            while end + 1 < self.size.1 && is_word(end + 1) {
                end += 1;
            }
            self.selection = Some((
                CellPos {
                    row: point.row,
                    col: start,
                },
                CellPos {
                    row: point.row,
                    col: end,
                },
            ));
            if style.copy_on_select {
                ui.ctx().copy_text(self.selected_text());
            }
        }
        if response.triple_clicked()
            && let Some(pos) = response.interact_pointer_pos()
        {
            let row = cell_at(pos).row;
            self.selection = Some((
                CellPos { row, col: 0 },
                CellPos {
                    row,
                    col: self.size.1 - 1,
                },
            ));
        }
    }

    fn send_input(&mut self, bytes: &[u8]) {
        if let Err(e) = self.write(bytes) {
            self.last_error = Some(e.to_string());
        }
    }
    fn scroll_by(&mut self, delta: isize) {
        if delta == 0 {
            return;
        }
        // Scrolling shifts visible rows under recorded OSC 8 spans.
        self.protocol.drop_links();
        let offset = self
            .parser
            .screen()
            .scrollback()
            .saturating_add_signed(delta);
        self.parser.set_scrollback(offset);
        self.selection = None;
    }
    fn send_mouse(&mut self, button: u8, pos: CellPos, pressed: bool, modifiers: Modifiers) {
        let mut code = button
            + if modifiers.shift { 4 } else { 0 }
            + if modifiers.alt { 8 } else { 0 }
            + if modifiers.ctrl { 16 } else { 0 };
        let bytes = match self.parser.screen().mouse_protocol_encoding() {
            vt100::MouseProtocolEncoding::Sgr => format!(
                "\x1b[<{};{};{}{}",
                code,
                pos.col + 1,
                pos.row + 1,
                if pressed { 'M' } else { 'm' }
            )
            .into_bytes(),
            vt100::MouseProtocolEncoding::Utf8 => {
                if !pressed {
                    code = 3;
                }
                format!(
                    "\x1b[M{}{}{}",
                    char::from_u32(code as u32 + 32).unwrap_or(' '),
                    char::from_u32(pos.col as u32 + 33).unwrap_or(' '),
                    char::from_u32(pos.row as u32 + 33).unwrap_or(' ')
                )
                .into_bytes()
            }
            _ => {
                if !pressed {
                    code = 3;
                }
                if pos.col > 222 || pos.row > 222 {
                    return;
                }
                vec![
                    27,
                    b'[',
                    b'M',
                    code + 32,
                    pos.col as u8 + 33,
                    pos.row as u8 + 33,
                ]
            }
        };
        self.send_input(&bytes);
    }
}

impl Drop for Terminal {
    fn drop(&mut self) {
        self.detach();
    }
}

fn ansi_color(
    color: vt100::Color,
    default: Color32,
    palette: &[Color32; 16],
    bright: bool,
) -> Color32 {
    match color {
        vt100::Color::Default => default,
        vt100::Color::Rgb(r, g, b) => Color32::from_rgb(r, g, b),
        vt100::Color::Idx(mut index) => {
            if bright && index < 8 {
                index += 8;
            }
            match index {
                0..=15 => palette[index as usize],
                16..=231 => {
                    let n = index - 16;
                    let level = |v| if v == 0 { 0 } else { 55 + 40 * v };
                    Color32::from_rgb(level(n / 36), level((n / 6) % 6), level(n % 6))
                }
                _ => Color32::from_gray(8 + (index - 232) * 10),
            }
        }
    }
}

/// xterm input encoding. Printable text comes from egui's separate Text event.
fn encode_key(key: Key, modifiers: Modifiers, application_cursor: bool) -> Option<Vec<u8>> {
    let parameter =
        1 + u8::from(modifiers.shift) + 2 * u8::from(modifiers.alt) + 4 * u8::from(modifiers.ctrl);
    let arrow = match key {
        Key::ArrowUp => Some('A'),
        Key::ArrowDown => Some('B'),
        Key::ArrowRight => Some('C'),
        Key::ArrowLeft => Some('D'),
        Key::Home => Some('H'),
        Key::End => Some('F'),
        _ => None,
    };
    if let Some(code) = arrow {
        return Some(
            if parameter > 1 {
                format!("\x1b[1;{parameter}{code}")
            } else if application_cursor {
                format!("\x1bO{code}")
            } else {
                format!("\x1b[{code}")
            }
            .into_bytes(),
        );
    }
    let numbered = match key {
        Key::Insert => Some(2),
        Key::Delete => Some(3),
        Key::PageUp => Some(5),
        Key::PageDown => Some(6),
        Key::F5 => Some(15),
        Key::F6 => Some(17),
        Key::F7 => Some(18),
        Key::F8 => Some(19),
        Key::F9 => Some(20),
        Key::F10 => Some(21),
        Key::F11 => Some(23),
        Key::F12 => Some(24),
        _ => None,
    };
    if let Some(code) = numbered {
        return Some(
            if parameter > 1 {
                format!("\x1b[{code};{parameter}~")
            } else {
                format!("\x1b[{code}~")
            }
            .into_bytes(),
        );
    }
    let function = match key {
        Key::F1 => Some('P'),
        Key::F2 => Some('Q'),
        Key::F3 => Some('R'),
        Key::F4 => Some('S'),
        _ => None,
    };
    if let Some(code) = function {
        return Some(
            if parameter > 1 {
                format!("\x1b[1;{parameter}{code}")
            } else {
                format!("\x1bO{code}")
            }
            .into_bytes(),
        );
    }
    let byte = match key {
        Key::Enter => b'\r',
        Key::Escape => 27,
        Key::Backspace => {
            if modifiers.ctrl {
                8
            } else {
                127
            }
        }
        Key::Tab if modifiers.shift => return Some(b"\x1b[Z".to_vec()),
        Key::Tab => b'\t',
        _ if modifiers.ctrl && !modifiers.alt => match key {
            Key::A => 1,
            Key::B => 2,
            Key::C => 3,
            Key::D => 4,
            Key::E => 5,
            Key::F => 6,
            Key::G => 7,
            Key::H => 8,
            Key::I => 9,
            Key::J => 10,
            Key::K => 11,
            Key::L => 12,
            Key::M => 13,
            Key::N => 14,
            Key::O => 15,
            Key::P => 16,
            Key::Q => 17,
            Key::R => 18,
            Key::S => 19,
            Key::T => 20,
            Key::U => 21,
            Key::V => 22,
            Key::W => 23,
            Key::X => 24,
            Key::Y => 25,
            Key::Z => 26,
            Key::OpenBracket | Key::Num3 => 27,
            Key::Backslash | Key::Num4 => 28,
            Key::CloseBracket | Key::Num5 => 29,
            Key::Num6 => 30,
            Key::Minus | Key::Slash | Key::Num7 => 31,
            Key::Space | Key::Num2 => 0,
            Key::Num8 => 127,
            _ => return None,
        },
        _ => return None,
    };
    Some(if modifiers.alt {
        vec![27, byte]
    } else {
        vec![byte]
    })
}

fn shell_basename(shell: &str) -> String {
    shell
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or(shell)
        .trim_end_matches(".exe")
        .to_ascii_lowercase()
}

/// Shell-style tokenization only; tokens are passed directly to OpenSSH, never a local shell.
pub fn split_command(raw: &str) -> Result<Vec<String>> {
    let mut tokens = Vec::new();
    let mut current = String::new();
    let mut quote = None;
    let mut escaped = false;
    let mut started = false;
    let mut chars = raw.chars().peekable();
    while let Some(ch) = chars.next() {
        if escaped {
            current.push(ch);
            escaped = false;
            started = true;
        } else if ch == '\\' && quote != Some('\'') {
            let windows_path =
                current.as_bytes().get(1) == Some(&b':') || current.starts_with("\\\\");
            if windows_path {
                current.push(ch);
                started = true;
            } else if current.is_empty() && chars.peek() == Some(&'\\') {
                current.push(ch);
                current.push(chars.next().unwrap());
                started = true;
            } else {
                escaped = true;
                started = true;
            }
        } else if let Some(q) = quote {
            if ch == q {
                quote = None;
            } else {
                current.push(ch);
            }
        } else if ch == '\'' || ch == '"' {
            quote = Some(ch);
            started = true;
        } else if ch.is_whitespace() {
            if started {
                tokens.push(std::mem::take(&mut current));
                started = false;
            }
        } else {
            current.push(ch);
            started = true;
        }
    }
    if escaped || quote.is_some() {
        bail!("Unclosed quote or escape in command");
    }
    if started {
        tokens.push(current);
    }
    Ok(tokens)
}

#[derive(Debug)]
struct SshPlan {
    transport: Vec<String>,
    remote: Vec<String>,
}

pub fn validate_ssh(raw: &str) -> Result<()> {
    parse_ssh(raw).map(|_| ())
}

/// Queue an explicit close even if a pane never had a successfully spawned transport.
pub fn close_session(kind: &SessionKind, transcript_dir: &Path) -> Result<()> {
    let session = match kind {
        SessionKind::Local => return Ok(()),
        SessionKind::LocalTmux { session } | SessionKind::Ssh { session, .. } => session,
    };
    validate_session(session)?;
    let directory = transcript_dir.join("pending-close");
    queue_cleanup(&directory, session, kind)?;
    retry_cleanup(directory);
    Ok(())
}

/// Service persisted close requests even when no workspaces or live terminals remain.
pub fn service_cleanup(transcript_dir: &Path) {
    static LAST: std::sync::OnceLock<std::sync::Mutex<Option<Instant>>> =
        std::sync::OnceLock::new();
    let Ok(mut last) = LAST.get_or_init(|| std::sync::Mutex::new(None)).lock() else {
        return;
    };
    if last.is_none_or(|at| at.elapsed() >= Duration::from_secs(30)) {
        *last = Some(Instant::now());
        retry_cleanup(transcript_dir.join("pending-close"));
    }
}

pub fn read_history(path: &Path) -> String {
    plain_text(&read_tail(path, 1024 * 1024))
}

fn parse_ssh(raw: &str) -> Result<SshPlan> {
    let mut tokens = split_command(raw)?;
    if tokens.len() == 1 && !tokens[0].starts_with('-') && shell_basename(&tokens[0]) != "ssh" {
        tokens.insert(0, "ssh".to_owned());
    }
    if tokens.first().is_none_or(|s| shell_basename(s) != "ssh") {
        bail!("Use an SSH command such as ssh user@host or a bare host name");
    }
    let valued = [
        "-p", "-i", "-J", "-l", "-o", "-F", "-L", "-R", "-D", "-w", "-b", "-c", "-m", "-S", "-B",
        "-E", "-e",
    ];
    let mut index = 1;
    while index < tokens.len() {
        let token = &tokens[index];
        if token == "--" {
            tokens.remove(index);
            break;
        }
        if !token.starts_with('-') {
            break;
        }
        let flag = token.get(..2).unwrap_or(token);
        if ["-W", "-O", "-Q"].contains(&flag) {
            bail!("SSH option {flag} does not start an interactive session");
        }
        if valued.contains(&flag) {
            if token.len() == 2 {
                index += 1;
                if index >= tokens.len() {
                    bail!("SSH option {flag} requires a value");
                }
            }
        } else if token.chars().skip(1).any(|c| "nNfVG".contains(c)) {
            bail!("SSH option {token} does not start an interactive session");
        }
        index += 1;
    }
    if index >= tokens.len() || tokens[index].is_empty() || tokens[index].starts_with('-') {
        bail!("SSH host is missing");
    }
    Ok(SshPlan {
        transport: tokens[1..=index].to_vec(),
        remote: tokens[index + 1..].to_vec(),
    })
}

fn quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

/// Quote a file path dropped from the OS for pasting into the pane: paths
/// under the pane directory paste as `./relative` form, and quoting only
/// applies when the text contains whitespace or shell metacharacters.
pub(crate) fn shell_quote(path: &Path, cwd: &Path) -> String {
    let text = match path.strip_prefix(cwd) {
        Ok(relative) if !relative.as_os_str().is_empty() => {
            format!("./{}", relative.display())
        }
        _ => path.display().to_string(),
    };
    let safe = !text.is_empty()
        && text
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"@%+=:,./-_".contains(&b));
    if safe {
        return text;
    }
    #[cfg(unix)]
    return quote(&text);
    #[cfg(windows)]
    return format!("\"{}\"", text.replace('"', "'"));
}
fn validate_session(session: &str) -> Result<()> {
    if session.is_empty()
        || session.starts_with('-')
        || !session
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
    {
        bail!("Invalid persistent session name");
    }
    Ok(())
}

const TMUX_CLOSE_KEY: &str = "\x1b[99999;1~";

fn tmux_bootstrap(
    session: &str,
    local: bool,
    directory: Option<&Path>,
    inner: &str,
    scrollback: usize,
) -> Result<String> {
    validate_session(session)?;
    let socket = if local { "bmux-local-v1" } else { "bmux-v1" };
    let directory = directory
        .map(|p| format!(" -c {}", quote(&p.to_string_lossy())))
        .unwrap_or_default();
    let inner = if inner.is_empty() {
        String::new()
    } else {
        format!(" {}", quote(inner))
    };
    let capture = format!(
        "tmux -L {socket} capture-pane -p -e -S -{} -E -1 -t {} 2>/dev/null || true; ",
        scrollback.clamp(100, 1_000_000),
        quote(&format!("={session}:"))
    );
    Ok(format!(
        "unset TMUX TMUX_PANE; command -v tmux >/dev/null 2>&1 || {{ printf 'bmux: install tmux 3.2 or newer {} to keep sessions running.\\r\\n' >&2; exit 78; }}; {capture}exec tmux -L {socket} -f /dev/null start-server \\; set-option -ga update-environment ' WAYLAND_DISPLAY XDG_RUNTIME_DIR XDG_CURRENT_DESKTOP XDG_SESSION_TYPE BROWSER' \\; set-option -g history-limit {} \\; set-option -g default-terminal tmux-256color \\; set-option -g status off \\; set-option -g mouse on \\; set-option -g prefix None \\; set-option -g prefix2 None \\; set-option -g user-keys[0] {} \\; bind-key -n User0 kill-session \\; set-option -g terminal-overrides '*:smcup@:rmcup@' \\; set-option -g destroy-unattached off \\; set-option -g window-size latest \\; new-session -A -D -s {}{directory}{inner}",
        if local {
            if cfg!(windows) {
                "in WSL"
            } else {
                "on this computer"
            }
        } else {
            "on the SSH host"
        },
        scrollback.clamp(100, 1_000_000),
        quote(TMUX_CLOSE_KEY),
        quote(session)
    ))
}

const POWERSHELL_PROMPT: &str = "$global:__bmuxOriginalPrompt = $function:prompt; function global:prompt { try { $uri = [Uri]::new($executionContext.SessionState.Path.CurrentLocation.ProviderPath); [Console]::Write([char]27 + ']7;' + $uri.AbsoluteUri + [char]7) } catch {}; if ($global:__bmuxOriginalPrompt) { & $global:__bmuxOriginalPrompt } else { 'PS ' + $PWD + '> ' } }";

fn build_command(spec: &SpawnSpec) -> Result<CommandBuilder> {
    match &spec.kind {
        SessionKind::Local => {
            let detected = default_shell();
            let shell = if spec.shell.trim().is_empty() {
                &detected
            } else {
                &spec.shell
            };
            let mut command = CommandBuilder::new(shell);
            let base = shell_basename(shell);
            if let Some(raw) = &spec.command {
                if base == "cmd" {
                    command.args(["/C", raw]);
                } else if base == "powershell" || base == "pwsh" {
                    command.args(["-NoLogo", "-Command", raw]);
                } else {
                    command.args(["-lc", raw]);
                }
            } else if base == "cmd" {
                command.arg("/Q");
                command.env("PROMPT", "$E]7;file://localhost/$P$E\\$P$G");
            } else if base == "powershell" || base == "pwsh" {
                command.args(["-NoLogo", "-NoExit", "-Command", POWERSHELL_PROMPT]);
            } else {
                command.arg("-l");
            }
            Ok(command)
        }
        SessionKind::LocalTmux { session } => {
            let detected = default_shell();
            let shell = if cfg!(windows) {
                "/bin/bash"
            } else if spec.shell.is_empty() {
                &detected
            } else {
                &spec.shell
            };
            let inner = spec
                .command
                .clone()
                .unwrap_or_else(|| format!("{} -l", quote(shell)));
            let bootstrap = tmux_bootstrap(
                session,
                true,
                if cfg!(windows) { None } else { Some(&spec.cwd) },
                &inner,
                spec.scrollback_lines,
            )?;
            if cfg!(windows) {
                let mut command = CommandBuilder::new("wsl.exe");
                command.arg("--cd");
                command.arg(&spec.cwd);
                command.args(["--exec", "sh", "-lc", &bootstrap]);
                Ok(command)
            } else {
                let mut command = CommandBuilder::new("/bin/sh");
                command.args(["-c", &bootstrap]);
                Ok(command)
            }
        }
        SessionKind::Ssh {
            command: raw,
            session,
        } => {
            let plan = parse_ssh(raw)?;
            let inner = plan
                .remote
                .iter()
                .map(|a| quote(a))
                .collect::<Vec<_>>()
                .join(" ");
            let bootstrap = tmux_bootstrap(session, false, None, &inner, spec.scrollback_lines)?;
            let mut command = CommandBuilder::new("ssh");
            command.args([
                "-tt",
                "-o",
                "ServerAliveInterval=15",
                "-o",
                "ServerAliveCountMax=3",
                "-o",
                "StdinNull=no",
                "-o",
                "SessionType=default",
                "-o",
                "RemoteCommand=none",
            ]);
            command.args(plan.transport);
            command.arg(bootstrap);
            Ok(command)
        }
    }
}

fn local_tmux_command() -> Command {
    if cfg!(windows) {
        let mut command = Command::new("wsl.exe");
        command.args(["--exec", "tmux"]);
        command
    } else {
        Command::new("tmux")
    }
}

fn cleanup_command(kind: &SessionKind) -> Result<Option<Command>> {
    match kind {
        SessionKind::Local => Ok(None),
        SessionKind::LocalTmux { session } => {
            validate_session(session)?;
            let target = quote(&format!("={session}"));
            let script = format!(
                "command -v tmux >/dev/null 2>&1 || exit 78; if tmux -L bmux-local-v1 has-session -t {target} 2>/dev/null; then tmux -L bmux-local-v1 kill-session -t {target}; else exit 0; fi"
            );
            let mut command = if cfg!(windows) {
                let mut command = Command::new("wsl.exe");
                command.args(["--exec", "sh", "-c"]);
                command
            } else {
                let mut command = Command::new("/bin/sh");
                command.arg("-c");
                command
            };
            command.arg(script);
            Ok(Some(command))
        }
        SessionKind::Ssh {
            command: raw,
            session,
        } => {
            validate_session(session)?;
            let plan = parse_ssh(raw)?;
            let mut command = Command::new("ssh");
            command.args([
                "-o",
                "BatchMode=yes",
                "-o",
                "ConnectTimeout=8",
                "-o",
                "ServerAliveInterval=5",
                "-o",
                "ServerAliveCountMax=1",
                "-o",
                "RemoteCommand=none",
                "-T",
            ]);
            command.args(plan.transport);
            command.arg(format!("command -v tmux >/dev/null 2>&1 || exit 78; if tmux -L bmux-v1 has-session -t {} 2>/dev/null; then tmux -L bmux-v1 kill-session -t {}; else exit 0; fi", quote(&format!("={session}")), quote(&format!("={session}"))));
            Ok(Some(command))
        }
    }
}

struct Recorder {
    path: PathBuf,
    file: File,
    bytes: u64,
    max_bytes: u64,
}
impl Recorder {
    fn new(path: PathBuf, max_bytes: u64) -> Result<Self> {
        let file = OpenOptions::new().create(true).append(true).open(&path)?;
        let bytes = file.metadata()?.len();
        let mut this = Self {
            path,
            file,
            bytes,
            max_bytes,
        };
        if this.bytes >= max_bytes {
            this.rotate()?;
        }
        Ok(this)
    }
    fn append(&mut self, data: &[u8]) -> Result<()> {
        if self.bytes > 0 && self.bytes + data.len() as u64 > self.max_bytes {
            self.rotate()?;
        }
        self.file.write_all(data)?;
        self.bytes += data.len() as u64;
        Ok(())
    }
    fn rotate(&mut self) -> Result<()> {
        self.file.flush()?;
        let archive = self.path.with_extension("ts.1");
        // Copy/truncate works on Windows while this recorder holds the active file.
        fs::copy(&self.path, &archive)?;
        self.file.set_len(0)?;
        self.bytes = 0;
        Ok(())
    }
}

fn read_tail(path: &Path, limit: usize) -> Vec<u8> {
    let Ok(mut file) = File::open(path) else {
        return Vec::new();
    };
    let Ok(size) = file.metadata().map(|m| m.len()) else {
        return Vec::new();
    };
    let start = size.saturating_sub(limit as u64);
    let _ = file.seek(SeekFrom::Start(start));
    let mut bytes = Vec::new();
    let _ = file.take(limit as u64).read_to_end(&mut bytes);
    if start > 0 {
        // Skip a partial UTF-8 prefix and an incomplete escape record where possible.
        let skip = bytes.iter().take_while(|b| **b & 0xc0 == 0x80).count();
        bytes.drain(..skip);
    }
    bytes
}

/// Preserve line-oriented transcript text without allocating a terminal-sized scrollback grid.
pub fn plain_text(bytes: &[u8]) -> String {
    let mut out = Vec::with_capacity(bytes.len());
    let mut state = 0;
    for &b in bytes {
        match state {
            0 => match b {
                27 => state = 1,
                b'\r' => {}
                8 => {
                    out.pop();
                }
                b'\n' | b'\t' | 32..=255 => out.push(b),
                _ => {}
            },
            1 => match b {
                b'[' => state = 2,
                b']' | b'P' | b'_' | b'^' => state = 3,
                b'(' | b')' | b'*' | b'+' => state = 5,
                _ => state = 0,
            },
            2 => {
                if (0x40..=0x7e).contains(&b) {
                    state = 0;
                }
            }
            3 => {
                if b == 7 {
                    state = 0;
                } else if b == 27 {
                    state = 4;
                }
            }
            4 => state = if b == b'\\' { 0 } else { 3 },
            _ => state = 0,
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Incremental handling of queries and OSC metadata not exposed by vt100.
struct Protocol {
    state: u8,
    data: Vec<u8>,
    cwd: Option<PathBuf>,
    focus_reporting: bool,
    colors: [Color32; 3],
    palette: [Color32; 16],
    active_link: Option<(String, u16, u16)>,
    links: Vec<Hyperlink>,
}
impl Default for Protocol {
    fn default() -> Self {
        let style = TerminalStyle::default();
        Self {
            state: 0,
            data: Vec::new(),
            cwd: None,
            focus_reporting: false,
            colors: [style.foreground, style.background, style.cursor],
            palette: style.palette,
            active_link: None,
            links: Vec::new(),
        }
    }
}
impl Protocol {
    fn process(&mut self, bytes: &[u8], parser: &mut vt100::Parser) -> Vec<Vec<u8>> {
        let mut replies = Vec::new();
        let mut parsed = 0;
        for (index, &byte) in bytes.iter().enumerate() {
            match self.state {
                // Feed plain text to the screen as it arrives so cursor queries
                // (OSC 8 attribution, reports) observe the current position
                // even when the sequence they belong to completes later.
                0 if byte == 27 => {
                    parser.process(&bytes[parsed..index]);
                    parsed = index;
                    self.state = 1;
                }
                1 => {
                    self.data.clear();
                    self.state = match byte {
                        b'[' => 2,
                        b']' => 3,
                        b'Z' => {
                            replies.push(b"\x1b[?1;2c".to_vec());
                            0
                        }
                        _ => 0,
                    };
                }
                2 => {
                    self.data.push(byte);
                    if (0x40..=0x7e).contains(&byte) {
                        parser.process(&bytes[parsed..=index]);
                        parsed = index + 1;
                        let screen = parser.screen();
                        let query = String::from_utf8_lossy(&self.data);
                        match query.as_ref() {
                            "5n" => replies.push(b"\x1b[0n".to_vec()),
                            "6n" | "?6n" => {
                                let (row, col) = screen.cursor_position();
                                replies.push(
                                    format!(
                                        "\x1b[{}{};{}R",
                                        if query.starts_with('?') { "?" } else { "" },
                                        row + 1,
                                        col + 1
                                    )
                                    .into_bytes(),
                                );
                            }
                            "c" | "0c" => replies.push(b"\x1b[?1;2c".to_vec()),
                            ">c" | ">0c" => replies.push(b"\x1b[>0;1;0c".to_vec()),
                            ">q" | ">0q" => replies.push(
                                format!("\x1bP>|bmux {}\x1b\\", env!("CARGO_PKG_VERSION"))
                                    .into_bytes(),
                            ),
                            "18t" => {
                                let (rows, cols) = screen.size();
                                replies.push(format!("\x1b[8;{rows};{cols}t").into_bytes());
                            }
                            "?1004h" => self.focus_reporting = true,
                            "?1004l" => self.focus_reporting = false,
                            _ => {}
                        }
                        self.state = 0;
                    }
                }
                3 if byte == 7 => {
                    let cursor = parser.screen().cursor_position();
                    self.finish_osc(&mut replies, cursor);
                    self.state = 0;
                }
                3 if byte == 27 => self.state = 4,
                3 => self.data.push(byte),
                4 => {
                    if byte == b'\\' {
                        let cursor = parser.screen().cursor_position();
                        self.finish_osc(&mut replies, cursor);
                    }
                    self.state = 0;
                }
                _ => {}
            }
            if self.data.len() > 8192 {
                self.data.clear();
                self.state = 0;
            }
        }
        parser.process(&bytes[parsed..]);
        replies
    }
    fn finish_osc(&mut self, replies: &mut Vec<Vec<u8>>, cursor: (u16, u16)) {
        let text = String::from_utf8_lossy(&self.data);
        if let Some(url) = text.strip_prefix("7;file://") {
            if let Some(slash) = url.find('/') {
                let decoded = percent_decode(&url[slash..]);
                #[cfg(windows)]
                let decoded = if decoded.as_bytes().get(2) == Some(&b':') {
                    decoded[1..].to_owned()
                } else {
                    decoded
                };
                if !decoded.contains('\0') {
                    self.cwd = Some(PathBuf::from(decoded));
                }
            }
        } else if let Some(rest) = text.strip_prefix("8;") {
            // OSC 8 hyperlink: `8;params;URI` opens, `8;;` closes. The URI
            // itself may contain `;`, so only the first separator counts.
            if let Some((_, uri)) = rest.split_once(';') {
                if uri.is_empty() {
                    if let Some((open_uri, row, col)) = self.active_link.take() {
                        self.push_link(open_uri, row, col, cursor.0, cursor.1);
                    }
                } else if let Some((open_uri, row, col)) =
                    self.active_link
                        .replace((uri.to_owned(), cursor.0, cursor.1))
                {
                    // A new link without a closing marker ends the previous one.
                    self.push_link(open_uri, row, col, cursor.0, cursor.1);
                }
            }
        } else {
            let parts: Vec<_> = text.split(';').collect();
            let answer = match parts.as_slice() {
                ["10", "?"] => Some(("10".to_owned(), self.colors[0])),
                ["11", "?"] => Some(("11".to_owned(), self.colors[1])),
                ["12", "?"] => Some(("12".to_owned(), self.colors[2])),
                ["4", index, "?"] => index.parse::<u8>().ok().map(|index| {
                    (
                        format!("4;{index}"),
                        ansi_color(
                            vt100::Color::Idx(index),
                            self.colors[0],
                            &self.palette,
                            false,
                        ),
                    )
                }),
                _ => None,
            };
            if let Some((code, color)) = answer {
                replies.push(
                    format!(
                        "\x1b]{code};rgb:{:04x}/{:04x}/{:04x}\x1b\\",
                        color.r() as u16 * 257,
                        color.g() as u16 * 257,
                        color.b() as u16 * 257
                    )
                    .into_bytes(),
                );
            }
        }
    }
    fn push_link(
        &mut self,
        uri: String,
        start_row: u16,
        start_col: u16,
        end_row: u16,
        end_col: u16,
    ) {
        self.links.push(Hyperlink {
            uri,
            start_row,
            start_col,
            end_row,
            end_col,
        });
        if self.links.len() > 256 {
            self.links.drain(..self.links.len() - 256);
        }
    }
    fn drop_links(&mut self) {
        self.links.clear();
        self.active_link = None;
    }
}
fn percent_decode(text: &str) -> String {
    let mut out = Vec::new();
    let mut bytes = text.bytes();
    while let Some(byte) = bytes.next() {
        if byte == b'%' {
            let pair: Vec<_> = bytes.by_ref().take(2).collect();
            if pair.len() == 2
                && let Ok(hex) = std::str::from_utf8(&pair)
                && let Ok(value) = u8::from_str_radix(hex, 16)
            {
                out.push(value);
            } else {
                out.push(byte);
                out.extend(pair);
            }
        } else {
            out.push(byte);
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

#[cfg(target_os = "linux")]
fn linux_foreground_command(mut pid: u32) -> String {
    for _ in 0..12 {
        let Ok(children) = fs::read_to_string(format!("/proc/{pid}/task/{pid}/children")) else {
            break;
        };
        let Some(child) = children
            .split_whitespace()
            .last()
            .and_then(|p| p.parse().ok())
        else {
            break;
        };
        pid = child;
    }
    fs::read_to_string(format!("/proc/{pid}/comm"))
        .unwrap_or_default()
        .trim()
        .to_owned()
}

fn queue_cleanup(directory: &Path, pane: &str, kind: &SessionKind) -> Result<()> {
    fs::create_dir_all(directory)?;
    let bytes = match kind {
        SessionKind::Local => return Ok(()),
        SessionKind::LocalTmux { session } => format!("local\0{session}"),
        SessionKind::Ssh { command, session } => format!("ssh\0{session}\0{command}"),
    };
    let path = directory.join(pane);
    let temporary = directory.join(format!("{pane}.tmp"));
    let mut file = File::create(&temporary)?;
    file.write_all(bytes.as_bytes())?;
    file.sync_all()?;
    drop(file);
    fs::rename(temporary, path)?;
    Ok(())
}

fn retry_cleanup(directory: PathBuf) {
    // At most one cleanup sweep per process; startup can restore many panes at once.
    static RUNNING: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
    if !directory.is_dir() || RUNNING.swap(true, std::sync::atomic::Ordering::AcqRel) {
        return;
    }
    thread::spawn(move || {
        struct Reset;
        impl Drop for Reset {
            fn drop(&mut self) {
                RUNNING.store(false, std::sync::atomic::Ordering::Release);
            }
        }
        let _reset = Reset;
        let Ok(entries) = fs::read_dir(directory) else {
            return;
        };
        for entry in entries.flatten() {
            if entry.path().extension().is_some() {
                continue;
            }
            let Ok(raw) = fs::read_to_string(entry.path()) else {
                continue;
            };
            let parts: Vec<_> = raw.splitn(3, '\0').collect();
            let kind = match parts.as_slice() {
                ["local", session] => SessionKind::LocalTmux {
                    session: session.to_string(),
                },
                ["ssh", session, command] => SessionKind::Ssh {
                    session: session.to_string(),
                    command: command.to_string(),
                },
                _ => continue,
            };
            let Ok(Some(mut command)) = cleanup_command(&kind) else {
                continue;
            };
            hide_console(&mut command);
            let Ok(mut child) = command
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
            else {
                continue;
            };
            let deadline = Instant::now() + Duration::from_secs(15);
            loop {
                match child.try_wait() {
                    Ok(Some(status)) => {
                        if status.success() {
                            let _ = fs::remove_file(entry.path());
                        }
                        break;
                    }
                    Err(_) => break,
                    _ if Instant::now() >= deadline => {
                        let _ = child.kill();
                        let _ = child.wait();
                        break;
                    }
                    _ => thread::sleep(Duration::from_millis(100)),
                }
            }
        }
    });
}

fn hide_console(command: &mut Command) {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000);
    }
    #[cfg(not(windows))]
    let _ = command;
}

pub fn default_shell() -> String {
    if cfg!(windows) {
        if let Some(path) = std::env::var_os("PATH") {
            for directory in std::env::split_paths(&path) {
                if directory.join("pwsh.exe").is_file() {
                    return "pwsh.exe".to_owned();
                }
            }
        }
        "powershell.exe".to_owned()
    } else {
        if let Ok(shell) = std::env::var("SHELL")
            && !shell.trim().is_empty()
        {
            return shell;
        }
        if let Ok(user) = std::env::var("USER")
            && let Ok(passwd) = fs::read_to_string("/etc/passwd")
        {
            for line in passwd.lines() {
                let fields: Vec<_> = line.split(':').collect();
                if fields.len() == 7 && fields[0] == user && !fields[6].is_empty() {
                    return fields[6].to_owned();
                }
            }
        }
        "/bin/sh".to_owned()
    }
}

fn metadata_command(kind: &SessionKind) -> Result<Command> {
    let format = "#{pane_current_path}\t#{pane_current_command}";
    match kind {
        SessionKind::Local => bail!("Local shell uses process metadata"),
        SessionKind::LocalTmux { session } => {
            validate_session(session)?;
            let mut command = local_tmux_command();
            command.args([
                "-L",
                "bmux-local-v1",
                "display-message",
                "-p",
                "-t",
                &format!("={session}:"),
                format,
            ]);
            Ok(command)
        }
        SessionKind::Ssh {
            command: raw,
            session,
        } => {
            validate_session(session)?;
            let plan = parse_ssh(raw)?;
            let mut command = Command::new("ssh");
            command.args([
                "-o",
                "BatchMode=yes",
                "-o",
                "ConnectTimeout=5",
                "-o",
                "ServerAliveInterval=3",
                "-o",
                "ServerAliveCountMax=1",
                "-o",
                "RemoteCommand=none",
                "-T",
            ]);
            command.args(plan.transport);
            command.arg(format!(
                "tmux -L bmux-v1 display-message -p -t {} {}",
                quote(&format!("={session}:")),
                quote(format)
            ));
            Ok(command)
        }
    }
}

fn bounded_output(
    command: &mut Command,
    timeout: Duration,
) -> std::io::Result<std::process::Output> {
    hide_console(command);
    let mut child = command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()?;
    let deadline = Instant::now() + timeout;
    loop {
        if child.try_wait()?.is_some() {
            return child.wait_with_output();
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            return Err(std::io::Error::new(
                std::io::ErrorKind::TimedOut,
                "Terminal metadata request timed out",
            ));
        }
        thread::sleep(Duration::from_millis(50));
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    static COUNTER: AtomicUsize = AtomicUsize::new(0);

    fn spec() -> SpawnSpec {
        let id = format!(
            "test-{}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::Relaxed)
        );
        SpawnSpec {
            pane_id: id.clone(),
            cwd: std::env::temp_dir(),
            command: None,
            shell: if cfg!(windows) {
                "powershell.exe"
            } else {
                "/bin/sh"
            }
            .to_owned(),
            kind: SessionKind::Local,
            transcript_dir: std::env::temp_dir().join(format!("bmux-{id}")),
            transcript_max_mb: 1,
            restore_tail_kb: 0,
            scrollback_lines: 1000,
        }
    }
    #[cfg(unix)]
    fn wait_for(terminal: &mut Terminal, needle: &str) {
        let deadline = Instant::now() + Duration::from_secs(8);
        while Instant::now() < deadline {
            terminal.poll();
            if terminal.screen_text().contains(needle) {
                return;
            }
            thread::sleep(Duration::from_millis(10));
        }
        panic!(
            "Expected {needle:?}, got {:?}; exit={:?}, error={:?}",
            terminal.screen_text(),
            terminal.exit_code(),
            terminal.last_error
        );
    }

    #[test]
    fn ssh_tokens_preserve_boundaries_and_reject_malformed_options() {
        let plan = parse_ssh("ssh -p 2222 -i '/tmp/key with spaces' -o 'ProxyCommand=nc %h %p' user@host printf 'hello world'").unwrap();
        assert_eq!(
            plan.transport,
            [
                "-p",
                "2222",
                "-i",
                "/tmp/key with spaces",
                "-o",
                "ProxyCommand=nc %h %p",
                "user@host"
            ]
        );
        assert_eq!(plan.remote, ["printf", "hello world"]);
        assert_eq!(
            parse_ssh(r#"ssh -i "C:\Users\Alice Smith\key" host"#)
                .unwrap()
                .transport[1],
            r"C:\Users\Alice Smith\key"
        );
        assert_eq!(
            parse_ssh(r"ssh -i C:\Users\Alice\key host")
                .unwrap()
                .transport[1],
            r"C:\Users\Alice\key"
        );
        assert_eq!(parse_ssh("alice@host").unwrap().transport, ["alice@host"]);
        for bad in [
            "ssh",
            "ssh -i",
            "ssh -N host",
            "ssh -W host:80 jump",
            "ssh 'unterminated",
            "mosh host",
        ] {
            assert!(parse_ssh(bad).is_err(), "accepted {bad:?}");
        }
        let command = parse_ssh("ssh user@host '; touch /tmp/no' ").unwrap();
        assert_eq!(command.remote, ["; touch /tmp/no"]);
    }

    #[test]
    fn shell_quoting_and_session_validation_block_command_injection() {
        assert_eq!(quote("a'b"), "'a'\\''b'");
        for bad in [
            "",
            "-danger",
            "hello world",
            "ok;rm",
            "../../somewhere",
            "name:1",
        ] {
            assert!(validate_session(bad).is_err());
        }
        assert!(validate_session("bmux-test_123-abc").is_ok());
    }

    #[test]
    fn keyboard_encodes_readline_vim_and_modified_navigation() {
        assert_eq!(encode_key(Key::C, Modifiers::CTRL, false).unwrap(), b"\x03");
        assert_eq!(encode_key(Key::W, Modifiers::CTRL, false).unwrap(), b"\x17");
        assert_eq!(
            encode_key(Key::ArrowUp, Modifiers::NONE, true).unwrap(),
            b"\x1bOA"
        );
        assert_eq!(
            encode_key(Key::ArrowLeft, Modifiers::CTRL, false).unwrap(),
            b"\x1b[1;5D"
        );
        assert_eq!(
            encode_key(Key::Tab, Modifiers::SHIFT, false).unwrap(),
            b"\x1b[Z"
        );
        assert_eq!(
            encode_key(Key::F12, Modifiers::NONE, false).unwrap(),
            b"\x1b[24~"
        );
        assert!(encode_key(Key::A, Modifiers::NONE, false).is_none());
    }

    #[test]
    fn ansi_queries_are_incremental_and_cursor_reply_precedes_later_text() {
        let mut parser = vt100::Parser::new(24, 80, 100);
        let mut protocol = Protocol::default();
        assert!(protocol.process(b"abc\x1b[", &mut parser).is_empty());
        assert_eq!(
            protocol.process(b"6nmore", &mut parser),
            vec![b"\x1b[1;4R".to_vec()]
        );
        assert_eq!(parser.screen().contents(), "abcmore");
        protocol.process(b"\x1b]7;file://remote/home/a%20b\x1b", &mut parser);
        protocol.process(b"\\", &mut parser);
        assert_eq!(protocol.cwd, Some(PathBuf::from("/home/a b")));
        protocol.process(b"\x1b[?1004h", &mut parser);
        assert!(protocol.focus_reporting);
    }

    #[test]
    fn ansi_screen_preserves_unicode_truecolor_and_alternate_screen() {
        let mut parser = vt100::Parser::new(8, 40, 100);
        parser.process("\x1b[38;2;10;20;30m界e\u{301}".as_bytes());
        assert!(parser.screen().cell(0, 0).unwrap().is_wide());
        assert!(parser.screen().cell(0, 1).unwrap().is_wide_continuation());
        assert_eq!(parser.screen().cell(0, 2).unwrap().contents(), "e\u{301}");
        assert_eq!(
            parser.screen().cell(0, 0).unwrap().fgcolor(),
            vt100::Color::Rgb(10, 20, 30)
        );
        parser.process(b"\x1b[?1049hfullscreen\x1b[?1049l");
        assert_eq!(parser.screen().contents(), "界e\u{301}");
    }

    #[test]
    fn transcript_rotation_is_bounded_and_has_no_metadata() {
        let spec = spec();
        fs::create_dir_all(&spec.transcript_dir).unwrap();
        let path = spec.transcript_dir.join("test.ts");
        let mut recorder = Recorder::new(path.clone(), 12).unwrap();
        recorder.append(b"12345678").unwrap();
        recorder.append(b"abcdefgh").unwrap();
        assert_eq!(fs::read(&path).unwrap(), b"abcdefgh");
        assert_eq!(fs::read(path.with_extension("ts.1")).unwrap(), b"12345678");
        drop(recorder);
        assert_eq!(read_tail(&path, 4), b"efgh");
        assert_eq!(
            plain_text(b"\x1b[31mred\x1b[0m\r\n\x1b]0;title\x07hello"),
            "red\nhello"
        );
        fs::remove_dir_all(spec.transcript_dir).unwrap();
    }

    #[test]
    fn persistent_bootstrap_preserves_history_and_forwards_ctrl_b() {
        let script = tmux_bootstrap("test-session", false, None, "", 5000).unwrap();
        assert!(script.contains("capture-pane -p -e -S -5000 -E -1"));
        assert!(script.contains("smcup@:rmcup@"));
        assert!(script.contains("prefix None"));
    }

    #[test]
    #[cfg(unix)]
    fn actual_pty_handles_input_resize_exit_and_records_output() {
        let mut spec = spec();
        spec.command = Some(
            "printf 'PTY_READY\\n'; read value; printf 'GOT:%s\\n' \"$value\"; stty size; exit 7"
                .to_owned(),
        );
        let dir = spec.transcript_dir.clone();
        let mut terminal = Terminal::spawn(spec, egui::Context::default()).unwrap();
        wait_for(&mut terminal, "PTY_READY");
        terminal.resize(37, 111).unwrap();
        terminal.write(b"hello terminal\n").unwrap();
        wait_for(&mut terminal, "37 111");
        wait_for(&mut terminal, "GOT:hello terminal");
        let deadline = Instant::now() + Duration::from_secs(2);
        while terminal.is_alive() && Instant::now() < deadline {
            terminal.poll();
            thread::sleep(Duration::from_millis(10));
        }
        assert_eq!(terminal.exit_code(), Some(7));
        assert!(terminal.history().contains("GOT:hello terminal"));
        drop(terminal);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    #[cfg(unix)]
    fn actual_tmux_survives_detach_and_explicit_close_removes_session() {
        if Command::new("tmux").arg("-V").output().is_err() {
            eprintln!("tmux integration test skipped: tmux is not installed");
            return;
        }
        let mut spec = spec();
        let session = format!("bmux-rust-{}", spec.pane_id);
        spec.kind = SessionKind::LocalTmux {
            session: session.clone(),
        };
        let directory = spec.transcript_dir.clone();
        let target = format!("={session}");
        struct Cleanup(String);
        impl Drop for Cleanup {
            fn drop(&mut self) {
                let _ = Command::new("tmux")
                    .args(["-L", "bmux-local-v1", "kill-session", "-t", &self.0])
                    .output();
            }
        }
        let _cleanup = Cleanup(target.clone());
        let mut terminal = Terminal::spawn(spec.clone(), egui::Context::default()).unwrap();
        terminal
            .write(b"printf 'BMUX_PERSIST_%s\\n' 'READY'\n")
            .unwrap();
        wait_for(&mut terminal, "BMUX_PERSIST_READY");
        assert!(
            !terminal.parser.screen().alternate_screen(),
            "tmux must leave normal scrollback available"
        );
        terminal.write(b"i=0; while [ $i -lt 60 ]; do printf 'HISTORY_MARK_%s\\n' \"$i\"; i=$((i+1)); done\n").unwrap();
        wait_for(&mut terminal, "HISTORY_MARK_59");
        terminal
            .write(b"export BMUX_TEST_PERSIST='survived'\n")
            .unwrap();
        thread::sleep(Duration::from_millis(150));
        drop(terminal);
        thread::sleep(Duration::from_millis(100));
        assert!(
            Command::new("tmux")
                .args(["-L", "bmux-local-v1", "has-session", "-t", &target])
                .status()
                .unwrap()
                .success()
        );
        let mut restored = Terminal::spawn(spec, egui::Context::default()).unwrap();
        restored
            .write(b"printf 'AFTER:%s\\n' \"$BMUX_TEST_PERSIST\"\n")
            .unwrap();
        wait_for(&mut restored, "AFTER:survived");
        restored.scroll_top();
        assert!(
            restored.screen_text().contains("HISTORY_MARK_0"),
            "History missing after reattach: {:?}",
            restored.screen_text()
        );
        restored.scroll_bottom();
        restored.close().unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        let mut exists = true;
        while exists && Instant::now() < deadline {
            exists = Command::new("tmux")
                .args(["-L", "bmux-local-v1", "has-session", "-t", &target])
                .output()
                .unwrap()
                .status
                .success();
            thread::sleep(Duration::from_millis(25));
        }
        assert!(
            !exists,
            "Explicit close must destroy only its own tmux session"
        );
        let _ = fs::remove_dir_all(directory);
    }
    /// Set BMUX_TEST_SSH_COMMAND to an SSH destination under your control with tmux installed.
    #[test]
    #[cfg(unix)]
    #[ignore = "requires BMUX_TEST_SSH_COMMAND and an isolated SSH fixture"]
    fn actual_ssh_survives_transport_loss_and_closes_only_its_session() {
        let command = std::env::var("BMUX_TEST_SSH_COMMAND").expect("Set BMUX_TEST_SSH_COMMAND");
        let mut spec = spec();
        let directory = spec.transcript_dir.clone();
        let session = format!("bmux-rust-ssh-{}", spec.pane_id);
        let kind = SessionKind::Ssh {
            command: command.clone(),
            session: session.clone(),
        };
        spec.kind = kind.clone();
        struct Cleanup(SessionKind);
        impl Drop for Cleanup {
            fn drop(&mut self) {
                if let Ok(Some(mut cmd)) = cleanup_command(&self.0) {
                    let _ = cmd.output();
                }
            }
        }
        let _cleanup = Cleanup(kind.clone());
        let mut term = Terminal::spawn(spec.clone(), egui::Context::default()).unwrap();
        term.write(b"export BMUX_SSH_VALUE=survived; printf 'SSH_%s\\n' READY\n")
            .unwrap();
        wait_for(&mut term, "SSH_READY");
        term.resize(37, 111).unwrap();
        term.write(b"stty size\n").unwrap();
        wait_for(&mut term, "37 111");
        term.write(b"printf 'REMOTE_PID:%s\\n' $$\n").unwrap();
        wait_for(&mut term, "REMOTE_PID:");
        drop(term);
        thread::sleep(Duration::from_millis(200));
        let mut restored = Terminal::spawn(spec, egui::Context::default()).unwrap();
        restored
            .write(b"printf 'SSH_RESTORED:%s\\n' \"$BMUX_SSH_VALUE\"\n")
            .unwrap();
        wait_for(&mut restored, "SSH_RESTORED:survived");
        restored
            .write(b"cd /tmp; printf 'CWD_%s\\n' READY\n")
            .unwrap();
        wait_for(&mut restored, "CWD_READY");
        let deadline = Instant::now() + Duration::from_secs(8);
        while restored.cwd() != Path::new("/tmp") && Instant::now() < deadline {
            restored.poll();
            thread::sleep(Duration::from_millis(50));
        }
        assert_eq!(restored.cwd(), PathBuf::from("/tmp"));
        restored.spec.kind = SessionKind::Ssh {
            command: "ssh -o ProxyCommand=false unavailable-test-host".into(),
            session: session.clone(),
        };
        restored.close().unwrap();
        let plan = parse_ssh(&command).unwrap();
        let deadline = Instant::now() + Duration::from_secs(8);
        let mut exists = true;
        while exists && Instant::now() < deadline {
            exists = Command::new("ssh")
                .args(["-o", "BatchMode=yes", "-o", "ConnectTimeout=2"])
                .args(&plan.transport)
                .arg(format!(
                    "tmux -L bmux-v1 has-session -t {}",
                    quote(&format!("={session}"))
                ))
                .output()
                .unwrap()
                .status
                .success();
            thread::sleep(Duration::from_millis(100));
        }
        assert!(!exists, "Explicit SSH close must end its remote session");
        let _ = fs::remove_dir_all(directory);
    }

    #[test]
    #[cfg(unix)]
    fn egui_copy_event_and_ctrl_c_key_send_exactly_one_interrupt_byte() {
        let mut spec = spec();
        spec.command = Some("stty -echo -icanon -isig min 0 time 3; printf 'RAW_READY\\n'; dd bs=1 count=8 2>/dev/null | od -An -tu1; printf 'RAW_DONE\\n'".into());
        let dir = spec.transcript_dir.clone();
        let ctx = egui::Context::default();
        let mut terminal = Terminal::spawn(spec, ctx.clone()).unwrap();
        wait_for(&mut terminal, "RAW_READY");
        let input = egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(800.0, 500.0))),
            modifiers: Modifiers::CTRL,
            events: vec![
                egui::Event::Copy,
                egui::Event::Key {
                    key: Key::C,
                    physical_key: Some(Key::C),
                    pressed: true,
                    repeat: false,
                    modifiers: Modifiers::CTRL,
                },
            ],
            ..Default::default()
        };
        let result = ctx.run(input, |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                terminal.show(ui, ui.max_rect(), true, &TerminalStyle::default());
            });
            if ctx.current_pass_index() == 0 {
                ctx.request_discard("Exercise raw terminal input across a sizing pass");
            }
        });
        assert_eq!(result.platform_output.num_completed_passes, 2);
        wait_for(&mut terminal, "RAW_DONE");
        let text = terminal.screen_text();
        let numbers: Vec<_> = text
            .lines()
            .filter(|line| {
                line.trim() == "3"
                    || line.split_whitespace().filter(|part| *part == "3").count() > 1
            })
            .collect();
        assert_eq!(numbers.len(), 1, "Missing control byte: {text:?}");
        assert_eq!(numbers[0].trim(), "3", "Duplicate control byte: {text:?}");
        drop(terminal);
        let _ = fs::remove_dir_all(dir);
    }
    #[test]
    #[cfg(unix)]
    fn overlaid_controls_do_not_select_terminal_text_underneath() {
        let mut spec = spec();
        spec.command = Some("printf 'VISIBLE_TERMINAL_TEXT\\n'; read value".into());
        let directory = spec.transcript_dir.clone();
        let ctx = egui::Context::default();
        let mut terminal = Terminal::spawn(spec, ctx.clone()).unwrap();
        wait_for(&mut terminal, "VISIBLE_TERMINAL_TEXT");
        let screen = Rect::from_min_size(Pos2::ZERO, Vec2::new(600.0, 400.0));
        let overlay = Rect::from_min_size(Pos2::new(20.0, 20.0), Vec2::new(180.0, 80.0));
        let draw = |terminal: &mut Terminal, ctx: &egui::Context| {
            egui::CentralPanel::default().show(ctx, |ui| {
                terminal.show(ui, ui.max_rect(), true, &TerminalStyle::default());
                ui.put(overlay, egui::Button::new("Pane actions"));
            });
        };
        let _ = ctx.run(
            egui::RawInput {
                screen_rect: Some(screen),
                ..Default::default()
            },
            |ctx| draw(&mut terminal, ctx),
        );
        let position = Pos2::new(60.0, 35.0);
        let _ = ctx.run(
            egui::RawInput {
                screen_rect: Some(screen),
                events: vec![
                    egui::Event::PointerMoved(position),
                    egui::Event::PointerButton {
                        pos: position,
                        button: egui::PointerButton::Primary,
                        pressed: true,
                        modifiers: Modifiers::NONE,
                    },
                ],
                ..Default::default()
            },
            |ctx| draw(&mut terminal, ctx),
        );
        assert!(
            terminal.selection.is_none(),
            "Overlay click selected text in the terminal underneath"
        );
        let _ = ctx.run(
            egui::RawInput {
                screen_rect: Some(screen),
                events: vec![egui::Event::PointerButton {
                    pos: position,
                    button: egui::PointerButton::Primary,
                    pressed: false,
                    modifiers: Modifiers::NONE,
                }],
                ..Default::default()
            },
            |ctx| draw(&mut terminal, ctx),
        );
        let position = Pos2::new(350.0, 120.0);
        let _ = ctx.run(
            egui::RawInput {
                screen_rect: Some(screen),
                events: vec![
                    egui::Event::PointerMoved(position),
                    egui::Event::PointerButton {
                        pos: position,
                        button: egui::PointerButton::Primary,
                        pressed: true,
                        modifiers: Modifiers::NONE,
                    },
                ],
                ..Default::default()
            },
            |ctx| draw(&mut terminal, ctx),
        );
        assert!(
            terminal.selection.is_some(),
            "Uncovered terminal must still accept selection"
        );
        drop(terminal);
        let _ = fs::remove_dir_all(directory);
    }

    #[test]
    fn find_urls_detects_common_forms() {
        let hits = find_urls("see https://example.com/a?b=c#d and http://x.io/y plus www.t.org/z!");
        assert_eq!(hits.len(), 3);
        assert_eq!(hits[0].2, "https://example.com/a?b=c#d");
        assert_eq!(hits[1].2, "http://x.io/y");
        assert_eq!(hits[2].2, "https://www.t.org/z");
        let hits = find_urls("(https://example.com).");
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].2, "https://example.com");
        assert!(find_urls("https:// and wwwexample").is_empty());
        let hits = find_urls("open file:///home/u/my%20file.txt now");
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].2, "file:///home/u/my%20file.txt");
    }

    #[test]
    fn hyperlink_covers_cells_within_span() {
        let link = Hyperlink {
            uri: "https://example.test".into(),
            start_row: 4,
            start_col: 6,
            end_row: 4,
            end_col: 10,
        };
        assert!(link.covers(CellPos { row: 4, col: 8 }));
        assert!(!link.covers(CellPos { row: 4, col: 5 }));
        assert!(!link.covers(CellPos { row: 5, col: 8 }));
        let wrapped = Hyperlink {
            uri: "https://example.test/long".into(),
            start_row: 4,
            start_col: 70,
            end_row: 6,
            end_col: 3,
        };
        assert!(wrapped.covers(CellPos { row: 4, col: 79 }));
        assert!(wrapped.covers(CellPos { row: 5, col: 0 }));
        assert!(wrapped.covers(CellPos { row: 6, col: 3 }));
        assert!(!wrapped.covers(CellPos { row: 6, col: 4 }));
        assert!(!wrapped.covers(CellPos { row: 3, col: 79 }));
    }

    #[test]
    fn shell_quote_prefers_relative_and_quotes_spaces() {
        let cwd = Path::new("/home/u/proj");
        assert_eq!(
            shell_quote(Path::new("/home/u/proj/src/main.rs"), cwd),
            "./src/main.rs"
        );
        assert_eq!(
            shell_quote(Path::new("/home/u/proj/my file.txt"), cwd),
            "'./my file.txt'"
        );
        assert_eq!(
            shell_quote(Path::new("/etc/hostname"), cwd),
            "/etc/hostname"
        );
        assert_eq!(shell_quote(Path::new("/home/u/proj/a;b"), cwd), "'./a;b'");
    }

    #[test]
    fn tmux_bootstrap_enables_mouse_and_desktop_env() {
        let bootstrap = tmux_bootstrap("fixture-1", true, None, "true", 1000).unwrap();
        assert!(
            bootstrap.contains("mouse on"),
            "tmux must forward mouse events; got: {bootstrap}"
        );
        assert!(!bootstrap.contains("mouse off"));
        for variable in ["WAYLAND_DISPLAY", "XDG_RUNTIME_DIR", "BROWSER"] {
            assert!(
                bootstrap.contains(variable),
                "tmux must propagate {variable}; got: {bootstrap}"
            );
        }
    }

    #[test]
    #[cfg(unix)]
    fn sgr_mouse_press_and_release_reach_shell() {
        let mut spec = spec();
        spec.command = Some("printf '\\033[?1000h\\033[?1006hMOUSE_READY\\n'; stty -icanon min 0 time 25; dd bs=1 count=48 2>/dev/null | od -An -tu1; printf 'MOUSE_DONE\\n'".into());
        let directory = spec.transcript_dir.clone();
        let ctx = egui::Context::default();
        let mut terminal = Terminal::spawn(spec, ctx.clone()).unwrap();
        wait_for(&mut terminal, "MOUSE_READY");
        let screen = Rect::from_min_size(Pos2::ZERO, Vec2::new(800.0, 500.0));
        let position = Pos2::new(400.0, 250.0);
        let draw = |terminal: &mut Terminal, ctx: &egui::Context, events: Vec<egui::Event>| {
            let _ = ctx.run(
                egui::RawInput {
                    screen_rect: Some(screen),
                    events,
                    ..Default::default()
                },
                |ctx| {
                    egui::CentralPanel::default().show(ctx, |ui| {
                        terminal.show(ui, ui.max_rect(), true, &TerminalStyle::default());
                    });
                },
            );
        };
        draw(
            &mut terminal,
            &ctx,
            vec![egui::Event::PointerMoved(position)],
        );
        draw(
            &mut terminal,
            &ctx,
            vec![
                egui::Event::PointerMoved(position),
                egui::Event::PointerButton {
                    pos: position,
                    button: egui::PointerButton::Primary,
                    pressed: true,
                    modifiers: Modifiers::NONE,
                },
            ],
        );
        draw(
            &mut terminal,
            &ctx,
            vec![egui::Event::PointerButton {
                pos: position,
                button: egui::PointerButton::Primary,
                pressed: false,
                modifiers: Modifiers::NONE,
            }],
        );
        wait_for(&mut terminal, "MOUSE_DONE");
        let bytes: Vec<u8> = terminal
            .screen_text()
            .split_whitespace()
            .filter_map(|part| part.parse().ok())
            .collect();
        let sgr = |window: &[u8]| {
            window.len() >= 5
                && window[0] == 27
                && window[1] == 91
                && window[2] == 60
                && window[3] == b'0'
                && window[4] == b';'
        };
        assert!(
            bytes.windows(5).any(sgr),
            "Missing SGR press/release sequence: {bytes:?}"
        );
        assert!(
            bytes.contains(&77),
            "Missing SGR press terminator (M): {bytes:?}"
        );
        assert!(
            bytes.contains(&109),
            "Missing SGR release terminator (m): {bytes:?}"
        );
        drop(terminal);
        let _ = fs::remove_dir_all(directory);
    }

    #[test]
    fn osc8_sequences_record_spans() {
        let mut parser = vt100::Parser::new(24, 80, 1000);
        let mut protocol = Protocol::default();
        protocol.process(b"visit https://example.com here\r\n", &mut parser);
        protocol.process(
            b"\x1b]8;;https://osc8.example/inner\x1b\\LINKTEXT\x1b]8;;\x1b\\\r\n",
            &mut parser,
        );
        assert_eq!(protocol.links.len(), 1, "links: {:?}", protocol.links);
        let link = &protocol.links[0];
        assert_eq!(link.uri, "https://osc8.example/inner");
        assert!(
            link.covers(CellPos {
                row: link.start_row,
                col: link.start_col,
            }),
            "span must cover its own start: {link:?}"
        );
    }

    #[test]
    #[cfg(unix)]
    fn secondary_click_arms_menu_without_selecting() {
        let mut spec = spec();
        spec.command = Some("printf 'MENU_READY\\n'; read value".into());
        let directory = spec.transcript_dir.clone();
        let ctx = egui::Context::default();
        let mut terminal = Terminal::spawn(spec, ctx.clone()).unwrap();
        wait_for(&mut terminal, "MENU_READY");
        let screen = Rect::from_min_size(Pos2::ZERO, Vec2::new(600.0, 400.0));
        let position = Pos2::new(200.0, 120.0);
        let draw = |terminal: &mut Terminal, ctx: &egui::Context, events: Vec<egui::Event>| {
            let _ = ctx.run(
                egui::RawInput {
                    screen_rect: Some(screen),
                    events,
                    ..Default::default()
                },
                |ctx| {
                    egui::CentralPanel::default().show(ctx, |ui| {
                        terminal.show(ui, ui.max_rect(), true, &TerminalStyle::default());
                    });
                },
            );
        };
        draw(
            &mut terminal,
            &ctx,
            vec![egui::Event::PointerMoved(position)],
        );
        for pressed in [true, false] {
            draw(
                &mut terminal,
                &ctx,
                vec![
                    egui::Event::PointerMoved(position),
                    egui::Event::PointerButton {
                        pos: position,
                        button: egui::PointerButton::Secondary,
                        pressed,
                        modifiers: Modifiers::NONE,
                    },
                ],
            );
        }
        assert!(
            terminal.selection.is_none(),
            "Right-click must not start a text selection"
        );
        assert!(
            terminal.context_cell.is_some(),
            "Right-click must arm the terminal menu"
        );
        assert!(
            egui::Popup::is_any_open(&ctx),
            "Right-click must open the terminal menu"
        );
        drop(terminal);
        let _ = fs::remove_dir_all(directory);
    }

    #[test]
    #[cfg(unix)]
    fn osc8_and_printed_urls_resolve_under_cursor() {
        let mut spec = spec();
        spec.command = Some("printf 'visit https://example.com/docs?x=1 here\\n'; printf '\\033]8;;https://osc8.example/inner\\033\\\\LINKTEXT\\033]8;;\\033\\\\\\n'; printf 'READY\\n'".into());
        let directory = spec.transcript_dir.clone();
        let ctx = egui::Context::default();
        let mut terminal = Terminal::spawn(spec, ctx.clone()).unwrap();
        wait_for(&mut terminal, "READY");
        let text = terminal.screen_text();
        let url_row = text
            .lines()
            .position(|line| line.contains("https://example.com"))
            .expect("printed URL missing") as u16;
        let url_col = text
            .lines()
            .nth(url_row as usize)
            .unwrap()
            .find("https://")
            .unwrap();
        // Map the line offset back to a cell: lines here are plain ASCII.
        let hit = terminal
            .hit_link(CellPos {
                row: url_row,
                col: url_col as u16 + 8,
            })
            .expect("printed URL must resolve");
        assert_eq!(hit.uri, "https://example.com/docs?x=1");
        let osc_row = text
            .lines()
            .position(|line| line.contains("LINKTEXT"))
            .expect("OSC 8 text missing") as u16;
        let osc_col = text
            .lines()
            .nth(osc_row as usize)
            .unwrap()
            .find("LINKTEXT")
            .unwrap();
        let hit = terminal
            .hit_link(CellPos {
                row: osc_row,
                col: osc_col as u16 + 2,
            })
            .expect("OSC 8 link must resolve");
        assert_eq!(hit.uri, "https://osc8.example/inner");
        drop(terminal);
        let _ = fs::remove_dir_all(directory);
    }
}

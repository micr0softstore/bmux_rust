# Rust port: architecture and parity

The original application is a macOS SwiftUI shell around libghostty. This
port replaces the application, persistence, terminal integration, and UI with
Rust code in `src/`, and the Swift sources are no longer kept in this checkout.

| Area | Rust implementation |
| --- | --- |
| Desktop UI | eframe/egui, custom application chrome and theme-matched surfaces |
| Terminal process | portable-pty Unix PTY / Windows ConPTY |
| ANSI screen | vt100 parser with Rust cell rendering |
| Persistence | JSON settings, workspace trees, pane identifiers, bounded transcripts |
| Fonts | Bundled Geist Mono and fallback font coverage |
| Colors | Vendored Ghostty-format catalog, dark/light themes, color overrides |
| Linux packaging | Debian package, desktop launcher, standard hicolor icon sizes, portable archive |
| Windows packaging | Executable with icon, DPI manifest, version resource, NSIS installer, portable ZIP |

## Visual fidelity

The interface follows the source's warm near-black Srcery background,
parchment foreground, orange accent, compact sidebar rows with color strips,
per-pane folder labels, pill-shaped tab bar, pane controls, hairline split
dividers, and eight settings sections. The navigator stays hidden until the
pointer enters a thin band at the top edge — clicks alone never reveal it —
then floats over the terminal as glass pills without pushing the grid around
or covering a full row with an opaque bar. A split tab displays its pane count and
a picker for every pane; split panes remain grouped in their original tab.
Long tab lists scroll to the active tab, with a compact overflow menu when
space is tight. Light themes also recolor the application chrome.

The whole window has antialiased 12-point corners, including the terminal and
any overlapping menus. Windows reported as maximized or fullscreen by the
window backend stay square. The final
OpenGL pass masks the window's alpha channel, so rounded corners require the
desktop compositor's transparency support. Split dividers fill their complete
gap with an opaque theme color and align to physical pixels at fractional DPI;
terminal opacity does not make the dividers see-through.

SwiftUI's native text layout, SF Symbols, titlebar behavior, and macOS 26
Liquid Glass are platform APIs. Their Rust counterparts approximate the
shape, spacing, color, and behavior with portable drawing. Pixel identity and
Apple's dynamic glass/refraction effects are not claimed. The portable glass
material uses translucent gradients, highlights, rounded outlines, and soft
shadows, with nearly opaque menu surfaces to preserve text readability. It
does not blur content behind a popup or the desktop behind the window.
Desktop font rasterization and window controls also vary by platform and scale.

`assets/bmux.png` is the supplied 500-pixel PNG, preserved byte for byte.
`packaging/export-icon.py` exports ICO and standard 16–512-pixel hicolor PNGs
from that artwork without redrawing it. Linux packages also install the PNG
under `pixmaps` as a fallback. The earlier 1024-only hicolor path was not listed
by common desktop icon themes and could leave the installed app without an
icon.

## Terminal-engine differences

The terminal engine is a portable VT screen implementation rather than
libghostty. Normal interactive shells and ANSI terminal programs are the
target. This is not a complete reimplementation of every Ghostty extension.
Kitty graphics, Sixel image output, Ghostty shader configuration, and
Ghostty-specific keybinding/configuration directives are not supported.
Font ligature shaping and full bidirectional-script terminal shaping are not
equivalent to Ghostty. Background blur is unsupported by the portable renderer.
Remote directory and command labels use tmux metadata when batch-mode SSH
authentication is available, with shell OSC 7 directory notifications as a
fallback. Keep these limitations in mind for specialized TUIs.

Pointer input follows the application: clicks, drags and wheel events are
encoded with the active mouse protocol (including SGR), so vim-style programs
receive left clicks. Tmux-backed sessions run with tmux mouse support enabled
for the same reason, and propagate the Wayland/display and browser environment
so `xdg-open` works inside them; Shift+click always selects terminal-side
text instead. Ctrl+Click opens the hyperlink or URL under the cursor
(OSC 8 links and plain-text URLs, including links wrapped across rows) in the
host browser, which also works for remote panes. Files dropped from the OS
paste their paths into the focused pane, relative (`./…`) when under its
directory. Window-system drag-and-drop only reaches the app on X11 and
Windows: the underlying toolkit does not deliver file drops on Wayland, so
the terminal menu also offers “Browse Files to Paste…” as a file-picker path
that works everywhere. Right-click opens the terminal menu (copy, paste,
select all, link actions, file picker, clear); while a mouse-aware program
owns the pointer, use Shift+right-click for the menu instead.

Raw Ghostty configuration cannot be forwarded to a Ghostty engine in this
build. The Advanced settings panel documents the subset the Rust port
understands; unsupported options must not be treated as working merely
because the settings text is preserved.

Persistence has the same distinction as the original: a regular local shell
is restarted with a transcript replay; a tmux session preserves live
processes. Transcript replay restores recorded display history, not the
original process, shell variables, or its exact terminal parser state.
Scrollback capacity is represented as a line budget in the Rust parser,
so a megabyte setting is an estimate rather than Ghostty's exact allocation.

## Platform behavior

Linux requires a graphical X11/Wayland session and OpenGL. The default local
shell is discovered from the user's environment, with a Unix shell fallback.
Windows uses ConPTY and a native Windows shell. Local tmux on Windows runs
inside the default WSL distribution and requires tmux there. SSH uses the
platform's OpenSSH client, and persistent sessions require tmux on the remote
host. Password and key-passphrase prompts run in the terminal itself.

Application shortcuts translate macOS Command actions to Ctrl+Shift. The
second split shortcut becomes Ctrl+Shift+E to avoid colliding with split
right. See the README for the complete mapping.

Saved state can retain directories from another OS; such paths need to be
updated for that OS. Native Windows paths and WSL paths name different file
systems. Copies of original Swift state should be treated as migration input,
and the original files should be backed up before use.

## Verification and release expectations

`cargo test` exercises persistence, workspace trees, terminal parsing, input,
and session-command construction. `.github/workflows/rust.yml` runs formatting,
Clippy, tests, and release packaging on Linux and Windows. CI configuration
is not evidence that a remote workflow has run; inspect its actual result.

Before publishing a release, test these user flows on each platform:

1. Launch a shell; type Unicode; resize; copy and paste; run a fullscreen TUI.
2. Create and reorder workspaces, change colors, duplicate, rename, and delete.
3. Add tabs and nested horizontal/vertical splits; drag dividers; cycle focus.
4. Reopen the application and verify selection, layout, settings, and history.
5. Detach/reconnect local tmux and SSH, verifying process survival; explicitly
   close a pane and verify only that pane's session is terminated.
6. Exercise password/key prompts, failed SSH reconnect, server outages, and
   custom SSH aliases with real servers.
7. Check light/dark themes, high-DPI scaling, keyboard layouts, IME, multi-monitor
   placement, and accessibility with the target platform's native tools.

## Regenerating imported assets

Icon export requires Python 3 and Pillow:

```sh
python3 packaging/export-icon.py
```

Theme import reads only the `ghostty` directory and license in an existing
checkout. The generated source manifest records that checkout's exact commit:

```sh
python3 packaging/import-themes.py /path/to/iTerm2-Color-Schemes
```

The current import's license is in `assets/themes.LICENSE` and its repository,
commit, and count are in `assets/themes.source.json`.

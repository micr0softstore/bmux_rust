# [bmux] — a workspace terminal for Linux and Windows

This checkout contains a native Rust port of
[imbencetsx/bmux](https://github.com/imbencetsx/bmux). It keeps the warm Srcery
appearance, workspace sidebar, centered terminal tabs, split panes, SSH/tmux
sessions, settings, Geist Mono typography, and the supplied app-icon artwork.

Version 1.2.3 adds rounded window corners and solid split dividers. The tab
navigator stays visible, shows a pane count and picker for split terminals,
and scrolls newly selected tabs into view.
The toolbar, navigator, sidebar, and menus use a portable frosted-glass style.
Linux packages include the supplied app icon in standard desktop-theme sizes.

The Rust application uses `egui`/`eframe` for its desktop interface,
`portable-pty` for Unix PTYs and Windows ConPTY, and `vt100` for terminal state.
It does not need Swift, Xcode, a browser, or a JavaScript runtime.
Saved workspaces and settings from the original Swift build are still
accepted as migration input (back them up first).

See [docs/PORTING.md](docs/PORTING.md) for platform differences and the limits
of parity with the SwiftUI/Ghostty version. Actual checks and remaining runtime
coverage are recorded in [docs/VALIDATION.md](docs/VALIDATION.md).

## Build and run

Use stable Rust 1.88 or newer with Cargo. The lockfile is checked in. For a
first-time Rust installation, follow [rustup.rs](https://rustup.rs/).

### Linux

On Debian/Ubuntu:

```sh
sudo apt-get install build-essential pkg-config libx11-dev libxkbcommon-dev libwayland-dev libgl1-mesa-dev
cargo run --release --locked
```

A graphical X11 or Wayland session and an OpenGL-capable driver are required.
Install `openssh-client` for SSH workspaces and `tmux` for persistent local
workspaces. Local tmux and remote SSH workspaces require tmux 3.2 or newer
where the session runs.

To build and install the executable, desktop launcher, and icon for your user:

```sh
./packaging/install-linux.sh
```

The default prefix is `~/.local`; pass an absolute directory to change it:
`./packaging/install-linux.sh /your/prefix`. You can also build a portable
archive with `./packaging/package-linux.sh`.

To build an installable Debian/Ubuntu package (native amd64 or arm64):

```sh
sudo apt-get install dpkg-dev binutils python3 python3-pil
./packaging/build-deb.sh
sudo apt install ./dist/bmux_1.2.5-1_amd64.deb
```

Use the filename printed by the script for your version and architecture.
Building the package needs no root privileges. It installs `/usr/bin/bmux`,
the application-menu launcher, the original icon, and notices; remove it with
`sudo apt remove bmux`. Saved workspaces and settings stay in your home directory.

Build on the oldest Debian/Ubuntu release you intend to support. The script
derives library version requirements from the executable and adds the X11,
Wayland, and OpenGL libraries loaded at runtime. A package built on a newer
distribution may not install on an older one.

Use `--binary /path/to/bmux` to package an existing native release executable,
or `--output-dir /path/to/packages` to choose another output folder. The version
comes from `Cargo.toml`; `DEB_REVISION` defaults to `1`. For distribution, set
`DEB_MAINTAINER='Your Name <you@example.com>'` instead of the default local-build
identity. Run `./packaging/build-deb.sh --help` for options.

### Windows

Use Windows 10 version 1809 or newer (ConPTY), or Windows 11. Install the Rust
MSVC toolchain and Visual Studio Build Tools with **Desktop development with
C++** and a Windows SDK, then run from a developer terminal:

```powershell
cargo run --release --locked
```

The release executable is `target\release\bmux.exe`; the icon and version
information are embedded in the executable. OpenSSH Client must be installed
and `ssh.exe` available on PATH for SSH workspaces. Persistent local tmux
sessions use WSL with tmux installed in the default distribution.

To create a portable ZIP in `dist\`:

```powershell
.\packaging\package-windows.ps1
```

To build an installable Windows setup executable, also install
[NSIS 3](https://nsis.sourceforge.io/Download), then run:

```powershell
.\packaging\build-windows-installer.ps1
```

The script builds the x64 MSVC release and writes the installer to `dist\`.
It uses the original app icon, installs for the current user without requiring
administrator access, creates a Start Menu shortcut, and offers a desktop
shortcut. Remove it through Windows **Installed apps**. Reinstalling upgrades
the same location; uninstalling preserves `%LOCALAPPDATA%\Bmux` settings and
workspaces. Close bmux before upgrading or uninstalling.

The default installation folder is `%LOCALAPPDATA%\Programs\Bmux`. The MSVC
build links the C runtime statically so the installer does not require a
separate Visual C++ runtime installation. Installers are unsigned unless you
sign them separately. SSH and WSL/tmux remain optional system components.

Use `-NsisPath 'C:\path\to\makensis.exe'` if NSIS is installed somewhere other
than the standard location. For a prebuilt executable, use
`-SkipBuild -BinaryPath 'C:\path\to\bmux.exe'`; pass the matching `-Target` when
using the GNU Windows toolchain. See `Get-Help
.\packaging\build-windows-installer.ps1 -Detailed` for all options.

Portable archives, Debian packages, and Windows installers are also uploaded
as artifacts by the **Rust desktop** GitHub Actions workflow when it runs on
Linux and Windows. The workflow is included in this checkout; its presence
does not imply a Windows build has already run.

## Workspaces and persistence

Use the **+** menu to create a local, local tmux, or SSH workspace. Each
workspace has its own tabs, nested splits, selected pane, color, and directory.
Workspace context menus provide rename, duplicate, reorder, and deletion.

Ordinary local shells restart when the application is reopened with a fresh,
empty screen; scrollback starts empty too. Their original processes do not
survive quitting. Tmux workspaces reattach to persistent sessions and likewise
reopen with a fresh screen; quitting detaches
the application, while explicitly closing a pane ends that pane's session.
Tmux persistence survives application restarts, not machine reboots.

SSH workspaces use the system SSH client and its existing configuration, keys,
agent, host-key verification, and interactive authentication prompts. Supply a
command such as `ssh user@host` or `ssh -p 2222 my-host`, plus an optional tmux
session prefix. The remote host must provide `tmux`.

## Keyboard shortcuts

Application actions use **Ctrl+Shift** on Linux and Windows so ordinary
terminal Control shortcuts remain available. A shortcut containing additional
Alt uses all three modifiers.

| Action | Shortcut |
| --- | --- |
| Toggle sidebar | Ctrl+Shift+S |
| New local workspace / SSH workspace | Ctrl+Shift+N / Ctrl+Shift+Alt+N |
| New terminal tab | Ctrl+Shift+T |
| Close pane / terminal tab | Ctrl+Shift+W / Ctrl+Shift+Alt+W |
| Split right / down | Ctrl+Shift+D / Ctrl+Shift+E |
| Previous / next tab | Ctrl+Shift+[ / Ctrl+Shift+] |
| Select tabs 1–8 / last tab | Ctrl+Shift+1–8 / Ctrl+Shift+9 |
| Previous / next pane | Ctrl+Shift+Alt+Left / Ctrl+Shift+Alt+Right |
| Previous / next workspace | Ctrl+Shift+Up / Ctrl+Shift+Down |
| Select workspaces 1–8 / last workspace | Ctrl+Shift+Alt+1–8 / Ctrl+Shift+Alt+9 |
| Pane history | Ctrl+Shift+H |
| Clear terminal | Ctrl+Shift+K |
| Scroll to top / bottom | Ctrl+Shift+Home / Ctrl+Shift+End |
| Increase / decrease / reset font size | Ctrl+Shift+Plus / Ctrl+Shift+Minus / Ctrl+Shift+0 |
| Reconnect pane | Ctrl+Shift+R |
| Settings | Ctrl+Shift+Comma |
| Copy selection / paste | Ctrl+Shift+C / Ctrl+Shift+V |
| Terminal menu (copy/paste/select/clear/link) | Right-click |
| Open link under cursor | Ctrl+Click |
| Select terminal text when the app owns the mouse | Shift+Drag |
| Paste dropped file paths | Drag files into the terminal (X11/Windows) or Right-click → Browse Files to Paste… |

The tab strip stays hidden until the pointer reaches the top edge. The History
viewer filters recorded output by text. **Insert Replay Command**
types a command into the pane; you review it and press Enter to execute it.

## Data and themes

- Linux: `$XDG_DATA_HOME/Bmux`, normally `~/.local/share/Bmux`.
- Windows: `%LOCALAPPDATA%\Bmux`.
- Set `BMUX_DATA_DIR` to use a separate data directory, including for testing.

Workspaces and settings are JSON files; per-pane recordings live in the
`transcripts` subdirectory. Settings can reveal the support folder. Keep a
backup before moving data between the original and Rust implementations.

The bundled theme catalog includes 769 full ANSI palettes from
[iTerm2-Color-Schemes](https://github.com/mbadolato/iTerm2-Color-Schemes), with
Srcery and Alabaster as the dark and light defaults. Font and icon assets are
embedded, so packaged executables do not need separate runtime asset files.
See [THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md) for provenance and licenses.

## Development checks

```sh
cargo fmt --all --check
cargo clippy --all-targets --locked -- -D warnings
cargo test --all-targets --locked
cargo build --release --locked
```

Platform tests that need tmux or another external tool may require installing
that tool. A successful compile does not replace interactive terminal, SSH,
IME, high-DPI, and window-manager testing on each target OS.

# Validation record — 2026-10-10

## Version 1.2.3 window and separator corrections

- 43 tests passed; the opt-in SSH fixture test remained ignored. New checks
  inspect the actual divider meshes for opaque, gap-free nested partitions at
  1x, 1.25x, and 2x DPI, verify divider dragging, and ensure the corner mask is
  applied once after all UI layers, including multipass rendering. Maximized
  and fullscreen viewport-state transitions are covered by the mask test.
- Formatting and native/Windows GNU Clippy passed with warnings denied. Linux
  and Windows GNU release builds completed; Windows was not launched natively.
- Visual checks used an isolated Xvfb display, Openbox, xcompmgr, and disposable
  application data. All four corner pixels matched the desktop exactly; curved
  edges were antialiased. Switching the desktop from mint to magenta changed
  translucent terminals while both divider samples remained RGB (55, 52, 45).
- Resizing, maximizing/restoring with the app's window button, menus, and
  125% scaling passed. The maximized window covered all four screen corners;
  restoring brought the rounded corners back. Native Wayland and Windows
  compositors have not been exercised for this change.
- An externally forced X11 fullscreen transition exposed a winit limitation:
  the backend did not update its fullscreen flag, so the corners stayed round.
  Normal maximize/restore and reported fullscreen-state changes are handled.
- The supplied icon is unchanged in this build. Inspection found alpha 128 in
  its outer black background, including all four corners. Two generative
  background-removal attempts changed the artwork and were rejected. An
  alpha-only correction of the source asset is still pending.

## Version 1.2.2 UI and icon update

- 39 tests passed; the opt-in SSH fixture test remained ignored in this run.
  New regression checks cover persistent navigator geometry without hover,
  split-pane membership and focus, overflow/new-tab visibility, sizing-pass
  action delivery, mouse input beneath overlays, and translucent light themes.
- Formatting and native/Windows GNU target Clippy passed with warnings denied.
  Both Linux and Windows GNU release builds completed.
- Native Linux UI checks ran in an isolated Xvfb session with disposable app
  data: nested splits, the three-pane picker, navigator visibility with the
  pointer outside the window, eleven tabs resized to a 760-pixel window, and
  light settings at 50% terminal opacity. The active tab remained visible and
  the navigator did not cover terminal text. Xvfb checks do not validate a
  desktop compositor's transparency or blur.
- A real GTK icon lookup failed for the old 1024-only layout and succeeded
  for the replacement standard hicolor layout. All eight exported PNG sizes
  were verified; the canonical PNG is byte-for-byte the supplied `icon.png`.
  The desktop file's startup class matches the X11 `WM_CLASS` observed with
  `xprop`: `dev.bmux.app`. The same ID is used for Wayland desktop matching.
- The app icon is rebuilt into both executables. The revised Linux package
  installs 16/24/32/48/64/128/256/512-pixel icons and a pixmaps fallback.

The frosted material is portable painting (tint, gradient, rim, and shadow),
not macOS Liquid Glass or native backdrop blur. No existing installation or
user workspace data was changed during these checks. Windows was cross-built,
not launched on Windows.

## Initial port and packaging checks

This record describes checks actually run in the development workspace, not
results inferred from the CI configuration.

Environment: Ubuntu 26.04 x86_64, Rust 1.98.1, isolated Xvfb display using Mesa
software rendering, tmux 3.6, OpenSSH 10.2 loopback fixture. Test data and SSH
keys were created under temporary directories, separate from real Bmux data.

- 29 default Rust tests passed; one opt-in SSH integration test is excluded
  from the default run and was run separately, successfully.
- Real PTY: input, terminal resize (`stty size`), process exit status, transcript.
- Real local tmux: shell/environment survival after detach, reattachment,
  restored old scrollback, explicit session termination.
- Real SSH: key-authenticated loopback login through the system SSH client,
  resize propagation, environment survival across transport teardown and
  reconnect, remote cwd metadata, explicit remote session termination.
- GUI checked in the isolated display: ANSI-colored output, typing, sidebar,
  workspace navigation, tabs, nested splits, history, and all settings
  categories. Geometry regression tests cover the settings window.
- Settings/model tests cover themes and contrast, font validation, migration
  from Swift JSON, corrupt-file backups, identity preservation, and split
  collapse. Input tests verify app shortcuts preserve terminal Control keys
  and avoid duplicate Ctrl+C bytes.
- Linux and Windows-target Clippy run with warnings denied; formatting checked.
- Linux release build and Windows GNU cross-build completed. The Windows PE
  includes the original-derived icon and DPI manifest; DLL inspection showed
  only Windows system imports.

Windows was cross-compiled on Linux, not launched on a Windows machine.
Windows ConPTY behavior, WSL, MSVC builds, IME, accessibility, GPU/hardware
compositors and high-DPI/multi-monitor behavior still need native testing.
The loopback SSH fixture uses key authentication; interactive passwords,
encrypted private-key prompts, jump hosts, and long network outages were not
exercised. See PORTING.md for feature differences from Ghostty/SwiftUI.

The locally built Linux archive targets the build machine's glibc. Build from
source on an older distribution, or use an artifact built on that distribution;
the included CI targets Ubuntu 24.04 for its Linux builds.

To rerun the optional SSH test against a disposable host with tmux installed:

```sh
BMUX_TEST_SSH_COMMAND='ssh your-test-host' cargo test --locked actual_ssh -- --ignored --nocapture
```

The test uses a randomly named session on the isolated `bmux-v1` tmux server
and removes only its own session. Use an authorized testing destination.

## Installer packaging

The following additional checks were run on the same Linux development host:

- `packaging/build-deb.sh` completed a Cargo release build and produced
  `dist/bmux_1.2.1-1_amd64.deb`. Packaging an existing binary also passed.
- Shell syntax and ShellCheck 0.11.0 passed for the Debian script.
- Debian control fields, root ownership, executable permissions, and all
  packaged MD5 checksums were verified after extraction. The executable,
  original icon, and desktop launcher match the source files byte for byte.
  `desktop-file-validate` passed, as did an APT installation simulation.
- Debian invalid-argument, missing-file, non-ELF, and wrong-architecture
  rejection checks passed. Running outside the repository with an output
  directory containing spaces also worked.
- PowerShell 7.6.6 ran `packaging/build-windows-installer.ps1` with the existing
  x64 GNU Windows executable. NSIS 3.10 compiled
  `dist/bmux-1.2.1-windows-x86_64-setup.exe` with warnings treated as errors.
  The installer archive passed `7z t`; its executable and original icon match
  their inputs after extraction.
- Windows packaging checks passed for paths containing spaces, custom Cargo
  target directories, invalid executables, unsupported targets, and missing
  tools. A failed installer compilation preserved the previous installer and
  removed the temporary output directory.
- The Windows CI installation test parses successfully and refuses to run
  outside a Windows GitHub Actions runner. The workflow builds both installers
  and runs the Windows install/upgrade/uninstall test, including user-data
  preservation, when CI is triggered.

No package was installed on this development machine. Native Windows execution
of the installer, the MSVC build, and the CI installation test have **not** been
run locally. The Windows installer built here contains the GNU cross-build;
the default Windows build script uses MSVC with a static C runtime.

The local Debian package correctly declares `libc6 (>= 2.43)` for this host's
binary. Build on the oldest Debian/Ubuntu release you want to support to obtain
a package with that release's library requirements. Local ARM64 package builds
have not been exercised.

#!/usr/bin/env bash
# Build a native Debian/Ubuntu package without needing root or cargo-deb.
set -euo pipefail
umask 022

usage() {
    cat <<'EOF'
Usage: packaging/build-deb.sh [--binary PATH] [--output-dir PATH]

Build bmux in release mode and create dist/bmux_VERSION_ARCH.deb.
Run on Debian/Ubuntu, on the architecture and oldest distribution you support.

  --binary PATH      Package an existing native release binary; skip compilation.
  --output-dir PATH  Output directory (default: repository dist/).
  -h, --help         Show this help.

Relative argument paths are resolved from the caller's working directory.
DEB_MAINTAINER overrides the local-build identity in the package control file.
DEB_REVISION overrides the Debian revision (default: 1).
SOURCE_DATE_EPOCH is honored when creating package timestamps.
EOF
}

die() { printf 'Error: %s\n' "$*" >&2; exit 1; }

repo=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)
binary=
output="$repo/dist"
while (($#)); do
    case "$1" in
        --binary|--output-dir)
            (($# >= 2)) || die "$1 requires a path"
            if [[ "$1" == --binary ]]; then
                binary=$(realpath -m -- "$2")
            else
                output=$(realpath -m -- "$2")
            fi
            shift 2
            ;;
        -h|--help) usage; exit 0 ;;
        *) die "Unknown argument: $1 (use --help)" ;;
    esac
done

[[ $(uname -s) == Linux ]] || die 'Build the .deb on Debian or Ubuntu Linux.'
for tool in cargo python3 dpkg dpkg-deb dpkg-shlibdeps install md5sum; do
    command -v "$tool" >/dev/null || die "Missing $tool. Install Rust and the packages documented in README.md."
done
python3 -c 'import PIL.Image' 2>/dev/null || die 'Missing Python Pillow. Install python3-pil (apt) or pillow (pip); see README.md.'
architecture=$(dpkg --print-architecture)
case "$architecture" in
    amd64|arm64) ;;
    *) die "Unsupported build architecture: $architecture (supported: amd64, arm64)." ;;
esac

maintainer=${DEB_MAINTAINER:-'Bmux local build <local@localhost>'}
[[ "$maintainer" != *$'\n'* && "$maintainer" != *$'\r'* && "$maintainer" == *' <'*'@'*'>' ]] ||
    die 'DEB_MAINTAINER must be a single line: Your Name <you@example.com>'
revision=${DEB_REVISION:-1}
[[ "$revision" =~ ^[0-9A-Za-z][0-9A-Za-z.+~]*$ ]] || die 'Invalid DEB_REVISION.'

work=$(mktemp -d "${TMPDIR:-/tmp}/bmux-deb.XXXXXXXX")
trap 'rm -rf -- "$work"' EXIT
cd "$repo"
cargo metadata --no-deps --format-version 1 --locked > "$work/metadata.json"
version=$(python3 - "$work/metadata.json" <<'PY'
import json
import sys

with open(sys.argv[1], encoding="utf-8") as stream:
    package = next(p for p in json.load(stream)["packages"] if p["name"] == "bmux")
# Debian prereleases sort before the corresponding stable release.
print(package["version"].replace("-", "~", 1))
PY
)
version="$version-$revision"
dpkg --validate-version "$version"

if [[ -z "$binary" ]]; then
    # Cargo reports the actual artifact path, including custom target directories
    # and configured build targets. Compiler progress still appears on stderr.
    cargo build --release --locked --bin bmux --message-format=json-render-diagnostics > "$work/build.json"
    binary=$(python3 - "$work/build.json" <<'PY'
import json
import sys

executables = []
with open(sys.argv[1], encoding="utf-8") as stream:
    for line in stream:
        message = json.loads(line)
        if message.get("reason") == "compiler-artifact" and message["target"]["name"] == "bmux" and message.get("executable"):
            executables.append(message["executable"])
if not executables:
    sys.exit("Cargo did not report a bmux executable.")
print(executables[-1])
PY
)
fi
[[ -f "$binary" ]] || die "Binary not found: $binary"
python3 - "$binary" "$architecture" <<'PY'
import struct
import sys

with open(sys.argv[1], "rb") as stream:
    header = stream.read(64)
expected_machine = {"amd64": 62, "arm64": 183}[sys.argv[2]]
if len(header) < 64 or header[:6] != b"\x7fELF\x02\x01" or struct.unpack_from("<H", header, 18)[0] != expected_machine:
    sys.exit("The binary must be a native 64-bit Linux ELF for " + sys.argv[2] + ". Cross-packaging is not supported.")
PY

stage="$work/package"
install -Dm755 "$binary" "$stage/usr/bin/bmux"
install -Dm644 packaging/dev.bmux.app.desktop "$stage/usr/share/applications/dev.bmux.app.desktop"
# Regenerate icons from the canonical PNG, auto-repairing a regressed alpha
# baseline so the app menu never ships a semi-transparent black square.
# This refreshes the checked-in assets (bmux.png, hicolor PNGs, bmux.ico);
# a clean tree stays byte-identical apart from mtimes.
python3 packaging/export-icon.py --remove-alpha-floor auto
for size in 16 24 32 48 64 128 256 512; do
    icon="${size}x${size}/apps/dev.bmux.app.png"
    install -Dm644 "assets/icons/hicolor/$icon" "$stage/usr/share/icons/hicolor/$icon"
done
install -Dm644 assets/bmux.png "$stage/usr/share/pixmaps/dev.bmux.app.png"
doc="$stage/usr/share/doc/bmux"
mkdir -p "$doc" "$stage/DEBIAN" "$work/debian"
install -m644 README.md THIRD_PARTY_NOTICES.md "$doc/"
install -m644 docs/PORTING.md docs/VALIDATION.md "$doc/"
install -m644 assets/themes.LICENSE "$doc/themes.LICENSE"
install -m644 assets/themes.source.json "$doc/themes.source.json"
install -m644 Assets/Fonts/OFL.txt "$doc/GeistMono-OFL.txt"
install -m644 THIRD_PARTY_NOTICES.md "$doc/copyright"

# dpkg-shlibdeps needs a source control file and the build host's library
# metadata. Fail if it cannot determine dependencies; never guess the libc floor.
cat > "$work/debian/control" <<EOF
Source: bmux
Section: utils
Priority: optional
Maintainer: $maintainer

Package: bmux
Architecture: $architecture
Description: Workspace-oriented terminal
EOF
shlibs=$(cd "$work" && dpkg-shlibdeps -O -e"$stage/usr/bin/bmux")
dependencies=$(printf '%s\n' "$shlibs" | sed -n 's/^shlibs:Depends=//p')
[[ -n "$dependencies" ]] || die 'dpkg-shlibdeps did not report runtime dependencies.'

# Winit/glutin open these at runtime, so ELF dependency scanning cannot see them.
dependencies+=", libx11-6, libx11-xcb1, libxcb1, libxcursor1, libxi6, libxkbcommon0, libxkbcommon-x11-0, libwayland-client0, libwayland-cursor0, libwayland-egl1, libegl1, libgl1, hicolor-icon-theme"
installed_size=$(du -sk "$stage/usr" | cut -f1)
cat > "$stage/DEBIAN/control" <<EOF
Package: bmux
Version: $version
Section: utils
Priority: optional
Architecture: $architecture
Maintainer: $maintainer
Installed-Size: $installed_size
Depends: $dependencies
Recommends: openssh-client, tmux (>= 3.2), xdg-utils, xdg-desktop-portal, xdg-desktop-portal-gtk | xdg-desktop-portal-kde
Homepage: https://github.com/imbencetsx/bmux
Description: A calm, workspace-oriented terminal
 Native Rust terminal with workspaces, tabs, split panes, SSH/tmux sessions,
 persistent settings, and the original bmux app icon.
EOF

# Desktop and icon cache refreshes are handled by the distribution's file
# triggers. The package owns no user data and needs no maintainer scripts.
(
    cd "$stage"
    find usr -type f -print0 | LC_ALL=C sort -z | xargs -0 md5sum > DEBIAN/md5sums
)
mkdir -p "$output"
package="$output/bmux_${version}_${architecture}.deb"
# Build in the temporary directory first, so a failed build cannot replace an
# earlier successful package. Root ownership in the archive needs no sudo.
dpkg-deb --root-owner-group -Zxz --build "$stage" "$work/bmux.deb"
install -m644 "$work/bmux.deb" "$package"
printf '\nCreated %s\nInstall with: sudo apt install "%s"\n' "$package" "$package"

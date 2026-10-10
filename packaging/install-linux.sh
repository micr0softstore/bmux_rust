#!/bin/sh
# Install this checkout (or a Linux release archive) into a user-owned prefix.
set -eu
cd "$(dirname "$0")/.."

prefix="${1:-$HOME/.local}"
case "$prefix" in
    /*) ;;
    *) echo "Installation prefix must be an absolute path." >&2; exit 2 ;;
esac

if [ -f ./bmux ]; then
    binary=./bmux
else
    cargo build --release --locked
    binary=target/release/bmux
fi

install -Dm755 "$binary" "$prefix/bin/bmux"
# Use directories declared by the standard hicolor theme. A 1024-only icon is
# ignored by themes whose index stops at 512, including common Debian desktops.
for size in 16 24 32 48 64 128 256 512; do
    icon="${size}x${size}/apps/dev.bmux.app.png"
    install -Dm644 "assets/icons/hicolor/$icon" "$prefix/share/icons/hicolor/$icon"
done
install -Dm644 assets/bmux.png "$prefix/share/pixmaps/dev.bmux.app.png"
mkdir -p "$prefix/share/applications" "$prefix/share/doc/bmux"
install -m644 README.md THIRD_PARTY_NOTICES.md assets/themes.LICENSE Assets/Fonts/OFL.txt "$prefix/share/doc/bmux/"

# Desktop Exec is not a shell command; escape its reserved characters.
desktop_binary=$(printf '%s' "$prefix/bin/bmux" | sed 's/[\\`"$]/\\&/g; s/%/%%/g')
while IFS= read -r line; do
    case "$line" in
        Exec=*) printf 'Exec="%s"\n' "$desktop_binary" ;;
        TryExec=*) printf 'TryExec=%s\n' "$prefix/bin/bmux" ;;
        *) printf '%s\n' "$line" ;;
    esac
done < packaging/dev.bmux.app.desktop > "$prefix/share/applications/dev.bmux.app.desktop"

if command -v update-desktop-database >/dev/null 2>&1; then
    update-desktop-database "$prefix/share/applications" || true
fi
if command -v gtk-update-icon-cache >/dev/null 2>&1; then
    gtk-update-icon-cache -f -t "$prefix/share/icons/hicolor" >/dev/null 2>&1 || true
fi
printf 'Installed %s/bin/bmux\n' "$prefix"
printf 'Launch it from your application menu or run "%s/bin/bmux".\n' "$prefix"

#!/bin/sh
set -eu
cd "$(dirname "$0")/.."
cargo build --release --locked

version=$(sed -n 's/^version = "\([^"]*\)".*/\1/p' Cargo.toml | head -n 1)
name="bmux-$version-linux-$(uname -m)"
stage="dist/$name"
mkdir -p "$stage/assets" "$stage/Assets/Fonts" "$stage/packaging" "$stage/docs"
install -m755 target/release/bmux "$stage/bmux"
install -m755 packaging/install-linux.sh "$stage/packaging/install-linux.sh"
install -m644 packaging/dev.bmux.app.desktop "$stage/packaging/"
install -m644 assets/bmux.png assets/themes.LICENSE assets/themes.source.json "$stage/assets/"
cp -R assets/icons "$stage/assets/"
install -m644 Assets/Fonts/OFL.txt "$stage/Assets/Fonts/"
install -m644 README.md THIRD_PARTY_NOTICES.md "$stage/"
install -m644 docs/PORTING.md docs/VALIDATION.md "$stage/docs/"
tar -C dist -czf "dist/$name.tar.gz" "$name"
printf 'Created dist/%s.tar.gz\n' "$name"

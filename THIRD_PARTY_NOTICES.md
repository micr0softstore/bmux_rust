# Third-party materials

- The original bmux source comes from <https://github.com/imbencetsx/bmux>.
  `assets/bmux.png` is the exact PNG supplied by the user as `icon.png` for
  this port. `assets/bmux.ico` and `assets/icons/hicolor/` contain only resized
  platform exports of that PNG, preserving its artwork and transparency.
- `Assets/Fonts/GeistMono-Regular.ttf` and `GeistMono-SemiBold.ttf` are
  distributed under the SIL Open Font License in `Assets/Fonts/OFL.txt`.
- `assets/themes.json` contains the Ghostty-format palette collection from
  <https://github.com/mbadolato/iTerm2-Color-Schemes>, commit
  `c02052159ff9a1c438ff419b7451cd33f9ab4934`. The collection's MIT license is
  reproduced in `assets/themes.LICENSE`; the individual theme authors retain
  their copyrights. `assets/themes.source.json` records the import provenance.
- Rust dependency versions and sources are recorded in `Cargo.lock`.
  Dependency licenses remain with their respective packages. Use
  `cargo metadata --format-version 1` to inspect package license metadata.

#!/usr/bin/env python3
"""Import the data-only Ghostty theme catalog from a local upstream checkout.

Usage: python3 packaging/import-themes.py /path/to/iTerm2-Color-Schemes
No network or code execution from upstream is performed by this script.
"""

import json
from pathlib import Path
import re
import shutil
import subprocess
import sys

ROOT = Path(__file__).resolve().parent.parent


def main():
    source = Path(sys.argv[1]).resolve()
    commit = subprocess.check_output(["git", "-C", str(source), "rev-parse", "HEAD"], text=True).strip()
    themes = []
    color_pattern = re.compile(r"^#[0-9a-fA-F]{6}$")
    for path in sorted((source / "ghostty").iterdir(), key=lambda p: p.name.casefold()):
        if not path.is_file() or path.name.startswith("."):
            continue
        values, palette = {}, {}
        for line in path.read_text().splitlines():
            if "=" not in line or line.lstrip().startswith("#"):
                continue
            key, value = [part.strip() for part in line.split("=", 1)]
            if key == "palette":
                index, color = value.split("=", 1)
                palette[int(index)] = color.strip().upper()
            else:
                values[key] = value.upper()
        theme = {
            "name": path.name,
            "background": values["background"],
            "foreground": values["foreground"],
            "cursorColor": values.get("cursor-color", values["foreground"]),
            "selectionBackground": values.get("selection-background", values["foreground"]),
            "selectionForeground": values.get("selection-foreground", values["background"]),
            "palette": [palette[index] for index in range(16)],
        }
        colors = [v for k, v in theme.items() if k not in ("name", "palette")] + theme["palette"]
        if not all(color_pattern.fullmatch(v) for v in colors):
            raise ValueError(f"Invalid colors in {path.name}")
        themes.append(theme)
    output = ROOT / "assets"
    output.mkdir(exist_ok=True)
    (output / "themes.json").write_text(json.dumps(themes, ensure_ascii=False, separators=(",", ":")) + "\n")
    shutil.copyfile(source / "LICENSE", output / "themes.LICENSE")
    (output / "themes.source.json").write_text(json.dumps({
        "repository": "https://github.com/mbadolato/iTerm2-Color-Schemes",
        "commit": commit,
        "directory": "ghostty",
        "themeCount": len(themes),
        "format": "Ghostty theme colors, normalized into JSON; no upstream executable code",
    }, indent=2) + "\n")
    print(f"Imported {len(themes)} themes from {commit}")


if __name__ == "__main__":
    main()

#!/usr/bin/env python3
"""Export platform icon formats from the supplied PNG without redrawing it.

Requires Pillow only when regenerating the checked-in exports. With an optional
source argument, that PNG is copied byte-for-byte to assets/bmux.png. Without an
argument, the existing canonical PNG is kept untouched. Colors, transparency,
shape, and padding are preserved; only resolution and file format change.
The explicit --remove-alpha-floor option additionally repairs an incorrect
alpha baseline without changing RGB pixels; normal exports never apply it.
Use --remove-alpha-floor auto to detect the baseline from the source image
(minimum alpha > 0) instead of supplying a known value; packaging scripts use
this so a regressed source can never ship a semi-transparent black square.
"""

import argparse
import shutil
from pathlib import Path

from PIL import Image

ROOT = Path(__file__).resolve().parent.parent
OUT = ROOT / "assets"
PNG_SIZES = (16, 24, 32, 48, 64, 128, 256, 512)
ICO_SIZES = (16, 24, 32, 48, 64, 128, 256)


def alpha_floor_table(floor):
    """Map a known alpha baseline to zero while keeping opaque pixels opaque."""
    if not 0 <= floor < 255:
        raise ValueError("the alpha floor must be an integer from 0 to 254")
    span = 255 - floor
    return [max(0, (alpha - floor) * 255 + span // 2) // span for alpha in range(256)]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("source", nargs="?", type=Path, default=OUT / "bmux.png")
    parser.add_argument(
        "--remove-alpha-floor",
        metavar="0..254|auto",
        help="remap this known alpha floor to 0 and 255 to 255, preserving RGB; "
        "'auto' detects the floor as the source minimum alpha and repairs only when it is above 0",
    )
    args = parser.parse_args()
    with Image.open(args.source) as source:
        if source.format != "PNG" or source.width != source.height:
            parser.error("the source must be a square PNG")
        image = source.convert("RGBA")
    floor = None
    if args.remove_alpha_floor is not None:
        raw = args.remove_alpha_floor
        if isinstance(raw, str) and raw.lower() == "auto":
            floor = image.getchannel("A").getextrema()[0]
            if floor == 0:
                print("Alpha baseline is already 0; exporting without repair")
            else:
                print(f"Detected alpha floor {floor}; remapping it to 0")
        else:
            try:
                floor = int(raw)
            except (TypeError, ValueError):
                parser.error("the alpha floor must be an integer from 0 to 254 or 'auto'")
    alpha_table = None
    if floor is not None and floor != 0:
        try:
            alpha_table = alpha_floor_table(floor)
        except ValueError as error:
            parser.error(str(error))
    if alpha_table is not None:
        alpha = image.getchannel("A")
        if alpha.getextrema()[0] < floor:
            parser.error("source alpha is below the supplied floor; refusing to repair an incompatible or already corrected image")
        image.putalpha(alpha.point(alpha_table))

    OUT.mkdir(exist_ok=True)
    canonical = OUT / "bmux.png"
    if alpha_table is not None:
        image.save(canonical)
    elif args.source.resolve() != canonical.resolve():
        shutil.copyfile(args.source, canonical)

    for size in PNG_SIZES:
        destination = OUT / "icons/hicolor" / f"{size}x{size}/apps/dev.bmux.app.png"
        destination.parent.mkdir(parents=True, exist_ok=True)
        image.resize((size, size), Image.Resampling.LANCZOS).save(destination)
    image.save(OUT / "bmux.ico", sizes=[(size, size) for size in ICO_SIZES])
    print(f"Exported {len(PNG_SIZES)} Linux icon sizes and Windows ICO from {canonical}")


if __name__ == "__main__":
    main()

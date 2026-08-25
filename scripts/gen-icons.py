# Regenerates every raster icon from the SVG sources in assets/brand/
# (docs/15-DESIGN.md, brand section; decision K-251).
#
# In plain terms: the SVGs are the only artwork anyone edits. The operating
# systems want pixels, not drawings — Windows wants one .ico holding several
# PNG sizes — so this script renders each SVG at every needed size and packs
# the results. Each size is rendered straight from the SVG (not scaled down
# from a big render), which is what keeps the small sizes crisp.
#
# The macOS APPLICATION icon is NOT made here: it is the layered Icon Composer
# document assets/brand/lumit-icon.icon, compiled by Xcode (K-309). Everything
# below is Windows, Linux, and the document icons.
#
#   pip install resvg-py pillow
#   python scripts/gen-icons.py
#
# Outputs (all committed):
#   flutter_ui/windows/runner/resources/app_icon.ico   <- lumit-mark.svg (bare)
#   assets/brand/lumit-project.ico                     <- lumit-project.svg (.lum)
#   assets/brand/lumit-preset.ico                      <- lumit-preset.svg (.lumfx)
#   assets/brand/lumit-theme.ico                       <- lumit-theme.svg (.lumtheme)
#   packaging/macos/lumit-project.icns, lumit-preset.icns, lumit-theme.icns
#                                                      <- the same three SVGs
#
# The Windows installer (packaging/windows/lumit.iss) registers the .ico files
# with the .lum/.lumfx/.lumtheme associations. The .icns files are resources of
# the macOS Runner target, referenced in place from packaging/macos/, which is
# where Info.plist's CFBundleTypeIconFile entries look for them.
#
# One honest caveat on the .icns: Pillow's ICNS writer takes a single image and
# derives the smaller sizes itself, so unlike the .ico files above these are
# downscaled from the 1024 render rather than drawn fresh at each size.

import io
from pathlib import Path

import resvg_py
from PIL import Image

ROOT = Path(__file__).resolve().parent.parent
BRAND = ROOT / "assets" / "brand"

ICO_SIZES = [256, 128, 64, 48, 32, 24, 16]


def render(svg_path: Path, size: int) -> Image.Image:
    png = bytes(resvg_py.svg_to_bytes(svg_string=svg_path.read_text(), width=size))
    return Image.open(io.BytesIO(png)).convert("RGBA")


def write_ico(svg_path: Path, out: Path) -> None:
    frames = [render(svg_path, s) for s in ICO_SIZES]
    frames[0].save(
        out,
        format="ICO",
        append_images=frames[1:],
        sizes=[(s, s) for s in ICO_SIZES],
    )
    print(f"{out.relative_to(ROOT)}: {ICO_SIZES}")


def main() -> None:
    write_ico(
        BRAND / "lumit-mark.svg",
        ROOT / "flutter_ui" / "windows" / "runner" / "resources" / "app_icon.ico",
    )
    write_ico(BRAND / "lumit-project.svg", BRAND / "lumit-project.ico")
    write_ico(BRAND / "lumit-preset.svg", BRAND / "lumit-preset.ico")
    write_ico(BRAND / "lumit-theme.svg", BRAND / "lumit-theme.ico")

    for name in ["lumit-project", "lumit-preset", "lumit-theme"]:
        out = ROOT / "packaging" / "macos" / f"{name}.icns"
        render(BRAND / f"{name}.svg", 1024).save(out, format="ICNS")
        print(f"{out.relative_to(ROOT)}")


if __name__ == "__main__":
    main()

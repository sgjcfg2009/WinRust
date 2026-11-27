"""Generates Windows 11 Fluent app.ico files from fluentui-system-icons-main SVGs.
Requires: Pillow, resvg-py
"""
import io
import os
import re
import struct
from pathlib import Path
from PIL import Image
import resvg_py

ROOT = Path(__file__).resolve().parent.parent
FLUENT_ASSETS = Path(os.environ.get("FLUENT_ICONS_DIR") or ROOT.parent / "fluentui-system-icons-main" / "assets")
M = 1024
PAD = 64
SIZES = [16, 24, 32, 48, 64, 128, 256]

# (relative_app_dir, asset_subpath, optional (top_hex, bottom_hex) gradient for monochrome _filled.svg)
APPS = [
    ("launcher/app.ico", r"Apps\SVG\ic_fluent_apps_48_color.svg", None),
    ("apps/rust-settings.ico", r"Settings\SVG\ic_fluent_settings_48_color.svg", None),
    ("apps/photoview.ico", r"Image\SVG\ic_fluent_image_48_color.svg", None),
    ("apps/rtaskmgr-win32.ico", r"Gauge\SVG\ic_fluent_gauge_32_color.svg", None),
    ("rust-search/app.ico", r"Search Sparkle\SVG\ic_fluent_search_sparkle_48_color.svg", None),
    ("apps/rust-explorer.ico", r"Folder\SVG\ic_fluent_folder_48_filled.svg", ("#FCE100", "#F59E0B")),
    ("apps/topbar.ico", r"Panel Bottom\SVG\ic_fluent_panel_bottom_20_filled.svg", ("#60CDFF", "#0078D4")),
    ("apps/debloat.ico", r"Broom\SVG\ic_fluent_broom_32_filled.svg", ("#34D399", "#059669")),
    ("apps/purge.ico", r"Delete\SVG\ic_fluent_delete_48_filled.svg", ("#FB7185", "#E11D48")),
    ("apps/rs-shell.ico", r"Window Console\SVG\ic_fluent_window_console_20_filled.svg", ("#38BDF8", "#0284C7")),
    ("apps/reshell.ico", r"Desktop\SVG\ic_fluent_desktop_48_filled.svg", ("#818CF8", "#2563EB")),
]


def apply_gradient(svg_text: str, top_hex: str, bottom_hex: str) -> str:
    defs = (
        '<defs><linearGradient id="w11g" x1="0%" y1="0%" x2="0%" y2="100%">'
        f'<stop offset="0%" stop-color="{top_hex}"/>'
        f'<stop offset="100%" stop-color="{bottom_hex}"/>'
        "</linearGradient></defs>"
    )
    svg_text = re.sub(r'fill="#[0-9a-fA-F]{3,8}"', 'fill="url(#w11g)"', svg_text)
    svg_text = re.sub(r'fill="currentColor"', 'fill="url(#w11g)"', svg_text)
    svg_text = re.sub(r"(<svg[^>]*>)", r"\1" + defs, svg_text, count=1)
    return svg_text


def render_svg(svg_path: Path, grad) -> Image.Image:
    svg_text = svg_path.read_text(encoding="utf-8")
    if grad is not None:
        svg_text = apply_gradient(svg_text, grad[0], grad[1])
    inner = M - 2 * PAD
    png_bytes = bytes(resvg_py.svg_to_bytes(svg_string=svg_text, width=inner, height=inner))
    glyph = Image.open(io.BytesIO(png_bytes)).convert("RGBA")
    canvas = Image.new("RGBA", (M, M), (0, 0, 0, 0))
    canvas.paste(glyph, (PAD, PAD), glyph)
    return canvas


def frame_bytes(img: Image.Image, s: int) -> bytes:
    buf = io.BytesIO()
    img.resize((s, s), Image.LANCZOS).save(buf, "PNG")
    return buf.getvalue()


def write_ico(path: Path, img: Image.Image) -> None:
    frames = [(s, frame_bytes(img, s)) for s in SIZES]
    head = struct.pack("<HHH", 0, 1, len(frames))
    off = 6 + 16 * len(frames)
    entries, data = b"", b""
    for s, b in frames:
        entries += struct.pack("<BBBBHHII", s % 256, s % 256, 0, 0, 1, 32, len(b), off + len(data))
        data += b
    path.write_bytes(head + entries + data)


def main() -> None:
    missing = [FLUENT_ASSETS / asset_rel for _, asset_rel, _ in APPS if not (FLUENT_ASSETS / asset_rel).is_file()]
    if missing:
        raise FileNotFoundError(f"Missing Fluent SVG: {missing[0]}")

    for old_ico in ROOT.rglob("*.ico"):
        old_ico.unlink()
        print("deleted:", old_ico.relative_to(ROOT))

    for rel, asset_rel, grad in APPS:
        svg_path = FLUENT_ASSETS / asset_rel
        img = render_svg(svg_path, grad)
        out_ico = ROOT / rel
        write_ico(out_ico, img)
        print("wrote:", out_ico.relative_to(ROOT), "<-", asset_rel)


if __name__ == "__main__":
    main()

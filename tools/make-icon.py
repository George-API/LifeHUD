"""Generates the app icon: assets/icon.png (512 px, for the README and Settings) and
assets/lifehud.ico (16–256 px, for the exe, Start menu, taskbar and tray).

Run from the repo root:  python tools/make-icon.py   (needs Pillow)

The mark is the HUD itself: a bar across the top and two widget tiles beneath it, on a rounded
square. Everything is drawn once at 1024 px and downsampled, so every size is crisp, including the
16 px tray icon and 4K Start menus.
"""
from pathlib import Path

from PIL import Image, ImageChops, ImageDraw

S = 1024  # master canvas
ASSETS = Path("assets")

# Deep blue to violet, diagonally.
TOP_LEFT = (64, 112, 255)
BOTTOM_RIGHT = (124, 77, 255)


def rounded_mask(box, radius):
    m = Image.new("L", (S, S), 0)
    ImageDraw.Draw(m).rounded_rectangle(box, radius=radius, fill=255)
    return m


def diagonal_gradient(a, b):
    """a at the top-left corner to b at the bottom-right."""
    ramp = Image.linear_gradient("L").resize((S, S))  # 0 at the top, 255 at the bottom
    diag = ImageChops.add(ramp, ramp.rotate(90), scale=2)  # mixes vertical and horizontal
    return Image.composite(Image.new("RGB", (S, S), b), Image.new("RGB", (S, S), a), diag)


def master():
    inset = round(S * 0.055)  # Windows icons leave a little air around the tile
    box = (inset, inset, S - inset, S - inset)
    radius = round(S * 0.235)
    shape = rounded_mask(box, radius)

    icon = Image.new("RGBA", (S, S), (0, 0, 0, 0))
    icon.paste(diagonal_gradient(TOP_LEFT, BOTTOM_RIGHT), (0, 0), shape)

    # A soft sheen across the top half, and a little depth at the bottom.
    sheen = Image.linear_gradient("L").resize((S, S)).point(lambda v: max(0, 44 - v * 44 // 150))
    icon = Image.alpha_composite(icon, Image.merge("RGBA", (*[Image.new("L", (S, S), 255)] * 3, ImageChops.multiply(sheen, shape))))
    shade = Image.linear_gradient("L").resize((S, S)).point(lambda v: max(0, (v - 150) * 40 // 105))
    icon = Image.alpha_composite(icon, Image.merge("RGBA", (*[Image.new("L", (S, S), 0)] * 3, ImageChops.multiply(shade, shape))))

    # Drawn on their own layer and blended in (ImageDraw replaces pixels; it doesn't blend).
    glyph = Image.new("RGBA", (S, S), (0, 0, 0, 0))
    d = ImageDraw.Draw(glyph)
    # Hairline inner edge, so the tile reads on light and dark backgrounds alike.
    edge = round(S * 0.006)
    d.rounded_rectangle(box, radius=radius, outline=(255, 255, 255, 46), width=edge)

    # The HUD: a slim bar, and two frosted widget tiles.
    left, right = S * 0.235, S * 0.765
    bar_top, bar_h = S * 0.27, S * 0.095
    d.rounded_rectangle((left, bar_top, right, bar_top + bar_h), radius=bar_h / 2, fill=(255, 255, 255, 255))
    gap = S * 0.05
    tile_top, tile_bottom = S * 0.435, S * 0.735
    mid = S / 2
    tile_r = S * 0.065
    for x0, x1 in ((left, mid - gap / 2), (mid + gap / 2, right)):
        d.rounded_rectangle((x0, tile_top, x1, tile_bottom), radius=tile_r, fill=(255, 255, 255, 120))
    return Image.alpha_composite(icon, glyph)


def main():
    big = master()
    big.resize((512, 512), Image.Resampling.LANCZOS).save(ASSETS / "icon.png", optimize=True)
    sizes = [16, 20, 24, 32, 40, 48, 64, 96, 128, 256]
    big.resize((256, 256), Image.Resampling.LANCZOS).save(
        ASSETS / "lifehud.ico", format="ICO", sizes=[(s, s) for s in sizes]
    )
    print("wrote assets/icon.png (512 px) and assets/lifehud.ico (" + ", ".join(map(str, sizes)) + " px)")


if __name__ == "__main__":
    main()

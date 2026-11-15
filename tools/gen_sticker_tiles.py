"""Generates ui/stickers/tile-<set>.svg: one seamless repeating tile per mood theme.

Run from the repo root:  python tools/gen_sticker_tiles.py

Cards draw the pattern as a single tiled Image (colorized to the theme ink), not one element per
motif, so redraws under it stay cheap. Everything that makes the pattern feel hand-placed is baked
in here, because the software renderer can't rotate or scale at runtime:

- Four motifs per theme, placed sparsely by seeded Poisson-disc sampling on a torus, with generous
  clear space. The tile is large and irregular, so the repeat is hard to spot, and neighbouring
  motifs are never the same kind.
- Scale hierarchy and depth: mostly small motifs with the occasional larger one, each size
  jittered slightly. Larger motifs are fainter, so the pattern stays a quiet texture with no
  focal point. Turns are gentle and cluster near upright.
- Tiny, dim accent shapes (dots, sparkles, leaves, ...) add rhythm in the gaps.
- Tonal depth: colorize keeps only alpha, which would flatten each sticker to a silhouette. Each
  motif is drawn through a luminance mask instead. The die-cut border is light, and each part's
  tone follows its original colour (darker colours are denser), so details like the book's pages
  or the tram's windows read as a soft emboss.

Output is deterministic: each theme's layout is seeded by its name.
"""
import math
import random
import re
from pathlib import Path

STICKERS = Path("ui/stickers")
# Theme sticker set -> its four motifs. Order matches `with_stickers` in src/theme.rs (1-based).
SETS = {
    "hearth": ["hearth-mug", "hearth-candle", "hearth-book", "hearth-cookie"],
    "atelier": ["atelier-palette", "atelier-brush", "atelier-pencil", "atelier-tube"],
    "metro": ["metro-skyline", "metro-lamp", "metro-tram", "metro-sign"],
    "evergreen": ["evergreen-pine", "evergreen-mushroom", "evergreen-leaf", "evergreen-mountain"],
    "neon": ["neon-bolt", "neon-heart", "neon-note", "neon-sparkle"],
    "coast": ["coast-shell", "coast-sail", "coast-wave", "coast-star"],
}

# Accent shapes in a 64-unit box centred on (32, 32).
ACCENT_SHAPES = {
    "dot": '<circle cx="32" cy="32" r="20"/>',
    "ring": '<circle cx="32" cy="32" r="18" fill="none" stroke="#fff" stroke-width="9"/>',
    "plus": '<path d="M32 10v44M10 32h44" stroke="#fff" stroke-width="14" stroke-linecap="round"/>',
    "sparkle": '<path d="M32 2C34 24 40 30 62 32C40 34 34 40 32 62C30 40 24 34 2 32C24 30 30 24 32 2z"/>',
    "square": '<rect x="14" y="14" width="36" height="36" rx="8"/>',
    "leaf": '<path d="M10 54C10 26 26 10 54 10C54 38 38 54 10 54z"/>',
    "heart": '<path d="M32 56C14 42 6 32 6 22a13 13 0 0 1 26-4 13 13 0 0 1 26 4c0 10-8 20-26 34z"/>',
}
ACCENTS = {
    "hearth": ["dot", "heart"],
    "atelier": ["dot", "ring", "plus"],
    "metro": ["square", "dot"],
    "evergreen": ["leaf", "dot"],
    "neon": ["plus", "sparkle", "ring"],
    "coast": ["dot", "ring"],
}

# A larger tile with fewer, more spaced-out motifs reads as texture rather than wallpaper, and
# makes the repeat hard to spot.
TILE_W, TILE_H = 360, 272
# Scale hierarchy: mostly small motifs, an occasional larger one. (px, weight)
MOTIF_SIZES = [(16, 3), (19, 4), (22, 3), (26, 2), (33, 1)]
MOTIF_COUNT, ACCENT_COUNT = 17, 14
MOTIF_GAP, ACCENT_GAP = 20, 11  # minimum clear space around each element, px
ROTATION = 16  # max turn in degrees; most land near upright
# (dx, dy, px) from the tile centre: fits the ~110x50 px window a Settings swatch shows.
SWATCH_CLUSTER = [(-34, 6, 19), (4, -10, 22), (38, 8, 17)]
BORDER_TONE = 0.42  # alpha of the die-cut border relative to the motif body


def hex_to_lum(hex_colour):
    h = hex_colour.lstrip("#")
    if len(h) == 3:
        h = "".join(c * 2 for c in h)
    r, g, b = (int(h[i : i + 2], 16) / 255 for i in (0, 2, 4))
    return 0.2126 * r + 0.7152 * g + 0.0722 * b


def grey(v):
    v = round(max(0.0, min(1.0, v)) * 255)
    return f"#{v:02x}{v:02x}{v:02x}"


def tone(hex_colour):
    """Darker source colours become denser ink; light ones stay soft."""
    return grey(1.0 - 0.5 * hex_to_lum(hex_colour.group(0)))


def load(name):
    svg = (STICKERS / f"{name}.svg").read_text(encoding="utf-8")
    body = re.sub(r"^\s*<svg[^>]*>", "", svg).rsplit("</svg>", 1)[0]
    end = body.index("</g>") + len("</g>")
    outline, colour = body[:end], body[end:]
    colours = re.compile(r"#[0-9a-fA-F]{6}\b|#[0-9a-fA-F]{3}\b")
    mask = colours.sub(grey(BORDER_TONE), outline) + colours.sub(tone, colour)
    return outline, mask


def torus_dist(a, b):
    dx = abs(a[0] - b[0])
    dy = abs(a[1] - b[1])
    return math.hypot(min(dx, TILE_W - dx), min(dy, TILE_H - dy))


def layout(rng, kinds, accents):
    placed = []  # (x, y, size, kind, is_motif)
    counts = {k: 0 for k in range(kinds)}
    sizes = [s for s, w in MOTIF_SIZES for _ in range(w)]

    def free(x, y, size, gap):
        return all(torus_dist((x, y), (px, py)) >= (size + ps) / 2 + gap for px, py, ps, _, _ in placed)

    # A small composed cluster at the tile's centre: the Settings theme swatches centre the tile,
    # so each always shows three distinct motifs rather than whatever lands in a tiny window.
    for kind, (dx, dy, size) in enumerate(SWATCH_CLUSTER[:kinds]):
        placed.append((TILE_W / 2 + dx, TILE_H / 2 + dy, size, kind, True))
        counts[kind] += 1

    for target, is_motif in ((MOTIF_COUNT - len(placed), True), (ACCENT_COUNT, False)):
        added = 0
        for _ in range(20000):
            if added == target:
                break
            x, y = rng.uniform(0, TILE_W), rng.uniform(0, TILE_H)
            if is_motif:
                size = round(rng.choice(sizes) * rng.uniform(0.94, 1.06), 1)
                if not free(x, y, size, MOTIF_GAP):
                    continue
                near = {k for px, py, _, k, m in placed if m and torus_dist((x, y), (px, py)) < 62}
                options = [k for k in range(kinds) if k not in near] or list(range(kinds))
                low = min(counts[k] for k in options)
                kind = rng.choice([k for k in options if counts[k] == low])
                counts[kind] += 1
            else:
                size = rng.uniform(3.5, 6)
                if not free(x, y, size, ACCENT_GAP):
                    continue
                kind = rng.randrange(len(accents))
            placed.append((x, y, size, kind, is_motif))
            added += 1
    return placed


def tile(name, motifs):
    rng = random.Random(name)
    parts = [load(m) for m in motifs]
    accents = ACCENTS[name]
    defs = []
    for i, (_, mask) in enumerate(parts):
        defs.append(
            f'<mask id="m{i}" maskUnits="userSpaceOnUse" x="-4" y="-4" width="72" height="72">{mask}</mask>'
        )
    out = []
    for x, y, size, kind, is_motif in layout(rng, len(parts), accents):
        deg = rng.triangular(-ROTATION, ROTATION, 0)
        if is_motif:
            # Depth: larger motifs sit further back (fainter), so no single shape shouts.
            smallest, largest = MOTIF_SIZES[0][0], MOTIF_SIZES[-1][0]
            recede = (size - smallest) / (largest - smallest)
            alpha = rng.uniform(0.7, 1.0) * (1 - 0.35 * max(0.0, min(1.0, recede)))
        else:
            alpha = rng.uniform(0.3, 0.6)
        if is_motif:
            outline = parts[kind][0]
            inner = (
                f'<svg x="{{x:.1f}}" y="{{y:.1f}}" width="{size:.1f}" height="{size:.1f}" viewBox="-4 -4 72 72">'
                f'<g mask="url(#m{kind})">{outline}</g></svg>'
            )
        else:
            inner = (
                f'<svg x="{{x:.1f}}" y="{{y:.1f}}" width="{size:.1f}" height="{size:.1f}" viewBox="0 0 64 64" '
                f'fill="#fff">{ACCENT_SHAPES[accents[kind]]}</svg>'
            )
        # Wrapped copies so anything crossing an edge continues seamlessly into the next tile.
        reach = size * 0.75
        for dx in (-TILE_W, 0, TILE_W):
            for dy in (-TILE_H, 0, TILE_H):
                cx, cy = x + dx, y + dy
                if cx + reach < 0 or cx - reach > TILE_W or cy + reach < 0 or cy - reach > TILE_H:
                    continue
                left, top = cx - size / 2, cy - size / 2
                out.append(
                    f'<g opacity="{alpha:.2f}" transform="rotate({deg:.1f} {cx:.1f} {cy:.1f})">'
                    + inner.format(x=left, y=top)
                    + "</g>"
                )
    return (
        f'<svg xmlns="http://www.w3.org/2000/svg" width="{TILE_W}" height="{TILE_H}" '
        f'viewBox="0 0 {TILE_W} {TILE_H}">'
        "<!-- Generated by tools/gen_sticker_tiles.py. Do not edit by hand. -->"
        f"<defs>{''.join(defs)}</defs>" + "".join(out) + "</svg>\n"
    )


for name, motifs in SETS.items():
    (STICKERS / f"tile-{name}.svg").write_text(tile(name, motifs), encoding="utf-8")
    print(f"wrote ui/stickers/tile-{name}.svg")

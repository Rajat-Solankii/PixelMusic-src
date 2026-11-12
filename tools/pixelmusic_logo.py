#!/usr/bin/env python3
"""Builds the Pixel Music logo assets from the Android vector drawables in docs/brand/pixelmusic/.

    python tools/pixelmusic_logo.py            # write everything
    python tools/pixelmusic_logo.py --compare DIR   # also print the difference to DIR/<name>.png (512 px references)

Source, per colourway (the adaptive icon of the PixelMusic Android app):
    launcher_bg_<name>.xml      <shape> with a linear gradient (the background layer)
    ic_pixelmusic_<name>.xml    <vector>, viewport 940 x 940 = 108 dp (the foreground layer)

An adaptive icon shows only the middle 72 dp of its 108 dp layers, so the SVG keeps that window
(viewport 156.67 .. 783.33) and puts it on a rounded square instead of the launcher mask.

Output:
    src/materialfy/assets/logo/<name>.svg           the logo (sidebar, window icon, Settings swatches)
    src/materialfy/assets/logo/<name>-<16|24|32>.png  tray icons
    src/materialfy/assets/materialfy.svg            = blue_purple.svg (default window icon)
    src/materialfy/assets/tray-<black|white>-<16|24|32>.png  one-colour silhouette (macOS template icon)
    crates/sf-auth/src/logo.svg                     = blue_purple.svg (the OAuth "close this tab" page)
    materialfy.ico (16..256), materialfy.png (512)  exe / installer icon, from blue_purple

The PNG and ICO files are rasterised here with Pillow (the paths are plain polygons with linear gradients),
4x supersampled. Needs: pip install pillow numpy.
"""
import re
import sys
import xml.etree.ElementTree as ET
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
SRC = ROOT / "docs" / "brand" / "pixelmusic"
ASSETS = ROOT / "src" / "materialfy" / "assets"
A = "{http://schemas.android.com/apk/res/android}"
AAPT = "{http://schemas.android.com/aapt}"

# id, title, background file suffix, foreground file suffix
COLORWAYS = [
    ("blue_purple", "Blue Purple", "blue_purple", "blue_purple"),
    ("baby_pink_purple", "Baby Pink Purple", "baby_pink_purple", "baby_pink_purple"),
    ("graphite", "Graphite Gray", "graphite", "graphite_gray"),
    ("lime_green", "Lime Green", "lime_green", "lime_green"),
    ("lime_lemon", "Lime Lemon", "lime_lemon", "lime_lemon_yellow"),
    ("orange_yellow", "Orange Yellow", "orange_yellow", "orange_yellow"),
]
DEFAULT = "blue_purple"

VIEWPORT = 940.0
SAFE = VIEWPORT * 72 / 108          # visible part of an adaptive icon
ORIGIN = (VIEWPORT - SAFE) / 2
RADIUS = 0.225                      # corner radius of the rounded square, share of the side


def num(v):
    return float(v)


def fmt(v):
    return f"{v:.2f}".rstrip("0").rstrip(".")


def android_color(c):
    """#AARRGGBB / #RRGGBB -> (#RRGGBB, opacity)"""
    c = c.lstrip("#")
    if len(c) == 8:
        return "#" + c[2:].upper(), int(c[:2], 16) / 255
    return "#" + c.upper(), 1.0


def parse_background(path):
    g = ET.parse(path).getroot().find("gradient")
    angle = int(g.get(A + "angle", "0")) % 360
    # GradientDrawable: 0 = left to right, 90 = bottom to top, 135 = bottom-right to top-left
    vec = {0: (0, .5, 1, .5), 45: (0, 1, 1, 0), 90: (.5, 1, .5, 0), 135: (1, 1, 0, 0),
           180: (1, .5, 0, .5), 225: (1, 0, 0, 1), 270: (.5, 0, .5, 1), 315: (0, 0, 1, 1)}[angle]
    return {"vec": vec, "stops": [(0.0, g.get(A + "startColor")), (1.0, g.get(A + "endColor"))]}


def parse_polygons(d):
    """pathData of these drawables is only 'M x,y x,y ... Z' subpaths"""
    if re.search(r"[^MZ0-9., \-]", d):
        raise SystemExit("unexpected path command in " + d[:60])
    out = []
    for sub in re.findall(r"M([^MZ]*)Z", d):
        n = [float(x) for x in re.findall(r"-?\d+(?:\.\d+)?", sub)]
        out.append(list(zip(n[0::2], n[1::2])))
    return out


def parse_foreground(path):
    """-> list of {"d", "polys", "matrix": (sx, sy, tx, ty), "fill": colour | gradient dict}"""
    root = ET.parse(path).getroot()
    assert num(root.get(A + "viewportWidth")) == VIEWPORT and num(root.get(A + "viewportHeight")) == VIEWPORT
    shapes = []

    def walk(node, m):
        for child in node:
            if child.tag == "group":
                sx, sy = num(child.get(A + "scaleX", "1")), num(child.get(A + "scaleY", "1"))
                tx, ty = num(child.get(A + "translateX", "0")), num(child.get(A + "translateY", "0"))
                # no pivot or rotation in these files: point' = scale * point + translate, then the parent
                walk(child, (m[0] * sx, m[1] * sy, m[0] * tx + m[2], m[1] * ty + m[3]))
            elif child.tag == "path":
                fill = child.get(A + "fillColor")
                attr = child.find(AAPT + "attr")
                if attr is not None:
                    g = attr.find("gradient")
                    assert g.get(A + "type") == "linear"
                    fill = {"vec": tuple(num(g.get(A + k)) for k in ("startX", "startY", "endX", "endY")),
                            "stops": [(num(i.get(A + "offset")), i.get(A + "color")) for i in g.findall("item")]}
                d = child.get(A + "pathData")
                shapes.append({"name": child.get(A + "name"), "d": d, "polys": parse_polygons(d), "matrix": m, "fill": fill})

    walk(root, (1.0, 1.0, 0.0, 0.0))
    return shapes


# ---------------------------------------------------------------- SVG

def svg_path_data(polys):
    return "".join("M" + " ".join(f"{fmt(x)},{fmt(y)}" for x, y in p) + "Z" for p in polys)


def build_svg(title, bg, shapes):
    """viewBox 0..SAFE; foreground paths keep their own coordinates under the group transforms of the drawable"""
    s = fmt(SAFE)
    defs, body = [], []
    x1, y1, x2, y2 = (v * SAFE for v in bg["vec"])
    stops = "".join(f'<stop offset="{fmt(o)}" stop-color="{android_color(c)[0]}"/>' for o, c in bg["stops"])
    defs.append(f'<linearGradient id="bg" gradientUnits="userSpaceOnUse" x1="{fmt(x1)}" y1="{fmt(y1)}" x2="{fmt(x2)}" y2="{fmt(y2)}">{stops}</linearGradient>')
    body.append(f'<rect width="{s}" height="{s}" rx="{fmt(SAFE * RADIUS)}" fill="url(#bg)"/>')
    for i, sh in enumerate(shapes):
        sx, sy, tx, ty = sh["matrix"]
        transform = f'matrix({fmt(sx)} 0 0 {fmt(sy)} {fmt(tx - ORIGIN)} {fmt(ty - ORIGIN)})'
        if isinstance(sh["fill"], dict):
            gx1, gy1, gx2, gy2 = sh["fill"]["vec"]
            stops = "".join(f'<stop offset="{fmt(o)}" stop-color="{android_color(c)[0]}"/>' for o, c in sh["fill"]["stops"])
            defs.append(f'<linearGradient id="g{i}" gradientUnits="userSpaceOnUse" x1="{fmt(gx1)}" y1="{fmt(gy1)}" x2="{fmt(gx2)}" y2="{fmt(gy2)}">{stops}</linearGradient>')
            fill = f'fill="url(#g{i})"'
        else:
            color, opacity = android_color(sh["fill"])
            fill = f'fill="{color}"' + (f' fill-opacity="{fmt(opacity)}"' if opacity < 1 else "")
        body.append(f'<path transform="{transform}" {fill} d="{svg_path_data(sh["polys"])}"/>')
    return (f'<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 {s} {s}" width="512" height="512">'
            f'<title>Pixel Music — {title}</title>\n<defs>{"".join(defs)}</defs>\n' + "\n".join(body) + "\n</svg>\n")


# ---------------------------------------------------------------- raster (Pillow)

def rgb(c):
    c = android_color(c)[0].lstrip("#")
    return [int(c[i:i + 2], 16) for i in (0, 2, 4)]


def gradient_image(size, vec, stops):
    """RGB image of a linear gradient; vec in pixels"""
    import numpy as np
    from PIL import Image
    x1, y1, x2, y2 = vec
    ys, xs = np.mgrid[0:size, 0:size].astype(np.float32) + 0.5
    dx, dy = x2 - x1, y2 - y1
    t = np.clip(((xs - x1) * dx + (ys - y1) * dy) / (dx * dx + dy * dy), 0, 1)
    offs = [o for o, _ in stops]
    cols = np.array([rgb(c) for _, c in stops], dtype=np.float32)
    out = np.stack([np.interp(t, offs, cols[:, k]) for k in range(3)], axis=-1)
    return Image.fromarray(out.round().astype("uint8"), "RGB")


def to_px(sh, scale):
    sx, sy, tx, ty = sh["matrix"]
    return [[((x * sx + tx - ORIGIN) * scale, (y * sy + ty - ORIGIN) * scale) for x, y in p] for p in sh["polys"]]


def render(bg, shapes, size, square=False):
    """RGBA logo, size x size; square=True — without rounded corners (to compare with the reference PNGs)"""
    from PIL import Image, ImageDraw
    ss = 4 if size >= 128 else 8
    n = size * ss
    scale = n / SAFE
    img = Image.new("RGBA", (n, n), (0, 0, 0, 0))
    mask = Image.new("L", (n, n), 0)
    draw = ImageDraw.Draw(mask)
    if square:
        draw.rectangle([0, 0, n, n], fill=255)
    else:
        draw.rounded_rectangle([0, 0, n - 1, n - 1], radius=n * RADIUS, fill=255)
    img.paste(gradient_image(n, [v * n for v in bg["vec"]], bg["stops"]), (0, 0), mask)
    for sh in shapes:
        mask = Image.new("L", (n, n), 0)
        draw = ImageDraw.Draw(mask)
        for poly in to_px(sh, scale):
            draw.polygon(poly, fill=255)
        if isinstance(sh["fill"], dict):
            sx, sy, tx, ty = sh["matrix"]
            gx1, gy1, gx2, gy2 = sh["fill"]["vec"]
            vec = [(gx1 * sx + tx - ORIGIN) * scale, (gy1 * sy + ty - ORIGIN) * scale,
                   (gx2 * sx + tx - ORIGIN) * scale, (gy2 * sy + ty - ORIGIN) * scale]
            layer = gradient_image(n, vec, sh["fill"]["stops"]).convert("RGBA")
        else:
            color, opacity = android_color(sh["fill"])
            layer = Image.new("RGBA", (n, n), tuple(rgb(color)) + (255,))
            mask = mask.point(lambda v: int(v * opacity))
        layer.putalpha(mask)
        img = Image.alpha_composite(img, layer)
    return img.resize((size, size), Image.LANCZOS)


def render_silhouette(shapes, size, color):
    """one-colour mark with the note cut out (tray template)"""
    from PIL import Image, ImageChops, ImageDraw
    ss = 8
    n = size * ss
    # the mark alone, a little larger than in the launcher icon: fit its bounding box into the canvas
    pts = [pt for sh in shapes for poly in to_px(sh, 1.0) for pt in poly]
    x0, y0 = min(p[0] for p in pts), min(p[1] for p in pts)
    x1, y1 = max(p[0] for p in pts), max(p[1] for p in pts)
    k = n * 0.94 / max(x1 - x0, y1 - y0)
    ox, oy = (n - (x1 - x0) * k) / 2 - x0 * k, (n - (y1 - y0) * k) / 2 - y0 * k
    body = Image.new("L", (n, n), 0)
    note = Image.new("L", (n, n), 0)
    for sh in shapes:
        if sh["name"] == "music_note_shadow":
            continue
        target = note if sh["name"] == "music_note" else body
        draw = ImageDraw.Draw(target)
        for poly in to_px(sh, 1.0):
            draw.polygon([(x * k + ox, y * k + oy) for x, y in poly], fill=255)
    alpha = ImageChops.subtract(body, note).resize((size, size), Image.LANCZOS)
    img = Image.new("RGBA", (size, size), color + (0,))
    img.putalpha(alpha)
    return img


def compare(img, ref_path):
    """mean absolute difference per channel (0..255) against a reference PNG"""
    import numpy as np
    from PIL import Image
    ref = Image.open(ref_path).convert("RGB").resize(img.size, Image.LANCZOS)
    a = np.asarray(img.convert("RGB"), dtype=np.float32)
    b = np.asarray(ref, dtype=np.float32)
    return float(np.abs(a - b).mean())


def main():
    ref_dir = Path(sys.argv[sys.argv.index("--compare") + 1]) if "--compare" in sys.argv else None
    logo_dir = ASSETS / "logo"
    logo_dir.mkdir(exist_ok=True)
    for name, title, bg_name, fg_name in COLORWAYS:
        bg = parse_background(SRC / f"launcher_bg_{bg_name}.xml")
        shapes = parse_foreground(SRC / f"ic_pixelmusic_{fg_name}.xml")
        svg = build_svg(title, bg, shapes)
        (logo_dir / f"{name}.svg").write_text(svg, encoding="utf-8", newline="\n")
        for size in (16, 24, 32):
            render(bg, shapes, size).save(logo_dir / f"{name}-{size}.png", optimize=True)
        if ref_dir:
            print(f"{name}: mean abs diff to reference = {compare(render(bg, shapes, 512, square=True), ref_dir / (name + '.png')):.2f}")
        if name == DEFAULT:
            (ASSETS / "materialfy.svg").write_text(svg, encoding="utf-8", newline="\n")
            (ROOT / "crates" / "sf-auth" / "src" / "logo.svg").write_text(svg, encoding="utf-8", newline="\n")
            render(bg, shapes, 512).save(ROOT / "materialfy.png", optimize=True)
            sizes = [16, 24, 32, 48, 64, 128, 256]
            frames = [render(bg, shapes, s) for s in sizes]
            frames[-1].save(ROOT / "materialfy.ico", format="ICO", sizes=[(s, s) for s in sizes], append_images=frames[:-1])
            for size in (16, 24, 32):
                render_silhouette(shapes, size, (0, 0, 0)).save(ASSETS / f"tray-black-{size}.png", optimize=True)
                render_silhouette(shapes, size, (255, 255, 255)).save(ASSETS / f"tray-white-{size}.png", optimize=True)
    print("done")


if __name__ == "__main__":
    main()

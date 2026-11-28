#!/usr/bin/env python3
"""The test sheet printed over security backgrounds (#322): a measure set for
tools/ocr-accuracy.py.

    python3 -I tools/ocr-sheet/patterns.py OUT_DIR
    python3 -I tools/ocr-accuracy.py OUT_DIR/manifest.toml --work WORK

Renders sheet.pdf's pages at 300 dpi with Poppler's pdftoppm and prints each over made-up
backgrounds, as security printing lays text over them: crossing waves (light, dense and dark), a
guilloche rosette, a dot screen with VOID in larger dots, grey fine print (3-point text) and
microprint (under a point, as on cheques), waves over a colour gradient, and the plain page to
compare. Each is saved as it is (PNG), as a phone scan (a little blur, uneven light, noise and
JPEG) and as a phone app's document look (the contrast pushed).
OUT_DIR/manifest.toml lists them all with the sheet's true text. Needs numpy and Pillow.
"""
import argparse
import io
import math
import pathlib
import shutil
import subprocess
import tempfile

import numpy as np
from PIL import Image, ImageDraw, ImageFilter, ImageFont

DPI = 300
HERE = pathlib.Path(__file__).resolve().parent
FONTS = HERE.parent.parent / "crates" / "limarepdf-core" / "assets" / "fonts"


def blank(size):
    return Image.new("RGB", size, "white")


def waves(size, colour, spacing, amp, period, width, phase_step):
    """Two families of sine curves crossing, as on cheques and certificates."""
    w, h = size
    im = blank(size)
    d = ImageDraw.Draw(im)
    xs = np.arange(0, w + 8, 4, dtype=float)
    for sign in (1, -1):
        for k, y0 in enumerate(range(-amp * 2, h + amp * 2, spacing)):
            ys = y0 + sign * amp * np.sin(2 * math.pi * xs / period + k * phase_step)
            d.line(list(zip(xs.tolist(), ys.tolist())), fill=colour, width=width)
    return im


def rosette(size, colour, width, rings=60):
    """Wavy rings round the middle of the page: a guilloche rosette."""
    w, h = size
    im = blank(size)
    d = ImageDraw.Draw(im)
    t = np.linspace(0, 2 * math.pi, 4000)
    for r in range(rings):
        rr = 80 + r * 28 + 22 * np.sin(18 * t + r * 0.35)
        xs, ys = w / 2 + rr * np.cos(t), h / 2 + rr * np.sin(t)
        d.line(list(zip(xs.tolist(), ys.tolist())), fill=colour, width=width)
    return im


def small_text(size, colour, px, pitch):
    """Rows of text `px` pixels high, `pitch` apart."""
    w, h = size
    im = blank(size)
    d = ImageDraw.Draw(im)
    font = ImageFont.truetype(str(FONTS / "LiberationSans-Regular.ttf"), px)
    line = "SECURITY DOCUMENT ORIGINAL " * (w // px)
    for y in range(0, h, pitch):
        d.text((-(y * 7) % 300, y), line, fill=colour, font=font)
    return im


def void_screen(size, colour):
    """A dot screen whose dots are larger inside a big VOID, as on copy-evident paper."""
    w, h = size
    mask = Image.new("L", size, 0)
    font = ImageFont.truetype(str(FONTS / "LiberationSans-Bold.ttf"), 700)
    ImageDraw.Draw(mask).text((w * 0.12, h * 0.3), "VOID", fill=255, font=font)
    yy, xx = np.mgrid[0:h, 0:w]
    small = ((xx % 6) < 1) & ((yy % 6) < 1)
    big = ((xx % 6) < 2) & ((yy % 6) < 2)
    a = np.full((h, w, 3), 255, np.uint8)
    a[np.where(np.asarray(mask) > 0, big, small)] = colour
    return Image.fromarray(a)


def gradient(size):
    """A pale colour running across the page, as on many certificates."""
    w, h = size
    x = np.linspace(0, 1, w)[None, :, None]
    row = np.array([255, 230, 215], float) * (1 - x) + np.array([210, 235, 255], float) * x
    return Image.fromarray(np.broadcast_to(row, (h, w, 3)).astype(np.uint8))


def printed(*layers):
    """The layers printed over each other: each darkens what is under it."""
    out = np.ones_like(np.asarray(layers[0], float))
    for im in layers:
        out *= np.asarray(im, float) / 255
    return Image.fromarray((out * 255).round().astype(np.uint8))


def phone(im, seed=1):
    """A phone scan: a little blur, uneven light, noise and JPEG."""
    rng = np.random.default_rng(seed)
    a = np.asarray(im.filter(ImageFilter.GaussianBlur(0.8)), float)
    h, w = a.shape[:2]
    yy, xx = np.mgrid[0:h, 0:w]
    light = 1.0 - 0.10 * ((xx / w - 0.3) ** 2 + (yy / h - 0.2) ** 2)
    a = a * light[..., None] + rng.normal(0, 4, a.shape)
    buf = io.BytesIO()
    Image.fromarray(a.clip(0, 255).astype(np.uint8)).save(buf, "JPEG", quality=80)
    return Image.open(io.BytesIO(buf.getvalue())).convert("RGB")


def enhanced(im, seed=1):
    """A phone app's document look: the phone scan with the paper pushed to white."""
    a = np.asarray(phone(im, seed), float)
    return Image.fromarray(((a - 40) * 255 / (215 - 40)).clip(0, 255).astype(np.uint8))


def backgrounds(size):
    return {
        "plain": blank(size),
        "waves-green": waves(size, (150, 200, 160), 12, 9, 140, 2, 0.6),
        "waves-blue-dense": waves(size, (165, 195, 230), 7, 6, 90, 1, 0.9),
        "waves-dark": waves(size, (110, 150, 120), 12, 9, 140, 2, 0.6),
        "rosette-pink": rosette(size, (225, 160, 185), 2),
        "fine-print": small_text(size, (170, 170, 170), 13, 18),
        "microprint": small_text(size, (140, 140, 140), 4, 6),
        "void-screen": void_screen(size, (150, 150, 150)),
        "gradient-waves": printed(gradient(size), waves(size, (175, 185, 215), 10, 8, 120, 2, 0.5)),
    }


def main():
    ap = argparse.ArgumentParser(description="The test sheet over security backgrounds; see the file's docstring.")
    ap.add_argument("out", type=pathlib.Path)
    out = ap.parse_args().out.resolve()
    out.mkdir(parents=True, exist_ok=True)
    cases = []
    with tempfile.TemporaryDirectory() as tmp:
        render = pathlib.Path(tmp) / "page"
        subprocess.run(["pdftoppm", "-r", str(DPI), "-png", HERE / "sheet.pdf", render], check=True)
        for n, render in enumerate(sorted(pathlib.Path(tmp).glob("page-*.png")), 1):
            shutil.copy(HERE / f"page-{n}.txt", out / f"page-{n}.txt")
            sheet = Image.open(render).convert("RGB")
            for name, background in backgrounds(sheet.size).items():
                page = printed(sheet, background)
                captures = (("clean", page, "png"), ("phone", phone(page), "jpg"), ("enhanced", enhanced(page), "jpg"))
                for capture, im, ext in captures:
                    path = f"page-{n}/{name}-{capture}.{ext}"
                    (out / path).parent.mkdir(exist_ok=True)
                    im.save(out / path, dpi=(DPI, DPI), **({"quality": 95} if ext == "jpg" else {}))
                    truth = f'truth = ["page-{n}.txt"]\n'
                    cases.append(f'[[case]]\nname = "p{n} {name} {capture}"\ninput = "{path}"\n{truth}')
    (out / "manifest.toml").write_text("\n".join(cases), encoding="utf-8")
    print(f"{len(cases)} pages, listed in {out / 'manifest.toml'}")


if __name__ == "__main__":
    main()

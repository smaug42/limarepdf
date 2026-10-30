"""Make the made-up font the text pipeline's tests fall back to: tests/data/fonts/LimareHanTest.ttf.

Usage, from the repository: python3 crates/limarepdf-core/tools/make_han_test_font.py

"Limare Han Test" is a TrueType font with a few Chinese, Japanese and Korean characters
(plain boxes, each of its own shape) and no Latin letters but A, so LimarePDF doesn't offer
it in font lists: text in another font falls back to it for those characters
(`src/layout.rs`), on any machine, with or without installed fonts for them. Latin A and
Cyrillic А (U+0410) share one glyph, as in many fonts, so the tests show each reads back as
itself.

The output is the same on every run (fixed timestamps). Needs fontTools.
"""

from pathlib import Path

from fontTools.fontBuilder import FontBuilder
from fontTools.pens.ttGlyphPen import TTGlyphPen

OUT = Path(__file__).resolve().parents[1] / "tests" / "data" / "fonts" / "LimareHanTest.ttf"
EM = 1000
# 2024-01-01, in seconds since 1904 as `head` gives dates.
WHEN = 3786825600
# The characters it has: Chinese and Japanese ideographs, kana and Hangul.
CJK = "漢字中文日本語国あいう한국어"


def box(x0, y0, x1, y1):
    """A glyph that is a filled rectangle."""
    pen = TTGlyphPen(None)
    pen.moveTo((x0, y0))
    pen.lineTo((x0, y1))
    pen.lineTo((x1, y1))
    pen.lineTo((x1, y0))
    pen.closePath()
    return pen.glyph()


def empty():
    return TTGlyphPen(None).glyph()


def main():
    glyphs = {".notdef": (box(50, 0, 450, 700), 500), "space": (empty(), 250)}
    cmap = {0x20: "space"}
    for i, c in enumerate(CJK):
        name = f"uni{ord(c):04X}"
        # Each its own shape, so a glyph drawn for another shows.
        glyphs[name] = (box(50 + 10 * i, -100, 950 - 10 * i, 800), 1000)
        cmap[ord(c)] = name
    glyphs["A"] = (box(60, 0, 540, 700), 600)
    cmap[0x41] = "A"
    cmap[0x410] = "A"

    order = list(glyphs)
    fb = FontBuilder(EM, isTTF=True)
    fb.setupGlyphOrder(order)
    fb.setupCharacterMap(cmap)
    fb.setupGlyf({n: g for n, (g, _) in glyphs.items()})
    metrics = {}
    for n, (g, w) in glyphs.items():
        g.recalcBounds(fb.font["glyf"])
        metrics[n] = (w, getattr(g, "xMin", 0))
    fb.setupHorizontalMetrics(metrics)
    fb.setupHorizontalHeader(ascent=800, descent=-200)
    fb.setupNameTable(
        {
            "familyName": "Limare Han Test",
            "styleName": "Regular",
            "uniqueFontIdentifier": "LimareHanTest-Regular",
            "fullName": "Limare Han Test",
            "psName": "LimareHanTest-Regular",
            "version": "Version 1.000",
        }
    )
    # Embedding allowed (fsType 0), as LimarePDF writes only such fonts.
    fb.setupOS2(
        fsType=0,
        sTypoAscender=800,
        sTypoDescender=-200,
        usWinAscent=800,
        usWinDescent=200,
        sCapHeight=700,
        sxHeight=500,
        achVendID="NONE",
    )
    fb.setupPost()
    fb.setupHead(unitsPerEm=EM, created=WHEN, modified=WHEN)
    fb.font.recalcTimestamp = False
    fb.save(str(OUT))
    print(OUT)


if __name__ == "__main__":
    main()

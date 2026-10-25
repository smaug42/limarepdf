"""Make the made-up font the emoji tests use: tests/data/fonts/LimareEmojiTest.ttf.

Usage, from the repository: python3 crates/limarepdf-core/tools/make_emoji_test_font.py

"Limare Emoji Test" is a TrueType font with the Latin letters, digits and space (plain boxes, so
LimarePDF offers it as an installed family) and these emoji (BUG-24):

- U+1F600 (grinning face): a glyph with an outline, which LimarePDF writes;
- U+1F60E (smiling face with sunglasses): drawn by colour layers only (COLR and CPAL), its own
  glyph empty, so a PDF would show nothing: refused;
- the parts of emoji sequences, each with a glyph of its own (U+1F44D and the skin tone U+1F3FD,
  U+1F468, U+1F469 and the zero-width joiner U+200D, the regional indicators U+1F1EC and U+1F1E7,
  the keycap U+20E3 and the emoji variation selector U+FE0F), so the tests show sequences are
  refused by rule, not for want of a glyph.

The output is the same on every run (fixed timestamps). Needs fontTools.
"""

from pathlib import Path

from fontTools.colorLib.builder import buildCOLR, buildCPAL
from fontTools.fontBuilder import FontBuilder
from fontTools.pens.ttGlyphPen import TTGlyphPen

OUT = Path(__file__).resolve().parents[1] / "tests" / "data" / "fonts" / "LimareEmojiTest.ttf"
EM = 1000
# 2024-01-01, in seconds since 1904 as `head` gives dates.
WHEN = 3786825600


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
    for c in [*range(0x41, 0x5B), *range(0x61, 0x7B), *range(0x30, 0x3A)]:
        name = f"uni{c:04X}"
        top = 700 if c < 0x61 else 500
        glyphs[name] = (box(60, 0, 540, top), 600)
        cmap[c] = name
    outlined = [0x1F600, 0x1F44D, 0x1F3FD, 0x1F468, 0x1F469, 0x1F1EC, 0x1F1E7, 0x20E3]
    for c in outlined:
        name = f"u{c:04X}"
        glyphs[name] = (box(50, -100, 950, 800), 1000)
        cmap[c] = name
    # Marks that join or vary emoji take no room, as in real fonts.
    for c in [0x200D, 0xFE0F]:
        name = f"uni{c:04X}"
        glyphs[name] = (empty(), 0)
        cmap[c] = name
    # The colour-only emoji: an empty glyph of its own, drawn by two layers.
    glyphs["u1F60E"] = (empty(), 1000)
    glyphs["u1F60E.face"] = (box(50, -100, 950, 800), 1000)
    glyphs["u1F60E.glasses"] = (box(150, 400, 850, 550), 1000)
    cmap[0x1F60E] = "u1F60E"

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
            "familyName": "Limare Emoji Test",
            "styleName": "Regular",
            "uniqueFontIdentifier": "LimareEmojiTest-Regular",
            "fullName": "Limare Emoji Test",
            "psName": "LimareEmojiTest-Regular",
            "version": "Version 1.000",
        }
    )
    # Embedding allowed (fsType 0), as LimarePDF offers only such fonts.
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
    fb.font["COLR"] = buildCOLR({"u1F60E": [("u1F60E.face", 0), ("u1F60E.glasses", 1)]})
    fb.font["CPAL"] = buildCPAL([[(1.0, 0.8, 0.2, 1.0), (0.1, 0.1, 0.1, 1.0)]])
    fb.font.recalcTimestamp = False
    fb.save(str(OUT))
    print(OUT)


if __name__ == "__main__":
    main()

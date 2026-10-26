"""Make the made-up fonts the tests of fonts' own encodings use (PB-23), in tests/data/fonts/:

- LimareOwnEncoding.pfa: a Type 1 font whose built-in encoding puts its glyphs where TeX's
  OT1 fonts (CMR10 and kin) do: quotedblright at 0x22, quotedblleft at 0x5C, endash at 0x7B,
  emdash at 0x7C, the ligatures ff and fi at 0x0B and 0x0C, and A to C where ASCII has them;
- LimareOwnEncoding.cff: the same glyphs and encoding as a bare CFF program (`/FontFile3`
  `/Subtype /Type1C`);
- LimareSymbolic.ttf: a symbolic TrueType font with the same glyphs, reached by a (3,0) cmap
  (codes at 0xF000 and up) and a (1,0) one, its glyph names in `post`.

Usage, from the repository: python3 crates/limarepdf-core/tools/make_encoding_test_fonts.py

Every glyph is a plain box. The output is the same on every run (fixed timestamps, no random
bytes). Needs fontTools.
"""

import io
from pathlib import Path

from fontTools.fontBuilder import FontBuilder
from fontTools.misc import eexec
from fontTools.pens.t2CharStringPen import T2CharStringPen
from fontTools.pens.ttGlyphPen import TTGlyphPen
from fontTools.ttLib.tables._c_m_a_p import CmapSubtable

OUT = Path(__file__).resolve().parents[1] / "tests" / "data" / "fonts"
EM = 1000
# 2024-01-01, in seconds since 1904 as `head` gives dates.
WHEN = 3786825600
WIDTH = 500

# Each code of the built-in encoding and the glyph there.
ENCODING = {
    0x0B: "ff",
    0x0C: "fi",
    0x22: "quotedblright",
    0x41: "A",
    0x42: "B",
    0x43: "C",
    0x5C: "quotedblleft",
    0x7B: "endash",
    0x7C: "emdash",
}
GLYPHS = [".notdef", *ENCODING.values()]


def t1_number(v):
    """A number in a Type 1 charstring."""
    if -107 <= v <= 107:
        return bytes([v + 139])
    if 108 <= v <= 1131:
        v -= 108
        return bytes([v // 256 + 247, v % 256])
    if -1131 <= v <= -108:
        v = -v - 108
        return bytes([v // 256 + 251, v % 256])
    return b"\xff" + v.to_bytes(4, "big", signed=True)


def t1_box():
    """A Type 1 charstring drawing a box 400 by 700, WIDTH wide, encrypted with 4 zero bytes."""
    ops = [
        (0, WIDTH, 13),  # hsbw
        (50, 0, 21),  # rmoveto
        (400, 0, 5),  # rlineto
        (0, 700, 5),
        (-400, 0, 5),
        (9,),  # closepath
        (14,),  # endchar
    ]
    out = b""
    for op in ops:
        *args, code = op
        out += b"".join(t1_number(a) for a in args) + bytes([code])
    data, _ = eexec.encrypt(b"\0\0\0\0" + out, 4330)
    return data


def type1():
    names = " ".join(f"dup {c} /{n} put" for c, n in ENCODING.items())
    clear = (
        "%!PS-AdobeFont-1.0: LimareOwnEncoding 001.000\n"
        "11 dict begin\n"
        "/FontInfo 2 dict dup begin /FullName (Limare Own Encoding) readonly def end readonly def\n"
        "/FontName /LimareOwnEncoding def\n"
        "/Encoding 256 array 0 1 255 {1 index exch /.notdef put} for\n"
        f"{names}\n"
        "readonly def\n"
        "/PaintType 0 def\n/FontType 1 def\n"
        "/FontMatrix [0.001 0 0 0.001 0 0] readonly def\n"
        "/FontBBox {0 0 500 700} readonly def\n"
        "currentdict end\ncurrentfile eexec\n"
    ).encode()
    box = t1_box()
    strings = b"".join(
        f"/{n} {len(box)} RD ".encode() + box + b" ND\n" for n in GLYPHS
    )
    private = (
        b"dup /Private 8 dict dup begin\n"
        b"/RD{string currentfile exch readstring pop}executeonly def\n"
        b"/ND{noaccess def}executeonly def\n"
        b"/NP{noaccess put}executeonly def\n"
        b"/MinFeature{16 16}def\n/password 5839 def\n/lenIV 4 def\n"
        b"/BlueValues [] def\n/Subrs 0 array\nND\n"
        + f"2 index /CharStrings {len(GLYPHS)} dict dup begin\n".encode()
        + strings
        + b"end\nend\nreadonly put\nnoaccess put\n"
        b"dup /FontName get exch definefont pop\nmark currentfile closefile\n"
    )
    encrypted, _ = eexec.encrypt(b"\0\0\0\0" + private, 55665)
    trailer = ("0" * 64 + "\n") * 8 + "cleartomark\n"
    return clear + encrypted + trailer.encode()


def cff():
    fb = FontBuilder(EM, isTTF=False)
    fb.setupGlyphOrder(GLYPHS)
    strings = {}
    for n in GLYPHS:
        pen = T2CharStringPen(WIDTH, None)
        pen.moveTo((50, 0))
        pen.lineTo((50, 700))
        pen.lineTo((450, 700))
        pen.lineTo((450, 0))
        pen.closePath()
        strings[n] = pen.getCharString()
    fb.setupCFF("LimareOwnEncoding", {"FullName": "Limare Own Encoding"}, strings, {})
    font = fb.font["CFF "].cff
    encoding = [".notdef"] * 256
    for c, n in ENCODING.items():
        encoding[c] = n
    font.topDictIndex[0].Encoding = encoding
    out = io.BytesIO()
    font.compile(out, fb.font)
    return out.getvalue()


def truetype():
    fb = FontBuilder(EM, isTTF=True)
    fb.setupGlyphOrder(GLYPHS)
    fb.setupCharacterMap({})
    glyphs = {}
    for n in GLYPHS:
        pen = TTGlyphPen(None)
        pen.moveTo((50, 0))
        pen.lineTo((50, 700))
        pen.lineTo((450, 700))
        pen.lineTo((450, 0))
        pen.closePath()
        glyphs[n] = pen.glyph()
    fb.setupGlyf(glyphs)
    fb.setupHorizontalMetrics({n: (WIDTH, 50) for n in GLYPHS})
    fb.setupHorizontalHeader(ascent=800, descent=-200)
    fb.setupNameTable({"familyName": "Limare Symbolic", "styleName": "Regular"})
    fb.setupOS2(sTypoAscender=800, sTypoDescender=-200, usWinAscent=800, usWinDescent=200)
    fb.setupPost(keepGlyphNames=True)
    fb.setupHead(created=WHEN, modified=WHEN)
    symbol = CmapSubtable.newSubtable(4)
    symbol.platformID, symbol.platEncID, symbol.language = 3, 0, 0
    symbol.cmap = {0xF000 + c: n for c, n in ENCODING.items()}
    mac = CmapSubtable.newSubtable(0)
    mac.platformID, mac.platEncID, mac.language = 1, 0, 0
    mac.cmap = dict(ENCODING)
    fb.font["cmap"].tables = [mac, symbol]
    out = io.BytesIO()
    fb.font.save(out)
    return out.getvalue()


def main():
    for name, data in [
        ("LimareOwnEncoding.pfa", type1()),
        ("LimareOwnEncoding.cff", cff()),
        ("LimareSymbolic.ttf", truetype()),
    ]:
        (OUT / name).write_bytes(data)
        print(OUT / name, len(data))


if __name__ == "__main__":
    main()

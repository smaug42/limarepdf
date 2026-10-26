"""Print the tables and copyright notices of src/std14.rs from Adobe's Core 14 AFM files.

Usage: python3 gen_std14.py <folder with the 14 AFM files and their readme.txt> <zapfdingbats.txt>

The output's tables (from CP1252_HIGH on) and notices go into src/std14.rs by hand; the
lookup functions there are not generated. Needs fontTools, for the Adobe Glyph List.
The AFM files and readme are Adobe's, as distributed for example with matplotlib
(mpl-data/fonts/pdfcorefonts). zapfdingbats.txt is Adobe's ITC Zapf Dingbats Glyph List
(https://github.com/adobe-type-tools/agl-aglfn), which gives ZapfDingbats' glyph names (a1,
a2, ...) their characters, as the Adobe Glyph List gives Symbol's.
"""

import sys
from pathlib import Path

from fontTools.agl import toUnicode

AFM_DIR = Path(sys.argv[1])
DINGBATS = {}
for line in Path(sys.argv[2]).read_text(encoding="utf-8").splitlines():
    if line and not line.startswith("#"):
        glyph, code = line.split(";")
        DINGBATS[glyph] = chr(int(code, 16))
FONTS = [
    ("HELVETICA", "Helvetica"),
    ("HELVETICA_BOLD", "Helvetica-Bold"),
    ("TIMES_ROMAN", "Times-Roman"),
    ("TIMES_BOLD", "Times-Bold"),
    ("TIMES_ITALIC", "Times-Italic"),
    ("TIMES_BOLD_ITALIC", "Times-BoldItalic"),
]
# Fonts with their own built-in encoding: tables by code, not by character.
SYMBOLIC = [
    ("SYMBOL", "Symbol"),
    ("ZAPF_DINGBATS", "ZapfDingbats"),
]
# Characters with no glyph of their own in the AFM files: measured as this glyph.
SAME_AS = {" ": " ", "­": "-"}


def widths(name):
    by_char = {}
    notice = None
    for line in (AFM_DIR / f"{name}.afm").read_text(encoding="latin-1").splitlines():
        if line.startswith("Notice "):
            notice = line[len("Notice "):].strip()
        if not line.startswith("C "):
            continue
        fields = dict(
            f.strip().split(" ", 1) for f in line.split(";") if f.strip() and " " in f.strip()
        )
        ch = toUnicode(fields["N"])
        if len(ch) == 1:
            by_char.setdefault(ch, int(float(fields["WX"])))
    row = []
    for code in range(32, 256):
        try:
            ch = bytes([code]).decode("cp1252")
        except UnicodeDecodeError:
            row.append(0)
            continue
        ch = SAME_AS.get(ch, ch)
        row.append(by_char.get(ch, 0))
    return notice, row


def widths_by_code(name):
    by_code = {}
    notice = None
    for line in (AFM_DIR / f"{name}.afm").read_text(encoding="latin-1").splitlines():
        if line.startswith("Notice "):
            notice = line[len("Notice "):].strip()
        if not line.startswith("C "):
            continue
        fields = dict(
            f.strip().split(" ", 1) for f in line.split(";") if f.strip() and " " in f.strip()
        )
        code = int(fields["C"])
        if code >= 0:
            by_code[code] = int(float(fields["WX"]))
    return notice, [by_code.get(code, 0) for code in range(32, 256)]


def text_by_code(name):
    """The character of each code 32..=255 of the font's built-in encoding, by its glyph
    name; NUL for a code with none."""
    by_code = {}
    for line in (AFM_DIR / f"{name}.afm").read_text(encoding="latin-1").splitlines():
        if not line.startswith("C "):
            continue
        fields = dict(
            f.strip().split(" ", 1) for f in line.split(";") if f.strip() and " " in f.strip()
        )
        code, glyph = int(fields["C"]), fields["N"]
        ch = DINGBATS.get(glyph) or toUnicode(glyph)
        if code >= 0 and len(ch) == 1:
            by_code[code] = ch
    return [by_code.get(code, "\0") for code in range(32, 256)]


def text_table(out, const, name, row):
    out.append(f"/// {name}: the character of each code of its built-in encoding 32..=255, by its")
    out.append("/// glyph's name; NUL where the code has none.")
    out.append("#[rustfmt::skip]")
    out.append(f"const {const}: [char; 224] = [")
    for i in range(0, 224, 8):
        cells = ", ".join("'\\0'" if c == "\0" else f"'\\u{{{ord(c):04X}}}'" for c in row[i:i + 8])
        out.append(f"    {cells}, // {32 + i}-{32 + i + 7}")
    out.append("];")
    out.append("")


def table(out, const, name, row, what):
    out.append(f"/// {name}: widths (1/1000 em) of {what} 32..=255.")
    out.append("#[rustfmt::skip]")
    out.append(f"const {const}: [u16; 224] = [")
    for i in range(0, 224, 16):
        cells = ", ".join(str(w) for w in row[i:i + 16])
        out.append(f"    {cells}, // {32 + i}-{32 + i + 15}")
    out.append("];")
    out.append("")


readme = (AFM_DIR / "readme.txt").read_text(encoding="latin-1")
permission = readme[readme.index("This file and the 14"):].strip()

out = []
for const, name in FONTS:
    notice, _ = widths(name)
    out.append(f"//! {name}.afm: {notice}")
for const, name in SYMBOLIC:
    notice, _ = widths_by_code(name)
    out.append(f"//! {name}.afm: {notice}")
out.append("//!")
out.append("//! The permission notice that accompanies the AFM files, unmodified:")
out.append("//!")
for line in permission.splitlines():
    out.append(f"//! {line.rstrip()}".rstrip())
out.append("//!")
out.append("//! ZapfDingbats' characters are from the ITC Zapf Dingbats Glyph List, whose notice follows,")
out.append("//! unmodified:")
out.append("//!")
notice = Path(sys.argv[2]).read_text(encoding="utf-8").split("# Name:")[0]
for line in notice.splitlines():
    if not line.startswith("# ---"):
        out.append(f"//! {line[2:]}".rstrip())
out.append("")
out.append("/// Windows-1252 codes 128..=159, as characters; NUL where the code has none.")
out.append("#[rustfmt::skip]")
high = []
for code in range(128, 160):
    try:
        high.append(bytes([code]).decode("cp1252"))
    except UnicodeDecodeError:
        high.append("\0")
out.append("const CP1252_HIGH: [char; 32] = [")
for i in range(0, 32, 8):
    cells = ", ".join("'\\0'" if c == "\0" else f"'\\u{{{ord(c):04X}}}'" for c in high[i:i + 8])
    out.append(f"    {cells},")
out.append("];")
out.append("")
for const, name in FONTS:
    _, row = widths(name)
    table(out, const, name, row, "Windows-1252 codes")
for const, name in SYMBOLIC:
    _, row = widths_by_code(name)
    table(out, const, name, row, "the codes of its built-in encoding")
for const, name in SYMBOLIC:
    text_table(out, f"{const}_TEXT", name, text_by_code(name))
print("\n".join(out), end="")

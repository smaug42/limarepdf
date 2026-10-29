"""Print the glyph-name tables of src/base_encodings.rs: the glyph each code of the PDF's
WinAnsiEncoding and MacRomanEncoding names, and the codes Mac OS Roman adds to MacRoman.

Usage: python3 gen_base_encodings.py

The output's tables go into src/base_encodings.rs by hand; the lookup functions there are
not generated. Needs fontTools (for its Mac OS Roman glyph names and the Adobe Glyph List).
StandardEncoding's names come from read-fonts, which has them.

The tables are those a renderer looks a simple font's glyph up by, so they follow the names
the PDF readers use (PDFBox, and hayro after it): Windows-1252 through the Adobe Glyph List
for New Fonts, with 0xA0 named nbspace and 0xAD sfthyphen (the PDF specification's Annex D
names them space and hyphen as second codes of those characters), and the superior figures
by their Adobe Glyph List names; MacRoman is Mac OS Roman less the 16 codes Annex D leaves
out (and 0xDB, currency in Annex D, Euro in Mac OS Roman), which come after it as extras.
"""

import fontTools.encodings.MacRoman as mac
from fontTools.agl import UV2AGL

# Windows-1252 characters the Adobe Glyph List for New Fonts has no name for, and the two
# codes the readers name apart from their characters.
WIN_ANSI_NAMES = {
    0xA0: "nbspace",
    0xAD: "sfthyphen",
    0xB2: "twosuperior",
    0xB3: "threesuperior",
    0xB9: "onesuperior",
}
# Mac OS Roman's codes that Annex D's MacRomanEncoding leaves out.
MAC_OS_ONLY = [
    0xAD, 0xB0, 0xB2, 0xB3, 0xB6, 0xB7, 0xB8, 0xB9, 0xBA, 0xBD, 0xC3, 0xC5, 0xC6, 0xD7,
    0xDB, 0xF0,
]


def win_ansi():
    names = []
    for code in range(256):
        name = ""
        if code in WIN_ANSI_NAMES:
            name = WIN_ANSI_NAMES[code]
        elif code >= 0x20:
            try:
                name = UV2AGL.get(ord(bytes([code]).decode("cp1252")), "")
            except UnicodeDecodeError:
                pass
        names.append(name)
    return names


def mac_roman():
    names = []
    for code in range(256):
        name = mac.MacRoman[code]
        if code < 0x20 or code == 0x7F or code in MAC_OS_ONLY or name == ".notdef":
            name = ""
        names.append(name)
    names[0xDB] = "currency"
    extras = [(code, "Euro" if code == 0xDB else mac.MacRoman[code]) for code in MAC_OS_ONLY]
    return names, extras


def table(const, doc, names):
    print(f"/// {doc}")
    print("#[rustfmt::skip]")
    print(f"const {const}: [&str; 256] = [")
    for row in range(0, 256, 8):
        print("    " + " ".join(f'"{n}",' for n in names[row : row + 8]))
    print("];")
    print()


table("WIN_ANSI", "The glyph each code of WinAnsiEncoding names; empty for none.", win_ansi())
names, extras = mac_roman()
table("MAC_ROMAN", "The glyph each code of MacRomanEncoding names; empty for none.", names)
print("/// The codes Mac OS Roman adds to MacRomanEncoding, with their glyphs.")
print(f"const MAC_OS_ROMAN: [(u8, &str); {len(extras)}] = [")
for code, name in extras:
    print(f'    (0x{code:02X}, "{name}"),')
print("];")

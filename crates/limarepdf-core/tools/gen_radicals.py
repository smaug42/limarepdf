"""Print the table of src/radicals.rs: each CJK radical (the Kangxi Radicals block and the CJK
Radicals Supplement) with the unified ideograph it looks like, from the Unicode Character
Database's EquivalentUnifiedIdeograph.txt.

Usage: python3 gen_radicals.py EquivalentUnifiedIdeograph.txt

The file is https://www.unicode.org/Public/16.0.0/ucd/EquivalentUnifiedIdeograph.txt (sha256
628929adae97427348337003e39d29da7b55acdddfac21e06d3dacad92237834). The output's table goes
into src/radicals.rs by hand; the lookup function there is not generated. The CJK strokes the
file also lists are left out: a font that maps a glyph to a stroke means the stroke.
"""

import sys


def radical(code):
    return 0x2E80 <= code <= 0x2EFF or 0x2F00 <= code <= 0x2FDF


def main(path):
    pairs = []
    with open(path, encoding="utf-8") as f:
        for line in f:
            line = line.split("#", 1)[0].strip()
            if not line:
                continue
            source, target = (part.strip() for part in line.split(";"))
            first, _, last = source.partition("..")
            for code in range(int(first, 16), int(last or first, 16) + 1):
                if radical(code):
                    pairs.append((code, int(target, 16)))
    pairs.sort()
    print(f"pub(crate) static RADICALS: [(u16, u32); {len(pairs)}] = [")
    for i in range(0, len(pairs), 4):
        row = " ".join(f"(0x{a:04X}, 0x{b:04X})," for a, b in pairs[i : i + 4])
        print(f"    {row}")
    print("];")


if __name__ == "__main__":
    main(sys.argv[1])

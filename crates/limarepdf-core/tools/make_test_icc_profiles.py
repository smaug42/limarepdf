"""Make the made-up colour profiles the picture tests use: tests/data/images/linear-rgb.icc and
linear-gray.icc.

Usage, from the repository: python3 crates/limarepdf-core/tools/make_test_icc_profiles.py

Both are ICC version 2 display profiles with straight-line tone curves (gamma 1.0) and a D50
white; the RGB one has sRGB's primaries. A sample of 128 is half the light, which sRGB shows
as about 188: far from what the same number means in sRGB, so a test can tell a picture shown
through its profile from one read as plain sRGB. The header's device model is left empty, not
"sRGB", which readers take as plain sRGB without reading the rest.

The output is the same on every run (a fixed date). Standard library only.
"""

import struct
from pathlib import Path

OUT = Path(__file__).resolve().parents[1] / "tests" / "data" / "images"
D50 = (0.9642, 1.0, 0.8249)
# sRGB's primaries, adapted to D50 (as in the common sRGB profiles).
RED = (0.4361, 0.2225, 0.0139)
GREEN = (0.3851, 0.7169, 0.0971)
BLUE = (0.1431, 0.0606, 0.7141)


def s15f16(v):
    return struct.pack(">i", round(v * 65536))


def xyz(v):
    return b"XYZ " + bytes(4) + b"".join(s15f16(c) for c in v)


def curve_gamma_1():
    # One entry: the gamma as u8Fixed8 (1.0), padded to four bytes.
    return b"curv" + bytes(4) + struct.pack(">IH", 1, 0x0100) + bytes(2)


def desc(text):
    a = text.encode("ascii") + b"\0"
    # textDescriptionType: ASCII, then empty Unicode and ScriptCode parts.
    return (
        b"desc" + bytes(4) + struct.pack(">I", len(a)) + a
        + struct.pack(">II", 0, 0) + struct.pack(">HB", 0, 0) + bytes(67)
    )


def text(t):
    return b"text" + bytes(4) + t.encode("ascii") + b"\0"


def profile(space, tags):
    """An ICC v2 monitor profile for `space` (b"RGB " or b"GRAY") with `tags`, a list of
    (signature, data); equal data is stored once."""
    count = len(tags)
    offset = 128 + 4 + 12 * count
    table, body, placed = b"", b"", {}
    for sig, data in tags:
        if data not in placed:
            while (offset + len(body)) % 4:
                body += b"\0"
            placed[data] = offset + len(body)
            body += data
        table += sig + struct.pack(">II", placed[data], len(data))
    while len(body) % 4:
        body += b"\0"
    size = offset + len(body)
    header = (
        struct.pack(">I", size)
        + bytes(4)  # preferred CMM
        + struct.pack(">I", 0x02100000)  # version 2.1
        + b"mntr" + space + b"XYZ "
        + struct.pack(">6H", 2026, 11, 9, 0, 0, 0)
        + b"acsp"
        + bytes(4)  # platform
        + bytes(4)  # flags
        + bytes(4)  # device manufacturer
        + bytes(4)  # device model: not "sRGB", which readers take as plain sRGB
        + bytes(8)  # attributes
        + bytes(4)  # rendering intent: perceptual
        + b"".join(s15f16(c) for c in D50)
        + bytes(4)  # creator
        + bytes(44)
    )
    assert len(header) == 128
    return header + struct.pack(">I", count) + table + body


def main():
    note = text("No copyright: made for LimarePDF's tests.")
    rgb = profile(
        b"RGB ",
        [
            (b"desc", desc("Linear RGB (sRGB primaries, gamma 1.0)")),
            (b"cprt", note),
            (b"wtpt", xyz(D50)),
            (b"rXYZ", xyz(RED)),
            (b"gXYZ", xyz(GREEN)),
            (b"bXYZ", xyz(BLUE)),
            (b"rTRC", curve_gamma_1()),
            (b"gTRC", curve_gamma_1()),
            (b"bTRC", curve_gamma_1()),
        ],
    )
    gray = profile(
        b"GRAY",
        [
            (b"desc", desc("Linear grey (gamma 1.0)")),
            (b"cprt", note),
            (b"wtpt", xyz(D50)),
            (b"kTRC", curve_gamma_1()),
        ],
    )
    for name, data in [("linear-rgb.icc", rgb), ("linear-gray.icc", gray)]:
        out = OUT / name
        out.write_bytes(data)
        print(out)


if __name__ == "__main__":
    main()

#!/usr/bin/env python3
"""The made-up "Quarterly Field Report" the user guide's screenshots show (docs/images).

    python3 -I tools/docshots/mkreport.py OUT [--plain | --review]

Three pages, each a heading, a paragraph or two and a bar chart. By default page 1 carries Pat Example's
highlight, note, pen tick and text box with an arrow. --plain leaves them out. --review adds Alex Kim's
and Sam Lee's review: two notes, a highlight and an underline on page 1, a reply to a note and one to the
highlight, and a note on page 2 (12 comments in all).

The comments are written as LimarePDF writes them, without appearance streams (the app draws them).
Their dates are fixed and carry no time zone, so the app shows them as written. The text is set in
Helvetica and Times with their widths in the file, read from the Liberation fonts (the same widths),
found under the usual font folders. Standard library only.
"""
import pathlib
import struct
import sys

FONTS = {
    "F1": ("Helvetica", "LiberationSans-Regular.ttf"),
    "F2": ("Helvetica-Bold", "LiberationSans-Bold.ttf"),
    "F3": ("Times-Roman", "LiberationSerif-Regular.ttf"),
}
FONT_DIRS = ["/usr/share/fonts", "/usr/local/share/fonts", "~/.local/share/fonts", "~/.fonts"]
FIRST, LAST = 32, 126
_widths = {}


def glyph_widths(path):
    """Each printable ASCII character's advance in thousandths of the font size, from a TrueType
    file's `cmap` (format 4, Windows Unicode) and `hmtx` tables."""
    data = path.read_bytes()
    tables = {}
    for i in range(struct.unpack(">H", data[4:6])[0]):
        tag, _, off, length = struct.unpack(">4sIII", data[12 + 16 * i : 28 + 16 * i])
        tables[tag.decode("latin-1")] = data[off : off + length]
    upem = struct.unpack(">H", tables["head"][18:20])[0]
    metrics = struct.unpack(">H", tables["hhea"][34:36])[0]
    advance = [struct.unpack(">H", tables["hmtx"][4 * i : 4 * i + 2])[0] for i in range(metrics)]
    cmap = tables["cmap"]
    for i in range(struct.unpack(">H", cmap[2:4])[0]):
        platform, encoding, off = struct.unpack(">HHI", cmap[4 + 8 * i : 12 + 8 * i])
        if (platform, encoding) == (3, 1) and struct.unpack(">H", cmap[off : off + 2])[0] == 4:
            sub = cmap[off:]
            break
    else:
        sys.exit("mkreport.py: %s has no Unicode character map" % path)
    n2 = struct.unpack(">H", sub[6:8])[0]
    n = n2 // 2
    ends = struct.unpack(">%dH" % n, sub[14 : 14 + n2])
    starts = struct.unpack(">%dH" % n, sub[16 + n2 : 16 + 2 * n2])
    deltas = struct.unpack(">%dh" % n, sub[16 + 2 * n2 : 16 + 3 * n2])
    offsets_at = 16 + 3 * n2
    offsets = struct.unpack(">%dH" % n, sub[offsets_at : offsets_at + n2])
    widths = {}
    for code in range(FIRST, LAST + 1):
        glyph = 0
        for s in range(n):
            if starts[s] <= code <= ends[s]:
                if offsets[s] == 0:
                    glyph = (code + deltas[s]) & 0xFFFF
                else:
                    at = offsets_at + 2 * s + offsets[s] + 2 * (code - starts[s])
                    glyph = struct.unpack(">H", sub[at : at + 2])[0]
                    glyph = (glyph + deltas[s]) & 0xFFFF if glyph else 0
                break
        widths[code] = round(advance[min(glyph, metrics - 1)] * 1000 / upem)
    return widths


def widths(font):
    if font not in _widths:
        name = FONTS[font][1]
        found = (p for d in FONT_DIRS for p in sorted(pathlib.Path(d).expanduser().rglob(name)))
        path = next(found, None)
        if path is None:
            sys.exit("mkreport.py: %s not found under %s" % (name, ", ".join(FONT_DIRS)))
        _widths[font] = glyph_widths(path)
    return _widths[font]


def width(font, size, text):
    return sum(widths(font)[ord(c)] for c in text) * size / 1000


def esc(s):
    return s.replace("\\", "\\\\").replace("(", "\\(").replace(")", "\\)").replace("\n", "\\n")


def nums(*v):
    return " ".join("%.2f" % x for x in v)


def text(font, size, x, y, s, rgb=(0, 0, 0)):
    return "%.3f %.3f %.3f rg BT /%s %g Tf %.2f %.2f Td (%s) Tj ET\n" % (*rgb, font, size, x, y, esc(s))


BLUE = (0.15, 0.39, 0.92)
YELLOW = (1, 0.85, 0.2)
PAGES = [
    ("Quarterly Field Report", "Northern Water Survey, third quarter",
     ["This report summarises the samples collected at",
      "twelve monitoring stations along the river between",
      "July and September. Flow rates were close to the",
      "ten-year average, and every station met its targets.",
      "",
      "The team replaced two sensors and recalibrated the",
      "upstream gauges after the August storms."],
     [90, 140, 115, 175, 130, 110]),
    ("Station results", "Flow and quality by station",
     ["Stations 1 to 6 sit above the weir; stations 7 to 12",
      "below it. Readings below the weir were steadier this",
      "quarter, as the new gauges came into use."],
     [120, 100, 150, 135, 160, 125]),
    ("Next steps", "Plans for the fourth quarter",
     ["Replace the last of the old sensors at stations 3 and 9.",
      "Share the station data with the regional office.",
      "Review the sampling schedule before the winter floods."],
     [100, 130, 110, 155, 120, 145]),
]


def page_content(n, title, sub, body, bars):
    c = text("F1", 9, 540 - width("F1", 9, "Northern Water Survey"), 740, "Northern Water Survey", (0.2, 0.2, 0.2))
    c += "%.3f %.3f %.3f rg 72 716 468 3 re f\n" % BLUE
    c += text("F2", 24, 72, 678, title, (0.1, 0.1, 0.12))
    c += text("F1", 13, 72, 656, sub, (0.42, 0.45, 0.5))
    y = 610
    for line in body:
        if line:
            c += text("F3", 12, 72, y, line)
        y -= 17
    c += "0.92 0.95 1 rg 0.7 0.78 0.95 RG 0.75 w 72 120 468 230 re B\n"
    for i, h in enumerate(bars):
        c += "%.3f %.3f %.3f rg %d 120 40 %d re f\n" % (*BLUE, 96 + i * 76, h)
    c += text("F1", 9, 306 - width("F1", 9, str(n)) / 2, 60, str(n), (0.4, 0.4, 0.4))
    return c


# Who and when: each comment's author and fixed date (no time zone: shown as written).
PAT = ("Pat Example", "D:20261020093000")
ALEX = ("Alex Kim", "D:20261028192900")
SAM = ("Sam Lee", "D:20261028192900")
PRINT, NO_ZOOM, NO_ROTATE, NO_VIEW = 4, 8, 16, 32


def annot(sub, who, flags, rest):
    """An annotation's dictionary; {P} is its page and {IRT} what it answers, filled in by build()."""
    return "<< /Type /Annot /Subtype /%s /P {P} /F %d /T (%s) /M (%s) /CreationDate (%s) /NM ({NM}) %s >>" % (
        sub, flags, esc(who[0]), who[1], who[1], rest)


def words(line_no, first, count):
    """The box (x0, y0, x1, y1) of `count` words from word `first` (both from 0) on page 1's body
    line `line_no` (from 0)."""
    w = PAGES[0][2][line_no].split(" ")
    x0 = 72 + width("F3", 12, " ".join(w[:first]) + (" " if first else ""))
    x1 = 72 + width("F3", 12, " ".join(w[: first + count]))
    base = 610 - 17 * line_no
    return x0, base - 3, x1, base + 11


def markup(sub, box, color, who):
    x0, y0, x1, y1 = box
    rest = "/Rect [%s] /QuadPoints [%s] /C [%s]" % (
        nums(x0 - 1, y0 - 1, x1 + 1, y1 + 1), nums(x0, y1, x1, y1, x0, y0, x1, y0), nums(*color))
    return annot(sub, who, PRINT, rest)


def note_rect(at):
    """A 20-point note's box centred on `at` (x, and y down from the page's top, as the app places it)."""
    x, y = at[0], 792 - at[1]
    return (x - 10, y - 10, x + 10, y + 10)


def note(at, contents, who):
    rest = "/Rect [%s] /C [%s] /Name /Comment /Contents (%s)" % (nums(*note_rect(at)), nums(*YELLOW), esc(contents))
    return annot("Text", who, PRINT | NO_ZOOM | NO_ROTATE, rest)


def reply(at, contents, who):
    rest = "/Rect [%s] /IRT {IRT} /RT /R /Name /Comment /Contents (%s)" % (nums(*note_rect(at)), esc(contents))
    return annot("Text", who, NO_ZOOM | NO_ROTATE | NO_VIEW, rest)


def comments(mode):
    """Each page's comments (by page index): (key, the key of what it answers or None, dictionary)."""
    if mode == "plain":
        return {}
    pages = {0: [
        ("pat-highlight", None, markup("Highlight", words(3, 3, 5), YELLOW, PAT)),
        ("pat-note", None, annot("Text", PAT, PRINT, "/Rect [452 590 472 610] /Name /Comment /C [%s] "
                                 "/Contents (Check the September readings with Sam.)" % nums(*YELLOW))),
        ("pat-tick", None, annot("Ink", PAT, PRINT, "/Rect [470 540 520 580] /InkList [[476 560 484 551 489 546 "
                                 "497 556 507 568 514 576]] /BS << /W 2.5 >> /C [0.13 0.55 0.24]")),
        ("pat-box", None, annot("FreeText", PAT, PRINT, "/Rect [372 422 528 444] /DA (/Helv 10 Tf 0.85 0.12 0.12 rg) "
                                "/C [1 0.96 0.95] /BS << /W 1 >> /Contents (Peak after the August storms)")),
        ("pat-arrow", None, annot("Line", PAT, PRINT, "/Rect [330 285 445 425] /L [440 420 336 297] "
                                  "/LE [/None /OpenArrow] /BS << /W 1.5 >> /C [0.85 0.12 0.12]")),
    ]}
    if mode == "review":
        pages[0] += [
            ("sam-note", None, note((520, 120), "Can we give the figure for last year too?", SAM)),
            ("alex-reply", "sam-note", reply((520, 120), "Added in the next draft.", ALEX)),
            ("alex-highlight", None, markup("Highlight", words(1, 0, 3), YELLOW, ALEX)),
            ("sam-reply", "alex-highlight", annot("Text", SAM, NO_ZOOM | NO_ROTATE | NO_VIEW, (
                "/Rect [%s] /IRT {IRT} /RT /R /Name /Comment "
                "/Contents (Is this still right after the March figures?)") % nums(*words(1, 0, 3)))),
            ("sam-underline", None, markup("Underline", words(3, 1, 4), (0, 0.4, 0.9), SAM)),
            ("alex-note", None, note((520, 400), 'Shorter heading here, please.\nMaybe just "Results".', ALEX)),
        ]
        pages[1] = [
            ("sam-page-2", None, note((100, 200), "Check these totals against the appendix before we send it out.", SAM)),
        ]
    return pages


def font_dict(key):
    w = widths(key)
    return "<< /Type /Font /Subtype /Type1 /BaseFont /%s /Encoding /WinAnsiEncoding /FirstChar %d /LastChar %d /Widths [%s] >>" % (
        FONTS[key][0], FIRST, LAST, " ".join(str(w[c]) for c in range(FIRST, LAST + 1)))


def build(mode):
    objs = {}

    def add(body=None):
        objs[len(objs) + 1] = body
        return len(objs)

    catalog, pages = add(), add()
    fonts = " ".join("/%s %d 0 R" % (k, add(font_dict(k))) for k in FONTS)
    notes = comments(mode)
    kids = []
    for i, page in enumerate(PAGES):
        c = page_content(i + 1, *page).encode()
        cid = add(b"<< /Length %d >>stream\n" % len(c) + c + b"\nendstream")
        pid = add()
        ids = {key: add() for key, _, _ in notes.get(i, [])}
        for key, parent, d in notes.get(i, []):
            d = d.replace("{P}", "%d 0 R" % pid).replace("{NM}", "docshots-%s" % key)
            objs[ids[key]] = d.replace("{IRT}", "%d 0 R" % ids[parent]) if parent else d
        listed = " /Annots [%s]" % " ".join("%d 0 R" % v for v in ids.values()) if ids else ""
        objs[pid] = "<< /Type /Page /Parent %d 0 R /MediaBox [0 0 612 792] /Contents %d 0 R /Resources << /Font << %s >> >>%s >>" % (
            pages, cid, fonts, listed)
        kids.append(pid)
    objs[catalog] = "<< /Type /Catalog /Pages %d 0 R >>" % pages
    objs[pages] = "<< /Type /Pages /Kids [%s] /Count %d >>" % (" ".join("%d 0 R" % k for k in kids), len(kids))
    info = add("<< /Title (Quarterly Field Report) /Author (Pat Example) /CreationDate (D:20261020090000) >>")
    out = b"%PDF-1.7\n"
    offsets = []
    for i in range(1, len(objs) + 1):
        offsets.append(len(out))
        body = objs[i] if isinstance(objs[i], bytes) else objs[i].encode()
        out += b"%d 0 obj\n" % i + body + b"\nendobj\n"
    xref = len(out)
    out += b"xref\n0 %d\n0000000000 65535 f \n" % (len(objs) + 1)
    for o in offsets:
        out += b"%010d 00000 n \n" % o
    out += b"trailer\n<< /Size %d /Root %d 0 R /Info %d 0 R >>\nstartxref\n%d\n%%%%EOF\n" % (
        len(objs) + 1, catalog, info, xref)
    return out


def main(args):
    flags = [a for a in args if a.startswith("--")]
    paths = [a for a in args if not a.startswith("--")]
    if len(paths) != 1 or len(flags) > 1 or not set(flags) <= {"--plain", "--review"}:
        sys.exit(__doc__.strip().splitlines()[2].strip())
    mode = flags[0][2:] if flags else "comments"
    pathlib.Path(paths[0]).write_bytes(build(mode))


if __name__ == "__main__":
    main(sys.argv[1:])

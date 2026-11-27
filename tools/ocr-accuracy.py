#!/usr/bin/env python3
"""OCR accuracy measure (#221): runs LimarePDF's OCR over scans and scores the text it adds
against the true text, read back with Poppler's pdftotext, as other apps read it, or with
`--reader limarepdf` by LimarePDF's own `text`, as its search and copying read it. The method
behind the "Accuracy and speed" table in docs/scanning-and-ocr.md; testing.md's "Scanning and
OCR" record says what ran.

A manifest (TOML) lists the cases, with paths relative to it:

    [[case]]
    name = "sheet, phone"
    input = "phone/sheet.pdf"              # a PDF, or a picture (JPEG, PNG, TIFF)
    truth = ["page-1.txt", "page-2.txt"]   # the true text of each page in `pages`
    pages = [1, 2]                         # optional: 1, 2, ... one per truth file
    words_only = false                     # optional: truth in no known order, words found only
    lang = "deu"                           # optional: Tesseract's language, eng by default

tools/ocr-sheet/ holds a two-page test sheet with the true text of each page (page-1.txt, page-2.txt):
print sheet.pdf, scan or photograph it, and list the scans with that truth. sheet.pdf is made from
sheet.fodt with `soffice --headless --convert-to pdf --outdir tools/ocr-sheet tools/ocr-sheet/sheet.fodt`.

Each case is measured as it is and after optimize-scans, the guide's order. The OCR skips a PDF
that already carries text (phone scanner apps add their own), so that text is scored as "own
text" and the PDF is rebuilt from its pictures first.

Scores, per page:
  CER       edit distance over the true text's length, after NFC, ligatures expanded, line-end
            hyphens joined and spaces collapsed
  CER(f)    the same with accents and typographic signs folded to ASCII: what's misread beyond
            accents and signs such as curly quotes (signs with no ASCII form still count)
  words     the share of the true words found, in any order, ignoring case; (f) folded
  s/page    the OCR command's time over its pages, start-up included
With --tesseract, Tesseract reads the same pages rendered as the OCR renders them, in the case's
language, its models from --tessdata DIR if given. A page whose pictures are marked at a lower
resolution than they have (phone scans often say 72 dpi) is rendered larger than its pixels, which
hurts Tesseract more than LimarePDF; the measure notes it, and `images --dpi` with the true
resolution makes a fairer reference.

    python3 -I tools/ocr-accuracy.py MANIFEST --work DIR [--cli PATH] [--tesseract] [--tessdata DIR]
                                     [--reader poppler|limarepdf] [--json OUT]
    python3 -I tools/ocr-accuracy.py --self-test
"""
import argparse
import collections
import dataclasses
import json
import pathlib
import random
import re
import shutil
import subprocess
import sys
import time
import tomllib
import unicodedata

CLI = pathlib.Path(__file__).resolve().parent.parent / "target" / "release" / "limarepdf-cli"
OCR_DPI, MAX_SIDE = 300, 7000  # as limarepdf-core's ocr.rs renders a page
LIGATURES = str.maketrans({"ﬀ": "ff", "ﬁ": "fi", "ﬂ": "fl", "ﬃ": "ffi", "ﬄ": "ffl", "ﬅ": "st", "ﬆ": "st"})
FOLD = str.maketrans(
    {"ł": "l", "Ł": "L", "ø": "o", "Ø": "O", "đ": "d", "Đ": "D", "ı": "i", "ß": "ss", "ẞ": "SS"}
    | {"œ": "oe", "Œ": "OE", "æ": "ae", "Æ": "AE", "…": "..."}
    | dict.fromkeys("‘’‚‛′‹›", "'")
    | dict.fromkeys("“”„‟″«»", '"')
    | dict.fromkeys("‐‑‒–—―−", "-")
)


@dataclasses.dataclass
class Case:
    name: str
    input: pathlib.Path
    pages: list
    truth: list
    words_only: bool = False
    lang: str = "eng"


def levenshtein(a, b):
    """The edit distance between `a` and `b` (Myers' bit-parallel method, in Hyyrö's form)."""
    if not a:
        return len(b)
    peq = {}
    for i, c in enumerate(a):
        peq[c] = peq.get(c, 0) | 1 << i
    mask, top = (1 << len(a)) - 1, 1 << (len(a) - 1)
    pv, mv, d = mask, 0, len(a)
    for c in b:
        eq = peq.get(c, 0)
        xv = eq | mv
        xh = (((eq & pv) + pv) ^ pv) | eq
        ph = mv | (~(xh | pv) & mask)
        mh = pv & xh
        if ph & top:
            d += 1
        elif mh & top:
            d -= 1
        ph = (ph << 1 | 1) & mask
        mh = (mh << 1) & mask
        pv = mh | (~(xv | ph) & mask)
        mv = ph & xv
    return d


def normalise(text):
    """`text` in NFC, ligatures expanded, line-end hyphens joined, spaces collapsed."""
    text = unicodedata.normalize("NFC", text).translate(LIGATURES)
    text = re.sub(r"(\w)-[ \t]*\n\s*(?=\w)", r"\1", text)
    return " ".join(text.split())


def fold(text):
    """`text` with accents and typographic signs folded to ASCII."""
    text = unicodedata.normalize("NFD", text.translate(FOLD))
    text = "".join(c for c in text if not unicodedata.combining(c))
    return " ".join(unicodedata.normalize("NFC", text).split())


def cer(truth, read):
    return levenshtein(truth, read) / max(len(truth), 1)


def words_found(truth, read):
    """The share of `truth`'s words that `read` has, in any order, ignoring case."""
    t, r = (collections.Counter(re.findall(r"\w+", s.casefold())) for s in (truth, read))
    return sum((t & r).values()) / max(sum(t.values()), 1)


def parse_cases(data, base):
    cases = []
    for c in data.get("case", []):
        name = c.get("name", "?")
        unknown = set(c) - {"name", "input", "truth", "pages", "words_only", "lang"}
        if unknown:
            raise ValueError(f"{name}: unknown keys {sorted(unknown)}")
        truth = [base / t for t in c["truth"]]
        pages = c.get("pages", list(range(1, len(truth) + 1)))
        if len(pages) != len(truth):
            raise ValueError(f"{name}: {len(pages)} pages but {len(truth)} truth files")
        lang = c.get("lang", "eng")
        if not re.fullmatch(r"[a-z_]+(\+[a-z_]+)*", lang):
            raise ValueError(f"{name}: lang {lang!r} isn't a Tesseract language like deu or fra+eng")
        cases.append(Case(name, base / c["input"], pages, truth, c.get("words_only", False), lang))
    if not cases:
        raise ValueError("no [[case]] in the manifest")
    return cases


def run(*args):
    """The command's output; its error message if it fails."""
    r = subprocess.run([str(a) for a in args], capture_output=True, text=True, errors="replace")
    if r.returncode:
        raise RuntimeError(f"{pathlib.Path(str(args[0])).name} {args[1]}: {(r.stderr or r.stdout).strip()}")
    return r.stdout


def page_text(pdf, page):
    return run("pdftotext", "-enc", "UTF-8", "-f", page, "-l", page, pdf, "-")


def read_back(cli, pdf, page, reader):
    """The page's text as `reader` reads it: Poppler's pdftotext, or LimarePDF's `text`."""
    if reader == "poppler":
        return page_text(pdf, page)
    out = run(cli, "text", "--pages", page, pdf)
    return out.split("\n", 1)[1] if out.startswith("--- page") else out


def page_size(pdf, page):
    m = re.search(r"size:\s+([\d.]+) x ([\d.]+) pts", run("pdfinfo", "-f", page, "-l", page, pdf))
    return float(m.group(1)), float(m.group(2))


def render_dpi(w, h):
    """The dpi the OCR renders a `w` × `h` pt page at."""
    return min(OCR_DPI, MAX_SIDE * 72 / max(w, h))


def picture_ppi(pdf, page):
    """The resolution the page's largest picture is marked at; None if it has none."""
    pictures = [ln.split() for ln in run("pdfimages", "-f", page, "-l", page, "-list", pdf).splitlines()[2:]]
    pictures = [p for p in pictures if len(p) > 13 and p[2] == "image"]
    if not pictures:
        return None
    return float(max(pictures, key=lambda p: int(p[3]) * int(p[4]))[12])


def upscale_note(page, ppi, w, h):
    """A note when the page's pictures are rendered over 1.5 times larger than their pixels."""
    k = render_dpi(w, h) / ppi if ppi else 0
    if k <= 1.5:
        return None
    return (
        f"page {page}: pictures marked {ppi:.0f} dpi, so read at {k:.1f} times their pixels, which"
        " hurts Tesseract; `images --dpi` at their true resolution makes a fairer reference"
    )


def rebuild(cli, pdf, d, notes):
    """`pdf` without its text: its pictures saved and made into pages again, at their own dpi."""
    pictures = [ln.split() for ln in run("pdfimages", "-list", pdf).splitlines()[2:]]
    pictures = [p for p in pictures if len(p) > 13 and p[2] == "image"]
    per_page = collections.Counter(int(p[0]) for p in pictures)
    if any(n > 1 for n in per_page.values()) or len(per_page) != int(
        re.search(r"^Pages:\s+(\d+)", run("pdfinfo", pdf), re.M).group(1)
    ):
        raise RuntimeError("not one picture a page, so it can't be rebuilt")
    dpis = {(p[12], p[13]) for p in pictures}
    if len(dpis) > 1:
        notes.append(f"pictures at {len(dpis)} resolutions: rebuilt at the first page's")
    saved = d / "pictures"
    run(cli, "save-images", "--out-dir", saved, pdf)
    out = d / "image-only.pdf"
    run(cli, "images", "-o", out, "--dpi", pictures[0][12], *sorted(saved.iterdir()))
    return out


def tesseract_read(pdf, page, d, lang, tessdata):
    dpi = render_dpi(*page_size(pdf, page))
    png = d / f"tesseract-page-{page}"
    run("pdftoppm", "-r", f"{dpi:.2f}", "-f", page, "-l", page, "-gray", "-png", "-singlefile", pdf, png)
    models = ["--tessdata-dir", tessdata] if tessdata else []
    return run("tesseract", png.with_suffix(".png"), "-", *models, "-l", lang)


def score(case, label, read, secs):
    rows = []
    for page, text, truth in zip(case.pages, read, case.truth):
        nt, nr = normalise(truth.read_text(encoding="utf-8")), normalise(text)
        ft, fr = fold(nt), fold(nr)
        rows.append(
            {
                "case": case.name,
                "variant": label,
                "page": page,
                "cer": None if case.words_only else cer(nt, nr),
                "cer_folded": None if case.words_only else cer(ft, fr),
                "words": words_found(nt, nr),
                "words_folded": words_found(ft, fr),
                "seconds": secs,
            }
        )
    return rows


def measure(case, cli, work, tesseract, tessdata, reader):
    """The case's score rows and notes; its outputs and read text left in a folder of `work`."""
    d = work / (re.sub(r"\W+", "-", case.name.lower()).strip("-") or "case")
    shutil.rmtree(d, ignore_errors=True)
    d.mkdir(parents=True)
    rows, notes, pdf = [], [], case.input
    if pdf.suffix.lower() != ".pdf":
        run(cli, "images", "-o", d / "input.pdf", pdf)
        pdf = d / "input.pdf"
    own = [page_text(pdf, p) for p in case.pages]
    if any(t.strip() for t in own):
        notes.append("already carries text, which the OCR skips: that text scored, then rebuilt")
        rows += score(case, "own text", own, None)
        pdf = rebuild(cli, pdf, d, notes)
    for p in case.pages:
        note = upscale_note(p, picture_ppi(pdf, p), *page_size(pdf, p))
        if note:
            notes.append(note)
    pages = ",".join(map(str, case.pages))
    run(cli, "optimize-scans", "-o", d / "optimized.pdf", pdf, "--pages", pages)
    for label, src in (("as is", pdf), ("optimized", d / "optimized.pdf")):
        stem = label.replace(" ", "-")
        start = time.perf_counter()
        log = run(cli, "ocr", "-o", d / f"{stem}-ocr.pdf", src, "--pages", pages)
        secs = (time.perf_counter() - start) / len(case.pages)
        notes += [f"{label}: {ln}" for ln in log.splitlines() if "skipped" in ln]
        read = [read_back(cli, d / f"{stem}-ocr.pdf", p, reader) for p in case.pages]
        for p, text in zip(case.pages, read):
            (d / f"{stem}-page-{p}.txt").write_text(text, encoding="utf-8")
        rows += score(case, label, read, secs)
    if tesseract:
        start = time.perf_counter()
        read = [tesseract_read(pdf, p, d, case.lang, tessdata) for p in case.pages]
        rows += score(case, "Tesseract", read, (time.perf_counter() - start) / len(case.pages))
    return rows, notes


def report(results):
    pct = lambda x: "     -" if x is None else f"{x * 100:5.1f}%"
    print(f"{'case':<28} {'variant':<10} {'page':>4} {'CER':>6} {'CER(f)':>6} {'words':>6} {'(f)':>6} {'s/page':>6}")
    for r in (r for res in results for r in res["rows"]):
        secs = "     -" if r["seconds"] is None else f"{r['seconds']:6.2f}"
        print(
            f"{r['case'][:28]:<28} {r['variant']:<10} {r['page']:>4} {pct(r['cer'])} "
            f"{pct(r['cer_folded'])} {pct(r['words'])} {pct(r['words_folded'])} {secs}"
        )
    for res in results:
        for n in res["notes"]:
            print(f"{res['case']}: {n}")


def slow_levenshtein(a, b):
    row = list(range(len(b) + 1))
    for i, ca in enumerate(a, 1):
        prev, row[0] = row[0], i
        for j, cb in enumerate(b, 1):
            prev, row[j] = row[j], min(row[j] + 1, row[j - 1] + 1, prev + (ca != cb))
    return row[-1]


def self_test():
    failures, checks = [], 0

    def check(what, got, want):
        nonlocal checks
        checks += 1
        if got != want:
            failures.append(f"{what}: expected {want!r}, got {got!r}")

    rng = random.Random(221)
    for _ in range(300):
        a, b = ("".join(rng.choice("abé ") for _ in range(rng.randrange(150))) for _ in "ab")
        check(f"levenshtein({a!r}, {b!r})", levenshtein(a, b), slow_levenshtein(a, b))
    check("kitten, sitting", levenshtein("kitten", "sitting"), 3)
    check("to empty", levenshtein("abc", ""), 3)
    check("normalise", normalise("ﬁne  été collec-\n  ted A-12\f"), "fine été collected A-12")
    check(
        "fold",
        fold("Zażółć gęślą jaźń. Größe, cœur « oui » – “ok” …"),
        'Zazolc gesla jazn. Grosse, coeur " oui " - "ok" ...',
    )
    check("words found", words_found("the cat the dog", "The dog, cat!"), 0.75)
    check("cer", cer("abcd", "abed"), 0.25)
    check("render dpi, letter", render_dpi(612, 792), 300)
    check("render dpi, a 29 x 37 in phone scan", round(render_dpi(29 * 72, 37 * 72), 1), 189.2)
    check("no note at the pictures' own size", upscale_note(1, 300, 612, 792), None)
    check("no note without pictures", upscale_note(1, None, 612, 792), None)
    check("note when upscaled", "2.6 times" in (upscale_note(2, 72, 29 * 72, 37 * 72) or ""), True)
    toml = '[[case]]\nname = "a"\ninput = "x.pdf"\ntruth = ["1.txt", "2.txt"]\n'
    cases = parse_cases(tomllib.loads(toml), pathlib.Path("/m"))
    check("default pages", cases[0].pages, [1, 2])
    check("default lang", cases[0].lang, "eng")
    check("lang", parse_cases(tomllib.loads(toml + 'lang = "fra+eng"\n'), pathlib.Path("/m"))[0].lang, "fra+eng")
    check("relative paths", (cases[0].input, cases[0].truth[1]), (pathlib.Path("/m/x.pdf"), pathlib.Path("/m/2.txt")))
    for bad, why in (
        (toml + "pages = [2]\n", "1 pages but 2"),
        (toml + "dpi = 3\n", "unknown keys"),
        (toml + 'lang = "-l x"\n', "isn't a Tesseract language"),
        ("", "no [[case]]"),
    ):
        try:
            parse_cases(tomllib.loads(bad), pathlib.Path("/m"))
            check(f"refuses {why}", "accepted", why)
        except ValueError as e:
            check(f"refuses {why}", why in str(e), True)
    for f in failures:
        print(f"self-test: {f}")
    print(f"self-test: {checks} checks, {len(failures)} failed")
    return not failures


if __name__ == "__main__":
    if sys.argv[1:] == ["--self-test"]:
        sys.exit(0 if self_test() else 1)
    ap = argparse.ArgumentParser(description="OCR accuracy measure; see the file's docstring.")
    ap.add_argument("manifest", type=pathlib.Path)
    ap.add_argument("--work", type=pathlib.Path, required=True, help="where outputs and read text go")
    ap.add_argument("--cli", type=pathlib.Path, default=CLI)
    ap.add_argument("--tesseract", action="store_true")
    ap.add_argument("--tessdata", type=pathlib.Path, help="Tesseract's models (*.traineddata), if not its own")
    ap.add_argument("--reader", choices=["poppler", "limarepdf"], default="poppler", help="what reads the text back")
    ap.add_argument("--json", type=pathlib.Path, help="the scores and notes as JSON")
    args = ap.parse_args()
    manifest = args.manifest.resolve()
    tessdata = args.tessdata and args.tessdata.resolve()
    results, failed = [], 0
    for case in parse_cases(tomllib.loads(manifest.read_text(encoding="utf-8")), manifest.parent):
        try:
            rows, notes = measure(case, args.cli, args.work.resolve(), args.tesseract, tessdata, args.reader)
        except (RuntimeError, OSError) as e:
            rows, notes, failed = [], [f"failed: {e}"], failed + 1
        results.append({"case": case.name, "rows": rows, "notes": notes})
    report(results)
    if args.json:
        args.json.write_text(json.dumps(results, indent=1, ensure_ascii=False), encoding="utf-8")
    print(f"{len(results)} cases, {failed} failed")
    sys.exit(1 if failed else 0)

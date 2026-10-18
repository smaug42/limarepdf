"""Time limarepdf-cli and record its peak memory on large generated files and named corpus files.

Usage, from the repository: python3 crates/limarepdf-core/tools/measure.py [--runs N] [--only TEXT]

Builds the release CLI, makes the test files in target/tmp/measure (a 1,000-page text file and a
200-page colour scan at 300 dpi), runs each task N times (3 by default) and prints a Markdown table
for docs/performance.md: the median time and the largest peak memory (resident set size) of the
runs, with the machine and commit. --only runs the tasks whose name contains TEXT.

The corpus rows use files from tools/fetch-render-samples.sh and are skipped if those aren't
fetched. Peak memory comes from the operating system through os.wait4, as `/usr/bin/time -v`
reports it, so this runs on Linux and macOS only. On Linux a reading can't go below what this
script had resident when it started the program (about 20 MB); the table marks such readings
with ≤. Standard library only.
"""

import argparse
import os
import platform
import random
import statistics
import struct
import subprocess
import sys
import time
import zlib
from pathlib import Path

ROOT = Path(__file__).resolve().parents[3]
CLI = ROOT / "target" / "release" / "limarepdf-cli"
WORK = ROOT / "target" / "tmp" / "measure"
CORPUS = ROOT / "crates" / "limarepdf-core" / "tests" / "data" / "render"

TEXT_PAGES = 1000
SCAN_PAGES = 200
# US Letter at 300 dpi.
SCAN_WIDTH = 2550
SCAN_HEIGHT = 3300

WORDS = (
    "the of and to in is that for it as with was on be by this are or from at which an have not "
    "document page section report figure table value result method sample measure annual review "
    "between within during following provided required general specific information management"
).split()


def pdf_file(path, contents):
    """Write a PDF with one page per content stream, in Helvetica, on US Letter pages."""
    objects = [
        b"<< /Type /Catalog /Pages 2 0 R >>",
        None,  # the page tree, once the pages' numbers are known
        b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Encoding /WinAnsiEncoding >>",
    ]
    kids = []
    for content in contents:
        data = zlib.compress(content)
        objects.append(
            b"<< /Length %d /Filter /FlateDecode >>\nstream\n" % len(data) + data + b"\nendstream"
        )
        objects.append(
            b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] "
            b"/Resources << /Font << /F1 3 0 R >> >> /Contents %d 0 R >>" % (len(objects))
        )
        kids.append(b"%d 0 R" % len(objects))
    objects[1] = b"<< /Type /Pages /Kids [%s] /Count %d >>" % (b" ".join(kids), len(kids))
    out = bytearray(b"%PDF-1.7\n%\xe2\xe3\xcf\xd3\n")
    offsets = []
    for number, body in enumerate(objects, 1):
        offsets.append(len(out))
        out += b"%d 0 obj\n" % number + body + b"\nendobj\n"
    xref = len(out)
    out += b"xref\n0 %d\n0000000000 65535 f \n" % (len(objects) + 1)
    for offset in offsets:
        out += b"%010d 00000 n \n" % offset
    out += b"trailer\n<< /Size %d /Root 1 0 R >>\nstartxref\n%d\n%%%%EOF\n" % (len(objects) + 1, xref)
    path.write_bytes(out)


def text_file(path):
    """A report-like text file: a heading, 48 lines of words and a rule per page."""
    rng = random.Random(1)
    contents = []
    for page in range(1, TEXT_PAGES + 1):
        lines = [b"BT /F1 16 Tf 72 720 Td (Section %d: measured results) Tj ET" % page]
        lines.append(b"0.5 w 72 708 m 540 708 l S")
        for line in range(48):
            words = " ".join(rng.choice(WORDS) for _ in range(rng.randint(9, 13)))
            lines.append(b"BT /F1 10 Tf 72 %d Td (%s) Tj ET" % (690 - line * 13, words.encode()))
        lines.append(b"BT /F1 9 Tf 300 40 Td (%d) Tj ET" % page)
        contents.append(b"\n".join(lines))
    pdf_file(path, contents)


def png_file(path, width, height):
    """A colour page like a scan: off-white paper with noise, and lines of dark word-shaped blocks."""
    rng = random.Random(2)
    paper = bytes(246 + i % 6 for i in range(256))
    ink = bytes(30 + i % 8 for i in range(256))
    rows = []
    margin = width // 12
    line_height = height // 60
    for top in range(margin, height - margin, line_height):
        spans = []
        x = margin
        while x < width - margin:
            word = rng.randint(width // 60, width // 12)
            spans.append((x, min(x + word, width - margin)))
            x += word + width // 100
        rows.append((top, top + line_height * 2 // 3, spans))
    raw = bytearray()
    band = 0
    for y in range(height):
        row = bytearray(rng.randbytes(width * 3).translate(paper))
        while band < len(rows) and y >= rows[band][1]:
            band += 1
        if band < len(rows) and y >= rows[band][0]:
            for left, right in rows[band][2]:
                row[left * 3 : right * 3] = rng.randbytes((right - left) * 3).translate(ink)
        raw += b"\0" + row

    def chunk(kind, data):
        return struct.pack(">I", len(data)) + kind + data + struct.pack(">I", zlib.crc32(kind + data))

    header = struct.pack(">IIBBBBB", width, height, 8, 2, 0, 0, 0)
    path.write_bytes(
        b"\x89PNG\r\n\x1a\n"
        + chunk(b"IHDR", header)
        + chunk(b"IDAT", zlib.compress(bytes(raw), 6))
        + chunk(b"IEND", b"")
    )


def scan_file(path):
    """One scanned page made with the CLI (picture to PDF, then a colour JPEG at 300 dpi),
    merged SCAN_PAGES times."""
    png = WORK / "scan-page.png"
    one = WORK / "scan-page-flate.pdf"
    jpeg = WORK / "scan-page.pdf"
    png_file(png, SCAN_WIDTH, SCAN_HEIGHT)
    cli("images", "--dpi", "300", "--output", one, png)
    cli("optimize-scans", "--mode", "color", "--max-dpi", "300", "--no-deskew", "--output", jpeg, one)
    cli("merge", "--output", path, *([jpeg] * SCAN_PAGES))
    png.unlink()
    one.unlink()


def cli(*args):
    subprocess.run([CLI, *map(str, args)], check=True, stdout=subprocess.DEVNULL)


def measure(args, program=CLI):
    """Run the CLI once; return the seconds it took and its peak memory in bytes.

    On Linux a child's peak starts at what this process had resident when it forked, so
    the files are made in a separate process and the floor is measured (see main)."""
    start = time.perf_counter()
    process = subprocess.Popen(
        [program, *map(str, args)], stdout=subprocess.DEVNULL, stderr=subprocess.PIPE
    )
    _, status, usage = os.wait4(process.pid, 0)
    seconds = time.perf_counter() - start
    process.returncode = os.waitstatus_to_exitcode(status)
    error = process.stderr.read().decode(errors="replace").strip()
    process.stderr.close()
    if process.returncode != 0:
        raise SystemExit(f"{program} {' '.join(map(str, args))} failed: {error}")
    # Kilobytes on Linux, bytes on macOS.
    peak = usage.ru_maxrss if sys.platform == "darwin" else usage.ru_maxrss * 1024
    return seconds, peak


def tasks(text, scan):
    out = WORK / "out"
    rows = [
        ("1,000 text pages: info", ["info", text]),
        ("1,000 text pages: extract all text", ["text", text]),
        ("1,000 text pages: search", ["search", text, "annual review"]),
        ("1,000 text pages: render at 100 dpi", ["render", "--dpi", "100", "--out-dir", out, text]),
        ("1,000 text pages: watermark", ["watermark", "--text", "DRAFT", "--output", out / "w.pdf", text]),
        ("1,000 text pages: merge 3 copies", ["merge", "--output", out / "m.pdf", text, text, text]),
        ("1,000 text pages: split into 20 files", ["split", "--every", "50", "--out-dir", out, text]),
        (
            "1,000 text pages: AES-256 encrypt",
            ["encrypt", "--owner-password", "o", "--output", out / "e.pdf", text],
        ),
        ("200-page scan: info", ["info", scan]),
        ("200-page scan: render at 100 dpi", ["render", "--dpi", "100", "--out-dir", out, scan]),
        ("200-page scan: watermark", ["watermark", "--text", "DRAFT", "--output", out / "w.pdf", scan]),
        (
            "200-page scan: AES-256 encrypt",
            ["encrypt", "--owner-password", "o", "--output", out / "e.pdf", scan],
        ),
    ]
    corpus = [
        ("pdfbox/5134.pdf", "complex vectors: 73,000 paths on one page"),
        ("pdfjs/bug1791583.pdf", "complex vectors: 27,000 paths on one page"),
        ("pdfbox/1910.pdf", "stand-in fonts: 42 pages, Japanese fonts not embedded"),
        ("pdfjs/bug1905623.pdf", "stand-in fonts: 203 pages, Japanese fonts not embedded"),
    ]
    for name, what in corpus:
        path = CORPUS / name
        if path.exists():
            rows.append((f"{name} ({what}): render at 150 dpi", ["render", "--out-dir", out, path]))
        else:
            print(f"skipped {name}: run tools/fetch-render-samples.sh first", file=sys.stderr)
    return rows


def size(path):
    return f"{path.stat().st_size / 1e6:.1f} MB"


def machine():
    cpu = platform.processor() or platform.machine()
    memory = ""
    try:
        for line in Path("/proc/cpuinfo").read_text().splitlines():
            if line.startswith("model name"):
                cpu = line.split(":", 1)[1].strip()
                break
        for line in Path("/proc/meminfo").read_text().splitlines():
            if line.startswith("MemTotal"):
                memory = f", {int(line.split()[1]) / 1024 / 1024:.0f} GB memory"
                break
    except OSError:
        pass
    commit = subprocess.run(
        ["git", "rev-parse", "--short", "HEAD"], cwd=ROOT, capture_output=True, text=True
    ).stdout.strip()
    dirty = subprocess.run(
        ["git", "status", "--porcelain", "--untracked-files=no"],
        cwd=ROOT,
        capture_output=True,
        text=True,
    ).stdout.strip()
    return (
        f"{cpu}, {os.cpu_count()} threads{memory}, {platform.system()} {platform.release()}; "
        f"commit {commit}{' with uncommitted changes' if dirty else ''}"
    )


def main():
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--runs", type=int, default=3)
    parser.add_argument("--only", default="")
    parser.add_argument("--make", action="store_true", help=argparse.SUPPRESS)
    options = parser.parse_args()
    text = WORK / f"text-{TEXT_PAGES}.pdf"
    scan = WORK / f"scan-{SCAN_PAGES}.pdf"
    if options.make:
        if not text.exists():
            print("making the text file", file=sys.stderr)
            text_file(text)
        if not scan.exists():
            print("making the scan", file=sys.stderr)
            scan_file(scan)
        return
    if not hasattr(os, "wait4"):
        raise SystemExit("needs os.wait4 (Linux or macOS)")

    subprocess.run(
        ["cargo", "build", "--release", "-p", "limarepdf-cli"], cwd=ROOT, check=True
    )
    (WORK / "out").mkdir(parents=True, exist_ok=True)
    subprocess.run([sys.executable, __file__, "--make"], check=True)
    floor = max(measure([], "true")[1] for _ in range(3))

    print(f"Machine: {machine()}. Release build; median of {options.runs} runs.\n")
    print(f"Files: {text.name} ({size(text)}), {scan.name} ({size(scan)}).\n")
    print(
        f"Peak memory can't be measured below {floor / 1e6:.0f} MB here (the measuring "
        "process's own), so ≤ marks a reading at that floor.\n"
    )
    print("| Task | Time | Peak memory |")
    print("| --- | --- | --- |")
    for name, args in tasks(text, scan):
        if options.only not in name:
            continue
        results = [measure(args) for _ in range(options.runs)]
        seconds = statistics.median(r[0] for r in results)
        peak = max(r[1] for r in results)
        shown = f"≤ {floor / 1e6:.0f} MB" if peak <= floor * 1.1 else f"{peak / 1e6:.0f} MB"
        print(f"| {name} | {seconds:.2f} s | {shown} |", flush=True)


if __name__ == "__main__":
    main()

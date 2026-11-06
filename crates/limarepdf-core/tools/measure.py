"""Time limarepdf-cli and record its peak memory on large generated files and named corpus files.

Usage, from the repository: python3 crates/limarepdf-core/tools/measure.py [--runs N] [--only TEXT]
[--timeout SECONDS] [--text-pages N]

Builds the release CLI, makes the test files the chosen tasks need in target/tmp/measure (a 1,000-page
text file and a 200-page colour scan at 300 dpi, each made once and kept), runs each task N times (3 by
default) and prints a Markdown table for docs/developer/performance.md: the median time and the largest
peak memory (resident set size) of the runs, with the machine and commit. --only runs the tasks whose
name contains TEXT, and makes only the files those tasks use. --text-pages makes a text file of N pages
instead, for checking the script itself quickly (the table's numbers are then not comparable). Each run
writes into an output folder of its own, empty when it starts and removed after, so a run never meets
the files the one before wrote.

A run that takes longer than SECONDS (600 by default) is stopped, and so is making the test files,
with every program it started. A task that fails or runs out of time shows as such in the table, the
other tasks still run, and the script then exits with an error naming them. A run's error output goes to a temporary file, so a program that writes a lot of it
can't stall.

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
import shutil
import signal
import statistics
import struct
import subprocess
import sys
import tempfile
import threading
import time
import zlib
from pathlib import Path

ROOT = Path(__file__).resolve().parents[3]
CLI = ROOT / "target" / "release" / "limarepdf-cli"
WORK = ROOT / "target" / "tmp" / "measure"
# Each run's output folder, made empty before the run and removed after it.
OUT = WORK / "out"
CORPUS = ROOT / "crates" / "limarepdf-core" / "tests" / "data" / "render"

TEXT_PAGES = 1000
SCAN_PAGES = 200
# US Letter at 300 dpi.
SCAN_WIDTH = 2550
SCAN_HEIGHT = 3300
# Seconds a run may take before it's stopped. The slowest row took about a minute.
TIMEOUT = 600
# Bytes of a failed run's error output shown, from its end.
ERROR_TAIL = 2000

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


def text_file(path, pages):
    """A report-like text file of `pages` pages: a heading, 48 lines of words and a rule per page."""
    rng = random.Random(1)
    contents = []
    for page in range(1, pages + 1):
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


class Failed(Exception):
    """A run that exited with an error or was stopped for taking too long."""

    def __init__(self, message, timed_out=False):
        super().__init__(message)
        self.timed_out = timed_out


def tail(file):
    """The end of a failed run's error output, which usually says what went wrong."""
    length = file.seek(0, os.SEEK_END)
    file.seek(max(0, length - ERROR_TAIL))
    text = file.read().decode(errors="replace").strip()
    if length > ERROR_TAIL:
        return f"[the last {ERROR_TAIL:,} of {length:,} bytes] ...{text}"
    return text


def measure(args, program=CLI, timeout=TIMEOUT):
    """Run the CLI once; return the seconds it took and its peak memory in bytes.

    Raises Failed if it exits with an error, or if it runs longer than timeout seconds, when
    it's killed. Its error output goes to a temporary file rather than a pipe, which a program
    that writes a lot would fill and then wait on. On Linux a child's peak starts at what this
    process had resident when it forked, so the files are made in a separate process and the
    floor is measured (see main)."""
    command = f"{program} {' '.join(map(str, args))}"
    with tempfile.TemporaryFile() as errors:
        start = time.perf_counter()
        process = subprocess.Popen(
            [program, *map(str, args)], stdout=subprocess.DEVNULL, stderr=errors
        )
        expired = threading.Event()

        def stop():
            expired.set()
            os.kill(process.pid, signal.SIGKILL)

        timer = threading.Timer(timeout, stop)
        timer.daemon = True
        timer.start()
        # Wait for it to end without reaping it, so its process ID can't be reused while the
        # timer might still kill it. Then stop the timer and reap it with its resource usage.
        os.waitid(os.P_PID, process.pid, os.WEXITED | os.WNOWAIT)
        seconds = time.perf_counter() - start
        timer.cancel()
        timer.join()
        _, status, usage = os.wait4(process.pid, 0)
        process.returncode = os.waitstatus_to_exitcode(status)
        if expired.is_set() and process.returncode == -signal.SIGKILL:
            raise Failed(f"{command} timed out after {timeout:g} s", timed_out=True)
        if process.returncode != 0:
            raise Failed(f"{command} failed: {tail(errors)}")
    # Kilobytes on Linux, bytes on macOS.
    peak = usage.ru_maxrss if sys.platform == "darwin" else usage.ru_maxrss * 1024
    return seconds, peak


def tasks(text, pages, scan):
    out = OUT
    t = f"{pages:,} text pages"
    parts = -(-pages // 50)
    rows = [
        (f"{t}: info", ["info", text]),
        (f"{t}: extract all text", ["text", text]),
        (f"{t}: search", ["search", text, "annual review"]),
        (f"{t}: render at 100 dpi", ["render", "--dpi", "100", "--out-dir", out, text]),
        (f"{t}: watermark", ["watermark", "--text", "DRAFT", "--output", out / "w.pdf", text]),
        (f"{t}: merge 3 copies", ["merge", "--output", out / "m.pdf", text, text, text]),
        (
            f"{t}: split into {parts} file{'s' if parts > 1 else ''}",
            ["split", "--every", "50", "--out-dir", out, text],
        ),
        (
            f"{t}: AES-256 encrypt",
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


def run_once(args, timeout):
    """One run of a task, writing into OUT, which is empty when it starts and removed after: the
    CLI won't replace files, so a run must not meet the ones the run before wrote."""
    shutil.rmtree(OUT, ignore_errors=True)
    OUT.mkdir(parents=True)
    try:
        return measure(args, timeout=timeout)
    finally:
        shutil.rmtree(OUT, ignore_errors=True)


def make_files(needed, pages, timeout):
    """Make the generated files in a process of its own, in a session of its own, so that one
    that takes longer than timeout seconds is stopped with every program it started."""
    command = [sys.executable, __file__, "--text-pages", str(pages), "--make", *needed]
    process = subprocess.Popen(command, start_new_session=True)
    try:
        process.wait(timeout=timeout)
    except subprocess.TimeoutExpired:
        # Not reaped yet, so its process group's ID can't have been reused.
        os.killpg(process.pid, signal.SIGKILL)
        process.wait()
        raise SystemExit(f"making the test files took longer than {timeout:g} s; stopped")
    if process.returncode != 0:
        raise SystemExit(f"making the test files failed (exit status {process.returncode})")


def inputs(rows, text, scan):
    """The generated files the given tasks read, by name: "text", "scan", both or neither."""
    return [
        kind
        for kind, path in (("text", text), ("scan", scan))
        if any(path in args for _, args in rows)
    ]


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
    parser.add_argument("--timeout", type=float, default=TIMEOUT)
    parser.add_argument("--text-pages", type=int, default=TEXT_PAGES, help=argparse.SUPPRESS)
    parser.add_argument("--make", nargs="+", choices=("text", "scan"), help=argparse.SUPPRESS)
    options = parser.parse_args()
    pages = options.text_pages
    if pages < 1:
        parser.error("--text-pages needs at least 1")
    text = WORK / f"text-{pages}.pdf"
    scan = WORK / f"scan-{SCAN_PAGES}.pdf"
    if options.make:
        if "text" in options.make and not text.exists():
            print("making the text file", file=sys.stderr)
            text_file(text, pages)
        if "scan" in options.make and not scan.exists():
            print("making the scan", file=sys.stderr)
            scan_file(scan)
        return
    if not hasattr(os, "wait4") or not hasattr(os, "waitid"):
        raise SystemExit("needs os.wait4 and os.waitid (Linux or macOS)")

    subprocess.run(
        ["cargo", "build", "--release", "-p", "limarepdf-cli"], cwd=ROOT, check=True
    )
    WORK.mkdir(parents=True, exist_ok=True)
    chosen = [(name, args) for name, args in tasks(text, pages, scan) if options.only in name]
    needed = inputs(chosen, text, scan)
    if needed:
        make_files(needed, pages, options.timeout)
    try:
        floor = max(measure([], "true")[1] for _ in range(3))
    except (Failed, OSError) as error:
        raise SystemExit(f"couldn't measure the memory floor with `true`: {error}")

    print(f"Machine: {machine()}. Release build; median of {options.runs} runs.\n")
    files = [path for kind, path in (("text", text), ("scan", scan)) if kind in needed]
    if files:
        print(f"Files: {', '.join(f'{path.name} ({size(path)})' for path in files)}.\n")
    print(
        f"Peak memory can't be measured below {floor / 1e6:.0f} MB here (the measuring "
        "process's own), so ≤ marks a reading at that floor.\n"
    )
    print("| Task | Time | Peak memory |")
    print("| --- | --- | --- |")
    unfinished = []
    for name, args in chosen:
        # A task's remaining runs are skipped once one fails or runs out of time.
        try:
            results = [run_once(args, options.timeout) for _ in range(options.runs)]
        except Failed as error:
            print(error, file=sys.stderr, flush=True)
            if error.timed_out:
                print(f"| {name} | timed out after {options.timeout:g} s | – |", flush=True)
                unfinished.append(f"{name} (timed out)")
            else:
                print(f"| {name} | failed | – |", flush=True)
                unfinished.append(f"{name} (failed)")
            continue
        seconds = statistics.median(r[0] for r in results)
        peak = max(r[1] for r in results)
        shown = f"≤ {floor / 1e6:.0f} MB" if peak <= floor * 1.1 else f"{peak / 1e6:.0f} MB"
        print(f"| {name} | {seconds:.2f} s | {shown} |", flush=True)
    if unfinished:
        raise SystemExit(
            f"{len(unfinished)} of {len(chosen)} tasks didn't finish: {'; '.join(unfinished)}"
        )


if __name__ == "__main__":
    main()

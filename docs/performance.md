# Performance

Measured on 18 October 2026 on an Intel Core i5-9600K (6 cores) with 15 GB of memory, Linux, release build (LTO, one
codegen unit), at commit 6b67145, with `limarepdf-cli` and the measuring script described in
[Testing](testing.md#automated-tests). Each time is the median of three runs and includes starting the program. Peak
memory is the most the program held in memory at once (its resident set) in any of the three runs; readings shown as
"≤ 20 MB" are at the lowest the script can measure.

The files: a generated 1,000-page text file (1.5 MB; a heading, 48 lines of text and a rule on each page), a generated
200-page colour scan (139 MB; a 300 dpi Letter page of 694 KB JPEG, 200 times), and four public test files from the
[comparison with poppler](rendering.md#compared-with-poppler).

| Task | Time | Peak memory |
| --- | --- | --- |
| 1,000 text pages: document information | 0.01 s | ≤ 20 MB |
| 1,000 text pages: extract all text | 1.32 s | ≤ 20 MB |
| 1,000 text pages: search | 0.24 s | ≤ 20 MB |
| 1,000 text pages: render at 100 dpi to PNG files (all cores) | 4.59 s | 75 MB |
| 1,000 text pages: watermark | 0.09 s | 28 MB |
| 1,000 text pages: merge 3 copies into 3,000 pages | 0.07 s | 34 MB |
| 1,000 text pages: split into 20 files | 0.07 s | 125 MB |
| 1,000 text pages: AES-256 encrypt | 0.03 s | 29 MB |
| 200-page scan: document information | 0.06 s | 426 MB |
| 200-page scan: render at 100 dpi | 2.20 s | 426 MB |
| 200-page scan: watermark | 0.18 s | 569 MB |
| 200-page scan: AES-256 encrypt | 0.41 s | 568 MB |
| Complex drawing, 73,000 paths on one page (PDFBox 5134): render at 150 dpi | 58.5 s | 9,707 MB |
| The same, drawn in bands (2026-10-19, one run) | 54.4 s | 1,520 MB |
| Complex drawing, 27,000 paths on one page (pdf.js bug1791583): render at 150 dpi | 2.05 s | 642 MB |
| Japanese fonts not embedded, drawn with installed stand-ins, 42 pages (PDFBox 1910): render at 150 dpi | 0.53 s | 562 MB |
| The same, 203 pages (pdf.js bug1905623): render at 150 dpi | 2.69 s | 559 MB |

What the numbers show:

- Text documents are fast and small, even at 1,000 pages.
- Memory grows with the file: the 139 MB scan takes about three times its size to open, before any page is drawn,
  and about four times to change and save.
- Drawing cost grows with the number of paths and with resolution. The 73,000-path page takes 2.6 s and 707 MB at
  36 dpi but nearly a minute and almost 10 GB at 150 dpi; poppler draws it at 150 dpi in 9 s with 329 MB. In the app,
  zooming in on such a page is limited to the [tiles in view](rendering.md#high-zoom), but the whole page at a normal
  zoom costs the same as here. Since 2026-10-19 large pages are [drawn in bands](rendering.md#large-drawings): the
  same page at 150 dpi now takes 54 s and 1.5 GB, and at 72 dpi 9.9 s and 1.4 GB (was 10.4 s and 2.7 GB).
- The reasons for the 560 to 640 MB of the last three files weren't looked into: the 27,000-path file has no fonts at
  all, and one page of PDFBox 1910 at 36 dpi takes 114 MB.

## Memory in the desktop app

Measured by hand on the same machine and commit, each in a newly started app, reading the app's peak and current
memory (`VmHWM` and `VmRSS` in `/proc/<pid>/status`):

| Check | Peak memory | Memory afterwards |
| --- | --- | --- |
| Open the 200-page scan | 1.49 GB | 0.97 GB |
| The same, then 20 edits | 3.71 GB | 3.58 GB |
| Open the scan, zoom to 6400% and scroll | 1.47 GB | 0.98 GB |
| Three documents open: the scan, the 1,000 text pages and pdf.js bug1905623 | 0.92 GB | 0.92 GB |
| With the undo limit (2026-10-19): open the scan | 0.59 GB | 0.50 GB |
| With the undo limit: the same, then 20 edits | 1.94 GB | 1.66 GB |

Edits on the scan felt immediate. Each one added about 130 MB, close to the file's 139 MB, because each undo step
keeps a whole copy of the document. The first rows were measured when the app kept up to 20 steps per document
whatever its size. Undo steps are now limited to 1 GB per document, the oldest dropped first (about 7 steps on this
scan), and the last two rows were measured with the limit: 20 edits added 1.16 GB instead of 2.6 GB.
Zooming to 6400% added nothing measurable. Peaks vary somewhat from one session to the next: the three documents
together peaked lower than the scan alone did in the other sessions.

These are timings of single tasks, not a benchmark suite, and apart from the one poppler timing no comparison with
other software was made. hayro's own documentation says its renderer has not yet been tuned for performance.

OCR speed is in [Scanning and OCR](scanning-and-ocr.md#accuracy-and-speed).

## How the desktop app stays responsive

- Pages are drawn lazily on a pool of worker threads, newest request first, with one page of prefetch either side.
- Pages outside the view are dropped from memory, and thumbnails are drawn at their own small size.
- At high zoom only the part of the page in view is drawn, in 512-pixel tiles
  ([details](rendering.md#high-zoom)). A tile takes about 1 ms on text pages and about 20 ms on a dense form, at
  any zoom, while a whole Letter page at 1152 dpi takes about 0.4 s and 500 MB.
- A whole-page drawing that is no longer wanted (a zoom passed through, changed colours, an edited document) stops
  at its next [band](rendering.md#large-drawings) instead of running to the end.
- Edits re-save the document without compressing it, and undo keeps plain snapshots (a whole copy of the document
  per step, up to 1 GB per document, see [above](#memory-in-the-desktop-app)).
- OCR, optimising, exporting images and printing run in the background.
- A [recovery copy](gui.md#recovery-copies) is made at most once a minute per changed document. The window only
  copies the document in memory; a background thread turns that into a file and writes it. On the 200-page scan the
  copy in memory took 16 to 57 ms and making the file about 20 ms (138 MB, before writing it to disk); on the 1,000
  text pages about 2 ms each (measured 2026-10-19). The file leaves out objects nothing uses any more, as a save
  does, which takes longest on files with many objects: on a 13,000-object file 57 ms instead of 10 ms without it
  (measured 2026-10-19). While a copy is written, the app holds up to about two more copies of the document's size
  in memory.

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
  zoom costs the same as here.
- The reasons for the 560 to 640 MB of the last three files weren't looked into: the 27,000-path file has no fonts at
  all, and one page of PDFBox 1910 at 36 dpi takes 114 MB.

These are timings of single tasks, not a benchmark suite, and apart from the one poppler timing no comparison with
other software was made. hayro's own documentation says its renderer has not yet been tuned for performance.

OCR speed is in [Scanning and OCR](scanning-and-ocr.md#accuracy-and-speed).

## How the desktop app stays responsive

- Pages are drawn lazily on a pool of worker threads, newest request first, with one page of prefetch either side.
- Pages outside the view are dropped from memory, and thumbnails are drawn at their own small size.
- At high zoom only the part of the page in view is drawn, in 512-pixel tiles
  ([details](rendering.md#high-zoom)). A tile takes about 1 ms on text pages and about 20 ms on a dense form, at
  any zoom, while a whole Letter page at 1152 dpi takes about 0.4 s and 500 MB.
- Edits re-save the document without compressing it, and undo keeps plain snapshots.
- OCR, optimising, exporting images and printing run in the background.

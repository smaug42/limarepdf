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

### Splitting

Measured on 20 October 2026 on the same machine, one run each, `limarepdf-cli split --every N`: before at commit
ec72e97, after with each part holding only what its pages use and written as soon as it is made. Before, every part
was a whole copy of the document, and all of them were held until the last was made.

| Split | Before | After |
| --- | --- | --- |
| 200-page scan, 1 page per file | over 8 GB (stopped at that limit) | 0.31 s, 406 MB |
| 200-page scan, 2 pages per file | over 8 GB (stopped at that limit) | 0.23 s, 406 MB |
| 200-page scan, 10 pages per file | 0.95 s, 2,827 MB | 0.17 s, 406 MB |
| 200-page scan, 50 pages per file | 0.32 s, 680 MB | 0.16 s, 407 MB |
| 1,000 text pages, 1 page per file | 6.42 s, 5,219 MB | 2.00 s, 39 MB |
| 1,000 text pages, 50 pages per file | 0.09 s, 119 MB | 0.06 s, 38 MB |

What is left is mostly the document itself, as opened. The files written are the same, byte for byte, as before
(compared on 390 of the 1,089 public test files, split every 3 pages).

### Reading order, renderers, picture sizes and signatures

Measured on 20 October 2026 on the same machine: before at commit dc4e5e4, after with the changes below. Five
places an outside review suggested could be faster; three were changed. The command-line rows are the median of
three runs, the rest single runs of small measuring programs, each in a process of its own.

**Reading order** (the order a page's words are read in, used by selection, search, copying and text export). A
page is cut at the widest empty band, then each side again, so a page of many separate lines was cut one line at a
time: time grew with the square of the lines, and each cut went one call deeper, until the program ran out of
stack and closed. Now every wide band is cut at once, which gives the same order (shown in a test against the old
method, and the text of all 1,089 public test files is the same), with no deepening calls.

| Generated page | Before | After |
| --- | --- | --- |
| 30,000 words in 3,000 lines set apart | 0.42 s | 0.10 s |
| The same words, lines close together | 0.10 s | 0.10 s |
| 12,000 words in 6,000 lines set apart | 0.32 s | 0.04 s |
| 20,000 lines set apart | 1.74 s; closed with a stack overflow on a thread of 2 MB (the app's and the command line's) | 0.08 s |
| 50,000 lines set apart (a 51 KB file) | closed with a stack overflow | 0.17 s |
| 50,000 words spaced along one line | closed with a stack overflow | 4.5 s |

Columns are still cut one at a time, as how wide a column must be depends on what is left; only a crafted page
gets slow that way.

**One renderer per thread.** Exporting pages and finding blank pages made a new renderer for every page, so each
page read its fonts again. Each thread now keeps one:

| Task | Before | After |
| --- | --- | --- |
| 1,000 text pages: render at 100 dpi | 3.90 s, 71 MB | 3.41 s, 80 MB |
| 200-page scan: render at 100 dpi | 2.13 s, 407 MB | 1.98 s, 407 MB |
| PDFBox 1910 (42 pages, Japanese fonts): render at 150 dpi | 0.51 s, 523 MB | 0.42 s, 599 MB |
| pdf.js bug1905623 (203 pages, Japanese fonts): render at 150 dpi | 2.13 s, 597 MB | 1.60 s, 634 MB |
| pdf.js bug1905623: remove blank pages | 0.95 s, 485 MB | 0.67 s, 457 MB |

Drawn one after another, the 203 pages take 2.9 s with a new renderer each and 0.87 s with one, but one renderer
keeps every font it has read (1910: 161 MB against 533 MB), which is why it is one per thread. OCR still makes one
per page: it reads only pages without text, where there are no fonts to keep.

**Picture sizes.** Choosing a picture to place, and a picture stamp, decoded the whole picture to learn its size;
now the size comes from the file's header. Peak memory includes the file itself, read whole.

| Picture | Before | After |
| --- | --- | --- |
| PNG, 8000 × 8000 (181 MB) | 0.60 s, 544 MB | under 1 ms, 177 MB |
| TIFF, 10 pages of 2550 × 3300 (252 MB, every page decoded) | 0.03 s, 487 MB | under 1 ms, 246 MB |
| JPEG, 8000 × 8000 (55 MB; only its header was ever read) | 7 ms, 109 MB | under 1 ms, 57 MB |

**Not changed, as the numbers didn't justify it:**

- *Reusing the app's page workers after an edit.* Each edit starts new workers on the new document. Reading the
  document again takes under 1 ms (the 1,000 text pages 2 ms); the copy handed to the workers takes 1 to 21 ms (the
  scan) and is needed anyway. What is lost is the renderer's fonts: the next page takes 19 ms instead of 6 ms on
  PDFBox 1910, 12 ms instead of 2 ms on bug1905623. A renderer can't be kept across documents, as the fonts belong
  to the old one.
- *Pictures decoded again for each band.* Since pages are [drawn in bands](rendering.md#large-drawings), a picture
  across several bands is decoded once per band: a 300 dpi scan page takes 0.11 s instead of 0.05 s, at 600 dpi
  0.34 s instead of 0.11 s, the same pixels either way. Only hayro, which draws the pages, can keep decoded
  pictures, so this is recorded for discussion with it.
- *Reading signed documents again, and trusted certificates.* Checking a file with 1, 5, 20 and 50 signatures
  takes 1 ms, 8 ms, 74 ms and 0.40 s, of which reading the document is 1 to 4 ms. The trusted certificates are
  read again for each signature: with 121 of them (a whole system list, far more than anyone trusts in LimarePDF)
  that adds 5 ms per signature, with a few nothing measurable.

The command-line program is 28.04 MB after these changes, 30 KB (0.1%) larger than before.

### Straightening scans

Measured on 21 October 2026 on the same machine: before at commit c611c9b, after with the change below, two runs
each in a process of their own, Optimise scans on one generated 4000 × 4000 page with despeckling off. To find a
scan's tilt, LimarePDF kept the position of every dark pixel before taking a sample of at most about 200,000 of
them. It now counts them first, stops there when there are too few or more than half the page, and keeps only the
sample: the same points, so the same angle.

| Page | Before, straightening on | After, straightening on | Straightening off |
| --- | --- | --- | --- |
| All black | 0.23 s, 211 MB | 0.18 s, 86 MB | 0.18 s, 86 MB |
| Two black rows in five (40% dark) | 0.26 s, 138 MB | 0.24 s, 86 MB | 0.17 s, 86 MB |

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
| With bands (2026-10-19): open PDFBox 5134, zoom at once to over 400%, then to over 1200% | 3.96 GB | 2.28 GB |

Edits on the scan felt immediate. Each one added about 130 MB, close to the file's 139 MB, because each undo step
keeps a whole copy of the document. The first rows were measured when the app kept up to 20 steps per document
whatever its size. Undo steps are now limited to 1 GB per document, the oldest dropped first (about 7 steps on this
scan), and the last two rows were measured with the limit: 20 edits added 1.16 GB instead of 2.6 GB.
Zooming to 6400% added nothing measurable. The 5134 row was measured on a 3840 × 2160 screen at scale 1, the
window not full screen: zooming in quickly straight after opening keeps several whole-page drawings and the tiles
running at once, each drawing stopping only at its next band. The page showed enlarged until zooming stopped, then
was drawn again quickly. Before bands, the same zooming would have come to an estimated 11 GB. Peaks vary somewhat from one session to the next: the three documents
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

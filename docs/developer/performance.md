# Performance

Measured on 18 October 2026 on an Intel Core i5-9600K (6 cores) with 15 GB of memory, Linux, release build (LTO, one
codegen unit), at commit 6b67145, with `limarepdf-cli` and the measuring script described in
[Testing](testing.md#automated-tests). Each time is the median of three runs and includes starting the program. Peak
memory is the most the program held in memory at once (its resident set) in any of the three runs; readings shown as
"≤ 20 MB" are at the lowest the script can measure.

The files: a generated 1,000-page text file (1.5 MB; a heading, 48 lines of text and a rule on each page), a generated
200-page colour scan (139 MB; a 300 dpi Letter page of 694 KB JPEG, 200 times), and four public test files from the
[comparison with poppler](testing.md#compared-with-poppler).

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
  zooming in on such a page is limited to the [tiles in view](../rendering.md#high-zoom), but the whole page at a normal
  zoom costs the same as here. Since 2026-10-19 large pages are [drawn in bands](../rendering.md#large-drawings): the
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
- *Pictures decoded again for each band.* Since pages are [drawn in bands](../rendering.md#large-drawings), a picture
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

OCR speed is in [Scans and OCR](../scanning-and-ocr.md#accuracy-and-speed).

## How the desktop app stays responsive

- Pages are drawn lazily on a pool of worker threads, newest request first, with one page of prefetch either side.
- Pages outside the view are dropped from memory, and thumbnails are drawn at their own small size.
- At high zoom only the part of the page in view is drawn, in 512-pixel tiles
  ([details](../rendering.md#high-zoom)). A tile takes about 1 ms on text pages and about 20 ms on a dense form, at
  any zoom, while a whole Letter page at 1152 dpi takes about 0.4 s and 500 MB.
- A whole-page drawing that is no longer wanted (a zoom passed through, changed colours, an edited document) stops
  at its next [band](../rendering.md#large-drawings) instead of running to the end.
- Edits re-save the document without compressing it, and undo keeps plain snapshots (a whole copy of the document
  per step, up to 1 GB per document, see [above](#memory-in-the-desktop-app)).
- OCR, optimising, exporting images, printing, search and saving run in the background (search since batch 22: a
  thread per tab, [below](#search); saving since batch 37, ENH-35: a thread per save, which writes a copy of the
  document, [measured below](#startup-and-responsiveness)). A save's questions and checks still come first on the
  window's thread.
- A [recovery copy](../gui.md#recovery-copies) is made at most once a minute per changed document. The window only
  copies the document in memory; a background thread turns that into a file and writes it. On the 200-page scan the
  copy in memory took 16 to 57 ms and making the file about 20 ms (138 MB, before writing it to disk); on the 1,000
  text pages about 2 ms each (measured 2026-10-19). The file leaves out objects nothing uses any more, as a save
  does, which takes longest on files with many objects: on a 13,000-object file 57 ms instead of 10 ms without it
  (measured 2026-10-19). While a copy is written, the app holds up to about two more copies of the document's size
  in memory.

## Search

Batch 22 (ENH-02) moved the app's search to a thread of its own, made each page's text read once for its matches,
their snippets and their highlights together, and kept that text for the next queries (`TextIndex`). Measured on
26 October 2026 on the machine above with the fourth audit's search component benchmark, like for like: its probe
(`performance-probe.rs`) and its fixtures (12, 60, 120 and 300 pages of 38 short lines, "report" on each line; their
checksums in `target/searchbench/`), the `ci-test` profile, two Rayon threads, the median of three runs. "Before"
is HEAD at 1726ccc, "after" this change.

| Pages (matches) | Before: the app's search, on the window's thread (`search`, then each page with matches read again for its boxes) | After: the first query, on the search thread (pages read once, boxes included) | After: the next query, the text already read | All page sizes, before / after |
| --- | --- | --- | --- | --- |
| 12 (468) | 20.1 ms (6.8 + 13.3) | 5.0 ms | 0.19 ms | 0.07 / 0.01 ms |
| 60 (2,340) | 74.1 ms (24.5 + 49.6) | 23.3 ms | 0.86 ms | 1.5 / 0.06 ms |
| 120 (4,680) | 153.6 ms (52.3 + 101.3) | 46.4 ms | 1.7 ms | 6.0 / 0.12 ms |
| 300 (11,700) | 516 ms (172 + 344) | 121 ms | 4.0 ms | 58 / 0.37 ms |

What they show:

- The app's search did the work twice; it now reads each page once, and the window doesn't wait for it at all.
  The probe's own `search` call alone went from 172 to 126 ms on 300 pages (page IDs collected once, not twice a
  page); its second step, which reads each page again for its boxes through `find_text_matches`, is unchanged, but
  the app no longer takes it.
- Once read, the text answers another query in milliseconds: 4 ms for 11,700 matches on 300 pages, so typing more
  of a word searches almost at once.
- Collecting every page's size walked the page tree once a page (58 ms on 300 pages, growing with the square of the
  page count); `Pdf::page_sizes` walks it once (0.37 ms).
- The kept text took 22 MB for the 300 pages (73 KB a page of 38 lines: each character's folded and original form,
  its place, and each piece's character positions). All tabs together are capped at 256 MB, about 3,500 such pages:
  past that, the tab searched longest ago is let go first, then the pages already searched of a document bigger
  than that alone, read again when next searched.
- Handing the document to the search thread is a copy in memory, made once per change while a query is in the
  field: 0.3 ms for text-300.pdf, 1.8 ms for the 17 MB scan-20.pdf and 1.0 ms for the corpus's largest file (5 MB,
  107 pages), about 0.1 ms a megabyte (each the best of five).

## Styling selected words

Batch 24 (ENH-05) writes every changed line of a page in one parse and one commit (`Pdf::edit_texts`), each line
with the selected word written anew and the words around it cut out of its own shows, and every cut line read back
before the commit. Measured on 26 October 2026 on the machine above with a throwaway release-built probe
(`target/b24perf/`) on REL-07's `text-300.pdf`: the second word of every line made bold, as the read tool's
selection writes it, the median of five runs.

| Selection | Lines | `edit_texts` | The pages' words and objects read again after (the app's next frame) |
| --- | --- | --- | --- |
| One page | 49 | 4.0 ms | 3.3 ms |
| 20 pages | 980 | 80 ms | 69 ms |

No line fell back to a similar font. The change runs on the window's thread with the undo step and the pages'
redraw, as every edit does; at these rates a whole 300-page document would take about two seconds there, which
is recorded, not a threshold (6.3 has none for edits).

## Several objects at once

Batch 25 (ENH-06) moves, restyles and deletes the objects selected on a page in one parse and one commit
(`Pdf::set_object_boxes`, `set_path_styles`, `delete_objects`), what goes in before which operator after them found
once (`Following::all`, also for `edit_texts`), so a page is gone through once however many are selected; the
window finds the selection's boxes, the rectangle's objects and the selection again after a change in one pass
each. Measured on 26 October 2026 on the machine above, every object of page 1 changed at once, the median of five
runs, with a throwaway release-built probe (`target/b25perf/`), and the window's frames, headless, with a temporary
release-built app test (kept as `target/b25perf/frames_probe.rs.txt`):

| Page | Objects | Read them | Move all | Restyle the shapes | Bold every line | Delete all |
| --- | --- | --- | --- | --- | --- | --- |
| `text-300.pdf`, page 1 | 50 lines | 1.4 ms | 3.2 ms | – | 1.8 ms | 1.5 ms |
| `artwork.pdf` (the outlier) | 27,440 (27,274 shapes) | 270 ms | 1.1 s | 0.75 s | – | 0.80 s |

| Page, all its objects selected with Ctrl+A | A frame | While they are dragged | The frame letting go (the move, its undo step and redraw) |
| --- | --- | --- | --- |
| `text-300.pdf`, page 1 (50) | 0.6 ms | 0.6 ms | 6.9 ms |
| `artwork.pdf` (27,440) | 5.3 ms | 5.6 ms | 1.2 s |

A move of the artwork's objects takes less than half the 2.9 s the page takes to draw, and its frames stay well
within a 60 Hz frame while they are selected and dragged. Recorded, not thresholds (6.3 has none for edits).

## Startup and responsiveness

Measured on 26 October 2026 as the release checks' baseline (REL-07), from which their thresholds were set (accepted
the same day, for this machine and setup only; search's total wait after batch 22: ≤ 450 ms worst, and its
processing ≤ 300 ms, both met below):
the release build of `add-enhancements-02` at edaf365 with the change that adds the [timing log](architecture.md),
on the machine above (Intel Core i5-9600K, 6 cores, 15 GB of memory, NVMe SSD with btrfs, Linux
7.3). The app ran on an Xvfb display (1600 × 1000, 24-bit, no window manager), where Mesa draws in software, as
Xvfb has no GPU: the results describe the app with software drawing, not what a GPU and a real screen add or save.
The window was 1200 × 850 at scale 1 (a new settings folder's size), light theme, recovery copies off, each run
with new settings and recovery folders. The usual desktop session ran alongside (an editor, a browser): load
average 0.9 to 3.1, 2.4 to 7.7 GB of memory available. How it was measured is in
[Testing](testing.md#automated-tests).

Times are from the start (the launch, a key sent) to two ends: **built**, when the app's log says it finished the
frame showing the result (before that frame is painted and shown, which the app can't see), and **shown**, the
first screen capture showing it (captures of a small area every 5 ms: 5.0 ms apart at the median and 7.4 ms at
worst, each costing 0.2 ms, 1.4 ms at worst; a result is shown at most one interval, about 5 ms, before the
capture that saw it). Each figure is **median / worst**: 10 warm runs after one discarded, 5 cold ones (each after
the page cache was dropped), 200 keys for typing. No percentiles are given below 20 samples.

The documents, made up or public, the same byte for byte each time they are made:

| File | What it is |
| --- | --- |
| letter.pdf | a one-page letter, Helvetica |
| form.pdf | one page, 12 text fields without appearances |
| text-120.pdf | 120 pages of 48 lines of words each |
| text-300.pdf | the same, 300 pages |
| scan-20.pdf | 20 Letter pages of 300 dpi grey JPEG, 17 MB |
| artwork.pdf | pdf.js bug1791583, one page of 27,000 paths (the outlier) |

```
afccbe7c8377de8e8f3ce432a3fdde2608baa9d3abba2298558bfd4e006ae287  letter.pdf
d2549e55e613e52055dee31e682f07c2b455bbfdedeb293d04b95874f4307142  form.pdf
b6ddc85569e3e0ed17e78c9d4b83e1a6af70a9794ea1e2706437f7fc55bc1b1c  text-120.pdf
6ed2bc00ebc292460f2c869277f67a4b8b028168d8aad2041a60d406d4b3f20d  text-300.pdf
7084d39460e6073042bb6f2aaf911f883946d176c8fd5612cabbaf2367b2ae7e  scan-20.pdf
40538a43f8eb3843a8667d9e01d2f31973749f490231b742805d567395a0b8fc  artwork.pdf
```

### Opening a file from the command line

The first useful page: the app started with the file, until page 1 is drawn at the size it shows at.

| File | Built | Shown |
| --- | --- | --- |
| letter.pdf, warm | 335 / 373 ms | – |
| letter.pdf, warm, with screen captures | 305 / 356 ms | 325 / 375 ms |
| letter.pdf, cold | 862 / 898 ms | 885 / 920 ms |
| form.pdf, warm | 314 / 380 ms | – |
| text-120.pdf, warm | 368 / 507 ms | – |
| text-300.pdf, warm | 523 / 568 ms | – |
| scan-20.pdf, warm | 447 / 495 ms | – |
| artwork.pdf, warm | 2,904 / 3,048 ms | – |

- Most of the time to a small file's page is before the first frame: making the window, starting the renderer
  and `App::new` take about 260 ms warm and 820 ms cold (the process reaches `main` in 5 ms warm, 15 ms cold);
  the letter's page follows about 30 ms after the first frame.
- From built to shown took 20 / 22 ms warm and 22 / 24 ms cold: the software paint and the display.
- Taking captures every 5 ms changed nothing measurable (the warm runs with captures were no slower).
- Page 1 takes longer on longer documents although only page 1 is drawn: from the file read to page 1's drawing,
  about 90 ms on 120 pages and 220 ms on 300. The renderer reads the whole file again for itself (`worker.rs`,
  `render::load`); that this is the cost was not checked.
- Memory once the page was shown: 237 MB for the letter (230 MB cold), 257 MB for 300 pages, 346 MB for the scan,
  651 MB for the artwork.

### Typing

Each key sent alone, 60 to 120 ms after the last one's frame, 200 a script.

| Where | Built | Shown | Over 50 ms shown | Over 100 ms shown |
| --- | --- | --- | --- | --- |
| A form field (form.pdf) | 1.1 / 4.4 ms | 26.4 / 45.9 ms (200 keys) | 0 | 0 |
| Add text, a new box (letter.pdf) | 2.4 / 23.1 ms | 25.4 / 45.8 ms (58 keys) | 0 | 0 |
| Editing a line (letter.pdf) | 2.4 / 23.0 ms | 26.8 / 52.1 ms (98 keys) | 1 | 0 |

"Shown" covers only the keys whose characters landed in the captured area: past it, the box and the line ran on
to the page's edge. In the worst built times (23 ms), the key reached the app's frame 23 ms after it was sent and
the frame then took about a millisecond; why it waited was not looked into.

### Working while something runs

| What | Measured |
| --- | --- |
| Typing in the search box while artwork.pdf's page is drawn (1.34 s a drawing; 18 keys in 3 runs) | built 0.7 / 1.0 ms |
| Search on text-120.pdf (1,487 matches): Enter to the results' frame | 168 / 205 ms, of which searching 163 / 200 ms |
| Search on text-300.pdf (3,552 matches) | 513 / 521 ms, of which searching 506 / 515 ms |
| Ctrl+P to the Print dialog's frame, the printer search (a stand-in `lpstat` taking 4 s a call, called twice) running on | 2.9 / 3.8 ms; printers listed after 8.0 s |
| Ctrl+S on letter.pdf with an Add text box open: to the next frame | 27 / 41 ms (writing 4 / 12 ms) |
| The same on text-300.pdf | 218 / 236 ms (before the write 116 / 132 ms, writing 35 / 47 ms) |

Before Save writes, it keeps the open box's typing and makes its checks; which of them takes the 116 ms on 300
pages (5 ms on the letter) wasn't measured in the baseline. Save holds the window for its whole time: no frame, and
so no busy sign, until it finishes (ENH-35). Search held it too, until batch 22 (below).

Save on a thread of its own (batch 37, ENH-35, measured the same day the same way, the release build of this
change; 10 runs each, built times). Split with temporary points in the timing log, the 116 ms before the write
were: keeping the box's typing 7 ms (the edit itself), checking the file hadn't changed elsewhere 8 to 9 ms (the
file read once in full, as its time was within 2 s of when it was read: the reference copy is made just before
the app opens it; [PB-28](architecture.md)), and counting the document's redaction marks 63 ms. That count, and the
same one after every change, went through the whole page list once for every page (the annotation listing looked
each page up by its number); it now goes through it once. Before this change, once a file had been saved (its time then always within 2 s of
the reading after the write) or was opened just after a change, every save read it in full twice before writing
and once after, as a file found unchanged never became the version known; it now does, and the version after
writing is read on the save's thread.

| Ctrl+S with an Add text box open | To the busy sign's frame | To the saved frame | Writing (on its thread) |
| --- | --- | --- | --- |
| letter.pdf | 4.3 / 35.5 ms | 37.6 / 71.0 ms | 9.2 / 12.3 ms |
| text-300.pdf | 34.1 / 50.7 ms | 84.9 / 108.3 ms | 48.0 / 52.2 ms |

The busy sign showed in all 20 runs (the frame after Ctrl+S's). On the window's thread a save now holds it for
keeping the typing, its checks and the copy of the document (0.6 ms on text-300.pdf); the write and reading the
file back take place on the save's thread. A 300-page signed document (text-300.pdf signed with a test ID) asks
first, as typed text is a change its signature doesn't allow: Ctrl+S to the question, adding the changes after
what was signed included, took about 42 ms (one run, built). Typing while a save is written wasn't measured: on
these documents the write takes about 50 ms.

Search as you type (batch 22, measured the same day the same way, the release build of this change; the driver's
`typed` mode: each run empties the field with Ctrl+A and Backspace and types "glacier", 50 ms between keys; 10 runs
after the first search, which is reported apart as it reads every page; built times from the timing log). "Pause"
is the last key to the search starting, "processing" the search starting to the frame showing the first match,
"complete" to the frame with every match, "total" the last key to the first match's frame.

| Document | First search after opening: processing, complete, total | Later searches: pause, processing, total (median / worst) |
| --- | --- | --- |
| text-120.pdf (1,487 matches) | 47, 74, 250 ms | 212 / 222, 41 / 60, 256 / 272 ms |
| text-300.pdf (3,552 matches) | 37, 146, 246 ms | 210 / 223, 36 / 40, 248 / 253 ms |

The window keeps drawing while a search runs: during the first search on 300 pages (145 ms from start to end) it
built 5 frames, never more than 40 ms apart, where the search before held it for 506 ms with none. The later
searches find the text already read, so their matches come in one group, and "complete" is "processing". Most of
"processing" is the frame itself (the highlights and the results drawn), not the search, which takes 2 to 4 ms
then ([above](#search)). Keys typed while a search runs weren't timed on their own here.

### Memory over repeated use

text-300.pdf opened (the first time from the command line, then from the start screen's recent list), ten pages
down, a search for a word with 3,650 matches, the tab closed; the app's resident memory after each of 10 cycles:
313, 322, 327, 319, 324, 327, 325, 319, 328 and 334 MB. A line fitted to it rises 1.3 MB a cycle, within a 21 MB
spread: too few cycles to tell a slow leak from the allocator keeping memory; whether the closed documents' own memory is all given back was not checked, as no heap
profiler is installed here.

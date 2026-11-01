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
scan), and the last two rows were measured with the limit: 20 edits added 1.16 GB instead of 2.6 GB. Since batch
43 a step keeps only what its edit changed ([Undo history](#undo-history)): a note on a scan keeps about 5 KB, so
the history of 20 such edits is a few hundred kilobytes at most. The app's memory with them wasn't measured again.
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
- Undo keeps only the old values of the objects each edit changed, up to 100 steps within 1 GB per document
  ([Undo history](#undo-history)). The pages are still drawn from a copy of the document written out without
  compressing after each edit and each Undo.
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

## Replace all

Batch 46's stage 3 (NEW-04b) prepares Replace all on a copy of the document, off the window's thread, then reviews
it. Measured on 31 October 2026 on the machine above with the opt-in test `replace_all_speed_on_300_pages` (in
`editor/replace_all.rs`; `cargo test -p limarepdf-gui --bin limarepdf --profile ci-test -- --ignored
replace_all_speed --nocapture`): a made-up document of 300 Letter pages, ten 12-point Helvetica lines a page,
1,100 matches of "walrus" (three lines a page with one, every third page a line with two more) replaced with
"otter", every match replaced, nothing running over. The figures are the steady ones of three runs (a first run
read 2.4 s for the lines while the machine was busy).

| Lines changed as | Planning (pages read in parallel) | All of it: planning, writing the copy, judging each change |
| --- | --- | --- |
| Lines alone (Paragraph off) | 38 ms | 1.16 s |
| Paragraphs (Paragraph on: the page's ten lines are one) | 380 ms | 3.47 s |

What they show:

- Writing is the cost: each page's lines are written in one rewrite of its content (read back to check, as every
  edit is), then the page is read again to judge each change by the overflow rule. Pages are written one after
  another on the copy; planning, which only reads, runs in parallel.
- Before two changes, the paragraph case took 5.8 s and planning 2.05 s: the pages are now planned in parallel, and
  the first paragraph written on a page uses the setting its planning made instead of reading it again.
- When some replacements don't fit and others do, a second copy without them is made for Replace where it fits,
  doubling the time. Taking the copy is one undo step, as any edit: the document's objects compared once.

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

## Undo history

Batch 43 (PF-42) changed what an undo step keeps. Before, each step was a whole copy of the document as written
out (`Pdf::snapshot`), up to 20 steps within 1 GB per document, and Undo read that copy back as a new document
(`Pdf::reopen`). Now an edit clones the document's objects first (`Pdf::before`), compares them with the edited
ones after it (`Before::undo`) and keeps only the old values of what changed (a `Restore`); Undo and Redo put
those in place (`Pdf::restore`), up to 100 steps within 1 GB. Measured on 29 October 2026 on the machine above
with a throwaway release-built probe (`target/b43perf/`, its `--shipped` mode), core alone (no window, no
drawing), on REL-07's reference set, batch 41's made-up 300- and 10,000-page documents (`p300`, `big`) and two
made in memory: `scan-100` (the 20-page scan five times over) and `mixed` (text-300, then scan-20). The median of
five runs; the old way measured in the same run; memory counted by the probe's own allocator. Before the change,
the same probe compared the old way with compressed copies and a prototype of this one (scans don't compress: 99
to 105 % of their size, at 1 to 2 s a step on scan-100).

| Document | Its objects in memory | Old step | New step: a note, highlight, move, picture, redaction or properties | New: turn, delete or move a page | New: a watermark on every page |
| --- | --- | --- | --- | --- | --- |
| text-300 (300 pages) | 2.7 MB | 1.6 MB | 1.9 to 4.5 KB | 4.0 to 37 KB | 0.94 MB |
| artwork (1 page, 27,000 paths) | 4.2 MB | 3.0 MB | 2.7 to 57 KB | 56 KB (turn) | 57 KB |
| p300 (300 pages) | 1.3 MB | 0.27 MB | 1.9 to 4.5 KB | 0.83 to 0.84 MB (the first page change) | 0.94 MB |
| scan-20 | 16.5 MB | 16.3 MB | 1.9 to 4.6 KB | 4.0 to 4.3 KB | 64 KB |
| mixed (320 pages) | 19.1 MB | 18.0 MB | 1.9 to 4.5 KB | 4.0 to 39 KB | 1.0 MB |
| scan-100 | 82.3 MB | 81.7 MB | 1.9 to 4.6 KB | 4.0 to 14 KB | 0.32 MB |
| big (10,000 pages) | 44.8 MB | 9.3 MB | 1.9 to 4.5 KB | 27.6 MB (the first page change) | 31.4 MB |

| Document | Old: the copy before an edit | Old: Undo (a copy of now, read back) | New: clone and compare, each edit | New: Undo / Redo |
| --- | --- | --- | --- | --- |
| text-300 | 0.7 to 1.2 ms | 2.1 to 3.8 ms | 0.7 to 1.2 ms | ≤ 0.3 / 0.2 ms |
| artwork | 1.0 ms | 2.5 to 4.9 ms | 1.0 to 1.2 ms | 0 ms |
| p300 | 0.5 to 0.6 ms | 1.7 to 2.9 ms | 0.4 to 0.9 ms | ≤ 0.3 / 0.2 ms |
| scan-20 | 4.1 to 4.5 ms | 6.4 to 8.7 ms | 3.1 to 3.8 ms | ≤ 0.02 ms |
| mixed | 3.2 to 6.4 ms | 9.2 to 14.9 ms | 4.1 to 6.2 ms | ≤ 0.4 / 0.2 ms |
| scan-100 (quiet run) | 9.8 to 36 ms | 54 to 78 ms | 15.5 to 16.5 ms | ≤ 0.1 ms |
| big | 17.7 to 19.4 ms | 64 to 67 ms (107 after the watermark) | 21 to 24 ms | 0 ms; a page change 4.2 to 4.4 / 3.6 ms; the watermark 12.4 / 9.0 ms |

What the figures show:

- A step is now what its edit changed: a few kilobytes for everyday edits on every document, where the old copy
  was 0.3 to 82 MB. Two kinds are larger than the old copy: the first page change on a long document (it
  flattens the page tree, so every page's dictionary changes: 27.6 MB on 10,000 pages) and a watermark on every
  page of one (31.4 MB). The 1 GB budget holds 33 to 38 such steps there, and 100 of every other kind measured.
- Undo and Redo put the values in place: 0 to 12 ms, where the old Undo copied the document and read the copy back
  (up to 107 ms on 10,000 pages, 78 ms on scan-100).
- Each edit's own cost is close to the old copy's on small and middling documents and a little more on the
  largest: the clone and the compare cost 21 to 24 ms on 10,000 pages (the old copy 17.7 to 19.4 ms) and 15.5 to
  16.5 ms on scan-100 (the old copy about 10 ms for four edits, 36 ms for the fifth). Times vary with the
  machine's load: in a first run beside other work, scan-100's clone and compare took 39 to 43 ms against 20 to
  31 ms for the old copy in the same run, and its old Undo 80 to 87 ms.
- While an edit is made the clone is held until the compare ends, about the document's objects in memory (2.7 MB
  on text-300, 82 MB on scan-100, 44 MB on 10,000 pages), and then let go; the old copy was the written-out file
  (1.6, 82 and 9.3 MB), kept as the step. Undo and Redo allocate at most 4.9 MB (the watermark on 10,000 pages).
- `Restore::size`, which the budget counts, was within 1 % of the allocator's count on every step over 10 KB, and
  within 0.6 KB on the smallest.
- After every edit, Undo and Redo the document written out was the same, byte for byte, as before the edit and
  after it, on every document here and on the corpus (testing.md).
- Not changed: the pages are drawn from a copy of the document written out after each edit and each Undo
  (`app_snapshot`, then hayro reads it): 1.6 to 2.9 ms on text-300, 15 to 47 ms on scan-100, 42 to 101 ms on
  10,000 pages. On large documents that is now most of an Undo's cost (PB-37).

In the app, on batch 41's made-up 10,000-page `big.pdf`: the release builds of `main` before batch 43 (1449943)
and of the batch (cf9bec6), run in turn four times each on 29 October 2026 under Xvfb (1280 × 800, scale 1) with
`target/dialog-shots/perf.sh`'s timing log, from the key to the frame built; in Organise, pages 3 to 5 selected,
then moved to the end (Alt+Shift+End), Undo, Redo, Delete, Undo, Redo. The median (and range) of four:

| 10,000 pages | Before | After |
| --- | --- | --- |
| Move pages 3 to 5 to the end | 330 ms (322 to 430) | 354 ms (338 to 364) |
| Its Undo | 161 ms (156 to 167) | 105 ms (103 to 110) |
| Its Redo | 295 ms (289 to 334) | 249 ms (241 to 258) |
| Delete them | 386 ms (386 to 490) | 434 ms (414 to 449) |
| Its Undo | 296 ms (290 to 426) | 241 ms (240 to 248) |
| Its Redo | 304 ms (290 to 388) | 241 ms (239 to 259) |

Undo and Redo are 46 to 63 ms quicker; the move and the deletion 24 and 48 ms slower, more than the core probe's 3
to 5 ms for the clone and compare against the old copy. Not traced: the app also clears the Redo steps and makes the
step after the pages are drawn, the clone held meanwhile (about 44 MB more in use while the pages are drawn again).
On 300 pages (`p300.pdf`, the batch's build, two runs) each of them took 3 to 29 ms. PB-37 records both. REL-07's
typing and search rows, run again on the batch's build, are as before and within their thresholds: typing 24.6 to
26.3 ms median shown and none over 50 ms (built 1.4 to 2.1 ms median, 22.2 ms at worst); search as you type 249 to
261 ms in total at the median, 265 ms at worst (processing 40 to 49 ms).

## Editing in a line's own font

Batch 31 (ENH-31) writes an edited line in its own fonts. On the window's thread: the fonts of a page's lines,
read when one of them is first selected (`Pdf::page_line_fonts`, kept by the editor until the document changes),
and the edit itself; the preview font that draws a kept font while typing (`LineFont::preview`) is made on the
typing fonts' loader thread. Measured on 29 October 2026 on the machine above with a throwaway release-built probe
(`target/b31s3/`, built at opt-level 3 without the shipped build's link-time optimisation), core alone, best of
three with nothing else running. Made-up pages: 40 lines of Chinese in a whole embedded WenQuanYi Zen Hei (11.7 MB,
TrueType; with a ToUnicode map, and without one, read through the program) and in a whole Noto Sans CJK JP (16.5
MB, CID-keyed CFF); a 10,000-page document sharing one embedded Liberation Sans; 1,000 pages sharing the Zen Hei
font; and the corpus pages slowest in the edit run below.

| Page | Its objects read | All its lines' fonts (the first line selected) | One line's fonts | Preview font |
| --- | --- | --- | --- | --- |
| Zen Hei, 40 lines | 5.5 ms | 10 ms | 9.8 ms | 17 to 18 ms (423 KB) |
| Zen Hei, no ToUnicode map | 83 ms | 21 ms | 23 ms | 52 to 56 ms (1.3 MB) |
| Noto Sans CJK JP, 40 lines | 5.4 ms | 12 ms | 12 ms | 20 to 21 ms (297 KB) |
| 10,000 pages, page 1 / page 10,000 | 8.0 / 7.1 ms | 7.9 / 7.8 ms | 8.3 / 8.4 ms | – |
| 1,000 pages of Zen Hei, page 1 / page 1,000 | 7.4 / 7.4 ms | 9.9 / 9.7 ms | 12 / 12 ms | – |
| `pdfjs/issue2618.pdf` (heavy content, 3 unembedded fonts) | 365 ms | 307 ms | 305 ms | – |
| `pdfbox/5134.pdf` (heavy content) | 370 ms | 289 ms | 276 ms | – |
| `pdfbox/2957.pdf` | 116 ms | 260 ms | 149 ms | 175 ms (66 KB) |
| `pdfjs/issue2956.pdf` (two CID subsets) | 61 ms | 131 ms | 95 ms | 19 ms |
| `pdfjs/ichiji.pdf` (15 fonts) | 6.9 ms | 51 ms | 8.0 ms | 613 ms (104 KB) |

In the corpus edit run (testing.md; 489 files' first text page, two processes at once, so rough): one line's fonts
took 1.4 ms at the median, 22 ms at the 95th percentile and over 100 ms on 12 pages (2.5 %), 360 ms at most; a
preview font 0.4 ms at the median, over 100 ms for 4 fonts, 625 ms at most; an edit 3.3 ms at the median against
6e454fb's 2.4 ms (the deleted and reordered words, 956 edits), 23 ms more at the 95th percentile and 400 ms more
at most (`pdfbox/5134.pdf`'s heavy page: 1,086 ms against 688 ms).

What the figures show:

- The length of a document doesn't matter: page 1 and page 10,000 cost the same. A whole embedded CJK font costs
  8 to 23 ms to read, its preview 17 to 56 ms, off the window's thread.
- Before stage 3 each line selected read the page and the line's fonts again, a tenth to a third of a second a
  click on the slowest pages. Now the first line selected on a page reads all its lines' fonts, and later
  selections on that page read nothing until the document changes. That first selection reads every font of the
  page, not just the line's, so on a page with several two-byte (CID) fonts it costs more than one line did (131
  against 95 ms on `issue2956`, 51 against 8 ms on `ichiji`), and on the slowest pages it is still over REL-07's
  100 ms for a click's response, once a page (PB-52).
- Where the time goes: parsing the page, as reading its objects does (365 ms on `issue2618`'s heavy page), and for
  each CID font its decoding context (its ToUnicode map read twice, by LimarePDF and by lopdf: 15 to 20 ms for each
  of `issue2956`'s DejaVu subsets) and its codes (all 65,536 walked, about 15 ms). Stage 3 builds each font's
  decoding context once instead of twice (`OwnFont::read` takes the page's), which took about 20 ms off a line's
  fonts there.
- An edit keeping the line's font asks for the installed fonts for the words that might not keep it
  (`FontChoice::of_font`) before knowing whether any needs them: the app reads them on another thread from its
  start (`font_list::warm`), so only an edit in the first second or so could wait for it; the probe's first edit
  of each file paid it (1.1 to 1.2 s) and is left out of the edit figures above.

## Setting new text

Batch 45 (NEW-02) sets all new text through one pipeline (`layout.rs`: fonts a word at a time, shaping, line
breaks; architecture.md) and the app draws typing as it lays it out. The text design's budget (section 9):
shaping and breaking 2,000 characters within 2 ms, so typing stays within REL-07's rows; fonts and fallback
choices kept per face and word for the run; a search of installed fonts off the window's thread where it reads
files. Measured on 30 October 2026 on the machine above (its locale `en_CA`, so not a Chinese, Japanese or
Korean system), with the page cache warm unless said.

- **A paragraph:** `layout.rs`'s `speed_of_a_paragraph` (opt-in, release build: `cargo test --release -p
  limarepdf-core --lib -- --ignored speed_of`) breaks 2,000 characters of Latin, Polish, Greek and Cyrillic text
  with kerning pairs into 300-point lines at 12 points in Helvetica, its words' fonts known and its shaping not, as
  each keystroke would: median 1.18 ms over 30 runs, the first 4.5 ms (it fails over 2 ms at the median, or 50 ms
  the first time).
- **A text box's lines while typing** (`BoxSetting::lines`, which the typing box takes its rows from): 200
  characters, 0.15 ms a keystroke at the median, the first 3.2 ms.
- **Fallback fonts**, measured with a throwaway checker crate (`target/fontwarm/`, core as a path dependency, built
  with the release-like `ci-test` profile): the list of installed fonts read in 24 to 26 ms; `warm_fallbacks`
  23 ms (on this system only 😀 is warmed, in each standard font); a character no installed font has 3.5 to 3.9 ms
  the first time (every family's character map is asked), then nothing (the answer is kept); an Ol Chiki letter
  0.2 ms (a small font for its script). The first ideograph in Times on a non-CJK system took 105 to 107 ms after
  warming, as warming reads only the system language's faces and 漢 in a serif font reads Noto Serif CJK (26 MB;
  PB-61), 133 ms with nothing read before, and 1.16 s on the first run after a build (a cold cache).
- **The interface's fallback fonts** (ENH-33): `fallback_faces("漢あ한😀")`, which the app's start-up thread asks
  for the faces it adds to the interface's fonts, took 120 ms warm and 1.18 s on the first run after a build; 漢,
  あ and 한 82 to 87 ms each (each reads the 19 MB Noto Sans CJK collection) and 😀 15 ms (Symbola).

What they show:

- A paragraph is set within the budget, a text box's lines in a fraction of a millisecond: typing's cost is the
  frame, not the layout (REL-07's rows below).
- Typing never waits for the list of installed fonts for text its font has: the fonts kept for each word are
  checked against the count of families found so far (`typefaces::typeface_count`), which doesn't start reading
  the list, and `tests/font_scan.rs` (a test binary of its own, so nothing has read the list before it) fails if
  that changes. Text its font lacks needs the list and the fallback face's file: the app reads the list on its
  start-up thread (`font_list::warm`) and then `warm_fallbacks` sets 漢 (and あ or 한) on a Chinese, Japanese or
  Korean system, and 😀 everywhere, in Helvetica, Times and Courier, so the first such character typed doesn't
  wait for the faces. What still waits once is a character whose face that didn't read: about a tenth of a second
  for the first ideograph in a serif font on another system (PB-61), a few milliseconds for a character no font
  has.

## Finding paragraphs

Batch 45's stage 2 (NEW-01) finds a page's paragraphs once, with its lines' fonts (`Pdf::page_blocks`;
architecture.md). The paragraph design's budget (section 11): REL-07's "none over 50 ms" for a click on ordinary
pages, sharing PB-52's read of the page's fonts. Measured on 30 October 2026 on the machine above, built with the
release-like `ci-test` profile:

- **Over the paragraph check's 508 pages** (both answer sets, `tests/paragraph_check.rs`, tags and fonts read):
  median 4.5 ms a page; the worst 96, 83 and 63 ms (PDFBox `4996` page 1, pdf.js `issue6737` page 7 and `issue6165`
  page 4), then 41 ms and under.
- **What a page's time is**, best of five with a throwaway checker crate (`target/b45s2/`, `timing`): reading the
  page's objects, then everything, then everything but the fonts and tags, then the fonts alone.

  | Page | Objects | Blocks | Without fonts and tags | Fonts (`page_line_fonts`) |
  | --- | --- | --- | --- | --- |
  | made-up `lo01_ragged_serif11` p2 | 1.2 ms | 3.0 ms | 1.4 ms | 2.8 ms |
  | made-up `ch05_cols3_softhyph` p2 | 15.6 ms | 25.9 ms | 16.1 ms | 25.3 ms |
  | pdf.js `issue12294-print` p3 | 4.7 ms | 15.4 ms | 5.4 ms | 14.9 ms |
  | PDFBox `4996` p1 | 29.2 ms | 33.4 ms | 29.4 ms | 33.4 ms |
  | pdf.js `issue6165` p4 | 23.6 ms | 25.5 ms | 23.7 ms | 25.3 ms |
  | pdf.js `issue6737` p7 | 68.5 ms | 72.2 ms | 67.5 ms | 72.6 ms |

What they show:

- Finding the blocks costs under a millisecond beyond reading the page (the "without" column against the objects).
  The rest is the page's own read, which showing its objects already pays, and the fonts' read, which selecting a
  line on the page already pays (PB-52); so the app, keeping both for the page as it keeps the fonts today, adds
  little. The pages over 50 ms are over it in reading their objects alone (PB-52's heavy pages).

Batch 45's 2b-1 (30 October 2026) cut regions widest band first and some columns before their bands, and reads
the colour spaces a page names (PB-65, each space and its tint transform once a parse). Over the check's 541
pages: median 4.9 ms (4.4 ms on 26f236f in the same run order); a single run's worst is noise (174 ms against
81 ms for `issue6737` p7, which best of five gives as 75 to 77 ms against 74 ms; `pdfbox-4996` p1 36 against
35 ms). Every page's words over the corpus's 1,089 files read in 8.2 s against 8.1 s (three at a time).

Batch 45's 2b-2 (30 October 2026) dropped Q3's rule, so detection no longer reads the page's fonts: median 1.9 ms
a page over the check's 541 pages; the slowest `issue6165` p4 91 ms and `issue6737` p7 75 ms (one run each).

## Editing a paragraph

Batch 45's 2b-2 (NEW-01) reads a block for editing, sets it again at each keystroke and writes it once
(architecture.md, "Editing a paragraph"). The budgets (the paragraph design's 11): setting within NEW-02's 2 ms
for 2,000 characters a keystroke, so typing stays within REL-07's rows; the write once, about a line edit's cost
a line. Measured on 30 October 2026 on the machine above, with nothing else running, by the throwaway probe
(`target/b31s3/para/`, `b45para time BIG`, built with the `ci-test` profile) on a made-up paragraph of 2,000
characters in 21 lines of Helvetica at 10 points, 440 points wide:

| What | Median | 95th percentile | Most |
| --- | --- | --- | --- |
| Reading it (`Pdf::paragraph` and `paragraph_setting`, 20 runs) | 2.39 ms | 2.64 ms | 25.6 ms (the first) |
| A keystroke at the start (`ParagraphSetting::set`, 200) | 0.20 ms | 0.24 ms | 0.32 ms |
| A keystroke in the middle (200) | 0.19 ms | 0.20 ms | 0.23 ms |
| A keystroke at the end (200) | 0.18 ms | 0.19 ms | 0.20 ms |
| Greek typed in the middle, in another font (200) | 0.21 ms | 0.22 ms | 8.6 ms (the first) |
| A character deleted in the middle (200) | 0.17 ms | 0.18 ms | 0.21 ms |
| The write, a word typed at the start, every line (`Pdf::edit_paragraph`, 10) | 1.94 ms | 2.00 ms | 2.00 ms |
| The write, a letter changed in the middle (10) | 1.28 ms | 1.29 ms | 1.29 ms |
| The document's words on the made-up 10,000-page `big.pdf` (`Pdf::document_words`, 3) | 7.49 s | 7.63 s | 7.63 s |
| The same on two threads, as the app reads them (`document_words_on(2)`, 3; batch 45's 3a, the same day, every core 6.43 s then) | 18.59 s | 18.91 s | 18.91 s |
| Every page's size on `big.pdf` (`Pdf::page_sizes`, BUG-43's page box, 20) | 16.1 ms | 16.6 ms | 16.6 ms |

Over the corpus edit run's real paragraphs (testing.md, "Editing paragraphs"; two files at a time, so indicative):
reading a block median 5.8 ms, setting it 0.05 ms, writing it 4.8 ms for confident blocks; the slowest reads and
writes, up to about 1.7 s, are pages whose fonts take that long to read (PB-52) and the first word in a fallback
face.

What they show:

- A keystroke sets a 2,000-character paragraph in about a tenth of NEW-02's budget, wherever it is typed, a word
  in another font included once its face is read; the write is a couple of milliseconds.
- The document's words (line-end hyphens' evidence) take seconds on a very long document, as they read every
  page, so the app reads them off the window's thread when a tab gets its document (batch 45's 3a), on two
  threads of their own so the pages draw meanwhile: under 19 s on 10,000 pages; until they are there a
  paragraph's hyphens read as unknown (kept, marked).
- **Typing in a paragraph in the app** (batch 45's 3a): the opt-in `speed_of_typing_in_a_paragraph`
  (`app/tests/paragraphs.rs`, the `ci-test` profile, the window drawn without a GPU) times a frame of the whole
  window for each of 40 keystrokes in the first line of a paragraph of 2,021 characters, which each set it again
  on its lines and lay its box out: median 6.80 ms, slowest 18.72 ms, the app's own share against REL-07's "none
  over 50 ms" (6.3); reading a paragraph to open it is the row above, a few milliseconds.
- `page_sizes` reads a page's media box and crop box as before (a page without a crop box read both before too):
  16 ms over 10,000 pages.
- **A frame edge dragged** (batch 45's 3b-2, measured 31 October 2026 the same way, `b45para time`, nothing else
  running): each frame of a drag reads how narrow the frame may go (`ParagraphSetting::narrowest`, 200 frames:
  median 0.16 ms, 95th percentile 0.17 ms) and sets the 2,000 characters between the moved edges
  (`set_between`, every line broken anew: median 0.18 ms, 95th percentile 0.19 ms, most 0.25 ms), as a keystroke
  costs; the write narrowed 100 points, every line (`Pdf::edit_paragraph_between`, 10): median 2.13 ms, as a word
  typed at the start. Keystrokes measured again the same day: 0.17 to 0.21 ms at the median, as above. The corpus
  edit run again after 3b-2 (edges as they were) gave every block's record as 3a's run did, timings aside.

## The page under text being edited

Batch 45's 3b-1 (ENH-21) makes the page alone without the text being edited once when a line or paragraph starts
being edited, on the window's thread, and draws the hidden part of it on a thread of its own (architecture.md,
"The page under text being edited"). Measured on 31 October 2026 on the machine above by the throwaway probe
(`target/b31s3/para/`, `b45para without FILE PAGE…`, the `ci-test` profile), each page's largest block, 20 runs
(the heavy page 3); "drawn" is a one-page load and `render_region_in` of the block's box at 2.67 pixels a point,
as the app draws it at 100% on a 2× screen:

| Page | `Pdf::page_without` (median / most) | Its size | Drawn (median / most) |
| --- | --- | --- | --- |
| `big.pdf` (10,000 pages), pages 1, 5,001 and 10,000 | 3.35 to 3.55 / 4.46 ms | 0.9 KB | 0.14 to 0.16 / 0.43 ms |
| pdf.js `bug810214`, two pages with pictures | 0.70 and 1.37 / 1.47 ms | 50 and 58 KB | 4.7 and 31.3 / 32.5 ms |
| pdf.js `issue11306`, a 2 MB picture on page 1 | 0.75 / 2.18 ms | 2.1 MB | 15.8 / 19.8 ms |
| PDFBox `4060` page 1 (a 5 MB page of 2,750 objects) | 68.3 / 70.7 ms | 5.4 MB | 6.09 s (one run) |

What they show:

- Making the page alone costs per page, not per document: about 3.5 ms on any page of 10,000, mostly finding the
  page in the page tree (a whole copy of that document, as a thread would need, takes 23.5 ms). On the heaviest
  page it is 68 ms, less than reading that page's objects and deleting one of them (38.5 and 92.7 ms), which
  editing there already costs.
- Drawing is off the window's thread, a few milliseconds as a rule; on the heaviest page, seconds, during which
  the colour around the text stands in. It isn't done again as the text is typed, only after a zoom or a change
  of page colours.

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

Run again on 30 October 2026 for batch 45 (typing drawn as core sets it: each word's face, clusters, input methods,
wrapping boxes), on the release build of 473edaa, the same way (Xvfb, one run at a time, nothing building beside
it; `PERF_BIN=… python3 -I drive.py typing KIND` in `target/perf/`). "Shown" is given for the keys typed inside
the captured area (the first 51):

| Where | Built | Shown, keys 0 to 50 | Over 50 ms shown | Over 100 ms shown |
| --- | --- | --- | --- | --- |
| A form field (form.pdf) | 1.2 / 5.0 ms | 26.0 / 45.6 ms | 0 | 0 |
| Add text, a new box (letter.pdf), two runs | 3.6 / 22.4 and 3.2 / 23.3 ms | 26.7 / 46.9 and 26.4 / 46.0 ms | 0 | 0 |
| Editing a line (letter.pdf) | 3.2 / 23.6 ms | 27.5 / 49.1 ms | 0 | 0 |

All within the accepted thresholds; built is a millisecond or so above 26 October's (1.1 / 4.4, 2.4 / 23.1, 2.4 /
23.0 ms). Over the whole run, Add text's shown worst was 834 and 780 ms: that is the poller's, not the app's. The
new text passes the captured area's right edge (920 pixels) at about key 51 and the page's edge at key 57, where
its outline turns to the caution colour; that change is the first the poller sees after keys 51 to 56, so it
credits them with it (834, 702, 600, 470, 322 and 193 ms in the first run, falling as later keys were sent
closer to it), each of which the app built in 0.7 to 3.2 ms. The line's whole run had 4 keys shown in 52 to 71 ms, past key 105, none
over 100 ms.

Run again on 31 October 2026 for batch 45's 3b-1 (ENH-21: the line being edited hidden by the page drawn without
it, a picture drawn under the typing each frame in place of one colour), the release builds of 459b53e and of
3b-1 one after the other, the same way:

| Editing a line (letter.pdf) | Built | Shown, keys 0 to 50 | Over 50 ms shown | Over 100 ms shown |
| --- | --- | --- | --- | --- |
| 459b53e (before) | 3.5 / 23.0 ms | 28.2 / 47.3 ms | 0 | 0 |
| 3b-1 | 2.8 / 24.1 ms | 26.8 / 46.6 ms | 0 | 0 |

The same within the runs' spread, and within the accepted thresholds; the whole runs' shown worst, past the
captured area, 65.3 and 60.0 ms. The paper behind letter.pdf's line is white, but what the picture shows doesn't
change a frame's cost: it is one picture drawn in place of one rectangle, made off the window's thread.

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

# Performance

Measured on the project's build machine (2 cores, Linux), release build (LTO, one codegen unit), with a 300-page
text-and-graphics test file of 538 KB:

| Task | Time |
| --- | --- |
| Render 300 pages at 100 dpi to PNG files (both cores) | 3.5 s |
| Full-text search of 300 pages | 0.05 s (see below) |
| Watermark 300 pages | 0.03 s |
| Merge 3 × 300 pages into 900 pages | 0.03 s |
| Split 900 pages into 18 files | 0.14 s |
| AES-256 encrypt 300 pages | 0.02 s |

These are single runs on small, simple pages, not a benchmark suite, and no comparison with other software was made.

Search was measured when it used the document library's text extraction. It now reads pages with LimarePDF's own text
reader, which gets far more files right and takes about twice as long on the same files: searching the 477-page
Ghostscript manual takes 0.17 s instead of 0.09 s on a 6-core machine, including starting the program.
Pages with large images, heavy transparency or many fonts are slower, and hayro's own documentation says its renderer
has not yet been tuned for performance.

OCR speed is in [Scanning and OCR](scanning-and-ocr.md#accuracy-and-speed).

## How the desktop app stays responsive

- Pages are drawn lazily on a pool of worker threads, newest request first, with one page of prefetch either side.
- Pages outside the view are dropped from memory, and thumbnails are drawn at their own small size.
- At high zoom only the part of the page in view is drawn, in 512-pixel tiles
  ([details](rendering.md#high-zoom)). A tile takes about 1 ms on text pages and about 20 ms on a dense form, at
  any zoom, while a whole Letter page at 1152 dpi takes about 0.4 s and 500 MB.
- Edits re-save the document without compressing it, and undo keeps plain snapshots.
- OCR, optimising, exporting images and printing run in the background.

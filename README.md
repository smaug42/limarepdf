# MPE

A fast PDF viewer and editor written entirely in Rust, with no C or C++ dependencies. It started from a
functional specification of an existing Qt/C++ editor. This is a from-scratch implementation: it contains
none of that product's code, strings, stamps, fonts or other assets.

**Status: 0.2, for testing.** Viewing, page operations, overlays (headers, footers, Bates, watermark), metadata,
password security, basic content editing (text, images, vector shapes), image import, scan optimisation and OCR
(Latin script) are in. Annotation, forms, signatures and most export formats are not started.
The coverage table below is the complete list.

| Part | What it is |
| --- | --- |
| `mpe-core` | Library: page operations, overlays, metadata, encryption, text search, rendering |
| `mpe` | Command-line tool for scripting and batch work |
| `mpe-gui` | Desktop app (egui): tabs, thumbnails, continuous scrolling, undo/redo, dialogs |

Engines: [`lopdf`](https://crates.io/crates/lopdf) for the document structure and encryption,
[`hayro`](https://crates.io/crates/hayro) for rasterisation (CPU, pure Rust), [`egui`](https://crates.io/crates/egui)
for the UI, [`rfd`](https://crates.io/crates/rfd) for file dialogs, [`image`](https://crates.io/crates/image) to decode
PNG/JPEG for insertion, and [Phosphor Icons](https://phosphoricons.com) (MIT, via
[`egui-phosphor`](https://crates.io/crates/egui-phosphor)) for the toolbar.

## Build

Requires Rust 1.95 or newer, the minimum set by egui 0.36 (developed on 1.97 and 1.99).

```
cargo build --release
# binaries: target/release/mpe  and  target/release/mpe-gui
cargo test --workspace
```

Linux needs a working OpenGL driver and an X11 or Wayland session at run time. Nothing else is required at
build or run time. Windows and macOS builds use the same code; see "Platform status".

## Use

```
mpe-gui file.pdf --page 3            # open at page 3; or File > Open, or drop files on the window
mpe-gui scan.tif photo.jpg           # image files open as a new document, one page per image

mpe info file.pdf
mpe merge a.pdf b.pdf -o out.pdf
mpe split big.pdf --every 50 --out-dir parts
mpe extract in.pdf -o out.pdf --pages 1-3,7
mpe delete in.pdf -o out.pdf --pages 2
mpe rotate in.pdf -o out.pdf --pages all --degrees 90
mpe reorder in.pdf -o out.pdf --order 3,1,2
mpe crop in.pdf -o out.pdf --page 1 --rect 50,600,400,780
mpe insert-blank in.pdf -o out.pdf --after 0
mpe images scan.tif photo.jpg -o out.pdf            # one page per image; --dpi overrides the file's resolution
mpe insert-images in.pdf -o out.pdf --after 2 --images a.png b.jpg
mpe optimize-scans in.pdf -o out.pdf --mode auto --max-dpi 200 --quality 75   # also --no-deskew, --no-despeckle
mpe ocr in.pdf -o out.pdf --pages all    # add a searchable text layer to pages without text
mpe watermark in.pdf -o out.pdf --text CONFIDENTIAL --opacity 0.3
mpe header-footer in.pdf -o out.pdf --footer-center "Page {page} of {pages}" --header-right "{date}"
mpe bates in.pdf -o out.pdf --prefix ACME- --digits 6
mpe set-info in.pdf -o out.pdf --title "Report" --author "Me"
mpe encrypt in.pdf -o out.pdf --owner-password OWN --user-password USR --cipher aes256 --no-copy
mpe decrypt in.pdf -o out.pdf --password USR
mpe search in.pdf "invoice"
mpe text in.pdf --pages 1-2
mpe objects in.pdf --pages 1        # list editable objects with their boxes
mpe render in.pdf --dpi 150 --out-dir pngs
```

Editing in the GUI: pick a tool in the toolbar (hand = read only, arrow = select). With the arrow, click an object to
select it, drag to move, drag the handles to resize (Shift keeps proportions), arrow keys nudge (Shift = 10 pt),
Delete removes it. The right-hand panel shows position and size, and for text the string, font, size and colour; for
shapes fill, outline and width; for images "Replace image". The other tools add text (click, type, Enter), rectangles,
ellipses, lines (drag, or click for a default size) and images (choose a PNG, JPEG or TIFF, then drag a box). Everything is
undoable with Ctrl+Z.

GUI keys: Ctrl+O/S/W, Ctrl+Shift+S, Ctrl+Z / Ctrl+Y, Ctrl+F, Ctrl+= / Ctrl+- / Ctrl+0 (fit width) / Ctrl+1,
Ctrl+scroll or pinch to zoom, PageUp/PageDown/Home/End. Click thumbnails (Ctrl/Shift for multi-select) to choose
the pages that Document-menu operations apply to.

## Coverage against the specification

Labels: **Done** = implemented and exercised by tests or by running it. **Partial** = works with the stated limits.
**No** = not implemented.

| Specification area | Status | Notes |
| --- | --- | --- |
| Tabbed multi-document window, thumbnails, drag-and-drop open | Done | |
| Zoom, fit width, continuous scrolling, page navigation | Done | Single-page / facing-page layouts: No |
| Command line `file.pdf --page N` | Done | |
| Rendering | Partial | hayro 0.8, checked by pixel on small synthetic pages in `crates/mpe-core/tests/render_coverage.rs`. Correct: all 16 blend modes; isolated transparency groups; a blend mode applied to a whole group; colour-key image masks (`/Mask [..]`); annotations that have an `/AP` appearance stream (Text, Highlight, Link, Stamp, FreeText, Widget tested), including Widget `/AS` state selection and the Hidden flag; annotations with the NoView flag are hidden in the GUI (MPE marks them Hidden before rendering, since hayro ignores NoView); password-protected files with all four ciphers. Wrong: knockout groups render as non-knockout; non-isolated groups render as isolated; `mpe render` (CLI) draws NoView annotations; annotations without `/AP` are not drawn at all (tested: Text, Highlight, Link, Stamp, FreeText, Widget). Other PDF features are untested by MPE |
| Text search | Partial | Hits with snippets; matches are highlighted on the page and the view scrolls to them (CLI: `mpe search --boxes` prints their positions). Highlights are placed by character position within each piece of text, so they can be a letter off in proportional fonts; a phrase broken across lines is listed but not highlighted. Extraction is lopdf's and can miss text in some CID/embedded-font encodings. Scanned pages have no text without OCR. Highlight placement checked on a real scan; the GUI highlight drawing was not clicked through in testing |
| Page operations: insert blank, delete, rotate, extract, reorder, crop box | Done | Reorder and crop are in the core library and the CLI, not yet in the GUI (no drag-to-reorder) |
| Merge / append, split by page count | Done | Bookmarks, form fields and named destinations of appended files are not carried over (not tested with such files). Split by file size: No |
| Undo / redo | Done | 20 levels, by snapshots |
| Headers, footers, page numbers, date, Bates numbering | Done | Standard Helvetica, Latin-1 characters only. No saved presets, no multi-file manager |
| Watermark | Partial | Text only, any angle, opacity, above or below content. No image / PDF / page-number sources |
| Document properties (title, author, subject, keywords, creator, producer) | Done | |
| Password security: open and permissions passwords, RC4-40, RC4-128, AES-128, AES-256, permission flags | Done | Round-trip tested for all four. Saving a file you opened with a password removes protection unless you set it again in Document > Password security |
| Opening password-protected files | Done | User password |
| Certificate encryption | No | |
| Digital signatures, signature validation, certificate manager, PKCS#11 tokens | No | |
| Content editing: edit text, images, vector objects | Partial | See "Content editing" below |
| Object inspector (position, size, text, fill, outline) | Partial | Properties listed above only; no layers, no colour replacement across the page |
| Links, layers, attachments, bookmarks | No | |
| Annotations, comments, stamps, measurement tools | No | Existing annotations are drawn only if they have an `/AP` appearance stream; see Rendering |
| Form editing, form data import/export, barcodes, document JavaScript | No | |
| Redaction | No | |
| OCR (Tesseract), scanning (SANE), optimise scanned pages | Partial | Importing images as pages works: PNG, JPEG and TIFF (multi-page, 1-bit, CCITT G4), sized from the file's resolution (else 150 dpi), turned upright from EXIF or TIFF orientation, JPEG embedded without recompression. CLI `images` and `insert-images`; GUI File > New from images, Document > Insert images, drag and drop. Tested in `crates/mpe-core/tests/import.rs` and by hand with ImageMagick-made files (including CMYK JPEG and 300 dpi G4 TIFF); the two GUI menu items were not clicked in testing. Scanning: No, and not planned: Tesseract and SANE are C/C++, so scan with the system's own tool and import the images. OCR works on pages without text: adds an invisible, searchable and selectable text layer (Helvetica, WinAnsi characters; others become `?`). Latin script only. Words are found by MPE's own image processing and read by the bundled ocrs recognition model (see Licence). Black and white is decided by a local (Sauvola) threshold, so shadows and uneven lighting in phone scans are handled. On test pages made with ImageMagick: 0.4–1.7% character errors on clean 300 dpi text in serif, sans and mono; 4–14% on deliberately blurred, noisy, tilted scans, mostly punctuation; 0–1.2% under strong shadows. One real phone scan (a photocopied card with textured bands) has been tried: all its body text and headings were read, plus a few short junk lines from the texture, and a two-column table came out column by column. Speed: two Letter pages in 2.2 s on a 6-core machine (about 5 s of CPU time per page; pages run in parallel). Text in pictures, handwriting and vertical text are not handled. CLI `ocr`; GUI Document > Recognise text (runs in the background, undoable). The text layer is invisible. In MPE, View > Show recognised text outlines each recognised line and shows its text on hover; with the arrow tool a line can be selected and its text corrected in the inspector (it stays invisible and in place; clearing it removes the line). `mpe objects` lists these lines as "(hidden)". Correction was tested in code, not by clicking through the GUI. Run Optimise first: it does not straighten pages that already have text. Optimising scanned pages works on pages that are one image covering the page: black and white is stored as CCITT G4, greyscale and colour as JPEG (downsampled to 200 dpi by default); automatic choice per page, straightening of tilts up to 5°, speck removal. Reads raw/Flate, greyscale and RGB JPEG, and CCITT G4 images; skips CMYK JPEG, JBIG2, JPEG 2000, masked images and pages with other content, and says why. An image is only replaced when it gets smaller or was straightened or cleaned. Pages with text (such as an OCR layer) are not straightened. CLI `optimize-scans`; GUI Document > Optimise scanned pages (runs in the background, undoable). Tested in `crates/mpe-core/tests/scan.rs` and by hand on ImageMagick-made 300 dpi scans (5.3 MB to 0.5 MB, 1.2° tilt corrected); the black-and-white/greyscale choice is a heuristic not yet tried on real scanner output, and the GUI dialog was not clicked in testing |
| PDF/A conversion, Save Optimized | No | Output is pruned and stream-compressed; that is all |
| Export to Word, Excel, CSV, images | Partial | Page text and PNG render via the CLI only. Word, Excel, CSV: No |
| Compare documents | No | |
| Printing dialog | No | |
| Sessions, autosave, backup on save, custom keyboard shortcuts, preferences dialog, 42 translations, toolbars customisation | No | Unsaved-changes prompts on close are in |
| XFA forms, XPS documents | No | |
| Licence activation, update checks, OCR language downloads | Not applicable | MPE makes no network connections and has no licence mechanism |

## Content editing

How it works: the page's content stream is parsed into objects (one object per text line, per painted path, per
image or form XObject, per inline image). Moving or resizing wraps the object's operators in `q <matrix> cm … Q`.
Text edits rewrite the string in place when the font is a plain, non-subset, standard-encoded simple font and the new
text is ASCII; otherwise the old line is removed and the new text is added in one of the 12 standard fonts at the same
position, size and colour. Insertions are appended on top of the page. Every edit is checked to re-parse before it is
applied, and a failed edit leaves the document unchanged.

Limits (stated, not hidden):

- Text objects are lines recovered by heuristics from the content stream, not paragraphs. There is no re-flow and no
  multi-line editing. Text boxes are estimated from font metrics, so they can be slightly off for unusual fonts.
- Text in embedded subset fonts, and in CID (Type 0) fonts, cannot keep its font when edited: it becomes Helvetica,
  Times or Courier (the inspector says so). Text edited into a new font is limited to WinAnsi characters; others
  print as `?`.
- Editing text does not change its width constraints: longer text can overlap neighbours.
- A clip region set before an object stays where it was, so a moved or enlarged object can be cut off by its old
  clip. A path that is itself used as a clip (`W n`) is not selectable.
- Shading and pattern fills are selectable but their colour cannot be changed in the inspector (it may apply a flat
  colour instead). Colour readout for non-RGB/Gray/CMYK colour spaces is approximate.
- Inline images can be moved, resized and deleted, not replaced. Pages whose inline images cannot be located safely
  are refused rather than risk losing them.
- Replacing an image fits the new picture inside the old box, centred. JPEG EXIF rotation is not applied; CMYK
  JPEGs are not supported.
- Rotation of objects, grouping, z-order changes, and copy/paste between pages are not implemented.
- Moving an object does not update links or annotations over it (the annotations stay where they were).

## Performance

Measured on this project's build machine (2 cores, Linux), release profile (LTO, one codegen unit), 300-page
text-and-graphics test file, 538 KB:

| Task | Time |
| --- | --- |
| Render 300 pages at 100 dpi to PNG files (both cores) | 3.5 s |
| Full-text search of 300 pages | 0.05 s |
| Watermark 300 pages | 0.03 s |
| Merge 3 × 300 pages into 900 pages | 0.03 s |
| Split 900 pages into 18 files | 0.14 s |
| AES-256 encrypt 300 pages | 0.02 s |

These are single runs on small, simple pages, not a benchmark suite. No comparison against other software was made.
Pages with large images, heavy transparency or many fonts will be slower, and hayro's own documentation says
its renderer has not yet been performance-tuned.

How the GUI stays responsive: pages are rendered lazily on a pool of worker threads, newest request first,
with one page of prefetch either side; textures outside the viewport are dropped; thumbnails render at their own
small size; edits re-serialise without compressing and undo uses plain snapshots.

## Platform status

| Platform | Status |
| --- | --- |
| Linux x86_64 | Built, tested (unit and integration tests), CLI run on real PDFs, GUI run under a virtual X server with software GL and checked by screenshot |
| Windows, macOS (Intel and Apple Silicon) | Not built or run by the author: the build machine could not install those targets. The code has no platform-specific parts and all dependencies are Rust, so it is expected to build. `.github/workflows/ci.yml` builds, tests and packages all four on GitHub Actions |

GUI checked by scripted input: mouse-wheel scrolling, thumbnail selection, View > Zoom in, Document menu, Rotate
(including the unsaved marker and thumbnail refresh), selecting, moving and resizing a shape, editing a text line
(typing into the inspector with keyboard focus set by hand), the text, rectangle and ellipse tools, and undo of those.
Not exercised in the GUI: the image tool and "Replace image" (they open a native file dialog); the core functions
behind them are covered by tests. Not exercised in the GUI: keyboard shortcuts (the virtual display
had no window manager to give it focus), file dialogs, the unsaved-changes prompts, and the Watermark, Header/Footer,
Security, Metadata, Split and Insert-blank dialogs. The operations behind those dialogs are covered by the core tests
and the CLI run.

## Known limits

- Header/footer, Bates and watermark text use Helvetica and Latin-1 only; other characters print as `?`.
- Overlays are added as extra content on the page. They are not annotations and cannot be removed individually
  (use Undo before saving).
- Saving rewrites the whole file; incremental save is not implemented.
- Files hayro cannot parse show "Rendering…" on those pages; editing may still work if lopdf can read them.
- Digitally signed documents lose their signature validity when saved, as with any rewrite.

## Licence

Dual-licensed under MIT or Apache-2.0, at your option. See `LICENSE-MIT` and `LICENSE-APACHE`.
The toolbar icons are Phosphor Icons (MIT, © Phosphor Icons), embedded as a font by `egui-phosphor`.
The OCR text recognition model built into MPE (`crates/mpe-core/models/text-recognition.rten`) is by Robert Knight,
from the [ocrs](https://github.com/robertknight/ocrs) project, licensed
[CC BY-SA 4.0](https://creativecommons.org/licenses/by-sa/4.0/) and included unmodified; see
`crates/mpe-core/models/README.md` (shipped as `OCR-MODEL.md` in release archives). With the inference code, OCR adds about 14 MB to each binary (the Linux CLI grows from 9.4 MB to 23 MB).
Dependencies carry their own licences. A scan of the dependency metadata for all platforms found only permissive
licences (MIT, Apache-2.0, BSD, Zlib, Unicode, ISC, Unlicense, BSL-1.0 and similar), plus the SIL OFL 1.1 and Ubuntu
Font Licence for the UI fonts bundled with egui. A few crates are dual-licensed with LGPL or GPL as one option;
the permissive option applies. If you redistribute binaries, keep the licence texts of the dependencies with them
(`cargo about` or `cargo license` can generate the list).

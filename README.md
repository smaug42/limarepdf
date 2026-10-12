# MPE

A fast PDF viewer and editor written entirely in Rust, with no C or C++ dependencies. It started from a
functional specification of an existing Qt/C++ editor. This is a from-scratch implementation: it contains
none of that product's code, strings, stamps, fonts or other assets.

**Status: 0.2, for testing.** Viewing, page operations, overlays (headers, footers, Bates, watermark), metadata,
password security and basic content editing (text, images, vector shapes) are in. Annotation, forms, signatures, OCR
and most export formats are not started.
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

Requires a recent stable Rust toolchain (developed on 1.97).

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

mpe info file.pdf
mpe merge a.pdf b.pdf -o out.pdf
mpe split big.pdf --every 50 --out-dir parts
mpe extract in.pdf -o out.pdf --pages 1-3,7
mpe delete in.pdf -o out.pdf --pages 2
mpe rotate in.pdf -o out.pdf --pages all --degrees 90
mpe reorder in.pdf -o out.pdf --order 3,1,2
mpe crop in.pdf -o out.pdf --page 1 --rect 50,600,400,780
mpe insert-blank in.pdf -o out.pdf --after 0
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
ellipses, lines (drag, or click for a default size) and images (choose a PNG/JPEG, then drag a box). Everything is
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
| Rendering | Partial | hayro 0.8, checked by pixel on small synthetic pages in `crates/mpe-core/tests/render_coverage.rs`. Correct: all 16 blend modes; isolated transparency groups; a blend mode applied to a whole group; colour-key image masks (`/Mask [..]`); annotations that have an `/AP` appearance stream (Text, Highlight, Link, Stamp, FreeText, Widget tested), including Widget `/AS` state selection and the Hidden flag; password-protected files with all four ciphers. Wrong: non-isolated groups render as isolated; knockout groups render as non-knockout; annotations with the NoView flag are drawn; annotations without `/AP` are not drawn at all (tested: Text, Highlight, Link, Stamp, FreeText, Widget). Other PDF features are untested by MPE |
| Text search | Partial | Page-level hits with snippets. No highlight on the page. Extraction is lopdf's and can miss text in some CID/embedded-font encodings. Scanned pages have no text without OCR |
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
| OCR (Tesseract), scanning (SANE), optimise scanned pages | No | |
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
Dependencies carry their own licences. A scan of the dependency metadata for all platforms found only permissive
licences (MIT, Apache-2.0, BSD, Zlib, Unicode, ISC, Unlicense, BSL-1.0 and similar), plus the SIL OFL 1.1 and Ubuntu
Font Licence for the UI fonts bundled with egui. A few crates are dual-licensed with LGPL or GPL as one option;
the permissive option applies. If you redistribute binaries, keep the licence texts of the dependencies with them
(`cargo about` or `cargo license` can generate the list).

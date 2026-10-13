# MPE

MPE is a PDF viewer and editor for Linux, Windows and macOS, written entirely in Rust with no C or C++ dependencies.
It organises, edits, stamps, secures, scans and prints PDFs, from a desktop app or the command line.
This is a from-scratch implementation (with AI assistance): it contains no other product's code, strings, stamps, fonts or other assets.

**Status: 0.2, for testing.** Most testing has been on Linux. Annotations, forms, digital signatures and redaction
are not started; see [Feature status](#feature-status).

![The MPE window: the top bar, page thumbnails on the left, a report open in the middle, and the inspector on the right](docs/images/mpe-gui.png)

## Features

- **View:** tabs, thumbnails, continuous scrolling, zoom, and search with highlights on the page.
- **Organise pages:** drag thumbnails to reorder; rotate, insert, delete and extract pages; merge and split files.
- **Edit content:** move, resize, restyle and delete text, images and shapes; edit text; add text, shapes and images.
- **Stamp pages:** headers, footers, page numbers, Bates numbers and text watermarks.
- **Scans:** turn images into PDFs, shrink and straighten scanned pages, and add a searchable text layer with
  built-in OCR (Latin script).
- **Copy text,** including recognised text.
- **Protect:** open and permissions passwords, RC4 or AES encryption, permission flags.
- **Export and print:** pages as PNG or JPEG, text as a `.txt` file, and printing (through CUPS on Linux and macOS,
  the print spooler on Windows).
- **Command line:** everything above, for scripts and batches.
- **Private:** no network connections, accounts or licence checks.

## Install

There are no packaged releases yet. Each run of the project's continuous integration on GitHub Actions builds
archives for Linux, Windows and macOS (Intel and Apple Silicon); download them from the run's artifacts (this needs
a GitHub sign-in).

To build from source you need Rust 1.95 or newer:

```
cargo build --release
# the binaries: target/release/mpe-gui and target/release/mpe
cargo test --workspace
```

On Linux the desktop app needs a working OpenGL driver and an X11 or Wayland session. Nothing else is needed.

## Quick start

Open a file in the desktop app with the Open button, Ctrl+O, or by dropping it on the window, or from a terminal:

```
mpe-gui report.pdf --page 3       # open at page 3
mpe-gui scan.tif photo.jpg        # images open as a new document, one page per image
```

Click what you want to work on, and the panel on the right shows its options: the document when nothing is selected,
pages when you select thumbnails, an object when you select one with the arrow tool. Right-click anything for a short
menu, or press Ctrl+K to search all commands. See [Using the desktop app](docs/gui.md).

A few commands from the command line:

```
mpe info report.pdf
mpe merge a.pdf b.pdf -o both.pdf
mpe extract report.pdf -o summary.pdf --pages 1-3,7
mpe ocr scan.pdf -o searchable.pdf
mpe watermark report.pdf -o draft.pdf --text DRAFT --opacity 0.3
```

See the [command-line reference](docs/cli.md) for all 25 commands.

## Feature status

**Done:** works, and is covered by tests or has been run. **Partial:** works within the limits noted. **No:** not
available.

| Area | Status | Notes |
| --- | --- | --- |
| Tabs, thumbnails, opening by drag and drop | Done | |
| Zoom, fit width, continuous scrolling, page navigation | Done | Single-page and facing-page layouts: No |
| Open at a page from the command line | Done | `mpe-gui file.pdf --page N` |
| Command palette, right-click menus, floating toolbar | Done | Toolbar customisation: No |
| Rendering | Partial | Some transparency groups are drawn wrongly, and annotations without an appearance stream are not drawn. [Details](docs/rendering.md) |
| Search | Partial | Highlights can be a letter off in proportional fonts. [Details](docs/search-and-text.md) |
| Select and copy text | Done | One page at a time; includes recognised text |
| Page operations: insert, delete, rotate, extract, reorder, crop | Done | Crop is in the command line only |
| Merge and split | Done | Bookmarks, forms and named destinations of appended files are not carried over. Split by file size: No |
| Undo and redo | Done | 20 levels. Password settings cannot be undone |
| Headers, footers, page numbers, dates, Bates numbers | Done | Helvetica and Latin-1 characters only |
| Watermarks | Partial | Text only |
| Document properties | Done | Title, author, subject, keywords, creator, producer |
| Password security | Done | All four ciphers and permission flags. [Things to know](docs/security.md) |
| Opening protected files | Done | With the open (user) password |
| Content editing | Partial | Works on lines, not paragraphs; some fonts are replaced when edited. [Details](docs/content-editing.md) |
| Importing images as pages | Done | PNG, JPEG, TIFF. [Details](docs/scanning-and-ocr.md#importing-images) |
| Optimising scanned pages | Partial | Pages that are one image. [Details](docs/scanning-and-ocr.md#optimising-scanned-pages) |
| OCR | Partial | Latin script; built-in recognition model. [Details](docs/scanning-and-ocr.md#recognising-text-ocr) |
| Export pages as images, export text | Done | [Details](docs/export-and-printing.md) |
| Printing | Partial | No paper size or scaling options; not yet tried with a real printer. [Details](docs/export-and-printing.md#printing) |
| Driving scanners | No | Not planned: scan with your system's tool and import the images |
| Export to Word, Excel, CSV | No | |
| Annotations, comments, stamps, measuring | No | Existing annotations are shown if they have an appearance stream |
| Forms, form data, barcodes, JavaScript | No | |
| Links, layers, attachments, bookmarks | No | |
| Redaction | No | |
| Digital signatures, certificate encryption | No | |
| PDF/A, optimised saving | No | Saved files are pruned and compressed |
| Comparing documents | No | |
| Sessions, autosave, backups, preferences, custom shortcuts, translations | No | You are asked before closing unsaved work |
| XFA forms, XPS documents | No | |

## Known limits

- Header, footer, Bates and watermark text uses Helvetica and Latin-1 only; other characters print as `?`.
- Headers, footers and watermarks are added to the page content. They are not annotations and cannot be removed one
  by one later; use Undo before saving.
- Saving rewrites the whole file; incremental saving is not available.
- Saving removes the validity of digital signatures, as any rewrite does.
- Files the renderer cannot read show "Rendering…" on their pages; editing may still work.

## Documentation

- [Using the desktop app](docs/gui.md)
- [Command-line reference](docs/cli.md)
- [Content editing](docs/content-editing.md)
- [Scanning and OCR](docs/scanning-and-ocr.md)
- [Search and text](docs/search-and-text.md)
- [Export and printing](docs/export-and-printing.md)
- [Password security](docs/security.md)
- [Rendering](docs/rendering.md)
- [Performance](docs/performance.md)
- [Testing and platforms](docs/testing.md)

## How it is built

| Part | What it is |
| --- | --- |
| `mpe-core` | The library: document operations, rendering, editing, OCR, printing |
| `mpe` | The command-line tool |
| `mpe-gui` | The desktop app |

It uses [`lopdf`](https://crates.io/crates/lopdf) for the document structure and encryption,
[`hayro`](https://crates.io/crates/hayro) to draw pages (on the CPU), [`egui`](https://crates.io/crates/egui) for the
interface, [`rfd`](https://crates.io/crates/rfd) for file dialogs, [`image`](https://crates.io/crates/image) for
images, and [`ocrs`](https://github.com/robertknight/ocrs) for text recognition.

## Licence

Dual-licensed under MIT or Apache-2.0, at your option. See `LICENSE-MIT` and `LICENSE-APACHE`.

Included works:

- **Icons:** Phosphor Icons (MIT, © Phosphor Icons), embedded as a font by `egui-phosphor`.
- **Interface font:** Inter 4.1 by The Inter Project Authors, SIL Open Font License 1.1, included unmodified; see
  `crates/mpe-gui/assets/fonts/`. Release archives include the licence as `FONT-LICENSE-Inter.txt`.
- **OCR model:** the text recognition model by Robert Knight, from the
  [ocrs](https://github.com/robertknight/ocrs) project, [CC BY-SA 4.0](https://creativecommons.org/licenses/by-sa/4.0/),
  included unmodified; see `crates/mpe-core/models/README.md`, shipped as `OCR-MODEL.md` in release archives. With
  its inference code it adds about 14 MB to each binary.

Dependencies carry their own licences. A scan of the dependency metadata for all platforms found only permissive
licences (MIT, Apache-2.0, BSD, Zlib, Unicode, ISC, Unlicense, BSL-1.0 and similar), plus the SIL OFL 1.1 and Ubuntu
Font Licence for the fonts bundled with egui. A few crates are dual-licensed with LGPL or GPL as one option; the
permissive option applies. If you redistribute binaries, keep the dependencies' licence texts with them
(`cargo about` or `cargo license` can generate the list).

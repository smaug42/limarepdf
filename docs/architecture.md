# How it is built

MPE is written entirely in Rust, with no C or C++ dependencies.

| Part | What it is |
| --- | --- |
| `mpe-core` | The library: document operations, rendering, editing, OCR, printing |
| `mpe` | The command-line tool |
| `mpe-gui` | The desktop app |

It uses:

- [`lopdf`](https://crates.io/crates/lopdf) for the document structure and encryption;
- [`hayro`](https://crates.io/crates/hayro) to draw pages, on the CPU (see [Rendering](rendering.md));
- [`egui`](https://crates.io/crates/egui) for the interface, and [`rfd`](https://crates.io/crates/rfd) for file
  dialogs;
- [`image`](https://crates.io/crates/image) for images;
- [`ocrs`](https://github.com/robertknight/ocrs) for text recognition (see [Scanning and OCR](scanning-and-ocr.md)).

Page content is read by MPE's own parser (`crates/mpe-core/src/objects.rs`), which splits it into the objects that
editing works on and reads the text that search, copying and export use, with its own reader for fonts' ToUnicode
maps and Adobe's widths for the standard fonts (see [Search and text](search-and-text.md#text-extraction)).

In the desktop app, every command is defined once in `crates/mpe-gui/src/actions.rs`; the menu, the top bar, the
command palette, the right-click menus, the floating toolbar and the keyboard shortcuts are all built from that list.

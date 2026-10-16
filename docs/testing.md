# Testing and platforms

## Platforms

| Platform | Status |
| --- | --- |
| Linux x86_64 | Built and tested; the command line run on real PDFs; the desktop app used by hand and checked by screenshot under a virtual X server, including the narrow-window layout |
| Windows x86_64 | Built and tested by continuous integration (`.github/workflows/ci.yml`, GitHub Actions), which also packages all four platforms. Printing, the only platform-specific code, has been tried by hand and works |
| macOS (Intel and Apple Silicon) | Built and tested by continuous integration. Not run by hand |

## Automated tests

Run them with `cargo test --workspace`.

| Tests | What they cover |
| --- | --- |
| `crates/mpe-core/tests/integration.rs` | Overlays, encryption round trips with all four ciphers, search, crop margins on turned pages |
| `crates/mpe-core/tests/edit.rs` | Content editing: objects and text edits. Reading text: words, search boxes placed by glyph widths, ToUnicode maps, word gaps, text inside form XObjects, phrases across lines |
| `crates/mpe-core/tests/render_coverage.rs` | Rendering, pixel by pixel; see [Rendering](rendering.md) |
| `crates/mpe-core/tests/import.rs` | Importing images as pages |
| `crates/mpe-core/tests/scan.rs` | Optimising scanned pages |
| `crates/mpe-core/tests/ocr.rs` | OCR text layers, their placement, search and correction |
| `crates/mpe-core/tests/export.rs` | Exporting pages as images and text |
| `crates/mpe-core/tests/print.rs` | What is sent to the printer, with a stand-in for `lp` |
| `crates/mpe-core/tests/outline.rs` | Bookmarks: reading nesting, every kind of destination, broken and looping outlines; adding, renaming and removing them, with the open/closed counts kept right |
| `crates/mpe-core/tests/links.rs` | Reading links: every kind of target, links that can't be followed, placement on turned pages |
| `crates/mpe-core/tests/annots.rs` | Annotations: reading them, drawing each type that has no drawing (on screen only), removing them with replies and popups, flattening all or some pages; stamps placed, resized, faded, flattened and upright on turned pages; stamps with your own wording, and wording the font can't show refused |
| `crates/mpe-core/tests/forms.rs` | Reading form fields: every kind, inherited values, broken field trees, turned pages, XFA-only and hybrid forms |
| `crates/mpe-core/tests/form_fill.rs` | Filling in forms: values, drawn appearances (checked by rendering, including a turned field), fonts that can't be used, refused values, passwords, reset, drawing missing appearances |
| Unit tests in `crates/mpe-core/src` | Page ranges, document operations, matrices, overlays, print placement, the ToUnicode reader, standard font widths, laying out form fields (wrapping, auto size, alignment, comb, lists) |
| Unit tests in `crates/mpe-gui/src` | The command list (unique entries and shortcuts, menus), selection rules, page ranges, moving pages, fitting the zoom, document notices, recent files, settings, the security form, form fields (Tab order, what a click does, keeping typed text) |

The rendering tests assert what the PDF standard says should appear. The cases hayro gets wrong are marked
`#[ignore]` with the reason; after a hayro upgrade, run
`cargo test -p mpe-core --test render_coverage -- --ignored` to see whether any have been fixed.

## The desktop app

There are no automated tests of the desktop app's screens. Its commands, selection rules and page moves are
unit-tested, and the operations behind it are covered by the core tests and the command line. The redesigned window
(top bar, inspector, command palette, right-click menus, floating toolbar, thumbnail dragging, collapsing inspector)
has been used by hand on Linux. Later additions (recent files, document notices, fit width, the theme button, crop)
were checked by screenshot under a virtual X server; not every one has been clicked through by hand. Of the newest:
the page layouts (facing and single page) have unit tests for pairing and fitting but were not clicked through;
viewer preferences were checked through their core tests and `mpe view-prefs`; following them on opening and the
Opening view section were not clicked through;
the stamp tool (including your own wording) was checked through its core tests and `mpe stamp` renders (including
turned pages), not clicked through;
filling in forms has been used by hand on one form; the Bookmarks tab (including adding, renaming and deleting
bookmarks) and links were checked through their core tests, `mpe bookmarks` and real PDFs, and the XFA notice by
screenshot, but none of these was clicked through. Filled forms were also opened in poppler and
Ghostscript, including encrypted ones; not yet in Adobe Reader.

Each feature page says what was not tested, under "How this was tested".

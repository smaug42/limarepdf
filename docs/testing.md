# Testing and platforms

## Platforms

| Platform | Status |
| --- | --- |
| Linux x86_64 | Built and tested; the command line run on real PDFs; the desktop app used by hand and checked by screenshot under a virtual X server, including the narrow-window layout |
| Windows x86_64, macOS (Intel and Apple Silicon) | Built and tested by continuous integration (`.github/workflows/ci.yml`, GitHub Actions), which also packages all four platforms. Not run by hand. The only platform-specific code is Windows printing, which has been compiled but not run |

## Automated tests

Run them with `cargo test --workspace`.

| Tests | What they cover |
| --- | --- |
| `crates/mpe-core/tests/integration.rs` | Overlays, encryption round trips with all four ciphers, search |
| `crates/mpe-core/tests/edit.rs` | Content editing: objects, text edits, words and search boxes |
| `crates/mpe-core/tests/render_coverage.rs` | Rendering, pixel by pixel; see [Rendering](rendering.md) |
| `crates/mpe-core/tests/import.rs` | Importing images as pages |
| `crates/mpe-core/tests/scan.rs` | Optimising scanned pages |
| `crates/mpe-core/tests/ocr.rs` | OCR text layers, their placement, search and correction |
| `crates/mpe-core/tests/export.rs` | Exporting pages as images and text |
| `crates/mpe-core/tests/print.rs` | What is sent to the printer, with a stand-in for `lp` |
| Unit tests in `crates/mpe-core/src` | Page ranges, document operations, matrices, overlays, print placement |
| Unit tests in `crates/mpe-gui/src` | The command list (unique entries and shortcuts, menus), selection rules, page ranges, moving pages |

The rendering tests assert what the PDF standard says should appear. The cases hayro gets wrong are marked
`#[ignore]` with the reason; after a hayro upgrade, run
`cargo test -p mpe-core --test render_coverage -- --ignored` to see whether any have been fixed.

## The desktop app

There are no automated tests of the desktop app's screens. Its commands, selection rules and page moves are
unit-tested, and the operations behind it are covered by the core tests and the command line. The redesigned window
(top bar, inspector, command palette, right-click menus, floating toolbar, thumbnail dragging, collapsing inspector)
has been used by hand on Linux.

Each feature page says what was not tested, under "How this was tested".

# Changelog

## [Unreleased]

### Desktop app

- Pages fit the window's width by default and keep fitting as the window is resized or the side panels open and
  close. Zooming turns fitting off; the Fit width button or Ctrl+0 turns it back on. Zooming keeps your place in
  the document.
- Recent files on the start screen and under ☰ > File > Open recent.
- Notices above the pages when a document is password-protected, has scanned pages without text, has form fields,
  or is large, with a button to deal with it where there is one.
- Crop pages from the inspector, by margins from each side of the page.
- A Print button in the top bar. The Export menu now holds only the exports.
- A theme button in the top bar to choose System, Light or Dark, remembered between runs.
- Applying password security can be undone.
- The inspector's heading and the button that hides it share one row.

### Fixes

- Long names in the start screen's recent files no longer run past the card.

## [0.3.0] - 2026-10-13

MPE is still a test release: most testing has been on Linux. See
[Features and status](docs/features.md) for what works and its limits.

### Desktop app

- A redesigned window: a top bar with the tools and an Export menu, the Inter font, Phosphor icons, and light and
  dark themes that follow the system.
- An inspector on the right that follows the selection: document settings when nothing is selected, page actions
  for selected pages, and an object's or tool's options. It replaces most dialogs, and collapses to a rail on narrow
  windows.
- A command palette (Ctrl+K), right-click menus on pages, objects, words and thumbnails, and a floating toolbar over
  a selected object or words.
- Drag thumbnails to reorder pages. Delete removes the selected pages or object, with an Undo button in a notice.
- An Open button, and a start screen for opening files or starting from images.

### Scanning and OCR

- Import PNG, JPEG and TIFF images as pages, sized from their resolution and turned upright.
- Optimise scanned pages: store black and white as CCITT G4 and greyscale and colour as JPEG, straighten tilts and
  remove specks.
- Recognise text (Latin script) with a built-in model, adding an invisible, searchable text layer. Recognised text
  can be shown, corrected, selected and copied.

### Text

- Search highlights matches on the page and scrolls to them.
- Select and copy text with the text tool, including recognised text.

### Export and printing

- Export pages as PNG or JPEG images, and the document's text as a `.txt` file.
- Print through CUPS on Linux and macOS, and through the print spooler on Windows. Windows printing has been
  compiled but not yet run, and printing has not yet been tried with a real printer.

### Fixes

- AES-256 files saved by MPE now open in viewers based on poppler, such as Okular and Evince. AES-256 files saved by
  0.2.0 still need to be saved again with the password set; see [Password security](docs/security.md).
- Some encrypted files saved by MPE (about 1 in 2,500; 1 in 550 with AES-256) could not be opened by
  `mpe render --password`. Fixed for files saved from this version.
- Annotations flagged NoView are no longer shown in the desktop app.

### Other changes

- Building MPE now needs Rust 1.95 or newer (was 1.80).
- The command line gains `images`, `insert-images`, `optimize-scans`, `ocr`, `printers` and `print`; `render` can
  write JPEG; `search --boxes` prints where matches are.
- The documentation moved into [docs/](docs/); the README is now a landing page.

## [0.2.0] - 2026-10-12

First test release: viewing, page operations, headers, footers, Bates numbers and watermarks, document properties,
password security, and basic content editing (text, images and shapes), in a desktop app and a command-line tool.

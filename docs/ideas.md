# Ideas pool

Features other PDF tools have that MPE doesn't, collected from a comparison with PDF-XChange Editor, Stirling-PDF,
pdfcpu, Acrobat and Foxit (October 2026). This is a pool to pick from as the app grows, not a plan or a promise.
Items already planned are in [features.md](features.md#planned-and-proposed) and aren't repeated here.

Sizes are rough guesses: **S** small, **M** medium, **L** large. A dash means not yet judged.

## Good first picks

Small or medium additions that build on what MPE already has:

- **Remove blank pages** (S): MPE already renders pages and can measure how empty they are.
- **Split by bookmarks** (S): the bookmark reader and splitting already exist.
- **Extract images** (S–M): save a document's embedded images as files.
- **N-up and booklet layouts** (M): several pages on one sheet, or pages ordered for folding.
- **Page labels** (S–M): show and set page numbering like i, ii, 1, 2.
- **Fill-and-sign with a drawn or typed signature** (M): a natural step before digital signatures.
- **Page colour modes** (S–M): dark pages, inverted or high contrast, to match the app's dark mode.
- **Shell completions** for `mpe` (S): needs the `clap_complete` crate.

## Pages and layout

- N-up, booklet imposition, poster tiling, cutting pages into parts, contact-sheet grids (M)
- Resize or scale to a paper size, make all pages the same size, join everything into one tall page (M)
- Overlay or underlay one PDF on another (M)
- Page boxes: view, add and remove the trim, bleed and art boxes (S)
- Turn pages automatically by text direction (M); remove blank pages (S)
- Split by bookmarks or blank pages (S); split by file size (M)

## Structure and metadata

- Bookmarks: add, edit, sort and delete; import and export as JSON; generate from headings or a table of contents (M)
- Page labels and numbering ranges (S–M)
- XMP metadata editing, keywords and custom properties (M)
- Viewer preferences: the page and layout a document opens with (S)
- Attachments (add, list, extract) and portfolios (M)
- Create and edit links, turn URLs in text into links, links from bookmarks (M)
- A full report on a file: its fonts, images and structure, beyond today's `mpe info` (S–M)

## Optimise and repair

- General compression: downsample and recompress images, remove duplicates (M; today only scans are optimised)
- Linearise for fast web view (M)
- Repair damaged files, and a structure checker (M–L)
- Rasterise pages; convert colours to grey or CMYK, invert or replace colours (M)
- Extract or remove embedded images (S–M)

## Content editing

- Find and replace text (M); spell check (L, needs a dictionary)
- Layers: add, merge, reorder, flatten (L)
- Unicode, CJK and right-to-left text with embedded fonts (L: font subsetting, text shaping and bidirectional
  layout, likely new crates)
- Background images (M; image watermarks are already queued)
- QR codes and barcodes stamped onto pages (S–M, needs a crate)
- Guides, rulers, a grid and snapping (M)

## Annotations and review

- Stamps: custom, made from selected content, or with the date and name filled in (M)
- Callouts, clouds, polylines, carets, typewriter text, file attachments, an eraser for pen strokes (M each)
- Measurement tools with scale calibration and CSV export (L)
- Comment summary and export, saved comment styles (M)
- XFDF and FDF import and export (M)

## Forms

- Create fields: text, check box, radio button, list, button, date, image, signature (L)
- Find fields automatically in flat PDFs (L)
- Calculations and validation (L, without running JavaScript)
- Lock, unlock and remove fields (S); fill many copies from one CSV or data file (M)

## Signing, beyond the planned digital signatures

- Certify documents with permission levels, timestamps, long-term validation (L)
- Remove signatures (S)

## Redaction, beyond the planned redaction

- Find and redact by pattern (regular expressions, saved sets) (M)
- Blur or pixelate instead of a black box (M)

## Standards and accessibility

- PDF/X and PDF/E, and preflight checks (L)
- An accessibility checker, tags, reading order and alt text (L)

## Conversion and creation

- Export to HTML, Markdown and CSV tables (M–L)
- Create a PDF from Markdown, text, CSV, the clipboard or a web page (M each; web pages are L)
- Create a PDF from JSON describing tables and image boxes (M)
- New blank PDF (S); a virtual PDF printer (L, per operating system)
- Convert from Office formats (L, very large)

## OCR and scanning

- OCR on a selected area, more languages, reviewing pages where OCR is unsure (M each)
- Find and split several photos scanned on one page (M)

## Viewing

- Page colour modes: dark pages, invert, high contrast (S–M)
- Split view, several windows, a magnifier, a snapshot tool (M each)
- Full-screen presentation mode (S–M)
- Reflow view (L); read aloud (L)
- Right-to-left layout, touch layout (M)

## Printing

- Booklet and N-up printing, printing comments, paper size and scaling (M)

## Automation and the app

- Watch a folder and process what arrives (M)
- Macros (L)
- An MCP server so AI agents can run MPE's operations (M)
- Shell completions (S); renaming files by their content (M)
- Autosave and crash recovery, sessions, custom shortcuts (M each)

## Left out on purpose

These conflict with MPE working offline and keeping a small attack surface:

- Cloud storage connectors and DocuSign
- Running JavaScript in documents
- 3D, audio and video annotations
- Document sharing and signature-request workflows

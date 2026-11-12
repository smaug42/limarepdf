# Ideas pool

Features other PDF tools have that LimarePDF doesn't, collected from a comparison with PDF-XChange Editor, Stirling-PDF,
pdfcpu, Acrobat and Foxit (October 2026). This is a pool to pick from as the app grows, not a plan or a promise.
Items already planned are in [Features and status](feature-status.md#planned-and-proposed) and aren't repeated
here; items that ship are taken out.

Sizes are rough guesses: **S** small, **M** medium, **L** large. A dash means not yet judged.

Standard features that everyday users of other editors rely on come first. Niche items, marked *(niche)*, stay here
as possible later work but aren't picked ahead of them, so the app doesn't fill up with rarely used tools.

## Good first picks

Small or medium additions that build on what LimarePDF already has:

- **Shell completions** for `limarepdf-cli` (S): needs the `clap_complete` crate.

## Pages and layout

- Saving pages per sheet or a booklet as a PDF (printing them is built), poster tiling, cutting pages into parts,
  contact-sheet grids (M)
- Resize or scale to a paper size, make all pages the same size, join everything into one tall page (M)
- Overlay or underlay one PDF on another (M)
- Page boxes: view, add and remove the trim, bleed and art boxes (S)
- Turn pages automatically by text direction (M)
- Split at blank pages (S, *niche*: scan batches with blank separator sheets); split by file size (M)

## Structure and metadata

- Bookmarks: sort; import and export as JSON; generate from headings or a table of contents (M)
- XMP metadata editing, keywords and custom properties (M)
- Attachments (add, list, extract) and portfolios (M)
- Turn URLs in text into links; links to a bookmark, a named destination or another PDF file (M)
- A full report on a file: its fonts, images and structure, beyond today's `limarepdf-cli info` (S–M)

## Optimise and repair

- Compression beyond Reduce file size: remove duplicate pictures and fonts, make fonts smaller, reduce the pictures
  it leaves (CMYK, palette, black-and-white, JPEG 2000 and JBIG2 ones) and remove parts cropped away (M)
- Linearise for fast web view (M)
- A structure checker that lists what is wrong with a file (M–L; damaged files are already repaired when opened)
- Rasterise pages; convert colours to grey or CMYK, invert or replace colours (M)
- Remove every picture at once (S; saving them and deleting one are built)

## Content editing

- Spelling for the pages' own text in Review > Spelling (PF-60), and the system's spell checkers on Windows and
  macOS (PF-61; spell checking itself is built, NEW-06); `spelling` on the command line (S); `replace` on the
  command line (S–M, on Replace all's preparation, which is built: NEW-04b)
- Layers: add, merge, reorder, flatten (L)
- Right-to-left (Hebrew, Arabic) and shaped (Indic, Thai) text, and emoji made of several characters (L:
  bidirectional layout and these scripts' shaping); Chinese, Japanese and Korean when no installed font has them
- Bold and italic made up for an installed family that lacks them, as word processors do: thicker strokes and a
  slant (M: the writers and the drawing while typing)
- Background images (S: a picture watermark at 100%, opaque and behind the content comes close; filling the page
  edge to edge is what's missing)
- QR codes and barcodes stamped onto pages (S–M, needs a crate)
- Guides of your own, rulers and a grid to snap to (M; snapping to the page's and other objects' edges and middles
  is built, NEW-08)

## Annotations and review

- Stamps made from selected content (S–M)
- Callouts, clouds, polylines, carets, typewriter text, file attachments, an eraser for pen strokes (M each)
- Measurement tools with scale calibration and CSV export (L)
- Comment summary and export, saved comment styles (M)
- XFDF and FDF import and export (M)

## Forms

- Create fields: text, check box, radio button, list, button, date, image, signature (L)
- Find fields automatically in flat PDFs (L)
- Calculations and validation (L, without running JavaScript)
- Lock, unlock and remove fields (S); fill many copies from one CSV or data file (M)

## Signing, beyond digital signatures

- Timestamps from a timestamp server when signing (M; connects to the server you choose, only while signing;
  Acrobat has none set up by default), then long-term validation, adding revocation answers to the document (L).
  Waiting until someone asks for it; signatures carry the computer's time until then
- Revocation checking (OCSP, CRL) for certificates from authorities (M; network)
- Adobe's list of trusted authorities (AATL; only distributed to Adobe products as far as known) or the EU
  trusted lists (large, changing XML with its own signatures) (L)
- Name constraints in certificate authorities (a limit on the names an authority may issue for): checked as RFC
  5280 says, instead of refusing to trust chains that set them (M)
- Importing Acrobat's FDF certificate exchange files (`.fdf`) as trusted certificates (S; `.cer` and `.p7c` work)
- Smart cards, USB tokens, the Windows certificate store and macOS Keychain (L)
- Exporting digital IDs in the older 3DES form with less password stretching, for software that can't read LimarePDF's
  AES-256 exports (S; only if testing with Acrobat shows it is needed, as an explicit option with a warning)
- Exporting certificates as PEM as well as DER `.cer` (S)
- Signing encrypted documents (M)
- Checking signatures on Brainpool curves, used by some European ID cards, and P-521 (S each; the `bp256`,
  `bp384` or `p521` crates, new dependencies)
- Remove signatures (S)
- Fill and sign: typed signatures and text in other scripts (M; more fonts), variable-width strokes from pen
  pressure (M; egui doesn't report pressure yet), `--signature-image` with the paper made transparent from the
  command line (S)

## Redaction, beyond what LimarePDF does

- Find and redact by pattern (regular expressions, saved sets) (M)
- Blur or pixelate instead of a black box (M)

## Standards and accessibility

- PDF/X and PDF/E, and preflight checks (L)
- An accessibility checker, tags, reading order and alt text (L)

## Conversion and creation

- Export to HTML, Markdown and CSV tables (M–L)
- Create a PDF from Markdown, text, CSV, the clipboard or a web page (M each; web pages are L)
- Create a PDF from JSON describing tables and image boxes (M)
- A virtual PDF printer (L, per operating system); the new blank document is built (NEW-09's stage 1)
- Convert from Office formats (L, very large)

## OCR and scanning

- OCR on a selected area, more languages, reviewing pages where OCR is unsure (M each)
- Find and split several photos scanned on one page (M)

## Viewing

- Split view, several windows, a magnifier, a snapshot tool (M each)
- Full-screen presentation mode (S–M)
- Reflow view (L); read aloud (L)
- Right-to-left layout, touch layout (M)

## Printing

- Printing pop-up notes or a comment summary with the pages (M)

## Automation and the app

- Watch a folder and process what arrives (M)
- Macros (L)
- An MCP server so AI agents can run LimarePDF's operations (M)
- Renaming files by their content (M)
- Sessions (reopen the last documents), custom shortcuts (M each)
- File drops on Linux Wayland: a setting or environment variable to run under XWayland, which has drops, or the
  winit upgrade once it supports Wayland drops ([today's workaround](../troubleshooting.md#dropping-files-on-the-window-does-nothing)) (S)

## Left out on purpose

These conflict with LimarePDF working offline and keeping a small attack surface:

- Cloud storage connectors and DocuSign
- Running JavaScript in documents
- 3D, audio and video annotations
- Document sharing and signature-request workflows

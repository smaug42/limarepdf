# Features and status

**Status: 0.5, for testing.** Most testing has been on Linux. Annotations can be viewed, moved, deleted and flattened;
text can be highlighted and marked; notes, text boxes, shapes, lines, freehand drawings and replies added. Digital
signatures and redaction are not started.

## What MPE does

- **View:** tabs, thumbnails, continuous scrolling, zoom, search with highlights on the page, bookmarks, links and
  comments.
- **Organise pages:** drag thumbnails to reorder; rotate, insert, delete and extract pages; merge and split files.
- **Edit content:** move, resize, restyle and delete text, images and shapes; edit text; add text, shapes and images.
- **Fill in forms:** text fields, checkboxes, radio buttons and lists, in the app or from JSON on the command line;
  flatten them so the values can't be changed.
- **Stamp pages:** headers, footers, page numbers, Bates numbers and text watermarks.
- **Scans:** turn images into PDFs, shrink and straighten scanned pages, and add a searchable text layer with
  built-in OCR (Latin script).
- **Copy text,** including recognised text.
- **Protect:** open and permissions passwords, RC4 or AES encryption, permission flags.
- **Export and print:** pages as PNG or JPEG, text as a `.txt` file, and printing (through CUPS on Linux and macOS,
  the print spooler on Windows).
- **Command line:** everything above, for scripts and batches.
- **Private:** no network connections, accounts or licence checks.

## Feature status

**Done:** works, and is covered by tests or has been run. **Partial:** works within the limits noted. **No:** not
available.

| Area | Status | Notes |
| --- | --- | --- |
| Tabs, thumbnails, opening by drag and drop | Done | |
| Recent files | Done | Start screen and ☰ > File > Open recent; 10 files, kept on your computer only |
| Notices about the document | Done | Password-protected, scanned pages, form fields, large files. [Details](gui.md#notices-about-the-document) |
| Zoom, fit width, continuous scrolling, page navigation | Done | Pages fit the window's width by default and follow resizing; zooming turns that off. Single-page and facing-page layouts: No |
| Open at a page from the command line | Done | `mpe-gui file.pdf --page N` |
| Command palette, right-click menus, floating toolbar | Done | Toolbar customisation: No |
| App icon | Done | Window, taskbar and Windows `.exe`. Wayland needs the desktop entry installed. [Details](install.md#menu-entry-and-icon-on-linux) |
| Light and dark themes | Done | Follows the system, or chosen with the top bar's theme button; remembered between runs |
| Rendering | Partial | Some transparency groups are drawn wrongly. Annotations without a drawing of their own are drawn by MPE for the common types only. [Details](rendering.md) |
| Search | Done | Phrases across line breaks are found and highlighted; hyphenated words split over lines are not. [Details](search-and-text.md) |
| Select and copy text | Done | With the default read tool; across pages, scrolling while you drag; in the order on the page, not the file's drawing order; includes recognised text |
| Print or save selected text | Done | As the pages look, full width, cut down to the selected lines, or as plain text in Helvetica (Western European characters only); also saves a .txt file |
| Page operations: insert, delete, rotate, extract, reorder, crop | Done | Crop by margins; no dragging a crop box on the page |
| Merge and split | Done | Bookmarks, forms and named destinations of appended files are not carried over. Split by file size: No |
| Undo and redo | Done | 20 levels, including password settings |
| Headers, footers, page numbers, dates, Bates numbers | Done | Helvetica and Latin-1 characters only |
| Watermarks | Partial | Text only |
| Document properties | Done | Title, author, subject, keywords, creator, producer |
| Password security | Done | All four ciphers and permission flags. [Things to know](security.md) |
| Opening protected files | Done | With the open (user) password |
| Content editing | Partial | Works on lines, not paragraphs; some fonts are replaced when edited. [Details](content-editing.md) |
| Importing images as pages | Done | PNG, JPEG, TIFF. [Details](scanning-and-ocr.md#importing-images) |
| Optimising scanned pages | Partial | Pages that are one image. [Details](scanning-and-ocr.md#optimising-scanned-pages) |
| OCR | Partial | Latin script; built-in recognition model. [Details](scanning-and-ocr.md#recognising-text-ocr) |
| Export pages as images, export text | Done | [Details](export-and-printing.md) |
| Printing | Partial | No paper size or scaling options. Tried on Windows; not yet with a real printer on Linux or macOS. [Details](export-and-printing.md#printing) |
| Driving scanners | No | Not planned: scan with your system's tool and import the images |
| Export to Word, Excel, CSV | No | |
| Annotations and comments | Partial | Shown (MPE draws the common types when the file has no drawing), listed in the Comments tab with replies, deleted, flattened; `mpe annot list\|remove\|flatten`. Highlight, underline, strike out and squiggly-underline selected text; add sticky notes and text boxes (Helvetica, Latin-1 only), rectangles, ellipses, lines with an optional arrowhead, and freehand pen drawings; reply to any annotation; select any annotation to change its colour, opacity and comment (and a shape's width, fill and arrowhead), and move or resize it with the arrow; drag a line's ends to turn it, and turn freehand drawings with their rotate handle. Not yet: turning rectangles, ellipses and text boxes (see below), stamps, polygons, measuring, attachments, XFDF. [Details](gui.md#shapes-lines-and-drawings) |
| Stamps, measuring | No | |
| Forms | Partial | Fill in text fields, checkboxes, radio buttons and lists in the app (Tab between fields, highlight, reset, flatten) and from the command line (`mpe form`, values as JSON, flatten all or some pages). Latin-1 text only; JavaScript doesn't run; creating fields and signing: No. [Details](gui.md#filling-in-forms) |
| XFA-only forms (made for Adobe Reader) | No | Recognised and explained in a notice and by `mpe form`; showing or filling them is not planned (see below). Hybrid forms are filled through their ordinary fields |
| Form data import/export (FDF, XFDF), barcodes, JavaScript | No | |
| Bookmarks | Partial | Shown in the Bookmarks tab (also from the command palette) and followed; adding, editing or deleting them: No. [Details](gui.md#the-window) |
| Links | Partial | Followed: to pages, and to web and email addresses after asking. Links to other PDF files, and adding or editing links: No. [Details](gui.md#links) |
| Layers, attachments | No | |
| Installers and packages | No | Releases are archives of the programs. Packages are planned for later; see below |
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
- Saving removes the validity of digital signatures, as any rewrite does. The app asks first when a document is
  signed; the command line doesn't.
- Files the renderer cannot read show "Rendering…" on their pages; editing may still work.

## Planned and proposed

Ideas for what comes next. None of these are available yet, and the list is not a commitment or an order. A wider
pool of features other PDF tools have is in [ideas.md](ideas.md).

### XFA forms

Many government forms are XFA-only: the PDF holds a "Please wait..." page, and the real form is XML that only Adobe
software lays out. MPE recognises them and says so. Beyond that:

- **Possible later: their data from the command line.** Export an XFA form's values as JSON and fill them from
  JSON, to view, sign or print in Adobe Reader. Useful for batches; it needs an XML parser crate.
- **Not planned: showing and filling them in MPE.** It would need an XFA layout engine (the specification runs to
  over 1,500 pages) and a script engine for their JavaScript and FormCalc, months of work for a format Adobe has
  deprecated and PDF 2.0 removed.

### Turning annotation boxes

Deferred until the core features are complete. PDF stores rectangle, ellipse and text box annotations as upright
boxes, with no angle, so turning them needs a workaround:

- **Text boxes in quarter turns:** the `/Rotate` entry Acrobat writes (MPE already uses it on turned pages). Other
  readers understand it.
- **Rectangles and ellipses at any angle:** saved as polygons, which every reader shows but lists as polygons
  (an ellipse becomes many short sides). A turned drawing inside an upright box is the other way, but readers that
  redraw the shape show it upright again, so it is not planned.

### Digital signatures

A future task, not yet scheduled. Create your own signing identity (a self-signed certificate), sign documents with
a standard PDF signature that other readers check, and verify signatures, in the app and from the command line.
Readers show such a signature as valid but from an unknown signer until the recipient trusts the certificate by
checking its fingerprint with you. Signing adds to the end of the file, so earlier signatures stay valid. Not
planned for the first
version: timestamps, revocation checks, hardware tokens and signing encrypted files.

### Packaging

Since 0.5: every release file and `SHA256SUMS` are GPG-signed with MPE's own key, and each file has a build
attestation (`gh attestation verify`); a dry run of the release job checks this without releasing. Each release has
Linux `.deb` and `.rpm` packages (the rpm signed), signed apt and dnf repositories on the project's GitHub Pages
(rebuilt when a release is published), a universal macOS `MPE.app` in a `.dmg`, and a Windows MSI installer. The
archives stay.

- **Untested:** whether a PDF opened from Finder's Open With loads in MPE, rather than just starting it; installing
  the MSI on Windows.
- **Next:** an AUR package, a Homebrew cask in MPE's own tap, and winget and Scoop manifests. Maybe Flatpak.
- **Code signing** (macOS notarization, Windows Authenticode) needs certificates MPE doesn't have yet, so those builds
  stay unsigned; the GPG signatures and attestations cover them meanwhile.

### Features that would set MPE apart

- **Redaction you can check.** Remove the text, images and metadata under a redaction, not just cover them, then
  report what was removed and verify that none of it remains in the saved file.
- **Sanitise before sharing.** One command that strips metadata (including XMP), JavaScript, attachments and hidden
  layers. It pairs with redaction and with MPE making no network connections.
- **Offline AI.** Summaries, questions and answers, and meaning-based search, run by a model on your own computer, so
  documents never leave it.
- **Comparing documents.** Text and visual differences between two PDFs, also from the command line, so a comparison
  can run in a build or review pipeline.
- **Recipes for batches.** Describe a sequence of steps in a file (for example: recognise text, watermark,
  Bates-number, encrypt) and run it on many documents from the command line.
- **Better OCR.** Scripts beyond Latin; reading the images on pages that also have text (today OCR only reads pages
  with no text at all, so a scanned figure or a pasted screenshot in a normal document is skipped); and matching the
  original fonts when scanned text is edited.
- **Editing paragraphs.** Edit text as paragraphs that reflow, not line by line, as a command you choose ("Edit as
  paragraph") rather than the default: MPE outlines the lines it takes to be the paragraph, you adjust or confirm,
  and the reflow is one undo step. Line-by-line editing stays as it is.
- **Incremental saving.** Save only what changed, which is faster for large files and keeps existing digital
  signatures valid when you add comments or fill in forms.
- **Standards output.** PDF/A for archiving, and PDF/UA tagging for accessibility.
- **Verifiable signatures.** Apply and validate digital signatures, including certificate-based ones, with revocation
  checked offline.

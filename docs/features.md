# Features and status

**Status: 0.7, for testing.** Most testing has been on Linux. Annotations can be viewed, moved, deleted and flattened;
text can be highlighted and marked; notes, text boxes, shapes, lines, freehand drawings and replies added. Content
can be redacted for good, and documents filled in and signed with a picture of your signature. Digital signatures
can be checked, made with your own or an imported digital ID, certified, and kept valid when a signed document is
saved, in the app and from the command line; Adobe Acrobat Reader shows LimarePDF's signatures, and fuller round trips
with Acrobat are still to be tested.

## What LimarePDF does

- **View:** tabs, thumbnails, continuous scrolling, zoom, search with highlights on the page, bookmarks, links and
  comments.
- **Organise pages:** drag thumbnails to reorder; rotate, insert, delete and extract pages; merge and split files.
- **Edit content:** move, resize, restyle and delete text, images and shapes; edit text; add text, shapes and images.
- **Fill in forms:** text fields, checkboxes, radio buttons and lists, in the app or from JSON on the command line;
  flatten them so the values can't be changed.
- **Fill and sign:** put a drawn, typed or picture signature, initials, the date, your name and ticks on any
  document, then fix them into the pages. Not a digital signature.
- **Stamp pages:** headers, footers, page numbers, Bates numbers, and text or picture watermarks.
- **Scans:** turn images into PDFs, shrink and straighten scanned pages, and add a searchable text layer with
  built-in OCR (Latin script).
- **Redact:** mark text, areas or search results, then remove what is under them for good (text, the covered parts
  of pictures and scans, drawings), checked afterwards; and remove hidden information such as metadata, scripts,
  attachments and hidden layers.
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
| Tabs, thumbnails, opening by drag and drop | Done | Dropping files doesn't work in Linux Wayland sessions yet ([workaround](gui.md#opening-files)) |
| Recent files | Done | Start screen and ☰ > File > Open recent; 10 files, kept on your computer only |
| Notices about the document | Done | Password-protected, scanned pages, form fields, large files. [Details](gui.md#notices-about-the-document) |
| Zoom, fit width, continuous scrolling, page navigation | Done | Pages fit the window's width by default and follow resizing; zooming turns that off. Zoom from 10% to 6400%, drawn in tiles at high zoom so pages stay sharp. Single-page and facing-page layouts (top-bar button or View > Page layout; cover page alone or paired), per document, following the document's own preference when it has one |
| Open at a page from the command line | Done | `limarepdf file.pdf --page N` |
| Command palette, right-click menus, floating toolbar | Done | Toolbar customisation: No |
| App icon | Done | Window, taskbar and Windows `.exe`. Wayland needs the desktop entry installed. [Details](install.md#menu-entry-and-icon-on-linux) |
| About window | Done | Version (copyable for bug reports), links, contributors, credits; "Made in Canada". [Details](gui.md#look-and-feel) |
| Light and dark themes | Done | Follows the system, or chosen with the top bar's theme button; remembered between runs |
| Page colour modes | Done | View > Page colours: Normal, Automatic (dark pages with the dark theme), Dark, High contrast, Sepia. Pictures keep their colours unless "Recolour pictures too" is on. On screen only: the file, printing and exports keep their colours. See [Rendering](rendering.md#page-colour-modes) |
| Rendering | Done | Pages that can't be drawn, and pictures and fonts that can't, are reported; high zoom is drawn in tiles up to 6400%; fonts that aren't embedded use installed fonts or similar stand-ins where hayro asks (CID fonts, which Chinese, Japanese and Korean text uses), with a notice saying what to install; annotations without a drawing of their own are drawn by LimarePDF; damaged files are repaired when opened, with a notice. Known limits, mostly hayro's: some transparency groups are drawn wrongly, hairlines can come out light grey, plain fonts that aren't embedded use the standard fonts, media and 3D annotations aren't drawn, and no rich text or line captions. Compared with poppler over about 1,100 public test PDFs. [Details](rendering.md) |
| Search | Done | Phrases across line breaks, and words hyphenated at a line end, are found and highlighted. [Details](search-and-text.md) |
| Select and copy text | Done | With the default read tool; across pages, scrolling while you drag; in the order on the page, not the file's drawing order; includes recognised text |
| Print or save selected text | Done | As the pages look, full width, cut down to the selected lines, or as plain text in Helvetica (Western European characters only); also saves a .txt file |
| Page operations: insert, delete, rotate, extract, reorder, crop, remove blank pages | Done | Crop by margins; no dragging a crop box on the page. Blank means one even colour, give or take specks or a lone page number |
| Merge and split | Done | Split by page count or at bookmarks, files named after them. Bookmarks, forms and named destinations of appended files are not carried over, and split parts have no bookmarks. Split by file size: No |
| Undo and redo | Done | 20 levels, including password settings |
| Headers, footers, page numbers, dates, Bates numbers | Done | Helvetica and Latin-1 characters only |
| Watermarks | Done | Text or a PNG, JPEG or TIFF picture; centred, with size, opacity, rotation, above or behind |
| Document properties | Done | Title, author, subject, keywords, creator, producer |
| Password security | Done | All four ciphers and permission flags. [Things to know](security.md) |
| Opening protected files | Done | With the open (user) password |
| Content editing | Partial | Works on lines, not paragraphs; some fonts are replaced when edited. [Details](content-editing.md) |
| Importing images as pages | Done | PNG, JPEG, TIFF. [Details](scanning-and-ocr.md#importing-images) |
| Optimising scanned pages | Partial | Pages that are one image. [Details](scanning-and-ocr.md#optimising-scanned-pages) |
| OCR | Partial | Latin script; built-in recognition model. [Details](scanning-and-ocr.md#recognising-text-ocr) |
| Export pages as images, export text | Done | [Details](export-and-printing.md) |
| Printing | Partial | No paper size or scaling options. Tried on paper on Linux (HP LaserJet, CUPS): pages, mixed sizes, a turned page, comments and form values print correctly. On Windows only the print dialog has been tried, and nothing has been printed on macOS. [Details](export-and-printing.md#printing) |
| Driving scanners | No | Not planned: scan with your system's tool and import the images |
| Export to Word, Excel, CSV | No | |
| Annotations and comments | Partial | Shown (LimarePDF draws the common types when the file has no drawing), listed in the Comments tab with replies, deleted, flattened; `limarepdf-cli annot list\|remove\|flatten`. Highlight, underline, strike out and squiggly-underline selected text; add sticky notes and text boxes (Helvetica, Latin-1 only), rectangles, ellipses, lines with an optional arrowhead, freehand pen drawings and stamps; reply to any annotation; select any annotation to change its colour, opacity and comment (and a shape's width, fill and arrowhead), and move or resize it with the arrow; drag a line's ends to turn it, and turn freehand drawings with their rotate handle. Not yet: turning rectangles, ellipses and text boxes (see below), polygons, measuring, attachments, XFDF. [Details](gui.md#shapes-lines-and-drawings) |
| Stamps | Done | Approved, Not approved, Draft, Final, Confidential, For comment and Void, or your own wording in green, red or blue (Western European characters only), optionally with the author's name and the date, or a PNG or JPEG picture; click to place or drag to size, then move and resize; `limarepdf-cli stamp`. Not yet: stamps made from selected content, or wording in other scripts. [Details](gui.md#stamps) |
| Fill and sign | Done | A signature or initials drawn, typed in one of four handwriting styles or from a picture (paper made transparent), up to five of each saved for next time; the date in three styles, your name, text and tick, cross and dot marks, in black or blue ink; placed anywhere or in an empty signature field, movable until Finish signing draws them into the pages. Typed signatures from the command line: `limarepdf-cli stamp --signature`. Not a digital signature; typed signatures and text are Latin-1 only; the date is in UTC. [Details](gui.md#filling-and-signing) |
| Measuring | No | |
| Forms | Partial | Fill in text fields, checkboxes, radio buttons and lists in the app (Tab between fields, highlight, reset, flatten) and from the command line (`limarepdf-cli form`, values as JSON, flatten all or some pages). Latin-1 text only; JavaScript doesn't run; creating fields: No; signature fields take a fill-and-sign signature, or a digital signature (Document > Sign with digital ID, or `limarepdf-cli sign --field`). [Details](gui.md#filling-in-forms) |
| XFA-only forms (made for Adobe Reader) | No | Recognised and explained in a notice and by `limarepdf-cli form`; showing or filling them is not planned (see below). Hybrid forms are filled through their ordinary fields |
| Form data import/export (FDF, XFDF), barcodes, JavaScript | No | |
| Viewer preferences | Done | The layout, side panel (Pages or Bookmarks) and page a document opens with: followed when opening, set in the inspector's Opening view or with `limarepdf-cli view-prefs`. Full screen and hiding the toolbar or menu: No. [Details](cli.md#viewer-preferences) |
| Page labels | Done | Numbering such as i, ii, 1, 2 or A-1: shown in the status bar, thumbnails and bookmark tips; set from the selected pages in the inspector or with `limarepdf-cli labels`; kept with their pages when pages move. Going to a page by its label: No. [Details](cli.md#page-labels) |
| Bookmarks | Partial | Shown in the Bookmarks tab (also from the command palette) and followed; added, renamed and deleted in that tab or with `limarepdf-cli bookmarks`. Moving or reordering them, styles and colours: No. [Details](gui.md#the-window), [CLI](cli.md#bookmarks) |
| Links | Partial | Followed: to pages, and to web and email addresses after asking. Links to other PDF files, and adding or editing links: No. [Details](gui.md#links) |
| Layers, attachments | No | |
| Installers and packages | Done | `.deb` and `.rpm` packages with apt and dnf repositories, a macOS `.dmg`, a Windows MSI, and archives; GPG-signed. [Install](install.md); more package managers planned, see below |
| Redaction | Done | Mark areas, selected text or search results (standard redaction marks), check them, then apply: text, drawings, comments and links under them are removed for good, pictures (scans included) blacked out there, then checked. In the app and with `limarepdf-cli redact`. [Details](redaction.md) |
| Removing hidden information | Done | Metadata (including XMP), JavaScript and automatic actions, attachments, hidden layers and their content, thumbnails and private data; if chosen, comments, form values, bookmarks and what lies outside the crop box. Document > Remove hidden information… and `limarepdf-cli sanitise`. [Details](redaction.md#removing-hidden-information) |
| Digital signatures | Done | Checking in the app (a coloured banner, the Signatures tab, Details, View signed version, trusting signers and authorities, Trusted certificates) and from the command line (`limarepdf-cli verify`): unchanged or not, signer's certificate, signing time and timestamps, certification and locked fields, warnings; RSA and ECDSA P-256/P-384; refuses the known wrapping and copying attacks; lists the changes made after each signature and judges them by Acrobat's rules (stricter when unsure), catching the shadow and incremental-saving attacks, and won't vouch for or sign as it is a file PDF apps read differently; saves the version a signature signed; trusting signers and certificate authorities (`limarepdf-cli trust`), with chains checked by RFC 5280's rules at the signing time, in LimarePDF only. Digital IDs in the app (Edit > Digital IDs) and from the command line (`limarepdf-cli id`): make a self-signed one (RSA 3072-bit, five years, like Acrobat's), import `.p12`/`.pfx` files from Acrobat, Windows, macOS or a certificate authority (old and new encryptions), export an ID or its certificate. Signing and certifying in the app (Document > Sign with digital ID, Certify; edits made since opening are signed with it, asking first when earlier signatures wouldn't allow them) and from the command line (`limarepdf-cli sign`): in an empty signature field or a new box, as Acrobat signs (`adbe.pkcs7.detached`, SHA-256), with a visible look (your name in handwriting or a picture, "Digitally signed by" and the date), added after the file's bytes so earlier signatures stay valid; follows certifications and field locks; every signed file checked before it is written. Saving a signed document (Save in the app, every editing command) adds the changes after what was signed, so the signatures stay valid; asks first (the command line refuses without a flag) when they don't allow the changes, and before writing the whole file when it must (redactions, removed hidden information, passwords, damaged files). Not yet: timestamps when signing, encrypted documents; only partly tested with Adobe Acrobat (see below). [Details](signatures.md) |
| Certificate encryption | No | |
| PDF/A, optimised saving | No | Saved files are pruned and compressed |
| Comparing documents | No | |
| Sessions, autosave, backups, preferences, custom shortcuts, translations | No | You are asked before closing unsaved work. Crash recovery is planned (see below) |
| XFA forms, XPS documents | No | |

## Known limits

- Header, footer, Bates and watermark text uses Helvetica and Latin-1 only; other characters print as `?`.
- Headers, footers and watermarks are added to the page content. They are not annotations and cannot be removed one
  by one later; use Undo before saving.
- Saving rewrites the whole file, except for digitally signed documents, whose changes are added after what was
  signed so the signatures stay valid ([details](signatures.md#saving-a-signed-document)). It writes a temporary file
  beside the document, readable only by you (on Linux and macOS) when it replaces a file, then puts it in the
  document's place, so a failed save leaves the original as it was, permissions included.
- Pages the renderer cannot draw say so, with the reason, and a notice says when a picture couldn't be drawn
  ([details](rendering.md#when-something-cant-be-drawn)); editing may still work. Another notice names fonts that
  aren't in the file or installed and says what to install.
- A damaged file is repaired to open it, with a notice; the original is only replaced if you choose to when saving
  ([details](rendering.md#damaged-files)). Stream data is skipped by its stated length, so an attached PDF or text
  crafted to look like objects isn't taken for the file's own content.
- A malformed file that makes LimarePDF fail while opening, editing or saving gets an error message, and a failed edit
  leaves the document as it was. A failure while showing the document (pages, thumbnails, search, bookmarks,
  comments, the inspector) turns that tab into an error page with Save a copy and Close; other tabs carry on. So does
  a menu command, shortcut or panel button that fails: an error message when it changed nothing, else that tab's
  error page. Redactions and removed hidden information are recorded with the change itself, so a later save of a
  signed document always writes the file out in full.
- Text from a file that the app shows as a label or message (link addresses, bookmark names, comment authors and
  text, file names in tab and window titles and print job titles) has direction-changing (bidi), invisible and
  control characters removed and is shortened when very long, so a link can't pretend to go somewhere else. Form
  field values and the command line's output are shown as they are.

## Planned and proposed

Ideas for what comes next. None of these are available yet, and the list is not a commitment or an order. A wider
pool of features other PDF tools have is in [ideas.md](ideas.md).

### Reliability

- **Crash recovery:** when LimarePDF stops without closing normally, the next start offers the documents that had
  unsaved changes, from private copies kept while you work. Your own files are never written by it: a recovered
  document opens as a new unsaved document.
- **Acceptance checks** of everyday workflows by hand, in other readers and on Windows and macOS, as listed in
  [Testing](testing.md#acceptance-checks). They have been run on Linux.
- **A memory limit for undo:** each undo step keeps a whole copy of the document, so 20 edits on a large scan
  can take gigabytes ([measured](performance.md#memory-in-the-desktop-app)). The oldest steps would go first.
- **Some protected files with an empty password** (RC4, with a non-standard password entry) ask for a password
  that other readers don't need.
- **Very complex pages:** one test page with 73,000 drawn paths takes about 10 GB of memory to draw at 150 dpi
  ([Performance](performance.md)).

### Rendering

Done for now (see the Rendering row above). Left for a later round:

- **hayro's own gaps** (knockout and non-isolated transparency groups, thin-line snapping, some shadings, CMaps that
  use other CMaps, and installed fonts for plain fonts that aren't embedded, which hayro doesn't ask for yet),
  listed in [Known to be wrong](rendering.md#known-to-be-wrong). Not reported to hayro yet; whether to is still to
  be decided.
- **Notices for what hayro draws wrongly:** plain fonts that aren't embedded (such as Calibri or Wingdings, drawn
  with a standard font), and knockout or non-isolated transparency groups. Offered, not yet decided.
- **A check on a HiDPI screen** (scale factor 2): tile sizes, the 4096-pixel cap and sharpness have only been tried
  at scale 1.
- **Damaged files LimarePDF still can't open:** show them read-only as hayro reads them, when the repair finds no
  pages. No file in the test set needs it yet.
- **Annotation drawings:** line captions, a caret's paragraph symbol, rich text in text boxes, and Acrobat's artwork
  for standard stamps.

### XFA forms

Many government forms are XFA-only: the PDF holds a "Please wait..." page, and the real form is XML that only Adobe
software lays out. LimarePDF recognises them and says so. Beyond that:

- **Possible later: their data from the command line.** Export an XFA form's values as JSON and fill them from
  JSON, to view, sign or print in Adobe Reader. Useful for batches; it needs an XML parser crate.
- **Not planned: showing and filling them in LimarePDF.** It would need an XFA layout engine (the specification runs to
  over 1,500 pages) and a script engine for their JavaScript and FormCalc, months of work for a format Adobe has
  deprecated and PDF 2.0 removed.

### Turning annotation boxes

Deferred until the core features are complete. PDF stores rectangle, ellipse and text box annotations as upright
boxes, with no angle, so turning them needs a workaround:

- **Text boxes in quarter turns:** the `/Rotate` entry Acrobat writes (LimarePDF already uses it on turned pages). Other
  readers understand it.
- **Rectangles and ellipses at any angle:** saved as polygons, which every reader shows but lists as polygons
  (an ellipse becomes many short sides). A turned drawing inside an upright box is the other way, but readers that
  redraw the shape show it upright again, so it is not planned.

### Digital signatures

Checking, trust, digital IDs, signing, certifying and saving signed documents are done (see the table and
[Digital signatures](signatures.md)). Still to come:

- **Round trips with Adobe Acrobat**, on a tester's computer: Acrobat-signed and certified files checked in LimarePDF,
  LimarePDF's signatures and exported digital IDs used in Acrobat, and signed files saved with changes going each way.
  The compatibility list in [Digital signatures](signatures.md#compatibility) names only what has been tested.
- **Timestamps** from a timestamp server when signing, then long-term validation (revocation answers added to the
  document).

Not planned for now: revocation checks, Adobe's and the EU's lists of trusted authorities, smart cards and system
keystores, signing encrypted files (see [ideas](ideas.md)).

### Packaging

Since 0.5: every release file and `SHA256SUMS` are GPG-signed with LimarePDF's own key, and each file has a build
attestation (`gh attestation verify`); a dry run of the release job checks this without releasing. Each release has
Linux `.deb` and `.rpm` packages (the rpm signed), signed apt and dnf repositories on the project's GitHub Pages
(rebuilt when a release is published), a universal macOS `LimarePDF.app` in a `.dmg`, and a Windows MSI installer. The
archives stay.

- **Untested:** whether a PDF opened from Finder's Open With loads in LimarePDF, rather than just starting it.
- **Next:** an AUR package, a Homebrew cask in LimarePDF's own tap, and winget and Scoop manifests. Maybe Flatpak.
- **Code signing** (macOS notarization, Windows Authenticode) needs certificates LimarePDF doesn't have yet, so those builds
  stay unsigned; the GPG signatures and attestations cover them meanwhile.

### Features that would set LimarePDF apart

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
  paragraph") rather than the default: LimarePDF outlines the lines it takes to be the paragraph, you adjust or confirm,
  and the reflow is one undo step. Line-by-line editing stays as it is.
- **Incremental saving.** Save only what changed, which is faster for large files. Signed documents are already
  saved this way (above); this item is about using it for every document.
- **Standards output.** PDF/A for archiving, and PDF/UA tagging for accessibility.
- **Revocation checked offline.** Use the revocation answers a signed document carries to tell whether a signer's
  certificate was withdrawn, without going online.

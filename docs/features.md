# Features and status

**Status: 0.8, for testing.** Most testing has been on Linux. Annotations can be viewed, moved, deleted and flattened;
text can be highlighted and marked; notes, text boxes, shapes, lines, freehand drawings and replies added. Content
can be redacted for good, and documents filled in and signed with a picture of your signature. Digital signatures
can be checked, made with your own or an imported digital ID, certified, and kept valid when a signed document is
saved, in the app and from the command line; Adobe Acrobat Reader shows LimarePDF's signatures, and fuller round trips
with Acrobat are still to be tested.

## What LimarePDF does

- **View:** tabs, thumbnails, continuous scrolling, zoom, search with highlights on the page, bookmarks, links and
  comments.
- **Organise pages:** drag thumbnails to reorder; rotate, insert, delete and extract pages; merge and split files.
- **Edit content:** double-click a line of text to edit it in place; move, resize, restyle and delete text, images
  and shapes; add text, shapes and images. Changes in the inspector apply as you make them.
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

## What doesn't work yet

Things you would expect an everyday PDF editor to do that LimarePDF doesn't yet, as of 0.8. Each is removed from
this list once it works. The feature table below has the details.

**Typing and editing text**

- Only Latin (Vietnamese included), Greek and Cyrillic characters can be added: in new or edited text, text
  boxes, headers, footers, Bates numbers, watermarks, stamps, form values and fill-and-sign text. New and edited
  text can also be Chinese, Japanese or Korean in an installed font that has them (Noto Sans CJK, say), not in
  Helvetica, Times or Courier (typed in those, a note at the caret asks for an installed font that has them).
  Hebrew and Arabic (written right to left), the scripts whose letters join or change shape (Devanagari and the
  other Indic scripts, Thai) and emoji can't, in any font: when adding text, editing a line or typing a header,
  footer or watermark, a note at the caret says so; form values and stamp wording with them are refused; a plain-text page of selected text, and text boxes another app made without a
  drawing, show them as `?`.
  Typed signatures show what their handwriting has: Caveat has Cyrillic, the other three Latin only.
- Installed fonts are offered only when their licence allows embedding and they have the Latin letters; a
  family's missing bold or italic can't be made up (the button is off), and one line can't mix fonts, so a
  character the chosen font lacks is refused rather than taken from another font. Editing a line keeps its font
  only when the file's font allows it (not for most embedded fonts); otherwise the line is written in the
  installed family it came from, when LimarePDF can embed it, or else in Helvetica, Times or Courier
  ([details](content-editing.md#limits)).
- Text is edited one line at a time: a paragraph doesn't rewrap as you type, so a longer line can run into
  whatever is beside it.
- A line turned on the page, and invisible recognised (OCR) text, are edited in a plain box rather than drawn as
  they look. Over a picture or a shaded background, the line being edited is hidden under one flat colour, the
  one most common around it, until you finish.
- No spell checking.

**Selecting and arranging**

- One object at a time: several pictures, shapes or lines of text can't be selected to move or delete together.
- Bookmarks can't be moved or reordered, and links can't be added or edited.
- Merging doesn't carry over the bookmarks and form fields of the files added.

**Forms, signatures and printing**

- Forms made only for Adobe Reader (XFA) can't be shown or filled; form data files (FDF, XFDF) and form
  JavaScript aren't supported.
- Digital signatures carry your computer's time, not a timestamp from a timestamp server; encrypted documents
  can't be signed; digital IDs on smart cards and USB tokens can't be used.
- Printing has no paper size or scaling options.

**Other**

- No export to Word or Excel, no comparing documents, no PDF/A.
- Text recognition (OCR) reads Latin script only.
- No autosave or custom shortcuts, and English only (unsaved work is kept in recovery copies).
- Tested mostly on Linux: Windows only partly, macOS not at all; printing on paper only on Linux. Dropping files
  onto the window doesn't work in Linux Wayland sessions yet.
- The side panel and the inspector can be made wider or narrower only by dragging their edge with the mouse, not
  from the keyboard. Screen readers haven't been tried yet (Orca, Narrator, VoiceOver).

## Feature status

**Done:** works, and is covered by tests or has been run. **Partial:** works within the limits noted. **No:** not
available.

| Area | Status | Notes |
| --- | --- | --- |
| Tabs, thumbnails, opening by drag and drop | Done | Dropping files doesn't work in Linux Wayland sessions yet ([workaround](gui.md#opening-files)) |
| Recent files | Done | Start screen and ☰ > File > Open recent; 10 files, kept on your computer only |
| Notices about the document | Done | Password-protected, scanned pages, form fields, large files. [Details](gui.md#notices-about-the-document) |
| Zoom, fit width, continuous scrolling, page navigation | Done | Pages fit the window's width by default and follow resizing; zooming turns that off. Zoom from 10% to 6400%, drawn in tiles at high zoom so pages stay sharp. Single-page and facing-page layouts (the layout button at the bottom right or View > Page layout; cover page alone or paired), per document, following the document's own preference when it has one |
| Open at a page from the command line | Done | `limarepdf file.pdf --page N` |
| Command palette, right-click menus, floating toolbar | Done | A tool's tooltip says how to use it; the inspector shows controls, not instructions. Toolbar customisation: No |
| Pages boxes | Done | Numbers and ranges (`1-3,7`, `5-`, `all`) in printing and the inspector's tools; going into one that says `all` selects it, so typing replaces it. [Details](gui.md#selecting-and-the-inspector) |
| App icon | Done | Window, taskbar and Windows `.exe`. Wayland needs the desktop entry installed. [Details](install.md#menu-entry-and-icon-on-linux) |
| About window | Done | Version (copyable for bug reports), links, contributors, credits; "Made in Canada". [Details](gui.md#look-and-feel) |
| Dialogs | Done | One look for every dialog and the command palette, as in the design's mockups (settings in headed groups, labels above fields, outlined fields and buttons, short fields side by side when there is room, smaller margins in narrow windows): task and document in the title, the action button highlighted (red when Undo can't take it back) in the platform's order, the window behind blocked, Esc as Cancel unless work would be lost, Copy on messages, reasons under Details, problems beside their fields (a missing or wrong password under the password, when signing too), and a digital ID form whose work fails comes back with what was typed (passwords aside); Tab goes through fields and buttons in the order shown, and screen readers can name each one (a row's buttons say which document or certificate); long file and printer names are cut short. Each dialog is checked by the app's tests in wide, 400-point-narrow and 300-point-short windows at scales 1, 1.5 and 2, and by screenshot; not yet with a screen reader. Checked on Linux only |
| Names for screen readers | Partial | Every button, field, list and menu in the window, the inspector, the side views and the dialogs has a name a screen reader can say: icons and symbols such as ◀ in words, Bold, Italic and the picked signature or mark with whether they are on. Checked by the app's tests on Linux; not yet tried with a screen reader. The panels' edges are named but can't be moved from the keyboard. [Details](gui.md#screen-readers) |
| Window size | Done | Remembered between runs, and whether it was maximized; position: No. [Details](gui.md#look-and-feel) |
| Light and dark themes | Done | Follows the system, or chosen in View > Theme; remembered between runs |
| Page colour modes | Done | View > Page colours: Normal, Automatic (dark pages with the dark theme), Dark, High contrast, Sepia. Pictures keep their colours unless "Recolour pictures too" is on. On screen only: the file, printing and exports keep their colours. See [Rendering](rendering.md#page-colour-modes) |
| Rendering | Done | Pages that can't be drawn, and pictures and fonts that can't, are reported; high zoom is drawn in tiles up to 6400%; large page images are drawn in bands, which bounds the memory a complex page takes (one test page from 9.7 GB to 1.5 GB), and the app stops drawing a page at a zoom it has moved past; fonts that aren't embedded use installed fonts or similar stand-ins where hayro asks (CID fonts, which Chinese, Japanese and Korean text uses), with a notice saying what to install; annotations without a drawing of their own are drawn by LimarePDF; damaged files are repaired when opened, with a notice. Known limits, mostly hayro's: some transparency groups are drawn wrongly, hairlines can come out light grey, plain fonts that aren't embedded use the standard fonts, media and 3D annotations aren't drawn, and no rich text or line captions. Compared with poppler over about 1,100 public test PDFs. [Details](rendering.md) |
| Search | Done | Phrases across line breaks, and words hyphenated at a line end, are found and highlighted. Chinese, Japanese and Korean text in fonts without a ToUnicode map is read through the font's character map. [Details](search-and-text.md) |
| Select and copy text | Done | With the default read tool; across pages, scrolling while you drag; in the order on the page, not the file's drawing order; includes recognised text |
| Print or save selected text | Done | As the pages look, full width, cut down to the selected lines, or as plain text in Helvetica (Latin, Greek and Cyrillic; other characters come out as `?`); also saves a .txt file |
| Page operations: insert, delete, rotate, extract, reorder, crop, remove blank pages | Done | Crop by dragging the crop box on the pages, shown whole while it is set, or by typing margins. Blank means one even colour, give or take specks or a lone page number. Deleted and extracted-away pages leave nothing in the saved file: bookmarks, named destinations, links, form fields and tags no longer refer to them, and the tags' alternative text for their content goes. A form nested more than 32 deep is refused, changing nothing ([details](gui.md#selecting-and-the-inspector)) |
| Merge and split | Done | Split by page count or at bookmarks, files named after them. Bookmarks, forms and named destinations of appended files are not carried over. Split parts, like extracted pages, keep the bookmarks to their own pages and nothing of the other pages; each is written as soon as it is made, so memory stays near the document's own size. Split by file size: No |
| Undo and redo | Done | 20 levels, including password settings; fewer on very large documents (up to 1 GB per document) |
| Headers, footers, page numbers, dates, Bates numbers | Done | Shown on the pages as you set them, added with Add to pages, then updated in place as you change them. Helvetica; Latin, Greek and Cyrillic characters; Bates numbers up to 18446744073709551615 (more is refused, changing nothing) |
| Watermarks | Done | Text or a PNG, JPEG or TIFF picture; centred, with size, opacity, rotation, above or behind. Shown on the pages as you set them, added with Add to pages, then updated in place |
| Document properties | Done | Title, author, subject, keywords, creator, producer; each kept as you type it (Enter or leaving the field), one undo step each |
| Password security | Done | All four ciphers and permission flags. [Things to know](security.md) |
| Opening protected files | Done | With the open (user) password; files with an empty one (only a permissions password) open without asking, including RC4 files with a shortened password entry that other readers accept, and their notice says so |
| Content editing | Partial | New text is typed on the page where you click, in the font, size and colour it will have, several lines added as one undo step. The font list offers Helvetica, Times and Courier and the installed fonts that allow embedding, searchable, each name in its own font. Double-click a line with the read tool, or click it with the Text tool, to edit it on the page, drawn in the font, size and colour it will have over the line; pictures, shapes and comments are selected, moved and resized with the read tool too, page text after Move or resize text on its toolbar (the read tool stays on) or with the arrow. Changes in the inspector apply at once, each an undo step; typing into one line again soon joins one step. Works on lines, not paragraphs; some fonts are replaced when edited; the text after a deleted, edited or moved line stays where it was (except in vertical writing). [Details](content-editing.md) |
| Importing images as pages | Done | PNG, JPEG, TIFF. [Details](scanning-and-ocr.md#importing-images) |
| Optimising scanned pages | Partial | Pages that are one image; only the pages chosen change, also when they share a picture with others. [Details](scanning-and-ocr.md#optimising-scanned-pages) |
| OCR | Partial | Latin script; built-in recognition model. [Details](scanning-and-ocr.md#recognising-text-ocr) |
| Export pages as images, export text | Done | [Details](export-and-printing.md) |
| Printing | Partial | No paper size or scaling options. Form values that would come out empty (no drawing, and text LimarePDF can't draw): the app asks first, the command line refuses and names the fields. Tried on paper on Linux (HP LaserJet, CUPS): pages, mixed sizes, a turned page, comments and form values print correctly. On Windows only the print dialog has been tried, and nothing has been printed on macOS. [Details](export-and-printing.md#printing) |
| Driving scanners | No | Not planned: scan with your system's tool and import the images |
| Export to Word, Excel, CSV | No | |
| Annotations and comments | Partial | Shown (LimarePDF draws the common types when the file has no drawing; up to 10,000 per page, with a notice on a page that has more, whose others are still removed by Remove comments and redaction), listed in the Comments view with replies, deleted, flattened; `limarepdf-cli annot list\|remove\|flatten`. Highlight, underline, strike out and squiggly-underline selected text; add sticky notes and text boxes (Helvetica; Latin, Greek and Cyrillic characters), rectangles, ellipses, lines with an optional arrowhead, freehand pen drawings and stamps; reply to any annotation; select any annotation to change its colour, opacity and comment (and a shape's width, fill and arrowhead), and move or resize it with the read tool or the arrow; drag a line's ends to turn it, and turn freehand drawings with their rotate handle. Not yet: turning rectangles, ellipses and text boxes (see below), polygons, measuring, attachments, XFDF. [Details](gui.md#shapes-lines-and-drawings) |
| Stamps | Done | Approved, Not approved, Draft, Final, Confidential, For comment and Void, or your own wording in green, red or blue (Latin, Greek and Cyrillic characters), optionally with the author's name and the date, or a PNG or JPEG picture; click to place or drag to size, then move and resize; `limarepdf-cli stamp`. Not yet: stamps made from selected content, or wording in other scripts. [Details](gui.md#stamps) |
| Fill and sign | Done | A signature or initials drawn, typed in one of four handwriting styles or from an image (paper made transparent), up to five of each saved for next time; the date in three styles, your name, text and tick, cross and dot marks, in black or blue ink, picked from the Fill and sign button's menu or pane; placed anywhere or in an empty signature field (one at a time, or several with the tool kept active), movable until Finish signing draws them into the pages. Typed signatures from the command line: `limarepdf-cli stamp --signature`. Not a digital signature; text is Latin, Greek and Cyrillic, typed signatures what their handwriting has (Caveat Latin and Cyrillic, the others Latin); the date is in UTC. [Details](gui.md#filling-and-signing) |
| Measuring | No | |
| Forms | Partial | Fill in text fields, checkboxes, radio buttons and lists in the app (Tab between fields, fields highlighted unless turned off, reset, flatten) and from the command line (`limarepdf-cli form`, values as JSON, flatten all or some pages). Latin, Greek and Cyrillic text, drawn in an embedded font when the field's own can't show it (the field keeps naming its own font for other apps), so a value filled in elsewhere in other scripts shows only if that app drew it; flattening refuses, naming the fields, rather than lose one it can't draw; JavaScript doesn't run; creating fields: No; signature fields take a fill-and-sign signature, or a digital signature (Document > Digital signatures > Sign with digital ID, or `limarepdf-cli sign --field`). [Details](gui.md#filling-in-forms) |
| XFA-only forms (made for Adobe Reader) | No | Recognised and explained in a notice and by `limarepdf-cli form`; showing or filling them is not planned (see below). Hybrid forms are filled through their ordinary fields |
| Form data import/export (FDF, XFDF), barcodes, JavaScript | No | |
| Viewer preferences | Done | The layout, side panel (Pages or Bookmarks) and page a document opens with: followed when opening, set in the inspector's Opening view (each choice applies as you make it) or with `limarepdf-cli view-prefs`. Full screen and hiding the toolbar or menu: No. [Details](cli.md#viewer-preferences) |
| Page labels | Done | Numbering such as i, ii, 1, 2 or A-1: shown in the status bar, thumbnails and bookmark tips; set from the selected pages in the inspector, applying as you set them, or with `limarepdf-cli labels`; kept with their pages when pages move. Going to a page by its label: No. [Details](cli.md#page-labels) |
| Bookmarks | Partial | Shown in the Bookmarks view (also from the command palette) and followed; added, renamed and deleted in that view or with `limarepdf-cli bookmarks`. Moving or reordering them, styles and colours: No. [Details](gui.md#the-window), [CLI](cli.md#bookmarks) |
| Links | Partial | Followed: to pages, and to web and email addresses after asking. Links to other PDF files, and adding or editing links: No. [Details](gui.md#links) |
| Layers, attachments | No | |
| Installers and packages | Done | `.deb` and `.rpm` packages with apt and dnf repositories, a macOS `.dmg`, a Windows MSI, and archives; GPG-signed. [Install](install.md); more package managers planned, see below |
| Redaction | Done | Mark areas, selected text or search results (standard redaction marks), check them, then apply: text, drawings, comments and links under them (every one, also past the 10,000 a page shows) are removed for good, pictures (scans included) blacked out there, then checked. Text whose characters can't be placed for certain (some Chinese, Japanese and Korean fonts) goes a whole piece at a time. Refuses, changing nothing, when a form field under a mark or the tags' index is nested more than 32 deep, or the tags' index is too large to search. In the app and with `limarepdf-cli redact`. [Details](redaction.md) |
| Removing hidden information | Done | Metadata (including XMP), JavaScript and automatic actions (also those following an ordinary link or action), attachments, hidden layers and their content (in nested drawings, comments' drawings and patterns too), thumbnails and private data; if chosen, comments, form values (cleared for good, defaults and read-only fields included), bookmarks and what lies outside the crop box. Refuses, changing nothing, when it can't look into everything a hidden layer could be in, damaged compressed data included, or a drawing shared by places that give its layers different meanings, or when form values are chosen and the form is nested more than 32 deep. Document > Remove hidden information… and `limarepdf-cli sanitise`. [Details](redaction.md#removing-hidden-information) |
| Digital signatures | Done | Checking in the app (a coloured banner, the Digital signatures view, Details, View signed version, trusting signers and authorities, Trusted certificates) and from the command line (`limarepdf-cli verify`): unchanged or not, signer's certificate, signing time and timestamps, certification and locked fields, warnings; RSA and ECDSA P-256/P-384, holding RSA keys to the RSA-PSS limits their certificates set; refuses the known wrapping and copying attacks; lists the changes made after each signature and judges them by Acrobat's rules (stricter when unsure), catching the shadow and incremental-saving attacks, and won't vouch for or sign as it is a file PDF apps read differently; saves the version a signature signed; trusting signers and certificate authorities (`limarepdf-cli trust`), with chains checked by RFC 5280's rules at the signing time, in LimarePDF only. Digital IDs in the app (Document > Digital signatures > Digital IDs) and from the command line (`limarepdf-cli id`): make a self-signed one (RSA 3072-bit, five years, like Acrobat's), import `.p12`/`.pfx` files from Acrobat, Windows, macOS or a certificate authority (old and new encryptions; not IDs allowed only RSA-PSS signatures), export an ID or its certificate. Signing and certifying in the app (Document > Digital signatures > Sign with digital ID, Certify; edits made since opening are signed with it, asking first when earlier signatures wouldn't allow them) and from the command line (`limarepdf-cli sign`): in an empty signature field or a new box, as Acrobat signs (`adbe.pkcs7.detached`, SHA-256), with a visible look (your name in handwriting or a picture, "Digitally signed by" and the date), added after the file's bytes so earlier signatures stay valid; follows certifications and field locks; every signed file checked before it is written. Saving a signed document (Save in the app, every editing command) adds the changes after what was signed, so the signatures stay valid; asks first (the command line refuses without a flag) when they don't allow the changes, and before writing the whole file when it must (redactions, removed hidden information, passwords, damaged files). Not yet: timestamps when signing, encrypted documents (or ones set to get a password when saved); only partly tested with Adobe Acrobat (see below). [Details](signatures.md) |
| Certificate encryption | No | |
| PDF/A, optimised saving | No | Saved files are pruned and compressed |
| Comparing documents | No | |
| Crash recovery | Done | Private copies of documents with unsaved changes, about a minute behind; when LimarePDF stops without closing normally, the next start offers them, and a restored one opens as a new unsaved document. Your own files are never written by it. None for password-protected documents, or documents a protected one's pages are appended to; a redaction, removal of hidden information or added password, or a redo of a redaction or removal, deletes the earlier copies first, including the one a restored document came from. A damaged copy is listed as such and never stops the app starting. Can be turned off. [Details](gui.md#recovery-copies) |
| Sessions, autosave, backups, preferences, custom shortcuts, translations | No | You are asked before closing unsaved work, text still being typed included, and Save keeps that text; unsaved changes are kept in [recovery copies](gui.md#recovery-copies) |
| XFA forms, XPS documents | No | |

## Known limits

- Text with characters the standard fonts lack (beyond Western European) is written in Liberation Sans, Serif or
  Mono, which have the same widths as Helvetica, Times and Courier, embedded in the file and cut down to the
  characters used (a few KB). One font per face grows as text is added; in a signed document, each save adds
  fonts of its own instead, so the signed part stays as it was. Text in an installed font is always embedded
  the same way (OpenType CFF fonts as such; a variable font at the weight chosen).
- `limarepdf-cli render` leaves a filled form field empty when the field has no drawing of its own; the app's
  exports and all printing draw it. To be fixed ([details](export-and-printing.md#pages-as-images)).
- Headers, footers and watermarks are added to the page content. They are not annotations and cannot be removed one
  by one later; use Undo before saving. The one just added can be changed until anything else changes.
- While a watermark or headers and footers are being set, the pages show them drawn over the content, also when
  they are set to go behind it, and in Liberation Sans, which has Helvetica's widths; once added, the pages show
  them as written.
- Saving rewrites the whole file, except for digitally signed documents, whose changes are added after what was
  signed so the signatures stay valid ([details](signatures.md#saving-a-signed-document)). It writes a temporary file
  beside the document, readable only by you (on Linux and macOS) when it replaces a file, then puts it in the
  document's place, so a failed save leaves the original as it was, permissions included. On Linux and macOS the
  save is then confirmed on disk, so a power cut can't undo it; when the system can't confirm it, a message says so
  and the document stays unsaved ([details](gui.md#opening-files)). Settings, recent files and saved
  signatures are written the same way.
- Pages the renderer cannot draw say so, with the reason, and a notice says when a picture couldn't be drawn
  ([details](rendering.md#when-something-cant-be-drawn)); editing may still work. Another notice names fonts that
  aren't in the file or installed and says what to install.
- A damaged file is repaired to open it, with a notice; the original is only replaced if you choose to when saving
  ([details](rendering.md#damaged-files)). Stream data is skipped by its stated length, so an attached PDF or text
  crafted to look like objects isn't taken for the file's own content.
- A file other PDF apps may show differently (parts it deletes still in use, data after its end, entries pointing
  at the wrong object) is shown as the PDF standard says, with a notice and a Save as… button that writes a copy
  every app reads the same; the command line prints a note
  ([details](rendering.md#files-other-apps-may-show-differently)).
- A malformed file that makes LimarePDF fail while opening, editing or saving gets an error message, and a failed edit
  leaves the document as it was. A failure while showing the document (pages, thumbnails, search, bookmarks,
  comments, the inspector) turns that tab into an error page with Save a copy and Close; other tabs carry on. So does
  a menu command, shortcut or panel button that fails: an error message when it changed nothing, else that tab's
  error page. Redactions and removed hidden information are recorded with the change itself, so a later save of a
  signed document always writes the file out in full. `limarepdf-cli` prints such a failure as a plain error and
  exits with status 1.
- Sizes a file states are capped before memory is set aside for them, so a small crafted file can't make LimarePDF
  ask for gigabytes and close: a page image has at most 268 million pixels (A3 at 1152 dpi;
  [export](export-and-printing.md#pages-as-images)), Optimise scans skips pictures of more than 268 million pixels
  with the reason, redaction removes a fax picture of more than that whole, as it does any picture it can't read,
  and a font's character widths are read for 65,536 characters at most. A font's map from characters to text
  (ToUnicode) is read for about a million characters at most, up to 32 MB, with up to 512 bytes of text for one
  character, as the PDF specification allows ([details](search-and-text.md#text-extraction)). Pages of any real
  size show at every zoom. Working out the reading order of a page with tens of thousands of separate lines can't
  run out of stack (which closes a program outright, with no chance to catch it). Looking a name up in a document's
  list of named destinations (for a bookmark, link or the page to open at) reads each part of the list once and at
  most 100,000 parts, so a crafted list can't keep the app busy for hours; a name it gives up on is a destination
  that can't be followed.
- `limarepdf-cli` never changes a command's input: an output naming the input, by any path or link to it, is
  refused with nothing written, and the result is written whole or not at all. When
  the command's own output can't be written (a full disk), it stops with an error and status 1, saying whether its
  file was already saved; a reader that stops early (`… | head`) stops it quietly ([details](cli.md)).
- Text from a file that the app shows as a label or message (link addresses, bookmark names, comment authors and
  text, file names in tab and window titles and print job titles) has direction-changing (bidi), invisible and
  control characters removed and is shortened when very long, so a link can't pretend to go somewhere else. Form
  field values are shown as they are. The command line's listings (`info`, `bookmarks list`, `annot list`, `form
  list`, `labels list`, `view-prefs list`, `objects`, `search`, `verify`) clean such text the same way, without
  shortening it, so a file can't send escape codes to the terminal; `--json` output stays exact.

## Planned and proposed

Ideas for what comes next. None of these are available yet, and the list is not a commitment or an order. A wider
pool of features other PDF tools have is in [ideas.md](ideas.md).

### Reliability

- **More from recovery copies:** copies of password-protected documents, kept encrypted; a recovered copy of a
  signed document saved by adding to the original, when the original is unchanged, so its signatures stay; reopening
  the last session's documents.
- **Acceptance checks** of everyday workflows by hand, in other readers and on Windows and macOS, as listed in
  [Testing](testing.md#acceptance-checks). They have been run on Linux.
- **Very complex pages draw slowly:** one test page with 73,000 drawn paths and 84 shaded areas takes 54 s to draw
  at 150 dpi, where poppler takes 9 s ([Performance](performance.md)). Its memory is now bounded by drawing in bands
  ([Rendering](rendering.md#large-drawings)).

### Rendering

Done for now (see the Rendering row above). Left for a later round:

- **hayro's own gaps** (knockout and non-isolated transparency groups, thin-line snapping, some shadings, CMaps that
  use other CMaps, and installed fonts for plain fonts that aren't embedded, which hayro doesn't ask for yet),
  listed in [Known to be wrong](rendering.md#known-to-be-wrong). They are documented here and in the build plan,
  and not reported to hayro.
- **A very large memory request:** on one test file hayro asks for 7.6 GB at once to fill a single shape at 96 dpi.
  On Linux it draws, since the memory is never used; on Windows, which doesn't promise memory it hasn't got, it
  could stop the app. Not tried on Windows yet; the check is in
  [Testing](testing.md#platform-smoke-checks).
- **Notices for what hayro draws wrongly:** plain fonts that aren't embedded (such as Calibri or Wingdings, drawn
  with a standard font), and knockout or non-isolated transparency groups. Offered, not yet decided.
- **A check on a HiDPI screen** (scale factor 2): tile sizes, the 4096-pixel cap and sharpness have only been tried
  at scale 1.
- **Damaged files LimarePDF still can't open:** show them read-only as hayro reads them, when the repair finds no
  pages. No file in the test set needs it yet.
- **Annotation drawings:** line captions, a caret's paragraph symbol, rich text in text boxes, and Acrobat's artwork
  for standard stamps.

### Printing

- **Print preview and page setup:** a preview beside the Print settings, and paper size, orientation, scaling and
  pages per sheet, for the app and `limarepdf-cli print`. It needs the printer's paper sizes and margins, and one
  layout of pages on sheets shared by the preview and what is printed, so the preview can't differ from the
  printout. Designed separately; today's Print dialog shows only what works now.

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

Not planned for now: timestamps from a timestamp server when signing (the signing time comes from your computer's
clock) and long-term validation, revocation checks, Adobe's and the EU's lists of trusted authorities, smart cards and system
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

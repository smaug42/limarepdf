# Changelog

## [Unreleased]

### Added

- A notice above the pages when parts of the document couldn't be drawn, such as a picture that can't be decoded.
- Zoom up to 6400%, as Acrobat (was 800%). At high zoom only the part of the page in view is drawn, in tiles, so
  pages stay sharp and quick to draw, and memory use stays about the same at any zoom.
- More annotations that files store without a drawing are drawn, on screen and when printing: stamps (the stamp's
  name in a coloured frame), carets, file attachment icons, the seven standard note icons (in the app too), cloudy
  borders, text box callout lines and line leader lines. LimarePDF's own notes now show the Comment icon, a speech
  bubble, as their icon name says.
- Chinese, Japanese and Korean text in fonts the file doesn't embed is drawn with installed fonts, looked up by name,
  family, style and language (Noto CJK, Source Han, MS Mincho and the like), instead of being left out. Such fonts
  named Arial, Times New Roman, Courier New, Calibri or Cambria use Liberation, Carlito or Caladea when the original
  isn't installed, and other Latin ones a similar installed sans, serif or monospaced font instead of showing wrong
  characters.
- A notice names fonts that aren't in the file or installed and says what to install: the font itself when a
  similar one is shown in its place, or a font for the language (such as Noto Sans CJK JP) when no installed font
  has its characters.
- Damaged files open: when a file's cross-reference information (where each object is) is broken or missing, its
  objects are found by reading through the file, as other PDF apps do. A notice says the file was repaired and some
  content may be missing, with a Save as… button; the document counts as changed, and Save asks before replacing
  the damaged original. A repaired file's
  signatures never show as valid and unchanged. `limarepdf-cli` repairs them too, with a note on stderr.

### Fixed

- A page that can't be drawn shows "This page couldn't be drawn" and the reason, instead of "Rendering…" forever
  while the app tried again on every frame.
- Zooming in far on a large page could close the app, because the page was drawn as one picture bigger than the
  graphics card takes. Pages shown more than 4096 pixels a side are now drawn in tiles.

## [0.7.1] - 2026-10-17

### Changed

- MPE is now LimarePDF. The desktop app is `limarepdf`, the command-line tool `limarepdf-cli`, and the packages
  `limarepdf`; the app shows as LimarePDF in "Open with", menus, the Start menu and on macOS. Settings, digital IDs
  and trust lists are kept in a `limarepdf` folder (`~/.config/limarepdf/` on Linux). The repository and package
  site are now github.com/smaug42/limarepdf and smaug42.github.io/limarepdf.

## [0.7.0] - 2026-10-17

### Added

- Page colour modes, from View > Page colours: Dark (dark paper and light text, hues kept), High contrast (white on
  black), Sepia, or Automatic (dark pages while the app's theme is dark). Pictures keep their own colours unless
  "Recolour pictures too" is on. Remembered between runs. On screen only: the file, printing and exports are not
  changed.
- Bookmark editing: add a bookmark to the current page, rename one, add one inside another, or delete one with the
  bookmarks inside it, from the Bookmarks tab (Undo reverses each), and list, add, rename and remove them with
  `mpe bookmarks`.
- Page layouts, from a top-bar button or View > Page layout: facing pages (two side by side, with the first page shown alone like a book
  cover, or paired) and single page (one at a time; scrolling past a page's end turns it), besides continuous
  pages. The choice applies to the document being viewed and is remembered for documents opened later.
- Viewer preferences: a document that sets the layout, side panel (Pages or Bookmarks) or page it opens with now
  opens that way. Set them in the inspector's Opening view section, or with `mpe view-prefs list|set|clear`.
- Stamps: Approved, Not approved, Draft, Final, Confidential, For comment and Void, your own wording in green, red
  or blue, with your name and the date if you like, or a picture of your own, from the Annotate menu. Click to put
  one down or drag to size it, then move, resize or fade it like other annotations. Your name can be changed right
  in the stamp tool's pane. The last five wordings and pictures are remembered. Also `mpe stamp` (`--text` and
  `--color` for your own wording).
- Redaction: mark areas (Annotate > Mark areas for redaction), selected text or every search result, check the
  red-outlined marks, then Document > Apply redactions… removes the text, drawings, comments, links and form fields
  under them from the file (not just covering them), blacks out the pixels of pictures and scanned pages there, and
  paints the areas black. It then checks that nothing readable is left there, and reports what was removed and
  where the removed text still appears elsewhere. Marks are standard PDF redaction marks, so they can be reviewed
  in other apps. While there are marks, a notice above the pages offers Apply redactions…; saving with marks not
  yet applied asks first. Also `mpe redact` (`--text`, `--area`, `--mark-only`, `--apply`).
- Fill and sign: the Sign button in the top bar puts your signature, initials, the date (in three styles), your
  name, text or a tick, cross or dot on the page, in black or blue. Signatures and initials are drawn on a pad,
  typed in one of four handwriting styles (Caveat, Cedarville Cursive, Homemade Apple and Zeyada, bundled), or taken from a photo or scan
  with the paper made transparent; up to five of each are remembered. Click to put one down or drag to size it;
  clicking an empty signature field signs in it. Items stay movable until Finish signing (in a notice above the
  pages, the Sign pane and the Document menu) draws them into the pages. Typed signatures from the command line with
  `mpe stamp --signature` (`--style`, `--color`, `--date`). This is a picture of a signature, not a digital
  signature.
- Remove hidden information: Document > Remove hidden information… lists what the document carries besides its
  pages and removes what you tick: metadata (including XMP), JavaScript and automatic actions, attachments, hidden
  layers and their content, thumbnails and private data, and if you like comments, form values, bookmarks and what
  lies outside the crop box. Also `mpe sanitise` (`--list`, `--keep`, `--also`).
- Checking digital signatures: `mpe verify` tells for each signature whether the signed part is unchanged, who
  signed according to their certificate and who issued it, when (from a timestamp authority or the signer's
  computer), certification and locked fields, and warnings such as SHA-1, short keys or an expired certificate.
  RSA and ECDSA (P-256, P-384) signatures, embedded and document timestamps; copied, moved and wrapped signatures
  are refused. It also lists what changed after each signature (form filled in, signed again, comment added, page
  content changed, and so on) and whether the signer allowed it, by Acrobat's rules: filling, signing and comments
  are allowed after an ordinary signature, a certification allows less, a locked field may not change, and
  anything MPE doesn't recognise counts as not allowed. Malformed additions and the published "shadow" and
  incremental-saving attacks are caught. `--signed-version N -o FILE` saves the version a signature signed.
  `--json` for scripts. See [Digital signatures](docs/signatures.md).
- Trusting signers: `mpe trust add` trusts a signer (from a signed PDF or a `.cer`, `.pem` or `.p7c` file) or a
  certificate authority, `mpe trust list` and `mpe trust remove` manage the list, and `mpe verify` then says
  whether each signer is trusted and why not. Chains to a trusted authority are built from the certificates the
  signature and the document carry and checked by RFC 5280's rules at the signing time (each issuer allowed to
  issue, key usages, path lengths, certificate policies); forged issuers, look-alike certificates with a trusted
  name, SHA-1 or 1024-bit links and unknown critical extensions aren't trusted. Embedded and document timestamps'
  authorities are judged the same way. `--require-trusted` makes `mpe verify` fail unless every signer is
  trusted, and `--trust-file` uses another list. Trust settings affect MPE only.
- Digital IDs, for signing: `mpe id create` makes a self-signed ID like Acrobat's (RSA 3072-bit,
  valid five years) protected by a password you choose; `mpe id import` imports a `.p12` or `.pfx` file from
  Acrobat, Windows, macOS or a certificate authority, old and new encryptions alike, refusing ones MPE couldn't sign
  with and saying why; `mpe id export` saves an ID with its key (`.p12`, for other apps or a backup) or its
  certificate (`.cer`, for others to trust you); `mpe id list` and `mpe id remove` manage them. IDs are kept in
  the settings folder, each encrypted with AES-256 under its own password (600,000 PBKDF2 iterations); passwords
  are asked for without being shown, never taken on the command line, and never stored.
- Signing with a digital ID: `mpe sign` signs in an empty signature field (`--field`) or a new box (`--page`,
  `--rect`), or certifies the document (`--certify no-changes|forms|comments`), as Acrobat does
  (`adbe.pkcs7.detached`, SHA-256, SHA-384 for P-384 keys), with your reason and location if given. The signature
  is always visible: your name in handwriting (or a picture of your signature, `--image`) beside "Digitally signed
  by NAME" and the date in your time zone. It is added after the file's existing bytes, so earlier signatures stay
  valid. MPE refuses expired IDs, encrypted documents, and what a certification or an earlier signer's lock doesn't
  allow; a field that locks others makes them read-only; fill-and-sign items are drawn in first (locked in place on
  a document someone already signed). Every signed file is checked before it is written: the new revision read back
  object by object, then the signatures checked as `mpe verify` would.
- Checking: a form field made read-only when a field that locks it is signed is now an allowed change (form-filling
  level), as Acrobat makes locked fields read-only when signing.
- Signing in the app: Document > Sign with digital ID (also in the Sign tool's pane) and Certify. Click an empty
  signature field or drag a box, then pick the ID, type its password, and optionally a reason, a location and, for
  a certification, what others may change; your saved fill-and-sign signature can be the look's left part. The
  signed copy is saved as a new file (`NAME-signed.pdf` suggested) and opens in place of the document. Edits made
  since opening are signed with it; when they would show on earlier signatures as not allowed, MPE says which and
  asks before signing. After redacting or removing hidden information, the document is written out in full before
  it is signed, so nothing removed stays in it (a document already signed is saved first). Signing runs in the
  background; a password is used once and overwritten.
- Digital IDs window (Edit > Digital IDs): create, import, export (certificate or with its key), make default and
  remove digital IDs, as `mpe id` does.
- Digital signatures in the app: a signed document is checked when it opens, and a banner above the pages gives
  the result, coloured green, amber, grey or red. The Signatures tab of the side panel (View > Signatures) lists
  each signature with its result, the changes made after it and any warnings, then the empty signature fields.
  Details (also by clicking a signed field) shows the signer, certificate, times, changes and certificates, with
  Trust this signer…, Trust an authority…, View signed version (opens in a new tab) and Go to field. Edit >
  Trusted certificates… lists, removes and adds trusted certificates (also from a `.cer`, `.pem` or `.p7c` file).
  After saving, the saved file is checked again.
- Saving a signed document keeps its signatures: Save and Save as add your changes after what was signed, as
  Acrobat does, so each signature stays valid and lists the changes. MPE asks first when the signatures don't allow
  the changes, naming each signer and what they will be told, and asks before writing the whole file when it must,
  which breaks the signatures: after redacting or removing hidden information (the signed version would still hold
  what was removed), with a password set, or for encrypted or damaged files. Every editing command on the command
  line does the same, refusing unless `--accept-disallowed` or `--break-signatures` is given; `mpe sanitise` takes
  `--break-signatures` too. A signed form's fields without a drawing of their own are drawn on screen and in print
  only, not into the document.

### Changed

- A new app icon: the PDF icon from the start page, on the window, the taskbar, the About window, the Windows `.exe`
  and installer, the Mac app and the Linux packages.
- A new About window (☰ > Help > About): the app icon, version with a Copy button (version and system, for bug
  reports), "Made in Canada" with a maple leaf, links to the website, licence and issue tracker, a Contributors page,
  and the credits for bundled fonts, icons and the text recognition model. The start screen also says "Made in
  Canada".
- The form notice ("This document has a form") no longer shows for documents whose only fields are signature
  fields.

- Link addresses, bookmark names, comments and their authors, tab and window titles and print job titles are
  cleaned before they are shown: direction-changing (bidi), invisible and control characters are removed and very
  long text is shortened, so a link can't disguise where it goes.

### Fixed

- The Windows build's tests: git no longer changes line endings in PDFs on checkout, which broke the signed test
  files. CI jobs now get a read-only token unless they need more.
- Files whose trailer says `/Encrypt null` (not encrypted, written by some signing tools) open instead of asking
  for a password that doesn't exist.
- Parts of a file that a later revision deletes, and copies in object streams that the file's index doesn't list,
  are no longer shown, edited or saved: MPE reads them as the PDF standard says.
- Text in Type 3 fonts whose glyphs aren't measured in thousandths of an em is placed correctly for selection,
  search highlights and copying.

## [0.6.0] - 2026-10-15

Page tools and sturdier handling of damaged files: remove blank pages, split at bookmarks, page labels and picture
watermarks; search that finds hyphenated words; and a malformed file now shows an error instead of closing the app.
The Windows installer shows its steps.

### Added

- Remove blank pages: pages of one even colour, give or take specks or a lone page number. In the app (☰ menu and
  command palette; Undo brings them back) and as `mpe remove-blank`, with `--tolerance` for how much may be on a
  page.
- Split at bookmarks, down to a chosen level, with each file named after its bookmark: in the app's Split into
  files section and as `mpe split --bookmarks [LEVEL]`. Splitting in the app runs in the background.
- Page labels (page numbering such as i, ii, 1, 2 or A-1): shown in the status bar, thumbnails and bookmark tips, set
  from the selected pages in the inspector, and listed, set and removed with `mpe labels`. Labels stay with their
  pages when pages are deleted, inserted, moved, extracted or appended.
- Picture watermarks: a PNG, JPEG or TIFF picture centred on each page, sized as a share of the page, with opacity,
  rotation, and above or behind the content. In the app's Watermark section (Text or Picture) and as
  `mpe watermark --image FILE [--scale PERCENT]`. The picture is stored once, however many pages it is on.

### Changed

- New rectangles and ellipses have no fill; turn Fill on in the inspector to fill them.
- In the app, after adding a watermark or headers and footers, the section's Update button changes what was just
  added instead of stamping another copy on top; Add another keeps it and adds a second.
- The Windows installer shows its steps, says in plain words that MPE is free and open source, and lets you choose
  where to install MPE.

### Fixed

- Search finds words hyphenated at a line end, with or without the hyphen, in the app and in `mpe search`.
- Search highlights follow the page's reading order, as the results list does, so every match listed is highlighted
  and stepping through results goes to the right place.
- On Windows, a console window no longer opens behind the app.
- The start screen card's outline stays lit while the pointer is over its buttons and recent files, instead of
  flickering.
- Two watermarks with different opacities on the same page each keep their own opacity.
- Headers, footers, watermarks and OCR text no longer break the page's own text and images on pages that take their
  resources from the page tree.
- A malformed file that makes MPE fail while opening, editing or saving shows an error instead of closing the app.
  A failed edit leaves the document as it was. A failure while showing a document (pages, thumbnails, search,
  bookmarks, comments, the inspector) puts an error page in that tab, with Save a copy and Close, instead of
  closing the app with all its tabs.
- Release builds check integer arithmetic for overflow, so a malformed file stops with an error instead of producing
  wrong results.

## [0.5.0] - 2026-10-15

Installable packages and signed releases: a Windows installer, a macOS app, Linux packages with apt and dnf
repositories, and GPG signatures and build attestations for every download. Saving is also safer.

### Packages and signed releases

- Releases are signed: every file and `SHA256SUMS` has a GPG signature (`.asc`) made with MPE's own key, and a build
  attestation that `gh attestation verify` checks. [docs/install.md](docs/install.md#checking-a-download) explains
  how.
- Linux `.deb` and `.rpm` packages (the rpm GPG-signed), installing both programs, the menu entry, the icon and the
  docs, with the libraries the app needs as dependencies.
- Signed apt and dnf package repositories at https://smaug42.github.io/mpe/, for Debian, Ubuntu, Fedora and
  openSUSE, so MPE updates with the rest of the system.
- A macOS app, `MPE.app` in a `.dmg`, for Apple Silicon and Intel in one, listed under Open With for PDFs. The
  command-line tool is inside it.
- A Windows installer (`.msi`): Program Files, a Start menu entry, Open with for PDFs (and a choice in Default
  apps), `mpe` on the `PATH`, upgrades and an uninstaller.

### Saving and safety

- Saving a digitally signed document asks first, since the saved copy's signatures will show as invalid. Saving as
  a new file keeps the signed original. Core: `Pdf::signature_count`.
- Saving is safe: MPE writes to a temporary file beside the document and then replaces it, so a failed save (a
  full disk, an error part way) leaves the original untouched. Saving through a symbolic link keeps the link, and
  a replaced file keeps its permissions. Applies to the app and every command.
- A page's content, a drawing inside a page and text stored as a stream are decompressed up to 256 MB, so a small
  crafted file can't use up all memory when MPE reads its text.

### Desktop app

- "New from images…" is now "Make a PDF from images…", on the start screen and in the menu, with a hover tip on the
  start screen.

## [0.4.0] - 2026-10-14

Annotations and forms: highlight, comment on and draw on documents, reply to comments, fill in and flatten forms,
follow links and bookmarks, and select text without changing tools.

### Annotations

- A Comments tab beside Bookmarks lists the document's annotations (comments, notes, highlights, drawings) page by
  page, with replies under what they answer. Click one to go to it; right-click to delete it. ☰ > Document >
  Flatten annotations… makes them part of the pages. See [Comments and markup](docs/gui.md#comments-and-markup).
- Highlight selected text (the Highlight button above the selection, right-click, or Ctrl+Shift+H), or underline,
  strike it out or squiggly-underline it, in colours that are remembered. The marks are standard annotations that
  other readers show. Click a mark or comment to select it; the inspector changes its colour, opacity and comment,
  and Delete removes it. Marks carry your name, the computer's user name unless set under ☰ > Edit > Your name for
  comments…. See [Marking text](docs/gui.md#marking-text).
- Sticky notes and text boxes: the new Annotate button in the top bar holds both tools. Click to place a note and
  type its comment beside it; drag a text box and type straight onto the page. Double-click either to change its
  text. Text boxes on turned pages read upright. See [Notes and text boxes](docs/gui.md#notes-and-text-boxes).
- The arrow tool selects, moves and resizes annotations (notes keep their size; text markup stays on its text), and
  the arrow keys nudge them.
- The rectangle, ellipse and line tools now add annotations, and have moved into the Annotate button's menu
  with a new pen for freehand drawing (strokes made in quick succession form one drawing). Lines can have an
  arrowhead, and Shift keeps squares, circles and 45° lines. "Draw on page (permanent)" in the inspector keeps
  the old behaviour of drawing into the page content. A selected shape, line or drawing has its width, fill and
  arrowhead in the inspector. See [Shapes, lines and drawings](docs/gui.md#shapes-lines-and-drawings).
- A selected line or arrow has handles on its two ends instead of a box, so it can be turned and pointed
  elsewhere with the arrow tool. A selected freehand drawing has a rotate handle.
- Reply to an annotation from the Comments tab (right-click > Reply…) or the inspector, which also lists the
  replies it has.
- Annotations stored without a drawing of their own, which used to be invisible, are drawn: highlights,
  underlines, strike-outs, squiggly underlines, notes, text boxes, rectangles, ellipses, lines, polygons and
  freehand drawings. The file is left as it is.

### Forms, links and text

- Fill in forms: click a field with the read tool to type in it, tick it or choose from its
  options; Tab moves between fields. Each change is one undo step. ☰ > View > Highlight fields shows every field,
  and the inspector's Form section has Clear field, Reset form and Flatten form…, which makes the fields part of
  the pages so the values can't be changed (Undo brings them back). Fields with a value but no drawing of their own
  now show their value. See [Filling in forms](docs/gui.md#filling-in-forms).
- Forms made only for Adobe Reader (XFA forms, which show a "Please wait..." page) get a notice explaining that
  only Adobe software can show or fill them; `mpe form list` and `mpe form fill` say the same.
- Links work: click one with the read tool (dragging still selects text), or Ctrl+click with the editing tools.
  Links to pages go there; links to web and email addresses show the address and ask before opening it in your
  browser or email program.
- A Bookmarks tab beside Pages and Search shows the document's outline. Click a bookmark to go to its page.
  ☰ > View > Bookmarks and the command palette open it.
- Select text without changing tools: the hand tool and the text cursor are now one read tool, which every
  document opens with. Dragging anywhere on a page selects text from the nearest word; dragging past the edge of
  the view scrolls it, so a selection can run across pages. Triple-click selects a line and Shift+click extends
  the selection. See [Copying text](docs/gui.md#copying-text).
- Print or save just the selected text: right-click it, or use ☰ > Edit. Choose the pages as they look, full width but
  cut down to the selected lines, or the words alone as plain text; saving also offers a text file. See
  [Copying text](docs/gui.md#copying-text).
- Selecting, copying and exporting text follow the order on the page, not the order the file draws it in, so a
  selection no longer jumps when it reaches a block drawn out of place (common in forms), and exported text comes
  out in reading order. See [Text extraction](docs/search-and-text.md#text-extraction).

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
- MPE has its own icon, on the window, the taskbar and, on Windows, `mpe-gui.exe` itself. The Linux archive
  includes a desktop entry and icon, which Wayland desktops need to show it; see
  [Install](docs/install.md#menu-entry-and-icon-on-linux).

### Command line

- `mpe form list` lists a form's fields with their kinds, values, pages and flags; `--json` prints the values as a
  JSON object.
- `mpe form fill` fills in text fields, checkboxes, radio buttons, drop-downs and lists, from `--set NAME=VALUE`
  or a JSON file like the one `form list --json` prints, and `mpe form reset` puts fields back to their defaults.
  MPE draws the filled fields itself, so the values show in every reader. See [Forms](docs/cli.md#forms).
- `mpe form flatten` draws a form's fields into its pages and removes them (`--pages` for some pages only), and
  `mpe form fill --flatten` fills in and flattens in one go.
- `mpe annot list` lists annotations (also `--json`), `mpe annot remove` removes them, all or by type, author and
  page, and `mpe annot flatten` draws them into the pages. See [Annotations](docs/cli.md#annotations).
- `mpe render` draws pages as the app does: annotations marked not to be shown on screen are left out, and
  annotations without a drawing of their own are drawn.

### Fixes

- Search highlights and selected words sit on the text: characters are placed by their widths in the font instead
  of being spread evenly, which put highlights a letter or more off in proportional fonts. Files that use the
  standard PDF fonts without their widths are measured with Adobe's published widths.
- Text is read from many more files correctly. ToUnicode maps that the document library rejected (LibreOffice
  writes them, for one) are now read, where before the text came out as nonsense and could not be searched.
  Export text, `mpe text` and search now read the page the same way as highlights and copying.
- Words separated only by a gap, with no space character, keep their space in extracted text, search and copying,
  including in tightly justified lines and headings.
- Ligatures such as "ﬁ" come out as their letters, so searches find words written with them.
- A phrase that runs across a line break is highlighted on both lines; before, it was listed but not highlighted.
- Text inside form XObjects (blocks of page content that some PDF makers put all their text in) is now read, so it
  can be searched, selected, copied and exported. Pages whose only text is there are no longer taken for scans.
- Headers, footers and watermarks measure accented Latin-1 letters by their real widths, so centred and
  right-aligned text containing them is placed correctly.
- Long names in the start screen's recent files no longer run past the card.

### Testing

- Printing on Windows, built in 0.3.0 but not run then, has been tried and works.

### Other changes

- Search takes about twice as long as before, as it now reads pages with MPE's own text reader: 0.17 s for a
  477-page manual. See [Performance](docs/performance.md).
- Release notes on GitHub end with a link to every change since the previous version.
- Installers and packages (a macOS app, Linux packages, Homebrew) are planned for once the app is more complete;
  see [Features and status](docs/features.md#packaging).

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

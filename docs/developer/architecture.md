# How it is built

LimarePDF is written entirely in Rust, with no C or C++ dependencies.

| Part | What it is |
| --- | --- |
| `limarepdf-core` | The library: document operations, rendering, editing, OCR, printing |
| `limarepdf-cli` | The command-line tool |
| `limarepdf-gui` | The desktop app |

It uses:

- [`lopdf`](https://crates.io/crates/lopdf) for the document structure and encryption;
- [`hayro`](https://crates.io/crates/hayro) to draw pages, on the CPU (see [Rendering](../rendering.md)), and
  [`fontdb`](https://crates.io/crates/fontdb) to find installed fonts for fonts a file doesn't embed;
- [`egui`](https://crates.io/crates/egui) for the interface, and [`rfd`](https://crates.io/crates/rfd) for file
  dialogs;
- [`image`](https://crates.io/crates/image) for images;
- [`ocrs`](https://github.com/robertknight/ocrs) for text recognition (see [Scans and OCR](../scanning-and-ocr.md));
  the usual scanner and OCR libraries, SANE and Tesseract, are C/C++, so LimarePDF doesn't drive scanners;
- [`serde_json`](https://crates.io/crates/serde_json) in the command line, for form values as JSON.

Forms are read in `crates/limarepdf-core/src/forms.rs`. When a value changes, LimarePDF draws the field's appearance itself
(`form_ap.rs`, written into the file by `form_write.rs`) rather than asking other readers to, so filled fields look
the same everywhere; the desktop app's side is `crates/limarepdf-gui/src/fields.rs`.

Every dialog in the desktop app is built from the same parts in `crates/limarepdf-gui/src/dialogs.rs`, so they all
look like Print: `Shell` (the window over the dimmed app, its title and file, and the dialogs' look from
`theme::dialog_style`: outlined fields and buttons 32 points tall), `content` (the settings, scrolling in a short
window, with radio buttons and checkboxes on the dialog's background from `theme::dialog_choices`), `group` (a
heading and its settings), `field` (a label above a field, which also names it for screen readers; text fields go
through `theme::field`), `columns` (two fields side by side, one under the other when narrow), `list_box`,
`list_rows`, `list_row` and `list_pick` (an outlined list, rows with their buttons on the right, or rows to pick),
`fact` (a name and a value to read) and `footer` (a note and the buttons, in the platform's order). Margins are
smaller in windows under 600 points wide. A button on a list's row is named with what the row is (`set_name`), and
`list_rows` gives each row its own id for what it remembers. Tests draw a dialog with `dialogs::drawn_window` and
check it with `fits` (inside the screen, its buttons inside it) and `unclear_names`. The command palette (`palette_ui` in `app.rs`) uses the same look
without `Shell`, as a launcher with no title or buttons.

Damaged files are repaired in `crates/limarepdf-core/src/repair.rs` when lopdf reads a file with no pages, or had
to rebuild its table itself (its own scan ignores `/Length`, so its result is kept only when the repair fails): it
scans the bytes for `N G obj` headers (skipping stream data by a `/Length` followed by `endstream`, else to the
first `endstream`; an indirect length is looked up among every `N G obj <integer> endobj` in the file, a first pass,
taking the latest that fits and isn't inside the data its own value or the longest fitting one gives, so a copy
planted in an attachment can't end the stream early; a later copy of a number wins), takes the trailer entries from the newest trailer or
cross-reference stream dictionary outside stream data that names a catalog, appends a rebuilt cross-reference
table, and has lopdf read the file again. Object stream members lopdf's reader skips are then read one by one, and
when the trailers name no usable catalog, the last catalog object with a page tree is used. See
[Rendering](../rendering.md#damaged-files).

Two workarounds in `crates/limarepdf-core/src/doc.rs` read a corrected copy of a file when lopdf refuses it (offsets
stay where they were, so nothing counts as repaired). `blank_null_encrypt` blanks a trailer's `/Encrypt null`, which
lopdf takes as encryption. `pad_user_hash` handles a standard-security file at revision 3 or 4 whose `/U` is
shorter than 32 bytes: readers compare only its first 16 bytes there, and some writers store just those (PDFBox's
test file 1910), but lopdf 0.45 refuses any length but 32 (`InvalidHashLength`) and so reports a wrong password.
The copy has the encryption dictionary appended as an update with `/U` zero-padded to 32 bytes. The workaround
stays until lopdf accepts the shorter `/U`; no issue has been filed upstream.

Page content is read by LimarePDF's own parser (`crates/limarepdf-core/src/objects.rs`), which splits it into the objects that
editing works on and reads the text that search, copying and export use, with its own reader for fonts' ToUnicode
maps and Adobe's widths for the standard fonts (see [How text is read](../reading-and-finding.md#how-text-is-read)). CID
fonts' character maps, and Adobe's tables from character numbers to Unicode, are read with hayro's CMap parser
(`hayro_cmap`, reached through hayro, which carries the predefined maps); the predefined ones are read once per
process (`fonts::predefined`).

Text LimarePDF writes (new and edited text, headers, footers, watermarks, stamps, text boxes, form values, the
signature look, selected text as a plain page) goes through a pen (`embed::Pen`, from `Pdf::pen`): the standard
font, WinAnsi-encoded through `fonts::winansi_byte`, while the text fits it, else the embedded font below.
`writable` (core) tells the app which characters can be typed. Form appearances measure and encode through
`form_ap::Metrics`, which is one-byte widths or an embedded font's codes. In the
app, new text is typed on the page in `crates/limarepdf-gui/src/page_text.rs`, drawn while typing in the bundled
Liberation fonts that `typing_fonts.rs` picks (same widths as Helvetica, Times and Courier), then written line by
line with `Pdf::add_text` as one undo step. A line being edited (`editor.rs`'s line editor, sharing `page_text`'s
drawing) sits on `PageObject::baseline` and is drawn in the font `TextEdit::keeps_font` says `Pdf::edit_text` will
write: the line's own (installed, else its Liberation look-alike), the installed family it is from, or a
standard one. The line underneath is
covered in the colour the page worker (`worker.rs`, `Worker::sample`) finds around it, from a drawing of that
part of the page in the current page colours.

Text the standard fonts can't write goes into an embedded font (`crates/limarepdf-core/src/embed.rs`,
`Pdf::embed_codes`): the bundled Liberation face of the standard font (`LIBERATION`, in core's `assets/fonts/`,
stored compressed by core's `build.rs` and unpacked by `bundled_face` the first time each is used; the app's
`typing_fonts.rs` draws with the same copy, added to egui then too) as a Type 0 font with a CIDFontType2 descendant, `Identity-H`,
`/W` widths and a ToUnicode map. The font file is cut down to the glyphs used by the `subsetter` crate. There is
one such font per face per document, rewritten in place as text is added: the old glyphs are given to the
subsetter first, so they keep their numbers and earlier text stays valid. The font dictionary's `/LimarePDF` entry
records the face, the glyphs with their characters and a checksum of the font file; a font another app has
changed since is left alone and a new one started. In a signed document only fonts made since it was opened
(`Pdf::made_fonts`) are grown, as growing one a signed revision holds would show as a change to it. Characters
are put together (NFC) first; right-to-left and shaped scripts are refused (Liberation has Hebrew). The app's
caret note (`page_text::note`) points to installed fonts only for characters `embed::installed_could_write`
allows (not control characters, right-to-left or shaped scripts, or emoji).

Installed fonts for writing text are listed by `crates/limarepdf-core/src/typefaces.rs`: the families in
`InstalledFonts::system()`'s `fontdb` database whose fonts allow embedding and have the Latin letters, each with
up to four faces (`FaceFile`: file, index, variation settings, PostScript name), read once (`typefaces()`; the
app starts it on another thread at launch, `font_list::warm`). The list only grows, so `Family::Installed(n)`,
its number, stays valid; `FontChoice` stays `Copy`. `embed::Face::of` reads an installed face from its file
the first time and keeps it, and the same embedding applies: always embedded (no WinAnsi path), static CFF fonts
as CIDFontType0 with `/FontFile3 /OpenType`, variable fonts set at their weight by `subsetter`'s `variable-fonts`
feature (TrueType out, CFF2 included). `FontChoice::of_font` finds the family a PDF font is from by its
PostScript name, for edited lines; the standard fonts' own names never match an installed font. In the app,
`font_list.rs` is the inspector's searchable list (names drawn in their own fonts through `typing_fonts::preview`,
files over 4 MB left in the interface font), and `typing_fonts.rs` draws typed text from the family's file with
egui's variation coordinates (`typing_fonts::coords`).

Redaction (`crates/limarepdf-core/src/redact.rs`) has its own pass over the page content rather than editing the
parser's objects: it rewrites each text-showing operator character by character, keeping the rest in place with
spacing numbers, cuts drawings into sub-paths, and redacts inside form XObjects as copies. Pictures are decoded,
blacked out and saved again in `redact_img.rs`. Afterwards the pages are parsed again to check that nothing readable
is left under the marks. Hidden information (metadata, scripts, attachments, hidden layers) is removed by
`sanitise.rs`. See [Redaction](../redaction.md).

Fill-and-sign signatures (`crates/limarepdf-core/src/sign.rs`) are stamp annotations whose artwork LimarePDF draws once:
pen strokes smoothed into curves, typed names as the glyph outlines of a bundled handwriting font (read with the
`skrifa` crate, so no font goes into the PDF), or a picture with its paper made transparent. Finish signing
flattens just those annotations. The app's saved signatures and the Create signature window are in
`crates/limarepdf-gui/src/signing.rs`.

Digital signatures are checked in `crates/limarepdf-core/src/sig_verify.rs` (finding them, the byte-range rules, and
tying each signature value to its dictionary in the version it signed, by position) and `cms_check.rs` (the CMS
signature, certificates, timestamps), over a small BER/DER reader, `asn1.rs`, that walks the data exactly as written
so signed attributes are never re-encoded before checking. The cryptography is RustCrypto's (`rsa`, `p256`, `p384`,
`sha1`, `sha2`, `x509-cert`). What changed after each signature is worked out in `sig_changes.rs`: it reads the
appended revisions' cross-reference sections itself (lopdf ignores free entries and repairs broken tables, which
would hide changes), checks that lopdf reads the same objects, then compares each signed version with the next,
judging every object that differs by each role it plays (page content, comment, form field, signature, document
structure) against the signer's permissions. Trust is in `trust.rs`: the trust list file (`trust.txt` in the
settings folder, whose location `limarepdf_core::settings_dir` gives both the app and the command line), and the chain
search from a signer's certificate to a trusted authority over the certificates the signature and the document's
`/DSS` carry, with RFC 5280's checks at the signing time, including certificate policy processing; each link's
signature is checked over the certificate as written (`cms_check::check_cert_signature`). Trust is applied after
checking (`SignatureReport::apply_trust`), so a changed list can be applied again without checking again. In the
app, `crates/limarepdf-gui/src/signatures.rs` runs the check on a background thread when a file with signature fields
opens (and after a save), keeping the file's bytes for View signed version, and draws the banner, the Signatures
tab, the Details window and the trust windows; a trust change re-applies the list to every open document. Digital
IDs are in `ids.rs` (the ID folder, making self-signed certificates with `x509-cert`'s builder, finding an ID's
certificate by checking a signature made with its key rather than by labels) over `pkcs12.rs`, LimarePDF's own PKCS#12
reader and writer on `asn1.rs` (which also writes DER): it derives and wipes the password keys itself (`pbkdf2`,
`hmac`, the PKCS#12 derivation), so `pkcs5` isn't needed, and reads BER as Windows writes it. Private files (ID
files, the trust list) go through `limarepdf_core::write_private`. It and every document save (`Pdf::save`,
`write_file`, which the app's settings, recent files and signing settings also use) replace files through `atomic.rs`:
a temporary file beside the target, always created new under a fresh name (never one already there), private to the
user on Unix while written, synced, then renamed over the target, and on Unix the folder is synced after the rename
so a power cut can't undo it. When only that folder sync fails, the error is a `NotConfirmed`, a third outcome apart
from saved and failed: the app and the CLI treat the file as the new one and warn that it may need saving again (the
app keeps the document marked unsaved, and its recovery copy). A file system that can't sync folders at all counts as
synced. Platform note: Windows has no call for syncing a folder, so there the file is synced before the rename and a
power cut just after it can still lose the rename.

Crash recovery copies are kept by `crates/limarepdf-gui/src/recovery.rs`, in a private
per-user state folder with one subfolder per running app, which holds a locked `lock` file while the app runs. A tab's
copy is a PDF and a description naming it (length, SHA-256, title, original path), published in an order that leaves
a whole pair after an app crash or a power cut: the new PDF is renamed into place and the folder synced, then the
description, synced again, and only then is the previous PDF removed. Withdrawing a tab's copies, before a redaction,
a removal of hidden information or an added password takes effect, shares a per-tab lock with publishing and reports
whether an earlier copy could still be offered, so the command can stop instead. Temporary files come from
`limarepdf_core::disk`, with `atomic.rs`'s rules; free space is read with `rustix` (Unix) and `windows-sys`
(Windows). The file operations go through a small interface, so the tests can make any of them fail or run them on a
model file system that simulates power cuts (`recovery/model.rs`). The app's side is `app/recover.rs`: each frame,
`recovery/schedule.rs` says which tabs are due (a minute after the first change, at most once a minute); the app
takes a ticket and a clone of the document on the UI thread and queues the write, and one recovery thread
serialises it, without the objects nothing uses (`Pdf::pruned_snapshot`, so nothing a redaction or Remove hidden
information detached is in it), and publishes it in order with saves' and closes' removals. Redaction, Remove
hidden information and an added password withdraw the tab's copies before their edit runs, and for a restored tab
the crashed session's copy it came from too (`Crashed::withdraw`), and stop when either reports that an earlier copy
could still be offered. So does Redo when the step it redoes removed content (each undo step records that, as
`Step::removal`, since `scrubbed` may already be set), as a copy written after the undo holds the content. A
crashed copy whose withdrawal couldn't be confirmed (the folder sync failed) is kept in `Crashed::unconfirmed` and
tried again in full at every withdrawal until a sync succeeds; its folder stays meanwhile. Append PDFs does the same when a file it adds is encrypted, and marks the tab protected. At start the app finds the sessions of runs that ended without cleaning up and offers
their copies; a normal exit (`on_exit`, which a crash never reaches) ends the session and deletes its folder.

Signing (`sig_sign.rs`, `sign_document`) adds the signature dictionary with placeholders, its field or widget and
its look, locks and certification to the document, then hands it to the appended-revision writer,
`incremental.rs`: every object that differs from the file as opened (by value: a real `150.0` and an integer `150`
are the same once written) goes into one revision appended with lopdf's `IncrementalDocument`, with a trailer of
its own (`/Root`, `/Info`, `/ID`, `/Prev`), and is read back twice before it is used: `sig_changes::check_appended`
reads the new cross-reference section with the strict reader and must find exactly the objects written, and lopdf's
reading of the result must give every object as it is in memory. Signing then finds the two placeholders in the
signature object (where that section says it is), patches the byte range, makes the CMS with `cms_write.rs` (DER
by hand, so the pre-release `cms` crate isn't needed) and fills it in, and checks the result with
`verify_signatures` before returning it. When the edits would make an earlier signature report a change it
doesn't allow, it returns a `DisallowedChanges` error naming each signer and change, so the app can ask before
signing again with `accept_disallowed`. See [Digital signatures](../signatures.md).

Saving a signed document (`sig_save.rs`, `append_edits`) uses the same writer without a signature: nothing is
appended when no object changed; otherwise every signature is checked before and after, each must check as it did
(a signature gone, as flattening the form makes it, is refused), and `sig_sign::newly_disallowed`, shared with
signing, names the signers whose signature now reports a disallowed change it didn't report before. The app's
`save_signed` (and the CLI's `save_edited`) call it for a document that had signatures when opened, with the bytes
kept for the Digital signatures view as the base, and fall back to a full rewrite only after asking (encrypted, password
set, damaged, a read-back failure, or `Tab::scrubbed`: redactions applied or hidden information removed since the
last save, tracked through undo and redo, since the signed version would keep what was removed). A signed
document's fields are not drawn into it when it opens (`Tab::new`); the snapshots for display and printing draw
them on a copy instead (`snapshot_hiding`), so a save adds only real edits.

In the app, `crates/limarepdf-gui/src/digital.rs` holds the Digital IDs window and the signing window; making, importing
and exporting IDs, and unlocking and signing, run on a background thread each, with the password moved into that
thread in a wrapper that overwrites it when dropped. The editor's `Tool::DigitalSign` takes the place (an empty
field or a dragged box); the app then gathers what to sign: a document opened with signature fields kept its bytes
(`SigState`) and signs those plus its edits, any other document is written out in full first. The signed file
replaces the tab and is checked like any opened file.

In the desktop app, every command is defined once in `crates/limarepdf-gui/src/actions.rs`; the menu, the top bar, the
command palette, the right-click menus, the floating toolbar and the keyboard shortcuts are all built from that list.

## Content editing

How editing works in the page content (moved from [Editing text and pictures](../content-editing.md), which describes it in
plain words). The page's content stream is split into objects: one per text line, per painted path, per image or
form XObject, and per inline image.

- **Moving or resizing** wraps the object's drawing operators in `q <matrix> cm … Q`.
- **Editing text** rewrites the string in place when the font is a plain, non-subset, standard-encoded simple font
  and the new text is ASCII, with no font, size or colour change. Otherwise the old line is removed and the new
  text is added at the same position, size and colour, in the font chosen, else in the installed family the
  line's font is from (found by its PostScript name, the subset tag left off; `FontChoice::of_font`), else in the
  nearest of the 12 standard fonts. While a line is edited on the page, the app makes the same test as it goes
  (`TextEdit::keeps_font`) and draws the line in the font it will have: the line's own font installed on the
  computer, or else the Liberation face of its kind, when the font is kept; the installed family's font, or the
  Liberation face of the standard font replacing it, otherwise. The old line is covered with the most
  common colour in a 3-point ring around its box, from a separate drawing of that part of the page in the current
  page colours.
- **Adding** text, shapes or images draws them on top of the page. New text of several lines is written as one
  text line each, their baselines 1.2 times the size apart (Acrobat's spacing), in one of the 12 standard fonts
  with the WinAnsi encoding. A line with characters WinAnsi lacks (Polish, Czech, Vietnamese, Greek, Cyrillic and
  so on) is written in the bundled Liberation face of that font instead, embedded in the file and cut down to the
  characters used; one font object per face grows as more is typed. Characters Liberation can't write (right-to-left
  and joined scripts, Chinese, Japanese, Korean) can't be typed. While it is typed, the app draws the text in the
  Liberation face, which has the standard font's widths, so the caret and line length match the written result.
- **Installed fonts** (the inspector's font list after the three standard ones) are the families whose fonts
  allow embedding (`OS/2 fsType`: not restricted, subsetting allowed, not bitmaps only), have outlines and the
  Latin letters; up to four faces each (regular, bold, italic, bold italic), a variable font set at weight 400 or
  700 (and its `ital` or `slnt` axis for italic). Text in one is always embedded, cut down to the characters used,
  the same way: TrueType fonts and variable fonts (set at the weight, which makes them TrueType) as
  CIDFontType2 with `/FontFile2`, OpenType CFF fonts as CIDFontType0 with `/FontFile3 /OpenType`. The app draws
  the text in the font's own file, at that weight.
- **Invisible text** from OCR is edited as invisible text and stays in place.
- **Text after a changed line** stays where it was. Text on the same line can carry on from where the line ended
  (a show operator with no line move before it); when the line is deleted, edited or moved, its end is set again
  with `Tm` before that text, and the line position is set again before the next line move, which `Tm` also
  changes.
- **Inline images** are lifted out of the content before it is read, each replaced by a placeholder operator, and
  written back in place of those operators only, never of text that looks like one. The placeholder's name is
  chosen per content stream so that nothing in the stream already has it.

Every edit is checked to re-parse before it is applied, inline images lifted out as when the page is next read.
An edit that fails leaves the document unchanged.

## Signature crates

Checking and digital IDs use these pure-Rust crates (moved from [Digital signatures](../signatures.md#security-notes)),
from the RustCrypto project unless noted, pinned to exact versions where they are pre-releases:

| Crate | Version | Status | Used for |
| --- | --- | --- | --- |
| `rsa` | 0.10.0-rc.19 | pre-release | RSA keys and signatures |
| `pkcs1` | 0.8.0-rc.5 | pre-release | RSA key encoding |
| `der`, `spki`, `x509-cert` | 0.8.2, 0.8.1, 0.3.0 | stable | certificates; making self-signed ones |
| `p256`, `p384` | 0.14.0 | stable | ECDSA keys and signatures |
| `sha1`, `sha2` | 0.11.0 | stable | digests |
| `hmac`, `pbkdf2` | 0.13.0 | stable | ID file integrity checks and password stretching |
| `aes`, `cbc` | 0.9, 0.2.1 | stable | ID file encryption |
| `des`, `rc2` | 0.9.0 | stable | reading older ID files only (3DES, RC2) |
| `zeroize`, `subtle` | 1.9, 2.6 | stable | wiping secrets, constant-time comparison |
| `getrandom` | 0.4 | stable | random numbers from the operating system |
| `rpassword` | 7.5.4 | stable, not RustCrypto | asking for passwords without showing them (command line) |

"Pre-release" means their programming interface may still change, not that they are untested; they come from the
same project and review as the stable crates. LimarePDF's own reader and writer handle the signature data (see
[Digital signatures](../signatures.md#security-notes)), so the pre-release `cms` crate isn't used.

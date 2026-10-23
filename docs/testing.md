# Testing and platforms

## Platforms

| Platform | Status |
| --- | --- |
| Linux x86_64 | Built and tested; the command line run on real PDFs; the desktop app used by hand and checked by screenshot under a virtual X server, including the narrow-window layout; the acceptance checks below run, including printing on paper |
| Windows x86_64 | Built and tested by continuous integration (`.github/workflows/ci.yml`, GitHub Actions), which also packages all four platforms. Printing, the only platform-specific code, has been tried by hand up to the print dialog; nothing has been printed on paper |
| macOS (Intel and Apple Silicon) | Built and tested by continuous integration. Not run by hand |

## Automated tests

Run them with `cargo test --workspace`.

| Tests | What they cover |
| --- | --- |
| `crates/limarepdf-core/tests/integration.rs` | Overlays, encryption round trips with all four ciphers, RC4 files with an empty password and a 16-byte `/U` (revisions 3 and 4), whether a file needed a password to open, encrypted files reading as other apps read them, search, crop margins on turned pages |
| `crates/limarepdf-core/tests/edit.rs` | Content editing: objects and text edits (moving one object leaves the next one's colour, place and stacking as they were); edited, added and invisible (OCR) text upright and in place on a page that flips and scales itself without restoring the state afterwards, as pages printed from web browsers do, and repeated edits not nesting the old content deeper; the text after a deleted, edited (in place, in a new font, invisible) or moved line kept where it was, drawn and as objects, and left as it was after vertical writing; inline images written back only where they were, not into text that reads like their placeholder or an operator spelled like one. Reading text: words, search boxes placed by glyph widths, ToUnicode maps, predefined Chinese, Japanese and Korean character maps without one (UCS-2, UTF-16 and Shift-JIS; text and widths by character number), word gaps, text inside form XObjects, phrases across lines. Added text keeps every WinAnsi character, those of codes 128 to 159 included. Each text line's baseline and whether it runs upright (scaled, turned, upside down, mirrored, on a turned page, with a rise); whether an edit keeps the line's font, as the app predicts it, matching the font written |
| `crates/limarepdf-core/tests/repair.rs` | Damaged files built by hand open with their page drawn and are marked repaired: wrong offsets, no table or trailer, a cross-reference stream that doesn't decode, a missing `endobj`, a later copy of an object winning, a trailer naming the wrong catalog, object stream members starting with comments, and an encrypted file (a wrong password is still reported as one); a page list holding a page itself (also with the content's offset wrong), and a page whose number a second object takes (as two public test files do), mended and saved clean; intact files aren't repaired; files with no pages to find (an empty page list, one naming a missing page, a document made with none) are refused, not opened empty; a repaired document saves as a clean file. Crafted files: `endstream` in a content stream's comment followed by a fake page, an uncompressed attached PDF reusing the outer file's object numbers (its length given directly, or in an object before or after it, or before it with a spoofed copy of that object inside the attachment), `stream` and a line break inside a string, a stream cut off by the end of the file, a `/Length` too short, an object freed by a later revision (not on the page, not saved), and a number reused at a higher generation. Unit tests in `repair.rs` (dictionary ends, object headers, stream data skipped by `/Length`, direct or in an object of its own (not a copy inside the data), or to `endstream`, trailers inside stream data ignored, trailer values written back) |
| `crates/limarepdf-core/tests/render_coverage.rs` | Rendering, pixel by pixel; see [Rendering](rendering.md) |
| `crates/limarepdf-core/tests/render_compare.rs` | Opt-in: LimarePDF's rendering compared with poppler's `pdftoppm` over about 1,100 public test PDFs; see below |
| `crates/limarepdf-core/tests/save_reopen.rs` | Opt-in: the same public test PDFs saved unchanged, and with page 1 turned and a comment added, then opened again: pages, page sizes and labels, annotations, form fields and values, bookmarks, page text and drawn pages compared with the document before saving; see below |
| `crates/limarepdf-core/tests/render_region.rs` | Drawing part of a page, as the app's tiles at high zoom do: tiles match the whole page pixel by pixel, are cut at the page's edges, land in place at the highest scale, and keep pictures in dark pages; a page drawn in bands matches one drawn at once, and a drawing stops between bands when asked |
| `crates/limarepdf-core/tests/import.rs` | Importing images as pages; picture sizes read from the file's header, with each format's orientation |
| `crates/limarepdf-core/tests/scan.rs` | Optimising scanned pages |
| `crates/limarepdf-core/tests/ocr.rs` | OCR text layers, their placement, search and correction |
| `crates/limarepdf-core/tests/export.rs` | Exporting pages as images and text |
| `crates/limarepdf-core/tests/print.rs` | What is sent to the printer, with a stand-in for `lp`; fields that would print empty (a filled Japanese field with no drawing) named before printing, without changing the document; a filled field with no drawing drawn in when sent as images from a file just opened, as the command line prints |
| `crates/limarepdf-core/tests/outline.rs` | Bookmarks: reading nesting, every kind of destination, broken and looping outlines; named destinations in a list 32 deep whose parts each list the next twice (found, or known missing, at once) and in one too large to search (a name past 100,000 parts goes nowhere, one before is found); adding, renaming and removing them, with the open/closed counts kept right. `src/trees.rs` tests the lookup itself: parts read once (a visit count), too deep and too large told apart from missing, ranges (`Limits`) and loops |
| `crates/limarepdf-core/tests/links.rs` | Reading links: every kind of target, links that can't be followed, placement on turned pages |
| `crates/limarepdf-core/tests/annots.rs` | Annotations: reading them, drawing each type that has no drawing (on screen only), removing them with replies and popups, flattening all or some pages; stamps placed, resized, faded, flattened and upright on turned pages; stamps with your own wording, and wording the font can't show refused; fill and sign: typed, drawn and picture signatures (paper made transparent and margins cropped), initials and marks drawn, labelled and sized, upright on turned pages, and Finish signing drawing in only those items |
| `crates/limarepdf-core/tests/forms.rs` | Reading form fields: every kind, inherited values, broken field trees, turned pages, XFA-only and hybrid forms; which signature fields are empty |
| `crates/limarepdf-core/tests/redact.rs` | Redaction: characters removed with the rest kept in place (spacing, kerning, turned pages, hidden text, alternative text, Shift-JIS codes of one and two bytes), fonts with their own character map read where poppler puts the characters (two-byte codes the map leaves out, mixed one- and two-byte code ranges, a code mapped outside its ranges), with an area over a character there, or where LimarePDF draws it, removing it, and a map without code ranges removing a whole piece; drawings removed or clipped, pictures blacked out (soft masks, JPEG kept as JPEG, inline images, shared pictures) or removed when unreadable, shared forms, annotations, the after-check, redaction marks kept, saved and applied. Saved output checked by `tests/common/saved.rs`: nothing left of the text, or of what touched a mark (a comment with its pop-up and reply, a filled field and its drawing, alternative text on the structure tree, the thumbnail, private data), with what didn't touch it kept. Limits: alternative text 70 tags up goes; a parent tree 33 deep, one too large to search (its page past 100,000 parts), and a field under a mark 34 deep, refused with nothing changed (32 deep, a parent tree listing itself, and the page first in a large one, work; a deep field away from the marks doesn't stop redaction); a shared picture another page draws from forms 12 deep, or through a form without resources using that page's names, stays listed. Opt-in: `-- --ignored cross_check` reads made-up output with `pdftotext` (also `-bbox`, for the character-map case) and Ghostscript, and redacts the Magick++ tutorial from ImageMagick's documentation (subset fonts, so only a reader can tell the text is gone): words throughout and half a picture, checked with `pdftotext`, Ghostscript and `pdfimages` |
| `crates/limarepdf-core/tests/sanitise.rs` | Removing hidden information: metadata and XMP, scripts (web links kept; scripts following ordinary actions in links, buttons, bookmarks and the opening action, and in an annotation written into a page's list), attachments, hidden layers (marked content and a form in a hidden layer; forms 12 deep, annotations' drawings and patterns; a form without resources two pages share with the same layer names removed; refused, with nothing changed, for forms 40 deep, for a drawing that can't be read, and for a form without resources two pages give different layer names, drawn on both or listed on one, in either order), form values (current, default, inherited, rich text and read-only values all gone, signature, options and names kept, Reset form bringing nothing back; a value 32 fields above a box the field list leaves out, and in a loop of parents, gone; 33 above, refused with nothing changed), content outside the crop box, finding without changing. Saved output checked by `tests/common/saved.rs`. Opt-in: `-- --ignored cross_check` reads the output with Ghostscript (form fields drawn), `pdfinfo -js` and `pdftotext` |
| `crates/limarepdf-core/tests/pages_removed.rs` | Extract, delete, split (by count and at bookmarks): nothing of the other pages in the saved file (text, forms, notes and their drawings, field values, alternative text), whatever referred to them (bookmarks, both kinds of named destinations, links, the opening action, a reply, the structure tree, a field with widgets on two pages, a form two pages share); which bookmarks stay; kept pages draw exactly as before; split parts read from the files written, and an extracted part holding in memory only what its pages use. Alternative text reached in every way the structure tree allows (marked-content references as objects of their own and in an array of their own, an object reference to a note, a page given only by the parent, content held by a kid, content on both pages) gone, with page 1's kept and a loop of tags ending; a form 34 deep refused with nothing changed, 33 deep removed; a named destination 40 kids down (with a loop) found. Opt-in: `-- --ignored cross_check` with `pdftotext` and Ghostscript |
| `crates/limarepdf-core/tests/common/saved.rs` | Not tests: the saved-output checker the three files above and `many_annotations.rs` use (stages E and C of the build plan). It reads a saved file as written, before any repair or pruning: every string form of a marker in the raw bytes and in every stream decoded, reachable or not, after checking the file's own cross-reference; the strings each stream shows when read as page content (escapes undone, a `TJ` array's pieces joined); and form fields, widget drawings, actions, optional content, pages and references one by one |
| `crates/limarepdf-core/tests/saved_checker.rs` | The saved-output checker itself: a marker in page content is found however it is written (octal escapes, escaped brackets, line continuations, spaced hex, split across a `TJ` array) as well as plain, hex and UTF-16; parts of a marker in separate shows aren't. Opt-in: `-- --ignored cross_check` confirms `pdftotext` reads each of those forms |
| `crates/limarepdf-core/tests/form_flatten.rs` | Flattening forms: drawn where they were (turned fields too), a checkbox's chosen state, some pages only; refused, with the document unchanged, when a filled value has no drawing and LimarePDF can't draw it, while values with their own drawing, hidden, empty and sizeless fields and buttons flatten |
| `crates/limarepdf-core/tests/many_annotations.rs` | Pages listing more than the 10,000 annotations shown, and annotations written into a page's list directly: Remove comments, Remove attachments, removing a comment with its reply, redaction and applying a mark past the limit deal with every one, checked in the saved file with `tests/common/saved.rs`; comments outside a mark, with their pop-up and reply, kept. Opt-in: `-- --ignored cross_check` reads the output with Ghostscript (Poppler refuses pages with 10,000 annotations or more) |
| `crates/limarepdf-core/tests/form_fill.rs` | Filling in forms: values, drawn appearances (checked by rendering, including a turned field), fonts that can't be used, refused values, passwords, reset, drawing missing appearances |
| `crates/limarepdf-core/tests/digital_ids.rs` | Digital IDs: importing PKCS#12 files in every shape other apps write (OpenSSL-made with `crates/limarepdf-core/tools/make-signature-keys.sh ids`: OpenSSL 3 and 1 defaults, Windows' 3DES, AES-128, PBES2 with 3DES, SHA-512 and PBMAC1 integrity checks, unencrypted parts, P-256 and P-384 keys, non-ASCII and empty passwords, an ID with its authority, an expired one); wrong passwords; files refused with the reason (no integrity check, keys too short or too long, a key not for signing, an unsupported curve, no key, a certificate for another key); every truncation and many changed bytes never panic; making self-signed IDs (certificate contents, five years, file permissions); several IDs, finding them and the default; the folder listing and files put there by hand; exporting and importing elsewhere. Unit tests in `pkcs12.rs`: the PKCS#12 key derivation against published test values, BER as Windows writes it, costly files refused before the work; in `ids.rs`, IDs allowed only RSA-PSS signatures (by the certificate, or by the key too) refused. Opt-in: `-- --ignored` has OpenSSL read LimarePDF's files and take out the key, which then signs a PDF LimarePDF checks and trusts |
| `crates/limarepdf-core/tests/signature_changes.rs` | Changes after signing: form filling, further signatures, comments, metadata and validation data allowed; certification levels and field locks applied; page content, pages, signatures, new fields, actions and links refused; the shadow attacks (hide by freeing or re-pointing objects, page content posing as a comment, a missing object filled in), a change undone before the next signature, malformed and damaged appended data never passing as unchanged; the signed version being exactly the file as signed |
| `crates/limarepdf-core/tests/signature_trust.rs` | Trust: signers trusted directly, and chains to a trusted authority (OpenSSL-made with `crates/limarepdf-core/tools/make-signature-keys.sh trust`), including chains carried in the document's validation data; issuers that may not issue, path lengths, expired links, name constraints, short keys, SHA-1, unknown critical extensions, key usages; certificate policies (required, mapped, any policy inhibited); a forged issuer and look-alike certificates named like a trusted one; loops and over-long chains; timestamp authorities, a document timestamp's authority reached through the intermediate it carries; hand-made certificates for shapes OpenSSL won't make (a repeated extension, mismatched algorithms, a changed certificate, an authority whose key is allowed only RSA-PSS signatures, linking only with one it allows); the trust file (round trip, private permissions, unusable entries kept but never trusted) and certificate files (DER, PEM, `.p7c`). Opt-in: `-- --ignored` compares the chain verdicts with `openssl verify` |
| `crates/limarepdf-core/tests/signing.rs` | Signing with digital IDs (the `ids` test files): RSA, P-256, P-384 and a CA-issued ID with its chain, in new boxes and empty fields, with reasons, locations and pictures; second and third signatures leaving earlier ones valid; certification at each level and what it then allows; field locks making fields read-only and locked fields refused; expired IDs, encrypted documents, bad boxes and wrong fields refused; edits signed along with the signature (a field filled; a page turned only with consent); fill-and-sign items drawn in or locked; forms asking readers to draw fields; classic tables, cross-reference and object streams, damaged files, turned pages. Saving signed documents (`append_edits`): nothing added without changes or after an undo; a comment and a filled field allowed, and saved again on top; disallowed changes named once; what each certification level lets a save add; fields drawn for display only; a deleted comment not written again; passwords, encrypted and damaged files and a removed signature refused; numbers written back without their decimals (`300.0` as `300`) not taken as a change; removing the document information (a change to the trailer alone) added after what was signed, with the earlier bytes kept and the signature valid. Unit tests in `incremental.rs` (only changed objects written, nothing appended without changes, a change to the trailer alone (`/Info` removed, a new `/ID`) appended and read back as edited, deleted objects read as gone, anything that reads back differently refused, hybrid files recognised) and `sig_sign.rs` (dates, text fitting, names the font can't show, finding only the signature's own placeholders). Opt-in: `-- --ignored cross_check` has `pdfsig` and `openssl cms -verify` check LimarePDF's signatures, one of them on a document saved with changes after signing |
| `crates/limarepdf-core/tests/signatures.rs` | Checking digital signatures: every supported algorithm, digest and signature format (made in the test with OpenSSL-made keys, `tests/sigkit/`), timestamps, certification and field locks, warnings; the published attack classes (universal forgery, signature wrapping, incremental saving, copied and spoofed signatures, certificate swaps, tampered CMS); RSA keys allowed only RSA-PSS signatures (RFC 4055) against signatures that keep or break the limits, including timestamps and `adbe.x509.rsa_sha1`; truncated and corrupted files never pass; a file with more signed versions than LimarePDF opens; pyHanko and node-signpdf samples with the result each should get, and the changes each should show. Opt-in: `-- --ignored dss` checks the EU DSS samples (fetch them first with `crates/limarepdf-core/tools/fetch-dss-samples.sh`), and `-- --ignored cross_check` compares LimarePDF's results with poppler's `pdfsig` and OpenSSL, and the RSA-PSS cases with `openssl cms -verify` |
| Unit tests in `crates/limarepdf-core/src` | Page ranges, document operations, matrices, overlays, print placement, the ToUnicode reader, standard font widths, laying out form fields (wrapping, auto size, alignment, comb, lists), replacing files (temporary files created new, never a stale file or a symbolic link, private while written; the folder synced once after the rename, through a test seam; a failed folder sync reported as saved but not confirmed, with the new file in place, for documents and private files; a file system that can't sync folders counting as synced), free disk space |
| `crates/limarepdf-gui/src/recovery/tests.rs` | The crash recovery store: copies written, replaced, found and read back; an app crash and a power cut at each step of writing a copy (power cuts on a model disk where any unsynced change may be lost), with a wrong order shown to fail; a failure at each step; withdrawing copies before a redaction, with every failure case and races against a write in progress, and a crashed session's copy with each outcome (one whose folder sync failed staying unconfirmed at every try, with its folder and leftover files kept track of, until a sync succeeds; no power cut bringing it back afterwards); turning recovery off; damaged and expired copies; a session another running process holds; a session folder, or the recovery folder, that is a symbolic link or not a real folder of this user's left untouched at start (on the model disk and on the real disk, with an unrelated PDF in the folder it leads to); free space, permissions and a read-only folder; the recovery thread's order |
| `crates/limarepdf-gui/src/app/recover.rs`, `recovery/schedule.rs` | Recovery copies in the app, on the model disk: when a copy is due (a minute after the first change, coalesced, one write at a time, a change during a write, failures retried no faster, at once after a withdrawal); copies removed when a tab is saved or closed and at a normal exit; none for password-protected documents, pages taken from them, or a document their pages are appended to (the copy withdrawn first, the append stopping if it can't be); a password withdrawing the copy first; redaction and removing hidden information withdrawing the copy and writing a new one at once, which holds none of the redacted text, metadata or attachments (every object and stream searched); redoing a redaction or a removal of metadata or attachments after undoing it withdrawing the copy written in between, in an ordinary and a restored tab and for a second removal, with no crash or power cut able to offer the content afterwards, and a redo that can't withdraw leaving the document, its flags and both undo lists unchanged at every try; on a restored tab, each of the three withdrawing the copy it came from, or stopping if it can't, at every try while its folder sync fails; each of the three stopping, with the document unchanged and the file named, when an earlier copy could still be offered, and going ahead with a notice when it was only left on disk; turning recovery off always taking effect; recovery that can't start; a restored copy of a signed document (notice at once, no signature check, signatures listed as not checked, Save and Sign asking first); a damaged copy offered, never restored |
| `crates/limarepdf-cli/src/main.rs`, `crates/limarepdf-cli/tests/panics.rs` | The command-line tool's panic guard: a panic becomes a plain error. Opt-in (`-- --ignored`, needs the render corpus): `render` on PDFBox 2363 at 150 dpi, which makes the renderer panic, exits with status 1 and a plain message, without Rust's panic message |
| `crates/limarepdf-cli/tests/escapes.rs` | The command line's listings on a made-up file with an escape code, the one-byte CSI, a bidi override and a line break in its title, a bookmark, a comment's author, a field name, a page label, a font name and the page text: none reaches the output, and the visible text does |
| `crates/limarepdf-cli/tests/verify.rs` | `verify --signed-version` never changes its input: the input's own name, as given or as another path, and a symbolic link to it are refused; a hard link is refused on Unix, and elsewhere gets a file of its own; a separate output holds exactly the signed bytes |
| `crates/limarepdf-cli/tests/input_kept.rs` | `rotate`, `set-info`, `sanitise` and `merge` refuse an output that is their input (its own name, another path to it, a symbolic link, and on Unix a hard link) with the input unchanged; another output works |
| `crates/limarepdf-cli/tests/output.rs` | Output that can't be written: `info`, `text` and `form list --json` to a full disk (`/dev/full`, Linux only) exit 1 with a plain message; `set-info` says its file was saved, and `remove-blank`, whose report comes first, saves nothing; a reader that went away stops quietly, with status 0, or 1 for `verify --require-trusted` on a file with no signatures; ordinary output as before |
| `crates/limarepdf-cli/tests/notes.rs` | The command line's note on stderr for a file other PDF apps may show differently (data after its end), and none for a plain file |
| `crates/limarepdf-cli/tests/print.rs` | `print` refusing, naming the field, when a filled field would print empty; stand-ins for `lp` and `lpoptions` come first on the path, so no printer is ever used, and no job reaches them (Unix only) |
| Unit tests in `crates/limarepdf-gui/src` | The command list (unique entries and shortcuts, menus), selection rules, page ranges, moving pages, fitting the zoom, document notices, recent files, settings and saved signatures (each replaced in one step, so a hard link to the old file keeps it), the security form, form fields (Tab order, what a click does, keeping typed text), digital signature results (colour, sentence and banner for valid, unknown, trusted, warned, broken, changed and certified samples; names from the file cleaned), signing from the signing window (a valid signed copy written, a wrong password, earlier signers named before disallowed changes, signing after consent), the signed copy taking an unchanged tab's place but opening in a new tab when a password was set or text typed meanwhile, and one written but not confirmed on disk taking it too, with the message saying so and the document left marked unsaved, signing refused while a password waits to be saved, passwords overwritten, Print asking first (and sending nothing on Cancel) when a field would print empty, the notice on a page with more annotations than are shown; Add text typed on the page (driven through the window's AccessKit tree: the editor's name, keyboard and baseline on the click, typing and Enter, a refused character and its note, Ctrl+B, the inspector's Bold giving the keyboard back, dragging the outline, a click away adding every line as one undo step, Esc keeping it, nothing typed adding nothing, another tool kept, a click on page text editing that line), a line edited on the page (opened by a double-click with the word selected, or a click with the Text tool; the editor's name, keyboard and place on the baseline, the caret placed by the line as drawn, the line hidden in the colour around it, a refused character, Ctrl+B for the whole line, Esc keeping the change on the same baseline, one Undo taking it back; the font it is drawn in, as core will write it; turned and invisible lines in a plain box), a drag with the Text tool selecting text, the colour around a box found by the page worker (the most common in the ring, in the page colours), typed text in the page colours, the fonts for text being typed (each standard font's Liberation face, the order fonts are tried, installed fonts loaded, Liberation Sans as wide as Helvetica) |

The rendering tests assert what the PDF standard says should appear. The cases hayro gets wrong are marked
`#[ignore]` with the reason; after a hayro upgrade, run
`cargo test -p limarepdf-core --test render_coverage -- --ignored` to see whether any have been fixed.

The comparison with poppler needs `pdftoppm` (poppler-utils) and curl. From `crates/limarepdf-core`, run
`sh tools/render-compare.sh`. It downloads the files listed in `tools/render-samples.txt` (the pdf.js and PDFBox test
files hayro is tested on, about 400 MB, checked against their SHA-256 and never committed) into `tests/data/render`,
then draws up to three pages of each at 96 dpi with LimarePDF and with poppler. The run takes about two minutes. A
pixel counts as different only when its colour is outside the range of the pixels around it in the other picture, so
anti-aliasing doesn't count. The report, `target/tmp/render-compare/index.html`, lists the pages with more than 0.5%
of their pixels different, a different size, something LimarePDF couldn't draw, or a failure on either side, worst
first, with the two pictures and the differences side by side. `summary.txt` beside it has every page's share, to
compare runs before and after a change. `sh tools/render-compare.sh issue12` limits the run to file names containing
`issue12`, and `RENDER_COMPARE_DPI` changes the resolution.

poppler isn't always right: many of the worst-listed pages are poppler's mistakes (a grey page, a garbled scan). Each
listed page needs a person to judge which side is right.

The save-and-reopen check uses the same files. From the repository, after fetching them:
`cargo test --release -p limarepdf-core --test save_reopen -- --ignored --nocapture`. Each file is opened as the app
opens it and saved as the app saves it (a signed file by adding a revision, unless it was repaired), twice: unchanged,
and with page 1 turned and a comment added. The copy opened again must need no repair and have the same pages, page
sizes, page labels, annotations, form fields and their values, bookmarks and page text, and draw the same pages
`render_compare.rs` draws, allowing only for anti-aliasing; in the second check, the pages after the first must draw
as in the file itself. Bytes aren't compared, since a save may renumber objects and compress streams. Each file is
checked in a process of its own, two at a time (`SAVE_REOPEN_JOBS`), each allowed 2 GiB on Linux
(`SAVE_REOPEN_MEMORY`, in MiB) and three minutes, so a file needing more stops only its own check and is listed as
skipped. The report, `target/tmp/save-reopen/index.html`, lists every difference, with pictures of pages that draw
differently; `summary.txt` has one line per file. The test fails when a file not in `tools/save-reopen-known.txt`
(the known limits, each with its reason) differs. `SAVE_REOPEN_ONLY=issue12` limits the run. A full run takes about
five minutes; on 2026-10-20, after the page-list repair, 1,082 of 1,089 files were the same, 1 differed (the known
limit) and 6 were skipped (4 need a password, 1 doesn't open, and PDFBox 2363, where hayro asks for 7.6 GB at
once to fill one shape at 96 dpi). To check that it catches failures, a scratch copy whose save dropped annotations, kept page 1
unturned, cleared field values or drew a square on page 2 was flagged in every file where the breakage showed.

The timings and memory figures in [Performance](performance.md) come from
`python3 crates/limarepdf-core/tools/measure.py`, run from the repository (Linux or macOS, Python's standard library
only). It builds the release CLI, makes a 1,000-page text file and a 200-page colour scan in `target/tmp/measure`,
times each task three times and prints the table with the machine and commit. Peak memory is the program's largest
resident set, as `/usr/bin/time -v` reports it. The four rows for public test files need the files from
`tools/render-compare.sh` (or `tools/fetch-render-samples.sh`) and are skipped without them. `--runs 1` makes a quicker
run, and `--only scan` runs only the tasks whose names contain `scan`. A full run takes about five minutes, and one
of the test files needed about 10 GB of memory before pages were drawn in bands (about 1.5 GB since).

## The desktop app

Its commands, selection rules and page moves are unit-tested, and the operations behind it are covered by the core
tests and the command line. A few app tests draw the whole window without a screen and read what a screen reader
would see: every control has a name at five window widths, and a name that says what it is (not an icon or a
symbol alone) in each tool's inspector, the inspector's sections, the side views, Fill and sign, Create signature,
the digital ID windows and open lists and menus, with Bold, Italic and picked choices saying whether they are on
(not yet tried with a real screen reader); Tab reaches the bars and the Print and Create signature
dialogs in the order shown, docked controls stay docked through resizing, Esc closes the inspector's pop-out and
returns to its button, and dialog buttons are clicked as a user would (confirmations act on their own document).
The dialogs' layout is checked from where their parts are drawn: labels above their fields (Password, Your name
for comments, the digital ID forms, Sign and Certify, the Trust question), headed groups (Remove hidden
information, Signature details), rows with their buttons beside them or under them when narrow (Recover unsaved
changes), Reason beside Location when there is room, and the command palette's field, rows and insets. Every
question and message the app asks (22 of them) is drawn in a wide, a 400-point-narrow and a 300-point-short window
at scales 1, 1.5 and 2, with long file, printer and certificate names: the window inside the screen, its title and
buttons in view, no text past its sides, every control clearly named, Enter answering nothing as it opens (but in
a dialog's only field), Tab reaching each button, and Esc closing it without acting. The Digital IDs window (its
list, forms and running work), Sign and Certify, Signature details, the Trust question, Trusted certificates and
Recover unsaved changes are checked the same way for size, names and Esc. The redesigned window
(top bar, inspector, command palette, right-click menus, floating toolbar, thumbnail dragging, collapsing inspector)
has been used by hand on Linux. Later additions (recent files, document notices, fit width, the theme button, crop)
were checked by screenshot under a virtual X server; not every one has been clicked through by hand. Of the newest:
the page layouts (facing and single page) have unit tests for pairing and fitting but were not clicked through;
viewer preferences were checked through their core tests and `limarepdf-cli view-prefs`; following them on opening and the
Opening view section were not clicked through;
the stamp tool (including your own wording) was checked through its core tests and `limarepdf-cli stamp` renders (including
turned pages), not clicked through;
fill and sign was checked through its core tests (including empty signature fields), unit tests of the saved
signatures file, `limarepdf-cli stamp --signature` renders in all four handwriting styles, and screenshots of the Sign pane, the
notice and the Create signature window (Draw and Type), then used by hand on Linux, including placing a scanned
signature and checking that unsaved signatures are gone after a restart;
redaction was checked through its core tests and `limarepdf-cli redact` on real and scanned PDFs (renders checked), and its
app part (Redact tool, marking selected text and search results, the notice, Apply, the save warning) was used by
hand; removing hidden information was checked through its core tests and `limarepdf-cli sanitise` on real PDFs, its dialog not
clicked through;
checking digital signatures in the app was checked through unit tests on the pyHanko and node-signpdf samples
and screenshots of the banner (unknown signer, broken, certified); the signatures view, Details, the trust windows,
View signed version and clicking a signed field were not clicked through;
signing (`limarepdf-cli sign`) was checked through its core tests, `pdfsig` and OpenSSL, and by hand on a public PDF (the
look rendered by poppler and by LimarePDF); not yet in Acrobat; signing in the app was checked through unit tests that
run the signing window's background work (signed copy written and valid, wrong password, the disallowed-changes
question and signing after consent) and screenshots of the Digital IDs window, the signing and certifying windows
and the placing tool's pane; none of these was clicked through;
filling in forms has been used by hand on one form; the Bookmarks tab (including adding, renaming and deleting
bookmarks) and links were checked through their core tests, `limarepdf-cli bookmarks` and real PDFs, and the XFA notice by
screenshot, but none of these was clicked through. Filled forms were also opened in poppler and
Ghostscript, including encrypted ones; not yet in Adobe Reader. Drawing pages in tiles at high zoom was checked by
screenshot at 3200% and 6400% and used by hand on Linux, zooming and scrolling up to 6400%; not yet on a HiDPI
screen. Drawing large pages in bands was used by hand on Linux, zooming PDFBox 5134 on a 4K screen at scale 1 with
its memory recorded (performance.md).

The 2026 window redesign (bars, panels, docking, editing in the read tool, placement pins, dialogs) was checked by
screenshot on Linux under a virtual X server, driven with xdotool: at 1400, 1200, 1000, 800, 700 and 400 points
wide, in the light and dark themes, at display scales 1, 1.5 and 2 (the X11 scale factor, not a real HiDPI
screen); with the Pages, Bookmarks and Comments views; a comment selected with its controls floating, docked, and
docked in a narrow window with the pop-out opened and closed by Esc; a form (fields highlighted) and a signed
document (banner); and the Print (also in a 450-point-high window, where its settings scroll and the buttons stay),
close confirmation and Digital IDs dialogs, the last filled in with the keyboard alone to show each kind of problem
beside its field. Nothing was printed. Not tried: Windows and macOS, a real HiDPI screen, a screen reader.

Add text typed on the page (2026-10-22) was checked the same way: two lines typed at 1200, 800 and 400 points wide
and in a 420-point-high window, in the light, dark and system themes, at scales 1, 1.5 and 2, with the note for a
refused character; the text written after a click elsewhere lands where it was typed, within a pixel or two. Not
tried: Windows and macOS, a real HiDPI screen, a screen reader, input methods for other languages.

Editing a line on the page (2026-10-22) was checked the same way, on a made-up page with white Helvetica Bold on a
blue bar, black Times and red Arial (not embedded): a word double-clicked with the read tool and typed over, and a
click with the Text tool and typing, at 1200, 1000, 800 and 400 points wide and in a 420-point-high window, in the
light, dark and system themes, with the page colours Normal, Dark and Sepia, at scales 1, 1.5 and 2. Each line was
drawn in its look-alike font, size and colour (the page colours applied), over the old line hidden in the colour
around it (the bar's blue, the paper), with a caret that shows on dark paper. Not tried: Windows and macOS, a real
HiDPI screen, a screen reader, an installed font being picked up for a kept line, turned lines.

Notes and form lists without Done (2026-10-22) were checked the same way, on a made-up form with a list allowing
several choices: the list opened, a second choice clicked with the list staying open, Enter and Esc closing it with
both choices kept; a note typed and kept by a click outside it and by Esc, the read tool coming back with it
selected; at 1200 and 400 points wide, in the light, dark and system themes, at scales 1, 1.5 and 2. Not tried:
Windows and macOS, a real HiDPI screen, a screen reader.

The inspector's page and document settings applying as they are set (2026-10-22) were checked the same way, on a
made-up two-page document: the Watermark, Headers, footers, Bates, Crop and Properties sections at 1200, 800 and
400 points wide, in the light, dark and system themes, at scales 1, 1.5 and 2. The watermark and footer previews
showed on the pages; after Add to pages the written watermark covered its preview to within 1.5 pixels at a
110-pixel size (the difference is the renderer's stand-in Helvetica against Liberation Sans); the crop box was
dragged down 87 points, the cut-off part dimmed as it moved, and Esc showed the pages cropped. Not tried: Windows
and macOS, a real HiDPI screen, a screen reader, a picture watermark's preview by screenshot (its decoding and
placement are tested), a turned page's previews by screenshot (their layout is tested).

Every dialog in its final look (2026-10-22) was checked the same way, on a made-up document with a made-up digital
ID, a trusted certificate and a stand-in printer list (one queue name 120 characters long; nothing printed): each
at 1200×800 in the light and dark themes (some in the system theme), the ones reached from the keyboard also at
400×800, 1200×300 and 400×300 and at scales 1.5 and 2; Recover unsaved changes after ending the app with
`kill -9` once a recovery copy was made, wide, narrow and dark; and the close confirmation. Not shot: Import a
digital ID (it opens from the system's file chooser), Print and Save selected text, the signed and repaired save
questions, and busy states (their layout is tested). Not tried: Windows and macOS, a real HiDPI screen, a screen
reader.

Each feature page says what was not tested, under "How this was tested".

## Acceptance checks

The automated tests cover the library and the command line. These checks cover what they can't: the desktop app used
end to end, files opened in other readers, and other platforms. They are run by hand on public or made-up files,
before a release and after a change to the area a row names. The Status column holds each row's last result: Pass,
Fail, Blocked or Not run, with the date and platform. "Earlier run" means the row passed when its feature was built,
before this table had a Status column, and wasn't run again on the date given.

| Workflow | Already covered | Check by hand | Status |
| --- | --- | --- | --- |
| Open, move around, search, select, copy | Text reading, search and selection rules (`edit.rs`, the app's unit tests) | Open a text PDF, a scan with recognised text and a Chinese, Japanese or Korean file by drag and drop, Open recent and the command line; go to a page; search a phrase that crosses a line; select across a page break and paste into a text editor | Pass with a known limit, 2026-10-18, Linux (KDE, Wayland): dropping a file on the app does nothing on Wayland (the windowing library has no file drops there; documented in gui.md with a workaround). The rest passed. Noted, not a failure: a phrase that runs across a page break isn't found by search. |
| Edit, undo and redo, save, reopen | Content editing (`edit.rs`), saving (`save.rs`) | Edit a line of text, move a picture, delete a shape; undo everything, redo everything; save, reopen, and compare the page in another reader | Pass, 2026-10-18, Linux. 2026-10-19: editing a line on a page printed from a web browser, which used to turn the line upside down and shrink it, checked after the fix, Linux |
| Comments and markup | `annots.rs` | Add one of each kind and a reply, save, reopen in LimarePDF, Firefox and Acrobat Reader: places, colours, text and replies match | Pass (earlier run), Linux |
| Forms | `form_fill.rs` (renders), poppler and Ghostscript | Fill every kind of field, save, reopen; open in Acrobat Reader: values and looks match; flatten and check again | Pass (earlier run), Linux |
| Pages: merge, split, reorder, rotate, crop, export | `integration.rs`, `export.rs`, the command line | Merge two files, split one, drag pages into a new order, turn and crop one; save and reopen; export pages as images and text | Pass, 2026-10-18, Linux |
| Printing | `print.rs`, with a stand-in for `lp` | Print a file with mixed page sizes and a turned page: size, orientation and placement on paper; comments and form values printed | Pass, 2026-10-18, Linux, HP LaserJet P1606dn through CUPS, after three fixes: pages printed blurred until jobs named the paper size; comments printed upside down on a page whose content leaves the drawing state flipped (glm manual, page 1); filled form fields printed empty because CUPS's `pdftopdf` drops fields. Now A4 pages fit whole on Letter paper, a turned (landscape) page prints upright filling the portrait sheet, and comments and form values print where they belong. |
| Digital signatures | `signatures.rs`, `signing.rs`, `signature_changes.rs`, pdfsig and OpenSSL | Check a signed sample; sign, certify, add a comment and save; reopen and check; then the Acrobat round trips below | Pass (earlier run), Linux |
| Redaction, removing hidden information | `redact.rs`, `sanitise.rs`, the after-check | Redact a phrase and part of a picture, save, then search the output with `pdftotext` and another reader's search; remove hidden information and check `pdfinfo -meta` | Pass (earlier run), Linux |
| Damaged files | `repair.rs`, the corpus | Open a damaged file: the notice shows; Save asks; Save as writes a new file; the original's checksum is unchanged | Pass (earlier run), Linux |
| Errors | `guard()` and `run_command` unit tests, a tab's error page | Open a cut-short file and a file with a broken page: a message, the app stays open, other tabs keep their unsaved changes | Pass, 2026-10-18, Linux |
| Crash recovery | `recovery/tests.rs`, `app/recover.rs` (on a model disk) | Open three tabs: one changed and saved, one changed with a file, one untitled and changed (Make a PDF from images); wait over a minute; end the app with `kill -9` (Windows: Task Manager, End task). On restart two are offered: restore one (it opens as "*name* (recovered)", unsaved, with the notice) and discard the other. The original files' checksums are unchanged. Quit normally: the recovery folder is empty | Pass, 2026-10-19, Linux |

Printing can be checked partly without paper: CUPS's own processing of what LimarePDF sends can be run with
`cupsfilter`, using the printer's description file, which CUPS serves locally. To capture what LimarePDF sends,
put a stand-in `lp` first on the `PATH` that saves its input, and print with `limarepdf-cli print` (or start the
app from that shell):

```
mkdir -p /tmp/fakelp
printf '#!/bin/sh\nprintf "%%s\\n" "$@" > /tmp/fakelp/args.txt\ncat > /tmp/fakelp/job.pdf\necho "request id is fake-1"\n' > /tmp/fakelp/lp
chmod +x /tmp/fakelp/lp
PATH=/tmp/fakelp:$PATH limarepdf-cli print --pages 1 file.pdf
curl -o printer.ppd http://localhost:631/printers/<queue>.ppd
cupsfilter -p printer.ppd -i application/pdf -m application/vnd.cups-pdf /tmp/fakelp/job.pdf > after.pdf
```

`args.txt` holds the job's options, one per line (`-o media=` among them).

`after.pdf` is the page as CUPS passes it on to the printer's driver; open it, or search it with `pdftotext`.
This is how the dropped form fields were found.

## Round trips with Adobe Acrobat

From the digital signatures design. Each row is done in the order given, on Windows. "Reader" rows need only the free
Acrobat Reader; "Acrobat" rows need Acrobat Pro or Standard. Results go into
[Compatibility](signatures.md#compatibility); a failure is fixed or written down as a limit.

| Check | Needs | Result |
| --- | --- | --- |
| A LimarePDF-signed file in Acrobat: valid, signer unknown | Reader | Done: shown as a self-signed signature, details in place |
| The same after trusting LimarePDF's exported certificate (`.cer`) | Acrobat | Not run (Reader asks for an upgrade) |
| A comment added in Acrobat to a LimarePDF-signed file; checked in LimarePDF and Acrobat: valid, changes allowed | Reader | Not run |
| An empty field signed in Acrobat on a file LimarePDF signed first; both signatures checked in LimarePDF | Reader | Not run |
| A LimarePDF-certified file in Acrobat: certification level shown; a comment and form filling allowed or refused as the level says | Reader | Not run |
| Locked fields from a LimarePDF signature: Acrobat refuses to change them | Reader | Not run |
| A LimarePDF-signed file changed in a page's content by a tamper tool, in Acrobat: changed after signing | Reader | Not run |
| An Acrobat-signed and an Acrobat-certified file in LimarePDF: the same verdicts as Acrobat, apart from trust | Acrobat | Not run |
| LimarePDF fills a field, adds a comment, adds a second signature to an Acrobat-signed file; Acrobat still finds the first signature valid with allowed changes | Acrobat | Not run |
| LimarePDF changes a page of an Acrobat-signed file after asking; Acrobat reports a change that isn't allowed | Acrobat | Not run |
| A LimarePDF ID exported as `.p12` imported into Acrobat and used to sign; an Acrobat ID imported into LimarePDF and used to sign | Acrobat | Not run |

## Platform smoke checks

A short pass on each platform after installing a release build. Continuous integration builds and tests every
platform but never runs the app, so nothing here counts as checked from CI. "Not recorded" means it has likely
happened in everyday use but no result was written down.

| Check | Linux | Windows | macOS |
| --- | --- | --- | --- |
| Install from the package or installer, start from the menu | apt and dnf repositories checked (0.5) | MSI installed and started by hand (0.5) | Not run |
| Open a PDF from the file manager (double-click, Open with) | Not recorded | Not run | Not run (Finder's Open with) |
| Open a PDF passed on the command line, with `--page` | Not recorded | Not run | Not run |
| Edit, save, reopen | Used by hand | Not run | Not run |
| File dialogs (open, save as, choose a folder) | Used by hand | Not run | Not run |
| Print to a real printer | Printed on paper (HP LaserJet P1606dn through CUPS, 2026-10-18; the "Printing" acceptance check) | Print dialog only; nothing printed on paper | Not run |
| Keyboard shortcuts (Ctrl, or Cmd on macOS) | Used by hand | Not run | Not run |
| HiDPI screen (scale 2) | Not run | Not run | Not run |
| Screen reader (Orca, Narrator, VoiceOver): the bars, panels, each dialog, filling in a form; what is said and where the keyboard goes | Not run (names checked by the app's tests only) | Not run | Not run |
| Dialogs: open each, Tab through it, Esc; in a narrow and a short window | Screenshots under a virtual X server (2026-10-22), not clicked through on a desktop | Not run | Not run |
| Window size remembered: resize, close, reopen; maximize, close, reopen, restore | KDE Wayland (2026-10-19): size and maximized kept; restoring a window that started maximized puts it at the top-left (KDE's placement) | Not run | Not run |
| Name and icon in the taskbar, dock and Open with | Not recorded | Icon checked by hand (0.5) | Not run |
| A damaged file and a file that fails to draw: message, no crash | Not recorded | Not run | Not run |
| A page whose drawing asks for gigabytes at once (PDFBox 2363, below): drawn, or a message, without closing | Drawn (2026-10-20; the 7.6 GB asked for is never used, so Linux doesn't need it) | Not run | Not run |

**The large memory request, on Windows.** On PDFBox 2363 (`tests/data/render/pdfbox/2363.pdf` after
`tools/fetch-render-samples.sh`; copy it over from a Linux machine), hayro asks for one block of 7.6 GB to fill a
single shape at 96 dpi (4.3 GB at 72 dpi), whatever the band size. Linux and macOS hand out such memory only as it
is used, and this block is barely used, so the page draws. Windows reserves the whole block at once against RAM plus
the page file, so on a machine with less than that free the request may fail, and a failed request closes the
program at once, without the error page a crash in drawing gets and without saving recovery copies first. To check,
on a Windows machine with 16 GB of memory or less: open another document and make a change (so there is unsaved
work), then open 2363 at 100% and zoom to 150% and 200%; then run `limarepdf-cli render 2363.pdf --dpi 96 --out-dir out`
(at 150 dpi the renderer stops with an internal error on every platform, which the CLI reports plainly). Record whether the page draws, how high Task Manager's "Committed" figure goes, and whether the app
or the CLI closes or prints `memory allocation of ... failed`. If it closes, D12 in the build plan becomes P0 (a
crash that loses unsaved work), and LimarePDF needs a way around hayro's request until hayro changes.

On Linux, HiDPI can be tried without such a screen: run the app under a virtual X server with
`WINIT_X11_SCALE_FACTOR=2` and take screenshots, as for other screenshots. A real HiDPI screen is still needed for
sharpness.

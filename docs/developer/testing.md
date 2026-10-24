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
| `crates/limarepdf-core/tests/render_coverage.rs` | Rendering, pixel by pixel; see [Rendering](#rendering) below |
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
| `crates/limarepdf-core/tests/unicode_text.rs` | Text beyond Western European in every writer, read back after saving: new lines in each family, edited and invisible lines, text boxes and fill-and-sign text (drawn, grown to fit, flattened), form values in single-line, multi-line and list fields (the field's own font kept in `/DA`, flattening no longer refusing), headers, footers and Bates numbers sharing one font across pages, watermarks, stamp wording and names (a name the font can't show left off), selected text as a plain page; text the standard fonts can write embedding nothing; other scripts refused with nothing changed. Opt-in: `-- --ignored cross_check` has poppler (`pdftotext`, `pdffonts`) and Ghostscript read every writer's text from one file, list its fonts as embedded subsets with text maps, and draw it without a message |
| `crates/limarepdf-core/tests/line_styles.rs` | Styles within a line, on made-up pages: a line with a bold word read as one object in three parts (one-style lines read as before), two font objects or subsets of one font looking alike, raised and lowered parts keeping their place, a gap read as a space kept; editing one part keeping the others exactly as they were (their shows and spacing), a restyled word reading back as part of its line, a one-style line changed in words only changed in place, characters a part can't write refused, parts that don't make up the line refused; the text after a line in parts staying where it was, also once the line is moved; a line in parts searched, selected, moved and redacted as one; new text added in parts |
| `crates/limarepdf-core/tests/digital_ids.rs` | Digital IDs: importing PKCS#12 files in every shape other apps write (OpenSSL-made with `crates/limarepdf-core/tools/make-signature-keys.sh ids`: OpenSSL 3 and 1 defaults, Windows' 3DES, AES-128, PBES2 with 3DES, SHA-512 and PBMAC1 integrity checks, unencrypted parts, P-256 and P-384 keys, non-ASCII and empty passwords, an ID with its authority, an expired one); wrong passwords; files refused with the reason (no integrity check, keys too short or too long, a key not for signing, an unsupported curve, no key, a certificate for another key); every truncation and many changed bytes never panic; making self-signed IDs (certificate contents, five years, file permissions); several IDs, finding them and the default; the folder listing and files put there by hand; exporting and importing elsewhere. Unit tests in `pkcs12.rs`: the PKCS#12 key derivation against published test values, BER as Windows writes it, costly files refused before the work; in `ids.rs`, IDs allowed only RSA-PSS signatures (by the certificate, or by the key too) refused. Opt-in: `-- --ignored` has OpenSSL read LimarePDF's files and take out the key, which then signs a PDF LimarePDF checks and trusts |
| `crates/limarepdf-core/tests/signature_changes.rs` | Changes after signing: form filling, further signatures, comments, metadata and validation data allowed; certification levels and field locks applied; page content, pages, signatures, new fields, actions and links refused; the shadow attacks (hide by freeing or re-pointing objects, page content posing as a comment, a missing object filled in), a change undone before the next signature, malformed and damaged appended data never passing as unchanged; the signed version being exactly the file as signed |
| `crates/limarepdf-core/tests/signature_trust.rs` | Trust: signers trusted directly, and chains to a trusted authority (OpenSSL-made with `crates/limarepdf-core/tools/make-signature-keys.sh trust`), including chains carried in the document's validation data; issuers that may not issue, path lengths, expired links, name constraints, short keys, SHA-1, unknown critical extensions, key usages; certificate policies (required, mapped, any policy inhibited); a forged issuer and look-alike certificates named like a trusted one; loops and over-long chains; timestamp authorities, a document timestamp's authority reached through the intermediate it carries; hand-made certificates for shapes OpenSSL won't make (a repeated extension, mismatched algorithms, a changed certificate, an authority whose key is allowed only RSA-PSS signatures, linking only with one it allows); the trust file (round trip, private permissions, unusable entries kept but never trusted) and certificate files (DER, PEM, `.p7c`). Opt-in: `-- --ignored` compares the chain verdicts with `openssl verify` |
| `crates/limarepdf-core/tests/signing.rs` | Signing with digital IDs (the `ids` test files): RSA, P-256, P-384 and a CA-issued ID with its chain, in new boxes and empty fields, with reasons, locations and pictures; second and third signatures leaving earlier ones valid; certification at each level and what it then allows; field locks making fields read-only and locked fields refused; expired IDs, encrypted documents, bad boxes and wrong fields refused; edits signed along with the signature (a field filled; a page turned only with consent); fill-and-sign items drawn in or locked; forms asking readers to draw fields; classic tables, cross-reference and object streams, damaged files, turned pages. Saving signed documents (`append_edits`): nothing added without changes or after an undo; a comment and a filled field allowed, and saved again on top; disallowed changes named once; what each certification level lets a save add; fields drawn for display only; a deleted comment not written again; passwords, encrypted and damaged files and a removed signature refused; numbers written back without their decimals (`300.0` as `300`) not taken as a change; removing the document information (a change to the trailer alone) added after what was signed, with the earlier bytes kept and the signature valid; a signer's name beyond Western European shown in an embedded font, each signature with a font of its own so the first stays valid, and text boxes in other scripts saved twice after signing as allowed comments. Unit tests in `incremental.rs` (only changed objects written, nothing appended without changes, a change to the trailer alone (`/Info` removed, a new `/ID`) appended and read back as edited, deleted objects read as gone, anything that reads back differently refused, hybrid files recognised) and `sig_sign.rs` (dates, text fitting, names the font can't show, finding only the signature's own placeholders). Opt-in: `-- --ignored cross_check` has `pdfsig` and `openssl cms -verify` check LimarePDF's signatures, one of them on a document saved with changes after signing |
| `crates/limarepdf-core/tests/signatures.rs` | Checking digital signatures: every supported algorithm, digest and signature format (made in the test with OpenSSL-made keys, `tests/sigkit/`), timestamps, certification and field locks, warnings; the published attack classes (universal forgery, signature wrapping, incremental saving, copied and spoofed signatures, certificate swaps, tampered CMS); RSA keys allowed only RSA-PSS signatures (RFC 4055) against signatures that keep or break the limits, including timestamps and `adbe.x509.rsa_sha1`; truncated and corrupted files never pass; a file with more signed versions than LimarePDF opens; pyHanko and node-signpdf samples with the result each should get, and the changes each should show. Opt-in: `-- --ignored dss` checks the EU DSS samples (fetch them first with `crates/limarepdf-core/tools/fetch-dss-samples.sh`), and `-- --ignored cross_check` compares LimarePDF's results with poppler's `pdfsig` and OpenSSL, and the RSA-PSS cases with `openssl cms -verify` |
| Unit tests in `crates/limarepdf-core/src` | Page ranges, document operations, matrices, overlays, print placement, the ToUnicode reader, standard font widths, embedded fonts (`embed.rs`: the bundled Liberation faces, stored compressed, unpacked byte for byte with their released checksums, every script read back, subsets checked glyph by glyph against the face, widths, growing in place with earlier text unchanged, fonts changed by others left alone; installed fonts from the test fonts: an OpenType CFF font written as such and checked glyph by glyph, variable fonts set at their weight (widths and bolder strokes), growing after reopening, what a font lacks refused naming it; opt-in `-- --ignored cross_check` with poppler and Ghostscript for both), the installed families offered (`typefaces.rs`: faces and weights found, Liberation and the standard fonts' names not offered again, fonts named in PDFs found by PostScript name, embedding permissions), typed signatures naming the styles that can show a name, laying out form fields (wrapping, auto size, alignment, comb, lists), replacing files (temporary files created new, never a stale file or a symbolic link, private while written; the folder synced once after the rename, through a test seam; a failed folder sync reported as saved but not confirmed, with the new file in place, for documents and private files; a file system that can't sync folders counting as synced), free disk space |
| `crates/limarepdf-gui/src/recovery/tests.rs` | The crash recovery store: copies written, replaced, found and read back; an app crash and a power cut at each step of writing a copy (power cuts on a model disk where any unsynced change may be lost), with a wrong order shown to fail; a failure at each step; withdrawing copies before a redaction, with every failure case and races against a write in progress, and a crashed session's copy with each outcome (one whose folder sync failed staying unconfirmed at every try, with its folder and leftover files kept track of, until a sync succeeds; no power cut bringing it back afterwards); turning recovery off; damaged and expired copies; a session another running process holds; a session folder, or the recovery folder, that is a symbolic link or not a real folder of this user's left untouched at start (on the model disk and on the real disk, with an unrelated PDF in the folder it leads to); free space, permissions and a read-only folder; the recovery thread's order |
| `crates/limarepdf-gui/src/app/recover.rs`, `recovery/schedule.rs` | Recovery copies in the app, on the model disk: when a copy is due (a minute after the first change, coalesced, one write at a time, a change during a write, failures retried no faster, at once after a withdrawal); copies removed when a tab is saved or closed and at a normal exit; none for password-protected documents, pages taken from them, or a document their pages are appended to (the copy withdrawn first, the append stopping if it can't be); a password withdrawing the copy first; redaction and removing hidden information withdrawing the copy and writing a new one at once, which holds none of the redacted text, metadata or attachments (every object and stream searched); redoing a redaction or a removal of metadata or attachments after undoing it withdrawing the copy written in between, in an ordinary and a restored tab and for a second removal, with no crash or power cut able to offer the content afterwards, and a redo that can't withdraw leaving the document, its flags and both undo lists unchanged at every try; on a restored tab, each of the three withdrawing the copy it came from, or stopping if it can't, at every try while its folder sync fails; each of the three stopping, with the document unchanged and the file named, when an earlier copy could still be offered, and going ahead with a notice when it was only left on disk; turning recovery off always taking effect; recovery that can't start; a restored copy of a signed document (notice at once, no signature check, signatures listed as not checked, Save and Sign asking first); a damaged copy offered, never restored |
| `crates/limarepdf-cli/src/main.rs`, `crates/limarepdf-cli/tests/panics.rs` | The command-line tool's panic guard: a panic becomes a plain error. Opt-in (`-- --ignored`, needs the render corpus): `render` on PDFBox 2363 at 150 dpi, which makes the renderer panic, exits with status 1 and a plain message, without Rust's panic message |
| `crates/limarepdf-cli/tests/escapes.rs` | The command line's listings on a made-up file with an escape code, the one-byte CSI, a bidi override and a line break in its title, a bookmark, a comment's author, a field name, a page label, a font name and the page text: none reaches the output, and the visible text does |
| `crates/limarepdf-cli/tests/verify.rs` | `verify --signed-version` never changes its input: the input's own name, as given or as another path, and a symbolic link to it are refused; a hard link is refused on Unix, and elsewhere gets a file of its own; a separate output holds exactly the signed bytes |
| `crates/limarepdf-cli/tests/input_kept.rs` | `rotate`, `set-info`, `sanitise` and `merge` refuse an output that is their input (its own name, another path to it, a symbolic link, and on Unix a hard link) with the input unchanged; another output works |
| `crates/limarepdf-cli/tests/output.rs` | Output that can't be written: `info`, `text` and `form list --json` to a full disk (`/dev/full`, Linux only) exit 1 with a plain message; `set-info` says its file was saved, and `remove-blank`, whose report comes first, saves nothing; a reader that went away stops quietly, with status 0, or 1 for `verify --require-trusted` on a file with no signatures; ordinary output as before |
| `crates/limarepdf-cli/tests/notes.rs` | The command line's note on stderr for a file other PDF apps may show differently (data after its end), and none for a plain file |
| `crates/limarepdf-cli/tests/objects.rs` | `objects` listing a line in more than one style once, with its first part's font and size, marked "mixed" |
| `crates/limarepdf-cli/tests/print.rs` | `print` refusing, naming the field, when a filled field would print empty; stand-ins for `lp` and `lpoptions` come first on the path, so no printer is ever used, and no job reaches them (Unix only) |
| Unit tests in `crates/limarepdf-gui/src` | The command list (unique entries and shortcuts, menus), selection rules, page ranges, moving pages, fitting the zoom, document notices, recent files, settings and saved signatures (each replaced in one step, so a hard link to the old file keeps it), the security form, form fields (Tab order, what a click does, keeping typed text), typing undo kept to its box (driven through the window: Ctrl+Z in a field never bringing in another document's field of the same name, a field's second widget or a reopened document starting with nothing to undo, a second note never undoing into the first, new or reopened; Ctrl+Z, Ctrl+Shift+Z and Ctrl+Y in a form field, a new note, a new text box, Add text, the inspector's comment, a document property and a watermark's text acting on that box's typing only, never the document, with and without document history, and the document's Undo and Redo again once out of the box; values read back from the saved bytes), typing kept before commands (Add text then Rotate clockwise from the palette, the text kept as its own Undo step; a form field, a new note, a new text box, Add text, a line typed over, the inspector's comment and a document property each kept before a page command and through its Undo; toolbar Undo and Redo with a new note open; a thumbnail reorder and opening Crop; Remove blank pages taking its copy after the typing, and a result that arrives after typing started discarded with the typing kept; Ctrl+P straight from a field printing the typed value through stand-ins for `lpstat` and `lp` in a child copy of the tests, and a refused value stopping Print with the typing and keyboard kept), digital signature results (colour, sentence and banner for valid, unknown, trusted, warned, broken, changed and certified samples; names from the file cleaned), signing from the signing window (a valid signed copy written, a wrong password, earlier signers named before disallowed changes, signing after consent), the signed copy taking an unchanged tab's place but opening in a new tab when a password was set or text typed meanwhile, and one written but not confirmed on disk taking it too, with the message saying so and the document left marked unsaved, signing refused while a password waits to be saved, passwords overwritten, Print asking first (and sending nothing on Cancel) when a field would print empty, the notice on a page with more annotations than are shown; Add text typed on the page (driven through the window's AccessKit tree: the editor's name, keyboard and baseline on the click, typing and Enter, a refused character and its note, Ctrl+B before typing, the inspector's Bold at the caret styling what is typed next and giving the keyboard back, dragging the outline, a click away adding every line as one undo step (a line in two styles written in two parts, the next text carrying on in the last style), Esc keeping it, nothing typed adding nothing, another tool kept, a click on page text editing that line), a line edited on the page (opened by a double-click with the word selected, or a click with the Text tool; the editor's name, keyboard and place on the baseline, the caret placed by the line as drawn, the line hidden in the colour around it, a refused character, Esc keeping the change on the same baseline, one Undo taking it back; the font it is drawn in, as core will write it; turned and invisible lines in a plain box), styles within a line (`styled.rs`: typed text taking the style around it, Word's rules for Bold, Italic, font, size and colour on a selection, the word at the caret or what is typed next, blanks for mixed values, parts for core, a read line keeping its parts' fonts until changed, new text written a line at a time spaced by its sizes, undo and redo of typing and styles grouped as editors group them; in the app: Ctrl+B on a double-clicked word bolding it alone, the inspector's font, size and colour blank for a mixed selection, Ctrl+Z, Ctrl+Y and Ctrl+Shift+Z in the line and in the inspector's text box never reaching the document, a word in a turned line styled alone; parts of a line drawn on one baseline, raised ones above it), a drag with the Text tool selecting text, the colour around a box found by the page worker (the most common in the ring, in the page colours), typed text in the page colours, the fonts for text being typed (each standard font's Liberation face, loaded the first time it is used and not before, the order fonts are tried, installed fonts loaded, Liberation Sans as wide as Helvetica, an installed family drawn from its file at its weight), the font list (standard fonts first, search, a family chosen in a style it has; picked by searching and Enter in the app, Bold and Italic off for faces it lacks, typed text written in it, a refused character's note naming it), Ctrl+B and Ctrl+I only to faces a family has, a line in an installed family shown as it |

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

Styles within a line (2026-10-23) were checked the same way, on a made-up page with a line in four styles
(Helvetica, a red Helvetica Bold word, larger blue Times Italic words, a raised 11-point "2") and the same page
turned a quarter: a word double-clicked, then the whole line selected (the inspector's font list and size box blank,
the colour button showing the three colours), typing at the end and Ctrl+Z, at 1000 points wide in the light
theme, with the page colours Normal and Dark, at scales 1 and 2; the turned line's box (its styles at their size on
the page, kept in view) with Ctrl+I on a word and the line written with it; and Add text with a bold word and a
second line. Each part was drawn in its font, size and colour on the line's baseline, the raised part above it.
Joining lines across styles was checked over the render corpus (1,089 files, the first 15 pages of each, one file
per process): 18% fewer text lines, none more, and a sample of 45 joined lines looked at by eye (reference lists,
species names, maths, bullets, sub- and superscripts, CJK and Cyrillic lines) with no bad join. Not tried: Windows and macOS, a real HiDPI screen, a screen reader,
installed fonts in a mixed line, input methods for other languages.

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

## How each feature was checked

The record for each feature, moved here from the user pages, which keep a short summary under "How this was
tested" or "What has been checked".

### Rendering

`crates/limarepdf-core/tests/render_coverage.rs` checks rendering pixel by pixel on small made-up pages. Drawn correctly:

- all 16 blend modes;
- isolated transparency groups, and a blend mode applied to a whole group;
- colour-key image masks (`/Mask [..]`);
- annotations that have an appearance stream (`/AP`): Text, Highlight, Link, Stamp, FreeText and Widget were tested,
  including the Widget `/AS` state and the Hidden flag;
- annotations with the NoView flag are hidden, in the desktop app and `limarepdf-cli render` (LimarePDF marks them Hidden before
  drawing, since hayro ignores NoView);
- annotations without an appearance stream: hayro draws nothing for them, so LimarePDF draws its own for Highlight,
  Underline, StrikeOut, Squiggly, Text (notes, with the seven standard icons), FreeText (with callout lines),
  Square, Circle, Line (with leader lines), Polygon, PolyLine, Ink, Stamp (the stamp's name in a coloured frame),
  Caret and FileAttachment (the four standard icons), with cloudy borders on Square, Circle, Polygon and FreeText,
  in the copy it renders and prints, not in the file (`crates/limarepdf-core/tests/annots.rs`);
- password-protected files with all four ciphers.

#### Compared with poppler

An opt-in check draws about 1,100 public test PDFs (the pdf.js and PDFBox test files hayro is tested on) with
LimarePDF and with poppler, and lists the pages that differ beyond anti-aliasing, with pictures side by side
([how to run it](#automated-tests)). These test files were collected from bug reports, so they are much harder than
everyday documents. The first run (2026-10-17, hayro 0.8.0, poppler 26.08) drew 1,311 pages from 1,089 files:

- 1,079 pages match apart from anti-aliasing (0.5% of pixels or fewer different);
- 220 pages differ by more than that, 23 of them by more than 10%. Many of the worst are poppler's mistakes, such as
  a grey background, garbled scans or striped photos where LimarePDF's page looks right. Differences that are
  LimarePDF's include very thin lines drawn light grey where poppler draws them solid, and fonts that aren't
  embedded (on 39 pages LimarePDF reported a font it couldn't draw; with installed fonts and stand-ins, none on a
  machine with the Noto CJK and Liberation fonts);
- 7 damaged files that poppler repairs couldn't be opened by LimarePDF, and 2 failed in both. Since LimarePDF
  [repairs damaged files](../rendering.md#damaged-files) (2026-10-18), all 7 open and match poppler to within 2% of
  their pixels, one that poppler can't draw opens too, and one fails in both. A few pages poppler can't draw are
  drawn by LimarePDF.

None of the files crashed LimarePDF or took longer than a minute to draw.

The latest run (2026-10-18, with installed fonts and damaged-file repair) drew 1,312 pages from the same files and
listed 228. More pages are listed than in the first run because files that couldn't be opened before are now drawn
and compared, and stand-in fonts differ from poppler's choices. Not every listed page has been looked at by eye; the
differences found to be LimarePDF's are the gaps under [Known to be wrong](../rendering.md#known-to-be-wrong).

### Export and printing

- Export: `crates/limarepdf-core/tests/export.rs` (sizes, content, re-import, names) and by running the CLI.
- Printing: `crates/limarepdf-core/tests/print.rs`, with a stand-in for `lp` (its arguments, the document it receives, and
  errors), and against a real CUPS with no printers (error messages). The Windows page placement and rotation are
  unit-tested.
- On paper on Linux, with an HP LaserJet P1606dn through CUPS (October 2026): pages, mixed A4 and Letter pages, a
  turned page, comments and filled form fields print correctly. Getting there took three fixes, each for a way
  CUPS handles PDFs on that setup that other PDFs sent with `lp` suffer from too: the job must name the paper
  size, or every page prints blurred; comments are added after the page's content without resetting the drawing
  state, so they came out upside down on pages that leave it turned; and form fields are removed without being
  drawn. The last two were reproduced without a printer through CUPS's own `cupsfilter`
  ([Acceptance checks](#acceptance-checks)).
- On Windows only the print dialog has been tried; nothing has been printed on paper. Nothing has been printed on
  macOS.

### Redaction and hidden information

- Redaction: `crates/limarepdf-core/tests/redact.rs` (made-up files: text with kerning and spacing, turned pages, hidden
  text, alternative text, drawings, pictures with soft masks, JPEG, inline images, shared pictures and forms,
  unreadable pictures, annotations, redaction marks), and `limarepdf-cli redact` on the PDFs in `/usr/share/doc` (common words
  removed throughout, renders checked, nothing found by search afterwards) and on a scanned page in three forms
  (plain, CCITT and JPEG) with a recognised text layer. Since 2026-10-20 (build plan, C2) the tests search the saved
  output as written, the same way as for removing hidden information below, and check that what touched a mark
  (comments with their pop-ups and replies, form fields and their drawings, alternative text in the structure tree,
  thumbnails, private data) is gone. Opt-in cross-checks read the output with poppler (`pdftotext`, and
  `pdfimages` for a blacked-out picture) and Ghostscript: `cargo test -p limarepdf-core --test redact -- --ignored
  cross_check`. One of them redacts the Magick++ tutorial from ImageMagick's documentation, a real file whose
  fonts are subsets, so its text can't be found in the file's bytes and only a reader can show it's gone. Another
  places an area over a character of a font whose character map leaves codes out, where poppler shows it, and
  checks with `pdftotext -bbox` that poppler then reads nothing there and the characters around it in their places.
  Where those characters are was taken from poppler first (since the third outside audit, 2026-10-21), so the
  tests don't only agree with LimarePDF's own reading.
- The app's part (the Redact tool, marking selected text and search results, the notice, Apply and its report, the
  save warning) was used by hand on Linux.
- Removing hidden information: `crates/limarepdf-core/tests/sanitise.rs` (made-up files for each kind) and `limarepdf-cli sanitise`
  on the PDFs in `/usr/share/doc`, which have metadata and bookmarks but no scripts, attachments or layers. Its dialog
  was not clicked through. Since the second outside audit (2026-10-19), each kind's saved output is searched as
  written, before anything repairs or prunes it: every string form of the removed text in the raw file and in every
  stream in it, decoded, whether still used or not, with the form fields, actions and layers checked one by one
  (`tests/common/saved.rs`). Opt-in cross-checks read the output with poppler (`pdftotext`, `pdfinfo -js`) and
  Ghostscript (`txtwrite`, which draws form fields): `cargo test -p limarepdf-core --test sanitise -- --ignored
  cross_check`. poppler lists only a field's own scripts, not scripts that follow other actions, so for those the
  search is the evidence. A refused removal is checked to leave the document, its undo history and its recovery
  copies as they were.
- Marks made by LimarePDF were not yet opened in Acrobat or other apps, nor marks made by them applied in LimarePDF.

### Scanning and OCR

- Import: `crates/limarepdf-core/tests/import.rs`, and by hand with ImageMagick-made files, including CMYK JPEG and 300 dpi
  G4 TIFF.
- Optimising: `crates/limarepdf-core/tests/scan.rs`, and by hand on ImageMagick-made 300 dpi scans (5.3 MB down to 0.5 MB, a
  1.2° tilt corrected). The black-and-white or greyscale choice is a heuristic not yet tried on real scanner output.
- OCR: `crates/limarepdf-core/tests/ocr.rs`, the measurements in
  [Scans and OCR](../scanning-and-ocr.md#accuracy-and-speed), and one real scan. Correcting recognised text was
  tested in code.
- The GUI's inspector sections for these were not clicked through in testing.

### Digital signatures

Checked against third-party signed files with known results: the pyHanko and node-signpdf samples in LimarePDF's tests,
and the EU DSS project's validation samples (signatures from Adobe products, Italian, Spanish and German signers,
PAdES with timestamps, and published attacks). Each file's expected result and why is recorded in
`crates/limarepdf-core/tests/signatures.rs`. LimarePDF's own test signatures are also checked with poppler's `pdfsig` and
OpenSSL, which agree. Signatures by RSA keys whose certificates allow them only for RSA-PSS were checked with
`openssl cms -verify` (without judging trust): OpenSSL and LimarePDF agree on all 12 cases, among them PKCS#1 v1.5
signatures by such keys, a different digest or mask, a salt shorter, equal and longer than the key's minimum,
limits given as defaults or left out, and limits that can't be read. On a 13th, a limit to an undefined trailer
field, LimarePDF is stricter (see [Limitations](../signatures.md#limitations)).

Where LimarePDF and `pdfsig` differ, LimarePDF follows the PDF rules: it refuses byte ranges with gaps or uncovered parts that
`pdfsig` accepts, refuses two signers in one signature, and accepts signers named by key identifier, which `pdfsig`
calls invalid.

Changes after signing were checked on the same samples, among them files changed by Adobe Acrobat and Acrobat
Reader (a comment added, a second field signed), iText (a signature added) and PAdES tools (timestamps and
validation data), which LimarePDF reports as allowed, and the DSS attack samples (shadow hide and replace, a page added,
a signature appearance changed, a reason replaced), which LimarePDF refuses. One DSS sample whose later revisions point
to cross-reference sections that don't exist is refused as malformed, and its document timestamp, which covers those
damaged revisions, shows *can't check*, where DSS repairs the file and accepts it.
`pdfsig` doesn't judge changes after signing, so there is no cross-check for this part.

Trust was checked on chains made with OpenSSL for the purpose (good chains, and each way a chain can fail), whose
verdicts are cross-checked with `openssl verify` at the signing time with S/MIME signing checks and RFC 5280 policy
processing; they agree on 21 chains. LimarePDF is stricter than OpenSSL by design on four others: name constraints, a
1024-bit RSA authority, a SHA-1 link, and a policy mapping to "any policy" (which RFC 5280 forbids). On the
third-party samples, trusting the authority from a signature's own chain makes 14 of 16 CA-issued signers trusted,
including EU qualified-signature chains that require certificate policies; the other two have SHA-1-signed
certificates, which LimarePDF refuses.

Digital ID files were checked against OpenSSL both ways: LimarePDF imports files OpenSSL makes in each shape other apps
write (OpenSSL 3's default, OpenSSL 1's and Windows' 3DES and RC2 forms, AES-128, PBES2 with 3DES, SHA-512 and
PBMAC1 integrity checks, unencrypted parts, P-256 and P-384 keys, non-ASCII and empty passwords, an ID with its
authority), and Windows' BER encoding, made from those files. OpenSSL reads the files LimarePDF writes, both its own and
exported ones, takes out the same certificate and a key that signs a PDF LimarePDF then checks as valid and, once
trusted, trusted. `openssl x509` shows the extensions described under
[making a digital ID](../signatures.md#making-a-digital-id).

LimarePDF's own signatures were checked with `pdfsig` and OpenSSL (`openssl cms -verify` over the signed bytes): RSA-3072,
P-256, P-384 and a CA-issued ID with its chain, a second and third signature on a signed file, a certification, and
a signature in the field a certified document left empty. Both call every one valid, and `pdfsig` finds each
earlier signature valid too. Signed files with classic cross-reference tables, cross-reference streams and object
streams, and pages turned 90 degrees, were checked by LimarePDF.

Saving signed documents was checked on the 53 third-party samples LimarePDF can open with a signature in them. Saved
without changes, or after an undo, nothing is added. With a comment added, every signature checks as before, by
LimarePDF and by `pdfsig` alike, and LimarePDF reports the comment as allowed except where the document's certification or a
signer's lock forbids comments; with a page changed, LimarePDF reports a change not allowed. LimarePDF declines to add to five:
three damaged files, one hybrid file whose objects its PDF library would misread afterwards (see the limitations),
and one whose signature was already broken (its byte range ran past the end of the file). A document LimarePDF signed and
then saved with a comment and a filled-in field is in the `pdfsig` and OpenSSL cross-check above.

A document LimarePDF signed with a self-signed digital ID, opened in Adobe Acrobat Reader on Windows, shows its signature
as made with a self-signed certificate, with the signer's details in their places.

Not yet tested: signatures made in Adobe Acrobat by LimarePDF's testers, LimarePDF-made certifications in Acrobat, LimarePDF's
signatures in Acrobat beyond that first look, documents signed in Acrobat and saved with changes in LimarePDF (and the other way round), and digital ID files
exported from Acrobat or Windows, or LimarePDF's exported files imported there. Where LimarePDF is stricter than Acrobat (see
the limitations) is to be confirmed in those tests. The checks still to run are listed under
[Round trips with Adobe Acrobat](#round-trips-with-adobe-acrobat).

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
| Text beyond Western European | `unicode_text.rs`, `embed.rs`, poppler and Ghostscript (opt-in) | Add Polish, Greek and Cyrillic text in each font, a text box, a form value, a header, a watermark and a stamp, and sign with an ID whose name has such characters; try typing Chinese in Helvetica (the note at the caret asks for an installed font) and Hebrew (it says the character can't be added yet), and Chinese in a watermark (no installed font is offered); save, reopen in Acrobat Reader and Firefox: everything shows, search finds it, copy pastes it; print it on paper | Not run |
| Installed fonts | `typefaces.rs`, `embed.rs`, `unicode_text.rs`, the app's `font_list.rs` and `installed_fonts_are_chosen_from_the_list_and_written_in`, poppler and Ghostscript (opt-in) | Open the font list: the installed fonts after Helvetica, Times and Courier, each name in its own font, search narrowing it; add text in a TrueType, an OpenType CFF and a variable font (bold too), and type a character one lacks (the note names it); edit a line in one; save, reopen in Acrobat Reader and Firefox on a computer without those fonts: the same look, search finds the text; print it on paper. On Windows and macOS too | Not run |
| Styles within a line | `line_styles.rs`, the CLI's `objects.rs`, the app's `styled.rs` and its line and Add text tests | Open a file from another app with a bold or italic word in a line (a reference list, say): the line is one object; double-click it, make another word bold, change one word's size and colour, undo and redo with Ctrl+Z and Ctrl+Y, keep it; add text with a bold word and a second, larger line; save, reopen in Acrobat Reader and Firefox: the same look, search and copy read each line whole. On Windows and macOS too | Not run |
| Typing undo stays in its box | The app's `same_named_fields_in_two_documents_keep_their_typing_apart`, `a_field_shown_twice_starts_each_box_with_nothing_to_undo`, `a_second_note_on_a_page_does_not_undo_into_the_first`, `undo_in_a_form_field_stays_in_the_field`, `undo_keys_in_every_text_box_act_on_its_typing_only` | Open two forms with a field of the same name; type in one, switch tabs, press Ctrl+Z in the other's field: nothing changes. Type Name, Tab, type Notes, then Ctrl+Z, Ctrl+Shift+Z and Ctrl+Y: only Notes changes; Esc, then Ctrl+Z takes back Notes, then Name. Type in two notes on a page, the inspector's comment, a watermark's text and the document's title: the keys act on each box alone. Save, reopen, read the values with `limarepdf-cli form`. On Windows and macOS (Cmd+Z, Cmd+Shift+Z) too | Not run: deferred to the RC cycle |
| Commands keep what is being typed | The app's `new_text_is_kept_before_the_page_is_rotated`, `every_kind_of_typing_is_kept_before_a_page_command`, `undo_on_the_toolbar_takes_back_a_note_being_typed`, `typing_is_kept_before_the_tab_changes_or_redraws_its_pages`, `background_results_keep_what_is_being_typed`, and `print_right_after_typing_prints_the_typed_value` (the whole Print route, Ctrl+P to the job, in a copy of the tests with stand-ins for `lpstat` and `lp`; a refused value stops it) | Add text, type, then Rotate clockwise from the palette: the page turns with the text on; Undo turns it back with the text, Undo again takes the text. Type in a field, a note, a text box, a line and the inspector, then each of Delete page, Remove blank pages, Undo on the toolbar and drag a thumbnail. Type in a field and press Ctrl+P, then Print to a stand-in `lp` (above): the job holds the value; type Arabic and press Ctrl+P: Print doesn't open, the reason shows, the typing stays. Save, reopen and read the values with `limarepdf-cli form` and `pdftotext`. On Windows and macOS too | Not run: deferred to the RC cycle |
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
Acrobat Reader; "Acrobat" rows need Acrobat Pro or Standard. Results go into [Digital signatures](#digital-signatures)
above, summed up in the user page's [Compatibility](../signatures.md#compatibility); a failure is fixed or written
down as a limit.

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

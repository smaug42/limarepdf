# Testing and platforms

## Platforms

| Platform | Status |
| --- | --- |
| Linux x86_64 | Built and tested; the command line run on real PDFs; the desktop app used by hand and checked by screenshot under a virtual X server, including the narrow-window layout |
| Windows x86_64 | Built and tested by continuous integration (`.github/workflows/ci.yml`, GitHub Actions), which also packages all four platforms. Printing, the only platform-specific code, has been tried by hand and works |
| macOS (Intel and Apple Silicon) | Built and tested by continuous integration. Not run by hand |

## Automated tests

Run them with `cargo test --workspace`.

| Tests | What they cover |
| --- | --- |
| `crates/mpe-core/tests/integration.rs` | Overlays, encryption round trips with all four ciphers, search, crop margins on turned pages |
| `crates/mpe-core/tests/edit.rs` | Content editing: objects and text edits. Reading text: words, search boxes placed by glyph widths, ToUnicode maps, word gaps, text inside form XObjects, phrases across lines |
| `crates/mpe-core/tests/render_coverage.rs` | Rendering, pixel by pixel; see [Rendering](rendering.md) |
| `crates/mpe-core/tests/import.rs` | Importing images as pages |
| `crates/mpe-core/tests/scan.rs` | Optimising scanned pages |
| `crates/mpe-core/tests/ocr.rs` | OCR text layers, their placement, search and correction |
| `crates/mpe-core/tests/export.rs` | Exporting pages as images and text |
| `crates/mpe-core/tests/print.rs` | What is sent to the printer, with a stand-in for `lp` |
| `crates/mpe-core/tests/outline.rs` | Bookmarks: reading nesting, every kind of destination, broken and looping outlines; adding, renaming and removing them, with the open/closed counts kept right |
| `crates/mpe-core/tests/links.rs` | Reading links: every kind of target, links that can't be followed, placement on turned pages |
| `crates/mpe-core/tests/annots.rs` | Annotations: reading them, drawing each type that has no drawing (on screen only), removing them with replies and popups, flattening all or some pages; stamps placed, resized, faded, flattened and upright on turned pages; stamps with your own wording, and wording the font can't show refused; fill and sign: typed, drawn and picture signatures (paper made transparent and margins cropped), initials and marks drawn, labelled and sized, upright on turned pages, and Finish signing drawing in only those items |
| `crates/mpe-core/tests/forms.rs` | Reading form fields: every kind, inherited values, broken field trees, turned pages, XFA-only and hybrid forms; which signature fields are empty |
| `crates/mpe-core/tests/redact.rs` | Redaction: characters removed with the rest kept in place (spacing, kerning, turned pages, hidden text, alternative text), drawings removed or clipped, pictures blacked out (soft masks, JPEG kept as JPEG, inline images, shared pictures) or removed when unreadable, shared forms, annotations, the after-check, redaction marks kept, saved and applied |
| `crates/mpe-core/tests/sanitise.rs` | Removing hidden information: metadata and XMP, scripts (web links kept), attachments, hidden layers (marked content and a form in a hidden layer), content outside the crop box, finding without changing |
| `crates/mpe-core/tests/form_fill.rs` | Filling in forms: values, drawn appearances (checked by rendering, including a turned field), fonts that can't be used, refused values, passwords, reset, drawing missing appearances |
| `crates/mpe-core/tests/digital_ids.rs` | Digital IDs: importing PKCS#12 files in every shape other apps write (OpenSSL-made with `crates/mpe-core/tools/make-signature-keys.sh ids`: OpenSSL 3 and 1 defaults, Windows' 3DES, AES-128, PBES2 with 3DES, SHA-512 and PBMAC1 integrity checks, unencrypted parts, P-256 and P-384 keys, non-ASCII and empty passwords, an ID with its authority, an expired one); wrong passwords; files refused with the reason (no integrity check, keys too short or too long, a key not for signing, an unsupported curve, no key, a certificate for another key); every truncation and many changed bytes never panic; making self-signed IDs (certificate contents, five years, file permissions); several IDs, finding them and the default; the folder listing and files put there by hand; exporting and importing elsewhere. Unit tests in `pkcs12.rs`: the PKCS#12 key derivation against published test values, BER as Windows writes it, costly files refused before the work. Opt-in: `-- --ignored` has OpenSSL read MPE's files and take out the key, which then signs a PDF MPE checks and trusts |
| `crates/mpe-core/tests/signature_changes.rs` | Changes after signing: form filling, further signatures, comments, metadata and validation data allowed; certification levels and field locks applied; page content, pages, signatures, new fields, actions and links refused; the shadow attacks (hide by freeing or re-pointing objects, page content posing as a comment, a missing object filled in), a change undone before the next signature, malformed and damaged appended data never passing as unchanged; the signed version being exactly the file as signed |
| `crates/mpe-core/tests/signature_trust.rs` | Trust: signers trusted directly, and chains to a trusted authority (OpenSSL-made with `crates/mpe-core/tools/make-signature-keys.sh trust`), including chains carried in the document's validation data; issuers that may not issue, path lengths, expired links, name constraints, short keys, SHA-1, unknown critical extensions, key usages; certificate policies (required, mapped, any policy inhibited); a forged issuer and look-alike certificates named like a trusted one; loops and over-long chains; timestamp authorities; hand-made certificates for shapes OpenSSL won't make (a repeated extension, mismatched algorithms, a changed certificate); the trust file (round trip, private permissions, unusable entries kept but never trusted) and certificate files (DER, PEM, `.p7c`). Opt-in: `-- --ignored` compares the chain verdicts with `openssl verify` |
| `crates/mpe-core/tests/signing.rs` | Signing with digital IDs (the `ids` test files): RSA, P-256, P-384 and a CA-issued ID with its chain, in new boxes and empty fields, with reasons, locations and pictures; second and third signatures leaving earlier ones valid; certification at each level and what it then allows; field locks making fields read-only and locked fields refused; expired IDs, encrypted documents, bad boxes and wrong fields refused; edits signed along with the signature (a field filled; a page turned only with consent); fill-and-sign items drawn in or locked; forms asking readers to draw fields; classic tables, cross-reference and object streams, damaged files, turned pages. Saving signed documents (`append_edits`): nothing added without changes or after an undo; a comment and a filled field allowed, and saved again on top; disallowed changes named once; what each certification level lets a save add; fields drawn for display only; a deleted comment not written again; passwords, encrypted and damaged files and a removed signature refused; numbers written back without their decimals (`300.0` as `300`) not taken as a change. Unit tests in `incremental.rs` (only changed objects written, nothing appended without changes, deleted objects read as gone, anything that reads back differently refused, hybrid files recognised) and `sig_sign.rs` (dates, text fitting, names the font can't show, finding only the signature's own placeholders). Opt-in: `-- --ignored cross_check` has `pdfsig` and `openssl cms -verify` check MPE's signatures, one of them on a document saved with changes after signing |
| `crates/mpe-core/tests/signatures.rs` | Checking digital signatures: every supported algorithm, digest and signature format (made in the test with OpenSSL-made keys, `tests/sigkit/`), timestamps, certification and field locks, warnings; the published attack classes (universal forgery, signature wrapping, incremental saving, copied and spoofed signatures, certificate swaps, tampered CMS); truncated and corrupted files never pass; pyHanko and node-signpdf samples with the result each should get, and the changes each should show. Opt-in: `-- --ignored dss` checks the EU DSS samples (fetch them first with `crates/mpe-core/tools/fetch-dss-samples.sh`), and `-- --ignored cross_check` compares MPE's results with poppler's `pdfsig` and OpenSSL |
| Unit tests in `crates/mpe-core/src` | Page ranges, document operations, matrices, overlays, print placement, the ToUnicode reader, standard font widths, laying out form fields (wrapping, auto size, alignment, comb, lists) |
| Unit tests in `crates/mpe-gui/src` | The command list (unique entries and shortcuts, menus), selection rules, page ranges, moving pages, fitting the zoom, document notices, recent files, settings, the security form, form fields (Tab order, what a click does, keeping typed text), digital signature results (colour, sentence and banner for valid, unknown, trusted, warned, broken, changed and certified samples; names from the file cleaned), signing from the signing window (a valid signed copy written, a wrong password, earlier signers named before disallowed changes, signing after consent), passwords overwritten |

The rendering tests assert what the PDF standard says should appear. The cases hayro gets wrong are marked
`#[ignore]` with the reason; after a hayro upgrade, run
`cargo test -p mpe-core --test render_coverage -- --ignored` to see whether any have been fixed.

## The desktop app

There are no automated tests of the desktop app's screens. Its commands, selection rules and page moves are
unit-tested, and the operations behind it are covered by the core tests and the command line. The redesigned window
(top bar, inspector, command palette, right-click menus, floating toolbar, thumbnail dragging, collapsing inspector)
has been used by hand on Linux. Later additions (recent files, document notices, fit width, the theme button, crop)
were checked by screenshot under a virtual X server; not every one has been clicked through by hand. Of the newest:
the page layouts (facing and single page) have unit tests for pairing and fitting but were not clicked through;
viewer preferences were checked through their core tests and `mpe view-prefs`; following them on opening and the
Opening view section were not clicked through;
the stamp tool (including your own wording) was checked through its core tests and `mpe stamp` renders (including
turned pages), not clicked through;
fill and sign was checked through its core tests (including empty signature fields), unit tests of the saved
signatures file, `mpe stamp --signature` renders in all four handwriting styles, and screenshots of the Sign pane, the
notice and the Create signature window (Draw and Type), then used by hand on Linux, including placing a scanned
signature and checking that unsaved signatures are gone after a restart;
redaction was checked through its core tests and `mpe redact` on real and scanned PDFs (renders checked), and its
app part (Redact tool, marking selected text and search results, the notice, Apply, the save warning) was used by
hand; removing hidden information was checked through its core tests and `mpe sanitise` on real PDFs, its dialog not
clicked through;
checking digital signatures in the app was checked through unit tests on the pyHanko and node-signpdf samples
and screenshots of the banner (unknown signer, broken, certified); the Signatures tab, Details, the trust windows,
View signed version and clicking a signed field were not clicked through;
signing (`mpe sign`) was checked through its core tests, `pdfsig` and OpenSSL, and by hand on a public PDF (the
look rendered by poppler and by MPE); not yet in Acrobat; signing in the app was checked through unit tests that
run the signing window's background work (signed copy written and valid, wrong password, the disallowed-changes
question and signing after consent) and screenshots of the Digital IDs window, the signing and certifying windows
and the placing tool's pane; none of these was clicked through;
filling in forms has been used by hand on one form; the Bookmarks tab (including adding, renaming and deleting
bookmarks) and links were checked through their core tests, `mpe bookmarks` and real PDFs, and the XFA notice by
screenshot, but none of these was clicked through. Filled forms were also opened in poppler and
Ghostscript, including encrypted ones; not yet in Adobe Reader.

Each feature page says what was not tested, under "How this was tested".

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
| `crates/limarepdf-core/tests/integration.rs` | Overlays, encryption round trips with all four ciphers, search, crop margins on turned pages |
| `crates/limarepdf-core/tests/edit.rs` | Content editing: objects and text edits; edited, added and invisible (OCR) text upright and in place on a page that flips and scales itself without restoring the state afterwards, as pages printed from web browsers do, and repeated edits not nesting the old content deeper. Reading text: words, search boxes placed by glyph widths, ToUnicode maps, word gaps, text inside form XObjects, phrases across lines |
| `crates/limarepdf-core/tests/repair.rs` | Damaged files built by hand open with their page drawn and are marked repaired: wrong offsets, no table or trailer, a cross-reference stream that doesn't decode, a missing `endobj`, a later copy of an object winning, a trailer naming the wrong catalog, object stream members starting with comments, and an encrypted file (a wrong password is still reported as one); intact files and documents with no pages aren't repaired; a repaired document saves as a clean file. Crafted files: `endstream` in a content stream's comment followed by a fake page, an uncompressed attached PDF reusing the outer file's object numbers (its length given directly, or in an object before or after it), `stream` and a line break inside a string, a stream cut off by the end of the file, a `/Length` too short, an object freed by a later revision (not on the page, not saved), and a number reused at a higher generation. Unit tests in `repair.rs` (dictionary ends, object headers, stream data skipped by `/Length`, direct or in an object of its own, or to `endstream`, trailers inside stream data ignored, trailer values written back) |
| `crates/limarepdf-core/tests/render_coverage.rs` | Rendering, pixel by pixel; see [Rendering](rendering.md) |
| `crates/limarepdf-core/tests/render_compare.rs` | Opt-in: LimarePDF's rendering compared with poppler's `pdftoppm` over about 1,100 public test PDFs; see below |
| `crates/limarepdf-core/tests/render_region.rs` | Drawing part of a page, as the app's tiles at high zoom do: tiles match the whole page pixel by pixel, are cut at the page's edges, land in place at the highest scale, and keep pictures in dark pages |
| `crates/limarepdf-core/tests/import.rs` | Importing images as pages |
| `crates/limarepdf-core/tests/scan.rs` | Optimising scanned pages |
| `crates/limarepdf-core/tests/ocr.rs` | OCR text layers, their placement, search and correction |
| `crates/limarepdf-core/tests/export.rs` | Exporting pages as images and text |
| `crates/limarepdf-core/tests/print.rs` | What is sent to the printer, with a stand-in for `lp` |
| `crates/limarepdf-core/tests/outline.rs` | Bookmarks: reading nesting, every kind of destination, broken and looping outlines; adding, renaming and removing them, with the open/closed counts kept right |
| `crates/limarepdf-core/tests/links.rs` | Reading links: every kind of target, links that can't be followed, placement on turned pages |
| `crates/limarepdf-core/tests/annots.rs` | Annotations: reading them, drawing each type that has no drawing (on screen only), removing them with replies and popups, flattening all or some pages; stamps placed, resized, faded, flattened and upright on turned pages; stamps with your own wording, and wording the font can't show refused; fill and sign: typed, drawn and picture signatures (paper made transparent and margins cropped), initials and marks drawn, labelled and sized, upright on turned pages, and Finish signing drawing in only those items |
| `crates/limarepdf-core/tests/forms.rs` | Reading form fields: every kind, inherited values, broken field trees, turned pages, XFA-only and hybrid forms; which signature fields are empty |
| `crates/limarepdf-core/tests/redact.rs` | Redaction: characters removed with the rest kept in place (spacing, kerning, turned pages, hidden text, alternative text), drawings removed or clipped, pictures blacked out (soft masks, JPEG kept as JPEG, inline images, shared pictures) or removed when unreadable, shared forms, annotations, the after-check, redaction marks kept, saved and applied |
| `crates/limarepdf-core/tests/sanitise.rs` | Removing hidden information: metadata and XMP, scripts (web links kept), attachments, hidden layers (marked content and a form in a hidden layer), content outside the crop box, finding without changing |
| `crates/limarepdf-core/tests/form_fill.rs` | Filling in forms: values, drawn appearances (checked by rendering, including a turned field), fonts that can't be used, refused values, passwords, reset, drawing missing appearances |
| `crates/limarepdf-core/tests/digital_ids.rs` | Digital IDs: importing PKCS#12 files in every shape other apps write (OpenSSL-made with `crates/limarepdf-core/tools/make-signature-keys.sh ids`: OpenSSL 3 and 1 defaults, Windows' 3DES, AES-128, PBES2 with 3DES, SHA-512 and PBMAC1 integrity checks, unencrypted parts, P-256 and P-384 keys, non-ASCII and empty passwords, an ID with its authority, an expired one); wrong passwords; files refused with the reason (no integrity check, keys too short or too long, a key not for signing, an unsupported curve, no key, a certificate for another key); every truncation and many changed bytes never panic; making self-signed IDs (certificate contents, five years, file permissions); several IDs, finding them and the default; the folder listing and files put there by hand; exporting and importing elsewhere. Unit tests in `pkcs12.rs`: the PKCS#12 key derivation against published test values, BER as Windows writes it, costly files refused before the work. Opt-in: `-- --ignored` has OpenSSL read LimarePDF's files and take out the key, which then signs a PDF LimarePDF checks and trusts |
| `crates/limarepdf-core/tests/signature_changes.rs` | Changes after signing: form filling, further signatures, comments, metadata and validation data allowed; certification levels and field locks applied; page content, pages, signatures, new fields, actions and links refused; the shadow attacks (hide by freeing or re-pointing objects, page content posing as a comment, a missing object filled in), a change undone before the next signature, malformed and damaged appended data never passing as unchanged; the signed version being exactly the file as signed |
| `crates/limarepdf-core/tests/signature_trust.rs` | Trust: signers trusted directly, and chains to a trusted authority (OpenSSL-made with `crates/limarepdf-core/tools/make-signature-keys.sh trust`), including chains carried in the document's validation data; issuers that may not issue, path lengths, expired links, name constraints, short keys, SHA-1, unknown critical extensions, key usages; certificate policies (required, mapped, any policy inhibited); a forged issuer and look-alike certificates named like a trusted one; loops and over-long chains; timestamp authorities; hand-made certificates for shapes OpenSSL won't make (a repeated extension, mismatched algorithms, a changed certificate); the trust file (round trip, private permissions, unusable entries kept but never trusted) and certificate files (DER, PEM, `.p7c`). Opt-in: `-- --ignored` compares the chain verdicts with `openssl verify` |
| `crates/limarepdf-core/tests/signing.rs` | Signing with digital IDs (the `ids` test files): RSA, P-256, P-384 and a CA-issued ID with its chain, in new boxes and empty fields, with reasons, locations and pictures; second and third signatures leaving earlier ones valid; certification at each level and what it then allows; field locks making fields read-only and locked fields refused; expired IDs, encrypted documents, bad boxes and wrong fields refused; edits signed along with the signature (a field filled; a page turned only with consent); fill-and-sign items drawn in or locked; forms asking readers to draw fields; classic tables, cross-reference and object streams, damaged files, turned pages. Saving signed documents (`append_edits`): nothing added without changes or after an undo; a comment and a filled field allowed, and saved again on top; disallowed changes named once; what each certification level lets a save add; fields drawn for display only; a deleted comment not written again; passwords, encrypted and damaged files and a removed signature refused; numbers written back without their decimals (`300.0` as `300`) not taken as a change. Unit tests in `incremental.rs` (only changed objects written, nothing appended without changes, deleted objects read as gone, anything that reads back differently refused, hybrid files recognised) and `sig_sign.rs` (dates, text fitting, names the font can't show, finding only the signature's own placeholders). Opt-in: `-- --ignored cross_check` has `pdfsig` and `openssl cms -verify` check LimarePDF's signatures, one of them on a document saved with changes after signing |
| `crates/limarepdf-core/tests/signatures.rs` | Checking digital signatures: every supported algorithm, digest and signature format (made in the test with OpenSSL-made keys, `tests/sigkit/`), timestamps, certification and field locks, warnings; the published attack classes (universal forgery, signature wrapping, incremental saving, copied and spoofed signatures, certificate swaps, tampered CMS); truncated and corrupted files never pass; a file with more signed versions than LimarePDF opens; pyHanko and node-signpdf samples with the result each should get, and the changes each should show. Opt-in: `-- --ignored dss` checks the EU DSS samples (fetch them first with `crates/limarepdf-core/tools/fetch-dss-samples.sh`), and `-- --ignored cross_check` compares LimarePDF's results with poppler's `pdfsig` and OpenSSL |
| Unit tests in `crates/limarepdf-core/src` | Page ranges, document operations, matrices, overlays, print placement, the ToUnicode reader, standard font widths, laying out form fields (wrapping, auto size, alignment, comb, lists), replacing files (temporary files created new, never a stale file or a symbolic link, private while written), free disk space |
| `crates/limarepdf-gui/src/recovery/tests.rs` | The crash recovery store: copies written, replaced, found and read back; an app crash and a power cut at each step of writing a copy (power cuts on a model disk where any unsynced change may be lost), with a wrong order shown to fail; a failure at each step; withdrawing copies before a redaction, with every failure case and races against a write in progress; turning recovery off; damaged and expired copies; a session another running process holds; free space, permissions and a read-only folder; the recovery thread's order |
| `crates/limarepdf-gui/src/app/recover.rs`, `recovery/schedule.rs` | Recovery copies in the app, on the model disk: when a copy is due (a minute after the first change, coalesced, one write at a time, a change during a write, failures retried no faster, at once after a withdrawal); copies removed when a tab is saved or closed and at a normal exit; none for password-protected documents or pages taken from them, and a password withdrawing the copy first; redaction and removing hidden information withdrawing the copy and writing a new one at once; each of the three stopping, with the document unchanged and the file named, when an earlier copy could still be offered, and going ahead with a notice when it was only left on disk; turning recovery off always taking effect; recovery that can't start; a restored copy of a signed document (notice at once, no signature check, signatures listed as not checked, Save and Sign asking first); a damaged copy offered, never restored |
| Unit tests in `crates/limarepdf-gui/src` | The command list (unique entries and shortcuts, menus), selection rules, page ranges, moving pages, fitting the zoom, document notices, recent files, settings, the security form, form fields (Tab order, what a click does, keeping typed text), digital signature results (colour, sentence and banner for valid, unknown, trusted, warned, broken, changed and certified samples; names from the file cleaned), signing from the signing window (a valid signed copy written, a wrong password, earlier signers named before disallowed changes, signing after consent), passwords overwritten |

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

The timings and memory figures in [Performance](performance.md) come from
`python3 crates/limarepdf-core/tools/measure.py`, run from the repository (Linux or macOS, Python's standard library
only). It builds the release CLI, makes a 1,000-page text file and a 200-page colour scan in `target/tmp/measure`,
times each task three times and prints the table with the machine and commit. Peak memory is the program's largest
resident set, as `/usr/bin/time -v` reports it. The four rows for public test files need the files from
`tools/render-compare.sh` (or `tools/fetch-render-samples.sh`) and are skipped without them. `--runs 1` makes a quicker
run, and `--only scan` runs only the tasks whose names contain `scan`. A full run takes about five minutes, and one
of the test files needs about 10 GB of memory.

## The desktop app

There are no automated tests of the desktop app's screens. Its commands, selection rules and page moves are
unit-tested, and the operations behind it are covered by the core tests and the command line. The redesigned window
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
and screenshots of the banner (unknown signer, broken, certified); the Signatures tab, Details, the trust windows,
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
screen.

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
| Print to a real printer | Not run (no printer) | Printed by hand (0.3) | Not run |
| Keyboard shortcuts (Ctrl, or Cmd on macOS) | Used by hand | Not run | Not run |
| HiDPI screen (scale 2) | Not run | Not run | Not run |
| Name and icon in the taskbar, dock and Open with | Not recorded | Icon checked by hand (0.5) | Not run |
| A damaged file and a file that fails to draw: message, no crash | Not recorded | Not run | Not run |

On Linux, HiDPI can be tried without such a screen: run the app under a virtual X server with
`WINIT_X11_SCALE_FACTOR=2` and take screenshots, as for other screenshots. A real HiDPI screen is still needed for
sharpness.

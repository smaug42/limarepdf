# How it is built

MPE is written entirely in Rust, with no C or C++ dependencies.

| Part | What it is |
| --- | --- |
| `mpe-core` | The library: document operations, rendering, editing, OCR, printing |
| `mpe` | The command-line tool |
| `mpe-gui` | The desktop app |

It uses:

- [`lopdf`](https://crates.io/crates/lopdf) for the document structure and encryption;
- [`hayro`](https://crates.io/crates/hayro) to draw pages, on the CPU (see [Rendering](rendering.md));
- [`egui`](https://crates.io/crates/egui) for the interface, and [`rfd`](https://crates.io/crates/rfd) for file
  dialogs;
- [`image`](https://crates.io/crates/image) for images;
- [`ocrs`](https://github.com/robertknight/ocrs) for text recognition (see [Scanning and OCR](scanning-and-ocr.md));
- [`serde_json`](https://crates.io/crates/serde_json) in the command line, for form values as JSON.

Forms are read in `crates/mpe-core/src/forms.rs`. When a value changes, MPE draws the field's appearance itself
(`form_ap.rs`, written into the file by `form_write.rs`) rather than asking other readers to, so filled fields look
the same everywhere; the desktop app's side is `crates/mpe-gui/src/fields.rs`.

Page content is read by MPE's own parser (`crates/mpe-core/src/objects.rs`), which splits it into the objects that
editing works on and reads the text that search, copying and export use, with its own reader for fonts' ToUnicode
maps and Adobe's widths for the standard fonts (see [Search and text](search-and-text.md#text-extraction)).

Redaction (`crates/mpe-core/src/redact.rs`) has its own pass over the page content rather than editing the
parser's objects: it rewrites each text-showing operator character by character, keeping the rest in place with
spacing numbers, cuts drawings into sub-paths, and redacts inside form XObjects as copies. Pictures are decoded,
blacked out and saved again in `redact_img.rs`. Afterwards the pages are parsed again to check that nothing readable
is left under the marks. Hidden information (metadata, scripts, attachments, hidden layers) is removed by
`sanitise.rs`. See [Redaction](redaction.md).

Fill-and-sign signatures (`crates/mpe-core/src/sign.rs`) are stamp annotations whose artwork MPE draws once:
pen strokes smoothed into curves, typed names as the glyph outlines of a bundled handwriting font (read with the
`skrifa` crate, so no font goes into the PDF), or a picture with its paper made transparent. Finish signing
flattens just those annotations. The app's saved signatures and the Create signature window are in
`crates/mpe-gui/src/signing.rs`.

Digital signatures are checked in `crates/mpe-core/src/sig_verify.rs` (finding them, the byte-range rules, and
tying each signature value to its dictionary in the version it signed, by position) and `cms_check.rs` (the CMS
signature, certificates, timestamps), over a small BER/DER reader, `asn1.rs`, that walks the data exactly as written
so signed attributes are never re-encoded before checking. The cryptography is RustCrypto's (`rsa`, `p256`, `p384`,
`sha1`, `sha2`, `x509-cert`). What changed after each signature is worked out in `sig_changes.rs`: it reads the
appended revisions' cross-reference sections itself (lopdf ignores free entries and repairs broken tables, which
would hide changes), checks that lopdf reads the same objects, then compares each signed version with the next,
judging every object that differs by each role it plays (page content, comment, form field, signature, document
structure) against the signer's permissions. See [Digital signatures](signatures.md).

In the desktop app, every command is defined once in `crates/mpe-gui/src/actions.rs`; the menu, the top bar, the
command palette, the right-click menus, the floating toolbar and the keyboard shortcuts are all built from that list.

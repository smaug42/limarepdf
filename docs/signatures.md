# Digital signatures

Digital signatures are being built in stages. What works now: **checking** the signatures in a PDF from the
command line (`mpe verify`), including what changed after each signature and whether the signer allowed it, and
saving the version a signature signed. Still to come: trusting signers, checking in the app, digital IDs, signing
and certifying, and saving signed documents without breaking their signatures. This page grows with each stage.

## What a digital signature is and isn't

A digital signature proves that the signed bytes of a document haven't changed since a particular key signed them.
It doesn't by itself say who owns that key: the signature carries a certificate naming someone, and anyone can make
a certificate with any name. Whether to believe the name is a separate question, called trust.

It is not the same as [fill and sign](gui.md#filling-and-signing), which puts a picture of your signature on the
page. Anyone can copy a picture; a digital signature breaks if the document is changed.

## Requirements

Checking needs nothing: no network, no account, no certificates installed. MPE never connects anywhere to check a
signature.

## Checking a signed PDF

```
mpe verify contract.pdf
mpe verify contract.pdf --json
mpe verify locked.pdf --password secret
```

For each signature it shows the field and page, the result, who signed according to the certificate and who issued
that certificate, when it was signed and where that time comes from, the reason, location and contact the signer
gave, the key and algorithms, fields the signature locked, the certificate's serial number, validity and SHA-256
fingerprint, and any warnings. Signature fields that haven't been signed yet are listed too, with the fields
signing them would lock.

When something was added to the file after a signature, it lists the changes under that signature, each marked
allowed or NOT ALLOWED with the reason (see [Changes after signing](#changes-after-signing)).

`mpe verify` exits with status 0 when every signature checks as valid (also when there are none) and nothing
changed after any of them that its signer didn't allow, and 1 otherwise, including when a signature can't be
checked or the file can't be opened. `--json` gives the same information for scripts.

### The version that was signed

```
mpe verify contract.pdf --signed-version 1 -o as-signed.pdf
```

saves the file exactly as it was when signature 1 (numbered as `mpe verify` lists them) was signed, without
anything added later. Open it to see what the signer saw, or compare it with the current file. This is Acrobat's
"View signed version". MPE refuses it for a broken signature, whose "signed version" isn't genuine, and warns when
the signature couldn't be checked.

The signer's name, reason, location and contact come from the file and are claims by whoever made it; MPE removes
characters that could disguise them (direction changes, invisible characters) before showing them.

## What each result means

**Valid: not changed since NAME signed it; MPE can't confirm who that is.** The signed bytes are exactly as signed,
the signature matches the certificate it carries, and the signature is laid out the way the PDF rules require.
MPE doesn't yet keep a list of signers you trust, so every valid signature is from an unknown signer for now, as
in Acrobat before you trust someone. Check the SHA-256 fingerprint with the signer in some other way (a phone call,
their website) if who signed matters.

**Valid: only allowed changes since NAME signed it.** The signed part is intact, and everything added since is a
change the signer allowed, such as filling in the form, another signature or a comment. The changes are listed;
read them, because an allowed change can still change what the page shows (a form field filled in, a comment
placed over the text).

**Valid** for a **document timestamp**: a timestamp authority confirms the document existed like this at the time
shown. No person signed it.

**CHANGED: changed after NAME signed it in ways the signer didn't allow.** The signed part is intact, but
something added later is a change the signer didn't allow, or one MPE can't recognise. The list says what. The
document as you see it may not be what was signed: look at the signed version (above), and ask the sender.

**BROKEN: reason.** Not a genuine, intact signature:

- *the signed part was changed: its digest doesn't match*: bytes the signature covers were changed;
- *the signature value doesn't match the signer's certificate*: the signature wasn't made by that certificate's key;
- *the signature names a different certificate from the one it carries*: someone swapped the certificate;
- *something other than the signature value sits between the signed ranges*, *the byte range …*, *the signed part
  doesn't end where a version of the document ends*: the signature doesn't cover the document the way the rules
  require, the pattern of the known "signature wrapping" attacks;
- *the signature's dictionary was added after the version it claims to sign (a copied signature)*, *the signature
  value isn't inside this signature's dictionary*: a signature copied from another field or moved, a known
  spoofing attack;
- *the timestamp is for different data* (document timestamps).

**Can't check: reason.** MPE can't tell either way: an algorithm it doesn't support (MD5, RIPEMD-160, Brainpool
and P-521 curves), a missing or unreadable certificate, more than one signer in one signature (the PDF rules allow
exactly one), or data it can't read. Don't rely on such a signature without checking it elsewhere.

### Warnings

A warning doesn't make a valid signature invalid, but is worth knowing:

- *uses SHA-1*: SHA-1 is no longer safe against forgery; a signature relying on it may have been forged.
- *uses a short RSA key*: under 2048 bits, which is no longer considered safe.
- *the signer's certificate wasn't valid at the signing time*: expired, or not yet valid, according to the signing
  time shown.
- *the signer's certificate isn't meant for signing*: the certificate limits its key to other uses.
- *the timestamp authority's certificate isn't meant for timestamps*.
- *the embedded timestamp can't be used*: the signature is still checked, but its time then comes from the signer's
  computer.
- *extra data follows the signature value*: unusual, harmless to the check itself.
- *the same signature is used by more than one field*: a sign of tampering.
- *the signature's details were replaced after signing*: the reason, name or time was changed in a later addition;
  MPE shows the values that were signed.
- *this certification isn't the first signature*, *the document's permissions don't point to this certification*:
  the certification doesn't follow the rules, so its limits may not hold.

### Signing times

A signing time comes from a **timestamp authority** when the signature carries a valid timestamp; otherwise from
**the signer's computer**, which can be set to any time.

### Certification and locked fields

A **certification** is the author's signature, which also says what may change afterwards: nothing, filling in
forms and signing, or that plus comments. MPE shows the level. A signature can also **lock** form fields: all of
them, some, or all but some, and (PDF 2.0, Acrobat's "Lock document after signing") limit what may change in the
whole document, like a certification does.

## Changes after signing

A signed PDF can still be changed: the changes are added to the end of the file, and the signed part stays as it
was. That is how a form is filled in after the first signature, or a second person signs. It is also how a signed
document can be altered, so MPE compares each version with the next and judges every change by Acrobat's rules:

| Change | After an ordinary signature | Certified: form filling and signing | Certified: also comments | Certified: no changes |
| --- | --- | --- | --- | --- |
| Form field filled in | allowed | allowed | allowed | not allowed |
| Signed in an empty signature field | allowed | allowed | allowed | not allowed |
| Signature field added (signed or not) | allowed | only if invisible | only if invisible | not allowed |
| Comment added, changed or removed | allowed | not allowed | allowed | not allowed |
| Form settings for filling in (fonts) | allowed | allowed | allowed | not allowed |
| Document timestamp, validation data | allowed | allowed | allowed | allowed (a timestamp only if invisible) |
| Document information and metadata | allowed | allowed | allowed | allowed |
| Page content, pages added or removed | not allowed | not allowed | not allowed | not allowed |
| A signature changed or removed | not allowed | not allowed | not allowed | not allowed |
| Form field added (other than a signature field), or changed beyond its value | not allowed | not allowed | not allowed | not allowed |
| Links and other annotations that aren't comments | not allowed | not allowed | not allowed | not allowed |
| Actions and scripts, document structure | not allowed | not allowed | not allowed | not allowed |
| Anything MPE doesn't recognise | not allowed | not allowed | not allowed | not allowed |

A certification limits every signature in the document, not only its own. Each signature's own locks apply on top:
a locked field may not be filled in or signed (a document timestamp is still allowed). When a change is refused,
the reason says which rule: *never allowed after signing*, *the certification allows only …*, or *this signature
locked it*.

Each version a later signature signed is compared with the one before it, so a change made and undone between two
signatures still counts against both: the second signer signed something the first didn't.

### How MPE judges a change

Every object that differs is judged by each part it plays in the newer version: page content, a comment, a form
field's value or appearance, a signature, the document's structure. A change is allowed only when every one of
those parts allows it. So an object that is both page content and, say, a comment's appearance can't be changed
as "just a comment". Pointers from one structure to another (a bookmark to a page, a tag to an annotation) don't
count as parts.

MPE reads the later additions with its own strict reader rather than the lenient one it uses to open files, and
refuses as malformed:

- a later revision that doesn't lead back to the signed version, or data after the end of the file;
- an object whose cross-reference entry points into the signed part, outside its own revision, or at another
  object (the published "shadow" attacks that re-point objects at content hidden in the signed file);
- an object that can't be read, or that MPE and its PDF library read differently.

An object deleted while still in use, and an object the signed version referred to but didn't have, are refused
too. These are the shapes of the published shadow (hide, replace, hide-and-replace) and incremental-saving attacks.
A text field drawn on the page but missing from the form's list of fields, as in the published "replace" attack,
counts as an annotation that isn't a comment, so filling it in isn't allowed.

## Compatibility

Checked against third-party signed files with known results: the pyHanko and node-signpdf samples in MPE's tests,
and the EU DSS project's validation samples (signatures from Adobe products, Italian, Spanish and German signers,
PAdES with timestamps, and published attacks). Each file's expected result and why is recorded in
`crates/mpe-core/tests/signatures.rs`. MPE's own test signatures are also checked with poppler's `pdfsig` and
OpenSSL, which agree.

Where MPE and `pdfsig` differ, MPE follows the PDF rules: it refuses byte ranges with gaps or uncovered parts that
`pdfsig` accepts, refuses two signers in one signature, and accepts signers named by key identifier, which `pdfsig`
calls invalid.

Changes after signing were checked on the same samples, among them files changed by Adobe Acrobat and Acrobat
Reader (a comment added, a second field signed), iText (a signature added) and PAdES tools (timestamps and
validation data), which MPE reports as allowed, and the DSS attack samples (shadow hide and replace, a page added,
a signature appearance changed, a reason replaced), which MPE refuses. One DSS sample whose later revisions point
to cross-reference sections that don't exist is refused as malformed, where DSS repairs the file and accepts it.
`pdfsig` doesn't judge changes after signing, so there is no cross-check for this part.

Not yet tested: signatures made in Adobe Acrobat by MPE's testers, and MPE-made signatures in Acrobat (MPE can't
sign yet). Where MPE is stricter than Acrobat (see the limitations) is to be confirmed in those tests.

## Limitations

- No trust yet: every valid signature shows an unknown signer.
- Changes after signing are judged strictly where Acrobat's exact behaviour isn't documented, so MPE may refuse
  changes Acrobat accepts: a new visible signature field after a certification (or a lock limiting changes), new
  pages from page templates, changes to the document's tags or other structure made while commenting, link
  annotations, and comments that carry actions. False alarms are better than missing an attack; a refused change
  is listed by name, so you can judge it.
- A change made and undone between two revisions that nobody signed is not reported: nobody signed it, and the
  file no longer shows it.
- An allowed change can still change what the page shows (a form field filled in, a comment placed over text).
  MPE lists it; it can't tell whether that was the intent.
- Malformed later additions are refused even when another reader can repair them.
- Revocation (whether a certificate was withdrawn) isn't checked. Acrobat checks it online for certificates from
  authorities; MPE makes no network connections.
- Algorithms: RSA (PKCS#1 v1.5 and PSS, up to 8192 bits), ECDSA on P-256 and P-384, with SHA-1, SHA-224, SHA-256,
  SHA-384 and SHA-512. Brainpool curves (used by some European ID cards), P-521, MD5 and RIPEMD-160 can't be
  checked.
- Certificates must be properly DER-encoded; a few old signers wrote them otherwise, and those can't be checked.
- Adobe's Reader-enabling signatures (usage rights, which let the free Reader save forms) aren't listed, as in
  Acrobat.
- Signature fields must be in the document's form; a signature drawn on a page without a field isn't one.
- A file the PDF reader can't open at all can't be checked.

## Security notes

Checking uses these pure-Rust crates from the RustCrypto project, pinned to exact versions where they are
pre-releases:

| Crate | Version | Status |
| --- | --- | --- |
| `rsa` | 0.10.0-rc.19 | pre-release |
| `pkcs1` | 0.8.0-rc.5 | pre-release |
| `der`, `spki`, `x509-cert` | 0.8.2, 0.8.1, 0.3.0 | stable |
| `p256`, `p384` | 0.14.0 | stable |
| `sha1`, `sha2` | 0.11.0 | stable |

"Pre-release" means their programming interface may still change, not that they are untested; they come from the
same project and review as the stable crates. MPE reads the signature data with its own small reader, which works
on the bytes as written and never re-encodes what was signed.

**RUSTSEC-2023-0071**, a timing leak in `rsa` with no fixed version yet, concerns using a private key. Checking
uses only public keys, so it doesn't apply to checking. It will apply to signing with an RSA key: an attacker would
have to time many signings on your computer. CI runs `cargo audit` on every change with this one advisory set
aside; [releasing](releasing.md) has a step to look for final versions and new advisories before each release.

## Troubleshooting

- **Changes listed** on a document you expect to be untouched: someone, or some program, saved it with changes
  after signing. Allowed ones are normal for forms and documents signed by several people; for anything else, save
  the signed version (`--signed-version`) and compare, or ask the sender for the original.
- **"CHANGED … in ways the signer didn't allow"** on a document that Acrobat shows as fine: see the limitations
  above; MPE is stricter where it can't be sure. The list says which change, and the signed version shows what was
  signed.
- **"the later additions are malformed"**: the file was saved by a program that wrote it incorrectly, or tampered
  with. The signed version is still available.
- **"Can't check"**: try another reader, or ask the signer which algorithm they used; see the limitations above.

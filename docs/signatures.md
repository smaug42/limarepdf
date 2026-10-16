# Digital signatures

Digital signatures are being built in stages. What works now: **checking** the signatures in a PDF from the
command line (`mpe verify`). Still to come: telling allowed changes after a signature from disallowed ones,
trusting signers, checking in the app, digital IDs, signing and certifying, and saving signed documents without
breaking their signatures. This page grows with each stage.

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

`mpe verify` exits with status 0 when every signature checks as valid (also when there are none), and 1 otherwise,
including when a signature can't be checked or the file can't be opened. `--json` gives the same information for
scripts.

The signer's name, reason, location and contact come from the file and are claims by whoever made it; MPE removes
characters that could disguise them (direction changes, invisible characters) before showing them.

## What each result means

**Valid: not changed since NAME signed it; MPE can't confirm who that is.** The signed bytes are exactly as signed,
the signature matches the certificate it carries, and the signature is laid out the way the PDF rules require.
MPE doesn't yet keep a list of signers you trust, so every valid signature is from an unknown signer for now, as
in Acrobat before you trust someone. Check the SHA-256 fingerprint with the signer in some other way (a phone call,
their website) if who signed matters.

**Valid** for a **document timestamp**: a timestamp authority confirms the document existed like this at the time
shown. No person signed it.

**Changes: the file was changed after this signature.** Something was added to the end of the file after signing.
That is how filling in a form, adding a comment or a second signature after the first one works, and it is also
how a signed document can be altered. The signed part is still intact; MPE doesn't yet tell which kind of change it
was. Treat such a document with care until that is built.

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
them, some, or all but some.

## Compatibility

Checked against third-party signed files with known results: the pyHanko and node-signpdf samples in MPE's tests,
and the EU DSS project's validation samples (signatures from Adobe products, Italian, Spanish and German signers,
PAdES with timestamps, and published attacks). Each file's expected result and why is recorded in
`crates/mpe-core/tests/signatures.rs`. MPE's own test signatures are also checked with poppler's `pdfsig` and
OpenSSL, which agree.

Where MPE and `pdfsig` differ, MPE follows the PDF rules: it refuses byte ranges with gaps or uncovered parts that
`pdfsig` accepts, refuses two signers in one signature, and accepts signers named by key identifier, which `pdfsig`
calls invalid.

Not yet tested: signatures made in Adobe Acrobat by MPE's testers, and MPE-made signatures in Acrobat (MPE can't
sign yet).

## Limitations

- No trust yet: every valid signature shows an unknown signer.
- Changes after signing are reported but not yet judged allowed or not.
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

- **"Changes: the file was changed after this signature"** on a document you expect to be untouched: someone, or
  some program, saved it with changes after signing. Ask the sender for the original.
- **"Can't check"**: try another reader, or ask the signer which algorithm they used; see the limitations above.

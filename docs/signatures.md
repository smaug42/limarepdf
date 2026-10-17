# Digital signatures

MPE does all of this in the app and from the command line: **checking** the signatures in a PDF (`mpe verify`),
including what changed after each signature and whether the signer allowed it, opening or saving the version a
signature signed, and **trusting** signers and certificate authorities (`mpe trust`), making, importing and
exporting your **digital IDs** (`mpe id`), **signing and certifying** with them (`mpe sign`), and **saving** signed
documents without breaking their signatures. It follows Adobe Acrobat's rules so signed documents can go back and
forth between them; round trips with Acrobat itself are still to be tested (see [compatibility](#compatibility)).

## What a digital signature is and isn't

A digital signature proves that the signed bytes of a document haven't changed since a particular key signed them.
It doesn't by itself say who owns that key: the signature carries a certificate naming someone, and anyone can make
a certificate with any name. Whether to believe the name is a separate question, called trust.

It is not the same as [fill and sign](gui.md#filling-and-signing), which puts a picture of your signature on the
page. Anyone can copy a picture; a digital signature breaks if the document is changed.

## Requirements

Checking needs nothing: no network, no account, no certificates installed. MPE never connects anywhere to check a
signature.

For MPE to show a signer as **trusted**, you need their certificate, or the certificate of the authority that
issued theirs, and a way to be sure it is the right one: its SHA-256 fingerprint, compared with the owner by phone,
in person or on a website you trust. The certificate can come from the signed PDF itself, or from a file the
signer or your organisation sends you (`.cer`, `.crt`, `.pem`, or `.p7c` as Acrobat exports it).

[Signing](#signing) needs a [digital ID](#digital-ids) and its password: one you make in MPE, or one you import
from Acrobat, your computer or a certificate authority. The ID must be valid on the day you sign, by your
computer's clock, and the document must not be encrypted. For others to see your signatures as trusted, they need
your certificate, which MPE exports, and a way to check its fingerprint with you.

## Checking a signed PDF

### In the app

Open a signed PDF and MPE checks its signatures in the background; a banner above the pages says "Checking the
digital signatures…" meanwhile, and then gives the result, coloured by the worst signature:

| Colour | Meaning |
| --- | --- |
| Green | Every signature is valid, nothing changed after any of them that the signer didn't allow, you trust every signer, and there are no warnings. |
| Amber | Nothing is broken and nothing changed that the signers didn't allow, but MPE can't confirm who signed (you haven't trusted them), or there are warnings. |
| Grey | MPE can't check a signature (see [Can't check](#what-each-result-means)). |
| Red | A signature is broken, the document was changed after signing in ways the signer didn't allow, or a signer you trust has a certificate that fails a check. |

With one signature the banner gives its result in a sentence, such as "Not changed since Jane Doe signed it. MPE
can't confirm who Jane Doe is."; with several it sums them up. A certified document's banner starts with who
certified it and what the certification allows. The banner's **Signatures** button opens the **Signatures** tab of
the side panel (also View > Signatures).

The Signatures tab lists the signatures in the order they were made. For each: what it is and who made it (click to
go to it on the page), its result in a sentence, the changes made after it (those the signer didn't allow in red;
the first five, all of them in Details), any warnings, and the buttons **Details…** and **Signed version**.
Signature fields nobody has signed yet are listed below the signatures; click one to go to it, and hover to see the
fields signing it would lock. Clicking a signed signature field on the page, with the read tool, opens its details.

**Details** shows everything `mpe verify` does (below): the result, trust, who signed and who issued their
certificate, the signing time and where it comes from, any timestamp, the reason, location and contact the signer
gave (marked as their own words), the algorithm, locked fields, the certificate's serial number, validity and
SHA-256 fingerprint, how much of the file the signature covers, every change after signing, warnings, and the
certificates the signature carries. Its buttons trust the signer or an authority (see
[Trusting signers](#trusting-signers)), open the signed version, and go to the field.

The app checks the **file**: after you change the document, the Signatures tab says that your unsaved changes
aren't checked. Saving adds your changes after what was signed, so the signatures stay (see
[Saving a signed document](#saving-a-signed-document)); MPE then checks the saved file, so the tab shows what other
readers will see in the saved copy, your changes listed under each signature.

### From the command line

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

In the app, **Signed version** in the Signatures tab (or **View signed version** in Details) opens it in a new tab
named after the document, "(signed version N)", where it is checked like any signed file. Save as to keep it.

The signer's name, reason, location and contact come from the file and are claims by whoever made it; MPE removes
characters that could disguise them (direction changes, invisible characters) before showing them.

## Trusting signers

A valid signature proves the document is unchanged since a certain key signed it. Whose key that is, is a
separate question: anyone can make a digital ID with any name on it. MPE answers it from your **trust list**, as
Acrobat does from its trusted certificates. Each valid signature gets a **Trust** line in `mpe verify`:

- **trusted: you trust "NAME"**: the signer's own certificate is in your list (**trusted as a signer**). This is
  how you trust someone with a self-made digital ID, such as one made in Acrobat or MPE.
- **trusted: issued under "AUTHORITY", a certificate authority you trust**: the signer's certificate was issued by
  an authority in your list (**trusted as a certificate authority**), directly or through intermediate
  authorities, and every link checks out. The chain is shown, signer first. This is how you trust everyone at an
  organisation whose certificates come from one authority.
- **not confirmed: why**: nothing in your list vouches for the signer. Usual for a self-made ID you haven't
  trusted yet. The reason says what is missing: *a self-made digital ID you haven't trusted*, *issued by "ROOT",
  which you haven't trusted*, or *MPE doesn't have the certificate of "ISSUER"* (the signature doesn't carry the
  whole chain; get the authority's certificate from them).
- **NOT TRUSTED: why**: the certificate is in your list, or leads to an authority there, but fails a check. Take a
  closer look before relying on the signature; see [the reasons](#not-trusted-reasons).

Trust settings affect MPE only: trusting someone in MPE doesn't change Acrobat or any other program, and the
signer's own Acrobat knows nothing about your list.

### Adding and removing trust

**In the app:** in a valid signature's Details, **Trust this signer…** shows the signer's certificate with its
SHA-256 fingerprint, asks for the name to keep it under in your list, and trusts it when you choose Trust. **Trust
an authority…** does the same for an authority from the certificates the signature carries (choose one when there
are several). **Edit > Trusted certificates…** (also at the bottom of the Signatures tab) shows your list, removes
an entry (Remove…, then Stop trusting), and adds a certificate from a file (Add certificate file…; as a signer
unless you choose certificate authority, which only an authority's certificate allows). Every open document is
judged again as soon as the list changes. A change made with `mpe trust` while the app is open counts for documents
opened after it.

**From the command line:**

```
mpe verify contract.pdf                                   # read the SHA-256 line of signature 1
mpe trust add contract.pdf --signature 1                  # trust that signer
mpe trust add contract.pdf --signature 1 --authority      # or an authority from that signature's chain
mpe trust add jane.cer                                    # or a certificate file someone sent you
mpe trust add company-ca.cer --authority --label "Our company"
mpe trust list
mpe trust remove 2                                        # by its number in the list, or its fingerprint
```

**Compare the fingerprint first.** `mpe trust add` prints the certificate's SHA-256 fingerprint; check it with the
owner in a way you trust, such as reading it out over the phone. A certificate taken from a signed PDF is only as
trustworthy as that comparison: whoever made the PDF chose the certificate.

When a signature carries several authority certificates, `--authority` lists them and `--pick N` chooses one, as
does a certificate file holding several. Trusting an authority trusts every signer it issued a certificate to,
directly or through the intermediate authorities it allowed to issue them; trust the most specific authority that
covers the signers you mean. A certificate you already trust can be added again with the other role to change it.
A certificate authority must say it is one (basic constraints) and be allowed to sign certificates, or MPE refuses
to add it as one. `mpe trust add` on a PDF only takes the signer of a valid signature.

The list is `trust.txt` in MPE's settings folder (`$XDG_CONFIG_HOME/mpe/` or `~/.config/mpe/` on Linux,
`~/Library/Application Support/mpe/` on macOS, `%APPDATA%\mpe\` on Windows), a text file with each certificate in
PEM form, its label, the date it was added and its fingerprint. The app uses the same list. `--trust-file
FILE` on `mpe verify` and `mpe trust` uses another list, for example one shared by a team for a script that checks
incoming documents with `mpe verify --require-trusted --trust-file team.txt`.

To let others trust **your** signatures, they need your certificate: send them a signed document and let them
compare the fingerprint with you, or export your certificate as a `.cer` file ([Digital IDs](#digital-ids)). In
Acrobat, a recipient trusts you from the signature's Signature Properties > Show Signer's Certificate, Trust tab,
Add to Trusted Certificates (from Adobe's documentation; not yet tried with an MPE signature).

### What MPE checks before trusting

A trusted signer's own certificate must be usable for signing at the signing time:

- valid at the signing time (or now, when the signature gives no time);
- its key allowed to sign (key usage) and meant for documents: when it limits its uses (extended key usage), they
  must include email protection, code signing, any use, Adobe Authentic Documents, document signing or Microsoft
  document signing (Acrobat's list plus the two document-signing uses);
- an RSA key of at least 2048 bits, no repeated extensions, and no critical extension MPE doesn't know.

For a chain to a trusted authority, each certificate on the way must also be signed by the next one's key (checked
over the certificate exactly as written, with matching signature algorithms), and each issuer must:

- say it is a certificate authority and, if it limits its key's uses, allow signing certificates;
- be valid at the signing time;
- allow as many authorities below it as the chain has (path length);
- not sign with SHA-1 or MD5, and have an RSA key of at least 2048 bits;
- not set name constraints, which MPE doesn't check yet (it refuses rather than ignore them).

Certificate policies are processed as RFC 5280 says, with any policy acceptable: when an authority requires an
explicit policy (as qualified-signature authorities in the EU do), some policy must run through every certificate
down to the signer, following policy mappings and any limit on "any policy". MPE follows issuers through the
certificates the signature carries and those in the document's validation data (`/DSS`, added by PAdES tools),
up to ten certificates long. Those certificates only help find the way: trust comes from your list alone. The trust
list's own certificate for an authority is the end of the chain, whatever the document carries under that name.

A **timestamp authority** is judged the same way, for its timestamp: trusted directly or through an authority, and
its certificate must be meant for timestamps. A document timestamp's Trust line is about its authority, and an
embedded timestamp shows *(trusted)*, *(MPE can't confirm this authority)* or *(NOT TRUSTED: …)* on its Timestamp
line.

## Digital IDs

A digital ID is what you sign with: a private key, which stays secret, and a certificate naming you, which goes
into every signature you make. MPE keeps your IDs, each protected by its own password. You make, import, export,
list and remove them in the app's **Digital IDs** window (☰ > Edit > Digital IDs…) or with `mpe id`, and
[sign](#signing) with them.

**In the app**, the Digital IDs window lists your IDs; pick one to see its certificate, key, validity and SHA-256
fingerprint. **Create…** asks for the name signatures show, an optional organisation and email address, and a
password typed twice; **Import…** asks for a `.p12` or `.pfx` file and its password; **Export…** saves the
certificate only (`.cer`) or the ID with its key (`.p12`, with the ID's password or a new one); **Make default**
chooses the ID signing starts with; **Remove…** asks a second time (**Remove for good**). Making, importing and
exporting run in the background and the window shows when they are done. The same rules apply as on the command
line, described below.

IDs are kept in the `ids` folder of MPE's settings folder: `~/.config/mpe/ids/` on Linux,
`~/Library/Application Support/mpe/ids/` on macOS, `%APPDATA%\mpe\ids\` on Windows. Each is one PKCS#12 file
(`.p12`), the same kind of file Acrobat, Windows and macOS use. Copy files *into* MPE with `mpe id import`, not by
putting them in the folder.

### Making a digital ID

```
mpe id create --name "Jane Doe" --organisation "Example Ltd" --email jane@example.com
```

MPE asks for a password twice: at least 8 characters, and a long one is better, because anyone who gets a copy of
the file can try passwords on it at their leisure. The organisation and email address are optional. Making the ID
takes a few seconds.

The ID is like the self-signed IDs Acrobat makes: an RSA key (3072-bit, where Acrobat's are 2048-bit), a certificate
signed with SHA-256 and valid for five years from today, its key allowed for digital signatures and
non-repudiation, and meant for email protection and Adobe's authentic documents, as Acrobat's are. The email
address is in the name and also as the certificate's alternative name. MPE checks that its own trust checks would
accept the new certificate before keeping it.

"Self-signed" means nobody vouches for the name in it: your signatures show as *not confirmed* to others until they
trust your certificate. Send it to them (`mpe id export jane-doe -o jane.cer`) and tell them its SHA-256
fingerprint another way, such as by phone, so they can check the file is yours.

The ID made, imported or signed with last is the **default**, the one signing uses unless told otherwise.

### Importing a digital ID

```
mpe id import work.p12
```

MPE asks for the file's password. It reads `.p12` and `.pfx` files from Acrobat, Windows, macOS, OpenSSL and
certificate authorities, with the encryptions in use, old and new: AES and 3DES (PBES2 with PBKDF2), the older
3DES and RC2 encryptions, and the integrity checks HMAC-SHA-1 to HMAC-SHA-512 and PBMAC1. The ID keeps its
password, re-encrypted with MPE's own settings (see [security notes](#security-notes)); the file you imported is
left as it was, so keep it safe or delete it. `--name` chooses the ID's name in MPE; by default it comes from the
certificate. An ID from a certificate authority keeps the authority certificates that came with it.

MPE refuses a file it couldn't sign with, and says why:

- a key MPE can't sign with: RSA shorter than 2048 bits (not safe) or longer than 4096 bits (Acrobat can't check
  it), elliptic curves other than P-256 and P-384, or other kinds of key;
- a certificate whose key isn't allowed to sign, or that is meant for other uses than signing documents (the same
  rules as [trusting a signer](#what-mpe-checks-before-trusting));
- a file without an integrity check, or protected by a public key instead of a password;
- a file with no key (a certificate on its own: trust it with `mpe trust add` instead), with several keys, or
  whose certificate doesn't belong to its key;
- an ID that is already in MPE.

An expired ID is imported, so you can keep and export it, but MPE won't sign with it; `mpe id list` marks it.

### Exporting a digital ID

```
mpe id export jane-doe -o jane.cer
mpe id export jane-doe -o jane.p12
```

To a `.cer` file, MPE writes the certificate only (DER, as Acrobat exports it): no key, safe to share, for others
to trust your signatures. To a `.p12` or `.pfx` file, it writes the ID with its key, to use in another app or keep
as a backup somewhere safe; it asks for the ID's password and protects the file with the same one, or with a new
one with `--new-password`. Anyone with that file and its password can sign as you.

### Listing and removing digital IDs

`mpe id list` shows each ID with a number, its name in MPE, the default, its certificate, key, validity and
fingerprint, and warns about files in the folder that other users could read or that MPE can't list. `mpe id
remove 2` (or a name) deletes an ID after you type `yes`: it can't be undone, and the ID can't sign again unless you
have an exported copy. Signatures already made with it stay valid.

## Signing

### In the app

1. Choose ☰ > Document > **Sign with digital ID…** (or the button of that name in the Sign tool's pane), or
   **Certify…** to [certify](#what-signing-does) the document (only before anyone has signed it).
2. Click an empty signature field on the page, or drag a box where the signature goes (at least 12 points each
   way). The pane also lists the document's empty signature fields to pick from. Cancel there, or Esc, to stop.
3. In the window, pick the digital ID (the default is picked; expired IDs can't sign), type its password, and
   optionally a reason and a location. **Show my saved signature beside the text** uses the first signature in
   your fill-and-sign list for the left part of the look; without one, your name is written in handwriting. For a
   certification, choose what others may do afterwards. **Digital IDs…** opens the Digital IDs window, to make or
   import one.
4. **Sign…** (or **Certify…**) asks where to save the signed document, suggesting `NAME-signed.pdf` beside the
   original, then signs in the background. The signed document opens in place of the one you signed, and is checked
   as any signed document is.

What you sign is the document as it is in MPE, with every change made since you opened it. Those changes go into
the same new revision as your signature, so earlier signatures judge them (and fill-and-sign items are drawn in or
locked, as described below). A document that had no signature fields when it was opened is written out in full
first and then signed; it has no earlier signatures to keep. So is one you applied redactions to or removed hidden
information from, so that nothing removed stays in the file; if it already has signatures, MPE asks you to save it
first (which breaks them, see [Saving a signed document](#saving-a-signed-document)).

**When your changes would show on earlier signatures as not allowed** (a page turned or edited, for example), MPE
stops before writing anything and says which earlier signatures would report which changes. **Sign anyway** goes
ahead after you type the password again; Cancel leaves everything as it was. Earlier signatures stay valid for the
version they signed either way.

The password is used once and then overwritten in memory: after a wrong password, or after the question above, you
type it again. It is never saved or logged. (egui, the app's interface library, may hold copies of typed text in
memory until it is redrawn or MPE closes; none is written anywhere.)

### From the command line

```
mpe sign contract.pdf -o contract-signed.pdf --page 2 --rect 350,600,200,60
mpe sign form.pdf -o form-signed.pdf --field Approver --reason "Approved" --location Ottawa
```

The first puts your signature in a new box on page 2: 200 by 60 points, 350 points from the left and 600 from the
top of the page as shown. The second signs the document's empty signature field named `Approver` (`mpe verify`
lists empty fields by name). MPE asks for the ID's password; `--password-file` reads it from a file instead, and
`--id` chooses another ID than the default (its number or name in `mpe id list`). The ID you sign with becomes the
default. `--reason` and `--location` are your own statements, shown with the signature by every reader. On the
command line, signing adds only the signature (and draws in fill-and-sign items).

### What signing does

The rest applies to signing in the app and from the command line alike.

**What MPE writes**, as Acrobat does by default: an `adbe.pkcs7.detached` signature (CMS) with SHA-256 (SHA-384 for
a P-384 key), carrying your certificate and, for an ID from a certificate authority, the authority certificates
that came with it; the signing time from your computer's clock; the signer's certificate named inside the signed
data (ESS signing-certificate-v2, so a certificate with the same key can't be put in its place); and the signature
field's look. The look is always visible: on the left your name in handwriting (or your own signature: in the
app your saved fill-and-sign signature, on the command line a picture with `--image`; `--style` picks the
handwriting), on the right "Digitally signed by NAME" and "Date:
2026.10.16 14:23:05 -04'00'" in your computer's time zone, as Acrobat shows it. On a turned page the look is turned
with it, so it reads upright.

**Earlier signatures stay valid.** The signature is added after the file's existing bytes, in one new revision,
and nothing before it changes. A second signer's signature shows on the first one as *signed in field …*, an
allowed change.

**Certifying** (Certify… in the app, `--certify` on the command line) is the author's signature, which also says
what others may still do: make no changes (`no-changes`), fill in forms and sign (`forms`), or that plus add
comments (`comments`). A document can be certified only before anyone
signs it, as in Acrobat. After a certification, MPE signs only in the signature fields the document already has
(a new visible signature field is a change the certification doesn't allow), and not at all when it allows no
changes.

**The document's rules are followed.** MPE refuses, and says why, to sign: a field an earlier signer locked; any
field once an earlier signer locked the whole document; a field that is already signed, read-only, or not a
signature field. When the field you sign locks other fields (Acrobat's "Mark as read-only" in the field's
properties), MPE makes those fields read-only and records the lock in the signature, so later changes to them show
as not allowed.

**Fill-and-sign items** (your signature, initials, marks and text put down with them) are drawn into the page
before signing, so they become part of what you sign. On a document someone already signed they are locked in
place instead, because drawing them in would change the page, which the earlier signatures don't allow.

**Forms that ask readers to draw their fields** (a setting some form tools leave on, which makes each reader show
the values its own way) have their fields drawn by MPE and the setting turned off before the first signature, so
every reader shows what you signed.

**MPE checks before writing.** The signed file is checked the way `mpe verify` checks any file: the new signature
must be valid, cover the whole file, name your certificate and have no warnings, and every earlier signature must
check as it did before. Before that, the new revision is read back and must hold exactly what was meant. If any of
this fails, nothing is written. A file whose structure MPE had to repair to open it is written out in full first,
when it has no signatures yet (the message says so); with signatures, MPE refuses rather than break them.

**What you see is what you sign.** A PDF can mark one of its parts as deleted, or keep a copy of a part where the
file's index doesn't list it, and PDF apps don't all treat such parts alike: one shows them, another doesn't. MPE
reads files as the PDF standard says and leaves those parts out. When something in the file still uses such a part,
or MPE can't read a part the way other apps do, apps may show different content, so MPE doesn't add a signature to
the file as it is. With no signatures yet, it writes what it shows out in full first (the message says so), and the
signature covers exactly that in every app; with signatures, it refuses, because writing the file out would break
them. A file that only deletes parts nothing uses any more, as ordinary editing does, is signed as usual.

## Saving a signed document

**Save** and **Save as** keep a signed document's signatures. MPE adds your changes after what was signed, as one
new revision, the way Acrobat saves signed files, and leaves every earlier byte as it was. Each signature stays
valid for the version it signed and lists your changes (see [Changes after signing](#changes-after-signing)), so a
comment or a filled-in field shows as an allowed change. Saving again adds another revision after that one. A
document you haven't changed is saved exactly as it was.

**MPE checks before writing**, as it does when signing: the new revision is read back and must hold exactly your
changes, and every signature must check as it did before. If not, nothing is written.

**When the signatures don't allow your changes** (a page edited, turned, added or deleted; a comment on a document
certified for form filling only; any change to one certified for no changes), MPE asks first, naming each signer
and what their signature will report. **Save anyway** writes it: the signatures stay valid for what was signed, and
every reader shows that the document was changed afterwards in ways the signers didn't allow. Save as a new file to
keep the signed original as it is. A change a signature already reports isn't asked about again.

**When MPE can't add your changes after what was signed**, it says why and asks before writing the whole file
instead (*Break the signatures?*), which breaks every signature in the saved copy. That happens when:

- you applied redactions or removed hidden information: the signed version would still hold everything removed,
  and anyone with the file could open it, so only writing the whole file really removes it;
- the document is encrypted, or you set a password for it (MPE can add a password only by writing the whole file);
- the file is damaged (MPE repaired it to open it), other PDF apps may read it differently from MPE (see [What
  signing does](#what-signing-does)), or MPE can't add to its cross-reference layout reliably (some files from older
  Acrobat versions);
- your changes remove a signature, as flattening the form does;
- the result didn't read back or check correctly, which would be a fault in MPE.

**What you delete stays in the signed version.** A comment you delete, a page you remove, a value you overwrite:
the version that was signed still holds it, and anyone with the file can open that version
([The version that was signed](#the-version-that-was-signed)). Signed PDFs work this way in every editor. To remove
something for good, use Redact or Remove hidden information, which write the whole file and break the signatures.

**Form fields are shown, not redrawn.** When a signed form has fields without a drawing of their own, MPE draws them
on screen and in print but doesn't write those drawings into the document, so saving adds only what you changed.

### From the command line

Every command that edits a document (`mpe set-info`, `mpe stamp`, `mpe form fill`, `mpe annot remove`, `mpe
rotate` and the others) saves a signed input the same way. Where the app would ask, the command writes nothing and
says why, and a flag goes ahead:

- `--accept-disallowed` saves even when the signatures don't allow the changes;
- `--break-signatures` writes the whole file when the changes can't be added after what was signed (the cases
  above), which breaks the signatures. `mpe redact` and `mpe sanitise` always need it on a signed input.

```
mpe form fill signed.pdf -o filled.pdf --set Name="Jane Doe"
mpe rotate signed.pdf -o turned.pdf --pages 1 --degrees 90 --accept-disallowed
mpe redact signed.pdf -o redacted.pdf --text "account number" --break-signatures
```

## What each result means

**Valid: not changed since NAME signed it.** The signed bytes are exactly as signed, the signature matches the
certificate it carries, and the signature is laid out the way the PDF rules require. Whether NAME is who signed
is on the Trust line (above): when nothing in your list vouches for the signer, the status adds *MPE can't confirm
who that is*, as Acrobat says "signer's identity is unknown" before you trust someone.

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

### NOT TRUSTED reasons

A certificate you trust, or one leading to an authority you trust, failed a check:

- *you trust "NAME", but the certificate … wasn't valid at the signing time*: signed before the certificate's
  validity began or after it ended (by the signing time shown).
- *… has the same name as "NAME", which you trust, but is a different certificate*: a self-made ID or authority
  calling itself by a trusted name. Someone may be impersonating them, or they made a new ID; ask them, and trust
  the new one only after comparing its fingerprint.
- *… names "AUTHORITY", which you trust, as its issuer, but the issuer's signature on the certificate doesn't
  match*: a certificate claiming to come from a trusted authority that didn't issue it. Treat it as forged.
- *it was issued under "AUTHORITY", which you trust, but …*: the chain reaches a trusted authority but a link fails:
  an issuer that isn't a certificate authority or may not sign certificates, too many authorities for a path
  length, an expired intermediate, SHA-1 or a short RSA key, name constraints, an unknown critical extension, or no
  certificate policy running through the chain when one is required.
- *the signer's certificate doesn't allow its key to sign* or *is meant for other uses than signing documents*:
  for example a web server's certificate.
- *… has a 1024-bit RSA key*, *repeats an extension*, *has a critical extension MPE doesn't know*.

None of these change the signature's own status: the document may still be unchanged since that key signed it.
They mean MPE can't vouch for who holds the key.

### Signing times

A signing time comes from a **timestamp authority** when the signature carries a valid timestamp; otherwise from
**the signer's computer**, which can be set to any time. Certificates are checked for trust at this time, so
without a timestamp the check relies on the signer's claim of when they signed.

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
| Form field made read-only (signing a field that locks it) | allowed | allowed | allowed | not allowed |
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

Trust was checked on chains made with OpenSSL for the purpose (good chains, and each way a chain can fail), whose
verdicts are cross-checked with `openssl verify` at the signing time with S/MIME signing checks and RFC 5280 policy
processing; they agree on 21 chains. MPE is stricter than OpenSSL by design on four others: name constraints, a
1024-bit RSA authority, a SHA-1 link, and a policy mapping to "any policy" (which RFC 5280 forbids). On the
third-party samples, trusting the authority from a signature's own chain makes 14 of 16 CA-issued signers trusted,
including EU qualified-signature chains that require certificate policies; the other two have SHA-1-signed
certificates, which MPE refuses.

Digital ID files were checked against OpenSSL both ways: MPE imports files OpenSSL makes in each shape other apps
write (OpenSSL 3's default, OpenSSL 1's and Windows' 3DES and RC2 forms, AES-128, PBES2 with 3DES, SHA-512 and
PBMAC1 integrity checks, unencrypted parts, P-256 and P-384 keys, non-ASCII and empty passwords, an ID with its
authority), and Windows' BER encoding, made from those files. OpenSSL reads the files MPE writes, both its own and
exported ones, takes out the same certificate and a key that signs a PDF MPE then checks as valid and, once
trusted, trusted. `openssl x509` shows the extensions described under [making a digital ID](#making-a-digital-id).

MPE's own signatures were checked with `pdfsig` and OpenSSL (`openssl cms -verify` over the signed bytes): RSA-3072,
P-256, P-384 and a CA-issued ID with its chain, a second and third signature on a signed file, a certification, and
a signature in the field a certified document left empty. Both call every one valid, and `pdfsig` finds each
earlier signature valid too. Signed files with classic cross-reference tables, cross-reference streams and object
streams, and pages turned 90 degrees, were checked by MPE.

Saving signed documents was checked on the 53 third-party samples MPE can open with a signature in them. Saved
without changes, or after an undo, nothing is added. With a comment added, every signature checks as before, by
MPE and by `pdfsig` alike, and MPE reports the comment as allowed except where the document's certification or a
signer's lock forbids comments; with a page changed, MPE reports a change not allowed. MPE declines to add to five:
three damaged files, one hybrid file whose objects its PDF library would misread afterwards (see the limitations),
and one whose signature was already broken (its byte range ran past the end of the file). A document MPE signed and
then saved with a comment and a filled-in field is in the `pdfsig` and OpenSSL cross-check above.

Not yet tested: signatures made in Adobe Acrobat by MPE's testers, MPE-made signatures and certifications in
Acrobat, documents signed in Acrobat and saved with changes in MPE (and the other way round), and digital ID files
exported from Acrobat or Windows, or MPE's exported files imported there. Where MPE is stricter than Acrobat (see
the limitations) is to be confirmed in those tests.

## Limitations

- Trust comes from your own list only. MPE doesn't include Adobe's list of trusted authorities (AATL) or the EU's
  trusted lists, so a signature from a commercial or government authority that Acrobat shows as trusted is
  *not confirmed* in MPE until you trust its authority.
- Trust is stricter than Acrobat where safety is in doubt: certificates signed with SHA-1, 1024-bit RSA keys in a
  chain, and authorities that set name constraints aren't trusted. Old documents from authorities that used SHA-1
  show *NOT TRUSTED* with that reason, even though the signature itself is valid.
- Certificates are checked at the signing time, which comes from the signer's computer unless the signature has a
  timestamp, so a signer with a stolen key could claim a time when the certificate was still valid. Revocation
  (below) would catch that; MPE doesn't check it.
- Acrobat's `.fdf` certificate exchange files can't be imported; export the certificate as `.cer` or `.p7c`
  instead.
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
- Files that PDF apps read differently (something uses a part the file deletes or doesn't list, or the file is
  damaged) are shown as the PDF standard says, which other apps may not do. Their signatures show *can't check*
  even where another app calls them valid, and MPE signs such a file only by writing it out in full first, which it
  can't do without breaking signatures already in it (see [What signing does](#what-signing-does)).
- Revocation (whether a certificate was withdrawn) isn't checked. Acrobat checks it online for certificates from
  authorities; MPE makes no network connections.
- Algorithms: RSA (PKCS#1 v1.5 and PSS, up to 8192 bits), ECDSA on P-256 and P-384, with SHA-1, SHA-224, SHA-256,
  SHA-384 and SHA-512. Brainpool curves (used by some European ID cards), P-521, MD5 and RIPEMD-160 can't be
  checked.
- Certificates must be properly DER-encoded; a few old signers wrote them otherwise, and those can't be checked.
- Adobe's Reader-enabling signatures (usage rights, which let the free Reader save forms) aren't listed, as in
  Acrobat.
- Signature fields must be in the document's form; a signature drawn on a page without a field isn't one.
- Saving a signed document adds your changes after what was signed, so whatever you delete stays readable in the
  signed version; redaction and removing hidden information write the whole file instead, which breaks the
  signatures (see [Saving a signed document](#saving-a-signed-document)).
- Encrypted signed documents, damaged ones and those with the older mixed (hybrid) cross-reference layout can be
  saved only by writing the whole file, which breaks their signatures (MPE asks first). For hybrid files, MPE's PDF
  library reads such a layout correctly only while it is the newest part of the file; MPE detects when that would go
  wrong by reading the result back, so some hybrid files do save with their signatures kept.
- Objects MPE writes again are written the way its PDF library writes them: a number stored as `612.0` comes out as
  `612`, and decimals beyond about seven significant digits are rounded. MPE judges numbers by value, so this is no
  change to it; whether Acrobat agrees is to be confirmed in the round trips.
- Page commands (turn, delete, insert, reorder) also rebuild the document's page tree, so on a signed document the
  signature reports "pages added, removed or reordered" as well as the change itself. Both are not allowed after
  signing anyway.
- Each signature's signed version is opened to check it, so MPE opens at most 100 of them, 1 GB together, for one
  file; signatures beyond that show *can't check*, saying why. This keeps a file crafted with very many versions
  from using up the computer's memory; real documents come nowhere near it.
- The app checks the file as opened or last saved, not unsaved changes; after a trust change made with `mpe
  trust`, reopen documents that were already open.
- A file the PDF reader can't open at all can't be checked.
- In the app, a signature goes in a field or a box you choose on the page; it can't be moved or resized afterwards
  (sign again in a new copy instead).
- Signatures carry no timestamp from a timestamp authority yet (planned), so their signing time is your computer's
  claim, and once your certificate expires, checkers can't tell the signature was made while it was valid. No
  long-term validation data is added either.
- Encrypted (password-protected) documents can't be signed.
- The text in a signature's look is in Helvetica, which covers Western European languages; a name in other
  scripts is left out of the text ("Digitally signed"), and a handwriting style that can't show it isn't used.
  Every reader still shows the full name from the certificate.
- A document can be certified only before anyone signs it; after a certification, MPE signs only in signature
  fields the document already has.
- On the command line, signing adds only the signature (and draws in fill-and-sign items); edits made in the app
  are signed along with the signature there.
- Digital IDs on smart cards and USB tokens, and those kept in the Windows certificate store or the macOS Keychain,
  can't be used directly: export them as a `.p12` or `.pfx` file (if the card or store allows it) and import that.
- MPE signs with RSA keys of 2048 to 4096 bits and P-256 and P-384 keys; IDs with other keys (Ed25519, P-521,
  Brainpool, DSA) can't be imported.
- Exported `.p12` files use AES-256 with strong password stretching, which current Windows, macOS and OpenSSL read.
  Some older software only reads the older 3DES encryption, and some may not accept that much stretching; whether
  Acrobat imports MPE's exported files is still to be tested.
- A forgotten password can't be recovered, by MPE or anyone: make a new ID, and give the people who trusted the
  old one your new certificate.
- Add ID files with `mpe id import`, not by copying them into the ID folder: MPE can list an ID without its
  password only when its certificates aren't encrypted, as in the files MPE writes, so most other files show as a
  problem there; and one that MPE can't sign with is refused when it is opened.

## Security notes

Checking and digital IDs use these pure-Rust crates, from the RustCrypto project unless noted, pinned to exact
versions where they are pre-releases:

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
same project and review as the stable crates. MPE reads the signature data with its own small reader, which works
on the bytes as written and never re-encodes what was signed. It reads and writes PKCS#12 ID files with its own
code on the same reader, checked against OpenSSL (see [compatibility](#compatibility)), and writes the signatures
it makes with its own small writer, so the pre-release `cms` crate isn't used.

**Checking** treats every signed file as possibly hostile. Malformed signature data, certificates, dates and
cross-reference sections are refused with a reason, and work is bounded: how deeply data may nest,
how many certificates are tried when building a chain, how many cross-reference sections and objects are read, and
how many signed versions are opened (see the limitations). Text from the file (names, reasons, field names, and the
reasons a check failed) is cleaned before it is shown, in the app and on the command line: line breaks, control
characters (terminal escape codes among them) and invisible characters that reorder text are removed, so a name
can't pose as a different one or rewrite the terminal. `--json` output keeps the text exact, as JSON escapes it.

**What a signature covers** must read the same in every app, or "no changes since" means little. MPE's PDF library
on its own reads some files differently from the PDF standard: it keeps parts a later revision deletes, and takes
copies an object stream holds where the file's index doesn't list them. So every file MPE opens is also read with
MPE's own strict reader of the cross-reference sections, from the newest back to the first, and the two readings
are compared: parts the standard's reading doesn't have are dropped, so MPE shows what the standard says. When
something still uses a dropped part, or MPE can't read a part the way the standard says (the library repaired or
misread it, or couldn't read it at all), apps may show different content; poppler, for one, rebuilds such files by
scanning them and shows the dropped parts. A signature whose signed version is like that shows *can't check*, with
the reason, never *no changes since* (`mpe verify` exits with status 1), and a later revision like that is a change
no signature allows.

**Signing** never hands out a file it hasn't checked: the new revision is read back with MPE's strict reader and
compared object by object with what was meant, and the signed file is then checked like any other (see
[signing](#signing)). The space for the signature is sized by a trial signature first, and only the two
placeholders in the signature's own object are filled in, found where the new revision's cross-reference section
says that object is. The ID's key is used only for the signature and wiped from memory with the ID. Signing and
saving a signed document add to a file only when the comparison above found nothing apps may read differently, and
the new revision must read back the same way; otherwise a file without signatures is written out in full, as MPE
shows it, and one with signatures is refused.

**RUSTSEC-2023-0071**, a timing leak in `rsa` with no fixed version yet, concerns using a private key. Checking
uses only public keys, so it doesn't apply to checking. It applies to signing with an RSA key: an attacker would
have to time many signings on your computer. MPE blinds its RSA signatures, which makes such timing harder. CI runs
`cargo audit` on every change with this one advisory set aside; [releasing](releasing.md) has a step to look for
final versions and new advisories before each release.

**Digital IDs** are as safe as their passwords. In the files MPE writes, the key is encrypted with AES-256, using a
key stretched from your password by PBKDF2-HMAC-SHA256 with 600,000 iterations (OWASP's current figure); the
file's integrity check is HMAC-SHA256 with 1,200,000 iterations of the PKCS#12 derivation, which costs as much to
guess through as the key does. In MPE's own folder the certificates aren't encrypted (they are public, and it lets
MPE list IDs without asking for passwords); in exported files they are, as other apps write them. MPE never stores
a password, writes it to a log, or takes it on the command line; the command line asks for it without showing it,
or reads it from a file you name, and the app shows dots as you type. Keys, passwords and everything derived from
them are wiped from memory once used, in the memory MPE controls; the crates' internal working state, the
interface library's copies of typed text (egui keeps some, for drawing and undo, until they are replaced or MPE
closes), the terminal and the operating system may keep copies MPE can't reach. In the app a password goes to the
background thread that uses it and is overwritten there; it is never kept for a second try. ID files are
written readable by you only (mode 600, in a folder of mode 700, on Linux and macOS), in one step, and are read back
with the password before they are kept; `mpe id list` warns about an ID file other users can read. On Windows the
settings folder is in your own profile. A file being imported can't make MPE work for long: a file asking for more
than 20 million iterations in all, holding more than 256 items, or nested too deeply is refused before the work.

**The trust list** decides whose signatures MPE calls trusted, so anyone who can change `trust.txt` can make MPE
trust any signature. MPE writes it readable and writable by you only (mode 600 on Linux and macOS), replacing it in
one step so it is never half-written, and on Linux and macOS it refuses to use a trust list that other users can
change, saying so (`chmod 600` it to use it again). On Windows the settings folder is in your own profile, which
other users can't change by default. An entry that can't be read, or whose written fingerprint doesn't match its
certificate, is never used, and is kept as written when the list is saved; `mpe trust list` shows the problem. The
certificates a document carries never become trusted by being there.

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
- **"not confirmed"** (amber in the app) for someone you know: compare the SHA-256 fingerprint in Details or `mpe
  verify` with them, then trust them (Trust this signer…, or `mpe trust add FILE --signature N`), or trust their
  organisation's authority.
- **"MPE doesn't have the certificate of …"**: the signature doesn't carry its whole chain. Ask the signer's
  organisation for its authority's certificate and add it with `--authority`.
- **"has the same name as … which you trust, but is a different certificate"**: someone used the name of a
  person or authority you trust. Ask them whether they have a new digital ID before trusting it.
- **"… can be changed by other users of this computer, so MPE doesn't use it"** (shown by `mpe verify`, and at
  the top of the app's Signatures tab, Details and Trusted certificates): the trust list's permissions were
  changed; until fixed, nobody is trusted and the app won't change the list. It is plain text: look through it for entries you didn't add, remove them, then `chmod 600` it.
- **A trusted signer shows "NOT TRUSTED"** after their certificate expired: only signatures made while it was valid
  are trusted. Signatures they make with a new ID need that ID trusted too.
- **"the password is wrong, or the file is damaged"** when importing, exporting or opening an ID: PKCS#12 files
  can't tell the two apart. Check the keyboard layout and Caps Lock; a file made without a password takes an empty
  one (press Enter). If the password is right, the file was changed or damaged in transfer.
- **Forgotten password**: there is no way back in, by design. Make a new ID, and send the people who trusted the old
  one your new certificate. A backup copy (`mpe id export`) only helps if you know its password.
- **"MPE can't sign with this ID: …"** on import: the reason follows (key too short, certificate not for signing,
  …). Ask whoever issued it for an ID meant for signing documents.
- **"… isn't listed: it isn't a digital ID file MPE wrote"** in `mpe id list`: a file was put in the ID folder by
  hand. Move it out and import it with `mpe id import`.
- **An expired ID**: MPE keeps it but won't sign with it. Self-made IDs last five years; make a new one and send
  its certificate to the people who trusted the old one.
- **"the digital ID isn't valid until …"**: the ID's certificate starts later than your computer's clock says it
  is now. Check the clock.
- **"a document can be certified only before anyone signs it"**, **"which allows signing only in the signature
  fields it already has"**, **"locked the field … when signing"**: the document's certification or an earlier
  signer's lock doesn't allow what you asked. Sign an existing empty field, or ask the author for a version that
  allows it.
- **"signing now would make …'s signature report changes it doesn't allow"**: the change listed would show as not
  allowed on that earlier signature, so MPE didn't sign.
- **"the file's cross-reference information is damaged"** on a signed document: MPE won't add to a file it had to
  repair, because the earlier signatures would break. Ask the sender for an undamaged copy.
- **"other PDF apps may read the file differently from MPE"** or **"… the version this signature covers
  differently …"**: the file is built so that apps may show different content (see [What signing
  does](#what-signing-does)). Without signatures, MPE writes it out in full before signing; with signatures, MPE
  can't vouch for them or add to the file. Ask the sender for a copy saved normally.
- **"the signed document didn't check correctly"**, **"the saved document didn't check correctly"** or **"the new
  revision didn't read back correctly"**: MPE's check before writing found a problem, and nothing was written.
  Please report it, with the document if you can.
- **"Save changes the signers didn't allow?"** when saving: your changes are ones the signatures will report as not
  allowed (the list says which). Save as a new file to keep the signed original, or undo the changes.
- **"Break the signatures?"** when saving a signed document: MPE can't add your changes after what was signed, for
  the reason it gives (see [Saving a signed document](#saving-a-signed-document)). Save as a new file to keep the
  signed original. On the command line the same cases stop with the reason and `--break-signatures` to go ahead.
- **"uses the older mixed (hybrid) layout, and MPE can't add to it reliably"**: the file was saved by a program
  (often an older Acrobat) in a way MPE's PDF library misreads once anything is added. Ask for a copy saved by a
  current version, or add your changes in another editor.

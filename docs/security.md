[User guide](README.md) › Everyday tasks

# Password protection

Protect a document with a password, open a protected one, or take the protection off.

## Protect a document

1. Clear any selection (Esc), and open **Password security** in the inspector (or choose ☰ > Document > Password
   security…).
2. Tick **Protect with passwords when saving**.
3. Type an **Open** password, which is needed to open the document, a **Permissions** password, which is needed to
   change what others may do with it, or both.
4. Choose the **Encryption**: 256-bit AES (the default) unless an old PDF reader needs another. Under **Allow**,
   untick what others may not do.
5. Press **Apply**, then save. Save As (Ctrl+Shift+S) keeps the original unprotected.

The passwords take effect when you save. Applying them can be undone with Ctrl+Z, like any other change.

The encryptions are RC4 40-bit and 128-bit (both weak, for old readers only), and AES 128-bit and 256-bit. What
others may be allowed to do: printing, high-resolution printing, modifying, copying content, commenting, filling
form fields and page assembly. These permissions rely on the reader honouring them: other apps may ignore them.

## Open a protected document

Opening a document with an open password asks for it: type it and press Enter (or **Open**). A wrong password asks
again. Opening several protected documents at once asks for each in turn, and **Cancel** leaves only that one
unopened. A document with only a permissions password opens without asking, as it does in other readers, and a
notice says it has one.

**Append PDFs…** asks for a protected file's password in the same way: **Add pages** adds its pages, **Skip this
file** goes on without it, and **Cancel** adds nothing
([Organising pages](organising-pages.md#add-pages-and-merge-documents)).

## Take the protection off

Open the document with its password, and save it: saving removes the protection unless you set it again in Password
security. A notice says so when such a document opens.

## Things to know

- To keep a protected document protected when you save it, set its passwords again in Password security first.
- Password-protected documents get no [recovery copies](gui.md#recovery-copies).
- Digital signatures can't be added to a password-protected document, nor to one set to get a password when saved.
- Certificate encryption is not supported.
- On the command line: `limarepdf-cli encrypt` and `limarepdf-cli decrypt` (see
  [Properties and security](cli.md#properties-and-security)).

## How this was tested

Every cipher is round-trip tested: encrypted, then opened by LimarePDF's document library and its renderer. Files
from the current version have also been checked in poppler (pdftoppm) and Ghostscript.

---

[← Export and printing](export-and-printing.md) · [User guide](README.md) · [Redaction →](redaction.md)

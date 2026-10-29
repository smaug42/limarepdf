[User guide](README.md) › Everyday tasks

# Password protection

Protect a document with a password, open a protected one, save it, or take the protection off. LimarePDF doesn't
keep a document's passwords: saving a protected document you opened removes its protection, unless you set it
again before saving.

## Protect a document

1. Choose **Protect > Passwords** (or ☰ > Document > Password security…). **Password security** opens in the
   inspector.
2. Tick **Protect with passwords when saving**.
3. Type an **Open** password, which is needed to open the document, a **Permissions** password, which is needed to
   change what others may do with it, or both.
4. Choose the **Encryption**: 256-bit AES (the default) unless an old PDF reader needs another. Under **Allow**,
   untick what others may not do.
5. Press **Apply**, then close the tool with the × at the top of the inspector, and save. Save As (Ctrl+Shift+S)
   keeps the original unprotected.

The passwords take effect when you save. Applying them can be undone with Ctrl+Z, like any other change. With
nothing selected, the inspector's **Security** line says how the document will be saved: None, Open password or
Permissions password; for a protected document you opened, Removed on save, until you set a password again. While
passwords are set, the status bar says **Will be encrypted on save**.

The encryptions are RC4 40-bit and 128-bit (both weak, for old readers only), and AES 128-bit and 256-bit. What
others may be allowed to do: printing, high-resolution printing, modifying, copying content, commenting, filling
form fields and page assembly. These permissions rely on the reader honouring them: other apps may ignore them.

## Open a protected document

Opening a document with an open password asks for it: type it and press Enter (or **Open**). A wrong password asks
again. Opening several protected documents at once asks for each in turn, and **Cancel** leaves only that one
unopened. A document with only a permissions password opens without asking, as it does in other readers, and a
notice says it has one.

**Insert PDF pages…** and **Append PDFs…** ask for a protected file's password in the same way: **Add pages** adds
its pages, **Skip this file** goes on without it, and **Cancel** adds nothing
([Organising pages](organising-pages.md#add-pages-and-merge-documents)).

## Save a protected document and keep it protected

A protected document you open is saved **without** its protection unless you set it again: LimarePDF doesn't keep
its passwords or its permissions. A notice says so when it opens, and with nothing selected the inspector's
**Security** line says **Removed on save**. To keep it protected:

1. Choose **Protect > Passwords**, or press **Password security** in the notice. Password security starts empty:
   the document's old settings aren't filled in.
2. Tick **Protect with passwords when saving**, type the passwords (the old ones or new ones), choose the
   encryption, and untick under **Allow** what others may not do.
3. Press **Apply**, then save.

**Save** writes over the file, protected only if you set it again. **Save As** writes a new file the same way and
leaves the original file as it was, still protected.

## Take the protection off

Open the document with its password, and save it: saving removes the protection unless you set it again in Password
security. A notice says so when such a document opens. Save As instead keeps the protected original.

## Things to know

- Password-protected documents get no [recovery copies](gui.md#recovery-copies), and neither does a document you
  add a protected document's pages to.
- Digital signatures can't be added to a password-protected document, nor to one set to get a password when saved.
- A digitally signed document that is protected, or that you set a password on, can be saved only by writing the
  whole file, which breaks its signatures; LimarePDF asks first
  ([Saving a signed document](signatures.md#saving-a-signed-document)).
- Certificate encryption is not supported.
- On the command line: `limarepdf-cli encrypt` and `limarepdf-cli decrypt` (see
  [Properties and security](cli.md#properties-and-security)). Every other command writes its output without a
  password, even from a protected input.

## How this was tested

Every cipher is round-trip tested: encrypted, then opened by LimarePDF's document library and its renderer. Files
from the current version have also been checked in poppler (pdftoppm) and Ghostscript.

---

[← Export and printing](export-and-printing.md) · [User guide](README.md) · [Redaction →](redaction.md)

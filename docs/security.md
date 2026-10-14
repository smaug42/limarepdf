# Password security

MPE opens password-protected PDFs (with the open, or user, password) and can protect documents with:

- an open password, a permissions password, or both;
- RC4 40-bit, RC4 128-bit, AES 128-bit or AES 256-bit encryption;
- permission flags: printing, high-resolution printing, modifying, copying, commenting, filling forms and page
  assembly.

Permission flags are advisory: they rely on the viewer honouring them.

Where: the inspector's Password security section, with nothing selected. The settings apply when you save. Applying
them can be undone with Ctrl+Z, like any other change. On the command line: `mpe encrypt` and `mpe decrypt`.

## Things to know

- Saving a document you opened with a password removes the protection, unless you set it again in Password security.
  The desktop app says so in a notice when such a document opens.
- AES-256 files saved by MPE 0.2.0 and earlier show blank or garbled pages in viewers based on poppler, such as Okular
  and Evince, because of a missing key length. To fix one, open it in MPE, set the password again and save.
- Certificate encryption is not supported.

## How this was tested

Every cipher is round-trip tested: encrypted, then opened by MPE's document library and its renderer. Files from the
current version have also been checked in poppler (pdftoppm) and Ghostscript.

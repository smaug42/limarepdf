# Test documents

Real, public PDFs for the tests that read LimarePDF's output with other programs; not in the binaries.

`Magick++_tutorial.pdf` is "The ImageMagick graphics library: A gentle introduction to Magick++" (Rev 1.0.5, 26
pages, written with OpenOffice.org 3.1), unmodified. `tests/redact.rs` redacts its page 14, which has text, a
picture and drawings, and reads the saved file with Poppler and Ghostscript (group G6 in
`tools/ignored-tests.toml`); its fonts are subsets, so its text isn't in the file as written and only a reader can
tell it's gone. It isn't part of LimarePDF and isn't under LimarePDF's licence. As its page 1 says, it is Copyright
(c) Information Technology Group www.itgroup.ro, (c) Alin Avasilcutei, Cornel Paslariu, Lucian Ungureanu, Virgil
Mager, and licensed under the GNU Free Documentation License, Version 1.2, with no Invariant Sections, no
Front-Cover Texts and no Back-Cover Texts. `GFDL-1.2.txt` is that licence as GNU publishes it
(https://www.gnu.org/licenses/old-licenses/fdl-1.2.txt).

| File | Document | From | Licence | sha256 |
| --- | --- | --- | --- | --- |
| Magick++_tutorial.pdf | A gentle introduction to Magick++, Rev 1.0.5 | ImageMagick's documentation: https://github.com/ImageMagick/ImageMagick6, `www/Magick++/tutorial/Magick++_tutorial.pdf` on `main`; the same file Arch Linux's `imagemagick` 7.1.2.35 installs | `GFDL-1.2.txt` | `ae916f64d810ebd51e2fe373fcd725488e5648848cdb519b41684de32f8a0b5c` |

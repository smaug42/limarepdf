[User guide](README.md) › Reference

# Licence

LimarePDF is dual-licensed under MIT or Apache-2.0, at your option. See [LICENSE-MIT](../LICENSE-MIT) and
[LICENSE-APACHE](../LICENSE-APACHE).

## Included works

- **Icons:** Phosphor Icons 2.1 (MIT, © 2020 Phosphor Icons), its regular and fill fonts included unmodified;
  see `crates/limarepdf-gui/assets/fonts/`. The icon names' code points are from
  [egui-phosphor](https://github.com/amPerl/egui-phosphor) 0.14.0 (MIT or Apache-2.0, © 2023 Romet Tagobert). The
  app icon is Phosphor's file-pdf icon, drawn by `crates/limarepdf-gui/tools/make_icon.py`. Release archives
  include the licence as `FONT-LICENSE-Phosphor.txt`.
- **Interface font:** Inter 4.1 by The Inter Project Authors, SIL Open Font License 1.1, included unmodified; see
  `crates/limarepdf-gui/assets/fonts/`. Release archives include the licence as `FONT-LICENSE-Inter.txt`.
- **Fonts for text typed on the page:** the Liberation fonts 2.1.5 (Sans, Serif and Mono, in four styles
  each) by Red Hat, Inc., with digitized data by Google Corporation, SIL Open Font License 1.1 with the Reserved
  Font Name "Liberation", included unmodified; see `crates/limarepdf-core/assets/fonts/`. They draw text while it
  is typed, and are embedded in PDFs, cut down to the characters used, for text the standard PDF fonts can't
  write; the licence allows embedding in documents, whole or as a subset. The programs store them compressed and
  unpack them, unchanged byte for byte, when first used (a test checks each against its released checksum).
  Release archives include the licence as `FONT-LICENSE-Liberation.txt`.
- **Handwriting fonts** for typed signatures, included unmodified; see `crates/limarepdf-core/assets/fonts/`: Caveat by
  The Caveat Project Authors, Cedarville Cursive and Zeyada by Kimberly Geswein (SIL Open Font License 1.1), and
  Homemade Apple by Font Diner (Apache License 2.0). They are drawn as shapes and never embedded in PDFs. Release
  archives include the licences as `FONT-LICENSE-Caveat.txt`, `FONT-LICENSE-CedarvilleCursive.txt`,
  `FONT-LICENSE-HomemadeApple.txt` and `FONT-LICENSE-Zeyada.txt`.
- **Fonts that come with crates LimarePDF is built with,** included unmodified as the crates ship them:
  - egui's fonts: Hack (MIT, with the Bitstream Vera licence) for monospaced text, and Ubuntu Light (Ubuntu Font
    Licence 1.0) and a few icons cut from emoji-icon-font by John Slegers (MIT) for characters Inter and Phosphor
    lack;
  - Foxit's fonts, © PDFium Authors (BSD-3-Clause), in hayro, for text in the standard PDF fonts when a PDF
    doesn't embed them;
  - on Linux under Wayland, Cantarell 0.05, © Understanding Limited and Jakub Steiner (SIL Open Font License 1.1),
    in sctk-adwaita, for the window title bars.

  Their licences are in `THIRD-PARTY-LICENSES.html` (see [Dependencies](#dependencies)), and Cantarell's in
  `FONT-LICENSE-Cantarell.txt` in the Linux packages.
- **Font metrics:** the glyph widths of the standard PDF fonts (Helvetica, Times, Symbol and ZapfDingbats),
  extracted from Adobe's Core 14 AFM files, © 1985–1997 Adobe Systems Incorporated, which may be used, copied and
  distributed for any purpose with their copyright notices kept. The notices, Adobe's permission notice and a note of
  what was changed are in `crates/limarepdf-core/src/std14.rs`; `crates/limarepdf-core/tools/gen_std14.py` makes the tables from
  the AFM files. Helvetica and Times are trademarks of Linotype-Hell AG and/or its subsidiaries; ITC Zapf Dingbats
  is a registered trademark of International Typeface Corporation.
- **Spelling dictionaries:** SCOWL's English dictionaries 2020.12.07 (Canadian, US, British and
  Australian) by Kevin Atkinson and others, under SCOWL's permissive licence, which asks that its
  copyright and permission notices be kept; and Grammalecte's French dictionary 7.7 by Olivier R.
  and its contributors, under the Mozilla Public License 2.0; all included unmodified, see
  `crates/limarepdf-core/assets/dictionaries/`. They are read by the
  [spellbook](https://github.com/helix-editor/spellbook) crate (MPL-2.0, used unmodified; its
  source is at that address and on crates.io). The programs store the dictionaries compressed
  (about 1.1 MB) and unpack one, unchanged byte for byte, when its language is first checked.
  Release archives include the notices as `DICTIONARY-LICENSE-English.txt` and
  `DICTIONARY-LICENSE-French.txt`, and the MPL-2.0 as `LICENSE-MPL-2.0.txt`.
- **OCR model:** the text recognition model by Robert Knight, from the
  [ocrs](https://github.com/robertknight/ocrs) project, [CC BY-SA 4.0](https://creativecommons.org/licenses/by-sa/4.0/),
  included unmodified; see `crates/limarepdf-core/models/README.md`, shipped as `OCR-MODEL.md` in release archives. With
  its inference code it adds about 14 MB to each binary.

## Test data

These are in the source repository only, never in release archives:

- **Signed sample PDFs** in `crates/limarepdf-core/tests/data/signatures/`: from
  [pyHanko](https://github.com/MatthiasValvekens/pyHanko) (`pyhanko/`, MIT License, © 2020-2026 Matthias
  Valvekens) and [node-signpdf](https://github.com/vbuch/node-signpdf) (`node-signpdf/`, MIT License, © 2019 The
  Authors), unmodified, with made-up identities. The MIT License's permission notice: Permission is hereby
  granted, free of charge, to any person obtaining a copy of this software and associated documentation files (the
  "Software"), to deal in the Software without restriction, including without limitation the rights to use, copy,
  modify, merge, publish, distribute, sublicense, and/or sell copies of the Software, and to permit persons to whom
  the Software is furnished to do so, subject to the following conditions: The above copyright notice and this
  permission notice shall be included in all copies or substantial portions of the Software. THE SOFTWARE IS
  PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF
  MERCHANTABILITY, FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE AUTHORS OR COPYRIGHT
  HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE,
  ARISING FROM, OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE SOFTWARE.
- **Test keys and certificates** in `crates/limarepdf-core/tests/data/signatures/keys/`, and the digital ID files made
  from them in `crates/limarepdf-core/tests/data/signatures/ids/`, made with OpenSSL by
  `crates/limarepdf-core/tools/make-signature-keys.sh`, are LimarePDF's own, with made-up identities; the keys and passwords are
  public and for tests only.
- **Test fonts** in `crates/limarepdf-core/tests/data/fonts/`, for the tests of installed fonts written into
  PDFs: Source Serif 4 by Adobe (with Reserved Font Name "Source") and Cantarell by The Cantarell Authors, SIL Open
  Font License 1.1, included unmodified with their licences; the folder's README says where each is from.
- The EU DSS project's samples (LGPL-2.1) are not included: `crates/limarepdf-core/tools/fetch-dss-samples.sh`
  downloads them for an optional test.

## Dependencies

The Rust crates LimarePDF is built with carry their own licences. Every package (the archives, the Windows
installer, the Linux packages and the Mac app) includes `THIRD-PARTY-LICENSES.html`: each licence text, with the
crates it covers, for the crates built into that platform's programs. It is made with
[cargo-about](https://github.com/EmbarkStudios/cargo-about) from each crate's own licence files, and also holds
the licences of the fonts above and of code that the subsetter crate adapted from ttf-parser and fonttools (MIT),
from its NOTICE file. CI checks every package's page against the crates actually built.

The licences the crates may use are listed in `deny.toml`, and CI checks every crate in `Cargo.lock` against them:

- permissive licences: MIT, Apache-2.0 (with or without the LLVM exception), the BSD licences, 0BSD, BSL-1.0,
  ISC, Unicode-3.0, Unlicense and Zlib;
- for fonts: the SIL Open Font License 1.1 and the Ubuntu Font Licence 1.0;
- the Mozilla Public License 2.0, a file-level copyleft licence, used only by the spellbook crate (see
  [Included works](#included-works)).

Where a crate offers a choice of licences, the page lists one: MIT if offered, otherwise Apache-2.0, otherwise the
next in `about.toml`'s order. One crate, self_cell, offers GPL-2.0 as an alternative to Apache-2.0; Apache-2.0
applies.

The licences are as each crate states them; this page and `THIRD-PARTY-LICENSES.html` aren't legal advice. If you
redistribute LimarePDF, keep `THIRD-PARTY-LICENSES.html` and the other licence files with it.

---

[← Digital signatures in depth](signatures-in-depth.md) · [User guide](README.md) · [Troubleshooting →](troubleshooting.md)

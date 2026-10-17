# Licence

MPE is dual-licensed under MIT or Apache-2.0, at your option. See [LICENSE-MIT](../LICENSE-MIT) and
[LICENSE-APACHE](../LICENSE-APACHE).

## Included works

- **Icons:** Phosphor Icons (MIT, © Phosphor Icons), embedded as a font by `egui-phosphor`.
- **Interface font:** Inter 4.1 by The Inter Project Authors, SIL Open Font License 1.1, included unmodified; see
  `crates/mpe-gui/assets/fonts/`. Release archives include the licence as `FONT-LICENSE-Inter.txt`.
- **Handwriting fonts** for typed signatures, included unmodified; see `crates/mpe-core/assets/fonts/`: Caveat by
  The Caveat Project Authors, Cedarville Cursive and Zeyada by Kimberly Geswein (SIL Open Font License 1.1), and
  Homemade Apple by Font Diner (Apache License 2.0). They are drawn as shapes and never embedded in PDFs. Release
  archives include the licences as `FONT-LICENSE-Caveat.txt`, `FONT-LICENSE-CedarvilleCursive.txt`,
  `FONT-LICENSE-HomemadeApple.txt` and `FONT-LICENSE-Zeyada.txt`.
- **Font metrics:** the glyph widths of the standard PDF fonts (Helvetica, Times, Symbol and ZapfDingbats),
  extracted from Adobe's Core 14 AFM files, © 1985–1997 Adobe Systems Incorporated, which may be used, copied and
  distributed for any purpose with their copyright notices kept. The notices, Adobe's permission notice and a note of
  what was changed are in `crates/mpe-core/src/std14.rs`; `crates/mpe-core/tools/gen_std14.py` makes the tables from
  the AFM files. Helvetica and Times are trademarks of Linotype-Hell AG and/or its subsidiaries; ITC Zapf Dingbats
  is a registered trademark of International Typeface Corporation.
- **OCR model:** the text recognition model by Robert Knight, from the
  [ocrs](https://github.com/robertknight/ocrs) project, [CC BY-SA 4.0](https://creativecommons.org/licenses/by-sa/4.0/),
  included unmodified; see `crates/mpe-core/models/README.md`, shipped as `OCR-MODEL.md` in release archives. With
  its inference code it adds about 14 MB to each binary.

## Test data

These are in the source repository only, never in release archives:

- **Signed sample PDFs** in `crates/mpe-core/tests/data/signatures/`: from
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
- **Test keys and certificates** in `crates/mpe-core/tests/data/signatures/keys/`, and the digital ID files made
  from them in `crates/mpe-core/tests/data/signatures/ids/`, made with OpenSSL by
  `crates/mpe-core/tools/make-signature-keys.sh`, are MPE's own, with made-up identities; the keys and passwords are
  public and for tests only.
- The EU DSS project's samples (LGPL-2.1) are not included: `crates/mpe-core/tools/fetch-dss-samples.sh`
  downloads them for an optional test.

## Dependencies

Dependencies carry their own licences. A scan of the dependency metadata for all platforms found only permissive
licences (MIT, Apache-2.0, BSD, Zlib, Unicode, ISC, Unlicense, BSL-1.0 and similar), plus the SIL OFL 1.1 and Ubuntu
Font Licence for the fonts bundled with egui. A few crates are dual-licensed with LGPL or GPL as one option; the
permissive option applies.

If you redistribute binaries, keep the dependencies' licence texts with them (`cargo about` or `cargo license` can
generate the list).

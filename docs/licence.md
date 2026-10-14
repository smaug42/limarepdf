# Licence

MPE is dual-licensed under MIT or Apache-2.0, at your option. See [LICENSE-MIT](../LICENSE-MIT) and
[LICENSE-APACHE](../LICENSE-APACHE).

## Included works

- **Icons:** Phosphor Icons (MIT, © Phosphor Icons), embedded as a font by `egui-phosphor`.
- **Interface font:** Inter 4.1 by The Inter Project Authors, SIL Open Font License 1.1, included unmodified; see
  `crates/mpe-gui/assets/fonts/`. Release archives include the licence as `FONT-LICENSE-Inter.txt`.
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

## Dependencies

Dependencies carry their own licences. A scan of the dependency metadata for all platforms found only permissive
licences (MIT, Apache-2.0, BSD, Zlib, Unicode, ISC, Unlicense, BSL-1.0 and similar), plus the SIL OFL 1.1 and Ubuntu
Font Licence for the fonts bundled with egui. A few crates are dual-licensed with LGPL or GPL as one option; the
permissive option applies.

If you redistribute binaries, keep the dependencies' licence texts with them (`cargo about` or `cargo license` can
generate the list).

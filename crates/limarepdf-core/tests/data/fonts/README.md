# Test fonts

Fonts for the tests of installed fonts written into PDFs (`src/embed.rs`, `src/installed_fonts.rs`,
`tests/unicode_text.rs`, and the app's tests); not in the binaries. Source Serif 4 and Cantarell are unmodified and
licensed under the SIL Open Font License 1.1. The TrueType variable font the tests use is the bundled Caveat
(`assets/fonts/`). Limare Emoji Test is made up for LimarePDF's emoji tests by `tools/make_emoji_test_font.py`
(fontTools), the same file on every run, and is part of LimarePDF under its own licence: plain boxes for the Latin
letters and digits, 😀 with an outline, 😎 drawn by colour layers only, and a glyph for each part of the emoji
sequences the tests refuse.

| File | Font | Kind | From | Licence | sha256 |
| --- | --- | --- | --- | --- | --- |
| SourceSerif4-Regular.otf | Source Serif 4 by Adobe | OpenType CFF, static | https://github.com/adobe-fonts/source-serif, `OTF/SourceSerif4-Regular.otf` on the `release` branch at commit `80d3f889` | `OFL-SourceSerif4.md` | `edf160d0d584deee8a3bb2c3371b2a7624ca63580fbe02c57c1f4c91e84d8787` |
| Cantarell-VF.otf | Cantarell by The Cantarell Authors | OpenType CFF2, variable (weight 100 to 800) | Arch Linux's `cantarell-fonts` 0.311 | `OFL-Cantarell.txt` | `712db58a8b471fdcf205aeae1fb1fd0555f242bf8929cc7791082cdf00d5eec2` |
| LimareEmojiTest.ttf | Limare Emoji Test, made up | TrueType, static, with `COLR` and `CPAL` | `tools/make_emoji_test_font.py` (fontTools 4.67) | LimarePDF's (MIT or Apache-2.0) | `2d90b7face8da38186ae3362ec705e8acae641f26a362c2e1efa83551f268df6` |

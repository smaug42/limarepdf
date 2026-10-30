# Test fonts

Fonts for the tests of installed fonts written into PDFs (`src/embed.rs`, `src/installed_fonts.rs`,
`tests/unicode_text.rs`, and the app's tests); not in the binaries. Source Serif 4 and Cantarell are unmodified and
licensed under the SIL Open Font License 1.1. The TrueType variable font the tests use is the bundled Caveat
(`assets/fonts/`). Limare Emoji Test is made up for LimarePDF's emoji tests by `tools/make_emoji_test_font.py`
(fontTools), the same file on every run, and is part of LimarePDF under its own licence: plain boxes for the Latin
letters and digits, 😀 with an outline, 😎 drawn by colour layers only, and a glyph for each part of the emoji
sequences the tests refuse. Limare Han Test is made up for the text pipeline's tests (`tests/text_pipeline.rs`,
NEW-02) by `tools/make_han_test_font.py` (fontTools), the same file on every run, and is part of LimarePDF under its
own licence: plain boxes for a few Chinese, Japanese and Korean characters and no Latin letters but A, so text in
other fonts falls back to it on any machine, and one glyph Latin A and Cyrillic А share. Limare Own Encoding and
Limare Symbolic are made up for the tests of fonts' own
encodings (`tests/own_encoding.rs`, PB-23) by `tools/make_encoding_test_fonts.py` (fontTools), the same files on
every run, and are part of LimarePDF under its own licence: plain boxes named as TeX's OT1 fonts place them
(quotedblright at 0x22, endash at 0x7B, the ff and fi ligatures at 0x0B and 0x0C).

| File | Font | Kind | From | Licence | sha256 |
| --- | --- | --- | --- | --- | --- |
| LimareOwnEncoding.pfa | Limare Own Encoding, made up | Type 1 with its own encoding, as a PDF's `/FontFile` holds it | `tools/make_encoding_test_fonts.py` (fontTools 4.67) | LimarePDF's (MIT or Apache-2.0) | `9da140f92ba764345f9aea0ac80778f3d00686ed83e4a9d26164e9447f4bc11d` |
| LimareOwnEncoding.cff | Limare Own Encoding, made up | bare CFF with its own encoding (`/FontFile3` `/Type1C`) | `tools/make_encoding_test_fonts.py` (fontTools 4.67) | LimarePDF's (MIT or Apache-2.0) | `758cd2c0c3f92c4a3327c0f808e4833260811099595bc678cabd4c7837aa4b09` |
| LimareSymbolic.ttf | Limare Symbolic, made up | TrueType, symbolic: (3,0) and (1,0) cmaps, glyph names in `post` | `tools/make_encoding_test_fonts.py` (fontTools 4.67) | LimarePDF's (MIT or Apache-2.0) | `d1ebd921a2aeb8f926204895f631d1870e630a7d1b367cd3237eea3f1ad9de2b` |
| SourceSerif4-Regular.otf | Source Serif 4 by Adobe | OpenType CFF, static | https://github.com/adobe-fonts/source-serif, `OTF/SourceSerif4-Regular.otf` on the `release` branch at commit `80d3f889` | `OFL-SourceSerif4.md` | `edf160d0d584deee8a3bb2c3371b2a7624ca63580fbe02c57c1f4c91e84d8787` |
| Cantarell-VF.otf | Cantarell by The Cantarell Authors | OpenType CFF2, variable (weight 100 to 800) | Arch Linux's `cantarell-fonts` 0.311 | `OFL-Cantarell.txt` | `712db58a8b471fdcf205aeae1fb1fd0555f242bf8929cc7791082cdf00d5eec2` |
| LimareEmojiTest.ttf | Limare Emoji Test, made up | TrueType, static, with `COLR` and `CPAL` | `tools/make_emoji_test_font.py` (fontTools 4.67) | LimarePDF's (MIT or Apache-2.0) | `2d90b7face8da38186ae3362ec705e8acae641f26a362c2e1efa83551f268df6` |
| LimareHanTest.ttf | Limare Han Test, made up | TrueType, static, no Latin letters but A | `tools/make_han_test_font.py` (fontTools 4.67) | LimarePDF's (MIT or Apache-2.0) | `fe8ed5bf167da8cca0b1992eb118648a33b43361be187872fd06b9da81d538df` |

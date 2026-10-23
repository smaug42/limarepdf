# Test fonts

Fonts for the tests of installed fonts written into PDFs (`src/embed.rs`, `src/installed_fonts.rs`,
`tests/unicode_text.rs`); not in the binaries. Both are unmodified and licensed under the SIL Open Font License
1.1. The TrueType variable font the tests use is the bundled Caveat (`assets/fonts/`).

| File | Font | Kind | From | Licence | sha256 |
| --- | --- | --- | --- | --- | --- |
| SourceSerif4-Regular.otf | Source Serif 4 by Adobe | OpenType CFF, static | https://github.com/adobe-fonts/source-serif, `OTF/SourceSerif4-Regular.otf` on the `release` branch at commit `80d3f889` | `OFL-SourceSerif4.md` | `edf160d0d584deee8a3bb2c3371b2a7624ca63580fbe02c57c1f4c91e84d8787` |
| Cantarell-VF.otf | Cantarell by The Cantarell Authors | OpenType CFF2, variable (weight 100 to 800) | Arch Linux's `cantarell-fonts` 0.311 | `OFL-Cantarell.txt` | `712db58a8b471fdcf205aeae1fb1fd0555f242bf8929cc7791082cdf00d5eec2` |

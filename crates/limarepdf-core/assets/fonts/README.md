# Handwriting fonts

Used to draw typed signatures (`src/sign.rs`), as glyph outlines; they are never embedded in a PDF. All are
unmodified, from the Google Fonts repository (https://github.com/google/fonts). Caveat, Cedarville Cursive and
Zeyada are licensed under the SIL Open Font License 1.1; Homemade Apple under the Apache License 2.0.

| File | Font | From | Licence | sha256 |
| --- | --- | --- | --- | --- |
| Caveat.ttf | Caveat (variable, drawn at its default weight) by The Caveat Project Authors | `ofl/caveat/Caveat[wght].ttf` at commit `a85fc09e` (renamed only) | `OFL-Caveat.txt` | `0bdb6b660482d31531b3945849fba5916b3ef8695da7024a9e6b9ee3c4157988` |
| Cedarville-Cursive.ttf | Cedarville Cursive by Kimberly Geswein | `ofl/cedarvillecursive/Cedarville-Cursive.ttf` at commit `8ccda7bf` | `OFL-CedarvilleCursive.txt` | `5f86d71d08a68605d5912b9bcb639a060ce9c0debd7e0b715f5e268d4a4bc317` |
| HomemadeApple-Regular.ttf | Homemade Apple by Font Diner | `apache/homemadeapple/HomemadeApple-Regular.ttf` at commit `8e94f9c1` | `LICENSE-HomemadeApple.txt` | `dd1baaca3cde1b1f8415aed3b0aea6808655c9d9ca5a99c7282d9accc16c1a58` |
| Zeyada.ttf | Zeyada by Kimberly Geswein | `ofl/zeyada/Zeyada.ttf` at commit `90abd17b` | `OFL-Zeyada.txt` | `09f23d0d78b6e166ddb8480793bb550ab4c2aaf6602eda47a394fee93d2a9667` |

They are embedded in `limarepdf-core` with `include_bytes!`.

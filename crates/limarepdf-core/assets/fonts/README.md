# Fonts

## Handwriting fonts

Used to draw typed signatures (`src/sign.rs`), as glyph outlines; they are never embedded in a PDF. All are
unmodified, from the Google Fonts repository (https://github.com/google/fonts). Caveat, Cedarville Cursive and
Zeyada are licensed under the SIL Open Font License 1.1; Homemade Apple under the Apache License 2.0.

| File | Font | From | Licence | sha256 |
| --- | --- | --- | --- | --- |
| Caveat.ttf | Caveat (variable, drawn at its default weight) by The Caveat Project Authors | `ofl/caveat/Caveat[wght].ttf` at commit `a85fc09e` (renamed only) | `OFL-Caveat.txt` | `0bdb6b660482d31531b3945849fba5916b3ef8695da7024a9e6b9ee3c4157988` |
| Cedarville-Cursive.ttf | Cedarville Cursive by Kimberly Geswein | `ofl/cedarvillecursive/Cedarville-Cursive.ttf` at commit `8ccda7bf` | `OFL-CedarvilleCursive.txt` | `5f86d71d08a68605d5912b9bcb639a060ce9c0debd7e0b715f5e268d4a4bc317` |
| HomemadeApple-Regular.ttf | Homemade Apple by Font Diner | `apache/homemadeapple/HomemadeApple-Regular.ttf` at commit `8e94f9c1` | `LICENSE-HomemadeApple.txt` | `dd1baaca3cde1b1f8415aed3b0aea6808655c9d9ca5a99c7282d9accc16c1a58` |
| Zeyada.ttf | Zeyada by Kimberly Geswein | `ofl/zeyada/Zeyada.ttf` at commit `90abd17b` | `OFL-Zeyada.txt` | `09f23d0d78b6e166ddb8480793bb550ab4c2aaf6602eda47a394fee93d2a9667` |

## Liberation

The `Liberation*.ttf` files are the Liberation fonts 2.1.5 (Red Hat, Inc., digitized data by
Google Corporation; https://github.com/liberationfonts/liberation-fonts), unmodified, from the
release archive `liberation-fonts-ttf-2.1.5.tar.gz`
(sha256 `7191c669bf38899f73a2094ed00f7b800553364f90e2637010a69c0e268f25d0`). They have the widths
of Helvetica, Times and Courier. The app draws text being typed on the page in them, and
LimarePDF embeds them in PDFs, cut down to the characters used, for text the standard fonts can't
write (`src/embed.rs`). They are licensed under the SIL Open Font License 1.1 with the Reserved
Font Name "Liberation"; see `OFL-Liberation.txt` (the release's `LICENSE`). Because of the
Reserved Font Name, the files here must stay unmodified to keep their name. Embedding them in a
document, whole or as a subset, is allowed and is not a modification under the licence (OFL FAQ
1.12: "either in full or a subset ... The restrictions regarding font modification and
redistribution do not apply").

| File | sha256 |
| --- | --- |
| LiberationMono-Bold.ttf | `bd62a0672d0b9b6710b01df434c80ad54fa5f0835207eb7b17b7a761463067bb` |
| LiberationMono-BoldItalic.ttf | `79451f3c09fe25116098853b7a2ca6e2436220ccc11af022979adbcf195be130` |
| LiberationMono-Italic.ttf | `605c01c711b44480a7508d349dfbf3264e81fa43d69e61cfa7d10b86e764c4d1` |
| LiberationMono-Regular.ttf | `f2b83c763e8afd21709333370bed4774337fae82267937e2b5aea7e2fbd922c1` |
| LiberationSans-Bold.ttf | `788abee4c806d660e8aee46689dd8540cd4bb98da03dcc9d171ce3efd99a9173` |
| LiberationSans-BoldItalic.ttf | `698da70fc191cc5f33ad4d6d3fe830fe4624b898ea2e3169955928b7c491f1ee` |
| LiberationSans-Italic.ttf | `e5bae5c4cde31f22142753855f4f8fb86da6ff39955ed3c0a11248b0d16948b0` |
| LiberationSans-Regular.ttf | `76d04c18ea243f426b7de1f3ad208e927008f961dc5945e5aad352d0dfde8ee8` |
| LiberationSerif-Bold.ttf | `d754ba427cfe0bca54ae052384baa8f842da5bd6550ad4da024ac441e7a7d5ce` |
| LiberationSerif-BoldItalic.ttf | `f17db8af71e24d2066b587546021d4f0b296be389512b658dec3c09affeb11a7` |
| LiberationSerif-Italic.ttf | `0e3dea9f8d613e006ccfa62201f33e265d19167bd0907725c3e145368b04fc2e` |
| LiberationSerif-Regular.ttf | `058ea80864aef09a23f45cbec2bb5400bc3dfbdea01c3f10538a21fcb497fb74` |

All are embedded in `limarepdf-core` with `include_bytes!` (the handwriting fonts in `src/sign.rs`,
Liberation in `src/embed.rs`). The Liberation files are stored compressed (about 2.4 MB instead of
4.4 MB): `build.rs` packs each with zlib, and `src/embed.rs` unpacks it the first time it is used,
byte for byte as it is here; a test checks each unpacked face against the checksum above. The files
here stay as released.

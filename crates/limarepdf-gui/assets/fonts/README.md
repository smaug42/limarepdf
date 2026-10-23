# Fonts

`Inter-Regular.ttf`, `Inter-Medium.ttf` (dialog titles) and `Inter-SemiBold.ttf` are Inter 4.1 by
The Inter Project Authors (https://github.com/rsms/inter), unmodified, from `extras/ttf/` in the release archive
`Inter-4.1.zip` (sha256 `9883fdd4a49d4fb66bd8177ba6625ef9a64aa45899767dde3d36aa425756b11e`).
They are licensed under the SIL Open Font License 1.1; see `OFL.txt`.

| File | sha256 |
| --- | --- |
| Inter-Regular.ttf | `40d692fce188e4471e2b3cba937be967878f631ad3ebbbdcd587687c7ebe0c82` |
| Inter-Medium.ttf | `97ad806f526e41546d46365bb3a393145f75b7b1568913db74549ad8b8dba872` |
| Inter-SemiBold.ttf | `78a843fade9d4612a5567302fb595b56976eb5fcebf4fea5a5912d638bafcde3` |

The `Liberation*.ttf` files are the Liberation fonts 2.1.5 (Red Hat, Inc., digitized data by
Google Corporation; https://github.com/liberationfonts/liberation-fonts), unmodified, from the
release archive `liberation-fonts-ttf-2.1.5.tar.gz`
(sha256 `7191c669bf38899f73a2094ed00f7b800553364f90e2637010a69c0e268f25d0`). They draw text
being typed on the page, as they have the widths of Helvetica, Times and Courier. They are
licensed under the SIL Open Font License 1.1 with the Reserved Font Name "Liberation"; see
`OFL-Liberation.txt` (the release's `LICENSE`). Because of the Reserved Font Name, they must stay
unmodified (no subsetting) to keep their name.

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

They are embedded in `limarepdf` with `include_bytes!` (Inter in `src/theme.rs`, Liberation in
`src/typing_fonts.rs`).

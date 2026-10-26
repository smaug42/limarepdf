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

`Phosphor.ttf` (regular) and `Phosphor-Fill.ttf` are Phosphor Icons 2.1 (https://phosphoricons.com), unmodified,
from `res/` in the crate egui-phosphor 0.14.0. They are licensed under the MIT License; see `LICENSE-Phosphor.txt`
(the licence the fonts name, https://raw.githubusercontent.com/phosphor-icons/homepage/master/LICENSE).
`src/icon.rs` names the code points the app uses.

| File | sha256 |
| --- | --- |
| Phosphor.ttf | `06b91e022b7ee899a63efced879392a74f0bacbda54e4467e9f663220d173a10` |
| Phosphor-Fill.ttf | `a53f5d2630cab5e3b7536ecb9d69d71519a2190298c22b1f8d770dd37bc2940a` |

They are all embedded in `limarepdf` with `include_bytes!` (`src/theme.rs`).

The Liberation fonts that draw text being typed on the page are in `limarepdf-core`
(`crates/limarepdf-core/assets/fonts/`), which also embeds them in PDFs; the app uses core's copy.

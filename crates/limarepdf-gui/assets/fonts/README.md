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

They are embedded in `limarepdf` with `include_bytes!` (`src/theme.rs`).

The Liberation fonts that draw text being typed on the page are in `limarepdf-core`
(`crates/limarepdf-core/assets/fonts/`), which also embeds them in PDFs; the app uses core's copy.

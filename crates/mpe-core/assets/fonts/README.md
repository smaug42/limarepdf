# Handwriting fonts

Used to draw typed signatures (`src/sign.rs`), as glyph outlines; they are never embedded in a PDF. Both are
unmodified, from the Google Fonts repository (https://github.com/google/fonts), and licensed under the SIL Open
Font License 1.1.

| File | Font | From | Licence | sha256 |
| --- | --- | --- | --- | --- |
| DancingScript.ttf | Dancing Script (variable, drawn at its default weight) by The Dancing Script Project Authors | `ofl/dancingscript/DancingScript[wght].ttf` at commit `f8dc91f6` (renamed only) | `OFL-DancingScript.txt` | `21808625578fe8d8cd10cb684be546dca077b27cd03a53a2f1ec11dc743c924c` |
| GreatVibes-Regular.ttf | Great Vibes by The Great Vibes Pro Project Authors | `ofl/greatvibes/GreatVibes-Regular.ttf` at commit `a6039f38` | `OFL-GreatVibes.txt` | `8d509802186f1b51572531ecf313e8098f9a5bfdfaca93f0c9b34467f9982d15` |

They are embedded in `mpe-core` with `include_bytes!`.

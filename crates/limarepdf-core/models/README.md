# Bundled OCR model

`text-recognition.rten` is the text recognition model of the
[ocrs](https://github.com/robertknight/ocrs) OCR engine, by Robert Knight. It is built into
`limarepdf-core` and used for OCR.

- Source: <https://huggingface.co/robertknight/ocrs>, file
  `text-rec-checkpoint-s52qdbqt.rten`, revision `df0edd170279ab971b53e094c627255a87e1a503`
  (last modified 2024-01-30)
- SHA-256: `606d9a0414c6b73c99df75b707c11c70d1c8b12e1d4f900922e185fc37bfca65`
- Licence: [Creative Commons Attribution-ShareAlike 4.0 International (CC BY-SA 4.0)](https://creativecommons.org/licenses/by-sa/4.0/)
- Changes: none. The file is distributed exactly as published (only renamed).

This licence covers the model file only. The rest of LimarePDF is MIT OR Apache-2.0. If you modify
the model (for example by retraining or quantising it), the modified model must also be
shared under CC BY-SA 4.0.

The ocrs text *detection* model is not included: no licence is stated for the version that
works. LimarePDF finds words with its own image processing instead (see `src/ocr.rs`).

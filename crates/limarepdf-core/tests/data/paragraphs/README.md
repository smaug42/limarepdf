# Paragraph answer sets

The answers the opt-in paragraph check judges paragraph detection by (`tests/paragraph_check.rs`, NEW-01; run with
`cargo test -p limarepdf-core --test paragraph_check -- --ignored paragraph_check`). Made in batch 44 and written
here in batch 45; the scripts that made them are outside the repository (they need LibreOffice, Chromium and
groff).

- `madeup/`: 57 made-up PDFs, 292 pages, from LibreOffice, Chromium and groff, every block's text known: plain
  text, columns, headings, lists, quotes, footnotes, tables, captions, letters, forms, styled words, centred and
  right-aligned text, Greek, Cyrillic, Chinese and Japanese. Their words are made up. They embed subsets of free
  fonts (Noto Sans CJK, Liberation, Cantarell, DejaVu, Fira Sans, URW's P052) as any PDF made with them does;
  groff's use the standard fonts, not embedded.
- `madeup.tsv`: each made-up file's answers.
- `corpus.tsv`: the answers for the 32 render-corpus files with usable paragraph tags, read from their tags. The
  corpus isn't in the repository (`tools/fetch-render-samples.sh`); without it the check judges the made-up set
  alone and says so. No text of the corpus files is here, only where its lines are.

Each answer file is lines of tab-separated fields:

| Record | Fields | Meaning |
| --- | --- | --- |
| `F` | name, half, pages, tags | a file: `tune` or `test` (the half its figures count in), its pages, `1` when its tags are usable |
| `P` | part, kind, page, rows | an answer part: a block's lines on one page and in one column, its kind (`paragraph`, `list-item`, `quote`, `footnote`, `heading`, `cell` and others) and how many rows it has |
| `L` | page, left, top, right, bottom, baseline, size, answer | a line as batch 44's LimarePDF read it, in points on the displayed page (y down; pages from 0), and its part: `-` for none (no letters, or not in a block), `X` for one left out (a line holding two blocks' text, a line of a made-up block flagged as unclear, or of a tag element holding several paragraphs) |
| `X` | element, page, rows | set 1: a tag element holding several paragraphs or a list, its lines left out of the figures and listed |

The halves are fixed by the file name (the first byte of its SHA-1 even: tuning; `pdfjs-issue1133` tuning), so the
detector's constants are tuned on one half and judged on the other.

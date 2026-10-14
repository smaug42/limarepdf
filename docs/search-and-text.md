# Search and text

## Search

Search lists every match with a snippet. Matches are highlighted on the page, and the view scrolls to the current one.
On the command line, `mpe search --boxes` also prints where each match is on its page.

Limits:

- Highlights are placed using each character's width in the font. When a file uses one of the standard PDF fonts
  without including its widths, MPE uses Adobe's published widths for Helvetica, Times and Courier; for Symbol and
  ZapfDingbats it estimates them, and highlights there can be off.
- If the characters of a piece of text cannot be matched to the font's glyphs, that piece falls back to spacing its
  characters evenly, and its highlights can be a letter or two off.
- A phrase broken across lines is listed but not highlighted.
- Scanned pages have no text until you [recognise it](scanning-and-ocr.md#recognising-text-ocr).

Highlight placement was checked on a real scan, on documents made by LaTeX and Sphinx with embedded fonts, and on
one made by Ghostscript with the standard Times font and no widths; in all of them the highlights sit on the
matching words.

## Copying text

See [Using the desktop app](gui.md#copying-text). Selected words are copied with spaces between words and line
breaks between lines. Word boxes are placed the same way as search highlights, with the same limits.

## Text extraction

Text comes from the document library, lopdf. It can miss text in some CID and embedded-font encodings. Export text,
`mpe text`, search and copying all use it.

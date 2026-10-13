# Search and text

## Search

Search lists every match with a snippet. Matches are highlighted on the page, and the view scrolls to the current one.
On the command line, `mpe search --boxes` also prints where each match is on its page.

Limits:

- Highlights are placed by character position within each piece of text, so in proportional fonts they can be a
  letter off.
- A phrase broken across lines is listed but not highlighted.
- Scanned pages have no text until you [recognise it](scanning-and-ocr.md#recognising-text-ocr).

Highlight placement was checked on a real scan; drawing the highlights in the desktop app was not clicked through in
testing.

## Copying text

See [Using the desktop app](gui.md#copying-text). Selected words are copied with spaces between words and line
breaks between lines. Word boxes are placed by character position, so a selection highlight can be a letter off in
proportional fonts.

## Text extraction

Text comes from the document library, lopdf. It can miss text in some CID and embedded-font encodings. Export text,
`mpe text`, search and copying all use it.

# Search and text

## Search

Search lists every match with a snippet. Matches are highlighted on the page, and the view scrolls to the current one.
On the command line, `mpe search --boxes` also prints where each match is on its page.

Limits:

- A phrase can run across a line break: search treats the break as a space and highlights the part on each line.
  A word hyphenated at the end of a line is not found as one word.
- Scanned pages have no text until you [recognise it](scanning-and-ocr.md#recognising-text-ocr).

Highlights are placed using each character's width in the font. When a file uses one of the standard PDF fonts
without including its widths, MPE uses Adobe's published widths for them. Placement was checked on a real scan and
on documents made by LaTeX, Sphinx, Ghostscript and LibreOffice; in all of them the highlights sit on the matching
words.

## Copying text

See [Using the desktop app](gui.md#copying-text). Selected words are copied with spaces between words and line
breaks between lines. Word boxes are placed the same way as search highlights, with the same limits.

## Text extraction

MPE reads the text from the page content itself, the same way for export text, `mpe text`, search, highlights and
copying, so they always agree.

- **Characters:** taken from the font's ToUnicode map, which says what text each character code stands for, and
  otherwise from its encoding. Ligatures such as "ﬁ" come out as their letters, so searching for "find" finds it.
- **Spaces:** a gap between words is a space, whether the file has a space character there or only moves the next
  word along (as justified text and many PDF makers do).
- **Order:** words come in reading order worked out from where they sit on the page, not the order the file draws
  them in (forms and many other files draw blocks out of order, such as a footer first). Blocks separated by space
  are read top to bottom; columns of text a column at a time; anything else row by row, left to right. Lines are
  separated by line breaks. Narrow columns, such as the page numbers of a table of contents, stay with their rows.
  Tables with wide columns come out a column at a time.

Limits:

- Fonts with no ToUnicode map and a custom encoding (some Type 3 fonts, and some fonts made by old software) can
  give the wrong characters.
- CID fonts whose codes are not two bytes long (some older Chinese, Japanese and Korean encodings) can come out
  wrong.
- Highlights are placed using the order the file draws text in. A match that runs from one block into another the
  file draws elsewhere is listed, but not highlighted.
- Text inside form XObjects (reusable blocks of page content, which some PDF makers put all their text in) is read,
  searched and selected, but cannot be edited line by line: the form moves, resizes and deletes as one object.

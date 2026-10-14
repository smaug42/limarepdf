# Search and text

## Search

Search lists every match with a snippet. Matches are highlighted on the page, and the view scrolls to the current one.
On the command line, `mpe search --boxes` also prints where each match is on its page.

Limits:

- A phrase broken across lines is listed but not highlighted.
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
- **Order:** words come in the order the page draws them, which is usually reading order; lines are separated by
  line breaks. Text in multi-column layouts can come out a column at a time or interleaved, depending on the file.

Limits:

- Fonts with no ToUnicode map and a custom encoding (some Type 3 fonts, and some fonts made by old software) can
  give the wrong characters.
- CID fonts whose codes are not two bytes long (some older Chinese, Japanese and Korean encodings) can come out
  wrong.
- Text inside form XObjects (reusable blocks of page content) is not read.

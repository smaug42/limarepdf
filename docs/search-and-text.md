# Search and text

## Search

Search lists every match with a snippet. Matches are highlighted on the page, and the view scrolls to the current one.
On the command line, `limarepdf-cli search --boxes` also prints where each match is on its page.

Limits:

- A phrase can run across a line break: search treats the break as a space and highlights the part on each line.
  A word hyphenated at the end of a line is found whole, with or without its hyphen: "information" and
  "self-employed" find "infor-" + "mation" and "self-" + "employed". Soft hyphens are ignored.
- Scanned pages have no text until you [recognise it](scanning-and-ocr.md#recognising-text-ocr).

Highlights are placed using each character's width in the font. When a file uses one of the standard PDF fonts
without including its widths, LimarePDF uses Adobe's published widths for them. Placement was checked on a real scan and
on documents made by LaTeX, Sphinx, Ghostscript and LibreOffice; in all of them the highlights sit on the matching
words.

## Copying text

See [Using the desktop app](gui.md#copying-text). Selected words are copied with spaces between words and line
breaks between lines. Word boxes are placed the same way as search highlights, with the same limits.

## Text extraction

LimarePDF reads the text from the page content itself, the same way for export text, `limarepdf-cli text`, search, highlights and
copying, so they always agree.

- **Characters:** taken from the font's ToUnicode map, which says what text each character code stands for, and
  otherwise from its encoding. Ligatures such as "ﬁ" come out as their letters, so searching for "find" finds it.
  Chinese, Japanese and Korean fonts without a ToUnicode map (common in files whose fonts aren't embedded) are read
  through their character map, as pdf.js does: a Unicode one (such as `UniJIS-UCS2-H`) gives the text directly;
  others (such as Shift-JIS `90ms-RKSJ-H`) give character numbers, which Adobe's tables for Japanese, Simplified and
  Traditional Chinese and Korean turn into text. Their codes are split as the map says, one to four bytes each (a
  code the map leaves out takes as many bytes as the map's code ranges say, as poppler and pdf.js read it), and
  their widths found by character number, so highlights and selections sit on the right characters.
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
- CID fonts with no ToUnicode map whose character map can't be read, or that use their own character numbers
  rather than Adobe's tables, come out as "�" or the wrong characters. A file that names Adobe's tables but numbers
  its characters otherwise reads wrong in pdf.js too.
- After a code a font's character map leaves out, LimarePDF draws the rest of that piece of text further along
  than it reads it (see [Rendering](rendering.md#known-to-be-wrong)), so highlights there sit where other readers
  put the characters, not on the drawn ones.
- A page with tens of thousands of words spaced out along one line, which only a crafted file has, takes seconds to
  read (50,000 words: 4.5 s), as its columns are worked out one at a time. Tens of thousands of separate lines are
  quick (50,000: 0.2 s).
- A ToUnicode map is read for its first million or so characters and up to 32 MB of text, so a crafted file can't
  ask for gigabytes; a character whose text is longer than the 512 bytes the PDF specification allows is read from
  the font's encoding instead. Real maps are far smaller: the longest text for one character in the 2,079 maps of
  the test files is 42 bytes.
- Text inside form XObjects (reusable blocks of page content, which some PDF makers put all their text in) is read,
  searched and selected, but cannot be edited line by line: the form moves, resizes and deletes as one object.

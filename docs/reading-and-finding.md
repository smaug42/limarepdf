[User guide](README.md) › Everyday tasks

# Reading and finding

Move through a document, find words in it, follow its bookmarks and links, and copy its text. Shortcuts are
written with Ctrl; on a Mac, use ⌘ (Command) instead.

## Move around the pages

- **Scroll** with the mouse wheel, the touchpad or the scroll bars. PageUp and PageDown go to the previous and next
  page, Home and End to the first and last. Dragging the page itself doesn't scroll it (except on a touch screen):
  a drag selects text.
- **Go to a page** by clicking its thumbnail in the Pages view on the left; with the keyboard in the thumbnails, Up
  and Down go to the page before or after. The status bar at the bottom shows the page you are on.
- **See every page at once** with **Pages > Organise**, or **Organise pages** in the status bar: see
  [Organising pages](organising-pages.md#see-every-page-at-once). **Page view**, beside it, brings the pages back.
- **Zoom** with Ctrl+scroll, a pinch, Ctrl+= and Ctrl+-, or the zoom at the bottom right of the status bar: − and +
  zoom out and in, the slider between them sets the zoom (in windows 900 points wide or more), and the zoom level
  (such as "96%") opens a menu with Zoom in, Zoom out, Actual size and Fit width (also **View > Zoom**). Ctrl+1
  shows the pages at their actual size. Zoom goes from 10% to 6400%.
- **Fit width:** pages open fitted to the width between the side panels, and keep fitting as you resize the window
  or open and close the panels; Fit width is then ticked in the zoom menu. Zooming turns fitting off, and your zoom
  then stays as it is; **View > Fit width**, Fit width in the zoom menu, or Ctrl+0, turns it back on.

At high zoom, the part of the page in view is drawn in tiles as you scroll, over a softer view of the whole page
until they arrive.

### Page layout

The zoom menu at the bottom right, **View > Facing pages ▾**, or ☰ > View > Page layout, chooses how the pages are
laid out:

- **Continuous pages:** one column, scrolling. This is the default.
- **Facing pages:** two pages side by side, still scrolling; Fit width fits both. **Show cover page alone** (on by
  default) puts the first page on its own on the right, as in a book; turned off, pages 1 and 2 are a pair. PageUp
  and PageDown move a pair at a time.
- **Single page:** one page at a time. A wheel step past the page's top or bottom turns the page.

The choice changes the document you are viewing, and is remembered for documents opened later. A document that
sets its own [opening view](gui.md#opening-view) opens in its layout instead.

### Page colours for reading

**View > Page colours** (or ☰ > View > Page colours) changes how pages look on screen, for reading in the dark or
with low vision:

| Choice | Pages look like |
|---|---|
| Normal | Their own colours (the default) |
| Automatic | Dark while the app's theme is dark, Normal while it is light |
| Dark | Dark grey paper and light text; colours keep their hue, so red stays red |
| High contrast | Black paper; text, lines and anything coloured turn white |
| Sepia | Warm paper and dark brown text |

Photos and other pictures keep their own colours. Turn on **Recolour pictures too** in the same menu to change them
as well; a scanned page is one big picture, so this is what recolours scans. The choice applies to every open
document and the thumbnails, and is remembered between runs. It changes the screen only: the file, printing,
exported images and everything else keep the page's own colours.

## Find words

1. Press Ctrl+F, or choose **Edit > Find**. Search results opens on the left with the cursor in its search box:
   type what you are looking for.
2. As soon as you pause typing, Search results lists the matches, under a heading for each page, with the words
   around each match and the match in bold. The matches are highlighted on the pages, and the
   view goes to the first one on or after the page you were reading.
3. Press Enter to go to the next match, and Shift+Enter for the one before; or click a match, or use ◀ and ▶
   (Previous match and Next match). After the last match, Enter goes back to the first.

On a long document the matches come in page by page, with "Searching…" and the count so far; when it is done, the
count reads "1 match", "12 matches" or "No matches". Changing what you typed starts again, and the old matches go at
once. Emptying the search box clears the results. If you change the document while searching, the results are
brought up to date, and the match you were on stays the current one.

Search ignores capitals, and treats any run of spaces as one space. A phrase can run across a line break, or from
the bottom of one page to the top of the next: the break counts as a space, and the part on each line is
highlighted. A word hyphenated at the end of a line or a page is found whole, with or without its hyphen:
"information" and "self-employed" find "infor-" + "mation" and "self-" + "employed". Soft hyphens are ignored.

Scanned pages have no text until you [recognise it](scanning-and-ocr.md#recognising-text-ocr). To remove every
match from the document for good, press **Mark all for redaction** in Search results, then apply the marks (see
[Redaction](redaction.md)). It marks every match, even if the search is still running.

The headings in Search results, like the page thumbnails, use the document's own page numbering when it has one
(such as iii or A-1; see [Number pages your own way](organising-pages.md#number-pages-your-own-way)). To number the pages 1, 2, 3 from the first
page instead, turn off View > Show > **Use the document's page labels** (also in ☰ > View). The status bar shows
both.

On the command line, `limarepdf-cli search` lists the matches, one a line, as `page 3: …the words around it…`, then
the count; `--boxes` also prints where each one is on its page.

## Bookmarks

Bookmarks are a document's outline, like a table of contents. They are in the Bookmarks view: click its icon on the
left, or choose ☰ > View > Bookmarks. A document opens on its page thumbnails, unless the file asks to open on its
bookmarks.

- **Go to a bookmark's page** by clicking it. The arrow beside a bookmark shows or hides the bookmarks inside it,
  which start shown or hidden as the file asks. Bookmarks that go to another file or a web address are greyed out.
- **Add a bookmark** for the page you are on with Add bookmark. It goes after the bookmark you clicked last (or at
  the end, when you haven't clicked one), and its name opens for typing: Enter keeps what you typed, Esc keeps
  "Page N".
- **Rename a bookmark** by double-clicking it.
- **Right-click a bookmark** to rename it, add a bookmark inside it, or delete it along with the bookmarks inside
  it.

Undo reverses each of these. Bookmarks can't be moved or reordered yet. On the command line:
[`limarepdf-cli bookmarks`](cli.md#bookmarks).

## Links

Click a link with the read tool to follow it: the pointer turns into a hand over one, and hovering shows where it
goes. Dragging from a link selects text as usual. With the other tools, hold Ctrl and click, so links don't get in
the way of editing.

- A link to another page goes there.
- A link to a web or email address asks first, showing the full address, and then opens it in your browser or
  email program. LimarePDF itself never connects to the network.
- Links to anything else, such as a file or a program, are shown but not opened. Links to another PDF file aren't
  followed.

Links can't be added or edited yet.

## Copy text

1. With the read tool (the text cursor, which documents open with), drag across the words. A drag can start
   anywhere on the page, such as in the margin: it starts at the nearest word.
2. Press Ctrl+C, or use Copy on the small toolbar next to the selection.

More ways to select:

- Dragging past the top or bottom of the view scrolls it, faster the further out the pointer is, so a selection
  can run across pages.
- Shift+click extends the selection to where you click, on any page. Shift with the arrow keys extends it from the
  keyboard: Left and Right by a word, Up and Down to the line above or below, Home and End to the start or end of
  the line, across pages. Ctrl+A, or **Edit > Select > Select all text on the page**, selects the whole page in
  view, with the read tool or the Text tool. Esc,
  or a click, clears the selection. With no words selected, Shift+click on a picture or shape adds it to those
  selected instead ([Select several objects](content-editing.md#select-several-objects)).
- Double-clicking page text edits its line (see [Editing text and pictures](content-editing.md)). Where there is
  no line to edit, such as recognised (OCR) text, a double-click selects one word and a triple-click a line. A
  double-click or triple-click away from any text selects nothing.

Selected words can also be typed over, deleted or cut, as in a word processor (see
[Type over, delete or cut words](content-editing.md#type-over-delete-or-cut-words)).

Words are copied in reading order, with spaces between words and line breaks between lines and pages. Recognised
(OCR) text is selected and copied like any other text. **Edit > Copy** (or ☰ > Edit > Copy text) does the same as
Ctrl+C, and **Edit > Cut** (Ctrl+X) copies the words and then takes them out of the page.

On Linux, as in other Linux apps, the selected text can also be pasted into another app (or a LimarePDF text box)
by clicking there with the middle mouse button, without copying it first.

### Print or save just the selection

Right-click the selection and choose **Print selected text…** or **Save selected text…** (also under ☰ > Edit).
Both offer two layouts:

- **As it looks (cropped pages):** each page with selected words, at its full width with its margins, cut down
  from the top and bottom to the selected lines. Fonts, pictures and layout stay as they are, but anything else
  beside those lines shows too, such as the start of the first line, another column or a picture.
- **Plain text (PDF):** only the selected words, set in 11-point Helvetica on pages the size of the first selected
  page, keeping the line breaks. Characters outside Latin, Greek and Cyrillic come out as `?`.

Saving also offers **Text file (.txt)**: the same text that Ctrl+C copies. To save all of a document's text, see
[Export text](export-and-printing.md#text).

## How text is read

LimarePDF reads the text from the page content itself, the same way for search, highlights, copying, export text
and `limarepdf-cli text`, so they always agree.

- **Characters:** taken from the font's ToUnicode map, which says what text each character code stands for, and
  otherwise from its encoding. Where the file gives no base encoding, an embedded font's own encoding is the base,
  as the standard says and other readers do: the names the font gives its glyphs say what each code is (TeX's
  fonts put ” where the standard encoding has ", and – where it has {). The standard Symbol and ZapfDingbats fonts
  have encodings of their own too, so their Greek letters, maths signs and dingbats read as such. Ligatures such as
  "ﬁ" come out as their letters, so searching for "find" finds it.
  Chinese, Japanese and Korean fonts without a ToUnicode map (common in files whose fonts aren't embedded) are read
  through their character map, as pdf.js does: a Unicode one (such as `UniJIS-UCS2-H`) gives the text directly;
  others (such as Shift-JIS `90ms-RKSJ-H`) give character numbers, which Adobe's tables for Japanese, Simplified and
  Traditional Chinese and Korean turn into text. A character map stored in the file is read with the one it is
  built on, predefined or also stored in the file. Their codes are split as the map says, one to four bytes each (a
  code the map leaves out takes as many bytes as the map's code ranges say, as poppler and pdf.js read it), and
  their widths found by character number, so highlights and selections sit on the right characters. A code the
  map leaves out is drawn as no character, and reads as the replacement character �; so does one that is drawn
  but whose text no map gives.
- **Spaces:** a gap between words is a space, whether the file has a space character there or only moves the next
  word along (as justified text and many PDF makers do).
- **Order:** words come in reading order worked out from where they sit on the page, not the order the file draws
  them in (forms and many other files draw blocks out of order, such as a footer first). Blocks separated by space
  are read top to bottom; columns of text a column at a time; anything else row by row, left to right. Lines are
  separated by line breaks. Narrow columns, such as the page numbers of a table of contents, stay with their rows.
  Tables with wide columns come out a column at a time.
- **Where highlights go:** search highlights and selected words are placed using each character's width in the
  font. When a file uses one of the standard PDF fonts without including its widths, LimarePDF uses Adobe's
  published widths for them. Placement was checked on a real scan and on documents made by LaTeX, Sphinx,
  Ghostscript and LibreOffice; in all of them the highlights sit on the matching words.

Limits:

- Fonts with no ToUnicode map and a custom encoding whose glyph names don't say what they are (some Type 3 fonts,
  fonts not embedded, and fonts whose glyphs are named only by number) can give the wrong characters.
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
  searched and selected, but can't be edited line by line: the form moves, resizes and deletes as one object.

---

[← Features](features.md) · [User guide](README.md) · [Organising pages →](organising-pages.md)

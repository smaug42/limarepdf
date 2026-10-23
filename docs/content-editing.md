# Content editing

LimarePDF can move, resize, delete and restyle what is already on a page, edit text, and add text, shapes and images. This
page explains how that works and where it stops. For how to use it, see [Using the desktop app](gui.md#editing-page-content).

## How it works

The page's content stream is split into objects: one per text line, per painted path, per image or form XObject, and
per inline image.

- **Moving or resizing** wraps the object's drawing operators in `q <matrix> cm … Q`.
- **Editing text** rewrites the string in place when the font is a plain, non-subset, standard-encoded simple font
  and the new text is ASCII, with no font, size or colour change. Otherwise the old line is removed and the new
  text is added at the same position, size and colour, in the font chosen, else in the installed family the
  line's font is from (found by its PostScript name, the subset tag left off; `FontChoice::of_font`), else in the
  nearest of the 12 standard fonts. While a line is edited on the page, the app makes the same test as it goes
  (`TextEdit::keeps_font`) and draws the line in the font it will have: the line's own font installed on the
  computer, or else the Liberation face of its kind, when the font is kept; the installed family's font, or the
  Liberation face of the standard font replacing it, otherwise. The old line is covered with the most
  common colour in a 3-point ring around its box, from a separate drawing of that part of the page in the current
  page colours.
- **Adding** text, shapes or images draws them on top of the page. New text of several lines is written as one
  text line each, their baselines 1.2 times the size apart (Acrobat's spacing), in one of the 12 standard fonts
  with the WinAnsi encoding. A line with characters WinAnsi lacks (Polish, Czech, Vietnamese, Greek, Cyrillic and
  so on) is written in the bundled Liberation face of that font instead, embedded in the file and cut down to the
  characters used; one font object per face grows as more is typed. Characters Liberation can't write (right-to-left
  and joined scripts, Chinese, Japanese, Korean) can't be typed. While it is typed, the app draws the text in the
  Liberation face, which has the standard font's widths, so the caret and line length match the written result.
- **Installed fonts** (the inspector's font list after the three standard ones) are the families whose fonts
  allow embedding (`OS/2 fsType`: not restricted, subsetting allowed, not bitmaps only), have outlines and the
  Latin letters; up to four faces each (regular, bold, italic, bold italic), a variable font set at weight 400 or
  700 (and its `ital` or `slnt` axis for italic). Text in one is always embedded, cut down to the characters used,
  the same way: TrueType fonts and variable fonts (set at the weight, which makes them TrueType) as
  CIDFontType2 with `/FontFile2`, OpenType CFF fonts as CIDFontType0 with `/FontFile3 /OpenType`. The app draws
  the text in the font's own file, at that weight.
- **Invisible text** from OCR is edited as invisible text and stays in place.
- **Text after a changed line** stays where it was. Text on the same line can carry on from where the line ended
  (a show operator with no line move before it); when the line is deleted, edited or moved, its end is set again
  with `Tm` before that text, and the line position is set again before the next line move, which `Tm` also
  changes.
- **Inline images** are lifted out of the content before it is read, each replaced by a placeholder operator, and
  written back in place of those operators only, never of text that looks like one. The placeholder's name is
  chosen per content stream so that nothing in the stream already has it.

Every edit is checked to re-parse before it is applied, inline images lifted out as when the page is next read.
An edit that fails leaves the document unchanged.

## Limits

- Text objects are lines recovered from the content stream by heuristics, not paragraphs. There is no reflow and no
  multi-line editing. Text boxes are estimated from font metrics, so they can be slightly off for unusual fonts.
- Text in embedded subset fonts, and in CID (Type 0) fonts, cannot keep its font when edited: it is written in the
  installed family the font is from when LimarePDF can embed it, else it becomes Helvetica, Times or Courier (or
  their Liberation face, as new text is), and the inspector says so. Text written anew is limited to Latin, Greek
  and Cyrillic, and Chinese, Japanese and Korean in an installed font that has them: the app doesn't let others
  be typed, and a line that already has them (Chinese in a line that becomes Helvetica, say) can't be written
  anew; the change is refused with a message naming them. Right-to-left and shaped scripts (Hebrew, Arabic,
  Devanagari, Thai and the rest) are refused in every font, as they would come out wrong without reordering and
  shaping. An installed font lacking a character refuses it too, naming the font: one line is set in one font,
  with no fallback to another.
- Installed fonts whose licence restricts embedding, or that have no Latin letters, aren't offered; a family's
  missing bold or italic isn't made up. The list is read once, when the app starts (about a second, on another
  thread): fonts installed while it runs show after a restart.
- While a line that keeps its font is edited, its character and word spacing and horizontal scaling (`Tc`, `Tw`,
  `Tz`) aren't drawn, and an embedded font is drawn from the installed or Liberation look-alike rather than its own
  data, so the line can look slightly different until you finish. Turned lines and invisible text are edited in a
  plain box.
- Editing text does not change its width: longer text can overlap its neighbours.
- Where the text after a changed line starts is worked out from the font's character widths. Where a file doesn't
  give them for a font that isn't one of the standard 14, or a CID font's characters can't be told apart, they are
  estimated, so that text can move a little. In vertical writing the text after a changed line still moves with
  the change: LimarePDF measures vertical text as if it ran across, so it doesn't know where such a line ends.
- A clip region set before an object stays where it was, so a moved or enlarged object can be cut off by its old
  clip. A path that is itself used as a clip (`W n`) cannot be selected.
- Shading and pattern fills can be selected, but their colour cannot be changed in the inspector (it may apply a flat
  colour instead). Colours in colour spaces other than RGB, grey and CMYK are shown approximately.
- Inline images can be moved, resized and deleted, but not replaced. Pages whose inline images cannot be located
  safely are refused rather than risk losing them.
- Replacing an image fits the new picture inside the old box, centred. JPEG EXIF rotation is not applied, and CMYK
  JPEGs are not supported.
- Rotating objects, grouping, changing the stacking order, and copying and pasting between pages are not implemented.
- Moving an object does not move links or annotations over it.
- Text inside a form XObject (a reusable block of page content) is searched and selected like other text, but it is
  not a separate object: the form moves, resizes and deletes as a whole, and its text cannot be edited.

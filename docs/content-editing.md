# Content editing

LimarePDF can move, resize, delete and restyle what is already on a page, edit text, and add text, shapes and images. This
page explains how that works and where it stops. For how to use it, see [Using the desktop app](gui.md#editing-page-content).

## How it works

The page's content stream is split into objects: one per text line, per painted path, per image or form XObject, and
per inline image.

- **Moving or resizing** wraps the object's drawing operators in `q <matrix> cm … Q`.
- **Editing text** rewrites the string in place when the font is a plain, non-subset, standard-encoded simple font
  and the new text is ASCII. Otherwise the old line is removed and the new text is added in one of the 12 standard
  fonts, at the same position, size and colour.
- **Adding** text, shapes or images draws them on top of the page. New text of several lines is written as one
  text line each, their baselines 1.2 times the size apart (Acrobat's spacing), in one of the 12 standard fonts
  with the WinAnsi encoding; characters outside it can't be typed. While it is typed, the app draws it in the
  bundled Liberation font with the same widths, so the caret and line length match the written result.
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
- Text in embedded subset fonts, and in CID (Type 0) fonts, cannot keep its font when edited: it becomes Helvetica,
  Times or Courier, and the inspector says so. Text edited into a new font is limited to WinAnsi characters; others
  print as `?`.
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

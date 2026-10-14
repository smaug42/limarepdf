# Content editing

MPE can move, resize, delete and restyle what is already on a page, edit text, and add text, shapes and images. This
page explains how that works and where it stops. For how to use it, see [Using the desktop app](gui.md#editing-page-content).

## How it works

The page's content stream is split into objects: one per text line, per painted path, per image or form XObject, and
per inline image.

- **Moving or resizing** wraps the object's drawing operators in `q <matrix> cm … Q`.
- **Editing text** rewrites the string in place when the font is a plain, non-subset, standard-encoded simple font
  and the new text is ASCII. Otherwise the old line is removed and the new text is added in one of the 12 standard
  fonts, at the same position, size and colour.
- **Adding** text, shapes or images draws them on top of the page.
- **Invisible text** from OCR is edited as invisible text and stays in place.

Every edit is checked to re-parse before it is applied. An edit that fails leaves the document unchanged.

## Limits

- Text objects are lines recovered from the content stream by heuristics, not paragraphs. There is no reflow and no
  multi-line editing. Text boxes are estimated from font metrics, so they can be slightly off for unusual fonts.
- Text in embedded subset fonts, and in CID (Type 0) fonts, cannot keep its font when edited: it becomes Helvetica,
  Times or Courier, and the inspector says so. Text edited into a new font is limited to WinAnsi characters; others
  print as `?`.
- Editing text does not change its width: longer text can overlap its neighbours.
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

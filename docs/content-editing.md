# Content editing

LimarePDF can move, resize, delete and restyle what is already on a page, edit text, and add text, shapes and images. This
page explains how that works and where it stops. For how to use it, see [Using the desktop app](gui.md#editing-page-content).

## How it works

LimarePDF splits a page's content into objects, one per line of text, drawn shape or picture, and edits them in
the page itself.

- **Moving or resizing** places the object anew, leaving its drawing as it is.
- **Editing text** keeps the line's own font when it can: a plain standard font, with plain text and no change of
  font, size or colour. Otherwise the line is written again in the same place, size and colour, in the font you
  chose, else in the installed family the line's font comes from, else in the nearest of Helvetica, Times and
  Courier. While you edit a line it is drawn in the font it will have, and the old line underneath is hidden in
  the colour around it.
- **Adding** text, shapes or pictures draws them over the page. Each typed line becomes its own line of page text,
  spaced 1.2 times the size apart (as Acrobat does). Text in Helvetica, Times or Courier with characters those
  fonts lack (Polish, Czech, Vietnamese, Greek, Cyrillic and so on) is written in the bundled Liberation face of
  that font, which has the same widths, embedded in the file and cut down to the characters used.
- **Installed fonts** (the inspector's font list after the three standard ones) are the families whose licence
  allows embedding, with up to four styles each. Text in one is always embedded in the file, cut down to the
  characters used, so it looks the same on any computer.
- **Recognised (OCR) text** stays invisible and in place when edited, and text after a changed line stays where
  it was.

Every edit is checked before it is applied, and one that fails leaves the document unchanged. The technical
detail is in [How it is built](developer/architecture.md#content-editing).

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

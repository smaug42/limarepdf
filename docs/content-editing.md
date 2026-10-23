[User guide](README.md) › Everyday tasks

# Editing text and pictures

Change the text already on a page, add new text and pictures, and move, resize, restyle or delete what is there.
Edits are made line by line: a paragraph doesn't rewrap as you type (see [Limits](#limits)). Shortcuts are written
with Ctrl; on a Mac, use ⌘ (Command) instead.

## Edit a line of text

1. Double-click the line with the read tool (the text cursor). The word you double-clicked is selected, so typing
   replaces it.
2. Type your change.
3. Press Enter, Esc or Tab, or click elsewhere, to keep it.

While you type:

- The line is drawn as it will be written, with a thin outline around it and the old line hidden underneath in the
  colour around it. It keeps its own font when LimarePDF can keep it; otherwise it is written in the installed font
  it came from (when LimarePDF can embed it), or in the Helvetica, Times or Courier that replaces it, which switches
  as you type a character the line's font can't take. Its size and colour stay the same.
- Ctrl+Z and Ctrl+Y undo and redo typing in the line, and Ctrl+B and Ctrl+I switch the whole line to Bold or
  Italic. Once the change is kept, Undo takes the whole edit back. Editing the same line again within a few seconds,
  with nothing else changed in between, joins the same undo step.
- The inspector shows the line's font, size and colour. A change there shows on the line at once, the line stays
  in editing, and the keyboard comes back to it; the change is kept with the typing.

A line turned on the page (not running left to right on screen), and invisible recognised (OCR) text, are edited
in a plain box over the line instead.

## Add text

1. Pick **Add text** in the top bar.
2. Click where the text starts, and type. The line sits on the point you clicked; Enter starts a new line.
3. Press Esc, click elsewhere on the page, or choose another tool, to finish.

The text appears as you type, in the font, size and colour it will have, with a thin outline around it. Every line
is added as one undo step, and nothing typed adds nothing. Each line becomes its own line of page text, so
afterwards it moves and is edited line by line.

While you type, the inspector's font, Bold, Italic, size and colour change the text (the keyboard comes back to it
afterwards), Ctrl+B and Ctrl+I switch Bold and Italic, and dragging the outline moves the text. With this tool, a
click on text already on the page edits that line instead, with the caret where you clicked, and a drag selects
text, as with the read tool.

**Fonts:** the font list has Helvetica, Times and Courier first, then the fonts installed on the computer whose
licence allows embedding them, each name drawn in its own font. Type in its search field to narrow it; Enter picks
the first. Bold and Italic are off for a family that has no such face.

**Characters:** Latin (Vietnamese included), Greek and Cyrillic characters can be added. Those the standard fonts
lack are written in their Liberation face, and text in an installed font is written in that font, both embedded in
the file (cut down to the characters used). Chinese, Japanese and Korean can be typed in an installed font that has
them. Others (Hebrew, Arabic, Devanagari, Thai and so on, and emoji), and characters the chosen font doesn't have,
can't be typed or pasted, and a note at the caret says so. When another font could have the character (Chinese in
Helvetica, say, or a letter an installed font lacks), the note names the font in use and asks you to choose an
installed font that has it. The same goes for editing a line and the inspector's text field (see
[What doesn't work yet](features.md#what-doesnt-work-yet)).

To put the same text on every page, such as page numbers or DRAFT, use
[headers, footers and watermarks](headers-footers-watermarks.md).

## Add a picture

1. Pick **Add image** in the top bar, and choose a PNG, JPEG or TIFF file.
2. Drag a box on the page where it goes.

## Move, resize or delete

With the read tool:

- **Pictures and shapes:** click one to select it. Drag it to move it, and drag a handle to resize it (Shift keeps
  the proportions). Arrow keys move it by 1 point, or 10 with Shift. Delete or Backspace deletes it.
- **Page text:** double-click the line, then choose **Move or resize text** on its toolbar. The line gets handles:
  drag it or them, or use the arrow keys, until you click away or select something else. Text you have just added
  can be moved straight away in the same way.

A picture or shape covering most of the page, such as a scanned page or a background, isn't selected by a click,
so that clicking a scan doesn't pick it up. A drag that starts anywhere else still selects text.

**The arrow** (☰ > Tools > Select, move and resize objects) selects any object, page text and full-page pictures
included: click to select, drag to move, drag a handle to resize (Shift keeps the proportions), and arrow keys to
nudge.

## Change size, font and colour in the inspector

The inspector shows the selected object's position and size; for text, the string, font, size and colour; for
shapes, the fill, outline and width; for pictures, **Replace image…**. Changes there are kept as you make them,
without an Apply button: typed text when you press Enter or leave the box, a number when you stop dragging it or
leave its box, a colour when its picker closes. Each is one undo step.

The font size box shows the exact size, which isn't rounded to a listed one (10.126 shows as 10.126). Type any
size, or pick one from the list beside it. Going into the box and out again without typing changes nothing, and
text that isn't a size leaves the size as it was.

## After you put something down

Add text and Add image put one thing down and then give way to the read tool, with the new item selected so you
can adjust it. To add several in a row, pin the tool (see [Keep tool active](gui.md#keep-tool-active)). Everything
can be undone with Ctrl+Z, up to 20 steps back ([fewer on very large documents](gui.md#undo)).

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

---

[← Comments and markup](comments-and-markup.md) · [User guide](README.md) · [Filling in forms →](forms.md)

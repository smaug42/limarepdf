[User guide](README.md) › Everyday tasks

# Editing text and pictures

Change the text already on a page, add new text and pictures, and move, resize, restyle or delete what is there.
Edits are made line by line: a paragraph doesn't rewrap as you type (see [Limits](#limits)). Shortcuts are written
with Ctrl; on a Mac, use ⌘ (Command) instead.

## Edit a line of text

1. Double-click the line with the read tool (the text cursor). The word you double-clicked is selected, so typing
   replaces it. Or choose **Edit > Edit text**, then click the line: the caret goes where you click. With words
   selected, **Edit > Edit text** edits their line with them selected (see
   [Type over, delete or cut words](#type-over-delete-or-cut-words)).
2. Type your change.
3. Press Enter, Esc or Tab, or click elsewhere, to keep it.

While you type:

- The line is drawn as it will be written, with an outline around it (the accent colour with a white or black line
  inside, so it shows on any page) and the old line hidden underneath in the colour around it. The status bar says
  **Editing one line**: what you type changes this line only. A line with words in other fonts, sizes or colours
  (a bold word, a superscript) is edited whole, each word in its own style. Each keeps its own font, embedded in
  the PDF or not, letter by letter: letters you leave as they are keep their exact spacing, and new letters are
  written in the font when it holds them, drawn in its own letters as you type. A word with a letter the font
  lacks is written in the installed font it came from (when LimarePDF can embed it), or in the Helvetica, Times or
  Courier that replaces it, and switches as you type that letter; the rest of the line keeps its font. The
  inspector names those words and the font they use ("Not in this PDF's font: größe Maße. Those words use
  Helvetica."), or says that the font can't take new text at all (a Type 3 font, one not in the PDF, vertical
  writing), why when you point at it.
- When what you type runs past the edge of the page, or on past the line's end into the text beside it, the
  outline turns the caution colour and a tag under it says **Past the page edge** or **Over other text**. It is
  only a warning: what you type is kept as it is.
- What you type takes the style of the character before it (at the start of the line, the one after it).
- Ctrl+Z and Ctrl+Y, or Undo and Redo in the commands row, undo and redo typing and style changes in the line, in
  order. Once the change is kept, Undo takes the whole edit back. Typing into the same line again within a few
  seconds, with nothing else changed in between, joins the same undo step.
- **Edit > Font** in the band shows the font, size and colour of what is selected, or of the character before the
  caret. The inspector shows the font and size too, the font the PDF names (**PDF font**) and whether it is kept
  (**Source font**, see [Change size, font and colour](#change-size-font-and-colour)). Esc in the size box (or
  another box there) drops what you typed in it and goes back to the line.
- The floating toolbar has **B**, ***I***, Cut, Copy and Paste for what is selected in the line. It sits above,
  below or beside the line, clear of other text where there is room, never over the line; clicking it leaves the
  line open.
- Right-click what is selected in the line for Cut, Copy and Paste, then Bold and Italic: the selection stays as it
  was. New text, a note and a text box being typed have it too, with Bold and Italic where the text takes them.
- Going to another tab and back leaves the line open, ready to go on typing.

A line turned on the page (not running left to right on screen) is edited in a box over the line instead, showing
its fonts, sizes and colours; invisible recognised (OCR) text is edited in a plain box. The caret goes by the
character you clicked along the line, and a double-click selects the word you clicked, whichever way the line runs.

## Style words in a line

1. Double-click the line, then select the words: drag over them, or hold Shift and use the arrow keys. A
   double-click selects one word.
2. Press Ctrl+B for Bold or Ctrl+I for Italic, use **B** and ***I*** on the floating toolbar, or use
   **Edit > Font** in the band: the font list, size, Bold, Italic and text colour.
3. Press Enter, Esc or Tab, or click elsewhere, to keep it.

As in a word processor:

- With nothing selected, the caret inside a word changes that word. Between words, or at either end of a word,
  the change applies to what you type next there.
- Bold over words that are partly bold makes them all bold; over words that are all bold, it takes the bold off.
  Italic works the same way. Words in a family without a bold or italic face stay as they are: **B** or ***I***
  is greyed, and says why when you point at it, and Ctrl+B or Ctrl+I says so at the caret.
- Where the selection mixes fonts, sizes or colours, **Edit > Font** shows that one blank: the font list with no
  name, the size box empty, the colour button (an **A** over a bar of the text's colour) with the colours side by
  side in its bar. A colour picked then makes the
  selection all that colour, even the one the picker starts from (the first); the picker also offers the colours
  in the text (**In the text**), to make it all one of them in a click. Opening and closing the picker changes
  nothing. Bold and Italic show on only when all
  of it is bold or italic. Picking a value there sets all of it.
- A change in **Edit > Font** shows on the line at once, the line stays in editing, and the keyboard comes back to
  it; the change is kept with the typing.

Finishing (Esc, Enter, Tab or clicking elsewhere) leaves nothing selected, as in a word processor. A line selected
as a whole with the arrow takes the font, size and colour in **Edit > Font**, and Ctrl+B and Ctrl+I, as a whole,
with the same blanks for what it mixes; to change its words, double-click it or choose **Edit > Edit text**, and it
is edited on the page here too. With the arrow, the line stays selected when you finish.

## Style words you select while reading

You don't need to open a line to style its words. Select them with the read tool, as you would to copy them (drag
over them, across lines and pages if you like), then:

- press Ctrl+B for Bold or Ctrl+I for Italic, or use **B** and ***I*** on the floating toolbar or in the right-click
  menu;
- or pick a font, size or colour in **Edit > Font** in the band. The inspector, headed **Selected text**, shows
  their font and size.

The words stay selected, so you can go on changing them. Bold and Italic work over the whole selection as they do in
a line, and **Edit > Font** shows a value blank where the selection mixes it. Each change is one undo step, however
many lines and pages it covers.

The words around the ones you change are left exactly as they were, in their own font, even one LimarePDF can't
write new text in. Words whose size or colour is all you change keep their font too; Bold, Italic or another font
writes them in that font. Invisible recognised (OCR) text, text inside a block of page content reused as a whole,
and vertical writing keep their look; a short notice says when part of the selection was left that way.

## Type over, delete or cut words

Words you select with the read tool can be changed as in a word processor, without opening their line first:

- **Type** to replace them. Their line opens for editing with what you typed in their place, and you go on typing
  there; press Enter, Esc or Tab, or click elsewhere, to keep it. **Ctrl+V**, or **Paste** (in **Edit** in the
  band, the right-click menu or the floating toolbar), pastes over them the same way. Over words on more than one
  line nothing is typed, and a note says "Type over words within one line. Paragraph editing is planned." With no
  words selected and nothing being typed, Ctrl+V says where to click to paste.
- **Delete** or **Backspace** removes them, across lines and pages, in one step that Undo takes back. As in Word,
  a space beside them goes too when two would be left together, or one at the start or end of a line. A line left
  empty is gone; the lines around it stay where they are, as lines aren't joined together.
- **Cut** (Ctrl+X, in **Edit** in the band, the right-click menu or the floating toolbar) copies them, then removes
  them as Delete does.
- **Edit > Edit text** (also in the right-click menu and the floating toolbar) opens their line with them selected,
  to type over them or style them. Over more than one line, it opens the line where the selection ends.

To select them from the keyboard, hold **Shift** and use the arrow keys: Left and Right add or take off a word, Up
and Down go to the line above or below, and Home and End to the start or end of the line, across pages; the view
follows. Invisible recognised (OCR) text, text inside a block of page content reused as a whole, and vertical
writing are left as they are, and a short notice says so; when none of the selection can be changed, Cut only
copies it.

## Add text

1. Pick **Insert > Text** (the Add text tool), or **Edit > Edit text** with nothing selected.
2. Click where the text starts, and type. The line sits on the point you clicked; Enter starts a new line.
3. Press Esc, click elsewhere on the page, or choose another tool, to finish.

The text appears as you type, in the font, size and colour it will have, with an outline around it that turns the
caution colour, tagged **Past the page edge**, if the text runs off the page. Every line is added as one undo
step, and nothing typed adds nothing. Each line becomes its own line of page text, so
afterwards it moves and is edited line by line.

While you type, Ctrl+B, Ctrl+I and **Edit > Font** style what is selected, or what you type next, as when
[styling words in a line](#style-words-in-a-line) (the keyboard comes back to the text afterwards).
Ctrl+Z and Ctrl+Y undo and redo typing and styles, and dragging the outline moves the text (Esc during the drag
puts it back where it was). The next text you add
starts in the style this one ended in; before you click, **Edit > Font** sets it. A line with larger text in it
sits further below the one before, as in a word processor. With this tool, a click on text already on the page
edits that line instead, with the caret where you clicked (Edit > Font and the inspector then show that line's
look), and a drag selects text, as with the read tool.

**Fonts:** the font list in **Edit > Font** has Helvetica, Times and Courier first, then the fonts installed on the
computer whose licence allows embedding them, each name drawn in its own font. Type in its search field to narrow
it; Enter picks the first. Bold and Italic are off for a family that has no such face.

Some fonts are installed under code names, such as the URW fonts many Linux systems have. The list shows these by
the typeface they stand in for, with the font's own name after it: "Palatino (P052)", "New Century Schoolbook
(C059)", "Helvetica (Nimbus Sans)". Searching finds either name. They are made to the same letter widths as those
typefaces but aren't the typefaces themselves; point at one to see this. Unlike the standard Helvetica, Times and
Courier at the top of the list, they are embedded in the file, so the text looks the same everywhere.

**Characters:** Latin (Vietnamese included), Greek and Cyrillic characters can be added. Those the standard fonts
lack are written in their Liberation face, and text in an installed font is written in that font, both embedded in
the file (cut down to the characters used). Chinese, Japanese and Korean can be typed in an installed font that has
them, and so can single emoji (Symbola and DejaVu Sans have many). Others (Hebrew, Arabic, Devanagari, Thai and so
on, and emoji made of several characters: flags, skin tones, keycaps, joined emoji such as families), characters
the chosen font doesn't have, and emoji it draws only in colour, can't be typed or pasted, and a note at the caret
says so: for a script written right to left or with joined letters, that LimarePDF can't add that script yet.
When another font could have the character (Chinese or an emoji in Helvetica, say, or a letter an installed font
lacks), the note names the font in use and asks you to choose an installed font that has it. The same goes for
editing a line and the inspector's text field (see
[What doesn't work yet](developer/feature-status.md#what-doesnt-work-yet)).

To put the same text on every page, such as page numbers or DRAFT, use
[headers, footers and watermarks](headers-footers-watermarks.md).

## Add a picture

1. Pick **Insert > Image**, and choose a PNG, JPEG or TIFF file.
2. Drag a box on the page where it goes.

A photo shows upright, the way it was taken, even when the camera or phone was turned.

## Move, resize or delete

With the read tool:

- **Pictures and shapes:** click one to select it, and Shift+click more to [select several](#select-several-objects).
  Drag it to move it, and drag a handle to resize it (Shift keeps the proportions). Arrow keys move it by 1 point,
  or 10 with Shift. Delete or Backspace deletes it.
- **Page text:** double-click the line, then choose **Move or resize text** on its toolbar. The line gets handles:
  drag it or them, or use the arrow keys, until you click away or select something else. Text you have just added
  can be moved straight away in the same way.

A picture or shape covering most of the page, such as a scanned page or a background, isn't selected by a click,
so that clicking a scan doesn't pick it up. A drag that starts anywhere else still selects text.

**The arrow** (**Edit > Select > Select, move and resize objects**, or ☰ > Tools > Select, move and resize objects)
selects any object, page text and full-page pictures included: click to select, drag to move, drag a handle to
resize (Shift keeps the proportions), and arrow keys to nudge.

## Select several objects

Hold **Shift** and click pictures and shapes to add them to the selection. Shift+click one again to take it out. A
click without Shift selects that one alone. This works with the read tool, which a document opens with, and with
the arrow, which selects lines of page text this way too.

With the arrow you can also:

- Drag a rectangle from an empty part of the page: everything wholly inside it is selected, nothing it only
  touches. Hold Shift to add what it holds to the selection. A drag that starts on a background or a scanned page
  that isn't selected draws a rectangle too, so it isn't moved by accident.
- Press **Ctrl+A**, or choose **Edit > Select > Select all on the page**, to select everything on the page.

Each selected object is outlined, with a dashed box round them all. Drag any one of them, or use the arrow keys, to
move them together; Delete or Backspace deletes them all. The inspector says how many there are and what they are.
**Edit > Font** changes the lines of text among them, each line as a whole, and the inspector's Fill, Outline and
width change the shapes; a value they don't share shows blank or mixed. X and Y under **Position** move
them all. Right-click one of them for Copy the selected text lines (in reading order), Cut (when they are all
lines of text) and Delete. Each change is one undo step.

Several objects are selected on one page at a time. They move, restyle and delete together, but are resized one at
a time. Comments and markup are selected one at a time, never with page objects.

## Change size, font and colour

For text, **Edit > Font** in the band has the font list, then size, Bold, Italic and colour (the words themselves
are changed on the page). Pointing at the font list shows the names of the fonts the selected text uses in the PDF.

The inspector is headed by what is selected (Selected text, Image, Shape). For text it shows what is being edited
(One line, Selected words or New text), its font and size (Mixed where they differ) and, for a line being edited,
two more rows. **PDF font** names the fonts of what is selected in the line (or of all of it) as the PDF does,
without the six-letter tag a font cut down to the characters used has. **Source font** says how what you typed
will be written, and changes as you type:

- **Preserved**: in the line's own font.
- **Installed family**: in the installed family its font comes from.
- **Nearest standard**: in the nearest of Helvetica, Times and Courier; when the line's own font can't be kept, a
  note under it says "Non-standard font: edits use the nearest standard one."
- **Partly preserved**: some of the line in its own font, the rest not (the words you changed in a font LimarePDF
  can't keep, say).
- **Mixed**: some of it in an installed family, the rest in a standard font.
- **Font chosen**: all of it in a font you picked in **Edit > Font**.

(See [How it works](#how-it-works).) For shapes it shows the fill, outline and width; for pictures,
**Replace image…**. Below these, **Position and size** opens to show the object's X, Y, width and height; it
stays open or closed as you left it. For selected text, **Future text tools**, marked Planned, comes last: open it
to see what is planned (paragraphs that reflow, underline and strike-through). Changes in the band and the
inspector are kept as you make them,
without an Apply button: typed text when you press Enter or leave the box, a number when you stop dragging it or
leave its box, a colour when its picker closes. Each is one undo step.

The font size box shows the exact size, which isn't rounded to a listed one (10.126 shows as 10.126). Type any
size, or pick one from the list beside it. Going into the box and out again without typing changes nothing, and
text that isn't a size leaves the size as it was.

## After you put something down

Add text and Add image put one thing down and then give way to the read tool, with the new item selected so you
can adjust it: new text of several lines is selected as one, so it moves together. To add several in a row, pin the tool (see [Keep tool active](gui.md#keep-tool-active)). Everything
can be undone with Ctrl+Z, up to 100 steps back ([Undo](gui.md#undo)).

## How it works

LimarePDF splits a page's content into objects, one per line of text, drawn shape or picture, and edits them in
the page itself.

- **Moving or resizing** places the object anew, leaving its drawing as it is.
- **A line of text** is one object whatever its styles: a bold word or a superscript in it is a part of the line,
  not an object of its own.
- **Editing text** keeps each part's own font when it can: a part left as it was is written exactly as it was, so
  are whole words left as they were beside words you changed, and a changed part in a plain standard font keeps it
  for plain text. Otherwise that part is written again in its
  size and colour, in the font you chose, else in the installed family its font comes from, else in the nearest of
  Helvetica, Times and Courier, the line staying in place. While you edit a line it is drawn in the fonts it will
  have, and the old line underneath is hidden in the colour around it.
- **Adding** text, shapes or pictures draws them over the page. Each typed line becomes its own line of page text,
  1.2 times the size below the one before (as Acrobat does); with sizes that differ, 1.2 times a quarter of the
  line above's largest size and three quarters of its own. Text in Helvetica, Times or Courier with characters
  those fonts lack (Polish, Czech, Vietnamese, Greek, Cyrillic and so on) is written in the bundled Liberation
  face of that font, which has the same widths, embedded in the file and cut down to the characters used.
- **Installed fonts** (the font list in **Edit > Font**, after the three standard ones) are the families whose
  licence allows embedding, with up to four styles each. Text in one is always embedded in the file, cut down to the
  characters used, so it looks the same on any computer.
- **Recognised (OCR) text** stays invisible and in place when edited, and text after a changed line stays where
  it was.

Every edit is checked before it is applied, and one that fails leaves the document unchanged. The technical
detail is in [How it is built](developer/architecture.md#content-editing).

## Limits

- Text objects are lines recovered from the content stream by heuristics, not paragraphs. There is no reflow and no
  multi-line editing: typing over words selected on more than one line types nothing, and deleting words leaves
  each line where it was, an emptied one blank. Text boxes are estimated from font metrics, so they can be
  slightly off for unusual fonts. A line the page draws twice (as maps draw a label over a halo of its own) is
  changed where you click it; the copy under it keeps the old words.
- A changed line keeps its font only for letters the font verifiably holds: one that is cut down to the letters
  the file uses (a subset) can't take others, and a Type 3 font, a font not held in the PDF (beyond plain English
  letters in a standard one), vertical writing and fonts whose codes LimarePDF can't read take none. A word with
  such a letter is written in the installed family the font is from when LimarePDF can embed it, else in
  Helvetica, Times or Courier (or their Liberation face, as new text is), and the inspector says so; the words
  left as they were keep their font. Letters a font draws joined (a ligature such as "fi") keep it only together
  and unchanged. Where the changed line would read as other words (when it runs into the text after it, say, or a
  space it had as a small gap after a raised or lowered part no longer reads as a space), it is written in the
  similar font instead, and a notice says so. Text written anew is limited to Latin, Greek
  and Cyrillic, and Chinese, Japanese and Korean in an installed font that has them: the app doesn't let others
  be typed, and a line that already has them (Chinese in a line that becomes Helvetica, say) can't be written
  anew; the change is refused with a message naming them. Right-to-left and shaped scripts (Hebrew, Arabic,
  Devanagari, Thai and the rest) are refused in every font, as they would come out wrong without reordering and
  shaping. An installed font lacking a character refuses it too, naming the font: a typed character goes in the
  font of the words around it, with no fallback to another.
- Installed fonts whose licence restricts embedding, or that have no Latin letters, aren't offered; a family's
  missing bold or italic isn't made up. The list is read once, when the app starts (about a second, on another
  thread): fonts installed while it runs show after a restart.
- While a line that keeps its font is edited, its character and word spacing, horizontal scaling (`Tc`, `Tw`,
  `Tz`) and the fine spacing between its letters aren't drawn, a font not held in the PDF is drawn in the installed
  or Liberation look-alike, and the letters of a ligature in the Liberation look-alike, so the line can look
  slightly different until you finish. A part raised high above the line (a large
  superscript) is drawn lower while you type, within the line, and is written where it was. Turned lines are
  edited in a box over the line, and invisible text in a plain box.
- A word written in another font (a letter its font lacks, or another font chosen) loses its own character and
  word spacing and horizontal scaling, and words moved by cut and paste lose the fine spacing between their
  letters; letters left as they were keep theirs.
- There is no underline or strike-through for page text. Styling words doesn't rewrap: bold or larger words make
  their line longer and the rest of it moves along, and highlights, links and redaction marks over them stay where
  they were.
- Editing text does not change its width: longer text can overlap its neighbours. The outline warns of it (**Over
  other text**) only for text beside the line's end, on its right; new text is checked against the page's edge
  only. The warning's tag sits under the outline's right end, and can be hidden by the floating toolbar when the
  toolbar sits below the line.
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
- Replacing an image fits the new picture inside the old box, centred, upright as a photo's own orientation asks.
- Rotating objects, grouping, changing the stacking order, and copying and pasting between pages are not implemented.
- Moving an object does not move links or annotations over it.
- Text inside a form XObject (a reusable block of page content) is searched and selected like other text, but it is
  not a separate object: the form moves, resizes and deletes as a whole, and its text cannot be edited.

---

[← Comments and markup](comments-and-markup.md) · [User guide](README.md) · [Filling in forms →](forms.md)

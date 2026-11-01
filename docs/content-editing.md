[User guide](README.md) › Everyday tasks

# Editing text and pictures

Change the text already on a page, add new text and pictures, and move, resize, line up, restyle or delete what is
there. A double-click edits a line; turn on **Paragraph** to edit whole paragraphs instead, their lines rewrapping
as you type (see [Edit a paragraph](#edit-a-paragraph)). Shortcuts are written with Ctrl; on a Mac, use ⌘
(Command) instead.

## Edit a line of text

1. Double-click the line with the read tool (the text cursor). The word you double-clicked is selected, so typing
   replaces it. Or choose **Edit > Edit text**, then click the line: the caret goes where you click. With words
   selected, **Edit > Edit text** edits their line with them selected (see
   [Type over, delete or cut words](#type-over-delete-or-cut-words)).
2. Type your change.
3. Press Enter, Esc or Tab, or click elsewhere, to keep it. **Edit > Edit text**, shown pressed while you edit
   text or use the Text tool, also keeps it when you click it again, and goes back to the read tool.

While you type:

- The line is drawn as it will be written, with an outline around it (the accent colour with a white or black line
  inside, so it shows on any page) and the old line hidden underneath by the page as it is without it, so a photo,
  shading or coloured box behind it still shows (for a moment as you start, the colour around it). The status bar says
  **Editing one line**: what you type changes this line only. A line with words in other fonts, sizes or colours
  (a bold word, a superscript) is edited whole, each word in its own style. Each keeps its own font, embedded in
  the PDF or not, letter by letter: letters you leave as they are keep their exact spacing, and new letters are
  written in the font when it holds them, drawn in its own letters as you type. A word with a letter the font
  lacks is written in the installed font it came from (when LimarePDF can embed it), or in the Helvetica, Times or
  Courier that replaces it, and switches as you type that letter; the rest of the line keeps its font. A word that
  font lacks too (Chinese in Helvetica, say) goes on to an installed font that has it, as new text does (see
  [Add text](#add-text)). The inspector names those words and the font they use ("Not in this PDF's font:
  größe Maße. Those words use Helvetica."), or says that the font can't take new text at all (a Type 3 font, one
  not in the PDF, vertical writing), why when you point at it.
- When what you type runs past the edge of the page, or into text, a picture or a shape the line wasn't already
  over, the outline turns the caution colour and a tag just below the line, where it happens, says **Past the page
  edge**, **Over other text**, **Over a picture** or **Over a shape**. It is only a warning: what you type is kept
  as it is.
- What you type takes the style of the character before it (at the start of the line, the one after it).
- The caret, Delete and Backspace go by the character as you see it: a letter with its accent, a Korean syllable
  or an emoji is one step. Backspace after a letter typed with a separate accent takes off only the accent, so you
  can type another. Ctrl with the arrow keys and a double-click go by words as Unicode defines them, in which each
  Chinese or Japanese character is a word of its own.
- An input method (for Chinese, Japanese or Korean, say) types in the line as it does in other apps: what you are
  composing shows in the line, and is checked when you commit it. Clicking away or choosing another tool while
  composing keeps what was composed, as Windows and macOS do; a character in it that can't be written is left out,
  and a message says "Left out "…"" and why.
- Ctrl+Z and Ctrl+Y, or Undo and Redo in the commands row, undo and redo typing and style changes in the line, in
  order. Once the change is kept, Undo takes the whole edit back. Typing into the same line again within a few
  seconds, with nothing else changed in between, joins the same undo step.
- **Edit > Font** in the band shows the font, size and colour of what is selected, or of the character before the
  caret. The inspector shows the font and size too, the font the PDF names (**PDF font**) and whether it is kept
  (**Source font**, see [Change size, font and colour](#change-size-font-and-colour)). Esc in the size box (or
  another box there) drops what you typed in it and goes back to the line.
- The floating toolbar has **B**, ***I***, **U** and **S** (Bold, Italic, Underline and Strikethrough), Cut, Copy
  and Paste for what is selected in the line. It sits above, below or beside the line, clear of other text where
  there is room, never over the line; clicking it leaves the line open.
- Right-click what is selected in the line for Cut, Copy and Paste, then Bold, Italic, Underline and
  Strikethrough: the selection stays as it was. New text, a note and a text box being typed have it too, with the
  styles where the text takes them.
- Going to another tab and back leaves the line open, ready to go on typing.

A line turned on the page (not running left to right on screen) is edited in a box over the line instead, showing
its fonts, sizes and colours; invisible recognised (OCR) text is edited in a plain box. The caret goes by the
character you clicked along the line, and a double-click selects the word you clicked, whichever way the line runs.

## Edit a paragraph

To edit whole paragraphs, their lines rewrapping as you type, as in a word processor, turn on **Paragraph** in
**Edit > Text flow** in the band. It shows pressed while it is on. It is also **Edit as paragraph** in the
right-click menu and the command search (Ctrl+K), and **Paragraph** in the inspector's **Editing** row while text
is selected. Then:

1. Double-click a line of the paragraph with the read tool, or click it with the Text tool (or choose **Edit >
   Edit text** with it selected).
2. Type your change. Press Esc or Tab, or click elsewhere, to keep it.

**Paragraph** stays on, in every document and the next time you start LimarePDF, until you turn it off. Turned on
or off while you edit a line or a paragraph, it switches what you are editing: what you typed is kept first, and
the line or paragraph where the caret is opens. **One line** in the inspector turns it off too.

Off, a double-click edits a line: LimarePDF finds paragraphs from where their lines sit on the page, but it
doesn't yet find every one reliably enough to open them as paragraphs by itself. With **Paragraph** on, a line on
its own, text in a table and turned text are still edited a line at a time: the inspector's **Paragraph** is
greyed for them, and says why when you point at it.

While you type:

- The paragraph is drawn on the lines it will be written on, in the fonts it will be written in, with an outline
  round its frame and its old lines hidden underneath by the page as it is without them, as a line's are. The
  status bar says **Editing a paragraph**.
- Lines break where Word and Google Docs would break them, inside the paragraph's own edges and alignment (left,
  centred, right or justified, a justified line's spaces stretched to its edge). Lines before the change, and
  after the point where the line breaks fall where they did, stay exactly as they were: a typo fixed usually
  rewrites only its own line.
- To make the paragraph narrower or wider, drag the small square on the left or right edge of its outline: the
  lines rewrap as you drag, as a text box does in a word processor when you resize it. Or click the square (or
  press Tab from the text to reach the left edge, then the right) and use the Left and Right arrow keys to move
  that edge a point at a time, ten with Shift; click in the text to type again. The edges stay on the page, and
  the paragraph can't be narrower than its longest word. Lines whose words and place stay the same are left as
  they were.
- To align the paragraph, use **Edit > Alignment** in the band: **Align left**, **Centre**, **Align right** and
  **Justify**, the paragraph's own shown pressed. Or press Ctrl+L, Ctrl+E, Ctrl+R or Ctrl+J (⌘L, ⌘E, ⌘R or ⌘J on a
  Mac), as in Word. The lines move across between the paragraph's edges at once, keeping their words; justified,
  every line but the last reaches both edges. Pressing Centre, Align right or Justify again puts the paragraph back
  to the left, as in Word. Centred and right-aligned lines take no first-line indent; aligned left or justified
  again, the paragraph has its own back. The paragraph is aligned between its own edges, not the page's margins:
  drag an edge first to give it more room. Centre is on the floating toolbar too, and all four are in ☰ > Edit >
  Align text and the command search (Ctrl+K: type "align" or "centre").
- Once you move a paragraph's edges or change its alignment, LimarePDF keeps a small note of its edges and indent
  in the file (in the page's private data, which other apps don't show), so the paragraph opens again with the
  same edges, after saving too. Without it, a centred paragraph would open narrower, as its lines no longer
  reach its edges. If another app moves the paragraph's lines, its edges are worked out from the lines again.
- **Enter** starts a new paragraph within the frame, with the first line's indent if it has one; **Shift+Enter**
  starts a new line.
- Each word keeps its own style, and letters keep their font, as in a line (see
  [Edit a line of text](#edit-a-line-of-text)). Ctrl+B, Ctrl+I, Ctrl+U, **Edit > Font**, the floating toolbar and
  the right-click menu act on what is selected in the paragraph. The inspector shows its font and size, the PDF's
  font and how what you type is written, and names the words that go to another font.
- A hyphen that ended a line: LimarePDF reads from the rest of the document whether it belongs to its word
  ("well-known") or was only put in to break it ("exam-ple"). One put in to break a word goes when the word moves
  inside a line, and comes back if it ends a line again. One it can't tell is kept as a hyphen and, once a rewrap
  brings it inside a line, marked with a dotted line under it: right-click the word for **Join the hyphenated
  word** or **Keep the line-end hyphen**, or choose **Join all** in the inspector.
- When the paragraph grows past the page's edge, or over text, a picture or a shape it wasn't already over, the
  outline turns the caution colour and a tag just below the line where it happens says **Past the page edge**,
  **Over other text**, **Over a picture** or **Over a shape**. Nothing else on the page moves: a longer paragraph
  is written over what follows it, so make room, shorten it or widen it.
- While a link or a comment is over the paragraph, the inspector says **Links and comments over this paragraph
  stay in place**: they keep their place on the page, and don't move with the words under them.
- Ctrl+Z and Ctrl+Y (and **Undo** and **Redo** in the band) undo and redo your typing inside the paragraph, each
  move of its edges (a drag, or a run of arrow keys), with a frame edge's square selected too, and each change of
  its alignment. Once kept, the whole change is one undo step. Opening a paragraph and leaving it as it was changes nothing.

Words you select with the read tool in one paragraph (with **Paragraph** on; with it off, across the lines of one
LimarePDF is sure is a paragraph) are typed over, deleted and cut in the paragraph, which rewraps; see
[Type over, delete or cut words](#type-over-delete-or-cut-words).

## Style words in a line

1. Double-click the line, then select the words: drag over them, or hold Shift and use the arrow keys. A
   double-click selects one word.
2. Press Ctrl+B for Bold, Ctrl+I for Italic or Ctrl+U for Underline, use **B**, ***I***, **U** and **S** on the
   floating toolbar, or use **Edit > Font** in the band: the font list, size, Bold, Italic, Underline,
   Strikethrough and text colour.
3. Press Enter, Esc or Tab, or click elsewhere, to keep it.

As in a word processor:

- With nothing selected, the caret inside a word changes that word. Between words, or at either end of a word,
  the change applies to what you type next there.
- Bold over words that are partly bold makes them all bold; over words that are all bold, it takes the bold off.
  Italic, Underline and Strikethrough work the same way. Words in a family without a bold or italic face stay as
  they are: **B** or ***I*** is greyed, and says why when you point at it, and Ctrl+B or Ctrl+I says so at the
  caret.
- Strikethrough has no key on Windows and Linux, as in Word: use **S**, the right-click menu or the command search
  (Ctrl+K). On a Mac it is ⌘⇧X, as in Word for Mac.
- An underline or strike-through shows as you type where it will be written: at the font's own place and
  thickness for it, in the words' colour. Spaces inside an underlined stretch are underlined; spaces at either end
  of the line are not.
- Where the selection mixes fonts, sizes or colours, **Edit > Font** shows that one blank: the font list with no
  name, the size box empty, the colour button (an **A** over a bar of the text's colour) with the colours side by
  side in its bar. A colour picked then makes the
  selection all that colour, even the one the picker starts from (the first); the picker also offers the colours
  in the text (**In the text**), to make it all one of them in a click. Opening and closing the picker changes
  nothing. Bold, Italic, Underline and Strikethrough show on only when all
  of it has that style. Picking a value there sets all of it.
- A change in **Edit > Font** shows on the line at once, the line stays in editing, and the keyboard comes back to
  it; the change is kept with the typing.

Finishing (Esc, Enter, Tab or clicking elsewhere) leaves nothing selected, as in a word processor. A line selected
as a whole with the arrow takes the font, size, colour and styles in **Edit > Font**, and Ctrl+B, Ctrl+I and
Ctrl+U, as a whole, with the same blanks for what it mixes; to change its words, double-click it or choose
**Edit > Edit text**, and it is edited on the page here too. With the arrow, the line stays selected when you
finish.

## Style words you select while reading

You don't need to open a line to style its words. Select them with the read tool, as you would to copy them (drag
over them, across lines and pages if you like), then:

- press Ctrl+B for Bold, Ctrl+I for Italic or Ctrl+U for Underline, or use Bold, Italic, Underline and
  Strikethrough on the floating toolbar or in the right-click menu;
- or pick a font, size or colour in **Edit > Font** in the band. The inspector, headed **Selected text**, shows
  their font and size.

The words stay selected, so you can go on changing them. Bold, Italic, Underline and Strikethrough work over the
whole selection as they do in a line, and **Edit > Font** shows a value blank where the selection mixes it. Each
change is one undo step, however many lines and pages it covers.

The words around the ones you change are left exactly as they were, in their own font, even one LimarePDF can't
write new text in. Words whose size or colour is all you change keep their font too; Bold, Italic or another font
writes them in that font. Underline and Strikethrough never write the words again: they add a thin line under or
through them in their colour, so they work in any font. Invisible recognised (OCR) text, text inside a block of
page content reused as a whole, and vertical writing keep their look; a short notice says when part of the
selection was left that way.

Underlines and strike-throughs another app drew under or through a line's words are read as the words' own where
LimarePDF is sure of them (see [Limits](#limits)): they go with the words when you edit, move or delete the line,
Underline or Strikethrough in **Edit > Font** shows on for them, and Ctrl+U takes them off. An underline you add
next to one of these, on the same line and in the same colour, carries on in its look, as one line. A double
underline stays double. Clicking one with the arrow selects its line, as an underline is part of its words.

## Type over, delete or cut words

Words you select with the read tool can be changed as in a word processor, without opening their line first:

- **Type** to replace them. Their line opens for editing with what you typed in their place, and you go on typing
  there; press Enter, Esc or Tab, or click elsewhere, to keep it. **Ctrl+V**, or **Paste** (in **Edit** in the
  band, the right-click menu or the floating toolbar), pastes over them the same way. Over words of one paragraph
  (with **Paragraph** on; with it off, on several lines of one LimarePDF is sure is a paragraph), the paragraph opens
  with what you typed in their place (see [Edit a paragraph](#edit-a-paragraph)). Over words in different
  paragraphs nothing is typed, and a note says "Type over words within one paragraph." With no words selected and
  nothing being typed, Ctrl+V says where to click to paste. An input method types over them too: the line opens
  with what you compose in their place.
- **Delete** or **Backspace** removes them, across lines and pages, in one step that Undo takes back. As in Word,
  a space beside them goes too when two would be left together, or one at the start or end of a line. In one
  paragraph (as for typing), they are taken out of the paragraph, which rewraps and stays open with the caret where
  they were. Elsewhere a line left empty is gone, and the lines around it stay where they are, as lines aren't
  joined together.
- **Cut** (Ctrl+X, in **Edit** in the band, the right-click menu or the floating toolbar) copies them, then removes
  them as Delete does.
- **Edit > Edit text** (also in the right-click menu and the floating toolbar) opens their line with them selected,
  to type over them or style them. Over more than one line of one paragraph, it opens the paragraph with them
  selected; over lines of different paragraphs, the line where the selection ends.

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
caution colour if the text runs off the page or into text, a picture or a shape, with a tag just below the line
where it happens saying which (**Past the page edge**, **Over other text**, **Over a picture**, **Over a shape**).
A picture or coloured box the first line starts on doesn't count, so you can put a label on one. Every line is
added as one undo step, and nothing typed adds nothing. Each line becomes its own line of page text, so
afterwards it moves and is edited line by line.

While you type, Ctrl+B, Ctrl+I, Ctrl+U and **Edit > Font** style what is selected, or what you type next, as when
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

**Characters:** Latin (Vietnamese included), Greek, Cyrillic, Chinese, Japanese and Korean characters, and single
emoji, can be added in any font. As in a word processor, a word the font lacks is written whole in a font that has
it, embedded in the file (cut down to the characters used):

- letters Helvetica, Times or Courier lack (Polish, Czech, Greek, Cyrillic and so on) in that font's Liberation
  face;
- letters an installed font lacks in Noto Sans or DejaVu Sans (their serif or monospaced kind for a serif or
  monospaced font), else in the font's Liberation look-alike;
- Chinese, Japanese and Korean, a character at a time, in an installed font for the language, such as Noto Sans
  CJK or Source Han Sans (their serif kind for a serif font): the document's language says which, else the characters do
  (kana are Japanese, Hangul Korean), else your computer's language;
- an emoji in an installed font that draws it in outline, such as Noto Emoji, Symbola or DejaVu Sans (colour
  emoji fonts can't be written into a PDF);
- anything else in the first installed font that has it.

The text is drawn in those fonts as you type, and the inspector names the words and the fonts they use ("Not in
Calibri: 漢字. Those characters use Noto Sans CJK JP."), which word uses which when you point at it. Characters no
installed font has, Hebrew, Arabic and the other scripts written right to left or with joined letters
(Devanagari, Thai and so on), and emoji made of several characters (flags, skin tones, keycaps, joined emoji such
as families) can't be typed or pasted, and a note at the caret says why: "No font on this computer has this
character", or that LimarePDF can't add that script or emoji yet. The same goes for editing a line, text boxes and
the inspector's text field (see [What doesn't work yet](developer/feature-status.md#what-doesnt-work-yet)).

New text is set as the font sets it: kerned (the space between pairs such as AV and To closed up), with the
font's ligatures (fi) and accents placed by the font, in installed fonts and Liberation. Helvetica, Times and
Courier are kerned with their Liberation face's pairs, and have no ligatures.

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

Pictures, shapes and lines of text snap into line with the page and other objects as you drag them, a guide
showing where; hold **Alt** (Option on a Mac) to place them freely (see [Line up objects](#line-up-objects)).

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
them all. Right-click one of them for Copy the selected text lines (in reading order), **Align objects** (to line
them up or space them out), Cut (when they are all lines of text) and Delete. Each change is one undo step.

Several objects are selected on one page at a time. They move, restyle and delete together, and line up or space
out evenly with **Align objects** (see [Line up objects](#line-up-objects)), but are resized one at a time.
Comments and markup are selected one at a time, never with page objects.

## Line up objects

**Align objects ▾**, in the **Arrange** group of **Edit** in the band, lines up the selected objects or spaces
them out evenly, as Word's Align does. Its menu has **Align left**, **Align centre**, **Align right**, **Align
top**, **Align middle** and **Align bottom**, then **Distribute horizontally** and **Distribute vertically**, then
**Snap to objects**. The same commands are in ☰ > Edit > Align objects and in the command search (Ctrl+K: type
"align", "distribute" or "snap"). Right-click selected objects for an **Align objects** submenu of the Align and
Distribute commands that apply just then. It moves pictures, shapes and lines of text on the page; it doesn't
line text up inside a paragraph.

1. Select the objects: Shift+click them, or drag a rectangle round them with the arrow (see
   [Select several objects](#select-several-objects)).
2. Choose **Edit > Align objects**, then how to line them up.

Two or more objects line up with each other, by the box round them all:

- **Align left** puts every left edge at the leftmost one, and **Align right** every right edge at the rightmost
  one.
- **Align top** puts every top at the highest one, and **Align bottom** every bottom at the lowest one.
- **Align centre** puts each object's middle at the middle of the box round them all, across the page; **Align
  middle** does the same down the page.

One object on its own lines up with the page instead: **Align left** and **Align right** put it at the page's left
or right edge, **Align top** and **Align bottom** at its top or foot, **Align centre** centres it across the page,
and **Align middle** puts it halfway down. Objects keep their size.

**Distribute horizontally** and **Distribute vertically** space out three or more objects: the first and last (the
leftmost and rightmost, or the top and bottom ones, judged by their middles) stay where they are, and those
between move so that the gaps between neighbours are equal. With fewer than three selected they are greyed, and
say "Select three or more objects first."

Align and Distribute act on what the tool in use can move. With the arrow (**Edit > Select > Select, move and
resize objects**), that is any object, page text included. With the read tool, it is pictures and shapes, and page
text only after **Move or resize text** (see [Move, resize or delete](#move-resize-or-delete)) or text you have
just added. Otherwise they are greyed, and say "Select something the tool in use can move first." Each Align or
Distribute is one undo step; objects already lined up are left as they are, with nothing added to undo.

Objects also snap into line as you drag them. While you drag a picture, a shape or a line of text to move it, or
one of its handles to resize it, its edges and middle snap to the page's edges and middle, and to the edges and
middles of the other objects on that page, when they come within a few pixels of them on screen. A dashed pink
guide line shows along the line it snapped to and the objects that line up there. A resize snaps only the edges
you are dragging; with Shift on a corner, keeping the proportions, the nearer edge snaps and the other follows.

Hold **Alt** (Option on a Mac) while you drag to move or resize freely, without snapping, as in Word. **Edit >
Align objects > Snap to objects** turns snapping off, or back on: it is on at first, ticked while it is on, and remembered
the next time you start LimarePDF. The arrow keys still move selected objects by 1 point (10 with Shift), and
never snap.

Comments and markup don't snap and can't be aligned, as they are selected one at a time. Objects are lined up on
one page at a time, and there is no grid or ruler.

## Change size, font and colour

For text, **Edit > Font** in the band has the font list, then size, Bold, Italic, Underline (an underlined **U**),
Strikethrough (a struck-through **S**) and colour (the words themselves are changed on the page). Pointing at the
font list shows the names of the fonts the selected text uses in the PDF.

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
stays open or closed as you left it. Changes in the band and the inspector are kept as you make them, without an
Apply button: typed text when you press Enter or leave the box, a number when you stop dragging it or leave its
box, a colour when its picker closes. Each is one undo step.

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
- **Editing a paragraph** finds the paragraph's lines and its frame (its edges, line spacing, alignment and first
  line's indent), and sets the edited text in that frame. Each rewritten line goes where its old line was in the
  page's content, so what draws over what and the text's tags for screen readers stay; lines that don't change are
  left exactly as they were. Each written line is read back before the change is kept, and one that would read as
  other words is written in the similar font instead, with a notice; if it still wouldn't, nothing is changed and a
  message says so.
- **Adding** text, shapes or pictures draws them over the page. Each typed line becomes its own line of page text,
  1.2 times the size below the one before (as Acrobat does); with sizes that differ, 1.2 times a quarter of the
  line above's largest size and three quarters of its own. Text in Helvetica, Times or Courier with characters
  those fonts lack (Polish, Czech, Vietnamese, Greek, Cyrillic and so on) is written in the bundled Liberation
  face of that font, which has the same widths, embedded in the file and cut down to the characters used; a word
  that face lacks too goes to an installed font that has it (see [Add text](#add-text)).
- **Every kind of new text** (text added or edited, text boxes, form values, headers, footers, watermarks, stamps,
  fill-and-sign text) is set the same way: each word in the first font that has it, kerned, and broken into lines,
  where a box wraps, at the places Unicode's line-breaking rule allows (spaces, after hyphens, between Chinese or
  Japanese characters, never before closing punctuation). While you type, it is drawn in the fonts and on the
  lines it will be written in.
- **Installed fonts** (the font list in **Edit > Font**, after the three standard ones) are the families whose
  licence allows embedding, with up to four styles each. Text in one is always embedded in the file, cut down to the
  characters used, so it looks the same on any computer.
- **Recognised (OCR) text** stays invisible and in place when edited, and text after a changed line stays where
  it was.

Every edit is checked before it is applied, and one that fails leaves the document unchanged. The technical
detail is in [How it is built](developer/architecture.md#content-editing).

## Limits

- Text objects are lines recovered from the content stream by heuristics. Paragraphs are found from where those
  lines sit, and open as paragraphs only with **Paragraph** turned on: LimarePDF's check of how well it finds them
  isn't passed yet, so it starts off and a double-click edits a line. Outside a paragraph, typing over words selected in
  different paragraphs types nothing, and deleting words leaves each line where it was, an emptied one blank. Text
  boxes are estimated from font metrics, so they can be slightly off for unusual fonts. A line the page draws twice
  (as maps draw a label over a halo of its own) is changed where you click it; the copy under it keeps the old
  words.
- A paragraph grows and shrinks downwards in its frame; nothing else on the page moves, and text doesn't flow on
  to the next column or page. Links and comments stay where they are, so a paragraph rewrapped or made narrower
  can move its words out from under them. While you type in a line or a paragraph, a comment another app saved
  without a look of its own (which LimarePDF draws for it) is hidden with the old text until the change is kept;
  other comments and form fields show. A typed word takes the style of the character before it, so typed
  right after a symbol (a bullet, say) it is in the symbol's font. A letter TeX draws as its accent over its letter (an older way of writing
  "ü") reads as the accent, then the letter ("Pr¨ufung"), in the paragraph you type in too; it is written back as
  it was drawn, so other readers still read it as one letter.
- A changed line keeps its font only for letters the font verifiably holds: one that is cut down to the letters
  the file uses (a subset) can't take others, and a Type 3 font, a font not held in the PDF (beyond plain English
  letters in a standard one), vertical writing and fonts whose codes LimarePDF can't read take none. A word with
  such a letter is written in the installed family the font is from when LimarePDF can embed it, else in
  Helvetica, Times or Courier (or their Liberation face, as new text is), and the inspector says so; the words
  left as they were keep their font. Letters a font draws joined (a ligature such as "fi") keep it only together
  and unchanged. Where the changed line would read as other words (when it runs into the text after it, say, or a
  space it had as a small gap after a raised or lowered part no longer reads as a space), it is written in the
  similar font instead, and a notice says so. Text written anew is limited to Latin, Greek, Cyrillic, Chinese,
  Japanese, Korean and single emoji, each in a font that has it: the app doesn't let others be typed, and a line
  that already has them (Hebrew in a line written anew, say) can't be written anew; the change is refused with a
  message naming them. Right-to-left and shaped scripts (Hebrew, Arabic, Devanagari, Thai and the rest) are
  refused in every font, as they would come out wrong without reordering and shaping, even in a line's own font.
- A word written in another font because its font lacks a letter can look a little different in size and weight
  beside its neighbours, as in a word processor; the inspector names it.
- Installed fonts whose licence restricts embedding aren't offered or used. Those with no Latin letters aren't
  offered, but are used for the characters they have. A family's missing bold or italic isn't made up. The list
  is read once, when the app starts (on another thread), with the fonts for emoji, and for Chinese, Japanese and
  Korean when your computer's language is one of them: fonts installed while it runs are used after a restart.
- While a line that keeps its font is edited, its character and word spacing, horizontal scaling (`Tc`, `Tw`,
  `Tz`) and the fine spacing between its letters aren't drawn, a font not held in the PDF is drawn in the Liberation
  look-alike of the Helvetica, Times or Courier LimarePDF's pages show it in (Chinese, Japanese and Korean in the
  installed font they show it in), and the letters of a ligature in the Liberation look-alike, so the line can look
  slightly different until you finish. A part raised high above the line (a large
  superscript) is drawn lower while you type, within the line, and is written where it was. Turned lines are
  edited in a box over the line, and invisible text in a plain box.
- A word written in another font (a letter its font lacks, or another font chosen) loses its own character and
  word spacing and horizontal scaling, and words moved by cut and paste lose the fine spacing between their
  letters; letters left as they were keep theirs.
- Underline and Strikethrough are one plain line in the text's colour: there are no double, dotted, dashed or wavy
  ones, and no colour of their own. They aren't offered in text boxes (comments), form fields, headers and
  footers, watermarks and stamps, where they are greyed, nor on the command line (nor are Bold and Italic). An
  underline or strike-through another app drew that LimarePDF can't be sure of (in another colour than its text,
  under one character alone, dotted, dashed or wavy, or touching other drawings such as a table's rules or a box)
  is left a drawing, and stays where it is when its line changes.
- Styling words doesn't rewrap: bold or larger words make their line longer and the rest of it moves along, and
  highlights, links and redaction marks over them stay where they were.
- Alignment (Align left, Centre, Align right, Justify) works on a paragraph being edited only, for now: a line
  edited alone, words selected while reading, new text and text boxes can't be aligned yet, and the buttons are
  greyed for them. A paragraph is aligned between its own edges; there are no indents, line spacing or spacing
  before and after a paragraph to set.
- Editing a line does not change its width, and a paragraph keeps its frame's width unless you drag its edges:
  longer text can overlap its neighbours. The outline warns of it, a tag where it happens, but nothing is moved
  out of the way.
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
- Comments and markup don't snap and can't be aligned (they are selected one at a time). Objects are lined up on one
  page at a time, and there is no grid or ruler.
- Moving an object does not move links or annotations over it.
- Text inside a form XObject (a reusable block of page content) is searched and selected like other text, but it is
  not a separate object: the form moves, resizes and deletes as a whole, and its text cannot be edited.

---

[← Comments and markup](comments-and-markup.md) · [User guide](README.md) · [Filling in forms →](forms.md)

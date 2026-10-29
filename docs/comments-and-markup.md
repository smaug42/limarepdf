[User guide](README.md) › Everyday tasks

# Comments and markup

Highlight text, add notes, text boxes, shapes and drawings, and answer other people's comments. All of these are
annotations: other PDF readers show them and list them as comments, and they can be changed or removed later, here
or in another app. To make them part of the pages for good, [flatten](#flatten-comments-into-the-pages) them.

## Highlight text

1. Select text with the read tool (the text cursor).
2. Click **Review > Highlight**, or **Highlight** on the small toolbar above it, or right-click > Highlight, or press
   Ctrl+Shift+H.

**Review > Highlight ▾**, the small arrow next to Highlight on the toolbar, or right-click > Underline, strike out,
colour, has:

- Underline, Strike out and Squiggly underline.
- The highlight colour (yellow, green, blue, pink, orange) and the line colour (red, blue, green, black). The
  colours chosen are remembered between runs.

A selection across pages gets one highlight on each page, all in one undo step.

## Add a note or a text box

**Review** in the command band holds the commenting tools: **Note**, **Text box**, **Draw** (the pen; its ▾ has
rectangles, ellipses and lines) and **Stamp** (also in ☰ > Tools > Commenting).

**A note** (a sticky note with a comment):

1. Pick **Review > Note**, and click on the page.
2. Type your comment in the box that opens beside the note.
3. Click on the page outside it, or press Esc or Ctrl+Enter, to keep it. That click doesn't put down another note.

**A text box** (text shown on the page):

1. Pick **Review > Text box**, then drag a box on the page, or click for a default size.
2. Type straight onto the page. Enter starts a new line.
3. Click outside the box, or press Esc or Ctrl+Enter, to finish.

Before you start, or while you type, the inspector sets the note's colour, or the text box's text size and colour,
its background (none at first) and its border. When you finish, the read tool comes back with the new note or text
box selected, so you can move it straight away.

More about notes and text boxes:

- With nothing typed, no note or text box is added. While you type, Ctrl+Z and Ctrl+Y undo and redo the typing;
  once it is kept, Ctrl+Z takes the whole note or text box back. Finishing by choosing another tool keeps that
  tool.
- While you type a note, the colour in the inspector changes the note's colour: clicks in the inspector and its
  colour picker leave the note open, and Esc, or a click that closes the picker, closes only the picker. While you
  type a new text box, the inspector changes its text size and colour, background and border the same way, the box
  showing the size you pick, and the keyboard comes back to it. The same
  goes for a note or text box already on the page that you double-click to type in: the inspector changes it
  while it stays open, and the keyboard comes back to it. Its text is typed on the page only, so the inspector's
  own Comment or Text box is hidden meanwhile; the inspector's Delete removes it, with what you typed. Only a click
  on the page outside it closes it.
- Leave a note open to change it in the inspector and come back: Ctrl+Z still undoes what you typed in it. A new
  note or text box starts with nothing to undo.
- Notes show at the same size on screen whatever the zoom, in the note's colour and shaped like its icon: one of
  the seven standard ones (Comment, which LimarePDF's notes use, Key, Note, Help, New paragraph, Paragraph and
  Insert), or Note for any other. Printed, they keep their own drawing, or LimarePDF's, 20 points across. Hovering
  over a note shows its comment.
- Text boxes use Helvetica, and for Polish, Czech, Vietnamese, Greek, Cyrillic and other characters it lacks, its
  Liberation Sans face, embedded in the file. Other characters (Chinese, Hebrew, emoji and so on) can't be typed or
  pasted, on the page or in the inspector, and a note at the caret says so. Chinese, Japanese, Korean and single
  emoji can be written on the page instead with the Add text tool, in an installed font that has them. Hebrew,
  Arabic and other right-to-left or shaped scripts, emoji made of several characters (flags, skin tones, joined
  emoji) and emoji a font draws only in colour can't be written in any font yet
  ([details](content-editing.md#add-text)). A note's comment takes any character, emoji included.
- When you finish a text box, it grows downward to fit its text, keeping its width. It stops at the bottom of the
  page, and never shrinks (resize it by dragging its handles).
- The Text box tool can be kept on to add several in a row (see [Keep tool active](gui.md#keep-tool-active)). The
  Note tool can't, so after each note the read tool is back to select, move or open notes.

## Draw shapes, lines and freehand

**Review > Draw** turns on the pen, and **Review > Draw ▾** has the rectangle, ellipse and line tools too.

- **Rectangle, ellipse, line:** drag on the page, or click for a default size. Shift keeps to squares, circles,
  and lines at multiples of 45°. Before you start, the inspector sets the outline (or line) colour and width, the fill
  of rectangles and ellipses (off until you turn it on), and an arrowhead at the end of a line.
- **Pen:** drag to draw; a click makes a dot. Strokes started within 1.5 seconds of the last one, on the same page,
  join the same drawing, so a word, or a circle with a tick, is one annotation. Each stroke is still its own undo
  step. The inspector sets the pen's colour and width. The pen stays on until you press Esc or pick another tool,
  since a drawing takes several strokes; Esc during a stroke drops only that stroke, and the pen stays. Points closer together than about a third of a point to the line drawn
  are dropped, which keeps files small.
- **Draw on page (permanent):** with this switched on in the inspector, the rectangle, ellipse and line tools draw
  into the page content instead, as part of the page rather than as comments. Those shapes are selected with the
  arrow as page objects (see [Editing text and pictures](content-editing.md)).

Stamps (Approved, Draft, your own wording or a picture) are **Review > Stamp**: see
[Stamps](headers-footers-watermarks.md#stamps).

## Change, move or delete a comment

Click a highlight, note, shape or any other annotation with the read tool to select it. (With another tool on,
Ctrl+click it.) It is outlined, and the inspector shows who made it and when, with its colour, opacity and comment,
and for a shape its width, fill and arrowhead, for a text box its text, size, colours, background and border.

- **Change it** in the inspector. Changes are kept as you make them, each one a step to undo; a comment being typed
  is kept when you leave its box.
- **Change its text:** double-click a note or text box, and type on the page as when you added it.
- **Move it** by dragging it, or with the arrow keys (1 point at a time, or 10 with Shift). **Resize it** by
  dragging a handle.
- **Delete it** with Delete or Backspace. Esc lets it go.

Some things to know:

- Notes keep their size, and highlights and other text markup stay on their text, so neither can be resized, and
  markup can't be moved. Annotations the file locks can't be moved or deleted.
- A drag that starts on a highlight still selects the text under it. Hovering over a mark shows its comment.
- Lines and drawings are picked by clicking near their strokes, not anywhere in their box.
- A selected line has a round handle on each end instead of a box: drag either end to turn the line, or to point
  an arrow somewhere else (Shift keeps it at a multiple of 45° from the other end). Drag the line itself to move
  it.
- A selected freehand drawing has a round rotate handle above its box; the pointer becomes a circular arrow over
  it. Drag it around to turn the drawing about the middle of its box. The angle shows while you drag, and Shift
  keeps to steps of 15°. Rectangles, ellipses and text boxes can't be turned: PDF stores them as upright boxes.

## See every comment, and reply

The **Comments** view shows the document's annotations as cards under the name of their page, with replies
indented under the comment they answer: click its icon on the left or **Review > Comments**, or choose ☰ > View >
Comments. Each card shows who made it, what it is and when, and the first lines of its comment; a highlight or
other text markup with no comment shows the words it marks. Hover over a card for the whole comment.

- **Click a card** to select its annotation and show it on the page. Selecting an annotation on the page selects its
  card too, and brings it into view.
- **Previous and Next:** **Review > Previous** and **Review > Next** (or ☰ > View > Previous comment, Next comment)
  select the comment before or after the one selected, in the order the Comments view shows them, and show it on
  the page. With nothing selected, Next starts from the page you are reading.
- **Search** with the box at the top: the cards whose comment, replies or marked words hold what you type stay.
- **Filter ▾** shows only some types (Note, Highlight and so on), some authors, or the page you are reading. **Clear
  filters** shows them all again. The count above the cards says how many show, such as "4 of 12 comments". The
  filter and the search belong to the document's tab, and nothing about them is saved in the file.
- **With the keyboard:** click a card, or Tab to the list, then Up and Down go to the card before or after, Home
  and End to the first and last, each selected and shown on the page. Enter puts you in the reply box, Delete deletes
  the comment, and Esc goes back to the page, leaving the comment selected.
- Right-click a card to delete it, along with any replies. Undo brings it back.

**Reply:** the selected card's **Reply** button (or Reply… on its right-click menu, or Enter in the list) puts you in
the Reply box at the bottom of the inspector, which lists the replies the comment already has. **Add reply** or
Ctrl+Enter adds it. A reply you leave unfinished, by selecting another comment or another document, stays with its
comment as a draft, as in Word and Google Docs: its card says **Draft reply**, and it is back in the box when you
select that comment again. Saving, printing and the other commands that change or use the document add every draft
first, and closing asks about them, as for any text still being typed; Undo and Redo leave drafts as they are.
Replies are saved as standard reply comments, which aren't shown on the page or printed, and carry your name.

## Your name on comments

New comments are saved with your name, which is the computer's user name until you set it with **Review >
Comments ▾ > Your name for comments…** (or ☰ > Edit > Your name for comments…). Leave it empty to save no name.

## Flatten comments into the pages

**Review > Flatten…** (or ☰ > Document > Flatten annotations…) draws all the annotations into the pages as they look
now and removes them, so they are no longer comments in any reader: they can't be changed or removed as comments.
Their comment text is dropped. What was drawn is page content, so content editing can still move, resize or delete
each as a whole ([Move, resize or delete](content-editing.md#move-resize-or-delete)). Links and form fields stay,
and so do file attachments. Undo brings them back until you close the document.

## How comments are drawn

Some files store annotations without a drawing of their own, leaving each reader to draw them. LimarePDF draws
highlights, underlines, strike-outs, squiggly underlines, notes, text boxes, rectangles, ellipses, lines, polygons,
freehand drawings, stamps, carets and file attachment icons itself, on screen and when printing, including cloudy
borders, text box callout lines and line leader lines. The file isn't changed by this; flattening saves LimarePDF's
drawing into it (file attachments stay as they are). Text boxes are drawn in Helvetica, ignoring any rich-text
styling; a stamp shows its name in a coloured frame. A text box with characters Helvetica can't write (Chinese,
Hebrew and so on) shows them as `?` when LimarePDF draws it, so flattening refuses one without a drawing of its
own, naming its page, and a text box with its own drawing keeps it: a new colour, style, size or text for it is
refused ([Troubleshooting](troubleshooting.md#limarepdf-cant-draw-this-text-box-again)). [Rendering](rendering.md#known-to-be-wrong) lists what isn't
reproduced.

The app shows up to 10,000 annotations on a page, with a notice on a page that has more.

## From the command line

`limarepdf-cli annot` lists, removes and flattens annotations: see [Annotations](cli.md#annotations).

---

[← Organising pages](organising-pages.md) · [User guide](README.md) · [Editing text and pictures →](content-editing.md)

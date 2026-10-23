# Using the desktop app

![The LimarePDF window with a three-page report open](images/limarepdf.png)

## Opening files

Open PDFs with the Open button, Ctrl+O, or by dropping them on the window. Image files (PNG, JPEG, TIFF) open as a new
document with one page per image. From a terminal, `limarepdf file.pdf --page 3` opens a file at page 3.

On Linux in a Wayland session (the default on current KDE and GNOME), dropping files on the window does nothing:
the windowing library LimarePDF uses doesn't take file drops from Wayland yet, and the pointer shows a "no" sign.
Opening by any other way works, and so does dragging pages within the app. If you want drops, start LimarePDF
through XWayland, Wayland's X11 compatibility layer, with `env -u WAYLAND_DISPLAY limarepdf`. The cost: with
fractional display scaling (such as 125% or 150%) the window may look blurry, unless your desktop lets X11 apps
scale themselves (in KDE, System Settings > Display & Monitor > Legacy Applications (X11)). To make it the
default, copy LimarePDF's menu entry (`io.github.smaug42.limarepdf.desktop`, usually in `/usr/share/applications`) to
`~/.local/share/applications` and change its `Exec=` line to start with `env -u WAYLAND_DISPLAY`.

Recently opened PDFs are listed on the start screen and under ☰ > File > Open recent, which also clears the list.
Files that no longer exist are left out. The list holds 10 files and is kept on your computer only, in
`~/.config/limarepdf/recent.txt` on Linux, `~/Library/Application Support/limarepdf/recent.txt` on macOS, and
`%APPDATA%\limarepdf\recent.txt` on Windows.

The start screen also has Make a PDF from images…, which turns photos or scanned pages into a new document.

Each document gets a tab. The tab shows its name, with `*` while it has unsaved changes.

Closing a document or quitting asks first when there are unsaved changes, including text still being typed: a comment
or text box on the page, a form field, the selected comment's changes in the inspector, or a reply not yet added.
Saving keeps that text first, as clicking away would (Undo takes it back).

Dialogs (questions, messages, Print, Create signature, the digital signature windows, About) open over a dimmed
window, which takes no clicks or shortcuts until the dialog closes. Each names its task, and the document when it acts
on one. The button that does the task is highlighted, or red when what it does can't be taken back with Undo, or
breaks signatures; on Windows and Linux it comes before Cancel, on macOS after. Esc is Cancel, except in a window
holding work you would lose (a signature drawn, typed or chosen; a digital ID form; a signing form with something
typed), where only Cancel closes it. A message's Copy button copies what it says, to pass on an error. Reasons and
technical lists in a question are under Details. A dialog fits the window, also as it is resized: narrower in a
narrow one, and in a short one its content scrolls while its buttons stay in view.

A question about a document (Flatten, Apply redactions, Discard changes and the others) acts on the document it was
asked about, even if another tab has come to the front meanwhile, and brings it back to the front. If that document
changed or was closed while the question was open, nothing is done, and a message says so.

Saving writes a temporary file beside the document and then replaces the document with it, so a save that fails
part way leaves the original as it was. On Linux and macOS, the temporary file is readable only by you while it is
written, and the saved document keeps the original's permissions; afterwards LimarePDF asks the system to confirm the
change is on disk, so a power cut can't undo it. If the system can't confirm it, a message says "The file was saved,
but your system couldn't confirm it is safely on disk. If the computer lost power just now, save it again.", and the
document stays marked unsaved so saving again is one click. (Windows has no way to confirm this: there a power cut
in the moment after a save can still lose it.) LimarePDF saves by rewriting the whole file, except for a digitally signed
document: its changes are added after what was signed, so the signatures stay valid. LimarePDF asks first when the
signatures don't allow the changes, and before writing a signed document in full when it must (after redacting, for
example), which breaks the signatures; see [Saving a signed document](signatures.md#saving-a-signed-document).
Save As to a new file keeps the signed original.

## The window

- **Top bar:** the menu (☰), Open, Save and an Export menu (pages as images, text); Undo and Redo; the tools: Read
  and select text, Add text, Add image, Redact and Fill and sign, then the Comment button with its menu (notes, text
  boxes, shapes, the pen, stamps) and the Tools menu, which holds every tool; at the right, Search and Print. The
  buttons are icons, each named by its tooltip. As the window narrows, the search field becomes an icon, and then
  the tools fold one by one into the Tools menu: the Comment button first, then Add image, Redact, Add text and
  Fill and sign, coming back in reverse as the window widens. In very narrow windows Export and then Undo and Redo
  leave the bar too (they stay in ☰, and Undo and Redo on their keys). Read and select text stays. The Tools menu
  shows the current tool when that one isn't on the bar.
- **Tabs:** one per document, with a × inside it to close it. A * after the name means unsaved changes.
- **Bottom:** the page you are on and messages at the left; at the right, the page layout, Fit width and the zoom.
- **Left:** a column of icons for the navigation panel's views: Pages (the thumbnails), Search results, Bookmarks
  (the document's outline) and Comments. Clicking a view opens it; clicking the open one folds the panel away, and
  clicking it again brings it back. Documents open on Pages, unless the file asks to open on its bookmarks. Click a bookmark to go to its page; the arrow beside one shows or hides the bookmarks inside it, which
  start shown or hidden as the file asks. Bookmarks that go to another file or a web address are greyed out. Add
  bookmark bookmarks the page you are on, after the bookmark last clicked (or last, when none is), and opens its name
  for typing: Enter keeps it, Escape keeps "Page N". Double-click a bookmark to rename it; right-click it to rename
  it, add a bookmark under it, or delete it with the bookmarks inside it. Undo reverses each of these. Moving
  bookmarks isn't possible yet. ☰ > View > Bookmarks, or "Bookmarks" in the command palette, opens this view.
  Comments lists the document's annotations; see [Comments and markup](#comments-and-markup). The document's
  digital signatures, and whether they are valid, are listed in the same panel from ☰ > Document > Digital
  signatures > Signatures in this document; see [Digital signatures](#digital-signatures).
- **Middle:** the pages, scrolling continuously. By default they fit the width between the side panels, and keep
  fitting as you resize the window or open and close the panels. Zooming in or out, actual size, Ctrl+scroll or a
  pinch turns fitting off, and your zoom then stays as it is; the Fit width button (highlighted while fitting is on)
  or Ctrl+0 turns it back on. Zoom goes from 10% to 6400%. At high zoom the part of the page in view is drawn in
  tiles as you scroll, over a softer view of the whole page until they arrive. The page layout button at the bottom right (its icon shows the current layout),
  or ☰ > View > Page layout, chooses how the document's pages are laid out: **Continuous pages** (one column, the default), **Facing pages** (two side by side, still scrolling;
  fitting fits both) or **Single page** (one at a time; a wheel step past the page's top or bottom turns the page).
  In facing pages, **Show cover page alone** (on by default) puts the first page on its own on the right, as in a
  book; off pairs pages 1 and 2. PageUp and PageDown move a pair at a time when facing. The choice changes the
  document you are viewing and is remembered for documents opened later. A document that sets its own opening
  view (see below) opens in its layout instead, and can also open the Pages or Bookmarks view and go to a page.
- **Right:** the inspector, which shows options for what is selected. The » at its top collapses it to a strip;
  the « at the top of the strip expands it again, and the strip's options button shows the inspector's contents
  in a pop-out over the pages (a click elsewhere or Esc closes it; what you changed in it stays). ☰ > View >
  Inspector does the same.
- **Narrow windows:** below about 960 points wide the inspector collapses to its strip by itself, and below about
  760 the navigation panel folds away to its icons. They come back when the window is wide enough again, unless
  you closed them yourself; a panel you open in a narrow window stays open. Whether you closed or opened each
  panel is remembered between runs.

Ctrl+K opens the command palette, which lists every command. Type to filter, use Up and Down to choose, and press
Enter to run.

## Links

Click a link with the read tool to follow it: the pointer turns into a hand over one, and hovering shows where it
goes. Dragging from a link selects text as usual. With the editing tools, hold Ctrl (Cmd on macOS) and click, so
links don't get in the way of editing.

- A link to another page goes there.
- A link to a web or email address asks first, showing the full address, and then opens it in your browser or
  email program. LimarePDF itself never connects to the network. Links to anything else, such as a file or a program,
  are shown but not opened.
- Links that go to another PDF file are not followed. Links can't be added or edited yet.

## Comments and markup

Annotations (comments, sticky notes, highlights, text boxes, shapes and drawings that other programs added) are
shown on the pages. ☰ > View > Comments, or "Comments" in the command palette, opens the Comments view, which lists
them page by page, with replies indented under the comment they answer. Each row shows the author and the first
line of the comment; hover over it for the type, date and the whole comment.

- Click a row to go to the annotation and select it, as clicking it on the page does (see
  [Marking text](#marking-text)).
- Right-click a row to delete it, along with any replies. Annotations the file locks can't be deleted. Undo brings
  it back.
- Right-click a row and choose Reply…, or select an annotation and use the Reply box at the bottom of the
  inspector, to answer it. The inspector lists the replies it already has. Replies are saved as standard reply
  comments (`/IRT`) that are not shown on the page or printed, and they carry your name.
- ☰ > Document > Flatten annotations… draws all of them into the pages as they look now and removes them, so they
  can no longer be changed or removed in any reader. Their comment text is dropped. Links and form fields stay,
  and so do file attachments. Undo brings them back.

Some files store annotations without a drawing, leaving each reader to draw them. LimarePDF draws highlights,
underlines, strike-outs, squiggly underlines, notes, text boxes, rectangles, ellipses, lines, polygons, freehand
drawings, stamps, carets and file attachment icons itself, on screen and when printing, including cloudy borders,
text box callout lines and line leader lines. The file isn't changed by this; flattening saves LimarePDF's drawing
into it (file attachments stay as they are). Text boxes are drawn in Helvetica, ignoring any rich-text styling; a
stamp shows its name in a coloured frame. [Rendering](rendering.md#known-to-be-wrong) lists what isn't reproduced.

### Marking text

Select text with the read tool, then click **Highlight** on the bar above it (or right-click > Highlight, or
Ctrl+Shift+H). The small arrow next to it, or right-click > Underline, strike out, colour, has:

- Underline, Strike out and Squiggly underline.
- The highlight colour (yellow, green, blue, pink, orange) and the line colour (red, blue, green, black). The
  colours chosen are remembered between runs.

A selection across pages gets one highlight on each page, all in one undo step. The marks are saved as standard
annotations, so other readers show them and can change or remove them.

Click a mark (or a comment) with the read tool to select it: it is outlined, and the inspector shows who made it
and when, with its colour, opacity and comment. Changes are kept as you make them, each one a step to undo; a
comment being typed is kept when you leave its box. Delete or Backspace removes it, and Esc lets it go. A drag that starts on a mark still selects the text under it. Hovering over a mark shows its
comment.

New marks are saved with your name, which is the computer's user name until you set it with ☰ > Edit > Your name
for comments…. Leave it empty to save no name.

### Notes and text boxes

The **Comment** button in the top bar holds the commenting tools: notes, text boxes, rectangles, ellipses, lines,
the pen and stamps (also in ☰ > Tools > Commenting). It shows the one you used last; the small arrow beside it
lists them all. The inspector sets the note colour, or the text box's text size and colour,
background (none at first) and border, before you start.

- **Note:** click on the page. A comment box opens beside the note; type, then click on the page outside it or
  press Esc or Ctrl+Enter to keep the comment (that click doesn't put down another note). With nothing typed, no
  note is added. While you type, the colour in the inspector changes the note's colour: clicks in the inspector and
  its colour picker leave the note open, Esc or a click that closes the picker closes only the picker, and you
  carry on typing. Ctrl+Z takes a note back. Hovering over a note shows its comment. Notes
  show at the same size on screen whatever the zoom, in the note's colour and shaped like its icon: one of the
  seven standard ones (Comment, which LimarePDF's notes use, Key, Note, Help, New paragraph, Paragraph and Insert),
  or Note for any other; printed, they keep their own drawing, or LimarePDF's, 20 points across.
- **Text box:** drag a box, or click for a default size, then type straight onto the page. Enter starts a new
  line. Click outside the box or press Esc or Ctrl+Enter to finish; with nothing typed, no box is added. While you
  type in a note or text box, Ctrl+Z and Ctrl+Y undo and redo the typing. Finishing either by choosing another
  tool keeps that tool. Text boxes use Helvetica, so characters
  beyond Western European (Greek, Cyrillic, Chinese and so on) come out as `?`. When you finish, the box grows downward to fit its text, keeping its width; it
  stops at the bottom of the page, and it never shrinks (resize it with the arrow).

When a note or text box is finished, the read tool comes back with it selected, so it can be moved straight away.
The Text box tool can be kept on (see [Keep tool active](#keep-tool-active)); the Note tool can't, so after each
note the read tool is back to select, move or open notes. Double-click a note or text
box (with the read tool or the arrow) to change its text. Ctrl+click selects an annotation while an annotate tool
is on. The inspector for a selected text box also changes its text, size, colours, background and border.

With the read tool or the arrow, click any annotation to select it, drag it to move it, and drag a handle to
resize it. Arrow keys
move it by 1 point, or 10 with Shift. Notes keep their size, and text markup stays on its text, so neither can be
resized, and markup can't be moved. Locked annotations can't be moved.

### Shapes, lines and drawings

The Comment button's menu also holds the rectangle, ellipse, line and pen tools. What they draw are annotations:
other readers list them as comments, and they can be selected, moved, resized, restyled, commented on and deleted
like notes.

- **Rectangle, ellipse, line:** drag, or click for a default size. Shift keeps squares, circles and lines at
  multiples of 45°. The inspector sets the outline (or line) colour and width, the fill of rectangles and
  ellipses (off until you turn it on), and an arrowhead at the end of a line, before you start.
- **Pen:** drag to draw; a click makes a dot. Strokes started within 1.5 seconds of the last one, on the same page,
  join the same drawing, so a word or a circle with a tick is one annotation. Each stroke is still its own undo
  step. Points closer together than about a third of a point to the line drawn are dropped, which keeps files
  small. The inspector sets the pen's colour and width.
- **Draw on page (permanent):** with this switched on in the inspector, the rectangle, ellipse and line tools draw
  into the page content as they did before annotations. Those shapes are selected with the arrow as page objects,
  not as comments.

Select a shape, line or drawing (click it with the read tool or the arrow, or Ctrl+click with an annotate tool)
to change its colour, width, fill, arrowhead, opacity and comment in the inspector. Lines and drawings are picked
by clicking near their strokes, not anywhere in their box.

A selected line has a round handle on each end instead of a box: drag either end to turn the line
or point an arrow somewhere else (Shift keeps it at a multiple of 45° from the other end). Drag the line itself to
move it.

A selected freehand drawing has a round rotate handle above its box; the pointer becomes a circular arrow over it.
Drag it around to turn the drawing about the
middle of its box; the angle shows while you drag, and Shift keeps to steps of 15°. Rectangles, ellipses and text
boxes can't be turned: PDF stores them as upright boxes.

### Stamps

The stamp tool is in the Comment button's menu ("Add a stamp" in the command palette). Its inspector lists the
ready-made stamps (Approved, Not approved, Draft, Final, Confidential, For comment and Void), each a coloured frame
with the word in capitals, your own wordings, and the pictures used as stamps before. Pick one, then click on the page to put it down
at its own size, centred on the click, or drag a box to size it (it keeps its shape inside the box). The read tool
then comes back with the stamp selected, to move or resize (see [Keep tool active](#keep-tool-active)).

- **Add my name and the date:** on by default; adds a small line under the word with your name and today's
  date. The Name box under it is the same name as Edit > Your name for comments, so changing either changes both.
  The date is in UTC, as headers and footers use. The name and date are fixed when the stamp is put down. The
  choice is remembered.
- **Your own wording:** pick green, red or blue and type it in the box under the ready-made stamps, then click
  on the page; what you type is the stamp to put down. It is shown in capitals and goes into the list above when
  you leave the box (Enter or a click elsewhere). Picking a colour also recolours the picked wording of your own.
  The last five wordings are remembered with their colours (right-click one to take it off the list). The stamp's font covers
  Western European languages only, so wording with other characters (Greek, Cyrillic, Chinese and so on) can't
  be used yet; the pane says which characters.
- **Pictures:** Choose a picture… takes a PNG or JPEG file; it is put down with its longer side 160 points. The
  last five pictures are remembered by path (right-click one to take it off the list). Transparent parts of a PNG
  stay transparent.

Stamps are annotations: other readers list them as comments. Select one to change its opacity and comment;
its colours are part of the stamp. A stamp reads upright on a turned page. The text is Helvetica Bold, Western
European characters only, so a name in other scripts shows as `?`.

## Redaction

Redacting is done in two steps: mark what to remove, then apply the marks. Marks are outlined in red and remove
nothing by themselves, so they can be checked first, here or by someone else in another app (they are standard PDF
redaction marks). See [Redaction](redaction.md) for exactly what applying removes.

- **Mark areas:** the Redact button in the top bar (☰ > Tools > Mark areas for redaction), then drag a box over anything: text, pictures, drawings. The
  read tool then comes back; pin the Redact tool ([Keep tool active](#keep-tool-active)) to mark several areas in a
  row.
- **Mark text:** select it with the read tool, then right-click > Mark for redaction (or Edit > Mark for redaction).
- **Mark search results:** search, then in the navigation panel's Search results press Mark all for redaction (or Edit > Mark
  all search results for redaction). Every match is marked.

Marks are listed in the Comments view as "Redaction mark". Select one (click it with the read tool, Ctrl+click it
with another tool, or click it in the Comments view) to delete it or add a comment; like highlights, marks can't be moved or resized, so to change one, delete it
and mark again.

**Apply:** Document > Apply redactions… asks first, then removes everything under the marks, paints the areas black
and removes the marks. It then checks that nothing readable is left under them; if anything is, nothing is changed
and an error says what was found. A report lists what was removed from each page and where the removed text can
still be found elsewhere in the document (other pages, bookmarks, comments, form fields, document properties).
Undo brings it all back until the document is closed; once saved, the removed content is gone from the file.

While a document has marks, a notice above the pages says how many and has an Apply redactions… button; it comes
first, and goes once the marks are applied or deleted. Dismissed, it comes back when another area is marked. The
Redact tool's pane has the same button with the count. Saving a document that still has marks asks first, since
nothing under them has been removed.

**Remove hidden information:** Document > Remove hidden information… lists what the document carries besides its
pages (metadata, scripts, attachments, hidden layers, private data, and comments, form values, bookmarks and content
outside the crop box), with how many of each it has. Tick what to remove and press Remove; a summary follows. Undo
brings it back. See [Removing hidden information](redaction.md#removing-hidden-information).

## Filling in forms

Form fields work with the read tool, like links: hovering tints a field and shows its description, and a click
fills it in. With the editing tools, hold Ctrl (Cmd on macOS) and click.

- **Text fields:** a click opens a text box over the field. Enter (in a one-line field), Esc or clicking elsewhere
  keeps what you typed; Ctrl+Z takes it back. Password fields show dots, and their value is not kept in the file.
- **Checkboxes and radio buttons:** a click ticks or chooses.
- **Drop-downs and lists:** a click opens the options. In a list that allows several choices, each click adds or
  removes one at once; a click outside the list, Esc or Enter closes it. A drop-down you can type in has a text box
  at the top; Enter keeps the text.
- **Tab and Shift+Tab** move to the next and previous field, top to bottom and page by page. On a checkbox or radio
  button, Space ticks it; on a list, Space or Enter opens it.

Each value you change is one undo step. Fields are highlighted from the start: a light tint on every field, stronger
under the pointer, and a red outline on required ones. ☰ > View > Highlight fields, or the switch on the form's
banner, turns this off or on again for every document, and the choice is remembered between runs. The inspector's Form section shows the field you are in (its name, description and whether it is required),
with Clear field, Highlight fields and Reset form, which puts every field back to its default. Flatten form… (also
in the command palette) makes the fields part of the pages as they look now, so nobody can change the values,
here or in other readers; it asks first, and Undo brings the fields back until you close the document. Read-only fields
and buttons (they run scripts, which LimarePDF doesn't) can't be filled in; hovering says why. Clicking an empty signature
field puts your signature in it (see [Filling and signing](#filling-and-signing)); clicking a digitally signed one
shows its signature's details (see [Digital signatures](#digital-signatures)).

LimarePDF draws the filled fields itself, so the values show the same in other readers. Text must be Western European for
now; anything else is refused with a message rather than saved wrongly. Fields that have a value but
no drawing of their own are drawn when the file opens. A value filled in by another app in other characters (Japanese,
for example) shows only if that app drew it: if it didn't, LimarePDF can't either, so Flatten form… stops and names
the fields rather than lose their values, and Print asks whether to print without them, since they would come out
empty on paper. See [Command line](cli.md#forms) for filling forms in scripts.

## Filling and signing

The Fill and sign button in the top bar (its icon is a signature) puts your signature, initials, the date, your
name, other text or a tick, cross or dot on the page, for signing a document someone sent you. The small arrow
beside it opens a menu of all of these: Signature and Initials, each listing the saved ones and Create…, then Date,
Your name, Text, Tick, Cross and Dot. Picking one turns the tool on with it; the button itself turns the tool on
with the item picked last. It is a picture of your signature, not a digital signature: it doesn't prove who signed, and
anyone can copy it. For digital signatures, see [Digital signatures](#digital-signatures).

**Making a signature:** Signature > Create signature… in the button's menu, or in the Fill and sign pane Create
signature… (or New… beside Signatures), opens a window with three ways to make one:

- **Draw:** sign on the white pad with the mouse, touchpad or a pen; Clear starts again. The strokes are smoothed
  into curves and stay sharp at any zoom. In a narrow or short window the pad is smaller, at the same shape; what
  is drawn keeps its shape and comes out the same size on the page, also when the window is resized while drawing.
- **Type:** type your name (it starts as your name for comments) and pick one of four handwriting styles (Caveat,
  Cedarville Cursive, Homemade Apple and Zeyada), shown as they will look. Western European characters only; for other scripts, draw or use an image.
- **Image:** choose a photo or scan of your signature on paper (PNG, JPEG or TIFF). "Make the paper transparent"
  (on by default) keeps only the ink and cuts off the empty margins, so the signature sits on the page like ink.

The window also has the ink colour (the same choice as the pane's), and Use signature puts it in the list. Esc closes
the window only while nothing is drawn, typed (beyond the name it starts with) or chosen; picking another tool
closes it.

Initials are made the same way, with New… beside Initials; typing starts with your name's initials. Each new one
goes first in its list in the pane, up to five signatures and five sets of initials; right-click one to take it off
the list. "Save for next time" (on by default) keeps it there when LimarePDF starts again. Unticked, it is still in the
list to use until LimarePDF closes, marked "until LimarePDF closes", and is then gone. Saved signatures are stored unencrypted in LimarePDF's settings folder (`signatures.txt`, and pictures copied into its
`signatures` folder), on this computer only; taking a picture off the list deletes its copy.

**Putting things down:** pick an item in the pane, then click on the page to put it down at its usual size, centred
on the click, or drag a box to size it (it keeps its shape inside the box). The read tool then comes back with the
item selected, to move or resize it; pin the tool ([Keep tool active](#keep-tool-active)) to sign several places in a
row.

- **Signatures and initials:** a signature is up to 150 by 50 points, initials up to 50 by 30. With no signature
  saved yet, clicking the page opens the window first and puts the new signature where you clicked.
- **Date:** today's date, in the style picked beside it: 2026-10-16, 16 October 2026 or October 16, 2026. The date
  is in UTC, as stamps use, so late in the evening west of Greenwich it can already be tomorrow's; the date is a
  text box, so double-click it to change it.
- **Name:** your name for comments (the same as Edit > Your name for comments).
- **Text:** type in the box, then click; for anything else a form asks for.
- **Marks:** a tick, cross or dot, for boxes that aren't real form fields.
- **Ink:** black or blue, for what is put down next.

The style choices are remembered. Signature fields: clicking an empty one, with the Sign tool or the read tool, puts
the picked signature (or initials) inside its box. The field itself stays digitally unsigned, so other readers don't
report a broken signature.

**Moving and finishing:** what you put down is added as annotations, so it can still be moved, resized or deleted:
click an item with the read tool (or Ctrl+click it with another tool) to select it. While there are such items, a notice above the pages says how
many and has a Finish signing… button; the Sign pane has the same button. Finish signing (also in the Document menu)
draws them into the pages as they look, after asking, so they can no longer be moved or removed as comments; Undo
brings them back until you close the document. Saving without finishing keeps them movable, and the notice comes
back when the document is opened again. Other comments and stamps are left alone. Signatures are listed in the
Comments view as "Signature", "Initials", "Tick" and so on.

## Digital signatures

When a document with digital signatures opens, LimarePDF checks them in the background and shows the result in a
banner above the pages, coloured by the worst signature: green (valid, and you trust every signer), amber (nothing
broken or changed against the signers' wishes, but LimarePDF can't confirm who signed, or there are warnings), grey (LimarePDF
can't check a signature) or red (broken, changed in ways a signer didn't allow, or a trusted signer's certificate
fails a check). Its Signatures button, or ☰ > Document > Digital signatures > Signatures in this document, opens the Digital
signatures view of the navigation panel, which
lists each signature with its result, the changes made after it, and Details… and Signed version buttons, then the
signature fields nobody has signed yet. Click a signature's name to go to it; with the read tool, click a signed
field on the page to see its details.

Details shows who signed, who issued their certificate, when, the changes after signing, warnings and the
certificates; Trust this signer… and Trust an authority… add to your trust list (after showing the SHA-256
fingerprint to compare with the owner), and View signed version opens the document as it was when signed, in a new
tab. ☰ > Document > Digital signatures > Trusted certificates… lists, removes and adds trusted certificates. Trust settings affect LimarePDF only.

The check is of the file as opened or last saved; saving adds your changes after what was signed (see
[Saving a signed document](signatures.md#saving-a-signed-document)), and LimarePDF checks the saved copy, listing your
changes under each signature. See [Digital signatures](signatures.md) for what each result means.

### Signing with a digital ID

☰ > Document > Digital signatures > Sign with digital ID…, or Certify… for the author's
certification: click an empty signature field or drag a box on the page, then pick your digital ID, type its
password, and optionally a reason and a location. Sign… asks where to save the signed copy (`NAME-signed.pdf` by
default), signs in the background, and opens the signed copy in place of the document. Changes made since opening
are signed with it; when they would show on earlier signatures as not allowed, LimarePDF asks first. If the
document changes while it is being signed (an edit, a password set, text being typed), it stays open as it is and
the signed copy opens in a new tab. A document set to get a password when saved (Document > Password security)
isn't signed: LimarePDF can't sign password-protected documents, so the signed copy would be saved without it.

☰ > Document > Digital signatures > Digital IDs… makes, imports, exports and removes your digital IDs, and chooses the default. See
[Signing](signatures.md#signing) and [Digital IDs](signatures.md#digital-ids).

## Notices about the document

When a document opens, notices above the pages point out things worth knowing, each with a button where there is
something to do and × to dismiss it. At most two show at a time.

| Notice | Button |
| --- | --- |
| Its digital signatures are being checked, and then how they checked, coloured green, amber, grey or red (always first; see [Digital signatures](#digital-signatures)) | Signatures |
| It was restored from a recovery copy, as it was at the time shown, and isn't saved; a signed document's signatures aren't checked ([Recovery copies](#recovery-copies)) | Save as… |
| An earlier recovery copy, made before content was removed, couldn't be deleted, and where it is | Show file |
| Recovery copies of it can't be kept right now, and why | Save |
| The file is damaged, and LimarePDF repaired it to open it: some content may be missing. The original stays as it is: the document counts as changed (closing it warns of unsaved changes), and Save asks whether to replace the original or save the repaired copy as a new file. A signed file's signatures can't vouch for what is shown (they show *can't check*, or a change that isn't allowed), and the repaired copy can't keep them ([details](rendering.md#damaged-files)) | Save as… |
| The file isn't put together quite as the PDF format asks, so other PDF apps may show it differently: some content could be missing or different in them. Saving a copy writes it the way LimarePDF shows it, which every app reads the same, and the notice goes. Not shown for repaired files (the notice above covers them) or signed ones (their signatures show *can't check* and say why) ([details](rendering.md#files-other-apps-may-show-differently)) | Save as… |
| The document was password-protected: saving removes the protection unless you set it again. A document that opens without a password but has a permissions password (which can limit printing, copying or changes; other readers open it without asking too) says so instead: saving removes that password unless you set it again | Password security |
| Areas are marked for redaction but the marks haven't been applied (kept up to date while you work) | Apply redactions… |
| Signatures and other items put down with the Sign tool can still be moved (kept up to date while you work) | Finish signing… |
| Its pages are scans without text, so they cannot be searched or copied (checked on the first 20 pages) | Recognise text |
| A page has more annotations than LimarePDF shows (10,000), so some can't be seen or chosen; Remove comments, removing attachments and redaction still deal with all of them, and redaction marks among them are applied | |
| It is a form made for Adobe Reader (an XFA form), which only Adobe software can show or fill in; LimarePDF shows the placeholder page the file contains, often "Please wait..." | |
| Parts of it couldn't be drawn: a picture that can't be decoded. Found as pages are drawn, so it can appear later; it comes back if something new turns up after you dismiss it | |
| Fonts it names aren't in the file or installed: which ones are shown with a similar font, or which language's text can't be shown at all, and what to install to see it as it was made (such as Noto Sans CJK JP for Japanese). Found as pages are drawn, like the one above ([details](rendering.md#fonts-that-arent-embedded)) | |
| It has a form: click a field to fill it in, or press Tab (not for a form of signature fields alone) | Highlight fields |
| It is large (more than 500 pages or 100 MB) | |

The first two go away by themselves once dealt with: after password security is applied, or after text has been
recognised.

## Recovery copies

While you work, LimarePDF keeps a private copy of each document with unsaved changes, so that if it stops without
closing normally (a crash, the system ending it, a power cut), the next start can offer the documents back. These
copies are not autosave: **your own files are never written by them.**

- A copy is made about a minute after you change a document, then at most once a minute while you go on changing it.
  It holds the document as it was then, so changes made in the last minute or so before a crash can be lost. Text
  still being typed (a comment, a form field, a reply) isn't in a copy until it is kept, by clicking away or
  applying it.
- Saving or closing a document deletes its copy, and quitting normally deletes them all.
- Password-protected documents get no copies: not one opened with a password, not one you set a password on (its
  copy is deleted first), not pages taken from one, and not a document you add a protected document's pages to
  with Append PDFs (its copy is deleted first too; this includes protected files that open without a password).
- The copies are kept in a folder only you can read: `~/.local/state/limarepdf/recovery` on Linux (or
  `$XDG_STATE_HOME/limarepdf/recovery`), `~/Library/Application Support/limarepdf/recovery` on macOS, and
  `%LOCALAPPDATA%\limarepdf\recovery` on Windows. At the start, LimarePDF looks only in real folders of yours there:
  a recovery folder, or a folder inside it, that is a symbolic link or belongs to another user is left untouched.
- To stop them, turn off ☰ > File > **Keep recovery copies of unsaved changes** (also in the command palette).
  Copies already made are deleted at once, and the choice is remembered.
- If a copy can't be written, for example when the disk would be left with less than 1 GB free, a notice on the
  document says so, with a Save button. If recovery can't start at all, a message says so when the app starts.

**After a crash.** The next start lists the documents that had copies: name, where the document came from, when the
copy was made, how many pages, and until when the copy is kept. **Restore** opens one as a new, unsaved document
called "*name* (recovered)", with a notice saying when the copy was made; Save suggests "*name* (recovered).pdf"
beside the original. **Discard** deletes the copy. **Not now** keeps them all and asks again at the next start.
Copies are deleted 30 days after the crash if you do neither. A copy that turns out to be damaged is listed as such,
with Show file and Discard; so is one whose description LimarePDF wrote beside it is damaged, a time in it past the
year 9999 included. Damaged copies never stop LimarePDF from starting. A restored document's copy stays until you save or close it, so a second crash doesn't
lose it, or until you remove content from it or set a password (below). Files given on the command line open once
the list is closed.

A restored copy of a digitally signed document is LimarePDF's working copy, not the signed file, so its signatures
cover bytes that aren't in it. They aren't checked: the Digital signatures view lists each one as "Not checked: recovered
copy". Open the original to check them. Saving or signing the copy writes the whole file, which makes them invalid,
so LimarePDF asks first.

**Redactions, hidden information and passwords.** Once you apply redactions, remove hidden information or set a
password, no recovery copy made before can be offered after a crash: LimarePDF deletes the document's copy first,
and for a restored document the copy it came from, and makes a new one afterwards (none for a password), which
holds none of what was removed. Undoing a redaction or removal brings the content back, and the next copy holds it;
Redo then deletes that copy first too. If it can't delete a copy, the command (or Redo) doesn't run, and a message
says where the copy is, with Show file: delete it, or turn off recovery copies (which doesn't remove the copy a
restored document came from), then try again. If the copy was deleted but the disk can't confirm it (a power cut
could bring it back), the command doesn't run either, at every try, until the disk confirms it. If the copy
could be made unusable but its file couldn't be deleted, the command runs, and a notice says where the file is until
LimarePDF manages to delete it, or you do. The limits of this promise:

- A deleted file's bytes stay on the disk until they are overwritten, and on SSDs nothing reliably wipes them.
- On Windows a deletion counts as done once the file's name is gone, which NTFS normally keeps across a power cut but
  doesn't guarantee for the last moments before one.
- It covers recovery copies only. They never change your original file, so it holds whatever it held until you save
  over it.

## Selecting and the inspector

What you select decides what the inspector, the right-click menu and the floating toolbar offer.

| Selected | Inspector |
| --- | --- |
| Nothing | The document: properties, opening view, password security, crop, watermark, headers/footers/Bates, OCR, optimise scanned pages, export pages as images, split into files, and buttons for export text, print and append PDFs |
| Pages (click thumbnails; Shift for a range, Ctrl for one more) | Rotate, delete, extract to a new tab, insert images or a blank page after them, page labels from the first selected page, and the crop, watermark, headers/footers, OCR and optimise tools with the pages filled in |
| An object, words, or a tool that adds content | That object's or tool's options |

Menu commands for document and page settings open the matching section of the inspector.

Pages boxes, in the inspector's tools and the print dialog, take page numbers and ranges counted from 1, such as
`1-3,7`, `5-` (5 to the end) or `all`. Clicking or tabbing into a box that says `all` selects it, so typing page
numbers replaces it.

A watermark is text or a picture (PNG, JPEG or TIFF), centred on each page. After you apply a watermark or headers
and footers, the section's button becomes Update: change a setting and press it to redo what you just added rather
than stamping a second copy. Add another keeps it and adds a new one. Any other change returns the button to Apply.

If a damaged file makes LimarePDF fail while showing a document, that tab shows an error page with Save a copy and Close
this tab; other tabs carry on.

Page labels are the page numbers viewers show, such as i, ii, 1, 2 or A-1; they don't change what is printed on the
pages. When a document has them, the status bar shows the label with the page's place, such as "Page i (3 of 477)",
and the thumbnails show the labels. To set them, select the first page to number, open Page labels in the
inspector (or go to the page and choose ☰ > Pages > Page labels…), and pick a style, an optional prefix and the number to start at; the numbering runs to the next labelled
range. Labels stay with their pages when pages are deleted or moved.

Opening view, in the document's inspector, is how viewers that follow it show the document when it opens: its
layout (continuous, facing, facing with the cover alone, or single page), the side panel (Pages or Bookmarks) and
the page it opens at. Each can be left as Not set. Use current view fills them in from how you are viewing the
document now; Apply saves them into the document like any other change, and Undo reverses it.

Split into files makes files of a set number of pages, or starts a file at each bookmark down to the chosen level
(1 is the top level only). Files are named after their bookmark, such as `book-002 Introduction.pdf`, and the pages
before the first bookmark get a file of their own. Each file is written as soon as its part is made, so a large
document split into many files needs little more memory than the document itself.

Pages that are deleted, or left out when extracting or splitting, leave nothing in the saved file: bookmarks,
named destinations, links, the opening view, form fields and tags stop referring to them, so their content isn't
kept. A link to such a page goes nowhere, and a form field whose boxes were all on those pages leaves the form.
Tags with content on those pages lose their alternative text (also when they have content on kept pages too), and
so do the tags above them. A form nested more than 32 deep can't be followed to its end, so deleting, extracting
or splitting stops with a message saying why. An
extracted file or a split part keeps the bookmarks to its own pages; a bookmark to another page stays, without a
destination, only when bookmarks below it stay. Deleting pages keeps every bookmark, without a destination when its
page went.

Crop trims margins from each side of the page as it is shown, measured from the full page; the fields start with
the first page's current crop. Remove crop shows the whole page again. Cropping hides content but does not delete
it.

Right-click a page, an object, selected words or a thumbnail for a short menu of what applies there. Right-clicking
selects what is under the pointer first. A selected object or run of words also gets a small floating toolbar with
the same commands. Its last button, Dock controls on the right, moves these buttons to the top of the inspector (or
its strip, when collapsed) instead; Float controls near selection there brings the floating toolbar back. ☰ > View >
Selection controls on the right does either. The choice is remembered between runs; resizing the window doesn't
change it.

Drag thumbnails to put pages in a new order. A line shows where they will go. Dragging a selected thumbnail moves all
the selected pages.

Delete removes the selected object or pages. Deleting shows a notice with an Undo button.

Remove blank pages (☰ > Pages, or the command palette) deletes every page that is one even colour, give or take
scanner specks, faint show-through or a lone page number, and lists the pages it removed. Undo brings them back. A
document whose pages are all blank is left alone.

## Editing page content

Documents open with the read tool (the text cursor), which selects text, follows links and fills in forms. It also
edits:

- **Page text:** double-click a line to edit it there. The word you double-clicked is selected, so typing replaces
  it. The line is drawn as it will be written: in its own font when LimarePDF can keep it, otherwise in the
  Helvetica, Times or Courier that replaces it (which switches as you type a character the line's font can't
  take), at its size and in its colour, with a thin outline around it and the old line hidden underneath in the
  colour around it. Enter, Esc, Tab or a click elsewhere keeps the change. While you type, Ctrl+Z and Ctrl+Y undo
  and redo typing in the line, and Ctrl+B and Ctrl+I switch the whole line to Bold or Italic; once the change is
  kept, Undo takes the whole edit back. Editing the same line again within a few seconds, with nothing else
  changed in between, joins the same undo step. The inspector shows the line's font, size and colour; each change
  there is kept at once as its own undo step. A line turned on the page (not running left to right on screen)
  and invisible recognised (OCR) text are edited in a plain box over the line instead.
- **Pictures, shapes, comments and marks:** click one to select it, drag it to move it and drag a handle to resize
  it (Shift keeps the proportions). Arrow keys move it by 1 point, or 10 with Shift. A picture or shape covering
  most of the page, such as a scanned page or a background, isn't selected by a click, so that clicking a scan
  doesn't pick it up; the arrow selects it. A drag that starts anywhere else still selects text.
- **Moving page text:** select the line (double-click it), then choose Move or resize text on its floating
  toolbar: the line gets handles, and you drag it or them, or use the arrow keys, with the read tool still on,
  until you click away or select something else. Text you have just added can be moved straight away in the same
  way. The arrow (☰ > Tools > Select, move and resize objects) moves any line too.

The arrow (☰ > Tools > Select, move and resize objects) selects any object, page text included:

- Click an object to select it.
- Drag to move it; drag a handle to resize it (Shift keeps the proportions).
- Arrow keys move it by 1 point, or 10 with Shift.

The inspector shows the selected object's position and size, and for text the string, font, size and colour; for
shapes the fill, outline and width; for images "Replace image". Changes there are kept as you make them, without
an Apply button: typed text when you press Enter or leave the box, a number when you stop dragging it or leave its
box, a colour when its picker closes. Each is one undo step. The font size box shows the exact size, which isn't
rounded to a listed one (10.126 shows as 10.126); type any size, or pick one from the list beside it. Going into
the box and out again without typing changes nothing, and text that isn't a size leaves the size as it was.

The other tools add content. Each puts one thing down and then gives way to the read tool, with the new item
selected so you can adjust it (see [Keep tool active](#keep-tool-active)):

- **Text:** click where the text starts and type. The text appears on the page as you type, in the font, size
  and colour it will have, with a thin outline around it; the line sits on the point you clicked. Enter starts a
  new line. Esc, a click elsewhere on the page, or choosing another tool finishes, and every line is added as one
  undo step; nothing typed adds nothing. While you type, the inspector's font, Bold, Italic, size and colour
  change the text (the keyboard comes back to it afterwards), Ctrl+B and Ctrl+I switch Bold and Italic, and
  dragging the outline moves the text. A click on page text with this tool edits that line instead (see Page
  text above), with the caret where you clicked, and a drag selects text, as with the read tool. The font is Helvetica, Times or Courier, with Bold and Italic, and only Western
  European characters can be added: others can't be typed or pasted, and a note at the caret says so (see
  [What doesn't work yet](features.md#what-doesnt-work-yet)). Each line becomes its own line of page text, so
  it moves and is edited line by line afterwards.
- **Image:** choose a PNG, JPEG or TIFF, then drag a box.
- **Comment:** sticky notes, text boxes, rectangles, ellipses, lines and the pen, saved as annotations rather
  than page content; see [Notes and text boxes](#notes-and-text-boxes) and
  [Shapes, lines and drawings](#shapes-lines-and-drawings). A rectangle, ellipse or line can still be drawn into
  the page content with "Draw on page (permanent)". Its menu also has the stamp tool (see [Stamps](#stamps)).
- **Redact:** drag a box to mark an area for redaction (see [Redaction](#redaction)).
- **Fill and sign:** your signature, initials, the date and other items (see
  [Filling and signing](#filling-and-signing)).

### Keep tool active

The tools that put something down (Add text, Add image, the note, text box, rectangle, ellipse and line, stamps,
Redact and Fill and sign) go back to the read tool once they have put one thing down. The pin at the top right of a
tool's options, Keep tool active, keeps that tool on instead, so you can put down several in a row; Esc or another
tool leaves it. Each tool's pin is its own (pinning Redact doesn't pin Fill and sign) and is remembered between
runs. The Note tool has no pin: kept on, every click would put down a note, so existing notes couldn't be opened
or deleted. The pen always stays on until Esc or another tool, since a drawing takes several strokes. LimarePDF
always starts with the read tool.

Everything that changes the document can be undone with Ctrl+Z, up to 20 steps back. Each step keeps a whole copy
of the document, and a document's steps are limited to 1 GB together, so undo goes back fewer steps on very large
documents (about 7 on a 139 MB scan); the latest change can always be undone. See
[Content editing](content-editing.md) for how editing works and its limits.

## Copying text

With the read tool, drag to select words in reading order. A drag can start anywhere on the page, such as in the
margin: it starts at the nearest word. Dragging past the top or bottom of the view scrolls it, faster the further
out the pointer is, so a selection can run across pages. Double-clicking page text edits its line (see
[Editing page content](#editing-page-content)); where there is no line to edit, such as recognised (OCR) text,
double-click selects one word and triple-click a line. Shift+click extends the selection to where you click, on
any page. Ctrl+A selects the whole page in view. Esc or
a click clears the selection. Copy with Ctrl+C, ☰ > Edit > Copy text or the floating toolbar: words on a line are
separated by spaces, and lines and pages by line breaks. Recognised (OCR) text can be selected like any other
text.

To print or save just the selection, right-click it and choose Print selected text… or Save selected text… (also
under ☰ > Edit). Both offer two layouts:

- **As it looks (cropped pages):** each page with selected words at its full width, margins included, cut down
  from the top and bottom to the selected lines. Fonts, pictures and layout stay as they are, but anything else
  beside those lines shows too, such as the start of the first line, another column or a picture.
- **Plain text (PDF):** only the selected words, set in 11-point Helvetica on pages the size of the first selected
  page, keeping the line breaks. Characters outside Western European alphabets come out as `?`.

Saving also offers **Text file (.txt)**, the same text that Ctrl+C copies.

Scrolling is left to the mouse wheel, the touchpad, the scroll bars and the keyboard; dragging the page itself
doesn't scroll it (except on a touch screen).

## Keyboard shortcuts

Shortcuts show ⌘ instead of Ctrl on macOS.

| Keys | Action |
| --- | --- |
| Ctrl+O | Open |
| Ctrl+S, Ctrl+Shift+S | Save, Save As |
| Ctrl+W | Close the tab |
| Ctrl+P | Print |
| Ctrl+Z, Ctrl+Y (or Ctrl+Shift+Z) | Undo, redo |
| Ctrl+F | Search |
| Ctrl+K | Command palette |
| Ctrl+= , Ctrl+- | Zoom in, zoom out |
| Ctrl+0, Ctrl+1 | Fit width on or off, actual size |
| Ctrl+scroll, pinch | Zoom |
| PageUp, PageDown, Home, End | Previous, next, first, last page |
| Delete, Backspace | Delete the selected object, comment or mark, or the selected pages |
| Ctrl+Shift+H | Highlight the selected text |
| Arrow keys (Shift for 10 points) | Move the selected object |
| Ctrl+A, Ctrl+C | Select all text on the page, copy it (read tool) |
| Esc | Cancel: close an open list, menu or colour picker (only that), finish (and keep) new text, a line, a note, a text box or a form field being typed, clear the selection, or leave a tool for the read tool |
| Ctrl+B, Ctrl+I | Bold, Italic for new text being typed (Add text), or for the whole line being edited on the page |
| Tab, Shift+Tab | Next, previous form field; Space ticks a checkbox or opens a list |
| Tab, Shift+Tab (a document without fillable fields) | Next, previous button or field: the top bar, tabs, bottom bar and panels left to right; inside a dialog, its fields and then its buttons. Space or Enter presses the button. In a form, Tab moves between its fields; Ctrl+K reaches every command |

## Look and feel

The app follows the system's light or dark theme. ☰ > View > Theme chooses System, Light or Dark. The choice is
remembered between runs, in `settings.txt` in the same folder as the recent-files list, along with the markup
colours and your name for comments. The app uses the Inter font
and Phosphor icons. It also includes the Liberation fonts, which have the widths of Helvetica, Times and Courier,
to show text in its own look while it is typed on the page; they are only drawn on screen, never written into
PDFs, and don't add to the fonts you can choose.

The window opens at the size it had when the app last closed, and maximized if it was; a maximized window keeps
the size it had before, for when you restore it. A size kept on a larger monitor is shrunk to fit a smaller one.
Its position isn't remembered: on Linux Wayland sessions the desktop places windows, not the app, and tiling window
managers choose the size themselves. On KDE Wayland, restoring a window that opened maximized puts it at the
top-left of the screen, since the desktop never placed it at its normal size; a later start places it as usual.

☰ > Help > About shows the version (the Copy button next to it copies the version and system, for a bug report),
links to the website, the licence and the issue tracker, which open in your browser, and Credits for the bundled
fonts, icons and text recognition model. Contributors switches the window to the people who made LimarePDF; Back returns.
LimarePDF is made in Canada, which the About window and the foot of the start screen say beside a maple leaf.

☰ > View > Page colours changes how pages look on screen, for reading in the dark or with low vision:

| Choice | Pages look like |
|---|---|
| Normal | Their own colours (the default) |
| Automatic | Dark while the app's theme is dark, Normal while it is light |
| Dark | Dark grey paper and light text; colours keep their hue, so red stays red |
| High contrast | Black paper; text, lines and anything coloured turn white |
| Sepia | Warm paper and dark brown text |

Photos and other pictures keep their own colours. Turn on **Recolour pictures too** in the same menu to change them
as well; a scanned page is one big picture, so this is what recolours scans. The choice applies to every open
document and the thumbnails, and is remembered between runs. It is display only: the file, printing, exported
images and everything else keep the page's own colours. The commands are also in the command palette.

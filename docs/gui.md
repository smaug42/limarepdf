# Using the desktop app

![The MPE window with a three-page report open](images/mpe-gui.png)

## Opening files

Open PDFs with the Open button, Ctrl+O, or by dropping them on the window. Image files (PNG, JPEG, TIFF) open as a new
document with one page per image. From a terminal, `mpe-gui file.pdf --page 3` opens a file at page 3.

Recently opened PDFs are listed on the start screen and under ☰ > File > Open recent, which also clears the list.
Files that no longer exist are left out. The list holds 10 files and is kept on your computer only, in
`~/.config/mpe/recent.txt` on Linux, `~/Library/Application Support/mpe/recent.txt` on macOS, and
`%APPDATA%\mpe\recent.txt` on Windows.

The start screen also has Make a PDF from images…, which turns photos or scanned pages into a new document.

Each document gets a tab. The tab shows its name, with `*` while it has unsaved changes.

Saving writes a temporary file beside the document and then replaces the document with it, so a save that fails
part way leaves the original as it was. MPE saves by rewriting the whole file, which makes any digital signatures
invalid; when a document is signed, MPE asks first. Save As to a new file keeps the signed original.

## The window

- **Top bar:** the menu (☰), Open, Save, Undo and Redo, the tools (Sign among them), the page layout, zoom, search, Print, an Export menu (pages as
  images, text), and the theme. On narrow windows the search field and the tools fold into single buttons.
- **Left:** five tabs. Pages shows the thumbnails, Search the search results, and Bookmarks the document's
  outline. Click a bookmark to go to its page; the arrow beside one shows or hides the bookmarks inside it, which
  start shown or hidden as the file asks. Bookmarks that go to another file or a web address are greyed out. Add
  bookmark bookmarks the page you are on, after the bookmark last clicked (or last, when none is), and opens its name
  for typing: Enter keeps it, Escape keeps "Page N". Double-click a bookmark to rename it; right-click it to rename
  it, add a bookmark under it, or delete it with the bookmarks inside it. Undo reverses each of these. Moving
  bookmarks isn't possible yet. ☰ > View > Bookmarks, or "Bookmarks" in the command palette, opens this tab. Comments
  lists the document's annotations; see [Comments and markup](#comments-and-markup). Signatures lists its digital
  signatures and whether they are valid; see [Digital signatures](#digital-signatures).
- **Middle:** the pages, scrolling continuously. By default they fit the width between the side panels, and keep
  fitting as you resize the window or open and close the panels. Zooming in or out, actual size, Ctrl+scroll or a
  pinch turns fitting off, and your zoom then stays as it is; the Fit width button (highlighted while fitting is on)
  or Ctrl+0 turns it back on. The page layout button left of the zoom buttons (its icon shows the current layout),
  or ☰ > View > Page layout, chooses how the document's pages are laid out: **Continuous pages** (one column, the default), **Facing pages** (two side by side, still scrolling;
  fitting fits both) or **Single page** (one at a time; a wheel step past the page's top or bottom turns the page).
  In facing pages, **Show cover page alone** (on by default) puts the first page on its own on the right, as in a
  book; off pairs pages 1 and 2. PageUp and PageDown move a pair at a time when facing. The choice changes the
  document you are viewing and is remembered for documents opened later. A document that sets its own opening
  view (see below) opens in its layout instead, and can also open the Pages or Bookmarks tab and go to a page.
- **Right:** the inspector, which shows options for what is selected. On windows narrower than about 960 pixels it
  collapses to a thin rail; click the rail to slide it out. ☰ > View > Inspector shows or hides it.

Ctrl+K opens the command palette, which lists every command. Type to filter, use Up and Down to choose, and press
Enter to run.

## Links

Click a link with the read tool to follow it: the pointer turns into a hand over one, and hovering shows where it
goes. Dragging from a link selects text as usual. With the editing tools, hold Ctrl (Cmd on macOS) and click, so
links don't get in the way of editing.

- A link to another page goes there.
- A link to a web or email address asks first, showing the full address, and then opens it in your browser or
  email program. MPE itself never connects to the network. Links to anything else, such as a file or a program,
  are shown but not opened.
- Links that go to another PDF file are not followed. Links can't be added or edited yet.

## Comments and markup

Annotations (comments, sticky notes, highlights, text boxes, shapes and drawings that other programs added) are
shown on the pages. ☰ > View > Comments, or "Comments" in the command palette, opens the Comments tab, which lists
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

Some files store annotations without a drawing, leaving each reader to draw them. MPE draws highlights,
underlines, strike-outs, squiggly underlines, notes, text boxes, rectangles, ellipses, lines, polygons and
freehand drawings itself, on screen and when printing. The file isn't changed by this; flattening saves MPE's
drawing into it. Text boxes are drawn in Helvetica, ignoring any rich-text styling.

### Marking text

Select text with the read tool, then click **Highlight** on the bar above it (or right-click > Highlight, or
Ctrl+Shift+H). The small arrow next to it, or right-click > Underline, strike out, colour, has:

- Underline, Strike out and Squiggly underline.
- The highlight colour (yellow, green, blue, pink, orange) and the line colour (red, blue, green, black). The
  colours chosen are remembered between runs.

A selection across pages gets one highlight on each page, all in one undo step. The marks are saved as standard
annotations, so other readers show them and can change or remove them.

Click a mark (or a comment) with the read tool to select it: it is outlined, and the inspector shows who made it
and when, with its colour, opacity and comment, changed with Apply changes. Delete or Backspace removes it, and
Esc lets it go. A drag that starts on a mark still selects the text under it. Hovering over a mark shows its
comment.

New marks are saved with your name, which is the computer's user name until you set it with ☰ > Edit > Your name
for comments…. Leave it empty to save no name.

### Notes and text boxes

The **Annotate** button in the top bar holds the note and text box tools. It shows the one you used last; the small
arrow beside it picks the other. The inspector sets the note colour, or the text box's text size and colour,
background (none at first) and border, before you start.

- **Note:** click on the page. A comment box opens beside the note; type, then click Done (or press Ctrl+Enter).
  Cancel or Esc drops it, and so does Done with nothing typed. Hovering over a note shows its comment. Notes
  show at the same size on screen whatever the zoom, with MPE's own icon in the note's colour (also for notes other
  programs made with a different icon); printed, they keep their own drawing, 20 points across for MPE's.
- **Text box:** drag a box, or click for a default size, then type straight onto the page. Enter starts a new
  line. Click outside the box or press Ctrl+Enter to finish; Esc drops it. Text boxes use Helvetica, so characters
  outside Latin-1 come out as `?`. When you finish, the box grows downward to fit its text, keeping its width; it
  stops at the bottom of the page, and it never shrinks (resize it with the arrow).

A new note or text box is selected with the arrow, so it can be moved straight away. Double-click a note or text
box (with the read tool or the arrow) to change its text. Ctrl+click selects an annotation while an annotate tool
is on. The inspector for a selected text box also changes its text, size, colours, background and border, with Apply changes.

With the arrow, click any annotation to select it, drag it to move it, and drag a handle to resize it. Arrow keys
move it by 1 point, or 10 with Shift. Notes keep their size, and text markup stays on its text, so neither can be
resized, and markup can't be moved. Locked annotations can't be moved.

### Shapes, lines and drawings

The Annotate button's menu also holds the rectangle, ellipse, line and pen tools. What they draw are annotations:
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
to change its colour, width, fill, arrowhead, opacity and comment in the inspector, then Apply changes. Lines and
drawings are picked by clicking near their strokes, not anywhere in their box.

With the arrow, a selected line has a round handle on each end instead of a box: drag either end to turn the line
or point an arrow somewhere else (Shift keeps it at a multiple of 45° from the other end). Drag the line itself to
move it.

A selected freehand drawing has a round rotate handle above its box; the pointer becomes a circular arrow over it.
Drag it around to turn the drawing about the
middle of its box; the angle shows while you drag, and Shift keeps to steps of 15°. Rectangles, ellipses and text
boxes can't be turned: PDF stores them as upright boxes.

### Stamps

The stamp tool is in the Annotate button's menu ("Add a stamp" in the command palette). Its inspector lists the
ready-made stamps (Approved, Not approved, Draft, Final, Confidential, For comment and Void), each a coloured frame
with the word in capitals, your own wordings, and the pictures used as stamps before. Pick one, then click on the page to put it down
at its own size, centred on the click, or drag a box to size it (it keeps its shape inside the box). It is then
selected with the arrow, to move or resize.

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
its colours are part of the stamp. A stamp reads upright on a turned page. The text is Helvetica Bold, Latin-1
only, so a name in other scripts shows as `?`.

## Redaction

Redacting is done in two steps: mark what to remove, then apply the marks. Marks are outlined in red and remove
nothing by themselves, so they can be checked first, here or by someone else in another app (they are standard PDF
redaction marks). See [Redaction](redaction.md) for exactly what applying removes.

- **Mark areas:** Annotate > Mark areas for redaction, then drag a box over anything: text, pictures, drawings. The
  tool stays on, so several areas can be marked in a row.
- **Mark text:** select it with the read tool, then right-click > Mark for redaction (or Edit > Mark for redaction).
- **Mark search results:** search in the side panel's Search tab, then press Mark all for redaction (or Edit > Mark
  all search results for redaction). Every match is marked.

Marks are listed in the Comments tab as "Redaction mark". Select one (Ctrl+click it, or click it in the Comments
tab) to delete it or add a comment; like highlights, marks can't be moved or resized, so to change one, delete it
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

- **Text fields:** a click opens a text box over the field. Enter (in a one-line field) or clicking elsewhere keeps
  what you typed; Esc drops it. Password fields show dots, and their value is not kept in the file.
- **Checkboxes and radio buttons:** a click ticks or chooses.
- **Drop-downs and lists:** a click opens the options. In a list that allows several choices, each click adds or
  removes one; Done closes it. A drop-down you can type in has a text box at the top; Enter keeps the text.
- **Tab and Shift+Tab** move to the next and previous field, top to bottom and page by page. On a checkbox or radio
  button, Space ticks it; on a list, Space or Enter opens it.

Each value you change is one undo step. ☰ > View > Highlight fields tints every field and outlines the required ones
in red. The inspector's Form section shows the field you are in (its name, description and whether it is required),
with Clear field, Highlight fields and Reset form, which puts every field back to its default. Flatten form… (also
in the command palette) makes the fields part of the pages as they look now, so nobody can change the values,
here or in other readers; it asks first, and Undo brings the fields back until you close the document. Read-only fields
and buttons (they run scripts, which MPE doesn't) can't be filled in; hovering says why. Clicking an empty signature
field puts your signature in it (see [Filling and signing](#filling-and-signing)); clicking a digitally signed one
shows its signature's details (see [Digital signatures](#digital-signatures)).

MPE draws the filled fields itself, so the values show the same in other readers. Text must be Latin-1 (Western
European) for now; anything else is refused with a message rather than saved wrongly. Fields that have a value but
no drawing of their own are drawn when the file opens. See [Command line](cli.md#forms) for filling forms in
scripts.

## Filling and signing

The Sign button in the top bar (its icon is a signature; "Fill and sign" in the command palette) puts your
signature, initials, the date, your name, other text or a tick, cross or dot on the page, for signing a document
someone sent you. It is a picture of your signature, not a digital signature: it doesn't prove who signed, and
anyone can copy it. For digital signatures, see [Digital signatures](#digital-signatures).

**Making a signature:** in the Sign pane, Create signature… (or New… beside Signatures) opens a window with three
ways to make one:

- **Draw:** sign on the white pad with the mouse, touchpad or a pen; Clear starts again. The strokes are smoothed
  into curves and stay sharp at any zoom.
- **Type:** type your name (it starts as your name for comments) and pick one of four handwriting styles (Caveat,
  Cedarville Cursive, Homemade Apple and Zeyada), shown as they will look. Western European characters only; for other scripts, draw or use a picture.
- **Picture:** choose a photo or scan of your signature on paper (PNG, JPEG or TIFF). "Make the paper transparent"
  (on by default) keeps only the ink and cuts off the empty margins, so the signature sits on the page like ink.

Initials are made the same way, with New… beside Initials; typing starts with your name's initials. Each new one
goes first in its list in the pane, up to five signatures and five sets of initials; right-click one to take it off
the list. "Save for next time" (on by default) keeps it there when MPE starts again. Unticked, it is still in the
list to use until MPE closes, marked "until MPE closes", and is then gone. Saved signatures are stored unencrypted in MPE's settings folder (`signatures.txt`, and pictures copied into its
`signatures` folder), on this computer only; taking a picture off the list deletes its copy.

**Putting things down:** pick an item in the pane, then click on the page to put it down at its usual size, centred
on the click, or drag a box to size it (it keeps its shape inside the box). The tool stays on, so you can sign
several places in a row.

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
Ctrl+click an item (or use the arrow) to select it. While there are such items, a notice above the pages says how
many and has a Finish signing… button; the Sign pane has the same button. Finish signing (also in the Document menu)
draws them into the pages as they look, after asking, so they can no longer be moved or removed as comments; Undo
brings them back until you close the document. Saving without finishing keeps them movable, and the notice comes
back when the document is opened again. Other comments and stamps are left alone. Signatures are listed in the
Comments tab as "Signature", "Initials", "Tick" and so on.

## Digital signatures

When a document with digital signatures opens, MPE checks them in the background and shows the result in a
banner above the pages, coloured by the worst signature: green (valid, and you trust every signer), amber (nothing
broken or changed against the signers' wishes, but MPE can't confirm who signed, or there are warnings), grey (MPE
can't check a signature) or red (broken, changed in ways a signer didn't allow, or a trusted signer's certificate
fails a check). Its Signatures button, or ☰ > View > Signatures, opens the Signatures tab of the side panel, which
lists each signature with its result, the changes made after it, and Details… and Signed version buttons, then the
signature fields nobody has signed yet. Click a signature's name to go to it; with the read tool, click a signed
field on the page to see its details.

Details shows who signed, who issued their certificate, when, the changes after signing, warnings and the
certificates; Trust this signer… and Trust an authority… add to your trust list (after showing the SHA-256
fingerprint to compare with the owner), and View signed version opens the document as it was when signed, in a new
tab. ☰ > Edit > Trusted certificates… lists, removes and adds trusted certificates. Trust settings affect MPE only.

The check is of the file as opened or last saved; saving rewrites the file, which breaks its signatures (MPE asks
first), and MPE checks the saved copy. See [Digital signatures](signatures.md) for what each result means.

### Signing with a digital ID

☰ > Document > Sign with digital ID… (also a button in the Sign tool's pane), or Certify… for the author's
certification: click an empty signature field or drag a box on the page, then pick your digital ID, type its
password, and optionally a reason and a location. Sign… asks where to save the signed copy (`NAME-signed.pdf` by
default), signs in the background, and opens the signed copy in place of the document. Changes made since opening
are signed with it; when they would show on earlier signatures as not allowed, MPE asks first.

☰ > Edit > Digital IDs… makes, imports, exports and removes your digital IDs, and chooses the default. See
[Signing](signatures.md#signing) and [Digital IDs](signatures.md#digital-ids).

## Notices about the document

When a document opens, notices above the pages point out things worth knowing, each with a button where there is
something to do and × to dismiss it. At most two show at a time.

| Notice | Button |
| --- | --- |
| Its digital signatures are being checked, and then how they checked, coloured green, amber, grey or red (always first; see [Digital signatures](#digital-signatures)) | Signatures |
| The document was password-protected: saving removes the protection unless you set it again | Password security |
| Areas are marked for redaction but the marks haven't been applied (kept up to date while you work) | Apply redactions… |
| Signatures and other items put down with the Sign tool can still be moved (kept up to date while you work) | Finish signing… |
| Its pages are scans without text, so they cannot be searched or copied (checked on the first 20 pages) | Recognise text |
| It is a form made for Adobe Reader (an XFA form), which only Adobe software can show or fill in; MPE shows the placeholder page the file contains, often "Please wait..." | |
| It has a form: click a field to fill it in, or press Tab (not for a form of signature fields alone) | Highlight fields |
| It is large (more than 500 pages or 100 MB) | |

The first two go away by themselves once dealt with: after password security is applied, or after text has been
recognised.

## Selecting and the inspector

What you select decides what the inspector, the right-click menu and the floating toolbar offer.

| Selected | Inspector |
| --- | --- |
| Nothing | The document: properties, opening view, password security, crop, watermark, headers/footers/Bates, OCR, optimise scanned pages, export pages as images, split into files, and buttons for export text, print and append PDFs |
| Pages (click thumbnails; Shift for a range, Ctrl for one more) | Rotate, delete, extract to a new tab, insert images or a blank page after them, page labels from the first selected page, and the crop, watermark, headers/footers, OCR and optimise tools with the pages filled in |
| An object, words, or a tool that adds content | That object's or tool's options |

Menu commands for document and page settings open the matching section of the inspector.

A watermark is text or a picture (PNG, JPEG or TIFF), centred on each page. After you apply a watermark or headers
and footers, the section's button becomes Update: change a setting and press it to redo what you just added rather
than stamping a second copy. Add another keeps it and adds a new one. Any other change returns the button to Apply.

If a damaged file makes MPE fail while showing a document, that tab shows an error page with Save a copy and Close
this tab; other tabs carry on.

Page labels are the page numbers viewers show, such as i, ii, 1, 2 or A-1; they don't change what is printed on the
pages. When a document has them, the status bar shows the label with the page's place, such as "Page i (3 of 477)",
and the thumbnails show the labels. To set them, select the first page to number, open Page labels in the
inspector, and pick a style, an optional prefix and the number to start at; the numbering runs to the next labelled
range. Labels stay with their pages when pages are deleted or moved.

Opening view, in the document's inspector, is how viewers that follow it show the document when it opens: its
layout (continuous, facing, facing with the cover alone, or single page), the side panel (Pages or Bookmarks) and
the page it opens at. Each can be left as Not set. Use current view fills them in from how you are viewing the
document now; Apply saves them into the document like any other change, and Undo reverses it.

Split into files makes files of a set number of pages, or starts a file at each bookmark down to the chosen level
(1 is the top level only). Files are named after their bookmark, such as `book-002 Introduction.pdf`, and the pages
before the first bookmark get a file of their own.

Crop trims margins from each side of the page as it is shown, measured from the full page; the fields start with
the first page's current crop. Remove crop shows the whole page again. Cropping hides content but does not delete
it.

Right-click a page, an object, selected words or a thumbnail for a short menu of what applies there. Right-clicking
selects what is under the pointer first. A selected object or run of words also gets a small floating toolbar with
the same commands.

Drag thumbnails to put pages in a new order. A line shows where they will go. Dragging a selected thumbnail moves all
the selected pages.

Delete removes the selected object or pages. Deleting shows a notice with an Undo button.

Remove blank pages (☰ > Document, or the command palette) deletes every page that is one even colour, give or take
scanner specks, faint show-through or a lone page number, and lists the pages it removed. Undo brings them back. A
document whose pages are all blank is left alone.

## Editing page content

Pick a tool in the top bar. Documents open with the read tool (the text cursor), which selects text, follows links
and fills in forms but never changes page content. The arrow selects objects to edit.

With the arrow:

- Click an object to select it.
- Drag to move it; drag a handle to resize it (Shift keeps the proportions).
- Arrow keys move it by 1 point, or 10 with Shift.
- The inspector shows its position and size, and for text the string, font, size and colour; for shapes the fill,
  outline and width; for images "Replace image".

The other tools add content:

- **Text:** click where the text starts, type, press Enter.
- **Image:** choose a PNG, JPEG or TIFF, then drag a box.
- **Annotate:** sticky notes, text boxes, rectangles, ellipses, lines and the pen, saved as annotations rather
  than page content; see [Notes and text boxes](#notes-and-text-boxes) and
  [Shapes, lines and drawings](#shapes-lines-and-drawings). A rectangle, ellipse or line can still be drawn into
  the page content with "Draw on page (permanent)". Its menu also has the stamp tool (see [Stamps](#stamps)) and
  the tool that marks areas for redaction (see [Redaction](#redaction)).

Everything that changes the document can be undone with Ctrl+Z. See [Content editing](content-editing.md) for how
editing works and its limits.

## Copying text

With the read tool, drag to select words in reading order. A drag can start anywhere on the page, such as in the
margin: it starts at the nearest word. Dragging past the top or bottom of the view scrolls it, faster the further
out the pointer is, so a selection can run across pages. Double-click selects one word, triple-click a line, and
Shift+click extends the selection to where you click, on any page. Ctrl+A selects the whole page in view. Esc or
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
| Esc | Cancel: clear the selection, or leave a drawing tool |
| Tab, Shift+Tab | Next, previous form field; Space ticks a checkbox or opens a list |

## Look and feel

The app follows the system's light or dark theme. The theme button at the right end of the top bar (a half circle,
sun or moon, for the current choice) chooses System, Light or Dark; so does ☰ > View > Theme. The choice is
remembered between runs, in `settings.txt` in the same folder as the recent-files list, along with the markup
colours and your name for comments. The app uses the Inter font
and Phosphor icons.

☰ > Help > About shows the version (the Copy button next to it copies the version and system, for a bug report),
links to the website, the licence and the issue tracker, which open in your browser, and Credits for the bundled
fonts, icons and text recognition model. Contributors switches the window to the people who made MPE; Back returns.
MPE is made in Canada, which the About window and the foot of the start screen say beside a maple leaf.

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

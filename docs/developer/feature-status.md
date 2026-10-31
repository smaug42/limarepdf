# Features and status

Every feature's status in detail, with its limits, what doesn't work yet and what is planned, for testers and
people working on LimarePDF. The user guide's [Features](../features.md) is the short summary.

**Status: 0.11, for testing.** Most testing has been on Linux. Annotations can be viewed, moved, deleted and flattened;
text can be highlighted and marked; notes, text boxes, shapes, lines, freehand drawings and replies added. Content
can be redacted for good, and documents filled in and signed with a picture of your signature. Digital signatures
can be checked, made with your own or an imported digital ID, certified, and kept valid when a signed document is
saved, in the app and from the command line; Adobe Acrobat Reader shows LimarePDF's signatures, and fuller round trips
with Acrobat are still to be tested.

## What LimarePDF does

- **View:** tabs, thumbnails, continuous scrolling, zoom, search with highlights on the page, bookmarks, links and
  comments.
- **Organise pages:** every page as a card in Organise's grid, as a slide sorter: select with clicks, a drag or
  the keys, drag, cut and paste or Alt+Shift with an arrow to move, insert where you click between pages;
  rotate, insert, delete and extract pages; drag thumbnails to reorder; merge and split files.
- **Edit content:** double-click a line of text to edit it in place, making words bold, italic or another font,
  size or colour as in a word processor, or select words while reading (across lines and pages) and style them
  the same way; move, resize, restyle and delete text, images and shapes; add text, shapes and images. Changes in
  the inspector apply as you make them.
- **Fill in forms:** text fields, checkboxes, radio buttons and lists, in the app or from JSON on the command line;
  flatten them so the fields can no longer be filled in.
- **Fill and sign:** put a drawn, typed or picture signature, initials, the date, your name and ticks on any
  document, then fix them into the pages. Not a digital signature.
- **Stamp pages:** headers, footers, page numbers, Bates numbers, and text or picture watermarks.
- **Scans:** turn images into PDFs, shrink and straighten scanned pages, and add a searchable text layer with
  built-in OCR (Latin script).
- **Redact:** mark text, areas or search results, then remove what is under them for good (text, the covered parts
  of pictures and scans, drawings), checked afterwards; and remove hidden information such as metadata, scripts,
  attachments and hidden layers.
- **Copy text,** including recognised text.
- **Protect:** open and permissions passwords, RC4 or AES encryption, permission flags.
- **Export and print:** pages as PNG or JPEG, text as a `.txt` file, and printing (through CUPS on Linux and macOS,
  the print spooler on Windows).
- **Command line:** `limarepdf-cli`, for scripts and batches, organises pages, fills in and flattens forms, adds
  headers, footers, watermarks, stamps and Bates numbers, redacts, removes hidden information, protects, signs and
  checks signatures, makes PDFs from images, shrinks and recognises scans, exports pages as images and text, and prints. Editing page
  content, adding comments and Fill and sign are in the app only: the command line lists page objects, and lists,
  removes and flattens comments ([Command line](../cli.md)).
- **Private:** no network connections, accounts or licence checks.

## What doesn't work yet

Things you would expect an everyday PDF editor to do that LimarePDF doesn't yet, as of 0.11. Each is removed from
this list once it works. The feature table below has the details.

**Typing and editing text**

- Only Latin (Vietnamese included), Greek, Cyrillic, Chinese, Japanese and Korean characters, and single emoji,
  can be added: in new or edited text, text boxes, headers, footers, Bates numbers, watermarks, stamps, form
  values, fill-and-sign text and a digital signature's look. A word the font lacks goes to an installed font that
  has it (Chinese and Japanese a character at a time), so Chinese, Japanese, Korean and emoji need such a font
  installed (Noto Sans CJK, say, or Noto Emoji, Symbola or DejaVu Sans for emoji); colour emoji fonts aren't
  used, as their emoji would show nothing in the PDF. A character no installed font has is refused ("No font on
  this computer has this character").
  Hebrew and Arabic (written right to left), the scripts whose letters join or change shape (Devanagari and the
  other Indic scripts, Thai) and emoji made of several characters (flags, skin tones, keycaps, and joined emoji
  such as families) can't, in any font, a line's own font included: typed or pasted in new text, a line being
  edited, a text box, a header, footer or watermark, stamp wording or fill-and-sign text, they aren't typed and a
  note at the caret says so; form values with them are refused with a message. Interface text in those scripts
  (a file name, say) isn't drawn properly: as boxes, or unjoined and left to right. A text box made by another app with such characters can be
  moved and deleted, but a new colour, opacity, style, size or text is refused with a message, as LimarePDF would
  have to draw it again; if it has no drawing of its own, LimarePDF shows, prints and exports it with `?` for
  them, and flattening comments refuses, naming its page. A plain-text page of selected
  text shows them as `?`.
  Comments, replies, bookmarks and document properties keep every character, emoji included.
  Typed signatures show what their handwriting has: Caveat has Cyrillic, the other three Latin only.
- Installed fonts are offered only when their licence allows embedding and they have the Latin letters (those
  without are used only for the characters a chosen font lacks); a family's missing bold or italic can't be made
  up (the button is off). A word in a fallback font can look a little different in size and weight beside its
  neighbours, as in Word. On a system whose language isn't Chinese, Japanese or Korean, the first ideograph typed
  in a serif font waits about a tenth of a second for its font, once a run (PB-61). Editing a line
  keeps a changed word's font only for letters the file's font verifiably holds (a subset holds only the letters
  the file used; Type 3, unembedded beyond plain ASCII in a standard encoding, vertical and unreadable fonts take
  none); otherwise the word is written in the installed family it came from, when LimarePDF can embed it, or else
  in Helvetica, Times or Courier, and a line that would read as other words is written so whole (a notice says
  so). While typing, `TJ` spacing between letters isn't drawn, and a ligature's letters draw in the look-alike
  ([details](../content-editing.md#limits)). A field set to centre or right-align its text shows it from the left
  while it is typed (PB-59), and a text box from another app with inner margins, clouds or a callout can wrap
  differently while typed than once kept (PB-60). A space the line had as a small gap (after a subscript) may no
  longer read as one between larger letters, so such a line is written with its changed words in another font
  (PB-50); a line the page draws twice (a label over a halo) is edited where clicked, the copy under it
  keeping the old words (PB-51); the first line selected on a page with heavy content waits for the page to be
  read, a tenth to a third of a second on 2.5 % of the corpus's pages, once a page (PB-52). Fonts installed under
  code names are shown by the typeface they stand in for only for the URW fonts Linux installs; others show
  their own names.
- Text is edited one line at a time: a paragraph doesn't rewrap as you type, so a longer line can run into
  whatever is beside it. The outline warns of that ("Over other text") only for text right of the line's end, and
  new text only of the page's edge; the warning's tag can be under the floating toolbar when the toolbar sits
  below the line. Typing over words selected on more than one line types nothing (a note says paragraph editing
  is planned), and deleting or cutting words doesn't join lines: a line emptied that way is left blank, the lines
  around it where they were.
- Words selected with the read tool have no caret: the arrow keys, Home and End without Shift keep their reading
  meanings (scrolling, pages) rather than moving to the selection's start or end as a word processor does; with
  Shift they extend the selection.
- Page text has no underline or strike-through (underline and strike-out comments can be added over it).
  Styling selected words doesn't rewrap their lines, and highlights, links and redaction marks over them stay
  where they were. Invisible recognised text, text inside a form XObject and vertical writing can't be styled
  (a notice says what was left).
- A line turned on the page, and invisible recognised (OCR) text, are edited in a box over the line rather than
  drawn where they are (a turned line's box shows its fonts, sizes and colours; the caret goes by the character
  clicked). Over a picture or a shaded background, the line being edited is hidden under one flat colour, the one
  most common around it, until you finish. Text in a Type 3 font whose pictures of letters are stored upside down
  (pdf.js `issue2984`, PDFBox `1628`) reads as turned, as only its pixels say otherwise; a Type 3 font's size is
  measured from its letters' widths, so a font of few or unusual letters can read a little larger or smaller
  than it shows (PB-56).
- Text in a colour a PDF names (a spot colour, Lab, an indexed palette, an ICC profile) shows in the inspector
  and while typing as LimarePDF works the colour out, without the colour profiles a printer or Acrobat applies,
  so a shade can look a little different there; the page itself keeps the ink. A colour picked for such text is
  written in RGB.
- No spell checking.

**Selecting and arranging**

- Several objects selected together (one page at a time) move, restyle and delete together but can't be resized
  or turned together (PF-45), nor aligned or spaced evenly (NEW-08). Comments and markup are selected one at a
  time (PF-44), never with page objects.
- Comments have no status (resolved, accepted, rejected), so the Comments view can't filter by one, and can't be
  exported or imported as comment files (FDF, XFDF; PF-05). The view's filter and search last while the tab is
  open; they aren't remembered for the next time the document is opened. Replies are typed in the inspector's one
  reply box, not on the card.
- Bookmarks can't be moved or reordered, and links can't be added or edited.
- Merging doesn't carry over the bookmarks of the files added, and their form fields only drawn onto their pages:
  they can't be filled in afterwards. A file with an answer LimarePDF can't draw (text in characters it can't
  write in a field yet, a radio button with no drawing of its own) can't be appended.
- Pages are cut and pasted within their document only: they can't be copied (to have them twice), nor pasted
  into another document's tab. Organise's insertion bar between pages is set with the mouse; with the keys, new
  and pasted pages go after the pages selected.

**Forms, signatures and printing**

- Forms made only for Adobe Reader (XFA) can't be shown or filled; form data files (FDF, XFDF) and form
  JavaScript aren't supported.
- Digital signatures carry your computer's time, not a timestamp from a timestamp server; encrypted documents
  can't be signed; digital IDs on smart cards and USB tokens can't be used.
- Printing has no paper size or scaling options.

**Other**

- The command band's Paste and Copy work on text typed on the page (Paste also on words selected while reading
  and on pages cut); in a form field or one of the inspector's boxes, use the keys (Ctrl+V and Ctrl+C, ⌘V and ⌘C
  on macOS): a right-click there opens no menu. A comment text box's font size and colour are set in the inspector, not in
  Edit > Font. The inspector's edge takes a drag only from the panel's side of its line: a press on the line or
  just onto the pages goes to the pages (PB-33). There is no high-contrast theme for the window itself (High
  contrast is a page colour, for the pages only), and LimarePDF doesn't follow the system's high-contrast setting.
  In a window too narrow for the floating toolbar, it is cut off at the edge of the pages rather than folding to
  one button.
- No export to Word or Excel, no comparing documents, no PDF/A.
- Search has no Replace, and no options to match capitals or whole words only.
- Text recognition (OCR) reads Latin script only.
- No autosave or custom shortcuts, and English only (unsaved work is kept in recovery copies).
- Support for every system is provisional ([requirements](../install.md#requirements)): tested by hand mostly on
  Linux, Windows only partly, macOS not at all; printing on paper only on Linux. Dropping files onto the window
  doesn't work in Linux Wayland sessions yet. The Linux builds need glibc 2.39 or later (Ubuntu 24.04, Debian 13,
  Fedora 40); on older systems LimarePDF has to be built from source.
- Screen readers haven't been tried yet (Orca, Narrator, VoiceOver), so what they say as F6 and Tab move the
  keyboard round the window is known only from the app's own tests.

## Feature status

**Done:** works, and is covered by tests or has been run. **Partial:** works within the limits noted. **No:** not
available.

| Area | Status | Notes |
| --- | --- | --- |
| Tabs, thumbnails, opening by drag and drop | Done | With many documents open the tabs keep one row: each at its own width up to 240 points, sharing the room when they don't fit, down to 160 points (the icon, about ten characters of the name and the close button; a dot stands for Unsaved when there is no room for the word), names cut with "…" and shown whole on hover and to screen readers; past that the row scrolls by the wheel or the arrows at its right (a tab a click; held, they step again after 0.4 s and then every 0.12 s), and All open documents lists every document. Ctrl+Tab and Ctrl+F6 bring the next document to the front, with Shift the one before, round from the last to the first (☰ > View > Go to document; Ctrl on a Mac too, where Cmd+Tab is the system's; they work with the keyboard in a box; greyed with one document open). Ctrl+PageDown and Ctrl+PageUp are left out: Word and Acrobat use them for pages, and they move the keyboard in the thumbnails and Organise's grid. The tab in front is scrolled into view when it changes, a document opens or closes, or the window narrows; a tab reached by Tab comes into view. Typing open on a tab's page (new text, a line, a note or text box) waits while another tab is in front and has the keyboard again when its tab comes back. A tab says Unsaved, and the window's title is marked, from the first key typed (on a page, in a form field or the inspector), not once the typing is kept. Dropping files doesn't work in Linux Wayland sessions yet ([workaround](../troubleshooting.md#dropping-files-on-the-window-does-nothing)). Files opened from the file manager open in tabs, also while the app runs; on macOS (Finder's Open With, double-click, the Dock icon) not yet tried on a Mac |
| Workspace: commands, command band, planned features, inspector | Done | Batch 39 (ENH-37, ENH-38, ENH-41, ENH-08, PF-40): its first and second stages built. The tabs on top; under them one row of the document's commands as icons (☰ the File menu: File's commands, then a submenu per kind of command, so every command is in it; Open, Save, Undo, Redo), the band's names Edit, Insert, Review, Pages, Forms and sign, Protect and View, and Print and the compact commands button at the right; under that the chosen name's groups of labelled buttons, with ▾ menus for related commands. Every ordinary command has a place in a band, the File menu, a right-click menu or a key (checked by a test). Edit's Font group sets the look of the text selected or being typed. Planned features show greyed, marked Planned, where they will be, and do nothing (Show planned features, in the status bar and View > Show, hides them, remembered). Compact commands puts the band on one row of icons (its button, or View > Show; by itself under 600 points high). Groups without room fold into a button with their name; groups of planned features alone go first. The accent colour follows the desktop's (Linux portal, macOS, Windows) unless View > Accent colour says LimarePDF; teal where the desktop gives none. The icon strip at the left edge shows the side panel's five views, each keeping its own width for the session (Pages starting at 132 points, the views of text at 220, so their lines don't wrap a word or two at a time). The inspector (the second stage) is headed by what it shows, with its facts in muted labels at the top and its sections folded under thin lines: with nothing selected, the document (Pages, Paper, Security, and Form fields and Signatures when it has them; Properties, Opening view and Form folded), no longer every tool nor Export text, Print and Append PDFs; selected pages, which they are, their Paper and Orientation as shown, their buttons, and Insert blank page and Page properties (Crop, Page labels) folded; selected text, a line or words, ends with Future text tools, marked Planned (hidden with the planned features). A tool (Headers, footers, Bates; Watermark; Split into files; Crop; Page labels; Recognise text (OCR); Optimise scanned pages; Password security; Export pages as images) opens alone from its command (PF-40), headed by its name with a close button, in the panel and the pop-out; it stays open while pages are selected, its Pages box following them, and gives way to words, an object, a comment or a tool that adds content. Opening and closing one changes nothing in the document. The third stage compared captures of the workspace with the approved mockup, light and dark, wide, narrow and short (testing.md): the inspector now sits on the band's white surface with the mockup's padding and row spacing, selected pages no longer widen it, Planned tags are outlined, and the widths given the panels are remembered between runs. Left as they are: the differences listed in testing.md's batch 39 comparison (the selection shaded over the page's text, a few carets' shapes, the status bar's faint tick box, no Planned mark in Compact commands). Batch 40's third stage (ENH-39): a greyed button or menu item (the band, its menus, the commands row, ☰'s menus, Edit > Font's Bold and Italic) says why on hover, after its name and shortcut ("Nothing to undo.", "Select some text first.", "Click in a form field first.", "<family> has no bold."; "Not available just now." where no reason LimarePDF knows fits); the tools in the menus are radio items that mark the one in use, and Edit text shows pressed with the Text tool on; the status bar says "Editing one line" while a line is edited; a form field with the keyboard heads the inspector "Form field" and lets go of words selected before. [Details](../gui.md#the-command-band), [inspector](../gui.md#selecting-and-the-inspector) |
| Recent files | Done | Start screen and ☰ > Open recent; 10 files, kept on your computer only |
| A file open in one tab; changes made elsewhere | Done | Opening an open file again (by any path or link) shows its tab. Save asks before replacing a file another app or tab changed (Save as… or Replace); a notice with Reload shows when you come back to the window after such a change; ☰ > Reload from disk. Not checked while LimarePDF has the keyboard, only on coming back, opening and saving. [Details](../gui.md#saving) |
| Notices about the document | Done | Password-protected, scanned pages, form fields, large files. [Details](../gui.md#notices-about-the-document) |
| Zoom, fit width, continuous scrolling, page navigation | Done | Pages fit the window's width by default and follow resizing; zooming turns that off. Zoom from 10% to 6400%, drawn in tiles at high zoom so pages stay sharp. The status bar zooms as a word processor's: − and +, a slider from 10% to 1000% (none in a window under 900 points wide) and the zoom's menu, which says the zoom alone in a fixed space (Fit width ticked in it), so nothing beside it moves as the zoom changes. Single-page and facing-page layouts (the zoom menu at the bottom right, View > Facing pages ▾, or ☰ > View > Page layout; cover page alone or paired), per document, following the document's own preference when it has one. [Details](../reading-and-finding.md#move-around-the-pages) |
| Open at a page from the command line | Done | `limarepdf file.pdf --page N` |
| Timing log | Done | For measuring how quickly the app responds: `LIMAREPDF_TIMING=timing.tsv limarepdf file.pdf` writes when the app starts, takes input, finishes each frame, first shows each page sharp, and starts and ends reading, drawing, searching and saving. Off unless set. It can't see when the screen shows a frame, only when the app has built it. [Details](performance.md#startup-and-responsiveness) |
| Command palette, right-click menus, floating toolbar | Done | A tool's tooltip says how to use it, a greyed command's why it can't be used just now; the inspector shows controls, not instructions. The palette shows a command in a submenu with the submenu's name ("Theme › Dark", "Page colours › Dark"), for screen readers too, and finds it by that name. The floating toolbar sits by the selection where it covers no other text (above, below, then beside it), else just above or below it over the least text, never over the selection, as Word's mini toolbar; at the view's edge only when neither fits. While a line is edited on the page it has Bold and Italic, then Cut, Copy and Paste, for what is selected in the line, and a click on it leaves the line open (ENH-20; Paste since batch 43). Right-click menus hold what applies: selected words Cut, Copy, Paste (typing the clipboard's text over them), Select all text, then Edit text, Bold, Italic, and the rest; text being typed on the page (a line being edited, new text, a note or a text box) Cut, Copy and Paste, then Bold and Italic where they apply, a right-click on what is selected in it keeping it selected, as word processors do (batch 43, the user's find), the floating toolbar hidden while the menu is open; a line of page text Edit text, Move or resize text, Copy the selected text lines, then Cut and Delete; a picture or shape Replace image… and Delete. In a view narrower than the bar, it is cut off at the view's edge rather than folding to one button. Toolbar customisation: No |
| Pages boxes | Done | Numbers and ranges (`1-3,7`, `5-`, `all`) in printing and the inspector's tools; going into one that says `all` selects it, so typing replaces it. [Details](../gui.md#pages-boxes) |
| App icon | Done | Window, taskbar and Windows `.exe`. Wayland needs the desktop entry installed. [Details](../install.md#menu-entry-and-icon-on-linux) |
| About window | Done | Version (copyable for bug reports), links, contributors, credits; "Made in Canada". [Details](../gui.md#look-and-feel) |
| Dialogs | Done | One look for every dialog and the command palette, as in the design's mockups (settings in headed groups, labels above fields, outlined fields and buttons, short fields side by side when there is room, smaller margins in narrow windows): task and document in the title, the action button highlighted (red when Undo can't take it back) in the platform's order, the window behind blocked, Esc as Cancel unless work would be lost, Enter in a dialog's only field answering it, a question or message arriving while another is open waiting its turn (closing the window asks first, and the question it interrupted comes back after Cancel), Copy on messages, reasons under Details, problems beside their fields (a missing or wrong password under the password, when signing too), and a digital ID form whose work fails comes back with what was typed (passwords aside); Tab goes through fields and buttons in the order shown, from the last straight back to the first, and screen readers can name each one (a row's buttons say which document or certificate); long file and printer names are cut short. Each dialog is checked by the app's tests in wide, 400-point-narrow and 300-point-short windows at scales 1, 1.5 and 2, and by screenshot; not yet with a screen reader. Checked on Linux only |
| Names for screen readers | Partial | Every button, field, list and menu in the window, the inspector, the side views and the dialogs has a name a screen reader can say: icons and symbols such as ◀ in words, Bold, Italic and the picked signature or mark with whether they are on. Each region F6 goes to is named with its role (Tabs, the commands by the band shown, such as Edit commands, Notices, Navigation, Document, Inspector (the pop-out too), Status bar), the side panel by its view, a document's tab says whether it is the one shown and unsaved, the band's names are tabs saying which is shown, the icon strip's views say which is shown, a planned feature's name ends in "Planned" (the inspector's "Future text tools — Planned" too, a Tab stop as it opens), and a tool's close button names its tool ("Close Watermark"). The panels' edges are named Side panel width and Inspector width (before batch 26 egui's own "Resize panel" was left: the names went to the wrong id). Checked by the app's tests on Linux; not yet tried with a screen reader. [Details](../gui.md#screen-readers) |
| Keyboard: regions, panel widths | Done | F6 and Shift+F6 move the keyboard between the tabs, the commands, notices, navigation, the page, the inspector and the status bar, in their order on screen, to the control last used in each or its first; regions not shown are skipped (a narrow window's strips stand in for the panels); nothing happens while a dialog, the command palette, a list or a menu is open. The page takes the keys with nothing focused, outlined; typing open on the page (new text, a line, a note, a text box) waits, open, while the keyboard is elsewhere (the band's Font group and the inspector still change it; a click in its tab ends the wait, another tab coming to the front doesn't), and a form field's text box, kept as it is left, is gone back to. The panels' edges, reached by Tab, move with Left and Right (10 points, 50 with Shift), Home (as narrow as the panel goes) and End (half the window), never folding the panel away. With a tool open in the inspector, F6 there reaches its close button first; Enter closes the tool, leaving the keyboard on the collapse button (in the pop-out, on the strip's button that opens it). The outline that moves new text is mouse-only: Esc keeps the text selected and the arrow keys move it. Create signature… and Create initials… are in the command palette, ☰ > Tools and Forms and sign > Fill and sign ▾. Not yet: screen readers tried (REL-06). [Details](../gui.md#the-window) |
| Window size | Done | Remembered between runs, and whether it was maximised; position: No. [Details](../gui.md#look-and-feel) |
| Light and dark themes | Done | Follows the system, or chosen in View > Theme; remembered between runs |
| Page colour modes | Done | View > Page colours: Normal, Automatic (dark pages with the dark theme), Dark, High contrast, Sepia. Pictures keep their colours unless "Recolour pictures too" is on. On screen only: the file, printing and exports keep their colours. See [Rendering](../rendering.md#page-colour-modes) |
| Rendering | Done | Pages that can't be drawn, and pictures and fonts that can't, are reported; high zoom is drawn in tiles up to 6400%; large page images are drawn in bands, which bounds the memory a complex page takes (one test page from 9.7 GB to 1.5 GB), and the app stops drawing a page at a zoom it has moved past; fonts that aren't embedded use installed fonts or similar stand-ins where hayro asks (CID fonts, which Chinese, Japanese and Korean text uses), with a notice saying what to install; annotations without a drawing of their own are drawn by LimarePDF; damaged files are repaired when opened, with a notice. Known limits, mostly hayro's: some transparency groups are drawn wrongly, hairlines can come out light grey, plain fonts that aren't embedded use the standard fonts, media and 3D annotations aren't drawn, and no rich text or line captions. Compared with poppler over about 1,100 public test PDFs. [Details](../rendering.md) |
| Search | Done | As you type: the search runs on a thread of its own 200 ms after the typing pauses (Enter at once), a newer query stopping an older one and a changed query's old matches going at once, so matches for another query or an older state of the document are never shown; the first search reads each page's text once, for the matches, their snippets and their highlights together, and keeps it for the next queries, read again only for the pages an edit changed (all tabs' kept text capped at 256 MB, the tab searched longest ago let go first); an edit searches again, keeping the current match. It goes to the first match on or after the page being read; Enter and Shift+Enter, ◀ and ▶ step, round the ends. Results come in page by page under each page's heading, the match in bold among whole words cut with "…", with "Searching…" and the count so far, then "1 match", "12 matches" or "No matches"; only the rows in view are drawn. Mark all for redaction marks every match, finishing a search still running first. Phrases across line breaks and page breaks, and words hyphenated at a line or page end, are found and highlighted (on both pages). Pages are called by the document's labels, or by number with View > Use the document's page labels off (thumbnails, results, bookmarks; the status bar shows both). 300 pages: 516 ms on the window's thread before, 121 ms on the search thread for the first query and 4 ms for the next ([measurements](performance.md#search)). Chinese, Japanese and Korean text in fonts without a ToUnicode map is read through the font's character map; other embedded fonts without one (TeX's, say) through the font's own encoding and glyph names, as other readers do, and the standard Symbol and ZapfDingbats through theirs; Identity fonts without one (codes that are glyph numbers) through their embedded program's own character map, as poppler does, and a ToUnicode entry naming Identity as codes that are their characters' numbers (PB-26: 9 of the 1,089 test files read better, none worse). An Identity font whose program says nothing about its glyphs (not embedded, CFF, or cut down to no character map or names) still reads wrong. A character a Chinese, Japanese or Korean font's maps don't cover reads as the replacement character �. [Details](../reading-and-finding.md#find-words) |
| Select and copy text | Done | With the default read tool; across pages, scrolling while you drag; in the order on the page, not the file's drawing order; includes recognised text; shaded as word processors shade a selection, a band a line with the spaces between words, the lines of a paragraph meeting. Shift with the arrow keys, Home and End extends it by a word, a line, or to the line's start or end, across pages, the view scrolling only as far as needed; Ctrl+A selects the page's text with the read tool or the Text tool; a double-click or triple-click away from any text selects nothing. Ctrl+X cuts it (copies, then deletes it). On Linux, the selected text can also be pasted into other apps with the middle mouse button. [Details](../reading-and-finding.md#copy-text) |
| Print or save selected text | Done | As the pages look, full width, cut down to the selected lines, or as plain text in Helvetica (words it lacks in an installed font that has them; characters LimarePDF can't write come out as `?`); also saves a .txt file. [Details](../reading-and-finding.md#print-or-save-just-the-selection) |
| Page operations: insert, delete, rotate, extract, reorder, crop, remove blank pages | Done | Crop by dragging the crop box on the pages, shown whole while it is set, or by typing margins; as a word processor's picture crop, Pages > Crop shows pressed meanwhile and the status bar says "Cropping N pages", and a click away from the crop box's edges, Enter, Esc or Crop again ends it, showing the pages cropped (batch 42, the user's find). Blank means one even colour, give or take specks or a lone page number. Deleted and extracted-away pages leave nothing in the saved file: bookmarks, named destinations, links, form fields and tags no longer refer to them, and the tags' alternative text for their content goes (also where only the page's index of its tags places it, and where nothing tells its page). A form with fields nested more than 32 deep is refused, changing nothing ([details](../organising-pages.md)) |
| Organise pages: the page grid | Done | Batch 41 (NEW-14); checked on a 10,000-page document (it opens in about 0.1 s, each key, click and scroll of the grid is one quick frame; page changes there take 0.1 to 0.45 s, see Known limits) and beside the approved mockup's Organise state, light and dark, narrow, short and scaled. Pages › Organise, View › Organise pages (ticked), Ctrl+K and the status bar's Page view and Organise pages buttons show the tab's pages as a grid of cards in place of the pages, each the page fitted in a frame of the document's usual page size, with a checkbox and its name; the heading and status bar count the pages and those selected. Click, Shift+click, Ctrl/Cmd+click, the checkboxes, a drag across the background and Ctrl+A select; Esc lets go; double-click or Enter opens the page; page commands act on the cards selected; tools and page-view commands go back to the page view at the first selected page; back from the grid, the page view is where it was. Cards dragged show where they will land, scroll the grid at its edges, and move in one step (Esc or a drop outside the grid cancels). The grid is one Tab stop: arrows, Home, End, Page Up and Down move and select (Shift extends, Ctrl/Cmd moves alone, Space adds or takes out), a ring on the card the keys are at. Alt+Shift with an arrow moves the selected pages a place or a row, with Home or End to the start or end (also Pages › Move ▾ and the menus); pages apart stay apart. Cut and Paste move pages (cut ones faint until pasted, Esc takes the cut back; the clipboard gets their names, so the paste keys work). A click between cards puts the insertion bar there, where Insert PDF pages…, Insert blank page, Insert images… and Paste put pages (else after the pages selected); new pages are selected. Only the rows in view are drawn; the cards zoom on their own (8 to 60 %, remembered). The selection, the keys' place and the pages cut follow their pages through every change, Undo and Redo, by page object; moving, adding, deleting or turning pages, and its Undo and Redo, keep the other pages' pictures. The thumbnails are one Tab stop with the same keys, a column. Pages can't be copied or pasted into another tab yet. [Details](../organising-pages.md) |
| Merge and split | Done | Split by page count or at bookmarks, files named after them. Append PDFs adds files in order in one step, asking for a protected file's password (or skip that file); a file that can't be read adds nothing. Bookmarks and named destinations of appended files are not carried over; a file with form fields is asked about, then its fields are drawn onto its pages with their answers (no longer fillable), and an answer that can't be drawn adds nothing (`limarepdf-cli merge --flatten-forms` does the same). Split parts, like extracted pages, keep the bookmarks to their own pages and nothing of the other pages; each is written as soon as it is made, so memory stays near the document's own size. Asks once before replacing files already in the folder. Split by file size: No. [Details](../organising-pages.md#split-into-files) |
| Undo and redo | Done | 100 steps, as Word and LibreOffice keep, including password settings (batch 43, PF-42). Each step keeps only what its change changed (a note on a 100-page scan a few kilobytes, not a copy of the document), and Undo and Redo put that back in place without reading the document again, so a document opened with its password, or repaired when opened, stays known as such. Up to 1 GB per document: past it the oldest steps go first and the latest is always kept, and what a watermark or headers and footers just added keep so their tool can change them counts too (on a document too large for that, the tool stops changing them and says so; see Known limits). An object's number isn't given to another after Undo, so a draft reply stays with its comment. While you type in a box (new text, a line, a note, a form field, the inspector), Ctrl+Z and Ctrl+Y undo and redo that box's typing only, each document's apart, and so do Undo and Redo in the commands row, the keyboard staying in the box and the buttons greyed by that typing's history (batch 40's Q4); with a button holding the keyboard, Ctrl+Z and Ctrl+Y are the document's. A box the keyboard comes back to with nothing kept in the document since it left keeps its typing's history (ENH-32); a new note or text box starts empty. Undo and Redo keep words selected with the read tool when they still read the same. A deletion's notice offers Undo for that change in its own document, until something else changes it. [Details](../gui.md#undo) |
| Headers, footers, page numbers, dates, Bates numbers | Done | Shown on the pages and thumbnails as you set them (once a setting settles, as adding them would make the pages, ENH-23; drawn over the pages while it changes), added with Add to pages, then updated in place as you change them. Helvetica, words it lacks in its Liberation face or an installed font (Chinese, Japanese, Korean, single emoji); Bates numbers up to 18446744073709551615, with up to 20 digits (more is refused, changing nothing). [Details](../headers-footers-watermarks.md) |
| Watermarks | Done | Text or a PNG, JPEG or TIFF picture; centred, with size, opacity, rotation, above or behind. Shown on the pages and thumbnails as you set them: once a setting settles, as adding it would make the pages, behind the content when set behind it (ENH-23); drawn over the pages while a setting changes. Added with Add to pages, then updated in place. [Details](../headers-footers-watermarks.md#add-a-watermark) |
| Document properties | Done | Title, author, subject, keywords, creator, producer; each kept as you type it (Enter or leaving the field), one undo step each. Properties, bookmark titles and comments another program wrote in UTF-8 without saying so read as written. [Details](../gui.md#document-properties) |
| Password security | Done | All four ciphers and permission flags. [Things to know](../security.md) |
| Opening protected files | Done | With the open (user) password, typed and Enter or Open; several opened at once ask in turn, a wrong password asks again first, and Cancel leaves only that file unopened; files with an empty one (only a permissions password) open without asking, including RC4 files with a shortened password entry that other readers accept, and their notice says so. [Details](../security.md#open-a-protected-document) |
| Content editing | Partial | New text is typed on the page where you click, in the font, size and colour it will have, several lines added as one undo step. The font list offers Helvetica, Times and Courier and the installed fonts that allow embedding, searchable, each name in its own font; fonts installed under code names (the URW base-35 fonts, their older gsfonts names too) are listed by the typeface they stand in for, their own name after ("Palatino (P052)"), found by either, with a tooltip saying they are metric-compatible stand-ins, embedded. Double-click a line with the read tool, or click it with the Text tool, to edit it on the page, drawn in the fonts, sizes and colours it will have over the line; select words and press Ctrl+B or Ctrl+I, or pick a font, size or colour, to change those words only (the word at the caret, or what is typed next, with nothing selected), with Ctrl+Z and Ctrl+Y for typing and styles alike, and the same for new text. Words selected with the read tool, across lines and pages, take Ctrl+B and Ctrl+I (Word's rule over the whole selection), Bold and Italic on the floating toolbar and the right-click menu, and the font, size and colour in Edit > Font in the command band (blank where they mix values; the inspector, headed Selected text, shows their font and size), each change one undo step, the words staying selected; invisible recognised text, text in a form XObject and vertical writing are left as they were, with a notice. Selected words are also typed over as in a word processor (within one line: their line opens with the typing in their place, Ctrl+V and Paste on the floating toolbar and the right-click menu too; across lines nothing is typed and a note says paragraph editing is planned), deleted with Delete or Backspace across lines and pages as one undo step (a space beside them going by Word's rule; lines aren't joined, an emptied line goes) and cut with Ctrl+X, the band's Cut, the menus and the floating toolbar; Edit text with words selected opens their line with them selected (across lines, the line where the selection ends). Lines selected with the arrow are cut too. An edited line keeps its own font, embedded subsets and Identity CID fonts included (ENH-31): letters left as they were keep their codes and the spacing between them, new letters take the font's own code for them where it verifiably has one, and only a word with a letter it lacks goes to the installed family or nearest standard font (and on, a word that lacks too, as new text falls back); a line in one style changed in words alone is changed in place as one show; while typing, the line is drawn in the font's own glyphs at the PDF's widths, and the inspector names the words the font lacks and the font they use, or that the font can't take new text (Type 3, not embedded, vertical writing, unreadable codes), why on hover; Ctrl+B and Ctrl+I also change a line selected with the arrow. A colour picked for text in several colours makes it all that colour, the first one too, and the picker offers the text's own colours. Colour pickers show a colour as the page does and write the colour they show. A line with a bold word or a superscript in it is one object; pictures, shapes and comments are selected, moved and resized with the read tool too, page text after Move or resize text on its toolbar (the read tool stays on) or with the arrow; Esc during a move, resize, turn, shape or pen stroke cancels it. Shift+click selects several objects of a page with the read tool (pictures and shapes) and with the arrow (any object), and with the arrow so do a rectangle (what is wholly inside it; Shift adds) and Ctrl+A, each outlined and a dashed box round them all; they move together (a drag, the arrow keys, the inspector's X and Y), restyle together (the lines' font, size, colour, Bold and Italic, the shapes' fill, outline and width, blank or mixed where they differ) and delete together, each change one undo step, and stay selected as the page changes; new text of several lines is selected as one; a drag that starts on a background not selected draws a rectangle. With both tools a press held still is a click however long it is held; it starts a drag only once the pointer moves. A hairline thinner than half a point moves too. The look of the text being typed, a line selected or edited, selected words and several lines is set in Edit > Font in the command band: the font list (its tooltip names the PDF's fonts of the selected characters) and size, then Bold, Italic and colour (a bold B, a slanted I with serifs and an A over a bar of the colour, as word processors show them); a click there leaves the typing open and gives it the keyboard back, and each change is kept as one undo step whether or not the inspector is shown; Esc in a box there (the size, say) drops what was typed in it and goes back to the typing. While a line is edited, the floating toolbar's Bold and Italic change what is selected in it, as Ctrl+B and Ctrl+I, and its Cut, Copy and Paste act on it, as the right-click menu of the text being typed does (Cut, Copy, Paste, Bold, Italic; a right-click keeps the selection); in a family without the face, Ctrl+B or Ctrl+I says so at the caret. Text typed on the page is outlined in the accent with a white or black line inside, whichever contrasts more, so it shows on any page colour (ENH-22); the outline turns the caution colour, with a tag under it, when the text runs past the page's edge ("Past the page edge") or, in a line, on past its end into other text ("Over other text"). The status bar says "Editing one line" meanwhile. Ctrl+V with nothing being typed and no words selected says where to click. A line clicked with the Text tool shows its own look in Edit > Font and the inspector. Esc while new text is dragged by its outline puts it back. With nothing to style, they show the next new text's look, greyed. Paste, Cut and Copy in the band act on the text being typed. The inspector is headed by what it shows (Selected text, Image, New rectangle, Comment); for text it says how it is edited (One line, Selected words, New text), its font and size (Mixed where they differ) and, for a line being edited, the selection's fonts as the PDF names them without a subset's tag (PDF font) and how what is typed will be written, following the typing (Source font: Preserved, Installed family, Nearest standard, Partly preserved, Mixed or Font chosen), then for a line or words the planned text tools folded (Future text tools, marked Planned), with no text box: the words are edited on the page, with the arrow too (a double-click, or Edit > Edit text, which with no line selected chooses the text tool). Every object's position and size are in a Position and size section that starts closed and stays as left. The inspector's numbers show the object's values as they are, even outside what the field lets you set (a hairline outline as 0 pt, a rule under 1 pt tall), so selecting something never changes it; a value you set is kept within the field's range. Finishing a line edited with the read or Text tool leaves nothing selected; with the arrow the line stays selected. Changes in the inspector apply at once (the Text and Comment boxes' when you leave them, by Tab too), each an undo step; typing into one line again soon joins one step. Works on lines, not paragraphs; words with letters a line's font lacks, and lines in fonts that can't take new text, are written in another font; the text after a deleted, edited or moved line stays where it was (except in vertical writing). Every kind of new text is set through one pipeline (NEW-02, batch 45's stage 1): each word goes to the first font that has it (the font asked for, an installed font for the script, the font's Liberation stand-in, any installed family; Chinese and Japanese a character at a time, by the document's language, the text's kana or Hangul, or the system's), is shaped (kerning, ligatures, marks; Helvetica, Times and Courier kerned with Liberation's pairs) and is broken into lines by Unicode's rule; the inspector names the words in another font ("Not in Calibri: 漢字. Those characters use Noto Sans CJK JP.") for a line, new text, a text box and a form field, and the typing box draws each word in the face it is written in, within half a pixel at 2 pixels a point, text boxes and fields of several lines wrapping on the lines written. The caret, Delete and Backspace go by grapheme clusters (Backspace takes a separately typed accent alone), words by Unicode's word rule; input methods' compositions are filtered once committed, a composition left by a click, another tool or a band button taken as committed (PB-54), and an input method types over read-tool words. [Details](../content-editing.md) |
| Importing images as pages | Done | PNG, JPEG, TIFF. [Details](../scanning-and-ocr.md#importing-images) |
| Optimising scanned pages | Partial | Pages that are one image; only the pages chosen change, also when they share a picture with others. [Details](../scanning-and-ocr.md#optimising-scanned-pages) |
| OCR | Partial | Latin script; built-in recognition model. [Details](../scanning-and-ocr.md#recognising-text-ocr) |
| Export pages as images, export text | Done | Pages as images from the selected pages, or all. [Details](../export-and-printing.md) |
| Printing | Partial | No paper size or scaling options. The Print dialog opens at once and fills in the printers when found (cancellable, with Try again when the printing service doesn't answer within 10 seconds or there are no printers); on Linux and macOS a job CUPS doesn't take or confirm within 2 minutes is stopped and reported; on Windows, LimarePDF stops waiting for a job the spooler takes no step on for 2 minutes and says so (the job can't be stopped there, and may still print). Form values that would come out empty (no drawing, and text LimarePDF can't draw): the app asks first, the command line refuses and names the fields. Tried on paper on Linux (HP LaserJet, CUPS): pages, mixed sizes, a turned page, comments and form values print correctly. On Windows only the print dialog has been tried, and nothing has been printed on macOS. [Details](../export-and-printing.md#printing) |
| Driving scanners | No | Not planned: scan with your system's tool and import the images |
| Export to Word, Excel, CSV | No | |
| Annotations and comments | Partial | Shown (LimarePDF draws the common types when the file has no drawing; up to 10,000 per page, with a notice on a page that has more, whose others are still removed by Remove comments and redaction), listed in the Comments view as cards under their pages' labels with replies under them (batch 42, ENH-40: author, kind, date (in this computer's time zone when the file says the date's own) and first lines, or a markup's marked words; only the cards in view drawn; a search box, a count and Filter ▾ by type, author and this page, per tab and never in the file; a card selects its comment and shows its mark, a mark selected on the page selects its card and scrolls to it; one Tab stop: Up, Down, Home, End, Enter to the reply box, Delete, Esc to the page; Review > Previous and Next comment; a reply left unfinished kept per comment as a draft, its card saying Draft reply, added by Save, Print and the commands that use the document, asked about on close, left alone by Undo and Redo; Ctrl+Enter adds a reply), deleted, flattened; `limarepdf-cli annot list\|remove\|flatten`. Highlight, underline, strike out and squiggly-underline selected text; add sticky notes and text boxes (Helvetica, words it lacks in other fonts as new text; wrapped while typed on the lines written), rectangles, ellipses, lines with an optional arrowhead, freehand pen drawings and stamps; reply to any annotation; select any annotation to change its colour, opacity and comment (and a shape's width, fill and arrowhead), and move or resize it with the read tool or the arrow; a note or text box, new or already on the page, open for typing, stays open while the inspector changes it and gets the keyboard back (a new text box showing the size picked), its text typed on the page only (the inspector's Comment or Text box hidden meanwhile; ENH-13); drag a line's ends to turn it, and turn freehand drawings with their rotate handle. Not yet: turning rectangles, ellipses and text boxes (see below), polygons, measuring, attachments, XFDF. [Details](../comments-and-markup.md) |
| Stamps | Done | Approved, Not approved, Draft, Final, Confidential, For comment and Void, or your own wording in green, red or blue (the characters new text takes), optionally with the author's name and the date, or a PNG or JPEG picture; click to place or drag to size, then move and resize; `limarepdf-cli stamp`. Not yet: stamps made from selected content, or wording in right-to-left or shaped scripts. [Details](../headers-footers-watermarks.md#stamps) |
| Fill and sign | Done | A signature or initials drawn, typed in one of four handwriting styles or from an image (paper made transparent, a small signature found on a whole sheet; drag around it, or use the arrow keys, to use part of the picture; `limarepdf-cli sign --image --crop` on the command line), up to five of each saved for next time; the date in three styles, your name, text and tick, cross and dot marks, in black or blue ink, picked from the Fill and sign button's menu or pane; placed anywhere or in an empty signature field (one at a time, or several with the tool kept active), movable until Finish signing draws them into the pages. The pane (batch 42, ENH-09) follows the task from top to bottom: how many items are put down and that they are a picture, not a digital signature; Signature and Initials, each with the saved ones (a bin button beside each, or a right-click, takes one off the list) and one Create button; Text and marks (the date, your name, text, and a tick, cross and dot drawn with the icon font: they were boxes before, as Inter has no ✔ or ✖); the ink; and Finish signing…, the main button once something is down, greyed until then saying why (so is the command, in Fill and sign ▾ at its end, ☰ > Document and the palette). The Keep tool active pin is in the inspector's heading, beside its other buttons, for every tool that puts things down. Typed signatures from the command line: `limarepdf-cli stamp --signature`. Not a digital signature; text takes what text boxes take, typed signatures what their handwriting has (Caveat Latin and Cyrillic, the others Latin); the date is your computer's. [Details](../fill-and-sign.md) |
| Measuring | No | |
| Forms | Partial | Fill in text fields, checkboxes, radio buttons and lists in the app (Tab between fields, fields highlighted unless turned off, reset, flatten) and from the command line (`limarepdf-cli form`, values as JSON, flatten all or some pages). Text in the characters new text takes (Chinese, Japanese, Korean and emoji in an installed font), drawn in an embedded font when the field's own can't show it (the field keeps naming its own font for other apps), the inspector naming words in another font; fields of several lines wrap while typed on the lines written (a centred or right-aligned field shows its text from the left until kept, PB-59); a value filled in elsewhere in right-to-left or shaped scripts shows only if that app drew it; flattening refuses, naming the fields, rather than lose one it can't draw; JavaScript doesn't run; creating fields: No; signature fields take a fill-and-sign signature, or a digital signature (Document > Digital signatures > Sign with digital ID, or `limarepdf-cli sign --field`). [Details](../forms.md) |
| XFA-only forms (made for Adobe Reader) | No | Recognised and explained in a notice and by `limarepdf-cli form`; showing or filling them is not planned (see below). Hybrid forms are filled through their ordinary fields |
| Form data import/export (FDF, XFDF), barcodes, JavaScript | No | |
| Viewer preferences | Done | The layout, side panel (Pages or Bookmarks) and page a document opens with: followed when opening, set in the inspector's Opening view (each choice applies as you make it) or with `limarepdf-cli view-prefs`. Full screen and hiding the toolbar or menu: No. [Details](../cli.md#viewer-preferences) |
| Page labels | Done | Numbering such as i, ii, 1, 2 or A-1: shown in the status bar, thumbnails and bookmark tips; set from the selected pages in the inspector, applying as you set them, or with `limarepdf-cli labels`; kept with their pages when pages move. Going to a page by its label: No. [Details](../cli.md#page-labels) |
| Bookmarks | Partial | Shown in the Bookmarks view (also from the command palette) and followed; added, renamed and deleted in that view or with `limarepdf-cli bookmarks`. Moving or reordering them, styles and colours: No. [Details](../reading-and-finding.md#bookmarks), [CLI](../cli.md#bookmarks) |
| Links | Partial | Followed: to pages, and to web and email addresses after asking. Links to other PDF files, and adding or editing links: No. [Details](../reading-and-finding.md#links) |
| Layers, attachments | No | |
| Supported systems | Partial | Windows 10 and 11 (x64), macOS 11 or later (Apple Silicon and Intel), 64-bit Linux with glibc 2.39 or later. All provisional: built and tested automatically on each, but the release checks by hand haven't all passed on each yet. [Requirements](../install.md#requirements) |
| Installers and packages | Done | `.deb` and `.rpm` packages with apt and dnf repositories, a macOS `.dmg`, a Windows MSI (with a desktop shortcut if you tick it), and archives; GPG-signed. Each carries the licences and a README linking to the guide online for its version; the archives carry the guide itself. [Install](../install.md); more package managers planned, see below |
| Redaction | Done | Mark areas, selected text or search results (standard redaction marks), check them (batch 42, PF-37: a selected mark moves by a drag anywhere in its outline or the arrow keys, its areas together, one undo step each; a mark of one area also resizes by its handles, its area kept a point inside its box; a mark over several lines has no handles; Apply removes what is under each mark where it then is; Apply redactions… is greyed, saying why, while there are no marks), then apply: text, drawings, comments and links under them (every one, also past the 10,000 a page shows) are removed for good, pictures (scans included) blacked out there, then checked; a character goes when a quarter of it is under the marks, overlapping marks counted once. Text whose characters can't be placed for certain (some Chinese, Japanese and Korean fonts) goes a whole piece at a time. Refuses, changing nothing, when a form field under a mark or the tags' index is nested more than 32 deep, or the tags' index is too large to search. In the app and with `limarepdf-cli redact`. [Details](../redaction.md) |
| Removing hidden information | Done | Metadata (including XMP), JavaScript and automatic actions (also those following an ordinary link or action), attachments, hidden layers and their content (in nested drawings, comments' drawings and patterns too), thumbnails and private data; if chosen, comments, form values (cleared for good, defaults and read-only fields included), bookmarks and what lies outside the crop box. What shows stays as it was: settings made in a hidden layer (colour, position, clip) are kept, hidden text leaves a gap as wide. Refuses, changing nothing, when it can't look into everything a hidden layer could be in, damaged compressed data included, or a drawing shared by places that give its layers (or its fonts) different meanings, or hidden text that clips or whose width isn't known with visible text after it on the line, or when form values are chosen and the form is nested more than 32 deep. Document > Remove hidden information… and `limarepdf-cli sanitise`. [Details](../redaction.md#removing-hidden-information) |
| Digital signatures | Done | Checking in the app (a coloured banner, the Digital signatures view, Details, View signed version, trusting signers and authorities, Trusted certificates) and from the command line (`limarepdf-cli verify`): unchanged or not, signer's certificate, signing time and timestamps, certification and locked fields, warnings; RSA and ECDSA P-256/P-384, holding RSA keys to the RSA-PSS limits their certificates set; refuses the known wrapping and copying attacks; lists the changes made after each signature and judges them by Acrobat's rules (stricter when unsure), catching the shadow and incremental-saving attacks, and won't vouch for or sign as it is a file PDF apps read differently; saves the version a signature signed; trusting signers and certificate authorities (`limarepdf-cli trust`), with chains checked by RFC 5280's rules at the signing time, in LimarePDF only. Digital IDs in the app (Document > Digital signatures > Digital IDs) and from the command line (`limarepdf-cli id`): make a self-signed one (RSA 3072-bit, five years, like Acrobat's), import `.p12`/`.pfx` files from Acrobat, Windows, macOS or a certificate authority (old and new encryptions, an empty password too; not IDs allowed only RSA-PSS signatures), export an ID or its certificate. Signing and certifying in the app (Document > Digital signatures > Sign with digital ID, Certify; edits made since opening are signed with it, asking first when earlier signatures wouldn't allow them) and from the command line (`limarepdf-cli sign`): in an empty signature field or a new box, as Acrobat signs (`adbe.pkcs7.detached`, SHA-256), with a visible look (your name in handwriting or a picture, "Digitally signed by" and the date), added after the file's bytes so earlier signatures stay valid; follows certifications and field locks; every signed file checked before it is written. Saving a signed document (Save in the app, every editing command) adds the changes after what was signed, so the signatures stay valid; asks first (the command line refuses without a flag) when they don't allow the changes, and before writing the whole file when it must (redactions, removed hidden information, passwords, damaged files). Not yet: timestamps when signing, encrypted documents (or ones set to get a password when saved); only partly tested with Adobe Acrobat (see below). [Details](../signatures.md) |
| Certificate encryption | No | |
| PDF/A, optimised saving | No | Saved files are pruned and compressed |
| Comparing documents | No | |
| Crash recovery | Done | Private copies of documents with unsaved changes, about a minute behind; when LimarePDF stops without closing normally, the next start offers them, and a restored one opens as a new unsaved document. Your own files are never written by it. None for password-protected documents, or documents a protected one's pages are appended to; a redaction, removal of hidden information or added password, or a redo of a redaction or removal, deletes the earlier copies first, including the one a restored document came from. A damaged copy is listed as such and never stops the app starting. Can be turned off. [Details](../gui.md#recovery-copies) |
| Sessions, autosave, backups, preferences, custom shortcuts, translations | No | Closing or quitting with unsaved work, text still being typed included, offers Save, Discard or Cancel ([details](../gui.md#opening-files)), and Save, Print and every command that changes the document keep that text first ([details](../gui.md#undo)); unsaved changes are kept in [recovery copies](../gui.md#recovery-copies) |
| XFA forms, XPS documents | No | |

## Known limits

- Text with characters the standard fonts lack (beyond Western European) is written in Liberation Sans, Serif or
  Mono, which have the same widths as Helvetica, Times and Courier, embedded in the file and cut down to the
  characters used (a few KB). One font per face grows as text is added; in a signed document, each save adds
  fonts of its own instead, so the signed part stays as it was. Text in an installed font is always embedded
  the same way (OpenType CFF fonts as such; a variable font at the weight chosen). A TrueType face embedded since
  batch 45 numbers its own codes, so a ligature reads back as its letters and a glyph two letters share (Latin A
  and Cyrillic А) as each; a CFF face, and faces LimarePDF 0.11 wrote, keep a code for each glyph and grow as they
  were, a glyph wanted for a second text going to another font object of the face.
- Interface text that Inter lacks (Chinese, Japanese, Korean, emoji) is drawn in the installed faces LimarePDF
  writes those in, read at start on another thread (ENH-33); until they are read, such text shows as boxes. egui
  shapes interface and typed text itself (harfrust 0.12, no language), so right-to-left and joined scripts show
  unjoined and left to right, where a font has them.
- Headers, footers and watermarks are added to the page content. They are not annotations, and no command removes
  one from all its pages later: Undo takes it back until the document is closed (also after saving), and on each
  page its text and pictures can be edited or deleted like other page content. The one just added can be changed
  until anything else changes.
- While a watermark or headers and footers are being set, the pages and their thumbnails are drawn from a copy of
  the document with them put on, once the settings settle (ENH-23). While a setting changes, and until that
  drawing comes, they are drawn over the content, also when set to go behind it, in Liberation Sans, which has
  Helvetica's widths. The copy is made on the window's thread once per settled change (a large document's copy
  can take a moment there), and the pages beside those in view aren't drawn ahead.
- Saving writes the file on a thread of its own, with a busy sign in the status bar, while the window carries on
  (since batch 37, ENH-35): the document can be changed meanwhile, and the tab then stays unsaved; saving it again,
  closing it and quitting wait for the save, and Reload and signing are off until it ends. While it writes, the
  app holds a second copy of the document in memory. The save's questions and checks still come first, on the
  window's thread (on 300 pages about 20 ms, a signed document's added changes about 30 ms).
- Undo keeps up to 100 steps within 1 GB per document, each the size of what its change changed, so a few steps
  are larger than a copy of the document would be: the first page change on a long document (moving, deleting,
  turning or adding pages) changes every page's entry and keeps 28 MB on 10,000 pages, and a watermark on 10,000
  pages keeps 32 MB. Each change copies the document's contents first, to compare after: for a moment about twice
  the document's memory (about 16 ms on a 100-page scan, 21 to 24 ms on 10,000 pages; Undo and Redo themselves
  take under 13 ms there).
- Every change, and Undo and Redo, runs on the window's thread, and the pages are then drawn again from a copy of
  the whole document written out (42 to 101 ms on 10,000 pages; after every Undo and Redo, and after every change
  that shows on the pages, not the properties, say). Moving and deleting pages take 7 to 29 ms on 300 pages (their
  Undo and Redo 3 to 8 ms), but 0.34 to 0.45 s on 10,000, their Undo and Redo 0.10 to 0.26 s (PB-37; since batch
  43 Undo and Redo are 46 to 63 ms quicker there, the move and Delete themselves 24 and 48 ms slower). Opening such
  a document and its Organise grid stay quick (about 0.1 s to read it; the grid's keys and scrolling a frame or
  two).
- Saving rewrites the whole file, except for digitally signed documents, whose changes are added after what was
  signed so the signatures stay valid ([details](../signatures.md#saving-a-signed-document)). It writes a temporary file
  beside the document, readable only by you (on Linux and macOS) when it replaces a file, then puts it in the
  document's place, so a failed save leaves the original as it was, permissions included. On Linux and macOS the
  save is then confirmed on disk, so a power cut can't undo it; when the system can't confirm it, a message says so
  and the document stays unsaved ([details](../gui.md#saving)). Settings, recent files and saved
  signatures are written the same way. When they can't be written (a settings folder you can't write to, say),
  a message says so once a session, and what you changed works until LimarePDF closes; a signature saved for next
  time that can't be is said each time, and can still be used. Digital IDs and the trust list say when a change
  was made but your system couldn't confirm it is safely on disk, and an ID made or imported that couldn't be made
  the default is there all the same, with a warning ([details](../troubleshooting.md#settings-not-saved)).
- Pages the renderer cannot draw say so, with the reason, and a notice says when a picture couldn't be drawn
  ([details](../rendering.md#when-something-cant-be-drawn)); editing may still work. Another notice names fonts that
  aren't in the file or installed and says what to install.
- A damaged file is repaired to open it, with a notice; the original is only replaced if you choose to when saving
  ([details](../rendering.md#damaged-files)). Stream data is skipped by its stated length, so an attached PDF or text
  crafted to look like objects isn't taken for the file's own content.
- A file other PDF apps may show differently (parts it deletes still in use, data after its end, entries pointing
  at the wrong object) is shown as the PDF standard says, with a notice and a Save as… button that writes a copy
  every app reads the same; the command line prints a note
  ([details](../rendering.md#files-other-apps-may-show-differently)).
- A malformed file that makes LimarePDF fail while opening, editing or saving gets an error message, and a failed edit
  leaves the document as it was. A failure while showing the document (pages, thumbnails, search, bookmarks,
  comments, the inspector) turns that tab into an error page with Save a copy and Close; other tabs carry on. So does
  a menu command, shortcut or panel button that fails: an error message when it changed nothing, else that tab's
  error page. Redactions and removed hidden information are recorded with the change itself, so a later save of a
  signed document always writes the file out in full. `limarepdf-cli` prints such a failure as a plain error and
  exits with status 1.
- Sizes a file states are capped before memory is set aside for them, so a small crafted file can't make LimarePDF
  ask for gigabytes and close: a page image has at most 268 million pixels (A3 at 1152 dpi;
  [export](../export-and-printing.md#pages-as-images)), Optimise scans skips pictures of more than 268 million pixels
  with the reason, redaction removes a fax picture of more than that whole, as it does any picture it can't read,
  and a font's character widths are read for 65,536 characters at most. A font's map from characters to text
  (ToUnicode) is read for about a million characters at most, up to 32 MB, with up to 512 bytes of text for one
  character, as the PDF specification allows ([details](../reading-and-finding.md#how-text-is-read)). Pages of any real
  size show at every zoom. Working out the reading order of a page with tens of thousands of separate lines can't
  run out of stack (which closes a program outright, with no chance to catch it). Looking a name up in a document's
  list of named destinations (for a bookmark, link or the page to open at) reads each part of the list once and at
  most 100,000 parts, so a crafted list can't keep the app busy for hours; a name it gives up on is a destination
  that can't be followed.
- `limarepdf-cli` never changes a command's input: an output naming the input, by any path or link to it, is
  refused with nothing written, and the result is written whole or not at all. When
  the command's own output can't be written (a full disk), it stops with an error and status 1, saying whether its
  file was already saved; a reader that stops early (`… | head`) stops it quietly ([details](../cli.md)).
- Text from a file that the app shows as a label or message (link addresses, bookmark names, comment authors and
  text, file names in tab and window titles and print job titles) has direction-changing (bidi), invisible and
  control characters removed and is shortened when very long, so a link can't pretend to go somewhere else. Form
  field values are shown as they are. The command line's listings (`info`, `bookmarks list`, `annot list`, `form
  list`, `labels list`, `view-prefs list`, `objects`, `search`, `verify`) clean such text the same way, without
  shortening it, so a file can't send escape codes to the terminal; `--json` output stays exact.
- On Linux in a Wayland session the window's frames go to the screen without waiting for it, the compositor
  setting the pace of drawing (BUG-42): waiting could hang LimarePDF, and KDE Plasma 6.8 with it, with NVIDIA's
  driver once the screen had turned off. On X11, Windows and macOS each frame waits for the display, as before.

## Planned and proposed

Ideas for what comes next. None of these are available yet, and the list is not a commitment or an order. A wider
pool of features other PDF tools have is in [ideas.md](ideas.md).

### Reliability

- **More from recovery copies:** copies of password-protected documents, kept encrypted; a recovered copy of a
  signed document saved by adding to the original, when the original is unchanged, so its signatures stay; reopening
  the last session's documents.
- **Acceptance checks** of everyday workflows by hand, in other readers and on Windows and macOS, as listed in
  [Testing](testing.md#acceptance-checks). They have been run on Linux.
- **Very complex pages draw slowly:** one test page with 73,000 drawn paths and 84 shaded areas takes 54 s to draw
  at 150 dpi, where poppler takes 9 s ([Performance](performance.md)). Its memory is now bounded by drawing in bands
  ([Rendering](../rendering.md#large-drawings)).

### Rendering

Done for now (see the Rendering row above). Left for a later round:

- **hayro's own gaps** (knockout and non-isolated transparency groups, thin-line snapping, some shadings, CMaps
  stored in the file and built on another one, and installed fonts for plain fonts that aren't embedded, which
  hayro doesn't ask for yet), listed in [Known to be wrong](../rendering.md#known-to-be-wrong). They are documented here and in the build plan,
  and not reported to hayro. The transparency groups can't be fixed in LimarePDF for now: hayro doesn't tell it
  which groups are knockout or non-isolated, and none of the test files looked wrong because of them.
- **A very large memory request:** on one test file hayro asks for 7.6 GB at once to fill a single shape at 96 dpi.
  On Linux it draws, since the memory is never used; on Windows, which doesn't promise memory it hasn't got, it
  could stop the app. Not tried on Windows yet; the check is in
  [Testing](testing.md#platform-smoke-checks).
- **Notices for what hayro draws wrongly:** plain fonts that aren't embedded (such as Calibri or Wingdings, drawn
  with a standard font). Offered, not yet decided. Transparency groups get no notice: it would show on files that
  look right.
- **A check on a HiDPI screen** (scale factor 2): tile sizes, the 4096-pixel cap and sharpness have only been tried
  at scale 1.
- **Damaged files LimarePDF still can't open:** show them read-only as hayro reads them, when the repair finds no
  pages. No file in the test set needs it yet.
- **Annotation drawings:** line captions, a caret's paragraph symbol, rich text in text boxes, and Acrobat's artwork
  for standard stamps.

### Printing

- **Print preview and page setup:** a preview beside the Print settings, and paper size, orientation, scaling and
  pages per sheet, for the app and `limarepdf-cli print`. It needs the printer's paper sizes and margins, and one
  layout of pages on sheets shared by the preview and what is printed, so the preview can't differ from the
  printout. Designed separately; today's Print dialog shows only what works now.

### XFA forms

Many government forms are XFA-only: the PDF holds a "Please wait..." page, and the real form is XML that only Adobe
software lays out. LimarePDF recognises them and says so. Beyond that:

- **Possible later: their data from the command line.** Export an XFA form's values as JSON and fill them from
  JSON, to view, sign or print in Adobe Reader. Useful for batches; it needs an XML parser crate.
- **Not planned: showing and filling them in LimarePDF.** It would need an XFA layout engine (the specification runs to
  over 1,500 pages) and a script engine for their JavaScript and FormCalc, months of work for a format Adobe has
  deprecated and PDF 2.0 removed.

### Turning annotation boxes

Deferred until the core features are complete. PDF stores rectangle, ellipse and text box annotations as upright
boxes, with no angle, so turning them needs a workaround:

- **Text boxes in quarter turns:** the `/Rotate` entry Acrobat writes (LimarePDF already uses it on turned pages). Other
  readers understand it.
- **Rectangles and ellipses at any angle:** saved as polygons, which every reader shows but lists as polygons
  (an ellipse becomes many short sides). A turned drawing inside an upright box is the other way, but readers that
  redraw the shape show it upright again, so it is not planned.

### Digital signatures

Checking, trust, digital IDs, signing, certifying and saving signed documents are done (see the table and
[Digital signatures](../signatures.md)). Still to come:

- **Round trips with Adobe Acrobat**, on a tester's computer: Acrobat-signed and certified files checked in LimarePDF,
  LimarePDF's signatures and exported digital IDs used in Acrobat, and signed files saved with changes going each way.
  The compatibility list in [Digital signatures in depth](../signatures-in-depth.md#compatibility) names only what has been tested.

Not planned for now: timestamps from a timestamp server when signing (the signing time comes from your computer's
clock) and long-term validation, revocation checks, Adobe's and the EU's lists of trusted authorities, smart cards and system
keystores, signing encrypted files (see [ideas](ideas.md)).

### Packaging

Since 0.5: every release file and `SHA256SUMS` are GPG-signed with LimarePDF's own key, and each file has a build
attestation (`gh attestation verify`); a dry run of the release job checks this without releasing. Each release has
Linux `.deb` and `.rpm` packages (the rpm signed), signed apt and dnf repositories on the project's GitHub Pages
(rebuilt when a release is published), a universal macOS `LimarePDF.app` in a `.dmg`, and a Windows MSI installer that
offers a desktop shortcut (also for installs without its screens: `DESKTOPSHORTCUT=1`). The packages carry every
licence, the security notes and a README linking to the guide online for their version, each checked by CI as
installed. The archives stay, with the guide inside. Support for every system is provisional for now
([requirements](../install.md#requirements)). A release waits for the check of every library against known security advisories, and is
built from exactly the versions in `Cargo.lock`; security problems are reported privately, as
[SECURITY.md](../../SECURITY.md) describes.

- **Untested:** opening PDFs from Finder (Open With, a double-click, the Dock icon) on a Mac. The app now opens
  the files Finder sends, at start and while running, but this is only built and tested on Linux and by CI so far.
- **Next:** an AUR package, a Homebrew cask in LimarePDF's own tap, and winget and Scoop manifests. Maybe Flatpak.
- **Code signing** (macOS notarization, Windows Authenticode) needs certificates LimarePDF doesn't have yet, so those builds
  stay unsigned; the GPG signatures and attestations cover them meanwhile.

### Features that would set LimarePDF apart

- **Offline AI.** Summaries, questions and answers, and meaning-based search, run by a model on your own computer, so
  documents never leave it.
- **Comparing documents.** Text and visual differences between two PDFs, also from the command line, so a comparison
  can run in a build or review pipeline.
- **Recipes for batches.** Describe a sequence of steps in a file (for example: recognise text, watermark,
  Bates-number, encrypt) and run it on many documents from the command line.
- **Better OCR.** Scripts beyond Latin; reading the images on pages that also have text (today OCR only reads pages
  with no text at all, so a scanned figure or a pasted screenshot in a normal document is skipped); and matching the
  original fonts when scanned text is edited.
- **Editing paragraphs.** Edit text as paragraphs that rewrap in their frame, not line by line: a double-click in a
  block LimarePDF finds with confidence edits it as a paragraph, a single line or a block it isn't sure of edits a
  line as today, and a switch turns either into the other ("Edit as paragraph"). It becomes the default only if
  LimarePDF's check of how well it finds paragraphs passes; otherwise the switch comes first. One undo step.
  The core half is built (batch 45's stage 2): finding paragraphs, reading one for editing (its line-end hyphens
  by the document's own words), rewrapping it as you type and writing it in place, each letter in its own code;
  none of it is in the app yet (stage 3). The finding check doesn't pass yet (testing.md, "Finding paragraphs"),
  so the switch will come first. Known limits of the writer (testing.md, "Editing paragraphs"): marked content's
  alternative text (`ActualText`) isn't read, so a letter marked as another (a small capital) reads as its glyph's
  own and loses its mark when its line is rewritten (PB-71); a letter drawn as its base and an accent reads to
  other apps with the accent first once its line is rewritten (PB-73).
- **Incremental saving.** Save only what changed, which is faster for large files. Signed documents are already
  saved this way (above); this item is about using it for every document.
- **Standards output.** PDF/A for archiving, and PDF/UA tagging for accessibility.
- **Revocation checked offline.** Use the revocation answers a signed document carries to tell whether a signer's
  certificate was withdrawn, without going online.

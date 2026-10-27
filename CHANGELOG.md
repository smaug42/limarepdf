# Changelog

## [Unreleased]

### Added

- An optional timing log, for measuring how quickly LimarePDF responds: start it with the environment variable
  `LIMAREPDF_TIMING` set to a file, and it writes there when the app starts, takes input, finishes each frame,
  first shows each page sharp, and starts and ends reading, drawing, searching and saving. Off unless asked for.
- Search finds a phrase that runs from the bottom of one page to the top of the next, and a word hyphenated at the
  end of a page, highlighting it on both pages.
- View > Use the document's page labels: turned off, pages are numbered 1, 2, 3 from the first page in the
  thumbnails, search results and new bookmarks, instead of by the document's own numbering (such as iii or A-1).
  The status bar shows both. On unless turned off, and remembered.

### Changed

- Search as you type: the matches come as soon as you pause typing, with no need to press Enter, and the window
  stays responsive while a long document is searched, the matches coming in page by page. The view goes to the
  first match on or after the page you are reading. Enter and Shift+Enter go to the next and previous match.
  Changing the document brings the results up to date, keeping the match you were on. Searching again for
  something else on the same document is much quicker, as each page is read only once. The Find button in Search
  results is gone, as typing searches.
- Search results are easier to read: the matches are listed under a heading for each page, each with whole words
  around it, cut with "…", and the match in bold; the count reads "1 match", "12 matches" or "No matches".
- `limarepdf-cli search` prints each match as `page 3: …the words around it…` and the count as "1 match" or
  "N matches" ("No matches"), and `--boxes` as `page 3 [x0 y0 x1 y1]`.
- Mark all for redaction marks every match of the search, even while it is still running.
- Save writes the file in the background, with a busy sign in the status bar, so the window no longer freezes
  while a large document is saved; you can go on working, and a change made meanwhile leaves the document marked
  unsaved. Saving again, closing the document or quitting waits for the save to finish. Saving a long document
  also starts sooner, as LimarePDF no longer reads the whole file again at every check before writing.
- Changes to long documents are quicker: after each change LimarePDF went through the whole page list once for
  every page while looking for redaction marks (about 60 ms on 300 pages, more on longer ones); now once.
- Fonts installed under code names, such as the URW fonts on Linux, are listed in the font list by the typeface
  they stand in for, their own name after: "Palatino (P052)", "New Century Schoolbook (C059)", "Helvetica
  (Nimbus Sans)". The list is sorted by those names, and searching finds either. Pointing at one says it is made to
  that typeface's letter widths but isn't the typeface itself, and that it is embedded in the file.
- The inspector puts text first: it is headed by what is selected or the tool's options (Text, Image, New
  rectangle, Comment) instead of "Edit"; for text, the words come first, then a font list across the panel, then
  Bold, Italic, size and colour in one row. Position and size are in a Position and size section that starts
  closed and stays open or closed as you left it, for every kind of object. Pointing at the font list shows the
  names of the selected text's fonts in the PDF.
- The user guide's Features page is now a short summary of what LimarePDF can do. The detailed status of each
  feature, with its limits and what doesn't work yet, moved to the developer docs as
  [Features and status](docs/developer/feature-status.md).

### Fixed

- When another app changed an open document within a split second of LimarePDF reading it, at the same size,
  LimarePDF could miss the change (most likely on Windows), so Save wouldn't ask before replacing it and no notice
  showed. A file changed so recently is now checked by its contents.

## [0.10.0] - 2026-10-26

### Fixed

- A file whose cross-reference information (which says where each object is) goes wrong only part way through
  could open with objects missing, such as a page's content or the document's title and author, and without
  saying it was damaged. LimarePDF now rebuilds it by scanning the file, with the usual note that it was repaired.
- A signature whose document was already changed in a way LimarePDF can't read named a different object each time
  it was checked, so saving a comment into such a document sometimes asked about changes the signer didn't allow
  and sometimes didn't. The reason now names the same object every time, and the save gives the same answer.
- Ctrl+Z in a form field could bring back text typed in another document's field of the same name, and a new
  note could take back the text of the note typed before it on the page, both then saved. Each document's text
  boxes now keep their typing apart, and a box's own undo starts empty each time it gets the keyboard: what was
  typed before was kept when it was left, and Undo takes it back from there.
- Ctrl+Z, Ctrl+Shift+Z and Ctrl+Y in a form field, a comment, a document property, a watermark, header or footer
  text and the inspector's other boxes acted on the whole document (taking back another field's value, say)
  instead of the typing there. They now undo and redo only the typing in the box with the keyboard, and do
  nothing there once it has nothing left to undo; the document's Undo works as before outside the boxes.
- Text still being typed was lost by a command run before clicking away: new text typed and then Rotate clockwise
  from the command palette disappeared, and Undo couldn't bring it back. Undo or Redo clicked on the toolbar while
  a new note was typed acted on the change before it, and the note was added afterwards. Removing blank pages,
  recognising text and optimising scans left out text typed before they started, and wiped text typed while they
  ran. Now every command that changes or uses the document (page commands, Undo and Redo, saving, printing,
  exporting, signing, flattening, redaction, page drags in the thumbnails, bookmarks, opening Crop) first keeps
  what is being typed (new text, a line, a note or text box, a form field, a comment or reply, the inspector's
  boxes), as clicking away would, as an undo step of its own; Undo clicked while typing takes the typing back. A
  result that comes back after typing started is discarded, saying so. Text that can't be kept (a form value
  LimarePDF can't write) stays as typed with the reason shown, and the command doesn't run; Save no longer saves
  without it.
- Print pressed straight after typing in a form field, without leaving it, refused to print ("The document
  changed while the question was open"). The field is now kept before the Print dialog opens, so the job holds
  the typed value; the same goes for the other commands that ask a question first. A field being typed in when a
  dialog opens keeps its typing, and gets the keyboard back when the dialog closes.
- The Undo button on a notice such as "Deleted page 1" undid whatever came last in the tab in front: after going
  to another document, it took back that document's edit and left the page deleted; after another change in the
  same document, it took back that change. A notice's Undo now takes back only its own change in its own
  document, and the notice goes as soon as something comes after that change. Going to another tab hides it, and
  it shows again on coming back within 30 seconds. Notices that only tell you something ("There are no redaction
  marks", say) no longer have an Undo button, and "Reset the form" no longer shows when the reset was refused.
- Closing a tab behind the one in front, with its close button, switched to another document. The tab in front
  now stays in front, and a tab with unsaved changes asks about them without coming to the front; closing the tab
  in front still moves to the next one.
- A form value LimarePDF can't write (right-to-left text, say), typed and then left by clicking elsewhere, Tab,
  Enter or Esc, was lost once the reason was shown. It now stays typed in the field, which gets the keyboard back
  when the message closes, and Tab doesn't move on to the next field.
- Split into files replaced files of the same names in the chosen folder without asking. It now asks once, saying
  how many, before writing anything, as exporting pages as images does; Cancel writes no files. On the command
  line, `limarepdf-cli split` and `limarepdf-cli render` refuse to replace files, write nothing and list them;
  add `--overwrite` to replace them.
- Page images (in the app and with `limarepdf-cli render`), certificates exported with `limarepdf-cli id export`,
  exported text and selected text saved as a text file were written in place: a failure part way (a full disk,
  say) left a cut-off file, and a file linked to another one changed that other file too. Each is now written
  whole or not at all, as documents are, and a write the system couldn't confirm on disk is reported as such.
- The same file opened twice gave two tabs that saved over each other's changes without a word: a Name saved in
  one tab went back to the old one when the other tab saved its Notes. Opening a file that is already open (by
  any path to it, a link too, or while it is still being opened) now brings its tab to the front. Save checks
  first that the file still holds what LimarePDF last read or saved there: when another app or tab changed it,
  Save asks, offering Save as… or Replace, and Cancel keeps both the file and your changes. Save as, saving
  selected text and signing refuse to write over a file that is open in another tab.
- Opening several password-protected files at once asked for only one password: the other files were dropped
  without a word. A message or another result arriving while a password was typed (a file that couldn't be
  opened, say) replaced the prompt and what was typed, and closing the window replaced any open question with
  "Quit without saving?". Now each question and message waits its turn: every file asks for its password, a wrong
  password asks again before anything else, Cancel leaves only that file unopened, files that can't be opened are
  told about in one message, and a question interrupted by closing the window comes back after Cancel. A page
  asked for on the command line is kept through the password.
- Enter in the password field didn't open the document, and Enter in Your name for comments didn't save the name;
  only clicking the button did. Enter now answers both.
- Append PDFs couldn't add a password-protected file's pages: it stopped with "Merge failed". It now asks for the
  password (again when it is wrong), with Skip this file to go on without it; Cancel, or a file that can't be read,
  adds nothing. The files are read in the background, so the window doesn't stop while large files are read.
- Print froze the window until the printing system had listed the printers, which could take as long as it
  liked when the printing service was slow or stuck, with no way to cancel. The Print dialog now opens at once,
  says it is looking for printers, and can be cancelled; the printers fill it in when found. A printing service
  that doesn't answer within 10 seconds, or can't list the printers, is reported in the dialog with Try again, as
  is having no printers set up. On Linux and macOS a print job that the printing service doesn't take, or doesn't
  confirm, within 2 minutes is stopped, saying the job may have been sent and to check the printer's queue; until
  now it could keep other background work (Append PDFs, OCR, exports) from starting until LimarePDF was closed.
- Choosing black for text in black and red (the first of its colours, which the colour picker starts from) did
  nothing. A colour picked in the picker for text in several colours now makes it all that colour, the first one
  too, in one undo step; opening and closing the picker changes nothing. The picker also offers the colours in
  the text, to make it all one of them in a click.
- Colour pickers showed a document's colours lighter than the page does, apart from black, white and full
  colours: a mid grey looked light grey in the picker, and a mid colour picked by eye was written darker than it
  looked. Every colour picker (text, comments, shapes, notes, text boxes, the pen, watermarks) now shows a colour
  as the page does and writes the colour it shows; opening one and closing it leaves the colour exactly as it was.
- Picking a font size, font or colour in the inspector while editing a line or typing new text on the page applied
  only where the caret was, not to the characters selected, and the selection was lost. The selected characters
  now change, and stay selected when the line takes the keyboard back.
- An imported digital ID with no password couldn't sign or certify in the app, which asked for a password it
  doesn't have. With the password left empty, the app now tries the ID without one; an ID that has a password
  still asks for it.
- Settings, recent files and saved signatures that couldn't be written (a settings folder you can't write to,
  say) were dropped without a word. A message now says so once a session, and what you changed works until
  LimarePDF closes; a signature saved for next time that can't be is said each time and stays usable, including
  a picture LimarePDF couldn't copy, which was refused before.
- Making or importing a digital ID whose being the default couldn't be noted was reported as failed although the
  ID was kept, so trying again made a second copy. It is now reported as made, with a warning, in the app and on
  the command line (`warning:` lines; the command succeeds). Changes to digital IDs, exports and the trust list
  that your system couldn't confirm are safely on disk are reported as done with that warning, not as failures.
  `limarepdf-cli sign` reports the signed file apart from the ID it couldn't keep as the default, instead of
  failing after writing the file; the app's Signed message says so too.
- Bullets, euro signs, dashes, curly quotes and other punctuation in text written by other programs (document
  properties, bookmarks, form field names and values, field descriptions, comments and their authors, page
  labels) showed as gaps or odd characters: "Budget • €100 — “approved”" read as "Budget 100 approved". They now
  read as written, in the app and on the command line, and edit and save as they read. Text with control codes
  is written so it reads back unchanged.
- A photo added with Add image, or put in with Replace image, ignored the photo's own orientation: a picture
  taken with the phone turned showed on its side, and a click placed it in a box of the wrong shape, squeezing
  it. Both now show it upright, a click making a box of its upright shape, as importing images as pages already
  did; a JPEG is still embedded as it is.
- Arrowheads and other endings on a dashed line were drawn dashed too, with gaps. They are now solid, the line
  itself still dashed, in the app, when flattened and in other readers.
- Removing hidden layers could change what shows: a colour, position or clip set inside a hidden layer was lost
  with it, so artwork after the layer turned black or moved, and text after hidden text on the same line moved
  back. Only what hidden layers draw is removed now; their settings stay, and hidden text leaves a gap as wide.
  A shape begun before a hidden layer and filled or outlined inside it no longer shows up after the removal,
  drawn by the next visible shape, and a clip begun before the layer still clips. Where that can't be done for
  certain (hidden text that clips, or whose width depends on where a drawing is
  used), the removal stops and changes nothing, saying where.
- Appended pages with filled form fields could show the answers blank: their fields weren't carried over, so
  answers without a drawing of their own weren't drawn. Append PDFs now asks first, then draws those files'
  fields onto their pages with their answers (they can't be filled in afterwards); the document's own fields
  stay fillable. An answer LimarePDF can't draw stops the append, naming the field. `limarepdf-cli merge` refuses
  such files after the first unless given `--flatten-forms`, which does the same.
- Text boxes and fill-and-sign text took characters their font can't write (Chinese, emoji and the like), showed
  them while typed and saved them as `?`. Such characters are now left out as they are typed or pasted, on the
  page, in the inspector's Text box and in Fill and sign's Text, with a note at the caret, as for new text; a
  text box or fill-and-sign text that still has them (your name put down, say) is refused with a message, with
  nothing changed.
- An emoji an installed font draws only in colour, with no shape of its own, was written into the PDF as
  nothing. It is now refused at the caret, naming the font. Emoji made of several characters (flags, skin tones,
  keycaps, and joined emoji such as families) were written as their separate parts in fonts that have them,
  saying something else (a thumb and a colour square). They are now refused whole, with a note saying these
  aren't supported yet, in every place LimarePDF writes text; comments, bookmarks and document properties keep
  them.
- A text box made by another app with characters Helvetica can't write (Chinese, say) was drawn again with `?`
  for them, without a word, when its colour, opacity, style or size was changed in LimarePDF, and flattening
  comments drew `?` into the page for good when it had no drawing of its own. Those changes are now refused with
  a message naming the characters, the box keeping its own drawing (it can still be moved or deleted), and
  flattening refuses such a box, naming its page, with nothing changed. On screen, in print and in exported
  images, a box without a drawing still shows `?`.
- An RSA-PSS signature (or certificate) that left out its RSA-PSS parameters, which the standard requires, was
  checked as if it named the defaults (SHA-1, salt of 20 bytes), so one made that way could show as valid. It
  now shows as *can't check*, as OpenSSL refuses it; an empty list of parameters still means the defaults.
- `limarepdf-cli render` left a filled form field empty in its images when the field had no drawing of its own
  (some apps leave drawing it to the reader). It now draws such fields as the app does; a field whose text
  LimarePDF can't draw comes out empty with a warning naming it, and the images are still written.
- A photo or scan of a whole sheet with a small signature on it was refused by Create signature's Image tab ("the
  picture has no signature to keep"), and shaded or grainy paper could be kept as ink. Make the paper transparent
  now judges the paper around each part of the picture, so it finds a small signature on a whole sheet and leaves
  out shadows, grain and stray specks, while a blank or noisy sheet is still refused. You can also drag around the
  signature on the picture to use only that part (Use the whole picture goes back), which helps when other
  writing shares the sheet; the picture file isn't changed, and a saved signature keeps the part chosen. Paper can
  now be made transparent in 1-bit and CMYK TIFF scans too. The window shows the picture and the result while you
  choose.
- Chinese, Japanese and Korean text whose character map (CMap) is stored in the file and built on another one was
  read as the wrong characters, or not at all, by search, copying, text export and `limarepdf-cli text`. The map
  is now read with the one it is built on, predefined or also stored in the file, as poppler reads it. How such
  text is drawn is unchanged (see Rendering, Known to be wrong).
- Two messages pointed to menus that don't exist: after exporting a certificate, "Edit > Trusted certificates >
  Add", and after signing with a self-made ID, "Edit > Digital IDs". They now give Document > Digital signatures
  > Trusted certificates… > Add certificate file…, and Document > Digital signatures > Digital IDs…. Setting a
  signature field's value with `limarepdf-cli form fill` said "LimarePDF can't sign yet"; it now points to Sign
  with digital ID in the app and `limarepdf-cli sign --field`.
- The user guide said applying redactions can't be undone (Undo brings the content back until the document is
  closed, also after saving), that flattened comments and fields can't be changed (they are no longer comments or
  fields, but content editing can still move or delete them, as the Flatten annotations question, its command
  description and the command line's help now say too), that the command line does everything the app does (it
  doesn't edit page content, add comments or fill and sign), that every font LimarePDF writes is embedded (plain
  Western text in Helvetica, Times or Courier isn't), and that a digital signature proves who signed (only as far
  as you trust the signer's certificate). These and a few smaller statements now match the app.
- Making a digital ID and signing with one use updated cryptography libraries (`crypto-bigint` 0.7.8, with
  `elliptic-curve` 0.14.2 and `sha2` 0.11.1): in earlier versions, some calculations meant to take the same time
  whatever the numbers could take different times depending on secret values, with some compilers and
  processors, which could leak information about the key (RUSTSEC-2026-0387). The other libraries LimarePDF is
  built from are updated to their latest compatible versions too.
- On macOS, opening a PDF with LimarePDF from Finder (Open With, a double-click when it is the default app, or
  dropping it on the Dock icon) started the app or brought it to the front without opening the file. Those files
  now open in tabs of their own, whether LimarePDF is starting or already running, several at once too, asking
  for a password where one is needed; while the app offers recovery copies at start, they open once that window
  closes. A file Finder sends that can't be found is reported. Not yet tried on a Mac.
- Spaces or a tab after a line's last word made the line's box reach past its text, over whatever came next: on
  forms whose labels end in a tab, a click between two labels, or on the start of the next one, took the first
  label, and editing it hid the next label until you finished. A line's box now ends at its last letter and
  starts at its first, and the spaces stay in its text.
- A click in a line turned on the page put the caret by how far across the page the click was rather than along
  the line, so it landed in the wrong word, and a double-click on a word the line has twice selected another
  word; in invisible recognised (OCR) text the caret was placed as if every character were the same width. The
  caret now goes by the character clicked, and a double-click selects the word clicked, whichever way the line
  runs.
- The command palette listed two commands named Dark, both under View, with nothing to tell the app's theme from
  the page colours, and searching "theme" found nothing. Commands in a submenu are now shown with its name
  ("Theme › Dark", "Page colours › Dark"), screen readers hear it too, and searches find them by it ("theme",
  "dark theme", "page colours"). The menus keep their short names.
- A watermark or headers and footers just added kept a second whole copy of the document so its section could
  keep changing it, outside the 1 GB undo limit, so a large document could hold more than the limit. The copy is
  now the undo step's own, counted in the limit; when a very large document leaves no room for it, the section
  stops changing what was added, saying so, and what was added stays on the pages.
- Text in many embedded fonts without a map to Unicode, mostly from TeX and other programs that give a font its
  own encoding, read as the wrong characters or control codes in search, selection, copying and exported text:
  ” as ", – as {, accents as Ø, ligatures as boxes. LimarePDF now reads such a font's characters from the font
  itself (Type 1, CFF and TrueType), as other readers do, where the document doesn't say how its codes are
  encoded. An encoding that lists its characters but leaves out an optional entry, or names one character by a
  name LimarePDF doesn't know, is no longer passed over whole. Across LimarePDF's test documents this fixed the
  text of 29 files (17 now read with no unreadable character at all), and made none worse. Text in the standard
  Symbol and ZapfDingbats fonts, when the file doesn't include them, read as Latin letters ("abg" for "αβγ", "4"
  for "✔"); it now reads as the characters shown, and a form value typed in a field set in one of them is no longer
  drawn in it as the wrong symbols.
- Document properties, bookmark titles, comments and form field descriptions that another program wrote in
  UTF-8 without saying so read as strings like "CafÃ©". They now read as written. Field names and values still
  read as the standard says, as other readers find them.
- Esc pressed while moving, resizing or turning an object, drawing a shape or a pen stroke, or selecting words by
  dragging didn't stop it: the move was still made when the button was let go, and words were selected again.
  Esc now cancels what is being dragged, changing nothing, and the tool stays for another try; Esc again leaves
  it as before.
- An object being moved when its page scrolled out of view (with the keyboard, say) stayed half-moved after the
  button was let go: it was shown in its new place but not moved, and its toolbar didn't come back. The move now
  ends when the button is let go, wherever the pointer is.
- Tab in the inspector's Text or Comment box moved on to the next control but didn't keep what was typed until
  something else was clicked. It is now kept as the box is left, as clicking away does.
- Tab in a dialog stopped once on nothing after the last control before going back to the first. It now goes
  straight round, and Shift+Tab the other way.
- On Windows, a print job the print spooler never took (a printer that doesn't answer, say) kept the app busy
  until it closed: appending files, recognising text and exports waited for it. LimarePDF now stops waiting when
  the spooler takes no step for two minutes, as it does for printing on Linux and macOS, and says the job may
  still print, so check the printer's queue before printing again.
- Deleting or extracting pages of a tagged document could leave the alternative text of content on the pages
  that went, where the structure tree gives its page only through the page's own index of its tags. That index
  is now read too, and alternative text whose page can't be told at all goes, rather than stay.
- A form whose fields are nested 33 levels deep could have its pages removed, but removing hidden information or
  redacting over its fields refused it as nested too deep. Fields up to 32 levels deep, with their widgets one
  below, are now handled by all of them alike; deeper ones are refused by all.
- Redaction treated text in a font whose code map breaks its own layout (a code that also starts a longer one)
  as readable character by character, though readers split such text differently, so text another reader shows
  under a mark could stay. Such text now goes with its whole string, as other text LimarePDF can't place does.
- Redaction marks laid over each other counted the part they share twice, so a character beside the marks could
  be removed though only a fifth of it was under them. A character now goes when a quarter of it is under the
  marks taken together, however they overlap, as for a single mark.
- `limarepdf-cli bates --digits` above 65,535 ended in "internal error". More than 20 digits (enough for the
  largest Bates number) is now refused with a plain message, writing nothing.
  ([#57](https://github.com/smaug42/limarepdf/issues/57))
- On a page with more than 10,000 annotations, a fill-and-sign item past the first 10,000 wasn't counted, and
  wasn't locked in place when a document someone had already signed was signed again. Every one is now counted
  and locked; answering a comment past them works too (it said the comment was on no page).
  ([#58](https://github.com/smaug42/limarepdf/issues/58))
- In text that is copied, searched or exported, a character code that a font's own code map leaves out (drawn as
  no character) read as an unrelated letter, `<0042>` as "B". It now reads as the replacement character �, as
  does a code with a drawing but no text, which was left out (a hyphen between two words, say).
  ([#59](https://github.com/smaug42/limarepdf/issues/59))
- In a signed document, each undo of a text edit made the next edit start a new embedded font instead of
  growing the one already made, so repeated edits and undos left several small fonts in the saved file. Undo,
  and putting a failed change back, now keep growing the same font.
  ([#60](https://github.com/smaug42/limarepdf/issues/60))
- Remove hidden information showed kinds the document has none of greyed out but ticked, which read as "this will
  be removed". They are now shown unticked; your choice comes back for documents that have them.
  ([#61](https://github.com/smaug42/limarepdf/issues/61))
- Create signature's Type tab showed the chosen handwriting style's white paper as a box on the button's tint.
  Each style is now a white card, as the signature will look on paper, in light and dark themes alike, the chosen
  one outlined; the saved signatures in the Fill and sign pane and the toolbar's menu look the same way.
  ([#62](https://github.com/smaug42/limarepdf/issues/62))
- Certify with digital ID needed scrolling in a window 800 points high, where Sign with digital ID fits. What
  others may do afterwards is now chosen from a list, so it fits too.
  ([#63](https://github.com/smaug42/limarepdf/issues/63))

### Added

- `limarepdf-cli sign --image` takes `--crop L,T,R,B`, the part of the picture to use in its pixels (from the
  top left), so the signature on a photo of a whole sheet can be used without cropping it first.
- In Fill and sign's Create signature, Image tab, the part of the picture used can be chosen from the keyboard:
  with the picture focused (Tab, or a click), the arrow keys move the part and Shift with an arrow key sizes it.
- A CMYK JPEG of a signature (from some print workflows) can now have its paper made transparent and part of it
  chosen, as other pictures can; before, it could only be used whole.

- Closing a document with unsaved changes, or quitting, now offers Save (Save as… for a document with no file
  yet, such as a new or recovered one), Discard or Cancel, instead of only discarding the changes. Saving asks the
  usual questions first (a file changed by another app, a repaired original, redactions not applied, digital
  signatures), and the document closes once it is saved. Quitting with several documents unsaved offers Review…,
  which asks about each in turn, or Discard all. A save that is cancelled, fails or can't be confirmed on disk
  keeps the document open with its changes and stops closing or quitting; nothing closes until the app quits, so
  cancelling part way keeps every document open.
- When you come back to LimarePDF after another app changed a file open here, its tab shows a notice with
  Reload. ☰ > File > Reload from disk reads the file again as it now is, after asking when there are unsaved
  changes, which it loses. Anything typed or changed while the file is read (a document title, say) is kept, and
  the document isn't reloaded, saying so.
- A security policy, [SECURITY.md](SECURITY.md): which versions get fixes, and how to report a security problem
  privately through GitHub. It is linked from the README and Troubleshooting, and comes in the archives and every
  package and installer.
- The Windows installer offers a desktop shortcut, under the install folder (not ticked unless you tick it). For
  installs without its screens, `DESKTOPSHORTCUT=1` asks for one; a newer version keeps the choice made before,
  and `DESKTOPSHORTCUT=0` removes it. Uninstalling removes it.
- The supported systems are stated in [Install](docs/install.md#requirements), the README and Features: Windows 10
  and 11 (x64), macOS 11 or later on Apple Silicon and Intel, and 64-bit Linux with glibc 2.39 or later, all
  provisional until the release checks by hand have passed on each.

### Changed

- Today's date, in Fill and sign, the date under a signature, a stamp's name-and-date line and headers and
  footers (`{date}`), is now your computer's calendar date instead of the date in UTC, which late in the evening
  west of Greenwich was already tomorrow's. The times recorded for other programs (a comment's modification
  time, a digital signature's signing time) are unchanged.
- A single emoji typed in Helvetica, Times or Courier now gets the note that asks for an installed font that has
  it, and one an installed font lacks names that font, as for other characters: single emoji are written in
  installed fonts that draw them with outlines, such as Symbola and DejaVu Sans. The note used to say emoji
  can't be added at all.
- Releases now wait for the check of every library against known security advisories (RustSec), and are tested and
  built from exactly the library versions in `Cargo.lock`.
- The README in the `.deb`, `.rpm`, Windows installer and macOS app linked to guide pages they don't include. It
  now links to the guide online for its version, as do the changelog and the security notes there; the archives
  still carry the guide. The Windows installer and macOS app now include the licence of the Liberation fonts too.
- The `.rpm` now asks for glibc 2.39 or later, which the Linux builds need, so an older system refuses it when
  installing instead of the app failing to start. The Intel macOS builds now say macOS 11 or later, as the
  Apple Silicon ones and the docs do (they said 10.12). The docs no longer say older Linux systems can use the
  archive: it needs the same glibc.
- Black-and-white (fax) pictures more than 65,535 pixels wide are now read and written: redaction cuts them as it
  does other pictures instead of removing them whole, and Optimise scans stores them instead of skipping them.
- Signed documents with the older mixed (hybrid) cross-reference layout now save with their signatures kept, like
  any other; for many of them LimarePDF had to write the whole file. A file in which only an older part of that
  layout lists an object now reads as other apps read it.
- Some files LimarePDF called damaged and rebuilt now open as they are written, since their cross-reference
  information is read correctly; a few of them show the note that other apps may show them differently instead.
- A line break written as it is inside a piece of page text (rare) now reads as a line feed, as the PDF format
  says and as LimarePDF draws it. Parts of a page written after a picture and separated by null characters
  (rare) can now be selected and edited.
- Libraries moved to their new major versions: fax 0.3 (black-and-white pictures), lopdf 0.46 (reading and
  writing PDF files) and egui 0.37 (the app's window and controls). With egui 0.37, on Linux, text selected in
  another app or in a LimarePDF text box can be pasted into a text box with the middle mouse button, as in other
  Linux apps (what a box can't write is left out with a note, as when typing), and text selected on the page or
  in a text box can be pasted into other apps the same way, and trackpad scrolling springs back at the ends of the
  document. The Phosphor icons are now part of LimarePDF itself rather than a separate library, and the
  packages include their licence (`FONT-LICENSE-Phosphor.txt`).

## [0.9.0] - 2026-10-23

### Added

- Text beyond Western European: Polish, Czech, Turkish, Romanian, Vietnamese, Greek, Russian, Ukrainian and the
  other languages written in Latin, Greek and Cyrillic, in new and edited text, text boxes, fill-and-sign text,
  form values (flattening them too), headers, footers, Bates numbers, watermarks, stamp wording and the
  name-and-date line, the name in a digital signature's look, and selected text printed or saved as a plain page,
  in the app and on the command line. Text the standard fonts can write is written as before; other text is set
  in the bundled Liberation face of the same font (same widths), embedded in the file and cut down to the
  characters used, and reads back in search, copy and other apps. Form fields keep naming their own font.
  Hebrew, Arabic, the Indic scripts, Thai and emoji aren't supported yet, nor Chinese, Japanese and Korean
  outside installed fonts (below): they can't be typed (the note at the caret says so), and are refused
  elsewhere, naming them, instead of coming out as `?` (a stamp leaves such a name off its line; text boxes and
  a plain page of selected text still show `?`).
- A typed signature a handwriting style can't show names the styles that can (Caveat has Cyrillic).
- Installed fonts for new and edited text: the inspector's font list has Helvetica, Times and Courier, then the
  fonts installed on the computer whose licence allows embedding (TrueType, OpenType and variable fonts), each
  name drawn in its own font, with a search field. Text in one is embedded in the file, cut down to the
  characters used, so it looks the same anywhere; Bold and Italic use the family's own faces (off when it has
  none), and a character the font lacks is refused with a note naming the font. Chinese, Japanese and Korean
  can be typed in an installed font that has them; typed in Helvetica, Times or Courier, they get a note naming
  that font and asking for an installed font that has them. An edited line from a font that is installed is
  written in that font again rather than in Helvetica, Times or Courier.
- A user guide for new users, starting at [docs/README.md](docs/README.md): installing, a guided first session,
  what works and what doesn't yet, then a page for each everyday task (reading and finding, organising pages,
  comments and markup, editing text and pictures, forms, fill and sign, headers, footers, watermarks and stamps,
  scans and OCR, export and printing, password protection, redaction, digital signatures), each opening with
  short steps; then the desktop app and command-line references, troubleshooting and a glossary. Every page links
  to the one before and after. Screenshots show the parts of the window, the inspector, the Create signature
  window and the signatures view. The developer docs moved to `docs/developer/`.
- Styles within a line, as in a word processor: select words in a line being edited, or in new text, and press
  Ctrl+B or Ctrl+I, or pick a font, size or colour in the inspector, to change those words only. With nothing
  selected, the word the caret is in changes, or, between words, what is typed next. Typed text takes the style
  of the text before it. Where the selection mixes fonts, sizes or colours, the inspector shows them blank (the
  colour button shows the colours side by side). Ctrl+Z and Ctrl+Y undo and redo typing and style changes in the
  line, never the document. New text's lines are spaced by their sizes, and the next new text starts in the
  style the last one ended in. A line turned on the page is edited in a box showing its fonts, sizes and colours.

### Changed

- The main window is rearranged. The top bar holds Open, Save and Export; Undo and Redo; the everyday tools (Read
  and select text, Add text, Add image, Redact, Fill and sign); one Comment button for notes, text boxes, shapes,
  the pen and stamps; and Search and Print. As the window narrows, the tools fold one by one into a Tools menu,
  which holds every tool, and come back as it widens; in very narrow windows Export, then Undo and Redo, leave
  the bar too (they stay in the menu). The page layout, Fit width and zoom moved to the bottom
  right, and the theme to View > Theme.
- The navigation panel has a column of icons for Pages, Search results, Bookmarks and Comments; clicking the open
  view folds the panel away. A document's digital signatures are listed from Document > Digital signatures,
  which also holds signing, certifying, digital IDs and trusted certificates (Digital IDs and Trusted
  certificates were in Edit).
- The menu has every command: new Tools and Pages menus (page commands moved there from Document), and entries
  for editing a text object, replacing an image, moving the selected object, going to a page, moving between
  form fields and page labels (Pages > Page labels…), which had only a button, a key, a right-click or the
  inspector.
- Tabs have their close button inside them.
- The selection's buttons can be docked at the top of the inspector instead of floating by the selection (the
  floating bar's Dock controls on the right, or View > Selection controls on the right); remembered between runs.
- The collapsed inspector is a strip with a button to expand it, the docked selection buttons, and its contents in
  a pop-out over the pages. A narrow window folds the navigation panel and the inspector away and brings them back
  when there is room, unless you closed them; whether each panel is open is remembered between runs.
- Double-click a line of page text with the read tool to edit it on the page, with the word under the pointer
  selected; Enter, Esc, Tab or a click elsewhere keeps the change. Editing the same line again within a few
  seconds joins the same undo step. Double-click and triple-click still select a word and a line where there is
  no line to edit, such as recognised text.
- The read tool also selects pictures, shapes, comments and marks, with handles to move and resize them; not
  pictures or shapes covering most of the page, such as a scanned page. Page text moves with Move or resize text
  (on its floating toolbar), which keeps the read tool on until you click away, or with the arrow.
- Changes in the inspector apply as you make them, each one an undo step: the Apply text changes, Apply style and
  Apply changes buttons are gone.
- The font size is a box showing the exact size, with a list of the usual sizes beside it.
- The tools that put something down (text, images, notes, text boxes, shapes, stamps, redaction marks, fill and
  sign) go back to the read tool afterwards, with the new item selected. A pin in each tool's options, Keep tool
  active, keeps that tool on for several in a row (all but the Note tool, where every click would put down a note
  and existing ones couldn't be opened); each tool's pin is separate and remembered between runs. Esc leaves any
  tool for the read tool. The pen stays on until Esc or another tool.
- The Fill and sign button has a menu: Signature and Initials (the saved ones and Create…), Date, Your name, Text,
  Tick, Cross and Dot. The Create signature window also opens with the inspector folded away.
- Form fields are highlighted from the start, in a lighter tint; the form banner's Highlight fields is a switch, and
  turning it off (there or in View) is remembered for every document.
- Dialogs share one look: a title naming the task and the document, the settings, and the buttons at the bottom,
  the action that does the task highlighted (red when Undo can't take it back or it breaks signatures) and before
  Cancel (after it on macOS). The window behind is dimmed and takes no clicks or shortcuts while a dialog is open.
  Esc cancels any dialog, except one holding work that would be lost. Messages have a Copy button; a signed
  document's save and sign questions put the reason under Details. The title is larger, in Inter Medium (now
  included), with more room around the title, the settings and the buttons; fields, lists and buttons are
  outlined and taller; and in a window under 600 points wide the margins are smaller.
- Print: the printer above a list of printers; All pages and Page range on one row, with the pages to print
  below them once Page range is chosen (which puts the keyboard there); Copies beside a Print on list of one
  side or both (one under the other in a narrow window); and Print as image under Advanced, below a rule. A page
  range that can't be read is reported under it instead of in a separate message.
- Create signature's Type has a "Your name" label above an outlined field, and the drawing area an outline.
- Every dialog now has Print's look: text at 14 points, settings in headed groups, labels above the fields, and
  radio buttons and checkboxes outlined on the dialog's background. Password required and Your name for comments
  have a labelled field. Remove hidden information lists what is removed by default and what is kept by default
  in two groups. The digital ID windows are titled with the task (Create a digital ID, Import a digital ID,
  Export), their forms are grouped (About you, Password), "Again" is now "Confirm password", and the Digital IDs
  window lists the IDs in a box above the selected one's details. Sign and Certify show where the signature goes
  under the title, the digital ID as a full-width list, Reason beside Location when there is room, and what others
  may change under its own heading. Signature details groups its facts (Signature, Signer, Time, the signer's own
  words, changes, warnings) with its buttons after a rule; the Trust question groups the choice and the
  certificate, with "Name in your list" above its field. Trusted certificates and Recover unsaved changes list
  their entries in one box, each with its buttons beside it. Create signature puts "Ink" above its colours.
- The command palette (Ctrl+K) takes the dialogs' look without a title or buttons: an outlined search field with a
  search icon, a rule, and the commands with their category and shortcut in aligned columns.
- Create signature: Draw, Type and Image (was Picture), the ink colour in the window too, and Use signature (was
  Done). Picking another tool closes it.
- Go to field in Signature details closes the window, to show the field.
- Tab goes through the top bar, the tabs, the bottom bar, the panels and each dialog's buttons in the order they
  are shown, left to right. Closing the inspector's pop-out with Esc returns the keyboard to its button.
- Screen readers can name the buttons, fields, lists and menus in the window, the inspector, the side views and
  the dialogs, checked by the app's tests (not yet with a screen reader). Names added: the search field, the
  pages and their thumbnails, Print's printer, pages to print, copies and Print on, the signature drawing area and ink
  colours, the banners' Dismiss button; the font list, Bold and Italic (with whether they are on), the font
  sizes, every colour button, line widths, opacity and the inspector's fields (each named by its label);
  Search results' field and ◀ ▶ (Previous match, Next match, now also as tooltips); the Fill and sign pane's
  saved signatures, date format, name, text and marks (with which is picked); Create signature's name field
  and handwriting styles; the bookmark arrows (Expand or Collapse and the title); the command search field;
  the zoom level; the edges of the side panel and inspector (Side panel width, Inspector width; mouse only);
  and an open list or menu, after the button that opened it. Buttons with an icon are named by their words
  alone, without the icon's character.
- Creating a digital ID shows a problem beside the field it is about, and the keyboard starts in the Name field.
- The Fill and sign pane no longer has a Sign with digital ID button: digital signing and certifying are in
  Document > Digital signatures.
- limarepdf and limarepdf-cli include the Liberation fonts (SIL Open Font License 1.1), which have the widths of
  Helvetica, Times and Courier: the app shows text in its own look while it is typed on the page, and both write
  in them the characters the standard fonts lack (see Added). They are stored compressed, adding about 2.4 MB to
  each program rather than 4.4 MB, and each is unpacked, unchanged, the first time it is needed. Release
  archives and packages include their licence as `FONT-LICENSE-Liberation.txt`.
- Add text is typed on the page, where you click, in the font, size and colour it will have, instead of in a box
  in the inspector with Add and Cancel. Enter starts a new line; Esc, a click elsewhere or another tool finishes,
  and the lines are added as one undo step. The inspector's font, Bold, Italic, size and colour style the text
  as you type, as do Ctrl+B and Ctrl+I (see Added); drag the text's outline to move it. Characters LimarePDF
  can't write yet can't be typed or pasted there, and a note at the caret says so, instead of turning into `?`.
  With the Add text tool, a click on page text edits that line, the caret where you clicked, and a drag selects
  text.
- A line of page text being edited is drawn as it will be written, in the fonts it will have (its own when they
  can be kept, else the Helvetica, Times or Courier replacing them, switching as you type), its sizes and its
  colours, over the line, which is hidden underneath in the colour around it. Characters LimarePDF can't write
  yet can't be typed there or in the inspector's text box, and a note says so. Turned lines and invisible
  recognised text are still edited in a box over the line.
- Text being typed on the page matches the page colours: in Dark, Sepia and High contrast, new text and edited
  lines show in the colours the page will give them, with a caret that shows on dark paper.
- Esc keeps what you typed instead of throwing it away, in a text box, a note and a form's text field, as it
  already did for new text and edited lines; Ctrl+Z takes it back. A note's comment box has no Done or Cancel
  button: a click on the page outside it, Esc or Ctrl+Enter closes it, keeping the comment (a note with nothing
  typed isn't added), and that click doesn't put down another note. While a new note is typed, the colour picked in
  the inspector colours it, and the note stays open and gets the keyboard back. A list that allows several choices
  has no Done button: each click applies, and a click outside it, Esc or Enter closes it.
- Esc with a list, menu or colour picker open closes only that, instead of also leaving the tool or finishing what
  is being typed.
- Finishing a note or text box by choosing another tool keeps that tool, as Add text does, instead of going back
  to the read tool. While typing in a note or text box, Ctrl+Z and Ctrl+Y undo and redo the typing, not the
  document.
- Document properties, Opening view and Page labels apply as you set them, one undo step per change, instead of
  with Apply or Number from page N: a property or the label prefix when you press Enter or leave the field, a
  choice at once, a number when you let go of it. Page labels show the numbering the first selected page has,
  and a change starts numbering there. Use current view sets the opening view at once. Saving or closing keeps
  a field still being typed in.
- Crop is set on the pages: while the Crop section is open the pages are shown whole, the part outside each crop
  box dimmed, and the crop box's edges and corners can be dragged on the pages being cropped. Each drag or
  margin typed crops at once, one undo step; Esc or closing the section shows the pages cropped. The Crop button
  is gone; Remove crop stays.
- Watermarks and headers, footers and Bates numbers are shown on the pages as you set them, without being added
  to the document, until Add to pages (was Apply). After that, each change updates the one just added, as its own
  undo step, replacing the Update button; Undo steps back through the changes with the settings following. Add
  another stays.
- Selecting words no longer fills the inspector with instructions, a copy of the selected text and a Copy button;
  it stays on the document or pages. Copy and the other commands for the selection are on its floating toolbar
  (or docked buttons), the right-click menu and Ctrl+C.
- The inspector no longer explains how to use each tool in paragraphs: the tools' panels show their options, and
  how to use a tool is in its tooltip, which now follows its name in the top bar. Other long notes in the
  inspector are cut to a short line or a tooltip (Bates numbering's), or removed where the panel shows it already
  (page labels, crop, opening view, form fields). Signing with a digital ID loses its Cancel button: Esc or
  another tool stops it, as for every tool. A selected group of content is called "Group".
- A line of text with a word in another font, size or colour in it (a bold word, a superscript) is one object,
  as it looks, rather than one per style: it is selected, moved, deleted and edited whole. Search, selection and
  copying read it as one line. `limarepdf-cli objects` lists it once, with its first part's font and size,
  marked "mixed".

### Fixed

- Making a digital ID and signing with one use an updated cryptography library (`ctutils` 0.4.5): earlier
  versions could take measurably different times depending on secret values, which could leak information about
  the key (RUSTSEC-2026-0385).
- Choosing a font, Bold, Italic, a size or a colour in the inspector while editing a line on the page ended the
  editing and showed a text box of the line's text above the font list, moving it down. The line now stays in
  editing, shows the change, and takes the keyboard back.
- Ctrl+Z while typing in the inspector's text box for a line undid the document's last change instead of the
  typing.
- Double-clicking a word in a turned line selected the line's first word, and the box it was edited in took its
  font size from the line's length (a very large font in a box running off the screen); the word is selected,
  and the box is the line's size, kept in view.
- Going into the font size box and out again, or pressing Enter in it, without typing rounded an unusual size such
  as 10.126 to 10.13; it now changes nothing, and the box shows the exact size. A change of size smaller than a
  hundredth of a point was also lost.
- In a narrow window the Create signature window was wider than the screen, so part of the drawing pad was cut
  off. The pad now shrinks to fit, also in a short window, as do the typed and image previews, and what was drawn
  keeps its shape and size when the window is resized.
- A dialog shown in a tall window stayed as tall when the window got shorter, pushing its buttons off screen; it
  now fits again.
- A dialog taller than the window, such as Sign with digital ID in a narrow window, opened too low and grew a step
  at a time, so until the mouse moved its buttons could be below the bottom of the screen. It now opens at its
  size, inside the window.
- A long printer name made the Print window, and its list of printers, wider than the screen; it is now cut short,
  and shown whole on hover and in the list, over more than one line if need be.
  About's row of links ran into the window's margin since dialogs moved to 14-point text, and past the screen's
  edges in a small window; About is a little wider, and in a narrow window the links take two rows. Password
  required showed a very
  long file name in full; it is cut short as in the other dialogs.
- A menu that was open when a dialog opened by its shortcut (such as Ctrl+P with the page layout menu open) stayed
  on top of the dimmed window; it now closes.
- Esc could be missed in Recover unsaved changes and Trusted certificates when their rows had different buttons
  (such as a copy that can't be restored, or a certificate being removed): the window was laid out twice on every
  frame.
- Screen readers now say which document or certificate each Restore, Discard, Show file, Remove…, Stop trusting
  and Keep button acts on (they all had the same names), and name About's copy button.
- When making, importing or exporting a digital ID failed, for example with a wrong password, the Digital IDs
  window went back to its list with the error at the bottom, losing what was typed. The form now comes back with
  its name, file or export choices kept and the error beside the password, or at the end of the form for other
  failures; the passwords are typed again, as before.
- When signing or certifying with a digital ID, a missing or wrong password was reported at the bottom of the
  window, below the other options, where it could be out of sight in a short window. It is now said under the
  password, where the keyboard goes; other problems stay at the bottom. Screen readers now also name the
  Digital ID, Reason and Location fields.
- Ctrl+A and other single-key shortcuts were missed when the key and Ctrl arrived together, as with a quick tap.
- Selecting a coloured comment or mark could count as an unkept change (asking about it on closing), because the
  colour button stored its colour back very slightly changed.
- Esc didn't close the Create signature window, and keyboard shortcuts reached the document while a dialog was
  open.
- Tab couldn't leave the empty password field when importing a digital ID.
- Some Western European characters came out as `?` in text LimarePDF writes, although its fonts have them:
  Š š Ž ž Œ œ Ÿ ƒ † ‡ ‰ ‹ › ˆ ˜ in new and edited text, form values and the OCR text layer; those and also
  € ‚ „ … ‘ ’ “ ” • – — ™ in text boxes, headers, footers, Bates numbers, watermarks, stamps, Fill and sign's text
  and the name on a digital signature. They are now written as typed, and stamps and form fields no longer
  refuse them.

## [0.8.1] - 2026-10-21

### Changed

- `limarepdf-cli` refuses, with nothing written, an output (`-o`) that is the command's input, by any path or a
  link to it. It used to replace the input with the result, so a redaction or removed information left no
  original to go back to.
- A digital ID whose certificate or key allows only RSA-PSS signatures can't be imported or signed with, and the
  reason is given. LimarePDF signs RSA with PKCS#1 v1.5, which such an ID mustn't make. Certificate details show
  such a key as "RSA-PSS" with its size.

### Fixed

- Flattening a form lost a filled field's value when the field had no drawing of its own and LimarePDF can't draw
  its text (Japanese, for example, filled in by an app that leaves the drawing to the reader): the field went and
  its value with it. Flattening now refuses, with nothing changed, and names the fields. Printing asks first
  whether to print without them, as they would come out empty on paper, and the command line's `print` refuses
  and names them.
- On a page with more than 10,000 annotations, the ones past the 10,000 LimarePDF shows were left alone by
  Remove comments, Remove attachments (the attached file stayed in the document) and redaction (comments
  touching a marked area stayed, and a redaction mark among them was never applied, with nothing said). Removing a
  comment also kept its replies there. These now deal with every annotation, as do annotations written into the
  page in a way the PDF format doesn't allow. Such a page gets a notice, since what is past the limit can't be
  seen in the app.
- Printing from the command line with `--as-image`, and every command-line print on Windows, left a filled form
  field blank when the field had no drawing of its own, even one LimarePDF can draw. Its value is now drawn in, as
  the app and ordinary printing already did.
- Deleting, extracting or splitting pages kept the alternative text (the descriptions accessibility tools read) of
  content on the pages that went, when the document's tags pointed at that content in some of the ways the PDF
  format allows, or gave its page only on a tag further up. All of it now goes, and so does the text of a tag with
  content on both kept and removed pages, and of every tag above one.
- Deleting, extracting or splitting pages kept a form field of the pages that went, with its value, when the form
  was nested more than 32 deep; they now refuse such a form, with nothing changed. A link by name to a page that
  went stayed when the list of named destinations was nested that deep; it is now found at any depth.
- Removing form field values left a value behind when it was more than 32 fields above a form box that the form's
  list of fields leaves out. Such a form is now refused with nothing changed, as other forms nested that deep
  already were.
- Redaction kept the alternative text of tags more than 64 levels above the marked content, and the value of a
  form field touching a mark when its form was nested more than 32 deep. Tags at any depth now lose their text;
  such a form, or tags whose index is nested more than 32 deep, make redaction refuse with nothing changed.
- Redaction could make a picture disappear from another page, when that page drew it from inside drawings nested
  more than 8 deep, or through a drawing it shares with other pages that uses each page's own names for pictures,
  and the same picture was under a mark elsewhere.
- Removing hidden layers could make a hidden layer's content show, when a drawing that has no resources of its
  own was used on pages that give its layers different meanings: it was checked only as the first of them has it.
  Such a drawing now makes the command stop, with nothing changed, as other drawings it can't check do.
- Chinese, Japanese and Korean text in a font with its own character map put the characters after a code the map
  leaves out in the wrong places, so a redaction area over a character where other readers show it could leave
  that character in the file, with the check afterwards finding nothing. Such codes now take as many bytes as the
  map's code ranges say, as other readers take them. LimarePDF draws such codes differently, so a piece of text
  it draws elsewhere than it reads is removed whole when the area covers it in either place; so is text in a font
  whose code ranges can't be read.
- A small crafted file could keep the app busy for hours on opening or in the Bookmarks tab, with no way to stop it,
  when its list of named destinations listed the same part many times over. Each part is now read once, and a
  lookup gives up after 100,000 parts; a bookmark, link or opening page whose name it gave up on goes nowhere.
  Redaction refuses, with nothing changed, when the index from a page to its tags is too large to search that way.
- `limarepdf-cli verify --signed-version` wrote into the output file in place, so an output that was a hard link
  to the input cut the input short. It now writes a new file and puts it in place, as every other command does,
  and refuses an output that is the input itself or a link to it.
- `limarepdf-cli` said nothing and exited with status 0 when it couldn't write what it prints, a full disk for
  example. It now stops with a plain error and status 1, and says whether it had already saved its file. A reader
  that stops early (`… | head`) still stops it quietly.
- A signature by an RSA key whose certificate allows it only for RSA-PSS signatures was called valid (and could
  be trusted) when it broke those limits: a PKCS#1 v1.5 signature, or an RSA-PSS one with another digest or mask
  than the certificate names or a shorter salt. Such a signature is now broken, and limits that can't be read
  give "can't check". The same goes for timestamps, and for the certificates of a chain to a trusted authority.
- At the start, crash recovery followed a symbolic link in its folder, so a link there to another folder could get
  that folder's PDF files deleted, or offered as recovery copies. It now looks only in real folders of yours, and
  leaves anything else untouched.
- Saving a signed document whose only change was to the file's trailer, such as its document information removed,
  wrote nothing, so the change was lost on reopening. Such a change is now added as a revision of its own, and
  checked when read back.
- On Linux and macOS a save could be undone by a power cut just after it, as the folder holding the file wasn't
  synced. It now is. When the system can't confirm it, the file is in place all the same: the app says so and
  keeps the document marked unsaved, and `limarepdf-cli` says so and exits with status 1. Settings, the recent
  files list and saved signatures, which were written into the old file in place (a crash part way lost them), are
  now saved the same way.
- Headers, footers, Bates numbers and the text layer OCR adds could change the font of text already on the page,
  so its words came out wider or narrower, when the page's own font had the name LimarePDF gives its Helvetica.
  They now take a name the page doesn't use.
- Bates numbers that would go past 18446744073709551615 stopped part way, with the first pages stamped and the
  command line reporting an internal error. They are now refused before any page changes, with a plain message.
  A rotation by a number of degrees close to the largest LimarePDF can take did the same; it now turns the pages
  by what is left after whole turns.
- Straightening a dark scanned page took up to two and a half times the memory of optimising it without
  straightening (211 MB against 86 MB on a black 4000 × 4000 page). It now takes about the same.

## [0.8.0] - 2026-10-20

### Added

- Crash recovery: while you work, LimarePDF keeps a private copy of each document with unsaved changes, about a
  minute behind. If it stops without closing normally, the next start lists them: restore one as a new, unsaved
  document, discard it, or decide later (copies are kept for 30 days). Your own files are never written by it.
  Password-protected documents get no copies, nor documents you add a protected document's pages to. Applying
  redactions, removing hidden information or setting a password deletes the document's earlier copies first,
  including the one a restored document came from, and so does redoing a redaction or removal after undoing it;
  later copies hold none of what was removed. If an earlier copy can't be deleted, or the disk can't confirm the
  deletion, the command (or the redo) doesn't run, however often it is tried, until it can. A restored copy of a signed document doesn't have its signatures
  checked, since they cover the original file. A damaged copy, or a damaged description of one, is listed as
  damaged and never stops LimarePDF from starting. Turn it off with File > Keep recovery copies of unsaved changes.
- The window opens at the size it had when LimarePDF last closed, and maximized if it was. Its position isn't
  remembered, since Linux Wayland sessions place windows themselves.
- A notice when a file isn't put together quite as the PDF format asks, so other PDF apps may show it differently
  (for example, a part the file deletes is still in use, or data follows its end): a crafted file could show one
  thing in LimarePDF and another elsewhere. Its Save as… button writes a copy every app reads the same. The command
  line prints a note on stderr. Until now this only stopped signatures from being vouched for.

### Fixed

- Every encrypted file whose cross-reference information is a stream (the files LimarePDF itself writes with a
  password among them) counted as one other PDF apps may read differently, so a signature in one would show
  *can't check*. Of about 1,100 public test files, 17 no longer count as such, and no others changed.

- Text in many Chinese, Japanese and Korean files came out as "�" or the wrong characters when searching, copying
  or exporting: fonts without a ToUnicode map (common when the fonts aren't embedded) are now read through their
  character map, as pdf.js does, and highlights sit on the right characters. Of about 1,100 public test files, 43
  read better, such as a 42-page Japanese company report that came out entirely as "�" and now matches poppler,
  and none worse.

- The command line printed some text from the file exactly as it was: the document's title and other details
  (`info`), comment authors (`annot list`), form field names (`form list`), page labels (`labels list`,
  `view-prefs list`), font names (`objects`) and the text around search matches (`search`); bookmark titles
  (`bookmarks list`) kept direction-changing characters. A crafted file could use this to send escape codes to the
  terminal (changing colours or the window title, clearing the screen) or make a line read differently. These
  listings now remove control, invisible and direction-changing characters, as `verify` already did; `--json` output
  is unchanged.

- Typing page numbers into a pages box that said "all" (printing, crop, watermark, headers and footers, OCR,
  optimising scanned pages, export) gave "all2-4" instead of "2-4". Clicking or tabbing into such a box now selects "all", so typing replaces
  it.

- A small crafted file could make LimarePDF ask for gigabytes of memory and close, losing unsaved work in every tab:
  a page with a huge stated size (up to 17 GB to draw), a fax or 1-bit picture of billions of pixels when optimising
  scans or redacting (up to 4 GB), or a font's character widths (billions of entries; a width list with a huge first
  character code also made reading the page's text fail). These sizes are now capped: a page image at 268 million pixels, which
  gives an error naming the page (A3 still exports at 1152 dpi; of about 7,200 test pages, the largest poster
  exports at up to 208 dpi), pictures at 268 million pixels, and widths at the 65,536 characters a font can have.
  Printing on Windows draws pages larger than the limit at a lower resolution instead. A font's map from characters
  to text, used for searching, copying and editing text, could also ask for gigabytes (a map of 4 KB asked for
  64 MB), or keep LimarePDF busy for many minutes when it mapped the same characters over and over (a 16 MB map
  can ask for about 50 billion). Text for one character is now
  limited to the 512 bytes the PDF specification allows, a map to 32 MB, and reading a map stops after about a
  million characters; none of the test files' 2,079 maps comes near these limits.
- Undo on a large document could take gigabytes of memory: each step keeps a whole copy of the document, so 20 edits
  on a 139 MB scan took about 2.6 GB. Each document's undo steps are now limited to 1 GB together; beyond it the
  oldest steps are dropped, so undo is shorter on very large documents (about 7 steps on that scan). The latest
  change can always be undone.
- A page with a very complex drawing could take about 10 GB of memory to draw at 150 dpi, and in the app, zooming
  in on it a few steps quickly could draw it at several zooms at once, enough to close LimarePDF and lose unsaved
  work. Pages are now drawn in bands of about 4 million pixels, which bounds the memory of each drawing (that test
  page now takes 1.5 GB at 150 dpi), and the app stops drawing a page at a zoom you've moved past, in colours
  you've changed, or of an edited document's older state, at the next band. Large drawings, such as exports and
  printing at high resolution, take longer on some pages, since each band works out the page's pictures and
  shadings again: on test pages at 288 dpi about a third longer in the middle of the range, and up to several
  times longer on a few pages with many pictures.
- Editing a line of text, or adding text, a shape or a picture, on a page printed to PDF from a web browser drew the
  new content upside down, mirrored and shrunk (a 28 pt line became about 7 pt). Such pages flip and scale the whole
  page without restoring it afterwards, and the new content inherited that.
- Editing invisible text (an OCR layer) moved the line down by about a fifth of its height.
- Deleting or editing a line of text moved the text that carried on after it on the same line, such as a word in
  another font or colour, by the width of what was deleted or changed; moving a line moved the line below it
  sideways. The text after a changed line now stays where it was.
- Editing, redacting or removing hidden layers on a page with an inline picture (one written into the page's own
  content) changed any text on the page holding the word LimarePDF uses inside to mark such pictures, putting the
  picture's data in its place, and a crafted file could make the picture appear a second time. Pictures now go
  back only where they were, and text is left as it was.
- Repairing a damaged file could show a PDF attached to it, stored uncompressed, instead of the file's own pages,
  when the attachment's length was written in an object of its own. Such lengths are now looked up, and a copy of
  that object inside the attachment isn't used.
- Some damaged files opened as empty documents, with no pages and no message, where other readers show their pages:
  files whose list of pages holds a page itself instead of a reference to it, and files that give a second object
  the page's number. Their list of pages is now repaired, and the file is marked repaired as other damaged files
  are. A file in which no pages can be found is no longer opened empty: LimarePDF says it can't find any pages.
- Some RC4-protected files with no open password asked for one that other readers don't need: files whose password
  entry (`/U`) holds only the 16 bytes readers compare, instead of 32. They now open without asking.
- A document that opens without a password but has a permissions password showed the notice "This document was
  password-protected", as if a password had been needed. Its notice now says it opens without a password and that
  saving removes the permissions password unless you set it again.
- Removing hidden information didn't remove everything it said it had:
  - Form field values only went back to the form's defaults: read-only fields kept their values, default values
    stayed (and were filled back in), and so did values held by a parent field and rich text. Every value is now
    cleared for good, read-only fields included, along with defaults and the field drawings that showed them.
    Signature fields are left alone. Reset form still goes back to the defaults.
  - JavaScript and other actions that run by themselves stayed when they followed an ordinary action written
    inside a link, bookmark, button or the opening action (a web link that then runs a script), or belonged to an
    annotation written directly in a page's list. They are now found wherever they are; the ordinary actions stay.
  - Hidden layers stayed in drawings nested more than eight deep, in annotations' drawings and in patterns, and a
    drawing that couldn't be read was passed over; the removal still said the layers were gone. Drawings are now
    looked into up to 32 deep, annotations' drawings, patterns, soft masks and Type 3 glyphs included. If something
    can't be looked into, the command stops with a message saying what, and nothing is changed; leave out hidden
    layers to remove the rest. Compressed data damaged part way was read only as far as the damage, and data with
    a wrong checksum could lose its end, so content after it was never looked at (and was dropped from a drawing
    that was rewritten). Compressed data is now checked to its end: damaged data stops the command with nothing
    changed, and complete data with a wrong checksum is read in full, the result saying so.
- Extracting, deleting or splitting pages kept the other pages' content in the saved file whenever something still
  referred to them: a bookmark, a named destination, a link, the opening view, a form field or the document's
  structure (tags). Those references now go: a link to a page that went goes nowhere, form fields whose boxes were
  all on those pages leave the form, and alternative text of their tagged content goes. Extracted files and split
  parts keep the bookmarks to their own pages (a bookmark to another page stays, without a destination, only when
  it has bookmarks below it that stay); deleting keeps every bookmark, as before, without a destination when its
  page went. Printing selected pages goes the same way.
- A malformed file that made `limarepdf-cli` fail inside a library (such as a page the renderer can't draw at some
  resolutions) ended it with a Rust panic message and exit status 101. It now prints a plain error, "LimarePDF hit
  an internal error (...). The file may be damaged.", and exits with status 1, as for other errors. Setting
  `RUST_BACKTRACE=1` also prints where it happened, for bug reports.
- A question such as Flatten, Apply redactions, Remove hidden information or Discard changes could act on the wrong
  document: the questions don't stop the rest of the window being used, so another tab could come to the front
  (clicked, or a file finishing opening) or another tab could close before you answered. Each question now acts on
  the document it was asked about, bringing it to the front. If that document changed or was closed meanwhile,
  nothing is done, and a message says so.
- Text being typed but not yet kept was left out by Save and lost by Close and Quit without asking: a comment or
  text box typed on the page, a form field, the selected comment's changes in the inspector, and a reply not yet
  added. Save now keeps it first (as clicking away would; Undo takes it back), and Close and Quit ask as for unsaved
  changes.
- Optimising scanned pages changed pictures on pages that weren't selected when they used the same picture as a
  selected page, and a picture shared by selected pages was treated as the first one needed: a page showing it
  larger could lose detail, and one with a text layer could be straightened under its text. Only the selected pages
  change now; a shared picture is optimised as carefully as the most demanding of them needs, and the report says
  when the original stays elsewhere. The report also named the wrong page as the one optimised.
- A file with a text box comment that has no drawing of its own, and a colour in its style with a letter such as
  "é" in it, didn't open ("LimarePDF hit an internal error"), and `limarepdf-cli` couldn't draw its page. Such
  colours are now ignored, as other malformed colours already were; a colour with a sign in it (`#+1+2+3`) is
  ignored too, where it used to be read.
- Signing a document set to get a password when saved (Document > Password security), or setting one while signing
  ran, lost the password: the signed file, which LimarePDF writes without one, took the document's place. Signing
  is now refused while a password is waiting to be saved, with a message saying why. A password set, or text typed,
  while signing runs keeps the document open as it is, and the signed file opens in a new tab.
- A document timestamp issued by an intermediate authority showed as not trusted, even when you trusted the
  authority above it: the intermediate certificates the timestamp carries were ignored. They are now used to reach
  an authority you trust, as they already were for signatures and the timestamps inside them; carrying a
  certificate never makes it trusted.
- Splitting a document held a whole copy of it for every part until all were written: splitting a 139 MB scan into
  one file per page took more than 8 GB of memory, and a 1,000-page text file of 1.5 MB took 5.2 GB. Each part now
  holds only what its pages use and is written as soon as it is made (406 MB and 39 MB for those two, and faster).
  Extracting pages and printing selected pages also take less memory. If a part can't be written, the message says
  how many files already were.
- A small crafted page with tens of thousands of lines of text, each set apart from the next, closed LimarePDF
  (losing unsaved work in every tab) when its text was read for selecting, searching, copying or exporting, and
  closed `limarepdf-cli text` and `search`. Such pages now read normally, and pages with many separate lines read
  faster (3,000 lines: 0.1 s, was 0.4 s). The order text is read in is unchanged (the same for all 1,089 public
  test files).
- Choosing a picture to place on a page, or a picture stamp, decoded the whole picture only to learn its size,
  every page of a TIFF: an 8000 × 8000 PNG took 0.6 s and 370 MB more. The size now comes from the file's header.
  A picture damaged after its header is now refused when it is placed, with the same message, instead of when it
  is chosen.
- Exporting pages as images and removing blank pages read each page's fonts again for every page. Each worker now
  keeps them from page to page: exporting a 203-page Japanese document takes 1.6 s instead of 2.1 s, and finding its
  blank pages 0.7 s instead of 1.0 s, for up to about 15% more memory on such documents.

## [0.7.2] - 2026-10-18

### Added

- A notice above the pages when parts of the document couldn't be drawn, such as a picture that can't be decoded.
- Zoom up to 6400%, as Acrobat (was 800%). At high zoom only the part of the page in view is drawn, in tiles, so
  pages stay sharp and quick to draw, and memory use stays about the same at any zoom.
- More annotations that files store without a drawing are drawn, on screen and when printing: stamps (the stamp's
  name in a coloured frame), carets, file attachment icons, the seven standard note icons (in the app too), cloudy
  borders, text box callout lines and line leader lines. LimarePDF's own notes now show the Comment icon, a speech
  bubble, as their icon name says.
- Chinese, Japanese and Korean text in fonts the file doesn't embed is drawn with installed fonts, looked up by name,
  family, style and language (Noto CJK, Source Han, MS Mincho and the like), instead of being left out. Such fonts
  named Arial, Times New Roman, Courier New, Calibri or Cambria use Liberation, Carlito or Caladea when the original
  isn't installed, and other Latin ones a similar installed sans, serif or monospaced font instead of showing wrong
  characters.
- A notice names fonts that aren't in the file or installed and says what to install: the font itself when a
  similar one is shown in its place, or a font for the language (such as Noto Sans CJK JP) when no installed font
  has its characters.
- Damaged files open: when a file's cross-reference information (where each object is) is broken or missing, its
  objects are found by reading through the file, as other PDF apps do. A notice says the file was repaired and some
  content may be missing, with a Save as… button; the document counts as changed, and Save asks before replacing
  the damaged original. A repaired file's
  signatures never show as valid and unchanged. `limarepdf-cli` repairs them too, with a note on stderr. Stream data
  is skipped by its stated length, so a PDF attached to the damaged file, or text crafted to look like objects, is
  never taken for the file's own content.

### Fixed

- Security: while a document was saved over an existing file, its temporary copy could be read by other users of the
  computer, even when the document was private to you. It is now readable only by you until it replaces the
  document, which keeps its permissions. The temporary file is always made new under a fresh name, so a file
  already at that name (left by a crash, or put there by someone else) is never written to, followed or removed.
  The same goes for exported digital IDs and the trusted certificates list.
- A menu command, shortcut or panel button that failed on a malformed file could close the app, losing unsaved work
  in every tab. Now it shows an error message, or, when it had already changed the document, that tab's error page
  with Save a copy and Close; other tabs are not affected.
- A failure right after applying redactions or removing hidden information could leave the tab not knowing content
  was removed, so saving a signed document would have kept it in the earlier signed version. The two are now one step.
- A damaged file holding an uncompressed PDF attachment, or crafted to, could open showing the attachment's pages
  or other content than other PDF apps show, when lopdf rebuilt its table. LimarePDF's own repair, which reads
  stream data by its length, is now used instead.
- A page whose only picture can't be read no longer brings up the notice that the document has scanned pages.
- A page that can't be drawn shows "This page couldn't be drawn" and the reason, instead of "Rendering…" forever
  while the app tried again on every frame.
- Zooming in far on a large page could close the app, because the page was drawn as one picture bigger than the
  graphics card takes. Pages shown more than 4096 pixels a side are now drawn in tiles.
- Printing on Linux and macOS could come out blurred and unreadable: some printer setups (seen with an HP LaserJet
  through CUPS) need the job to name the paper size, as other apps' jobs do. Each job now names the printer's
  default paper.
- Comments could print upside down in the bottom half of pages whose content turns the page and doesn't turn it
  back, as some PDF makers write them: CUPS adds the comments after the page's content. Each page sent to the
  printer now puts the drawing state back at its end.
- Filled-in form fields could print empty: some CUPS setups remove a form's fields without drawing them. The copy
  sent to the printer now has the fields drawn into its pages (those meant for printing; the document itself is
  unchanged).

### Testing

- Correction: earlier notes (0.4.0) said printing on Windows had been tried and works. Only the print dialog was
  tried; nothing has yet been printed on paper from Windows.

## [0.7.1] - 2026-10-17

### Changed

- MPE is now LimarePDF. The desktop app is `limarepdf`, the command-line tool `limarepdf-cli`, and the packages
  `limarepdf`; the app shows as LimarePDF in "Open with", menus, the Start menu and on macOS. Settings, digital IDs
  and trust lists are kept in a `limarepdf` folder (`~/.config/limarepdf/` on Linux). The repository and package
  site are now github.com/smaug42/limarepdf and smaug42.github.io/limarepdf.

## [0.7.0] - 2026-10-17

### Added

- Page colour modes, from View > Page colours: Dark (dark paper and light text, hues kept), High contrast (white on
  black), Sepia, or Automatic (dark pages while the app's theme is dark). Pictures keep their own colours unless
  "Recolour pictures too" is on. Remembered between runs. On screen only: the file, printing and exports are not
  changed.
- Bookmark editing: add a bookmark to the current page, rename one, add one inside another, or delete one with the
  bookmarks inside it, from the Bookmarks tab (Undo reverses each), and list, add, rename and remove them with
  `mpe bookmarks`.
- Page layouts, from a top-bar button or View > Page layout: facing pages (two side by side, with the first page shown alone like a book
  cover, or paired) and single page (one at a time; scrolling past a page's end turns it), besides continuous
  pages. The choice applies to the document being viewed and is remembered for documents opened later.
- Viewer preferences: a document that sets the layout, side panel (Pages or Bookmarks) or page it opens with now
  opens that way. Set them in the inspector's Opening view section, or with `mpe view-prefs list|set|clear`.
- Stamps: Approved, Not approved, Draft, Final, Confidential, For comment and Void, your own wording in green, red
  or blue, with your name and the date if you like, or a picture of your own, from the Annotate menu. Click to put
  one down or drag to size it, then move, resize or fade it like other annotations. Your name can be changed right
  in the stamp tool's pane. The last five wordings and pictures are remembered. Also `mpe stamp` (`--text` and
  `--color` for your own wording).
- Redaction: mark areas (Annotate > Mark areas for redaction), selected text or every search result, check the
  red-outlined marks, then Document > Apply redactions… removes the text, drawings, comments, links and form fields
  under them from the file (not just covering them), blacks out the pixels of pictures and scanned pages there, and
  paints the areas black. It then checks that nothing readable is left there, and reports what was removed and
  where the removed text still appears elsewhere. Marks are standard PDF redaction marks, so they can be reviewed
  in other apps. While there are marks, a notice above the pages offers Apply redactions…; saving with marks not
  yet applied asks first. Also `mpe redact` (`--text`, `--area`, `--mark-only`, `--apply`).
- Fill and sign: the Sign button in the top bar puts your signature, initials, the date (in three styles), your
  name, text or a tick, cross or dot on the page, in black or blue. Signatures and initials are drawn on a pad,
  typed in one of four handwriting styles (Caveat, Cedarville Cursive, Homemade Apple and Zeyada, bundled), or taken from a photo or scan
  with the paper made transparent; up to five of each are remembered. Click to put one down or drag to size it;
  clicking an empty signature field signs in it. Items stay movable until Finish signing (in a notice above the
  pages, the Sign pane and the Document menu) draws them into the pages. Typed signatures from the command line with
  `mpe stamp --signature` (`--style`, `--color`, `--date`). This is a picture of a signature, not a digital
  signature.
- Remove hidden information: Document > Remove hidden information… lists what the document carries besides its
  pages and removes what you tick: metadata (including XMP), JavaScript and automatic actions, attachments, hidden
  layers and their content, thumbnails and private data, and if you like comments, form values, bookmarks and what
  lies outside the crop box. Also `mpe sanitise` (`--list`, `--keep`, `--also`).
- Checking digital signatures: `mpe verify` tells for each signature whether the signed part is unchanged, who
  signed according to their certificate and who issued it, when (from a timestamp authority or the signer's
  computer), certification and locked fields, and warnings such as SHA-1, short keys or an expired certificate.
  RSA and ECDSA (P-256, P-384) signatures, embedded and document timestamps; copied, moved and wrapped signatures
  are refused. It also lists what changed after each signature (form filled in, signed again, comment added, page
  content changed, and so on) and whether the signer allowed it, by Acrobat's rules: filling, signing and comments
  are allowed after an ordinary signature, a certification allows less, a locked field may not change, and
  anything MPE doesn't recognise counts as not allowed. Malformed additions and the published "shadow" and
  incremental-saving attacks are caught. `--signed-version N -o FILE` saves the version a signature signed.
  `--json` for scripts. See [Digital signatures](docs/signatures.md).
- Trusting signers: `mpe trust add` trusts a signer (from a signed PDF or a `.cer`, `.pem` or `.p7c` file) or a
  certificate authority, `mpe trust list` and `mpe trust remove` manage the list, and `mpe verify` then says
  whether each signer is trusted and why not. Chains to a trusted authority are built from the certificates the
  signature and the document carry and checked by RFC 5280's rules at the signing time (each issuer allowed to
  issue, key usages, path lengths, certificate policies); forged issuers, look-alike certificates with a trusted
  name, SHA-1 or 1024-bit links and unknown critical extensions aren't trusted. Embedded and document timestamps'
  authorities are judged the same way. `--require-trusted` makes `mpe verify` fail unless every signer is
  trusted, and `--trust-file` uses another list. Trust settings affect MPE only.
- Digital IDs, for signing: `mpe id create` makes a self-signed ID like Acrobat's (RSA 3072-bit,
  valid five years) protected by a password you choose; `mpe id import` imports a `.p12` or `.pfx` file from
  Acrobat, Windows, macOS or a certificate authority, old and new encryptions alike, refusing ones MPE couldn't sign
  with and saying why; `mpe id export` saves an ID with its key (`.p12`, for other apps or a backup) or its
  certificate (`.cer`, for others to trust you); `mpe id list` and `mpe id remove` manage them. IDs are kept in
  the settings folder, each encrypted with AES-256 under its own password (600,000 PBKDF2 iterations); passwords
  are asked for without being shown, never taken on the command line, and never stored.
- Signing with a digital ID: `mpe sign` signs in an empty signature field (`--field`) or a new box (`--page`,
  `--rect`), or certifies the document (`--certify no-changes|forms|comments`), as Acrobat does
  (`adbe.pkcs7.detached`, SHA-256, SHA-384 for P-384 keys), with your reason and location if given. The signature
  is always visible: your name in handwriting (or a picture of your signature, `--image`) beside "Digitally signed
  by NAME" and the date in your time zone. It is added after the file's existing bytes, so earlier signatures stay
  valid. MPE refuses expired IDs, encrypted documents, and what a certification or an earlier signer's lock doesn't
  allow; a field that locks others makes them read-only; fill-and-sign items are drawn in first (locked in place on
  a document someone already signed). Every signed file is checked before it is written: the new revision read back
  object by object, then the signatures checked as `mpe verify` would.
- Checking: a form field made read-only when a field that locks it is signed is now an allowed change (form-filling
  level), as Acrobat makes locked fields read-only when signing.
- Signing in the app: Document > Sign with digital ID (also in the Sign tool's pane) and Certify. Click an empty
  signature field or drag a box, then pick the ID, type its password, and optionally a reason, a location and, for
  a certification, what others may change; your saved fill-and-sign signature can be the look's left part. The
  signed copy is saved as a new file (`NAME-signed.pdf` suggested) and opens in place of the document. Edits made
  since opening are signed with it; when they would show on earlier signatures as not allowed, MPE says which and
  asks before signing. After redacting or removing hidden information, the document is written out in full before
  it is signed, so nothing removed stays in it (a document already signed is saved first). Signing runs in the
  background; a password is used once and overwritten.
- Digital IDs window (Edit > Digital IDs): create, import, export (certificate or with its key), make default and
  remove digital IDs, as `mpe id` does.
- Digital signatures in the app: a signed document is checked when it opens, and a banner above the pages gives
  the result, coloured green, amber, grey or red. The Signatures tab of the side panel (View > Signatures) lists
  each signature with its result, the changes made after it and any warnings, then the empty signature fields.
  Details (also by clicking a signed field) shows the signer, certificate, times, changes and certificates, with
  Trust this signer…, Trust an authority…, View signed version (opens in a new tab) and Go to field. Edit >
  Trusted certificates… lists, removes and adds trusted certificates (also from a `.cer`, `.pem` or `.p7c` file).
  After saving, the saved file is checked again.
- Saving a signed document keeps its signatures: Save and Save as add your changes after what was signed, as
  Acrobat does, so each signature stays valid and lists the changes. MPE asks first when the signatures don't allow
  the changes, naming each signer and what they will be told, and asks before writing the whole file when it must,
  which breaks the signatures: after redacting or removing hidden information (the signed version would still hold
  what was removed), with a password set, or for encrypted or damaged files. Every editing command on the command
  line does the same, refusing unless `--accept-disallowed` or `--break-signatures` is given; `mpe sanitise` takes
  `--break-signatures` too. A signed form's fields without a drawing of their own are drawn on screen and in print
  only, not into the document.

### Changed

- A new app icon: the PDF icon from the start page, on the window, the taskbar, the About window, the Windows `.exe`
  and installer, the Mac app and the Linux packages.
- A new About window (☰ > Help > About): the app icon, version with a Copy button (version and system, for bug
  reports), "Made in Canada" with a maple leaf, links to the website, licence and issue tracker, a Contributors page,
  and the credits for bundled fonts, icons and the text recognition model. The start screen also says "Made in
  Canada".
- The form notice ("This document has a form") no longer shows for documents whose only fields are signature
  fields.

- Link addresses, bookmark names, comments and their authors, tab and window titles and print job titles are
  cleaned before they are shown: direction-changing (bidi), invisible and control characters are removed and very
  long text is shortened, so a link can't disguise where it goes.

### Fixed

- The Windows build's tests: git no longer changes line endings in PDFs on checkout, which broke the signed test
  files. CI jobs now get a read-only token unless they need more.
- Files whose trailer says `/Encrypt null` (not encrypted, written by some signing tools) open instead of asking
  for a password that doesn't exist.
- Parts of a file that a later revision deletes, and copies in object streams that the file's index doesn't list,
  are no longer shown, edited or saved: MPE reads them as the PDF standard says.
- Text in Type 3 fonts whose glyphs aren't measured in thousandths of an em is placed correctly for selection,
  search highlights and copying.

## [0.6.0] - 2026-10-15

Page tools and sturdier handling of damaged files: remove blank pages, split at bookmarks, page labels and picture
watermarks; search that finds hyphenated words; and a malformed file now shows an error instead of closing the app.
The Windows installer shows its steps.

### Added

- Remove blank pages: pages of one even colour, give or take specks or a lone page number. In the app (☰ menu and
  command palette; Undo brings them back) and as `mpe remove-blank`, with `--tolerance` for how much may be on a
  page.
- Split at bookmarks, down to a chosen level, with each file named after its bookmark: in the app's Split into
  files section and as `mpe split --bookmarks [LEVEL]`. Splitting in the app runs in the background.
- Page labels (page numbering such as i, ii, 1, 2 or A-1): shown in the status bar, thumbnails and bookmark tips, set
  from the selected pages in the inspector, and listed, set and removed with `mpe labels`. Labels stay with their
  pages when pages are deleted, inserted, moved, extracted or appended.
- Picture watermarks: a PNG, JPEG or TIFF picture centred on each page, sized as a share of the page, with opacity,
  rotation, and above or behind the content. In the app's Watermark section (Text or Picture) and as
  `mpe watermark --image FILE [--scale PERCENT]`. The picture is stored once, however many pages it is on.

### Changed

- New rectangles and ellipses have no fill; turn Fill on in the inspector to fill them.
- In the app, after adding a watermark or headers and footers, the section's Update button changes what was just
  added instead of stamping another copy on top; Add another keeps it and adds a second.
- The Windows installer shows its steps, says in plain words that MPE is free and open source, and lets you choose
  where to install MPE.

### Fixed

- Search finds words hyphenated at a line end, with or without the hyphen, in the app and in `mpe search`.
- Search highlights follow the page's reading order, as the results list does, so every match listed is highlighted
  and stepping through results goes to the right place.
- On Windows, a console window no longer opens behind the app.
- The start screen card's outline stays lit while the pointer is over its buttons and recent files, instead of
  flickering.
- Two watermarks with different opacities on the same page each keep their own opacity.
- Headers, footers, watermarks and OCR text no longer break the page's own text and images on pages that take their
  resources from the page tree.
- A malformed file that makes MPE fail while opening, editing or saving shows an error instead of closing the app.
  A failed edit leaves the document as it was. A failure while showing a document (pages, thumbnails, search,
  bookmarks, comments, the inspector) puts an error page in that tab, with Save a copy and Close, instead of
  closing the app with all its tabs.
- Release builds check integer arithmetic for overflow, so a malformed file stops with an error instead of producing
  wrong results.

## [0.5.0] - 2026-10-15

Installable packages and signed releases: a Windows installer, a macOS app, Linux packages with apt and dnf
repositories, and GPG signatures and build attestations for every download. Saving is also safer.

### Packages and signed releases

- Releases are signed: every file and `SHA256SUMS` has a GPG signature (`.asc`) made with MPE's own key, and a build
  attestation that `gh attestation verify` checks. [docs/install.md](docs/install.md#checking-a-download) explains
  how.
- Linux `.deb` and `.rpm` packages (the rpm GPG-signed), installing both programs, the menu entry, the icon and the
  docs, with the libraries the app needs as dependencies.
- Signed apt and dnf package repositories at https://smaug42.github.io/mpe/, for Debian, Ubuntu, Fedora and
  openSUSE, so MPE updates with the rest of the system.
- A macOS app, `MPE.app` in a `.dmg`, for Apple Silicon and Intel in one, listed under Open With for PDFs. The
  command-line tool is inside it.
- A Windows installer (`.msi`): Program Files, a Start menu entry, Open with for PDFs (and a choice in Default
  apps), `mpe` on the `PATH`, upgrades and an uninstaller.

### Saving and safety

- Saving a digitally signed document asks first, since the saved copy's signatures will show as invalid. Saving as
  a new file keeps the signed original. Core: `Pdf::signature_count`.
- Saving is safe: MPE writes to a temporary file beside the document and then replaces it, so a failed save (a
  full disk, an error part way) leaves the original untouched. Saving through a symbolic link keeps the link, and
  a replaced file keeps its permissions. Applies to the app and every command.
- A page's content, a drawing inside a page and text stored as a stream are decompressed up to 256 MB, so a small
  crafted file can't use up all memory when MPE reads its text.

### Desktop app

- "New from images…" is now "Make a PDF from images…", on the start screen and in the menu, with a hover tip on the
  start screen.

## [0.4.0] - 2026-10-14

Annotations and forms: highlight, comment on and draw on documents, reply to comments, fill in and flatten forms,
follow links and bookmarks, and select text without changing tools.

### Annotations

- A Comments tab beside Bookmarks lists the document's annotations (comments, notes, highlights, drawings) page by
  page, with replies under what they answer. Click one to go to it; right-click to delete it. ☰ > Document >
  Flatten annotations… makes them part of the pages. See [Comments and markup](docs/comments-and-markup.md).
- Highlight selected text (the Highlight button above the selection, right-click, or Ctrl+Shift+H), or underline,
  strike it out or squiggly-underline it, in colours that are remembered. The marks are standard annotations that
  other readers show. Click a mark or comment to select it; the inspector changes its colour, opacity and comment,
  and Delete removes it. Marks carry your name, the computer's user name unless set under ☰ > Edit > Your name for
  comments…. See [Highlight text](docs/comments-and-markup.md#highlight-text).
- Sticky notes and text boxes: the new Annotate button in the top bar holds both tools. Click to place a note and
  type its comment beside it; drag a text box and type straight onto the page. Double-click either to change its
  text. Text boxes on turned pages read upright. See [Notes and text boxes](docs/comments-and-markup.md#add-a-note-or-a-text-box).
- The arrow tool selects, moves and resizes annotations (notes keep their size; text markup stays on its text), and
  the arrow keys nudge them.
- The rectangle, ellipse and line tools now add annotations, and have moved into the Annotate button's menu
  with a new pen for freehand drawing (strokes made in quick succession form one drawing). Lines can have an
  arrowhead, and Shift keeps squares, circles and 45° lines. "Draw on page (permanent)" in the inspector keeps
  the old behaviour of drawing into the page content. A selected shape, line or drawing has its width, fill and
  arrowhead in the inspector. See [Shapes, lines and drawings](docs/comments-and-markup.md#draw-shapes-lines-and-freehand).
- A selected line or arrow has handles on its two ends instead of a box, so it can be turned and pointed
  elsewhere with the arrow tool. A selected freehand drawing has a rotate handle.
- Reply to an annotation from the Comments tab (right-click > Reply…) or the inspector, which also lists the
  replies it has.
- Annotations stored without a drawing of their own, which used to be invisible, are drawn: highlights,
  underlines, strike-outs, squiggly underlines, notes, text boxes, rectangles, ellipses, lines, polygons and
  freehand drawings. The file is left as it is.

### Forms, links and text

- Fill in forms: click a field with the read tool to type in it, tick it or choose from its
  options; Tab moves between fields. Each change is one undo step. ☰ > View > Highlight fields shows every field,
  and the inspector's Form section has Clear field, Reset form and Flatten form…, which makes the fields part of
  the pages so the values can't be changed (Undo brings them back). Fields with a value but no drawing of their own
  now show their value. See [Filling in forms](docs/forms.md).
- Forms made only for Adobe Reader (XFA forms, which show a "Please wait..." page) get a notice explaining that
  only Adobe software can show or fill them; `mpe form list` and `mpe form fill` say the same.
- Links work: click one with the read tool (dragging still selects text), or Ctrl+click with the editing tools.
  Links to pages go there; links to web and email addresses show the address and ask before opening it in your
  browser or email program.
- A Bookmarks tab beside Pages and Search shows the document's outline. Click a bookmark to go to its page.
  ☰ > View > Bookmarks and the command palette open it.
- Select text without changing tools: the hand tool and the text cursor are now one read tool, which every
  document opens with. Dragging anywhere on a page selects text from the nearest word; dragging past the edge of
  the view scrolls it, so a selection can run across pages. Triple-click selects a line and Shift+click extends
  the selection. See [Copying text](docs/reading-and-finding.md#copy-text).
- Print or save just the selected text: right-click it, or use ☰ > Edit. Choose the pages as they look, full width but
  cut down to the selected lines, or the words alone as plain text; saving also offers a text file. See
  [Copying text](docs/reading-and-finding.md#copy-text).
- Selecting, copying and exporting text follow the order on the page, not the order the file draws it in, so a
  selection no longer jumps when it reaches a block drawn out of place (common in forms), and exported text comes
  out in reading order. See [How text is read](docs/reading-and-finding.md#how-text-is-read).

### Desktop app

- Pages fit the window's width by default and keep fitting as the window is resized or the side panels open and
  close. Zooming turns fitting off; the Fit width button or Ctrl+0 turns it back on. Zooming keeps your place in
  the document.
- Recent files on the start screen and under ☰ > File > Open recent.
- Notices above the pages when a document is password-protected, has scanned pages without text, has form fields,
  or is large, with a button to deal with it where there is one.
- Crop pages from the inspector, by margins from each side of the page.
- A Print button in the top bar. The Export menu now holds only the exports.
- A theme button in the top bar to choose System, Light or Dark, remembered between runs.
- Applying password security can be undone.
- The inspector's heading and the button that hides it share one row.
- MPE has its own icon, on the window, the taskbar and, on Windows, `mpe-gui.exe` itself. The Linux archive
  includes a desktop entry and icon, which Wayland desktops need to show it; see
  [Install](docs/install.md#menu-entry-and-icon-on-linux).

### Command line

- `mpe form list` lists a form's fields with their kinds, values, pages and flags; `--json` prints the values as a
  JSON object.
- `mpe form fill` fills in text fields, checkboxes, radio buttons, drop-downs and lists, from `--set NAME=VALUE`
  or a JSON file like the one `form list --json` prints, and `mpe form reset` puts fields back to their defaults.
  MPE draws the filled fields itself, so the values show in every reader. See [Forms](docs/cli.md#forms).
- `mpe form flatten` draws a form's fields into its pages and removes them (`--pages` for some pages only), and
  `mpe form fill --flatten` fills in and flattens in one go.
- `mpe annot list` lists annotations (also `--json`), `mpe annot remove` removes them, all or by type, author and
  page, and `mpe annot flatten` draws them into the pages. See [Annotations](docs/cli.md#annotations).
- `mpe render` draws pages as the app does: annotations marked not to be shown on screen are left out, and
  annotations without a drawing of their own are drawn.

### Fixes

- Search highlights and selected words sit on the text: characters are placed by their widths in the font instead
  of being spread evenly, which put highlights a letter or more off in proportional fonts. Files that use the
  standard PDF fonts without their widths are measured with Adobe's published widths.
- Text is read from many more files correctly. ToUnicode maps that the document library rejected (LibreOffice
  writes them, for one) are now read, where before the text came out as nonsense and could not be searched.
  Export text, `mpe text` and search now read the page the same way as highlights and copying.
- Words separated only by a gap, with no space character, keep their space in extracted text, search and copying,
  including in tightly justified lines and headings.
- Ligatures such as "ﬁ" come out as their letters, so searches find words written with them.
- A phrase that runs across a line break is highlighted on both lines; before, it was listed but not highlighted.
- Text inside form XObjects (blocks of page content that some PDF makers put all their text in) is now read, so it
  can be searched, selected, copied and exported. Pages whose only text is there are no longer taken for scans.
- Headers, footers and watermarks measure accented Latin-1 letters by their real widths, so centred and
  right-aligned text containing them is placed correctly.
- Long names in the start screen's recent files no longer run past the card.

### Testing

- Printing on Windows, built in 0.3.0 but not run then, has been tried and works.

### Other changes

- Search takes about twice as long as before, as it now reads pages with MPE's own text reader: 0.17 s for a
  477-page manual. See [Performance](docs/developer/performance.md).
- Release notes on GitHub end with a link to every change since the previous version.
- Installers and packages (a macOS app, Linux packages, Homebrew) are planned for once the app is more complete;
  see [Features and status](docs/developer/feature-status.md#packaging).

## [0.3.0] - 2026-10-13

MPE is still a test release: most testing has been on Linux. See
[Features and status](docs/developer/feature-status.md) for what works and its limits.

### Desktop app

- A redesigned window: a top bar with the tools and an Export menu, the Inter font, Phosphor icons, and light and
  dark themes that follow the system.
- An inspector on the right that follows the selection: document settings when nothing is selected, page actions
  for selected pages, and an object's or tool's options. It replaces most dialogs, and collapses to a rail on narrow
  windows.
- A command palette (Ctrl+K), right-click menus on pages, objects, words and thumbnails, and a floating toolbar over
  a selected object or words.
- Drag thumbnails to reorder pages. Delete removes the selected pages or object, with an Undo button in a notice.
- An Open button, and a start screen for opening files or starting from images.

### Scanning and OCR

- Import PNG, JPEG and TIFF images as pages, sized from their resolution and turned upright.
- Optimise scanned pages: store black and white as CCITT G4 and greyscale and colour as JPEG, straighten tilts and
  remove specks.
- Recognise text (Latin script) with a built-in model, adding an invisible, searchable text layer. Recognised text
  can be shown, corrected, selected and copied.

### Text

- Search highlights matches on the page and scrolls to them.
- Select and copy text with the text tool, including recognised text.

### Export and printing

- Export pages as PNG or JPEG images, and the document's text as a `.txt` file.
- Print through CUPS on Linux and macOS, and through the print spooler on Windows. Windows printing has been
  compiled but not yet run, and printing has not yet been tried with a real printer.

### Fixes

- AES-256 files saved by MPE now open in viewers based on poppler, such as Okular and Evince. AES-256 files saved by
  0.2.0 still need to be saved again with the password set; see [Password security](docs/security.md).
- Some encrypted files saved by MPE (about 1 in 2,500; 1 in 550 with AES-256) could not be opened by
  `mpe render --password`. Fixed for files saved from this version.
- Annotations flagged NoView are no longer shown in the desktop app.

### Other changes

- Building MPE now needs Rust 1.95 or newer (was 1.80).
- The command line gains `images`, `insert-images`, `optimize-scans`, `ocr`, `printers` and `print`; `render` can
  write JPEG; `search --boxes` prints where matches are.
- The documentation moved into [docs/](docs/); the README is now a landing page.

## [0.2.0] - 2026-10-12

First test release: viewing, page operations, headers, footers, Bates numbers and watermarks, document properties,
password security, and basic content editing (text, images and shapes), in a desktop app and a command-line tool.

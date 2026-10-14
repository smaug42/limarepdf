# Using the desktop app

![The MPE window with a three-page report open](images/mpe-gui.png)

## Opening files

Open PDFs with the Open button, Ctrl+O, or by dropping them on the window. Image files (PNG, JPEG, TIFF) open as a new
document with one page per image. From a terminal, `mpe-gui file.pdf --page 3` opens a file at page 3.

Recently opened PDFs are listed on the start screen and under ☰ > File > Open recent, which also clears the list.
Files that no longer exist are left out. The list holds 10 files and is kept on your computer only, in
`~/.config/mpe/recent.txt` on Linux, `~/Library/Application Support/mpe/recent.txt` on macOS, and
`%APPDATA%\mpe\recent.txt` on Windows.

Each document gets a tab. The tab shows its name, with `*` while it has unsaved changes.

## The window

- **Top bar:** the menu (☰), Open, Save, Undo and Redo, the tools, zoom, search, Print, an Export menu (pages as
  images, text), and the theme. On narrow windows the search field and the tools fold into single buttons.
- **Left:** three tabs. Pages shows the thumbnails, Search the search results, and Bookmarks the document's
  outline. Click a bookmark to go to its page; the arrow beside one shows or hides the bookmarks inside it, which
  start shown or hidden as the file asks. Bookmarks that go to another file or a web address are greyed out. They
  can't be edited yet. ☰ > View > Bookmarks, or "Bookmarks" in the command palette, opens this tab.
- **Middle:** the pages, scrolling continuously. By default they fit the width between the side panels, and keep
  fitting as you resize the window or open and close the panels. Zooming in or out, actual size, Ctrl+scroll or a
  pinch turns fitting off, and your zoom then stays as it is; the Fit width button (highlighted while fitting is on)
  or Ctrl+0 turns it back on.
- **Right:** the inspector, which shows options for what is selected. On windows narrower than about 960 pixels it
  collapses to a thin rail; click the rail to slide it out. ☰ > View > Inspector shows or hides it.

Ctrl+K opens the command palette, which lists every command. Type to filter, use Up and Down to choose, and press
Enter to run.

## Links

Click a link with the hand or text-select tool to follow it: the pointer turns into a hand over one, and hovering
shows where it goes. Dragging from a link with the text-select tool selects text as usual. With the editing tools,
hold Ctrl (Cmd on macOS) and click, so links don't get in the way of editing.

- A link to another page goes there.
- A link to a web or email address asks first, showing the full address, and then opens it in your browser or
  email program. MPE itself never connects to the network. Links to anything else, such as a file or a program,
  are shown but not opened.
- Links that go to another PDF file are not followed. Links can't be added or edited yet.

## Filling in forms

Form fields work with the hand and text-select tools, like links: hovering tints a field and shows its description,
and a click fills it in. With the editing tools, hold Ctrl (Cmd on macOS) and click.

- **Text fields:** a click opens a text box over the field. Enter (in a one-line field) or clicking elsewhere keeps
  what you typed; Esc drops it. Password fields show dots, and their value is not kept in the file.
- **Checkboxes and radio buttons:** a click ticks or chooses.
- **Drop-downs and lists:** a click opens the options. In a list that allows several choices, each click adds or
  removes one; Done closes it. A drop-down you can type in has a text box at the top; Enter keeps the text.
- **Tab and Shift+Tab** move to the next and previous field, top to bottom and page by page. On a checkbox or radio
  button, Space ticks it; on a list, Space or Enter opens it.

Each value you change is one undo step. ☰ > View > Highlight fields tints every field and outlines the required ones
in red. The inspector's Form section shows the field you are in (its name, description and whether it is required),
with Clear field, Highlight fields and Reset form, which puts every field back to its default. Read-only fields,
buttons (they run scripts, which MPE doesn't) and signature fields can't be filled in; hovering says why.

MPE draws the filled fields itself, so the values show the same in other readers. Text must be Latin-1 (Western
European) for now; anything else is refused with a message rather than saved wrongly. Fields that have a value but
no drawing of their own are drawn when the file opens. See [Command line](cli.md#forms) for filling forms in
scripts.

## Notices about the document

When a document opens, notices above the pages point out things worth knowing, each with a button where there is
something to do and × to dismiss it. At most two show at a time.

| Notice | Button |
| --- | --- |
| The document was password-protected: saving removes the protection unless you set it again | Password security |
| Its pages are scans without text, so they cannot be searched or copied (checked on the first 20 pages) | Recognise text |
| It has a form: click a field to fill it in, or press Tab | Highlight fields |
| It is large (more than 500 pages or 100 MB) | |

The first two go away by themselves once dealt with: after password security is applied, or after text has been
recognised.

## Selecting and the inspector

What you select decides what the inspector, the right-click menu and the floating toolbar offer.

| Selected | Inspector |
| --- | --- |
| Nothing | The document: properties, password security, crop, watermark, headers/footers/Bates, OCR, optimise scanned pages, export pages as images, split into files, and buttons for export text, print and append PDFs |
| Pages (click thumbnails; Shift for a range, Ctrl for one more) | Rotate, delete, extract to a new tab, insert images or a blank page after them, and the crop, watermark, headers/footers, OCR and optimise tools with the pages filled in |
| An object, words, or a tool that adds content | That object's or tool's options |

Menu commands for document and page settings open the matching section of the inspector.

Crop trims margins from each side of the page as it is shown, measured from the full page; the fields start with
the first page's current crop. Remove crop shows the whole page again. Cropping hides content but does not delete
it.

Right-click a page, an object, selected words or a thumbnail for a short menu of what applies there. Right-clicking
selects what is under the pointer first. A selected object or run of words also gets a small floating toolbar with
the same commands.

Drag thumbnails to put pages in a new order. A line shows where they will go. Dragging a selected thumbnail moves all
the selected pages.

Delete removes the selected object or pages. Deleting shows a notice with an Undo button.

## Editing page content

Pick a tool in the top bar. The hand is for reading only; the arrow selects.

With the arrow:

- Click an object to select it.
- Drag to move it; drag a handle to resize it (Shift keeps the proportions).
- Arrow keys move it by 1 point, or 10 with Shift.
- The inspector shows its position and size, and for text the string, font, size and colour; for shapes the fill,
  outline and width; for images "Replace image".

The other tools add content:

- **Text:** click where the text starts, type, press Enter.
- **Rectangle, ellipse, line:** drag, or click for a default size.
- **Image:** choose a PNG, JPEG or TIFF, then drag a box.

Everything that changes the document can be undone with Ctrl+Z. See [Content editing](content-editing.md) for how
editing works and its limits.

## Copying text

With the text-cursor tool, drag over words to select them in reading order, double-click to select one word, or
press Ctrl+A for the whole page in view. Esc clears the selection. Copy with Ctrl+C, ☰ > Edit > Copy text or the floating
toolbar: words on a line are separated by spaces and lines by line breaks. Recognised (OCR) text can be selected
like any other text. A selection stays on one page.

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
| Delete, Backspace | Delete the selected object or pages |
| Arrow keys (Shift for 10 points) | Move the selected object |
| Ctrl+A, Ctrl+C | Select all text on the page, copy it (text tool) |
| Esc | Cancel: clear the selection, or leave a drawing tool |
| Tab, Shift+Tab | Next, previous form field; Space ticks a checkbox or opens a list |

## Look and feel

The app follows the system's light or dark theme. The theme button at the right end of the top bar (a half circle,
sun or moon, for the current choice) chooses System, Light or Dark; so does ☰ > View > Theme. The choice is
remembered between runs, in `settings.txt` in the same folder as the recent-files list. The app uses the Inter font
and Phosphor icons.

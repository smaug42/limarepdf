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

- **Top bar:** the menu (☰), Open, Save, Undo and Redo, the tools, zoom, search, and an Export menu (export pages as
  images, export text, print). On narrow windows the search field and the tools fold into single buttons.
- **Left:** page thumbnails, or search results.
- **Middle:** the pages, scrolling continuously.
- **Right:** the inspector, which shows options for what is selected. On windows narrower than about 960 pixels it
  collapses to a thin rail; click the rail to slide it out. ☰ > View > Inspector shows or hides it.

Ctrl+K opens the command palette, which lists every command. Type to filter, use Up and Down to choose, and press
Enter to run.

## Notices about the document

When a document opens, notices above the pages point out things worth knowing, each with a button where there is
something to do and × to dismiss it. At most two show at a time.

| Notice | Button |
| --- | --- |
| The document was password-protected: saving removes the protection unless you set it again | Password security |
| Its pages are scans without text, so they cannot be searched or copied (checked on the first 20 pages) | Recognise text |
| It has form fields, which MPE cannot fill in yet | |
| It is large (more than 500 pages or 100 MB) | |

The first two go away by themselves once dealt with: after password security is applied, or after text has been
recognised.

## Selecting and the inspector

What you select decides what the inspector, the right-click menu and the floating toolbar offer.

| Selected | Inspector |
| --- | --- |
| Nothing | The document: properties, password security, watermark, headers/footers/Bates, OCR, optimise scanned pages, export pages as images, split into files, and buttons for export text, print and append PDFs |
| Pages (click thumbnails; Shift for a range, Ctrl for one more) | Rotate, delete, extract to a new tab, insert images or a blank page after them, and the watermark, headers/footers, OCR and optimise tools with the pages filled in |
| An object, words, or a tool that adds content | That object's or tool's options |

Menu commands for document and page settings open the matching section of the inspector.

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
| Ctrl+0, Ctrl+1 | Fit width, actual size |
| Ctrl+scroll, pinch | Zoom |
| PageUp, PageDown, Home, End | Previous, next, first, last page |
| Delete, Backspace | Delete the selected object or pages |
| Arrow keys (Shift for 10 points) | Move the selected object |
| Ctrl+A, Ctrl+C | Select all text on the page, copy it (text tool) |
| Esc | Cancel: clear the selection, or leave a drawing tool |

## Look and feel

The app follows the system's light or dark theme; ☰ > View > Theme overrides it. It uses the Inter font and Phosphor
icons.

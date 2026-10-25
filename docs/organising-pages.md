[User guide](README.md) › Everyday tasks

# Organising pages

Put pages in a new order, turn them, add and remove them, take some out into a new document, join documents and
split one into several files. Every change here is one step to undo with Ctrl+Z.

## Select pages

Click a page's thumbnail in the Pages view on the left to select it. Shift+click selects a range, and Ctrl+click
adds or removes one page. The inspector on the right then shows what you can do with the selected pages; so does
right-clicking a thumbnail. ☰ > Pages has the same commands, which work on the page you are on when no pages are
selected.

## Reorder pages

Drag thumbnails to put pages in a new order. A line shows where they will go. Dragging a selected thumbnail moves
all the selected pages.

## Rotate, delete and extract pages

Select the pages, then in the inspector:

- **Rotate clockwise** or **Rotate counter-clockwise** turns them a quarter turn.
- **Delete pages** removes them (so does the Delete key). A notice with an Undo button follows.
- **Extract pages to a new tab** opens a copy of them as a new document, to save on its own. The original keeps
  them.

## Add pages, and merge documents

- **A blank page:** select the page to insert after, open **Insert blank page** in the inspector, pick Letter, A4
  or Legal (or type a width and height in points), and press **Insert after page N**. ☰ > Pages > Insert blank page
  inserts one the size of the page you are on.
- **Pictures:** **Insert images…** adds PNG, JPEG or TIFF pictures after the selected pages, one page each (see
  [Scans and OCR](scanning-and-ocr.md#importing-images)).
- **Other PDFs:** **Append PDFs…** (☰ > Pages, or the button at the bottom of the inspector when nothing is
  selected) adds the pages of the PDFs you choose at the end of the document. To merge documents, open the first
  one and append the others; drag thumbnails afterwards to put the pages elsewhere.

The files are added in the order chosen, in one step that Ctrl+Z takes back. A password-protected file asks for its
password: **Add pages** adds its pages, **Skip this file** leaves it out and adds the others, and **Cancel** adds
nothing. A file that can't be read adds nothing either, and a message names it.

Appending adds the pages only: the bookmarks and named destinations of the files added aren't carried over, and
LimarePDF can't combine forms yet. When a file added has form fields, **Add pages with form fields?** asks first:
**Add pages** draws its fields onto its pages with their answers, as they look now, so they show everywhere but
can no longer be filled in; **Cancel** adds nothing. The document's own fields stay fillable. If LimarePDF can't
draw an answer (text in characters it can't write in a field yet, say), nothing is added, and a message names
the field and the file. Pages from a password-protected file make the whole document count as protected, so it
gets no [recovery copies](gui.md#recovery-copies).

## Split into files

1. Clear any selection (Esc), then open **Split into files** in the inspector (or choose ☰ > Pages > Split into
   files…).
2. Choose **Pages per file** and how many, or **At bookmarks, levels** and how many levels of bookmarks to split at
   (1 is the top level only).
3. Press **Choose folder…** and pick where the files go.

Files are numbered after the document, such as `book-001.pdf`; split at bookmarks, they are named after their
bookmark too, such as `book-002 Introduction.pdf`, and the pages before the first bookmark get a file of their
own. Each file is written as soon as its part is made, so a large document split into many files needs little
more memory than the document itself. Splitting doesn't change the document.

If files with the same names are already in the folder, LimarePDF asks once, saying how many, before writing
anything: **Replace** replaces them, **Cancel** writes no files at all.

## Remove blank pages

☰ > Pages > Remove blank pages (or the command palette) deletes every page that is one even colour, give or take
scanner specks, faint show-through or a lone page number, and lists the pages it removed. Undo brings them back. A
document whose pages are all blank is left alone.

## Crop pages

Crop trims margins from the pages as they are shown.

1. Select the pages to crop (or none, for every page), and open **Crop** in the inspector (or choose ☰ > Pages >
   Crop pages…). The pages are shown whole, with the part outside each page's crop box dimmed.
2. Drag the crop box's edges or corners on the page, or type the margins. Each drag or number crops all the pages
   in the Pages box at once, and dragging back out undoes a crop.
3. Close the section, or press Esc, to see the pages cropped.

While the section is open, clicks on the pages only move the crop box. The margin fields start with the first
page's crop, measured from the full page. **Remove crop** shows the whole page again.

Cropping hides content; it doesn't delete it. To remove what is outside the crop box for good, use
[Remove hidden information](redaction.md#removing-hidden-information).

## Number pages your own way

Page labels are the page numbers viewers show, such as i, ii, 1, 2 or A-1, for a document whose front pages are
numbered apart from the rest. They don't change what is printed on the pages.

1. Select the first page to number, and open **Page labels** in the inspector (or go to the page and choose ☰ >
   Pages > Page labels…).
2. Pick a style, an optional prefix, and the number to start at.

The numbering runs to the next labelled range, and each change applies at once. The section shows the numbering
the page has now. When a document has labels, the status bar shows the label with the page's place, such as
"Page i (3 of 477)", and the thumbnails show the labels. Labels stay with their pages when pages are deleted or
moved.

## What removed pages leave behind

Pages that are deleted, or left out when extracting or splitting, leave nothing in the saved file: bookmarks,
named destinations, links, the opening view, form fields and tags stop referring to them, so their content isn't
kept.

- A link to such a page goes nowhere, and a form field whose boxes were all on those pages leaves the form.
- Tags with content on those pages lose their alternative text (also when they have content on kept pages too),
  and so do the tags above them.
- An extracted file or a split part keeps the bookmarks to its own pages; a bookmark to another page stays, without
  a destination, only when bookmarks below it stay. Deleting pages keeps every bookmark, without a destination when
  its page went.
- A form nested more than 32 deep can't be followed to its end, so deleting, extracting or splitting stops with a
  message saying why, changing nothing.

## From the command line

`limarepdf-cli` has `extract`, `delete`, `rotate`, `reorder`, `insert-blank`, `remove-blank`, `crop`, `merge`,
`split` and `labels`: see [Pages](cli.md#pages) and [Page labels](cli.md#page-labels) in the command-line
reference.

---

[← Reading and finding](reading-and-finding.md) · [User guide](README.md) · [Comments and markup →](comments-and-markup.md)

[User guide](README.md) › Everyday tasks

# Headers, footers, watermarks and stamps

Put page numbers, a date or other text at the top or bottom of every page, number pages for legal work (Bates
numbers), mark pages DRAFT or CONFIDENTIAL with a watermark, or put a stamp such as APPROVED on a page.

Headers, footers and watermarks become part of the pages. Stamps are annotations, which can be moved or removed
later.

## Add page numbers, headers and footers

1. Select the pages to number (or none, for every page), and choose **Insert > Header and footer** (or ☰ >
   Document > Headers, footers, Bates…). **Headers, footers, Bates** opens in the inspector.
2. Type in the header and footer boxes: each has a left, a centre and a right part. The footer's centre starts as
   `Page {page} of {pages}`, which gives "Page 3 of 12".
3. Check the pages: they show the headers and footers as you type.
4. Press **Add to pages**.

These words in curly brackets are filled in on each page:

| Type | To get |
| --- | --- |
| `{page}` | The page's number, counted from 1 |
| `{pages}` | How many pages the document has |
| `{date}` | Today's date on your computer, such as 2026-10-23 |
| `{bates}` | The page's Bates number (below) |

**Font size** sets the text's size (10 points at first), and **Margin** how far it sits from the page's edges (36
points, half an inch, at first). The text is black Helvetica.

### Bates numbers

Bates numbers give every page of a set of documents its own number, such as `ACME-000123`. Tick **Bates
numbering**, then set:

- **Prefix** and **Suffix:** text before and after the number.
- **Digits:** how many digits the number has, with zeros in front (6 at first, up to 12; `limarepdf-cli bates
  --digits` takes up to 20).
- **Start:** the first page's number (1 at first). The numbers count the pages in the Pages box, from the first.

Put `{bates}` where the number goes; without it, the number goes at the bottom right. Numbers go up to
18446744073709551615; a set that would go past it is refused, changing nothing.

## Add a watermark

1. Select the pages (or none, for every page), and choose **Insert > Watermark** (or ☰ > Document > Watermark…).
   **Watermark** opens in the inspector.
2. Choose **Text** and type it (it starts as DRAFT), or **Picture** and choose a PNG, JPEG or TIFF file with
   **Choose…**.
3. Set its **Size** (for text, the letters' size; for a picture, how much of the page it may fill), **Opacity**,
   **Rotation** and, for text, **Colour**.
4. Choose **Above the content** or **Behind the content**.
5. Check the pages, then press **Add to pages**.

The watermark is centred on each page.

## Change it after adding

While the Watermark or Headers, footers, Bates section is open, the pages in its Pages box show what it describes as
you set it, drawn over them. Nothing is added to the document until you press Add to pages.

After that, the section says "Changes update the watermark just added" (or the headers and footers): each change
updates what you added instead of adding a second copy, one undo step each (a colour when its picker closes). Undo
steps back through the changes, and the section's settings follow. **Add another** keeps it and adds a new one.
To do this, LimarePDF keeps a copy of the document as it was before you added it, counted in the [undo
limit](gui.md#undo). On a document so large that the limit has no room for that copy, a message says the section
can't change what was added any more; what was added stays on the pages as last changed.

Any other change to the document ends this: the section shows Add to pages again, and the pages show the settings
again once you change one. From then on, what was added is part of the pages: no command removes it from all its
pages. Undo takes it back until you close the document, also after saving, and on each page its text and pictures
can be edited or deleted like any other page content (see [Move, resize or
delete](content-editing.md#move-resize-or-delete)).

## Characters

Header, footer, Bates and watermark text is written in Helvetica, and characters it lacks (Polish, Czech,
Vietnamese, Greek, Cyrillic and so on) in its Liberation Sans face, embedded in the file. Other scripts (Chinese,
Hebrew, Arabic and so on) and emoji can't be typed or pasted: a note at the caret says so.

While a section is open, the pages show its text in Liberation Sans, which has Helvetica's widths, over the content
even when it is set to go behind it. Once added, the pages show it as written.

## Stamps

A stamp is a word in a coloured frame, such as APPROVED or DRAFT, or a picture such as a company seal, put on one
page as an annotation.

1. Pick **Review > Stamp** ("Add a stamp" in the command palette).
2. In the inspector, pick a stamp.
3. Click on the page to put it down at its own size, centred on the click, or drag a box to size it (it keeps its
   shape inside the box).

The read tool then comes back with the stamp selected, to move or resize it. To put down several, pin the tool
(see [Keep tool active](gui.md#keep-tool-active)).

The inspector offers:

- **Ready-made stamps:** Approved, Not approved, Draft, Final, Confidential, For comment and Void, each a coloured
  frame with the word in capitals.
- **Add my name and the date:** on by default; adds a small line under the word with your name and today's date.
  The Name box under it is the same name as ☰ > Edit > Your name for comments, so changing either changes both. The
  date is today's on your computer, as headers and footers use. The name and date are fixed when the stamp is put
  down. The choice is remembered.
- **Your own wording:** pick green, red or blue, type the wording in the box under the ready-made stamps, then click
  on the page. It is shown in capitals, and goes into the list above when you leave the box (Enter or a click
  elsewhere). Picking a colour also recolours the picked wording of your own. The last five wordings are remembered
  with their colours (right-click one to take it off the list). The stamp's font covers Latin, Greek and Cyrillic,
  so wording with other characters (Chinese, Hebrew and so on) can't be used yet; the pane says which characters.
- **Pictures:** **Choose a picture…** takes a PNG or JPEG file; it is put down with its longer side 160 points.
  The last five pictures are remembered by where they are (right-click one to take it off the list). Transparent
  parts of a PNG stay transparent.

Stamps are annotations: other readers list them as comments. Select one to change its opacity and comment (its
colours are part of the stamp), or press Delete to remove it. A stamp reads upright on a turned page. The text is
Helvetica Bold, or its Liberation face, embedded in the file, for characters it lacks; a name it can't show (in
Chinese, say) is left off the name-and-date line. To make stamps part of the page, flatten them with the other
comments ([Flatten](comments-and-markup.md#flatten-comments-into-the-pages)).

## From the command line

`limarepdf-cli header-footer`, `bates`, `watermark` and `stamp` do the same: see [Stamps](cli.md#stamps).

---

[← Fill and sign](fill-and-sign.md) · [User guide](README.md) · [Scans and OCR →](scanning-and-ocr.md)

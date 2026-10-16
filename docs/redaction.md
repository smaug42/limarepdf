# Redaction

Redaction removes content from a document for good. It doesn't just cover it: the text, pictures and drawings under
a redacted area are taken out of the file, and the area is painted black. Copying, searching, moving the black box
or reading the file's bytes can't bring them back.

In the app, mark areas, selected text or search results, check the marks, then apply them (see
[Redaction in the app](gui.md#redaction)). From the command line, use `mpe redact` (see
[the command line](cli.md#redaction)), which can also just mark, or apply marks already in a file. Marks are
standard PDF redaction marks, so marks made in other apps can be applied in MPE and the other way round.

## What is removed

For each area:

- **Text**, visible or invisible (such as a scanned page's recognised text layer): each character at least a quarter
  under the area is removed. The characters left keep their exact places on the line.
- **Pictures**, scanned pages included: the pixels under the area are blacked out in the picture itself, which is
  then saved again (JPEG pictures as JPEG at about their own quality, black-and-white scans as CCITT, others without
  loss). Pixels only partly under the area are blacked out too. A picture's transparency (soft mask) is cut the same
  way. A picture shared with other pages or drawn elsewhere on the page is copied first, so only the covered copy
  changes. Pictures MPE can't read (JPEG 2000, JBIG2, CMYK JPEG, some unusual formats) are removed whole, and the
  report counts them.
- **Drawings**: the parts of a drawing mostly under the area are removed. Lines and fills that only cross the
  area's edge (a table border, a coloured background) are kept but cut out of the area. Colour gradients are cut
  out the same way.
- **Grouped content** that several pages share (a logo, a header) is redacted inside, on this page only; other pages
  keep it as it was.
- **Comments, links and form fields** touching the area are removed, with the replies to them. A removed field loses
  its value.
- **Alternative text** that accessibility tools and copying use, on content that lost anything.
- The page's **thumbnail picture** and private application data.

Pictures and drawings that no page uses any more are left out of the saved file.

## The check afterwards

When it is done, MPE reads the redacted pages again and checks that no text is left under the areas, and decodes
each picture there to check that its pixels under the areas are blacked out. If anything is left, the app changes
nothing and says what it found, and `mpe redact` lists it and doesn't write the file.

It then looks for the removed text elsewhere in the document: on any page, in bookmarks, comments, form fields and the
document's title, author, subject and keywords. Anything it finds is listed as a warning, since redaction only
changes the areas you give it.

## Removing hidden information

A file can carry information that isn't on any page. Document > Remove hidden information… (or `mpe sanitise`)
removes it. The app lists what this document has of each kind; what is ticked is removed:

- **Document information and XMP metadata:** title, author, subject, keywords, the program that made it, and the
  XML metadata in the document and in pictures and other parts.
- **JavaScript and actions that run by themselves:** document scripts, actions run on opening, closing, saving or
  printing, form field scripts (formatting, checking, calculating), and actions that launch programs, send form data,
  import data or open attached files. Ordinary links to pages and web addresses stay. A form with both XFA and
  ordinary fields loses its XFA part (it holds scripts); a form made only for Adobe Reader (XFA only) keeps it.
- **Attached files:** files attached to the document or to comments.
- **Hidden layers and their content:** the content of layers that are switched off is deleted, and those layers
  leave the layer list. Layers that are shown stay.
- **Thumbnails and private application data:** page thumbnail pictures, data other programs keep for themselves
  (such as Illustrator's), and web capture information.

Not ticked at first, as they are often wanted:

- **Comments and markup** (redaction marks stay).
- **Form field values:** fields go back to their default values.
- **Bookmarks.**
- **Content outside the visible page area:** what lies outside each page's crop box is redacted and the page becomes
  the size it is shown at.

Nothing else is kept: saving rewrites the whole file, leaving out everything no longer used, and earlier versions
of the file aren't kept in it.

## Limits

- The width of the gap shows how long the removed text was.
- Fonts embedded in the file keep the shapes of the letters they contain, including removed ones (not the text
  itself).
- Text in fonts whose characters can't be told apart reliably (some Asian-language encodings) is removed a whole
  piece at a time when any of it is under the area.
- Text set in vertical writing, and text and pictures inside tiling patterns, aren't redacted.
- A picture drawn at an angle (not a quarter turn) is blacked out over a rectangle in the picture that holds the
  whole area, so a little more than the area can go black.
- Saving a JPEG picture again loses a little quality across the whole picture.
- The removed text can still be in bookmarks, comments, file attachments or metadata; the check warns about the
  places it can search.

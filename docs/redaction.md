# Redaction

Redaction removes content from a document for good. It doesn't just cover it: the text, pictures and drawings under
a redacted area are taken out of the file, and the area is painted black. Copying, searching, moving the black box
or reading the file's bytes can't bring them back.

In the app, mark areas, selected text or search results, check the marks, then apply them (see
[Redaction in the app](gui.md#redaction)). From the command line, use `limarepdf-cli redact` (see
[the command line](cli.md#redaction)), which can also just mark, or apply marks already in a file. Marks are
standard PDF redaction marks, so marks made in other apps can be applied in LimarePDF and the other way round.

## What is removed

For each area:

- **Text**, visible or invisible (such as a scanned page's recognised text layer): each character at least a quarter
  under the area is removed. The characters left keep their exact places on the line.
- **Pictures**, scanned pages included: the pixels under the area are blacked out in the picture itself, which is
  then saved again (JPEG pictures as JPEG at about their own quality, black-and-white scans as CCITT, others without
  loss). Pixels only partly under the area are blacked out too. A picture's transparency (soft mask) is cut the same
  way. A picture shared with other pages or drawn elsewhere on the page is copied first, so only the covered copy
  changes. Pictures LimarePDF can't read (JPEG 2000, JBIG2, CMYK JPEG, some unusual formats) are removed whole, and the
  report counts them.
- **Drawings**: the parts of a drawing mostly under the area are removed. Lines and fills that only cross the
  area's edge (a table border, a coloured background) are kept but cut out of the area. Colour gradients are cut
  out the same way.
- **Grouped content** that several pages share (a logo, a header) is redacted inside, on this page only; other pages
  keep it as it was.
- **Comments, links and form fields** touching the area are removed, with the replies to them. A removed field loses
  its value. All of a page's are checked, also on a page with more than the 10,000 the app shows, and so are all
  of its redaction marks when they are applied.
- **Alternative text** that accessibility tools and copying use, on content that lost anything, and on every tag
  above it, however many there are.
- The page's **thumbnail picture** and private application data.

Pictures and drawings that no page uses any more are left out of the saved file.

Redaction stops with a message, and changes nothing, when it can't follow everything it would have to remove: when
a form field touching an area belongs to a form nested more than 32 deep (fields inside fields), or when the index
that leads from a page to its tags is nested more than 32 deep or too large to search (more than 100,000 parts
looked at).

## The check afterwards

When it is done, LimarePDF reads the redacted pages again and checks that no text is left under the areas, and decodes
each picture there to check that its pixels under the areas are blacked out. If anything is left, the app changes
nothing and says what it found, and `limarepdf-cli redact` lists it and doesn't write the file.

It then looks for the removed text elsewhere in the document: on any page, in bookmarks, comments, form fields and the
document's title, author, subject and keywords. Anything it finds is listed as a warning, since redaction only
changes the areas you give it.

## Removing hidden information

A file can carry information that isn't on any page. Document > Remove hidden information… (or `limarepdf-cli sanitise`)
removes it. The app lists what this document has of each kind; what is ticked is removed:

- **Document information and XMP metadata:** title, author, subject, keywords, the program that made it, and the
  XML metadata in the document and in pictures and other parts.
- **JavaScript and actions that run by themselves:** document scripts, actions run on opening, closing, saving or
  printing, form field scripts (formatting, checking, calculating), and actions that launch programs, send form data,
  import data or open attached files, wherever they are: also when they follow an ordinary action (a link that
  opens a web page and then runs a script loses only the script). Ordinary links to pages and web addresses stay.
  A form with both XFA and ordinary fields loses its XFA part (it holds scripts); a form made only for Adobe Reader
  (XFA only) keeps it.
- **Attached files:** files attached to the document or to comments.
- **Hidden layers and their content:** the content of layers that are switched off is deleted, wherever it is
  drawn: on the pages, in forms drawn inside forms (up to 32 deep), in comments' and fields' drawings, in patterns,
  soft masks and Type 3 fonts. Those layers leave the layer list; layers that are shown stay. If some drawing
  can't be read, or is nested deeper, LimarePDF can't tell whether it holds hidden content, so the command stops
  with a message saying what, and changes nothing. It stops in the same way for a drawing without resources of its
  own that is used in places giving its layers different meanings (hidden in one, shown in another), as its hidden
  content can't be taken out of one place only. Leave hidden layers out to remove the rest. Compressed data is
  checked to its end: if it is cut off or damaged part way, the command stops in the same way ("Part of this
  document is damaged..."). Data that is complete but whose checksum at the end is wrong is read in full, as other
  PDF readers read it, and the result says so.
- **Thumbnails and private application data:** page thumbnail pictures, data other programs keep for themselves
  (such as Illustrator's), and web capture information.

Not ticked at first, as they are often wanted:

- **Comments and markup** (redaction marks stay), every one, also on a page with more than the 10,000 the app
  shows. Attached files are removed in the same way.
- **Form field values:** every field's value is cleared for good, read-only fields included, along with default
  values (so Reset form brings nothing back), values a field takes from a parent field, rich text and the drawings
  that showed them; checkboxes and radio buttons are switched off. Fields LimarePDF can't draw lose their drawing,
  and readers draw them empty. Signature fields are left alone. The fields themselves, and their choices, stay. A
  form nested more than 32 deep (fields inside fields, form boxes included, also boxes the form's list of fields
  leaves out) can't be followed to its end, so the command stops with a message and changes nothing; leave form
  values out to remove the rest.
- **Bookmarks.**
- **Content outside the visible page area:** what lies outside each page's crop box is redacted and the page becomes
  the size it is shown at.

Nothing else is kept: saving rewrites the whole file, leaving out everything no longer used, and earlier versions
of the file aren't kept in it.

## Limits

- The width of the gap shows how long the removed text was.
- Fonts embedded in the file keep the shapes of the letters they contain, including removed ones (not the text
  itself).
- Text whose characters can't be placed for certain is removed a whole piece at a time when any of it is under the
  area: text in fonts whose character map, or its code ranges, can't be read (some Asian-language encodings), and
  text holding a code its font's character map leaves out. LimarePDF draws such a code as one byte, so the
  characters after it can show elsewhere than other readers put them; the piece goes when the area covers it in
  either place.
- Text set in vertical writing, and text and pictures inside tiling patterns, aren't redacted.
- A picture drawn at an angle (not a quarter turn) is blacked out over a rectangle in the picture that holds the
  whole area, so a little more than the area can go black.
- Saving a JPEG picture again loses a little quality across the whole picture.
- The removed text can still be in bookmarks, comments, file attachments or metadata; the check warns about the
  places it can search.
- Redaction marks can't be moved or resized; delete one and mark again.
- Removing hidden layers: any drawing whose compressed (Flate) data is cut off or damaged part way stops the
  command, even where other readers would show what comes before the damage, since what follows it can't be
  checked. A file with such damage can have its other kinds of hidden information removed with hidden layers left
  out. Data in the older compressions (LZW, run length) is checked only as far as lopdf reads it: if it decodes to
  nothing it stops the command.

## How this was tested

Redaction and removing hidden information are tested on made-up files for each case, on public PDFs and on a
scanned page. The saved file itself is searched for anything removed, and other readers (poppler and Ghostscript)
are used to confirm it's gone. The Redact tool was used by hand on Linux; the Remove hidden information dialog
wasn't clicked through. Marks made by LimarePDF haven't yet been opened in Acrobat or other apps, nor marks made
by them applied in LimarePDF. The full record is in
[Testing](developer/testing.md#redaction-and-hidden-information).

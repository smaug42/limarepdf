# Redaction

Redaction removes content from a document for good. It doesn't just cover it: the text, pictures and drawings under
a redacted area are taken out of the file, and the area is painted black. Copying, searching, moving the black box
or reading the file's bytes can't bring them back.

Today redaction is done with `mpe redact` (see [the command line](cli.md#redaction)). Marking areas in the app,
reviewing the marks and then applying them is coming next.

## What is removed

For each area:

- **Text**, visible or invisible (such as a scanned page's recognised text layer): each character at least a quarter
  under the area is removed. The characters left keep their exact places on the line.
- **Pictures** touching the area are removed whole. Blacking out only the covered part of a picture is planned.
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

When it is done, MPE reads the redacted pages again and checks that no text or picture is left under the areas. If
anything is, `mpe redact` lists it and doesn't write the file.

It then looks for the removed text elsewhere in the document: on any page, in bookmarks, comments, form fields and the
document's title, author, subject and keywords. Anything it finds is listed as a warning, since redaction only
changes the areas you give it.

## Limits

- The width of the gap shows how long the removed text was.
- Fonts embedded in the file keep the shapes of the letters they contain, including removed ones (not the text
  itself).
- Text in fonts whose characters can't be told apart reliably (some Asian-language encodings) is removed a whole
  piece at a time when any of it is under the area.
- Text set in vertical writing, and text inside tiling patterns, isn't redacted.
- The removed text can still be in bookmarks, comments, file attachments or metadata; the check warns about the
  places it can search.

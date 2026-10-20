# Rendering

LimarePDF draws pages with [hayro](https://crates.io/crates/hayro) 0.8, a pure-Rust PDF renderer that runs on the CPU.

## What has been checked

`crates/limarepdf-core/tests/render_coverage.rs` checks rendering pixel by pixel on small made-up pages. Drawn correctly:

- all 16 blend modes;
- isolated transparency groups, and a blend mode applied to a whole group;
- colour-key image masks (`/Mask [..]`);
- annotations that have an appearance stream (`/AP`): Text, Highlight, Link, Stamp, FreeText and Widget were tested,
  including the Widget `/AS` state and the Hidden flag;
- annotations with the NoView flag are hidden, in the desktop app and `limarepdf-cli render` (LimarePDF marks them Hidden before
  drawing, since hayro ignores NoView);
- annotations without an appearance stream: hayro draws nothing for them, so LimarePDF draws its own for Highlight,
  Underline, StrikeOut, Squiggly, Text (notes, with the seven standard icons), FreeText (with callout lines),
  Square, Circle, Line (with leader lines), Polygon, PolyLine, Ink, Stamp (the stamp's name in a coloured frame),
  Caret and FileAttachment (the four standard icons), with cloudy borders on Square, Circle, Polygon and FreeText,
  in the copy it renders and prints, not in the file (`crates/limarepdf-core/tests/annots.rs`);
- password-protected files with all four ciphers.

### Compared with poppler

An opt-in check draws about 1,100 public test PDFs (the pdf.js and PDFBox test files hayro is tested on) with
LimarePDF and with poppler, and lists the pages that differ beyond anti-aliasing, with pictures side by side
([how to run it](testing.md)). These test files were collected from bug reports, so they are much harder than
everyday documents. The first run (2026-10-17, hayro 0.8.0, poppler 26.08) drew 1,311 pages from 1,089 files:

- 1,079 pages match apart from anti-aliasing (0.5% of pixels or fewer different);
- 220 pages differ by more than that, 23 of them by more than 10%. Many of the worst are poppler's mistakes, such as
  a grey background, garbled scans or striped photos where LimarePDF's page looks right. Differences that are
  LimarePDF's include very thin lines drawn light grey where poppler draws them solid, and fonts that aren't
  embedded (on 39 pages LimarePDF reported a font it couldn't draw; with installed fonts and stand-ins, none on a
  machine with the Noto CJK and Liberation fonts);
- 7 damaged files that poppler repairs couldn't be opened by LimarePDF, and 2 failed in both. Since LimarePDF
  [repairs damaged files](#damaged-files) (2026-10-18), all 7 open and match poppler to within 2% of their pixels,
  one that poppler can't draw opens too, and one fails in both. A few pages poppler can't draw are drawn by
  LimarePDF.

None of the files crashed LimarePDF or took longer than a minute to draw.

The latest run (2026-10-18, with installed fonts and damaged-file repair) drew 1,312 pages from the same files and
listed 228. More pages are listed than in the first run because files that couldn't be opened before are now drawn
and compared, and stand-in fonts differ from poppler's choices. Not every listed page has been looked at by eye; the
differences found to be LimarePDF's are the gaps under [Known to be wrong](#known-to-be-wrong).

## Damaged files

A PDF file ends with a cross-reference table that says where each object starts. When that table is broken (offsets
that point at the wrong bytes, a cross-reference stream that can't be decoded) or missing, LimarePDF finds the
objects by reading through the file, as Acrobat, poppler and pdf.js do:

- every `N G obj` header at the start of a line counts, except inside stream data, and a later copy of an object
  replaces an earlier one, as a later revision would;
- a stream's data ends where its `/Length` says when `endstream` follows there, whether the length is in the
  stream's dictionary or in an object of its own (not a copy of that object inside the data), so a PDF attached
  uncompressed, or `endstream` in a comment, is never taken for the file's own objects; when the length is wrong, or its object can't be found, the data ends at
  the first `endstream`, as pdf.js, poppler and MuPDF do;
- the newest trailer outside stream data that names a catalog gives the catalog, the document information and, for
  encrypted files, the encryption settings; when none names a catalog that leads to pages, the last catalog object
  found is used;
- objects packed into object streams are read too, one by one when the stream's own index can't be followed.

The app then shows a notice that the file was repaired and some content may be missing, with a Save as… button.
The original file is never replaced without asking: a repaired document counts as changed, so closing it warns of
unsaved changes, and Save asks whether to replace the original or save the repaired copy as a new file.
`limarepdf-cli` prints a note on stderr, and writes only where `-o` says. Of about 1,100 test files collected from bug reports, 59 needed repair.

Limits:

- What the repair finds may not be what the file's author meant: an object whose header is damaged is lost, and
  where the file holds several versions of an object, the last one is used even if the damaged table pointed
  elsewhere. An object a later revision deleted can come back, though nothing in the document refers to it then,
  and saving leaves it out.
- A stream whose `/Length` is wrong, or is in an object that can't be found, ends at its first `endstream`; if its data
  holds that word (an attached PDF, say), what follows it in the data may be taken for objects.
- A repaired file's digital signatures never show as valid and unchanged, since other PDF apps may read it
  differently: a signature whose signed part is damaged shows *can't check*, and one whose signed part is intact
  reports a change that isn't allowed. Saving can't keep them: the app asks first, and `limarepdf-cli` needs
  `--break-signatures`.
- A file with no recognisable objects at all still can't be opened.

## Page colour modes

The app's Dark, High contrast and Sepia page colours (View > Page colours) are worked out on the drawn page, pixel
by pixel, in `crates/limarepdf-core/src/page_colours.rs`. Dark swaps light and dark but keeps each colour's hue. High
contrast turns anything inked, in any colour, white on black. Sepia maps white to warm paper and black to dark
brown. Printing, exports, `limarepdf-cli render` and the file itself always use the page's own colours.

To leave pictures alone, the page goes through hayro's interpreter a second time with a device that draws nothing
and notes the box of each raster image, clipped to the clip in force. The colour pass skips those boxes, so:

- a turned picture keeps the colours of everything inside its upright bounding box;
- text or lines drawn on top of a picture keep their own colours there;
- one-colour stencil masks (some logos, Type 3 glyphs) are treated as drawing, not pictures, and are recoloured;
- a scanned page is one picture and stays as it is unless "Recolour pictures too" is on.

## Known to be wrong

These come from hayro 0.8, the library that draws the pages:

- Very thin lines aren't snapped to the pixel grid, so a hairline that falls between two rows of pixels is drawn as
  two light-grey rows instead of one solid row (poppler and Acrobat snap them).
- Plain fonts that aren't embedded (not CID fonts) are drawn with one of the standard fonts, never an installed
  font; symbol fonts such as Wingdings come out as letters (see [Fonts that aren't embedded](#fonts-that-arent-embedded)).
- Knockout groups are drawn as non-knockout.
- Non-isolated groups are drawn as isolated, which changes colours where a blend mode such as Multiply is used
  inside the group.
- Shading patterns that carry their own graphics state, and CMaps built on other CMaps, aren't supported.

Other gaps:

- Other annotation types without an appearance stream (Sound, Movie, Screen, 3D and the like) are not drawn. For
  those LimarePDF draws: text boxes use Helvetica and ignore rich-text styling; line captions (`/Cap`) and a caret's
  paragraph symbol (`/Sy /P`) are left out; a cloudy border's bumps follow Acrobat's sizes only roughly; and a stamp
  with no drawing shows its name, not Acrobat's artwork for it.

Other PDF features have not been tested by LimarePDF.

## Fonts that aren't embedded

Most PDFs carry their fonts. Some only name them and leave the reader to find them, which other viewers do among the
fonts installed on the computer.

- **The 14 standard fonts** (Helvetica, Times, Courier, Symbol, ZapfDingbats and their styles) always use the copies
  bundled with hayro, so they look the same on every computer.
- **CID fonts**, which Chinese, Japanese and Korean text uses (and some programs use for Latin text too), are
  looked up among the installed fonts (`crates/limarepdf-core/src/installed_fonts.rs`):
  1. by the font's name (`MSMincho` finds MS Mincho), ignoring spaces, hyphens and case;
  2. by its family, the face closest to the weight and style asked for (`MSGothic,Bold` finds MS Gothic's bold);
  3. among free fonts with the same character widths as common Windows fonts: Liberation Sans or Arimo for Arial,
     Liberation Serif or Tinos for Times New Roman, Liberation Mono or Cousine for Courier New, Carlito for
     Calibri, Caladea for Cambria.

  Those show the document as it was made. Failing them, a stand-in is used:

  4. for Japanese, simplified Chinese, traditional Chinese and Korean text, among the usual fonts for that language
     (Noto CJK, Source Han, and the Windows and macOS ones), serif (Mincho, Song, Ming, Batang) or sans as the
     font is, and then any installed font with characters of that language. A font is only used for one of those
     languages if it has that language's characters;
  5. for other text, a common font of the same kind, sans, serif or monospaced (Liberation, DejaVu, Noto, Arial,
     Times New Roman and so on), and then any installed font with Latin letters.

  When a stand-in is used, the app's notice names the font and suggests installing it: "The font Tahoma isn't
  installed, so a similar font is shown in its place and some text may not look right. Install Tahoma to see the
  document as it was made." Some text needs the real font: when the file gives no map from its codes to characters,
  only the named font knows which glyph each code is, and a stand-in shows the wrong characters, as poppler does too.
  When no installed font has a language's characters, that text can't be shown, and the notice says so and names a
  free font to install: "This document has Japanese text, but no Japanese font is installed, so that text can't be
  shown. Install a Japanese font, such as Noto Sans CJK JP, to read it."
- **Other fonts that aren't embedded** (Calibri, Verdana, Wingdings as plain fonts) are drawn with the closest
  standard font, stretched to the widths the file gives, so lines keep their length but the letters look different.
  Symbol fonts such as Wingdings come out as letters. hayro 0.8 picks these itself without asking LimarePDF, so
  installed fonts can't be used for them yet.

The installed fonts are listed the first time a document needs one, once per run of the app. With about 700 font
families installed, that took about a second when the font files weren't in the system's file cache, and a few tens
of milliseconds when they were. What a page looks like, on screen, in print and in exports, then depends
on which fonts are installed, as in other viewers.

## When something can't be drawn

- A page that can't be drawn shows "This page couldn't be drawn" and the reason, such as a file hayro can't parse.
  It isn't tried again until the document changes. Editing may still work if the document library (lopdf) can read
  the file.
- When hayro reports a picture it can't decode, the app shows a notice that parts of the document couldn't be drawn.
- A second notice covers fonts that aren't embedded or installed: which are shown with a stand-in, or which
  language has no font at all, and what to install (see [Fonts that aren't embedded](#fonts-that-arent-embedded)).
  Plain fonts that aren't embedded are drawn with the closest of the standard fonts (Helvetica for Arial) and
  aren't reported, as hayro doesn't say which they are.
- hayro reports each font once, on the first page that uses it, so the notice is about the document, not one page.
- An annotation whose appearance hayro can't select isn't reported: that is also what happens, correctly, for a check
  box that is off and has a drawing only for on.

## High zoom

- The app draws a page as one bitmap up to 4096 pixels on its longer side (less when the graphics card's largest
  texture is smaller). A page shown bigger than that is drawn at that size, and the part in view is drawn over it at
  full size in tiles of 512 × 512 pixels, with a ring of one tile around the view so scrolling finds them ready.
- After a zoom change, tiles at the new zoom replace those of the old one as they arrive; until then the old tiles,
  or the whole-page bitmap, show stretched, so the page is never blank.
- Each tile is drawn by hayro on its own, with the view moved to that part of the page. A test checks that tiles
  match the same pixels of the whole page drawn at once. Inside raster pictures, a few pixels can differ slightly
  between a tile and the whole page, as the picture is sampled from a different starting point.
- Memory and texture size stay bounded at any zoom, up to the highest, 6400%. Tiles are drawn at up to 256 pixels
  per point (6400% on a screen with three pixels per point); beyond that they are shown stretched.
- Printing and exports don't use tiles. They draw whole pages ([in bands](#large-drawings) when large), at most
  1152 dpi and at most 268 million pixels
  ([details](export-and-printing.md#pages-as-images)). A page stated so large that even the whole-page bitmap the
  app draws at the lowest scale would pass that shows an error instead of being drawn.
- Tiles have only been tried on screens at scale 1, not on a HiDPI screen.

## Large drawings

- hayro holds everything it draws for a page until the drawing is done, and some content costs memory for every
  pixel: it fills each shaded area of the mesh kinds (types 4 to 7) by working out a picture of it as large as the
  area painted, which for an `sh` operator is the whole page. PDFBox test file 5134 paints 84 of them, so drawing
  its 8-million-pixel page at 72 dpi took 84 such pictures of 32 MB, 2.7 GB in all, and 9.7 GB at 150 dpi.
- So a page bitmap of more than about 4 million pixels is drawn in full-width bands of at most that size, each by
  hayro on its own, with the view moved to that part of the page, as tiles are. The memory of a drawing is that
  of one band (5134 at 150 dpi: 1.5 GB).
- The bands come out as one drawing would, except for slight differences in a few pixels, mostly within raster
  pictures, as with tiles. A test checks that a banded page matches one drawn at once.
- Each band reads the page's content again and works out its pictures, soft masks and shadings again. Measured on
  the first pages of 571 of the test files at 288 dpi (8 million pixels for Letter), of the pages that took more
  than 50 ms the middle one took about a third longer, and a few took several times longer, up to 0.6 s becoming
  5.9 s for a page with large pictures. Bitmaps of 4 million pixels or less, which include most pages on screen, are drawn at once as before.
- In the app, a whole-page drawing is stopped at its next band when it is no longer wanted: the page was drawn for
  a zoom since passed, the colours changed, or the document was edited. Before, zooming in a few steps quickly
  drew the page at every zoom passed through, one per drawing thread at a time (five on a six-core machine), which
  for 5134 would have come to an estimated 11 GB.

For timings and how the app keeps drawing responsive, see [Performance](performance.md).

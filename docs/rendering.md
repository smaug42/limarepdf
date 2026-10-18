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
  Underline, StrikeOut, Squiggly, Text (notes), FreeText, Square, Circle, Line, Polygon, PolyLine and Ink, in the
  copy it renders and prints, not in the file (`crates/limarepdf-core/tests/annots.rs`);
- password-protected files with all four ciphers.

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

- Knockout groups are drawn as non-knockout.
- Non-isolated groups are drawn as isolated.
- Other annotation types without an appearance stream (Stamp, Caret, FileAttachment and the rest) are not drawn.
  For those LimarePDF draws, cloudy borders, callout lines, leader lines, rich text and the note icon's shape (one icon
  for every kind of note) are not reproduced.

Other PDF features have not been tested by LimarePDF.

## When something can't be drawn

- A page that can't be drawn shows "This page couldn't be drawn" and the reason, such as a file hayro can't parse.
  It isn't tried again until the document changes. Editing may still work if the document library (lopdf) can read
  the file.
- When hayro reports a picture it can't decode, or a font it can't use, the app shows a notice that parts of the
  document couldn't be drawn. Chinese, Japanese and Korean fonts that aren't embedded count as fonts it can't use:
  hayro draws them with a Latin standard font, which lacks their characters. Other fonts that aren't embedded are
  drawn with the closest of the standard fonts (Helvetica for Arial) and aren't reported.
- hayro reports each font once, on the first page that uses it, so the notice is about the document, not one page.
- An annotation whose appearance hayro can't select isn't reported: that is also what happens, correctly, for a check
  box that is off and has a drawing only for on.
- The app draws a page as one bitmap, at most 8192 pixels a side (the graphics card's limit). Above that, pages are
  shown at the largest size that fits and look softer at high zoom.

For timings and how the app keeps drawing responsive, see [Performance](performance.md).

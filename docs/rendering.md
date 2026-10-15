# Rendering

MPE draws pages with [hayro](https://crates.io/crates/hayro) 0.8, a pure-Rust PDF renderer that runs on the CPU.

## What has been checked

`crates/mpe-core/tests/render_coverage.rs` checks rendering pixel by pixel on small made-up pages. Drawn correctly:

- all 16 blend modes;
- isolated transparency groups, and a blend mode applied to a whole group;
- colour-key image masks (`/Mask [..]`);
- annotations that have an appearance stream (`/AP`): Text, Highlight, Link, Stamp, FreeText and Widget were tested,
  including the Widget `/AS` state and the Hidden flag;
- annotations with the NoView flag are hidden, in the desktop app and `mpe render` (MPE marks them Hidden before
  drawing, since hayro ignores NoView);
- annotations without an appearance stream: hayro draws nothing for them, so MPE draws its own for Highlight,
  Underline, StrikeOut, Squiggly, Text (notes), FreeText, Square, Circle, Line, Polygon, PolyLine and Ink, in the
  copy it renders and prints, not in the file (`crates/mpe-core/tests/annots.rs`);
- password-protected files with all four ciphers.

## Known to be wrong

- Knockout groups are drawn as non-knockout.
- Non-isolated groups are drawn as isolated.
- Other annotation types without an appearance stream (Stamp, Caret, FileAttachment and the rest) are not drawn.
  For those MPE draws, cloudy borders, callout lines, leader lines, rich text and the note icon's shape (one icon
  for every kind of note) are not reproduced.

Other PDF features have not been tested by MPE. Files hayro cannot parse show "Rendering…" on those pages; editing may
still work if the document library (lopdf) can read them.

For timings and how the app keeps drawing responsive, see [Performance](performance.md).

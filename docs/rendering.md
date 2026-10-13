# Rendering

MPE draws pages with [hayro](https://crates.io/crates/hayro) 0.8, a pure-Rust PDF renderer that runs on the CPU.

## What has been checked

`crates/mpe-core/tests/render_coverage.rs` checks rendering pixel by pixel on small made-up pages. Drawn correctly:

- all 16 blend modes;
- isolated transparency groups, and a blend mode applied to a whole group;
- colour-key image masks (`/Mask [..]`);
- annotations that have an appearance stream (`/AP`): Text, Highlight, Link, Stamp, FreeText and Widget were tested,
  including the Widget `/AS` state and the Hidden flag;
- annotations with the NoView flag are hidden in the desktop app (MPE marks them Hidden before drawing, since hayro
  ignores NoView);
- password-protected files with all four ciphers.

## Known to be wrong

- Knockout groups are drawn as non-knockout.
- Non-isolated groups are drawn as isolated.
- `mpe render` (the command line) draws NoView annotations.
- Annotations without an appearance stream are not drawn at all (tested: Text, Highlight, Link, Stamp, FreeText,
  Widget).

Other PDF features have not been tested by MPE. Files hayro cannot parse show "Rendering…" on those pages; editing may
still work if the document library (lopdf) can read them.

For timings and how the app keeps drawing responsive, see [Performance](performance.md).

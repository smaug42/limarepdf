# How it is built

LimarePDF is written entirely in Rust, with no C or C++ dependencies.

| Part | What it is |
| --- | --- |
| `limarepdf-core` | The library: document operations, rendering, editing, OCR, printing |
| `limarepdf-cli` | The command-line tool |
| `limarepdf-gui` | The desktop app |

It uses:

- [`lopdf`](https://crates.io/crates/lopdf) for the document structure and encryption;
- [`hayro`](https://crates.io/crates/hayro) to draw pages, on the CPU (see [Rendering](../rendering.md)), and
  [`fontdb`](https://crates.io/crates/fontdb) to find installed fonts for fonts a file doesn't embed
  (`crates/limarepdf-core/src/installed_fonts.rs`, the lookup order in
  [Fonts that aren't embedded](../rendering.md#fonts-that-arent-embedded));
- [`egui`](https://crates.io/crates/egui) for the interface, and [`rfd`](https://crates.io/crates/rfd) for file
  dialogs. eframe's default features are off and the ones used listed (the workspace's `Cargo.toml`), so a new
  default isn't taken unseen (0.37's `system_fonts` isn't, and `NativeOptions::system_font_fallback` is off with
  it: interface text falls back to installed fonts through LimarePDF's own `typing_fonts::interface_fallback`,
  ENH-33, below); winit, which eframe then gives only its X11 and Wayland features, is named in `limarepdf-gui`'s
  `Cargo.toml` for its defaults (Wayland window decorations, libwayland loaded at run time), at eframe's version.
  The window is drawn by eframe's wgpu renderer (on Linux Vulkan, OpenGL where Vulkan fails). On Wayland its
  frames go to the screen without waiting for the display (wgpu's `AutoNoVsync`, set each frame by
  `src/present.rs`, BUG-42): with NVIDIA's driver under KWin 6.8 a frame handed over while the screen is off, or
  after a long idle gap, can wait for ever, and the desktop with it. winit already paces drawing there, asking for
  a frame only after the compositor's frame callback, as eframe's OpenGL renderer relies on too (egui #5136). X11,
  Windows and macOS keep eframe's waiting `AutoVsync`. NVIDIA takes immediate for it and marks its frames as fine
  to tear, which KWin allows only for a full-screen window it shows directly, with its tearing setting on (PB-66).
  The icons are the Phosphor fonts in `crates/limarepdf-gui/assets/fonts/`, by name in `src/icon.rs`;
- [`skrifa`](https://crates.io/crates/skrifa) to read font programs and [`subsetter`](https://crates.io/crates/subsetter)
  to cut embedded fonts down to the glyphs used; [`harfrust`](https://crates.io/crates/harfrust) 0.13.3 to shape
  new text (batch 45; it reads fonts through `read-fonts` 0.43, the version `skrifa` 0.46 uses, so core keeps one
  font reader, while egui's `epaint` keeps its own `harfrust` 0.12),
  [`unicode-linebreak`](https://crates.io/crates/unicode-linebreak) 0.1.5 for line breaks (UAX #14),
  [`unicode-script`](https://crates.io/crates/unicode-script) 0.5.8 for each character's script, and
  `unicode-segmentation` and `unicode-normalization` for words, grapheme clusters and NFC (the app uses both too,
  `unicode-normalization` since batch 45): the text pipeline below;
- [`image`](https://crates.io/crates/image) for images;
- [`url`](https://crates.io/crates/url) 2.5.8 for the addresses new links are written with (ENH-17: IDNA and
  percent-encoding; already in the tree through `webbrowser`);
- [`spellbook`](https://crates.io/crates/spellbook) 0.4.2 (MPL-2.0) for spelling, with Hunspell dictionaries, and
  [`zip`](https://crates.io/crates/zip) 9.3 to read the dictionary extensions added (NEW-06; "Spelling" below);
  the app asks the system's language of [`sys-locale`](https://crates.io/crates/sys-locale) 0.3;
- [`ocrs`](https://github.com/robertknight/ocrs) for text recognition (see [Scans and OCR](../scanning-and-ocr.md));
  the usual scanner and OCR libraries, SANE and Tesseract, are C/C++, so LimarePDF doesn't drive scanners;
- [`serde_json`](https://crates.io/crates/serde_json) in the command line, for form values as JSON.

Text strings in the file (properties, bookmarks, field names, values and descriptions, comments and their
authors, page labels, signature details) are all read by `doc::decode_text`: UTF-16BE or UTF-8 after a byte
order mark, else PDFDocEncoding, which differs from Latin-1 at 0x18 to 0x1F (accents) and 0x80 to 0xA0 (bullet,
dashes, curly quotes, euro and so on). Bytes it leaves undefined (other control codes, 0x7F, 0x9F) keep their own
code points, as pdf.js reads them, so nothing is lost and field names differing only there stay apart; showing
text strips control characters anyway (`display.rs`). `doc::encode_text` writes printable ASCII, tabs and line
breaks as they are and anything else as UTF-16BE, so every string reads back unchanged. lopdf's own
`decode_text_string` isn't used: its table drops tabs and line breaks. Text that is only shown (document
properties, bookmark titles, comments' contents, authors and subjects, field descriptions) goes through
`doc::decode_shown_text` instead (`Pdf::shown_text_value`; PB-18): unmarked bytes that are valid UTF-8 with a
character of more than one byte read as UTF-8, as the program that wrote them meant; in PDFDocEncoding those would
be runs like "Ã©". Field names, values and options, page labels and signature details stay on `decode_text`, so a
field is found by the name every reader finds. Windows-1252 can't be told from PDFDocEncoding (both define 0x80 to
0x9F) and reads by the standard. Font encodings in page content are separate (`fonts.rs`, `tounicode.rs`,
`own_encoding.rs`).

Forms are read in `crates/limarepdf-core/src/forms.rs`. When a value changes, LimarePDF draws the field's appearance itself
(`form_ap.rs`, written into the file by `form_write.rs`) rather than asking other readers to, so filled fields look
the same everywhere; the desktop app's side is `crates/limarepdf-gui/src/fields.rs`. Field lists go by purpose
(PB-174): `Pdf::form_fields` and `listed_nodes` take the first 10,000 (`MAX_FIELDS`), for what the app shows: the
on-screen pages, the empty signature boxes offered (`empty_signature_fields`), typing in a field (`field_setting`)
and `form list`; everything else takes every field
(`Pdf::every_form_field`, `form_nodes`, `Reach::Every`): finding a field by name (setting, signing, `form fill`),
reset, removing values, counting and checking signatures, field locks, new signature field names, redaction's
report, flattening, printing, the pages' pictures (`render_snapshot`, `unshown_fields`), drawing fields before
signing and telling XFA-only forms apart.

Every dialog in the desktop app is built from the same parts in `crates/limarepdf-gui/src/dialogs.rs`, so they all
look like Print: `Shell` (the window over the dimmed app, its title and file, and the dialogs' look from
`theme::dialog_style`: outlined fields and buttons 32 points tall), `content` (the settings, scrolling in a short
window, with radio buttons and checkboxes on the dialog's background from `theme::dialog_choices`), `group` (a
heading and its settings), `field` (a label above a field, which also names it for screen readers; text fields go
through `theme::field`), `columns` (two fields side by side, one under the other when narrow), `list_box`,
`list_rows`, `list_row` and `list_pick` (an outlined list, rows with their buttons on the right, or rows to pick),
`fact` (a name and a value to read) and `footer` (a note and the buttons, in the platform's order). Margins are
smaller in windows under 600 points wide. A button on a list's row is named with what the row is (`set_name`), and
`list_rows` gives each row its own id for what it remembers. Tests draw a dialog with `dialogs::drawn_window` and
check it with `fits` (inside the screen, its buttons inside it) and `unclear_names`. The command palette (`palette_ui` in `app.rs`) uses the same look
without `Shell`, as a launcher with no title or buttons. egui's `Modal` centres its window each frame, so a dialog
whose content grew moved up by half of what was added (BUG-50): `Shell::show` keeps the window's top from the
first frame it was seen (in egui's data under the dialog's id, with the screen's rect) and anchors it there
(`Modal::area`, `CENTER_TOP`), so it grows downward; the top moves up only as far as keeps the bottom `EDGE`
(8 points) above the screen's, and a dialog newly shown, or a screen of another size, is centred again. Tab goes
round a dialog (PB-15): at Tab on the last
control egui gives up the keyboard (and marks it for the next control that asks), so `Shell::show`, finding no
focus after a Tab, offers it to the first enabled focusable widget on the dialog's layer
(`Memory::interested_in_focus`, which also clears egui's mark), or gives it outright when that is the only control
(egui doesn't offer it back to the widget that just had it). The app's tests walk Tab through every question
dialog and fail on a stop with no control.

Damaged files are repaired in `crates/limarepdf-core/src/repair.rs` when lopdf reads a file with no pages, or had
to rebuild its table itself (its own scan ignores `/Length`, so its result is kept only when the repair fails), or
took a table it found near where `startxref` or `/Prev` points (`xref_recovery` is `OffsetCorrected`) and the file
doesn't read as other apps read it (`Pdf::differs`): lopdf takes such a table when most of its entries point at
their objects, and passes over the objects the others point past. The repair: it
scans the bytes for `N G obj` headers (skipping stream data by a `/Length` followed by `endstream`, else to the
first `endstream`; an indirect length is looked up among every `N G obj <integer> endobj` in the file, a first pass,
taking the latest that fits and isn't inside the data its own value or the longest fitting one gives, so a copy
planted in an attachment can't end the stream early; a later copy of a number wins), takes the trailer entries from the newest trailer or
cross-reference stream dictionary outside stream data that names a catalog, appends a rebuilt cross-reference
table, and has lopdf read the file again. Object stream members lopdf's reader skips are then read one by one, and
when the trailers name no usable catalog, the last catalog object with a page tree is used. See
[Rendering](../rendering.md#damaged-files).

Two workarounds in `crates/limarepdf-core/src/doc.rs` read a corrected copy of a file when lopdf refuses it (offsets
stay where they were, so nothing counts as repaired). `blank_null_encrypt` blanks a trailer's `/Encrypt null`, which
lopdf takes as encryption. `encryption_lopdf_reads` appends a copy of the encryption dictionary as an update in a form
lopdf reads, for two forms it doesn't:
- A trailer holding the dictionary itself rather than a reference to it (pdf.js's test files issue6010_1 and _2;
  PB-183). lopdf 0.47 looks for it only through a reference (`Document::get_encrypted`), so it refused every
  password, the right one too. The copy gets the next free object number, and the update's trailer points at it.
- A standard-security file at revision 3 or 4 whose `/U` is shorter than 32 bytes: readers compare only its first 16
  bytes there, and some writers store just those (PDFBox's test file 1910), but lopdf (0.46 too) refuses any length
  but 32 (`InvalidHashLength`) and so reports a wrong password. The copy's `/U` is zero-padded to 32 bytes.

The workarounds stay until lopdf reads both; no issue has been filed upstream.

A document's pages are its page tree's entries whose object is a `/Type /Page` dictionary (`doc::pages`, behind
`Pdf::page_count`, `Pdf::page_ids` and `page_index`). lopdf's own list (`get_pages`, since 0.46) also counts an
entry whose object is missing or isn't a typed dictionary, as a blank stand-in page; hayro passes over a missing one,
so taking lopdf's list would shift page numbers between editing and drawing, and a tree with no real page would
no longer be repaired. The tree is walked through `doc::page_tree`, which keeps lopdf's order but not its guess at
how many entries there are; clippy.toml refuses `page_iter` and `get_pages` anywhere else. The guess adds up the
`/Count` each branch of the tree states, so in a damaged file it was short (counting the pages then stopped with an
overflow, as the app is built), overflowed itself, or, as the pages were collected for drawing, asked for more
memory than a computer has, which ended the app (PB-173, found by the `readers` fuzz target).

hayro (0.9) sets aside memory for a picture from its stated width and height, not from its data: an inline image of
78 bytes stating 255,555,555 × 2 pixels made it ask for about 4 GB (PB-81, found by REL-12's `draw` fuzz target). So
every copy pages are drawn from leaves out a picture whose data can't come near filling it
(`crates/limarepdf-core/src/unfillable.rs`): `Pdf::render_snapshot`, `app_snapshot` and `uncropped_app_snapshot`
(through `snapshot_hiding`), `Pdf::drawing_snapshot` (OCR, Print as image, Windows printing and the Print preview,
on the sheets' copy) and `page_without` (the page under text being edited). A picture is left out when it states
more than 16 million pixels (`LARGE`) and its data, decoded through lopdf with the output capped at an eighth of what
the stated size needs, comes to less than that eighth (`PART`); unsure counts (bits, colours) take the smallest, so
the check errs towards drawing. A fax (CCITT) picture's rows are bounded by its data, at least a bit each, against
`/Rows` or its height, whichever is more, as hayro allocates; its width isn't (a blank row is one bit however wide),
so one over 2^17 columns (`MOST_FAX_COLUMNS`, by `/Width` or `/Columns`) is left out whatever its data (PB-11).
Data in JPEG, JPEG 2000 or JBIG2, or that lopdf can't decode, is left to hayro. Picture objects are any stream with
`/Subtype /Image`; inline images are looked for in page contents, form XObjects, tiling patterns and Type 3 glyphs,
cut out with `inline::extract` and put back with `Inline::restore`, the content written decoded in the copy. In
the copy a picture left out becomes a one-pixel `DCTDecode` picture whose data isn't a JPEG, so hayro reports
`ImageDecodeFailure` and the renderer `Missing::Picture`, the notice any undecodable picture gets; the document
keeps the picture. The app takes a snapshot after every edit, and looking for inline images decodes every content
stream (up to 56 ms on a 107-page test file), so the streams found fine are remembered on the `Pdf` by a
fingerprint (`Checked`: number, dictionary, length and 16 spans of 32 bytes spread over the data); after the first
snapshot the check costs at most 2 ms on the 104 test files measured (0.06 ms on average). A changed stream gets a
new fingerprint and is checked again; what LimarePDF writes never states more than its data holds, so only what a
file brings needs it. None of the 1,080 test files that open has a picture left out. `tests/unfillable.rs`.

The same copies leave out a tiling pattern hayro 0.9 can't tile (PB-11). hayro draws the pattern's cell into a tile
once for each copy of the cell the tile overlaps, about the cell's size over the step plus two each way, then repeats
the tile; each way the tile is the step long, or the canvas seen in the pattern's space if that is shorter, at the
pattern's scale. A cell 100,000 units a side stepping 1 is drawn ten billion times (drawing ran past five minutes),
and a skew grows the canvas seen: a pattern skewed 80 to 1 (`/Matrix [1 0 80 1 0 0]`, a cell 60,000 by 700 units) on
a Letter page asked for 13 GB at 72 dpi and panicked at 96, its tile over the 65,532 pixels a side vello_common can
snap to its tiles. The renderer draws a page with a tiling pattern in passes of at most 2,048 pixels a side
(`PASS_SIDE`; [Large drawings](../rendering.md#large-drawings)), so the canvas is at most a pass at every zoom: the
tile's pixels across are the canvas seen times a unit's length on it, the same at every zoom, or the step's, which
grow with the zoom.
`unfillable.rs` reckons them from the pattern's `/BBox`, `/XStep`, `/YStep` and `/Matrix` at the most zoom the
renderer takes (`MAX_REGION_SCALE`, 256), and leaves the pattern out when its copies would be over 10,000
(`MOST_COPIES`), a side of its tile over 65,530 pixels (`MOST_SIDE`: 65,532 less the 2 hayro may round up) or the
tile over 16 passes' pixels (`MOST_TILE`, 256 MiB; unskewed or turned a quarter a tile is at most one pass, turned an
eighth two, skewed 2 to 1 about 7). A pattern in a form, an annotation's appearance, a soft mask or a Type 3 glyph is drawn with their
matrices too, which can skew it where its own doesn't, and one in another pattern's cell is drawn on that pattern's
tile, which can be longer than a pass. So a walk of what the pages draw (`Nested`) follows `q`, `Q`, `cm`, `gs`, `Do`,
`BT`, `Tm`, `Tf`, `Tz` and the text Type 3 fonts show, reading a drawing's content only when something it draws holds
a tiling pattern, and reckons each pattern against the whole of a pass, or of the tile it is drawn on; every state of
an appearance, and hidden annotations, are looked into too. It looks 64 drawings deep at most (hayro draws 50), and
past 2^24 drawings looked into and operators gone through outside the pages it leaves out every tiling pattern. In
the copy a pattern left out becomes one whose one-unit tile draws the undecodable picture above, so the page is drawn
without it and the renderer reports `Missing::Picture`; the document keeps the pattern. hayro 0.8 sized the tile from
the step alone, so PDFBox's 2363, stepping 32,767 units, asked for 7.6 GB at 96 dpi and panicked at 150 (D12); 0.9
draws it at 50 MiB. None of the
646 tiling patterns in the 1,207 test files is left out: the most copies any has is 49, and the largest tile is 1.5
passes and 5,714 pixels long, turned (both pdf.js issue2177). `tests/patterns.rs`.

In the same copies a shape filled with a pattern of a mesh shading (types 4 to 7: triangles or patches) is drawn as
the shading itself (PB-11, `crates/limarepdf-core/src/mesh_fill.rs`). hayro 0.9 samples a mesh, for a fill with its
pattern, into a picture of all the shape covers on the pass, and keeps every such picture until the pass ends; for
`sh` it samples only the mesh's bounds. pdf.js's bug1260585, a 3D plot converted from PostScript, fills its figure
2,640 times with a pattern of one triangle each: 6.1 GB and 138 s at 288 dpi, now 111 MiB and 0.3 s, and without the
light seams the fills left between triangles (Ghostscript draws none). As the PDF reference describes such a fill,
the shape becomes the clip in a saved state, the pattern's matrix, taken from the space the drawing began in (which
hayro starts afresh in a form, an appearance and a tiling pattern's cell), is set with `cm`, and the shading is
painted with `sh` under a name of its own added to the drawing's shadings; a shape that is also outlined is
outlined after, and one setting a clip (`W f`) sets it after. Pages, forms, annotations' appearances and tiling
patterns' cells are redrawn, a page's streams as one; left as they are: a shading with a `/Background` (which `sh`
leaves out), a pattern with an `/ExtGState`, fills in Type 3 glyphs and in forms without resources of their own, and
a stream two drawings share, which is drawn with the resources of each. `tests/mesh_fills.rs` draws each case both
ways and finds them the same (a skewed shape's smoothed edges differ in 933 pixels of 1.9 million).

hayro 0.9 draws an annotation whatever its `/OC` says: it checks only the Hidden and NoView flags, though it honours
a drawing's own `/OC` and marked content tagged with a layer. So a comment or field in a layer that's off showed
in the app and in every picture of the page (PB-181; recorded, not filed upstream). The same copies
(`snapshot_hiding`, `drawing_snapshot`) mark such an annotation Hidden, and `page_without` leaves it out
(`drawn_annots`). Whether it is in one is worked out as Remove hidden information works it out, with the layers off
as the document opens and the rules read within their steps (`Pdf::layers_off`, `annot_layer_off`). The app
neither draws nor picks it: `Annot::layer_off` makes `on_page` false and `hidden` true. `tests/annots.rs`,
`tests/form_flatten.rs`.

The page colour modes (View > Page colours; [Rendering](../rendering.md#page-colour-modes)) are worked out on the
drawn page, pixel by pixel, in `crates/limarepdf-core/src/page_colours.rs` (`PageColours::pixel`, `recolour`). Only
`Renderer::render_in` and `render_region_in`, which draw pages and tiles for display, call it, so printing, exports
and `limarepdf-cli render` keep the page's own colours. Dark shifts the three channels by the same amount, so the
largest and smallest swap places around 255, which inverts lightness and keeps hue and chroma; High contrast takes
the weakest channel as the amount of ink (so yellow counts as much as black) and stretches it, pale tints going
black and anything stronger white; Sepia maps white to warm paper and black to dark brown. To leave pictures alone,
`page_colours::picture_boxes` runs the page through hayro's interpreter a second time with a device (`Boxes`) that
draws nothing and notes the box of each raster image, clipped to the clip in force (the crop box first), and
`recolour` skips those boxes. So a turned picture keeps the colours of everything inside its upright bounding box,
text or lines drawn over a picture keep their own colours there, and stencil masks (one-colour shapes painted in the
fill colour, such as some logos and Type 3 glyphs) are drawing, not pictures, and are recoloured. With Recolour
pictures too on (`keep_pictures` false), no boxes are kept.

Page content is read by LimarePDF's own parser (`crates/limarepdf-core/src/objects.rs`), which splits it into the objects that
editing works on and reads the text that search, copying and export use, with its own reader for fonts' ToUnicode
maps and Adobe's widths for the standard fonts (see [How text is read](../reading-and-finding.md#how-text-is-read)). CID
fonts' character maps, and Adobe's tables from character numbers to Unicode, are read with hayro's CMap parser
(`hayro_cmap`, reached through hayro, which carries the predefined maps); the predefined ones are read once per
process (`fonts::predefined`). A Type 0 code that neither the ToUnicode map nor the character numbers give text
for (`FontCtx::decode_raw`) reads as U+FFFD, one per code, when the CMap maps it to no character number (it is
drawn as nothing; PB-06: lopdf's simple-font encodings read `<0042>` as "B"). An Identity font with no
ToUnicode map and no character collection LimarePDF knows reads its CIDs through its embedded program (PB-26,
batch 31; `own_font::cid_text`, `FontCtx::glyph_text`), as poppler does: the CID's glyph as hayro finds it
(`/CIDToGIDMap`, or a CID-keyed CFF program's charset), then that glyph's character by the program's Unicode cmap
read backwards (the lowest outside the private use areas), else its name in `post` or a CFF or Type 1 program.
The text of each program is kept by a hash of its stream and map (64 programs). A font whose `/ToUnicode` names
Identity-H or Identity-V instead of holding a map (`FontCtx::unicode_codes`; pdf.js's `bug951051.pdf`,
`bug1650302.pdf`) has codes that are their characters' UTF-16 values, as poppler and hayro read it. A code with
no text still (the program names nothing: not embedded, CID-keyed CFF, or a subset without a cmap or names) goes
through lopdf's encodings as the last resort: in the corpus, files such as pdf.js's `issue11915.pdf` use codes
that are ASCII, which that reads right, and U+FFFD for those regressed about 40 files. With PB-06's rule, the
corpus's text changed in one file only (`issue1597.pdf`: a hyphen with no ToUnicode entry, dropped before, now
U+FFFD). With PB-26's, 9 files read better (Czech, Romanian, Russian and whole lines of shifted letters read
right; poppler 26.10's `pdftotext` reads 5 of them as shifted letters still) and none worse.

Simple fonts' codes without a ToUnicode entry are read through `own_encoding::simple_codes` (PB-23) when the
font's `/Encoding` is a dictionary or is missing: the base is the named `/BaseEncoding`, else the embedded
program's own encoding, else for Symbol and ZapfDingbats their built-in ones (`std14::symbolic_char`, PB-25;
tables generated with the widths by `tools/gen_std14.py` from the AFM glyph names, through the Adobe Glyph List
and Adobe's ITC Zapf Dingbats Glyph List, whose BSD notice is in `std14.rs`), else Standard, with `/Differences`
over it, and each glyph name read by the Adobe Glyph
List, `uniXXXX`/`uXXXX` and parts joined by `_` (`glyph_text`). The program's encoding comes from read-fonts (as
`skrifa::raw`, the version skrifa uses, so no new dependency): `Type1Font`'s for `/FontFile`, the CFF encoding and
charset for `/FontFile3 /Type1C` and OpenType CFF, and for TrueType that is symbolic or has no (3,1) cmap, each
code's glyph by the (3,0) cmap (the code at 0, 0xF000, 0xF100 or 0xF200) or (1,0), named in `post`. The programs'
tables are kept by a hash of the stream's bytes (256 programs), glyph names' text too. lopdf's glyph list and
encoding tables are private, so they are reached through lopdf's own encoding reader: a font whose one difference
is the name (`listed`), and fonts naming a base encoding (`named`). lopdf alone passes over an `/Encoding`
dictionary without its optional `/Type` and drops every difference when one name isn't in its list, which this
reading doesn't. A named `/Encoding` stays lopdf's. `FontCtx::reusable` (writing a form value back in the
field's own font as ASCII) is false for Symbol and ZapfDingbats, whose codes don't mean their ASCII characters. On
the render corpus (1,089 files) unreadable characters in
extracted text fell from 225,473 to 191,418 (files with any from 123 to 106), 29 files' text changed, each checked
against `pdftotext`, and none got worse.

Which code writes a character in a line's own embedded font (ENH-31, batch 31; `own_font.rs`) is answered by
`Pdf::line_fonts`: a `LineFont` for each part of the line, from `fonts_of` (the page parsed once, each font read
once per call, its program read with the decoding context the page is read with), holding the codes the line and
the page show in that font; `Pdf::page_line_fonts` gives every line's at once, the page parsed and each font read
once (the app reads a page's lines' fonts with it the first time one is selected). `LineFont::code` gives a character's
code only when one reads as exactly that character (`FontCtx::decode_code`, and for Type 0 fonts `known_text`,
which leaves out lopdf's last-resort guess), its glyph is found as hayro 0.9 finds it to draw (simple TrueType by
the code's glyph name through (3,1), then (1,0) by Mac OS Roman's code, then the first glyph so named in `post`, or
by code through (3,0) or (3,1) at 0, 0xF000, 0xF100 and 0xF200 when the descriptor's flags aren't nonsymbolic, an
undefined flag counting as symbolic as hayro's `from_bits` does, the cmaps read as read-fonts reads them, a `DMAP`
table's first; Type 1, whatever the program, by `/Differences` name, else the program's own encoding, else the base
encoding's name, a name found in an OpenType program's `post` before its CFF charset, and `nbspace` and `sfthyphen`
also as `space` and `hyphen`; CID fonts through the first 65,536 entries of `/CIDToGIDMap` or a CID-keyed CFF
charset), isn't `.notdef` and draws an outline (blank characters excepted), the PDF's width is within 2% or 2
units of the program's advance, a simple font's ToUnicode text matches the character its encoding names
(`disagree`; not symbolic TrueType, drawn by code), and the program's cmap read backwards, else the glyph's name,
doesn't make it another character.
Several codes for one character: the line's, else the page's, else the lowest when they draw one glyph, else
`Ambiguous`. The base encodings' glyph names are `base_encodings.rs`'s, generated by
`tools/gen_base_encodings.py` and checked equal to hayro's tables (code 0 `.notdef`, WinAnsi's unused codes past
40 bullets, MacRoman with Mac OS Roman's codes after it; MacExpert unread). A program is read by what it turns out
to be, whatever key holds it (OpenType without `maxp` as its `CFF ` table); a TrueType font's only from `/FontFile2`
and only when TrueType or OpenType, since hayro draws any other in a standard face; the unit tests
`simple_fonts_find_each_code_s_glyph_as_hayro_draws_it` and `cid_fonts_find_each_cid_s_glyph_as_hayro_draws_it`
run hayro's interpreter over every code of the test fonts and compare the glyphs it draws, so a hayro upgrade
that finds glyphs otherwise fails them. Refused as a whole
(`FontRefusal`): Type 3, not embedded (where plain ASCII is still written as itself when `FontCtx::reusable`),
a CMap other than Identity-H, vertical writing, a program that can't be read or isn't one hayro draws with, and
LimarePDF's own fonts (`embed::KEY`), which their pen grows. Each character is checked when first asked (kept in
the font's `checked`), so a large CID font's glyphs aren't all drawn; its codes are read once, all 65,536.
A page's fonts are read once while one edit writes several of its lines (`PageFonts`), and each `LineFont`
also holds how each of the line's characters is drawn (`edit::drawn`, below), shared by the line's fonts.

An edited line is written in its own fonts from that (ENH-31's writer, batch 31, stage 2; `edit.rs`). `drawn`
walks the line's shows as the parser reads them and gives each character what draws it (`Ink`: a code, or a
whole string where `codes_known` and `drawn_alike` can't vouch for its parting, as one unit; or a move read as
a space, a spacing number under -150 times the part's em (`Span::em`, as the parser reads it) or the gap to the
next show measured from piece to piece) and what lies
between it and the character before (that one's show's trailing numbers and no-character glyphs, the move to
this one's show, then this show's leading items). The app sends each character's place in the line as read
(`TextPart::origins`, `None` once typed or pasted; a words-only `TextEdit` gets them from the common start and
end, `parts_for`). `plan` decides each character: its own code when it is the line's own, unchanged, where it
was (a ligature's code only with all its letters, together), else `LineFont::code`, else a space as a move,
else missing (a character of a script not written yet always, PB-53); `pieces` sends each word with a missing
character (a word by UAX #29 since batch 45: punctuation and each ideograph stand apart), and the spaces between
two such words, to `FontChoice::of_font` (plan Q2), set through the pipeline below, which falls back further for
what that font lacks. `PageObject::kept_pieces` and `keeps_font` make the same decision for the app
from the `LineFont`s. `OwnPen` writes the kept pieces show by show, keeping the items between characters that
were neighbours as read (so text before the first change draws where it did) and each show's Tc, Tw and Tz;
new codes take the spacing of the show they join; a space with no code is the part's nearest move read as a
space, else -250; a move that would start a show (where it reads as no space) goes to the end of the show or
piece before. Spacing numbers that would stand side by side in a `TJ` are written as one, and one that moves
nothing is left out (`tj`): a reader takes each number alone, so the move back across a `Tm` of the line as read
followed by that show's own number forward again would read as a word gap (batch 31, stage 3; pdf.js
`issue19182`). A part that is its span's own text, unchanged, keeps its shows as they were. A gap read as a space
after one of its shows (`Run.gap`, which counts the next show's opening number) is written as the page had it
(S1-14): before a show kept as it was, only the move from where the one ends to where the next starts, the next
keeping its own number (PB-47: the whole gap was written and the number kept too, moving the words after it);
before a part written anew, after the show where a `TJ` number that size reads as a space (`reads_as_space`), as
a number of its own (`then_gap`: `tj` would join it to a number the show ends with), else as the next part's
opening number, which the reader counts between shows (PB-30: a 3-point gap after a
40-point show is −75, no space, and "Big smell" read back "Bigsmell"). A line in one part,
its style unchanged, in one spacing, starting with a show of characters, written all in its own font, becomes
one `TJ` in place of its first show (`in_place`); others go on top as before. Every line with characters
written this way is read back (`misread`; a line changed in place is found by its operator and may read as one
with text next to it): one that reads otherwise is written again with only its unchanged parts in its own
font, and its id returned (the app's "similar font" message). One that reads right is then checked for where its
words sit (`Pdf::misplaced`, S1-14, from the new operators parsed as the page will be): words that end before
the first changed character must start where they did (the word the change is in may move), and those that start
after the last changed one must all move by one shared amount (the line's reflow), each within 0.5 points
(`kept_extents`, from the edit's origins: a line with no word before the change and fewer than two after it isn't
checked). A line that fails is written again as a misread one is, held to the same words as its first try (a
misread line's retry is held only in its parts kept whole), and one that fails again refuses the edit with nothing
changed. Each try given up puts the page's
own `/Resources` back as it was before the first (PB-46: a retry left the first try's font named on the page;
`add_resource` writes only there, never through a reference).

The typing preview draws a kept embedded font's own glyphs (plan Q6): `LineFont::preview` writes a small
OpenType font (`preview_font.rs`, LimarePDF's own writer: CFF outlines recorded with a skrifa pen, a
TrueType program's quadratic segments as the cubics that draw them exactly, coordinates rounded to whole
units of a 1000-unit em; cmap formats 4 and 12; `hmtx` from the PDF's widths; `head`, `hhea`, `maxp` 0.5,
`post` 3, `name`, `OS/2` 4 with the descriptor's ascent and descent), holding the characters the line shows
with a code of their own, as it shows them, and up to 4,096 in all of those `code` gives. egui 0.37 reads it
with skrifa 0.44, which draws it and reads its metrics as the 0.50 core uses does: the font's charstrings are
moves, lines, curves and endchar, and 0.50's changes to CFF (Type 2's arithmetic and storage operators, an
operation limit, building multiple-master instances) touch none of them. The app makes it on its loader
thread (`typing_fonts.rs`, `Candidate::Preview`, keyed by `LineFont::preview_key`: the program and font
dictionaries hashed, with the codes the line shows), falling back to the Liberation face of its kind, then
Inter. TJ kerning isn't drawn while typing, and a ligature glyph's letters draw in the Liberation face.

Search (`text.rs`, batch 22) reads a page once into a `PageText`: its words in reading order (`ordered_words`,
through `Pdf::search_parts`, by page object so the page tree is walked once a search, not twice a page) with a
space between them, each character folded (lowercase) and original, its text line and place there, the lines
that join (for `joints`' hyphen and soft-hyphen skips) and each line's pieces, so a match's snippet and boxes come
from the same reading as the match. `find_on` matches a page's text with the start of the next page's appended
after a line break (as many characters as the query, three times over, for the skips), so a phrase or a
hyphenated word across a page break is found on the page it starts on, with boxes on both pages; its `spill` (the
next page's characters it took) is where the next page's matching starts, and `TextIndex::find` looks a page again
from there when its own first match began inside them, so a match is never found twice and the result is what one
reading of the whole text would give, a group of pages at a time or all at once. `TextIndex` keeps the pages read,
by page object: `update` forgets the pages an edit changed, or all of them when the pages themselves changed.
`Pdf::search` and `search_with` (the CLI) and `find_text_matches` (one page alone, no next page) use the same code.
Snippets are cut at word boundaries (`tail`, `head`; `Snippet::around` cuts shorter for the app's rows), and
`match_count` says the count in words for the app and the CLI.

A `Query` holds its options (`SearchOptions`, NEW-04a), so a changed option is a new search on the thread as a
changed query is. Match case compares the page's original characters (`orig`, at the same indices as the folded
ones, as `fold` maps one character to one) with the query only whitespace-collapsed (`collapse`), the skips worked
out from the same text; Whole words only is a `keep` test `find_all` asks of each candidate before taking it
(`whole_words`), so a candidate refused doesn't swallow the characters a later one starts in: no letter or digit
may run on into the match's first or last character from just before or after it, the skipped characters
passed over, so a word hyphenated at a line end is one word; across a page break `TextIndex::find` says whether the
page's first word goes on from the page before (`runs_on`, the joint rule `joints` uses, read from the page before
when it is in the index). Each hit also carries its `HitPart`s: per text line it touches, the page object
(`PageText::objects`, the page's own text lines in object order as `page_words` numbers them; lines inside form
XObjects none) and the characters there, the spaces between its words included, which Replace edits.
`Query::count_in` counts a query in plain text, for what a replacement holds of what it replaced.

In the app, `search.rs` runs a tab's searches on a thread of its own, started with its first search and ended
with the tab. `Tab::poll_search`, each frame, notices the query changing (`note_query`: a query begun from empty
starts from the page being read), clears the old matches at once, and starts the search once the typing has paused
for `PAUSE` (200 ms); Enter starts it at once (`Tab::search_keys`, the field's own Enter turned off with
`return_key(None)`) and otherwise steps. Each search gets a number: the thread stops the one running, between pages,
once a newer number is stored, takes only the newest of the jobs waiting (keeping the document a passed-over job
brought, `merge`), and sends its matches a group of 16 pages at a time with the number; `Search::take` drops
anything for another number, so an older query's matches are never shown. An edit (`soft_refresh`, `refresh`)
calls `Tab::search_changed`: the matches go, the pages it changed are noted (`Search::text_changed`), and the next
frame searches again, the document cloned for the thread only then and only with a query in the field, the current
match kept by its page and place there (`ToHit::Keep`). The results view (`Tab::show_results`) checks the query
again after its own field, before drawing, so a query typed in that frame shows no old rows; its rows are of fixed
height (a page heading, a two-line match) and only those in view are drawn. Mark all for redaction uses the
matches when the search for the query, its options and the document as they are is complete (`Search::complete`),
and otherwise runs `Pdf::search_with` itself first. The threads' indexes count against one budget (`fit_within`,
256 MB), letting go of the tab searched longest ago, then the pages already searched of the tab searching. The
options are the tab's (`Tab::search_options`), beside its query, noticed with it (`Search::seen`).

Replace (NEW-04a, `tab/replace.rs`, `editor/replace.rs`) types over a match the way typing over selected words
does, never substituting bytes. `Act::Replace` (Ctrl+H, `CT` so a Mac's Cmd+H stays the system's) shows Search
results with `Replace`'s field; its button and Enter there are `Act::ReplaceMatch`, which joins `uses_document`, so
typing waiting on the page is kept first, and is greyed through two `Ctx` conditions with `why_not` reasons
(`found_match`: `Tab::match_ready`, the search shown is for the query, options and document as they are and its
current match has come; `search_waits`). `Editor::replace` takes the hit's parts: on another page, in a group
(no object), invisible or vertical text, or across blocks it refuses with a `NotReplaced::Match` note drawn under
the match by `Editor::overlay` (`note_at_match`); a replacement `page_text::untypable` refuses in the font at the
match's start (the filter typing uses) is `NotReplaced::Text`, said under the field. Otherwise one line opens with
`start_text_edit(.., Pick::Chars(a, b))` (as a paragraph where it would), several lines of one block with
`paragraph_over` (shared with `words_paragraph`, `across` so a block LimarePDF is sure of opens with paragraph
editing off), and the replacement goes in as typed events (`Event::Text`, or `Delete` for an empty one, the range
first widened by a space by `with_a_space`). The editor's `park` flag then, in the frame the events are typed,
sets `Parked::Typing` (as F6 does), gives up the keyboard there (at the next frame's start the app would see the
line focused and unpark it) and sets `Editor::typed_over`, on which Search results puts the keyboard back in the
field: the line waits open, and Replace again (or a click, which unparks) keeps it as one step. The tab makes the
next match current without moving the view, remembering the match typed over (`Over`: hit, page, and how many
matches the replacement holds, `Query::count_in`); `Tab::search_changed` counts the current match again from it
(`after_replacing`: one fewer on that page after it, plus those the replacement holds), so after the search runs
again the current match is still the next. A Replace pressed while that search runs waits (`Replace::waiting`) and
goes on from `take_search` once the current match is placed (`replace_when_placed`), unless typing began
meanwhile or nothing is left. A waiting replacement that ends unchanged is forgotten (`forget_replaced`).
`Act::FindNext` and `Act::FindPrevious` (F3 and Shift+F3, Ctrl+G and Ctrl+Shift+G) go through `Tab::find_step`,
as Enter in the search field does; `App::shortcuts` lets them through with a text box focused, as F6, so F3
passes a match over from Replace's field, and they show Search results, where the search keeps up, without moving
the keyboard (Find, with nothing to search for). Menus and tooltips show a command's key through
`actions::shown_key`: the first, but on a Mac the Cmd one before a plain function key (⌘G, not F3). Alt+R is
the Replace button's key while Search results' fields, options or button have the keyboard, as Word's Find and
Replace window has it, read in `replace_ui`, not in `ACTIONS`, and not on a Mac (Option+R types there).

Replace all (NEW-04b, `editor/replace_all.rs`, the tab's side in `tab/replace.rs`) makes every replacement on a copy
of the document and reviews it before the document takes it. `Act::ReplaceAll` (Replace all beside Replace, Alt+A
read beside Alt+R, Edit > Replace all matches) joins `uses_document` and is greyed by `all_replaceable`: Replace's
conditions and `Ctx::search_running` (the search shown hasn't looked through every page, `Tab::search_running`).
`App::replace_all` keeps the page's typing, takes `Tab::replace_all_job` (the document, each hit's `parts`, the
replacement on one line, `Editor::paragraphs`, `doc_words`), runs `editor::replace_all` on a thread (`App::busy`, the
status bar saying so) and sends `Loaded::ReplaceAll`; a result for a tab whose revision moved, or with typing on its
pages since, is put aside with a message, as a job's is. `replace_all` plans, then writes:

- **Planning** (`plan`, the pages in parallel with Rayon, read-only on the document): each match is refused as
  `Editor::replace` refuses it, through the same functions (`replace.rs`'s `page_of`, `lines_of`, `typed_font`,
  `untypable`), counted as a `LeftOut` reason; otherwise it goes to its line (`Of::Line`) or to its paragraph
  (`Of::Paragraph`, by block) where Replace would open one: across a block's lines with paragraph editing on or a
  block LimarePDF is sure of, one line with paragraph editing on, the block not `refused` and its `paragraph` and
  `paragraph_setting` made (kept for writing). A line in a paragraph changed goes with it, so each line or
  paragraph is one change holding all its matches.
- **Writing** (`write`, on a clone of the document): a page's paragraph changes one at a time, then its line
  changes in one `Pdf::edit_texts`. Each change is a `Styled` read from the line (`Styled::of`) or paragraph
  (`of_paragraph`), the matches put in from the last (`put`: `Styled::sync` with the caret after each, so a
  replacement takes the style of what it types over, as typing does; an empty one widened by `with_a_space`), then
  `text_change` or `Styled::parts` with `ParagraphSetting::set`: the writers typing uses, never bytes. A rewrite can
  move the page's object ids, so each change finds its lines again by `follow` (the same kind, text and box as the
  page was planned); the first change on a page uses the plan's objects and paragraph setting. The copy is held
  (`Pdf::holding`), so core doesn't copy it again for each change, nor put one back when it fails: a change that
  fails marks it, and `settle` writes a new copy of the document without it (the failed write may have left some of
  itself in the old copy, which is dropped).
- **Judging** (`judge`): the page is read again and each line written anew is given to the change that wrote it (a
  set paragraph line where it was set, else the edited line it starts on the baseline of); its rows go through the
  one overflow rule, `page_text::overflow` over `obstacles_in` (shared with `Editor::obstacles`, in page points),
  against the page's other things but those the old text met. Another font is counted from `kept_pieces` (a line,
  with `page_line_fonts`) or `ParagraphSetting::pieces` (a paragraph) where a piece not kept touches a replacement.

`Prepared` holds the `Pass` with every replacement and, when some run over and others don't, one written again
without the changes that ran over (`fits`). The review, `dialogs::replace_all` (`ReplaceReview`, made by
`Tab::replace_review`, the pages named by `page_name`, the first eight), joins app.rs's `question` list; Replace all
or Replace where it fits takes that copy through `Tab::take_replaced`: `edit_pages` over the pages changed, the
whole document swapped in as `spawn_job`'s result is, one undo step, a notice with Undo. Nothing replaceable is a
message from the same review.

Text LimarePDF writes (new and edited text, headers, footers, watermarks, stamps, text boxes, form values, the
signature look, selected text as a plain page) is set through one pipeline, core's `layout.rs` (below, "New text is
set through one pipeline"): a `Setter` lays it out, then a `Pen` writes exactly the lines laid out.
`writable` (core) tells the app which characters can be typed: those the font asked for, or a font it falls back
to, can write. Form appearances measure through `form_ap::Metrics`: `Bytes`, a field's own one-byte font's widths
(the field's font kept), or `Set`, a `Setter`. In the
app, new text is typed on the page in `crates/limarepdf-gui/src/page_text.rs`, each word drawn while typing in the
face core will write it in (`page_text::runs`: a standard font's bundled Liberation face, which `typing_fonts.rs`
picks, with the widths of Helvetica, Times and Courier; an installed family's file; the installed face a word the
font lacks falls back to, by `limarepdf_core::set_in`), then written line by line with `Pdf::add_text_parts` as
one undo step. A line being edited (`editor.rs`'s line editor, sharing
`page_text`'s drawing) sits on `PageObject::baseline` and each part is drawn in the font
`PageObject::keeps_font` says `Pdf::edit_text` will write it in: the part's own (installed, else its Liberation
look-alike), the installed family it is from, or a standard one. `page_text::job` puts a line's parts of
different sizes and rises on one baseline, and each line's baseline `line_gap` below the one before, through
egui's per-part line height and vertical alignment: egui (since 0.37) centres a part in a line height of its own,
so each line's biggest part is made as high as the row and sets the baseline, and the others get the height and
alignment that put them on it (`parts_of_a_line_share_its_baseline`). Both edit a `styled::Styled`, the text with a
style per character: it keeps the styles through egui's plain-string `TextEdit` by comparing each frame's text
with the last, applies Word's rules for style commands, gives the inspector its values (`None` where the
selection mixes them), and hands core the line as `TextPart`s; `styled::History` is the undo inside the line,
which takes Ctrl+Z and Ctrl+Y before the `TextEdit` does. `Styled` keeps where each character was in the line as
read (`origins`, `None` once typed); `Styled::owned` keeps those of runs still as read, cut back to whole words
as read and as typed (letters typed onto a word change it), and `parts_in` gives them `TextPart::from`, so
untouched words keep their shows. Words selected with the read tool are restyled through `chosen::Chosen`, made
by `Editor::sync_chosen` at the start of each frame when the selection changes: each `PageWord` says its text
line (`object`, `None` inside a form XObject) and characters, so each line touched gets a `Styled` and its
selected ranges (words with only spaces between them one range), collected in a `styled::Across`, which the
inspector's `styles_ui` changes through the `styled::Restyle` trait as it changes one line (`InLine`): Bold and
Italic by Word's rule over the whole selection, blanks where any line mixes a value. Invisible lines, words in
form XObjects and vertical writing are counted (`chosen::Left`) and said. Changes go in as `Typed::Words` (one
`Action::Restyle`, every page's `edit_texts` in one undo step); `Editor::reselect` selects the same words again
after the edit's `invalidate`, and after `Tab::undo` and `Tab::redo`, only if they still read the same. Ctrl+B and Ctrl+I are commands
(`Editor::switch_style`, also for a line selected with the arrow) and page-only keys, left to a text box with
the keyboard. Selected words are painted as word processors shade a selection (`Editor::shaded`): a band a
reading row (`line_boxes`, what `selected_lines` gives the marks and redaction boxes made from a selection), a
paragraph's rows meeting halfway across the space between them (painting only), in the theme's selection
background at 45% (PB-31: perhaps too dark). Every other text box uses egui's own undo. While any
text box has the keyboard, `App::shortcuts` leaves the undo keys to it, so the document's Undo never runs from
inside a box; with a button holding the keyboard (`text_edit_focused` false) they are the document's. The
commands row's Undo and Redo do what the keys do there (batch 40's Q4): a press or release on them
(`App::undo_buttons`, their places last frame) while a text box has the keyboard sets `App::undo_held`, which sets
egui's `SurrenderFocusOn::Never` for that frame so the box keeps the keyboard, and `App::act` turns the command
into `App::undo_in_box`, sent to the box as Ctrl+Z or Ctrl+Shift+Z at the start of the next frame, before anything
else reads that frame's keys. They are greyed by that typing's history: `Editor::typing_history` reads the
`History` of new text and a line being edited (`History::can_undo`, `can_redo`), `page_text::box_history` egui's
undoer for the boxes that keep egui's own. Each tab's text boxes take their ids from `Editor::scope` (the tab's id)
and what they edit, and the inspector is drawn inside that id, so no two documents share a box's state.
`page_text::fresh_history`, run before each frame's keys, starts a box's undo afresh whenever it gets the
keyboard, except when an editor hands the keyboard back mid-typing (`page_text::hand_back`, a note after its
colour is picked) or the keyboard comes back to the box it last left with nothing kept in the document since
(ENH-32: `App::left_box` holds that box's id, its tab's id and the tab's revision then). A new note or text box
uses its page's one box id (`Editor::new_box`), so it never takes the history of the last one typed there. egui keeps only the
caret of a `TextEdit` without the keyboard, so the inspector's style controls read the line's or new text's
selection through `page_text::held`, which notes it while the box has the keyboard and gives it while a list, the
size box or the colour picker has it; `page_text::give_back` selects it again as the box takes the keyboard back
(BUG-12). The text colour button (`editor::text_color_button`) has its own pop-up around egui's
`color_picker_hsva_2d`: a press on its colour area or hue slider is a pick even of the colour it started from, and
mixed text offers its own colours to pick. egui's pickers work in linear light and a PDF's RGB values are
gamma-encoded as the page shows them, so every colour button converts at the boundary (`editor::to_linear`,
`from_linear`, BUG-38), and `editor::color_button` writes a colour back only when the picker changed it. The line
underneath is
covered in the colour the page worker (`worker.rs`, `Worker::sample`) finds around it, from a drawing of that
part of the page in the current page colours.

The read tool's words are also typed over, deleted and cut as in a word processor (batch 40's second stage).
`App::type_over`, run after `App::shortcuts` while no text box, dialog or the palette has the keyboard, hands the
frame's `Event::Text` and `Event::Paste` to `Editor::type_over`: with words in one line (`Chosen::one_line`), it
opens that line (`Editor::words_line`: the line of the selection's moving end, its selected characters as
`Pick::Chars`, from `Chosen::line_ids` and `selected_in`) and keeps the events in `LineEdit::typed`; `line_edit_ui`
splices them in front of the frame's events once the line has the keyboard, so they take the line editor's own
path (its refusals, the selection replaced, its history). Keys typed in the frame a double-click opens a line are
kept the same way (they were lost before). Words across lines of one paragraph (one LimarePDF is sure of, or with
paragraph editing on: `Editor::words_paragraph`) open the paragraph with the typing there (stage 3's plan, Q3);
words across paragraphs get the note "Type over words within one paragraph." at the selection's end
(`Editor::words_note`, drawn by `words_note_at` through `page_text::Refused::show_at`); words only in text that
can't change get `chosen::Left`'s notice. Delete and Backspace (`deletable`) and Cut call
`Editor::delete_words`: `Across::delete` takes each line's ranges out back to front (`Styled::delete`), and a space
beside a range with it, by Word's smart cut, when spaces sit on both sides or the range is at a line's start or
end; the result is one `Action::Restyle` across lines and pages, nothing left selected. A line emptied this way is
written as an empty `() Tj` in place of its first show (core's one-style in-place path): it reads and draws as
nothing, saved too. Lines aren't joined (paragraph editing, NEW-01). `Act::EditObjectText` with words selected is
`Editor::edit_words` (the moving end's line, its part selected). Shift with the arrows, Home and End is
`Act::Extend(selection::Toward)` (keys only: not in ☰ since ENH-49, nor in the palette):
`Editor::extend_by` moves the selection's moving end by a word, to the word on the line above or below nearest
across (`editor::across`), or to the line's start or end, reading the next or previous page with words when it
crosses one; `Tab::reveal` (from `Editor::words_end`) then scrolls the least that brings the end into view, against
the view kept from the last frame's scroll area (`Tab::last_view`). The read tool's double-click and triple-click
off a line's box select nothing (`Editor::word_on_line`: within the hit tolerance of the nearest word's line).

The typing says what it does (batch 40's third stage). `page_text::field` draws the outline in two lines (ENH-22):
the edge in the accent, or `theme::Tone::Caution` when a warning applies, and a 1-point line inside it
(`outline.shrink(1.0)`) in `theme::halo` of the edge, white or black, whichever contrasts more, so it shows on
paper of any colour, the accent's own too. `page_text::Style::warn` (and `ParaStyle::warn`) checks the text's
rows as laid out each frame by one rule for the three editors (PB-35, batch 45's 3b-2): `page_text::overflow`
takes the first row that leaves the page's rect (a point's leeway) or, across the middle half of its height
(where rows above and below touch), meets one of the obstacles, and says what (`page_text::over`: "Over other
text", "Over a picture", "Over a shape") and where: at the leftmost thing it meets, or the side of the page it
crosses (`Overflow { why, at, leftward }`). The obstacles are `Editor::obstacles`: the page's visible objects but
invisible text, a background over 80% of the page (`background`) and what each editor leaves out: a line being
edited (`line_edit_ui`) its own object and whatever its old box meets (`meet`); a paragraph its pieces and
whatever its old lines' box meets; new text (`adding_ui`, the user's Q3) a picture or shape its first line's start
meets where it sits now, so a label put on a coloured box isn't warned about the box (other text always counts,
and moving the box by its outline re-reads what it starts on). `outline_ui` puts the tag just below that row at
that place (its right end at the page's edge when the right edge is crossed), a `Label` so screen readers have
it, and returns its rect: `Field::tag`, which the line's and paragraph's `area` (`Editor::line_area`) take in, so
the floating bar (`bar_place`) keeps clear of it as of the text. A line being edited has the inspector rows
"PDF font" (`editor::pdf_fonts` of the selection, a subset's six-letter tag cut by `editor::untagged`) and "Source
font" (`editor::source_font`: `part_font` for each part still in a font of the line's own, a part given a chosen
font skipped, named Preserved, Installed family or Nearest standard; several, Partly preserved when one is
Preserved, else Mixed; none, Font chosen), with its second value, a standard font standing in, giving the
Non-standard note when the line's font isn't reusable. The status bar says "Editing one line" while
`Editor::editing_line`. `Tab::title` (the window's title) and `tab_button` take `Tab::unsaved` (`dirty`,
`Editor::pending` or typing in forms), so typing marks the tab from the first key. `writing_ui` keeps any box
being written open on a press in the inspector (`insp` is always `drawn_at` the inspector's place), and a text box
takes its text size and border each frame, a new one (no annotation id, not a note) from the Text box tool's
`box_style`, one already on the page from its annotation's `text_style` in `Editor::annots` (batch 45), so it
shows what it will be written with as the inspector changes it.

Typing on the page draws what core will write (the text design's section 5; batch 45, 1b). `page_text::runs`
splits each part by core's `set_in` (the document's `/Lang` from `Pdf::language`, noted in `Editor::lang`) and draws
each stretch in its own face (`typing_fonts::Wanted`); a kept part of a font LimarePDF wrote itself
(`FontRefusal::Own`: no preview, no installed font by the PDF's name) is drawn as `Wanted::Standard` of
`editor::font_of` its name, split the same way. A box of its own (a form field's, a text box's) is laid out by
`page_text::boxed_galley`, its `TextEdit`'s layouter (a password's dots excepted): the box's font (`Field::font`,
or Helvetica for a text box), each word it lacks in core's face. A box that wraps takes its lines from core, never
from egui's wrapping: `page_text::set_in_box` lays the text out with `BoxSetting::lines` (each line's bytes in the
text), puts a line break after each line core broke that has none, lays out with `job` at the box's leading
(`BoxSetting::leading`, 1.15; `job` keeps 1.2 elsewhere), then clears those rows' `ends_with_newline` and gives
the galley the text as typed (sections' bytes mapped back), so the caret, selection and egui's double-click read
the text the user typed; it adds the margin that puts the text where it is written (sides: border and padding,
`BoxSetting::margin`; the first baseline 0.8 times the size below that). Its rows go where `/Q` places the lines
as written (PB-59, batch 49's stage 3): `BoxSetting::placed` gives each line's start in from the text's left as
`form_ap::draw` aligns it (`align` over `shown`'s width, one `Metrics` for both), and `moved_rows` moves each row
by it, leaving the galley's `rect` as laid out: a right-aligned row's trailing space hangs past the side, and a
galley widened by it would be centred in the field by `Ui::put`, moving every row (found in the stage, 2 px).
`writing_ui` (text boxes: an existing one's setting and area from `Pdf::text_box_setting`, a new one's
`BoxSetting::text_box` with `Writing::border` and `Writing::align` from the box's `TextStyle`, `aligned`) and
`fields.rs`'s `text_box` (multi-line fields; the field's `BoxSetting` from `Pdf::field_setting`,
which sets `/Q` as the writer does, `widget_q`, kept in `Edit::Text`'s `set`, a `OnceCell` read when the box is
first drawn) use it; single-line fields and passwords lay out on one line, a single-line field (not a comb) placed
by `TextEdit::align` from `Widget::align` (centred, or at the right less the 2-pixel margin, where the writer puts
it less the border and padding). The caret and deletes go by grapheme cluster:
`page_text::Clusters` is the `TextBuffer` of every typing box (Delete and Backspace take a cluster, Backspace only
the last mark of a letter typed with separate accents, variation selectors excepted; its `type_id` is
`Clusters<'static>`'s), and `snap_to_clusters` (`snap_drawn`) moves a caret egui left inside a cluster to its
edge, the way it was going (`cluster_edge`), only when the caret moved and no `Ime` event came that frame (a
composition's range is the input method's). Words go by UAX #29 (`editor::unicode_pieces`: punctuation and each
ideograph a word; at a boundary the word with letters, then the one starting there) for `word_around`,
`word_spans` and opening a line at the word clicked; egui's Ctrl+arrows and double-click already were. Input
methods (PB-54): egui writes a composition into the box, selected and unchecked, and replaces it with what is
committed, which `leave_out_untypable` filters. `page_text::composition` follows the composition from this
frame's `Ime` events and the selection; each box keeps it (`composing` in `PageText`, `LineEdit`, `Writing`), and
one drawn without the keyboard takes it as committed and filters it (`commit_left`, `commit_left_styled`), as
Windows and macOS input methods commit when a box loses the keyboard; `Editor::keep` does the same for ends that
don't draw the box again (a band button, `set_tool`), and what was left out goes to `Editor::left_composing`, said
as the tab's info (`page_text::left_note`). With words selected and nothing holding the keyboard,
`input_method_at_words` sets egui's `IMEOutput` at the selection's end, so the input method is on and its first
composition types over the words (`App::type_over` passes `Ime` Preedit and Commit; `Editor::type_over` opens the
line only for a non-empty one); egui-winit turns the input method on only while some `IMEOutput` is set. The
inspector's notes: `page_text::fallback_note` names the words that go to another font ("Not in Calibri: 漢字.
Those characters use Noto Sans CJK JP.", which word uses which font on hover) for a line being edited, new text,
a new or existing text box and the form field typed in (`FormUi::typed_text`); `editor::font_note` (batch 31's,
which can show beside it) names the words, as typed, each once, that a line's own font can't write ("Not in this
PDF's font: größe Maße. Those words use Helvetica."). Not covered: a composition left in the inspector's
annotation text box reaches core, which refuses it with BUG-39's message; form fields have no typing filter at
all (core refuses what can't be written when the value is written); another app's text box with `/RD`, a cloud or
a callout can wrap otherwise while typed than as written (PB-60).

The interface's own text falls back to installed fonts too (ENH-33, built in batch 45's 1b instead of eframe's
`system_fonts`): `typing_fonts::interface_fallback`, called each frame from `App::ui`, starts once a thread that
asks core's `fallback_faces("漢あ한😀")` for the installed faces LimarePDF writes those characters in, reads each
file up to 64 MB (`MAX_INTERFACE`; larger collections are left out, and the interface shows boxes for them) and
adds it at the end of every interface font family, where typing uses it too. Typing and the interface can read
one face at once: both put what they read through `typing_fonts::arrived`, which keeps the other's `interface`
flag and waiting `then` list. Before that, a face typing was still reading when the interface's thread read it
was replaced by typing's read without `interface`, so Chinese, Japanese and Korean interface text showed boxes
for the rest of the run; a cold disk cache made typing's read slow enough to lose
(`a_face_read_for_typing_and_the_interface_at_once_serves_both`). Arabic, Hebrew and Indic scripts would still
show unjoined: egui doesn't shape them.

Several egui behaviours shaped this. `TextEdit` deletes and steps by `char` (hence `Clusters` and
`snap_to_clusters`), and a `TextBuffer`'s `type_id` must name a `'static` type. egui puts a composition in the
box's text, selected, and when the box loses the keyboard only collapses the selection, leaving the composed text
as typed; a focused box gives up the keyboard while it is drawn in the click's frame, so a band button can end
typing before the box is drawn again (hence `Editor::keep`'s part). A `Galley` can't be built outside epaint
(`intrinsic_size` is `pub(crate)`): it is laid out with line breaks added and its rows' public fields changed, and
its `job.text` must be the text as typed, as egui's double-click picks words from `galley.text()`. And a glyph's
`pos.x` is only its whole pixel: epaint draws a glyph (colour and CJK ones aside, which round) at its pen position
to the nearest quarter pixel, so where a glyph lands is measured from the pen (the sum of the `advance_width`s
before it), as the half-pixel test does. epaint shapes each section with its own `harfrust` 0.12 and no language;
the half-pixel test would catch a placement that differs from core's 0.13.

Greyed commands say why (`actions::why_not`). A command's `enabled` is a function of `actions::Ctx`, so `why_not`
gives a copy of the context each thing a command may wait for, in the order people meet them (a `Fix` each: busy,
saving, a file, undo, redo, words, typing, a line, a picture, an object, a movable one, items put down with Fill
and sign, redaction marks, a form, a field, the Select
tool, the Read tool, facing pages), and returns the reason of the first that alone enables the command, else of
the one that does with all those before it, else `NOT_NOW` ("Not available just now."); without a document, "Open
a document first." `actions::greyed_tip` is the disabled hover UI of the band's buttons, the commands row, the
command search and Search results' Replace buttons; `greyed_ui` draws it (since the user's findings on NEW-15's
stage 3, whose reason was lost as the tooltip's third line): the name and shortcut (`titled`), then the reason after
`icon::INFO`, both in `theme::Tone::Caution`'s colour, as `unavailable` says it ("Not available now. " and the
reason, its capitals kept for a font's name; `NOT_NOW` alone), then the hint. The band's menu buttons give their
label and first item's reason, `menu_item` the reason and hint (the label is the item), `font_list::styles` the
reason through `greyed_why`. `font_list::styles` names
B and I with their keys, and greyed says the family has no such face while the group is enabled, else the group's
reason. Ctrl+B and Ctrl+I in new text and in a line go through `page_text::switch_styles`: `Styled::switches` says
whether the switch would change anything, and if not `Refused` marks "<family> has no bold." at the caret.
`page_text::note` gives `SCRIPT` for a character core's `unwritten_script` (right to left, or shaped) marks,
outside an emoji sequence. `App::type_over` with a paste and no box to type in sets `Tab::info`, saying where to
click. Tool commands in menus are `egui::RadioButton`s (`menu_item`), so the tool in use is marked. A tool's
command chosen again (`App::act`'s `Act::Tool` with that tool on, not the read tool) is `set_tool(Tool::Read)`, as
Edit text and Crop already were, so a pressed tool button turns off; `set_tool` lets go of `pending_image` for any
tool but Image. `band::is_on` presses Edit text by `Ctx::edit_text` (`Editor::edit_text_on`): a line or
paragraph being edited, or the Text tool chosen with Edit text (`Editor::edit_text`, its `by_edit_text`, which
every `set_tool` clears), so Insert > Text's tool alone doesn't press it. `Editor::page_ui` gives form fields the
Text tool's plain click as the read tool's (`typing_tool` beside `filling`; links still take Ctrl+click), keeping
new text being typed first (`finish_adding`). `inspector::title` is "Form field" while a field has the keyboard
(`form.focused_field()`), and the app clears the selected words then, after `settle_look`.

Selected objects (ENH-06, batch 25; design `limarepdf-multiselect-design.md`) are one `Selection::Objects`: a page
and one or more `selection::Selected` (the object's index in `Pdf::page_objects`, its kind, box and a line's text),
in the order selected, with `loose` (page text among them moves with the read tool: text just added, Move or resize
text). `Selection::object` gives the object only when one is held, so everything built for one (handles, line
editing, Replace image, the one-object inspector) is unchanged. An index is only good until the page changes: a
deleted object drops out of the list and a rewritten line goes to its end. So after every change the objects are
found again (`Selection::refind`, run by `Editor::objects` whenever it reads a page's objects anew): at its index
when the object there fits (kind, text, a box meeting the one held, `Selected::fits`), else the fitting one
overlapping it most, else it is let go, never swapped for another. A change says first where each will be or what
it will read (`Selection::expect`: `Tab`'s `SetBox` gives the boxes, `EditText` the typed text, keeping its own
fallback for a line read back differently). With the arrow (`page_ui`'s `start_arrange`), Shift+click toggles one
(`Editor::toggle_object`), a press on nothing, on a background not held (`Editor::background`, the read tool's 80 %
rule) or with Shift away from the held objects starts a `Marquee`, selecting what is wholly inside it
(`inside_rect`, invisible text only while shown) on release (`marquee_done`; Shift adds), and a press on a held
object moves them all (`start_move`: `Drag::group`, the box round them all as `orig` and `cur`; `held_boxes` draws
them moved). The read tool's Shift+click toggles a picture or shape too, unless words are selected, which it
extends. With both tools a drag begins (`page_ui`'s `begins`) only once the pointer is further from the press than
egui's click distance: egui takes a press held still for 0.8 s for a drag, so it waits in `Editor::still`, and let
go before it moves it is a click (`still_click`). Ctrl+A is `Act::SelectAllObjects` (the arrow's), sharing the
key with `SelectAllText` (the read tool's and the Text tool's, `actions::text_tool`) by being enabled for another
tool, and a page-only key left to a text box. Several held get no handles: each is
outlined and a dashed box drawn round them all. Core changes several objects of a page in one rewrite
(`Pdf::set_object_boxes`, `transform_objects`, `delete_objects`, `set_path_styles`, the one-object functions calls
of them), the ids as the page is before the call, refusing all if any is refused; `pick_all` maps each operator to
its object, and `Following::all` finds once what goes in before which operator after them (also for
`edit_texts`), so a page of 27,000 objects is gone through once. The inspector for several (`Editor::group_ui`)
restyles their lines through a `Chosen` made from them (`chosen::Of::Lines`, each line whole, as batch 24's words)
and their shapes through a `Group` (each shape's `Look` as it is and as changed, `Typed::Objects`), and moves them
by the box round them (`Group::at`). `Tab::after_insert` selects everything a change put down (`Editor::select_new`
from the object count before), so new text of several lines is held as one.

Align, distribute and snap (NEW-08, batch 46's stage 5) are pure functions over boxes (displayed-page points, y
down) in `editor/arrange.rs`. `Act::Align(Align)` and `Act::Distribute(Axis)` call `Editor::align` and
`Editor::distribute`, which act on what `object_moves` allows (anything with the arrow; the read tool's pictures,
shapes and `loose` text) and queue one `Action::SetBox` holding only the boxes that move: one `set_object_boxes`
rewrite and one undo step, none when nothing moves, the selection following through `Selection::expect`. Two or
more align by the box round them (`union`), as Word aligns selected objects; one alone by its page, its size from
`Tab::sizes` (as the view draws it). Distribute orders the boxes by their middles (ties by index), keeps the first
and last where they are and gives the rest equal gaps, negative when the boxes are wider than the room. Snapping
lives in `drag_to`: the first time a move (`Mode::Move`) or resize (`Mode::Handle`) of objects follows the pointer
it reads an `arrange::Snaps` into `Drag::snapping`, the edges and middles of the page (its size as drawn, `r / k`)
and of the page's other objects a click could select (not those moving, nor a line's decorations, invisible text
only while shown), kept for the drag. A move snaps the nearest of the box's edges and middles to the nearest line
within `SNAP_PX` (6) screen pixels, across and down apart; a resize snaps only the edges its handle moves
(`moving_edges`), and with Shift on a corner the nearer of the two, giving the scale `resize` keeps the proportions
with. `Snaps::guides` makes a dashed line (`GUIDE`, a pink seen on any paper) through each place meeting a line,
along the box and every box meeting it there, drawn in `page_ui` after the marquee. Alt (`page_ui`'s `keys`) and
the app's `Settings::snap` (`Act::ToggleSnap`, lent to each tab's `Editor::snap` every frame as `paragraphs` is)
turn it off; comments (`Drag::annot`), a line's ends and turns never snap, nor do nudges. The band's Align objects
(Word's Align, named by its tooltip so it isn't read as aligning text) is a menu button (`band::ALIGN_MENU`), the
objects' right-click menu shows `actions::ALIGNS` as an Align objects submenu (`menu_of`), which `toolbar_items`
leaves off the floating bar and the docked buttons (as word processors' bars do), and the band's menus draw the
File menu's separators (`actions::menu_list`, `menu::line_before`). `Ctx::objects` counts the objects held,
for Distribute's three (`why_not`: "Select three or more objects first.").

Gestures (PB-14): `Editor::escape` first lets go of a move, resize or turn (`drag`), a shape (`draw`), a selection
rectangle (`marquee`), a pen stroke or a word selection being dragged (`text_drag`), changing nothing and keeping the
tool; only then does Esc do what it did. egui forgets a drag whose widget isn't drawn, so the page a gesture is on (`Editor::gesture_page`) keeps
its `page_ui` drawn by `Tab`'s page loop while it is out of view, and letting go ends the gesture there. The
inspector's Text and Comment boxes keep a change when they lose the keyboard (PB-20), as Tab moves on to a button
and `settled` waits while any widget has the keyboard.

Typing not yet in the document (`Editor::pending`: new text, a note or text box, a line or the inspector's
object changes, a form field, a comment's changes or a reply; the inspector's forms) goes in before anything uses
the document. `Tab::keep_page_typing` keeps each kind (`editor::Typed`) as its own undo step through
`Editor::keep`, which takes it out of the editor and queues its change; if the change fails, `Editor::put_back`
returns it, as it was, and the caller stops. It runs in `Tab::edit`, `Tab::edit_removing`, `Tab::undo` and
`Tab::redo`, before Crop shows the pages whole, in `App::spawn_job` (Print, exports, OCR and scans take their copy
after it) and before a job's result replaces the document (typing since the job started discards the result).
`Tab::keep_typing` adds the forms and runs in `App::act` for every command `Act::uses_document` names (an
exhaustive match: a new command must be classed), so a command's question (`App::asked`) is about the document
with the typing in it. Replies are drafts kept per comment (batch 42's Q1, as Word and Google Docs keep them):
`Editor::drafts` maps a comment's id to its page and text, the inspector's reply box (its own id per comment, so
its typing undo is the comment's) edits the selected comment's entry, an empty one is dropped, and nothing moves
when the selection changes. `Typed::Reply` keeps one draft a call, and `Tab::keep_pieces` loops it, so every draft
is its own undo step and one that fails is put back alone. Undo and Redo keep `Typed::BUT_DRAFTS` (`App::act`
through `Tab::keep_typing_but_drafts`, and `Tab::undo`/`redo`): a draft added first would be what Undo took back.
After every change and Undo or Redo, `Tab::place_drafts` finds each draft's comment with core's
`Pdf::annotation_pages` (one walk of the page tree, by the pages that list them): a draft follows its comment's
page, and one whose comment is gone has no page, so it is neither pending nor kept, until Redo brings the comment
back. Reload replaces the whole tab, and is refused while `pending` (DA02), drafts included. A form field that loses the keyboard to a dialog keeps its typing and takes the keyboard
back when the dialog closes (`Edit::Text`'s `back`); one whose own change is refused as it is left gets its typing
back the same way (`FormUi::kept`). Panel Apply buttons that redraw only some pages or nothing don't commit first:
they lose nothing, and their forms join their own undo steps.

Typing ended on the page (Esc, Enter, Tab, a click away, another tool, the selection moving on) is kept by
`Editor::keep_later` (new text, a note or text box, the selected object's line and look; BUG-41): `Editor::keep`
takes it and queues its change, and the held typing waits in `Editor::later` by the change's place in the queue.
`Tab::apply_actions` notes each change that fails (it set `Tab::error`; `Tab::failed_at`, before each change and
after the last, so the arms' `continue`s need no care) with the revision then, and hands them to
`Editor::settle_kept`: typing whose change was made is `done`; typing whose change failed is put back
(`Editor::put_back`, the line selected again) and given the keyboard (`refocus_typing`), unless a later change in
the same batch moved the revision, when the held object id may be another's and the typing is left out instead.
Put back, it is remembered as it was (`Editor::not_kept`): ended again unchanged it isn't tried again but dropped
(`refused_again`), and `Editor::left_out` makes the tab say so (`LEFT_OUT`). The look changes of words or several
objects (`Typed::Words`, `Typed::Objects`) still use `keep` alone: nothing typed is lost, and the inspector shows
the document's look again. Typing open on the page waits under any dialog (`dialogs::any_open` in `line_edit_ui`,
`adding_ui` and `writing_ui`), its Esc and clicks being the dialog's, and takes the keyboard back when it closes; a
failed change's message is such a dialog.

Undo steps (`tab.rs`'s `Step`; batch 43, PF-42) keep only what each change changed: `Step::doc` is an
`Option<Arc<Restore>>`, `None` for a change of the password settings alone, made by core's
`crates/limarepdf-core/src/history.rs`. Core's own operations are atomic ([Failure atomicity](contracts.md#failure-atomicity)):
each that changes the document runs in `Pdf::atomically`, which takes `Pdf::before` and calls `Pdf::go_back` when it
fails or panics, unless the document is held. `Tab::edit_inner` takes `Pdf::before` (a `Before`: the document
cloned) before the change and runs it in `Pdf::holding`, so the operation doesn't copy the document again, and after it `Before::undo(&pdf)` compares each object with its old value and gives a `Restore`: the old values of the
objects changed or removed, `None` for those added (to be removed again), and the version, binary mark, trailer
and highest object number when any of them changed. The comparison is order-sensitive and bit for bit (`same`,
`same_dict`): lopdf's `==` ignores a dictionary's key order and takes 0.0 for −0.0, which are written
differently, so a change of either would have been missed and left in the document by Undo. `Pdf::restore` puts a
`Restore`'s values in place and returns the one that reverses it (Undo's gives Redo's, and back); `Pdf::go_back`
puts the document back whole after a change that failed or panicked, or whose redraw panicked. A change joined to
the last step (typing into a line again soon, a setting nudged again) merges its `Restore` into the step's
(`Arc::make_mut`, `Restore::merge`, which keeps each object's older value), so Undo goes back to before the first.
`Tab::restore` can't fail and reads nothing again, so what was read when the document was opened stays as it was:
that it was protected, repaired or reads differently in other apps, lopdf's cross-reference data and encryption
state. Before batch 43 Undo put a written-out copy in place with `Pdf::reopen`, a new reading, which read all of
these afresh from a clean copy: a password-protected document came back as unprotected, so signing it was no
longer refused (`undo_keeps_a_password_protected_document_protected`). Object numbers are never used again in a
session: `restore` and `go_back` never lower `max_id`, so what the app keeps by object (a draft reply by its
comment, the selection, pages) can't come to mean another object; stage 2's tests found an undone comment's number
taken by a new object, which sent a draft reply to the wrong place. Writing a copy no longer changes the document
either: `Pdf::snapshot` puts `max_id` and the trailer back after lopdf's `save_to`, whose writer takes a new object
number for a cross-reference stream and writes the stream's keys into the document's trailer, so every copy
written of such a file (each redraw's, each undo step's) changed the document a little.

At most `MAX_UNDO` (100, as Word and LibreOffice keep by default) steps and `UNDO_BUDGET` (1 GiB) per tab are kept,
the oldest dropped first and the latest always kept (`trim`), counted by `Restore::size`: an estimate of the memory
the values hold (`map_size`, `heap`, `dict_heap`: the map's nodes and lopdf's dictionaries' tables as they are
allocated), within 1 % of a counting allocator's count (`tests/history_size.rs`, which allows 0.75 to 1.33 times it).
A step is the size of what changed: a note on a 100-page scan keeps about 5 KB where a whole copy was 82 MB. Some
are larger than a whole copy was: the first page change on an opened document flattens its page tree and so
changes every page dictionary (28 MB on 10,000 pages), and a watermark on 10,000 pages keeps 32 MB. Each change
clones the document first and compares after: a peak of about one copy of the document's heap, 16 ms on a 100-page
scan and 21 to 24 ms on 10,000 pages (performance.md's [Undo history](performance.md#undo-history)). The pages are still drawn from a written-out copy after each
change and each Undo and Redo (`refresh`, about 43 ms on 10,000 pages; PB-37).

A watermark or headers and footers just added stay linked to the inspector's form (`Tab::stamp`, a `Stamped`) until
anything else changes. The link's `base` is the `Restore` that takes the document back to before the stamp: when
it is added, the same `Arc` as its undo step's, so it costs nothing while that step is kept; after each update, the
link's joined with that update's step (`merge`), which adds the objects this stamp changed that the last didn't
(nothing else changed them since the stamp was added). An update (`put_stamp`) restores a copy of the base and adds
the stamp again, as one step, and draws again the pages it is on and those it was on (`StampForm::range`, read
again: nothing else changed since, so it gives the same pages); before batch 43 only the new pages were drawn
again, so a stamp updated for fewer pages still showed on the pages it left (`pages_a_stamp_leaves_are_drawn_again`).
`held` counts every `Restore` once, by `Arc::ptr_eq`, wherever it is held (steps, the links kept in steps, the form's
link). When the latest step and a link's `Restore` no step holds are over the budget on their own, those links end
(`Tab::keep_to_budget`, run after every push, every joined step and `put_stamp`) and `Tab::info` says so (BUG-36).

While the Watermark or Headers, footers, Bates tool is open and its stamp isn't added yet, the pages show it
(`tab/previews.rs`, `tab/proofs.rs`; ENH-23, the user's Q2 of batch 40). `Tab::stamping` says which stamps go on
which pages this pass. While a setting is changing (`inspector::changing`: the pointer held, a text field with
the keyboard, a pop-up open), the stamp is drawn over the pages, laid out by core's own functions
(`Watermark::text_start`, `picture_size`, `HeaderFooter::lines`) in Liberation Sans. Once they settle,
`Tab::sync_proofs` (once a frame, before the pages are drawn) clones the document on the window's thread and
`Worker::spawn_from` puts the stamps on the copy off it (`stamp_onto`, the code adding uses), then starts a page
worker of its own on the copy's snapshot. Its drawings of the pages and tiles in view and of the stamped pages'
thumbnails (`Proof`, keyed as the tab's own) replace the tab's own in `draw_previews` (`proof_page`) and
`show_thumbs` (`proof_thumb`, while `proof_shown`). A proof is for one `Ask` (the revision, the page colours and
the stamps with their pages): one for anything else is dropped, never shown, and a copy finished for settings
passed through is let go; a copy that can't be made (a picture file gone) leaves the drawn previews, logged as
"stamp preview failed", and adding the stamp says what is wrong. The document, its undo, its revision and whether
it has unsaved changes never change. Not done: the clone on the window's thread (once per settled change);
drawing the pages beside those in view ahead; proof drawings are logged as "render" with the tab's own.

Toasts (`App::toasts`, at most one per tab) belong to a tab by its id. One for an undoable change (`Tab::notice`)
records the tab's `Tab::history` (revision, undo and redo depths) and goes once that moves or typing starts, so its
Undo runs only while the change is the last thing in the tab; `Tab::info` notices have no Undo. A tab's toast waits
while the tab is behind (`TOAST_AWAY`). Tabs are closed by index through `App::close_tab`, which keeps the tab in
front, and `App::close_asking`; a confirmation names its tab by id (`Asked`). A tab's close button records the
tab's id, and the tab closes after the frame's commands (`App::ui`): the rest of the frame is drawn from what was
read at its start (`has`, the commands' state), so nothing after the tab strip reaches for a tab just closed (the
delta audit's DA01: closing the only tab crashed in the navigation rail).

New text is set through one pipeline (`crates/limarepdf-core/src/layout.rs`; the text design, NEW-02, batch 45's
1a), so what is measured is what is written; a writer never measures or encodes text another way. A `Setter`
(`Pdf::setter`: the font asked for, all the text it will set, and the document's `/Lang` from `Pdf::language`)
picks each word's font, shapes, measures (`Setter::width`) and breaks lines (`Setter::lines`) without changing the
document. `Pdf::pen_with` then makes a `Pen` for exactly the lines a writer will show: every glyph they need is
embedded first, each face's font object grown once, so no glyph is written that wasn't embedded; it is refused
with nothing changed when a character can't be written in any font or the font asked for can't be read
(`Setter::missing`, `unusable`). `Pen::ops`, `content` and `shows` give a line's operators by its index (`Tf`
where the font changes); `Pdf::pen` is both steps at once. Every writer uses them: `edit.rs` (new, edited and
restyled lines), `annot_ap.rs` and `annot_write.rs` (text boxes), `form_write.rs` and `form_ap.rs` (`Metrics::Set`;
an appearance laid out first as `form_ap::Drawn` and written once the pen exists), `overlay.rs`, `stamps.rs`,
`sign.rs` (`standard_tj`), `sig_sign.rs` and `excerpt.rs`. **Fonts** (4.3 of the design, adjusted): a word, as
Unicode's word rule finds it (UAX #29, `unicode-segmentation`'s word bounds: each Chinese or Japanese ideograph a
word of its own), goes to the first font that has all of it (`Setter::pieces`, `find_font`): the font asked for (a
standard font as itself while every character of the text fits WinAnsi, else its embedded face); installed fonts
named for the word's script (`script_fonts`, each in the asked font's kind, sans, serif or monospaced, and its
nearest style: for Chinese, Japanese and Korean the families `installed_fonts::Language` lists, the language
being the document's `/Lang`, else kana or Hangul in the text, else the `LC_ALL`, `LC_CTYPE`, `LANG` or
`LANGUAGE` variable, then the others in the order Simplified, Traditional, Japanese, Korean; `EMOJI` for a single
emoji; Noto Sans or DejaVu for Latin, Greek and Cyrillic; "Noto Sans {Script}" for others; `SYMBOLS` for symbols);
the asked font's standard stand-in (its Liberation face); any installed family that has the word
(`typefaces::every_typeface`); else it is refused. Any family comes after the stand-in so a Latin letter Calibri
lacks goes to Liberation, not to the first family alphabetically. A space between two words that went to one font
goes with them. An installed font is asked through its character map first (`could`, through
`installed_fonts::maps_all`: the system font list's mapped file, read only while asked, its answers kept for the
run, so looking through every installed font keeps none of them), and only then read whole (`covers`). Each word's
font is kept for the run (`UNITS`, by font asked, whether it is written as itself, the CJK language and the word)
until `typefaces::typeface_count` changes; that count doesn't start the scan of installed fonts, so text the asked
font has never reads them (`tests/font_scan.rs`, its own test binary, keeps it so: new code that runs as text is
typed reads coverage only through `maps_all` and must not start the scan). A word it lacks needs the list (`typefaces::family_named`,
`every_typeface`) and the fallback faces' files, so the app reads the list on its start-up thread
(`font_list::warm`), then calls `warm_fallbacks`, which sets the words 漢 (and あ and 한 for Japanese and Korean)
when `LANG` or `LC_*` names Chinese, Japanese or Korean, and 😀 always, once in each of Helvetica, Times and
Courier, so the first such character typed doesn't wait for them; tests never call it. **Shaping:** a face's text is shaped by `harfrust` (`Face::shape`, its plans kept by script, language and
kind) with the font's defaults (kerning, ligatures, contextual forms, marks placed), the document's `/Lang` as the
language and a variable font's coordinates, in stretches of one font and one script (`items`, `unicode-script`;
Common and Inherited characters join the stretch they are in). A standard font written as itself has no
ligatures (WinAnsi has none): one code a character, at Adobe's widths plus the kerning its Liberation face's pairs
give (`Face::shape` with `kerning_only`, `liga`, `clig` and `calt` off). The PDF's own fonts aren't shaped (ENH-31's
writer, above). **Writing:** `shows` places each glyph where shaping put it, with a `TJ` number wherever the
widths the font gives would leave it more than `DRIFT` (0.005 thousandths of an em) off; a glyph raised or lowered
(a mark) starts a show with its own `Ts`, put back after. **Line breaks:** `break_ranges` (UAX #14 through
`unicode-linebreak`) breaks at line breaks, then greedily where the rule allows (after spaces and hyphens, between
ideographs, never before closing punctuation or small kana), a word wider than a line between grapheme clusters;
it gives each line's bytes in the text, trailing spaces in (hanging: not in its width) and line breaks out, and
`break_lines` maps them through `shown` (trailing spaces off, a soft hyphen "-" at a line's end only), so the two
can't differ. They replace the three space splitters (`form_ap`'s wrapper, `excerpt.rs`'s, `sig_sign.rs`'s): the
excerpt starts no line after a final line break, and the signature's name shrinks before a word is broken.
`BoxSetting` (`form_ap.rs`, public) lays a box out as `form_ap::draw` writes it, for the app to wrap typed text on
the same lines: `lines(text, lang)` (the size, a field's auto size fitted as `draw` fits it, and each line's
bytes; `draw`'s `Body::Lines` uses the same `wrapped`), `placed(text, lang)` (those lines, each with its start as
`/Q` aligns it), `margin()` (border and `PAD`), `leading()` (1.15); made by
`BoxSetting::text_box(width, border, size)` (Helvetica; `.aligned(align)` sets its `/Q`) or
`Pdf::field_setting(name, widget)` (read only: the
field's own font as `pick_font` keeps it, its widths unkerned, else the standard font nearest; `OwnFont` and
`form_write::own_font` shared with `pick_font`; `Widget::id` names the widget). An existing text box's is
`Pdf::text_box_setting(id)` (PB-60, S1-18), a `TextBoxSetting`: its `BoxSetting`, the area its text is set in as
shown (in the page's displayed points), that area's own width and height, and whether it is upright as shown.
It comes from `annot_ap`'s `BoxGeometry` (`box_geometry`: the turn, `/RD` or a cloud's inset, `/DA` else `/DS`
for the size and colour, the border), which `free_text` draws from and `fit_text_box` grows by (PB-146), so the
three can't differ; a box not upright as shown is typed in upright at its text's size, centred over the area.
Core's other public text API:
`fallbacks(f, text)` (the words set in another font, and the font: what the inspector names), `set_in(f, text,
lang)` (each stretch's characters and font as it is set, a `SetIn`: a standard font as itself or in its embedded
face; `None` for what no font writes: what the typing box draws and decorates, `SetIn::lines`),
`fallback_faces(samples)` (the installed faces those characters go to: the interface's fallback),
`warm_fallbacks()`, `Pdf::language`, and `Field::font` (the standard font nearest a field's `/DA` font, Helvetica
when it names none; `forms.rs`'s `da_font`).

Text a standard font can't write as itself, and text in an installed family, goes into an embedded face
(`crates/limarepdf-core/src/embed.rs`, `Pdf::embed_glyphs`): the bundled Liberation face of the standard font
(`LIBERATION`, in core's `assets/fonts/`, stored compressed by core's `build.rs` and unpacked by `bundled_face` the
first time each is used; the app's `typing_fonts.rs` draws with the same copy, added to egui then too) or the
installed family's face, as a Type 0 font with `Identity-H`, `/W` widths and a ToUnicode map, the font file cut
down to the glyphs used by `subsetter`. A TrueType face (a CIDFontType2 descendant, variable fonts set at their
weight included) embedded since batch 45 has codes of its own, one for each glyph and the text it is written for,
mapped to its glyphs by a `/CIDToGIDMap` stream: a ligature's glyph maps to all its letters, and a glyph two
characters share (Latin A and Cyrillic А, in many fonts) to each, so both read back as typed. A CFF face
(CIDFontType0) takes no map, so its codes stay its glyphs' numbers, and a glyph wanted for a second text goes to
another font object of the face; the faces LimarePDF 0.11 wrote (`/CIDToGIDMap /Identity`) keep growing that way,
with their rounded widths. New objects give `/W` to a thousandth of a unit. Otherwise there is one font object per
face per document, rewritten in place as text is added: the old codes are given to the subsetter first, so they
keep their numbers and earlier text stays valid. The font dictionary's `/LimarePDF` entry (`embed::KEY`) records
the face (`Face`, its PostScript name; `Source`, a checksum of its file, index and settings), each code's glyph
and text in order (`Texts`), whether the codes are its own (`Codes /Own`) or its glyphs' (`/Identity`), whether
`/W` is exact (`Widths /Exact`) and a checksum of the font file written (`Sum`); 0.11's `Glyphs` list (each
glyph with its character) is still read. A font another app has changed since, or made from another version of
the face, is left alone and a new one started. In a signed document only fonts made since it was opened
(`Pdf::made_fonts`) are grown, as growing one a signed revision holds would show as a change to it. Going back to
an earlier state in the same session (undo and redo, a failed change put back, a stamp updated from its base) puts
the old objects back in place (`Pdf::restore`, `Pdf::go_back`; batch 43), reading nothing again, so the set stays
as it is (PB-07; `a_made_font_keeps_growing_after_undo_and_redo`): object numbers aren't used again in a session,
and signing opens the signed file in a new tab with no history, so no font of a signed revision can be in it; an
id the earlier state lacks belongs to nothing signed, whatever takes it later. Before batch 43 Undo read a copy in
the document's place with `Pdf::reopen`, which keeps the set (now used only by core's tests); reading the copy
with `Pdf::from_bytes` alone started a new font each time.
Characters
are put together (NFC) first; right-to-left and shaped scripts are refused (Liberation has Hebrew). A face covers
a character only when its glyph has an outline (`Face::outlined`, found once per glyph through skrifa), except
characters meant to be blank (`embed::blank`: spaces, default-ignorable marks, the blank Braille pattern): a
colour emoji's glyph is often empty, drawn by `COLR`, `CBDT` or `sbix` data a PDF has no use for, and written it
would show nothing. Text is spelled a grapheme cluster at a time (`unicode-segmentation`): an emoji sequence
(`embed::emoji_sequence`: a skin tone, regional indicators, the keycap mark, tags or the emoji variation selector,
or a zero-width joiner between pictographs) is refused whole, as its parts written one by one would say something
else (BUG-24; PF-14 is the support). A zero-width joiner outside one is an invisible mark and is written. A
character a chosen font draws only in colour goes to another font with its outline, when one is installed. What
no font writes is refused with `embed::cant_write`'s message (PB-34, batch 45: an emoji sequence, "flags, skin
tones, keycaps and joined emoji aren't supported"; a script not written yet, "right-to-left writing, and scripts
whose letters join, such as Arabic, Hebrew and Hindi, aren't supported"; else "No font on this computer has …";
batch 40's "Latin, Greek and Cyrillic characters are supported" is gone; a character that doesn't show, such as a
control, private-use or zero-width one, is named by its number, "the character U+F8E8", not quoted where no one
can see it: `embed::named`, PB-72), `box_refusal` wording the same as the
end of a message naming a text box. The app's caret note (`page_text::note`) is `SEQUENCE`, `SCRIPT` or `REFUSED`
("No font on this computer has this character"), naming no other font, as any font that has a character is
already tried; `page_text::keep_typable` keeps or leaves out a whole cluster. A line's own font (`edit.rs`'s
`plan`) takes no character of a script not written yet (PB-53): its word goes to the fallback, which refuses it
with the reason, as on every other path.

Every path that writes typed text checks it the same way (BUG-24), against the font asked for and the fonts it
falls back to: new and edited lines through `Pdf::pen`; text
boxes and fill-and-sign text, always Helvetica, through `embed::check_writable` in `Pdf::add_text_box` and in
`Pdf::set_annotation` for a box's new text, with the app leaving out at the caret what Helvetica and its fallbacks
can't write (the
text box on the page in `Editor::writing_ui`, the inspector's Text box in `annot_inspector`, Fill and sign's Text
in `SignPane::ui`, all through `page_text::leave_out_untypable` and `page_text::note`, which filter typed
text, pastes, committed input-method text and middle-click pastes alike; and the other way, `App::offer_selection` sends the
page text selected in the tab in front as egui's `TextSelectionSettled` once it settles, which egui-winit puts in
X11 and Wayland's PRIMARY selection); headers, footers,
Bates numbers and watermarks through `Pdf::pen` and `inspector::stamp_text`; form values through
`form_write::check_value`; stamp wording through `stamp_wording`; typed signatures through `typed_signature`; the
signer's name on a signature's look through `sig_sign::shown_name` (left out when it can't be written). Comments,
replies, bookmarks, document properties and page labels are text strings (`doc::encode_text`, UTF-16 with
surrogate pairs past U+FFFF), which take any character. A text box another app wrote may have characters no
font can write (`Pdf::box_unwritable`, through `embed::unwritable`: a script not written yet, an emoji sequence, a
character no installed font has; since batch 45 its Chinese, say, falls back to an installed font as any text
does): `Pdf::check_box_drawable` refuses whatever would draw it again
into the file (a new look in `set_annotation`, `set_text_style`, a resize in `move_annotation`; BUG-39), and
`flatten_annotations` refuses when such a box has no drawing, naming its page (`embed::box_refusal` words the
reason). `annot_ap.rs`'s `typeset` draws such characters as `?` only in drawings in a copy: the app's view and
page images (`snapshot_hiding`), and printing (`Pdf::print_sheets`).

Installed fonts for writing text are listed by `crates/limarepdf-core/src/typefaces.rs`: the families in
`InstalledFonts::system()`'s `fontdb` database whose fonts allow embedding, each with
up to four faces (`FaceFile`: file, index, variation settings, PostScript name), read once (the first of
`typefaces()`, `every_typeface()` or `family_named`; the app starts it on another thread at launch,
`font_list::warm`). Font lists offer only families with the Latin letters (`typefaces()`); the others (emoji
fonts, fonts for one script) are `listed: false`, found by `every_typeface` and `family_named` for the
pipeline's fallback only. The list only grows, so `Family::Installed(n)`,
its number, stays valid; `FontChoice` stays `Copy`. `embed::Face::of` reads an installed face from its file
the first time and keeps it, and the same embedding applies: always embedded (no WinAnsi path), static CFF fonts
as CIDFontType0 with `/FontFile3 /OpenType`, variable fonts set at their weight by `subsetter`'s `variable-fonts`
feature (TrueType out, CFF2 included). `FontChoice::of_font` finds the family a PDF font is from by its
PostScript name, for edited lines; the standard fonts' own names never match an installed font. A family's
`name` is the font's own; `Typeface::label` is what lists and messages show, the familiar typeface first for a
font with a code name (`FAMILIAR`: the URW base-35 fonts and their gsfonts names, "Palatino (P052)"), and the list
is sorted by it (`by_label`); `FontChoice::family_name` gives the label. In the app, `font_list.rs` is the
inspector's searchable list (names drawn in their own fonts through `typing_fonts::preview`, files over 4 MB left
in the interface font; a search matches the label, which holds both names; `about` is a family's tooltip), drawn
across the panel, with Bold and Italic apart (`font_list::styles`) so the caller puts them in one row with size and
colour; and `typing_fonts.rs` draws typed text from the family's file with egui's variation coordinates
(`typing_fonts::coords`). The list's tooltip names the PDF fonts of the selected characters
(`editor::pdf_fonts`).

The inspector's heading is `inspector::title`: for the editor's section, `Editor::heading` (a selected comment,
else the tool's options, else the selected object; the object's own heading is drawn again in the panel only below a
tool's options; `Editor::object_heading` calls a line of page text "Selected text", as selected words are, since
batch 39); else the task open (`Section::title`), else "Selected page", "Selected pages" or "Document" (see the
workspace's inspector below). An object's X, Y, width and height are in a `CollapsingHeader` with a fixed id
(`position-and-size`), so it starts closed and keeps its state from one object to the next; one field a row, as
the section's indent leaves no room for two in the default width (two widened the panel). A line of page text has no
text box in the inspector: its words are typed only on the page (`line_edit_ui`), which the arrow opens too (a
double-click; Edit text through `Editor::edit_selected_text`, the caret at the end). `Editor::end_line_edit` finishes
it (Esc, `escape`, the keyboard going elsewhere): it keeps the typing and, unless the tool is the arrow or the line
was made movable (`loose`), clears the line's selection when it is still that line, so the panel goes back to what
it showed before. Text colour buttons are `text_color_button`, painted as an A over a bar of the colour (the
colours side by side when mixed), the bar outlined in the text colour so black shows on the dark theme.

Redaction (`crates/limarepdf-core/src/redact.rs`) has its own pass over the page content rather than editing the
parser's objects: it rewrites each text-showing operator character by character, keeping the rest in place with
spacing numbers, cuts drawings into sub-paths, and redacts inside form XObjects as copies, as deep as a page's text
is read (`crate::MAX_FORM_NESTING`, 8; WP-03, 7 before); a deeper form under a mark goes whole. Pictures are decoded,
blacked out and saved again in `redact_img.rs`. Afterwards the pages are parsed again to check that nothing readable
is left under the marks. Hidden information (metadata, scripts, attachments, hidden layers) is removed by
`sanitise.rs`. See [Redaction](../redaction.md).

What is still drawn after a redaction or Remove hidden information is found by `reach.rs` (S1-01: H-01, H-02,
PB-84), one walk over every page, annotation appearance, form, pattern, soft-mask group and Type 3 glyph, each
visited once per `(id, fingerprint of the resources it inherits)`, so drawings that draw themselves end. It gives
the nodes and what each draws, the objects drawn (`Reach::drawn`), every dictionary listing resources
(`Reach::holders`, streams' included), each node's alternative text (named and inline `ActualText`, `Alt`, `E`),
the pictures and forms it draws (`Node::xobjects`), the `/Properties` entries its marked content uses
(`Node::properties`) and whether its content could be read (`Node::unread`: one that can't counts as drawing and
using everything its resources list). `Reach::certain` marks the nodes certainly drawn: pages, every
annotation appearance state (`Reach::appearances`) and what a readable node certainly drawn draws.
`find_elsewhere` reports removed text met in alternative text by the pages reaching it
(`Reach::pages_drawing`). Each rewritten copy (a page's resources, a form's copy) keeps only the `XObject` and
`Properties` names its new content uses, plus those its kept inheriting streams use, followed down
(`Redactor::needs`); a kept inheriting stream that can't be read keeps every name, and `report.left` names its
page. A replacement (a cut picture, a redacted form's copy) is bound under a fresh name in the
copy made for the marked invocation only; an original name is never re-pointed, so a kept drawing drawing it by
an inherited name keeps the original (S1-01's amendment A1, CP-03). Fonts and other resource kinds are left
alone.

After the change, `retained.rs` (`Pdf::clear_retained`; amendments A2 and A3, CP-01 and CP-02) works with the walk
after and what redaction removed (`retained::Removed`: the originals replaced or removed under a mark, the
alternative text taken off, the redacted pages' old content, thumbnails and private data, the annotations
removed). An original is kept when a certainly drawn readable node draws it, and those pages are reported
(`RedactReport::pictures_elsewhere`, one notice a page); used only by nodes that can't be read, it stays and the
page goes to `report.left`; drawn by nothing, it is unlisted. Then every `/Resources` and `/DR` in the file, at
any depth in any object, whatever holds it and whether or not anything draws with it, is found, and in their
`/XObject` and `/Properties` dictionaries the entries for unused originals, and properties holding removed
alternative text that no node uses (by reference, or a direct one by equality), are worked out first, reading
only, then removed in place, shared dictionaries too. Last, an independent walk follows every reference from the
trailer, as saving keeps it: the tag tree (`StructTreeRoot`) and private application data (`PieceInfo`) are
followed after everything else, so an object something else reaches counts as that, and each route still
reaching anything removed is a `report.left` sentence naming it ("removed content may still be in the file,
through the tag tree"), which makes both callers refuse: the app rolls back, the CLI writes nothing. Nothing is
cut on those routes (that needs its own design).
Removing hidden layers takes the same steps (`drop_hidden_layers`, PB-153; PB-111 with it): a walk before the
change (which refuses, as the check does, when the drawings are too complex) and one after every edit; the
removed set is the pictures and forms drawn before and not after, with those carrying a hidden `/OC` of their
own, and the tiling patterns and soft masks painted with before and not after (`Reach::painted`: `scn` and
`SCN` with a name, and the soft mask of a state `gs` sets, `reach::mask_drawing`; `Removed::painted`);
`clear_retained` unlists them from every `/XObject`, `/Pattern` and `/ExtGState` (by the state whose mask it is)
in the file, and a route still reaching one refuses, rolled back by `atomically`. Redaction passes `painted` too
(PB-177). Both first take out of each rewritten content the pattern and state settings nothing paints with
(`reach::drop_unpainted`: `scn` or `SCN` naming a pattern, and `gs`, replaced, restored or left at the end before
anything paints; a text operator showing no characters, as redaction's spacing-only `TJ`, paints nothing), so a
setting left from removed content doesn't keep its pattern or soft mask painted with. Widgets in a hidden layer
leave their fields as redaction's do (`prune_fields`, PB-178), not only the pages' `/Annots`. `prune_fields`
first takes each removed widget out of the `/Kids` of the field its `/Parent` names, and a field left without
kids out of its own parent, iteratively (`unparent`, PB-179), so a field the form's list leaves out doesn't keep
one; then it walks down from the list as before.
Decoded content is cached by object (`reach::Decoded`) and shared by the walks before and after a redaction,
which only adds objects; Remove hidden information edits streams in place, so its walk after starts afresh.
Its scan for hidden content (`scan_layers`, WP-03) runs on `Pdf::reach_listed`: the same walk following every
name the resources list, used or not, without reading the content, each set of resources (by fingerprint)
followed once, so it costs a step per entry of each drawing's resources and per name listed, not per page. Each
node carries its stream (`Node::stream`, `None` for a page's content) and the resources it draws with
(`Node::res`); a drawing without resources of its own is looked into with the first and compared with the others
(`check_shared`). Drawings are followed to any depth within the walk's budget; the scan's own recursion, with
its 32-deep limit, is gone. `drop_hidden_properties` stays a scan of every object, the boundary's safe superset.

The walk is charged against a `budget::Work` (`budget.rs`, `TooComplex`), one step per drawing reached, name
looked up, page, annotation, holder and entry of inherited resources. `reach::STEPS` is 2²² (4,194,304): 32
times the corpus maximum, 129,066 steps (`pdfjs/issue6692.pdf`, 109 pages; the 1,080 files that open, measured
2026-11-04 with the budget lifted). The walk runs first as a check, refusing before anything changes; the walk
after the change is allowed twice the limit plus the objects added, so it can't fail midway on a document that
passed (argued, not proven: should it, the app undoes the step). Finding hidden information (the count run on
a copy, not strict) leaves the pictures listed when that walk fails rather than refusing. The budget is part of
[Redaction's contract](contracts.md#redaction).

Reading a page (`objects.rs`, `parse_as_page`; S1-06, H-07) has a budget of its own: a step per operator and per
form opened, plus one per KiB of the form's content, charged across every form the page draws, however often (a
form is read again each time it is drawn), one per cell the underline grids go through (S1-07), and one for
each part of the rest of looking for underlines that can grow faster than the page (CP-05; both under
"Underlines and strike-throughs"). `objects::STEPS` is 2²³ (8,388,608): 7.5 times the corpus maximum,
1,124,715 steps (`pdfbox/5134.pdf`, page 1, 474,984 of them looking for underlines; every page of the 1,080
files that open, measured 2026-11-06 with the budget lifted; 649,731 before CP-05, 12.9 times, and 588,688
before the grids' cells were counted, 14 times). A step takes about 0.47 µs in an optimised build, whatever the
forms' depth or width, so a page is refused after about 4 s of reading; the heaviest corpus page reads in
0.43 s. A step looking for underlines takes far less, about 10 ns: made-up pages that run past the limit there
are refused in 29 to 106 ms (performance.md's "Reading underlines and strike-throughs"). The margin is below
S1-01's 32 times by the user's choice (2026-11-05, at 12.9 times; CP-05 brought it to 7.5 times by counting
more of the same reading, the limit unchanged): 32 times would be about 16 s, and the editor reads a page more
than once. An edit reads its own new content back before writing it (`parse_as_page`) within the same limit, so no
edit makes a page that can't then be read. A form drawn again inside itself is passed over where it repeats
(each form read once on the way down, as pdf.js does; its `issue19800-eq.pdf`), and the nesting limit, 8, stays:
`crate::MAX_FORM_NESTING`, which redacting inside drawings and reducing pictures follow too (WP-03).
A form whose content can't be decoded is passed over too; only a `TooComplex` refusal goes up to the page. Then
editing refuses (`page_objects` fails), redaction reads the text under every mark before changing any page and
refuses, search names each page it can't read, with why (`Matches::unread`, shown under Search results), and
`page_text` and `document_words` give nothing for the page. A test's limit (`budget_seam`) reaches the search
index's worker threads through `budget::Carry`.

A string is redacted character by character only when
`FontCtx::drawn_alike` says every reader splits it the same way; else it goes whole. Besides hayro's split (the
first mapped length) against the codespace's, it fails a code that also starts a longer one (`fonts::starts_longer`,
PB-03): a longer code the CMap maps (poppler reads that) or a longer codespace range (pdf.js reads by those). Not
caught: a longer mapping that starts with the same bytes but doesn't match the string's next ones (poppler then
reads a longer unmapped code), as hayro-cmap can't list mappings; a CMap like that breaks its own codespace.

Redaction marks move and resize as other comments do (PF-37, batch 42): `editor::movable` lets a mark through
although it has areas (`Annot::quads`; other text markup stays on its text), and `resizable` gives handles only to a
mark of one area, as a mark's areas over several lines follow their text. A press anywhere in a selected mark's box
grabs it (`in_mark_box`, in `grabs` for the read tool and `start_arrange` for the arrow), between its lines too,
as its outline shows. The move is `Action::MoveAnnot` through core's `move_annotation`, which maps `QuadPoints` with
the box; for a mark of one area that is resized, `inset_quad` puts the area back `PAD` (1 point) inside the new box,
corner by corner, so stretching doesn't widen the gap (the outline, drawn from the areas, is what Apply removes:
`redact.rs` reads the same quads). Arrow keys go through `Editor::nudge` as for other comments. Apply redactions…
and Finish signing… are enabled by `Ctx::marked` and `Ctx::signing`, from `Editor::redact_marks` and `sign_items`,
which `Tab::refresh_marks_banner` counts once per revision; `why_not` gives their reasons (`signing::NOTHING_PLACED`
is shared with the pane's greyed button).

The Comments view (ENH-40, batch 42) is `tab/comments.rs`. `Tab::read_comments` reads `Pdf::annotations` once per
revision (letting go of a selected comment no longer there); `Tab::shown_order` is `thread_order` filtered by
thread (the comment decides type, author and page; the search looks in the whole thread and in the words a
markup without a comment marks, read with `page_words` per page and kept in `Review::marked` for the revision),
cached in `Review::order` against `Shown` (revision, filter, the page read for This page, the lowercased query).
`Tab::review` (`Review`) holds the filter and search per tab, never written. `Tab::cards` lays the rows out as
search results do: heights from fixed parts (padding, the author line, the kind and date line, a Draft reply line,
the selected card's Reply button) and the comment's first lines, counted once per width with `glyph_width` (one
line when it fits, else `LINES`), so the tops are known without laying out text; `show_viewport` draws only the
rows in view, page headings by `Tab::page_name`'s rule. A selection that differs from `Review::shown` (a click on
the page, Previous/Next, the keys) sets `seek`, which scrolls the list only as far as the card needs, against last
frame's `Review::view`. The list is one Tab stop (`Tab::comments_id`, in `holds_keys`, so Delete and Esc reach
the commands as in the thumbnails); `Tab::list_keys` hands its keys to `Tab::comment_keys`: Up/Down/Home/End
select a card (`take_keyboard` cancels egui's arrow focus move), Enter sets `Editor::reply_focus`, Esc surrenders
the keyboard leaving the selection. A card's mark is shown by `Tab::show_mark`: `reveal` on the page being read,
else `go_to` with `goto_y`. `Act::PrevComment`/`NextComment` call `Tab::step_comment`, which steps by the
selected comment's place in the whole `thread_order` to the next card shown (so a comment the filter hides still
has a place), from nothing selected by the page being read, and returns what to say at the ends (`Tab::info`).
The inspector's heading names the comment's page by label through `Editor::annot_page`, lent by the app.
Character and drawing-part coverage is the share of the box under the marks' union (`share_under`, Q14): each
mark clipped to the box (`part_1d`; a flat box is all in or all out by its middle), the union's area summed in
vertical slabs between the marks' edges. It was the sum of each mark's share, capped at 1, which counted a part
two marks share twice and removed characters only a fifth under two marks laid on each other.

Removing pages (`pages_gone.rs`) takes the alternative text off structure elements with content on a page that
went (`strip_gone_alt`). A marked-content item's page is its own `/Pg`, the element's or the nearest parent's,
else the pages the parent tree lists the element on, by each page's `/StructParents` (kept and removed pages
alike; PB-04); content none of these places counts as on a page that went, so its text goes rather than the
removal being refused. Object references (`OBJR`) aren't marked content and keep their old test. Form field walks
share one depth rule (PB-05): fields at most `forms::MAX_DEPTH` (32) below the top field, their widgets one level
further, as `form_nodes` reads them: `value_nodes` and `field_goes` refuse a node with kids deeper than that, and
`orphan_widgets` a widget with more than 33 parents.

Deleting pages, extracting and splitting are held to redaction's promise (PB-137, D-22 (a)). `drawn_before` walks
the document's drawings first (`Pdf::reach`) and keeps what was drawn and painted, and the subset drawn for certain
by a drawing that could be read (`sure`, with the marked-content properties likewise). `keep_pages` then removes
the pages with their content, thumbnails, private data, comments and fields (with what `annots_held` finds those
held) and the alternative text, and cuts references to them. `clear_gone` walks again: what nothing draws now,
and what was drawn for certain, go to `retained::clear_retained` as removed, which keeps what the pages that stay
still draw and refuses what only a drawing that can't be read may draw. Properties nothing uses now go the same
way, with their alternative text. A resource list inside what went (a removed page's own) is left as it was, so a
signed file saved by adding to it doesn't write the page again. Anything still reached refuses the removal with `retained`'s route; deleting
runs `atomically` and extracting changes only the new document, so nothing changes. `extract_with` takes the walk from before so that a split walks
the document once; `Pdf::split` makes and checks every part before writing any, so a refused part writes no file,
then makes each again to write it (so no more parts are held at once than are being made). Printing some pages
(`sheets.rs`) extracts them unchecked, since nothing is saved.

Hidden layers (`Pdf::without_hidden`, BUG-16): content in a hidden layer still sets the graphics and text state
for what follows it (ISO 32000-1, 8.11.3.2), so only its painting goes. Inside a hidden marked-content section,
state operators (colour, `cm`, `q`/`Q`, line style, `gs`, text state and positioning, `BT`/`ET`) are kept as
they are; a path is collected until it is painted, then dropped, or kept with `n` when it has `W`/`W*` (so it
still clips); a hidden painting operator that ends a path begun before the hidden section (`open`: path
operators kept and not yet painted) becomes `n` too, so the next visible painting doesn't draw that path, and a
clip set before the section still clips (the delta audit's DA04); `sh`, `Do`, inline images and marked-content
points go. Hidden text becomes a text-free `TJ` as
wide (`hidden_show`, the same arithmetic as redaction's: glyph widths from `FontCtx`, `Tc` and `Tw` in
thousandths of the font size; `Tz` cancels out), and `'` and `"` keep their move to the next line and the
spacing `"` sets. `TextState` follows `Tf`, `Tc`, `Tw` and `Tr` through `q`/`Q`; a page starts with spacing 0,
a form or other drawing with nothing known, as it inherits where it is used. Refused (`Unsure`, reported as a
`blocked` entry): hidden text in a clipping mode (`Tr` 4 to 7; an inherited clipping mode in a form isn't looked
for, it is taken as 0); hidden text of unknown width (no font set in the drawing, a CMap LimarePDF can't split
for certain) followed by a visible `Tj`/`TJ` before a line move, `BT` or `ET` resets the position; a path left
open at the section's end. A drawing without resources of its own takes its fonts from where it is used too, so
`HiddenNames` includes the fonts, and `check_shared` compares the result for each set. A pattern choice (`scn`,
`SCN` with a name) or graphics state (`gs`) hidden content sets that nothing paints with while it holds
(restored by `Q`, set again at the same depth, or the drawing's end, with only operators known not to paint
between: `unused_settings`, `paints`) goes too, so the pattern or soft mask it names isn't counted as used.

Appending pages (`Pdf::append`) doesn't merge forms. Before adding a file with form fields, Append PDFs
(`App::append_read`, BUG-19) refuses if `Pdf::undrawable_fields` (the whole-form form of `unprintable_fields`) names
any, else asks (`Dialog::ConfirmAppendForms`, holding the read documents in `AppendForms`), then flattens each
such document with `flatten_form` before `App::add_pages` adds them in one step. `limarepdf-cli merge` does the same
for its second and later inputs with `--flatten-forms`, and refuses them without it; the first input's form is
the result's.

The added files' bookmarks and named destinations come with their pages (ENH-18, `merge.rs`).
`Pdf::insert_pages` starts with `carrying(at)`: where the bookmarks go among the document's top-level ones (after
the last going to a page before `at`, first when none does, after all of them when the pages go at the end), how
much room the outline has left (`outline::MAX_ITEMS`), and the names taken (`named_dests`: the catalog's `Dests`,
then the name tree, `tree_dests`, which `pages_gone`'s `dead_names` reads too). Each file, once renumbered and
before its objects join, goes through `carry`: each name gets one not taken (its own, else `-2`, `-3` and so on, in
UTF-16 for a name written in it; the catalog's `Dests` first, as `named_dest` looks there first); `Fix` walks every
object's direct values and changes each `Dest`, and each `GoTo` action's `D`, that gives a name (now always a
string, as the name tree holds it, renamed where taken) or a page number from 0 (to the file's page itself, or
null when it has none, so it can't land on the document's page of that number); then the bookmarks are linked
again as `outline_nodes` reads them (`relink` on each), so a loop or nesting past the limits is left out rather
than joined. `put_carried`, after `set_page_order`, splices the files' top-level bookmarks into the root's list and
fixes the counts, and writes one sorted name tree of a single node holding the document's tree names and the
carried ones (the catalog's own `Dests` stays as it is), into the catalog's `Names` whether direct or an object.
Undo needs nothing of its own: the history compares the whole document.

Fill-and-sign signatures (`crates/limarepdf-core/src/sign.rs`) are stamp annotations whose artwork LimarePDF draws once:
pen strokes smoothed into curves, typed names as the glyph outlines of a bundled handwriting font (read with the
`skrifa` crate, so no font goes into the PDF), or a picture with its paper made transparent. Finish signing
flattens just those annotations, after looking at every page without changing it (`Pdf::plan_page`), so an item
it can't place (no `/Rect`, or no drawing with a box) is refused with nothing changed, not removed undrawn
(PB-112). Counting and locking them (`Pdf::sign_items`, `lock_sign_items`) walk every
annotation a page lists (`all_annot_refs`, PB-02), not the 10,000 the app shows, as removal does; so does
finding an annotation's page when it has no `/P` (`annot_page`). The app's saved signatures and the Create
signature window are in `crates/limarepdf-gui/src/signing.rs`; every preview button is a white card that the
preview's paper fills (`paper_button`, PB-09), with a 2-point outline always drawn (clear, the border colour
under the pointer from the last frame's response, the accent when chosen), as a white fill hides the flat
style's hover fill. Certify with digital ID's "Others may" is a list, not three choices, so it fits 800 points
high as Sign does (PB-10, `digital::tests::sign_and_certify_fit_800_points_high` compares the space above their
buttons).

The Fill and sign pane (`SignPane::ui`, ENH-09, batch 42) is the task top to bottom: `inspector::facts` (Placed,
from `Editor::sign_items`; Signature, a picture, not digital), then `group`s that never fold (a thin line and the
section's name): Signature and Initials, each with its saved previews and one Create button whether or not any are
saved, Text and marks, the ink, and Finish signing (`theme::primary_button` once something is down). A saved
preview's row is its picture sized to the room its remove button leaves, the picture button's padding counted: a
row wider than the panel widens it every frame. The marks (`MARKS`, `mark_glyph`) are Phosphor glyphs, the tick and
cross from the outline font (the filled ones are cut out of a square), the dot from the filled one; Inter has no ✔
or ✖, which egui drew as boxes. The Keep tool active pin is drawn by `inspector_head` (`Editor::keeps_pin`, a
placing tool with no comment selected, and `keep_button`), first of the heading's buttons, so F6 to the inspector
reaches it first.

A picture's paper is made transparent by `clear_paper` (BUG-21): `paper_shade` finds the paper's shade in blocks of
about a 24th of the picture (each block's 90th percentile, a block much darker than its neighbours taking theirs),
blended between block centres, so shadows and uneven light are paper; `ink_alpha` keeps what is darker than that by
more than six times the paper's grain (the median distance from the usual darkness, at least 24 levels), soft from a
fifth to three fifths of the ink's darkness; `keep_writing` drops pieces (8-connected) of a few pixels and small
pieces farther from the strokes than the strokes are tall, and refuses a picture whose largest piece is under 16
pixels. The earlier "darkest 1%" estimate fell on the paper when the ink was under 1% of the picture (a signature on
a whole sheet). `SignArt::Picture`'s `crop` (shares of the picture as seen, mapped to the stored pixels by
`imgs::stored_point`) is applied first; the picture file is never changed, and the app saves the crop in
`signatures.txt` as a `cropped` line, which older versions skip. `picture_view` gives the Create window the
picture upright, at most 1200 pixels across. The window's picture takes the keyboard on a click or Tab, and
`Create::cropper` moves the part with the arrow keys and sizes it by its right and bottom edges with Shift
(`NUDGE`, 2% of the picture a press; Shift read from each key event); egui keeps the arrows for it
(`set_focus_lock_filter`) from the frame after it took the keyboard. `limarepdf-cli sign --crop L,T,R,B` takes
pixels of the picture as seen, turned into the same shares (`picture_part`, checked against
`image_dimensions`). CMYK JPEGs are read by `rgb_alpha` through `image`, which follows Adobe's marker for
inverted samples (PB-21), so they can be cleared and cropped too.

Dates people see (Fill and sign's Today, the date under a signature, a stamp's name-and-date line, headers' and
footers' `{date}`) come from `overlay::local_today` and `today_iso`, the computer's calendar date through chrono's
`Local`, asked for each time rather than kept, so an app left open overnight writes the new day. Times written for
other programs stay in UTC (`overlay::utc_now`: an annotation's `/M`); a digital signature's `/M` is local time
with its offset. A date read back for people to see (`Annot::modified`, shown on the comment cards, in the
inspector and by `annot list`) goes through `annots::pdf_date`: one that says its time zone (`Z`, `+hh'mm'`,
`-hh'mm'`) is parsed by `sig_verify::parse_pdf_date` and shown in chrono's `Local`, so LimarePDF's own UTC times
read as the computer's clock; one without a zone, whose clock is unknown, is shown as written (batch 42; before,
every date was shown as written). `tests/local_dates.rs` reruns itself with `TZ` set to UTC+14 and UTC−11 (POSIX
TZ strings, no time zone files needed), 25 hours apart, so at any moment at least one is on a different date from
UTC (`comment_dates_are_shown_in_local_time` checks fixed comment dates there); Unix only, as Windows ignores `TZ`.

Annotation drawings LimarePDF makes are built in `annot_ap.rs`. A line's endings (`line_end`) are painted with a
pen of their own: on a dashed line it sets an empty dash pattern (`[] 0 d`) before the ending, as `Pen::paint`
only writes `d` when the pen has a pattern and the shaft's dashes would otherwise carry on into it. The same
drawing is what flattening writes into the page. An annotation in a layer (`/OC`) is drawn into that layer, in
marked content tagged with it (`/OC /LimareField1 BDC … EMC`, the layer or rule named in the page's
`/Properties`), so it shows when the layer does, as before (PB-180). That is `flatten_page`, so flattening the
form or comments, finishing signing and printing all do it; a rule written in place is stored as an object of its
own, since renderers look a tag's layer up by reference.

Links (ENH-17) are `links.rs`, kept apart from comments: `AnnotKind::of` leaves them out, so they are never in
the Comments view or `Pdf::annotations`. `Pdf::page_links` reads every link of a page with an area: its id
(`LinkId::Object`, or `LinkId::Inline` for a link written inside the page's `/Annots` rather than as an object,
which a change makes an object first through `own_direct_annots` and returns the new id), its box, its areas
(each `/QuadPoints` quadrilateral's box, else the box; the app finds a link under the pointer by them), its
target and whether it is underlined (`/BS /S /U`, a width above 0). A target LimarePDF doesn't follow is
`LinkTarget::Unfollowed`: another file (`GoToR`, `GoToE`, named by `/F`, a file specification's `/UF` first),
a program (`Launch`, Windows' `/Win /F` too), a script, an address whose scheme isn't `http`, `https` or
`mailto` (`file:`, `javascript:`, `tel:`; an address without a scheme is followed, as before), another action
(`Print`, `SubmitForm`) or nowhere (a page or name the document hasn't). The rule for new addresses is in one
place: `web_link` and `email_link` turn what was typed into the address written (`https://` put in front of one
without a scheme; the `url` crate writes a name or path in other alphabets in ASCII, IDNA and percent-encoding,
as PDF's URI strings must be), refusing other schemes; every writer checks the address again (`checked`), so
nothing but a web or email address and this document's pages is ever written, whatever the caller passes.
`add_link`, `set_link`, `move_link` and `remove_links` write and change them: a page target is `/Dest [page /XYZ
null top null]`, the page's top with the zoom left alone, as `add_bookmark` writes; an address is a URI action.
Every new link has `/QuadPoints` (one area a line, in Acrobat's corner order as text markup's), `/H /I` and the
Print flag. Its look is set by `set_look`: underlined, `/BS << /W 1 /S /U >>`, `/C` Word's hyperlink blue
(`LINK_BLUE`) and a drawing LimarePDF makes (`annot_ap.rs`'s `Link` arm: Underline markup's line under each
area, so it shows in LimarePDF, which hayro draws any annotation's appearance for, and in readers that draw
appearances or link borders; Poppler draws it, `cross_check_links_with_poppler`); not underlined, `/BS` width 0,
`/Border [0 0 0]` and no drawing, so nothing shows. A link isn't in `annot_ap.rs`'s `DRAWN`, so other apps'
links are never given a drawing (`missing_appearances`); `move_link` draws an underlined one again at its new
size. The page's words are never restyled for a link (ENH-17's Q2).

In the app (ENH-17's stage 2), `Act::Link` (Ctrl+K, Word's key; the palette moved to Ctrl+Shift+P) goes through
`Tab::link_command`: words selected open the Insert link window over `Editor::selected_lines` (one area a line,
a link a page), underlined; a link selected opens Edit link (`Tab::ask_edit_link`); otherwise it turns
`Tool::Link` on or off (`band::is_on` shows Link pressed while it is on). The window is `dialogs::link` over a
`LinkAsk` (its `LinkForm`, the new link's areas or the link being changed, and what an unfollowed one does),
asked for through `Tab::ask_link` and opened by the app as `Dialog::Link(Asked, …)`, answered through
`Tab::write_link` or `remove_link`, each one `edit_pages` step. `LinkForm` holds Link to, the address or page and
Show as a link; its `target` is core's `web_link`/`email_link`, its `refusal` says why once something is typed,
and its `ui` draws the same controls in the window and in the inspector (`Editor::link_inspector`, applying a
change at once, an address when its box is left or at Enter, as `Action::SetLink`). A selected link is
`Selection::Link { page, id }` (a `LinkId`): its right-click menu (`context_items`) is Edit link…, Open link,
Copy link address and Remove link, greyed through `Ctx::link`, `link_opens` and `link_address` (`why_not` says
which is missing); Delete removes it (`deletable`, `Editor::delete_selected`); `Editor::selected_link` reads it
from the page's links cache (`Editor::page_links`, cleared with every change in `invalidate`), and a selected
link gone from its page (taken away by Undo) is let go where the page is drawn. `select_for_menu` selects a link
under a right-click before a comment or word (Word's link menu over its words). The Link tool is
`Editor::link_tool`: every link's areas dashed; a press on the selected link's handle resizes it (`resize`), on a
link selects and moves it (`LinkDrag`), elsewhere draws a box (`Draw`) for `Action::NewLink`; a double-click or
Enter gives `Action::EditLink`; the arrow keys move a selected link (`nudge`, `nudgeable`). `change_link` keeps a
changed link selected by the id core returns (an inline link becomes an object). The read tool's hover and
following find a link by its areas (`on_link`) and show `link_tip`.

**Bookmarks** are core's `outline.rs`. `Pdf::outline_nodes` walks the outline once (loops end the walk, at most
100,000 items and 64 levels) and gives each bookmark's object with its `OutlineItem`, in reading order, each
followed by those inside it; `outline` drops the objects. Positions in that list name bookmarks for
`add_bookmark`, `rename_bookmark`, `remove_bookmark` and `move_bookmark`, and `BookmarkPlace` says where:
`End`, `Before(i)`, `After(i)` (past those inside `i`) or `Under(i)` (last inside it). A move (ENH-16) is worked
out on the items alone by `plan_move` (the list it leaves and the one it joins, by position): `bookmark_moves`
gives the same answer beforehand, false when it would stay put and an error when it can't go (into itself, or so
deep that something inside it would pass the 64th level), so the app asks before making an undo step. A change
writes only the links: the list a bookmark leaves and the list it joins (adding joins one, removing leaves one, a
move both) are linked again from the outline as read (`relink`: the parent's `First` and `Last`, each child's
`Parent`, `Prev` and `Next`; the parent and the place by `place_parent` and `place_index`), which puts right any
wrong `Prev` or `Parent` and any link to an object that isn't there a file had in those lists, then every `Count`
is set again from the tree (`fix_outline_counts`, following only links to objects there are, as `outline_nodes`
does). An outline of 100,000 bookmarks or more, which the walk cuts short, is refused these changes, as linking a
list cut short would lose what is past the cut. Adding into a bookmark, or moving into a closed one, opens it. Its `Title`, `Dest`, `A`, `C` and `F` are never touched, so it goes where
it went. `bookmark_step` turns Word's outline keys into places: Up before the bookmark above at its level, Down
after the one below, Promote after the one it is in, Demote into the one above, last; `None` at the edges, so a
bookmark changes level only when promoted or demoted (ENH-16's Q2).

In the app, the Bookmarks view is `tab/bookmarks.rs`. `Outline` holds the bookmarks as read for a revision
(`Read`: objects and items), which ones the list has opened or closed (`open`, by object, over the file's own
`Count`; kept across edits and never written), the selection (`sel`, by object, so it follows a moved bookmark
and comes back with Undo), a rename and a drag. The list is one stop for Tab (`outline_id`, in `holds_keys` and
`list_keys`), its rows and carets sensing clicks and drags only, drawn for the rows in view at a fixed pitch.
`outline_keys` takes the tree's keys and, first, Alt+Shift with an arrow (`MOVE`, `STEPS`), before the page
commands with the same keys see them. A drag is followed by the pointer, not the row that started it (so it
survives the row scrolling out of view): `Read::drop_place` takes the row under the pointer, its top third
`Before`, middle `Under`, bottom third `After` (or `Before` the first one inside, when those are shown), below the
rows `End`, and keeps a place only where `bookmark_moves` says it moves. `Tab::move_bookmark` makes the move one
`edit_setting` step (bookmarks aren't drawn on the pages, so nothing is redrawn); a refused step or move says why
through `Tab::info`. The right-click menu's Move up, Move down, Promote and Demote are greyed by `bookmark_step`.

**Spelling** (NEW-06) is core's `spelling.rs` over [`spellbook`](https://crates.io/crates/spellbook) 0.4 (pure Rust,
MPL-2.0, used unmodified), which reads Hunspell's dictionaries; Nuspell's suggestions, without the sounds-alike
ones. The bundled dictionaries (`BUILT_IN`: SCOWL's English for Canada, the US, the UK with -ise and Australia,
Grammalecte's French, `assets/dictionaries/README.md`) are packed with zlib by `build.rs` and unpacked by
`BuiltIn::files` when first read; `Dictionary::from_bytes` decodes the files in the character set the `.aff`
names (UTF-8, ISO 8859-1, its default, or 15; others refused with `SpellingError::Encoding`). `words` gives the
words checked (UAX #29 word bounds through `unicode-segmentation`; words joined by a hyphen are one, checked whole
then by part), leaving out, as Word does, words in capitals, words with digits and runs between spaces that read
as web, email or file addresses. A `Speller` keeps each word's answer (`is_known`), so a box typed in looks up
only new words; `Exceptions` are the words added (kept, a lower-case entry covering any capitals, curly
apostrophes made straight) and ignored (the session). `Dictionary::suggest` gives at most five, and for a word
with a capital first also its small-letter form's capitalised: spellbook alone gave "Eh" and no "The" for "Teh".
`find` lists the `.dic` files with an `.aff` of the same name in folders (Linux's `system_folders`; none on
Windows and macOS), naming them from their file names (`tag_of`, `language_name`); `in_extension` takes the pairs
from a LibreOffice `.oxt` (a zip, read through [`zip`](https://crates.io/crates/zip) 9.3 with only the zlib-rs
deflate; each file at most 64 MB). What one choice of dictionaries unpacks is a `DictionaryBudget` (S1-08, M-07:
256 `.aff` and `.dic` files and 128 MiB by default; tests make small ones): `in_extension_within` counts every
`.aff` and `.dic` entry, those without a partner and those of the same name too, and adds their stated sizes from
the zip's list first, refusing (`TooMany`, `TooMuch`, `TooLarge`) before anything is unpacked, then takes each
from the budget as it is read (zip 9 also stops an entry at its stated size); a refusal gives no dictionaries and
leaves the budget as it was. The app's Add a dictionary… reads an extension's own file through `read_extension`
(128 MiB) and a chosen `.dic` and `.aff` through `read_within`, all on one budget. `best_match` matches a `/Lang` or locale to a dictionary by region, then
language. Each dictionary added is written by `settings::write_dictionary` (A-11): a dictionary is listed by its
`.dic`, so the `.dic` of one of the same name goes first, then the new `.aff` and `.dic` are written, and when
either fails, the pair there before is put back (the new `.aff` removed, when there was none). The folder never
holds one dictionary's `.aff` beside another's `.dic`; when the pair before can't be put back, it isn't listed,
and that is said.

In the app, `spell.rs` holds the state in the context's data (as `typing_fonts` holds its loads, so each test's
context has its own): `App::spelling_frame` sets each frame what spelling follows (`Config`: Check spelling as you
type, the language chosen, the dictionaries folder beside the settings, the front document's `/Lang`), saves the
added words to `words.txt` when they change (`take_added`, through `settings::write_own`, a failure said once as
other settings are) and says a dictionary that can't be read (`take_failures`). The language checked is the one
chosen, else the document's, the system's (`sys-locale`) or US English. A dictionary is read on a thread of its
own the first time it is needed, which bumps `revision` and asks for a frame; meanwhile nothing is marked. Each
typing box calls `spell::apply` before it is drawn (a suggestion picked for it goes into its text, as typing
would, so the box's own keeping, undo and `Styled::sync` treat it as typed) and `spell::marks` after, with its
`TextEditOutput`: the box's text is checked only when it, the language or `revision` changed (`Checked`), and a
wavy line is drawn under each word in the galley's rows (`rows_of`, `wave`, the theme's `Tone::Bad` red), except
the word ending at the caret while the box has the keyboard. A right-click on a marked word keeps it with its
suggestions (`Pick`, worked out then, never while typing) for `spell::menu`: the typing menu (`Tab::typing_menu`)
puts it first, and the boxes with no menu of their own (the inspector's comment and reply boxes, form fields) open
one with it alone (`menu_on`). The form field's box is shown as `Ui::put` lays it out but through `TextEdit::show`,
for its output. The Spelling view (`tab/spelling.rs`, `Side::Spelling`) lists what was written (`written`:
comments, replies and text boxes not locked, text fields not read-only or passwords, by page) and its words spelt
wrongly for the document's and spelling's revisions; Change and Change all write through one `Tab::edit`
(`set_annotation`, `set_field`), Change all every word alike in any capitals, each keeping its first letter's.
F7 joins F3 among the keys that work from a text box (`App::shortcuts`).

Digital signatures are checked in `crates/limarepdf-core/src/sig_verify.rs` (finding them, the byte-range rules, and
tying each signature value to its dictionary in the version it signed, by position) and `cms_check.rs` (the CMS
signature, certificates, timestamps), over a small BER/DER reader, `asn1.rs`, that walks the data exactly as written
so signed attributes are never re-encoded before checking. The cryptography is RustCrypto's (`rsa`, `p256`, `p384`,
`sha1`, `sha2`, `x509-cert`). What changed after each signature is worked out in `sig_changes.rs`: it reads the
appended revisions' cross-reference sections itself (lopdf corrects bad offsets and repairs broken tables, which
would hide changes), checks that lopdf reads the same objects (in number order, so a revision with several faults
always gives the same reason: saving compares the reasons before and after, and one naming another object would
count as a new change), then compares each signed version with the next,
judging every object that differs by each role it plays (page content, comment, form field, signature, document
structure) against the signer's permissions. Trust is in `trust.rs`: the trust list file (`trust.txt` in the
settings folder, whose location `limarepdf_core::settings_dir` gives both the app and the command line), and the chain
search from a signer's certificate to a trusted authority over the certificates the signature and the document's
`/DSS` carry, with RFC 5280's checks at the signing time, including certificate policy processing; each link's
signature is checked over the certificate as written (`cms_check::check_cert_signature`). Signatures and
certificates read their algorithm through one function, `cms_check::scheme`: an RSA-PSS algorithm whose parameters
are left out or NULL gives *can't check* (RFC 4055 requires them; only an empty list means the defaults), while an
RSA-PSS-only key with no parameters has no limits (`pss_key_limits`, RFC 4055 section 1.2). Trust is applied after
checking (`SignatureReport::apply_trust`), so a changed list can be applied again without checking again. In the
app, `crates/limarepdf-gui/src/signatures.rs` runs the check on a background thread when a file with signature fields
opens (and after a save), keeping the file's bytes for View signed version, and draws the banner, the Signatures
tab, the Details window and the trust windows; a trust change re-applies the list to every open document. Digital
IDs are in `ids.rs` (the ID folder, making self-signed certificates with `x509-cert`'s builder, finding an ID's
certificate by checking a signature made with its key rather than by labels) over `pkcs12.rs`, LimarePDF's own PKCS#12
reader and writer on `asn1.rs` (which also writes DER): it derives and wipes the password keys itself (`pbkdf2`,
`hmac`, the PKCS#12 derivation), so `pkcs5` isn't needed, and reads BER as Windows writes it. Private files (ID
files, the trust list, the copy of a signature picture kept for next time, O-06) go through
`limarepdf_core::write_private`. A new ID's file goes through `write_private_new` (`atomic::create_new`), which
never replaces: the temporary file is linked to the name, which fails when the name is taken (where the disk has no
hard links, a check and then a rename, which a test reaches through a seam failing the link: PB-168), and `IdStore`
tries the next free name, so two stores choosing one name at once both keep their IDs (PB-134). It and every
document save (`Pdf::save`,
`write_file`, which the app's settings, recent files and signing settings, page images, exported certificates and
text files also use) replace files through `atomic.rs`:
a temporary file beside the target, always created new under a fresh name (never one already there), private to the
user on Unix while written, synced, then renamed over the target, and on Unix the folder is synced after the rename
so a power cut can't undo it. When only that folder sync fails, the error is a `NotConfirmed`, a third outcome apart
from saved and failed: the app and the CLI treat the file as the new one and warn that it may need saving again (the
app keeps the document marked unsaved, and its recovery copy). A file system that can't sync folders at all counts as
synced. Tests reach this outcome through `atomic::sync_seam` (REL-10): folder syncs fail, on every thread, for files
in folders a test registers (`fail_in`, `stop_failing_in`) or in the one `LIMAREPDF_TEST_SYNC_FAILS` names (for the
CLI, run as a separate program). It is built for core's own tests and, through core's `test-seams` feature, only
for the app's and the CLI's: their dev-dependencies turn the feature on, so `cargo build --release` never has it,
and CI's "No test seam" steps check the release programs for the variable's name. The feature has a second seam
for the CLI, `render::panic_seam`: with `LIMAREPDF_TEST_DRAW_PANICS` set, drawing a page panics, so its tests reach
the CLI's panic guard now that no file is known to make hayro panic. Writes of LimarePDF's own files say what happened too (BUG-14): `IdStore::create` and `import` return a
`Stored` (the ID and warnings) once the ID's file is written, so a default that can't be noted, or a sync not
confirmed, is a warning, never a failure that a second try would repeat; `IdStore::remove` returns its warnings;
the app's ID window shows them (`Said::Warning`), the CLI prints `warning:` lines and succeeds, and the trust list,
Make default and exports treat a `NotConfirmed` as done with a warning (`app::unconfirmed`). The app's settings,
recent files and saved signatures go through `settings::write_own`; a failure is held (`Settings::take_unsaved`,
`Recent::take_unsaved`) and `App::say_unsaved` shows it once a session, a signature saved for next time each time
(`Settings::save_sign`, `SignKit::kept_since`); one at exit goes to stderr. Platform note: Windows has no call for syncing a folder, so there the file is synced before the rename and a
power cut just after it can still lose the rename. Commands writing several files (split, page images) never
replace one unasked: the whole list is worked out first (`Pdf::split_files`, `render::image_names`) and checked with
`limarepdf_core::existing` (a broken link counts), then the app asks once (`Dialog::ConfirmReplace`, for either
`Replacing` kind) and the CLI refuses without `--overwrite`. A file that appears at one of the names between that
check and the write is still replaced.

A file is open in one tab at a time, and a save never replaces changes it didn't read (BUG-06, the fourth audit's
A13). A file is known by its `disk::identity` (below, with the destinations): `App::tab_of` finds the tab whose
file a path names and `App::reading` a file being read or asking for its password (`app/destinations.rs`), and
`limarepdf_core::disk::same_file`, for the CLI's checks, is two identities matching. A link and a hard link count,
on Windows too (D-02).
`App::spawn_open` brings an open tab of the file to the front, and skips a file in `App::opening`, the reads in
flight; a result checks again, for a tab that got the file meanwhile. Each tab keeps `Tab::disk`, the
`disk::Version` of its file as last read (`disk::read`, the bytes and their metadata through one handle) or
written (`Tab::wrote`): size, modified time, the Unix inode (saving renames a new file over the old one, so it has
a new inode), the SHA-256 of the bytes and when it was read. `disk::compare` reads the file only when size, time or
inode differ, or when the file's time is less than 2 seconds older than the reading (git's "racy" rule, PB-28: a
change in the same tick of the file's clock keeps the time, and Windows has no inode to tell it; FAT keeps times to
2 seconds), and then calls it changed only when the bytes differ, so a touched or synced copy isn't a change. A
file so read and found unchanged is stored with its new version, by `App::check_disk`'s result and by the save's
own check (`App::stale`, since batch 37: before, a file saved once was read in full at every check of every later
save), so it is read once more after a save or a fresh write, not every time. `App::save` checks
before its other questions, and `App::save_target` again just before writing (Save, a signed document's appended
save, Save as onto the tab's own file by any path); a changed file asks `Dialog::ConfirmStale` (Save as… or Replace;
Replace stores that version in `App::replace_ok`, so the save then goes ahead unless the file changes again), and
shows the `ChangedOnDisk` notice, whose Reload the question has no room for (three answers and Cancel don't fit a
400-point window). Save as, saving selected text, exporting text, signing, splitting, exporting pages as images
and saving pictures refuse a file another tab has open (`App::in_other_tab`, through `App::conflict`; the last
three since S1-03's PB-86, a split part named like an open file having been written over it), and a file being
written (below). When the window gets the keyboard back (`egui`'s `focused`) and when an open file is opened
again, `App::check_disk` compares the files on a thread, and a tab whose file changed shows the notice, once per
new version (`Tab::changed_on_disk`). Reload (`Act::Reload`, asking with unsaved changes) reads the file through the
same path as Open, with the tab's id and revision, and swaps the new tab into the old one's place unless the
document changed meanwhile or something is being typed that isn't in it yet: on the pages (`Editor::pending`) or
in the inspector's properties and label prefix (`Tab::typing_in_forms`; the delta audit's DA02). Typing before
Reload was asked for is kept first (`Act::uses_document`), so the revision covers it, and agreeing to lose the
changes made before still reloads. The save's thread checks once more at the replacement boundary (S1-04, PB-99):
`atomic::replace` takes the version it may replace (`expect`, given by `Pdf::save_expecting` and
`write_file_expecting`; `save_target` returns the one its stale check accepted, for the tab's own file only) and,
with the temporary file written and synced, compares the target just before the rename, always by its bytes
(`disk::compare_bytes`, checksummed as they are read; PB-28 (2): `disk::compare` trusts a settled file's size, time
and inode, which an in-place change of the same size whose time is put back keeps). After that reading it looks at
the size, time and inode once more, and reads a file that changed meanwhile again, three times at most, so the gap
before the rename is no wider than a look at the file. A changed target isn't
replaced (the temporary file removed, the folder not synced): the error is a `ChangedOnDisk`, found in the chain as
`NotConfirmed` is. `App::saved` asks `ConfirmStale` again with the version now there, through `App::show`, so a Save
chosen meanwhile is answered by it; signing over the tab's own file says it in the signing window. Tests hold a
replace just before that comparison with `atomic::commit_seam` (`hold`, `arrived`, `release`; built as `sync_seam`
is, and every replace in the held folder waits, the test's own too). There is still no locking: a change landing
between that comparison and the rename is replaced, and so is one a program that kept the old file open writes
into it after the rename (both accepted, D-15 (d); [contracts](contracts.md#file-identity-and-destinations)).

Files reach the app from the command line (`read_args` in main.rs; on macOS an older system's `-psn_…` process
number is skipped), drops on the window, and on macOS from Finder (BUG-28): Open With, a double-click and a drop on
the Dock icon all send an "open documents" Apple Event (`aevt`/`odoc`), at launch or while running, and never as
arguments. The usual route, an `NSApplicationDelegate` with `application:openURLs:`, is closed: winit 0.30 installs
its own delegate and panics if another replaces it (winit issues 4015 and 4260; winit's own docs describe master,
not 0.30). So `macos.rs` leaves the delegate alone and takes the event from `NSAppleEventManager`: `macos::listen`,
called in `main` before the event loop, observes `NSApplicationWillFinishLaunchingNotification` (from the
notification centre, not the delegate), and then installs its handler for `odoc` over AppKit's, which AppKit
installs as launching finishes and which opens nothing without a delegate or document controller. Its handler
reads the event's direct object (a list, or one item) as file URLs, coercing aliases and bookmarks, and hands the
paths, with the count of items that aren't files, to `incoming::Incoming::system()`, a queue that wakes the window
(`request_repaint`) once `App::new` has given it the context. Events can come before the window exists or after
launch has finished (with the macOS 26 SDK, launch's own event can come after `applicationDidFinishLaunching`, as
Mozilla's bug 2036237 found), so the queue takes paths at any time and `App::open_incoming` empties it each frame
(and when the restore window closes) through `open_startup`, as command-line files open: while Recover unsaved
changes asks, they wait in the queue; items that aren't files are reported as "Cannot open file". The calls
objc2-foundation gates behind `objc2-core-services` (`setEventHandler:…`, `paramDescriptorForKeyword:`,
`descriptorType`) are sent with `msg_send!` with their four-character codes as `u32`, to keep that crate out. Once
eframe moves to winit 0.31, its delegate support may be the simpler route. Linux tests cover the queue and the
app's side; the handler runs only on a Mac, so away from one it is only compiled (clippy for both Apple targets,
CI's macOS builds).

The app shows one dialog at a time (`App::dialog`), and nothing that arrives while one is open replaces it (BUG-07,
the fourth audit's A01). Results from threads (reads, jobs, signing, files dropped on the window) go through
`App::show`, which queues them in `App::waiting`; `App::next_dialog` opens the next once the dialog is answered.
Waiting "Cannot open file" messages are joined into one. A password sent from `Dialog::Password` sets
`App::retrying` until that file's result comes back, so a wrong password is asked again (`App::ask_password`) before
anything waiting. Closing the window with unsaved work puts the open dialog back at the front of the queue and asks
the quit question first (`App::quit_question`). A password prompt says what it unlocks (`Unlock`): a tab's file to open (with its page)
or reload, or the next file of an Append PDFs (`Appending`, BUG-20), whose files `App::read_appending` reads in
order on a thread until one needs a password or fails; `App::append_read` adds them in one step if the tab is still
open. A file with a prompt open or waiting counts as being read (`App::reading`), so opening it again asks nothing
more. Whether an open failed for want of a password is judged from the error's text (`needs_password`). Commands
the user starts set `App::dialog` directly: the modal dialog blocks them while one is open. A thread's result for a
dialog already open fills it in place, matched by a number, or is dropped: Print opens at once and
`App::find_printers` lists the printers on a thread (`Loaded::Printers`, matched by search number to the Print
dialog open or waiting, BUG-11); core's `print::run_within` bounds every CUPS command (`print::ASK_WITHIN` 10 s,
`SEND_WITHIN` 2 minutes for `lp`, the whole process group killed). A Windows job makes synchronous spooler and GDI
calls that can't be stopped, so `Pdf::print` runs it through `print::while_beating` (PB-17): `print_win::print`
beats after each step (the printer's settings, opening it, starting the document, each sheet), and the wait gives
up when no beat comes for `SEND_WITHIN`, leaving the thread to finish on its own, so `App::busy` clears and other
background jobs run. The sheets are made and written out before the thread starts, so the document isn't copied
again. A sheet takes seconds to draw, so a long job isn't given up on.

A printer's paper sizes (ENH-14's stage 1) come from `print::papers` as a `PaperList` of `Paper`s (an id to send,
a label such as "Letter (8.5 × 11 in)", the size in points, the margins when known) and the default. On Linux and
macOS `paper::cups_papers` asks CUPS over IPP with LimarePDF's own client (`ipp.rs`: one Get-Printer-Attributes
request, IPP/1.1, over HTTP to the server `CUPS_SERVER` or `client.conf` names, else CUPS's local socket, else
`localhost:631`; the answer read whole, collections nested at most 8 deep, at most 16 MiB). `media-supported`'s
PWG names give the sizes and labels (`paper::pwg_label`), each taking the size and margins of the nearest
`media-col-database` entry within a millimetre, the bordered one where a size is borderless too (a PPD queue's
sizes are the PPD's own, A4 210.06 × 296.93 mm); custom sizes and the duplex variants CUPS lists as custom are
left out, but for the default. When IPP fails, `lpoptions -l`'s PPD names are read with a table of standard sizes,
margins unknown; when CUPS doesn't know the queue yet (one it found on the network, made on first use by its own
commands), IPP is asked again after `lpoptions` has run. Each question is bounded: the request by
`print::ASK_WITHIN` and its thread by `print::within`, which leaves a call that can't be stopped to finish on its
own (Windows' `printers` uses it too). `ipp::post` sets the socket's waits to the time left before each read; macOS
refuses that (EINVAL, `InvalidInput`) once CUPS has closed its end after answering, so that refusal is passed over,
the wait set before still ending by the deadline (BUG-48: the answer was given up on, and every Mac fell back to
`lpoptions`). On Windows the driver's lists are read in a copy of the program (C-01): `DeviceCapabilitiesW` fills
buffers sized by an earlier count and is given no capacity, so a driver whose list grew in between would write past
them. `print_win::papers` starts the program's own file (`current_exe`) with `print::PAPERS_ARG` and the printer's
name, with no window, and reads its answer through `child::run_kept` (at most `paper_answer::MAX_ANSWER`, 1 MiB,
within `print::ASK_WITHIN`, its thread under `print::within` 2 seconds longer). Both `main`s call
`print::answer_if_asked` first, which in a copy answers and exits, and otherwise allows the asking: a program whose
`main` doesn't call it gets an error, never a reading in its own process. The copy (`print_win::answer`) turns off
the crash dialog (`SetErrorMode`); counts the papers (at most `MAX_PAPERS`, 4,096); reads the numbers, names and
sizes (tenths of a millimetre) into buffers ending at an inaccessible page (`Guarded`: `VirtualAlloc`,
`VirtualProtect`), so a driver writing past them ends the copy at once; counts again (a changed count is an error);
takes the default from the printer's settings; and measures each paper's printable area held portrait from an
information context (`CreateICW`, `GetDeviceCaps`) for at most 3 seconds in all, dropping margins that leave none
of the paper. It writes them in `paper_answer`'s format (a version line, the default, a line a paper, then `end`
and their count), which the program asking checks whole before using any of it (`paper_answer::read`). A copy that
crashed (its exception number given), failed (its message, on one line of at most 300 characters), didn't answer
in time or answered with anything malformed gives an error; nothing falls back to reading in the program's own
process. `papers_seam` (test seams) puts a stand-in printer in front of the driver: `LIMAREPDF_TEST_PAPERS` makes
it write past its buffers or change its list, or makes the copy hang, or answer too much, malformed or cut off
(`crates/limarepdf-cli/tests/papers_win.rs`). `paper::listed` keeps each label once, the default's over others of
its label.

Every printout is laid out by LimarePDF itself (ENH-14's stage 2), on every platform, so what reaches the printer is
what the placements say, and Print's preview draws the same sheets. The design pass found the platforms
printing differently: CUPS enlarged a page smaller than the paper to fill the printable area and turned landscape
pages clockwise, where Windows (the old `print::place`, now gone) printed them at their size and anticlockwise; and
CUPS shrinks a sheet exactly the paper's size into the printable area unless sent `print-scaling=none`. The
placement model is `sheets.rs`'s `print::place(size, &PageSetup)`: a page's size as shown (after its `/Rotate`)
and a `PageSetup` (a `Paper`, an `Orientation` Automatic, Portrait or Landscape, a `Scale` ShrinkOversized, Fit,
ActualSize or Custom from 1 to `MAX_PERCENT`, 400) give a `Placed`: the sheet's size (the paper's as reported, never
swapped), whether the page is turned a quarter turn anticlockwise (Automatic: when its shape is the other way round
from the sheet's, a square page never; Portrait and Landscape: as the sheet is held; `Placed::turns`, quarter
turns anticlockwise, 0 or 1 here), its scale against the
printable area (the paper less its margins, or `ASSUMED_MARGIN`, a quarter of an inch a side as GTK assumes, when
the printer doesn't say: `Paper::margins_or_assumed`), its box on the sheet (centred on the sheet and kept within
the printable area by `min` then `max`, not `f32::clamp`, whose bounds rounding can cross for a page scaled to the
area's width or height, BUG-49; one larger than the area centred on it, losing as much on each side) and `cut`
when part of it falls outside the area.

**The plan of the sheets** (NEW-05's stage 1) is `print::sheets(sizes, &setup)`: the pages printed, by their sizes
as shown, give a `SheetPlan` a sheet (its size, the paper's; `sideways`, held sideways to be read, its left edge at
the top; its pages by their place among those printed, each with its `Placed`; `borders`). `PageSetup::per_sheet`
is a `PerSheet` (`pages`, one of `PAGES_PER_SHEET` 1, 2, 4, 6, 9, 16, another number taken as the next above it;
`order`, a `PageOrder`: `Across` left to right then top to bottom, `AcrossFromRight`, `Down` top to bottom then left
to right, `DownFromRight`; `borders`). One page a sheet is `place` as above, a sheet `sideways` when its page is
turned. With more, the pages go `count()` a sheet in a grid (`PerSheet::grid`: 2 × 1, 2 × 2, 3 × 2, 3 × 3, 4 × 4,
its longer side along the sheet's longer side as held). `places` works on the sheet as held: held sideways, its
size swapped and its margins turned with it (the paper's left margin at the top); the printable area split evenly,
no gap, the places in `order`, rows counted from the top. `fit` puts each page in its place, larger or smaller,
centred, turned a quarter turn anticlockwise (as the sheet is held) when its shape is the other way round from the
place's (Acrobat's "Auto-rotate pages"); `Scale` isn't used. Automatic holds each sheet the way its pages, fitted
upright, cover the most area (by more than a thousandth; otherwise as the paper comes), so two upright pages go
side by side on a sheet held sideways and the order reads as held; Portrait and Landscape hold it as for one page
a sheet. A box on a sheet held sideways goes onto the paper as (x, y) to (paper width − y, x), and its `turns` are
its turn in its place plus one, so a page turned in its place on a sheet held sideways is turned twice
(`Placed::matrix` handles 0 to 3 turns). Nothing is `cut`; `borders` only with more than one page a sheet.

**A booklet** (NEW-05's stage 2) is `PageSetup::booklet`, an `Option<Booklet>` (`binding`, a `Binding` Left or
Right; `sides`, a `BookletSides` Both, Fronts or Backs). Set, `sheets` gives `booklet`'s plan instead, and
`orientation`, `scale` and `per_sheet` aren't used. The pages are padded with blank places to a multiple of four,
the blanks at the end (`last`, the count so padded); sheet k's front holds pages last − 1 − 2k and 2k (as it is
held, left to right), its back 2k + 1 and last − 2 − 2k, so the sheets folded in half and stacked read straight
through; bound on the right, each side is mirrored. The sides are planned in the order printed, a `SheetPlan` each:
Both a sheet's front then its back, sheet by sheet; Fronts the fronts alone and Backs the backs alone, each in the
sheets' order (Acrobat's "Booklet subset": the fronts printed, the stack put back in, then the backs; Acrobat's
manual says the stack may need turning and reordering, by printer, and there is no option to print in reverse
order yet). A side with no pages (a blank back) is still a sheet. `Booklet::sides_printed(pages)` counts the sides
from the page count alone and `Booklet::side(i)` names the sheet and whether it is the front, so the preview can
count and name them before the sizes come. Each sheet is held with its longer side across: sideways for portrait
paper, as it comes for paper listed wider than it is tall (Ledger, as some printers list it). `held` (shared with
`places`) gives the sheet's size and margins as held; the two halves meet at the fold in the paper's middle, each
as wide as the narrower side's margin allows (`(middle − left).min(w − right − middle)`), so with uneven margins
the fold stays in the middle and the side with the smaller margin has room to spare. Each page is `fit` to its half,
larger or smaller, centred, and never turned in it, as Acrobat does (bindings along the top, Acrobat's Tall
bindings, are left for later, PF-56); `onto_paper` (shared with the grid) puts its box on the paper, its `turns` 1
on a sheet held sideways. No borders, nothing `cut`. `Booklet::sides_used` is how the printer prints the sides:
Both two-sided flipped on the short edge (`Sides::ShortEdge`), Fronts and Backs one-sided (`Sides::One`). Short
edge because the sheets are sent portrait with their pages laid out sideways: turning a sheet held sideways over
like a book's page is a flip about the paper's short edge, so both sides laid out the same way read the right way
up. `PrintOptions::sides_used` gives it for a booklet, whatever `PrintOptions::sides` says, and `sides` otherwise.

`Pdf::print_sheets(pages, &setup)` refuses no pages ("no pages selected"), plans the sheets from `page_sizes` (one
walk) and makes them with `Pdf::print_planned(pages, &plan)`, the plan's pages by their place in `pages`; the
preview makes one sheet at a time with it, from only that sheet's pages. Given no pages (a booklet's blank side made
alone), `print_planned` starts from `Pdf::new()` in place of `extract`, so the side is still a sheet of the paper.
It works on a copy: `extract`, unencrypted;
`flatten_form_for_print` (Print-flagged widgets drawn, the others and the form removed); the annotations without a
drawing given LimarePDF's (`missing_appearances`, `draw_annotations`); then for each page `flatten_page` with
`print_annot` (an annotation with the Print flag drawn into the content, pop-ups and the rest dropped) and
`as_drawing`. That makes the page's content and resources a form XObject whose `BBox` is the
visible box (`page_box`), with the page's `/Group` copied: a page with one content stream keeps it as stored, with
its filter; several are joined (`get_page_content_with_limit`, within `MAX_CONTENT`) and compressed at flate2's
fast level. `make_sheet` then makes each sheet a new page drawing its pages with `q M cm /LimarePage Do Q`
(`LimarePage1`, `LimarePage2` and so on for the others), M being the inverse of `visual_matrix` (the page's user
space to the page as shown) followed by `Placed::matrix` (the turns, the scale, the move to the box), so text stays
text, then the borders (half-point black lines round each page's box, `BORDER`). The sheet's own `MediaBox`,
`CropBox`, `Rotate` 0 and `Resources` are set so nothing comes from the page tree, with the `/Group` of its first
page that has one. The catalog's `PageLabels`, `OpenAction`, `Outlines` and `StructTreeRoot` go (they name the
pages, which the sheets replace), the sheets become the page tree (`set_page_order`), and the pages' own objects
are removed, so what they alone used is left out of what is written.
`Pdf::print` takes the paper from `PrintOptions::paper`, else `print::printer_paper` (the printer's default from
`print::papers`, else the first listed; the default printer from `printers`), and when that fails sends nothing
and says why. `print::paper_named` finds the command line's `--paper` by id, label, or label without its size
(`paper::named`, refusing a name several sizes share and naming them), and when the printer's list can't be read,
on CUPS only, a standard size by its PPD name or its name for people (`paper::standard`). CUPS gets
`Pdf::print_data`'s sheets (with `as_image` their pictures at `IMAGE_DPI`, through `Pdf::from_images`), `lp_args`
naming `media=` the paper's id and `print-scaling=none`, and `sides=` from `PrintOptions::sides_used`. Windows
(`print_win::print`) gets the sheets written out and their count, sets the paper's number, portrait and the sides
(`dmDuplex`, from `PrintOptions::sides_used`) in the printer's settings (`settings`, a DEVMODE), draws
each sheet at `print::sheet_scale` (the printer's resolution, at most 600 dpi and `render::MAX_PIXELS`), and
`print::printable_part` finds the part of the picture on the printable area, the sheet's top-left on the paper's,
pixels straddling its edges kept whole, and where it goes there; `print::to_bgra` turns that part into BGRA for
`StretchDIBits`. The command line warns of the pages `place` says are `cut` at the scale chosen. Its `--booklet`,
`--binding` and `--booklet-sides` set `PageSetup::booklet`; `run` refuses, before anything is sent,
`--pages-per-sheet`, `--page-order`, `--borders`, `--scale`, `--orientation` or `--sides` with `--booklet`, and
`--binding` or `--booklet-sides` without it. `--sides` and `--orientation` are `Option`s, their defaults (one side,
auto) applied in `run`, so that one given explicitly can be refused.

Print's page setup and preview (ENH-14's stage 3). `dialogs.rs`'s `Print` holds `papers: Papers` for the printer
chosen (`Unasked`, `Asking(search, printer)`, `Found(printer, PaperList)`, `Failed(printer, why)`), the paper
(an index into the list), `orientation`, `scale` and `percent` (Custom's, kept while another scale is chosen; the
list's Custom is `Scale::Custom(percent)`, set back each frame from the percentage box). `Print::papers_wanted`
names the printer chosen once the printers are found whose papers aren't asked for, found or failed;
`want_papers` (a free function, as are `ask_papers`, so they run while the dialog is borrowed) gives the papers
found earlier in the session (`App::papers_known`, by printer) at once, else asks `print::papers` on a thread, as
soon as `Loaded::Printers` fills the dialog in, and from `App::dialogs`, before the dialogs are drawn, when the
printer changes or Try again sets `Unasked`; `Loaded::Papers(search, printer, found)` keeps what was found in
`papers_known` and fills in the Print dialog open or waiting whose search number it is, and
`Print::papers_found` (`Print::use_papers`) chooses the paper last printed on with that printer
(`Settings::print_papers`, by id) when it is listed, else the printer's default, else the first.

The printers (ENH-47): `App::find_printers` runs `print::printers_in_steps` on a thread, which on Linux and macOS
first asks CUPS over IPP for its queues and default (`ipp::cups_printers`: CUPS-Get-Printers and CUPS-Get-Default,
posted to `/`, each printer's `printer-name` in a group of its own; 4.7 ms here) and gives them unless there are
none, then `print::printers` (`lpstat -e`, which waits about a second for printers the network announces), each
as `Loaded::Printers(search, found, last)`. The dialog is matched by `Search::Running(search)` or
`Print::listing == Some(search)`, the search whose fuller list is still to come. `Print::found` keeps the printer
chosen when it is still listed, else the one last printed to (`Settings::print_printer`, saved on Print), else the
default; it leaves a failure unsaid while this search has listed printers, not while those shown came from earlier
in the session (`Print::earlier`), when the service no longer answering is said with Try again. Each list found is
kept in `App::printers_known` with its search number (not replaced by an earlier search's, coming late); Print
opens with it (`Print::found_earlier`) and the papers known, ready, while its search looks again. `Print::setup`
gives the `PageSetup` only for papers found for the printer chosen: until then Print is greyed with its tooltip
("Finding the printer's paper sizes…"; `footer` shows a `Btn`'s tip with `on_disabled_hover_text` too, which egui's
`on_hover_text` doesn't on a disabled widget), Enter in the page range doesn't print, and a failure says why with
Try again in Print's place. On Print, `PrintOptions` carries `paper`, `orientation` and `scale` from the setup,
and the paper (for that printer), orientation and scale are kept in `Settings` and saved (`print_paper = <id>
<printer>`, the id having no spaces and the printer's name the rest of the line; `print_orientation =
automatic|portrait|landscape`; `print_scale = shrink|fit|actual|<percent>`, clamped to 1 to `MAX_PERCENT`), as
Acrobat keeps the last scaling. Print opens with the orientation and scale kept. `Act::PrintPreview` (File,
`uses_document`) opens the same dialog; its keys are `actions::PREVIEW_KEYS`, Word's Ctrl+F2 (⌘F2), with
LibreOffice's Ctrl+Shift+O first on Linux, as KDE takes Ctrl+F2 for its second desktop. With room for both
(`PRINT_SETTINGS` 470 and `PREVIEW_WIDTH` 330 within the window less 32 points), the settings and the preview are
columns under the title, the settings' content measured from the window's top (`content_below`, so a short
window still scrolls them with the buttons in view) and the preview as tall as the settings; the row's item
spacing is set to nothing and put back inside each column, as the columns' own controls need it. Otherwise the
preview is under the settings in the scrolling content, `PREVIEW_UNDER` (300) tall.

Print as a booklet (NEW-05's stage 2) is at the top of Layout (`print_layout`): `Print::booklet`, with `binding` and
`booklet_sides`, both kept while it is unticked. Ticked, Pages per sheet is greyed and Binding (Left, Right) takes
Page order's place as `columns`' second column (beside it when wide, under it when narrow); "Sides to print" (Both
sides, Front sides only, Back sides only) is under them, named so rather than "Print", which would clash with the
Print button's name. Borders around pages, Orientation and Scale are greyed with `BOOKLET_SETS` as their tooltip,
and Print on is greyed showing `Print::sides_used` (the booklet's sides, tooltip `BOOKLET_SIDES`), its own choice
kept for when the booklet is unticked. `Print::booklet_chosen` gives `Print::setup`'s `booklet`, and on Print
`PrintOptions::booklet` carries it. The booklet and binding are kept in `Settings::print_booklet` and
`print_binding` and saved (`print_booklet = true|false`, `print_binding = left|right`; anything else read as no
booklet, bound on the left); the sides to print aren't, so a later printout never prints the backs alone unasked.

**Folded sections** (2026-11-02, after booklets made Print too long for a small screen; the user's answers: Paper
size stays shown, each section starts closed every time, Scale and Layout their own sections with no level above
them, Print as image back under Advanced, each title saying what is chosen). `print_body` shows the printer, Pages
(or, printing selected text, its layout radios in a group of their own, "Selected text", in Pages' place), Copies
and sides, and Page setup (`page_setup`: Paper size with Orientation beside it, as wide as its longest choice,
through `columns`; one under the other where Paper size would be narrower than `PAPER_BESIDE`, 180 points, which a
400-point window still allows). Then `fold`, an egui `CollapsingHeader` titled in the groups' look, for each of
**Scale** (`scale_field`: the list, named "Scale" for screen readers, and the Custom percentage), **Layout**
(`print_layout`) and, on Linux and macOS, **Advanced** (Print as image, its tooltip `AS_IMAGE`), as Chrome's and
Windows 11's print dialogs fold what is less used. Each header's text, which is its accessible name, says what is
chosen (`scale_title`: "Scale: Shrink oversized pages", "Scale: Custom, 65%", "Scale: Each page fitted to its
place" with several a sheet or a booklet; `layout_title`: "Layout: 1 page per sheet", "Layout: 4 pages per sheet",
"Layout: Booklet, bound on the left, back sides only"), so it never shares the list's name; its id is fixed
(`id_salt`) as the text changes. `Print::folds` (`Folds`: scale, layout, advanced) holds which are open, all closed
each time Print opens; a header is drawn `open(Some(..))` and a click flips its own. `Print::needs_open` gives
those set to something other than the usual (Scale with a scale other than Shrink oversized pages; Layout with
more than one page a sheet or a booklet; Advanced with Print as image): each is opened every frame while it is,
a click on its header doing nothing and its tooltip (`KEPT_OPEN`) saying why, so a setting changing the printout,
remembered from last time, is never hidden; set back to the usual, it stays open until clicked. The closed dialog
fits a 1366 × 700-point window, the last section above the buttons (`sections_fold_what_is_less_used`).

The preview is `print_preview.rs`. Opening Print spawns a `Preview` with a `Source`: a copy of the document
(`Source::Pages(Box<Pdf>)`, cloned on the window's thread as a print job's copy is), or for selected text a
`Make` closure holding the copy and both layouts' `Excerpt`s, which makes the selection's document on the
preview's thread the first time a layout is wanted. Each frame `Print::preview_ask` gives an `Ask`: what is
printed (`What::Pages` from the page range, the last pages kept while the range can't be read; `What::Selection`
in its layout) and the `PageSetup`. A different `Ask` starts a new generation: the drawings and the queued jobs are
dropped, and the thread skips jobs of earlier generations; a selection whose document isn't made yet sets
`Queue::make`, so it is made and its sheets counted before any is asked for. The thread sends each document's
page sizes first (`Came::Sizes`); with them `Preview::replan` plans the sheets (`print::sheets`), which give the
sheet count (before the sizes come, the pages over `PerSheet::count`, rounded up, or a booklet's sides,
`Booklet::sides_printed`), whether each sheet is held
sideways (shown so, the paper's sides swapped) and the notes: the margins unknown, and the pages the plan says are
`cut`, named with `format_page_range` (sheets for a selection). A sheet's `Job` carries its own pages and its
plan, renumbered to them, and is drawn by `print_planned`, `to_bytes`, `render::load` and `Renderer::render` at a
step of a twentieth of a pixel a point (rounded up from what the space shown needs), the picture turned a quarter
turn clockwise when the sheet is held sideways, so it reads upright. Jobs are taken newest first: the neighbours are
asked for, then the sheet shown, so it is drawn first; sheets more than two from the one shown, and drawings at
another step once the step shown has come, are dropped (a drawing at another step stands in meanwhile). Drawing
failures show on the sheet. ‹ and › ("Previous sheet", "Next sheet") page through; one greyed as it is clicked
hands the keyboard to the other. For a booklet each side is a sheet (a blank side's `Job` has no pages), the arrows
are "Previous side" and "Next side", and `counter` names the side, "Front of sheet 2" or "Back of sheet 2"
(`Booklet::side`), above "3 of 4": added after the row, as the column is laid out from the bottom. Dropping the
`Preview` (the dialog closed) stops its thread after the sheet it is drawing.

Closing and quitting (ENH-01) ask `Dialog::ConfirmClose(asked, quitting)`, Save (Save as… for a tab with no file) /
Discard / Cancel; quitting with several unsaved documents asks `Dialog::ConfirmQuit` first, Review… / Discard all /
Cancel. Save runs the ordinary `App::save`, so every save question still comes, and `App::closing` holds what
follows: `Closing::Tab` (close the tab once saved, then bring back the one in front) or `Closing::Quit` (the tab
being saved and the documents still to ask about). `App::close_step` runs at the end of `App::dialogs` whenever no
dialog is open and no save question waits at the front of the queue (one the close request put there goes on first
after Cancel). It closes the saved tab, asks about the next document or quits; a document still `Tab::unsaved`
(the save cancelled, refused, failed, not confirmed on disk, or typing not kept) stops it. Nothing closes before
the app quits, so stopping part way keeps every document open. Answering a quit question drops waiting save
questions and a tab's closing in progress, since quitting asks about every unsaved document anyway. Tests never
open the save chooser: `App::save_as` takes the tests' `SAVE_AS` instead.

**Compact saving** (ENH-19's stage 1; `compact.rs`, `Pdf::write_to` in `doc.rs`): every write of the whole
document (`save`, `to_bytes`) prunes and compresses as before, then `compact::merge_identical` keeps each set of
identical streams once (same data, same dictionary but `/Length`; rounds until none, since pictures become
identical once their masks are merged; never object or cross-reference streams), every reference pointed at the
first. The document is written with lopdf's `save_with_options` (object streams of up to 100 objects at Flate
level 6, a cross-reference stream, the version raised to 1.5), and then `compact::put_back` returns the copies
and every object whose references changed, and `put_header` the version, `max_id`, trailer and cross-reference
kind lopdf's writer changed: the document in memory keeps each object where it was, so edits after a save (the
CLI's later steps, the tab's own document when a copy wasn't handed over) find them, and Undo's `Before` compares
like with like. Left as before: an encrypted document is written one object at a time (lopdf encrypts objects
before its writer would pack them, so an object stream it made would go out in the clear; the merging runs on the
copy encryption makes), a document whose XMP says `pdfaid:part` 1 with a table (`claims_pdfa1`; PDF/A-1 is PDF
1.4; the XMP decoded up to `MAX_XMP`, 4 MiB, and taken to say so when it is larger: S1-08, M-05, the user's choice;
a save has compressed every stream by then, so this is the case that matters), and one of under 32 objects lopdf could pack (`few_to_pack`) is written both ways in memory and the smaller
kept (three corpus files of 4 to 6 objects grew by 9 to 31 bytes otherwise). `snapshot`, `pruned_snapshot` and the
render copies don't go through it. The saved-output checker (`tests/common/saved.rs`) checks each object packed in
an object stream by the stream's header, and searches object streams decoded as every Flate segment.

**Reducing pictures** (ENH-19's stage 1; `reduce.rs`, `Pdf::reduce_pictures`, `limarepdf-cli reduce`): every page
is walked (its content after `inline::extract`, through the form XObjects it draws, 8 deep as a page's text is read,
`crate::MAX_FORM_NESTING`, 12 before WP-03, with `q`/`Q`/`cm` and each form's `/Matrix`), recording for each
picture where it is drawn (`Use`: page, transform, and the object holding the `/XObject` dictionary its name was
found in: a resources object, the `/XObject` dictionary's own, or the page or form holding it directly). A form
deeper than that isn't walked: every picture its resources list, and those of the forms they list at any depth,
goes in `Walk::deep` (`unfollowed`, PB-175), as where and how large it is drawn there isn't known. A picture with a
use on a chosen page is reduced only when every use is on a chosen page, it isn't in `Walk::deep`, and every object
referring to it (`referrers`, one pass over the objects) is one of those holders; otherwise (a page not chosen, a
comment's or field's appearance, a pattern, a drawing too deep) it is counted `Shared` and left, since
a copy beside the original the rest still needs makes the file larger (pdfbox's `2990` grew by half when copies
were tried). Its resolution is the lowest over its uses of `min(width / drawn width, height / drawn height)` in
pixels per inch (`shown_ppi`, the lengths of the transform's axes, so turns and skews count); above the choice's
by more than 5% it is decoded through `redact_img::decode`, scaled with Catmull-Rom (`image`'s `resize`) and saved
through `redact_img::encode` (which now writes the new `/Width` and `/Height`): a JPEG as JPEG at the lower of its
own quality (`imgs::jpeg_quality`) and the choice's, a picture of at most 256 colours losslessly (Flate), others
as JPEG at the choice's (HD and High fidelity 90, Print 85, Web 80, Email 75). A soft mask is scaled with it (to
the picture's new size when it had the picture's, as `/Matte` needs) into a new object, Flate; the pictures are
gone through in object order (`wanted` is a `BTreeSet`, PB-176), so the masks take the same numbers each run and
the same file comes out. The new picture
replaces the old in place (`objects.insert`, so Undo's comparison keeps the old), only when it and its mask are
smaller than before. Left and counted by `KeptWhy`: other spaces than grey and RGB (`plain_components`:
`imgs::device_equivalent`'s `Plain` or `Calibrated`, so Device, Cal, ICCBased of 1 or 3, saved again in that space;
never Indexed, Separation, DeviceN, Lab; a name followed through the page's colour spaces by `imgs::followed`, at
most 8 times and never back to one already met, S1-08, M-01), not 8 bits, `/Decode`, `/Mask`, `/ImageMask`,
filters other than DCT, Flate, LZW, run length and the ASCII ones, a JPEG whose components don't match its space,
a mask that isn't 8-bit grey, inline pictures. `Reduced::most` names the picture that lost the most bytes, for the
app's preview; of several that lost as much, the first in object order.

**Reduce file size in the app** (ENH-19's stage 2): `Act::ReduceSize` (File group, so no band place; greyed by
`reducible` while busy, saving or for a signed tab, `Ctx::signed` from `Tab::signed`, whose `why_not` fix gives
`actions::SIGNED_KEEPS`) opens `Dialog::ReduceSize(Asked, Box<dlg::Reduce>)`. `dlg::Reduce` holds the quality
(`Settings::reduce_quality`, saved on Reduce), all or the selected pages, and a `reduce_size::Estimate`: a thread of
its own on a copy of the document (as Print's preview), which first sends `Pdf::saved_size` and then, for each
choice `Estimate::ask`s (numbered; only the newest is worked out, older ones drained, older answers ignored), core's
`Pdf::reduce_preview` (reduces a copy, writes it for the size, cuts `PREVIEW_SIDE`-square parts of `most`'s picture
before and after with `picture_part`); `collect` turns the parts into textures. Dropping the dialog drops the
sender, so the thread ends after what it is doing. Reduce goes through `App::spawn_job` (`REDUCE_JOB`):
`reduce_pictures` and `saved_size` on the tab's copy, applied as one undo step when a picture changed, and
`reduce_size::reduced_text` as the job's message; nothing changed, nothing is applied.

**A new blank document** (NEW-09's stage 1): `Act::NewBlank` (Ctrl+N; Insert > Blank document, ☰ first, the start
screen's `empty_state`) runs `App::new_blank`: `Pdf::new` with one page of core's `print::new_document_size` (the
`paper.rs` part: Letter for CLDR's `LETTER_REGIONS`, else A4, the region read from the locale's first two-letter
subtag before any extension), added by `add_tab` with no path and `dirty` false, so `Tab::unsaved` is false and
`close_asking` closes it at once until it is changed; its Save goes to the chooser as any tab without a file does.
The locale is app.rs's `paper_locale`: on Linux and the BSDs `LC_ALL`, `LC_PAPER`, then `LANG` (POSIX's order for
the paper; `sys-locale` reads `LC_MESSAGES` instead), else `sys_locale::get_locale` (Windows and macOS name the
region there).

**Saving pictures** (NEW-09's stage 2; core's `pictures.rs`, `limarepdf-cli save-images`): every picture is a
picture object (an inline one through `redact_img::inline_stream`) and where it shows, `shown`, its unit square
to the displayed page (the object's `ctm` times the inverse of `vis_down`, so a page's `/Rotate` counts).
`format_of` writes it byte for byte (`PictureFormat::Jpeg`) only when `upright` (x along the page's x, y up, no
turn beyond a thousandth, stretched or not) and `jpeg_as_is`: `/DCTDecode` alone, no `/SMask`, `/Mask`,
`/Decode`, `/ImageMask` or `/ColorTransform` (each would show differently outside the PDF), a space
`imgs::device_equivalent` reads as `Space::Plain` (`/DeviceGray`, `/DeviceRGB`, or an `/ICCBased` profile of 3 whose
header names sRGB) matching the JPEG's components (S1-17, C-02: a calibrated space or another profile shows the
numbers changed, which a JPEG file doesn't carry), and
`imgs::jpeg_as_shown` (the JPEG's own size, no EXIF turn, which a photo viewer would apply and a PDF reader
ignores). Anything else is `picture_png`: `placed` lays the picture on a page of its own, each side as many
points as its pixels in the direction it shows (a side within 1e-4 of upright or across taken as exactly so),
the page the bounding box; `alone` writes a one-page document of the picture with every object its dictionary
reaches copied under the same numbers (`/OC`, `/Metadata`, `/Alternates`, `/OPI`, `/StructParent` dropped), and
`render.rs`'s `Renderer::render_clear` draws it at 72 dpi on a clear backdrop (`render_whole` with
`TargetInit::Clear(TRANSPARENT)`, then the colours divided by alpha); the three are `pictures::drawn_alone`, which
Optimise scanned pages draws a calibrated scan with too. Drawn 1:1 on whole pixels, the renderer
samples each pixel as stored, so a Flate RGB or grey picture comes out exactly (core's `tests/pictures.rs`, and a
flipped RGB picture of Ghostscript's `GS9_Color_Management.pdf` equal to pdfimages' flipped); its colours are the
renderer's, so CMYK, ICC, Lab, palettes, JPEG 2000, JBIG2 and masks show as on the page. A picture the renderer
reports it couldn't decode (`take_missing`) is an error, not an empty file. `png_of` writes grey, grey and alpha,
RGB or RGBA, whichever holds the pixels. Save all images walks the pages with reduce.rs's `walk_pages`, whose
`Walk::seen` now records each picture drawn, page content and the drawings it uses, in drawing order, an inline
one as a picture object; `found` keeps an object's first use only and numbers each picture by its place among
its page's (`names`: `<stem>-page-<page>-image-<n>`, pages padded to three digits at least as `image_names` pads
them, `n` to the most on a page). `save_all_pictures` draws them in parallel in batches of about 1 GB of bitmaps,
writes each whole (`write_file`), reports one that can't be saved by name and goes on, and returns a
`NotConfirmed` as `write_images` does. The app: `Act::SaveImage` (the Objects group of `context_items` beside
Replace image, so the right-click menu and the floating bar; ☰ > Edit) asks `picture_format` for the chooser's
kind, then `App::save_image` spawns the job (`picture_file`, `write_file`); `Act::SaveAllImages` (☰ > Export)
says when `picture_names` is empty, else `pick_folder` (now given its title) and `Replacing::Pictures` when
`existing` finds files, then `start_save_pictures`. Neither changes the document (`changed` false).

**Pictures and the clipboard** (NEW-09's stage 3): egui-winit sends Ctrl+V as `Event::PasteImage` when the
clipboard holds a picture and no text (arboard's `get_image`), and eframe answers `ViewportCommand::RequestPaste`
the same way, so both reach `App::type_over`. There, before the check that a text box has the keyboard, a picture
with text being typed (`Editor::typing_id`) gets a note; then, in the page view and not busy, `let_go_of_pages`
(the clipboard holds one thing: a picture copied after pages replaces them) and `Tab::paste_picture`: the
`ColorImage`'s pixels unmultiplied, core's `png_file` (as Save image as writes one: grey, RGB, with alpha only
when some pixel needs it), `pasted_rect` (96 pixels an inch, as Word sizes a screenshot, shrunk to fit inside
half-inch margins or a tenth of a small page's side, centred on the page in view, `Tab::current`), `add_image`
through `edit_pages` (one step), then the new object selected loose (`select_new`, the arrow kept, any other tool
the read tool). The band's Paste (`pasteable` now true in the page view, as the clipboard can't be read ahead)
with no pages held sends `RequestPaste` and keeps the frame in `App::paste_asked`; a later frame with no `Paste`
or `PasteImage` says the clipboard holds nothing to paste here. Copy (`Act::Copy`, `copyable`'s `one_picture`)
with one picture selected is `App::copy_picture`: core's `Pdf::picture_pixels` (`picture_bitmap`, the drawing
`picture_png` makes for Save image as) on a thread, `Loaded::PictureCopied` carrying `App::picture_copies`'s
number, then `ctx.copy_image`; Copy and Cut count `picture_copies` on, so a picture still being drawn when
something else is copied is dropped.

**A picture in front of text** (NEW-09's stage 3, the user's find): `Editor::picture_over_text` is the last
picture whose box holds the pointer drawn after every line of page text whose box holds it (the page's objects
are in drawing order), not hidden and under 0.8 of the page (`Editor::page_areas`, kept as each page is drawn),
so a scan with its recognised text under it, or text drawn after the picture (recognised text of a pasted
screenshot), still takes the click. Where it finds one, `hit_where` leaves page text out (so the arrow's
smallest-box rule can't pick a hidden line, and `text_hit` finds none), the read tool's `on_text` is false (no
text cursor, `object_at` selects the picture), and a double-click, a triple-click or a drag started there
selects the picture rather than the words. A drag started on visible text still selects the words across the
picture. `Tab::over_fields` (a visible widget of `Editor::form`'s fields overlapping the box) gives the note
`OVER_FIELDS` after `add_image` from Paste or the Image tool: widgets are annotations, drawn above the page's
content by every reader.

**Decoding parameters given as a reference**: lopdf's `decompressed_content` reads `/DecodeParms` only when it is
written in place (a dictionary, or an array of them); given as a reference (pdf.js's `issue17906`) it decodes as
if there were none, leaving a PNG predictor's byte at the start of every row, so the samples read shifted.
`imgs::direct_parms` makes a copy with them resolved, decoded by `redact_img::decode` (redaction, reducing) and
`scan.rs`'s `decode` (Optimise scanned pages, the CCITT parameters too); the copy is never written back.

**What a picture's colour space means** (S1-17): `imgs::device_equivalent` is the one reading of a picture's
colour space for saving it (`pictures.rs`'s `plain_space`), reducing it (`reduce.rs`'s `plain_components`) and
optimising a scan (`scan.rs`'s `decode`). `imgs::followed` follows its names through the page's `/ColorSpace`
resources, except the names that are spaces of their own: the device spaces, `G`, `RGB` and `CMYK`, `CalGray`,
`CalRGB`, `CalCMYK` and `Pattern`, which hayro's `ColorSpace::new_from_name` reads before any resource (and poppler
too; PB-107 was disproved by that). Then `Space::Plain(n)` is grey or RGB shown as stored: a device name, or an
`/ICCBased` profile of 3 whose header (bytes 52 to 55) names sRGB, which hayro's `ICCProfile::new` takes as
DeviceRGB without reading the curves. `Space::Calibrated(n)` is `CalGray`, `CalRGB` or another profile of 1 or 3,
whose numbers show changed; `Space::Other` the rest. Optimise scanned pages: `scan_target` measures with
reduce.rs's `shown_ppi`, the lower of the two directions (M-14; it was the width alone), and keeps the page's
`/ColorSpace` resources (`Target::spaces`; an image shared by pages is decoded with its first page's). `decode`
refuses CMYK (a JPEG of 4 components, or a space `redact_img::components` counts 4: the printer's profile can't be
followed; PB-144, C-03's CMYK half), and for a `Calibrated` scan returns, beside its own numbers, how it shows
(`Scan::shown`): the stream, its colour space replaced by the followed one so no resources are needed, drawn by
`drawn_alone` at its own pixel size, grey or RGB as its own. `process` chooses the mode from how it shows; kept as
it is (grey as grey, colour as colour) it re-encodes its own numbers and `encode` writes the followed colour space
back, otherwise it starts from how it shows and writes device grey (C-03). A 1-bit scan is black and white in any
calibration and is never drawn.

Saving writes on a thread of its own (ENH-35, batch 37; `app/save.rs`). The window's thread does what comes
before the write as it always did: keeping the typing, the questions (`App::save`, `save_unmarked`, `save_signed`,
with `append_edits` for a signed document), and `App::save_target`'s checks. Then `save_now` hands
`App::start_save` a copy of the document with its password set (`save::Write::Whole`), or `save_appended` the
bytes with the edits added (`Write::Appended`); the thread writes them (`Pdf::save` prunes, compresses and
encrypts the copy, so the tab's own document is no longer compressed by a save), reads the file back
(`disk::read`, the bytes for the signature check and the version for `Tab::wrote`), and sends `Loaded::Saved`.
`App::saves` holds each save being written (`save::Running`: its tab, number, the tab's `State` when handed over,
the password to check the saved signatures with), shown by the status bar's busy sign (`App::busy_sign`).
`App::saved` applies the result as the old synchronous save did, with the same messages through `App::show` (a
dialog may be open by then): Save failed, Saved, not confirmed, the status. The tab counts as saved only when its
`State` is the one handed over and the write was confirmed; changed meanwhile (any edit, Undo, a password) it stays
`dirty`, keeps `scrubbed` and keeps its recovery copy (`saved_copy` not confirmed). The tab's `State` changes with
the result (`dirty`, `path`), so a question asked about the tab while it saved, whose `Asked` held the state before,
is moved on to the state after (`Dialog::asked_mut`), and its answer still acts. While a tab's save is written:
Save or Save as again is remembered (`save_again`, Save as winning) and run once it ends (`save::AfterSave::Save`,
the tab brought to the front for its questions; dropped if the save failed, whose message says so, PB-89);
`close_asking` waits (`AfterSave::Close`, which then asks as usual); `App::quit` and the window's close request
wait (`AfterSave::Quit`, the close cancelled) for every save, every job writing files or printing, and signing in
progress (`App::quit_waits`, S1-04's PB-88; the status bar's `quit_sign`, "Quitting when Split into files
finishes…", unless a save's busy sign shows); `close_step` keeps its `Closing` while the save it waits for is
written, and doesn't quit while anything `quit_waits` counts is; `App::after_save` runs the first one ready at the
end of `App::dialogs`, with no dialog open or waiting. `App::jobs` lists each job `spawn_job` and Replace all start
(`Job`: number, title, document name, whether it writes files or prints, its thread), removed and joined when its
result arrives. With nothing left to wait for, jobs that only change a document are asked about
(`stop_question`, `Dialog::ConfirmStopTasks`: Quit now or Cancel); those Quit now was chosen for (`Job::stop_ok`)
aren't asked about again as quitting goes on, until quitting is asked for again (`ask_stopping_again`). A job whose
tab was closed while it ran still shows its message, ended with the document's name (`for_closed`); one that only
changes a document, and Replace all, say its result was discarded (`discarded`; PB-87). Adding pages from files
(`Loaded::Append`) isn't in `App::jobs`: quitting doesn't wait for it or ask. Reload, Sign with digital ID and Certify are off (`Ctx::saving`) and refused in `act`
with a message (a banner's Reload, placing a signature box); `Loaded::Disk` results for a tab being saved are left
out, as its own save is replacing the file. `on_exit` joins the save threads (`finish_saves`) and the writing
jobs' (`finish_jobs`), so a system shutting the app down doesn't cut a write short. Tests hold the next save's
write until they say go (`App::save_gate`) and wait for it with `saves_done`; `App::job_gate` holds the next job
the same way.

The files being written are reserved until their writers end (S1-03, H-05: two tabs' saves to one file could
both be admitted, the file ending as either tab's, both tabs marked saved). `App::destinations`
(`app/destinations.rs`) holds each claimed file's `disk::Identity` with its `Owner`: a save (`Owner::Save`, its tab
and number), a background job (`Owner::Job`, numbered by `App::jobs_made` and carried in `Loaded::Job`) or the
signing window (`Owner::Signing`). `disk::identity` takes the path canonicalised (for a file not there yet, its
folder canonicalised with the name), the file's own number when it is there (on Unix the device and inode; on
Windows the volume's serial number and the 128-bit file ID, from `GetFileInformationByHandleEx`'s `FileIdInfo` on a
handle opened with no access to the contents, D-02), and the path in lower case; `matches` is the same path or the
same number, so a file named before it exists, one a save renames a new file over, and a hard link are still the
same. `App::conflict` checks a path against the open tabs exactly (`in_other_tab`) and against the claims
ignoring case (`matches_ignoring_case`), on every system: Windows, macOS, FAT, exFAT and shared folders ignore
case, and a name not there yet can't tell which. A refusal names the file and its writer ("is being saved from
another tab", "is being written"), and nothing is written. The writer is admitted and claims on the window's
thread in the same frame, so nothing comes between: `save_target` checks and `start_save` claims, released in
`saved` however the save ended (written, not confirmed, failed, or its tab gone); `spawn_job` takes the files a
job writes (every part of a split from `split_names`, every image from `image_files`, every picture from
`picture_names`, the one picture saved), checks and claims them, released when its `Loaded::Job` arrives; signing
claims its file once `SignWindow::start` is signing, released by `digital_windows` on the first frame the window
isn't (signed, failed or cancelled). The split, image export and save-all-pictures check before asking to replace
files too. Exporting text and saving the selection write on the window's thread, so they only check. Debug builds
assert at exit, after `finish_saves`, that no save's claim is left. Not covered: exporting a digital ID or
certificate, the recovery store's private folder, other apps. Opening (`tab_of`, `reading`) and the CLI
(`same_file`, in `not_an_input` and `not_taken`) know a file by the same identity. The CLI's `not_an_input` takes
every file a command reads besides the document (a picture, a password file, form values, a trust list, a
certificate, the digital ID used) and refuses an output that is one of them (PB-139). The contract, its tests and
limits: [contracts](contracts.md#file-identity-and-destinations).

Listing annotations (`Pdf::annotations`, and `every_annotation` for the redaction marks counted after every change
and before every save) passes each page's id to `annotations_in`; looked up by number (`page_id_at`, which builds
the page list) it went through the page tree once a page, 63 ms on 300 pages (batch 37).

Crash recovery copies are kept by `crates/limarepdf-gui/src/recovery.rs`, in a private
per-user state folder with one subfolder per running app, which holds a locked `lock` file while the app runs. A tab's
copy is a PDF and a description naming it (length, SHA-256, title, original path), published in an order that leaves
a whole pair after an app crash or a power cut: the new PDF is renamed into place and the folder synced, then the
description, synced again, and only then is the previous PDF removed. Withdrawing a tab's copies, before a redaction,
a removal of hidden information or an added password takes effect, shares a per-tab lock with publishing and reports
whether an earlier copy could still be offered, so the command can stop instead. Temporary files come from
`limarepdf_core::disk`, with `atomic.rs`'s rules; free space is read with `rustix` (Unix) and `windows-sys`
(Windows). The file operations go through a small interface, so the tests can make any of them fail or run them on a
model file system that simulates power cuts (`recovery/model.rs`). The app's side is `app/recover.rs`: each frame,
`recovery/schedule.rs` says which tabs are due (a minute after the first change, at most once a minute); the app
takes a ticket and a clone of the document on the UI thread and queues the write, and one recovery thread
serialises it, without the objects nothing uses (`Pdf::pruned_snapshot`, so nothing a redaction or Remove hidden
information detached is in it), and publishes it in order with saves' and closes' removals. Redaction, Remove
hidden information and an added password withdraw the tab's copies before their edit runs, and for a restored tab
the crashed session's copy it came from too (`Crashed::withdraw`), and stop when either reports that an earlier copy
could still be offered. So does Redo when the step it redoes removed content (each undo step records that, as
`Step::removal`, since `scrubbed` may already be set), as a copy written after the undo holds the content. A
crashed copy whose withdrawal couldn't be confirmed (the folder sync failed) is kept in `Crashed::unconfirmed` and
tried again in full at every withdrawal until a sync succeeds; its folder stays meanwhile. Append PDFs does the same when a file it adds is encrypted, and marks the tab protected. At start the app finds the sessions of runs that ended without cleaning up and offers
their copies; a normal exit (`on_exit`, which a crash never reaches) ends the session and deletes its folder.

Signing (`sig_sign.rs`, `sign_document`) adds the signature dictionary with placeholders, its field or widget and
its look, locks and certification to the document, then hands it to the appended-revision writer,
`incremental.rs`: every object that differs from the file as opened (by value: a real `150.0` and an integer `150`
are the same once written) goes into one revision appended with lopdf's `IncrementalDocument`, with a trailer of
its own (`/Root`, `/Info`, `/ID`, `/Prev`), and is read back twice before it is used: `sig_changes::check_appended`
reads the new cross-reference section with the strict reader and must find exactly the objects written, and lopdf's
reading of the result must give every object as it is in memory. Signing then finds the two placeholders in the
signature object (where that section says it is), patches the byte range, makes the CMS with `cms_write.rs` (DER
by hand, so the pre-release `cms` crate isn't needed) and fills it in, and checks the result with
`verify_signatures` before returning it. When the edits would make an earlier signature report a change it
doesn't allow, it returns a `DisallowedChanges` error naming each signer and change, so the app can ask before
signing again with `accept_disallowed`. See [Digital signatures in depth](../signatures-in-depth.md#what-signing-does).

Saving a signed document (`sig_save.rs`, `append_edits`) uses the same writer without a signature: nothing is
appended when no object changed; otherwise every signature is checked before and after, each must check as it did
(a signature gone, as flattening the form makes it, is refused), and `sig_sign::newly_disallowed`, shared with
signing, names the signers whose signature now reports a disallowed change it didn't report before. The app's
`save_signed` (and the CLI's `save_edited`) call it for a document that had signatures when opened, with the bytes
kept for the Digital signatures view as the base, and fall back to a full rewrite only after asking (encrypted, password
set, damaged, a read-back failure, or `Tab::scrubbed`: redactions applied or hidden information removed since the
last save, tracked through undo and redo, since the signed version would keep what was removed). A signed
document's fields are not drawn into it when it opens (`Tab::new`); the snapshots for display and printing draw
them on a copy instead (`snapshot_hiding`, and for printing `Pdf::print_sheets`), so a save adds only real edits.
The command line never draws fields when opening a file, so `render_snapshot` (`limarepdf-cli render`) and
`print_sheets` (through `flatten_form_for_print`) draw every document's fields on their copy; `app_snapshot`
draws them only for signed documents, the app having drawn the others when opening.
`Pdf::unshown_fields` and `unprintable_fields` name the values neither can draw (BUG-23).

In the app, `crates/limarepdf-gui/src/digital.rs` holds the Digital IDs window and the signing window; making, importing
and exporting IDs, and unlocking and signing, run on a background thread each, with the password moved into that
thread in a wrapper that overwrites it when dropped. The editor's `Tool::DigitalSign` takes the place (an empty
field or a dragged box); the app then gathers what to sign: a document opened with signature fields kept its bytes
(`SigState`) and signs those plus its edits, any other document is written out in full first. The signed file
replaces the tab and is checked like any opened file.

In the desktop app, every command is defined once in `crates/limarepdf-gui/src/actions.rs`; the File menu, the commands
row, the command band, the command palette, the right-click menus, the floating toolbar and the keyboard shortcuts
are all built from that list.

Where each command sits in the File menu (☰) is `crates/limarepdf-gui/src/menu.rs`'s alone (ENH-49): `menu::MENU`,
a static tree of `Entry`s (`Cmd(Act)`, `Sub(name, entries)`, `Sep`, and `Recent` for Open recent), drawn by
`menu::show`: File's own commands, then a `Sub` for each of the band's names in `band::Band::ALL`'s order and Help
(`KINDS`), then Settings…, Close tab and Quit. Registry entries no longer carry a group, submenu, separator or
"in the menu" flag. `menu::LEFT_OUT` lists the commands deliberately not in it, each with the kind the palette files
it under: what keys alone do (`Escape`, `Nudge`, `Extend`, the regions), `JoinHyphen` and `KeepHyphen` (the typing
right-click menu's), `DeleteObject` and `ToggleRecovery` (Settings'). From the tree come `menu::path` (the
submenus' names and the label), `menu::way` (a message's "Protect > Password security…", "File > Properties…", so
the messages that name a menu can't go stale), `menu::category` and `menu::submenu` (the palette's column and
"Theme › Dark", and what `actions::matches` searches), and `menu::line_before`, which `actions::menu_list` uses so
the band's menus and the right-click submenus divide their commands as ☰ does. The tests in `menu.rs` keep it
honest: every registry entry in the tree once or left out, each submenu name once, no line at a submenu's ends,
the submenus the band's names in order, each band command under its band's name (or another band's holding it,
or File's own), and each band's groups in the band's order (`band::Group::acts`, test-only). Every shortcut
shown is written by `actions::key_text`, not `Context::format_shortcut` directly: egui off a Mac writes a
punctuation key by its name ("Ctrl+Comma"), which `key_text` turns into its sign ("Ctrl+,"), the arrows aside.

The Settings window (`dialogs::settings`, `Dialog::Settings(Box<dlg::Prefs>)`, ENH-49) shows `dlg::Prefs`, built
by `App::prefs` from the settings (the page layout and cover from the active tab, as View's). Each frame
`App::apply_prefs` compares the window's choices with the settings as they are, and takes what differs, the name
trimmed: through `toggle_recovery`, `Act::Theme`, `Act::Accent` and `set_paragraphs` where more than the setting
changes, else by setting it directly and passing page labels and field highlights to the open tabs, because many
of `App::act`'s handlers do nothing with no document open. A message a change gives (recovery copies that couldn't
be removed) is shown first and the window put back in `waiting`. The documents folder is `Settings::folder`
(`documents_folder` in `settings.txt`); `App::start_folder` gives the folder a chooser starts in: beside a
recovered copy's original, else beside the document's file, else the documents folder, when it still exists.
`App::pick_folder`, like `pick_dictionaries`, never opens the system's chooser in tests (`tests::FOLDER`).

The floating toolbar (`Tab::floating_toolbar`, ENH-20) shows `actions::toolbar_items`: the right-click menu's
commands that have icons, or, while a line is edited on the page (`Ctx::line_edit`), Bold, Italic | Cut, Copy,
Paste. `actions::context_items` gives the right-click menu by what is selected: words (Cut, Copy, Paste, Select
all text | Edit text, Bold, Italic | Highlight | Mark for redaction | printing or saving the selection); objects
with any line of page text among them (Edit text, Move or resize text, Copy the selected text lines | Cut, Delete
the object); other objects (Replace image… | Delete the object), so a picture's menu offers nothing greyed that
never applies. Paste with words selected while reading (`pasteable`: text being typed, or, when nothing is busy,
words selected or pages cut) sends egui's `ViewportCommand::RequestPaste` (`App::act`'s `(None, Clip::Paste)`
with `selection.words()`), and the paste that arrives is typed over the words by `App::type_over`, as Ctrl+V's
is; `why_not` skips "Select some text first." (`SELECT_TEXT`) for Paste, so a greyed Paste still says where typed
text goes. Text being typed on the page (a line being edited, new text, a note or a text box being written) has a
right-click menu of its own (batch 43, the user's find), `actions::typing_menu` (`TYPING`: Cut, Copy, Paste |
Bold, Italic, those that apply), opened by `Tab::typing_menu` after the pages are drawn, on `Editor::typing_box`:
the box's `Response` as drawn that frame (`page_text::Shown::edit` for new text, the line editor's and the writing
box's own), which the tab takes each frame. The floating bar hides while that menu is open, as it does for the
selection's. Two egui traps came up building it. egui's `TextEdit` moves its caret on a press of any button
inside it, so a right-click let go of what was selected: `page_text::keep_on_right_click`, called after each such
box is shown with what was selected before it was drawn (`TextEdit::load_state`), puts the selection back when the
right-click was inside it (one elsewhere puts the caret there, as word processors do). And
`Context::read_response(id)` (through `get_response`) makes a focused widget give up the keyboard when a press
came elsewhere in that frame, so a box read back by its id lost the keyboard it had just been given back: the menu
is opened on the `Response` kept when the box was drawn, never one read back by id.
`tab::bar_place` places it from its size as last drawn (kept in egui's data under the bar's id), the selection's
box on screen (for a line being edited, joined with where its editor was drawn as typed, `Editor::line_area`) and
the page's text (`Editor::text_boxes`: its lines and words as far as they have been read): above the selection,
below, right, then left, the first inside the view that covers no text; else just above or below it, whichever
covers less text (`overlap`), as Word's mini toolbar sits by the selection (the user, 2026-10-28, after the first
version docked it at the view's top by a column heading with text all round it); at the view's top or bottom edge
only when neither is in view. Bold and Italic there change what is selected in the line editor
(`Editor::switch_style` reads it with `page_text::held`, then sets the editor's `refocus`), and `App::act` doesn't
keep the typing before them as it does for other commands that use the document.
The menus show an entry's short `label` inside its submenu; the palette, which has no submenus, shows the
submenu's name before it in the weak colour ("Theme › Dark"), names the row `actions::spoken_title` for screen
readers ("Theme: Dark") and searches the label, submenu and kind (`actions::matches`; BUG-34, audit A18). A test
keeps every palette entry's name its own.

The workspace (batch 39, ENH-38 and ENH-41; build plan 2.2's approved mockup, adjusted by the user's checks) is
drawn by `App::ui` as panels from the top: the tabs (`tab_button`, the line under them broken under the tab shown),
then the commands panel: one row of the document's commands (`quick`, icons named in words; the File menu,
`menu::show`, laid out as the band, above), the band's names (`band::names`) and, at the right in reading order (`dialogs::reading_order`, its
width from the last frame in `App::commands_right`), Print and `band::compact_toggle`; under it `band::show`.
`crates/limarepdf-gui/src/band.rs` holds the seven bands as static tables of groups of `Item`s: `Button` (a
registered `Act` with the band's own short label and icon, and a `menu` of related `Act`s behind a caret; no `act`
makes a button that only opens its menu), `Font`, `Sign` and `Marks` (drawn by the editor, `signing::menu_ui` and
`markup::menu`) and `Planned` (a label, icon, the build plan's item and what it will do). A button's enabled state,
tooltip and shortcut come from the registry; it shows pressed while its command is on or a tool in its menu is in
use, not for a menu's ticked choices. Planned items are disabled buttons named "… — Planned" (no Tab stop, never
dispatched), hidden with `Settings::show_planned` (`Act::TogglePlanned`). `planned_button` draws one with
`Button::atom_ui`: its tag (thin spaces either side) carries an atom id (`tag_id`), whose laid-out rect
`outline_tag` outlines with a dashed line, as the mockup's chip; in Compact commands there is no tag. Each group's width, open or folded, is kept
in egui's memory from the last frame; `band::fold_level` folds them in `band::fold_order` (groups of planned
features alone first, left out rather than folded; then from the right) with a 12-point margin before one comes back,
a folded group being a menu button with its name holding its controls. `Parts::compact` (`Settings::compact_commands`,
`Act::ToggleCompact`, or a window under `SHORT_HEIGHT`) draws icons alone and the Font group on one row (`controls`
gives the row's height). Tests: `band`'s own (every button a registered command; every command placed in a band, the
File menu, a right-click menu or a key; planned items named for plan items; folding), and the app's band tests.

The Minimal interface (ENH-45, batch 48, the completed log) is the same commands panel drawn another way, not a
second shell: `Settings::minimal` (settings.txt `interface = minimal`, any other value Full), `Act::ToggleMinimal`
(View > Show, Ctrl+F1, ⌘⌥R on a Mac) and `Ctx::minimal`. With it `App::ui` draws the row alone: `quick` adds
`ToggleSide` after ☰ and `Find` and `ToggleInspector` before Print (pressed while their panel shows, from the
command's `checked`), and skips `band::names`, `compact_toggle` and `band::show`. Both interfaces end the tab
strip with `interface_switch` (the user's place, nearest the window's own buttons; a panel app can't add a button
to the system's title bar): the tabs panel is drawn on the start screen too, `tab_strip` gets the row less the
switch's width from the last frame (`App::switch_width`, the frame drawn again when it changes), and the switch,
labelled with where it goes (`SWITCH_TO_MINIMAL`, `SWITCH_TO_FULL`, a plain button for screen readers; View's
choice is the ticked state), is as wide with either word, so the tabs don't move. Its widget sits in a scope of
its own id (`UiBuilder::scope_id`), so the keyboard on it survives the widgets before it changing. The switch's
rect joins `Editor::keep_typing`, so its click doesn't end the typing, as a click in the Font group doesn't. A focused widget the other interface doesn't draw loses the keyboard at egui's
end of frame; `App::refocus` (set by the command when something had the keyboard or typing is open, as the switch's
click takes the keyboard from new text) then lets `App::hand_on_keyboard` give it back to the typing
(`page_text::give_back` on `Editor::typing_id`), else to the switch, within three frames. The status bar leaves out
Show planned features and adds `view_choices` (the page layout's menu and Fit width) in Minimal. Nothing else reads
the interface: the tabs, panels (`navigation_panel`, `inspector_panel`, `panel_widths`), docking and pins are one set
for both. `actions::why_not` says `FULL_ONLY` for Compact commands in Minimal before its document check, so the
start screen says it too. Tests: `app/tests/minimal.rs`, and `app/tests/minimal_tasks.rs` for everyday tasks
done by Minimal's routes (Tools > Image asks the system for a file, which tests never reach: its task arms the
tool with `Tab::arm_image`, as the chooser does).

Stage 3 adds Tools, the inspector's Font group and More. **Tools** is `band::TOOLS`, each band's name with
`Item`s that are the band's own: the buttons both have are `const` items (`EDIT_TEXT`, `SELECT`, `DRAW`, ...)
that the band's statics and `TOOLS` share, so they can't drift apart. `band::tools` draws them through `items`
(which takes a slice of items since then, a group's or one), labelled, in two columns of three bands (one
column was taller than a laptop's window), in a `ScrollArea` that leaves room for **All commands**, and closes
itself (`ui.close()`) when a command was pushed. All commands sets `App::open_file_menu`; the next frame opens
☰'s popup by its id (`App::file_button`, the ☰ button's id kept each frame, `with("popup")`, as
`Popup::default_response_id` makes it). `tools_button` is a `MenuButton` with
`PopupCloseBehavior::CloseOnClickOutside`, its rect added to `keep_typing` beside the switch's, so Tools'
Paragraph switches the line being edited; the other entries end the typing themselves (`Editor::set_tool`).
egui keeps one popup open at a time: a `MenuButton` inside a popup closes the one it is in. `band::menu_button`
uses a `SubMenuButton` when `menu::is_in_menu`, for the act-less buttons' menus and `caret`; that also mended
the carets inside Full's folded groups, which had the same fault. **The inspector's Font group**:
`Editor::text_targeted` mirrors `font_band`'s branches (new text or the Text tool's next, `Chosen` restylable,
`editing_line`, a text object selected); with it in Minimal, `inspector_text_look` draws `band::text_look`
(`font_band` on two rows, then `TEXT_ALIGNS` as `Item::Icons`) under the heading of the panel and of the
pop-out, in the tab's scope, and the docked `toolbar_items` above the panel drop what it has (`in_text_look`;
the strip keeps them). Clicks there keep the typing as any click in the inspector does (`insp_rect`).
`editor::font_field` narrows below its 136 points when the row has less than that and 84 for the size box
and its list: a resizable egui panel takes the width its content needs, so a fixed field widened the
inspector, a few points each frame. **More**: `more_folded` counts how many of `MORE_ORDER` (the panels'
buttons, which their strips repeat, then Print, Find, Redo, Undo, Save, Open) go under `more_button` for the
row's width and Tools' last width (`App::tools_width`), each button 30 points and the dividers 26; ☰ and
Tools always show. More's menu lists the folded commands in `ROW_ORDER` through `menu_item`.

Edit's Font group is `Editor::font_band`, which moved the text look controls out of the inspector: the same
`styles_ui` acts on new text being typed (`PageText`, with `page_text::held`'s selection, `refocus` after a change),
before it the next new text's look (`tfont`, `tsize`, `tcolor`; greyed with nothing to style), the read tool's words
or several arrow-selected lines (`Chosen`) and one selected or edited line (`Insp`, `line_edit`). The band is drawn
before the page, as the inspector is, so a press there reaches the line editor first: `band::show` returns the
places of the groups that change typing (Font, and Clipboard for Paste, Cut and Copy), lent to
`Editor::keep_typing`, and `keeping` treats a press or release there as in the inspector: the line being edited, or
a text box being written, stays open and takes the keyboard back once settled. The line editor counts the floating
bar too (`Editor::bar_rect`): the bar is drawn after the pages, so its place is last frame's. The inspector's place
is what the app draws of it, joined within each pass by `Editor::inspector_drawn`: the panel with its heading and
collapse button, the collapsed strip, and the pop-out with its heading (before batch 40's second stage it was the
scroll area's clip rect, which missed the heading and in the pop-out ran the window's whole width). It and the
bar's count only when drawn this frame or the last (`editor::drawn_at`), so a click where a hidden inspector was
ends a line edit as any click away does. With the Text tool, a click on page text edits that line;
`Editor::adding_text` is false then, so the band, the inspector's heading (`tool_heading`) and the status bar show
the line, not the next new text. Esc in a box of the band or the inspector while typing is open on the page
(`App::esc_to_typing`, set at the start of `App::ui` before `App::shortcuts` reads the keys, which then leaves Esc
alone; `Editor::refocus_typing`; `size_box` ignores Esc) drops what was typed in the box and gives the keyboard
back to the typing instead of ending it, as a word processor does. Esc while new text is dragged by its outline
puts it back: egui stops the drag on Esc before `PageText::ui` sees it, so that checks for Esc before the grip
(`Editor::escape` leaves the text alone). `Editor::settle_look`, run each frame from `App::ui` whether or
not the inspector is shown, observes the selected line's history and keeps finished changes (`Typed::Words`,
`commit_insp`) as undo steps. The inspector now shows facts for text (`inspector::facts`, `look_facts`). Paste, Cut
and Copy in the band (`Act::Paste`, `Act::Cut`, `Act::Copy`) for typing set `App::clip` and
`Editor::refocus_typing`; `App::clipboard`, at the start of each frame, sends egui's `ViewportCommand::RequestPaste`,
`RequestCut` or `RequestCopy` once that box has the keyboard again, so the platform's paste, cut or copy lands in it
(dropped if the typing ends first). `Act::Cut` uses the document, but with typing open `App::act` doesn't keep the
typing first (as for Bold in a line). Without typing, Cut copies the read tool's words (as Copy) and then
`Editor::delete_words`, or copies the arrow's lines (`Ctx::obj_only_text`: every object selected a line of page
text) and deletes them; where nothing could be taken out it says it copied. Ctrl+X arrives as egui's `Event::Cut`,
matched in `App::shortcuts` as a page-only key like Copy. A plain key's command there skips a key that a Shift
command took in the same frame (`shifted`), since egui's `matches_logically` ignores Shift: with words selected,
Shift+Home extends them and doesn't also go to the first page.

Every number box is `spin.rs`'s `Spin` (ENH-48), a spin box as in word processors, added with `ui.add` (or
`ui.add_enabled`): a single-line `TextEdit` with ▲ and ▼ painted in its right margin and sensed with
`Sense::CLICK` (not Tab stops; named "Increase …" and "Decrease …"). Its value changes only when typed (Enter, or
leaving the box; Esc leaves it unchanged) or stepped (the arrows, Up and Down, Page Up and Page Down ten steps,
read with `count_and_consume_key` before the `TextEdit` sees them and kept from moving the keyboard by a focus
lock filter set after it), and the response is then marked changed: callers keep a change on `changed()`, never on
a drag or `lost_focus`. What is typed is kept in egui's data under the box's id while it has the keyboard, all of
it selected as it gains it so typing replaces it. A unit after the number may be typed or not; commas between
thousands are thousands', otherwise a decimal point. A number out of range is replaced by the nearest allowed (a
fraction where whole numbers are wanted, the nearest whole number), and a note under the box, in an `Area` of
tooltip order with its label a polite live region, says so for six seconds, as for something that isn't a
number, which leaves the value. A value already outside the range is shown as it is and never changed by being
shown (one shape's outline width, several shapes' width, Position and size, several objects' X and Y, an
annotation's line width, a text box's border): egui's `DragValue`, which did that only with
`clamp_existing_to_range(false)`, raised a hairline outline (under 0.25 pt) to 0.25, and `insp_change`,
`Editor::pending` and `commit_insp` made an undo step nobody asked for, again every settled frame for a filled
shape, so the revision moved under a quit question; a rule under 1 pt tall was made 1 pt tall with the next X
typed. `Spin::mixed` shows nothing for objects whose values differ. Its width is that of the range's ends shown,
so it doesn't change with the value. `spin.rs`'s `every_number_box_is_a_spin_box` fails when egui's `DragValue`
appears anywhere else in the app's source, so a new box is a spin box too.

A note or text box open for typing on the page (`Editor::writing`, drawn by `Editor::writing_ui`) closes on Esc,
Ctrl+Enter or a click on the page outside it (a click on the page's own layer, outside the note's comment box). A
note, new or existing, and a text box already on the page stay open through clicks in the inspector (ENH-13):
`keeping` with the inspector's place sets `refocus`, and once settled `page_text::hand_back` gives the keyboard
back with its undo kept. Its text is typed there alone: while it is open the annotation inspector hides its own
Comment or Text box, its other changes still apply, and the inspector's Delete drops what was typed with it. A new
text box ends when it loses the keyboard, unless F6 parked it or a click in the band's typing groups kept it.

The accent (`crates/limarepdf-gui/src/accent.rs`) is read from the desktop: on Linux the XDG desktop portal's
`org.freedesktop.appearance` `accent-color` (`ReadOne`, red, green and blue from 0 to 1, out of range meaning none)
through zbus's blocking API with a 2-second method timeout, on a thread (`App::read_accent`; `App::new` waits for it
300 ms so the first frame has it, else it comes in as the window is drawn); on macOS `NSColor.controlAccentColor` in
sRGB; on Windows the DWM `AccentColor` value (ABGR) from the current user's registry. It is read at start, when
the window gets the focus back and when View > Accent colour changes (`Settings::accent`). `theme::set_accent`
keeps it in a process-wide `RwLock` and installs both themes' styles again; `Palette::with_accent` puts it in for the
teal, darkened (light theme, against white) or lightened (dark theme, against its surface) in 8% steps until its
contrast is at least 4.5:1, its faint and pressed backgrounds mixed with the surface, and `text_shading` a light tint
of it. Without one, the approved mockup's teal palette stays.

The side panel's views are the icon strip at the window's left edge (`SIDE_VIEWS`, each button's selected state a
toggle for screen readers), the panel's heading a plain label (`side_name`); Find (`Act::Find`) shows Search
results and sets `Tab::focus_query`, which `Tab::show_results` gives the keyboard; Replace (`Act::Replace`) shows
them with Replace's field, the keyboard there when the query isn't empty. `side_panel(Side)` gives each
view a panel id of its own, so egui keeps each view's width apart, and its starting width: `SIDE_WIDTH` (132
points, the workspace's) for Pages, `SIDE_TEXT_WIDTH` (220) for Search results, Bookmarks, Comments and Digital
signatures, whose lines wrapped a word or two at a time at 132. A width the user gives a panel is remembered
between runs (ENH-08, batch 39's third stage): `Settings::panel_widths`, by panel id ("inspector", "side",
"side-comments" and so on; `panel_width = <id> <points>` lines, kept from `panels::NARROWEST` to half the widest
window), is each panel's `default_size`, which egui uses until it has a width of its own. `App::keep_width`
watches each panel's edge with `panels::edge_held` (dragged, or with the keyboard on it) and, the frame it is let
go, keeps the panel's width (`Settings::keep_width`, saved when it changed); a panel widened by what it holds
(Watermark's controls) keeps the width given, and the inspector's strip, dragged closed, keeps nothing.
`panels::edge_keys` and `edge_shown` take that id. The panel sets its minimum width to its available width, since
egui narrows a panel to its contents, such as one short line wrapped ("This document has no comments or markup.").
The status bar says what is selected or the tool in use (`Editor::heading` while the editor owns the panel), then
(batch 41) `view_buttons` (Page view and Organise pages, `Act::Organise`) and `zoom_bar`: − (`icon::MINUS`), a
logarithmic `Slider` (10–1000 %, the grid's 8–60 % while organising, `SliderClamping::Never` so a zoom past its ends
isn't changed by being drawn; hidden under `SLIDER_ROOM`, 900 points), + and `zoom_menu`, which says the zoom alone
("96%", Fit width ticked in its menu) in a box as wide as "1000%", right-aligned: the row is laid out from the right,
so a label that changed width (it said "Fit width · 96%" until zooming turned fitting off) moved every button left
of it (the user's find, `the_status_bar_zoom_stays_put_when_zooming`). The box is an `allocate_ui_with_layout`
round the menu button, whose hover frame stays the text's. With the label's width fixed the status bar's `dialogs::reading_order`
no longer redoes the frames of a fit zoom changing as a panel folds (`request_discard`), so the inspector's fold
takes its own seven frames; `the_inspectors_heading_leaves_a_line_edit_open` settles twice for it.

**Organise** (NEW-14, batch 41, `tab/organise.rs`) shows a tab's pages as a grid of cards in place of the pages
while `Tab::organise` holds its state (`Tab::set_organise`; `Ctx::organise`; `App::act` sends the page view's own
commands, `of_the_page`, back to it at the first selected page: tools, Fit width, the layouts, the Pages view and
Crop, whose box is dragged on the pages). `Tab::show_grid` lays the cards out from the left
(`Cards`: a card is the document's median page size, `typical_size`, times `Tab::grid_zoom`, which is
`Settings::organise_zoom` lent and kept by `App::keep_grid_zoom`), draws and asks for pictures only for the rows in
view and one more (`AHEAD`), lets go of those beyond two (`KEEP`), and registers every card's and checkbox's
interaction before skipping the ones out of view (a click egui is following on a widget left out of a pass is
dropped). The background is the grid's one Tab stop (`Tab::grid_id`, a list box "Page grid", `Sense::click_and_drag`)
with a focus lock filter for the arrows and Esc; the cards (`Sense::CLICK | Sense::DRAG`, named "Page N", selected
state) and checkboxes take clicks only, and a click on either gives the grid the keyboard. The keys are
`Tab::list_keys`, called from `App::shortcuts` before the commands' keys when the grid has the keyboard or nothing
has (or the thumbnails' list has it): `list_key` takes one press with exactly the modifiers asked for (never Alt) of
the arrows, Home, End, Page Up and Down (a screen is `Organise::screen` rows, a row `Organise::cols`), Space
(with Ctrl/Cmd too) and Enter; `key_to` selects the card reached alone, from `Tab::anchor` (Shift), or nothing
(Ctrl/Cmd), and `Organise::focus` is the card the keys are at, ringed once `keyed`. `take_keyboard` requests the
grid's focus and cancels egui's own arrow-key focus move, which egui decided from the press at the start of the frame
when nothing had the keyboard and would apply at its end, taking the keyboard straight on from the grid. While the
grid or the list holds the keyboard (`Tab::holds_keys`), `App::shortcuts` and `App::type_over` treat it as nothing
holding it, so Delete, Esc, Ctrl+X and Ctrl+V and the Alt+Shift moves work there; the commands' plain keys now need
Alt to match exactly (egui's `matches_logically` ignores an extra Alt, so Alt+Shift+Left was also Nudge's Left).

Moving pages: a card's `drag_started` selects it alone unless it is selected and sets `Organise::dragging`; each
frame `Cards::gap` finds the gap nearest the pointer (over a card, the side of its half; past a row's end, after its
last card) as the place to land (`Organise::drop`), drawn as a bar (`Cards::bar`) with `moving_label` ("Move N
pages", shared with the thumbnails' `drag_feedback`) and `edge_scroll`; the pointer let go over the grid moves the
pages there with `Tab::move_selected_pages`, outside it (or after Esc, `Tab::escape_pages`) nothing. `Act::MovePages`
(`PageMove`: Earlier, Later, Up and Down a row, Start, End; Alt+Shift with an arrow, Home or End) is
`Tab::move_pages`: a place or a row by `shifted_order`, each selected page as far as it goes without passing the
ends or another selected page, so pages apart stay apart; to the start or end by `moved_order`, together. Both go
through `Tab::reorder_pages` (one `Tab::arrange` step); the selection follows its pages by object id
(`follow_pages`), and `show_placed` brings the first into view (the card while organising, else `go_to`). Cut with
pages selected (grid or thumbnails) keeps their objects in `Tab::cut` and puts "Pages 2, 3 of name" on the system
clipboard: egui-winit sends Ctrl+V as a paste only when the clipboard holds text or an image, and the page names
make sure it does. Paste (`Act::Paste`, never matched as a key: the paste arrives through `App::type_over`, which
gives it to the pages cut while organising, with pages selected, or with nothing selected or typed, as the band's
Paste does) is `App::paste_pages`: pages cut in the tab in front are `Tab::paste_pages`, a move to
`Tab::insert_at` (with nowhere else for them it says so). PF-48: Copy with pages selected (`App::copy_pages`)
keeps a document of them (`Pdf::extract`, as Extract makes) in `App::copied_pages` with the source's name and
whether it is protected (`Tab::is_protected`), the clipboard getting their names as for Cut; pages cut in another
tab are extracted from it at the paste, with a `CutFrom` (the tab, the pages by object as cut). Either goes in
through Append PDFs' path: `undrawable` refuses pages whose form answers can't be drawn, form fields go through
`Dialog::ConfirmAppendForms` (`AppendForms` now carries `protected` and `cut_from`), and `App::add_pages` (taking
`protected`, saying whether it added) inserts them at a `PagesGo` naming the page before `insert_at`. Once added,
`App::take_out_cut` deletes the cut pages still in their tab by object (its own `Tab::arrange` step), unless they
are all its pages, which stay (the paste was a copy, said in the tab in front). The clipboard holds one thing:
Copy and Cut of anything first `App::let_go_of_pages` (the copy dropped, every tab's cut taken back), so at most
one tab has pages cut and never with a copy. `Ctx::cut` (`App::pages_to_paste`: a copy, or any tab's cut) enables
Paste; `why_not` tries "Cut or copy pages first." and "Select pages first." before the page view's reasons while
organising. A click on the background between cards
puts the insertion bar there (`Organise::bar`, the place and whether it is drawn after the card before it) and lets
go of the selection, as a slide sorter; `Tab::insert_at` is the bar, else after the selected pages, else after the
current page, for Insert blank page (at the page before's size), Insert images…, Paste and Insert PDF pages…
(`Act::InsertPdf`: `App::insert_files` reads the files in the background as Append PDFs does, `Appending::to` a
`PagesGo` naming the page after which they go by its object, so a change meanwhile doesn't misplace them, through the
form-fields question too; core's `Pdf::insert_pages`, which `append` now calls at the end). `Tab::placed` selects
the pages added while organising. `follow_pages` drops cut pages that went, follows the keys' place and the
thumbnails' by object, and clears the bar; `escape_pages` takes back, in turn, a drag (cards or thumbnails), a cut,
the bar and, organising, the selection.

Pictures: `Tab::arrange` keeps every other page's picture and thumbnail by object (`pictures_by_page`,
`put_pictures`) and marks its undo step `Step::arranged` with the pages it turned; `Tab::restore` keeps them the same
way when it goes back or forth over such a step, so Undo and Redo of page moves, additions and removals draw again
only the pages turned. The side panel's thumbnails (`show_thumbs`) are one Tab stop too (`Tab::list_id`, "Page
thumbnails", `Sense::FOCUSABLE` over the panel, the thumbnails `Sense::CLICK | Sense::DRAG`), with `list_keys` in a
column (`Tab::thumb_focus`, `thumb_screen` thumbnails to a screen, `thumb_seek` to scroll one into view); a key that
selects goes to its page.

Long documents (batch 41's 10,000-page check): `Tab::new` and every `Tab::refresh` read the page sizes (and, while
the crop is set, the whole pages' sizes and the crop margins) from core's lists made in one walk of the page tree,
`Pdf::page_sizes`, `full_page_sizes` and `all_crop_margins`. A loop asking `page_size(i)` or `crop_margins(i)` page
by page walks the tree once a page (PB-29): that stopped the window for about 33 s on 10,000 pages, on opening and
after each page change (`a_long_document_is_read_again_in_one_pass`). The way back from the grid: `set_organise`
puts the page that was at the top of the page view back there by its object (`Tab::rezoom`), and a zoom change
before the page view is drawn keeps that place (`Tab::zoom_to` leaves a `rezoom` waiting as it is): Fit width works
its zoom out again in that first frame when the widest page changed while organising, and `Tab::view` is by place,
from before the pages moved.

The inspector (batch 39's second stage: PF-40, with ENH-38 and ENH-41) shows the editor's section for a selection
or a tool that adds content (`Editor::owns_panel`), else `inspector::show`, which draws the task open alone, else
the selected pages, else the document. A task is a `Section` for which `Section::is_task` holds (all but
`Properties`, a section of the document's inspector), headed by `Section::title`, and kept in `Forms::task` until
closed. `Forms::choose` makes a task the one open, in place of another, or closes it for Properties, and sets
`reveal`; `show` does the same with a section revealed, and a `Forms::close` naming the task closes it (Esc on Crop, `Act::Escape` while
`Tab::crop_view`, or `Tab::set_crop_view` when typing on the page can't be kept). Setting the crop ends as a word
processor's picture crop does, all through `Tab::end_crop` (batch 42): Esc, Enter with nothing focused and no
question open (`App::ui`, after `shortcuts`), Crop again (`Act::Crop` while `crop_view`; the band shows it pressed
from `Ctx::cropping`), a click on a page that isn't a drag of the crop box's edges (`crop_page_ui` senses clicks
and drags), and a click on the page view's canvas beside the pages (sensing clicks only while cropping); the status
bar says "Cropping N pages" from `Tab::cropping`. Each section's controls are one
function, `tool_ui`, used for the task alone and inside the selected pages' Page properties, where Crop and Page
labels are each a section of their own (a second `Sections`, whose open list joins the outer one, so the pages show
the crop box while either Crop is open); `split_ui` is Split's. The facts are `document_facts` (pages; paper;
security from `Tab::encryption`, the protection saving will write, or "Removed on save" for a protected file
opened (lopdf's `was_encrypted`) until a password is set again, as its notice says; form fields when there are any; signatures from
`Tab::sigs`' `Check` for a file with signature fields) and `page_facts` (paper, and `orientation`: Portrait,
Landscape, Square or Mixed), from `Tab::sizes`, the pages as shown, after rotation. `paper_of` names their paper
with `paper` (Letter, Legal, Tabloid, Executive, A3, A4, A5 and B5, either way round, within 2 points; else "W × H
pt"), "Mixed" when they differ, and `pages_of` writes the selection as people do ("Pages 1–2, 5 of 9"). The look is
four functions the editor's section uses too: `facts` (moved from `editor.rs`: muted 12-point labels at the left,
values at the right), `note` (a 12-point muted line, in place of `ui.weak`), `section` (a thin separator, then a
`CollapsingHeader`) and `section_name` (its 12-point name); the heading stays 15-point medium (`inspector_heading`),
and `inspector_body` sets the indent to 10 points. `facts` rows are at least 17 points high (egui's grid would make
them the interact height, 24), 8 apart, as the mockup's. The panel is `theme::inspector_frame`: the band's surface
colour (white, or the dark theme's surface), 14 points either side, 10 above (its strip keeps egui's 8 × 2);
`theme::inspector_style`, set in `inspector_body` (the panel and the pop-out), fills text boxes with the number
boxes' colour, as the surface's own would hide them. `act_button`'s label is its button's shrinking atom
(`atom_shrink`) and wraps (egui would otherwise take the icon, the first text atom, as the one to shrink), so a long
name such as "Extract pages to a new tab" never widens the panel, which would change the pages' zoom.

`App::reveal` opens the inspector, lets go of an object, word or comment selection (a comment's section otherwise
kept the panel from the tool), gives a content-adding tool way to the read tool, drops a page selection only for
Properties, then calls `Forms::choose`; a tool kept with a page selection fills its Pages box from it
(`sync_forms`), Export pages as images included. Each frame, before the panel is drawn, a selection or tool that
`Editor::owns_panel` closes the task, shown or not, so it doesn't come back when that is let go. `inspector_head`
draws the heading row for the panel (`collapse`) and the pop-out with `egui::Sides`: the title (wrapped onto a
second line when long, whole, as the menu names the tool), then, in
`dialogs::reading_order` so that Tab goes in the order shown, the close button (`icon::X`, named "Close" and the
task's title) and, in the panel, the collapse button. The buttons' width is kept from the last frame with how many
there were; when that changes it starts at 48 points a button, more than they take, so they never show past their
place and widen the panel, which egui would then keep (`the_inspector_keeps_its_width`). The collapse button has a
scope id of its own (`COLLAPSE`), the same with or without the close button before it, so a close from the keyboard
(no pointer click that frame) can give it the keyboard; in the pop-out the keyboard goes to the strip's button that
opens it (`POPOUT_OPENER`). The inspector's Future text tools, marked Planned, went with Underline and
Strikethrough built (batch 46's 4b), and with them `Editor::planned`, which showed it.

F6 and Shift+F6 (ENH-10, batch 26) are `Act::NextRegion` and `PrevRegion`. `App::shortcuts` runs them while a text
box has the keyboard, as it does F3, Shift+F3 and F7 (`FindNext`, `FindPrevious`, `Spelling`) and the Ctrl (⌘)
shortcuts other than those for what is selected on the page (`page_only`: Copy, Cut, Select all, Bold and the like,
which the box takes for its own text); F6, F3 and F7 never run while a list or menu is open. Shift is told apart
exactly for F6, as egui otherwise ignores an extra Shift. They only set `Regions::step`; `App::move_region` does the move at the end of
`App::ui`, once every region and control of the frame is drawn. `crates/limarepdf-gui/src/regions.rs` keeps where
each region was drawn this frame (`Regions::add` from each panel's response; the notices' scope from
`Tab::banners_at`; the inspector's pop-out with `add_over`, looked in first), finds a control's region by where its
middle is (the notices and the inspector looked for before the page, which they lie over), and remembers the
control last given the keyboard in each. The next region round is the first with a control that can take the
keyboard this frame (`regions::controls`: enabled, shown, focusable, on a layer not behind a dialog), the last
used if still there, else the first in Tab's order. The page is the exception while a document is open: there the
keys go to the document with nothing focused, as everywhere in the app, so moving to it surrenders egui's focus
and sets `Regions::ring`, lent to `Tab::page_ring`, which outlines the page being read until a press or any
control takes the keyboard. Leaving the page, `move_region` notes what had it in `Editor::parked`: typing still
open (`Parked::Typing`, the id from `Editor::typing_id`, its selection noted by `page_text::held`), which
`line_edit_ui` and `writing_ui` don't end while it waits, and which `App::keys_to_page` gives back with its
selection and its undo (`give_back`, `hand_back`); or a form field's text box (`Parked::Field`, by name and
widget), which keeps its value as it loses the keyboard and `FormUi::focus_field` opens again. A press in the tab
in front, or its typing getting the keyboard back or ending, ends the wait (start of `App::ui`). Switching tabs
parks typing the same way: `App::front` holds the id of the tab in front last frame; when another comes to the
front, the tab sent behind gets `Parked::Typing` for typing still open (`Editor::typing_id`), tabs behind keep what
is parked, and the tab brought forward gets its parked typing or field back through `keys_to_page`, so a line
being edited (which `line_edit_ui` would otherwise end, its editor without the keyboard) stays open across the
switch. Each panel is named
for screen readers with its role by `regions::name`, which finds the panel's own node up `Ui::stack` (egui 0.37
gives each panel a node, unnamed); the notices are a scope built with their role and name. A document tab is
`Role::Tab`, its selected state set on its node (egui's `WidgetInfo::selected` gives a toggle instead).

A panel's edge (ENH-11) is egui's resize handle, under the panel's id (`ui.scope_id().with(name)`, where `ui` is the
one the panel is shown in) with "__resize"; before batch 26 the names were set on `Id::unique(name)`, which isn't
it, so they never showed. `panels::edge_keys`, before the panel is shown, reads Left, Right, Home and End while the
edge has the keyboard, keeps the arrows from moving the keyboard on (`set_focus_lock_filter`), and puts the new
width in as an AccessKit SetValue request on the edge, the way egui 0.37 lets a screen reader resize a panel, so it
goes through egui's own clamping (the inspector no narrower than its contents); it never asks below
`panels::NARROWEST`, so the keyboard never folds the inspector to its strip. `panels::edge_shown` names the edge
and draws the accent line along it while it has the keyboard. The outline that moves new text (`page_text::field`)
senses drags only, without egui's focusable flag, so it is no stop for Tab; the start screen's card likewise.

The optional timing log (`crates/limarepdf-gui/src/timing.rs`, REL-07) is for measuring the app: with
`LIMAREPDF_TIMING` naming a file, `main` starts it first thing, and each point calls `timing::mark` (or
`timing::once`, for an event written only the first time with its detail) with a closure for the detail, so the
log off costs one `OnceLock` check and no formatting. It is one process-wide log behind a mutex, so the render
workers write to it without being handed anything. The points: `start` (in `main`), `app made` (end of `App::new`),
`input` (each key, text, click, wheel and clipboard event, in `App::raw_input_hook`, at the start of the frame that
takes it; pointer moves are left out, and what is typed is never written, as it may be a password: text by its
length, a key by its name only when it types nothing: Enter, Esc, Tab, Backspace, Delete, Insert and the
movement keys), `frame built` (end of `App::ui`, with egui's frame number and pass, the log
then flushed), `page drawn` (`Tab::show_pages`, the first time a page is painted from a drawing at the scale it
wants rather than a stand-in at another), `render start` and `end` (the worker, per page and tile; a stamp
preview's worker too, ENH-23), `stamp preview failed` (its copy couldn't be made, with why), `open start`
and `end` (the read thread), `search start`, `search first hit` and `search end` (`Tab::start_search` and
`take_search`, the query by its length, the matches by their count) and `search index` (the search thread, the
text kept in bytes), `save start`
and `end` (the save's thread, since batch 37) and `save shown` (the status bar drawing a save's busy sign, once per
save, by its number), `print dialog` (Print's `act`), `printers found` (`poll_loads`) and `exit`
(`on_exit`). A frame is built before egui
paints it and the window presents it, which the app can't see: the gap is measured from outside by screenshot
polling ([Performance](performance.md#startup-and-responsiveness)). Lines are tab-separated: epoch and log-relative
microseconds, event, detail. A file that can't be created, or a write that fails, is said once on stderr and the
log stops; the app is never affected otherwise. Tested by `timing`'s own tests and
`the_timing_log_follows_a_file_from_its_read_to_its_page_drawn`, which runs the log in a child test binary as it
is the process's.

## Content editing

How editing works in the page content (moved from [Editing text and pictures](../content-editing.md), which describes it in
plain words). The page's content stream is split into objects: one per text line, per painted path, per image or
form XObject, and per inline image.

- **A text line** is the show operators of one `BT … ET` block that sit next to each other on one baseline
  (`objects.rs`, `group_runs`): less than 0.3 of the larger text's size apart across the line, and from 0.3 back
  to 0.7 forward along it. Font, size and colour don't split a line: its parts in one style each are its spans
  (`PageObject::spans`: characters, font, size, colour, and rise, how far a part such as a superscript sits above
  the baseline of the line's largest text). A gap wider than 0.18 of the smaller text's size reads as a space,
  which belongs to the part before it, and so does a `TJ` spacing number under -150 times the font's em
  (`FontCtx::em`): the number is in thousandths of a unit of text space, which is a thousandth of an em only in
  a font of the usual scale (PB-64: in TeX's bitmap fonts, about 83 units to the em, dvips moves the pen inside a
  word by thousands, "Op erating"). A part's colour is the colour its fill shows as (`Span::fill`) and what set
  it (`Span::paint`, `colour.rs`, PB-65): the operators as read (`g`, `rg`, `k`, or `cs` then `sc` or `scn`), the
  colour worked out from the space a `cs` names in the page's resources, each read once a parse: device,
  calibrated and ICC-based spaces by their components, Lab to sRGB (its white taken as sRGB's), Indexed through
  its table, Separation and DeviceN through their tint transforms (`colour::Function`: types 0, 2, 3 and 4, the
  calculator's stack held to 100 values) into the alternate space; a pattern, or a space that can't be read,
  leaves the colour shown as it was. Parts in one font, size and rise split where the colour or what set it
  differs (grey and RGB judged by the colour alone). The line's box (`line_box`) covers its shows' boxes less those showing only
  whitespace and less the whitespace at the line's two ends, cut off at the characters' measured positions; the
  text keeps them (BUG-25: a label's tab in a larger font reached over the next label, which a click, the selection
  and the edit cover then took as the first label's). A Type 3 font draws its glyphs in a space of its own
  (`/FontMatrix`), so besides its widths (`FontCtx::scale`) its text's size is the size set times `FontCtx::em`:
  1, unless the font's letters and digits, each against its usual width (Times' and Helvetica's averaged), put
  its em more than a quarter away (TeX's bitmap fonts, glyphs in pixels at a size of 1, about 83); and a matrix
  that turns its glyphs over stands the line up again where the text matrix turns text space over
  (`FontCtx::flipped`, which way the glyphs stand read from their `d1` boxes, else the font's box; PB-56). A
  bitmap stored upside down stands only in its pixels, so such a font reads as it stands by its boxes. An edited
  line written anew keeps a flipped font's letters (`LineFont::flipped`, `Written::flipped`): each stretch of its
  parts in one is drawn in text space turned over, a `Tm` with its second row negated where the stretch starts
  and one the right way up where the next part starts, placed where the line has got to by reading the line as
  written (`Pdf::turned`, so the parser and the writer agree), its rise written the other way; a line changed in
  place keeps the page's own matrix. A kept Type 3 font's letters are written at the size in the font's own terms
  (`Span::em`).
- **The displayed page** every position is on (`page_box`, `visual_matrix`): the crop box where it overlaps the
  media box, as the PDF standard reduces a crop box reaching past it, turned with the page; a page without a
  media box is A4, and an overlap with no area an A4 page from its corner. These are hayro's rules, so the page as
  drawn and core's positions and sizes agree (BUG-43: pdfbox `1727`'s crop box, 648 by 1043 round a 612 by 792
  media box, put everything core gave on it up to 230 points off).
- **Where a line's characters are** on the displayed page: `PageObject::dir`, the way the line runs (a unit vector),
  and `PageObject::stops`, where each character starts along it from `baseline` and where the last ends, from the
  same measured positions as search boxes (`caret_stops`). A line edited in a plain box (turned, or invisible OCR
  text) puts the caret at the stop nearest a click, measured along `dir` (`editor::caret_along`), and a
  double-click finds the word by the words' order along `dir`, a repeated word too (BUG-26: the caret went by how
  far across the box the click was, which a quarter-turned line doesn't run along).
- **Paragraphs** (NEW-01, batch 45's stage 2; `paragraph.rs`, the paragraph design's section 4):
  `Pdf::page_blocks` reads a page once (its objects, its lines' marked content and fonts from one parse) and
  gives its blocks of text, each a run of visual lines with its frame (left and right edges, first baseline,
  spacing, alignment, first-line indent) and, unless it is confident, why not (`Doubt`). Visible horizontal text
  outside form XObjects only. The steps, each rule a named constant from batch 44's prototype: an XY cut of the
  lines' boxes into regions (a band 0.5 line heights empty cuts across, as against reading order's 0.8), each
  region in the column the nearest cut down the page made, columns cut in different bands one where they line up;
  bands are cut widest first (those at least 0.75 of a region's widest together), and a cut down a region comes
  before its bands when every band is the space between lines (under 0.8 line heights), the cut is wider than
  each, and the lines on both sides are typically a column's width (their median at least 10 line heights): so
  columns of lines on common baselines with loose spacing, each gap under a line a band across all the columns,
  are cut whole rather than into one-line pieces of different widths that don't line up (batch 45's 2b-1, made-up
  `lo06`; a table's narrow cells keep their bands first, so its rows still read as a table);
  in each region, lines on one baseline (and smaller raised pieces) a visual line, across text objects, split
  where a gap is over 1.5 sizes and over 2.5 times the line's word gap (its spaces' median; with none, its gaps',
  three or more); in each column, top to bottom, a line joining the block of the line above when that is the
  block's last and size, colour and font agree (a line under 80% one colour or font is emphasis), the baseline gap
  is 1 to 2 sizes (or the page's own spacing, up to 3) and within 15% of the block's median, the edges agree
  (left, right or centre; a first-line or hanging indent with the second line), no list label starts it (a bullet,
  "1." or "1)" always; "1.2", "(3)", "a)", "iv." only with a gap after them) and its first word wouldn't have
  fitted at the end of the line above (measured against the paragraph's own width, up to a picture beside it).
  Breaks are clear or weak; a block is confident with two lines or more (three centred), clear breaks both sides,
  even gaps and edges, at least 8 sizes wide and half its column, no table or form signs, some line reading on
  into the next, not a heading's size and nothing else in its frame (its font no longer counts since 2b-2: an edit
  keeps every letter's own code, so a font that sends typed words elsewhere, Type 3 or a CMap other than Identity,
  doesn't make it uncertain; the plan's Q3 dropped). The frame's alignment is justified when both edges agree on
  every line but the last, and for a block of two lines only when the first line's word gaps are also over 1.1
  times the last line's (`JUSTIFY_GAPS`): the longer line's end alone is no edge (batch 45's 2b-2, found by the
  corpus edit run, pdfbox `1570`: a ragged pair taken as justified had its rewritten first line stretched 1.2
  sizes a space). In a tagged document,
  lines whose marked content (`BDC` MCIDs through the parent tree, `struct_parents`) belongs to different block
  elements (P, H1 to H6, LBody, Lbl, TD, TH, Caption, Note and the like, by the role map) are never joined.
  `Pdf::page_blocks_by` turns the tags or the fonts off, for the opt-in check (`tests/paragraph_check.rs`, which
  judged the default off: testing.md). The app reads a page's blocks when one of its lines is selected (below,
  "Editing a paragraph in the app").
- **Editing a paragraph** (NEW-01, batch 45's 2b-2; `rewrap.rs`, `para_write.rs`, the paragraph design's sections
  6 and 7). `Pdf::paragraph(page, &Block, &DocWords)` reads a block for editing: its text as one (each line's
  pieces left to right, a space between two only a word's gap apart, 0.18 sizes; lines joined by a space except
  after a hyphen or dash, where a line ends or starts with one, or between Chinese or Japanese characters), a part
  (`TextSpan`) for each part of each line's text objects, each character's text object and place (`source`), each
  line's characters, and its line-end hyphens read by the document's own words (`Pdf::document_words`, every
  page's visible lines' words, built in parallel): discretionary (a soft hyphen there, or the word found joined
  elsewhere and not hyphenated) held in the text as a soft hyphen, hard, or unknown (kept, marked when a rewrap
  brings it inside a line). Refused for a turned, hidden or vertical line, or a text mode that clips (`Tr` 4 to 7).
  `Pdf::paragraph_setting` reads the parts' fonts once (`LineFont`, and the font each part's typed words go to,
  `FontChoice::of_font`); `ParagraphSetting::set(&[TextPart])` lays an edit out (`SetParagraph`: each line's
  characters, the old line it keeps or none, x, baseline, width, the stretch a space when justified, a soft hyphen
  shown), measured as written: a kept letter by where it was drawn, and what was drawn between it and its old
  neighbour while they stay neighbours (in one text object, or a letter drawn from inside the one before it from
  another, `Paragraph::drawn_into`: it starts inside that one's width, as TeX draws `\"u` with the accent first,
  PB-73, and pdf.js `issue12402` each Chinese word after a "、" in the comma's empty half, S1-14; the writer writes
  that move back, so Poppler reads the letter and its mark in order again, where it had read "̈u" once the line was
  rewritten, pdf.js `issue1127`, and a move back can't read as a space; other abutting objects keep a word's own
  gap, as keeping theirs misread more lines in the corpus edit run); a typed one by its font's PDF width
  (`LineFont::advance`); a space by the
  font's space or, where it has none, the narrower quarter of the paragraph's drawn spaces; words in another font
  by a `Setter` holding all of that font's words, each glyph cluster's advance on its first character (S1-14,
  M-12: the whole word's width went on its first character, so a word broken across lines was measured as all on
  the first). Lines break only where `Measured.legal` allows: a grapheme boundary not inside a cluster, whether
  the setter shaped it (`Face::spelled` maps each character it writes back to the caller's, `Laid::clusters`) or
  the page drew it as one code (a ligature). Once a line's end is chosen it is measured as it will be written
  (`ParagraphSetting::finalise`: its units, `Unit::Kept` stretches, `Unit::Space` and each `Unit::Word { .. }` laid by
  its setter, Tc counted once a code written), and moved back a boundary while wider than its room; one cluster
  wider than the room stands alone on its line, as overflow. `laid_in` returns those units with the
  `SetParagraph`, and the writer draws the glyphs that were measured (`pen_laid`), shaping nothing again; a line
  retried in its fallback fonts is measured again in place (`retry`) and refused if it no longer fits its room
  plus 0.5 points. Greedy breaking at UAX #14's places from the line the edit can
  first change: the line before is rewritten only when the next line's first word fits now and didn't before; a
  line that starts where it did ends where it did when the words greedy breaking would add fitted before too (the
  producer's breaks stand where the edit doesn't move them); once a line starts at an old line's start with the
  rest unchanged and nothing pushed down, the rest are kept. Baselines: the old lines', the frame's spacing past
  them; a line whose text is larger than any the paragraph had gets 1.2 times its size below the line before, at
  most 1.2 times what the size grew by, so a paragraph set tighter than single spacing keeps its spacing (2b-2's
  tests: the first rewritten line wasn't pushed, and a 10-on-11 paragraph drifted a point a line). Justified lines
  stretch each space to the edge, but the last and one before a typed break (U+2029 for Enter, a new paragraph
  with the first line's indent; `\n` for Shift+Enter); centred and right-aligned lines are placed on the frame's
  centre or edge (`Placing::on`, a frame's room by `room_in`). `ParagraphSetting::set_between(parts, (left,
  right))` sets it with the frame's edges dragged there (3b-2; `set` is it with the frame's own): moved, every line
  is broken anew to the new width from the first (no line before the edit kept, no producer's break standing, no
  rest kept), and a line is kept only where it starts and ends as an old line did in text that hasn't changed and
  `Placing::on` places it under the new edges as under the old ones (its x and, justified, its stretch), so a ragged
  paragraph's right edge moved keeps every line whose break stays, and a left edge moved or a justified edge moved
  writes them anew. `ParagraphSetting::set_in(parts, edges, align)` (NEW-15, batch 49's stage 1; `set_between` is
  it with the paragraph's own alignment) sets it aligned anew: the frame (`frame_in`) takes `align`, and the first
  line's indent only from a paragraph read as left or justified (one read as centred or right-aligned has none: its
  "indent" is its first line's width); `room_in` gives the indent's room to left and justified lines only. A change
  of alignment alone breaks no line anew, as Word realigns: with the edges as they were the producer's breaks stand
  as for an edit (lines kept, laid out by the same rules), then each kept line is placed again by `Placing::on`
  under the new frame and written anew (`kept: None`) only where that differs from its place under the old one, in
  x or stretch (the longest line of a ragged paragraph centred, and a justified one's last line made left, stay
  byte for byte); the laying-out loop keeps a line that starts and ends as it did and is placed the same, as for
  moved edges. Word spacing: a paragraph read as justified writes no `Tw` (its `Tw` was the stretch) whatever it
  is aligned to now (`spacing`, by the alignment read), so justified made left is written at a space's own width.
  `ParagraphSetting::narrowest(parts)` (`narrowest_in` for an alignment) is the widest stretch between break
  places, plus the first line's indent where it takes one: how narrow the app lets the frame go.
  `Pdf::edit_paragraph_between` writes that (`edit_paragraph` is it with the frame's own edges;
  `edit_paragraph_in` and `edit_paragraph_decorated` take the alignment). **Kept frames** (`para_frames.rs`, the
  user's finding in batch 49's stage 1): a PDF keeps no paragraph's edges, so a centred paragraph read again took
  its widest line's extent as its frame, and each realignment, kept and read again, moved it on (a justified
  72–512 frame went 87–500 centred, then 87–504, 95–504...); a widened one narrowed back. After a write whose
  edges or alignment differ from the frame read, whose frame was itself kept (`Paragraph::kept`, so the record
  follows its lines), or that writes more than one paragraph (BUG-47: Enter typed, U+2029 before a set line's
  first character), `keep_set_frame` records `[top bottom left right indent spacing]` for each paragraph written
  (its first and last baselines, the frame's edges, `frame_in`'s indent, which Enter's paragraphs take too, and
  `ParagraphSetting::leading`, the spacing the lines were set at) in the page's `/PieceInfo /LimarePDF /Private
  /Frames` (ISO 32000's page-piece dictionary; other apps' entries kept; its `/LastModified` and the page's, as
  the standard asks), replacing records of lines overlapping the edit's before or now (`keep_frames`, all of one
  edit's together), at most 500 a page; nothing is written for an edit of one paragraph in the frame read, so
  plain typing and Shift+Enter add nothing. A record of five numbers (none written since BUG-47) reads with no
  spacing. `Pdf::paragraph` takes a record's edges and indent, and its spacing where the paragraph has one line
  (`kept_for`: first and last baselines within 0.5 points, every line within its edges to its size, a hanging
  first line its indent further) and marks it kept, and `frame_in` then keeps the indent whatever the alignment
  read. `Pdf::page_blocks_by` reads the paragraphs recorded on its page as one block each (`regroup`): where
  detection splits their lines (a centred paragraph's shorter lines read as paragraph ends), joins a
  neighbour's line to them, or joins the one-line paragraphs Enter made into one narrow block, the lines between
  a record's first and last baselines within its edges become a confident block in its frame, aligned as they
  sit there (`frame_of`: justified, left, right or centred within 0.15 of a size or a point, else the alignment
  read; spacing the lines' median gap, the record's for one line), the blocks they came from keeping the rest,
  in doubt; a block that is a record's lines already gets that frame and is confident too, so a line LimarePDF
  wrote as a paragraph of its own isn't "on its own" (the app's `paragraph::refused` refuses a one-line block
  only when it isn't confident).
  Lines moved by other means stop matching and the frame is read from them again. Undo restores the page's
  dictionary with the content; Remove hidden data's private data drops `PieceInfo` and `LastModified`
  everywhere; redaction drops a redacted page's `PieceInfo`. The finding's other causes, found by realigning 406
  corpus paragraphs ten times each, writing and reading each anew (a throwaway probe; drifting on stage 1's
  code: 32 of 72 in the first sample and 269 of 334 in the second; now 3, two in fonts LimarePDF can't read
  and one block whose widest first line reads as hanging): **measuring**, a kept character whose neighbour no
  longer follows it is written at its code's own width (`LineFont::code_advance`) and is measured so (`own` in
  `measure`), not at its drawn width, which a page drawing a line's last letter with its width taken back
  (`[(e) 444] TJ`) reads as none: lines written anew had come out a letter (4–9 points) wider than set, right
  of where they were placed (Type 3 fonts, whose widths are in their own units, keep the drawn width); and
  **squeezed lines**, a line kept as its words were that is wider than its room at its spaces' own width (a
  justified line its producer squeezed, or one read a letter short) fills the room with its spaces squeezed
  (`Placing::on`, a negative stretch, at most 0.35 of its narrowest space, so a space written as a move still
  reads as one), whatever the alignment, rather than reaching over a neighbouring column (rebreaking it instead
  added a line over the text below). `Pdf::edit_paragraph(&ParagraphSetting,
  &[TextPart])` writes it in one rewrite (section 7): line
  *i* in place of old line *i*'s first show, lines added after the last old line's last show, the old lines'
  shows taken out (`move_only`, `Following` keeping what follows each in place); inside the text object a `Tm` in
  its CTM, the font, colour (as set, `Paint`), rise, `Tc`, `Tw` and `Tz` set, then put back as the show had them
  with `Tm` to its line matrix; no text object added, nothing appended, unchanged lines byte for byte. Each kept
  letter keeps its own code across the paragraph's lines (TeX's flipped fonts too, through `Pdf::turned`), a typed
  one its font's code, a word the font can't write goes by one pen a font with the paragraph's `Tc` and `Tz`.
  Each written line is read back (`misread`, `Check::Within`, with what it may join in its text object; the lines
  of text it reads as join as the parser joins a line's pieces, a space where one starts a word's gap past the
  other or on another line, `apart`, so a line whose wide move reads it as two isn't taken as misread, nor one
  with a raised footnote mark: 2b-2, found by the corpus edit run); one that misreads is written again with its
  letters in the fonts their words go to, and a second misread refuses the edit with nothing changed. Lines that
  read right are then checked for where they sit (`Pdf::misplaced`, S1-14): the first character must start at
  the line's x, the last end at x plus its width and stretch, and the box stay inside the frame's right edge
  (unless the line was set wider than its frame), each within 0.5 points, from the new operators parsed as the
  page will be; a line that fails is written again as a misread one is, and fails again refuses the edit. Each
  try given up puts the page's own `/Resources` back (PB-46). Kept pieces of one line are written with the move
  the measure counted between them (`to_neighbour`, PB-75: the move between "r" and "¨" was lost, so "Pr¨ufung"'s
  "u" started 1.2 points inside the "r"). A kept letter is written with the character spacing of the show it was
  drawn in, as the measure counts it (S1-14), and what was drawn between two neighbours in different shows of one
  text object opens the later one's show where, at the end of the earlier one's, `tj` would make it one number
  that reads as a space (`ends_in_space`, S1-14: pdf.js `issue12294-print` draws "th" and "e" 0.176 em apart in
  two shows, no space between shows, and they read "th e" written as one `TJ`). Returns the lines whose letters
  went to another font.
  Building `DocWords` reads every page (the app runs it off the window's thread, `Pdf::document_words_on` with two
  threads of its own, when a tab gets its document; until it is there, hyphens read as unknown).
  `ParagraphSetting::pieces` gives an edit's stretches in one font each, kept in a part's own font or sent to the
  font its words go to, as the writer plans them (`planned`), for the typing box to draw each in its face;
  `part_font(i)` the parts' `LineFont`s.
- **Editing a paragraph in the app** (NEW-01, batch 45's stage 3a; `editor/paragraph.rs`, the paragraph design's
  sections 5, 6.4, 8 and 9). Paragraph editing is a mode, off unless chosen (`PARAGRAPH_DEFAULT` stays off, as
  the check judged): the app's `Settings::paragraphs` (`text_editing` in settings.txt), set by `App::set_paragraphs`
  from `Act::EditParagraph` (a toggle: the band's Text flow, the menus, the command search) and the inspector's
  Editing row (`Editor::switch`, read by the app), lent to every tab's `Editor::paragraphs` each frame; it switches
  the text being edited in the tab in front (`Editor::set_paragraph`). `start_text_edit` opens the paragraph for a
  double-click, a Text-tool click or Edit text while it is on and the block can be one, else the line. The band's
  Text flow group keeps typing open (`Group::keeps_typing`), so that its click doesn't end the line being edited
  before the switch (the user's finding after 3a: it went back to reading). The page's blocks are read when one of
  its lines is selected (`read_blocks`, cached with the objects); the inspector's switch, and why its Paragraph is
  greyed (`para_state`: on its own, a table, turned, invisible), come from them. Edit text (`band::is_on`) is
  pressed while the Text tool is on or text is edited on the page, and clicked when pressed goes back to the read
  tool: the app reads that from the commands' state as the frame drew them (`App::shown`), as the click itself ends
  a line being edited first. A switch keeps what was
  typed first (`keep_later`) and opens the other editor at the caret's place once that is in the document
  (`Then`, `follow_then`: the line under that point, as ids change with a rewrite, or else the line in its row
  nearest it, `row_hit`, as a caret placed while the line's font is still loading, drawn in Inter, can sit past the
  line's end). The caret goes after the same characters, not to the same point (BUG-44: a font that came in
  between drew the other editor at another width, the caret three characters on): `Then` carries the line's text
  before the caret, as the line will read (a line's typed text to its caret, `LineEdit::caret_at`'s character; a
  paragraph's from the start of the caret's row as drawn, `ParaEdit::caret_at`'s row start from
  `Galley::layout_from_cursor`, soft hyphens left out), and where the line found starts with it (spaces that ended
  it dropped, as they aren't written) the editor opens with `Pick::Chars`, else at the point. A double-click or a
  Text-tool click that opens a line or paragraph is placed by the line itself in `start_text_edit` (`on_line`: the
  point among the line's characters by `PageObject::stops`, where the page draws them, the word around it for a
  double-click), never on the editor's drawing (BUG-45: drawn in Inter while the line's font loads, wider, the
  double-click selected an earlier word where the page's words didn't match the line's one for one, `word_pick`'s
  fallback, and the click put the caret characters back); `Pick::At` reaches the editors only from a switch and for
  a line whose `stops` don't match its characters. `ParaEdit` holds the block, the
  `ParagraphSetting` (built on the window's thread: the installed fonts' list is read at start, so reading one is
  a few milliseconds, the cost line editing's fonts already pay on heavy pages), the text as `Styled`
  (`Styled::of_paragraph`, every character's origin kept, so `parts` names what is kept), its own `History`, and
  the unknown hyphens kept as typed, the frame's edges as dragged (`edges`) and its alignment (`align`, NEW-15).
  Each frame the text is set between them, so aligned (`ParagraphSetting::set_in`), and drawn by
  `page_text::paragraph_field` on `paragraph_galley`: the text laid out with a line break after each of core's
  lines (put in, taking no character, as `boxed_galley` does), each row then moved to its line's x and baseline
  (rows' positions are public; a glyph's `pos.y` is its baseline), a justified line's spaces widened by a
  section's `leading_space` after them (egui's `extra_letter_spacing` spaces only between a section's glyphs), a
  soft hyphen shown as "-" where it ends a line and inkless elsewhere (egui's shaping gives it no width), U+2029 as
  a row's break; the galley's text is the text as typed, so the caret, selection, double-click and input methods
  go by it. Enter and Shift+Enter are taken before the box sees them and put U+2029 or `\n` into the text at the
  selection (egui types no break from a text event). The old lines are hidden, each in the box a line being edited
  is hidden in (`paragraph::covers`), by the page drawn without them (below, "The page under text being edited"),
  the flat colour around the block standing in until it comes (`cover_wanted`, one sample round the block's box).
  Overflow by the one rule above (`page_text::overflow`). **The frame's edges** (3b-2): `paragraph_field` gives the
  outline a handle on its left and right edges (`edge_ui`: an 8-point strip over the edge's height and a 7-point
  square at its middle, drawn as an object's handles, filled while dragged or holding the keyboard, a 2-point line
  along the edge then; `Sense::click_and_drag`, a horizontal-resize pointer; named "Paragraph's left edge" and
  "Paragraph's right edge" as AccessKit splitters; ids `page_text::edge_id`), registered after the text so they
  win where they meet it, and focusable, so Tab from the text reaches them. A drag moves the edge by the pointer's
  travel from its press (`ParaEdit::dragging` keeps the side and the edges it started from), the text keeping the
  keyboard (a drag doesn't surrender it); a click gives the handle the keyboard (`request_focus`: egui gives none
  on a click). While one has it, a focus lock filter keeps Left, Right and Esc for it: the arrows move it a point,
  ten with Shift (Shift taken first, as a plain key's match takes it too), Esc ends the editing as from the text,
  and the paragraph stays open (`on_edge`), as it does for the text; Ctrl+Z and Ctrl+Y go to the paragraph's
  history there too (`page_text::take_undo`; `App::shortcuts` counts `Editor::edge_held` as typing, so the
  document's Undo doesn't take them), as do the band's Undo and Redo (`App::state` and `Editor::typing_history`
  read the paragraph's history for a handle's id, and `undo_held` keeps the handle's keyboard through the click). `edge_to` keeps an edge on the page and no nearer the other than
  `ParagraphSetting::narrowest`. Each frame's edges go to the history (`History::observe_edges`, the user's Q2): a
  drag is one step however long, key moves within the pause typing joins over one, and Undo and Redo put the edges
  back with the text (`History::edges`). Core sets the lines between the new edges (`set_between`: every line
  broken anew, one kept only where it starts and ends as it did in unchanged text and the move places it as it
  was, across and in its stretch) and writes them (`Pdf::edit_paragraph_between`). **Links and comments**: when
  the editor opens, the page's annotations and links are read into the editor's caches (`annots`, `links`), and
  each frame `ParaEdit::marked_over` says whether one not hidden meets the old lines or the outline; the
  inspector then says "Links and comments over this paragraph stay in place." Kept through `Typed::Paragraph`
  (`keep`, `put_back`, `settle_kept`, `refused_again` with the edges too, `pending`), as `Action::EditParagraph`
  (the edges with it) through `Tab::edit_inner`, one step; no action when the text, the edges or what they set is
  unchanged. Words selected across a paragraph's lines (`words_paragraph`:
  one block, with paragraph editing on or confident; within one line, with it on only) are typed over, deleted (Word's space rule)
  and cut in it, and Edit text opens it with them selected. Marked hyphens (`SetParagraph::marked`, less those kept)
  are dotted under; `Act::JoinHyphen` and `KeepHyphen` act on the one at the right-click or the caret. The
  inspector's Editing row is a horizontal row as wide as the panel, not a grid of its own, which widened the panel.
  A band group of one command folds to that command's icon (`band::folded_group`), as a menu of one item would be
  wider and no nearer. **Alignment** (NEW-15, batch 49's stages 1 and 2): `Act::AlignText(TextAlign)` (`TextAlign` is
  core's `Align`; `editor::Align` lines objects up), four registry entries in ☰ > Edit's "Align text" submenu
  (named apart from Align objects' in the command search, `spoken_title`), Word's keys `k(C, L|E|R|J)`, global so
  they reach a paragraph with the keyboard (Bold's keys are taken by the box instead); enabled by
  `Ctx::aligns_text` (`Editor::aligns_text`: page text being edited, words selected, or a text line among the
  objects selected), else "Click in a paragraph's text first."; `checked` by `Ctx::text_align`
  (`Editor::text_align`: the paragraph being edited's, else the alignment every block of the other text's lines
  has, from blocks already read, `read_blocks` reading them for a visible page with selected lines too). In the band, Edit's Alignment group is `Item::Icons` (each command's registry icon
  alone, as Word's, named on hover with its key; a group that keeps typing open; without room it folds first after
  planned-only groups, `fold_order`, to the icon of the alignment in use and a caret, `folded_group`, 50 points
  against 160 open, so at a 1000-point window Editing's Edit text and Find stay shown); in menus they are buttons with
  their keys, the one in use selected, closing the menu (`menu_item`'s one-of-a-set case); the floating bar has
  Centre after the styles while a line or paragraph is edited, and for words or lines selected after the group with
  Bold or Edit text (`toolbar_items`; the right-click menu, `context_items`, hasn't it), pressed when centred. `App::act` treats them as the styles: they change the typing, which stays open (`in_line`).
  `Editor::align_text` sets `ParaEdit::align` (the one it has pressed again: Left, as Word) and gives the text the
  keyboard back (`refocus`); each frame `History::observe_align` makes a change a step of its own, and the
  paragraph's Ctrl+Z and Ctrl+Y put it back with the text and edges (`History::align`, `para_undo_redo`).
  `para_change` counts a changed alignment as a change, and `Action::EditParagraph` carries `(edges, align)` to
  `Pdf::edit_paragraph_decorated`. The Paragraph switch's icon is ¶, so it isn't read as Align left. **Page text not
  edited as a paragraph** (batch 49's stage 2): `App::act` hands `align_text` the document and shows what it returns
  as a notice. In a line edited alone it finds the line's block (`alignable`: hidden, vertical, turned, no block or
  `paragraph::refused` counted in `NotAligned` by why, the line given the keyboard back), then `line_to_paragraph`
  (shared with the Paragraph switch) keeps the line's typing and sets `Then::Paragraph`, which carries the
  alignment: `follow_then` opens the paragraph once the typing is in and applies it there by Word's rule, so
  `Editor::paragraphs` stays as it was; if the paragraph can't open then, the line opens again and `align_note`
  says why (the tab shows it after `apply_actions`). For words or lines selected (`lines_to_align`: `Chosen`'s lines,
  whose `Left` counts too, or the one text line selected), each alignable block once, in reading order; Word's rule
  over them all (every one already so gives Left), blocks already so left out, none left no action; else
  `Action::AlignParagraphs(pages' blocks, align, NotAligned)`, one `edit_pages` for every page, through
  `editor::align_paragraphs`: per block `Pdf::paragraph` (its kept frame), `paragraph_setting`,
  `Styled::of_paragraph`'s parts, `Pdf::edit_paragraph_in` between its frame's edges; a block after a page's first
  found again by `replace_all::follow` (its objects by kind, text and box, which the earlier writes left) and
  `paragraph::refound` (shared with Replace all). A block that fails is left and counted, the error only when none
  is written. Afterwards words are selected again (`reselect`) and selected lines found where they moved
  (`Editor::lines_moved` sets each one's expected box to the line of its text in its row before the page is read
  again, as `Selection::refind` needs boxes that meet). **New text, text boxes and fields** (batch 49's stage 3):
  `Editor::aligns_text` gives what the commands act on (`editor::Aligns`: page text, new text, a text box typed in
  or selected, a field typed in, nothing), in `Ctx::aligns`; `text_alignable` enables Left, Centre and Right for
  all but a field and nothing, `justifiable` Justify for page text only, and `why_not` says the reason for a field,
  and for Justify on new text or a text box, before its list. `App::act` leaves new text or a text box being typed
  open for them (`Editor::aligns_typing`). New text (`PageText::align`, from `Editor::talign`, the alignment last
  chosen, carried on as the style is by `carry_on`) aligns its rows among themselves while typed
  (`page_text::aligned_rows` in `field`'s layouter through `Style::align`: each row moved by the room its letters,
  measured from the pen as `band_rects` measures, leave beside the widest, `moved_rows`), its own `History` taking
  each change as a step (`History::aligned`, `observe_align`, put back on Ctrl+Z); `Action::AddText` carries it to
  core's `Pdf::add_text_lines`, which measures each line with the `Setter` the pen writes it with (the parts in one
  font by one setter, as `written_lines` makes them, `new_text_width`) and writes each with `add_text_decorated`
  (the widest at the point clicked, the others moved by `Align::share` of the room). A text box: `TextStyle::align`
  (`/Q` read by `Align::of_q`, written by `text_style`, removed for left, justified written left by `Align::q`);
  a new one changes the Text box tool's `box_style`, which `AddTextBox` writes; one typed in or selected gets an
  `Action::SetAnnot` (`align_box`, taking the inspector's changes not yet kept with it), the typing staying open
  as with the inspector's changes (ENH-13); `AnnotEdit::same_as` compares the alignment. A field's is its form's
  (`Widget::align`, its `/Q` else its field's), only shown.
- **Moving or resizing** wraps the object's drawing operators in `q <matrix> cm … Q`. What a text line's
  operators set between its shows (the font and colour of its parts) is set again after the `Q`, so the text
  after the line is drawn as before.
- **Editing text** keeps the line's own fonts where it can (ENH-31; the writer is described after "Which code
  writes a character" above). A line in parts (`TextEdit::parts`, each part text and a style; a words-only edit
  is one part in the first part's style) is written as one `BT … ET` with each part's font, colour, rise (`Ts`)
  and spacing set where they change; a part in a colour of the line's that `rg` wouldn't write as it was (a spot
  colour, CMYK, Lab) is set with the operators that set it there, the span it names first (`kept_paint`), the
  words that fall back to another font too (PB-65). A part keeps the font of the span it names when its text is that span's,
  unchanged (its shows are copied as they were, with their character spacing, word spacing and scaling, and a
  gap read as a space put back as the same move); otherwise character by character, each the line's own code
  where it is the line's own, unchanged, else the font's code for it (`LineFont::code`; for an unembedded
  standard-encoded font, plain ASCII as itself), a word with a character it can't write going to the font chosen
  for it, else the installed family the line's font is from (found by its PostScript name, the subset tag left
  off; `FontChoice::of_font`), else the nearest of the 12 standard fonts, the parts in one font sharing one font
  object; what that font lacks falls back a word at a time as all new text does (the text pipeline above). A line restyled in one style (a `TextEdit` with a font, size or colour) is written anew on top in that
  style. `Pdf::edit_texts` makes several lines of a page in one parse and one commit (the shows of every line
  taken out, each line's `Following` in the page's order, the lines on top in the order given), the parts of all
  its lines sharing the fonts they add (`written_lines`); `edit_text` is it for one line. A line in one style and
  spacing changed in words all in its own font is one `TJ` in place of its first show. Before committing, each
  line with characters written in its own font is read back as the page will be (`misread`; the lines of text it
  reads as joined by the parser's word gap, batch 45's 2b-2): one that reads as other words is written again with
  only its unchanged parts in its own font, and reported. While a line is
  edited on the page, the app makes the same decision as it goes, from the line's `LineFont`s (`Insp::fonts`;
  `PageObject::kept_pieces`, `editor::drawn_pieces`): the fonts of all a page's lines are read together the first
  time one of them is selected and kept with the page's objects until the document changes
  (`Editor::read_line_fonts`, `Pdf::page_line_fonts`), so selecting one line after another doesn't read the page
  again (up to a third of a second each time on a heavy page; performance.md). It draws each piece in the
  font it will have: a kept embedded font in its own glyphs (its preview font), a kept unembedded one in the face
  LimarePDF's pages draw it in once kept (ENH-25, batch 45's 3b-1; `LineFont::stand_in`, `stand_in.rs`): hayro
  0.9 draws a simple font it doesn't hold in one of its standard faces, the one its name is, else by a word in the
  name ("helvetica", then "arial" or "sans", "courier", "mono", "times", "serif", in that order; bold by a
  descriptor weight of 700 or "bold" or "demi", italic by a non-zero italic angle or "italic" or "oblique"), else
  by the descriptor's serif flag, bold and italic by "Bold" and "Italic" in the PostScript name and the italic
  flag (it reads no fixed-pitch flag or weight there), so the typing box draws the Liberation face of that
  standard font; a CID font without such a word goes to `InstalledFonts::find` with hayro's question
  (`InstalledQuery`: its names, style by name and character collection, the CMap's unless Identity, else
  `/CIDSystemInfo`), drawn in what that finds (`typing_fonts`' `Candidate::Found`, a stand-in too), else the
  standard face by the same rule. `tests/stand_in.rs` checks the rule against hayro's own drawing (each case's
  page against a font hayro can take one way only, at the same widths, pixel for pixel). Symbol, ZapfDingbats
  and a line whose fonts aren't read yet keep the installed font of the name, else the Liberation face of its
  kind by the name. The installed family's font, or the
  Liberation face of the standard font replacing it, otherwise. The inspector names the typed words, as typed and
  each once, with a character the font has no code for, and the family they use (batch 45: before, it listed the
  letters, each once and spaced, which read as a word spaced out), or that the font can't take new text at all (Type 3,
  not embedded, vertical writing, unreadable codes; `editor::font_note`). The old line is hidden by the page drawn
  without it (below), the most common colour in a 3-point ring around its box standing in until that comes, from a
  separate drawing of that part of the page in the current page colours. Below that note the inspector names the words that then fall back from the font they go to
  (`page_text::fallback_note`), and the typing box draws each in the face it will be written in.
- **Underlines and strike-throughs** (NEW-07, batch 46's stage 4a; `decorate.rs`, the design note
  `limarepdf-underline-design.md`). PDF text has none: a decoration is a thin band drawn under or through some
  characters. The parser records each painted path's shape (`Seg`; the dash pattern in `Gs::dashed`);
  `decorate::straight` makes it a band (`Straight`: centre line and thickness in user space) when it is filled
  rectangles (an `re`, or four corners, any way round) in a row of one thickness with gaps under a quarter of its
  length (macOS breaks an underline around descenders; a dotted line is half gaps), or one stroked segment without
  a dash, as thick as its round pen is across it as drawn (`lw |det ctm| / |ctm u|` for a segment along unit `u`;
  S1-16, M-13: the pen's width was projected onto the drawn normal, so a band under a scaled, turned or slanted
  transform was read thinner, half as thick at twice the size). The page's own content's bands are kept apart
  (`Painted`: the path's first operator, the band, its colour,
  and `InkAt`, the operators that set its colour, so no ink is copied while a page is read). `decorate::read` runs
  at the end of `parse_content` for the page's own content (not a form's), lines and other drawings kept in
  64-point cells (`Grid`) so a page of many objects isn't read pair by pair. A box over 4,096 cells (a page-wide
  background) is kept in, and looks in, only its edge cells, gone through directly, each once (`each_cell`; S1-07,
  H-08: the whole area was gone through, so a line drawn hundreds of metres across took billions of steps); one
  with more than `MOST_CELLS`, 2¹⁵ (32,768), even so is kept in none and finds nothing, so a box at most 8,193
  cells, about 185 metres, square still meets bands. Each cell a box is kept in or looks in is a step of the page's
  reading budget (`objects::STEPS`, "Reading a page" above), taken before any is touched, and so is each part of
  the rest that can grow faster than the page (CP-05), taken before it is done: each box the grids give for a
  band, as often as it is in the cells looked in (a lookup gives each box once by marking it, `Grid::seen`, not
  by sorting what it found); each line's way set against the ways before it, and each thin band's against the
  ways, up to the first that is the same (`any_of`); each character of a line a band is tried against; each line
  of a form's text looked at for a band; each decoration found before set against a band's; and each band already
  in the decoration a band joins (put in its place by halving). What isn't counted (a band's object found by
  halving, each line laid out once, the decorations put in order) grows no faster than n log n of the page's
  lines and bands. Of two lines a band fits as closely, the first on the page takes it, whatever order the grid
  gives them in. A band running no way the page's lines
  run is passed over, and a line is laid out only once a band is level with it and near its baseline
  (`Lite::may_take`; performance.md's "Reading underlines and strike-throughs"). It keeps a band for the line it
  fits best (`fits`, in the line's own frame,
  `Frame::of`: along its baseline and up its letters, so turned pages and rotated text read alike; `Laid` is a
  line's characters as laid out, their edges, sizes, rises, inks and UAX #29 word edges): at most 0.15 em thick
  and 0.2 em long for the characters it covers, level with the line, an underline's centre between 0.40 em below
  the baseline and 0.05 em above with its lower edge 0.02 em below, a strike-through's 0.15 to 0.45 em above,
  both ends within 0.15 em of a character's and on word edges, over two characters or more that aren't spaces with
  no jump of 2 em between neighbours, in their colour within 0.02. Then no other drawing may meet its ends (but a
  band under the same line's text, and a fill reaching past both ends behind it), no other line's box overlap it,
  and no text sit centred under it within its ends less than 0.6 em below while narrower than it or within 0.1 em
  of it (a fraction's denominator; a justified paragraph's next line is as wide as its underline). Nor may it be
  one of three or more bands in a row evenly spaced, those near it of its thickness (within a tenth), level with
  the line and along the band, their gaps within a tenth of each other, each wider than the band and narrower than
  an em: a music staff's lines (`evenly_spaced`; S1-16, PB-101: a staff drawn in pieces, one under a label such as
  "Violin I", was read as the label's underline and strike-throughs, and carried with it when it was edited). Two
  bands of one kind over the same characters are one `Decoration` with two bands (a double underline). `Parsed::decorations`
  holds them by line; `PageObject::decorations` gives a line its own (kind, characters, `Look`: bands in ems of
  the characters' size and the ink, and the paths' ids), `PageObject::decorates` a path its line. The rule's
  numbers come from producer samples and the corpus check in the design note (469 decorations in 65 files).
  `Pdf::edit_texts` carries a line's decorations with its characters (`decorate::carried`, by `TextEdit::origins`:
  a character keeps its decoration, one typed takes the one before it's); `edit_texts_decorated` takes them as
  given. A line whose words stay as they are (`TextEdit::unchanged`) isn't written, only its bands. After the
  edit is built and read back, the page is parsed once more and each line's characters as written are found by
  where they were written (`Edited::places`: the show in place, the operators on top, or an unchanged line's own,
  through `Edited::watched`) and lined up with the edit's text by their longest common run of characters that
  aren't spaces (`align`). A decoration wanted that is an old one in its look over the same characters where
  they were stays as drawn; the others' paths stop painting (their paint operator becomes `n`) and each
  decoration wanted is drawn on top, one filled rectangle a band (`band_ops`, in the line's frame) for each stretch
  of one part of the written line, at its size and rise: a read one in its own look and ink, a new one at its
  font's place (`font_lines`: an embedded TrueType or OpenType program's `post` underline and `OS/2` strike-out,
  each given by its top; else the standard fonts' AFM underline, 0.1 em below to its centre and 0.05 em thick,
  with the strike-out of the Liberation look-alike of the standard font the font's name is nearest,
  `FontChoice::from_name`, which reads no installed fonts). Spaces at either end of the line are left bare.
  `edit_paragraph` (`paragraph_with`) maps its lines' decorations onto the paragraph's text
  (`rewrap::decorations_of`, by `Paragraph::source`; `Pdf::paragraph` fills `Paragraph::decorations` the same way
  for the app), carries them as a line's are, stops all the old ones painting and draws each set line's stretch
  where that line is written (its `Check::Within`) or kept. `add_text_decorated` draws new text's in the layer the
  text is written in. `transform_objects` (and so `set_object_boxes`) moves a line's decoration paths with the
  line's own matrix, and `delete_objects` and `page_without` take them with it (`Pdf::with_decorations`). Redaction
  treats them as any drawing: one crossing a mark is cut out of it. Where a font puts its bands is public for the
  typing box (stage 4b): `LineFont::lines` (a line's part's, from the program bytes `OwnFont` holds, so none is read
  twice) and `SetIn::lines` for text as it is set (`set_in` gives each stretch's `SetIn`: a standard font written
  as itself has its AFM underline and look-alike strike-out; an embedded face, a standard font's bundled Liberation
  one or an installed one, its own `post` underline and the look-alike strike-out of the standard font its
  PostScript name is nearest, since `subsetter` keeps no `OS/2` in what it writes), each read once (a cache by
  `SetIn`). Both it and `font_lines` fill in what a program doesn't say by one rule (`lines_given`).
  `decoration_lines(f)` is `SetIn::of(f)`'s: `f` as it is written when it can write all of its text. (S1-16,
  PB-102: the typing box took a `FontChoice` alone, so words a standard font writes in its face, as it does all
  its text when some of it isn't WinAnsi, were drawn with the AFM underline, 0.042 em above the face's.)
- **Underline and Strikethrough in the app** (NEW-07, batch 46's stage 4b). `styled::CharStyle` has `underline` and
  `strike`, each a `Deco`: none, `Font` (at its font's place, in its colour) or `Read(k)`, the look of the text's
  decoration `k` as read (`Styled::looks`, filled by `Styled::of` from `PageObject::decorations` and by
  `of_paragraph` from `Paragraph::decorations`), so `CharStyle` stays `Copy` and typed characters take a read
  decoration as they take a font (`Styled::sync`). `Styled::switch_deco` is Word's rule (all on turns it off, else
  the characters without it take `Font`, those with it keep theirs); `Restyle::switch_deco` for a line's selection
  (`InLine`) and words across lines (`Across`, one selection), `Shown::underline` and `strike` all-on. Every writer
  passes decorations: `Styled::decorations` (each stretch with one on: a read one's in its look, a `Font` character
  beside a read one on the same stretch and in its colour in that look, others `look: None`), so the app carries the
  read ones itself; `Action::EditText` and `EditParagraph` carry them, `Action::Restyle` holds `LineChange`s (`(id,
  TextEdit, Vec<Decorated>)`), `Styled::lines` gives each new line its own, and Replace all writes through the
  decorated calls too. A decoration-only change leaves a line's words as they are (`TextEdit::unchanged`), and a
  paragraph's (`para_change` compares `decorations()` as well as the set lines). `Act::Underline` (Ctrl+U, ⌘U) and
  `Act::Strikethrough` (`STRIKE_KEY`, Cmd+Shift+X with `mac_cmd`, which egui matches only on a Mac; `shown_key`
  leaves such a key out elsewhere) run as Bold does (`styleable`, `uses_document`, `App::act`'s `styles`, the
  `page_only` keys, `Editor::switch_style` with a `Switch`), and inside the typing boxes `take_typing` takes Ctrl+U
  before egui's `TextEdit`, whose own Ctrl+U deletes to the start of the line; in any other box (a comment's text,
  a field, Search, a dialog's field) `App::shortcuts` takes it first and does nothing (PB-77), unless
  `Editor::styles_focused` says the box with the keyboard takes styles (new text, a visible line or a paragraph
  being edited). The Font group's buttons are
  `font_list::styles`'s (`font_list::Lines`; the next new text's are `Editor::tlines`, carried on as its font is),
  in place of band.rs's Planned `UNDERLINE` (gone, with `Editor::planned_text_tools` and `FontGroup`'s planned
  slot); the menus and floating bar list the styles in `actions::STYLES`. A line's decoration paths aren't
  selected apart from it: `Editor::hit_where` takes a click on one for its line (`PageObject::decorates`), and
  `select_all_objects` and the selection rectangle (`inside_rect`) leave them out. While typing, `page_text::Run::place` is
  where the font a run is written in puts its bands (`Place::Own`, the line's own font's, though the run is drawn in
  another; `Place::Set`, a stretch's `SetIn` from `set_in`, read when a band is drawn), `char_bands` gives each
  character its bands in pixels (`Styled::drawn_decos`), and `band_rects` places them on the galley: along each
  row from its first character that isn't a space to its last, from a stretch's first glyph's pen (the advances
  before it, plus a justified line's stretch after each space, `ParaStyle::bands`) to its last's end, on that
  glyph's baseline (`pos.y`, a raised run's own), painted after the galley by `field` and `paragraph_field`
  (egui's `TextFormat::underline` sits at the line box's bottom). `bands_are_drawn_where_they_are_written`
  checks them against the bands written, within 0.13 px at 2 px a point; two more (S1-16, PB-102) check words a
  standard font writes in its face, new text and a kept line's new words, within 0.02 em.
- **The page under text being edited** (ENH-21, batch 45's 3b-1; `tab/backdrop.rs`, `excerpt.rs`). A line drawn
  in place, or a paragraph, being edited asks for the page drawn without its text objects
  (`Editor::backdrop_wanted`: the page, the objects, the box the editor hides). Core's
  `Pdf::page_without(page, ids)` makes that page alone as a new document's bytes: its content with those objects
  taken out as `delete_objects` takes them (`Pdf::without_objects`, shared: a text object's shows by `move_only`,
  what follows kept in place by `Following`), its boxes, turn, inherited resources and transparency group put on
  it, the catalogue's optional content settings kept (a hidden layer stays hidden), the objects its resources and
  those settings reach copied with their numbers (`copy_used`, never a page or the page tree), and the
  annotations the app's pages draw (`drawn_annots`, PB-76, batch 45's 3b-2: those with an appearance, not Hidden
  or NoView, not sticky notes, which the app draws over the page itself, as `app_snapshot` leaves them out), each
  under its own number, cut from what reaches the rest of the document (`/P`, `/Parent`, `/Kids`, `/Popup`,
  `/IRT`, `/A`, `/AA`, `/Dest`, `/V`), their appearances copied by `copy_used`. A comment without an appearance,
  which `app_snapshot` draws for the page, is drawn into the page alone (`draw_annotations` on the new document,
  PB-76, S1-18); when a field on the page of a signed document has no appearance, the page alone is made from a
  copy of the document with its fields drawn (`refresh_appearances`), as `app_snapshot` draws them, so the
  document itself is never given a drawing. It never walks the page tree beyond finding
  the page (`extract` goes through every page), so it costs per page: 3.4 ms on big.pdf's pages, about 1 ms on
  corpus pages with large pictures, 68 ms on the heaviest corpus page, less than deleting one object there
  (performance.md), except on a signed document's page with an undrawn field, where it copies the document and
  draws its undrawn fields first (not measured); it runs on the window's thread when editing starts, once for the text edited (the tab's
  `Backdrop`, keyed by page, objects, box and the document's revision). A thread of its own (`backdrop::draw`)
  draws the box's part of it at the scale the page is shown at (its tiles' at a high zoom; at most 4,096 pixels a
  side, stretched past that) and in its colours (`render_region_in`, as the page worker draws tiles); the editor
  draws it under the typing (`Editor::backdrop`, `page_text::Cover::Page`, each hidden box taking its part of the
  picture). A zoom or colour change draws it again from the same bytes, one drawing at a time, the newest asked
  for once one ends; after a zoom the last drawing is shown stretched meanwhile, after a colour change the flat
  colour (`Cover::Flat`, the ring's), which also stands in until the first drawing comes and stays if it fails
  (the failure in the timing log). Nothing is drawn again as the text is typed.
- **Adding** text, shapes or images draws them on top of the page. New text of several lines is written as one
  text line each, their baselines 1.2 times the size apart (Acrobat's spacing), aligned among themselves as chosen
  (`Pdf::add_text_lines`, the widest starting at the point clicked; Alignment above). In a standard font it is written
  in one of the 12 standard fonts with the WinAnsi encoding, kerned with its Liberation face's pairs; a line with
  characters WinAnsi lacks (Polish, Czech, Vietnamese, Greek, Cyrillic and so on) is written in the bundled
  Liberation face of that font instead, embedded in the file and cut down to the characters used, shaped; one font
  object per face grows as more is typed. While it is typed, the app draws the text in the Liberation face, which
  has the standard font's widths, so the caret and line length match the written result. Text in an installed font
  (below) is always embedded, whatever its characters. A word the font lacks (Chinese, Japanese and Korean, a
  single emoji, a letter an installed font doesn't have) goes a word at a time, each ideograph alone, to an
  installed font that has it (the text pipeline above), the app drawing it in that font's file and the inspector
  naming it. Right-to-left and joined scripts and emoji sequences can't be typed in any font.
- **Pictures** added (`add_image`) or replaced (`replace_image`) are stored as the file has them (a JPEG's bytes as
  they are) and turned upright for the EXIF or TIFF orientation by a `cm` from `imgs::orient_matrix` inside their
  box, as image pages, image watermarks and picture stamps are (`add_upright_image`). A replacement is fitted into
  the old box by its upright size. `image_dimensions` gives the upright size too, from the file's header; the app's
  Image tool (`Tab::arm_image`) takes a click's box shape from it. A TIFF placed, previewed or made a signature
  picture is read to its first page only (`imgs::decode_first`); image pages (`insert_image_pages`) read every page
  on one `Work` of 2 GiB (`MOST_DECODED`; S1-08, M-06), each PNG or TIFF page charged its decoded size from its
  header before it is decoded and each JPEG its file's size, past which `PicturesTooLarge` refuses the lot.
- **Installed fonts** (the inspector's font list after the three standard ones) are the families whose fonts
  allow embedding (`OS/2 fsType`: not restricted, subsetting allowed, not bitmaps only), have outlines and the
  Latin letters; up to four faces each (regular, bold, italic, bold italic), a variable font set at weight 400 or
  700 (and its `ital` or `slnt` axis for italic). Text in one is always embedded, cut down to the characters used,
  the same way: TrueType fonts and variable fonts (set at the weight, which makes them TrueType) as
  CIDFontType2 with `/FontFile2` (codes of their own and a `/CIDToGIDMap`), OpenType CFF fonts as CIDFontType0
  with `/FontFile3 /OpenType` (codes their glyphs' numbers). The app draws the text in the font's own file, at that
  weight. Families without the Latin letters aren't offered, but words other fonts lack may fall back to them.
- **Invisible text** from OCR is edited as invisible text and stays in place.
- **Text after a changed line** stays where it was. Text on the same line can carry on from where the line ended
  (a show operator with no line move before it); when the line is deleted, edited or moved, its end is set again
  with `Tm` before that text, and the line position is set again before the next line move, which `Tm` also
  changes.
- **Inline images** are lifted out of the content before it is read, each replaced by a placeholder operator, and
  written back in place of those operators only, never of text that looks like one. The placeholder's name is
  chosen per content stream so that nothing in the stream already has it.

Every edit is checked to re-parse before it is applied, inline images lifted out as when the page is next read.
An edit that fails leaves the document unchanged.

## Signature crates

Checking and digital IDs use these pure-Rust crates (moved from [Digital signatures in depth](../signatures-in-depth.md#security-notes)),
from the RustCrypto project unless noted, all final releases:

| Crate | Version | Used for |
| --- | --- | --- |
| `rsa` | 0.10.0 | RSA keys and signatures |
| `pkcs1` | 0.8.0 | RSA key encoding |
| `der`, `spki`, `x509-cert` | 0.8.3, 0.8.1, 0.3.0 | certificates; making self-signed ones |
| `p256`, `p384` | 0.14.0 | ECDSA keys and signatures |
| `sha1`, `sha2` | 0.11.0, 0.11.1 | digests |
| `hmac`, `pbkdf2` | 0.13.0 | ID file integrity checks and password stretching |
| `aes`, `cbc` | 0.9.4, 0.2.1 | ID file encryption |
| `des`, `rc2` | 0.9.0 | reading older ID files only (3DES, RC2) |
| `zeroize`, `subtle` | 1.9.1, 2.6.1 | wiping secrets, constant-time comparison |
| `getrandom` | 0.4.3 | random numbers from the operating system (also the AES-256 file key and a protected file's ID when it has none, through `pkcs12::random`) |
| `rpassword` | 7.5.5 | asking for passwords without showing them (command line; not RustCrypto) |

`rsa` 0.10.0 refuses public keys shorter than 1,024 bits; `cms_check.rs` reads them itself, so a signature made
with one is still checked, and flagged as too short to be safe. LimarePDF's own reader and writer handle the
signature data (see [Digital signatures in depth](../signatures-in-depth.md#security-notes)), so the `cms` crate,
still a pre-release, isn't used.

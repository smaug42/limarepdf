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
  it); winit, which eframe then gives only its X11 and Wayland features, is named in `limarepdf-gui`'s
  `Cargo.toml` for its defaults (Wayland window decorations, libwayland loaded at run time), at eframe's version.
  The icons are the Phosphor fonts in `crates/limarepdf-gui/assets/fonts/`, by name in `src/icon.rs`;
- [`image`](https://crates.io/crates/image) for images;
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
the same everywhere; the desktop app's side is `crates/limarepdf-gui/src/fields.rs`.

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
without `Shell`, as a launcher with no title or buttons. Tab goes round a dialog (PB-15): at Tab on the last
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
lopdf takes as encryption. `pad_user_hash` handles a standard-security file at revision 3 or 4 whose `/U` is
shorter than 32 bytes: readers compare only its first 16 bytes there, and some writers store just those (PDFBox's
test file 1910), but lopdf (0.46 too) refuses any length but 32 (`InvalidHashLength`) and so reports a wrong password.
The copy has the encryption dictionary appended as an update with `/U` zero-padded to 32 bytes. The workaround
stays until lopdf accepts the shorter `/U`; no issue has been filed upstream.

A document's pages are its page tree's entries whose object is a `/Type /Page` dictionary (`doc::pages`, behind
`Pdf::page_count`, `Pdf::page_ids` and `page_index`). lopdf's own list (`get_pages`, since 0.46) also counts an
entry whose object is missing or isn't a typed dictionary, as a blank stand-in page; hayro passes over a missing one,
so taking lopdf's list would shift page numbers between editing and drawing, and a tree with no real page would
no longer be repaired.

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
drawn as nothing; PB-06: lopdf's simple-font encodings read `<0042>` as "B"). A code that is a character number
with no text (Identity-H with no ToUnicode map) still goes through lopdf's encodings first: in the corpus, files
such as pdf.js's `bug951051.pdf` and `issue11915.pdf` use codes that are ASCII, which that reads as poppler does
(poppler from the embedded TrueType program's cmap, which LimarePDF doesn't read yet), and U+FFFD for those
regressed about 40 files. With PB-06's rule, the corpus's text changed in one file only (`issue1597.pdf`: a
hyphen with no ToUnicode entry, dropped before, now U+FFFD).

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
`Pdf::search` (the CLI) and `find_text_matches` (one page alone, no next page) use the same code. Snippets are cut
at word boundaries (`tail`, `head`; `Snippet::around` cuts shorter for the app's rows), and `match_count` says the
count in words for the app and the CLI.

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
matches when the search for the query and the document as they are is complete (`Search::complete`), and otherwise
runs `Pdf::search` itself first. The threads' indexes count against one budget (`fit_within`, 256 MB), letting go of
the tab searched longest ago, then the pages already searched of the tab searching.

Text LimarePDF writes (new and edited text, headers, footers, watermarks, stamps, text boxes, form values, the
signature look, selected text as a plain page) goes through a pen (`embed::Pen`, from `Pdf::pen`): the standard
font, WinAnsi-encoded through `fonts::winansi_byte`, while the text fits it, else the embedded font below.
`writable` (core) tells the app which characters can be typed. Form appearances measure and encode through
`form_ap::Metrics`, which is one-byte widths or an embedded font's codes. In the
app, new text is typed on the page in `crates/limarepdf-gui/src/page_text.rs`, drawn while typing in the bundled
Liberation fonts that `typing_fonts.rs` picks (same widths as Helvetica, Times and Courier), then written line by
line with `Pdf::add_text_parts` as one undo step. A line being edited (`editor.rs`'s line editor, sharing
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
kept the same way (they were lost before). Words on more than one line get Q3's note at the selection's end
(`Editor::words_note`, drawn by `words_note_at` through `page_text::Refused::show_at`); words only in text that
can't change get `chosen::Left`'s notice. Delete and Backspace (`deletable`) and Cut call
`Editor::delete_words`: `Across::delete` takes each line's ranges out back to front (`Styled::delete`), and a space
beside a range with it, by Word's smart cut, when spaces sit on both sides or the range is at a line's start or
end; the result is one `Action::Restyle` across lines and pages, nothing left selected. A line emptied this way is
written as an empty `() Tj` in place of its first show (core's one-style in-place path): it reads and draws as
nothing, saved too. Lines aren't joined (paragraph editing, NEW-01). `Act::EditObjectText` with words selected is
`Editor::edit_words` (the moving end's line, its part selected). Shift with the arrows, Home and End is
`Act::Extend(selection::Toward)` (☰ > Edit > Extend selected text, keys only, not in the palette):
`Editor::extend_by` moves the selection's moving end by a word, to the word on the line above or below nearest
across (`editor::across`), or to the line's start or end, reading the next or previous page with words when it
crosses one; `Tab::reveal` (from `Editor::words_end`) then scrolls the least that brings the end into view, against
the view kept from the last frame's scroll area (`Tab::last_view`). The read tool's double-click and triple-click
off a line's box select nothing (`Editor::word_on_line`: within the hit tolerance of the nearest word's line).

The typing says what it does (batch 40's third stage). `page_text::field` draws the outline in two lines (ENH-22):
the edge in the accent, or `theme::Tone::Caution` when a warning applies, and a 1-point line inside it
(`outline.shrink(1.0)`) in `theme::halo` of the edge, white or black, whichever contrasts more, so it shows on
paper of any colour, the accent's own too. `page_text::Style::warn` checks the text's laid-out rect each frame:
new text (`PageText::ui`) with `page_text::past_edge` (the page's rect, a point's leeway); a line being edited
(`line_edit_ui`'s `warn`) with that, then the part typed past the line's end (from `line.max.x` plus a point,
the middle half of the line's height) against the page's other visible lines (`others`). A warning puts a tag
under the outline's right end, a `Label` so screen readers have it. A line being edited has the inspector rows
"PDF font" (`editor::pdf_fonts` of the selection, a subset's six-letter tag cut by `editor::untagged`) and "Source
font" (`editor::source_font`: `part_font` for each part still in a font of the line's own, a part given a chosen
font skipped, named Preserved, Installed family or Nearest standard; several, Partly preserved when one is
Preserved, else Mixed; none, Font chosen), with its second value, a standard font standing in, giving the
Non-standard note when the line's font isn't reusable. The status bar says "Editing one line" while
`Editor::editing_line`. `Tab::title` (the window's title) and `tab_button` take `Tab::unsaved` (`dirty`,
`Editor::pending` or typing in forms), so typing marks the tab from the first key. `writing_ui` keeps any box
being written open on a press in the inspector (`insp` is always `drawn_at` the inspector's place), and a new text
box (no annotation id, not a note) takes its size from the Text box tool's `box_style` each frame, so it shows
what it will be added with.

Greyed commands say why (`actions::why_not`). A command's `enabled` is a function of `actions::Ctx`, so `why_not`
gives a copy of the context each thing a command may wait for, in the order people meet them (a `Fix` each: busy,
saving, a file, undo, redo, words, typing, a line, a picture, an object, a movable one, items put down with Fill
and sign, redaction marks, a form, a field, the Select
tool, the Read tool, facing pages), and returns the reason of the first that alone enables the command, else of
the one that does with all those before it, else `NOT_NOW` ("Not available just now."); without a document, "Open
a document first." `actions::greyed_tip` (the tooltip, then the reason) is the disabled hover text of the band's
buttons, its menu buttons (their first item's reason), the commands row and `menu_item`. `font_list::styles` names
B and I with their keys, and greyed says the family has no such face while the group is enabled, else the group's
reason. Ctrl+B and Ctrl+I in new text and in a line go through `page_text::switch_styles`: `Styled::switches` says
whether the switch would change anything, and if not `Refused` marks "<family> has no bold." at the caret.
`page_text::note` gives `SCRIPT` for a character core's `unwritten_script` (right to left, or shaped) marks,
outside an emoji sequence. `App::type_over` with a paste and no box to type in sets `Tab::info`, saying where to
click. Tool commands in menus are `egui::RadioButton`s (`menu_item`), so the tool in use is marked; `band::is_on`
presses Edit text while the Text tool is on; `inspector::title` is "Form field" while a field has the keyboard
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
with the typing in it. A form field that loses the keyboard to a dialog keeps its typing and takes the keyboard
back when the dialog closes (`Edit::Text`'s `back`); one whose own change is refused as it is left gets its typing
back the same way (`FormUi::kept`). Panel Apply buttons that redraw only some pages or nothing don't commit first:
they lose nothing, and their forms join their own undo steps.

Undo steps (`tab.rs`'s `Step`) each hold a whole copy of the document as an `Arc`, at most `MAX_UNDO` of them and
`UNDO_BUDGET` (1 GB) per tab together, the oldest dropped first and the latest always kept (`trim`). A watermark or
headers and footers just added stay linked to the inspector's form (`Tab::stamp`, a `Stamped`) until anything else
changes; the link's `base`, the document before the stamp, is the same `Arc` as the undo step made when it was
added, so it costs nothing while that step is kept. `held` counts every copy once, wherever it is held (steps, the
links kept in steps, the form's link). When the latest step and a link's copy no step holds are over the budget on
their own, those links end (`Tab::keep_to_budget`, run after every push and after `put_stamp`) and `Tab::info`
says so (BUG-36).

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

Text the standard fonts can't write goes into an embedded font (`crates/limarepdf-core/src/embed.rs`,
`Pdf::embed_codes`): the bundled Liberation face of the standard font (`LIBERATION`, in core's `assets/fonts/`,
stored compressed by core's `build.rs` and unpacked by `bundled_face` the first time each is used; the app's
`typing_fonts.rs` draws with the same copy, added to egui then too) as a Type 0 font with a CIDFontType2 descendant, `Identity-H`,
`/W` widths and a ToUnicode map. The font file is cut down to the glyphs used by the `subsetter` crate. There is
one such font per face per document, rewritten in place as text is added: the old glyphs are given to the
subsetter first, so they keep their numbers and earlier text stays valid. The font dictionary's `/LimarePDF` entry
records the face, the glyphs with their characters and a checksum of the font file; a font another app has
changed since is left alone and a new one started. In a signed document only fonts made since it was opened
(`Pdf::made_fonts`) are grown, as growing one a signed revision holds would show as a change to it. Going back to
an earlier copy in the same session (undo and redo, a failed change put back, a stamp redone from its base) uses
`Pdf::reopen`, which keeps that set (PB-07): ids survive a snapshot and reload, and signing opens the signed file
in a new tab with no history, so no font of a signed revision can be in it; an id the copy lacks belongs to
nothing signed, whatever takes it later. Reading the copy with `Pdf::from_bytes` started a new font each time.
Characters
are put together (NFC) first; right-to-left and shaped scripts are refused (Liberation has Hebrew). A face covers
a character only when its glyph has an outline (`Face::outlined`, found once per glyph through skrifa), except
characters meant to be blank (`embed::blank`: spaces, default-ignorable marks, the blank Braille pattern): a
colour emoji's glyph is often empty, drawn by `COLR`, `CBDT` or `sbix` data a PDF has no use for, and written it
would show nothing. Text is spelled a grapheme cluster at a time (`unicode-segmentation`): an emoji sequence
(`embed::emoji_sequence`: a skin tone, regional indicators, the keycap mark, tags or the emoji variation selector,
or a zero-width joiner between pictographs) is refused whole, as its parts written one by one would say something
else (BUG-24; PF-14 is the support). A zero-width joiner outside one is an invisible mark and is written. The
app's caret note (`page_text::note`) points to installed fonts only for characters
`embed::installed_could_write` allows (not control characters, right-to-left or shaped scripts, or the parts of
emoji sequences; single emoji yes, as Symbola and DejaVu Sans draw them with outlines), and
`page_text::keep_typable` keeps or leaves out a whole cluster.

Every path that writes typed text checks it the same way (BUG-24): new and edited lines through `Pdf::pen`; text
boxes and fill-and-sign text, always Helvetica, through `embed::check_writable` in `Pdf::add_text_box` and in
`Pdf::set_annotation` for a box's new text, with the app leaving out at the caret what Helvetica can't write (the
text box on the page in `Editor::writing_ui`, the inspector's Text box in `annot_inspector`, Fill and sign's Text
in `SignPane::ui`, all through `page_text::leave_out_untypable` and `page_text::fixed_note`, which filter typed
text, pastes, input-method text and middle-click pastes alike; and the other way, `App::offer_selection` sends the
page text selected in the tab in front as egui's `TextSelectionSettled` once it settles, which egui-winit puts in
X11 and Wayland's PRIMARY selection); headers, footers,
Bates numbers and watermarks through `Pdf::pen` and `inspector::stamp_text`; form values through
`form_write::check_value`; stamp wording through `stamp_wording`; typed signatures through `typed_signature`; the
signer's name on a signature's look through `sig_sign::shown_name` (left out when it can't be written). Comments,
replies, bookmarks, document properties and page labels are text strings (`doc::encode_text`, UTF-16 with
surrogate pairs past U+FFFF), which take any character. A text box another app wrote may have characters
Helvetica can't write (`Pdf::box_unwritable`): `Pdf::check_box_drawable` refuses whatever would draw it again
into the file (a new look in `set_annotation`, `set_text_style`, a resize in `move_annotation`; BUG-39), and
`flatten_annotations` refuses when such a box has no drawing, naming its page (`embed::box_refusal` words the
reason). `Pdf::box_pen`'s `?` fallback is left only for drawings in a copy: the app's view, printing and page
images (`snapshot_hiding`). Finding an installed font that has the characters was considered and left: the
font list keeps no coverage, so it would mean reading every installed font file.

Installed fonts for writing text are listed by `crates/limarepdf-core/src/typefaces.rs`: the families in
`InstalledFonts::system()`'s `fontdb` database whose fonts allow embedding and have the Latin letters, each with
up to four faces (`FaceFile`: file, index, variation settings, PostScript name), read once (`typefaces()`; the
app starts it on another thread at launch, `font_list::warm`). The list only grows, so `Family::Installed(n)`,
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
spacing numbers, cuts drawings into sub-paths, and redacts inside form XObjects as copies. Pictures are decoded,
blacked out and saved again in `redact_img.rs`. Afterwards the pages are parsed again to check that nothing readable
is left under the marks. Hidden information (metadata, scripts, attachments, hidden layers) is removed by
`sanitise.rs`. See [Redaction](../redaction.md). A string is redacted character by character only when
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
`HiddenNames` includes the fonts, and `check_shared` compares the result for each set.

Appending pages (`Pdf::append`) doesn't merge forms. Before adding a file with form fields, Append PDFs
(`App::append_read`, BUG-19) refuses if `Pdf::undrawable_fields` (the whole-form form of `unprintable_fields`) names
any, else asks (`Dialog::ConfirmAppendForms`, holding the read documents in `AppendForms`), then flattens each
such document with `flatten_form` before `App::add_pages` adds them in one step. `limarepdf-cli merge` does the same
for its second and later inputs with `--flatten-forms`, and refuses them without it; the first input's form is
the result's.

Fill-and-sign signatures (`crates/limarepdf-core/src/sign.rs`) are stamp annotations whose artwork LimarePDF draws once:
pen strokes smoothed into curves, typed names as the glyph outlines of a bundled handwriting font (read with the
`skrifa` crate, so no font goes into the PDF), or a picture with its paper made transparent. Finish signing
flattens just those annotations. Counting and locking them (`Pdf::sign_items`, `lock_sign_items`) walk every
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
with its offset. `tests/local_dates.rs` reruns itself with `TZ` set to UTC+14 and UTC−11 (POSIX TZ strings, no
time zone files needed), 25 hours apart, so at any moment at least one is on a different date from UTC; Unix
only, as Windows ignores `TZ`.

Annotation drawings LimarePDF makes are built in `annot_ap.rs`. A line's endings (`line_end`) are painted with a
pen of their own: on a dashed line it sets an empty dash pattern (`[] 0 d`) before the ending, as `Pen::paint`
only writes `d` when the pen has a pattern and the shaft's dashes would otherwise carry on into it. The same
drawing is what flattening writes into the page.

Digital signatures are checked in `crates/limarepdf-core/src/sig_verify.rs` (finding them, the byte-range rules, and
tying each signature value to its dictionary in the version it signed, by position) and `cms_check.rs` (the CMS
signature, certificates, timestamps), over a small BER/DER reader, `asn1.rs`, that walks the data exactly as written
so signed attributes are never re-encoded before checking. The cryptography is RustCrypto's (`rsa`, `p256`, `p384`,
`sha1`, `sha2`, `x509-cert`). What changed after each signature is worked out in `sig_changes.rs`: it reads the
appended revisions' cross-reference sections itself (lopdf ignores free entries and repairs broken tables, which
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
files, the trust list) go through `limarepdf_core::write_private`. It and every document save (`Pdf::save`,
`write_file`, which the app's settings, recent files and signing settings, page images, exported certificates and
text files also use) replace files through `atomic.rs`:
a temporary file beside the target, always created new under a fresh name (never one already there), private to the
user on Unix while written, synced, then renamed over the target, and on Unix the folder is synced after the rename
so a power cut can't undo it. When only that folder sync fails, the error is a `NotConfirmed`, a third outcome apart
from saved and failed: the app and the CLI treat the file as the new one and warn that it may need saving again (the
app keeps the document marked unsaved, and its recovery copy). A file system that can't sync folders at all counts as
synced. Writes of LimarePDF's own files say what happened too (BUG-14): `IdStore::create` and `import` return a
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
A13). `limarepdf_core::disk::same_file` says whether two paths name one file (the same path, or the same once
canonicalised; on Unix also the same device and inode, so a hard link counts; the CLI's input checks use it too).
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
400-point window). Save as, saving selected text, exporting text and signing refuse a file another tab has open
(`App::in_other_tab`). When the window gets the keyboard back (`egui`'s `focused`) and when an open file is opened
again, `App::check_disk` compares the files on a thread, and a tab whose file changed shows the notice, once per
new version (`Tab::changed_on_disk`). Reload (`Act::Reload`, asking with unsaved changes) reads the file through the
same path as Open, with the tab's id and revision, and swaps the new tab into the old one's place unless the
document changed meanwhile or something is being typed that isn't in it yet: on the pages (`Editor::pending`) or
in the inspector's properties and label prefix (`Tab::typing_in_forms`; the delta audit's DA02). Typing before
Reload was asked for is kept first (`Act::uses_document`), so the revision covers it, and agreeing to lose the
changes made before still reloads. A change made between the check and the write is still replaced; there is no locking.

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
beats after each step (the printer's settings, opening it, starting the document, each page), and the wait gives
up when no beat comes for `SEND_WITHIN`, leaving the thread to finish on its own, so `App::busy` clears and other
background jobs run. The snapshot is taken before the thread starts, so the document isn't copied again. A page
takes seconds to draw, so a long job isn't given up on.

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
the tab brought to the front for its questions; dropped if the save failed, which says so); `close_asking` waits
(`AfterSave::Close`, which then asks as usual); `App::quit` and the window's close request wait for every save
(`AfterSave::Quit`, the close cancelled); `close_step` keeps its `Closing` while the save it waits for is written,
and doesn't quit while any other is; `App::after_save` runs the first one ready at the end of `App::dialogs`, with
no dialog open or waiting. Reload, Sign with digital ID and Certify are off (`Ctx::saving`) and refused in `act`
with a message (a banner's Reload, placing a signature box); `Loaded::Disk` results for a tab being saved are left
out, as its own save is replacing the file. `on_exit` joins the save threads (`finish_saves`), so a system
shutting the app down doesn't cut a write short. Tests hold the next save's write until they say go
(`App::save_gate`) and wait for it with `saves_done`.

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
them on a copy instead (`snapshot_hiding`), so a save adds only real edits. The command line never draws fields
when opening a file, so `render_snapshot` (`limarepdf-cli render`) and `print_snapshot` draw every document's fields
on their copy; `app_snapshot` draws them only for signed documents, the app having drawn the others when opening.
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

The floating toolbar (`Tab::floating_toolbar`, ENH-20) shows `actions::toolbar_items`: the right-click menu's
commands that have icons, or, while a line is edited on the page (`Ctx::line_edit`), Bold, Italic, Cut and Copy.
`actions::context_items` gives the right-click menu by what is selected: words; objects with any line of page
text among them (Edit text, Move or resize text, Copy the selected text lines | Cut, Delete the object); other
objects (Replace image… | Delete the object), so a picture's menu offers nothing greyed that never applies.
`tab::bar_place` places it from its size as last drawn (kept in egui's data under the bar's id), the selection's
box on screen (for a line being edited, joined with where its editor was drawn as typed, `Editor::line_area`) and
the page's text (`Editor::text_boxes`: its lines and words as far as they have been read): above the selection,
below, right, then left, the first inside the view that covers no text; else just above or below it, whichever
covers less text (`overlap`), as Word's mini toolbar sits by the selection (the user, 2026-10-28, after the first
version docked it at the view's top by a column heading with text all round it); at the view's top or bottom edge
only when neither is in view. Bold and Italic there change what is selected in the line editor
(`Editor::switch_style` reads it with `page_text::held`, then sets the editor's `refocus`), and `App::act` doesn't
keep the typing before them as it does for other commands that use the document.
The menus show an entry's short `label` inside its `submenu`; the palette, which has no submenus, shows the
submenu's name before it in the weak colour ("Theme › Dark"), names the row `actions::spoken_title` for screen
readers ("Theme: Dark") and searches the label, submenu and group (`actions::matches`; BUG-34, audit A18). A test
keeps every palette entry's name its own.

The workspace (batch 39, ENH-38 and ENH-41; build plan 2.2's approved mockup, adjusted by the user's checks) is
drawn by `App::ui` as panels from the top: the tabs (`tab_button`, the line under them broken under the tab shown),
then the commands panel: one row of the document's commands (`quick`, icons named in words; the File menu,
`App::menu`, lists File's commands then one submenu per `Group`, so every command is in it, with `band::FILE_PLANNED`
greyed in place), the band's names (`band::names`) and, at the right in reading order (`dialogs::reading_order`, its
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

The inspector's number fields (egui's `DragValue`) that show a document's value are built with
`clamp_existing_to_range(false)`: one shape's outline width, several shapes' width, Position and size, several
objects' X and Y, an annotation's line width and a text box's border. egui 0.37 clamps a value already outside a
field's range by default and marks the field changed, so showing a hairline outline (under 0.25 pt) raised it to
0.25: `insp_change` saw a change, `Editor::pending` made the tab unsaved, and `commit_insp` made it as soon as the
inspector settled, an undo step nobody asked for; for a filled shape, whose width isn't written, it was made again
every settled frame, so the revision moved under a quit question ("The document changed while the question was
open"). A rule under 1 pt tall was likewise made 1 pt tall (moving it down) with the next X typed. A value set by
dragging or typing is still kept in range. Any new field showing a document value needs the same.

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
results and sets `Tab::focus_query`, which `Tab::show_results` gives the keyboard. `side_panel(Side)` gives each
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
Paste does) is `Tab::paste_pages`, a move to
`Tab::insert_at`; with nowhere else for them it says so. `Ctx::cut` enables Paste; `why_not` tries "Cut pages first."
and "Select pages first." before the page view's reasons while organising. A click on the background between cards
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
`Tab::crop_view`, or `Tab::set_crop_view` when typing on the page can't be kept). Each section's controls are one
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
opens it (`POPOUT_OPENER`). `inspector_body` lends `Editor::planned` from Show planned features:
`Editor::planned_text_tools` draws Future text tools at the end of selected words' and a visible text line's
inspector, a `section` whose name is a `LayoutJob` with the 11-point Planned tag, named "Future text tools —
Planned" for screen readers (a Tab stop, unlike the planned buttons, as it opens to show its note).

F6 and Shift+F6 (ENH-10, batch 26) are `Act::NextRegion` and `PrevRegion`, the one shortcut `App::shortcuts` runs
while a text box has the keyboard (and never while a list or menu is open), with Shift told apart exactly, as egui
otherwise ignores an extra Shift. They only set `Regions::step`; `App::move_region` does the move at the end of
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
  which belongs to the part before it. The line's box (`line_box`) covers its shows' boxes less those showing only
  whitespace and less the whitespace at the line's two ends, cut off at the characters' measured positions; the
  text keeps them (BUG-25: a label's tab in a larger font reached over the next label, which a click, the selection
  and the edit cover then took as the first label's).
- **Where a line's characters are** on the displayed page: `PageObject::dir`, the way the line runs (a unit vector),
  and `PageObject::stops`, where each character starts along it from `baseline` and where the last ends, from the
  same measured positions as search boxes (`caret_stops`). A line edited in a plain box (turned, or invisible OCR
  text) puts the caret at the stop nearest a click, measured along `dir` (`editor::caret_along`), and a
  double-click finds the word by the words' order along `dir`, a repeated word too (BUG-26: the caret went by how
  far across the box the click was, which a quarter-turned line doesn't run along).
- **Moving or resizing** wraps the object's drawing operators in `q <matrix> cm … Q`. What a text line's
  operators set between its shows (the font and colour of its parts) is set again after the `Q`, so the text
  after the line is drawn as before.
- **Editing text** rewrites the string in place when the font is a plain, non-subset, standard-encoded simple font
  and the new text is ASCII, with no font, size or colour change. Otherwise the old line is removed and the new
  text is added at the same position, size and colour, in the font chosen, else in the installed family the
  line's font is from (found by its PostScript name, the subset tag left off; `FontChoice::of_font`), else in the
  nearest of the 12 standard fonts. A line in parts (`TextEdit::parts`, each part text and a style) is written
  that way too, as one `BT … ET` with each part's font, colour, rise (`Ts`) and spacing set where they change. A
  part keeps the font of the span it names when its text is that span's, unchanged (its shows are copied as they
  were, with their character spacing, word spacing and scaling, and a gap read as a space put back as the same
  move), or when the font can be reused and the text is ASCII (`PageObject::keeps_font`); otherwise it is written
  as above, the parts in one font sharing one font object. A part that is some of its span's own characters,
  unchanged (`TextPart::from`, where they start in the line as read), and starts and ends at a space or an end
  of its span (`cuttable`), keeps them as they were: `shows_cut` cuts the span's shows between character codes,
  read with the page's `FontCtx` exactly as the parser reads them (a code's decoded characters, a spacing number
  under -150 read as a space, the gap after a show), each spacing number going with the character before it (at
  a show's start, the one after). It gives up (`None`) on a cut through a code (a ligature, or a ToUnicode entry
  of several characters), through a string `codes_known` and `drawn_alike` don't vouch for, or when the
  characters don't add up to the piece's; the part is then written as above and the line reported. A space made
  by a move that would start a cut part goes at the end of the part before instead (`then_move`): at a show's
  start it reads as no space. `Pdf::edit_texts` makes several lines of a page in one parse and one commit (the
  shows of every line taken out, each line's `Following` in the page's order, the lines on top in the order
  given), the parts of all its lines sharing the fonts they add (`written_lines`); `edit_text` is it for one
  line. Before committing, each line written with cut shows is read back as the page will be (`misread`, by its
  operators' place): one that reads as other words is written again without cutting, and reported. A line in one style whose words alone change, in a
  font that can be reused, is still changed in place. While a line is edited on the page, the app makes the same test as it goes
  (`TextEdit::keeps_font`) and draws the line in the font it will have: the line's own font installed on the
  computer, or else the Liberation face of its kind, when the font is kept; the installed family's font, or the
  Liberation face of the standard font replacing it, otherwise. The old line is covered with the most
  common colour in a 3-point ring around its box, from a separate drawing of that part of the page in the current
  page colours.
- **Adding** text, shapes or images draws them on top of the page. New text of several lines is written as one
  text line each, their baselines 1.2 times the size apart (Acrobat's spacing). In a standard font it is written
  in one of the 12 standard fonts with the WinAnsi encoding; a line with characters WinAnsi lacks (Polish, Czech,
  Vietnamese, Greek, Cyrillic and so on) is written in the bundled Liberation face of that font instead, embedded
  in the file and cut down to the characters used; one font object per face grows as more is typed. While it is
  typed, the app draws the text in the Liberation face, which has the standard font's widths, so the caret and
  line length match the written result. The exception is text in an installed font (below), which is always
  embedded, whatever its characters: Chinese, Japanese, Korean and single emoji are written only that way, in an
  installed font that has them with outlines (`embed::installed_could_write`), and the app draws them in the
  font's own file. Right-to-left and joined scripts and emoji sequences can't be typed in any font.
- **Pictures** added (`add_image`) or replaced (`replace_image`) are stored as the file has them (a JPEG's bytes as
  they are) and turned upright for the EXIF or TIFF orientation by a `cm` from `imgs::orient_matrix` inside their
  box, as image pages, image watermarks and picture stamps are (`add_upright_image`). A replacement is fitted into
  the old box by its upright size. `image_dimensions` gives the upright size too, from the file's header; the app's
  Image tool (`Tab::arm_image`) takes a click's box shape from it.
- **Installed fonts** (the inspector's font list after the three standard ones) are the families whose fonts
  allow embedding (`OS/2 fsType`: not restricted, subsetting allowed, not bitmaps only), have outlines and the
  Latin letters; up to four faces each (regular, bold, italic, bold italic), a variable font set at weight 400 or
  700 (and its `ital` or `slnt` axis for italic). Text in one is always embedded, cut down to the characters used,
  the same way: TrueType fonts and variable fonts (set at the weight, which makes them TrueType) as
  CIDFontType2 with `/FontFile2`, OpenType CFF fonts as CIDFontType0 with `/FontFile3 /OpenType`. The app draws
  the text in the font's own file, at that weight.
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
from the RustCrypto project unless noted, pinned to exact versions where they are pre-releases:

| Crate | Version | Status | Used for |
| --- | --- | --- | --- |
| `rsa` | 0.10.0-rc.19 | pre-release | RSA keys and signatures |
| `pkcs1` | 0.8.0-rc.5 | pre-release | RSA key encoding |
| `der`, `spki`, `x509-cert` | 0.8.2, 0.8.1, 0.3.0 | stable | certificates; making self-signed ones |
| `p256`, `p384` | 0.14.0 | stable | ECDSA keys and signatures |
| `sha1`, `sha2` | 0.11.0, 0.11.1 | stable | digests |
| `hmac`, `pbkdf2` | 0.13.0 | stable | ID file integrity checks and password stretching |
| `aes`, `cbc` | 0.9, 0.2.1 | stable | ID file encryption |
| `des`, `rc2` | 0.9.0 | stable | reading older ID files only (3DES, RC2) |
| `zeroize`, `subtle` | 1.9, 2.6 | stable | wiping secrets, constant-time comparison |
| `getrandom` | 0.4 | stable | random numbers from the operating system (also the AES-256 file key and a protected file's ID when it has none, through `pkcs12::random`) |
| `rpassword` | 7.5.4 | stable, not RustCrypto | asking for passwords without showing them (command line) |

"Pre-release" means their programming interface may still change, not that they are untested; they come from the
same project and review as the stable crates. LimarePDF's own reader and writer handle the signature data (see
[Digital signatures in depth](../signatures-in-depth.md#security-notes)), so the pre-release `cms` crate isn't used.

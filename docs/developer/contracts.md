# Contracts

The rules several parts of LimarePDF rely on. Each says what it promises, the tests that check it, what it leaves
out and the code that owns it. A rule is changed there, with all its callers, never by a second version beside it
([Working on LimarePDF](working.md)). Contracts come here as their owners settle; until then a rule is described
where its code is, in [How it is built](architecture.md).

## File identity and destinations

**What it promises.**

- Two paths name one file when they are the same path once made absolute, with links and `..` followed, or when
  the file is there and both reach it, a hard link too: the file's own number tells it (the device and inode on
  Unix; on Windows the volume's serial number and the 128-bit file ID, as ReFS has no unique 64-bit one). A file
  not there yet is known by its folder's real path and its name.
- A file is open in one tab at a time. Opening it again, by any path or link to it, brings its tab to the front,
  and a file being read for a tab, or asking for its password, isn't read a second time.
- In the app, one writer at a time writes a file. A save, a background job (a split, page images, pictures saved),
  signing and a digital ID's export claim the files they write as they start, on the window's thread, and every
  other writer is refused them until that writer ends, by any spelling of the name, in capitals too. Exporting
  text, saving the selection and exporting a certificate write at once on the window's thread, so they check and
  don't claim. No writer writes the file another tab has open; a tab saves over its own.
- No writer in the app writes in the folder of digital IDs, by any path or link to it, where a file would take the
  place of an ID or the note of the default, or lie among them: Save As, the selection and the text saved, a signed
  copy, pictures and page images, a split and the Digital IDs window's exports are refused there (PB-167, PB-170).
- On the command line, a command never writes over a file it reads. An output (an `-o`, the trust list `trust add`
  writes, or a file named in `--out-dir`) that is the input, a picture, a password file, form values, a trust
  list, a certificate or the digital ID used, by any path or link to it, is refused with nothing written,
  `--overwrite` or not.
- A save never replaces, unasked, a file changed since LimarePDF read or saved it. The app checks before saving,
  and once more at the replacement boundary: the new file written and synced, just before it is renamed into
  place. That last check compares the bytes, so a change that kept the file's size, time and inode is seen too
  (PB-28 (2)).
- A digital ID is never written over another: two stored at once under one name are both kept, the second under
  the next free name (PB-134).
- App-owned private files (digital IDs, the trust list, recovery copies, remembered signature pictures) are for
  this user only: mode 600, and their folders 700, on Unix (O-06). On Windows they take the permissions of the
  folder they are in, `%APPDATA%\limarepdf` or `%LOCALAPPDATA%\limarepdf`, which Windows gives to the user, the
  system and administrators; LimarePDF sets no ACL of its own, and no test checks this.

**Checked by.**

- Core: `disk.rs`'s `same_file_through_any_path_to_it`, `identity_of_a_file_not_there_yet`,
  `identity_of_a_file_not_there_yet_follows_the_folders_that_are`, `identity_lasts_through_writing_and_replacing`
  and `identity_through_a_hard_link` (Unix and Windows); `atomic.rs`'s
  `a_file_changed_while_it_is_saved_is_not_replaced`, `a_change_that_keeps_the_size_and_time_is_not_replaced` and
  `an_expected_file_is_replaced`; `ids.rs`'s `two_ids_stored_at_once_under_one_name_are_both_kept` and, through a
  seam that fails the link as a disk without hard links does, `without_hard_links_an_id_there_first_is_kept`
  (PB-168).
- The app: `opening_a_file_twice_never_loses_a_saved_change`, `a_file_is_opened_once_by_any_path_to_it`,
  `a_file_asking_for_its_password_is_opened_once_by_any_path_to_it`, `nothing_is_saved_over_another_tabs_file`,
  `saving_never_silently_replaces_a_file_changed_since_it_was_read` and `save_asks_before_replacing_a_changed_file`;
  the claims in `app/tests/destinations.rs` (`every_writer_is_refused_a_file_being_saved` and the rest); saves and
  signing at the boundary in `app/tests/writers.rs`; `remembered_pictures_are_copied_and_deleted_with_their_entry`
  for the picture's modes. The folder of digital IDs: `no_writer_writes_in_the_id_folder` in
  `app/tests/destinations.rs`, every writer aimed there (PB-170). The exports:
  `id_exports_and_the_tabs_files_keep_apart` in `app/tests/destinations.rs`, and `digital.rs`'s
  `an_export_never_goes_in_the_id_stores_folder`, `an_export_is_refused_a_file_the_app_holds` and
  `an_id_exports_file_is_handed_to_the_app`.
- The command line: `input_kept.rs` (`a_hard_link_to_the_input_is_refused`,
  `a_generated_file_hard_linked_to_the_input_is_refused`, `a_file_read_besides_the_input_is_refused_as_the_output`)
  and `verify.rs`'s `a_hard_link_to_the_input_leaves_it_unchanged`.

**Limits.**

- There is no locking: on Linux and macOS a lock binds only the programs that ask for it, so LimarePDF can't stop
  another program writing. Two cases follow, accepted for leaving the freeze (D-15 (d)). A change another program
  makes in the moment between the last check and the rename is replaced. A program that keeps the old file open
  and writes into it after the rename writes into a file no longer at that name, so its change is lost. Either way
  the file at the name is whole: one version or the other, never a mix.
- The app's claims ignore case on every system, so on a disk that keeps case two names that differ only in case
  are refused to each other while one is written: a refusal that loses nothing.
- Not claimed in the app: the recovery store's private folder, and files other apps write.
- Where a disk has no hard links (some network and FAT drives), a digital ID's new file is put in place by a check
  and then a rename, which leaves a moment for another store to take the name.
- A file Windows won't open, even only to ask its ID, is known by its path alone.

**Owner.** Core `disk::identity` and `Identity::matches` (`disk::same_file` is the two together); core
`atomic::replace` with the version it expects (compared by `disk::compare_bytes`) and `atomic::create_new`, which
never replaces; core `write_private`. The app's `app/destinations.rs`: `Destinations` for the claims, and
`App::conflict`, `export_conflict`, `tab_of`, `reading` and `in_other_tab` for open tabs, and the folder of
digital IDs through `digital::in_id_folder`. The command line's `not_an_input` and `not_taken`
(`limarepdf-cli/src/main.rs`).

## Failure atomicity

**What it promises.**

- An operation that changes a document either finishes or leaves it as it was: refused, failing partway or
  panicking, it leaves every object, the trailer and the objects' numbers as they were before it started, never a
  half-made change for a save, Undo or the next edit to find (WP-02). This holds for the core's public edits:
  text, paragraphs, new text and pictures, stamps, annotations and their styles, links, bookmarks, pages inserted
  or appended, form values, hidden content removed, and redaction.
- Redacting is one operation with its check afterwards. When the check finds something of what was marked still in
  the file, `redact` and `apply_redactions` refuse with a `Leftovers` error carrying the report, and nothing is
  changed. No redaction keeps what the check found.
- In the app, an edit holds the document: the tab keeps its own copy, to put back on failure and to make the undo
  step, so the operation under it doesn't copy the document a second time. Replace all holds the copy it writes
  on: a change that fails there is written again on a new copy without it, so the document never takes a
  half-made one.

**Checked by.**

- Core: `atomicity.rs`, every case judged by the state oracle (`state_before` and `state_since`, each object and
  the trailer compared): the operations refused before they change anything, the redactions, stamps, text and
  paragraph edits refused late, and the survey's late failures (`annotation_changes_failing_late_change_nothing`,
  `bookmark_changes_failing_late_change_nothing`, `new_content_failing_late_changes_nothing`,
  `link_changes_failing_late_change_nothing`). Before S2-02, six of its tests and all 17 of the survey's cases
  failed. The bookmark cases fail at their last step, setting the outline's counts, through `outline_seam`.
- Core, besides: `redact.rs`'s `refused` and `all_refused` cases; `too_complex.rs`'s
  `a_redaction_over_a_page_too_complex_to_read_changes_nothing`; `sanitise.rs`'s
  `content_outside_the_crop_box_that_redaction_leaves_is_refused`; the form tests that end `changes_nothing` in
  `form_fill.rs`; `form_flatten.rs`'s and `many_annotations.rs`'s refused flattenings; `history.rs`'s
  `a_failed_edit_goes_back_as_it_was`.
- The app: `applying_what_the_tag_tree_would_keep_changes_nothing` and
  `content_outside_the_crop_box_redaction_leaves_is_refused_with_nothing_changed`; Replace all's
  `a_change_failing_as_it_is_written_leaves_nothing` (`editor/replace_all.rs`).
- The command line: `redact.rs`'s `redact_refuses_what_the_tag_tree_would_keep_and_writes_nothing`.

**Limits.**

- Only the document is put back. Nothing an operation does changes what was read when it was opened (its
  encryption, that it was repaired, that it reads differently in other apps), so that isn't copied.
- The highest object number isn't lowered: a number an object had for the failed operation isn't given to another
  in the session, as after Undo.
- Outside the app (the command line, the core's own callers), each operation copies the document before it starts,
  as the app's edit did already: on a large document an edit takes that copy's time and memory.

**Owner.** Core `Pdf::atomically` (`history.rs`), which every operation above goes through, and `Pdf::holding`, by
which a caller that keeps its own copy says so; the copy is `Pdf::before`, put back by `Pdf::go_back`. Core
`redact::Leftovers`. The app's `Tab::edit_inner` (`tab.rs`), which holds the document, and Replace all's `write`
and `settle` (`editor/replace_all.rs`), which hold its copy.

## Redaction

**What it promises.**

- What the marks cover goes from the file, not only from view: each character at least a quarter under the marks
  taken together, the drawings mostly under them (a line or fill crossing an edge is cut out of the marks instead),
  a picture's pixels under them (a picture LimarePDF can't read goes whole), alternative text on what lost
  anything and on every tag above it, and the comments, links and form fields touching the marks, with their
  replies, values, appearances and what they held. What is outside the marks stays as it was, the characters left
  in their places.
- Nothing redaction removed stays reachable in the saved file. Redaction reads the redacted pages again: no text is
  left under the marks, and every picture there is blacked out under them. Then it follows the file from its
  trailer as saving keeps it, looking for what it removed: the originals replaced or removed (a picture's with its
  soft mask, mask and alternate versions), the patterns and soft masks only removed content painted with, the
  pages' old content, thumbnails and private data, the alternative text taken off, and the removed annotations
  with their appearances and what they held. Every list of resources, shared or not, stops listing what nothing
  draws, and a removed comment's appearance leaves the document's named appearances. Anything still reached,
  through the tag tree, private application data or another part of the file, or that a drawing which can't be
  read may use, makes redaction refuse with `Leftovers` naming it, and nothing is changed
  ([Failure atomicity](#failure-atomicity)).
- What content left as it was still draws (an original outside the marks, on another page, in a pattern, a soft
  mask, a Type 3 glyph or a comment that stays) is kept for it, and the report names each page that draws it,
  once.
- Redaction refuses before changing anything when it can't follow all it would have to remove: a form field
  touching a mark in a form nested more than 32 deep; a parent tree nested more than 32 deep or past 100,000
  parts; a page too complex to read; or drawings too complex to follow. The walk over the drawings has a budget,
  `reach::STEPS`: 2²² (4,194,304) steps, 32 times the most any of the 1,080 corpus files that open takes
  (129,066, `pdfjs/issue6692.pdf`, measured 2026-11-04 with the budget lifted). It runs before the change; the
  walk after it is allowed twice that, plus the objects added.
- Flattening isn't held to this: it draws what it removes into the page, so what notes and fields show stays by
  design.

**Checked by.**

- Core `redact.rs`: what goes, kind by kind (`removes_the_marked_characters_and_keeps_the_rest_in_place`,
  `a_character_goes_when_a_quarter_of_it_is_under_the_marks`,
  `removes_drawings_under_a_mark_and_clips_those_crossing_it`, `blacks_out_the_covered_part_of_a_picture`,
  `removes_annotations_touching_a_mark`, `alternative_text_far_up_the_structure_tree_goes` and the rest); the
  saved bytes (`the_saved_output_holds_nothing_of_the_marked_text_or_what_touches_it`,
  `an_earlier_revision_of_the_file_leaves_nothing_in_the_redacted_output`); each route refused
  (`a_redacted_form_the_tag_tree_points_at_is_refused`, `a_removed_link_the_tag_tree_points_at_is_refused`,
  `a_redacted_form_another_page_s_private_data_points_at_is_refused`,
  `removed_content_another_part_of_the_file_points_at_is_refused`,
  `a_redacted_form_a_drawing_that_can_t_be_read_lists_is_refused`,
  `what_a_removed_comment_held_that_something_else_points_at_is_refused`,
  `removed_alternative_text_something_else_points_at_is_refused`,
  `a_blacked_out_picture_s_alternate_version_or_soft_mask_something_else_points_at_is_refused`); what stays for
  content left as it was (the tests ending `and_reported`, and
  `a_redacted_form_another_page_draws_is_kept_for_it`) and what goes when nothing draws it (those ending
  `leaves_no_original`); the limits (`drawings_too_complex_to_follow_are_refused_with_nothing_changed`,
  `a_parent_tree_nested_too_deep_is_refused_unchanged`, `a_parent_tree_too_large_to_search_is_refused_unchanged`,
  `a_widget_under_a_mark_in_a_form_nested_too_deep_is_refused_unchanged`) and `too_complex.rs`'s
  `a_redaction_over_a_page_too_complex_to_read_changes_nothing`.
- Core, by property: `destructive.rs`'s `redaction_removes_what_is_under_its_marks_and_keeps_the_rest`, and
  `flattening_notes_keeps_what_they_show` and `flattening_the_form_keeps_what_its_widgets_show` for flattening.
- Other readers: the `cross_check_` tests in `redact.rs` read the saved file with poppler and Ghostscript (run on
  request, not by default).
- The app: `applying_what_the_tag_tree_would_keep_changes_nothing`. The command line: `redact.rs`'s
  `redact_refuses_what_the_tag_tree_would_keep_and_writes_nothing` and
  `redact_warns_where_an_original_picture_is_still_drawn`.

**Limits.**

- A route still reaching something removed is refused, not cut: a tagged document whose tags point at a removed
  link or comment can't be redacted there (PF-73, after the freeze).
- What [Redaction](../redaction.md#limits) lists: the gap shows how long the removed text was, embedded fonts
  keep the shapes of the letters they hold, vertical writing and tiling patterns aren't redacted, a blacked-out
  picture keeps its own XMP metadata, and the removed text found elsewhere (other pages, bookmarks, comments,
  fields, the document's properties) is reported, not removed.
- The walk after the change can't fail on a document the walk before passed: argued, not proven. Should it, the
  change is undone as any failure is.
- Deleting and extracting pages aren't held to this yet (PB-137): a removed page's own objects can stay reachable
  through other parts of the file.

**Owner.** Core `Pdf::redact` and `Pdf::apply_redactions` (`redact.rs`), with `redact::Leftovers` and
`RedactReport`; the pictures in `redact_img.rs`; the walk over the drawings, `Pdf::reach` with `reach::STEPS`
(`reach.rs`, charged to a `budget::Work`); the check afterwards, `Pdf::clear_retained` with `retained::Removed`
and its `routes_to` (`retained.rs`). What the check follows is in [How it is built](architecture.md).

## Sanitise

**What it promises.**

- Remove hidden information (`Pdf::remove_hidden`, `limarepdf-cli sanitise`) removes each kind it is given
  everywhere the file holds it, or refuses and changes nothing: document information and every XMP stream;
  scripts and actions that run by themselves at any depth (in a chain of next actions, written in or stored
  apart), leaving ordinary links; attached files; the content of hidden layers and what layer rules hide,
  wherever it is drawn; thumbnails and private application data; comments, with their replies; every form
  field's value, also those taken from a parent, defaults, rich text and the drawings showing them; bookmarks;
  and content outside the crop box, which is redacted, with redaction's check.
- Removing hidden layers doesn't change what shows: what hidden content set (a colour, a position, a clip) still
  applies after it, and hidden text leaves a gap as wide. Where removing it would change what shows, it refuses.
- What a removal leaves undrawn (the pictures and forms only hidden content drew, the patterns and soft masks only
  it painted with) and the annotations it takes (comments, those holding attached files, those in a hidden layer),
  with their appearances and what they held, are held to redaction's rule: nothing lists them, and a route still
  reaching one, through the tag tree, private application data or another part of the file, refuses (PB-153,
  PB-182). A removed comment's appearance leaves the document's named appearances.
- It refuses, with nothing changed, when it can't tell what is hidden: a drawing or layer rule it can't read, a
  rule nested more than 32 deep or past 2²⁰ steps (`RULE_STEPS`), drawings too complex to follow (redaction's
  budget), compressed data cut off or damaged part way, a drawing without resources of its own used where its
  layers mean different things, and form fields nested more than 32 deep. `Pdf::check_remove_hidden` finds these
  before anything else is done; what the check afterwards finds is found only by removing.
- Saving rewrites the whole file, without the objects nothing uses and without earlier versions of it.

**Checked by.**

- Core `sanitise.rs`: a test for each kind (`removes_metadata_and_xmp`, `removes_scripts_but_keeps_web_links`,
  `scripts_in_nested_direct_actions_are_removed`, `scripts_in_stored_lists_of_next_actions_are_removed`,
  `removes_attachments`, `removes_the_content_of_hidden_layers`, `form_values_are_cleared_everywhere_they_are_kept`,
  `cuts_what_lies_outside_the_crop_box`); what shows (`removing_hidden_layers_keeps_what_shows`,
  `hidden_layers_whose_removal_would_change_what_shows_are_refused`); held to redaction's rule
  (`what_only_hidden_layers_draw_is_removed`, `what_only_hidden_layers_draw_that_the_tag_tree_keeps_is_refused`,
  `a_comment_the_tag_tree_points_at_is_refused`, `a_removed_comments_named_icon_goes_with_it`,
  `content_outside_the_crop_box_that_redaction_leaves_is_refused`); the refusals (the other tests ending
  `refused`, and `checking_layer_rules_has_a_step_limit`).
- Core, by property: `destructive.rs`'s `removing_hidden_layers_removes_only_what_they_alone_hold`,
  `removing_scripts_removes_every_script_and_keeps_the_rest`,
  `removing_form_values_removes_every_value_and_keeps_the_rest` and
  `removing_comments_removes_every_note_and_keeps_the_rest`.
- Other readers: `sanitise.rs`'s `cross_check_` tests, with poppler and Ghostscript (run on request).
- The app: `app/tests/hidden.rs`. The command line: `sanitise.rs`'s
  `content_outside_the_crop_box_redaction_leaves_is_refused`.

**Limits.**

- As for redaction, a route still reaching something removed is refused, not cut (PF-73).
- Data in the older compressions (LZW, run length) is checked only as far as lopdf reads it. In a file where
  nothing is hidden, a damaged drawing doesn't stop the command, and a layer rule inside it is missed.
- A form only for Adobe Reader (XFA only) keeps its XFA; signature fields keep their values.
- Counting what there is (`Pdf::find_hidden`, on a copy) doesn't refuse: when the walk over the drawings fails
  there, the pictures stay counted.

**Owner.** Core `Pdf::remove_hidden`, `Pdf::check_remove_hidden` and `Pdf::find_hidden` (`sanitise.rs`), with
`drop_hidden_layers` and `scan_layers` for hidden layers and `refuse_left` for the check afterwards, which is
redaction's `Pdf::clear_retained`. The app's Remove hidden information dialog (`dialogs::remove_hidden`).

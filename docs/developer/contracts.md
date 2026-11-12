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
- The Digital IDs window exports nothing into the ID store's folder, by any path or link to it, where it would take
  the place of an ID or the note of the default (PB-167).
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
  `an_expected_file_is_replaced`; `ids.rs`'s `two_ids_stored_at_once_under_one_name_are_both_kept`.
- The app: `opening_a_file_twice_never_loses_a_saved_change`, `a_file_is_opened_once_by_any_path_to_it`,
  `a_file_asking_for_its_password_is_opened_once_by_any_path_to_it`, `nothing_is_saved_over_another_tabs_file`,
  `saving_never_silently_replaces_a_file_changed_since_it_was_read` and `save_asks_before_replacing_a_changed_file`;
  the claims in `app/tests/destinations.rs` (`every_writer_is_refused_a_file_being_saved` and the rest); saves and
  signing at the boundary in `app/tests/writers.rs`; `remembered_pictures_are_copied_and_deleted_with_their_entry`
  for the picture's modes. The exports: `id_exports_and_the_tabs_files_keep_apart` in
  `app/tests/destinations.rs`, and `digital.rs`'s `an_export_never_goes_in_the_id_stores_folder`,
  `an_export_is_refused_a_file_the_app_holds` and `an_id_exports_file_is_handed_to_the_app`.
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
- Not claimed in the app: the recovery store's private folder, and files other apps write. Only the exports are
  kept out of the ID store's folder; the app's other writers aren't.
- Where a disk has no hard links (some network and FAT drives), a digital ID's new file is put in place by a check
  and then a rename, which leaves a moment for another store to take the name.
- A file Windows won't open, even only to ask its ID, is known by its path alone.

**Owner.** Core `disk::identity` and `Identity::matches` (`disk::same_file` is the two together); core
`atomic::replace` with the version it expects (compared by `disk::compare_bytes`) and `atomic::create_new`, which
never replaces; core `write_private`. The app's `app/destinations.rs`: `Destinations` for the claims, and
`App::conflict`, `export_conflict`, `tab_of`, `reading` and `in_other_tab` for open tabs; `IdsWindow::refused` in
`digital.rs` for the ID store's folder. The command line's `not_an_input` and `not_taken`
(`limarepdf-cli/src/main.rs`).

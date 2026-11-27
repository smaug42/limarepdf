//! The CLI's integration tests as one test program: each file beside this one is a module of
//! it, so they build and link once, not once a file. A test is named by its file, then its name
//! (`verify::the_input_itself_is_refused`). A new test file is added here, or it
//! never runs: the lint job's tools/check-test-programs.py fails until it is.

mod annotations;
mod bookmarks;
mod escapes;
mod flatten;
mod form;
mod input_kept;
mod merge;
mod no_replace;
mod not_confirmed;
mod notes;
mod objects;
mod ocr;
mod output;
mod panics;
mod papers_win;
mod print;
mod redact;
mod reduce;
mod render;
mod resolutions;
mod sanitise;
mod save_images;
mod search;
mod sign_picture;
mod unwritable_settings;
mod verify;

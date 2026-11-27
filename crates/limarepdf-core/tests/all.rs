//! Core's integration tests as one test program: each file beside this one is a module of it,
//! so they build and link once, not once a file. A test is named by its file, then its name
//! (`redact::removes_characters_of_a_shift_jis_font_whole`). The files setting their own
//! allocator are programs of their own (Cargo.toml). A new test file is added here, or it never
//! runs: the lint job's tools/check-test-programs.py fails until it is.

mod common;
mod sigkit;

mod actual_text;
mod annots;
mod atomicity;
mod blank;
mod compact;
mod decoration;
mod destructive;
mod digital_ids;
mod edit;
mod excerpt;
mod export;
mod font_scan;
mod form_fill;
mod form_flatten;
mod forms;
mod harness;
mod history;
mod import;
mod integration;
mod labels;
mod line_styles;
mod links;
mod local_dates;
mod many_annotations;
mod many_fields;
mod merge;
mod mesh_fills;
mod ocr;
mod outline;
mod own_encoding;
mod own_font;
mod page_colours;
mod page_walks;
mod pages_removed;
mod papers;
mod paragraph;
mod paragraph_check;
mod patterns;
mod pictures;
mod print;
mod redact;
mod reduce;
mod render_compare;
mod render_coverage;
mod render_region;
mod repair;
mod sanitise;
mod save;
mod save_reopen;
mod saved_checker;
mod scan;
mod search;
mod signature_changes;
mod signature_trust;
mod signatures;
mod signing;
mod spelling;
mod stand_in;
mod state;
mod text_pipeline;
mod text_strings;
mod too_complex;
mod unfillable;
mod unicode_text;
mod viewprefs;

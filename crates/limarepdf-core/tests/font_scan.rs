//! Typing never waits for the installed fonts to be read (the text design, 9): text a font
//! has is set in it without reading the list of installed fonts, which the app reads once,
//! at start, on another thread. Checked in a child process, so no other test has read the
//! list before it.

use std::time::Duration;

use crate::common::apart::{self, Ran};
use limarepdf_core::{fallbacks, set_in, typefaces_ready, writable, FontChoice, SetIn};

/// The child process of the next test: sets text in the default font, then says whether the
/// installed fonts were read.
#[test]
#[ignore = "started by text_a_font_has_is_set_without_reading_the_installed_fonts"]
fn font_scan_child() {
    if std::env::var("FONT_SCAN_CHILD").is_err() {
        return;
    }
    let f = FontChoice::default();
    let text = "Walrus ate fish, café";
    assert!(text.chars().all(|c| writable(f, c)));
    let n = text.chars().count();
    assert_eq!(set_in(f, text, None), [((0, n), Some(SetIn::Standard(f)))]);
    assert!(fallbacks(f, text).is_empty());
    println!("scan|installed fonts read: {}", typefaces_ready());
}

#[test]
fn text_a_font_has_is_set_without_reading_the_installed_fonts() {
    let env = [("FONT_SCAN_CHILD", "1".to_string())];
    let ran = apart::run(
        module_path!(),
        "font_scan_child",
        &env,
        Duration::from_secs(30),
    );
    let Ran::Ended(text, status) = ran else {
        panic!("not done within 30 s");
    };
    let said = apart::reports(&text, "scan|");
    assert_eq!(
        said,
        ["installed fonts read: false"],
        "{}",
        apart::why_stopped(status)
    );
}

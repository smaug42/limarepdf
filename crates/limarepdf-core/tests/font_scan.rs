//! Typing never waits for the installed fonts to be read (the text design, 9): text a font
//! has is set in it without reading the list of installed fonts, which the app reads once,
//! at start, on another thread. A test file of its own, so no other test has read the list
//! before it.

use limarepdf_core::{fallbacks, set_in, typefaces_ready, writable, FontChoice};

#[test]
fn text_a_font_has_is_set_without_reading_the_installed_fonts() {
    let f = FontChoice::default();
    let text = "Walrus ate fish, café";
    assert!(text.chars().all(|c| writable(f, c)));
    let n = text.chars().count();
    assert_eq!(set_in(f, text, None), [((0, n), Some(f))]);
    assert!(fallbacks(f, text).is_empty());
    assert!(!typefaces_ready(), "the installed fonts were read");
}

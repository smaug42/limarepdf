//! Fuzzed bytes as a page's content stream, read as the app reads a page: its text, its
//! editable objects and its paragraphs.
#![no_main]

mod page;

use libfuzzer_sys::fuzz_target;
use limarepdf_core::Pdf;

fuzz_target!(|data: &[u8]| {
    let Ok(pdf) = Pdf::from_bytes(&page::around(data), None) else {
        return;
    };
    let _ = pdf.page_text(0);
    let _ = pdf.page_objects(0);
    let _ = pdf.page_blocks(0);
});

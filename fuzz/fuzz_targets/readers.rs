//! Any bytes opened as a PDF, then read as the app's panels read it: comments and other
//! annotations, form fields, bookmarks, and the first pages' text.
#![no_main]

use libfuzzer_sys::fuzz_target;
use limarepdf_core::Pdf;

fuzz_target!(|data: &[u8]| {
    let Ok(pdf) = Pdf::from_bytes(data, None) else {
        return;
    };
    let _ = pdf.annotations();
    let _ = pdf.form_fields();
    let _ = pdf.outline();
    for i in 0..pdf.page_count().min(3) {
        let _ = pdf.page_text(i);
    }
});

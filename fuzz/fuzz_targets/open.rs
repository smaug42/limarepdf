//! Any bytes opened as a PDF (repair included), and a document that opens saved again.
#![no_main]

use libfuzzer_sys::fuzz_target;
use limarepdf_core::Pdf;

fuzz_target!(|data: &[u8]| {
    if let Ok(mut pdf) = Pdf::from_bytes(data, None) {
        let _ = pdf.page_count();
        let _ = pdf.to_bytes();
    }
});

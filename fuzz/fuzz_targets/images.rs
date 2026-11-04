//! Any bytes imported as a picture page (PNG, JPEG, TIFF), as Make a PDF from images does.
#![no_main]

use libfuzzer_sys::fuzz_target;
use limarepdf_core::{ImageImport, Pdf};

fuzz_target!(|data: &[u8]| {
    if let Ok(mut pdf) = Pdf::from_images(&[data.to_vec()], &ImageImport::default()) {
        let _ = pdf.to_bytes();
    }
});

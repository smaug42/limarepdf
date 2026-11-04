//! Fuzzed bytes as a page's content stream, drawn as the app draws a page (its view of the
//! file, then the renderer), at half size on the 200-point page.
#![no_main]

mod page;

use libfuzzer_sys::fuzz_target;
use limarepdf_core::render::{self, Renderer};
use limarepdf_core::Pdf;

fuzz_target!(|data: &[u8]| {
    let Ok(mut pdf) = Pdf::from_bytes(&page::around(data), None) else {
        return;
    };
    let Ok(bytes) = pdf.render_snapshot() else {
        return;
    };
    let Ok(hp) = render::load(bytes, None) else {
        return;
    };
    let _ = Renderer::new(&hp).render(0, 0.5);
});

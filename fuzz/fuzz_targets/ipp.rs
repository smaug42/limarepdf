//! Any bytes read as the printing service's answer (IPP), and the paper sizes and margins in it
//! read as printing reads them.
#![no_main]

use libfuzzer_sys::fuzz_target;
use limarepdf_core::fuzz_seam;

fuzz_target!(|data: &[u8]| {
    fuzz_seam::printer_answer(data);
});

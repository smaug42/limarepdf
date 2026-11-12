//! Any text read as a trust list (`trust.txt`), saved and read again: the same certificates are
//! trusted, as the same signers and authorities, before and after.
#![no_main]

use libfuzzer_sys::fuzz_target;
use limarepdf_core::TrustList;

fuzz_target!(|data: &[u8]| {
    let Ok(text) = std::str::from_utf8(data) else {
        return;
    };
    let read = TrustList::parse(text);
    let again = TrustList::parse(&read.to_text());
    assert_eq!(read.entries, again.entries);
});

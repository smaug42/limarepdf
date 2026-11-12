//! Any bytes opened as a PDF and its signatures checked as the app checks them, then checked
//! against a trust list of the repository's test certificates.
#![no_main]

use std::sync::LazyLock;

use libfuzzer_sys::fuzz_target;
use limarepdf_core::{verify_signatures, TrustList, TrustRole};

macro_rules! key {
    ($file:literal) => {
        include_bytes!(concat!(
            "../../crates/limarepdf-core/tests/data/signatures/keys/",
            $file
        ))
    };
}

static TRUST: LazyLock<TrustList> = LazyLock::new(|| {
    let mut list = TrustList::default();
    let certificates: [(&[u8], TrustRole); 3] = [
        (key!("ca.crt.der"), TrustRole::Authority),
        (key!("t-root.crt.der"), TrustRole::Authority),
        (key!("t-leaf.crt.der"), TrustRole::Signer),
    ];
    for (der, role) in certificates {
        list.add(der, role, None)
            .expect("a test certificate is trusted");
    }
    list
});

fuzz_target!(|data: &[u8]| {
    if let Ok(mut report) = verify_signatures(data, None, &TrustList::default()) {
        report.apply_trust(&TRUST);
    }
});

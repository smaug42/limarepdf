//! Any bytes read as a digital ID file (PKCS#12), with the password the repository's test IDs
//! take, under a lower key-derivation budget (`cfg(fuzzing)`). The integrity check comes before
//! anything encrypted is read, so what lies past it is reached by the test IDs as they are, not
//! by changes to them.
#![no_main]

use libfuzzer_sys::fuzz_target;
use limarepdf_core::fuzz_seam;

/// The test IDs' passwords (core's tests/digital_ids.rs); the first byte picks one.
const PASSWORDS: [&str; 3] = ["mpe-test-password", "pässwörd-тест", ""];

fuzz_target!(|data: &[u8]| {
    let Some((&pick, file)) = data.split_first() else {
        return;
    };
    fuzz_seam::digital_id(file, PASSWORDS[usize::from(pick) % PASSWORDS.len()]);
});

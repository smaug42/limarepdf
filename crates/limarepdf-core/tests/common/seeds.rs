//! The inputs the audit's regression tests build, kept as fuzzing seeds (O-04). When
//! `LIMAREPDF_FUZZ_SEEDS` names a folder, each is written to `<folder>/<target>/<test>-<digest>`,
//! `<target>` being the fuzz target that reads it and `<test>` the test that built it; otherwise
//! nothing is written. `fuzz/seed.sh` runs these tests so. Each input is still made in its test
//! alone: the seeds are copies of it, never a second way of making it.

use sha2::{Digest, Sha256};
use std::path::PathBuf;

/// Inputs longer than the fuzz runs' `-max_len` (fuzz/README.md) are left out.
const MAX_LEN: usize = 262_144;

fn folder() -> Option<PathBuf> {
    std::env::var_os("LIMAREPDF_FUZZ_SEEDS").map(PathBuf::from)
}

/// Keeps `bytes` as a seed for the fuzz target `target`.
pub fn seed(target: &str, bytes: &[u8]) {
    let Some(dir) = folder() else {
        return;
    };
    if bytes.len() > MAX_LEN {
        return;
    }
    let dir = dir.join(target);
    // The test's thread is named after it.
    let test = std::thread::current()
        .name()
        .unwrap_or("seed")
        .replace("::", "-");
    let digest: String = Sha256::digest(bytes)[..8]
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    std::fs::create_dir_all(&dir)
        .and_then(|_| std::fs::write(dir.join(format!("{test}-{digest}")), bytes))
        .unwrap_or_else(|e| panic!("can't keep a seed in {}: {e}", dir.display()));
}

/// Keeps a document as a seed for the targets that open one, `open` and `readers`.
pub fn pdf_bytes(bytes: &[u8]) {
    seed("open", bytes);
    seed("readers", bytes);
}

/// [`pdf_bytes`] for a document built in the test, written as a copy so the test's own is
/// left as it was.
pub fn pdf(p: &limarepdf_core::Pdf) {
    if folder().is_some() {
        pdf_bytes(&p.clone().to_bytes().unwrap());
    }
}

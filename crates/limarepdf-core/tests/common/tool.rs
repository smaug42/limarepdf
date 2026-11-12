//! Outside programs the tests check LimarePDF's output with (poppler, Ghostscript, OpenSSL,
//! CUPS's tools). Each runs with a deadline, so one that hangs fails its test instead of
//! stopping the run, with its output read as it comes, so a full pipe can't stop it (core's
//! `child_seam`, which the app's and the CLI's tests use too). Its files go in a folder of the
//! test's own under Cargo's folder for test files.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};

// Each test file uses only some of them.
#[allow(unused_imports)]
pub use limarepdf_core::child_seam::{
    ghostscript_errors, output, output_with_input, output_within, TIMEOUT,
};

/// A folder of one test's own in `target/tmp/tools`, its name unique to the run and the call.
/// Removed when dropped, unless the test is failing: then it stays, to look at.
pub struct Scratch(PathBuf);

impl Scratch {
    pub fn new(name: &str) -> Scratch {
        static N: AtomicUsize = AtomicUsize::new(0);
        let n = N.fetch_add(1, Ordering::SeqCst);
        let dir = Path::new(env!("CARGO_TARGET_TMPDIR"))
            .join("tools")
            .join(format!("{name}-{}-{n}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        Scratch(dir)
    }

    pub fn path(&self) -> &Path {
        &self.0
    }

    pub fn join(&self, p: impl AsRef<Path>) -> PathBuf {
        self.0.join(p)
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        if !std::thread::panicking() {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
}

/// Whether `program` is installed: it runs with `arg` (its version option, say) in time.
pub fn installed(program: &str, arg: &str) -> bool {
    output(Command::new(program).arg(arg)).is_ok()
}

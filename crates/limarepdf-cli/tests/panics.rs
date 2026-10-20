//! Runs the built `limarepdf-cli` on a file that makes the renderer panic.

use std::path::Path;
use std::process::Command;

/// PDFBox 2363 makes hayro's renderer (vello_common) panic at 150 dpi. Opt-in: the corpus is fetched by
/// `crates/limarepdf-core/tools/fetch-render-samples.sh` and isn't in the repository.
#[test]
#[ignore = "needs the fetched render corpus"]
fn a_panic_ends_in_a_plain_error() {
    let file = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../limarepdf-core/tests/data/render/pdfbox/2363.pdf");
    assert!(file.exists(), "fetch the render corpus first");
    let out_dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join("panics-2363");
    let run = Command::new(env!("CARGO_BIN_EXE_limarepdf-cli"))
        .args(["render", "--dpi", "150", "--out-dir"])
        .arg(&out_dir)
        .arg(&file)
        .env_remove("RUST_BACKTRACE")
        .output()
        .unwrap();
    let err = String::from_utf8_lossy(&run.stderr);
    assert_eq!(run.status.code(), Some(1), "stderr: {err}");
    assert!(
        err.contains("LimarePDF hit an internal error"),
        "stderr: {err}"
    );
    assert!(!err.contains("panicked at"), "stderr: {err}");
}

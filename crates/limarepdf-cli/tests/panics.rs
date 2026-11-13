//! Runs the built `limarepdf-cli` on a file that makes the renderer panic, and on numbers that
//! once did.

use std::path::Path;
use std::process::Command;

/// Bates numbers past the largest LimarePDF can count to, and a rotation of almost the largest
/// number, get plain answers rather than an internal error.
#[test]
fn extreme_numbers_get_plain_answers() {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join("panics-numbers");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let mut p = limarepdf_core::Pdf::new();
    for i in 0..2 {
        p.insert_blank(i, 200.0, 200.0).unwrap();
    }
    std::fs::write(dir.join("in.pdf"), p.to_bytes().unwrap()).unwrap();
    let run = |args: &[&str]| {
        limarepdf_core::child_seam::output(
            Command::new(env!("CARGO_BIN_EXE_limarepdf-cli"))
                .current_dir(&dir)
                .args(args)
                .env_remove("RUST_BACKTRACE"),
        )
        .unwrap()
    };
    let max = u64::MAX.to_string();
    let bates = run(&["bates", "in.pdf", "-o", "bates.pdf", "--start", &max]);
    let err = String::from_utf8_lossy(&bates.stderr);
    assert_eq!(bates.status.code(), Some(1), "stderr: {err}");
    assert!(
        err.contains("the Bates numbers would go past 18446744073709551615"),
        "stderr: {err}"
    );
    assert!(!err.contains("internal error"), "stderr: {err}");
    assert!(!dir.join("bates.pdf").exists());

    // More digits than any Bates number needs (PB-01): past 65,535 the padding panicked.
    for digits in ["21", "70000"] {
        let bates = run(&["bates", "in.pdf", "-o", "bates.pdf", "--digits", digits]);
        let err = String::from_utf8_lossy(&bates.stderr);
        assert_eq!(bates.status.code(), Some(1), "{digits}: {err}");
        assert!(
            err.contains("Bates numbers can have at most 20 digits"),
            "{digits}: {err}"
        );
        assert!(!dir.join("bates.pdf").exists(), "{digits}");
    }
    let bates = run(&["bates", "in.pdf", "-o", "bates.pdf", "--digits", "20"]);
    let err = String::from_utf8_lossy(&bates.stderr);
    assert_eq!(bates.status.code(), Some(0), "stderr: {err}");
    let written = limarepdf_core::Pdf::open(dir.join("bates.pdf"), None).unwrap();
    assert!(written.page_text(1).contains("00000000000000000002"));

    let degrees = "9223372036854775800";
    let turn = ["rotate", "in.pdf", "-o", "turned.pdf", "--degrees", degrees];
    let rotate = run(&turn);
    let err = String::from_utf8_lossy(&rotate.stderr);
    assert_eq!(rotate.status.code(), Some(0), "stderr: {err}");
}

/// A panic while drawing, as hayro 0.8.0's renderer had on PDFBox 2363 at 150 dpi, ends in a plain
/// error. No file is known to make hayro 0.9.0 panic, so core's test seam makes drawing panic in the
/// program the tests build.
#[test]
fn a_panic_ends_in_a_plain_error() {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join("panics-draw");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let mut p = limarepdf_core::Pdf::new();
    p.insert_blank(0, 200.0, 200.0).unwrap();
    std::fs::write(dir.join("in.pdf"), p.to_bytes().unwrap()).unwrap();
    let run = limarepdf_core::child_seam::output(
        Command::new(env!("CARGO_BIN_EXE_limarepdf-cli"))
            .current_dir(&dir)
            .args(["render", "--dpi", "150", "--out-dir", "out", "in.pdf"])
            .env(limarepdf_core::render::panic_seam::VAR, "1")
            .env_remove("RUST_BACKTRACE"),
    )
    .unwrap();
    let err = String::from_utf8_lossy(&run.stderr);
    assert_eq!(run.status.code(), Some(1), "stderr: {err}");
    assert!(
        err.contains("LimarePDF hit an internal error"),
        "stderr: {err}"
    );
    assert!(!err.contains("panicked at"), "stderr: {err}");
}

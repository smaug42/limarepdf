//! A file written but not confirmed on disk (decision D7, REL-10): the folder sync fails,
//! through core's test seam (`LIMAREPDF_TEST_SYNC_FAILS`, in the program the tests build;
//! never in a release build). The file is in place, so the message never says nothing was
//! written: it says the file was saved but not confirmed, and to run the command again if
//! the power went; the status is a failure.

use limarepdf_core::{sync_seam, Pdf};
use std::path::{Path, PathBuf};
use std::process::Output;

fn folder(name: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join("not_confirmed")
        .join(name);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// A PDF of `pages` blank pages at `path`.
fn pdf(path: &Path, pages: usize) {
    let mut p = Pdf::new();
    for i in 0..pages {
        p.insert_blank(i, 300.0, 200.0).unwrap();
    }
    std::fs::write(path, p.to_bytes().unwrap()).unwrap();
}

/// Runs the command in `dir`, with folder syncs failing in `failing` if given.
fn run(dir: &Path, failing: Option<&Path>, args: &[&str]) -> Output {
    let mut cmd = std::process::Command::new(env!("CARGO_BIN_EXE_limarepdf-cli"));
    cmd.current_dir(dir)
        .env("HOME", dir)
        .env("XDG_CONFIG_HOME", dir.join("config"))
        .env_remove(sync_seam::VAR)
        .args(args);
    if let Some(f) = failing {
        cmd.env(sync_seam::VAR, f);
    }
    cmd.output().unwrap()
}

fn pages(path: &Path) -> usize {
    Pdf::from_bytes(&std::fs::read(path).unwrap(), None)
        .unwrap()
        .page_count()
}

/// One file written: saved but not confirmed, said so, a failed status; the file is the new
/// one. Without the failing sync the same command succeeds quietly about it.
#[test]
fn a_file_not_confirmed_is_said_saved() {
    let dir = folder("one");
    let out_dir = dir.join("out");
    std::fs::create_dir_all(&out_dir).unwrap();
    pdf(&dir.join("in.pdf"), 2);
    let args = [
        "rotate",
        "in.pdf",
        "--degrees",
        "90",
        "-o",
        "out/turned.pdf",
    ];
    let res = run(&dir, Some(&out_dir), &args);
    let err = String::from_utf8_lossy(&res.stderr);
    assert!(!res.status.success(), "succeeded: {err}");
    assert!(
        err.contains("turned.pdf was saved, but your system couldn't confirm it is safely on disk"),
        "{err}"
    );
    assert!(
        err.contains("If the computer lost power just now, run the command again."),
        "{err}"
    );
    assert!(!err.contains("nothing was written"), "{err}");
    assert_eq!(pages(&out_dir.join("turned.pdf")), 2);

    let res = run(&dir, None, &args);
    let err = String::from_utf8_lossy(&res.stderr);
    assert!(res.status.success(), "{err}");
    assert!(!err.contains("couldn't confirm"), "{err}");
}

/// `split` writes its parts on several threads: every part is written, and the message
/// names one part as saved but not confirmed, not as a failure part way.
#[test]
fn a_split_not_confirmed_writes_every_part() {
    let dir = folder("split");
    let out_dir = dir.join("parts");
    std::fs::create_dir_all(&out_dir).unwrap();
    pdf(&dir.join("in.pdf"), 5);
    let res = run(
        &dir,
        Some(&out_dir),
        &["split", "in.pdf", "--every", "1", "--out-dir", "parts"],
    );
    let err = String::from_utf8_lossy(&res.stderr);
    assert!(!res.status.success(), "succeeded: {err}");
    assert!(
        err.contains("was saved, but your system couldn't confirm it is safely on disk"),
        "{err}"
    );
    assert!(!err.contains("were written before"), "{err}");
    for n in 1..=5 {
        let part = out_dir.join(format!("in-{n:03}.pdf"));
        assert_eq!(pages(&part), 1, "part {n}");
    }
}

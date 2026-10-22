//! `limarepdf-cli verify --signed-version` never changes its input, whatever name the output
//! is given for it (audit R16).

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

const SIGNED: &str =
    "../limarepdf-core/tests/data/signatures/pyhanko/minimal-two-fields-signed-twice.pdf";

/// A fresh folder holding a copy of the signed fixture as `input.pdf`, and an empty trust list.
fn setup(name: &str) -> (PathBuf, PathBuf) {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join("verify")
        .join(name);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let input = dir.join("input.pdf");
    std::fs::copy(Path::new(env!("CARGO_MANIFEST_DIR")).join(SIGNED), &input).unwrap();
    std::fs::write(dir.join("trust.txt"), "").unwrap();
    (dir, input)
}

/// `verify input.pdf --signed-version 1 -o output`, run in `dir`.
fn signed_version(dir: &Path, input: &Path, output: &Path) -> Output {
    Command::new(env!("CARGO_BIN_EXE_limarepdf-cli"))
        .current_dir(dir)
        .arg("verify")
        .arg(input)
        .args(["--trust-file", "trust.txt", "--signed-version", "1", "-o"])
        .arg(output)
        .output()
        .unwrap()
}

/// Fails, without printing whole files, unless `path` holds exactly `before`.
fn unchanged(path: &Path, before: &[u8]) {
    let now = std::fs::read(path).unwrap();
    assert!(
        now == before,
        "{} changed: {} bytes, was {}",
        path.display(),
        now.len(),
        before.len()
    );
}

fn refused(out: &Output) {
    let err = String::from_utf8_lossy(&out.stderr);
    assert_eq!(out.status.code(), Some(1), "{err}");
    assert!(err.contains("must be saved to another file"), "{err}");
}

/// Saved to a file of its own, the signed version is exactly the first bytes of the input, as
/// many as the report says.
#[test]
fn the_signed_version_is_saved_to_another_file() {
    let (dir, input) = setup("other");
    let before = std::fs::read(&input).unwrap();
    let output = dir.join("signed.pdf");
    let out = signed_version(&dir, &input, &output);
    let said = String::from_utf8_lossy(&out.stdout);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let saved = std::fs::read(&output).unwrap();
    assert!(saved.len() < before.len());
    assert!(
        said.contains(&format!("({} of {} bytes)", saved.len(), before.len())),
        "{said}"
    );
    assert!(saved == before[..saved.len()]);
    unchanged(&input, &before);
}

/// The input's own name, given as it is or as another path to it, is refused with the input
/// unchanged.
#[test]
fn the_input_itself_is_refused() {
    let (dir, input) = setup("same");
    let before = std::fs::read(&input).unwrap();
    for output in [
        input.clone(),
        "input.pdf".into(),
        "../same/./input.pdf".into(),
    ] {
        refused(&signed_version(&dir, &input, &output));
        unchanged(&input, &before);
    }
}

/// A symbolic link to the input is refused with the input unchanged.
#[cfg(unix)]
#[test]
fn a_symbolic_link_to_the_input_is_refused() {
    let (dir, input) = setup("symlink");
    let before = std::fs::read(&input).unwrap();
    let link = dir.join("link.pdf");
    std::os::unix::fs::symlink(&input, &link).unwrap();
    refused(&signed_version(&dir, &input, &link));
    unchanged(&input, &before);
}

/// A hard link to the input leaves the input unchanged: on Unix it is refused, as the same
/// file; elsewhere the output replaces the link's name with a new file. Writing into it once
/// cut the input short.
#[test]
fn a_hard_link_to_the_input_leaves_it_unchanged() {
    let (dir, input) = setup("hardlink");
    let before = std::fs::read(&input).unwrap();
    let alias = dir.join("alias.pdf");
    std::fs::hard_link(&input, &alias).unwrap();
    let out = signed_version(&dir, &input, &alias);
    unchanged(&input, &before);
    if cfg!(unix) {
        refused(&out);
        unchanged(&alias, &before);
    }
}

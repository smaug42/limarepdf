//! Commands that write a document never change their input: an output naming the input, by any
//! path or link, is refused with nothing written.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

/// A one-page PDF at `dir/name`.
fn make_pdf(dir: &Path, name: &str) -> PathBuf {
    let objects = [
        "<< /Type /Catalog /Pages 2 0 R >>",
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>",
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 200 200] >>",
    ];
    let mut pdf = b"%PDF-1.7\n".to_vec();
    let mut offsets = Vec::new();
    for (i, o) in objects.iter().enumerate() {
        offsets.push(pdf.len());
        pdf.extend_from_slice(format!("{} 0 obj\n{o}\nendobj\n", i + 1).as_bytes());
    }
    let xref = pdf.len();
    pdf.extend_from_slice(b"xref\n0 4\n0000000000 65535 f \n");
    for o in offsets {
        pdf.extend_from_slice(format!("{o:010} 00000 n \n").as_bytes());
    }
    pdf.extend_from_slice(
        format!("trailer\n<< /Size 4 /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n").as_bytes(),
    );
    let path = dir.join(name);
    std::fs::write(&path, pdf).unwrap();
    path
}

fn folder(name: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join("input_kept")
        .join(name);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn run(dir: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_limarepdf-cli"))
        .current_dir(dir)
        .args(args)
        .output()
        .unwrap()
}

/// Each command, with `OUT` where its output goes; `in.pdf` and `b.pdf` are its inputs.
const COMMANDS: [&[&str]; 4] = [
    &[
        "rotate",
        "in.pdf",
        "-o",
        "OUT",
        "--pages",
        "1",
        "--degrees",
        "90",
    ],
    &["set-info", "in.pdf", "-o", "OUT", "--title", "Walrus"],
    &["sanitise", "in.pdf", "-o", "OUT"],
    &["merge", "b.pdf", "in.pdf", "-o", "OUT"],
];

/// Runs every command with `out` as its output: each is refused, and `in.pdf` keeps its bytes.
fn refused_for(dir: &Path, out: &str) {
    let input = dir.join("in.pdf");
    let before = std::fs::read(&input).unwrap();
    for cmd in COMMANDS {
        let args: Vec<&str> = cmd
            .iter()
            .map(|&a| if a == "OUT" { out } else { a })
            .collect();
        let res = run(dir, &args);
        let err = String::from_utf8_lossy(&res.stderr);
        assert_eq!(res.status.code(), Some(1), "{args:?}: {err}");
        assert!(
            err.contains("must be saved to another file"),
            "{args:?}: {err}"
        );
        assert!(
            std::fs::read(&input).unwrap() == before,
            "{args:?} changed the input"
        );
    }
}

/// The input's own name, as given or by another path to it, is refused.
#[test]
fn the_input_itself_is_refused() {
    let dir = folder("same");
    make_pdf(&dir, "in.pdf");
    make_pdf(&dir, "b.pdf");
    let full = dir.join("in.pdf");
    for out in [
        "in.pdf",
        "./in.pdf",
        "../same/in.pdf",
        full.to_str().unwrap(),
    ] {
        refused_for(&dir, out);
    }
}

/// A symbolic link to the input is refused.
#[cfg(unix)]
#[test]
fn a_symbolic_link_to_the_input_is_refused() {
    let dir = folder("symlink");
    make_pdf(&dir, "in.pdf");
    make_pdf(&dir, "b.pdf");
    std::os::unix::fs::symlink("in.pdf", dir.join("link.pdf")).unwrap();
    refused_for(&dir, "link.pdf");
}

/// A hard link to the input is refused on Unix, where it can be told.
#[cfg(unix)]
#[test]
fn a_hard_link_to_the_input_is_refused() {
    let dir = folder("hardlink");
    make_pdf(&dir, "in.pdf");
    make_pdf(&dir, "b.pdf");
    std::fs::hard_link(dir.join("in.pdf"), dir.join("alias.pdf")).unwrap();
    refused_for(&dir, "alias.pdf");
}

/// Another file as the output works, and the input keeps its bytes.
#[test]
fn another_output_works() {
    let dir = folder("other");
    make_pdf(&dir, "in.pdf");
    make_pdf(&dir, "b.pdf");
    let before = std::fs::read(dir.join("in.pdf")).unwrap();
    for cmd in COMMANDS {
        let args: Vec<&str> = cmd
            .iter()
            .map(|&a| if a == "OUT" { "out.pdf" } else { a })
            .collect();
        let res = run(&dir, &args);
        assert!(
            res.status.success(),
            "{args:?}: {}",
            String::from_utf8_lossy(&res.stderr)
        );
        assert!(dir.join("out.pdf").exists());
        assert!(std::fs::read(dir.join("in.pdf")).unwrap() == before);
        std::fs::remove_file(dir.join("out.pdf")).unwrap();
    }
}

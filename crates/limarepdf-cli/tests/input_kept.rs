//! Commands that write a document never change their input: an output naming the input, by any
//! path or link, is refused with nothing written. So is any of the files `split`, `render` and
//! `save-images` name after their input, even with `--overwrite` (A-02).

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
    limarepdf_core::child_seam::output(
        Command::new(env!("CARGO_BIN_EXE_limarepdf-cli"))
            .current_dir(dir)
            .args(args),
    )
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

/// `input.pdf`: two small pages, each showing a picture of its own. Returns its bytes.
fn pictures_pdf(dir: &Path) -> Vec<u8> {
    use limarepdf_core::render::{encode, Bitmap, ImageFormat};
    let mut p = limarepdf_core::Pdf::new();
    for (i, colour) in [[200, 20, 20, 255], [20, 20, 200, 255]].iter().enumerate() {
        let dot = Bitmap {
            width: 2,
            height: 2,
            rgba: colour.repeat(4),
        };
        let png = encode(&dot, ImageFormat::Png, 72.0).unwrap();
        p.insert_blank(i, 200.0, 200.0).unwrap();
        p.add_image(i, [50.0, 50.0, 150.0, 150.0], &png).unwrap();
    }
    let bytes = p.to_bytes().unwrap();
    std::fs::write(dir.join("input.pdf"), &bytes).unwrap();
    bytes
}

/// The commands that write files named after `input.pdf` into `out`, each with the first and a
/// later of its files.
const GENERATED: [(&[&str], [&str; 2]); 3] = [
    (
        &["split", "input.pdf", "--every", "1", "--out-dir", "out"],
        ["input-001.pdf", "input-002.pdf"],
    ),
    (
        &["render", "input.pdf", "--out-dir", "out"],
        ["input-001.png", "input-002.png"],
    ),
    (
        &["save-images", "input.pdf", "--out-dir", "out"],
        ["input-page-001-image-1.png", "input-page-002-image-1.png"],
    ),
];

#[cfg(unix)]
fn with_overwrite<'a>(cmd: &[&'a str], overwrite: bool) -> Vec<&'a str> {
    cmd.iter()
        .copied()
        .chain(overwrite.then_some("--overwrite"))
        .collect()
}

/// The names in `dir`, sorted.
fn names(dir: &Path) -> Vec<String> {
    let mut v: Vec<String> = std::fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    v.sort();
    v
}

/// `out`, empty.
#[cfg(unix)]
fn empty(out: &Path) {
    let _ = std::fs::remove_dir_all(out);
    std::fs::create_dir(out).unwrap();
}

/// Runs `GENERATED[k]` with its first or a later file a symbolic link to the input, relative
/// or absolute: refused, with or without `--overwrite`, with nothing written, the input keeping
/// its bytes and the link as it was.
#[cfg(unix)]
fn a_link_is_refused(k: usize, folder_name: &str) {
    let (cmd, files) = GENERATED[k];
    let dir = folder(folder_name);
    let before = pictures_pdf(&dir);
    let (out, full) = (dir.join("out"), dir.join("input.pdf"));
    for name in files {
        for target in [Path::new("../input.pdf"), full.as_path()] {
            for overwrite in [true, false] {
                empty(&out);
                std::os::unix::fs::symlink(target, out.join(name)).unwrap();
                let args = with_overwrite(cmd, overwrite);
                let case = format!("{args:?} with out/{name} -> {}", target.display());
                let res = run(&dir, &args);
                let err = String::from_utf8_lossy(&res.stderr);
                assert_eq!(res.status.code(), Some(1), "{case}: {err}");
                assert!(
                    err.contains("must be saved to another file"),
                    "{case}: {err}"
                );
                assert!(
                    std::fs::read(&full).unwrap() == before,
                    "{case} changed the input"
                );
                assert_eq!(names(&out), [name], "{case} wrote a file");
                assert_eq!(
                    std::fs::read_link(out.join(name)).unwrap(),
                    target,
                    "{case}"
                );
            }
        }
    }
}

#[cfg(unix)]
#[test]
fn split_refuses_a_part_linked_to_the_input() {
    a_link_is_refused(0, "split-link");
}

#[cfg(unix)]
#[test]
fn render_refuses_an_image_linked_to_the_input() {
    a_link_is_refused(1, "render-link");
}

#[cfg(unix)]
#[test]
fn save_images_refuses_a_picture_linked_to_the_input() {
    a_link_is_refused(2, "save-images-link");
}

/// A generated file that is a hard link to the input is refused as `-o` is, though replacing it
/// would leave the input as it was: one rule for every file a command writes.
#[cfg(unix)]
#[test]
fn a_generated_file_hard_linked_to_the_input_is_refused() {
    use std::os::unix::fs::MetadataExt;
    let dir = folder("generated-hardlink");
    let before = pictures_pdf(&dir);
    let (out, full) = (dir.join("out"), dir.join("input.pdf"));
    let input = std::fs::metadata(&full).unwrap().ino();
    for (cmd, files) in GENERATED {
        for name in files {
            empty(&out);
            std::fs::hard_link(&full, out.join(name)).unwrap();
            let args = with_overwrite(cmd, true);
            let case = format!("{args:?} with out/{name} the input's file");
            let res = run(&dir, &args);
            let err = String::from_utf8_lossy(&res.stderr);
            assert_eq!(res.status.code(), Some(1), "{case}: {err}");
            assert!(
                err.contains("must be saved to another file"),
                "{case}: {err}"
            );
            assert!(
                std::fs::read(&full).unwrap() == before,
                "{case} changed the input"
            );
            assert_eq!(names(&out), [name], "{case} wrote a file");
            assert_eq!(
                std::fs::metadata(out.join(name)).unwrap().ino(),
                input,
                "{case}"
            );
        }
    }
}

/// With nothing at their names, the commands write all their files, and the input keeps its
/// bytes.
#[test]
fn generated_files_are_written_and_the_input_kept() {
    let dir = folder("generated");
    let before = pictures_pdf(&dir);
    let out = dir.join("out");
    for (cmd, files) in GENERATED {
        let _ = std::fs::remove_dir_all(&out);
        let res = run(&dir, cmd);
        let err = String::from_utf8_lossy(&res.stderr);
        assert!(res.status.success(), "{cmd:?}: {err}");
        assert_eq!(names(&out), files, "{cmd:?}");
        assert!(
            std::fs::read(dir.join("input.pdf")).unwrap() == before,
            "{cmd:?} changed the input"
        );
    }
}

/// With `--overwrite`, a file at a generated name is replaced and a link to another file is
/// written through, as before: only the input is refused.
#[cfg(unix)]
#[test]
fn overwrite_still_replaces_other_files() {
    let dir = folder("generated-overwrite");
    let before = pictures_pdf(&dir);
    let (out, other) = (dir.join("out"), dir.join("other"));
    for (cmd, [first, later]) in GENERATED {
        empty(&out);
        std::fs::write(&other, "another file").unwrap();
        std::os::unix::fs::symlink("../other", out.join(first)).unwrap();
        std::fs::write(out.join(later), "an old part").unwrap();
        let args = with_overwrite(cmd, true);
        let res = run(&dir, &args);
        let err = String::from_utf8_lossy(&res.stderr);
        assert!(res.status.success(), "{args:?}: {err}");
        assert_ne!(std::fs::read(&other).unwrap(), b"another file", "{args:?}");
        assert_ne!(std::fs::read(out.join(later)).unwrap(), b"an old part");
        let link = std::fs::symlink_metadata(out.join(first)).unwrap();
        assert!(link.file_type().is_symlink(), "{args:?}");
        assert!(
            std::fs::read(dir.join("input.pdf")).unwrap() == before,
            "{args:?} changed the input"
        );
    }
}

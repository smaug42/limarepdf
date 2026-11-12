//! Commands that write a document never change their input: an output naming the input, by any
//! path or link, is refused with nothing written. So is any of the files `split`, `render` and
//! `save-images` name after their input, even with `--overwrite` (A-02), and an output naming
//! any other file the command reads: a picture, a password, form values, a trust list, a
//! certificate or a digital ID (PB-139).

use limarepdf_core::render::{encode, Bitmap, ImageFormat};
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

/// A PDF of `objects`, numbered from 1, the first its catalog, at `dir/name`.
fn pdf_file(dir: &Path, name: &str, objects: &[&str]) -> PathBuf {
    let mut pdf = b"%PDF-1.7\n".to_vec();
    let mut offsets = Vec::new();
    for (i, o) in objects.iter().enumerate() {
        offsets.push(pdf.len());
        pdf.extend_from_slice(format!("{} 0 obj\n{o}\nendobj\n", i + 1).as_bytes());
    }
    let (xref, size) = (pdf.len(), objects.len() + 1);
    pdf.extend_from_slice(format!("xref\n0 {size}\n0000000000 65535 f \n").as_bytes());
    for o in offsets {
        pdf.extend_from_slice(format!("{o:010} 00000 n \n").as_bytes());
    }
    pdf.extend_from_slice(
        format!("trailer\n<< /Size {size} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n").as_bytes(),
    );
    let path = dir.join(name);
    std::fs::write(&path, pdf).unwrap();
    path
}

/// A one-page PDF at `dir/name`.
fn make_pdf(dir: &Path, name: &str) -> PathBuf {
    let objects = [
        "<< /Type /Catalog /Pages 2 0 R >>",
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>",
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 200 200] >>",
    ];
    pdf_file(dir, name, &objects)
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
            .env("HOME", dir)
            .env("XDG_CONFIG_HOME", dir.join("config"))
            .args(args),
    )
    .unwrap()
}

fn ok(dir: &Path, args: &[&str]) {
    let res = run(dir, args);
    let err = String::from_utf8_lossy(&res.stderr);
    assert!(res.status.success(), "{args:?}: {err}");
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

/// A hard link to the input is refused: told by its inode on Unix, by its file ID on Windows
/// (D-02).
#[cfg(any(unix, windows))]
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

#[cfg(any(unix, windows))]
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
#[cfg(any(unix, windows))]
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
/// would leave the input as it was: one rule for every file a command writes. On Windows too
/// since D-02.
#[cfg(any(unix, windows))]
#[test]
fn a_generated_file_hard_linked_to_the_input_is_refused() {
    let dir = folder("generated-hardlink");
    let before = pictures_pdf(&dir);
    let (out, full) = (dir.join("out"), dir.join("input.pdf"));
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
            // Still the link: a file of its own written there would hold a picture.
            assert!(
                std::fs::read(out.join(name)).unwrap() == before,
                "{case} replaced the link"
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

/// The files PB-139's commands read, in `dir`: `in.pdf`; `form.pdf` with a text field Name;
/// `pic.png`; the password `pw`, and in `pw.p12` too, as `id export` takes only that name; a new
/// password `new.p12`; a digital ID imported into `ids`; the values `v.json`; `signed.pdf`, signed
/// with the ID; `trust.txt` trusting it and `cert.pem` its certificate. The ID's file in `ids`.
fn read_files(dir: &Path) -> String {
    make_pdf(dir, "in.pdf");
    let form = [
        "<< /Type /Catalog /Pages 2 0 R /AcroForm << /Fields [4 0 R] >> >>",
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>",
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 200 200] /Annots [4 0 R] >>",
        "<< /Type /Annot /Subtype /Widget /FT /Tx /T (Name) /Rect [20 20 180 40] /P 3 0 R >>",
    ];
    pdf_file(dir, "form.pdf", &form);
    let (w, h) = (60, 30);
    let ink = |x: usize, y: usize| {
        (5..25).contains(&x) && (13..17).contains(&y)
            || (35..50).contains(&x) && (8..22).contains(&y)
    };
    let rgba = (0..w * h)
        .flat_map(|i| match ink(i % w, i / w) {
            true => [20, 20, 30, 255],
            false => [255, 255, 255, 255],
        })
        .collect();
    let sheet = Bitmap {
        width: w,
        height: h,
        rgba,
    };
    let png = encode(&sheet, ImageFormat::Png, 96.0).unwrap();
    std::fs::write(dir.join("pic.png"), png).unwrap();
    for name in ["pw", "pw.p12"] {
        std::fs::write(dir.join(name), "mpe-test-password\n").unwrap();
    }
    std::fs::write(dir.join("new.p12"), "walrus-and-carpenter\n").unwrap();
    std::fs::write(dir.join("v.json"), r#"{"Name": "Walrus"}"#).unwrap();
    let p12 = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../limarepdf-core/tests/data/signatures/ids/p256.p12"
    );
    std::fs::copy(p12, dir.join("p256.p12")).unwrap();
    let setup = [
        "id import p256.p12 --password-file pw --id-folder ids",
        "id export 1 -o pat.cer --id-folder ids",
        "trust add pat.cer --trust-file trust.txt",
        "sign in.pdf -o signed.pdf --id 1 --page 1 --rect 20,20,100,40 --password-file pw \
         --id-folder ids",
    ];
    for cmd in setup {
        ok(dir, &cmd.split_whitespace().collect::<Vec<_>>());
    }
    let trust = std::fs::read_to_string(dir.join("trust.txt")).unwrap();
    let begin = trust.find("-----BEGIN CERTIFICATE-----").unwrap();
    let end = trust.find("-----END CERTIFICATE-----").unwrap() + 25;
    std::fs::write(dir.join("cert.pem"), format!("{}\n", &trust[begin..end])).unwrap();
    let mut ids = names(&dir.join("ids"));
    ids.retain(|n| n.ends_with(".p12"));
    assert_eq!(ids.len(), 1, "{ids:?}");
    format!("ids/{}", ids[0])
}

const SIGN: &[&str] = &[
    "sign",
    "in.pdf",
    "-o",
    "OUT",
    "--id",
    "1",
    "--page",
    "1",
    "--rect",
    "20,20,100,40",
    "--image",
    "pic.png",
    "--password-file",
    "pw",
    "--id-folder",
    "ids",
];

const EXPORT: &[&str] = &[
    "id",
    "export",
    "1",
    "-o",
    "OUT",
    "--password-file",
    "pw.p12",
    "--new-password-file",
    "new.p12",
    "--id-folder",
    "ids",
];

/// Each command reading a file besides its input, with `OUT` where its output goes, and that
/// file (`ID` the digital ID's).
const READ: [(&[&str], &str); 12] = [
    (
        &[
            "insert-images",
            "in.pdf",
            "-o",
            "OUT",
            "--after",
            "1",
            "--images",
            "pic.png",
        ],
        "pic.png",
    ),
    (
        &["watermark", "in.pdf", "-o", "OUT", "--image", "pic.png"],
        "pic.png",
    ),
    (
        &["stamp", "in.pdf", "-o", "OUT", "--image", "pic.png"],
        "pic.png",
    ),
    (SIGN, "pic.png"),
    (SIGN, "pw"),
    (SIGN, "ID"),
    (EXPORT, "pw.p12"),
    (EXPORT, "new.p12"),
    (EXPORT, "ID"),
    (
        &[
            "verify",
            "signed.pdf",
            "--signed-version",
            "1",
            "-o",
            "OUT",
            "--trust-file",
            "trust.txt",
        ],
        "trust.txt",
    ),
    (
        &["trust", "add", "cert.pem", "--trust-file", "OUT"],
        "cert.pem",
    ),
    (
        &[
            "form", "fill", "form.pdf", "--values", "v.json", "-o", "OUT",
        ],
        "v.json",
    ),
];

/// PB-139: a file a command reads besides its input, named as its output by its path or by a
/// link to it, is refused as the input is, with nothing written: the file and the link keep
/// their bytes. Every case is run, and the file put back after each.
#[test]
fn a_file_read_besides_the_input_is_refused_as_the_output() {
    let dir = folder("read-files");
    let id = read_files(&dir);
    let mut failed = Vec::new();
    for (cmd, read) in READ {
        let read = if read == "ID" { id.as_str() } else { read };
        let file = dir.join(read);
        let before = std::fs::read(&file).unwrap();
        let base = Path::new(read).file_name().unwrap().to_str().unwrap();
        #[cfg_attr(not(any(unix, windows)), allow(unused_mut))]
        let mut outs = vec![read.to_string(), format!("./{read}")];
        #[cfg(unix)]
        outs.push(format!("link-{base}"));
        #[cfg(any(unix, windows))]
        outs.push(format!("hard-{base}"));
        for out in outs {
            let link = out.starts_with("link-") || out.starts_with("hard-");
            #[cfg(unix)]
            if out.starts_with("link-") {
                std::os::unix::fs::symlink(&file, dir.join(&out)).unwrap();
            }
            if out.starts_with("hard-") {
                std::fs::hard_link(&file, dir.join(&out)).unwrap();
            }
            let args: Vec<&str> = cmd
                .iter()
                .map(|&a| if a == "OUT" { out.as_str() } else { a })
                .collect();
            let res = run(&dir, &args);
            let err = String::from_utf8_lossy(&res.stderr);
            let refused =
                res.status.code() == Some(1) && err.contains("must be saved to another file");
            let kept = std::fs::read(&file).is_ok_and(|b| b == before);
            let linked = !out.starts_with("link-")
                || std::fs::symlink_metadata(dir.join(&out)).is_ok_and(|m| m.is_symlink());
            if !(refused && kept && linked) {
                failed.push(format!(
                    "{args:?} reading {read}: refused {refused}, kept {kept}, link kept \
                     {linked}: {}",
                    err.trim()
                ));
            }
            std::fs::write(&file, &before).unwrap();
            if link {
                std::fs::remove_file(dir.join(&out)).unwrap();
            }
        }
    }
    assert!(failed.is_empty(), "{}", failed.join("\n"));
}

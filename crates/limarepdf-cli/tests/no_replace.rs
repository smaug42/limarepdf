//! Commands that write several files never replace one silently: with a file already at any
//! of their names, nothing is written unless `--overwrite` is given (BUG-05, audit A07). Every
//! file is written whole or not at all, so a file linked to another one is replaced, never
//! changed in place (BUG-15).

use std::path::{Path, PathBuf};
use std::process::Output;

/// A PDF at `dir/name` with `pages` pages and, with `bookmark`, a top-level bookmark of that
/// title on its second page.
fn make_pdf(dir: &Path, name: &str, pages: usize, bookmark: Option<&str>) -> PathBuf {
    let first_page = 3;
    let kids: Vec<String> = (0..pages)
        .map(|i| format!("{} 0 R", first_page + i))
        .collect();
    let outline = first_page + pages;
    let mut objects = vec![
        match bookmark {
            Some(_) => format!("<< /Type /Catalog /Pages 2 0 R /Outlines {outline} 0 R >>"),
            None => "<< /Type /Catalog /Pages 2 0 R >>".into(),
        },
        format!(
            "<< /Type /Pages /Kids [{}] /Count {pages} >>",
            kids.join(" ")
        ),
    ];
    for _ in 0..pages {
        objects.push("<< /Type /Page /Parent 2 0 R /MediaBox [0 0 200 200] >>".into());
    }
    if let Some(title) = bookmark {
        let item = outline + 1;
        objects.push(format!(
            "<< /Type /Outlines /First {item} 0 R /Last {item} 0 R /Count 1 >>"
        ));
        objects.push(format!(
            "<< /Title ({title}) /Parent {outline} 0 R /Dest [{} 0 R /Fit] >>",
            first_page + 1
        ));
    }
    let mut pdf = b"%PDF-1.7\n".to_vec();
    let mut offsets = Vec::new();
    for (i, o) in objects.iter().enumerate() {
        offsets.push(pdf.len());
        pdf.extend_from_slice(format!("{} 0 obj\n{o}\nendobj\n", i + 1).as_bytes());
    }
    let xref = pdf.len();
    let size = objects.len() + 1;
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

fn folder(name: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join("no_replace")
        .join(name);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn run(dir: &Path, args: &[&str]) -> Output {
    std::process::Command::new(env!("CARGO_BIN_EXE_limarepdf-cli"))
        .current_dir(dir)
        .env("HOME", dir)
        .env("XDG_CONFIG_HOME", dir.join("config"))
        .args(args)
        .output()
        .unwrap()
}

/// Runs the command, which must succeed.
fn ok(dir: &Path, args: &[&str]) {
    let res = run(dir, args);
    let err = String::from_utf8_lossy(&res.stderr);
    assert!(res.status.success(), "{args:?}: {err}");
}

/// Runs the command, which must fail; returns its error.
fn refused(dir: &Path, args: &[&str]) -> String {
    let res = run(dir, args);
    let err = String::from_utf8_lossy(&res.stderr).into_owned();
    assert_eq!(res.status.code(), Some(1), "{args:?}: {err}");
    err
}

/// The names of the files in `dir`, sorted.
fn names(dir: &Path) -> Vec<String> {
    let mut v: Vec<String> = std::fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    v.sort();
    v
}

fn read(path: PathBuf) -> Vec<u8> {
    std::fs::read(path).unwrap()
}

fn pages(path: &Path) -> usize {
    limarepdf_core::Pdf::open(path, None).unwrap().page_count()
}

#[test]
fn split_refuses_to_replace_a_file_and_writes_nothing() {
    let dir = folder("split-count");
    make_pdf(&dir, "in.pdf", 3, None);
    std::fs::write(dir.join("in-002.pdf"), "someone's notes").unwrap();
    let err = refused(&dir, &["split", "in.pdf", "--out-dir", "."]);
    assert!(err.contains("in-002.pdf"), "{err}");
    assert!(err.contains("--overwrite"), "{err}");
    assert!(err.contains("Nothing was written"), "{err}");
    assert_eq!(read(dir.join("in-002.pdf")), b"someone's notes");
    assert_eq!(names(&dir), ["in-002.pdf", "in.pdf"], "a part was written");

    ok(&dir, &["split", "in.pdf", "--out-dir", ".", "--overwrite"]);
    for n in ["in-001.pdf", "in-002.pdf", "in-003.pdf"] {
        assert_eq!(pages(&dir.join(n)), 1, "{n}");
    }
}

#[test]
fn split_at_bookmarks_refuses_to_replace_a_titled_part() {
    let dir = folder("split-marks");
    make_pdf(&dir, "in.pdf", 3, Some("Second"));
    let out = dir.join("out");
    std::fs::create_dir(&out).unwrap();
    std::fs::write(out.join("in-002 Second.pdf"), "keep").unwrap();
    let split = ["split", "in.pdf", "--bookmarks", "--out-dir", "out"];
    let err = refused(&dir, &split);
    assert!(err.contains("in-002 Second.pdf"), "{err}");
    assert_eq!(names(&out), ["in-002 Second.pdf"]);
    assert_eq!(read(out.join("in-002 Second.pdf")), b"keep");

    ok(&dir, &[&split[..], &["--overwrite"]].concat());
    assert_eq!(names(&out), ["in-001.pdf", "in-002 Second.pdf"]);
    assert_eq!(pages(&out.join("in-002 Second.pdf")), 2);
}

/// A folder that can't be written to: an error, and no files, temporary ones included.
#[cfg(unix)]
#[test]
fn split_into_an_unwritable_folder_fails_and_leaves_nothing() {
    use std::os::unix::fs::PermissionsExt;
    let dir = folder("split-unwritable");
    make_pdf(&dir, "in.pdf", 3, None);
    let out = dir.join("out");
    std::fs::create_dir(&out).unwrap();
    std::fs::set_permissions(&out, std::fs::Permissions::from_mode(0o555)).unwrap();
    // Root can write anywhere; there's nothing to check then.
    if std::fs::write(out.join("probe"), "").is_ok() {
        return;
    }
    let res = run(&dir, &["split", "in.pdf", "--out-dir", "out"]);
    std::fs::set_permissions(&out, std::fs::Permissions::from_mode(0o755)).unwrap();
    let err = String::from_utf8_lossy(&res.stderr);
    assert_eq!(res.status.code(), Some(1), "{err}");
    assert!(names(&out).is_empty(), "{:?}", names(&out));
}

#[test]
fn render_refuses_to_replace_an_image() {
    let dir = folder("render");
    make_pdf(&dir, "in.pdf", 2, None);
    std::fs::write(dir.join("in-001.png"), "a photo").unwrap();
    let err = refused(&dir, &["render", "in.pdf", "--out-dir", "."]);
    assert!(err.contains("in-001.png"), "{err}");
    assert!(err.contains("--overwrite"), "{err}");
    assert_eq!(read(dir.join("in-001.png")), b"a photo");
    assert!(!dir.join("in-002.png").exists(), "an image was written");

    ok(&dir, &["render", "in.pdf", "--out-dir", ".", "--overwrite"]);
    for n in ["in-001.png", "in-002.png"] {
        assert!(read(dir.join(n)).starts_with(b"\x89PNG"), "{n}");
    }
}

/// Replacing an image linked to another file replaces the image only.
#[cfg(unix)]
#[test]
fn render_over_a_linked_image_leaves_the_other_file() {
    let dir = folder("render-linked");
    make_pdf(&dir, "in.pdf", 1, None);
    std::fs::write(dir.join("other.png"), "another file").unwrap();
    std::fs::hard_link(dir.join("other.png"), dir.join("in-001.png")).unwrap();
    ok(&dir, &["render", "in.pdf", "--out-dir", ".", "--overwrite"]);
    assert_eq!(read(dir.join("other.png")), b"another file");
    assert!(read(dir.join("in-001.png")).starts_with(b"\x89PNG"));
}

/// An exported certificate replaces a file linked to another one; the other file keeps its
/// contents.
#[cfg(unix)]
#[test]
fn a_certificate_export_over_a_linked_file_leaves_the_other_file() {
    let dir = folder("cer-linked");
    std::fs::write(dir.join("pw"), "walrus-otter-seal\n").unwrap();
    let ids = "--id-folder ids";
    let create = format!("id create --name Pat --password-file pw {ids}");
    ok(&dir, &create.split(' ').collect::<Vec<_>>());
    std::fs::write(dir.join("other.cer"), "another file").unwrap();
    std::fs::hard_link(dir.join("other.cer"), dir.join("pat.cer")).unwrap();
    let export = format!("id export 1 -o pat.cer {ids}");
    ok(&dir, &export.split(' ').collect::<Vec<_>>());
    assert_eq!(read(dir.join("other.cer")), b"another file");
    assert_eq!(read(dir.join("pat.cer"))[0], 0x30, "not a DER certificate");
}

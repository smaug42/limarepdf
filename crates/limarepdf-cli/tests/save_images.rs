//! `save-images` (NEW-09's stage 2): every picture to a folder, named by page and number, a
//! JPEG as it is and any other picture a PNG; files already there kept unless `--overwrite`.

use limarepdf_core::render::{encode, Bitmap, ImageFormat};
use limarepdf_core::Pdf;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn folder(name: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join("save-images")
        .join(name);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn run(dir: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_limarepdf-cli"))
        .current_dir(dir)
        .env("HOME", dir)
        .env("XDG_CONFIG_HOME", dir.join("config"))
        .args(args)
        .output()
        .unwrap()
}

fn ok(dir: &Path, args: &[&str]) -> String {
    let res = run(dir, args);
    let err = String::from_utf8_lossy(&res.stderr).into_owned();
    assert!(res.status.success(), "{args:?}: {err}");
    String::from_utf8_lossy(&res.stdout).into_owned()
}

/// A 40 × 20 bitmap, red on the left and blue on the right, or the other way round.
fn halves(red_left: bool) -> Bitmap {
    let (w, h) = (40, 20);
    let rgba = (0..w * h)
        .flat_map(|i| match (i % w < w / 2) == red_left {
            true => [200, 20, 20, 255],
            false => [20, 20, 200, 255],
        })
        .collect();
    Bitmap {
        width: w,
        height: h,
        rgba,
    }
}

/// `in.pdf`: page 1 a lossless picture and then a JPEG, page 2 no pictures, page 3 another
/// JPEG. Returns page 1's JPEG.
fn pictures_pdf(dir: &Path) -> Vec<u8> {
    let png = encode(&halves(true), ImageFormat::Png, 72.0).unwrap();
    let jpeg = encode(&halves(true), ImageFormat::Jpeg(90), 72.0).unwrap();
    let other = encode(&halves(false), ImageFormat::Jpeg(90), 72.0).unwrap();
    let mut p = Pdf::new();
    for i in 0..3 {
        p.insert_blank(i, 612.0, 792.0).unwrap();
    }
    p.add_image(0, [72.0, 600.0, 152.0, 640.0], &png).unwrap();
    p.add_image(0, [72.0, 400.0, 152.0, 440.0], &jpeg).unwrap();
    p.add_image(2, [72.0, 400.0, 152.0, 440.0], &other).unwrap();
    std::fs::write(dir.join("in.pdf"), p.to_bytes().unwrap()).unwrap();
    jpeg
}

fn names(dir: &Path) -> Vec<String> {
    let mut v: Vec<String> = std::fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    v.sort();
    v
}

#[test]
fn saves_every_picture_by_page_and_number() {
    let dir = folder("saved");
    let jpeg = pictures_pdf(&dir);
    let out = ok(&dir, &["save-images", "in.pdf", "--out-dir", "pics"]);
    assert!(out.contains("saved 3 picture(s) to pics"), "{out}");
    let pics = dir.join("pics");
    assert_eq!(
        names(&pics),
        [
            "in-page-001-image-1.png",
            "in-page-001-image-2.jpg",
            "in-page-003-image-1.jpg",
        ]
    );
    // The JPEG as it is in the document; the other a PNG of its own pixels.
    let saved = std::fs::read(pics.join("in-page-001-image-2.jpg")).unwrap();
    assert_eq!(saved, jpeg);
    let png = std::fs::read(pics.join("in-page-001-image-1.png")).unwrap();
    let (w, h, rgba) = limarepdf_core::picture_preview(&png, 1000).unwrap();
    assert_eq!((w, h), (40, 20));
    assert_eq!(rgba, halves(true).rgba);

    // Files already there: nothing written without --overwrite.
    std::fs::write(pics.join("in-page-001-image-1.png"), b"mine").unwrap();
    let res = run(&dir, &["save-images", "in.pdf", "--out-dir", "pics"]);
    assert!(!res.status.success());
    let err = String::from_utf8_lossy(&res.stderr);
    assert!(err.contains("in-page-001-image-1.png"), "{err}");
    assert_eq!(
        std::fs::read(pics.join("in-page-001-image-1.png")).unwrap(),
        b"mine"
    );
    ok(
        &dir,
        &["save-images", "in.pdf", "--out-dir", "pics", "--overwrite"],
    );
    assert_ne!(
        std::fs::read(pics.join("in-page-001-image-1.png")).unwrap(),
        b"mine"
    );
}

#[test]
fn a_document_without_pictures_says_so() {
    let dir = folder("none");
    let mut p = Pdf::new();
    p.insert_blank(0, 612.0, 792.0).unwrap();
    std::fs::write(dir.join("plain.pdf"), p.to_bytes().unwrap()).unwrap();
    let out = ok(&dir, &["save-images", "plain.pdf", "--out-dir", "pics"]);
    assert!(out.contains("no pictures to save"), "{out}");
    assert!(!dir.join("pics").exists());
}

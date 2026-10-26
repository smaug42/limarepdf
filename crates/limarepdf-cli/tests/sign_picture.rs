//! `sign --image` with `--crop`: only the part of the picture given is used (PB-21), such as
//! the signature on a photo of a whole sheet, in the picture's pixels as it is shown.

use limarepdf_core::render::{encode, Bitmap, ImageFormat};
use limarepdf_core::Pdf;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn folder(name: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join("sign_picture")
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

/// A folder with a digital ID imported, its password in `pw`, a one-page `in.pdf`, and
/// `sheet.png`: a white sheet 200 by 100 pixels with a printed bar on the left (x 10 to 60)
/// and a signature on the right (x 130 to 170).
fn setup(name: &str) -> PathBuf {
    let dir = folder(name);
    let p12 = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../limarepdf-core/tests/data/signatures/ids/p256.p12"
    );
    std::fs::copy(p12, dir.join("p256.p12")).unwrap();
    std::fs::write(dir.join("pw"), "mpe-test-password\n").unwrap();
    let import = "id import p256.p12 --password-file pw --id-folder ids";
    let import: Vec<&str> = import.split(' ').collect();
    ok(&dir, &import);
    let mut pdf = Pdf::new();
    pdf.insert_blank(0, 300.0, 200.0).unwrap();
    std::fs::write(dir.join("in.pdf"), pdf.to_bytes().unwrap()).unwrap();
    let (w, h) = (200, 100);
    let ink = |x: usize, y: usize| {
        (10..60).contains(&x) && (45..55).contains(&y)
            || (130..170).contains(&x) && (35..65).contains(&y)
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
    std::fs::write(dir.join("sheet.png"), png).unwrap();
    dir
}

fn sign(dir: &Path, more: &[&str]) -> Output {
    let base = "sign in.pdf -o out.pdf --id 1 --page 1 --rect 20,20,200,60 --image sheet.png \
                --password-file pw --id-folder ids";
    let base: Vec<&str> = base.split_whitespace().collect();
    run(dir, &[&base[..], more].concat())
}

/// That signing worked, or what it said.
fn signed(res: &Output) {
    let err = String::from_utf8_lossy(&res.stderr);
    assert!(res.status.success(), "{err}");
}

/// The widest picture in `out.pdf`, in pixels.
fn widest_picture(dir: &Path) -> i64 {
    let bytes = std::fs::read(dir.join("out.pdf")).unwrap();
    let p = Pdf::from_bytes(&bytes, None).unwrap();
    let widths = p.doc.objects.values().filter_map(|o| {
        let s = o.as_stream().ok()?;
        let image = s.dict.get(b"Subtype").ok()?.as_name().ok()? == b"Image";
        let width = || s.dict.get(b"Width").ok()?.as_i64().ok();
        image.then(width).flatten()
    });
    widths.max().expect("a picture")
}

#[test]
fn only_the_part_given_of_the_picture_is_used() {
    let dir = setup("crop");
    // The whole sheet: the printed bar and the signature, cropped to the ink.
    signed(&sign(&dir, &[]));
    let whole = widest_picture(&dir);
    assert!(whole >= 150, "{whole}");
    // The signature alone.
    std::fs::remove_file(dir.join("out.pdf")).unwrap();
    signed(&sign(&dir, &["--crop", "110,20,190,80"]));
    let part = widest_picture(&dir);
    assert!((40..80).contains(&part), "{part}");
}

#[test]
fn a_crop_that_isn_t_a_part_of_the_picture_is_refused() {
    let dir = setup("refused");
    let bad = "110,20,90,80 0,0,201,100 1,2,3 a,b,c,d -1,0,50,50";
    for crop in bad.split(' ') {
        let res = sign(&dir, &["--crop", crop]);
        let err = String::from_utf8_lossy(&res.stderr);
        assert!(!res.status.success(), "{crop}: {err}");
        assert!(err.contains("--crop"), "{crop}: {err}");
        assert!(!dir.join("out.pdf").exists(), "{crop}: signed anyway");
    }
}

//! Opening damaged files: cross-reference tables with wrong offsets, none at all, a
//! cross-reference stream that doesn't read, a missing `endobj`, a catalog found only by
//! scanning, and an object stream lopdf can't read. Each file is built here by hand.

use limarepdf_core::render;
use limarepdf_core::{Cipher, Encryption, Pdf, Restrictions};

/// A page's content: a black box over most of a 200 × 100 page.
const CONTENT: &str = "0 g 20 20 160 60 re f";

/// The objects of a one-page document by number: catalog, page tree, page, content and
/// an information dictionary.
fn objects() -> Vec<(u32, String)> {
    vec![
        (1, "<< /Type /Catalog /Pages 2 0 R >>".into()),
        (2, "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".into()),
        (3, page(200)),
        (
            4,
            format!(
                "<< /Length {} >>\nstream\n{CONTENT}\nendstream",
                CONTENT.len()
            ),
        ),
        (5, "<< /Title (Damaged) >>".into()),
    ]
}

fn page(width: u32) -> String {
    format!("<< /Type /Page /Parent 2 0 R /MediaBox [0 0 {width} 100] /Contents 4 0 R >>")
}

/// A file holding `objects`, and where each starts.
fn body(objects: &[(u32, String)]) -> (Vec<u8>, Vec<usize>) {
    let mut b = b"%PDF-1.7\n%\xe2\xe3\xcf\xd3\n".to_vec();
    let mut at = Vec::new();
    for (n, o) in objects {
        at.push(b.len());
        b.extend_from_slice(format!("{n} 0 obj\n{o}\nendobj\n").as_bytes());
    }
    (b, at)
}

/// Appends a cross-reference table giving `offsets` to objects 1 on, a trailer with
/// `trailer` in it, and a correct `startxref`.
fn table(b: &mut Vec<u8>, offsets: &[usize], trailer: &str) {
    let xref = b.len();
    let mut t = format!("xref\n0 {}\n0000000000 65535 f\r\n", offsets.len() + 1);
    for o in offsets {
        t.push_str(&format!("{o:010} 00000 n\r\n"));
    }
    t.push_str(&format!(
        "trailer\n<< /Size {} {trailer} >>\nstartxref\n{xref}\n%%EOF\n",
        offsets.len() + 1
    ));
    b.extend_from_slice(t.as_bytes());
}

/// `offsets` moved by `by` bytes.
fn shifted(offsets: &[usize], by: usize) -> Vec<usize> {
    offsets.iter().map(|o| o + by).collect()
}

/// Opens `bytes` and checks it is repaired, has its one page, and draws the box.
fn opens_repaired(bytes: &[u8], password: Option<&str>) -> Pdf {
    let mut pdf = Pdf::from_bytes(bytes, password).expect("doesn't open");
    assert!(pdf.repaired(), "not marked repaired");
    assert_eq!(pdf.page_count(), 1);
    assert_eq!(pdf.page_size(0).unwrap(), (200.0, 100.0));
    let hp = render::load(pdf.render_snapshot().unwrap(), None).unwrap();
    let b = render::Renderer::new(&hp).render(0, 1.0).unwrap();
    let mid = (b.height / 2 * b.width + b.width / 2) * 4;
    assert!(b.rgba[mid] < 50, "the page's content isn't drawn");
    pdf
}

#[test]
fn intact_files_are_not_repaired() {
    let (mut b, at) = body(&objects());
    table(&mut b, &at, "/Root 1 0 R /Info 5 0 R");
    let pdf = Pdf::from_bytes(&b, None).unwrap();
    assert!(!pdf.repaired());
    assert_eq!(pdf.page_count(), 1);
    // A document with no pages isn't damaged either.
    let empty = Pdf::new().to_bytes().unwrap();
    let pdf = Pdf::from_bytes(&empty, None).unwrap();
    assert!(!pdf.repaired());
    assert_eq!(pdf.page_count(), 0);
}

#[test]
fn wrong_offsets() {
    let (mut b, at) = body(&objects());
    table(&mut b, &shifted(&at, 3), "/Root 1 0 R /Info 5 0 R");
    let pdf = opens_repaired(&b, None);
    assert_eq!(pdf.info().title.as_deref(), Some("Damaged"));
}

#[test]
fn no_table_at_all() {
    let (mut b, _) = body(&objects());
    b.extend_from_slice(b"trailer\n<< /Root 1 0 R /Info 5 0 R >>\n%%EOF\n");
    opens_repaired(&b, None);
    // Nor a trailer: the catalog is found by scanning.
    let (b, _) = body(&objects());
    opens_repaired(&b, None);
}

#[test]
fn broken_cross_reference_stream() {
    let (mut b, _) = body(&objects());
    let xref = b.len();
    b.extend_from_slice(
        format!(
            "6 0 obj\n<< /Type /XRef /Size 7 /W [1 2 1] /Root 1 0 R /Info 5 0 R \
             /Filter /FlateDecode /Length 9 >>\nstream\nnot flate\nendstream\nendobj\n\
             startxref\n{xref}\n%%EOF\n"
        )
        .as_bytes(),
    );
    let pdf = opens_repaired(&b, None);
    assert_eq!(pdf.info().title.as_deref(), Some("Damaged"));
}

#[test]
fn missing_endobj() {
    // The page's `endobj` is missing: the next object's header ends it.
    let (b, at) = body(&objects());
    let mut b = String::from_utf8_lossy(&b).into_owned();
    let end = b[at[2]..].find("endobj\n").unwrap() + at[2];
    b.replace_range(end..end + 7, "");
    let mut b = b.into_bytes();
    table(&mut b, &shifted(&at, 1), "/Root 1 0 R");
    opens_repaired(&b, None);
}

#[test]
fn later_copies_win() {
    // A narrower first copy of the page, replaced later in the file as an appended
    // revision would.
    let mut o = objects();
    o[2].1 = page(50);
    o.push((3, page(200)));
    let (mut b, at) = body(&o);
    table(&mut b, &shifted(&at[..5], 2), "/Root 1 0 R");
    opens_repaired(&b, None);
}

#[test]
fn catalog_found_by_scanning() {
    // The trailer names the information dictionary as the catalog.
    let (mut b, at) = body(&objects());
    table(&mut b, &shifted(&at, 2), "/Root 5 0 R");
    opens_repaired(&b, None);
}

#[test]
fn object_stream_with_comments() {
    // The catalog, page tree and page are in an object stream whose members start with
    // comments, which lopdf's reader of object streams doesn't take; the table is gone.
    let o = objects();
    let mut index = String::new();
    let mut data = String::new();
    for (n, obj) in &o[..3] {
        index.push_str(&format!("{n} {} ", data.len()));
        data.push_str(&format!("% {n} 0 obj\n{obj}\n"));
    }
    index.push('\n');
    let stream = format!(
        "<< /Type /ObjStm /N 3 /First {} /Length {} >>\nstream\n{index}{data}\nendstream",
        index.len(),
        index.len() + data.len() + 1
    );
    let (mut b, _) = body(&[o[3].clone(), o[4].clone(), (6, stream)]);
    b.extend_from_slice(b"trailer\n<< /Root 1 0 R >>\nstartxref\n999999\n%%EOF\n");
    opens_repaired(&b, None);
}

#[test]
fn damaged_encrypted_files_need_their_password() {
    let (mut b, at) = body(&objects());
    table(&mut b, &at, "/Root 1 0 R /Info 5 0 R");
    let mut p = Pdf::from_bytes(&b, None).unwrap();
    p.set_encryption(Some(Encryption {
        user_password: "user".into(),
        owner_password: "owner".into(),
        cipher: Cipher::Aes128,
        restrictions: Restrictions::default(),
    }));
    let mut enc = p.to_bytes().unwrap();
    // Every offset in the table is now 40 bytes out.
    let line = enc.iter().position(|&c| c == b'\n').unwrap() + 1;
    enc.splice(line..line, *b"%                                      \n");
    let wrong = Pdf::from_bytes(&enc, Some("nope")).err().expect("opened");
    assert!(format!("{wrong:#}").contains("password"), "{wrong:#}");
    let pdf = opens_repaired(&enc, Some("user"));
    assert_eq!(pdf.info().title.as_deref(), Some("Damaged"));
}

#[test]
fn repaired_files_save_whole_and_clean() {
    let (mut b, at) = body(&objects());
    table(&mut b, &shifted(&at, 5), "/Root 1 0 R /Info 5 0 R");
    let mut pdf = opens_repaired(&b, None);
    let back = Pdf::from_bytes(&pdf.to_bytes().unwrap(), None).unwrap();
    assert!(!back.repaired());
    assert_eq!(back.page_count(), 1);
}

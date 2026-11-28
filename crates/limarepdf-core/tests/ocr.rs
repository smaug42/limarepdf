//! OCR text layers, with the bundled recognition model. Opt-in, Poppler reads the layer as
//! LimarePDF does: `cargo test -p limarepdf-core --test all -- --ignored ocr::cross_check`.

use crate::common;

use common::saved::{tools, Saved};
use image::{DynamicImage, GrayImage, Luma};
use limarepdf_core::print::{Orientation, PageSetup, Paper, PerSheet, Scale};
use limarepdf_core::{
    render, FontChoice, ImageImport, ObjectKind, Ocr, OcrResult, PageObject, Pdf, TextEdit,
};
use lopdf::{dictionary, Dictionary, Object, ObjectId, Stream};
use std::io::Cursor;

const LINES: [&str; 6] = [
    "Invoice 2026-0417, issued on 12 October 2026.",
    "Please pay the balance within 30 days.",
    "The committee met on Tuesday morning.",
    "Contact accounts@example.org for details.",
    "Shipping weight 14.7 kg, reference ZX-88.",
    "She quickly jumped over the fence.",
];

/// A 4 x 3 inch page of text rendered as a greyscale image at 300 dpi.
fn text_image() -> GrayImage {
    gray(&mut text_page(), 300.0 / 72.0)
}

/// A 4 x 3 inch page of LINES as text, in Helvetica.
fn text_page() -> Pdf {
    let mut content = String::from("BT /F1 12 Tf 16 190 Td 18 TL\n");
    for l in LINES {
        content.push_str(&format!("({l}) '\n"));
    }
    content.push_str("ET");
    helvetica_page(288.0, 216.0, content)
}

/// Lines as long as a page's. Tilted a few degrees, each drops past the next across the page.
const LONG: [&str; 6] = [
    "The harbour office opens at seven, and the first ferry leaves for the island at eight.",
    "Passengers with bicycles board first, then cars, then those travelling on foot alone.",
    "Tickets bought on board cost two euros more than those bought at the office window.",
    "In winter the last ferry back leaves at five, an hour earlier than it does in summer.",
    "Dogs travel free if kept on a lead, but they must stay on the open deck at all times.",
    "Lost property is kept at the office for a month, then given to the town's charities.",
];

/// An 8.5 x 4 inch page drawing `content`, which names Helvetica /F1, turned `deg` degrees
/// anticlockwise about its middle, as a greyscale image at 300 dpi: a scan a little tilted.
fn tilted(deg: f32, content: &str) -> GrayImage {
    let (s, c) = deg.to_radians().sin_cos();
    let (cx, cy) = (306.0, 144.0);
    let (e, f) = (cx - c * cx + s * cy, cy - s * cx - c * cy);
    let content = format!("q {c} {s} {} {c} {e} {f} cm {content} Q", -s);
    gray(&mut helvetica_page(612.0, 288.0, content), 300.0 / 72.0)
}

/// LONG, 11 pt on 14 pt, [`tilted`] `deg` degrees.
fn tilted_image(deg: f32) -> GrayImage {
    let mut content = String::from("BT /F1 11 Tf 60 200 Td 14 TL\n");
    for l in LONG {
        content.push_str(&format!("({l}) '\n"));
    }
    content.push_str("ET");
    tilted(deg, &content)
}

/// A long line, a short one and another long one.
const SHORT_AFTER_LONG: [&str; 3] = [LONG[0], "Shut on all bank days.", LONG[1]];
/// A table's rows, a cell in each of TABLE_AT.
const TABLE: [[&str; 5]; 3] = [
    ["Boat", "Leaves from", "Arrives", "Fare", "Deck"],
    ["Osprey", "Harbour Street", "Island", "four", "open"],
    ["Heron", "North Quay", "Mainland", "five", "shut"],
];
/// Where TABLE's columns start, as the OCR test sheet's do, in points.
const TABLE_AT: [u32; 5] = [68, 125, 284, 363, 435];

/// The lines of [`rows_image`]: SHORT_AFTER_LONG, then TABLE's rows.
fn rows() -> Vec<String> {
    let table = TABLE.iter().map(|r| r.join(" "));
    (SHORT_AFTER_LONG.iter().map(|l| l.to_string()))
        .chain(table)
        .collect()
}

/// SHORT_AFTER_LONG, 11 pt on 14 pt, and TABLE a line below, its rows 22 pt apart,
/// [`tilted`] `deg` degrees. Closer, LimarePDF reads its rows apart only just, and not with
/// the page straightened a little off.
fn rows_image(deg: f32) -> GrayImage {
    let mut content = String::from("BT /F1 11 Tf\n");
    for (i, l) in SHORT_AFTER_LONG.iter().enumerate() {
        content.push_str(&format!("1 0 0 1 60 {} Tm ({l}) Tj\n", 220 - 14 * i));
    }
    for (i, row) in TABLE.iter().enumerate() {
        let y = 164 - 22 * i;
        for (x, cell) in TABLE_AT.iter().zip(row) {
            content.push_str(&format!("1 0 0 1 {x} {y} Tm ({cell}) Tj\n"));
        }
    }
    content.push_str("ET");
    tilted(deg, &content)
}

/// A `w` x `h` point page drawing `content`, which names Helvetica /F1.
fn helvetica_page(w: f32, h: f32, content: String) -> Pdf {
    let mut p = Pdf::new();
    p.insert_blank(0, w, h).unwrap();
    let page = p.page_ids()[0];
    let font = p.doc.add_object(dictionary! {
        "Type" => "Font", "Subtype" => "Type1", "BaseFont" => "Helvetica", "Encoding" => "WinAnsiEncoding",
    });
    let cs = p
        .doc
        .add_object(Stream::new(Dictionary::new(), content.into_bytes()));
    let d = p.doc.get_dictionary_mut(page).unwrap();
    d.set("Contents", cs);
    d.set(
        "Resources",
        dictionary! { "Font" => dictionary! { "F1" => font } },
    );
    p
}

/// Lines in languages written with accented letters: French, Spanish, German, Polish, Czech
/// and Hungarian.
const ACCENTED: [&str; 6] = [
    "Le café ouvre à huit heures, près de la gare.",
    "El niño comió una manzana en el jardín.",
    "Die Brücke über den Fluss ist gesperrt, sagt Jörg.",
    "Łódź leży w środkowej Polsce, nad rzeką.",
    "Příliš žluťoučký kůň úpěl ďábelské ódy.",
    "Árvíztűrő tükörfúrógép és gyöngyös ősz.",
];

/// A 6 x 3 inch page of ACCENTED in a font LimarePDF embeds, as a greyscale image at 300 dpi.
fn accented_image() -> GrayImage {
    let mut p = Pdf::new();
    p.insert_blank(0, 432.0, 216.0).unwrap();
    for (i, l) in ACCENTED.iter().enumerate() {
        let y = 40.0 + 26.0 * i as f32;
        p.add_text(0, (16.0, y), l, FontChoice::default(), 12.0, [0.0; 3])
            .unwrap();
    }
    gray(&mut p, 300.0 / 72.0)
}

/// The words of `text` with a letter outside ASCII, end punctuation stripped.
fn accented_words(text: &str) -> Vec<String> {
    let strip = |w: &str| w.trim_matches(|c: char| ".,;:".contains(c)).to_string();
    (text.split_whitespace())
        .filter(|w| !w.is_ascii())
        .map(strip)
        .collect()
}

/// Whether `text` has four in five of ACCENTED's words with a letter outside ASCII. The
/// bundled model reads 22 of the 24: the font draws the carons of Czech ť and ď like
/// apostrophes, and they are read as t and d'.
fn reads_accented(text: &str) -> bool {
    let got = accented_words(text);
    let want = accented_words(&ACCENTED.join("\n"));
    let hits = want.iter().filter(|w| got.contains(w)).count();
    hits * 5 >= want.len() * 4
}

fn gray(p: &mut Pdf, scale: f32) -> GrayImage {
    let hp = render::load(p.snapshot().unwrap(), None).unwrap();
    let b = render::Renderer::new(&hp).render(0, scale).unwrap();
    GrayImage::from_fn(b.width as u32, b.height as u32, |x, y| {
        Luma([b.rgba[(y as usize * b.width + x as usize) * 4]])
    })
}

/// A scanned page: the image as the page's only content.
fn scanned(img: GrayImage) -> Pdf {
    scans(vec![img])
}

/// Scanned pages, each image a page's only content.
fn scans(imgs: Vec<GrayImage>) -> Pdf {
    let pngs: Vec<Vec<u8>> = (imgs.into_iter())
        .map(|img| {
            let mut png = Cursor::new(Vec::new());
            DynamicImage::ImageLuma8(img)
                .write_to(&mut png, image::ImageFormat::Png)
                .unwrap();
            png.into_inner()
        })
        .collect();
    let opts = ImageImport {
        dpi: Some(300.0),
        ..Default::default()
    };
    Pdf::from_images(&pngs, &opts).unwrap()
}

/// A white page, scanned.
fn blank_scan() -> Pdf {
    scanned(GrayImage::from_pixel(600, 600, Luma([255])))
}

/// A dictionary of page 1's resources, resolved.
fn resources(p: &Pdf, cat: &str) -> Dictionary {
    let page = p.page_ids()[0];
    let res = p.doc.get_dictionary(page).unwrap().get(b"Resources").ok();
    let dict = |o: Option<&Object>| {
        (o.and_then(|o| p.doc.dereference(o).ok()))
            .and_then(|(_, o)| o.as_dict().ok().cloned())
            .unwrap_or_default()
    };
    dict(dict(res).get(cat.as_bytes()).ok())
}

/// The names in page 1's resources of category `cat` (`Font`, `XObject`).
fn names(p: &Pdf, cat: &str) -> Vec<String> {
    (resources(p, cat).iter())
        .map(|(k, _)| String::from_utf8_lossy(k).into_owned())
        .collect()
}

/// `obj` put in page 1's resources of category `cat` as `name`.
fn set_resource(p: &mut Pdf, cat: &str, name: &str, obj: ObjectId) {
    let page = p.page_ids()[0];
    let mut d = resources(p, cat);
    d.set(name, obj);
    let mut res = Dictionary::new();
    for c in ["Font", "XObject"] {
        res.set(c, resources(p, c));
    }
    res.set(cat, d);
    let page = p.doc.get_dictionary_mut(page).unwrap();
    page.set("Resources", res);
}

fn helvetica(p: &mut Pdf) -> ObjectId {
    p.doc.add_object(dictionary! {
        "Type" => "Font", "Subtype" => "Type1", "BaseFont" => "Helvetica", "Encoding" => "WinAnsiEncoding",
    })
}

/// `content` drawn on top of page 1, which names Helvetica /LF1.
fn draw(p: &mut Pdf, content: &str) {
    let font = helvetica(p);
    set_resource(p, "Font", "LF1", font);
    let page = p.page_ids()[0];
    let more = Stream::new(Dictionary::new(), content.as_bytes().to_vec());
    let more = p.doc.add_object(more);
    let d = p.doc.get_dictionary_mut(page).unwrap();
    let mut contents = match d.get(b"Contents").unwrap().clone() {
        Object::Array(a) => a,
        one => vec![one],
    };
    contents.push(more.into());
    d.set("Contents", contents);
}

/// A form drawing `content`, which names Helvetica /LF1, and `xobjects`.
fn form(p: &mut Pdf, content: &str, xobjects: Dictionary) -> ObjectId {
    let font = helvetica(p);
    let dict = dictionary! {
        "Type" => "XObject", "Subtype" => "Form", "BBox" => vec![0.into(), 0.into(), 1000.into(), 1000.into()],
        "Resources" => dictionary! { "Font" => dictionary! { "LF1" => font }, "XObject" => xobjects },
    };
    (p.doc).add_object(Stream::new(dict, content.as_bytes().to_vec()))
}

/// A page number, a Bates number or a stamp: text of the page's own at its foot.
const STAMP: &str = "BT /LF1 10 Tf 200 20 Td (LIM-000123) Tj ET";

fn skipped(r: &[(usize, OcrResult)], why: &str) -> bool {
    matches!(r, [(_, OcrResult::Skipped(w))] if w.contains(why))
}

/// Whether page 1 alone was read, to six lines, and left the way up it was.
fn six_lines(r: &[(usize, OcrResult)]) -> bool {
    matches!(r, [(0, OcrResult::Added { lines: 6, turned, .. })] if *turned == 0)
}

/// Share of the words in LINES found in `text`, ignoring end punctuation.
fn recall(text: &str) -> f32 {
    recall_of(&LINES, text)
}

/// Share of the words in `lines` found in `text`, ignoring end punctuation.
fn recall_of(lines: &[&str], text: &str) -> f32 {
    let strip = |w: &str| w.trim_matches(|c: char| ".,;:".contains(c)).to_string();
    let got: Vec<String> = text.split_whitespace().map(strip).collect();
    let want: Vec<String> = lines
        .iter()
        .flat_map(|l| l.split_whitespace())
        .map(strip)
        .collect();
    let hits = want.iter().filter(|w| got.contains(w)).count();
    hits as f32 / want.len() as f32
}

/// Page 1's turn on screen, clockwise (its `/Rotate`).
fn rotation(p: &Pdf) -> i64 {
    let page = p.doc.get_dictionary(p.page_ids()[0]).unwrap();
    page.get(b"Rotate").and_then(Object::as_i64).unwrap_or(0)
}

/// Whether the lines of `text` are LONG's, in order, each with four in five of its words.
fn long_lines_in_order(text: &str) -> bool {
    let read: Vec<&str> = text.lines().filter(|l| !l.trim().is_empty()).collect();
    read.len() == LONG.len()
        && (LONG.iter().zip(&read)).all(|(want, got)| recall_of(&[want], got) >= 0.8)
}

/// Whether the words of `text` are those of `lines`, in order, nine in ten in their place,
/// however its lines are broken: neither LimarePDF nor Poppler reads a table's row as one
/// line where its cells are far apart.
fn words_in_order(text: &str, lines: &[String]) -> bool {
    let strip = |w: &str| w.trim_matches(|c: char| ".,;:".contains(c)).to_string();
    let got: Vec<String> = text.split_whitespace().map(strip).collect();
    let want: Vec<String> = (lines.iter().flat_map(|l| l.split_whitespace()))
        .map(strip)
        .collect();
    let placed = got.iter().zip(&want).filter(|(g, w)| g == w).count();
    got.len() == want.len() && placed as f32 >= 0.9 * want.len() as f32
}

#[test]
fn adds_an_invisible_searchable_layer() {
    let ocr = Ocr::bundled().unwrap();
    let mut p = scanned(text_image());
    let before = gray(&mut p, 1.0);
    let r = p.ocr_pages(&[0], &ocr).unwrap();
    assert!(matches!(r[0].1, OcrResult::Added { lines: 6, .. }), "{r:?}");
    let text = p.page_text(0);
    assert!(recall(&text) > 0.9, "recall {}: {text}", recall(&text));
    let lines = text.lines().filter(|l| !l.trim().is_empty()).count();
    assert_eq!(lines, 6, "{text}");
    assert!(!p.search("committee").unwrap().is_empty());
    // Invisible: the page looks the same.
    let after = gray(&mut p, 1.0);
    assert_eq!(before.as_raw(), after.as_raw());
    // Survives a save and reload.
    let again = Pdf::from_bytes(&p.to_bytes().unwrap(), None).unwrap();
    assert!(recall(&again.page_text(0)) > 0.9);
}

#[test]
fn reads_accented_letters() {
    let ocr = Ocr::bundled().unwrap();
    let img = accented_image();
    let lines = ocr.read(img.as_raw(), img.width(), img.height()).unwrap();
    let read: Vec<String> = lines.iter().map(|l| l.text()).collect();
    assert_eq!(read.len(), 6, "{read:#?}");
    assert!(reads_accented(&read.join("\n")), "{read:#?}");
}

/// The accented letters read reach the text layer, those beyond Western European too: it
/// was written in WinAnsi, which has no Ł, ő or ř.
#[test]
fn accented_letters_reach_the_text_layer() {
    let ocr = Ocr::bundled().unwrap();
    let mut p = scanned(accented_image());
    p.ocr_pages(&[0], &ocr).unwrap();
    let text = p.page_text(0);
    assert!(reads_accented(&text), "{text}");
    for c in ['Ł', 'ő', 'ř'] {
        assert!(text.contains(c), "{c}: {text}");
    }
    let again = Pdf::from_bytes(&p.to_bytes().unwrap(), None).unwrap();
    assert_eq!(again.page_text(0), text);
}

/// Poppler reads the accented page's text layer as LimarePDF does.
/// `cargo test -p limarepdf-core --test all -- --ignored ocr::cross_check`.
#[test]
#[ignore]
fn cross_check_accented_text_layer_with_poppler() {
    let ocr = Ocr::bundled().unwrap();
    let mut p = scanned(accented_image());
    p.ocr_pages(&[0], &ocr).unwrap();
    let ours: Vec<String> = p.page_text(0).split_whitespace().map(Into::into).collect();
    let poppler = tools::pdftotext(&p.to_bytes().unwrap());
    let theirs: Vec<&str> = poppler.split_whitespace().collect();
    assert_eq!(theirs, ours, "{poppler}");
    assert!(poppler.contains('Ł') && poppler.contains('ő'), "{poppler}");
}

#[test]
fn a_page_asked_twice_is_read_once() {
    let ocr = Ocr::bundled().unwrap();
    let mut p = scanned(text_image());
    let r = p.ocr_pages(&[0, 0], &ocr).unwrap();
    assert!(
        matches!(r[..], [(0, OcrResult::Added { lines: 6, .. })]),
        "{r:?}"
    );
    assert_eq!(hidden_lines(&p).len(), 6);
}

/// The OCR (hidden) text lines of page 1, top to bottom.
fn hidden_lines(p: &Pdf) -> Vec<PageObject> {
    let mut v: Vec<PageObject> = p
        .page_objects(0)
        .unwrap()
        .into_iter()
        .filter(|o| o.kind == ObjectKind::Text)
        .collect();
    assert!(v.iter().all(|o| o.hidden), "OCR text should be hidden");
    v.sort_by(|a, b| a.bbox[1].total_cmp(&b.bbox[1]));
    v
}

/// Box of the topmost text line, in points from the top-left of the displayed page.
fn first_line_box(p: &mut Pdf) -> [f32; 4] {
    hidden_lines(p).first().expect("no text objects").bbox
}

#[test]
fn words_sit_where_they_are_printed() {
    let ocr = Ocr::bundled().unwrap();
    let mut p = scanned(text_image());
    p.ocr_pages(&[0], &ocr).unwrap();
    // The first line's baseline is at y = 190 - 18 = 172 pt (from the bottom), so its
    // text occupies roughly 32..46 pt from the top; it starts at x = 16 pt.
    let b = first_line_box(&mut p);
    assert!((b[0] - 16.0).abs() < 4.0, "{b:?}");
    assert!(b[1] > 26.0 && b[1] < 38.0, "{b:?}");
    assert!(b[3] > 40.0 && b[3] < 52.0, "{b:?}");
}

#[test]
fn rotated_page_is_read_upright() {
    let ocr = Ocr::bundled().unwrap();
    // Stored sideways, shown upright through /Rotate 90.
    let img = image::imageops::rotate270(&text_image());
    let mut p = scanned(img);
    let page = p.page_ids()[0];
    p.doc.get_dictionary_mut(page).unwrap().set("Rotate", 90);
    let (w, h) = p.page_size(0).unwrap();
    assert!((w - 288.0).abs() < 1.0, "{w} x {h}");
    assert!((h - 216.0).abs() < 1.0, "{w} x {h}");
    let r = p.ocr_pages(&[0], &ocr).unwrap();
    assert!(six_lines(&r), "{r:?}");
    assert_eq!(rotation(&p), 90);
    let text = p.page_text(0);
    assert!(recall(&text) > 0.9, "{text}");
    let b = first_line_box(&mut p);
    assert!((b[0] - 16.0).abs() < 4.0, "{b:?}");
    assert!(b[1] > 26.0 && b[1] < 38.0, "{b:?}");
}

/// A scan lying on its side or upside down is turned upright on screen, as Pages > Rotate
/// turns a page, and read. The turn and the text are one undo step (#225).
#[test]
fn turned_scans_are_turned_upright() {
    let ocr = Ocr::bundled().unwrap();
    let img = text_image();
    // Each scan, and the turn that puts it upright, clockwise.
    let scans = [
        (image::imageops::rotate90(&img), 270),
        (image::imageops::rotate180(&img), 180),
        (image::imageops::rotate270(&img), 90),
    ];
    for (scan, turn) in scans {
        let mut p = scanned(scan);
        let before = p.before();
        let r = p.ocr_pages(&[0], &ocr).unwrap();
        let added =
            matches!(r[..], [(0, OcrResult::Added { lines: 6, turned, .. })] if turned == turn);
        assert!(added, "{turn}: {r:?}");
        assert_eq!(rotation(&p), turn as i64);
        let (w, h) = p.page_size(0).unwrap();
        let size = (w - 288.0).abs() < 1.0 && (h - 216.0).abs() < 1.0;
        assert!(size, "{turn}: {w} x {h}");
        let text = p.page_text(0);
        assert!(recall(&text) > 0.9, "{turn}: {text}");
        let b = first_line_box(&mut p);
        assert!((b[0] - 16.0).abs() < 4.0, "{turn}: {b:?}");
        assert!(b[1] > 26.0 && b[1] < 38.0, "{turn}: {b:?}");
        // Upright now, it is read again as it is.
        let mut again = p.clone();
        let r = again.ocr_pages_again(&[0], &ocr).unwrap();
        assert!(six_lines(&r), "{turn}: {r:?}");
        assert_eq!(rotation(&again), turn as i64);
        let saved = Saved::of(&mut p);
        assert_eq!(saved.unreachable(), [], "{turn}");
        let reopened = Pdf::from_bytes(&saved.bytes, None).unwrap();
        assert_eq!(rotation(&reopened), turn as i64);
        assert!(recall(&reopened.page_text(0)) > 0.9, "{turn}");
        p.restore(before.undo(&p));
        assert_eq!(rotation(&p), 0, "{turn}");
        assert_eq!(p.page_text(0).trim(), "", "{turn}");
    }
}

/// A page with a few words isn't turned, however it lies: there is too little to tell which
/// way up it is. On its side, it was turned round as if upside down.
#[test]
fn a_page_of_a_few_words_is_not_turned() {
    let ocr = Ocr::bundled().unwrap();
    let mut page = Pdf::new();
    page.insert_blank(0, 288.0, 216.0).unwrap();
    let font = FontChoice::default();
    page.add_text(0, (40.0, 100.0), "Total 42.00", font, 12.0, [0.0; 3])
        .unwrap();
    let img = gray(&mut page, 300.0 / 72.0);
    let turns = [
        (img.clone(), 0),
        (image::imageops::rotate90(&img), 90),
        (image::imageops::rotate180(&img), 180),
    ];
    for (scan, turn) in turns {
        let mut p = scanned(scan);
        let r = p.ocr_pages(&[0], &ocr).unwrap();
        let left = match r[..] {
            [(0, OcrResult::Added { turned, .. })] => turned == 0,
            [(0, OcrResult::Skipped(_))] => turn == 90,
            _ => false,
        };
        assert!(left, "{turn}: {r:?}");
        assert_eq!(rotation(&p), 0, "{turn}");
        if turn == 0 {
            assert!(p.page_text(0).contains("Total"), "{}", p.page_text(0));
        }
    }
}

/// A page scanned a little tilted is read line by line without optimising it first: its
/// long lines were read two at once, letters of each in turn (#320). Tilted under 1°, the
/// page is read as it is, each line straightened in its picture; more, it is straightened
/// first. Each line's text is written level, at the height the printed line crosses the
/// middle of the page's text: Poppler reads sloped text in pieces. Tilted 2° or more, where
/// that shows, the report says how much.
#[test]
fn tilted_pages_are_read_line_by_line() {
    let ocr = Ocr::bundled().unwrap();
    let mut level = scanned(tilted_image(0.0));
    level.ocr_pages(&[0], &ocr).unwrap();
    let level = hidden_lines(&level);
    for deg in [2.5f32, -4.5, 0.7] {
        let mut p = scanned(tilted_image(deg));
        let looks = gray(&mut p, 1.0);
        let r = p.ocr_pages(&[0], &ocr).unwrap();
        assert!(six_lines(&r), "{deg}°: {r:?}");
        // Turned anticlockwise, it is reported tilted the other way from 2°.
        let tilted = match r[..] {
            [(0, OcrResult::Added { tilted, .. })] => tilted,
            _ => None,
        };
        let reported = match tilted {
            Some(t) => (t + deg).abs() < 0.3,
            None => deg.abs() < 2.0,
        };
        assert!(reported, "{deg}°: {r:?}");
        let text = p.page_text(0);
        assert!(long_lines_in_order(&text), "{deg}°: {text}");
        let copied = limarepdf_core::words_text(&p.page_words(0).unwrap());
        assert!(long_lines_in_order(&copied), "{deg}°: {copied}");
        let middle = |word: &str| {
            let b = p.find_text_boxes(0, word).unwrap();
            assert_eq!(b.len(), 1, "{deg}°, {word}: {b:?}");
            (b[0][1] + b[0][3]) / 2.0
        };
        let (y0, y1) = (middle("harbour"), middle("eight"));
        assert!((y1 - y0).abs() < 0.5, "{deg}°: {y0} and {y1}");
        // Where the middle of each line of the level page went, turned anticlockwise about
        // the middle of the page (y down).
        let (s, c) = deg.to_radians().sin_cos();
        let lines = hidden_lines(&p);
        assert_eq!(lines.len(), level.len(), "{deg}°");
        let centre = |b: [f32; 4]| ((b[0] + b[2]) / 2.0, (b[1] + b[3]) / 2.0);
        for (l, at) in lines.iter().zip(&level) {
            let (x, y) = centre(at.bbox);
            let (x, y) = (x - 306.0, y - 144.0);
            let want = (306.0 + c * x + s * y, 144.0 - s * x + c * y);
            let got = centre(l.bbox);
            let near = (got.0 - want.0).abs() < 3.0 && (got.1 - want.1).abs() < 3.0;
            assert!(near, "{deg}°: {got:?}, not {want:?}: {l:?}");
            // As big as the 11 pt print, so lines 14 pt apart don't overlap.
            let (size, (dx, dy)) = (l.font_size.unwrap(), l.dir.unwrap());
            let fits = (9.0..13.0).contains(&size) && (dx - 1.0).abs() < 1e-3;
            assert!(fits && dy.abs() < 1e-3, "{deg}°: {size} pt, {l:?}");
        }
        assert_eq!(looks.as_raw(), gray(&mut p, 1.0).as_raw(), "{deg}°");
    }
}

/// Poppler reads the tilted page's text layer line by line, as LimarePDF does. Written
/// sloping with the lines, its ends came out as lines of their own, and its words in pieces.
/// `cargo test -p limarepdf-core --test all -- --ignored ocr::cross_check`.
#[test]
#[ignore]
fn cross_check_tilted_text_layer_with_poppler() {
    let ocr = Ocr::bundled().unwrap();
    for deg in [2.5f32, -4.5, 0.7] {
        let mut p = scanned(tilted_image(deg));
        p.ocr_pages(&[0], &ocr).unwrap();
        let poppler = tools::pdftotext(&p.to_bytes().unwrap());
        assert!(long_lines_in_order(&poppler), "{deg}°: {poppler}");
    }
    for deg in [4.0f32, -4.0] {
        let mut p = scanned(rows_image(deg));
        p.ocr_pages(&[0], &ocr).unwrap();
        let poppler = tools::pdftotext(&p.to_bytes().unwrap());
        assert!(words_in_order(&poppler, &rows()), "{deg}°: {poppler}");
    }
}

/// A tilted page's lines are written at the height each crosses the middle of its text, so
/// they keep their order: each across its own middle, a short line after a long one came out
/// among the long one's words, and a table a column at a time (#320).
#[test]
fn tilted_short_lines_and_tables_keep_their_order() {
    let ocr = Ocr::bundled().unwrap();
    for deg in [0.0f32, 4.0, -4.0] {
        let mut p = scanned(rows_image(deg));
        p.ocr_pages(&[0], &ocr).unwrap();
        let text = p.page_text(0);
        assert!(words_in_order(&text, &rows()), "{deg}°: {text}");
        let copied = limarepdf_core::words_text(&p.page_words(0).unwrap());
        assert!(words_in_order(&copied, &rows()), "{deg}°: {copied}");
    }
}

/// Text another app wrote sloping with a tilted scan's lines, as Tesseract does, is read line
/// by line: tilted, the end of each line drops past the start of the next (#320).
#[test]
fn sloped_text_is_read_line_by_line() {
    for deg in [2.5f32, -4.5] {
        let (s, c) = deg.to_radians().sin_cos();
        let mut content = String::from("BT 3 Tr /F1 11 Tf\n");
        for (i, l) in LONG.iter().enumerate() {
            // Where tilted_image's line starts, turned anticlockwise about the page's middle.
            let (x, y) = (60.0 - 306.0, 200.0 - 14.0 * i as f32 - 144.0);
            let (x, y) = (306.0 + c * x - s * y, 144.0 + s * x + c * y);
            content.push_str(&format!("{c} {s} {} {c} {x} {y} Tm ({l}) Tj\n", -s));
        }
        content.push_str("ET");
        let p = helvetica_page(612.0, 288.0, content);
        let text = p.page_text(0);
        assert!(long_lines_in_order(&text), "{deg}°: {text}");
        let copied = limarepdf_core::words_text(&p.page_words(0).unwrap());
        assert!(long_lines_in_order(&copied), "{deg}°: {copied}");
    }
}

/// A tilted page's lines are edited, read again, saved, removed and undone as a straight
/// page's are (#320).
#[test]
fn tilted_text_is_edited_read_again_and_removed() {
    let ocr = Ocr::bundled().unwrap();
    let mut p = scanned(tilted_image(2.5));
    let looks = gray(&mut p, 1.0);
    p.ocr_pages(&[0], &ocr).unwrap();
    let line = hidden_lines(&p)
        .into_iter()
        .find(|o| o.text.as_deref().unwrap_or("").contains("harbour"))
        .expect("no line with harbour");
    let edit = TextEdit {
        text: "The harbour office opens at nine.".into(),
        font: None,
        size: None,
        color: None,
        parts: Vec::new(),
    };
    p.edit_text(0, line.id, &edit).unwrap();
    let edited = (hidden_lines(&p).into_iter())
        .find(|o| o.text.as_deref() == Some(edit.text.as_str()))
        .expect("no edited line");
    assert_eq!(edited.dir, line.dir);
    let r = p.ocr_pages_again(&[0], &ocr).unwrap();
    assert!(six_lines(&r), "{r:?}");
    let text = p.page_text(0);
    assert!(long_lines_in_order(&text), "{text}");
    let words = ["harbour", "Passengers", "bicycles", "charities"];
    Saved::of(&mut p).assert_kept(&words);
    let again = Pdf::from_bytes(&p.to_bytes().unwrap(), None).unwrap();
    let again = again.page_text(0);
    assert!(long_lines_in_order(&again), "{again}");
    let before = p.before();
    let r = p.remove_recognised_text(&[0]).unwrap();
    assert_eq!(r, [(0, OcrResult::Removed { lines: 6 })]);
    assert_eq!(p.page_text(0).trim(), "");
    assert_eq!(looks.as_raw(), gray(&mut p, 1.0).as_raw());
    let saved = Saved::of(&mut p);
    saved.assert_gone(&words);
    assert_eq!(saved.unreachable(), []);
    p.restore(before.undo(&p));
    assert!(long_lines_in_order(&p.page_text(0)), "{}", p.page_text(0));
}

#[test]
fn recognised_pages_are_skipped() {
    let ocr = Ocr::bundled().unwrap();
    let mut p = scanned(text_image());
    p.ocr_pages(&[0], &ocr).unwrap();
    let r = p.ocr_pages(&[0], &ocr).unwrap();
    assert!(skipped(&r, "already has recognised text"), "{r:?}");
}

/// A page of text that isn't a scan has its text already.
#[test]
fn pages_of_text_alone_are_skipped() {
    let ocr = Ocr::bundled().unwrap();
    let r = text_page().ocr_pages(&[0], &ocr).unwrap();
    assert!(skipped(&r, "already has text"), "{r:?}");
}

/// A scan with a little text of its own, such as a Bates number, is read, and what the page
/// already has isn't added again (#223). Its text and the text read are read together: the
/// number at the foot of the page comes last.
#[test]
fn scans_with_some_text_are_read() {
    let ocr = Ocr::bundled().unwrap();
    let mut p = scanned(text_image());
    draw(&mut p, STAMP);
    let before = gray(&mut p, 1.0);
    let r = p.ocr_pages(&[0], &ocr).unwrap();
    assert!(six_lines(&r), "{r:?}");
    let text = p.page_text(0);
    assert!(recall(&text) > 0.9, "recall {}: {text}", recall(&text));
    let stamps = p.find_text_boxes(0, "LIM-000123").unwrap();
    assert_eq!(stamps.len(), 1, "{text}");
    assert!(text.trim_end().ends_with("fence.\nLIM-000123"), "{text}");
    let words = p.page_words(0).unwrap();
    let copied = limarepdf_core::words_text(&words);
    assert!(copied.contains("Invoice"), "{copied}");
    assert!(copied.ends_with("LIM-000123"), "{copied}");
    assert_eq!(before.as_raw(), gray(&mut p, 1.0).as_raw());
}

/// A page of text above bar charts is read: the bars, solid and holding more of the ink
/// than the letters, were taken for the page's letter size, and no text was found.
#[test]
fn text_beside_a_bar_chart_is_read() {
    let mut content = String::from("BT /F1 12 Tf 16 400 Td 18 TL\n");
    for l in LINES {
        content.push_str(&format!("({l}) '\n"));
    }
    content.push_str("ET\n");
    // Wide bars, and thin ones.
    for (i, h) in [25, 40, 55, 35, 60].iter().enumerate() {
        content.push_str(&format!("{} 40 40 {h} re f\n", 30 + 50 * i));
    }
    for i in 0..12 {
        content.push_str(&format!("{} 130 8 {} re f\n", 30 + 20 * i, 20 + 4 * i));
    }
    let mut page = helvetica_page(288.0, 432.0, content);
    let ocr = Ocr::bundled().unwrap();
    let mut p = scanned(gray(&mut page, 300.0 / 72.0));
    let r = p.ocr_pages(&[0], &ocr).unwrap();
    assert!(six_lines(&r), "{r:?}");
    let text = p.page_text(0);
    assert!(recall(&text) > 0.9, "recall {}: {text}", recall(&text));
}

/// Words in `text` that LINES hasn't, ignoring end punctuation and words read in two
/// pieces: marks read as text.
fn junk(text: &str) -> Vec<String> {
    let strip = |w: &str| w.trim_matches(|c: char| ".,;:".contains(c)).to_string();
    let want: Vec<String> = (LINES.iter())
        .flat_map(|l| l.split_whitespace())
        .map(strip)
        .collect();
    let read: Vec<String> = text.split_whitespace().map(strip).collect();
    let mut junk = Vec::new();
    let mut i = 0;
    while i < read.len() {
        if want.contains(&read[i]) {
            i += 1;
        } else if read
            .get(i + 1)
            .is_some_and(|n| want.contains(&(read[i].clone() + n)))
        {
            i += 2;
        } else {
            junk.push(read[i].clone());
            i += 1;
        }
    }
    junk
}

/// LINES printed over a background, as security printing lays text on cheques and
/// certificates: `background` draws it first, in grey.
fn over_background(background: &str) -> GrayImage {
    let mut content = format!("q {background} Q\nBT /F1 12 Tf 16 190 Td 18 TL\n");
    for l in LINES {
        content.push_str(&format!("({l}) '\n"));
    }
    content.push_str("ET");
    gray(&mut helvetica_page(288.0, 216.0, content), 300.0 / 72.0)
}

/// LINES are read over `background`, and it isn't read as text.
fn read_over(background: &str) {
    let ocr = Ocr::bundled().unwrap();
    let mut p = scanned(over_background(background));
    let r = p.ocr_pages(&[0], &ocr).unwrap();
    let text = p.page_text(0);
    assert!(six_lines(&r), "{r:?}: {text}");
    assert!(recall(&text) > 0.9, "recall {}: {text}", recall(&text));
    let junk = junk(&text);
    assert!(junk.len() <= 1, "junk {junk:?}: {text}");
}

/// Text printed over crossing grey waves, a guilloche lattice, is read: joined to the
/// letters, the waves made one group the size of the page, dropped with every letter in it
/// (#322).
#[test]
fn text_over_a_wave_lattice_is_read() {
    let mut waves = String::from("0.55 G 0.5 w\n");
    for sign in [1.0, -1.0] {
        for k in -2..76 {
            for i in 0..=144 {
                let x = i as f32 * 2.0;
                let a = x / 34.0 * std::f32::consts::TAU + k as f32 * 0.6;
                let y = k as f32 * 3.0 + sign * 2.0 * a.sin();
                let op = if i == 0 { "m" } else { "l" };
                waves.push_str(&format!("{x} {y:.2} {op}\n"));
            }
            waves.push_str("S\n");
        }
    }
    read_over(&waves);
}

/// Text printed over a grey dot screen is read: filling the gaps along rows joined the dots
/// into rules that tied the letters together (#322).
#[test]
fn text_over_a_dot_screen_is_read() {
    let mut dots = String::from("0.5 g\n");
    for j in 0..144 {
        for i in 0..192 {
            dots.push_str(&format!(
                "{} {} 0.5 0.5 re\n",
                i as f32 * 1.5,
                j as f32 * 1.5
            ));
        }
    }
    dots.push('f');
    read_over(&dots);
}

/// Read again, a page's recognised text is replaced: a line corrected wrongly reads as the
/// page shows it once more (#224).
#[test]
fn pages_read_again_have_their_recognised_text_replaced() {
    let ocr = Ocr::bundled().unwrap();
    let mut p = scanned(text_image());
    p.ocr_pages(&[0], &ocr).unwrap();
    let before = gray(&mut p, 1.0);
    let line = hidden_lines(&p)
        .into_iter()
        .find(|o| o.text.as_deref().unwrap_or("").contains("Tuesday"))
        .expect("no line with Tuesday");
    let edit = TextEdit {
        text: "The committee met on Wednesday morning.".into(),
        font: None,
        size: None,
        color: None,
        parts: Vec::new(),
    };
    p.edit_text(0, line.id, &edit).unwrap();
    let r = p.ocr_pages_again(&[0], &ocr).unwrap();
    assert!(six_lines(&r), "{r:?}");
    let text = p.page_text(0);
    assert!(text.contains("Tuesday"), "{text}");
    assert!(!text.contains("Wednesday"), "{text}");
    assert_eq!(hidden_lines(&p).len(), 6);
    assert_eq!(before.as_raw(), gray(&mut p, 1.0).as_raw());
}

/// A page read again that gives nothing keeps the text it had: another app's, here.
#[test]
fn pages_read_again_to_no_avail_keep_their_text() {
    let ocr = Ocr::bundled().unwrap();
    let mut p = blank_scan();
    draw(&mut p, "BT 3 Tr /LF1 12 Tf 20 100 Td (Hello there) Tj ET");
    let r = p.ocr_pages(&[0], &ocr).unwrap();
    assert!(skipped(&r, "already has recognised text"), "{r:?}");
    let r = p.ocr_pages_again(&[0], &ocr).unwrap();
    assert!(skipped(&r, "no text found"), "{r:?}");
    assert_eq!(p.page_text(0).trim(), "Hello there");
}

/// Removing a page's recognised text leaves the page as it looks, with its own text, and
/// without the fonts only the recognised text used, and the saved file holds none of the
/// words read. The page can then be recognised again (#224).
#[test]
fn recognised_text_can_be_removed() {
    let ocr = Ocr::bundled().unwrap();
    let mut p = scanned(text_image());
    draw(&mut p, STAMP);
    p.ocr_pages(&[0], &ocr).unwrap();
    let before = gray(&mut p, 1.0);
    let text = p.page_text(0);
    let words = long_words(&text);
    assert!(words.len() >= 8, "{words:?}");
    Saved::of(&mut p).assert_kept(&words);
    let r = p.remove_recognised_text(&[0]).unwrap();
    assert_eq!(r, [(0, OcrResult::Removed { lines: 6 })]);
    assert_eq!(p.page_text(0).trim(), "LIM-000123");
    assert_eq!(before.as_raw(), gray(&mut p, 1.0).as_raw());
    assert_eq!(names(&p, "Font"), ["LF1"]);
    let saved = Saved::of(&mut p);
    saved.assert_gone(&words);
    saved.assert_kept(&["LIM-000123"]);
    assert_eq!(saved.unreachable(), []);
    let r = p.ocr_pages(&[0], &ocr).unwrap();
    assert!(six_lines(&r), "{r:?}");
}

/// Other apps' recognised text is removed too: invisible text in the page's content (as
/// Tesseract and Acrobat write it) and a form drawing only that (as ocrmypdf adds it). The
/// form goes from the page's resources. Invisible text that clips is kept: without it, what
/// it clips would show.
#[test]
fn other_apps_recognised_text_is_removed() {
    let mut p = blank_scan();
    let layer = form(
        &mut p,
        "BT 3 Tr /LF1 12 Tf 20 60 Td (Grafted words) Tj ET",
        Dictionary::new(),
    );
    set_resource(&mut p, "XObject", "OCR1", layer);
    draw(
        &mut p,
        "BT 3 Tr /LF1 12 Tf 20 100 Td (Invisible words) Tj ET q /OCR1 Do Q \
         BT 7 Tr /LF1 12 Tf 20 20 Td (Clip) Tj ET",
    );
    let r = p.remove_recognised_text(&[0]).unwrap();
    assert_eq!(r, [(0, OcrResult::Removed { lines: 2 })]);
    assert_eq!(p.page_text(0).trim(), "Clip");
    assert_eq!(names(&p, "XObject").len(), 1, "{:?}", names(&p, "XObject"));
    assert!(!names(&p, "XObject").contains(&"OCR1".into()));
    assert_eq!(names(&p, "Font"), ["LF1"]);
    let saved = Saved::of(&mut p);
    saved.assert_gone(&["Grafted", "Invisible"]);
    saved.assert_kept(&["Clip"]);
    let r = p.remove_recognised_text(&[0]).unwrap();
    assert!(skipped(&r, "no recognised text"), "{r:?}");
}

/// Poppler reads only the page's own text once the recognised text is removed.
/// `cargo test -p limarepdf-core --test all -- --ignored ocr::cross_check`.
#[test]
#[ignore]
fn cross_check_removed_text_with_poppler() {
    let ocr = Ocr::bundled().unwrap();
    let mut p = scanned(text_image());
    draw(&mut p, STAMP);
    p.ocr_pages(&[0], &ocr).unwrap();
    p.remove_recognised_text(&[0]).unwrap();
    let poppler = tools::pdftotext(&p.to_bytes().unwrap());
    let words: Vec<&str> = poppler.split_whitespace().collect();
    assert_eq!(words, ["LIM-000123"], "{poppler}");
}

/// Paper `size` points large with no margins, `pages` a sheet: one page as large prints as
/// it is.
fn paper(size: (f32, f32), pages: u8) -> PageSetup {
    PageSetup {
        paper: Paper {
            id: "page".into(),
            label: "page".into(),
            size,
            margins: Some([0.0; 4]),
        },
        orientation: Orientation::Automatic,
        scale: Scale::ShrinkOversized,
        per_sheet: PerSheet {
            pages,
            ..PerSheet::default()
        },
        booklet: None,
    }
}

/// `p`'s first page printed as it is, as an app arranging pages puts it: the sheet draws the
/// page, its scan and its recognised text, as a form.
fn printed(p: &Pdf) -> Pdf {
    p.print_sheets(&[0], &paper((288.0, 216.0), 1)).unwrap()
}

/// The words of six letters or more in `text`: what recognised text holds, to look for in a
/// saved file.
fn long_words(text: &str) -> Vec<&str> {
    (text.split_whitespace())
        .filter(|w| w.len() >= 6 && w.chars().all(char::is_alphabetic))
        .collect()
}

/// Recognised text inside a form that draws more, such as a whole page put in a form by an
/// app arranging pages, is removed (#327): the page draws a copy of the form without it. The
/// page looks as it did, and the saved file holds none of the words read, the form itself
/// gone with nothing else drawing it.
#[test]
fn recognised_text_in_a_form_drawing_more_is_removed() {
    let ocr = Ocr::bundled().unwrap();
    let mut p = scanned(text_image());
    p.ocr_pages(&[0], &ocr).unwrap();
    let mut p = printed(&p);
    let before = gray(&mut p, 1.0);
    let text = p.page_text(0);
    let words = long_words(&text);
    assert!(words.len() >= 8, "{words:?}");
    let r = p.remove_recognised_text(&[0]).unwrap();
    assert_eq!(r, [(0, OcrResult::Removed { lines: 6 })]);
    assert_eq!(p.page_text(0).trim(), "");
    assert_eq!(before.as_raw(), gray(&mut p, 1.0).as_raw());
    let saved = Saved::of(&mut p);
    saved.assert_gone(&words);
    assert_eq!(saved.unreachable(), []);
    let r = p.ocr_pages(&[0], &ocr).unwrap();
    assert!(six_lines(&r), "{r:?}");
}

/// Poppler reads no words on a printed page once its recognised text is removed.
/// `cargo test -p limarepdf-core --test all -- --ignored ocr::cross_check`.
#[test]
#[ignore]
fn cross_check_text_removed_from_a_form_with_poppler() {
    let ocr = Ocr::bundled().unwrap();
    let mut p = scanned(text_image());
    p.ocr_pages(&[0], &ocr).unwrap();
    let mut p = printed(&p);
    p.remove_recognised_text(&[0]).unwrap();
    let poppler = tools::pdftotext(&p.to_bytes().unwrap());
    assert_eq!(poppler.trim(), "", "{poppler}");
}

/// Read again, recognised text inside a form that draws more is replaced, not doubled: a
/// line corrected wrongly reads as the scan shows it once more (#327).
#[test]
fn recognised_text_in_a_form_drawing_more_is_read_again() {
    let ocr = Ocr::bundled().unwrap();
    let mut p = printed(&misread_tuesday(&ocr));
    let before = gray(&mut p, 1.0);
    let r = p.ocr_pages(&[0], &ocr).unwrap();
    assert!(skipped(&r, "already has recognised text"), "{r:?}");
    let r = p.ocr_pages_again(&[0], &ocr).unwrap();
    assert!(six_lines(&r), "{r:?}");
    let text = p.page_text(0);
    assert!(
        text.contains("Tuesday") && !text.contains("Wednesday"),
        "{text}"
    );
    let committee = p.find_text_boxes(0, "committee").unwrap();
    assert_eq!(committee.len(), 1, "{text}");
    assert!(recall(&text) > 0.9, "recall {}: {text}", recall(&text));
    assert_eq!(before.as_raw(), gray(&mut p, 1.0).as_raw());
    let saved = Saved::of(&mut p);
    saved.assert_gone(&["Wednesday"]);
    assert_eq!(saved.unreachable(), []);
}

/// Pages printed two to a sheet, each drawn as a form of its own, lose the recognised text
/// of both.
#[test]
fn pages_printed_two_to_a_sheet_lose_their_recognised_text() {
    let ocr = Ocr::bundled().unwrap();
    let mut p = scans(vec![text_image(), text_image()]);
    p.ocr_pages(&[0, 1], &ocr).unwrap();
    let mut p = p.print_sheets(&[0, 1], &paper((576.0, 216.0), 2)).unwrap();
    assert_eq!(p.page_count(), 1);
    let before = gray(&mut p, 1.0);
    let text = p.page_text(0);
    let words = long_words(&text);
    assert!(words.len() >= 16, "{words:?}");
    let r = p.remove_recognised_text(&[0]).unwrap();
    assert_eq!(r, [(0, OcrResult::Removed { lines: 12 })]);
    assert_eq!(p.page_text(0).trim(), "");
    assert_eq!(before.as_raw(), gray(&mut p, 1.0).as_raw());
    let saved = Saved::of(&mut p);
    saved.assert_gone(&words);
    assert_eq!(saved.unreachable(), []);
}

/// Page `i`'s name for picture or form `name`.
fn drawn(p: &Pdf, i: usize, name: &str) -> Option<ObjectId> {
    let page = p.doc.get_dictionary(p.page_ids()[i]).ok()?;
    let res = page.get_deref(b"Resources", &p.doc).ok()?.as_dict().ok()?;
    let xobjects = res.get_deref(b"XObject", &p.doc).ok()?.as_dict().ok()?;
    xobjects.get(name.as_bytes()).ok()?.as_reference().ok()
}

/// A form drawing page 1, [`text_image`]'s scan, and recognised text of its own: "Hidden
/// words".
fn scan_and_words(p: &mut Pdf) -> ObjectId {
    let page = p.page_ids()[0];
    let content = String::from_utf8(p.doc.get_page_content(page)).unwrap();
    let content = format!("{content}\nBT 3 Tr /LF1 12 Tf 20 100 Td (Hidden words) Tj ET");
    let xobjects = resources(p, "XObject");
    form(p, &content, xobjects)
}

/// `content` as page `i`'s, drawing `forms` by their names.
fn draw_forms(p: &mut Pdf, i: usize, content: &str, forms: &[(&str, ObjectId)]) {
    let page = p.page_ids()[i];
    let content = Stream::new(Dictionary::new(), content.as_bytes().to_vec());
    let content = p.doc.add_object(content);
    let mut xobjects = Dictionary::new();
    for &(name, id) in forms {
        xobjects.set(name, id);
    }
    let d = p.doc.get_dictionary_mut(page).unwrap();
    d.set("Contents", content);
    d.set("Resources", dictionary! { "XObject" => xobjects });
}

/// A form is copied only for the page its recognised text is taken off: drawn twice there,
/// both draw the copy, and another page drawing it keeps it as it was, text and all.
#[test]
fn a_form_drawn_elsewhere_too_keeps_its_recognised_text_there() {
    let mut p = scanned(text_image());
    let whole = scan_and_words(&mut p);
    let (w, h) = p.page_size(0).unwrap();
    p.insert_blank(1, w, h).unwrap();
    let twice = "q /Page1 Do Q q 0.5 0 0 0.5 0 0 cm /Page1 Do Q";
    draw_forms(&mut p, 0, twice, &[("Page1", whole)]);
    draw_forms(&mut p, 1, "q /Page1 Do Q", &[("Page1", whole)]);
    let original = p.doc.get_object(whole).unwrap().clone();
    let before = gray(&mut p, 1.0);
    let r = p.remove_recognised_text(&[0]).unwrap();
    assert_eq!(r, [(0, OcrResult::Removed { lines: 2 })]);
    assert_eq!(p.page_text(0).trim(), "");
    assert_eq!(p.page_text(1).trim(), "Hidden words");
    assert_eq!(before.as_raw(), gray(&mut p, 1.0).as_raw());
    assert_eq!(drawn(&p, 1, "Page1"), Some(whole));
    assert_eq!(p.doc.get_object(whole).unwrap(), &original);
    let saved = Saved::of(&mut p);
    saved.assert_kept(&["Hidden"]);
    assert_eq!(saved.unreachable(), []);
    let r = p.remove_recognised_text(&[1]).unwrap();
    assert_eq!(r, [(1, OcrResult::Removed { lines: 1 })]);
    assert_eq!(p.page_text(1).trim(), "");
    let saved = Saved::of(&mut p);
    saved.assert_gone(&["Hidden"]);
    assert_eq!(saved.unreachable(), []);
}

/// Form `id`'s resources.
fn form_res(p: &Pdf, id: ObjectId) -> &Dictionary {
    let s = p.doc.get_object(id).and_then(Object::as_stream).unwrap();
    let res = s.dict.get_deref(b"Resources", &p.doc);
    res.and_then(Object::as_dict).unwrap()
}

/// The names in form `id`'s resources of category `cat` (`Font`, `XObject`).
fn form_names(p: &Pdf, id: ObjectId, cat: &str) -> Vec<String> {
    let res = form_res(p, id);
    let d = (res.get_deref(cat.as_bytes(), &p.doc).ok()).and_then(|o| o.as_dict().ok());
    (d.into_iter().flat_map(|d| d.iter()))
        .map(|(k, _)| String::from_utf8_lossy(k).into_owned())
        .collect()
}

/// A blue square, drawn in a form beside its recognised text.
const SQUARE: &str = "0 0 1 rg 10 10 20 20 re f";

/// Recognised text is taken out of forms inside forms too: a form drawing nothing else isn't
/// drawn, and one drawing more is drawn as a copy, by the copy of the form drawing it. Each
/// copy keeps only the fonts and forms it still uses.
#[test]
fn recognised_text_in_forms_inside_forms_is_removed() {
    let mut p = scanned(text_image());
    let only = "BT 3 Tr /LF1 12 Tf 20 40 Td (Nested words) Tj ET";
    let only = form(&mut p, only, Dictionary::new());
    let more = format!("{SQUARE} BT 3 Tr /LF1 12 Tf 20 60 Td (Inner words) Tj ET");
    let more = form(&mut p, &more, Dictionary::new());
    let outer = "BT /LF1 12 Tf 20 120 Td (Visible words) Tj ET \
                 BT 3 Tr /LF2 12 Tf 20 100 Td (Hidden words) Tj ET q /Only Do Q q /More Do Q";
    let forms = dictionary! { "Only" => only, "More" => more };
    let outer = form(&mut p, outer, forms);
    let hidden = helvetica(&mut p);
    let s = p.doc.get_object_mut(outer).and_then(Object::as_stream_mut);
    let res = (s.unwrap().dict.get_mut(b"Resources").unwrap()).as_dict_mut();
    let res = res.unwrap();
    (res.get_mut(b"Font").unwrap().as_dict_mut().unwrap()).set("LF2", hidden);
    set_resource(&mut p, "XObject", "Outer", outer);
    draw(&mut p, "q /Outer Do Q");
    let before = gray(&mut p, 1.0);
    let r = p.remove_recognised_text(&[0]).unwrap();
    assert_eq!(r, [(0, OcrResult::Removed { lines: 3 })]);
    assert_eq!(p.page_text(0).trim(), "Visible words");
    assert_eq!(before.as_raw(), gray(&mut p, 1.0).as_raw());
    assert!(!names(&p, "XObject").contains(&"Outer".into()));
    let copy = (resources(&p, "XObject").iter())
        .filter_map(|(_, o)| o.as_reference().ok())
        .find(|&id| {
            let s = p.doc.get_object(id).and_then(Object::as_stream);
            s.unwrap().dict.has(b"Resources")
        })
        .expect("no copy of the outer form");
    assert_eq!(form_names(&p, copy, "Font"), ["LF1"]);
    let inner = form_names(&p, copy, "XObject");
    let renamed = matches!(&inner[..], [n] if n != "Only" && n != "More");
    assert!(renamed, "{inner:?}");
    let inner = drawn_by(&p, copy, &inner[0]);
    assert_eq!(form_names(&p, inner, "Font"), Vec::<String>::new());
    let saved = Saved::of(&mut p);
    saved.assert_gone(&["Hidden", "Nested", "Inner"]);
    saved.assert_kept(&["Visible"]);
    assert_eq!(saved.unreachable(), []);
}

/// Form `id`'s name for picture or form `name`.
fn drawn_by(p: &Pdf, id: ObjectId, name: &str) -> ObjectId {
    let xobjects = (form_res(p, id).get_deref(b"XObject", &p.doc)).and_then(Object::as_dict);
    let id = xobjects.unwrap().get(name.as_bytes());
    id.and_then(Object::as_reference).unwrap()
}

/// A form with no resources of its own, which draws with its page's, keeps drawing with them
/// copied without its recognised text.
#[test]
fn a_form_using_its_pages_resources_still_does_when_copied() {
    let mut p = scanned(text_image());
    let content = format!("{SQUARE} BT 3 Tr /LF1 12 Tf 20 80 Td (Borrowed words) Tj ET");
    let dict = dictionary! {
        "Type" => "XObject", "Subtype" => "Form", "BBox" => vec![0.into(), 0.into(), 1000.into(), 1000.into()],
    };
    let borrowing = p.doc.add_object(Stream::new(dict, content.into_bytes()));
    set_resource(&mut p, "XObject", "Borrowing", borrowing);
    draw(&mut p, "q /Borrowing Do Q");
    let before = gray(&mut p, 1.0);
    let r = p.remove_recognised_text(&[0]).unwrap();
    assert_eq!(r, [(0, OcrResult::Removed { lines: 1 })]);
    assert_eq!(p.page_text(0).trim(), "");
    assert_eq!(before.as_raw(), gray(&mut p, 1.0).as_raw());
    let copies: Vec<ObjectId> = (resources(&p, "XObject").iter())
        .filter_map(|(_, o)| o.as_reference().ok())
        .filter(|&id| id != borrowing && p.doc.get_object(id).unwrap().as_stream().is_ok())
        .filter(|&id| {
            let s = p.doc.get_object(id).unwrap().as_stream().unwrap();
            s.dict.get(b"Subtype").and_then(Object::as_name).ok() == Some(b"Form")
        })
        .collect();
    assert_eq!(copies.len(), 1, "{copies:?}");
    let copy = p.doc.get_object(copies[0]).unwrap().as_stream().unwrap();
    assert!(!copy.dict.has(b"Resources"));
    let saved = Saved::of(&mut p);
    saved.assert_gone(&["Borrowed"]);
    assert_eq!(saved.unreachable(), []);
}

#[test]
fn blank_page_has_no_text() {
    let ocr = Ocr::bundled().unwrap();
    let mut p = scanned(GrayImage::from_pixel(600, 600, Luma([255])));
    let r = p.ocr_pages(&[0], &ocr).unwrap();
    let none = matches!(&r[0].1, OcrResult::Skipped(w) if w.contains("no text"));
    assert!(none, "{r:?}");
}

#[test]
fn search_boxes_cover_recognised_words() {
    let ocr = Ocr::bundled().unwrap();
    let mut p = scanned(text_image());
    p.ocr_pages(&[0], &ocr).unwrap();
    // Line 3, "The committee met on Tuesday morning.", has its baseline at 136 pt from
    // the bottom of a 216 pt page: 80 pt from the top. "committee" starts after "The ",
    // about 38 pt from the left.
    let b = p.find_text_boxes(0, "COMMITTEE").unwrap();
    assert_eq!(b.len(), 1, "{b:?}");
    let b = b[0];
    assert!((b[0] - 38.0).abs() < 6.0, "{b:?}");
    assert!(b[1] > 64.0 && b[1] < 76.0, "{b:?}");
    assert!(b[3] > 78.0 && b[3] < 88.0, "{b:?}");
    assert!(p.find_text_boxes(0, "zebra").unwrap().is_empty());
}

#[test]
fn recognised_words_can_be_selected() {
    let ocr = Ocr::bundled().unwrap();
    let mut p = scanned(text_image());
    p.ocr_pages(&[0], &ocr).unwrap();
    let words = p.page_words(0).unwrap();
    let want: usize = LINES.iter().map(|l| l.split_whitespace().count()).sum();
    let found = words.len();
    assert!(found * 10 >= want * 9, "{found} of {want} words");
    let text = limarepdf_core::words_text(&words);
    assert_eq!(text.lines().count(), 6, "{text}");
    assert!(recall(&text) > 0.9, "{text}");
    // The first word starts the first line: x = 16 pt, about 32..46 pt from the top.
    let first = &words[0];
    assert!((first.bbox[0] - 16.0).abs() < 4.0, "{first:?}");
    assert!(first.bbox[1] > 26.0 && first.bbox[3] < 52.0, "{first:?}");
}

#[test]
fn correcting_recognised_text_keeps_it_invisible_and_in_place() {
    let ocr = Ocr::bundled().unwrap();
    let mut p = scanned(text_image());
    p.ocr_pages(&[0], &ocr).unwrap();
    let before = gray(&mut p, 1.0);
    let line = hidden_lines(&p)
        .into_iter()
        .find(|o| o.text.as_deref().unwrap_or("").contains("Tuesday"))
        .expect("no line with Tuesday");
    // Non-ASCII text would take the "append new visible text" path for normal text; Ł
    // is beyond Western European.
    let edit = TextEdit {
        text: "The committee met on Wednesday morning, café, Łódź.".into(),
        font: None,
        size: None,
        color: None,
        parts: Vec::new(),
    };
    p.edit_text(0, line.id, &edit).unwrap();
    let after = gray(&mut p, 1.0);
    assert_eq!(before.as_raw(), after.as_raw(), "text became visible");
    assert!(p.page_text(0).contains("Wednesday"));
    assert!(p.page_text(0).contains("café, Łódź."));
    let lines = hidden_lines(&p);
    assert_eq!(lines.len(), 6);
    let w = p.find_text_boxes(0, "wednesday").unwrap();
    assert_eq!(w.len(), 1);
    // Still on the same line, and within the line's original extent.
    let (old, new) = (line.bbox, w[0]);
    let ends_inside = new[0] >= old[0] - 1.0 && new[2] <= old[2] + 1.0;
    assert!(ends_inside, "{old:?} {new:?}");
    let same_row = (new[1] + new[3] - old[1] - old[3]).abs() < 6.0;
    assert!(same_row, "{old:?} {new:?}");
}

/// Where [`page_with_a_picture`] shows LINES' picture: displayed-page points, y down.
const PICTURE_AT: [f32; 4] = [72.0, 144.0, 360.0, 360.0];

/// A letter-size page of a line of its own text and, under it, LINES' picture (4 x 3 inches,
/// at 300 dpi) at PICTURE_AT: a picture of text in an ordinary document.
fn page_with_a_picture() -> Pdf {
    let own = "BT /F1 14 Tf 72 720 Td (Quarterly report for the harbour board) Tj ET";
    let mut p = helvetica_page(612.0, 792.0, own.into());
    let mut png = Cursor::new(Vec::new());
    DynamicImage::ImageLuma8(text_image())
        .write_to(&mut png, image::ImageFormat::Png)
        .unwrap();
    p.add_image(0, PICTURE_AT, &png.into_inner()).unwrap();
    p
}

/// A picture of text in an ordinary page, which isn't read with the whole page, is read in a
/// box drawn over it (#226): its lines are added as a page's are, in one undo step, and the
/// page's own text stays as it was.
#[test]
fn an_area_over_a_picture_of_text_is_read() {
    let ocr = Ocr::bundled().unwrap();
    let mut p = page_with_a_picture();
    let r = p.ocr_pages(&[0], &ocr).unwrap();
    assert!(skipped(&r, "already has text"), "{r:?}");
    let look = gray(&mut p, 1.0);
    let before = p.before();
    let r = p.ocr_areas(&[(0, PICTURE_AT)], &ocr).unwrap();
    assert!(six_lines(&r), "{r:?}");
    let text = p.page_text(0);
    assert!(recall(&text) > 0.9, "recall {}: {text}", recall(&text));
    assert!(text.starts_with("Quarterly report"), "{text}");
    assert_eq!(p.find_text_boxes(0, "harbour").unwrap().len(), 1, "{text}");
    // As in search_boxes_cover_recognised_words, the picture's top left at (72, 144).
    let b = p.find_text_boxes(0, "COMMITTEE").unwrap();
    assert_eq!(b.len(), 1, "{b:?}");
    let b = b[0];
    assert!((b[0] - 110.0).abs() < 6.0, "{b:?}");
    assert!(b[1] > 208.0 && b[1] < 220.0, "{b:?}");
    assert!(b[3] > 222.0 && b[3] < 232.0, "{b:?}");
    assert_eq!(look.as_raw(), gray(&mut p, 1.0).as_raw());
    let reopened = Pdf::from_bytes(&p.to_bytes().unwrap(), None).unwrap();
    assert!(recall(&reopened.page_text(0)) > 0.9);
    p.restore(before.undo(&p));
    let own = "Quarterly report for the harbour board";
    assert_eq!(p.page_text(0).trim(), own);
}

/// A box over the page's own text adds nothing: the words are there already.
#[test]
fn an_area_over_the_pages_own_text_adds_nothing() {
    let ocr = Ocr::bundled().unwrap();
    let mut p = page_with_a_picture();
    let r = p
        .ocr_areas(&[(0, [60.0, 50.0, 400.0, 100.0])], &ocr)
        .unwrap();
    assert!(skipped(&r, "no text found besides the page's own"), "{r:?}");
    let own = "Quarterly report for the harbour board";
    assert_eq!(p.page_text(0).trim(), own);
}

/// LINES' third and fourth lines on [`text_image`]'s page, and the left of them: their
/// baselines are 80 and 98 pt from the top.
const LINES_3_4: [f32; 4] = [8.0, 68.0, 280.0, 104.0];
const LEFT_OF_3_4: [f32; 4] = [8.0, 68.0, 100.0, 104.0];

/// [`text_image`] scanned and recognised, its third line corrected wrongly: Tuesday to
/// Wednesday.
fn misread_tuesday(ocr: &Ocr) -> Pdf {
    let mut p = scanned(text_image());
    p.ocr_pages(&[0], ocr).unwrap();
    let line = hidden_lines(&p)
        .into_iter()
        .find(|o| o.text.as_deref().unwrap_or("").contains("Tuesday"))
        .expect("no line with Tuesday");
    let edit = TextEdit {
        text: "The committee met on Wednesday morning.".into(),
        font: None,
        size: None,
        color: None,
        parts: Vec::new(),
    };
    p.edit_text(0, line.id, &edit).unwrap();
    p
}

/// Read in a box, a page's recognised lines wholly inside it are replaced; lines crossing its
/// edge are kept, and what is read on them isn't added again (#226, the user's B). The
/// saved file holds the replaced line no more.
#[test]
fn an_area_replaces_the_recognised_lines_inside_it() {
    let ocr = Ocr::bundled().unwrap();
    let mut p = misread_tuesday(&ocr);
    let look = gray(&mut p, 1.0);
    let r = p.ocr_areas(&[(0, LEFT_OF_3_4)], &ocr).unwrap();
    assert!(skipped(&r, "no text found besides the page's own"), "{r:?}");
    assert!(p.page_text(0).contains("Wednesday"));
    let r = p.ocr_areas(&[(0, LINES_3_4)], &ocr).unwrap();
    let two = matches!(r[..], [(0, OcrResult::Added { lines: 2, .. })]);
    assert!(two, "{r:?}");
    let text = p.page_text(0);
    assert!(
        text.contains("Tuesday") && !text.contains("Wednesday"),
        "{text}"
    );
    assert!(recall(&text) > 0.9, "recall {}: {text}", recall(&text));
    assert_eq!(hidden_lines(&p).len(), 6);
    assert_eq!(look.as_raw(), gray(&mut p, 1.0).as_raw());
    let saved = Saved::of(&mut p);
    saved.assert_gone(&["Wednesday"]);
    saved.assert_kept(&["Tuesday", "committee"]);
    assert_eq!(saved.unreachable(), []);
}

/// Poppler reads a line replaced in a box as LimarePDF does.
/// `cargo test -p limarepdf-core --test all -- --ignored ocr::cross_check`.
#[test]
#[ignore]
fn cross_check_area_text_with_poppler() {
    let ocr = Ocr::bundled().unwrap();
    let mut p = misread_tuesday(&ocr);
    p.ocr_areas(&[(0, LINES_3_4)], &ocr).unwrap();
    let ours: Vec<String> = p.page_text(0).split_whitespace().map(Into::into).collect();
    let poppler = tools::pdftotext(&p.to_bytes().unwrap());
    let theirs: Vec<&str> = poppler.split_whitespace().collect();
    assert_eq!(theirs, ours, "{poppler}");
    assert!(
        poppler.contains("Tuesday") && !poppler.contains("Wednesday"),
        "{poppler}"
    );
}

/// Read in a box, recognised lines inside a form that draws more are replaced as a page's
/// are (#327): the page draws a copy of the form holding the rest of its lines.
#[test]
fn an_area_replaces_the_recognised_lines_inside_a_form() {
    let ocr = Ocr::bundled().unwrap();
    let mut p = printed(&misread_tuesday(&ocr));
    let look = gray(&mut p, 1.0);
    let r = p.ocr_areas(&[(0, LINES_3_4)], &ocr).unwrap();
    let two = matches!(r[..], [(0, OcrResult::Added { lines: 2, .. })]);
    assert!(two, "{r:?}");
    let text = p.page_text(0);
    assert!(
        text.contains("Tuesday") && !text.contains("Wednesday"),
        "{text}"
    );
    let once = |w: &str| p.find_text_boxes(0, w).unwrap().len() == 1;
    assert!(once("committee") && once("Contact"), "{text}");
    assert!(recall(&text) > 0.9, "recall {}: {text}", recall(&text));
    assert_eq!(look.as_raw(), gray(&mut p, 1.0).as_raw());
    let saved = Saved::of(&mut p);
    saved.assert_gone(&["Wednesday"]);
    saved.assert_kept(&["Tuesday", "committee"]);
    assert_eq!(saved.unreachable(), []);
}

/// A form drawn twice side by side, read in a box over one drawing, has its lines replaced
/// there alone: that drawing draws a copy, and the other the form as it was.
#[test]
fn an_area_over_one_drawing_of_a_form_replaces_its_lines_there() {
    let ocr = Ocr::bundled().unwrap();
    let mut p = printed(&misread_tuesday(&ocr));
    let whole = (resources(&p, "XObject").iter())
        .find_map(|(_, o)| o.as_reference().ok())
        .expect("the sheet draws no form");
    let twice = "q /Page Do Q q 1 0 0 1 288 0 cm /Page Do Q";
    draw_forms(&mut p, 0, twice, &[("Page", whole)]);
    let page = p.page_ids()[0];
    let d = p.doc.get_dictionary_mut(page).unwrap();
    for b in ["MediaBox", "CropBox"] {
        d.set(b, vec![0.into(), 0.into(), 576.into(), 216.into()]);
    }
    let original = p.doc.get_object(whole).unwrap().clone();
    let look = gray(&mut p, 1.0);
    let r = p.ocr_areas(&[(0, LINES_3_4)], &ocr).unwrap();
    let two = matches!(r[..], [(0, OcrResult::Added { lines: 2, .. })]);
    assert!(two, "{r:?}");
    let text = p.page_text(0);
    let at = |w: &str| p.find_text_boxes(0, w).unwrap();
    let left = matches!(at("Tuesday")[..], [b] if b[2] < 288.0);
    let right = matches!(at("Wednesday")[..], [b] if b[0] > 288.0);
    assert!(left && right, "{text}");
    assert_eq!(at("committee").len(), 2, "{text}");
    assert_eq!(look.as_raw(), gray(&mut p, 1.0).as_raw());
    assert_eq!(drawn(&p, 0, "Page"), Some(whole));
    assert_eq!(p.doc.get_object(whole).unwrap(), &original);
    let saved = Saved::of(&mut p);
    saved.assert_kept(&["Tuesday", "Wednesday"]);
    assert_eq!(saved.unreachable(), []);
}

/// Boxes on one page that overlap read the lines they share once, and read again, a box
/// replaces what they read.
#[test]
fn overlapping_areas_read_their_lines_once() {
    let ocr = Ocr::bundled().unwrap();
    let mut p = scanned(text_image());
    let top = [8.0, 30.0, 280.0, 104.0];
    let bottom = [8.0, 68.0, 280.0, 140.0];
    let r = p.ocr_areas(&[(0, top), (0, bottom)], &ocr).unwrap();
    let read = matches!(
        r[..],
        [
            (0, OcrResult::Added { lines: 4, .. }),
            (0, OcrResult::Added { lines: 2, .. })
        ]
    );
    assert!(read, "{r:?}");
    let once = |p: &Pdf, w: &str| p.find_text_boxes(0, w).unwrap().len() == 1;
    assert!(
        once(&p, "committee") && once(&p, "Contact"),
        "{}",
        p.page_text(0)
    );
    assert_eq!(hidden_lines(&p).len(), 6);
    assert!(recall(&p.page_text(0)) > 0.9, "{}", p.page_text(0));
    let r = p.ocr_areas(&[(0, bottom)], &ocr).unwrap();
    let four = matches!(r[..], [(0, OcrResult::Added { lines: 4, .. })]);
    assert!(four, "{r:?}");
    assert!(
        once(&p, "committee") && once(&p, "Contact"),
        "{}",
        p.page_text(0)
    );
    assert_eq!(hidden_lines(&p).len(), 6);
}

/// A box is read on the page as shown: here a scan stored sideways and shown upright.
#[test]
fn an_area_on_a_rotated_page_is_read_as_shown() {
    let ocr = Ocr::bundled().unwrap();
    let mut p = scanned(image::imageops::rotate270(&text_image()));
    let page = p.page_ids()[0];
    p.doc.get_dictionary_mut(page).unwrap().set("Rotate", 90);
    let r = p.ocr_areas(&[(0, LINES_3_4)], &ocr).unwrap();
    let two = matches!(r[..], [(0, OcrResult::Added { lines: 2, .. })]);
    assert!(two, "{r:?}");
    let b = p.find_text_boxes(0, "COMMITTEE").unwrap();
    assert_eq!(b.len(), 1, "{b:?}");
    let b = b[0];
    assert!((b[0] - 38.0).abs() < 6.0, "{b:?}");
    assert!(b[1] > 64.0 && b[1] < 76.0, "{b:?}");
    assert!(b[3] > 78.0 && b[3] < 88.0, "{b:?}");
    assert_eq!(rotation(&p), 90);
}

/// Text lying on its side or upside down in a box isn't read: a box can't be turned upright
/// without its page. The page is left as it was, and the reason says how to read it.
#[test]
fn turned_text_in_an_area_is_left() {
    let ocr = Ocr::bundled().unwrap();
    let img = text_image();
    let turned = [
        (image::imageops::rotate90(&img), "on its side"),
        (image::imageops::rotate180(&img), "upside down"),
    ];
    for (scan, way) in turned {
        let mut p = scanned(scan);
        let (w, h) = p.page_size(0).unwrap();
        let r = p.ocr_areas(&[(0, [0.0, 0.0, w, h])], &ocr).unwrap();
        assert!(
            skipped(&r, way) && skipped(&r, "turn the page"),
            "{way}: {r:?}"
        );
        assert_eq!(rotation(&p), 0, "{way}");
        assert_eq!(p.page_text(0).trim(), "", "{way}");
    }
}

/// A box off the page reads nothing; a page that isn't there is an error.
#[test]
fn areas_off_the_page_read_nothing() {
    let ocr = Ocr::bundled().unwrap();
    let mut p = scanned(text_image());
    let r = p
        .ocr_areas(&[(0, [300.0, 0.0, 400.0, 50.0])], &ocr)
        .unwrap();
    assert!(skipped(&r, "outside the page"), "{r:?}");
    assert!(p.ocr_areas(&[(1, [0.0, 0.0, 10.0, 10.0])], &ocr).is_err());
    assert_eq!(p.page_text(0).trim(), "");
}

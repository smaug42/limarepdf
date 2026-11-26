//! OCR text layers, with the bundled recognition model. Opt-in, Poppler reads the layer as
//! LimarePDF does: `cargo test -p limarepdf-core --test ocr -- --ignored cross_check`.

mod common;

use common::saved::{tools, Saved};
use image::{DynamicImage, GrayImage, Luma};
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
    let mut p = Pdf::new();
    p.insert_blank(0, 288.0, 216.0).unwrap();
    let mut content = String::from("BT /F1 12 Tf 16 190 Td 18 TL\n");
    for l in LINES {
        content.push_str(&format!("({l}) '\n"));
    }
    content.push_str("ET");
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
    let mut png = Cursor::new(Vec::new());
    DynamicImage::ImageLuma8(img)
        .write_to(&mut png, image::ImageFormat::Png)
        .unwrap();
    let opts = ImageImport {
        dpi: Some(300.0),
        ..Default::default()
    };
    Pdf::from_images(&[png.into_inner()], &opts).unwrap()
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

/// Whether page 1 alone was read, to six lines.
fn six_lines(r: &[(usize, OcrResult)]) -> bool {
    matches!(r, [(0, OcrResult::Added { lines: 6, .. })])
}

/// Share of the words in LINES found in `text`, ignoring end punctuation.
fn recall(text: &str) -> f32 {
    let strip = |w: &str| w.trim_matches(|c: char| ".,;:".contains(c)).to_string();
    let got: Vec<String> = text.split_whitespace().map(strip).collect();
    let want: Vec<String> = LINES
        .iter()
        .flat_map(|l| l.split_whitespace())
        .map(strip)
        .collect();
    let hits = want.iter().filter(|w| got.contains(w)).count();
    hits as f32 / want.len() as f32
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
/// `cargo test -p limarepdf-core --test ocr -- --ignored cross_check`.
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
    p.ocr_pages(&[0], &ocr).unwrap();
    let text = p.page_text(0);
    assert!(recall(&text) > 0.9, "{text}");
    let b = first_line_box(&mut p);
    assert!((b[0] - 16.0).abs() < 4.0, "{b:?}");
    assert!(b[1] > 26.0 && b[1] < 38.0, "{b:?}");
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
    let words: Vec<&str> = (text.split_whitespace())
        .filter(|w| w.len() >= 6 && w.chars().all(char::is_alphabetic))
        .collect();
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
/// `cargo test -p limarepdf-core --test ocr -- --ignored cross_check`.
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

/// Recognised text inside a form that draws more, such as a whole page put in a form by an
/// app arranging pages, is left, and the page isn't read again: its text would be doubled.
#[test]
fn recognised_text_in_a_form_drawing_more_is_left() {
    let ocr = Ocr::bundled().unwrap();
    let mut p = scanned(text_image());
    let page = p.page_ids()[0];
    let content = String::from_utf8(p.doc.get_page_content(page)).unwrap();
    let content = format!("{content}\nBT 3 Tr /LF1 12 Tf 20 100 Td (Hidden words) Tj ET");
    let xobjects = resources(&p, "XObject");
    let whole = form(&mut p, &content, xobjects);
    let drawing = Stream::new(Dictionary::new(), b"q /Page1 Do Q".to_vec());
    let drawing = p.doc.add_object(drawing);
    let d = p.doc.get_dictionary_mut(page).unwrap();
    d.set("Contents", drawing);
    d.set(
        "Resources",
        dictionary! { "XObject" => dictionary! { "Page1" => whole } },
    );
    let before = p.doc.get_dictionary(page).unwrap().clone();
    let r = p.remove_recognised_text(&[0]).unwrap();
    assert!(skipped(&r, "other content"), "{r:?}");
    let r = p.ocr_pages_again(&[0], &ocr).unwrap();
    assert!(skipped(&r, "other content"), "{r:?}");
    let r = p.ocr_pages(&[0], &ocr).unwrap();
    assert!(skipped(&r, "already has recognised text"), "{r:?}");
    assert_eq!(p.doc.get_dictionary(page).unwrap(), &before);
    assert_eq!(p.page_text(0).trim(), "Hidden words");
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

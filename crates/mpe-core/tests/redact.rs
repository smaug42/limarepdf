use lopdf::content::Content;
use lopdf::{dictionary, Dictionary, Object, ObjectId, Stream};
use mpe_core::{render, ObjectKind, Pdf};

const W: f32 = 300.0;
const H: f32 = 200.0;

/// A document with one 300×200 pt page per entry, each drawing that content with `/F1`
/// (Helvetica, no widths in the file).
fn make(pages: &[&str]) -> Pdf {
    let mut p = Pdf::new();
    for i in 0..pages.len() {
        p.insert_blank(i, W, H).unwrap();
    }
    let font = p.doc.add_object(dictionary! {
        "Type" => "Font", "Subtype" => "Type1", "BaseFont" => "Helvetica", "Encoding" => "WinAnsiEncoding",
    });
    for (i, c) in pages.iter().enumerate() {
        let page = p.page_ids()[i];
        let s = p
            .doc
            .add_object(Stream::new(Dictionary::new(), c.as_bytes().to_vec()));
        let d = p.doc.get_dictionary_mut(page).unwrap();
        d.set("Contents", Object::Reference(s));
        d.set(
            "Resources",
            dictionary! { "Font" => dictionary! { "F1" => font } },
        );
    }
    p
}

fn content(p: &Pdf, page: usize) -> String {
    let id = p.page_ids()[page];
    String::from_utf8_lossy(&p.doc.get_page_content(id)).into_owned()
}

/// Whether `needle` is anywhere in the saved file: in any string or (decompressed) stream.
fn file_has(p: &mut Pdf, needle: &str) -> bool {
    let bytes = p.to_bytes().unwrap();
    let q = Pdf::from_bytes(&bytes, None).unwrap();
    let n = needle.as_bytes();
    let has = |b: &[u8]| b.windows(n.len()).any(|w| w == n);
    q.doc.objects.values().any(|o| match o {
        Object::Stream(s) => has(&s
            .decompressed_content()
            .unwrap_or_else(|_| s.content.clone())),
        Object::String(b, _) => has(b),
        Object::Dictionary(d) => d
            .iter()
            .any(|(_, v)| matches!(v, Object::String(b, _) if has(b))),
        _ => false,
    })
}

fn images_in_file(p: &mut Pdf) -> usize {
    let bytes = p.to_bytes().unwrap();
    let q = Pdf::from_bytes(&bytes, None).unwrap();
    q.doc
        .objects
        .values()
        .filter(|o| {
            o.as_stream().is_ok_and(|s| {
                s.dict.get(b"Subtype").and_then(|x| x.as_name()).ok() == Some(b"Image")
            })
        })
        .count()
}

fn near(a: [f32; 4], b: [f32; 4]) -> bool {
    a.iter().zip(b).all(|(x, y)| (x - y).abs() < 0.05)
}

#[test]
fn removes_the_marked_characters_and_keeps_the_rest_in_place() {
    let mut p = make(&["BT /F1 20 Tf 30 100 Td (Name: Zebra Quokka here) Tj ET"]);
    let here = p.find_text_boxes(0, "here").unwrap();
    let name = p.find_text_boxes(0, "Name:").unwrap();
    let marks = p.find_redaction_marks("zebra quokka").unwrap();
    assert_eq!(marks.len(), 1);

    let rep = p.redact(&marks).unwrap();
    assert!(rep.left.is_empty(), "{:?}", rep.left);
    assert!(rep.elsewhere.is_empty(), "{:?}", rep.elsewhere);
    assert_eq!(rep.pages.len(), 1);
    assert_eq!(rep.pages[0].text, vec!["Zebra Quokka".to_string()]);
    assert_eq!(rep.pages[0].chars, 12);

    let text = p.page_text(0);
    assert!(text.contains("Name:") && text.contains("here"), "{text}");
    assert!(!text.to_lowercase().contains("zebra"), "{text}");
    assert!(near(p.find_text_boxes(0, "here").unwrap()[0], here[0]));
    assert!(near(p.find_text_boxes(0, "Name:").unwrap()[0], name[0]));
    assert!(!file_has(&mut p, "Zebra") && !file_has(&mut p, "Quokka"));
    assert!(!file_has(&mut p, "ebra") && !file_has(&mut p, "uokk"));
}

#[test]
fn keeps_tj_spacing_and_later_strings_in_place() {
    let mut p = make(&["BT /F1 20 Tf 30 100 Td [(Sec) -200 (retX) 50 (Yzw)] TJ ET"]);
    let sec = p.find_text_boxes(0, "Sec").unwrap()[0];
    let yzw = p.find_text_boxes(0, "Yzw").unwrap()[0];
    let marks = p.find_redaction_marks("retX").unwrap();
    let rep = p.redact(&marks).unwrap();
    assert_eq!(rep.pages[0].chars, 4);
    assert!(rep.left.is_empty(), "{:?}", rep.left);
    assert!(near(p.find_text_boxes(0, "Sec").unwrap()[0], sec));
    assert!(near(p.find_text_boxes(0, "Yzw").unwrap()[0], yzw));
    assert!(p.find_text_boxes(0, "ret").unwrap().is_empty());
}

#[test]
fn removes_hidden_text_and_its_alternative_text() {
    let mut p =
        make(&["/Span <</ActualText (Zebra)>> BDC BT 3 Tr /F1 20 Tf 30 100 Td (Zebra) Tj ET EMC"]);
    let marks = p.find_redaction_marks("zebra").unwrap();
    assert_eq!(marks.len(), 1, "hidden text is found");
    let rep = p.redact(&marks).unwrap();
    assert_eq!(rep.pages[0].chars, 5);
    assert!(!content(&p, 0).contains("ActualText"));
    assert!(!file_has(&mut p, "Zebra"));
}

#[test]
fn removes_drawings_under_a_mark_and_clips_those_crossing_it() {
    // A red background across the page, and a small blue square at 100,50 (user space).
    let mut p = make(&["q 1 0 0 rg 10 10 280 180 re f Q q 0 0 1 rg 100 50 20 20 re f Q"]);
    // Displayed y runs down: user y 40..80 is displayed 120..160.
    let rep = p.redact(&[(0, [90.0, 120.0, 130.0, 160.0])]).unwrap();
    assert_eq!(rep.pages[0].paths, 1);
    assert_eq!(rep.pages[0].clipped, 1);
    let ops = Content::decode(content(&p, 0).as_bytes())
        .unwrap()
        .operations;
    let rects: Vec<Vec<f32>> = ops
        .iter()
        .filter(|o| o.operator == "re")
        .map(|o| o.operands.iter().map(|x| x.as_float().unwrap()).collect())
        .collect();
    assert!(rects.contains(&vec![10.0, 10.0, 280.0, 180.0]), "{rects:?}");
    assert!(!rects.contains(&vec![100.0, 50.0, 20.0, 20.0]), "{rects:?}");
    assert!(ops.iter().any(|o| o.operator == "W*"));

    let bytes = p.snapshot().unwrap();
    let doc = render::load(bytes, None).unwrap();
    let bmp = render::Renderer::new(&doc).render(0, 1.0).unwrap();
    let px = |x: usize, y: usize| {
        let i = (y * bmp.width + x) * 4;
        [bmp.rgba[i], bmp.rgba[i + 1], bmp.rgba[i + 2]]
    };
    assert!(
        px(110, 140)[0] < 40 && px(110, 140)[2] < 40,
        "the mark is black"
    );
    assert!(
        px(50, 50)[0] > 200 && px(50, 50)[2] < 60,
        "the background stays red outside"
    );
}

#[test]
fn removes_a_picture_under_a_mark_from_the_file() {
    let mut p = make(&["BT /F1 20 Tf 30 30 Td (Caption) Tj ET"]);
    let img = image::RgbaImage::from_pixel(8, 8, image::Rgba([0, 200, 0, 255]));
    let mut png = std::io::Cursor::new(Vec::new());
    img.write_to(&mut png, image::ImageFormat::Png).unwrap();
    p.add_image(0, [100.0, 50.0, 150.0, 100.0], &png.into_inner())
        .unwrap();
    assert_eq!(images_in_file(&mut p), 1);
    let rep = p.redact(&[(0, [120.0, 60.0, 130.0, 70.0])]).unwrap();
    assert_eq!(rep.pages[0].images, 1);
    assert!(rep.left.is_empty(), "{:?}", rep.left);
    let objs = p.page_objects(0).unwrap();
    assert!(objs.iter().all(|o| o.kind != ObjectKind::Image));
    assert!(p.page_text(0).contains("Caption"));
    assert_eq!(images_in_file(&mut p), 0);
}

#[test]
fn drops_a_picture_from_shared_resources_when_no_page_draws_it() {
    let mut p = make(&[
        "q 50 0 0 50 100 50 cm /Im1 Do Q",
        "BT /F1 20 Tf 30 30 Td (Two) Tj ET",
    ]);
    let img = p.doc.add_object(Stream::new(
        dictionary! {
            "Type" => "XObject", "Subtype" => "Image", "Width" => 1, "Height" => 1,
            "ColorSpace" => "DeviceGray", "BitsPerComponent" => 8,
        },
        vec![0x80],
    ));
    let font = p
        .doc
        .get_dictionary(p.page_ids()[0])
        .unwrap()
        .get(b"Resources")
        .unwrap()
        .clone();
    let mut shared = font.as_dict().unwrap().clone();
    shared.set("XObject", dictionary! { "Im1" => img });
    let shared: ObjectId = p.doc.add_object(shared);
    for page in p.page_ids() {
        p.doc
            .get_dictionary_mut(page)
            .unwrap()
            .set("Resources", shared);
    }
    assert_eq!(images_in_file(&mut p), 1);
    let rep = p.redact(&[(0, [110.0, 100.0, 120.0, 110.0])]).unwrap();
    assert_eq!(rep.pages[0].images, 1);
    assert_eq!(images_in_file(&mut p), 0);
    assert!(p.page_text(1).contains("Two"));
}

#[test]
fn redacts_inside_a_shared_form_only_on_the_marked_page() {
    let mut p = make(&["q /Fm1 Do Q", "q /Fm1 Do Q"]);
    let font = p.doc.add_object(dictionary! {
        "Type" => "Font", "Subtype" => "Type1", "BaseFont" => "Helvetica",
    });
    let form = p.doc.add_object(Stream::new(
        dictionary! {
            "Type" => "XObject", "Subtype" => "Form", "BBox" => vec![0.into(), 0.into(), 300.into(), 200.into()],
            "Resources" => dictionary! { "Font" => dictionary! { "F1" => font } },
        },
        b"BT /F1 20 Tf 30 100 Td (Secret Walrus) Tj ET".to_vec(),
    ));
    for page in p.page_ids() {
        p.doc.get_dictionary_mut(page).unwrap().set(
            "Resources",
            dictionary! { "XObject" => dictionary! { "Fm1" => form } },
        );
    }
    let marks: Vec<_> = p
        .find_redaction_marks("walrus")
        .unwrap()
        .into_iter()
        .filter(|m| m.0 == 0)
        .collect();
    assert_eq!(marks.len(), 1);
    let rep = p.redact(&marks).unwrap();
    assert!(rep.left.is_empty(), "{:?}", rep.left);
    assert_eq!(rep.pages[0].chars, 6);
    assert!(!p.page_text(0).to_lowercase().contains("walrus"));
    assert!(p.page_text(0).contains("Secret"));
    assert!(
        p.page_text(1).contains("Walrus"),
        "the other page keeps the form as it was"
    );
    assert_eq!(
        rep.elsewhere,
        vec![("walrus".to_string(), vec!["page 2".to_string()])]
    );
}

#[test]
fn removes_annotations_touching_a_mark() {
    let mut p = make(&["BT /F1 20 Tf 30 100 Td (Zebra) Tj ET"]);
    let page = p.page_ids()[0];
    let link = |p: &mut Pdf, r: [i64; 4]| {
        p.doc.add_object(dictionary! {
            "Type" => "Annot", "Subtype" => "Link",
            "Rect" => r.iter().map(|&x| Object::Integer(x)).collect::<Vec<_>>(),
        })
    };
    let on = link(&mut p, [30, 95, 80, 120]);
    let off = link(&mut p, [200, 10, 250, 30]);
    p.doc.get_dictionary_mut(page).unwrap().set(
        "Annots",
        vec![Object::Reference(on), Object::Reference(off)],
    );
    let marks = p.find_redaction_marks("zebra").unwrap();
    let rep = p.redact(&marks).unwrap();
    assert_eq!(rep.pages[0].annots, 1);
    let left = p
        .doc
        .get_dictionary(page)
        .unwrap()
        .get(b"Annots")
        .unwrap()
        .as_array()
        .unwrap()
        .clone();
    assert_eq!(left, vec![Object::Reference(off)]);
}

#[test]
fn works_on_turned_pages() {
    let mut p = make(&["BT /F1 20 Tf 30 100 Td (Keep Zebra Keep) Tj ET"]);
    p.rotate_pages(&[0], 90).unwrap();
    let marks = p.find_redaction_marks("zebra").unwrap();
    let rep = p.redact(&marks).unwrap();
    assert_eq!(rep.pages[0].chars, 5);
    assert!(rep.left.is_empty(), "{:?}", rep.left);
    let text = p.page_text(0);
    assert!(
        !text.to_lowercase().contains("zebra") && text.contains("Keep"),
        "{text}"
    );
}

#[test]
fn rejects_marks_with_no_size_or_page() {
    let mut p = make(&[""]);
    assert!(p.redact(&[(0, [10.0, 10.0, 10.0, 50.0])]).is_err());
    assert!(p.redact(&[(3, [10.0, 10.0, 20.0, 50.0])]).is_err());
}

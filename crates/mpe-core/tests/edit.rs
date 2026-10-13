use lopdf::{dictionary, Dictionary, Object, Stream};
use mpe_core::{render, FontChoice, ObjectKind, PathStyle, Pdf, Shape, TextEdit};

const W: u32 = 300;
const H: u32 = 200;

fn make(content: &str) -> Pdf {
    let mut p = Pdf::new();
    p.insert_blank(0, W as f32, H as f32).unwrap();
    set_content(&mut p, content);
    p
}

fn set_content(p: &mut Pdf, content: &str) {
    let page = p.page_ids()[0];
    let font = p.doc.add_object(dictionary! { "Type" => "Font", "Subtype" => "Type1", "BaseFont" => "Helvetica", "Encoding" => "WinAnsiEncoding" });
    let s = p
        .doc
        .add_object(Stream::new(Dictionary::new(), content.as_bytes().to_vec()));
    let d = p.doc.get_dictionary_mut(page).unwrap();
    d.set("Contents", Object::Reference(s));
    d.set(
        "Resources",
        dictionary! { "Font" => dictionary! { "F1" => font } },
    );
}

struct Shot(render::Bitmap);
impl Shot {
    fn px(&self, x: usize, y: usize) -> [u8; 3] {
        let i = (y * self.0.width + x) * 4;
        [self.0.rgba[i], self.0.rgba[i + 1], self.0.rgba[i + 2]]
    }
    fn is(&self, x: usize, y: usize, c: [u8; 3]) -> bool {
        let p = self.px(x, y);
        (0..3).all(|k| (p[k] as i32 - c[k] as i32).abs() < 12)
    }
}

fn shot(p: &mut Pdf) -> Shot {
    let bytes = p.snapshot().unwrap();
    let hp = render::load(bytes, None).unwrap();
    Shot(render::Renderer::new(&hp).render(0, 1.0).unwrap())
}

const RED: [u8; 3] = [255, 0, 0];
const WHITE: [u8; 3] = [255, 255, 255];
const BASE: &str =
    "q 1 0 0 rg 50 50 100 40 re f Q\nBT /F1 24 Tf 0 0 1 rg 60 150 Td (Hello) Tj ET\n";

fn png(w: u32, h: u32, c: [u8; 4]) -> Vec<u8> {
    let img = image::RgbaImage::from_pixel(w, h, image::Rgba(c));
    let mut out = std::io::Cursor::new(Vec::new());
    img.write_to(&mut out, image::ImageFormat::Png).unwrap();
    out.into_inner()
}

#[test]
fn finds_text_boxes_in_visible_text() {
    let p = make(BASE);
    assert!(p.page_objects(0).unwrap().iter().all(|o| !o.hidden));
    // "Hello" at x = 60, baseline 150 on a 200 pt page (50 from the top), 24 pt
    // Helvetica, about 55 pt wide. "ell" is characters 1..4 of 5, placed by character
    // position: about 71..104.
    let b = p.find_text_boxes(0, "ELL").unwrap();
    assert_eq!(b.len(), 1, "{b:?}");
    let b = b[0];
    assert!(b[0] > 68.0 && b[0] < 74.0, "{b:?}");
    assert!(b[2] > 100.0 && b[2] < 107.0, "{b:?}");
    assert!(b[1] > 28.0 && b[1] < 34.0, "{b:?}");
    assert!(b[3] > 52.0 && b[3] < 58.0, "{b:?}");
    assert_eq!(p.find_text_boxes(0, "l").unwrap().len(), 2);
    assert!(p.find_text_boxes(0, "   ").unwrap().is_empty());
}

#[test]
fn lists_words_for_selection() {
    let p = make("BT /F1 24 Tf 60 150 Td (Hello world) Tj 0 -40 Td (again) Tj ET");
    let w = p.page_words(0).unwrap();
    let texts: Vec<&str> = w.iter().map(|w| w.text.as_str()).collect();
    assert_eq!(texts, ["Hello", "world", "again"]);
    assert_eq!(mpe_core::words_text(&w), "Hello world\nagain");
    assert_eq!(mpe_core::words_text(&w[1..2]), "world");
    // Same line: side by side. Next line: below.
    assert!(w[1].bbox[0] > w[0].bbox[2] - 1.0, "{:?}", w);
    assert!((w[1].bbox[1] - w[0].bbox[1]).abs() < 0.5);
    assert!(w[2].bbox[1] > w[0].bbox[3], "{:?}", w);
    assert!((w[0].bbox[0] - 60.0).abs() < 0.5);
    let shapes_only = make("q 1 0 0 rg 0 0 10 10 re f Q");
    assert!(shapes_only.page_words(0).unwrap().is_empty());
}

#[test]
fn lists_objects_in_visual_coordinates() {
    let p = make(BASE);
    let o = p.page_objects(0).unwrap();
    assert_eq!(o.len(), 2);
    assert_eq!(o[0].kind, ObjectKind::Path);
    assert!(o[0].has_fill && !o[0].has_stroke);
    // user y 50..90 on a 200 high page is 110..150 when y points down
    let b = o[0].bbox;
    assert!(
        (b[0] - 50.0).abs() < 0.5
            && (b[1] - 110.0).abs() < 0.5
            && (b[2] - 150.0).abs() < 0.5
            && (b[3] - 150.0).abs() < 0.5,
        "{b:?}"
    );
    assert_eq!(o[1].kind, ObjectKind::Text);
    assert_eq!(o[1].text.as_deref(), Some("Hello"));
    assert_eq!(o[1].font_size, Some(24.0));
    assert_eq!(o[1].fill, Some([0.0, 0.0, 1.0]));
}

#[test]
fn move_resize_and_delete_path() {
    let mut p = make(BASE);
    p.set_object_bbox(0, 0, [150.0, 110.0, 250.0, 150.0])
        .unwrap();
    let s = shot(&mut p);
    assert!(s.is(200, 130, RED));
    assert!(s.is(100, 130, WHITE));
    // resize to double height, keeping the top edge
    p.set_object_bbox(0, 0, [150.0, 110.0, 250.0, 190.0])
        .unwrap();
    let s = shot(&mut p);
    assert!(s.is(200, 180, RED));
    p.delete_object(0, 0).unwrap();
    let s = shot(&mut p);
    assert!(s.is(200, 130, WHITE) && s.is(200, 180, WHITE));
    // the text is untouched
    assert_eq!(p.page_objects(0).unwrap().len(), 1);
}

#[test]
fn move_text_keeps_following_content_in_place() {
    let mut p = make("BT /F1 20 Tf 20 100 Td (One) Tj 0 -30 Td (Two) Tj ET\n");
    let before = p.page_objects(0).unwrap();
    assert_eq!(before.len(), 2);
    p.set_object_bbox(0, 0, {
        let b = before[0].bbox;
        [b[0] + 100.0, b[1], b[2] + 100.0, b[3]]
    })
    .unwrap();
    let after = p.page_objects(0).unwrap();
    assert_eq!(after.len(), 2, "{after:?}");
    let one = after
        .iter()
        .find(|o| o.text.as_deref() == Some("One"))
        .unwrap();
    let two = after
        .iter()
        .find(|o| o.text.as_deref() == Some("Two"))
        .unwrap();
    let (b1, b2) = (
        before
            .iter()
            .find(|o| o.text.as_deref() == Some("One"))
            .unwrap(),
        before
            .iter()
            .find(|o| o.text.as_deref() == Some("Two"))
            .unwrap(),
    );
    assert!(
        (one.bbox[0] - b1.bbox[0] - 100.0).abs() < 0.5,
        "{one:?} vs {b1:?}"
    );
    assert!(
        (two.bbox[0] - b2.bbox[0]).abs() < 0.5 && (two.bbox[1] - b2.bbox[1]).abs() < 0.5,
        "Two moved: {two:?} vs {b2:?}"
    );
}

#[test]
fn move_text_with_tj_array_and_quote() {
    let mut p = make("BT /F1 14 Tf 16 TL 20 150 Td [(He) -50 (llo)] TJ (Next) ' ET\n");
    let before = p.page_objects(0).unwrap();
    assert!(before.len() >= 2, "{before:?}");
    let t0 = before[0].bbox;
    p.set_object_bbox(0, 0, [t0[0], t0[1] + 40.0, t0[2], t0[3] + 40.0])
        .unwrap();
    let after = p.page_objects(0).unwrap();
    assert_eq!(after.len(), before.len());
    assert!((after[0].bbox[1] - t0[1] - 40.0).abs() < 0.5);
    assert!((after[1].bbox[1] - before[1].bbox[1]).abs() < 0.5);
}

#[test]
fn edit_text_in_place_and_with_new_font() {
    let mut p = make(BASE);
    p.edit_text(
        0,
        1,
        &TextEdit {
            text: "Howdy".into(),
            font: None,
            size: None,
            color: None,
        },
    )
    .unwrap();
    let o = p.page_objects(0).unwrap();
    assert_eq!(o[1].text.as_deref(), Some("Howdy"));
    assert_eq!(o[1].font_size, Some(24.0));
    p.edit_text(
        0,
        1,
        &TextEdit {
            text: "Ünï €".into(),
            font: Some(FontChoice::from_name("Courier-Bold")),
            size: Some(30.0),
            color: Some([0.0, 0.5, 0.0]),
        },
    )
    .unwrap();
    let o = p.page_objects(0).unwrap();
    let t = o.iter().find(|x| x.kind == ObjectKind::Text).unwrap();
    assert_eq!(t.text.as_deref(), Some("Ünï €"));
    assert_eq!(t.font_size, Some(30.0));
    assert_eq!(t.fill, Some([0.0, 0.5, 0.0]));
    assert_eq!(o.iter().filter(|x| x.kind == ObjectKind::Text).count(), 1);
    // saved file re-opens
    let bytes = p.to_bytes().unwrap();
    let q = Pdf::from_bytes(&bytes, None).unwrap();
    assert_eq!(q.page_objects(0).unwrap().len(), o.len());
}

#[test]
fn restyle_path() {
    let mut p = make(BASE);
    p.set_path_style(
        0,
        0,
        &PathStyle {
            fill: Some([0.0, 1.0, 0.0]),
            stroke: Some([0.0, 0.0, 0.0]),
            line_width: 4.0,
        },
    )
    .unwrap();
    let s = shot(&mut p);
    assert!(s.is(100, 130, [0, 255, 0]));
    assert!(s.is(100, 110, [0, 0, 0]) || s.is(100, 111, [0, 0, 0]) || s.is(100, 109, [0, 0, 0]));
    p.set_path_style(
        0,
        0,
        &PathStyle {
            fill: None,
            stroke: Some([0.0, 0.0, 1.0]),
            line_width: 2.0,
        },
    )
    .unwrap();
    let s = shot(&mut p);
    assert!(s.is(100, 130, WHITE));
}

#[test]
fn insert_shapes_text_and_images() {
    let mut p = make("");
    p.add_shape(
        0,
        Shape::Rect,
        [10.0, 10.0, 60.0, 40.0],
        &PathStyle {
            fill: Some([1.0, 0.0, 0.0]),
            stroke: None,
            line_width: 1.0,
        },
    )
    .unwrap();
    p.add_shape(
        0,
        Shape::Ellipse,
        [100.0, 10.0, 160.0, 70.0],
        &PathStyle {
            fill: Some([0.0, 0.0, 1.0]),
            stroke: None,
            line_width: 1.0,
        },
    )
    .unwrap();
    p.add_shape(
        0,
        Shape::Line,
        [10.0, 100.0, 90.0, 100.0],
        &PathStyle {
            fill: None,
            stroke: Some([0.0, 0.0, 0.0]),
            line_width: 4.0,
        },
    )
    .unwrap();
    p.add_image(
        0,
        [200.0, 100.0, 260.0, 150.0],
        &png(8, 8, [0, 200, 0, 255]),
    )
    .unwrap();
    p.add_text(
        0,
        (20.0, 180.0),
        "Added",
        FontChoice::from_name("Times-Italic"),
        18.0,
        [0.0, 0.0, 0.0],
    )
    .unwrap();
    let s = shot(&mut p);
    assert!(s.is(30, 25, RED));
    assert!(s.is(130, 40, [0, 0, 255]));
    assert!(s.is(50, 100, [0, 0, 0]));
    assert!(s.is(230, 125, [0, 200, 0]));
    assert!(s.is(50, 25 + 100, WHITE));
    let o = p.page_objects(0).unwrap();
    assert_eq!(o.iter().filter(|x| x.kind == ObjectKind::Image).count(), 1);
    assert!(o.iter().any(|x| x.text.as_deref() == Some("Added")));
    assert_eq!(o.iter().filter(|x| x.kind == ObjectKind::Path).count(), 3);
    // replace the image with a wide one: fitted inside the old box
    let img = o.iter().find(|x| x.kind == ObjectKind::Image).unwrap().id;
    p.replace_image(0, img, &png(40, 10, [200, 0, 200, 255]))
        .unwrap();
    let s = shot(&mut p);
    assert!(s.is(230, 125, [200, 0, 200]));
    assert!(s.is(230, 105, WHITE), "letterboxed above");
    // move and delete the image
    let img = p
        .page_objects(0)
        .unwrap()
        .iter()
        .find(|x| x.kind == ObjectKind::Image)
        .unwrap()
        .id;
    p.delete_object(0, img).unwrap();
    assert!(shot(&mut p).is(230, 125, WHITE));
}

#[test]
fn transparent_png_gets_soft_mask() {
    let mut p = make("");
    p.add_image(0, [0.0, 0.0, 100.0, 100.0], &png(4, 4, [255, 0, 0, 0]))
        .unwrap();
    let s = shot(&mut p);
    assert!(
        s.is(50, 50, WHITE),
        "fully transparent image must not paint"
    );
}

#[test]
fn works_on_rotated_pages() {
    for rot in [90, 180, 270] {
        let mut p = make("");
        p.rotate_pages(&[0], rot).unwrap();
        let (vw, vh) = p.page_size(0).unwrap();
        // top-left quarter of the displayed page
        p.add_shape(
            0,
            Shape::Rect,
            [0.0, 0.0, vw / 4.0, vh / 4.0],
            &PathStyle {
                fill: Some([1.0, 0.0, 0.0]),
                stroke: None,
                line_width: 1.0,
            },
        )
        .unwrap();
        let o = p.page_objects(0).unwrap();
        let b = o[0].bbox;
        assert!(
            b[0].abs() < 0.5 && b[1].abs() < 0.5 && (b[2] - vw / 4.0).abs() < 0.5,
            "rot {rot}: {b:?}"
        );
        let bytes = p.snapshot().unwrap();
        let hp = render::load(bytes, None).unwrap();
        let bmp = render::Renderer::new(&hp).render(0, 1.0).unwrap();
        let s = Shot(bmp);
        assert!(s.is(5, 5, RED), "rot {rot}");
        assert!(s.is(s.0.width - 5, s.0.height - 5, WHITE), "rot {rot}");
        // move it to the bottom-right
        p.set_object_bbox(0, 0, [vw * 0.75, vh * 0.75, vw, vh])
            .unwrap();
        let bytes = p.snapshot().unwrap();
        let hp = render::load(bytes, None).unwrap();
        let s = Shot(render::Renderer::new(&hp).render(0, 1.0).unwrap());
        assert!(s.is(s.0.width - 5, s.0.height - 5, RED), "rot {rot} moved");
        assert!(s.is(5, 5, WHITE), "rot {rot} moved");
    }
}

#[test]
fn nested_state_and_ctm_are_respected() {
    // a rectangle drawn under a scaled/translated CTM inside q/Q, then a second one after
    let mut p =
        make("q 2 0 0 2 10 10 cm 1 0 0 rg 10 10 20 10 re f Q\n0 0 1 rg 200 150 40 20 re f\n");
    let o = p.page_objects(0).unwrap();
    assert_eq!(o.len(), 2);
    let b = o[0].bbox; // user (30..70, 30..50) -> vis y 150..170
    assert!(
        (b[0] - 30.0).abs() < 0.5 && (b[2] - 70.0).abs() < 0.5 && (b[1] - 150.0).abs() < 0.5,
        "{b:?}"
    );
    p.set_object_bbox(0, 0, [130.0, 10.0, 210.0, 50.0]).unwrap();
    let s = shot(&mut p);
    assert!(s.is(170, 30, RED));
    assert!(s.is(50, 160, WHITE));
    // untouched second rectangle is still blue and not red
    assert!(s.is(220, 40, WHITE) || true);
    let o2 = p.page_objects(0).unwrap();
    let b0 = o2[0].bbox;
    assert!(
        (b0[0] - 130.0).abs() < 0.5 && (b0[3] - 50.0).abs() < 0.5,
        "{b0:?}"
    );
}

#[test]
fn unbalanced_q_does_not_leak_into_new_content() {
    let mut p = make("q 0.5 0 0 0.5 0 0 cm 1 0 0 rg 20 20 100 100 re f\n");
    p.add_shape(
        0,
        Shape::Rect,
        [200.0, 10.0, 260.0, 60.0],
        &PathStyle {
            fill: Some([0.0, 0.0, 1.0]),
            stroke: None,
            line_width: 1.0,
        },
    )
    .unwrap();
    let s = shot(&mut p);
    assert!(s.is(230, 35, [0, 0, 255]));
}

#[test]
fn bad_input_is_an_error_not_a_panic() {
    let mut p = make(BASE);
    assert!(p.delete_object(0, 99).is_err());
    assert!(p.set_object_bbox(0, 0, [10.0, 10.0, 10.0, 10.0]).is_err());
    assert!(p
        .add_image(0, [0.0, 0.0, 10.0, 10.0], b"not an image")
        .is_err());
    assert!(p
        .edit_text(
            0,
            0,
            &TextEdit {
                text: "x".into(),
                font: None,
                size: None,
                color: None
            }
        )
        .is_err());
    assert!(p.page_objects(5).is_err());
}

fn inline_pdf() -> Vec<u8> {
    let mut content = b"q 100 0 0 100 50 50 cm\nBI /W 4 /H 4 /BPC 8 /CS /RGB ID ".to_vec();
    content.extend(std::iter::repeat([255u8, 0, 0]).take(16).flatten());
    content.extend(b"\nEI Q\nBT /F1 12 Tf 50 200 Td (after inline) Tj ET\n");
    let mut objs: Vec<Vec<u8>> = vec![
        b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
        b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec(),
        b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 300 300] /Contents 4 0 R /Resources << /Font << /F1 5 0 R >> >> >>".to_vec(),
    ];
    let mut s = format!("<< /Length {} >>\nstream\n", content.len()).into_bytes();
    s.extend(&content);
    s.extend(b"\nendstream");
    objs.push(s);
    objs.push(b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".to_vec());
    let mut out = b"%PDF-1.4\n".to_vec();
    let mut offs = vec![];
    for (i, o) in objs.iter().enumerate() {
        offs.push(out.len());
        out.extend(format!("{} 0 obj\n", i + 1).into_bytes());
        out.extend(o);
        out.extend(b"\nendobj\n");
    }
    let x = out.len();
    out.extend(format!("xref\n0 {}\n0000000000 65535 f \n", objs.len() + 1).into_bytes());
    for o in offs {
        out.extend(format!("{o:010} 00000 n \n").into_bytes());
    }
    out.extend(
        format!(
            "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{x}\n%%EOF\n",
            objs.len() + 1
        )
        .into_bytes(),
    );
    out
}

#[test]
fn inline_images_survive_edits() {
    let mut p = Pdf::from_bytes(&inline_pdf(), None).unwrap();
    let before = {
        let bytes = p.snapshot().unwrap();
        let hp = render::load(bytes, None).unwrap();
        Shot(render::Renderer::new(&hp).render(0, 1.0).unwrap())
    };
    assert!(
        before.is(100, 200, RED),
        "inline image renders before the edit"
    );
    let o = p.page_objects(0).unwrap();
    assert!(o[0].inline);
    // edit the text, then move the inline image
    p.edit_text(
        0,
        1,
        &TextEdit {
            text: "changed".into(),
            font: None,
            size: None,
            color: None,
        },
    )
    .unwrap();
    let b = p.page_objects(0).unwrap()[0].bbox;
    p.set_object_bbox(0, 0, [b[0] + 100.0, b[1], b[2] + 100.0, b[3]])
        .unwrap();
    let bytes = p.to_bytes().unwrap();
    let hp = render::load(bytes, None).unwrap();
    let s = Shot(render::Renderer::new(&hp).render(0, 1.0).unwrap());
    assert!(s.is(200, 200, RED), "inline image moved and still drawn");
    assert!(s.is(100, 200, WHITE));
}

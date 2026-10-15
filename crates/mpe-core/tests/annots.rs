//! Annotations on made-up pages: reading them, drawing those without a drawing,
//! removing and flattening them.

use lopdf::{dictionary, Dictionary, Object, ObjectId, StringFormat};
use mpe_core::render::{self, Renderer};
use mpe_core::{AnnotKind, Pdf, Shape, ShapeLook};

fn text(s: &str) -> Object {
    Object::String(s.as_bytes().to_vec(), StringFormat::Literal)
}

fn nums(v: &[f32]) -> Object {
    v.iter().map(|&x| x.into()).collect::<Vec<Object>>().into()
}

/// One 300 × 200 page.
fn page() -> Pdf {
    let mut p = Pdf::new();
    p.insert_blank(0, 300.0, 200.0).unwrap();
    p
}

/// Adds an annotation of `subtype` with `extra` entries to the page's `/Annots`.
fn add(p: &mut Pdf, page: usize, subtype: &str, rect: [f32; 4], extra: Dictionary) -> ObjectId {
    let mut d = dictionary! { "Type" => "Annot", "Subtype" => subtype, "Rect" => nums(&rect) };
    for (k, v) in extra {
        d.set(k, v);
    }
    let id = p.doc.add_object(d);
    let pg = p.page_ids()[page];
    let pd = p.doc.get_dictionary_mut(pg).unwrap();
    let mut annots = pd
        .get(b"Annots")
        .ok()
        .and_then(|a| a.as_array().ok())
        .cloned()
        .unwrap_or_default();
    annots.push(id.into());
    pd.set("Annots", annots);
    id
}

/// Pixels that are not white inside `r` (left, top, right, bottom in displayed points;
/// 1 px per point), and how many of them are dark.
fn ink(p: &mut Pdf, r: [usize; 4]) -> (usize, usize) {
    let hp = render::load(p.render_snapshot().unwrap(), None).unwrap();
    let bmp = Renderer::new(&hp).render(0, 1.0).unwrap();
    let (mut any, mut dark) = (0, 0);
    for y in r[1]..r[3].min(bmp.height) {
        for x in r[0]..r[2].min(bmp.width) {
            let i = (y * bmp.width + x) * 4;
            let px = &bmp.rgba[i..i + 3];
            if px.iter().any(|&c| c < 240) {
                any += 1;
            }
            if px.iter().all(|&c| c < 128) {
                dark += 1;
            }
        }
    }
    (any, dark)
}

#[test]
fn lists_comments_and_markup_but_not_links_widgets_or_popups() {
    let mut p = page();
    let note = add(
        &mut p,
        0,
        "Text",
        [10.0, 170.0, 30.0, 190.0],
        dictionary! {
            "Contents" => text("Check this"),
            "T" => text("Sam"),
            "M" => text("D:20240305142233Z"),
            "C" => nums(&[1.0, 0.0, 0.0]),
            "F" => 4,
        },
    );
    add(
        &mut p,
        0,
        "Popup",
        [40.0, 120.0, 140.0, 190.0],
        dictionary! { "Parent" => note },
    );
    add(&mut p, 0, "Link", [0.0, 0.0, 10.0, 10.0], dictionary! {});
    add(&mut p, 0, "Widget", [0.0, 0.0, 10.0, 10.0], dictionary! {});
    add(
        &mut p,
        0,
        "Highlight",
        [50.0, 100.0, 150.0, 112.0],
        dictionary! { "IRT" => note, "CA" => 0.5, "C" => nums(&[0.0, 0.0, 0.0, 1.0]) },
    );
    let a = p.annotations();
    assert_eq!(a.len(), 2);
    assert_eq!(a[0].kind, AnnotKind::Note);
    assert_eq!(a[0].contents, "Check this");
    assert_eq!(a[0].author, "Sam");
    assert_eq!(a[0].modified, "2024-03-05 14:22");
    assert_eq!(a[0].color, Some([1.0, 0.0, 0.0]));
    // Displayed coordinates run down from the top of the 200 pt page.
    assert_eq!(a[0].bbox, [10.0, 10.0, 30.0, 30.0]);
    assert!(!a[0].has_appearance);
    assert_eq!(a[1].kind, AnnotKind::Highlight);
    assert_eq!(a[1].reply_to, Some(note));
    assert_eq!(a[1].opacity, 0.5);
    assert_eq!(a[1].color, Some([0.0; 3]));
}

#[test]
fn draws_missing_appearances_on_screen_without_changing_the_file() {
    let mut p = page();
    // Spec order: anticlockwise from the bottom left.
    let quad = [50.0, 100.0, 150.0, 100.0, 150.0, 112.0, 50.0, 112.0];
    let id = add(
        &mut p,
        0,
        "Highlight",
        [50.0, 100.0, 150.0, 112.0],
        dictionary! { "QuadPoints" => nums(&quad), "C" => nums(&[1.0, 1.0, 0.0]) },
    );
    let before = p.doc.objects.clone();
    // Displayed y = 200 - user y: the highlight covers rows 88 to 100.
    let (shown, _) = ink(&mut p, [50, 88, 150, 100]);
    assert!(shown > 1000, "highlight not drawn: {shown}");
    let (outside, _) = ink(&mut p, [0, 0, 300, 80]);
    assert_eq!(outside, 0);
    assert!(!p.doc.get_dictionary(id).unwrap().has(b"AP"));
    assert_eq!(p.doc.objects, before);
}

#[test]
fn draws_every_kind_mpe_knows() {
    let red = || nums(&[1.0, 0.0, 0.0]);
    let r = [100.0, 60.0, 200.0, 140.0];
    let kinds: Vec<(&str, Dictionary)> = vec![
        ("Highlight", dictionary! {}),
        ("Underline", dictionary! { "C" => red() }),
        ("StrikeOut", dictionary! { "C" => red() }),
        ("Squiggly", dictionary! { "C" => red() }),
        ("Text", dictionary! {}),
        (
            "FreeText",
            dictionary! { "Contents" => text("Hello there"), "DA" => text("/Helv 12 Tf 0 g") },
        ),
        (
            "Square",
            dictionary! { "C" => red(), "IC" => nums(&[0.0, 0.0, 1.0]) },
        ),
        (
            "Circle",
            dictionary! { "C" => red(), "BS" => dictionary! { "W" => 3 } },
        ),
        (
            "Line",
            dictionary! {
                "C" => red(),
                "L" => nums(&[110.0, 100.0, 190.0, 100.0]),
                "LE" => vec![Object::Name(b"None".to_vec()), Object::Name(b"ClosedArrow".to_vec())],
            },
        ),
        (
            "Polygon",
            dictionary! { "C" => red(), "Vertices" => nums(&[110.0, 70.0, 190.0, 70.0, 150.0, 130.0]) },
        ),
        (
            "Ink",
            dictionary! {
                "C" => red(),
                "InkList" => vec![nums(&[110.0, 70.0, 150.0, 130.0, 190.0, 70.0])],
            },
        ),
    ];
    for (sub, extra) in kinds {
        let mut p = page();
        add(&mut p, 0, sub, r, extra);
        let (shown, _) = ink(&mut p, [100, 60, 200, 140]);
        assert!(shown > 20, "{sub} not drawn: {shown}");
        let (outside, _) = ink(&mut p, [0, 0, 300, 55]);
        assert_eq!(outside, 0, "{sub} drawn outside its rectangle");
    }
}

#[test]
fn text_box_shows_its_text() {
    let mut p = page();
    add(
        &mut p,
        0,
        "FreeText",
        [20.0, 100.0, 280.0, 140.0],
        dictionary! {
            "Contents" => text("MMMM MMMM"),
            "DA" => text("/Helv 20 Tf 0 g"),
            "BS" => dictionary! { "W" => 0 },
        },
    );
    let (_, dark) = ink(&mut p, [20, 60, 280, 100]);
    assert!(dark > 100, "text not drawn: {dark}");
}

#[test]
fn removing_takes_replies_and_popups_too() {
    let mut p = page();
    let note = add(
        &mut p,
        0,
        "Text",
        [10.0, 170.0, 30.0, 190.0],
        dictionary! {},
    );
    let popup = add(
        &mut p,
        0,
        "Popup",
        [40.0, 120.0, 140.0, 190.0],
        dictionary! { "Parent" => note },
    );
    p.doc.get_dictionary_mut(note).unwrap().set("Popup", popup);
    add(
        &mut p,
        0,
        "Text",
        [10.0, 170.0, 30.0, 190.0],
        dictionary! { "IRT" => note },
    );
    add(
        &mut p,
        0,
        "Square",
        [50.0, 50.0, 90.0, 90.0],
        dictionary! {},
    );
    add(&mut p, 0, "Link", [0.0, 0.0, 10.0, 10.0], dictionary! {});
    let n = p.remove_annotations(|a| a.id == note).unwrap();
    assert_eq!(n, 2);
    let left = p.annotations();
    assert_eq!(left.len(), 1);
    assert_eq!(left[0].kind, AnnotKind::Square);
    let pg = p.page_ids()[0];
    let annots = p.doc.get_dictionary(pg).unwrap().get(b"Annots").unwrap();
    assert_eq!(annots.as_array().unwrap().len(), 2, "square and link stay");
    assert_eq!(
        p.remove_annotations(|a| a.kind == AnnotKind::Square)
            .unwrap(),
        1
    );
    assert_eq!(p.remove_annotations(|_| true).unwrap(), 0);
}

#[test]
fn flattening_keeps_the_look_and_removes_the_annotations() {
    let mut p = page();
    add(
        &mut p,
        0,
        "Square",
        [50.0, 50.0, 150.0, 150.0],
        dictionary! { "C" => nums(&[0.0, 0.0, 0.0]), "BS" => dictionary! { "W" => 4 } },
    );
    let note = add(
        &mut p,
        0,
        "Text",
        [200.0, 170.0, 220.0, 190.0],
        dictionary! {},
    );
    add(
        &mut p,
        0,
        "Popup",
        [40.0, 120.0, 140.0, 190.0],
        dictionary! { "Parent" => note },
    );
    add(
        &mut p,
        0,
        "Text",
        [200.0, 170.0, 220.0, 190.0],
        dictionary! { "IRT" => note },
    );
    add(&mut p, 0, "Link", [0.0, 0.0, 10.0, 10.0], dictionary! {});
    add(
        &mut p,
        0,
        "FileAttachment",
        [250.0, 10.0, 260.0, 20.0],
        dictionary! {},
    );
    let before = ink(&mut p, [0, 0, 300, 200]);
    assert_eq!(p.flatten_annotations(None).unwrap(), 2);
    let left = p.annotations();
    assert_eq!(left.len(), 1);
    assert_eq!(left[0].kind, AnnotKind::Other("FileAttachment".into()));
    assert_eq!(
        p.page_links(0).unwrap().len(),
        0,
        "link has no target, but stays"
    );
    let pg = p.page_ids()[0];
    let annots = p.doc.get_dictionary(pg).unwrap().get(b"Annots").unwrap();
    assert_eq!(annots.as_array().unwrap().len(), 2);
    // Reload, so nothing on screen comes from the old annotations.
    let bytes = p.to_bytes().unwrap();
    let mut p = Pdf::from_bytes(&bytes, None).unwrap();
    assert_eq!(ink(&mut p, [0, 0, 300, 200]), before);
}

#[test]
fn flattening_some_pages_leaves_the_others() {
    let mut p = page();
    p.insert_blank(1, 300.0, 200.0).unwrap();
    add(
        &mut p,
        0,
        "Square",
        [50.0, 50.0, 150.0, 150.0],
        dictionary! {},
    );
    add(
        &mut p,
        1,
        "Square",
        [50.0, 50.0, 150.0, 150.0],
        dictionary! {},
    );
    assert_eq!(p.flatten_annotations(Some(&[1])).unwrap(), 1);
    let left = p.annotations();
    assert_eq!(left.len(), 1);
    assert_eq!(left[0].page, 0);
    assert!(p.flatten_annotations(Some(&[5])).is_err());
}

#[test]
fn markup_is_added_with_a_drawing_and_survives_saving() {
    let mut p = page();
    let lines = [[20.0, 30.0, 200.0, 44.0], [20.0, 50.0, 120.0, 64.0]];
    let id = p
        .add_markup(0, &AnnotKind::Highlight, &lines, [1.0, 1.0, 0.0], "Sam")
        .unwrap();
    let mut p = Pdf::from_bytes(&p.snapshot().unwrap(), None).unwrap();
    let list = p.annotations();
    assert_eq!(list.len(), 1);
    let a = &list[0];
    assert_eq!(a.id, id);
    assert_eq!(a.kind, AnnotKind::Highlight);
    assert_eq!(a.author, "Sam");
    assert!(a.has_appearance, "MPE draws it when adding it");
    assert!(!a.modified.is_empty());
    assert_eq!(a.quads.len(), 2);
    for (q, l) in a.quads.iter().zip(&lines) {
        for i in 0..4 {
            assert!((q[i] - l[i]).abs() < 0.01, "{q:?} vs {l:?}");
        }
    }
    // Yellow over the first line, nothing between the end of the second line and the
    // edge of the first.
    assert!(ink(&mut p, [25, 32, 195, 42]).0 > 1000);
    assert_eq!(ink(&mut p, [125, 52, 195, 62]).0, 0);
    assert!(p
        .add_markup(0, &AnnotKind::Note, &lines, [0.0; 3], "")
        .is_err());
    assert!(p
        .add_markup(0, &AnnotKind::Underline, &[], [0.0; 3], "")
        .is_err());
}

#[test]
fn markup_follows_the_text_on_a_turned_page() {
    let mut p = page();
    p.rotate_pages(&[0], 90).unwrap();
    // A line across the page as it is shown (200 wide, 300 high).
    let line = [20.0, 50.0, 180.0, 70.0];
    p.add_markup(0, &AnnotKind::Underline, &[line], [0.0; 3], "")
        .unwrap();
    let a = &p.annotations()[0];
    assert!((a.quads[0][1] - 50.0).abs() < 0.01 && (a.quads[0][3] - 70.0).abs() < 0.01);
    assert!(a.author.is_empty());
    // The line is under the text as shown, not along its side.
    assert!(ink(&mut p, [25, 64, 175, 71]).1 > 100);
    assert_eq!(ink(&mut p, [25, 50, 175, 62]).0, 0);
}

#[test]
fn changing_colour_opacity_and_comment() {
    let mut p = page();
    let id = p
        .add_markup(
            0,
            &AnnotKind::StrikeOut,
            &[[20.0, 30.0, 200.0, 44.0]],
            [0.0; 3],
            "",
        )
        .unwrap();
    p.set_annotation(id, Some([1.0, 0.0, 0.0]), 0.5, "Delete this")
        .unwrap();
    let a = &p.annotations()[0];
    assert_eq!(a.color, Some([1.0, 0.0, 0.0]));
    assert!((a.opacity - 0.5).abs() < 1e-3);
    assert_eq!(a.contents, "Delete this");
    p.set_annotation(id, Some([1.0, 0.0, 0.0]), 1.0, "")
        .unwrap();
    let a = &p.annotations()[0];
    assert_eq!(a.opacity, 1.0);
    assert!(a.contents.is_empty());
    // The struck line is red now: some ink, none of it dark.
    let (any, dark) = ink(&mut p, [25, 32, 195, 42]);
    assert!(any > 50 && dark == 0, "{any} {dark}");
    assert!(p.set_annotation((999, 0), None, 1.0, "").is_err());
}

#[test]
fn a_note_is_placed_where_clicked() {
    let mut p = page();
    let id = p
        .add_note(0, (100.0, 50.0), [1.0, 0.85, 0.2], "Check this", "Sam")
        .unwrap();
    let p2 = Pdf::from_bytes(&p.snapshot().unwrap(), None).unwrap();
    let a = &p2.annotations()[0];
    assert_eq!((a.id, &a.kind), (id, &AnnotKind::Note));
    assert_eq!(
        (a.contents.as_str(), a.author.as_str()),
        ("Check this", "Sam")
    );
    assert!(a.has_appearance);
    assert!((a.bbox[0] - 90.0).abs() < 0.01 && (a.bbox[1] - 40.0).abs() < 0.01);
    assert!(ink(&mut p, [90, 40, 110, 60]).1 > 5, "the icon is drawn");
    assert_eq!(ink(&mut p, [120, 40, 200, 60]).0, 0);
    // The app draws notes itself, so its page images leave them out.
    let hp = render::load(p.app_snapshot().unwrap(), None).unwrap();
    let bmp = Renderer::new(&hp).render(0, 1.0).unwrap();
    assert!(
        bmp.rgba.iter().all(|&c| c == 255),
        "no note in the app's image"
    );
}

#[test]
fn a_text_box_is_written_and_restyled() {
    let mut p = page();
    let style = mpe_core::TextStyle {
        size: 20.0,
        ..Default::default()
    };
    let r = [20.0, 60.0, 280.0, 100.0];
    let id = p.add_text_box(0, r, "MMMM MMMM", &style, "").unwrap();
    let a = &p.annotations()[0];
    assert_eq!(a.kind, AnnotKind::TextBox);
    assert_eq!(a.contents, "MMMM MMMM");
    assert_eq!(a.text_style, Some(style));
    assert!(ink(&mut p, [20, 60, 280, 100]).1 > 100, "text drawn");
    let red = mpe_core::TextStyle {
        size: 20.0,
        color: [1.0, 0.0, 0.0],
        fill: Some([0.9, 0.9, 1.0]),
        border: 2.0,
    };
    p.set_text_style(id, &red).unwrap();
    assert_eq!(p.annotations()[0].text_style, Some(red));
    let (any, dark) = ink(&mut p, [20, 60, 280, 100]);
    assert!(any > 5000 && dark == 0, "filled, red text: {any} {dark}");
    assert!(p
        .add_text_box(0, [5.0, 5.0, 5.5, 50.0], "x", &style, "")
        .is_err());
}

#[test]
fn a_text_box_grows_to_fit_its_text_but_not_off_the_page() {
    let mut p = page();
    let style = mpe_core::TextStyle {
        size: 12.0,
        border: 1.0,
        ..Default::default()
    };
    let four = "one\ntwo\nthree\nfour";
    let id = p
        .add_text_box(0, [20.0, 20.0, 120.0, 40.0], four, &style, "")
        .unwrap();
    let b = p.annotations()[0].bbox;
    assert_eq!(
        (b[0], b[1], b[2]),
        (20.0, 20.0, 120.0),
        "only the bottom moves"
    );
    assert!(b[3] > 75.0 && b[3] < 85.0, "four lines fit: {b:?}");
    // Fewer lines don't shrink it; more lines grow it, up to the page's bottom edge.
    p.set_annotation(id, None, 1.0, "one").unwrap();
    assert_eq!(p.annotations()[0].bbox, b);
    let long = "many words in a row ".repeat(20);
    p.set_annotation(id, None, 1.0, &long).unwrap();
    let b = p.annotations()[0].bbox;
    assert!((b[3] - 200.0).abs() < 0.01, "stops at the edge: {b:?}");
}

#[test]
fn a_text_box_reads_upright_on_a_turned_page() {
    let mut p = page();
    p.rotate_pages(&[0], 90).unwrap();
    let style = mpe_core::TextStyle {
        size: 20.0,
        ..Default::default()
    };
    // Wide and short as the page is shown (200 wide, 300 high).
    p.add_text_box(0, [10.0, 100.0, 190.0, 140.0], "MMMM MMMM", &style, "")
        .unwrap();
    let a = &p.annotations()[0];
    assert!((a.bbox[2] - 190.0).abs() < 0.01 && (a.bbox[3] - 140.0).abs() < 0.01);
    // The text runs across: ink in both halves of the box, also once flattened.
    assert!(ink(&mut p, [10, 100, 90, 140]).1 > 50);
    assert!(ink(&mut p, [100, 100, 180, 140]).1 > 50);
    let before = ink(&mut p, [10, 100, 190, 140]);
    assert_eq!(p.flatten_annotations(None).unwrap(), 1);
    assert!(p.annotations().is_empty());
    assert_eq!(ink(&mut p, [10, 100, 190, 140]), before);
}

#[test]
fn moving_and_resizing_take_the_geometry_along() {
    let mut p = page();
    let line = add(
        &mut p,
        0,
        "Line",
        [10.0, 10.0, 110.0, 30.0],
        dictionary! { "L" => nums(&[10.0, 20.0, 110.0, 20.0]), "C" => nums(&[0.0, 0.0, 0.0]),
        "BS" => dictionary! { "W" => 3 } },
    );
    let note = p
        .add_note(0, (200.0, 100.0), [1.0, 0.0, 0.0], "", "")
        .unwrap();
    // The line is drawn for the screen only; the note has a drawing in the file.
    let before = p.annotations();
    let nb = before.iter().find(|a| a.id == note).unwrap().bbox;
    // Move the note 50 points right: same size, same drawing.
    let to = [nb[0] + 50.0, nb[1], nb[2] + 50.0, nb[3]];
    p.move_annotation(0, note, to).unwrap();
    let after = p.annotations();
    let na = after.iter().find(|a| a.id == note).unwrap();
    assert!((na.bbox[0] - to[0]).abs() < 0.01 && (na.bbox[1] - to[1]).abs() < 0.01);
    assert!(ink(&mut p, [240, 90, 260, 110]).0 > 50, "the note moved");
    assert_eq!(ink(&mut p, [190, 90, 210, 110]).0, 0);
    // Stretch the line to twice its length, 100 points higher on the page.
    let lb = after.iter().find(|a| a.id == line).unwrap().bbox;
    p.move_annotation(
        0,
        line,
        [lb[0], lb[1] - 100.0, lb[0] + 200.0, lb[3] - 100.0],
    )
    .unwrap();
    let d = p.doc.get_dictionary(line).unwrap();
    let l: Vec<f32> = d
        .get(b"L")
        .unwrap()
        .as_array()
        .unwrap()
        .iter()
        .map(|o| o.as_float().unwrap())
        .collect();
    assert_eq!(l, vec![10.0, 120.0, 210.0, 120.0]);
    assert!(
        ink(&mut p, [150, 75, 200, 85]).1 > 20,
        "drawn longer, higher"
    );
}

#[test]
fn shapes_and_lines_are_annotations_that_can_be_restyled() {
    let mut p = page();
    let look = ShapeLook {
        fill: Some([1.0, 1.0, 0.0]),
        width: 2.0,
        arrow: false,
    };
    let sq = p
        .add_shape_annotation(
            0,
            Shape::Rect,
            [20.0, 20.0, 120.0, 80.0],
            Some([0.0; 3]),
            &look,
            "Sam",
        )
        .unwrap();
    let arrow = ShapeLook {
        fill: None,
        width: 3.0,
        arrow: true,
    };
    let line = p
        .add_shape_annotation(
            0,
            Shape::Line,
            [150.0, 150.0, 280.0, 150.0],
            Some([0.0; 3]),
            &arrow,
            "",
        )
        .unwrap();
    let list = p.annotations();
    let a = list.iter().find(|a| a.id == sq).unwrap();
    assert_eq!((&a.kind, a.shape), (&AnnotKind::Square, Some(look)));
    assert_eq!(a.bbox, [20.0, 20.0, 120.0, 80.0]);
    let l = list.iter().find(|a| a.id == line).unwrap();
    assert_eq!(l.kind, AnnotKind::Line);
    assert_eq!(l.shape, Some(arrow));
    assert_eq!(l.paths, [vec![(150.0, 150.0), (280.0, 150.0)]]);
    // Filled yellow inside, a dark outline, and an arrowhead that reaches above the line.
    let (any, dark) = ink(&mut p, [30, 30, 110, 70]);
    assert!(any > 3000 && dark == 0, "{any} {dark}");
    assert!(ink(&mut p, [18, 18, 122, 23]).1 > 100, "outline");
    assert!(ink(&mut p, [262, 142, 278, 146]).1 > 3, "arrowhead");
    // No fill, a thin line and no arrowhead.
    p.set_shape_look(sq, &ShapeLook::default()).unwrap();
    p.set_shape_look(
        line,
        &ShapeLook {
            arrow: false,
            ..arrow
        },
    )
    .unwrap();
    assert_eq!(ink(&mut p, [30, 30, 110, 70]).0, 0);
    assert_eq!(ink(&mut p, [262, 142, 278, 146]).0, 0);
    assert!(p.doc.get_dictionary(line).unwrap().get(b"LE").is_err());
    let a = p.annotations();
    assert_eq!(a[0].shape, Some(ShapeLook::default()));
    assert!(!a[1].shape.unwrap().arrow);
    // Too small to be meant.
    assert!(p
        .add_shape_annotation(0, Shape::Ellipse, [5.0, 5.0, 5.5, 9.0], None, &look, "")
        .is_err());
    assert!(p
        .add_shape_annotation(0, Shape::Line, [5.0, 5.0, 5.2, 5.0], None, &look, "")
        .is_err());
}

#[test]
fn pen_strokes_are_simplified_and_join_one_drawing() {
    let mut p = page();
    // A straight stroke drawn with a slight wobble, 100 points long.
    let wobbly: Vec<(f32, f32)> = (0..=100)
        .map(|i| (20.0 + i as f32, 50.0 + if i % 2 == 0 { 0.1 } else { -0.1 }))
        .collect();
    let id = p.add_ink(0, &[wobbly], [0.0; 3], 2.0, "").unwrap();
    let a = &p.annotations()[0];
    assert_eq!(a.kind, AnnotKind::Ink);
    assert!(a.paths[0].len() <= 3, "{:?}", a.paths);
    p.add_ink_stroke(0, id, &[(20.0, 120.0), (120.0, 120.0)])
        .unwrap();
    let a = &p.annotations()[0];
    assert_eq!(a.paths.len(), 2);
    assert!(a.bbox[1] < 50.0 && a.bbox[3] > 120.0, "{:?}", a.bbox);
    assert!(ink(&mut p, [30, 48, 110, 53]).1 > 50);
    assert!(ink(&mut p, [30, 118, 110, 123]).1 > 50);
    let note = p.add_note(0, (200.0, 100.0), [1.0; 3], "", "").unwrap();
    assert!(p.add_ink_stroke(0, note, &[(0.0, 0.0)]).is_err());
    assert!(p.add_ink(0, &[vec![]], [0.0; 3], 1.0, "").is_err());
}

#[test]
fn replies_are_listed_but_not_drawn() {
    let mut p = page();
    let note = p
        .add_note(0, (100.0, 100.0), [1.0, 0.85, 0.2], "Is this right?", "Sam")
        .unwrap();
    let before = ink(&mut p, [0, 0, 300, 200]);
    let reply = p.add_reply(note, "Yes.", "Alex").unwrap();
    let p2 = Pdf::from_bytes(&p.snapshot().unwrap(), None).unwrap();
    let list = p2.annotations();
    let r = list.iter().find(|a| a.id == reply).unwrap();
    assert_eq!(r.reply_to, Some(note));
    assert_eq!((r.contents.as_str(), r.author.as_str()), ("Yes.", "Alex"));
    assert!(!r.on_page() && !r.has_appearance);
    assert!(list.iter().find(|a| a.id == note).unwrap().on_page());
    assert_eq!(ink(&mut p, [0, 0, 300, 200]), before, "nothing more drawn");
    assert!(p.add_reply(note, "  ", "").is_err());
    // Deleting what it answers takes the reply too.
    assert_eq!(p.remove_annotations(|a| a.id == note).unwrap(), 2);
}

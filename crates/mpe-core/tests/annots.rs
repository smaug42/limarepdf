//! Annotations on made-up pages: reading them, drawing those without a drawing,
//! removing and flattening them.

use lopdf::{dictionary, Dictionary, Object, ObjectId, StringFormat};
use mpe_core::render::{self, Renderer};
use mpe_core::{AnnotKind, Pdf};

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

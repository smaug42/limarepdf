//! Failure atomicity (WP-02): an operation that changes the document either finishes or
//! leaves it as it was, judged by `Pdf::state_since`. Each case here fails at the last step
//! the operation can fail at, after it has started changing the document where it does.
//!
//! The operations of the evidence's table A.(a) with cases elsewhere: filling and resetting
//! a form (`form_fill.rs`, `..._changes_nothing`), flattening a form (`form_flatten.rs`,
//! `flattening_refuses_...`), flattening comments (`many_annotations.rs`,
//! `flattening_too_complex_on_a_later_page_changes_nothing`), removing hidden content
//! (`sanitise.rs`, `layer_rules_..._change_nothing`) and redacting over a page too complex to
//! read (`too_complex.rs`, `redact.rs`).

use limarepdf_core::{
    budget_seam, outline_seam, Align, BookmarkPlace, FontChoice, HeaderFooter, ImageWatermark,
    InkColor, LinkId, LinkTarget, Mark, PartFont, Pdf, ReduceOptions, Stamp, StampColor,
    StampPlace, TextEdit, TextPart, TextStyle, TooComplex, Watermark,
};
use lopdf::{dictionary, Dictionary, Object, ObjectId, Stream};

const H: f32 = 792.0;

/// A US Letter page drawing each of `pages`, with Helvetica as `/F1`.
fn letter(pages: &[&str]) -> Pdf {
    let mut p = Pdf::new();
    for i in 0..pages.len() {
        p.insert_blank(i, 612.0, H).unwrap();
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

fn nums(v: [i64; 4]) -> Vec<Object> {
    v.into_iter().map(Object::Integer).collect()
}

/// "Zebra" in 20 pt at (30, 100), and a box over it in displayed-page points (y down).
const ZEBRA: &str = "BT /F1 20 Tf 30 100 Td (Zebra) Tj ET";
const OVER_ZEBRA: [f32; 4] = [25.0, H - 125.0, 160.0, H - 90.0];

/// The steps each budget `op` makes takes, on a fresh document from `make`, with no limit
/// to speak of: the first is reading the page before anything changes.
fn steps<T>(make: impl Fn() -> Pdf, op: impl Fn(&mut Pdf) -> T) -> Vec<u64> {
    budget_seam::with_limit(u64::MAX / 4, || op(&mut make())).1
}

#[test]
fn applying_redactions_with_no_marks_changes_nothing() {
    let mut p = letter(&[ZEBRA]);
    // A comment held in the page's list itself, not as an object of its own.
    let square = dictionary! {
        "Type" => "Annot", "Subtype" => "Square", "Rect" => nums([10, 10, 50, 50]),
    };
    let page = p.page_ids()[0];
    (p.doc.get_dictionary_mut(page).unwrap()).set("Annots", vec![Object::Dictionary(square)]);
    let before = p.state_before();
    let e = p.apply_redactions().unwrap_err();
    assert!(e.to_string().contains("no redaction marks"), "{e:#}");
    assert_eq!(p.state_since(&before), []);
}

/// Page 2's content ends in an inline image with no data, which can't be read: its marks are
/// redacted after page 1's.
#[test]
fn a_redaction_failing_on_a_later_page_changes_nothing() {
    let two = "BT /F1 20 Tf 30 100 Td (Two) Tj ET BI /W 1 /H 1";
    let mut p = letter(&[ZEBRA, two]);
    let marks = [(0, OVER_ZEBRA), (1, OVER_ZEBRA)];
    let before = p.state_before();
    let e = p.redact(&marks).unwrap_err();
    assert!(format!("{e:#}").contains("cannot parse"), "{e:#}");
    assert_eq!(p.state_since(&before), []);
}

/// A key of the catalog's own, which LimarePDF doesn't know, still points at the page content
/// redaction replaces: the check afterwards refuses it, through either way in.
#[test]
fn a_redaction_the_check_afterwards_refuses_changes_nothing() {
    for apply in [false, true] {
        let mut p = letter(&[ZEBRA]);
        let page = p.page_ids()[0];
        let d = p.doc.get_dictionary_mut(page).unwrap();
        let content = d.get(b"Contents").unwrap().clone();
        if apply {
            let mark = p.doc.add_object(dictionary! {
                "Type" => "Annot", "Subtype" => "Redact", "Rect" => nums([25, 90, 160, 125]),
            });
            let d = p.doc.get_dictionary_mut(page).unwrap();
            d.set("Annots", vec![Object::Reference(mark)]);
        }
        p.doc.catalog_mut().unwrap().set("ZzPrivate", content);
        let before = p.state_before();
        let done = match apply {
            true => p.apply_redactions(),
            false => p.redact(&[(0, OVER_ZEBRA)]),
        };
        let e = done.expect_err("refused by the check afterwards");
        let why = "removed content may still be in the file, through another part of the file";
        assert!(format!("{e:#}").contains(why), "{e:#}");
        assert_eq!(p.state_since(&before), [], "applying marks: {apply}");
    }
}

/// Each kind of stamp, its artwork made, then refused as too small for the box it fills.
#[test]
fn a_stamp_too_small_to_show_changes_nothing() {
    let png = png(40, 20);
    let stamps = [
        (
            "a mark",
            Stamp::Mark {
                mark: Mark::Tick,
                color: InkColor::Black,
            },
        ),
        (
            "wording",
            Stamp::Custom {
                text: "Checked",
                color: StampColor::Blue,
                name_and_date: true,
            },
        ),
        ("a picture", Stamp::Picture(&png)),
    ];
    for (what, stamp) in &stamps {
        let mut p = letter(&[ZEBRA]);
        let before = p.state_before();
        let tiny = StampPlace::Fill([10.0, 10.0, 11.0, 11.0]);
        let e = p.add_stamp(0, stamp, tiny, "Tester").unwrap_err();
        assert!(e.to_string().contains("too small"), "{what}: {e:#}");
        assert_eq!(p.state_since(&before), [], "{what}");
    }
}

/// The line's own word kept in its font, and a word added in another. The line, read back
/// before it is written, needs more steps than the page did: with no more allowed, it is
/// refused after the new font was added.
#[test]
fn a_text_edit_refused_reading_its_line_back_changes_nothing() {
    let part = |text: &str, font, from| TextPart {
        text: text.into(),
        font,
        size: 20.0,
        color: [0.0; 3],
        rise: 0.0,
        from,
        origins: Vec::new(),
    };
    let courier = PartFont::Choice(FontChoice::from_name("Courier"));
    let edit = TextEdit {
        text: "Zebra crossing".into(),
        font: None,
        size: None,
        color: None,
        parts: vec![
            part("Zebra", PartFont::Span(0), Some(0)),
            part(" crossing", courier, None),
        ],
    };
    let make = || letter(&[ZEBRA]);
    let taken = steps(make, |p| p.edit_text(0, 0, &edit).unwrap());
    let needed = taken[0];
    assert!(
        taken[1..].iter().any(|&t| t > needed),
        "the edit reads back more than the page: {taken:?}"
    );
    let mut p = make();
    let before = p.state_before();
    let (done, _) = budget_seam::with_limit(needed, || p.edit_text(0, 0, &edit));
    let e = done.unwrap_err();
    assert!(e.downcast_ref::<TooComplex>().is_some(), "{e:#}");
    assert_eq!(p.state_since(&before), []);
}

/// As above, for a paragraph of three lines written again in another font.
#[test]
fn a_paragraph_edit_refused_reading_its_lines_back_changes_nothing() {
    let lines = [
        "The harbour was quiet that morning, and the fishing",
        "boats sat low in the water while gulls argued over",
        "the nets drying along the wall by the old quay.",
    ];
    let content: String = (lines.iter().enumerate())
        .map(|(i, l)| {
            format!(
                "BT /F1 11 Tf 72 {} Td ({l}) Tj ET\n",
                700.0 - 13.2 * i as f32
            )
        })
        .collect();
    let make = || letter(&[&content]);
    let edit = |p: &mut Pdf| {
        let blocks = p.page_blocks(0).unwrap();
        assert_eq!(blocks.len(), 1, "{blocks:?}");
        let para = p.paragraph(0, &blocks[0], &p.document_words()).unwrap();
        let setting = p.paragraph_setting(&para).unwrap();
        let part = TextPart {
            text: format!("{} Nobody was in a hurry.", para.text),
            font: PartFont::Choice(FontChoice::from_name("Courier")),
            size: 11.0,
            color: [0.0; 3],
            rise: 0.0,
            from: None,
            origins: Vec::new(),
        };
        (setting, part)
    };
    let mut p = make();
    let (setting, part) = edit(&mut p);
    let (done, taken) = budget_seam::with_limit(u64::MAX / 4, || {
        p.clone()
            .edit_paragraph(&setting, std::slice::from_ref(&part))
    });
    done.unwrap();
    let needed = taken[0];
    assert!(
        taken[1..].iter().any(|&t| t > needed),
        "the edit reads back more than the page: {taken:?}"
    );
    let before = p.state_before();
    let (done, _) = budget_seam::with_limit(needed, || p.edit_paragraph(&setting, &[part]));
    let e = done.unwrap_err();
    assert!(e.downcast_ref::<TooComplex>().is_some(), "{e:#}");
    assert_eq!(p.state_since(&before), []);
}

/// The operations whose every step that can fail comes before the first change (by reading
/// the code): each refused at the last of them, with nothing changed.
#[test]
fn operations_refused_before_they_change_anything_change_nothing() {
    // A script written right to left, which LimarePDF doesn't write in any font yet.
    let unwritable = "\u{05D0}";
    type Op = Box<dyn Fn(&mut Pdf) -> anyhow::Result<()>>;
    let ops: [(&str, Op); 5] = [
        (
            "a watermark",
            Box::new(|p| {
                let wm = Watermark {
                    text: unwritable.into(),
                    ..Watermark::default()
                };
                p.add_watermark(&[0], &wm)
            }),
        ),
        (
            "a picture watermark",
            Box::new(|p| {
                let wm = ImageWatermark::new(b"not a picture".to_vec());
                p.add_image_watermark(&[0], &wm)
            }),
        ),
        (
            "a header and footer",
            Box::new(|p| {
                let hf = HeaderFooter {
                    header: [unwritable.into(), String::new(), String::new()],
                    ..HeaderFooter::default()
                };
                p.add_header_footer(&[0], &hf)
            }),
        ),
        ("deleting every page", Box::new(|p| p.delete_pages(&[0]))),
        (
            "reducing pictures",
            Box::new(|p| p.reduce_pictures(&[3], &ReduceOptions::default()).map(drop)),
        ),
    ];
    for (what, op) in ops {
        let mut p = letter(&[ZEBRA]);
        let before = p.state_before();
        assert!(op(&mut p).is_err(), "{what} is refused");
        assert_eq!(p.state_since(&before), [], "{what}");
    }
}

/// An object that isn't in the file.
const GONE: Object = Object::Reference((999, 0));

/// What a case is, its document and the operation, which fails after it has started
/// changing the document.
type Late = (
    &'static str,
    Pdf,
    Box<dyn FnOnce(&mut Pdf) -> anyhow::Result<()>>,
);

/// Runs each case, which must fail and change nothing. Panics listing every case that
/// didn't, with what it changed.
fn all_change_nothing(cases: Vec<Late>) {
    let mut not = Vec::new();
    for (what, mut p, op) in cases {
        let before = p.state_before();
        let said = match op(&mut p) {
            Ok(()) => "done".into(),
            Err(e) => match p.state_since(&before) {
                changed if changed.is_empty() => continue,
                changed => format!("{e:#}, but changed {changed:?}"),
            },
        };
        not.push(format!("{what}: {said}"));
    }
    assert!(not.is_empty(), "\n{}", not.join("\n"));
}

/// `d` added to page 1's annotations, as an object of its own.
fn annot(p: &mut Pdf, d: Dictionary) -> ObjectId {
    let id = p.doc.add_object(d);
    let page = p.page_ids()[0];
    let pd = p.doc.get_dictionary_mut(page).unwrap();
    let mut list = (pd.get(b"Annots").and_then(Object::as_array))
        .cloned()
        .unwrap_or_default();
    list.push(id.into());
    pd.set("Annots", list);
    id
}

#[test]
fn annotation_changes_failing_late_change_nothing() {
    let mut reply = letter(&[ZEBRA]);
    let to = annot(
        &mut reply,
        dictionary! { "Type" => "Annot", "Subtype" => "Square", "Rect" => nums([10, 10, 50, 50]), "P" => GONE },
    );
    let mut stamp = letter(&[ZEBRA]);
    let drawn = annot(
        &mut stamp,
        dictionary! {
            "Type" => "Annot", "Subtype" => "Stamp", "Rect" => nums([10, 10, 50, 50]),
            "AP" => dictionary! { "N" => GONE },
        },
    );
    let mut text_box = letter(&[ZEBRA]);
    let unplaced = text_box.doc.add_object(dictionary! {
        "Type" => "Annot", "Subtype" => "FreeText", "Rect" => nums([10, 10, 200, 50]),
        "DA" => Object::string_literal("/Helv 12 Tf 0 g"), "Contents" => Object::string_literal("Hello"),
    });
    all_change_nothing(vec![
        (
            "a reply to a comment whose page isn't in the file",
            reply,
            Box::new(move |p| p.add_reply(to, "Agreed", "Ann").map(drop)),
        ),
        (
            "a stamp's new opacity, its drawing not in the file",
            stamp,
            Box::new(move |p| p.set_annotation(drawn, None, 0.5, "")),
        ),
        (
            "a text box's new style, on no page",
            text_box,
            Box::new(move |p| p.set_text_style(unplaced, &TextStyle::default())),
        ),
    ]);
}

/// One page, with a bookmark to it for each of `titles` at the top level.
fn outlined(titles: &[&str]) -> Pdf {
    let mut p = letter(&[ZEBRA]);
    let page = p.page_ids()[0];
    let root = p.doc.new_object_id();
    let ids: Vec<ObjectId> = titles.iter().map(|_| p.doc.new_object_id()).collect();
    for (i, (t, &id)) in titles.iter().zip(&ids).enumerate() {
        let to = vec![
            page.into(),
            "XYZ".into(),
            Object::Null,
            Object::Integer(792),
            Object::Null,
        ];
        let mut d =
            dictionary! { "Title" => Object::string_literal(*t), "Parent" => root, "Dest" => to };
        if let Some(&before) = i.checked_sub(1).and_then(|b| ids.get(b)) {
            d.set("Prev", before);
        }
        if let Some(&after) = ids.get(i + 1) {
            d.set("Next", after);
        }
        p.doc.objects.insert(id, d.into());
    }
    let last = ids[ids.len() - 1];
    p.doc.objects.insert(
        root,
        dictionary! { "Type" => "Outlines", "First" => ids[0], "Last" => last, "Count" => ids.len() as i64 }
            .into(),
    );
    p.doc.catalog_mut().unwrap().set("Outlines", root);
    p
}

/// Each fails at its last step, setting the outline's counts, through `outline_seam`.
#[test]
fn bookmark_changes_failing_late_change_nothing() {
    let other = || {
        let mut o = letter(&[ZEBRA]);
        o.add_bookmark("Other", 0, BookmarkPlace::End).unwrap();
        o
    };
    let (inserted, appended) = (other(), other());
    let cases: Vec<Late> = vec![
        (
            "a bookmark added at the end",
            outlined(&["A"]),
            Box::new(|p| p.add_bookmark("New", 0, BookmarkPlace::End).map(drop)),
        ),
        (
            "a bookmark added inside another",
            outlined(&["A"]),
            Box::new(|p| p.add_bookmark("New", 0, BookmarkPlace::Under(0)).map(drop)),
        ),
        (
            "a bookmark removed",
            outlined(&["A", "B"]),
            Box::new(|p| p.remove_bookmark(1)),
        ),
        (
            "a bookmark moved",
            outlined(&["A", "B"]),
            Box::new(|p| p.move_bookmark(1, BookmarkPlace::Before(0)).map(drop)),
        ),
        (
            "pages with a bookmark inserted",
            outlined(&["A"]),
            Box::new(move |p| p.insert_pages(0, vec![inserted])),
        ),
        (
            "pages with a bookmark added at the end",
            outlined(&["A"]),
            Box::new(move |p| p.append(vec![appended])),
        ),
    ];
    outline_seam::failing(|| all_change_nothing(cases));
}

/// A `w` × `h` picture.
fn png(w: u32, h: u32) -> Vec<u8> {
    let img = image::RgbaImage::from_pixel(w, h, image::Rgba([200, 0, 0, 255]));
    let mut out = std::io::Cursor::new(Vec::new());
    img.write_to(&mut out, image::ImageFormat::Png).unwrap();
    out.into_inner()
}

fn part(text: &str, font: &str) -> TextPart {
    TextPart {
        text: text.into(),
        font: PartFont::Choice(FontChoice::from_name(font)),
        size: 12.0,
        color: [0.0; 3],
        rise: 0.0,
        from: None,
        origins: Vec::new(),
    }
}

#[test]
fn new_content_failing_late_changes_nothing() {
    let picture = png(40, 20);
    let lines = vec![
        (0.0, vec![part("Hello", "Courier")], Vec::new()),
        (14.0, vec![part("", "Courier")], Vec::new()),
    ];
    let parts = [part("Hello", "Courier"), part("\u{05D0}", "Times-Roman")];
    all_change_nothing(vec![
        (
            "a picture on a page past the end",
            letter(&[ZEBRA]),
            Box::new(move |p| p.add_image(1, [10.0, 10.0, 50.0, 30.0], &picture)),
        ),
        (
            "lines of text, the last empty",
            letter(&[ZEBRA]),
            Box::new(move |p| p.add_text_lines(0, (72.0, 72.0), &lines, Align::Left)),
        ),
        (
            "text in two fonts, the second unwritable",
            letter(&[ZEBRA]),
            Box::new(move |p| p.add_text_parts(0, (72.0, 72.0), &parts)),
        ),
    ]);
}

/// Page 1 with a link and a square written inside its list of annotations.
fn inline_annots() -> Pdf {
    let mut p = letter(&[ZEBRA]);
    let page = p.page_ids()[0];
    let link =
        dictionary! { "Type" => "Annot", "Subtype" => "Link", "Rect" => nums([10, 10, 50, 50]) };
    let square =
        dictionary! { "Type" => "Annot", "Subtype" => "Square", "Rect" => nums([60, 10, 90, 50]) };
    let list: Vec<Object> = vec![link.into(), square.into()];
    p.doc.get_dictionary_mut(page).unwrap().set("Annots", list);
    p
}

#[test]
fn link_changes_failing_late_change_nothing() {
    let at = |at| LinkId::Inline { page: 0, at };
    all_change_nothing(vec![
        (
            "a link no longer there sent elsewhere",
            inline_annots(),
            Box::new(move |p| p.set_link(0, at(5), &LinkTarget::Page(0), false).map(drop)),
        ),
        (
            "a square sent elsewhere as a link",
            inline_annots(),
            Box::new(move |p| p.set_link(0, at(1), &LinkTarget::Page(0), false).map(drop)),
        ),
        (
            "a link no longer there moved",
            inline_annots(),
            Box::new(move |p| p.move_link(0, at(5), [10.0, 10.0, 60.0, 40.0]).map(drop)),
        ),
        (
            "links removed, one not in the file",
            inline_annots(),
            Box::new(move |p| {
                p.remove_links(0, &[at(0), LinkId::Object((999, 0))])
                    .map(drop)
            }),
        ),
    ]);
}

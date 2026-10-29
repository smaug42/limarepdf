//! Undo history by what each edit changed (batch 43, PF-42): every kind of edit put back byte
//! for byte by its `Restore`, and done again by the reverse, on made-up documents.

use limarepdf_core::{
    AnnotKind, BookmarkPlace, Cipher, Encryption, FieldValue, FontChoice, HeaderFooter, Info,
    LabelRange, LabelStyle, ObjectKind, Pdf, Restore, Restrictions, TextEdit, Watermark,
};
use lopdf::{dictionary, Dictionary, Object, ObjectId, Stream, StringFormat};
use std::panic::{catch_unwind, AssertUnwindSafe};

const W: f32 = 300.0;
const H: f32 = 200.0;

/// The document as written out, from a copy, so the document itself is untouched.
fn bytes(p: &Pdf) -> Vec<u8> {
    p.clone().snapshot().unwrap()
}

/// One page with a red rectangle and "Hello" in Helvetica, as `tests/edit.rs` makes them.
fn text_page() -> Pdf {
    let mut p = Pdf::new();
    p.insert_blank(0, W, H).unwrap();
    let page = p.page_ids()[0];
    let font = p.doc.add_object(dictionary! {
        "Type" => "Font", "Subtype" => "Type1", "BaseFont" => "Helvetica",
        "Encoding" => "WinAnsiEncoding",
    });
    let content =
        b"q 1 0 0 rg 50 50 100 40 re f Q\nBT /F1 24 Tf 0 0 1 rg 60 150 Td (Hello) Tj ET\n";
    let s = p
        .doc
        .add_object(Stream::new(Dictionary::new(), content.to_vec()));
    let d = p.doc.get_dictionary_mut(page).unwrap();
    d.set("Contents", Object::Reference(s));
    d.set(
        "Resources",
        dictionary! { "Font" => dictionary! { "F1" => font } },
    );
    p
}

/// `n` pages, each saying which it is.
fn pages(n: usize) -> Pdf {
    let mut p = Pdf::new();
    for i in 0..n {
        p.insert_blank(i, W, H).unwrap();
    }
    let font = FontChoice::default();
    for i in 0..n {
        let line = format!("Page {} of the walrus survey", i + 1);
        p.add_text(i, (20.0, 40.0), &line, font, 14.0, [0.0; 3])
            .unwrap();
    }
    p
}

/// `n` pages in a tree of `Pages` nodes ten pages each under the root, as other apps
/// write long documents: the first page change flattens it.
fn nested(n: usize) -> Pdf {
    let mut p = Pdf::new();
    let root = p
        .doc
        .catalog()
        .unwrap()
        .get(b"Pages")
        .unwrap()
        .as_reference()
        .unwrap();
    let mut groups = Vec::new();
    for g in 0..n.div_ceil(10) {
        let node = p.doc.new_object_id();
        let mut kids = Vec::new();
        for i in g * 10..((g + 1) * 10).min(n) {
            let text = format!("BT /F1 12 Tf 20 150 Td (Page {}) Tj ET", i + 1);
            let c = p
                .doc
                .add_object(Stream::new(Dictionary::new(), text.into_bytes()));
            let page = p.doc.add_object(dictionary! {
                "Type" => "Page", "Parent" => node,
                "MediaBox" => vec![0.into(), 0.into(), W.into(), H.into()],
                "Contents" => c,
            });
            kids.push(Object::Reference(page));
        }
        let count = kids.len() as i64;
        p.doc.objects.insert(
            node,
            Object::Dictionary(dictionary! {
                "Type" => "Pages", "Parent" => root, "Kids" => kids, "Count" => count,
            }),
        );
        groups.push(Object::Reference(node));
    }
    let font = p.doc.add_object(dictionary! {
        "Type" => "Font", "Subtype" => "Type1", "BaseFont" => "Helvetica",
        "Encoding" => "WinAnsiEncoding",
    });
    let d = p.doc.get_dictionary_mut(root).unwrap();
    d.set("Kids", groups);
    d.set("Count", n as i64);
    d.set(
        "Resources",
        dictionary! { "Font" => dictionary! { "F1" => font } },
    );
    assert_eq!(p.page_count(), n);
    p
}

fn text(s: &str) -> Object {
    Object::String(s.as_bytes().to_vec(), StringFormat::Literal)
}

/// A page with one text field, `name`, that is its own widget, and Helvetica in the form's
/// resources, as `tests/form_fill.rs` builds them.
fn form_doc() -> Pdf {
    let mut p = Pdf::new();
    p.insert_blank(0, W, H).unwrap();
    let field = p.doc.add_object(dictionary! {
        "Type" => "Annot", "Subtype" => "Widget", "FT" => "Tx", "T" => text("name"),
        "Rect" => vec![10.into(), 150.into(), 160.into(), 170.into()], "F" => 4,
    });
    let page = p.page_ids()[0];
    p.doc
        .get_dictionary_mut(page)
        .unwrap()
        .set("Annots", vec![Object::Reference(field)]);
    let helv = p.doc.add_object(dictionary! {
        "Type" => "Font", "Subtype" => "Type1", "BaseFont" => "Helvetica",
        "Encoding" => "WinAnsiEncoding",
    });
    let form = p.doc.add_object(dictionary! {
        "Fields" => vec![Object::Reference(field)],
        "DA" => text("/Helv 0 Tf 0 g"),
        "DR" => dictionary! { "Font" => dictionary! { "Helv" => helv } },
    });
    p.doc.catalog_mut().unwrap().set("AcroForm", form);
    p
}

/// Three pages with a bookmark to the second and a link on the first going to it.
fn linked_pages() -> Pdf {
    let mut p = pages(3);
    p.add_bookmark("The second page", 1, BookmarkPlace::End)
        .unwrap();
    let ids = p.page_ids();
    let link = p.doc.add_object(dictionary! {
        "Type" => "Annot", "Subtype" => "Link",
        "Rect" => vec![10.into(), 20.into(), 110.into(), 50.into()],
        "Dest" => vec![Object::Reference(ids[1]), "Fit".into()],
    });
    p.doc
        .get_dictionary_mut(ids[0])
        .unwrap()
        .set("Annots", vec![Object::Reference(link)]);
    p
}

/// A PNG of noise, `w` by `h`, which compresses poorly: a large picture.
fn noise_png(w: u32, h: u32) -> Vec<u8> {
    let mut seed = 0x2545_f491_u32;
    let img = image::RgbImage::from_fn(w, h, |_, _| {
        seed ^= seed << 13;
        seed ^= seed >> 17;
        seed ^= seed << 5;
        let [r, g, b, _] = seed.to_le_bytes();
        image::Rgb([r, g, b])
    });
    let mut out = std::io::Cursor::new(Vec::new());
    img.write_to(&mut out, image::ImageFormat::Png).unwrap();
    out.into_inner()
}

fn jpeg() -> Vec<u8> {
    let at = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/data/images/cmyk-cyan-red.jpg"
    );
    std::fs::read(at).unwrap()
}

/// The largest stream's id and length: a picture added to a page of text.
fn largest_stream(p: &Pdf) -> (ObjectId, usize) {
    p.doc
        .objects
        .iter()
        .filter_map(|(&id, o)| o.as_stream().ok().map(|s| (id, s.content.len())))
        .max_by_key(|&(_, n)| n)
        .unwrap()
}

/// The first text object on page 1 and its box.
fn first_text(p: &Pdf) -> (usize, [f32; 4]) {
    let objs = p.page_objects(0).unwrap();
    let o = objs.iter().find(|o| o.kind == ObjectKind::Text).unwrap();
    (o.id, o.bbox)
}

/// The fonts LimarePDF made in the document (those with its `/LimarePDF` record).
fn own_fonts(p: &Pdf) -> Vec<ObjectId> {
    p.doc
        .objects
        .iter()
        .filter(|(_, o)| o.as_dict().is_ok_and(|d| d.has(b"LimarePDF")))
        .map(|(&id, _)| id)
        .collect()
}

/// Runs `edit` on `p` and checks, byte for byte, that its Restore puts the document back
/// as it was, that the reverse does the edit again, and the same once more; `p` is left
/// edited. Returns how many objects the Restore put back.
fn round_trip(p: &mut Pdf, what: &str, edit: impl FnOnce(&mut Pdf)) -> usize {
    let was = bytes(p);
    let before = p.before();
    edit(p);
    let now = bytes(p);
    assert!(was != now, "{what}: the edit changed nothing");
    let undo = before.undo(p);
    assert!(!undo.is_empty(), "{what}: an empty Restore");
    let n = undo.objects();
    assert!(n > 0, "{what}: no objects to put back");
    let redo = p.restore(undo);
    assert!(bytes(p) == was, "{what}: Undo didn't put the document back");
    let undo = p.restore(redo);
    assert!(bytes(p) == now, "{what}: Redo didn't do the edit again");
    let redo = p.restore(undo);
    assert!(bytes(p) == was, "{what}: the second Undo differs");
    p.restore(redo);
    assert!(bytes(p) == now, "{what}: the second Redo differs");
    n
}

type Edit = Box<dyn Fn(&mut Pdf)>;

/// An edit, named, and the document it is made on.
type Case = (&'static str, fn() -> Pdf, Edit);

/// Every kind of edit the app makes is put back byte for byte, and done again the same.
#[test]
fn each_kind_of_edit_is_undone_and_redone_byte_for_byte() {
    let jpg = jpeg();
    let three = || pages(3);
    let cases: Vec<Case> = vec![
        (
            "the words of a line",
            text_page,
            Box::new(|p| {
                let (id, _) = first_text(p);
                let edit = TextEdit {
                    text: "Hello there".into(),
                    font: None,
                    size: None,
                    color: None,
                    parts: vec![],
                };
                p.edit_texts(0, &[(id, edit)]).unwrap();
            }),
        ),
        (
            "a sticky note",
            text_page,
            Box::new(|p| {
                p.add_note(0, (100.0, 100.0), [1.0, 1.0, 0.0], "Check this", "Sam Lee")
                    .unwrap();
            }),
        ),
        (
            "a highlight",
            text_page,
            Box::new(|p| {
                let (_, b) = first_text(p);
                p.add_markup(0, &AnnotKind::Highlight, &[b], [1.0, 1.0, 0.0], "Sam Lee")
                    .unwrap();
            }),
        ),
        (
            "an object moved",
            text_page,
            Box::new(|p| {
                let o = p.page_objects(0).unwrap()[0].clone();
                let b = o.bbox;
                let to = [b[0] + 10.0, b[1] + 10.0, b[2] + 10.0, b[3] + 10.0];
                p.set_object_boxes(0, &[(o.id, to)]).unwrap();
            }),
        ),
        (
            "a page turned",
            three,
            Box::new(|p| p.rotate_pages(&[1], 90).unwrap()),
        ),
        (
            "a page deleted, with a bookmark and a link to it",
            linked_pages,
            Box::new(|p| p.delete_pages(&[1]).unwrap()),
        ),
        (
            "pages reordered",
            three,
            Box::new(|p| p.reorder(&[2, 0, 1]).unwrap()),
        ),
        (
            "a watermark on every page",
            three,
            Box::new(|p| p.add_watermark(&[0, 1, 2], &Watermark::default()).unwrap()),
        ),
        (
            "headers and footers",
            three,
            Box::new(|p| {
                p.add_header_footer(&[0, 1, 2], &HeaderFooter::default())
                    .unwrap()
            }),
        ),
        (
            "a redaction",
            text_page,
            Box::new(|p| {
                let (_, b) = first_text(p);
                p.redact(&[(0, b)]).unwrap();
            }),
        ),
        (
            "the document's properties",
            three,
            Box::new(|p| {
                let info = Info {
                    title: Some("The walrus survey".into()),
                    author: Some("Pat Example".into()),
                    ..Default::default()
                };
                p.set_info(&info).unwrap()
            }),
        ),
        (
            "a picture",
            text_page,
            Box::new(move |p| p.add_image(0, [20.0, 20.0, 120.0, 90.0], &jpg).unwrap()),
        ),
        (
            "a form field filled in",
            form_doc,
            Box::new(|p| {
                p.set_field("name", FieldValue::Text("Pat Example".into()))
                    .unwrap()
            }),
        ),
        (
            "another document's pages inserted",
            three,
            Box::new(|p| p.insert_pages(1, vec![pages(2)]).unwrap()),
        ),
        (
            "a page cropped",
            three,
            Box::new(|p| p.set_crop_margins(0, [10.0, 20.0, 10.0, 20.0]).unwrap()),
        ),
        (
            "page labels",
            three,
            Box::new(|p| {
                let range = LabelRange {
                    page: 0,
                    style: Some(LabelStyle::LowerRoman),
                    prefix: String::new(),
                    first: 1,
                };
                p.set_label_range(range).unwrap()
            }),
        ),
        (
            "a bookmark",
            three,
            Box::new(|p| {
                p.add_bookmark("Chapter 2", 1, BookmarkPlace::End).unwrap();
            }),
        ),
        (
            "Greek text, in a font LimarePDF makes",
            three,
            Box::new(|p| {
                let font = FontChoice::default();
                p.add_text(0, (20.0, 80.0), "Ελληνικά", font, 14.0, [0.0; 3])
                    .unwrap()
            }),
        ),
    ];
    let mut failed = Vec::new();
    for (what, make, edit) in &cases {
        let mut p = make();
        let done = catch_unwind(AssertUnwindSafe(|| round_trip(&mut p, what, edit)));
        if let Err(e) = done {
            let why = (e.downcast_ref::<String>().cloned())
                .or_else(|| e.downcast_ref::<&str>().map(|s| s.to_string()))
                .unwrap_or_default();
            failed.push(format!("{what}: {why}"));
        }
    }
    assert!(failed.is_empty(), "{failed:#?}");
}

/// The first page change in a document whose pages are in a tree of nodes flattens the
/// tree, changing every page: all of it is put back.
#[test]
fn the_first_page_change_in_a_nested_page_tree_is_put_back() {
    let mut p = nested(45);
    let n = round_trip(&mut p, "a page turned", |p| {
        p.rotate_pages(&[0], 90).unwrap()
    });
    assert!(n >= 45, "only {n} objects put back");
    // Later changes to the flat tree are small.
    let n = round_trip(&mut p, "a page turned again", |p| {
        p.rotate_pages(&[3], 90).unwrap()
    });
    assert!(n <= 3, "{n} objects put back");
    let mut p = nested(45);
    round_trip(&mut p, "a page deleted", |p| p.delete_pages(&[12]).unwrap());
    assert_eq!(p.page_count(), 44);
}

/// Several edits undone one by one, newest first, each going back to the state as first
/// recorded, then redone the same way; a new edit after Undo leaves the history's other
/// branch behind.
#[test]
fn several_edits_go_back_and_forth_through_each_state() {
    let mut p = pages(4);
    let mut states = vec![bytes(&p)];
    let mut undo: Vec<Restore> = Vec::new();
    let edits: Vec<Edit> = vec![
        Box::new(|p| {
            p.add_note(0, (50.0, 50.0), [1.0, 1.0, 0.0], "First", "Alex Kim")
                .unwrap();
        }),
        Box::new(|p| p.rotate_pages(&[2], 180).unwrap()),
        Box::new(|p| {
            p.add_watermark(&[0, 1, 2, 3], &Watermark::default())
                .unwrap()
        }),
        Box::new(|p| p.delete_pages(&[1]).unwrap()),
        Box::new(|p| p.reorder(&[2, 1, 0]).unwrap()),
    ];
    for e in &edits {
        let before = p.before();
        e(&mut p);
        undo.push(before.undo(&p));
        states.push(bytes(&p));
    }
    let mut redo = Vec::new();
    for (at, state) in states[..edits.len()].iter().enumerate().rev() {
        redo.push(p.restore(undo.pop().unwrap()));
        assert!(bytes(&p) == *state, "Undo to state {at}");
    }
    for (at, state) in states.iter().enumerate().skip(1) {
        undo.push(p.restore(redo.pop().unwrap()));
        assert!(bytes(&p) == *state, "Redo to state {at}");
    }
    // Back two, then a different change: the two undone are gone, the rest still undo.
    for _ in 0..2 {
        p.restore(undo.pop().unwrap());
    }
    assert!(bytes(&p) == states[3]);
    let before = p.before();
    p.set_crop_margins(0, [5.0, 5.0, 5.0, 5.0]).unwrap();
    let branch = bytes(&p);
    assert!(branch != states[3]);
    let back = p.restore(before.undo(&p));
    assert!(bytes(&p) == states[3]);
    let again = p.restore(back);
    assert!(bytes(&p) == branch, "Redo of the new change");
    p.restore(again);
    for (at, state) in states[..3].iter().enumerate().rev() {
        p.restore(undo.pop().unwrap());
        assert!(bytes(&p) == *state, "Undo after the branch to {at}");
    }
    assert!(undo.is_empty());
}

/// A document nothing was done to gives an empty Restore, which changes nothing.
#[test]
fn nothing_done_gives_an_empty_restore() {
    let mut p = text_page();
    let was = bytes(&p);
    let before = p.before();
    let undo = before.undo(&p);
    assert!(undo.is_empty());
    assert_eq!(undo.objects(), 0);
    let back = p.restore(undo);
    assert!(back.is_empty());
    assert!(bytes(&p) == was);
}

/// An edit that fails part way, or panics, is put back as it was by going back to the
/// state before it.
#[test]
fn a_failed_edit_goes_back_as_it_was() {
    let mut p = text_page();
    let was = bytes(&p);
    let before = p.before();
    let failed: limarepdf_core::Result<()> = (|| {
        p.add_note(0, (100.0, 100.0), [1.0, 1.0, 0.0], "Half", "Sam Lee")?;
        // Not a multiple of 90: refused, after the note went in.
        p.rotate_pages(&[0], 45)
    })();
    assert!(failed.is_err());
    assert!(bytes(&p) != was);
    p.go_back(before);
    assert!(bytes(&p) == was, "a failed edit wasn't put back");

    let before = p.before();
    let panicked = catch_unwind(AssertUnwindSafe(|| {
        p.add_note(0, (100.0, 100.0), [1.0, 1.0, 0.0], "Half", "Sam Lee")
            .unwrap();
        panic!("an edit stopped half way");
    }));
    assert!(panicked.is_err());
    assert!(bytes(&p) != was);
    p.go_back(before);
    assert!(bytes(&p) == was, "a panicked edit wasn't put back");
}

/// A dictionary whose entries only changed order is a change (it is written differently),
/// and is put back in its order.
#[test]
fn keys_put_in_another_order_are_put_back() {
    let mut p = pages(1);
    let page = p.page_ids()[0];
    let was = bytes(&p);
    let before = p.before();
    let d = p.doc.get_dictionary_mut(page).unwrap();
    let entries: Vec<(Vec<u8>, Object)> = d.iter().map(|(k, v)| (k.clone(), v.clone())).collect();
    assert!(entries.len() > 2);
    let mut turned = Dictionary::new();
    for (k, v) in entries.into_iter().rev() {
        turned.set(k, v);
    }
    *d = turned;
    assert!(bytes(&p) != was, "the order shows in the file");
    let undo = before.undo(&p);
    assert_eq!(undo.objects(), 1);
    p.restore(undo);
    assert!(bytes(&p) == was);
}

/// A number changed from 0 to -0 is a change, compared by its bits, and put back.
#[test]
fn a_zero_turned_negative_is_put_back() {
    let mut p = pages(1);
    let page = p.page_ids()[0];
    let set = |p: &mut Pdf, v: f32| {
        p.doc
            .get_dictionary_mut(page)
            .unwrap()
            .set("UserUnit", Object::Real(v));
    };
    set(&mut p, 0.0);
    let before = p.before();
    set(&mut p, -0.0);
    let undo = before.undo(&p);
    assert!(!undo.is_empty());
    assert_eq!(undo.objects(), 1);
    p.restore(undo);
    let page = p.doc.get_dictionary(page).unwrap();
    match page.get(b"UserUnit").unwrap() {
        Object::Real(v) => assert_eq!(v.to_bits(), 0.0f32.to_bits()),
        o => panic!("{o:?}"),
    }
}

/// Two edits joined into one step (typing into a line again soon) go back together to
/// before the first, and forward together to after the second.
#[test]
fn two_edits_merged_undo_together() {
    let mut p = text_page();
    let was = bytes(&p);
    let before = p.before();
    p.add_note(0, (100.0, 100.0), [1.0, 1.0, 0.0], "One", "Sam Lee")
        .unwrap();
    let mut first = before.undo(&p);
    let before = p.before();
    p.rotate_pages(&[0], 90).unwrap();
    p.add_note(0, (150.0, 60.0), [0.0, 1.0, 1.0], "Two", "Sam Lee")
        .unwrap();
    let after = bytes(&p);
    first.merge(before.undo(&p));
    let redo = p.restore(first);
    assert!(bytes(&p) == was, "Undo didn't go back to before the first");
    let undo = p.restore(redo);
    assert!(bytes(&p) == after, "Redo didn't go to after the second");
    p.restore(undo);
    assert!(bytes(&p) == was);
}

/// A merge keeps the older value of an object both edits changed: the page's, changed by
/// each, goes back to before the first.
#[test]
fn a_merge_keeps_the_older_value() {
    let mut p = pages(2);
    let was = bytes(&p);
    let before = p.before();
    p.rotate_pages(&[0], 90).unwrap();
    let mut first = before.undo(&p);
    let n = first.objects();
    let before = p.before();
    p.rotate_pages(&[0], 90).unwrap();
    let second = before.undo(&p);
    first.merge(second);
    assert_eq!(first.objects(), n, "the same page counted twice");
    p.restore(first);
    assert!(bytes(&p) == was);
}

/// A password-protected document opened with its password stays known as protected through
/// an edit's Undo and Redo: reading the document's copy again (`reopen`), as Undo did
/// before, cleared both.
#[test]
fn a_protected_document_stays_protected_through_undo_and_redo() {
    let mut p = pages(2);
    p.set_encryption(Some(Encryption {
        user_password: "user".into(),
        owner_password: "owner".into(),
        cipher: Cipher::Aes256,
        restrictions: Restrictions::default(),
    }));
    let locked = p.to_bytes().unwrap();
    let mut q = Pdf::from_bytes(&locked, Some("user")).unwrap();
    assert!(q.encrypted() && q.password_needed());
    let mut reread = q.clone();
    reread.reopen(&q.clone().snapshot().unwrap()).unwrap();
    assert!(!reread.encrypted() && !reread.password_needed());
    let before = q.before();
    q.add_note(0, (50.0, 50.0), [1.0, 1.0, 0.0], "Private", "Pat Example")
        .unwrap();
    let undo = before.undo(&q);
    let redo = q.restore(undo);
    assert!(q.encrypted() && q.password_needed(), "after Undo");
    q.restore(redo);
    assert!(q.encrypted() && q.password_needed(), "after Redo");
    // A failed edit put back keeps them too.
    let before = q.before();
    q.rotate_pages(&[0], 90).unwrap();
    q.go_back(before);
    assert!(q.encrypted() && q.password_needed(), "after going back");
}

/// The font LimarePDF made for Greek text goes on growing after an Undo and Redo of text
/// added in it, rather than a second one starting.
#[test]
fn a_made_font_keeps_growing_after_undo_and_redo() {
    let mut p = pages(1);
    let font = FontChoice::default();
    p.add_text(0, (20.0, 80.0), "Ελληνικά", font, 14.0, [0.0; 3])
        .unwrap();
    let made = own_fonts(&p);
    assert_eq!(made.len(), 1);
    let before = p.before();
    p.add_text(0, (20.0, 110.0), "λόγος", font, 14.0, [0.0; 3])
        .unwrap();
    let undo = before.undo(&p);
    let redo = p.restore(undo);
    assert_eq!(own_fonts(&p), made, "after Undo");
    p.restore(redo);
    assert_eq!(own_fonts(&p), made, "after Redo");
    p.add_text(0, (20.0, 140.0), "Ωω ψ", font, 14.0, [0.0; 3])
        .unwrap();
    assert_eq!(own_fonts(&p), made, "a second font was started");
    assert!(p.page_text(0).contains("Ωω"));
}

/// A note added beside a large picture holds a little, not the picture.
#[test]
fn a_small_edit_beside_a_large_picture_holds_little() {
    let mut p = text_page();
    p.add_image(0, [20.0, 20.0, 280.0, 180.0], &noise_png(600, 400))
        .unwrap();
    let (_, picture) = largest_stream(&p);
    assert!(picture > 500_000, "the picture is only {picture} bytes");
    let before = p.before();
    p.add_note(0, (100.0, 100.0), [1.0, 1.0, 0.0], "Small", "Sam Lee")
        .unwrap();
    let undo = before.undo(&p);
    assert!(
        undo.size() < 64 * 1024 && undo.size() < picture / 10,
        "{} bytes held beside a {picture}-byte picture",
        undo.size()
    );
}

/// Undoing a picture keeps it for Redo: what Redo holds counts at least the picture.
#[test]
fn a_picture_undone_counts_its_bytes() {
    let mut p = text_page();
    let before = p.before();
    p.add_image(0, [20.0, 20.0, 280.0, 180.0], &noise_png(600, 400))
        .unwrap();
    let (_, picture) = largest_stream(&p);
    let undo = before.undo(&p);
    assert!(undo.size() < picture, "Undo holds {} bytes", undo.size());
    let redo = p.restore(undo);
    assert!(
        redo.size() >= picture,
        "Redo holds {} bytes, the picture {picture}",
        redo.size()
    );
}

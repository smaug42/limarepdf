//! Reading bookmarks from made-up outlines, including the broken shapes real files have.

use lopdf::{dictionary, Object, ObjectId, StringFormat};
use mpe_core::{OutlineItem, Pdf};

fn four_pages() -> Pdf {
    let mut p = Pdf::new();
    for i in 0..4 {
        p.insert_blank(i, 300.0, 200.0).unwrap();
    }
    p
}

fn text(s: &str) -> Object {
    Object::String(s.as_bytes().to_vec(), StringFormat::Literal)
}

fn item(title: &str, depth: usize, page: Option<usize>, open: bool, kids: bool) -> OutlineItem {
    OutlineItem {
        title: title.into(),
        depth,
        page,
        open,
        has_children: kids,
    }
}

/// Sets the catalog's outline root, with `first` as its first item.
fn set_outline(p: &mut Pdf, first: ObjectId) {
    let root = dictionary! { "Type" => "Outlines", "First" => first };
    let root = p.doc.add_object(root);
    p.doc.catalog_mut().unwrap().set("Outlines", root);
}

#[test]
fn no_outline_gives_no_bookmarks() {
    assert!(four_pages().outline().is_empty());
}

#[test]
fn reads_nested_bookmarks_and_every_kind_of_destination() {
    let mut p = four_pages();
    let pg = p.page_ids();

    // Named destinations: "sec11" in a name tree with kids, "old" in the PDF 1.1 Dests.
    let leaf = p.doc.add_object(dictionary! {
        "Limits" => vec![text("a"), text("z")],
        "Names" => vec![text("intro"), vec![pg[0].into(), "Fit".into()].into(),
                        text("sec11"), dictionary! { "D" => vec![pg[1].into(), "Fit".into()] }.into()],
    });
    let other = p.doc.add_object(dictionary! {
        "Limits" => vec![text("zz"), text("zzz")],
        "Names" => vec![text("zz"), vec![pg[3].into(), "Fit".into()].into()],
    });
    let tree = dictionary! { "Kids" => vec![other.into(), leaf.into()] };
    let tree = p.doc.add_object(tree);
    let old = dictionary! { "old" => vec![pg[2].into(), "Fit".into()] };
    let old = p.doc.add_object(old);
    {
        let cat = p.doc.catalog_mut().unwrap();
        cat.set("Names", dictionary! { "Dests" => tree });
        cat.set("Dests", old);
    }

    let ids: Vec<ObjectId> = (0..6).map(|_| p.doc.new_object_id()).collect();
    let [ch1, s11, s12, ch2, web, broken] = ids[..] else {
        unreachable!()
    };
    let title = "Chapitre 2\r\n été".encode_utf16();
    let bytes = [0xFE, 0xFF].into_iter();
    let utf16 = bytes.chain(title.flat_map(u16::to_be_bytes)).collect();
    let utf16 = Object::String(utf16, StringFormat::Hexadecimal);
    let items = [
        dictionary! {
            "Title" => text("Chapter 1"), "Dest" => vec![pg[0].into(), "Fit".into()],
            "First" => s11, "Count" => 2, "Next" => ch2,
        },
        dictionary! {
            "Title" => text("Section 1.1"),
            "A" => dictionary! { "S" => "GoTo", "D" => text("sec11") }, "Next" => s12,
        },
        dictionary! { "Title" => text("Section 1.2"), "Dest" => "old" },
        dictionary! {
            "Title" => utf16,
            "Dest" => vec![pg[3].into(), "XYZ".into(), Object::Null, Object::Null, Object::Null],
            "Next" => web,
        },
        dictionary! {
            "Title" => text("Website"),
            "A" => dictionary! { "S" => "URI", "URI" => text("https://example.com") },
            "Next" => broken,
        },
        // Points at a page that isn't in the document, and loops back to the start.
        dictionary! {
            "Title" => text("Broken"), "Dest" => vec![old.into(), "Fit".into()], "Next" => ch1,
        },
    ];
    for (id, d) in ids.into_iter().zip(items) {
        p.doc.objects.insert(id, d.into());
    }
    set_outline(&mut p, ch1);

    assert_eq!(
        p.outline(),
        vec![
            item("Chapter 1", 0, Some(0), true, true),
            item("Section 1.1", 1, Some(1), false, false),
            item("Section 1.2", 1, Some(2), false, false),
            item("Chapitre 2 été", 0, Some(3), false, false),
            item("Website", 0, None, false, false),
            item("Broken", 0, None, false, false),
        ]
    );
}

#[test]
fn bookmarks_follow_pages_that_move() {
    let mut p = four_pages();
    let pg = p.page_ids();
    let b = p.doc.add_object(dictionary! {
        "Title" => text("Last page"), "Dest" => vec![pg[3].into(), "Fit".into()],
    });
    set_outline(&mut p, b);
    p.reorder(&[3, 0, 1, 2]).unwrap();
    assert_eq!(p.outline()[0].page, Some(0));
    p.delete_pages(&[0]).unwrap();
    assert_eq!(p.outline()[0].page, None);
}

#[test]
fn splits_at_bookmarks() {
    let mut p = four_pages();
    assert!(p.split_at_bookmarks(1).is_err());
    let pg = p.page_ids();
    let ids: Vec<ObjectId> = (0..3).map(|_| p.doc.new_object_id()).collect();
    let items = [
        dictionary! {
            "Title" => text("Part one"), "Dest" => vec![pg[1].into(), "Fit".into()],
            "First" => ids[1], "Count" => 1, "Next" => ids[2],
        },
        dictionary! { "Title" => text("Inside"), "Dest" => vec![pg[2].into(), "Fit".into()] },
        dictionary! { "Title" => text("Part two"), "Dest" => vec![pg[3].into(), "Fit".into()] },
    ];
    for (id, d) in ids.iter().zip(items) {
        p.doc.objects.insert(*id, d.into());
    }
    set_outline(&mut p, ids[0]);

    let summary = |level| -> Vec<(Option<String>, usize)> {
        let parts = p.split_at_bookmarks(level).unwrap();
        parts
            .into_iter()
            .map(|(t, d)| (t, d.page_count()))
            .collect()
    };
    let s = |t: &str| Some(t.to_string());
    assert_eq!(
        summary(1),
        vec![(None, 1), (s("Part one"), 2), (s("Part two"), 1)]
    );
    assert_eq!(
        summary(2),
        vec![
            (None, 1),
            (s("Part one"), 1),
            (s("Inside"), 1),
            (s("Part two"), 1)
        ]
    );
}

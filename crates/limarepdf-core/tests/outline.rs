//! Reading bookmarks from made-up outlines, including the broken shapes real files have.

use limarepdf_core::{bookmark_moves, bookmark_step, BookmarkPlace, BookmarkStep, OutlineItem};
use limarepdf_core::{Pdf, SplitBy};
use lopdf::{dictionary, Dictionary, Object, ObjectId, StringFormat};
use std::collections::HashMap;

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

/// Bookmarks titled by `names`, each going to the named destination of the same name.
fn named_bookmarks(p: &mut Pdf, names: &[&str]) {
    let ids: Vec<ObjectId> = names.iter().map(|_| p.doc.new_object_id()).collect();
    for (i, name) in names.iter().enumerate() {
        let mut d = dictionary! { "Title" => text(name), "Dest" => text(name) };
        if let Some(next) = ids.get(i + 1) {
            d.set("Next", *next);
        }
        p.doc.objects.insert(ids[i], d.into());
    }
    set_outline(p, ids[0]);
}

/// A name tree whose nodes list the same kid twice is looked through once per node: at the
/// depth limit, a name it doesn't have is soon known not to be there (it once took about
/// 2^32 visits), and one in its deepest leaf is found (audit R05).
#[test]
fn a_name_tree_of_shared_kids_is_read_once_per_node() {
    let mut p = four_pages();
    let pg = p.page_ids();
    let mut node = p.doc.add_object(dictionary! {
        "Names" => vec![text("there"), vec![pg[2].into(), "Fit".into()].into()],
    });
    for _ in 0..32 {
        let kids = vec![Object::Reference(node), Object::Reference(node)];
        node = p.doc.add_object(dictionary! { "Kids" => kids });
    }
    p.doc
        .catalog_mut()
        .unwrap()
        .set("Names", dictionary! { "Dests" => node });
    named_bookmarks(&mut p, &["nowhere", "there"]);
    assert_eq!(
        p.outline(),
        vec![
            item("nowhere", 0, None, false, false),
            item("there", 0, Some(2), false, false),
        ]
    );
}

/// A name tree too large to search gives up, and its names are destinations that can't be
/// followed, rather than read on and on; a name met before the lookup gives up is found.
#[test]
fn a_name_tree_too_large_to_search_gives_up() {
    let mut p = four_pages();
    let pg = p.page_ids();
    let leaf = |name: &str, page| -> Object {
        dictionary! { "Names" => vec![text(name), vec![Object::Reference(page), "Fit".into()].into()] }
            .into()
    };
    let mut kids = vec![leaf("first", pg[1])];
    kids.extend((0..100_000).map(|_| Object::Dictionary(dictionary! {})));
    kids.push(leaf("last", pg[3]));
    let tree = p.doc.add_object(dictionary! { "Kids" => kids });
    p.doc
        .catalog_mut()
        .unwrap()
        .set("Names", dictionary! { "Dests" => tree });
    named_bookmarks(&mut p, &["first", "last"]);
    assert_eq!(
        p.outline(),
        vec![
            item("first", 0, Some(1), false, false),
            item("last", 0, None, false, false),
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
    assert!(p.split_parts(SplitBy::Bookmarks(1)).is_err());
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
        let parts = p.split_parts(SplitBy::Bookmarks(level)).unwrap();
        parts
            .into_iter()
            .map(|(t, pages)| (t, pages.len()))
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
    let files = p
        .split_files(SplitBy::Bookmarks(1), |n, t| {
            limarepdf_core::part_file_name("book", n, t).into()
        })
        .unwrap();
    assert_eq!(
        files,
        [
            "book-001.pdf",
            "book-002 Part one.pdf",
            "book-003 Part two.pdf"
        ]
        .map(std::path::PathBuf::from)
    );
}

/// The outline's titles with their depths, and each `Count` from the root down.
fn shape(p: &Pdf) -> Vec<(String, usize)> {
    p.outline()
        .into_iter()
        .map(|i| (i.title, i.depth))
        .collect()
}

fn counts(p: &Pdf) -> Vec<Option<i64>> {
    let mut out = Vec::new();
    let root = p.doc.catalog().unwrap().get(b"Outlines").unwrap();
    let root = root.as_reference().unwrap();
    fn walk(p: &Pdf, id: ObjectId, out: &mut Vec<Option<i64>>) {
        let d = p.doc.get_dictionary(id).unwrap();
        out.push(d.get(b"Count").ok().and_then(|o| o.as_i64().ok()));
        let mut kid = d.get(b"First").ok().and_then(|o| o.as_reference().ok());
        while let Some(k) = kid {
            walk(p, k, out);
            kid = p
                .doc
                .get_dictionary(k)
                .unwrap()
                .get(b"Next")
                .ok()
                .and_then(|o| o.as_reference().ok());
        }
    }
    walk(p, root, &mut out);
    out
}

#[test]
fn adds_renames_and_removes_bookmarks() {
    let mut p = four_pages();
    assert_eq!(p.add_bookmark("One", 0, BookmarkPlace::End).unwrap(), 0);
    assert_eq!(p.add_bookmark("Three", 2, BookmarkPlace::End).unwrap(), 1);
    assert_eq!(
        p.add_bookmark("Two", 1, BookmarkPlace::After(0)).unwrap(),
        1
    );
    assert_eq!(
        p.add_bookmark("One A", 0, BookmarkPlace::Under(0)).unwrap(),
        1
    );
    assert_eq!(
        p.add_bookmark("One B", 3, BookmarkPlace::Under(0)).unwrap(),
        2
    );
    // After a bookmark with children goes after the children too.
    assert_eq!(
        p.add_bookmark("One and a half", 1, BookmarkPlace::After(0))
            .unwrap(),
        3
    );
    assert_eq!(
        p.add_bookmark("Ünïcode", 3, BookmarkPlace::Under(5))
            .unwrap(),
        6
    );
    let s = |t: &str, d| (t.to_string(), d);
    assert_eq!(
        shape(&p),
        vec![
            s("One", 0),
            s("One A", 1),
            s("One B", 1),
            s("One and a half", 0),
            s("Two", 0),
            s("Three", 0),
            s("Ünïcode", 1),
        ]
    );
    let pages: Vec<_> = p.outline().iter().map(|i| i.page).collect();
    assert_eq!(pages, [0, 0, 3, 1, 1, 2, 3].map(Some));
    // Adding under a bookmark opens it; the root counts everything shown.
    assert!(p.outline()[0].open && p.outline()[5].open);
    assert_eq!(
        counts(&p),
        [Some(7), Some(2), None, None, None, None, Some(1), None]
    );

    p.rename_bookmark(3, "Interlude").unwrap();
    assert_eq!(p.outline()[3].title, "Interlude");

    // Removing takes the children with it, and the file still reads after a save.
    p.remove_bookmark(0).unwrap();
    p.remove_bookmark(3).unwrap();
    let mut p = Pdf::from_bytes(&p.to_bytes().unwrap(), None).unwrap();
    assert_eq!(
        shape(&p),
        vec![s("Interlude", 0), s("Two", 0), s("Three", 0)]
    );
    assert_eq!(counts(&p), [Some(3), None, None, None]);
    assert!(p.rename_bookmark(3, "x").is_err());
    assert!(p.add_bookmark("x", 4, BookmarkPlace::End).is_err());

    // Removing the last one drops the outline.
    for _ in 0..3 {
        p.remove_bookmark(0).unwrap();
    }
    assert!(p.outline().is_empty());
    assert!(p.doc.catalog().unwrap().get(b"Outlines").is_err());
}

#[test]
fn closed_bookmarks_keep_negative_counts() {
    let mut p = four_pages();
    p.add_bookmark("Parent", 0, BookmarkPlace::End).unwrap();
    p.add_bookmark("Child", 1, BookmarkPlace::Under(0)).unwrap();
    // Close the parent, as a file might have it, then add after it.
    let root = p.doc.catalog().unwrap().get(b"Outlines").unwrap();
    let root = root.as_reference().unwrap();
    let first = p.doc.get_dictionary(root).unwrap().get(b"First").unwrap();
    let first = first.as_reference().unwrap();
    p.doc.get_dictionary_mut(first).unwrap().set("Count", -1);
    p.add_bookmark("Next", 2, BookmarkPlace::After(0)).unwrap();
    assert!(!p.outline()[0].open);
    assert_eq!(counts(&p), [Some(2), Some(-1), None, None]);
}

/// A, with A1 and A2 inside; B; C, with C1 inside: positions 0 to 5.
fn book() -> Pdf {
    let mut p = four_pages();
    use BookmarkPlace::{End, Under};
    for (title, page, place) in [
        ("A", 0, End),
        ("A1", 1, Under(0)),
        ("A2", 2, Under(0)),
        ("B", 1, End),
        ("C", 2, End),
        ("C1", 3, Under(4)),
    ] {
        p.add_bookmark(title, page, place).unwrap();
    }
    p
}

/// The outline on one line: each title after a dot for each level it is in.
fn tree(p: &Pdf) -> String {
    let items = p.outline();
    let shown = items
        .iter()
        .map(|i| format!("{}{}", ".".repeat(i.depth), i.title));
    shown.collect::<Vec<_>>().join(" ")
}

/// Each bookmark's page, by its title.
fn pages_by_title(p: &Pdf) -> HashMap<String, Option<usize>> {
    p.outline().into_iter().map(|i| (i.title, i.page)).collect()
}

fn link(d: &Dictionary, key: &[u8]) -> Option<ObjectId> {
    d.get(key).ok().and_then(|o| o.as_reference().ok())
}

/// Every list in the outline is linked both ways: each child names its parent, the one
/// before and after it, and its parent its first and last.
fn links_agree(p: &Pdf) {
    let root = p.doc.catalog().unwrap().get(b"Outlines").unwrap();
    let root = root.as_reference().unwrap();
    let mut parents = vec![root];
    while let Some(parent) = parents.pop() {
        let d = p.doc.get_dictionary(parent).unwrap();
        let mut kids = Vec::new();
        let mut at = link(d, b"First");
        while let Some(k) = at {
            assert!(kids.len() < 1000, "a loop under {parent:?}");
            kids.push(k);
            at = link(p.doc.get_dictionary(k).unwrap(), b"Next");
        }
        assert_eq!(link(d, b"Last"), kids.last().copied(), "{parent:?}'s Last");
        for (n, &k) in kids.iter().enumerate() {
            let kd = p.doc.get_dictionary(k).unwrap();
            assert_eq!(link(kd, b"Parent"), Some(parent), "{k:?}'s Parent");
            let before = n.checked_sub(1).map(|b| kids[b]);
            assert_eq!(link(kd, b"Prev"), before, "{k:?}'s Prev");
            parents.push(k);
        }
    }
}

/// The outline's dictionaries without the keys that place them, by object.
fn contents(p: &Pdf) -> HashMap<ObjectId, Dictionary> {
    let ids = p.outline_nodes().into_iter().map(|(id, _)| id);
    ids.map(|id| {
        let mut d = p.doc.get_dictionary(id).unwrap().clone();
        for key in ["Parent", "Prev", "Next", "First", "Last", "Count"] {
            d.remove(key.as_bytes());
        }
        (id, d)
    })
    .collect()
}

#[test]
fn moves_bookmarks_to_every_place() {
    use BookmarkPlace::{After, Before, End, Under};
    let mut p = book();
    let pages = pages_by_title(&p);
    assert_eq!(tree(&p), "A .A1 .A2 B C .C1");
    // A bookmark moves with the bookmarks inside it.
    assert_eq!(p.move_bookmark(4, Before(0)).unwrap(), 0);
    assert_eq!(tree(&p), "C .C1 A .A1 .A2 B");
    // After a bookmark with children goes after the children.
    assert_eq!(p.move_bookmark(5, After(0)).unwrap(), 2);
    assert_eq!(tree(&p), "C .C1 B A .A1 .A2");
    assert_eq!(p.move_bookmark(5, Under(2)).unwrap(), 3);
    assert_eq!(tree(&p), "C .C1 B .A2 A .A1");
    assert_eq!(p.move_bookmark(1, End).unwrap(), 5);
    assert_eq!(tree(&p), "C B .A2 A .A1 C1");
    // From one bookmark's children to another's, before the first.
    assert_eq!(p.move_bookmark(4, Before(2)).unwrap(), 2);
    assert_eq!(tree(&p), "C B .A1 .A2 A C1");
    links_agree(&p);
    // B opened when A2 went in; C and A no longer hold any.
    assert!(p.outline()[1].open);
    assert_eq!(counts(&p), [Some(6), None, Some(2), None, None, None, None]);
    // Each still goes where it went, and the file reads the same after a save.
    assert_eq!(pages_by_title(&p), pages);
    let q = Pdf::from_bytes(&p.to_bytes().unwrap(), None).unwrap();
    assert_eq!(tree(&q), "C B .A1 .A2 A C1");
    links_agree(&q);
}

/// Moving changes only where a bookmark is: its title, destination, action and anything
/// else it holds stay as they were, whatever kind of destination it has.
#[test]
fn moving_keeps_what_each_bookmark_holds() {
    let mut p = book();
    let ids: Vec<ObjectId> = p.outline_nodes().into_iter().map(|(id, _)| id).collect();
    let pg = p.page_ids();
    let named = dictionary! { "fourth" => vec![pg[3].into(), "Fit".into()] };
    let named = p.doc.add_object(named);
    p.doc.catalog_mut().unwrap().set("Dests", named);
    let set = |p: &mut Pdf, i: usize, key: &str, value: Object| {
        let d = p.doc.get_dictionary_mut(ids[i]).unwrap();
        d.remove(b"Dest");
        d.set(key, value);
    };
    set(&mut p, 1, "Dest", text("fourth"));
    let go = dictionary! { "S" => "GoTo", "D" => vec![pg[2].into(), "Fit".into()] };
    set(&mut p, 2, "A", go.into());
    let web = dictionary! { "S" => "URI", "URI" => text("https://example.com") };
    set(&mut p, 3, "A", web.into());
    // A colour and bold, as other apps write them.
    let d = p.doc.get_dictionary_mut(ids[5]).unwrap();
    d.set("C", vec![1.into(), 0.into(), 0.into()]);
    d.set("F", 2);
    let was = contents(&p);
    let pages = pages_by_title(&p);
    use BookmarkPlace::{After, Before, End, Under};
    for (from, to) in [(1, End), (5, Before(0)), (3, Under(1)), (0, After(2))] {
        p.move_bookmark(from, to).unwrap();
    }
    assert_eq!(contents(&p), was);
    assert_eq!(pages_by_title(&p), pages);
    assert_eq!(pages["A1"], Some(3));
    assert_eq!(pages["B"], None);
}

#[test]
fn a_bookmark_cant_move_into_itself_or_too_deep() {
    use BookmarkPlace::{After, Before, Under};
    let mut p = book();
    let was = p.doc.objects.clone();
    for (from, to) in [
        (0, Under(0)),
        (0, Under(1)),
        (0, After(2)),
        (0, Before(1)),
        (4, Under(5)),
        (6, After(0)),
        (0, After(6)),
        (0, Under(6)),
    ] {
        let items = p.outline();
        let what = format!("{from} to {to:?}");
        assert!(bookmark_moves(&items, from, to).is_err(), "{what}");
        assert!(p.move_bookmark(from, to).is_err(), "{what}");
    }
    assert!(p.doc.objects == was, "a refused move changed the document");

    // Nested 64 levels deep (0 to 63), the most there can be: a bookmark with one inside
    // it fits in only where the one inside lands at most at the bottom level; on its own,
    // a level lower.
    let mut p = four_pages();
    p.add_bookmark("0", 0, BookmarkPlace::End).unwrap();
    for i in 1..64 {
        p.add_bookmark(&i.to_string(), 0, Under(i - 1)).unwrap();
    }
    assert!(p.add_bookmark("too deep", 0, Under(63)).is_err());
    p.add_bookmark("X", 1, BookmarkPlace::End).unwrap();
    p.add_bookmark("X1", 1, Under(64)).unwrap();
    for to in [63, 62] {
        let e = p.move_bookmark(64, Under(to)).unwrap_err();
        assert!(e.to_string().contains("nested deeper"), "{e}");
    }
    assert_eq!(p.move_bookmark(64, Under(61)).unwrap(), 64);
    assert_eq!(p.outline()[65].depth, 63);
    assert!(p.move_bookmark(65, Under(63)).is_err());
    assert_eq!(p.move_bookmark(65, Under(62)).unwrap(), 64);
    assert_eq!(p.outline()[64].depth, 63);
}

/// A move to where a bookmark already is changes nothing, and says so beforehand.
#[test]
fn a_move_to_where_it_is_changes_nothing() {
    use BookmarkPlace::{After, Before, End, Under};
    let mut p = book();
    let was = p.doc.objects.clone();
    for (from, to) in [
        (0, Before(3)),
        (0, Before(0)),
        (0, After(0)),
        (1, Before(2)),
        (2, After(1)),
        (2, Under(0)),
        (4, End),
        (3, After(0)),
    ] {
        let what = format!("{from} to {to:?}");
        assert!(!bookmark_moves(&p.outline(), from, to).unwrap(), "{what}");
        assert_eq!(p.move_bookmark(from, to).unwrap(), from, "{what}");
    }
    assert!(p.doc.objects == was, "the document changed");
    assert!(bookmark_moves(&p.outline(), 3, Before(0)).unwrap());
}

/// The keyboard's steps: up and down among the bookmarks at the same level, out to just
/// after the one it is in, and into the one above as its last.
#[test]
fn keyboard_steps_stay_at_their_level_until_promoted_or_demoted() {
    use BookmarkPlace::{After, Before, Under};
    use BookmarkStep::{Demote, Down, Promote, Up};
    let p = book();
    let items = p.outline();
    let step = |i, s| bookmark_step(&items, i, s);
    assert_eq!(step(0, Up), None);
    // The first inside A leaves it only when promoted.
    assert_eq!(step(1, Up), None);
    assert_eq!(step(3, Up), Some(Before(0)));
    assert_eq!(step(3, Down), Some(After(4)));
    assert_eq!(step(4, Down), None);
    assert_eq!(step(2, Down), None);
    assert_eq!(step(1, Down), Some(After(2)));
    assert_eq!(step(1, Promote), Some(After(0)));
    assert_eq!(step(0, Promote), None);
    assert_eq!(step(0, Demote), None);
    assert_eq!(step(3, Demote), Some(Under(0)));
    assert_eq!(step(2, Demote), Some(Under(1)));
    assert_eq!(step(6, Up), None);

    let mut p = book();
    p.move_bookmark(3, Under(0)).unwrap();
    assert_eq!(tree(&p), "A .A1 .A2 .B C .C1");
    let promote = bookmark_step(&p.outline(), 1, Promote).unwrap();
    p.move_bookmark(1, promote).unwrap();
    assert_eq!(tree(&p), "A .A2 .B A1 C .C1");
    let down = bookmark_step(&p.outline(), 3, Down).unwrap();
    p.move_bookmark(3, down).unwrap();
    assert_eq!(tree(&p), "A .A2 .B C .C1 A1");
}

/// A bookmark moved into a closed one opens it, so it can be seen where it went.
#[test]
fn moving_into_a_closed_bookmark_opens_it() {
    let mut p = book();
    let c = p.outline_nodes()[4].0;
    p.doc.get_dictionary_mut(c).unwrap().set("Count", -1);
    assert!(!p.outline()[4].open);
    p.move_bookmark(3, BookmarkPlace::Under(4)).unwrap();
    assert_eq!(tree(&p), "A .A1 .A2 C .C1 .B");
    assert!(p.outline()[3].open);
    let shown = [Some(6), Some(2), None, None, Some(2), None, None];
    assert_eq!(counts(&p), shown);
}

/// A file's outline with wrong links (each Prev naming itself, no Parent, the last Next
/// looping back to the first) is put right in the lists a move changes.
#[test]
fn moving_in_a_broken_outline_puts_its_links_right() {
    let mut p = four_pages();
    let pg = p.page_ids();
    let ids: Vec<ObjectId> = (0..3).map(|_| p.doc.new_object_id()).collect();
    for (n, title) in ["X", "Y", "Z"].into_iter().enumerate() {
        let next = ids[(n + 1) % 3];
        let d = dictionary! {
            "Title" => text(title), "Dest" => vec![pg[n].into(), "Fit".into()],
            "Prev" => ids[n], "Next" => next,
        };
        p.doc.objects.insert(ids[n], d.into());
    }
    set_outline(&mut p, ids[0]);
    assert_eq!(tree(&p), "X Y Z");
    assert_eq!(p.move_bookmark(2, BookmarkPlace::Before(0)).unwrap(), 0);
    assert_eq!(tree(&p), "Z X Y");
    links_agree(&p);
    assert_eq!(p.move_bookmark(0, BookmarkPlace::Under(2)).unwrap(), 2);
    assert_eq!(tree(&p), "X Y .Z");
    links_agree(&p);
}

#[test]
fn adds_a_bookmark_before_another() {
    let mut p = book();
    assert_eq!(
        p.add_bookmark("First", 0, BookmarkPlace::Before(0))
            .unwrap(),
        0
    );
    assert_eq!(
        p.add_bookmark("Between", 0, BookmarkPlace::Before(3))
            .unwrap(),
        3
    );
    assert_eq!(tree(&p), "First A .A1 .Between .A2 B C .C1");
    links_agree(&p);
}

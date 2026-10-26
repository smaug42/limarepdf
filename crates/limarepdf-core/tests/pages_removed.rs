//! Extracting, deleting and splitting pages leave nothing of the pages that went in the saved
//! file (build plan E4): no references to them from bookmarks, named destinations, links,
//! the opening action, form fields or the structure tree, which would keep their content.

mod common;

use common::saved::{tools, Saved};
use limarepdf_core::render::{self, Renderer};
use limarepdf_core::{part_file_name, FieldValue, LinkTarget, Pdf, SplitBy};
use lopdf::{dictionary, Dictionary, Object, ObjectId, Stream};

fn s(t: &str) -> Object {
    Object::string_literal(t)
}

fn rect(r: [i64; 4]) -> Object {
    r.into_iter()
        .map(Object::Integer)
        .collect::<Vec<_>>()
        .into()
}

fn fit(page: ObjectId) -> Object {
    vec![Object::Reference(page), "Fit".into()].into()
}

fn push_annot(p: &mut Pdf, page: ObjectId, id: ObjectId) {
    let d = p.doc.get_dictionary_mut(page).unwrap();
    match d.get_mut(b"Annots") {
        Ok(Object::Array(a)) => a.push(id.into()),
        _ => d.set("Annots", vec![Object::Reference(id)]),
    }
}

/// Everything on page `n` (1-based) that must go with it.
fn own(n: usize) -> Vec<String> {
    ["PAGE", "FORM", "NOTE", "NOTE-AP", "VALUE"]
        .iter()
        .map(|k| format!("WALRUS-{k}-{n}"))
        .collect()
}

/// Four pages, each with its own text, form, note (with its own drawing), field and bookmark,
/// and everything that can point across pages: named destinations (both kinds), links, the
/// opening action, a reply to a note on another page, the structure tree, a field with
/// widgets on pages 1 and 3, a form pages 1 and 3 share, and a chapter bookmark for page 3
/// whose child goes to page 2.
fn four_pages() -> Pdf {
    let mut p = Pdf::new();
    for i in 0..4 {
        p.insert_blank(i, 300.0, 200.0).unwrap();
    }
    let pages = p.page_ids();
    let font = p.doc.add_object(dictionary! {
        "Type" => "Font", "Subtype" => "Type1", "BaseFont" => "Helvetica", "Encoding" => "WinAnsiEncoding",
    });
    let form = |p: &mut Pdf, text: &str| {
        p.doc.add_object(Stream::new(
            dictionary! {
                "Type" => "XObject", "Subtype" => "Form", "BBox" => rect([0, 0, 300, 200]),
                "Resources" => dictionary! { "Font" => dictionary! { "F1" => font } },
            },
            format!("BT /F1 10 Tf 10 60 Td ({text}) Tj ET").into_bytes(),
        ))
    };
    let shared = form(&mut p, "WALRUS-SHARED-FORM");
    let mut notes = Vec::new();
    let mut elems = Vec::new();
    let mut fields = Vec::new();
    let mut nums = Vec::new();
    let tree = p.doc.new_object_id();
    for (i, &page) in pages.iter().enumerate() {
        let n = i + 1;
        let own_form = form(&mut p, &format!("WALRUS-FORM-{n}"));
        let mut xobjects = dictionary! { "Own" => own_form };
        let mut content = format!(
            "/Figure <</MCID 0>> BDC BT /F1 14 Tf 10 150 Td (WALRUS-PAGE-{n}) Tj ET EMC q /Own Do Q"
        );
        if n == 1 || n == 3 {
            xobjects.set("Shared", shared);
            content.push_str(" q 1 0 0 1 0 -40 cm /Shared Do Q");
        }
        let c = p
            .doc
            .add_object(Stream::new(Dictionary::new(), content.into_bytes()));
        let d = p.doc.get_dictionary_mut(page).unwrap();
        d.set("Contents", c);
        d.set(
            "Resources",
            dictionary! { "Font" => dictionary! { "F1" => font }, "XObject" => xobjects },
        );
        d.set("StructParents", i as i64);
        let ap = p.doc.add_object(Stream::new(
            dictionary! {
                "Type" => "XObject", "Subtype" => "Form", "BBox" => rect([0, 0, 20, 20]),
                "Resources" => dictionary! { "Font" => dictionary! { "F1" => font } },
            },
            format!("BT /F1 4 Tf 1 8 Td (WALRUS-NOTE-AP-{n}) Tj ET").into_bytes(),
        ));
        let note = p.doc.add_object(dictionary! {
            "Type" => "Annot", "Subtype" => "Text", "Rect" => rect([250, 150, 270, 170]),
            "Contents" => s(&format!("WALRUS-NOTE-{n}")), "P" => page,
            "AP" => dictionary! { "N" => ap },
        });
        push_annot(&mut p, page, note);
        notes.push(note);
        let widget = p.doc.add_object(dictionary! {
            "Type" => "Annot", "Subtype" => "Widget", "FT" => "Tx", "T" => s(&format!("field-{n}")),
            "Rect" => rect([10, 10, 150, 30]), "P" => page, "F" => 4,
        });
        push_annot(&mut p, page, widget);
        fields.push(Object::Reference(widget));
        let alt = if n == 3 {
            "WALRUS-ALT-3".to_string()
        } else {
            format!("Kept alt {n}")
        };
        let elem = p.doc.add_object(dictionary! {
            "Type" => "StructElem", "S" => "Figure", "P" => tree, "Pg" => page, "K" => 0,
            "Alt" => s(&alt),
        });
        elems.push(Object::Reference(elem));
        nums.push(Object::Integer(i as i64));
        nums.push(vec![Object::Reference(elem)].into());
    }
    // A field with a widget on page 1 and one on page 3.
    let kids: Vec<ObjectId> = [0, 2]
        .into_iter()
        .map(|i| {
            p.doc.add_object(dictionary! {
                "Type" => "Annot", "Subtype" => "Widget", "Rect" => rect([160, 10, 290, 30]),
                "P" => pages[i], "F" => 4,
            })
        })
        .collect();
    let both = p.doc.add_object(dictionary! {
        "FT" => "Tx", "T" => s("shared"),
        "Kids" => kids.iter().map(|&k| Object::Reference(k)).collect::<Vec<_>>(),
    });
    for (&k, &i) in kids.iter().zip(&[0, 2]) {
        p.doc.get_dictionary_mut(k).unwrap().set("Parent", both);
        push_annot(&mut p, pages[i], k);
    }
    fields.push(Object::Reference(both));
    p.doc.objects.insert(
        tree,
        dictionary! {
            "Type" => "StructTreeRoot", "K" => elems,
            "ParentTree" => dictionary! { "Nums" => nums },
        }
        .into(),
    );
    let helv = font;
    let catalog = p.doc.catalog_mut().unwrap();
    catalog.set("StructTreeRoot", tree);
    catalog.set("MarkInfo", dictionary! { "Marked" => true });
    catalog.set(
        "AcroForm",
        dictionary! {
            "Fields" => fields, "DA" => s("/Helv 0 Tf 0 g"),
            "DR" => dictionary! { "Font" => dictionary! { "Helv" => helv } },
        },
    );
    catalog.set("OpenAction", fit(pages[2]));
    catalog.set("Dests", dictionary! { "dest-old-four" => fit(pages[3]) });
    // Named destinations in two leaves; the second's pairs are an object of their own.
    let pairs = p.doc.add_object(vec![
        s("dest-two"),
        fit(pages[1]),
        s("dest-four-apart"),
        fit(pages[3]),
    ]);
    let leaves = [
        p.doc.add_object(dictionary! {
            "Names" => vec![s("dest-one"), fit(pages[0]), s("dest-three"), fit(pages[2])],
        }),
        p.doc.add_object(dictionary! { "Names" => pairs }),
    ];
    p.doc.catalog_mut().unwrap().set(
        "Names",
        dictionary! { "Dests" => dictionary! {
            "Kids" => leaves.iter().map(|&l| Object::Reference(l)).collect::<Vec<_>>(),
        } },
    );
    for n in 1..=4 {
        p.set_field(
            &format!("field-{n}"),
            FieldValue::Text(format!("WALRUS-VALUE-{n}")),
        )
        .unwrap();
    }
    p.set_field("shared", FieldValue::Text("WALRUS-SHARED".into()))
        .unwrap();

    // Links on page 1: to page 3, to page 3 by name, to page 2, and to the web.
    let links = [
        dictionary! { "Dest" => fit(pages[2]) },
        dictionary! { "Dest" => s("dest-three") },
        dictionary! { "A" => dictionary! { "S" => "GoTo", "D" => fit(pages[1]) } },
        dictionary! { "A" => dictionary! { "S" => "URI", "URI" => s("https://example.org/") } },
    ];
    for (k, extra) in links.into_iter().enumerate() {
        let x = 10 + 40 * k as i64;
        let mut d = dictionary! {
            "Type" => "Annot", "Subtype" => "Link", "Rect" => rect([x, 100, x + 30, 120]),
        };
        for (key, v) in extra.iter() {
            d.set(key.clone(), v.clone());
        }
        let id = p.doc.add_object(d);
        push_annot(&mut p, pages[0], id);
    }
    // On page 2, a reply to page 3's note.
    let reply = p.doc.add_object(dictionary! {
        "Type" => "Annot", "Subtype" => "Text", "Rect" => rect([250, 120, 270, 140]),
        "Contents" => s("A reply"), "IRT" => notes[2], "P" => pages[1],
    });
    push_annot(&mut p, pages[1], reply);

    // Bookmarks: one per page; page 3's has a child going to page 2.
    let ids: Vec<ObjectId> = (0..5).map(|_| p.doc.new_object_id()).collect();
    let root = p.doc.new_object_id();
    let top = [ids[0], ids[1], ids[2], ids[4]];
    let titles = ["Bookmark 1", "Bookmark 2", "Chapter 3", "Bookmark 4"];
    for (k, (&id, title)) in top.iter().zip(titles).enumerate() {
        let mut d = dictionary! {
            "Title" => s(title), "Parent" => root, "Dest" => fit(pages[[0, 1, 2, 3][k]]),
        };
        if k > 0 {
            d.set("Prev", top[k - 1]);
        }
        if k < 3 {
            d.set("Next", top[k + 1]);
        }
        if k == 2 {
            d.set("First", ids[3]);
            d.set("Last", ids[3]);
            d.set("Count", 1);
        }
        p.doc.objects.insert(id, d.into());
    }
    p.doc.objects.insert(
        ids[3],
        dictionary! { "Title" => s("Back to 2"), "Parent" => ids[2], "Dest" => fit(pages[1]) }
            .into(),
    );
    p.doc.objects.insert(
        root,
        dictionary! { "Type" => "Outlines", "First" => ids[0], "Last" => ids[4], "Count" => 5 }
            .into(),
    );
    p.doc.catalog_mut().unwrap().set("Outlines", root);
    p
}

/// Pages `kept` (1-based) of `p`'s original, as `out` saved them: nothing of the other pages in
/// the file, everything of these, and no references to pages that aren't there.
fn only_pages(out: &mut Pdf, kept: &[usize]) -> Saved {
    only_pages_in(Saved::of(out), kept)
}

/// [`only_pages`] for a file already written.
fn only_pages_in(saved: Saved, kept: &[usize]) -> Saved {
    for n in 1..=4 {
        let marks = own(n);
        let marks: Vec<&str> = marks.iter().map(String::as_str).collect();
        if kept.contains(&n) {
            saved.assert_kept(&marks);
        } else {
            saved.assert_gone(&marks);
        }
    }
    if kept.contains(&1) || kept.contains(&3) {
        saved.assert_kept(&["WALRUS-SHARED-FORM", "WALRUS-SHARED"]);
    } else {
        saved.assert_gone(&["WALRUS-SHARED"]);
    }
    if kept.contains(&3) {
        saved.assert_kept(&["WALRUS-ALT-3"]);
    } else {
        saved.assert_gone(&["WALRUS-ALT-3"]);
    }
    let stray = saved.stray_pages();
    assert!(stray.is_empty(), "{stray:?}");
    let unreachable = saved.unreachable();
    assert!(unreachable.is_empty(), "{unreachable:?}");
    assert_eq!(saved.doc.get_pages().len(), kept.len());
    saved
}

/// The pages of `out` draw exactly as pages `kept` (1-based) of `original`.
fn draws_the_same(original: &mut Pdf, out: &mut Pdf, kept: &[usize]) {
    let a = render::load(original.to_bytes().unwrap(), None).unwrap();
    let b = render::load(out.to_bytes().unwrap(), None).unwrap();
    let (ra, rb) = (Renderer::new(&a), Renderer::new(&b));
    for (i, &n) in kept.iter().enumerate() {
        let x = ra.render(n - 1, 1.0).unwrap();
        let y = rb.render(i, 1.0).unwrap();
        assert!(x.rgba == y.rgba, "page {n} draws differently");
    }
}

fn field_names(saved: &Saved) -> Vec<String> {
    saved.fields().into_iter().map(|f| f.name).collect()
}

fn targets(p: &Pdf, page: usize) -> Vec<LinkTarget> {
    p.page_links(page)
        .unwrap()
        .into_iter()
        .map(|l| l.target)
        .collect()
}

#[test]
fn extracted_pages_keep_nothing_of_the_others() {
    let mut p = four_pages();
    let mut out = p.extract(&[0, 1]).unwrap();
    let saved = only_pages(&mut out, &[1, 2]);
    draws_the_same(&mut p, &mut out, &[1, 2]);
    assert_eq!(
        saved.bookmarks(),
        [
            ("Bookmark 1".to_string(), true),
            ("Bookmark 2".to_string(), true),
            ("Chapter 3".to_string(), false),
            ("Back to 2".to_string(), true),
        ]
    );
    assert_eq!(field_names(&saved), ["field-1", "field-2", "shared"]);
    let q = Pdf::from_bytes(&saved.bytes, None).unwrap();
    assert_eq!(
        targets(&q, 0),
        [
            LinkTarget::Page(1),
            LinkTarget::Uri("https://example.org/".into())
        ]
    );
    let catalog = saved.doc.catalog().unwrap();
    assert!(catalog.get(b"OpenAction").is_err());
    // The named destinations to pages 1 and 2 stay; those to pages 3 and 4 go, and so does
    // the link's use of one.
    saved.assert_kept(&["dest-one", "dest-two"]);
    saved.assert_gone(&["dest-three", "dest-old-four", "dest-four-apart"]);
}

#[test]
fn deleted_pages_leave_nothing_behind() {
    let mut p = four_pages();
    let mut out = p.clone();
    out.delete_pages(&[2, 3]).unwrap();
    let saved = only_pages(&mut out, &[1, 2]);
    draws_the_same(&mut p, &mut out, &[1, 2]);
    // Deleting keeps the bookmarks, without a destination where their page went.
    assert_eq!(
        saved.bookmarks(),
        [
            ("Bookmark 1".to_string(), true),
            ("Bookmark 2".to_string(), true),
            ("Chapter 3".to_string(), false),
            ("Back to 2".to_string(), true),
            ("Bookmark 4".to_string(), false),
        ]
    );
    assert_eq!(field_names(&saved), ["field-1", "field-2", "shared"]);
}

#[test]
fn split_parts_keep_nothing_of_the_other_parts() {
    let mut p = four_pages();
    let dir = std::env::temp_dir().join(format!("limarepdf-split-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let file = |n: usize, title: Option<&str>| dir.join(part_file_name("part", n, title));
    let read = |n: usize, title: Option<&str>| Saved::new(std::fs::read(file(n, title)).unwrap());

    assert_eq!(p.split(SplitBy::Every(2), file).unwrap(), 2);
    let first = only_pages_in(read(1, None), &[1, 2]);
    let second = only_pages_in(read(2, None), &[3, 4]);
    for (saved, kept) in [(&first, [1, 2]), (&second, [3, 4])] {
        let mut part = Pdf::from_bytes(&saved.bytes, None).unwrap();
        draws_the_same(&mut p, &mut part, &kept);
    }
    assert_eq!(first.bookmarks().len(), 4);
    assert_eq!(
        second.bookmarks(),
        [
            ("Chapter 3".to_string(), true),
            ("Bookmark 4".to_string(), true)
        ]
    );
    assert_eq!(field_names(&second), ["field-3", "field-4", "shared"]);
    let q = Pdf::from_bytes(&second.bytes, None).unwrap();
    assert_eq!(q.outline()[0].page, Some(0));

    let parts = p.split_parts(SplitBy::Bookmarks(1)).unwrap();
    assert_eq!(p.split(SplitBy::Bookmarks(1), file).unwrap(), 4);
    for (i, (title, pages)) in parts.into_iter().enumerate() {
        assert_eq!(pages, [i]);
        let saved = only_pages_in(read(i + 1, title.as_deref()), &[i + 1]);
        assert_eq!(saved.bookmarks()[0].0, title.unwrap());
    }
    let _ = std::fs::remove_dir_all(&dir);
}

/// A part holds only what its pages use, before it is saved: splitting a large file into
/// many parts made a whole copy of it per part (build plan E14).
#[test]
fn a_part_holds_only_what_its_pages_use() {
    let p = four_pages();
    let part = p.extract(&[0]).unwrap();
    let unused = part.clone().doc.prune_objects();
    assert!(unused.is_empty(), "objects nothing uses: {unused:?}");
    let holding = |mark: &str| {
        part.doc.objects.values().any(|o| {
            o.as_stream()
                .is_ok_and(|s| s.content.windows(mark.len()).any(|w| w == mark.as_bytes()))
        })
    };
    assert!(holding("WALRUS-PAGE-1") && holding("WALRUS-FORM-1"));
    for n in 2..=4 {
        assert!(!holding(&format!("WALRUS-PAGE-{n}")), "page {n}'s content");
        assert!(!holding(&format!("WALRUS-FORM-{n}")), "page {n}'s form");
    }
}

/// Two pages drawing `marked` content items each (page 1's MCIDs from 0, page 2's likewise),
/// with Helvetica as `/F1`.
fn two_marked_pages(marked: [usize; 2]) -> Pdf {
    let mut p = Pdf::new();
    for i in 0..2 {
        p.insert_blank(i, 300.0, 200.0).unwrap();
    }
    let font = p.doc.add_object(dictionary! {
        "Type" => "Font", "Subtype" => "Type1", "BaseFont" => "Helvetica", "Encoding" => "WinAnsiEncoding",
    });
    for (i, &page) in p.page_ids().iter().enumerate() {
        let content: String = (0..marked[i])
            .map(|m| {
                let y = 180 - 20 * m;
                format!(
                    "/P <</MCID {m}>> BDC BT /F1 12 Tf 10 {y} Td (Page {} item {m}) Tj ET EMC ",
                    i + 1
                )
            })
            .collect();
        let c = p
            .doc
            .add_object(Stream::new(Dictionary::new(), content.into_bytes()));
        let d = p.doc.get_dictionary_mut(page).unwrap();
        d.set("Contents", c);
        d.set(
            "Resources",
            dictionary! { "Font" => dictionary! { "F1" => font } },
        );
    }
    p
}

/// A structure element with `entries`, whose parent is `parent`.
fn elem(p: &mut Pdf, s: &str, parent: ObjectId, entries: Dictionary) -> ObjectId {
    let mut d = dictionary! { "Type" => "StructElem", "S" => s, "P" => parent };
    for (k, v) in entries.iter() {
        d.set(k.clone(), v.clone());
    }
    p.doc.add_object(d)
}

/// Two tagged pages whose structure elements reach their content in every way the structure
/// tree allows: a marked-content reference that is an object of its own (R06), one in an array
/// that is an object of its own, an object reference to an annotation, a page given only by
/// the parent element, an element whose content is only in its kid, and an element with
/// content on both pages. Every one of those touching page 2 has alternative text marked
/// WALRUS, which must go with page 2; the elements only on page 1 keep theirs. Two elements
/// listing each other as kid and parent make a loop, which must end.
fn tagged_two_pages() -> Pdf {
    let mut p = two_marked_pages([3, 5]);
    let pages = p.page_ids();
    let root = p.doc.new_object_id();
    let mcr = |p: &mut Pdf, page: usize, mcid: i64| {
        p.doc
            .add_object(dictionary! { "Type" => "MCR", "Pg" => pages[page], "MCID" => mcid })
    };
    let mut tops = Vec::new();
    // Page 1 only, through a marked-content reference of its own.
    let m = mcr(&mut p, 0, 0);
    tops.push(elem(
        &mut p,
        "P",
        root,
        dictionary! { "K" => m, "Alt" => s("Kept MCR alt") },
    ));
    // Page 2, through a marked-content reference of its own.
    let m = mcr(&mut p, 1, 0);
    tops.push(elem(
        &mut p,
        "P",
        root,
        dictionary! { "K" => m, "Alt" => s("WALRUS-MCR-ALT") },
    ));
    // Page 2, through an array of its own holding a marked-content reference.
    let m = mcr(&mut p, 1, 1);
    let list = p.doc.add_object(vec![Object::Reference(m)]);
    tops.push(elem(
        &mut p,
        "P",
        root,
        dictionary! { "K" => list, "ActualText" => s("WALRUS-ARRAY-ACTUAL") },
    ));
    // Page 2, through an object reference to a note there, with no page given.
    let note = p.doc.add_object(dictionary! {
        "Type" => "Annot", "Subtype" => "Text", "Rect" => rect([250, 150, 270, 170]),
        "Contents" => s("A note"), "P" => pages[1],
    });
    push_annot(&mut p, pages[1], note);
    tops.push(elem(
        &mut p,
        "Link",
        root,
        dictionary! {
            "K" => dictionary! { "Type" => "OBJR", "Obj" => note },
            "E" => s("WALRUS-OBJR-E"),
        },
    ));
    // Page 2's marked content 2, the page given only by the parent.
    let sect = elem(&mut p, "Sect", root, dictionary! { "Pg" => pages[1] });
    let span = elem(
        &mut p,
        "Span",
        sect,
        dictionary! { "K" => 2, "Alt" => s("WALRUS-INHERITED-ALT") },
    );
    p.doc
        .get_dictionary_mut(sect)
        .unwrap()
        .set("K", vec![Object::Reference(span)]);
    tops.push(sect);
    // Page 2's marked content 3, held by a kid: the parent's text describes it too.
    let div = elem(
        &mut p,
        "Div",
        root,
        dictionary! { "Alt" => s("WALRUS-PARENT-ALT") },
    );
    let m = mcr(&mut p, 1, 3);
    let kid = elem(&mut p, "P", div, dictionary! { "K" => m });
    p.doc
        .get_dictionary_mut(div)
        .unwrap()
        .set("K", vec![Object::Reference(kid)]);
    tops.push(div);
    // Content on both pages (decision D3): the text may describe page 2's, so it goes.
    let (a, b) = (mcr(&mut p, 0, 1), mcr(&mut p, 1, 4));
    tops.push(elem(
        &mut p,
        "P",
        root,
        dictionary! {
            "K" => vec![Object::Reference(a), Object::Reference(b)],
            "Alt" => s("WALRUS-BOTH-ALT"),
        },
    ));
    // Page 1's marked content 2, the page given by the parent: both keep their text.
    let div = elem(
        &mut p,
        "Div",
        root,
        dictionary! { "Pg" => pages[0], "Alt" => s("Kept parent alt") },
    );
    let span = elem(
        &mut p,
        "Span",
        div,
        dictionary! { "K" => 2, "Alt" => s("Kept inherited alt") },
    );
    p.doc
        .get_dictionary_mut(div)
        .unwrap()
        .set("K", vec![Object::Reference(span)]);
    tops.push(div);
    // A loop: each is the other's kid and parent.
    let (x, y) = (p.doc.new_object_id(), p.doc.new_object_id());
    for (id, other, alt) in [(x, y, "Kept loop alt"), (y, x, "Kept loop alt too")] {
        let d = dictionary! {
            "Type" => "StructElem", "S" => "Div", "P" => other,
            "K" => vec![Object::Reference(other)], "Alt" => s(alt),
        };
        p.doc.objects.insert(id, d.into());
    }
    tops.push(x);
    p.doc.objects.insert(
        root,
        dictionary! {
            "Type" => "StructTreeRoot",
            "K" => tops.into_iter().map(Object::Reference).collect::<Vec<_>>(),
        }
        .into(),
    );
    let catalog = p.doc.catalog_mut().unwrap();
    catalog.set("StructTreeRoot", root);
    catalog.set("MarkInfo", dictionary! { "Marked" => true });
    p
}

/// Removing page 2 takes the alternative text of everything with content there, however the
/// structure tree reaches it (audit R06), and nothing else's.
#[test]
fn alternative_text_goes_with_its_page_however_the_tree_reaches_it() {
    let kept = [
        "Kept MCR alt",
        "Kept parent alt",
        "Kept inherited alt",
        "Kept loop alt",
        "Kept loop alt too",
    ];
    let mut p = tagged_two_pages();
    let before = Saved::of(&mut p.clone());
    before.assert_kept(&kept);
    before.assert_kept(&[
        "WALRUS-MCR-ALT",
        "WALRUS-ARRAY-ACTUAL",
        "WALRUS-OBJR-E",
        "WALRUS-INHERITED-ALT",
        "WALRUS-PARENT-ALT",
        "WALRUS-BOTH-ALT",
    ]);
    let mut part = p.extract(&[0]).unwrap();
    let mut deleted = p.clone();
    deleted.delete_pages(&[1]).unwrap();
    for out in [&mut part, &mut deleted] {
        let saved = Saved::of(out);
        saved.assert_gone(&["WALRUS"]);
        saved.assert_kept(&kept);
        assert_eq!(saved.doc.get_pages().len(), 1);
        draws_the_same(&mut p, out, &[1]);
    }
}

/// Elements with marked content and no page given anywhere (PB-04): the parent tree tells
/// their page by each page's `/StructParents`, and one it doesn't list may be on the page that
/// went, so its text goes too rather than the removal being refused.
#[test]
fn alternative_text_of_content_with_no_page_goes_by_the_parent_tree() {
    let mut p = two_marked_pages([1, 2]);
    let pages = p.page_ids();
    let root = p.doc.new_object_id();
    let on_1 = elem(
        &mut p,
        "P",
        root,
        dictionary! { "K" => 0, "Alt" => s("Kept by the parent tree") },
    );
    let on_2 = elem(
        &mut p,
        "P",
        root,
        dictionary! { "K" => 0, "Alt" => s("WALRUS-PARENT-TREE-ALT") },
    );
    let unsure = elem(
        &mut p,
        "P",
        root,
        dictionary! {
            "K" => dictionary! { "Type" => "MCR", "MCID" => 1 },
            "ActualText" => s("WALRUS-UNSURE-ACTUAL"),
        },
    );
    let list = p.doc.add_object(vec![Object::Reference(on_2)]);
    p.doc.objects.insert(
        root,
        dictionary! {
            "Type" => "StructTreeRoot",
            "K" => vec![on_1.into(), on_2.into(), unsure.into()],
            "ParentTree" => dictionary! {
                "Nums" => vec![0.into(), vec![Object::Reference(on_1)].into(), 1.into(), list.into()],
            },
        }
        .into(),
    );
    for (i, page) in pages.iter().enumerate() {
        let d = p.doc.get_dictionary_mut(*page).unwrap();
        d.set("StructParents", i as i64);
    }
    let catalog = p.doc.catalog_mut().unwrap();
    catalog.set("StructTreeRoot", root);
    catalog.set("MarkInfo", dictionary! { "Marked" => true });
    let before = Saved::of(&mut p.clone());
    before.assert_kept(&[
        "Kept by the parent tree",
        "WALRUS-PARENT-TREE-ALT",
        "WALRUS-UNSURE-ACTUAL",
    ]);
    let mut part = p.extract(&[0]).unwrap();
    let mut deleted = p.clone();
    deleted.delete_pages(&[1]).unwrap();
    for out in [&mut part, &mut deleted] {
        let saved = Saved::of(out);
        saved.assert_gone(&["WALRUS"]);
        saved.assert_kept(&["Kept by the parent tree"]);
    }
}

/// Two pages, with a field whose only widget is on page 2, `levels` fields below the form's
/// top-level field, which holds a value.
fn deep_field(levels: usize) -> Pdf {
    let mut p = two_marked_pages([1, 1]);
    let page = p.page_ids()[1];
    let widget = p.doc.add_object(dictionary! {
        "Type" => "Annot", "Subtype" => "Widget", "Rect" => rect([10, 10, 150, 30]), "P" => page,
        "F" => 4,
    });
    push_annot(&mut p, page, widget);
    let mut kid = widget;
    for d in (0..levels).rev() {
        let node = p.doc.add_object(dictionary! {
            "T" => s(&format!("level-{d}")), "Kids" => vec![Object::Reference(kid)],
        });
        p.doc.get_dictionary_mut(kid).unwrap().set("Parent", node);
        kid = node;
    }
    let top = p.doc.get_dictionary_mut(kid).unwrap();
    top.set("FT", "Tx");
    top.set("V", s("WALRUS-DEEP-VALUE"));
    p.doc.catalog_mut().unwrap().set(
        "AcroForm",
        dictionary! { "Fields" => vec![Object::Reference(kid)] },
    );
    p
}

/// A form nested too deep to tell which fields go with the pages makes removing them refuse,
/// with nothing changed; just inside the limit, the fields go with their pages.
#[test]
fn page_removal_refuses_a_form_nested_too_deep_to_follow() {
    let mut p = deep_field(34);
    // Serialising moves lopdf's object counter on, so copies are compared.
    let before = p.clone().snapshot().unwrap();
    let err = format!("{:#}", p.delete_pages(&[1]).unwrap_err());
    assert!(err.contains("nested more than 32 deep"), "{err}");
    assert_eq!(p.clone().snapshot().unwrap(), before);
    let Err(err) = p.extract(&[0]) else {
        panic!("extracting from a form nested too deep worked");
    };
    let err = format!("{err:#}");
    assert!(err.contains("nested more than 32 deep"), "{err}");

    let mut p = deep_field(33);
    let mut part = p.extract(&[0]).unwrap();
    let mut kept = p.clone();
    p.delete_pages(&[1]).unwrap();
    for out in [&mut part, &mut p] {
        let saved = Saved::of(out);
        saved.assert_gone(&["WALRUS"]);
        assert!(saved.fields().is_empty());
    }
    // The same form stays whole when its page does (PB-05).
    kept.delete_pages(&[0]).unwrap();
    Saved::of(&mut kept).assert_kept(&["WALRUS-DEEP-VALUE"]);
}

/// Two pages, with a named destination to page 2 `levels` kids down the name tree (one of
/// which also lists the tree's top again), and a link on page 1 using its name.
fn deep_named_dest(levels: usize) -> Pdf {
    let mut p = two_marked_pages([1, 1]);
    let pages = p.page_ids();
    let mut node = p.doc.add_object(dictionary! {
        "Names" => vec![s("WALRUS-DEEP-DEST"), fit(pages[1])],
    });
    let top = p.doc.new_object_id();
    for d in 0..levels {
        let mut kids = vec![Object::Reference(node)];
        if d == levels / 2 {
            kids.push(Object::Reference(top));
        }
        node = if d + 1 == levels {
            p.doc
                .objects
                .insert(top, dictionary! { "Kids" => kids }.into());
            top
        } else {
            p.doc.add_object(dictionary! { "Kids" => kids })
        };
    }
    p.doc
        .catalog_mut()
        .unwrap()
        .set("Names", dictionary! { "Dests" => node });
    let link = p.doc.add_object(dictionary! {
        "Type" => "Annot", "Subtype" => "Link", "Rect" => rect([10, 100, 40, 120]),
        "Dest" => s("WALRUS-DEEP-DEST"),
    });
    push_annot(&mut p, pages[0], link);
    p
}

/// A named destination to a page that went is found however deep in the name tree it is,
/// and the link using it loses it.
#[test]
fn named_destinations_deep_in_the_tree_go_with_their_page() {
    for levels in [2, 32, 40] {
        let p = deep_named_dest(levels);
        let mut part = p.extract(&[0]).unwrap();
        let mut deleted = p.clone();
        deleted.delete_pages(&[1]).unwrap();
        for out in [&mut part, &mut deleted] {
            Saved::of(out).assert_gone(&["WALRUS"]);
        }
    }
}

/// Poppler and Ghostscript read only the kept pages' text:
/// `cargo test -p limarepdf-core --test pages_removed -- --ignored cross_check`.
#[test]
#[ignore]
fn cross_check_extracted_pages_with_poppler_and_ghostscript() {
    let p = four_pages();
    let bytes = p.extract(&[0, 1]).unwrap().to_bytes().unwrap();
    for text in [tools::pdftotext(&bytes), tools::gs_text(&bytes)] {
        assert!(
            text.contains("WALRUS-PAGE-1") && text.contains("WALRUS-PAGE-2"),
            "{text}"
        );
        assert!(
            !text.contains("WALRUS-PAGE-3") && !text.contains("WALRUS-PAGE-4"),
            "{text}"
        );
    }
}

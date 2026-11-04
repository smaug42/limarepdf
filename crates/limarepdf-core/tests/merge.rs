//! What adding another file's pages carries over (ENH-18): its bookmarks, put among the
//! document's in page order, and its named destinations, renamed where the name is taken.

use limarepdf_core::{BookmarkPlace, LinkTarget, Pdf, Unfollowed};
use lopdf::{dictionary, Dictionary, Object, ObjectId, StringFormat};

const NOWHERE: LinkTarget = LinkTarget::Unfollowed(Unfollowed::Nowhere);

fn pages(n: usize) -> Pdf {
    let mut p = Pdf::new();
    for i in 0..n {
        p.insert_blank(i, 300.0, 200.0).unwrap();
    }
    p
}

fn text(s: &str) -> Object {
    Object::String(s.as_bytes().to_vec(), StringFormat::Literal)
}

/// `n` pages with a top-level bookmark for each of `marks`: its title and page (from 0).
fn marked(n: usize, marks: &[(&str, usize)]) -> Pdf {
    let mut p = pages(n);
    for &(title, page) in marks {
        p.add_bookmark(title, page, BookmarkPlace::End).unwrap();
    }
    p
}

/// The outline on one line: each title after a dot for each level it is in, then its page
/// from 1 (`-` for none).
fn tree(p: &Pdf) -> String {
    let items = p.outline().into_iter().map(|i| {
        let page = i.page.map_or("-".into(), |n| (n + 1).to_string());
        format!("{}{}:{page}", ".".repeat(i.depth), i.title)
    });
    items.collect::<Vec<_>>().join(" ")
}

fn link(d: &Dictionary, key: &[u8]) -> Option<ObjectId> {
    d.get(key).ok().and_then(|o| o.as_reference().ok())
}

/// Every list in the outline is linked both ways: each child names its parent, the one
/// before it, and its parent its last.
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

fn set_outline(p: &mut Pdf, first: ObjectId) {
    let root = dictionary! { "Type" => "Outlines", "First" => first };
    let root = p.doc.add_object(root);
    p.doc.catalog_mut().unwrap().set("Outlines", root);
}

fn fit(p: &Pdf, page: usize) -> Object {
    vec![p.page_ids()[page].into(), "Fit".into()].into()
}

/// Gives `p` the named destinations `names` (each a name and a page from 0) in a name tree
/// of one node.
fn name_pages(p: &mut Pdf, names: &[(&[u8], usize)]) {
    let mut names = names.to_vec();
    names.sort();
    let pairs = names.iter().flat_map(|&(name, page)| {
        let name = Object::String(name.to_vec(), StringFormat::Literal);
        [name, fit(p, page)]
    });
    let tree = dictionary! { "Names" => pairs.collect::<Vec<_>>() };
    let tree = p.doc.add_object(tree);
    p.doc
        .catalog_mut()
        .unwrap()
        .set("Names", dictionary! { "Dests" => tree });
}

/// Makes `dest` the only link on page `page`.
fn link_to(p: &mut Pdf, page: usize, dest: Object) {
    let rect = [10, 10, 60, 30].map(Object::Integer).to_vec();
    let annot =
        dictionary! { "Type" => "Annot", "Subtype" => "Link", "Rect" => rect, "Dest" => dest };
    let annot = p.doc.add_object(annot);
    let page = p.page_ids()[page];
    let page = p.doc.get_dictionary_mut(page).unwrap();
    page.set("Annots", vec![annot.into()]);
}

fn link_target(p: &Pdf, page: usize) -> LinkTarget {
    p.page_links(page).unwrap()[0].target.clone()
}

/// Where a link to the named destination `name` goes, tried on the last page.
fn goes(p: &mut Pdf, name: &[u8]) -> LinkTarget {
    let last = p.page_count() - 1;
    let name = Object::String(name.to_vec(), StringFormat::Literal);
    link_to(p, last, name);
    link_target(p, last)
}

#[test]
fn appending_keeps_the_added_files_bookmarks() {
    let mut p = marked(2, &[("Base", 0)]);
    let mut added = marked(3, &[("One", 0), ("Two", 2)]);
    added
        .add_bookmark("One a", 1, BookmarkPlace::Under(0))
        .unwrap();
    // One is closed in its file.
    let one = added.outline_nodes()[0].0;
    let one = added.doc.get_dictionary_mut(one).unwrap();
    one.set("Count", -1);
    p.append(vec![added]).unwrap();
    assert_eq!(tree(&p), "Base:1 One:3 .One a:4 Two:5");
    assert!(!p.outline()[1].open);
    links_agree(&p);
    // The root shows the three at the top level, One being closed.
    let root = p.doc.catalog().unwrap().get(b"Outlines").unwrap();
    let root = p.doc.get_dictionary(root.as_reference().unwrap()).unwrap();
    assert_eq!(root.get(b"Count").unwrap().as_i64().unwrap(), 3);
    // Saved and read again, they are still there.
    let again = Pdf::from_bytes(&p.to_bytes().unwrap(), None).unwrap();
    assert_eq!(tree(&again), tree(&p));
}

#[test]
fn inserted_files_bookmarks_go_among_the_documents_in_page_order() {
    let mut p = marked(4, &[("A", 0), ("B", 2)]);
    p.add_bookmark("A1", 1, BookmarkPlace::Under(0)).unwrap();
    let x = marked(1, &[("X", 0)]);
    let y = marked(2, &[("Y", 1)]);
    p.insert_pages(2, vec![x, y]).unwrap();
    assert_eq!(tree(&p), "A:1 .A1:2 X:3 Y:5 B:6");
    links_agree(&p);

    // Before every page, before every bookmark.
    let mut p = marked(2, &[("A", 0)]);
    p.insert_pages(0, vec![marked(1, &[("X", 0)])]).unwrap();
    assert_eq!(tree(&p), "X:1 A:2");

    // Appended, after all of them, a bookmark to no page too.
    let mut p = marked(2, &[("A", 0)]);
    let web = p.doc.add_object(dictionary! {
        "Title" => text("Web"),
        "A" => dictionary! { "S" => "URI", "URI" => text("https://example.com") },
    });
    p.add_bookmark("Last", 1, BookmarkPlace::End).unwrap();
    let last = p.outline_nodes()[1].0;
    p.doc.get_dictionary_mut(last).unwrap().set("Next", web);
    p.append(vec![marked(1, &[("X", 0)])]).unwrap();
    assert_eq!(tree(&p), "A:1 Last:2 Web:- X:3");
    links_agree(&p);
}

#[test]
fn a_document_without_bookmarks_gains_the_added_files() {
    let mut p = pages(1);
    p.append(vec![marked(2, &[("Only", 1)])]).unwrap();
    assert_eq!(tree(&p), "Only:3");
    links_agree(&p);
    // Files without bookmarks add no outline.
    let mut p = pages(1);
    p.append(vec![pages(1)]).unwrap();
    assert!(p.doc.catalog().unwrap().get(b"Outlines").is_err());
}

#[test]
fn the_added_files_named_destinations_come_with_them_renamed_where_taken() {
    let mut p = pages(2);
    name_pages(&mut p, &[(b"intro", 1), (b"intro-2", 0)]);
    link_to(&mut p, 0, text("intro"));
    let mut added = pages(3);
    name_pages(&mut added, &[(b"intro", 2)]);
    // A name of PDF 1.1's Dests dictionary, which a bookmark's action goes to.
    let old = dictionary! { "extra" => fit(&added, 1) };
    added.doc.catalog_mut().unwrap().set("Dests", old);
    link_to(&mut added, 0, text("intro"));
    let mark = added.doc.add_object(dictionary! {
        "Title" => text("Extra"),
        "A" => dictionary! { "S" => "GoTo", "D" => Object::Name(b"extra".to_vec()) },
    });
    set_outline(&mut added, mark);

    p.append(vec![added]).unwrap();
    // Each file's link to "intro" goes to its own page.
    assert_eq!(link_target(&p, 0), LinkTarget::Page(1));
    assert_eq!(link_target(&p, 2), LinkTarget::Page(4));
    assert_eq!(tree(&p), "Extra:4");
    // The names are the document's now: the one taken twice over gets the next number.
    assert_eq!(goes(&mut p, b"intro"), LinkTarget::Page(1));
    assert_eq!(goes(&mut p, b"intro-2"), LinkTarget::Page(0));
    assert_eq!(goes(&mut p, b"intro-3"), LinkTarget::Page(4));
    assert_eq!(goes(&mut p, b"extra"), LinkTarget::Page(3));
    // Saved and read again, they still go there.
    let mut again = Pdf::from_bytes(&p.to_bytes().unwrap(), None).unwrap();
    assert_eq!(goes(&mut again, b"intro-3"), LinkTarget::Page(4));
}

#[test]
fn files_added_together_each_get_a_name_of_their_own() {
    let mut p = pages(1);
    name_pages(&mut p, &[(b"top", 0)]);
    let mut a = pages(1);
    let mut b = pages(2);
    name_pages(&mut a, &[(b"top", 0)]);
    // A name in UTF-16 is numbered in UTF-16.
    let e: &[u8] = &[0xFE, 0xFF, 0x00, 0xE9];
    name_pages(&mut b, &[(b"top", 1), (e, 0)]);
    let mut e2 = e.to_vec();
    e2.extend([0x00, b'-', 0x00, b'2']);
    p.append(vec![a, b]).unwrap();
    assert_eq!(goes(&mut p, b"top"), LinkTarget::Page(0));
    assert_eq!(goes(&mut p, b"top-2"), LinkTarget::Page(1));
    assert_eq!(goes(&mut p, b"top-3"), LinkTarget::Page(3));
    assert_eq!(goes(&mut p, e), LinkTarget::Page(2));
    assert_eq!(goes(&mut p, &e2), NOWHERE);
}

#[test]
fn page_numbers_in_an_added_files_destinations_stay_its_own() {
    let mut p = pages(3);
    let mut added = pages(2);
    // Some makers write a page number from 0 rather than the page.
    link_to(&mut added, 0, vec![Object::Integer(1), "Fit".into()].into());
    // A page the file doesn't have, but the merged document does: still nowhere.
    link_to(&mut added, 1, vec![Object::Integer(4), "Fit".into()].into());
    p.append(vec![added]).unwrap();
    assert_eq!(link_target(&p, 3), LinkTarget::Page(4));
    assert_eq!(link_target(&p, 4), NOWHERE);
}

#[test]
fn a_loop_in_an_added_outline_ends_where_reading_it_does() {
    let mut added = pages(2);
    let (a, b) = (added.doc.new_object_id(), added.doc.new_object_id());
    let first = dictionary! { "Title" => text("A"), "Dest" => fit(&added, 0), "Next" => b };
    // B's Next and First both go back to A.
    let second = dictionary! {
        "Title" => text("B"), "Dest" => fit(&added, 1), "Next" => a, "First" => a,
    };
    added.doc.objects.insert(a, first.into());
    added.doc.objects.insert(b, second.into());
    set_outline(&mut added, a);
    let mut p = marked(1, &[("Base", 0)]);
    p.append(vec![added]).unwrap();
    assert_eq!(tree(&p), "Base:1 A:2 B:3");
    links_agree(&p);
}

//! Reading link annotations from made-up pages, including the broken shapes real files
//! have.

use limarepdf_core::{Link, LinkTarget, Pdf};
use lopdf::{dictionary, Object, StringFormat};

fn three_pages() -> Pdf {
    let mut p = Pdf::new();
    for i in 0..3 {
        p.insert_blank(i, 300.0, 200.0).unwrap();
    }
    p
}

fn text(s: &str) -> Object {
    Object::String(s.as_bytes().to_vec(), StringFormat::Literal)
}

fn rect() -> Object {
    vec![10.into(), 20.into(), 110.into(), 50.into()].into()
}

/// Puts these annotations on the first page.
fn set_annots(p: &mut Pdf, annots: Vec<lopdf::Dictionary>) {
    let refs: Vec<Object> = annots
        .into_iter()
        .map(|a| p.doc.add_object(a).into())
        .collect();
    let first = p.page_ids()[0];
    p.doc.get_dictionary_mut(first).unwrap().set("Annots", refs);
}

fn targets(p: &Pdf) -> Vec<LinkTarget> {
    p.page_links(0)
        .unwrap()
        .into_iter()
        .map(|l| l.target)
        .collect()
}

#[test]
fn a_page_without_links_has_none() {
    assert!(three_pages().page_links(0).unwrap().is_empty());
}

#[test]
fn reads_every_kind_of_target() {
    let mut p = three_pages();
    let pg = p.page_ids();
    let link = |extra: lopdf::Dictionary| {
        let mut d = dictionary! { "Type" => "Annot", "Subtype" => "Link", "Rect" => rect() };
        d.extend(&extra);
        d
    };
    set_annots(
        &mut p,
        vec![
            link(dictionary! { "Dest" => vec![pg[2].into(), "Fit".into()] }),
            link(dictionary! { "A" => dictionary! {
            "S" => "GoTo", "D" => vec![pg[1].into(), "XYZ".into()] } }),
            link(dictionary! { "A" => dictionary! {
            "S" => "URI", "URI" => text(" https://example.org/a ") } }),
            link(dictionary! { "A" => dictionary! { "S" => "Named", "N" => "LastPage" } }),
            link(dictionary! { "A" => dictionary! { "S" => "Named", "N" => "PrevPage" } }),
            link(dictionary! { "A" => dictionary! { "S" => "Named", "N" => "NextPage" } }),
        ],
    );
    assert_eq!(
        targets(&p),
        vec![
            LinkTarget::Page(2),
            LinkTarget::Page(1),
            LinkTarget::Uri("https://example.org/a".into()),
            LinkTarget::Page(2),
            LinkTarget::Page(0),
            LinkTarget::Page(1),
        ]
    );
}

#[test]
fn skips_what_cannot_be_followed() {
    let mut p = three_pages();
    set_annots(
        &mut p,
        vec![
            // Not a link.
            dictionary! { "Subtype" => "Text", "Rect" => rect(),
            "Dest" => vec![0.into(), "Fit".into()] },
            // Another file, a program, a missing name, an empty address, no rectangle.
            dictionary! { "Subtype" => "Link", "Rect" => rect(), "A" => dictionary! {
            "S" => "GoToR", "F" => text("other.pdf"), "D" => vec![0.into(), "Fit".into()] } },
            dictionary! { "Subtype" => "Link", "Rect" => rect(), "A" => dictionary! {
            "S" => "Launch", "F" => text("run.exe") } },
            dictionary! { "Subtype" => "Link", "Rect" => rect(), "Dest" => text("nowhere") },
            dictionary! { "Subtype" => "Link", "Rect" => rect(), "A" => dictionary! {
            "S" => "URI", "URI" => text("  ") } },
            dictionary! { "Subtype" => "Link", "Dest" => vec![0.into(), "Fit".into()] },
        ],
    );
    assert!(p.page_links(0).unwrap().is_empty());
}

#[test]
fn places_links_on_the_displayed_page() {
    let mut p = three_pages();
    let pg = p.page_ids();
    let link = dictionary! { "Subtype" => "Link", "Rect" => rect(),
    "Dest" => vec![pg[1].into(), "Fit".into()] };
    set_annots(&mut p, vec![link]);
    // 300 x 200 page: y is measured down from the top.
    let want = Link {
        bbox: [10.0, 150.0, 110.0, 180.0],
        target: LinkTarget::Page(1),
    };
    assert_eq!(p.page_links(0).unwrap(), vec![want.clone()]);

    // Turned a quarter clockwise, the page's bottom edge is on the left.
    p.rotate_pages(&[0], 90).unwrap();
    let got = &p.page_links(0).unwrap()[0];
    assert_eq!(got.bbox, [20.0, 10.0, 50.0, 110.0]);
}

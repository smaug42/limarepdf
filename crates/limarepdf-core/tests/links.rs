//! Reading link annotations from made-up pages, including the broken shapes real files
//! have, and making and changing links (ENH-17).

#[allow(dead_code)]
#[path = "common/tool.rs"]
mod tool;

use limarepdf_core::{email_link, web_link, Link, LinkId, LinkTarget, Pdf, Unfollowed};
use lopdf::{dictionary, Dictionary, Object, StringFormat};

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
fn says_what_it_does_not_follow() {
    let mut p = three_pages();
    let link = |a: Dictionary| dictionary! { "Subtype" => "Link", "Rect" => rect(), "A" => a };
    set_annots(
        &mut p,
        vec![
            // Not a link, and a link without a rectangle: left out.
            dictionary! { "Subtype" => "Text", "Rect" => rect(),
            "Dest" => vec![0.into(), "Fit".into()] },
            dictionary! { "Subtype" => "Link", "Dest" => vec![0.into(), "Fit".into()] },
            link(dictionary! { "S" => "GoToR", "F" => text("other.pdf"),
            "D" => vec![0.into(), "Fit".into()] }),
            link(dictionary! { "S" => "GoToR", "F" => dictionary! {
            "Type" => "Filespec", "F" => text("old.pdf"), "UF" => text("report.pdf") } }),
            link(dictionary! { "S" => "Launch", "F" => text("run.exe") }),
            link(dictionary! { "S" => "Launch", "Win" => dictionary! { "F" => text("cmd.exe") } }),
            link(dictionary! { "S" => "JavaScript", "JS" => text("app.alert(1)") }),
            link(dictionary! { "S" => "URI", "URI" => text("file:///etc/passwd") }),
            link(dictionary! { "S" => "URI", "URI" => text("JavaScript:alert(1)") }),
            link(dictionary! { "S" => "Named", "N" => "Print" }),
            link(dictionary! { "S" => "SubmitForm" }),
            // A missing name, an empty address, no action at all.
            dictionary! { "Subtype" => "Link", "Rect" => rect(), "Dest" => text("nowhere") },
            link(dictionary! { "S" => "URI", "URI" => text("  ") }),
            dictionary! { "Subtype" => "Link", "Rect" => rect() },
        ],
    );
    let un = LinkTarget::Unfollowed;
    assert_eq!(
        targets(&p),
        vec![
            un(Unfollowed::File("other.pdf".into())),
            un(Unfollowed::File("report.pdf".into())),
            un(Unfollowed::Program("run.exe".into())),
            un(Unfollowed::Program("cmd.exe".into())),
            un(Unfollowed::Script),
            un(Unfollowed::Address("file:///etc/passwd".into())),
            un(Unfollowed::Address("JavaScript:alert(1)".into())),
            un(Unfollowed::Action("Print".into())),
            un(Unfollowed::Action("SubmitForm".into())),
            un(Unfollowed::Nowhere),
            un(Unfollowed::Nowhere),
            un(Unfollowed::Nowhere),
        ]
    );
}

#[test]
fn follows_web_and_email_addresses_and_those_without_a_scheme() {
    let mut p = three_pages();
    let uri = |u: &str| {
        dictionary! { "Subtype" => "Link", "Rect" => rect(), "A" => dictionary! {
        "S" => "URI", "URI" => text(u) } }
    };
    set_annots(
        &mut p,
        vec![
            uri("HTTP://example.org"),
            uri("mailto:you@example.org"),
            uri("www.example.org"),
            uri("example.org:8080/a"),
        ],
    );
    let all = targets(&p);
    let followed = all.iter().all(|t| matches!(t, LinkTarget::Uri(_)));
    assert!(followed, "{all:?}");
}

#[test]
fn places_links_on_the_displayed_page() {
    let mut p = three_pages();
    let pg = p.page_ids();
    let link = dictionary! { "Subtype" => "Link", "Rect" => rect(),
    "Dest" => vec![pg[1].into(), "Fit".into()] };
    set_annots(&mut p, vec![link]);
    let annots = p.doc.get_dictionary(pg[0]).unwrap().get(b"Annots").unwrap();
    let annot = annots.as_array().unwrap()[0].as_reference().unwrap();
    // 300 x 200 page: y is measured down from the top.
    let want = Link {
        id: LinkId::Object(annot),
        bbox: [10.0, 150.0, 110.0, 180.0],
        areas: vec![[10.0, 150.0, 110.0, 180.0]],
        target: LinkTarget::Page(1),
        underlined: false,
    };
    assert_eq!(p.page_links(0).unwrap(), vec![want.clone()]);

    // Turned a quarter clockwise, the page's bottom edge is on the left.
    p.rotate_pages(&[0], 90).unwrap();
    let got = &p.page_links(0).unwrap()[0];
    assert_eq!(got.bbox, [20.0, 10.0, 50.0, 110.0]);
    assert_eq!(got.areas, vec![got.bbox]);
}

// Making and changing links (ENH-17).

/// Two lines' boxes on a 300 x 200 page, in displayed points.
const LINES: [[f32; 4]; 2] = [[20.0, 30.0, 140.0, 44.0], [20.0, 46.0, 90.0, 60.0]];

fn near(a: [f32; 4], b: [f32; 4]) -> bool {
    a.iter().zip(b).all(|(x, y)| (x - y).abs() < 0.01)
}

fn only_link(p: &Pdf) -> Link {
    let links = p.page_links(0).unwrap();
    assert_eq!(links.len(), 1, "{links:?}");
    links.into_iter().next().unwrap()
}

/// What an address is refused with.
fn refusal(r: anyhow::Result<String>) -> String {
    r.unwrap_err().to_string()
}

/// A link's action.
fn action(p: &Pdf, id: (u32, u16)) -> &Dictionary {
    let d = p.doc.get_dictionary(id).unwrap();
    d.get(b"A").unwrap().as_dict().unwrap()
}

#[test]
fn typed_web_addresses_are_written_whole_and_in_ascii() {
    for (typed, want) in [
        ("example.com", "https://example.com/"),
        ("  www.example.com/a b  ", "https://www.example.com/a%20b"),
        ("HTTP://Example.COM/Path", "http://example.com/Path"),
        (
            "https://bücher.example/straße",
            "https://xn--bcher-kva.example/stra%C3%9Fe",
        ),
        ("example.com:8080/x", "https://example.com:8080/x"),
        ("localhost:3000", "https://localhost:3000/"),
        ("MAILTO:you@example.com", "mailto:you@example.com"),
    ] {
        assert_eq!(web_link(typed).unwrap(), want, "{typed}");
    }
}

#[test]
fn other_schemes_and_non_addresses_are_refused() {
    let only = "LimarePDF links to web pages and email addresses only.";
    for typed in [
        "javascript:alert(1)",
        "file:///etc/passwd",
        "ftp://example.com",
        "tel:+15551234",
    ] {
        assert_eq!(refusal(web_link(typed)), only, "{typed}");
    }
    assert_eq!(refusal(web_link("  ")), "Type the web page's address.");
    for typed in ["exa mple.com", "https://", "http://[::1"] {
        let why = refusal(web_link(typed));
        assert!(why.starts_with("That isn't a web address"), "{typed}");
    }
    let long = format!("example.com/{}", "a".repeat(5000));
    assert_eq!(refusal(web_link(&long)), "That address is too long.");
}

#[test]
fn typed_email_addresses_are_written_as_mailto() {
    let plain = email_link("you@example.com").unwrap();
    assert_eq!(plain, "mailto:you@example.com");
    assert_eq!(
        email_link(" mailto:you@Bücher.example?subject=Hi there ").unwrap(),
        "mailto:you@xn--bcher-kva.example?subject=Hi%20there"
    );
    for typed in [
        "you",
        "@example.com",
        "you@",
        "a b@example.com",
        "a@b@example.com",
        "you@exa mple.com",
    ] {
        let why = refusal(email_link(typed));
        assert!(why.starts_with("That isn't an email address"), "{typed}");
    }
    assert_eq!(refusal(email_link("mailto:")), "Type the email address.");
}

#[test]
fn a_link_over_two_lines_goes_to_a_page_and_reads_back() {
    let mut p = three_pages();
    let id = p.add_link(0, &LINES, &LinkTarget::Page(2), true).unwrap();
    let l = only_link(&p);
    assert_eq!(l.id, LinkId::Object(id));
    assert_eq!(l.target, LinkTarget::Page(2));
    assert!(l.underlined);
    assert_eq!(l.areas.len(), 2);
    let areas = l.areas.iter().zip(LINES).all(|(&a, b)| near(a, b));
    assert!(areas, "{:?}", l.areas);
    assert!(near(l.bbox, [20.0, 30.0, 140.0, 60.0]), "{:?}", l.bbox);

    // Written as a destination at the page's top, the zoom left alone, as bookmarks are.
    let d = p.doc.get_dictionary(id).unwrap();
    let dest = d.get(b"Dest").unwrap().as_array().unwrap();
    assert_eq!(dest[0].as_reference().unwrap(), p.page_ids()[2]);
    assert_eq!(dest[1].as_name().unwrap(), b"XYZ");
    assert_eq!(dest[3].as_float().unwrap(), 200.0);
    assert!(!d.has(b"A"));

    // Saved and opened again.
    let q = Pdf::from_bytes(&p.snapshot().unwrap(), None).unwrap();
    let again = only_link(&q);
    let read = (again.target, again.underlined, again.areas.len());
    assert_eq!(read, (LinkTarget::Page(2), true, 2));
}

#[test]
fn a_link_to_an_address_is_a_uri_action_checked_again() {
    let mut p = three_pages();
    let to = LinkTarget::Uri(web_link("example.com/über").unwrap());
    let id = p.add_link(0, &LINES[..1], &to, false).unwrap();
    let written = LinkTarget::Uri("https://example.com/%C3%BCber".into());
    assert_eq!(only_link(&p).target, written);
    assert_eq!(action(&p, id).get(b"S").unwrap().as_name().unwrap(), b"URI");
    assert!(!p.doc.get_dictionary(id).unwrap().has(b"Dest"));

    // Whatever the caller passes, only web and email addresses and this document's pages
    // are written.
    let before = p.page_links(0).unwrap().len();
    for bad in [
        LinkTarget::Uri("javascript:alert(1)".into()),
        LinkTarget::Uri("file:///etc/passwd".into()),
        LinkTarget::Uri("not an address".into()),
        LinkTarget::Unfollowed(Unfollowed::Script),
        LinkTarget::Page(3),
    ] {
        assert!(p.add_link(0, &LINES, &bad, false).is_err(), "{bad:?}");
    }
    let empty = [[5.0, 5.0, 5.0, 9.0]];
    assert!(p.add_link(0, &empty, &LinkTarget::Page(0), false).is_err());
    assert_eq!(p.page_links(0).unwrap().len(), before);
}

#[test]
fn an_underlined_link_draws_a_blue_line_and_one_without_draws_nothing() {
    let mut p = three_pages();
    let id = p.add_link(0, &LINES, &LinkTarget::Page(1), true).unwrap();
    let d = p.doc.get_dictionary(id).unwrap();
    assert!(d.has(b"AP"));
    let bs = d.get(b"BS").unwrap().as_dict().unwrap();
    assert_eq!(bs.get(b"S").unwrap().as_name().unwrap(), b"U");
    // Blue under each line's bottom, nothing at its middle.
    let blue = blue_rows(&mut p);
    for b in LINES {
        let bottom = b[3] as usize;
        let under = (bottom - 3..=bottom).any(|y| blue[y] > 20);
        assert!(under, "no line under {b:?}: {blue:?}");
        assert_eq!(blue[((b[1] + b[3]) / 2.0) as usize], 0);
    }

    let plain = LinkTarget::Page(1);
    p.set_link(0, LinkId::Object(id), &plain, false).unwrap();
    let d = p.doc.get_dictionary(id).unwrap();
    assert!(!d.has(b"AP"));
    assert!(!only_link(&p).underlined);
    assert!(blue_rows(&mut p).iter().all(|&n| n == 0));
}

/// Whether a pixel is blue, or the paler blue of a line drawn across two rows of pixels.
fn bluish(px: &[u8]) -> bool {
    px[2] > px[0].saturating_add(40) && px[2] > px[1]
}

/// How many blue pixels each row of the first page has, drawn at a pixel a point.
fn blue_rows(p: &mut Pdf) -> Vec<usize> {
    let hp = limarepdf_core::render::load(p.snapshot().unwrap(), None).unwrap();
    let r = limarepdf_core::render::Renderer::new(&hp);
    let b = r.render(0, 1.0).unwrap();
    let blue = |x: usize, y: usize| bluish(&b.rgba[(y * b.width + x) * 4..][..3]);
    (0..b.height)
        .map(|y| (0..b.width).filter(|&x| blue(x, y)).count())
        .collect()
}

#[test]
fn a_link_is_pointed_elsewhere_and_loses_what_it_did() {
    let mut p = three_pages();
    let launch = dictionary! { "Subtype" => "Link", "Rect" => rect(), "A" => dictionary! {
    "S" => "Launch", "F" => text("run.exe") } };
    set_annots(&mut p, vec![launch]);
    let l = only_link(&p);
    let to = LinkTarget::Uri(email_link("you@example.com").unwrap());
    let id = p.set_link(0, l.id, &to, true).unwrap();
    assert_eq!(LinkId::Object(id), l.id);
    let l = only_link(&p);
    assert_eq!((l.target, l.underlined), (to, true));
    assert!(!action(&p, id).has(b"F"));

    p.set_link(0, l.id, &LinkTarget::Page(1), true).unwrap();
    let d = p.doc.get_dictionary(id).unwrap();
    assert!(d.has(b"Dest") && !d.has(b"A"));
    let file = LinkTarget::Uri("file:///x".into());
    assert!(p.set_link(0, l.id, &file, true).is_err());
    assert_eq!(only_link(&p).target, LinkTarget::Page(1));
}

#[test]
fn a_link_moves_and_stretches_its_lines() {
    let mut p = three_pages();
    let id = p.add_link(0, &LINES, &LinkTarget::Page(1), true).unwrap();
    // Twice as wide, moved down 100 points.
    let to = [20.0, 130.0, 260.0, 160.0];
    p.move_link(0, LinkId::Object(id), to).unwrap();
    let l = only_link(&p);
    assert!(near(l.bbox, to), "{:?}", l.bbox);
    let first = near(l.areas[0], [20.0, 130.0, 260.0, 144.0]);
    let second = near(l.areas[1], [20.0, 146.0, 160.0, 160.0]);
    assert!(first && second, "{:?}", l.areas);
    let blue = blue_rows(&mut p);
    assert!((141..=144).any(|y| blue[y] > 100), "{blue:?}");
    assert!(blue[..100].iter().all(|&n| n == 0));
}

#[test]
fn links_are_removed_and_other_annotations_stay() {
    let mut p = three_pages();
    let to = LinkTarget::Page(1);
    let a = p.add_link(0, &LINES[..1], &to, false).unwrap();
    let b = p.add_link(0, &LINES[1..], &to, false).unwrap();
    let yellow = [1.0, 0.85, 0.2];
    p.add_note(0, (200.0, 100.0), yellow, "A note", "").unwrap();
    assert_eq!(p.remove_links(0, &[LinkId::Object(a)]).unwrap(), 1);
    assert_eq!(only_link(&p).id, LinkId::Object(b));
    assert_eq!(p.annotations().len(), 1);
    // Something that isn't a link isn't removed as one.
    let note = p.annotations()[0].id;
    assert!(p.remove_links(0, &[LinkId::Object(note)]).is_err());
    assert_eq!(p.annotations().len(), 1);
}

#[test]
fn a_link_written_inside_the_page_list_is_changed_and_removed() {
    let mut p = three_pages();
    let first = p.page_ids()[0];
    let inline = dictionary! { "Subtype" => "Link", "Rect" => rect(), "A" => dictionary! {
    "S" => "URI", "URI" => text("https://example.org") } };
    let list = vec![Object::Dictionary(inline)];
    p.doc.get_dictionary_mut(first).unwrap().set("Annots", list);
    let l = only_link(&p);
    assert_eq!(l.id, LinkId::Inline { page: 0, at: 0 });
    let id = p.set_link(0, l.id, &LinkTarget::Page(2), false).unwrap();
    let l = only_link(&p);
    assert_eq!((l.id, l.target), (LinkId::Object(id), LinkTarget::Page(2)));
    assert_eq!(p.remove_links(0, &[l.id]).unwrap(), 1);
    assert!(p.page_links(0).unwrap().is_empty());
}

#[test]
fn a_link_to_a_deleted_page_goes_nowhere_or_goes() {
    let mut p = three_pages();
    p.add_link(0, &LINES, &LinkTarget::Page(2), false).unwrap();
    p.delete_pages(&[2]).unwrap();
    let left = p.page_links(0).unwrap();
    let nowhere = LinkTarget::Unfollowed(Unfollowed::Nowhere);
    assert!(left.iter().all(|l| l.target == nowhere), "{left:?}");
}

/// Poppler reads what LimarePDF writes: `pdfinfo -url` lists the addresses, and the
/// underline shows when Poppler draws the page. Opt-in: needs `pdfinfo` and `pdftoppm`.
#[test]
#[ignore]
fn cross_check_links_with_poppler() {
    let mut p = three_pages();
    let web = LinkTarget::Uri(web_link("example.com/a").unwrap());
    let email = LinkTarget::Uri(email_link("you@example.com").unwrap());
    p.add_link(0, &LINES[..1], &web, true).unwrap();
    p.add_link(0, &LINES[1..], &email, false).unwrap();
    let first = LinkTarget::Page(0);
    p.add_link(1, &LINES[..1], &first, true).unwrap();
    let dir = tool::Scratch::new("links");
    let file = dir.join("links.pdf");
    std::fs::write(&file, p.snapshot().unwrap()).unwrap();

    let out = tool::output(std::process::Command::new("pdfinfo").arg("-url").arg(&file)).unwrap();
    let urls = String::from_utf8_lossy(&out.stdout);
    assert!(out.status.success(), "{urls}");
    assert!(urls.contains("https://example.com/a"), "{urls}");
    assert!(urls.contains("mailto:you@example.com"), "{urls}");

    let png = dir.join("page");
    let ok = tool::output(
        std::process::Command::new("pdftoppm")
            .args(["-r", "72", "-f", "1", "-l", "1", "-png", "-singlefile"])
            .arg(&file)
            .arg(&png),
    )
    .unwrap()
    .status;
    assert!(ok.success());
    let img = image::open(png.with_extension("png")).unwrap().to_rgb8();
    let blue_in = |rows: std::ops::Range<u32>| {
        rows.flat_map(|y| (0..img.width()).map(move |x| (x, y)))
            .filter(|&(x, y)| bluish(&img.get_pixel(x, y).0))
            .count()
    };
    // The first line is underlined, the second isn't.
    assert!(blue_in(40..45) > 50, "no underline drawn by Poppler");
    assert_eq!(blue_in(46..61), 0);
}

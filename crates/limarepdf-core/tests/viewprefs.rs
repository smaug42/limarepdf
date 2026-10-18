//! Viewer preferences: reading, writing and keeping what LimarePDF reads the same way.

use limarepdf_core::{OpenLayout, OpenPanel, Pdf, ViewPrefs};
use lopdf::{dictionary, Object};

fn pages(n: usize) -> Pdf {
    let mut p = Pdf::new();
    for i in 0..n {
        p.insert_blank(i, 300.0, 200.0).unwrap();
    }
    p
}

fn reopen(p: &mut Pdf) -> Pdf {
    Pdf::from_bytes(&p.to_bytes().unwrap(), None).unwrap()
}

#[test]
fn none_by_default() {
    assert_eq!(pages(2).view_prefs(), ViewPrefs::default());
}

#[test]
fn set_and_read_back() {
    let mut p = pages(3);
    let prefs = ViewPrefs {
        layout: Some(OpenLayout::FacingCover),
        panel: Some(OpenPanel::Bookmarks),
        page: Some(2),
    };
    p.set_view_prefs(&prefs).unwrap();
    let p = reopen(&mut p);
    assert_eq!(p.view_prefs(), prefs);
    let cat = p.doc.catalog().unwrap();
    assert_eq!(
        cat.get(b"PageLayout").unwrap().as_name().unwrap(),
        b"TwoColumnRight"
    );
    assert_eq!(
        cat.get(b"PageMode").unwrap().as_name().unwrap(),
        b"UseOutlines"
    );
}

#[test]
fn clearing_removes_the_keys() {
    let mut p = pages(2);
    p.set_view_prefs(&ViewPrefs {
        layout: Some(OpenLayout::Single),
        panel: Some(OpenPanel::Pages),
        page: Some(1),
    })
    .unwrap();
    p.set_view_prefs(&ViewPrefs::default()).unwrap();
    let cat = p.doc.catalog().unwrap();
    for key in [&b"PageLayout"[..], b"PageMode", b"OpenAction"] {
        assert!(cat.get(key).is_err());
    }
}

#[test]
fn reads_every_layout_name_and_goto_actions() {
    let mut p = pages(3);
    let third = p.page_ids()[2];
    let cat = p.doc.catalog_mut().unwrap();
    cat.set("PageLayout", Object::Name(b"TwoPageLeft".to_vec()));
    cat.set(
        "OpenAction",
        dictionary! { "S" => "GoTo", "D" => vec![third.into(), "Fit".into()] },
    );
    let prefs = p.view_prefs();
    assert_eq!(prefs.layout, Some(OpenLayout::Facing));
    assert_eq!(prefs.page, Some(2));
}

#[test]
fn unchanged_values_are_kept_as_written() {
    let mut p = pages(2);
    let cat = p.doc.catalog_mut().unwrap();
    cat.set("PageLayout", Object::Name(b"TwoPageLeft".to_vec()));
    cat.set("PageMode", Object::Name(b"FullScreen".to_vec()));
    cat.set(
        "OpenAction",
        dictionary! { "S" => "JavaScript", "JS" => Object::string_literal("") },
    );
    let mut prefs = p.view_prefs();
    assert_eq!(
        prefs,
        ViewPrefs {
            layout: Some(OpenLayout::Facing),
            ..Default::default()
        }
    );
    // Only the panel changes; the layout name, and the opening action that isn't a page, stay.
    prefs.panel = Some(OpenPanel::Pages);
    p.set_view_prefs(&prefs).unwrap();
    let cat = p.doc.catalog().unwrap();
    assert_eq!(
        cat.get(b"PageLayout").unwrap().as_name().unwrap(),
        b"TwoPageLeft"
    );
    assert!(cat.get(b"OpenAction").unwrap().as_dict().is_ok());
    p.set_view_prefs(&ViewPrefs::default()).unwrap();
    assert!(p.doc.catalog().unwrap().get(b"OpenAction").is_ok());
}

#[test]
fn page_must_exist() {
    let mut p = pages(2);
    let prefs = ViewPrefs {
        page: Some(2),
        ..Default::default()
    };
    assert!(p.set_view_prefs(&prefs).is_err());
}

#[test]
fn names_round_trip() {
    for l in OpenLayout::ALL {
        assert_eq!(OpenLayout::from_name(l.name()), Some(l));
    }
    for panel in OpenPanel::ALL {
        assert_eq!(OpenPanel::from_name(panel.name()), Some(panel));
    }
}

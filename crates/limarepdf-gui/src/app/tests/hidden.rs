//! S1-02: Remove hidden information counts content that only a layer rule hides (every layer
//! shown), and counts, then refuses, with nothing changed and no undo step, to cut what lies
//! outside the crop box when redaction's check finds something left.

use super::*;
use limarepdf_core::Hidden;
use lopdf::{dictionary, Dictionary, Object, ObjectId, Stream};

/// A one-page 300×200 pt document drawing `content` with `/F1` (Helvetica).
fn make(content: &str) -> (Pdf, ObjectId) {
    let mut p = Pdf::new();
    p.insert_blank(0, 300.0, 200.0).unwrap();
    let font = p.doc.add_object(dictionary! {
        "Type" => "Font", "Subtype" => "Type1", "BaseFont" => "Helvetica", "Encoding" => "WinAnsiEncoding",
    });
    let s = p
        .doc
        .add_object(Stream::new(Dictionary::new(), content.as_bytes().to_vec()));
    let page = p.page_ids()[0];
    let d = p.doc.get_dictionary_mut(page).unwrap();
    d.set("Contents", s);
    d.set(
        "Resources",
        dictionary! { "Font" => dictionary! { "F1" => font } },
    );
    (p, font)
}

/// Set `key` in the page's resources.
fn res_set(p: &mut Pdf, key: &str, v: Object) {
    let page = p.page_ids()[0];
    let d = p.doc.get_dictionary_mut(page).unwrap();
    let res = d.get_mut(b"Resources").unwrap().as_dict_mut().unwrap();
    res.set(key, v);
}

/// The app with `pdf` in a tab of its own.
fn app_with(ctx: &egui::Context, pdf: Pdf) -> App {
    let mut app = App::with(Settings::from_file(None), Recent::from_file(None));
    assert!(app.add_tab(ctx, pdf, None, None, false));
    app
}

#[test]
fn content_only_a_layer_rule_hides_is_counted_in_the_dialog() {
    let ctx = egui::Context::default();
    let (mut p, _) = make(
        "/OC /R BDC BT /F1 12 Tf 30 150 Td (RULE-SECRET) Tj ET EMC \
         BT /F1 12 Tf 30 100 Td (Visible text) Tj ET",
    );
    let a = p
        .doc
        .add_object(dictionary! { "Type" => "OCG", "Name" => Object::string_literal("A") });
    p.doc.catalog_mut().unwrap().set(
        "OCProperties",
        dictionary! { "OCGs" => vec![Object::Reference(a)], "D" => dictionary! {} },
    );
    let not_a = vec![Object::Name(b"Not".to_vec()), Object::Reference(a)];
    let r = p
        .doc
        .add_object(dictionary! { "Type" => "OCMD", "VE" => not_a });
    res_set(&mut p, "Properties", dictionary! { "R" => r }.into());
    let mut app = app_with(&ctx, p);
    app.act(&ctx, Act::RemoveHidden);
    let Dialog::RemoveHidden(_, s) = &app.dialog else {
        panic!("no Remove hidden information dialog");
    };
    assert!(
        s.kinds.contains(&(Hidden::HiddenLayers, 1, true)),
        "{:?}",
        s.kinds
    );
}

#[test]
fn content_outside_the_crop_box_redaction_leaves_is_refused_with_nothing_changed() {
    let ctx = egui::Context::default();
    let (mut p, font) = make("BT /F1 12 Tf 100 100 Td (Inside) Tj ET q /Fm Do Q");
    let form = p.doc.add_object(Stream::new(
        dictionary! {
            "Type" => "XObject", "Subtype" => "Form",
            "BBox" => vec![0.into(), 0.into(), 300.into(), 200.into()],
            "Resources" => dictionary! { "Font" => dictionary! { "F1" => font } },
        },
        b"BT /F1 6 Tf 2 100 Td (OutZebra) Tj ET BT /F1 12 Tf 100 60 Td (Kept) Tj ET".to_vec(),
    ));
    res_set(&mut p, "XObject", dictionary! { "Fm" => form }.into());
    p.crop_page(0, [50.0, 20.0, 250.0, 180.0]).unwrap();
    // The document's tag tree points at the form, so the original may stay in the file.
    let page = p.page_ids()[0];
    let root = p.doc.new_object_id();
    let mcr = dictionary! { "Type" => "MCR", "Pg" => page, "Stm" => form, "MCID" => 0 };
    let elem = p.doc.add_object(dictionary! {
        "Type" => "StructElem", "S" => "P", "P" => root, "Pg" => page,
        "K" => vec![Object::Dictionary(mcr)],
    });
    p.doc.objects.insert(
        root,
        dictionary! { "Type" => "StructTreeRoot", "K" => elem }.into(),
    );
    let c = p.doc.catalog_mut().unwrap();
    c.set("StructTreeRoot", root);
    c.set("MarkInfo", dictionary! { "Marked" => true });
    let mut app = app_with(&ctx, p);
    let (was, before) = (app.tabs[0].state(), app.tabs[0].pdf.state_before());
    // The dialog counts the page, so it can be chosen.
    app.act(&ctx, Act::RemoveHidden);
    let Dialog::RemoveHidden(_, s) = &app.dialog else {
        panic!("no Remove hidden information dialog");
    };
    assert!(
        s.kinds.contains(&(Hidden::OutsideCrop, 1, false)),
        "{:?}",
        s.kinds
    );
    app.remove_hidden(&ctx, &[Hidden::Metadata, Hidden::OutsideCrop]);
    let t = &mut app.tabs[0];
    let said = t.error.take().unwrap_or_default();
    assert!(
        said.contains(
            "Content outside the visible page area can't be removed for certain: the check \
             after redacting found removed content may still be in the file, through the tag \
             tree"
        ),
        "{said:?}"
    );
    assert!(t.state() == was, "the tab changed");
    assert_eq!(t.pdf.state_since(&before), []);
}

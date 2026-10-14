//! Reading form fields from made-up forms, including the broken shapes real files have.

use lopdf::{dictionary, Dictionary, Object, ObjectId, StringFormat};
use mpe_core::{Field, FieldKind, FieldValue, Pdf};

fn two_pages() -> Pdf {
    let mut p = Pdf::new();
    for i in 0..2 {
        p.insert_blank(i, 300.0, 200.0).unwrap();
    }
    p
}

fn text(s: &str) -> Object {
    Object::String(s.as_bytes().to_vec(), StringFormat::Literal)
}

fn rect(x: i64, y: i64) -> Object {
    vec![x.into(), y.into(), (x + 100).into(), (y + 20).into()].into()
}

/// A checkbox or radio appearance with an "on" state named `on`.
fn states(on: &str) -> Object {
    let n = dictionary! { on => Object::Null, "Off" => Object::Null };
    dictionary! { "N" => n }.into()
}

/// Adds widget annotations to a page's `/Annots`.
fn on_page(p: &mut Pdf, page: usize, annots: &[ObjectId]) {
    let id = p.page_ids()[page];
    let d = p.doc.get_dictionary_mut(id).unwrap();
    let mut list = d
        .get(b"Annots")
        .ok()
        .and_then(|o| o.as_array().ok())
        .cloned()
        .unwrap_or_default();
    list.extend(annots.iter().map(|&a| Object::from(a)));
    d.set("Annots", list);
}

/// Sets the form's top-level fields, returning the form's id.
fn set_form(p: &mut Pdf, fields: &[ObjectId]) -> ObjectId {
    let fields: Vec<Object> = fields.iter().map(|&f| f.into()).collect();
    let form = p.doc.add_object(dictionary! { "Fields" => fields });
    p.doc.catalog_mut().unwrap().set("AcroForm", form);
    form
}

fn widget(extra: Dictionary) -> Dictionary {
    let mut d = dictionary! { "Type" => "Annot", "Subtype" => "Widget" };
    d.extend(&extra);
    d
}

fn find<'a>(fields: &'a [Field], name: &str) -> &'a Field {
    fields
        .iter()
        .find(|f| f.name == name)
        .unwrap_or_else(|| panic!("no field {name}"))
}

#[test]
fn no_form_gives_no_fields() {
    assert!(two_pages().form_fields().is_empty());
}

#[test]
fn reads_every_kind_of_field() {
    let mut p = two_pages();

    // A text field that is its own widget, with flags, a tooltip and a value.
    let name = p.doc.add_object(widget(dictionary! {
        "FT" => "Tx", "T" => text("name"), "TU" => text("Your name"), "V" => text("Ann"),
        "Ff" => 2 | 4096, "MaxLen" => 40, "Rect" => rect(10, 150),
    }));

    // A parent "address" with an inherited type and two child text fields.
    let street = p.doc.add_object(widget(
        dictionary! { "T" => text("street"), "Rect" => rect(10, 120) },
    ));
    let city = p.doc.add_object(widget(dictionary! {
        "T" => text("city"), "V" => text("Oslo"), "Ff" => 1 << 24, "Rect" => rect(10, 90),
    }));
    let address = p.doc.add_object(dictionary! {
        "T" => text("address"), "FT" => "Tx", "Kids" => vec![street.into(), city.into()],
    });

    // A checkbox, ticked.
    let ap = states("Yes");
    let agree = p.doc.add_object(widget(dictionary! {
        "FT" => "Btn", "T" => text("agree"), "V" => "Yes", "AS" => "Yes", "AP" => ap,
        "Rect" => rect(10, 60), "Ff" => 1,
    }));

    // A radio group with two widgets on different pages, whose states index /Opt.
    let (ap0, ap1) = (states("0"), states("1"));
    let r0 = p
        .doc
        .add_object(widget(dictionary! { "AP" => ap0, "Rect" => rect(150, 60) }));
    let r1 = p.doc.add_object(widget(
        dictionary! { "AP" => ap1, "Rect" => rect(150, 60), "F" => 2 },
    ));
    let colour = p.doc.add_object(dictionary! {
        "FT" => "Btn", "Ff" => 1 << 15, "T" => text("colour"), "V" => "1",
        "Opt" => vec![text("red"), text("blue")], "Kids" => vec![r0.into(), r1.into()],
    });

    // A drop-down with [export, shown] pairs, and a multi-select list.
    let size = p.doc.add_object(widget(dictionary! {
        "FT" => "Ch", "Ff" => 1 << 17, "T" => text("size"), "V" => text("m"),
        "Opt" => vec![vec![text("s"), text("Small")].into(), vec![text("m"), text("Medium")].into()],
        "Rect" => rect(150, 30),
    }));
    let pets = p.doc.add_object(widget(dictionary! {
        "FT" => "Ch", "Ff" => 1 << 21, "T" => text("pets"),
        "V" => vec![text("cat"), text("dog")], "Opt" => vec![text("cat"), text("dog"), text("eel")],
        "Rect" => rect(10, 30),
    }));

    // A push button and a signature field.
    let go = p.doc.add_object(widget(dictionary! {
        "FT" => "Btn", "Ff" => 1 << 16, "T" => text("go"), "Rect" => rect(150, 150),
    }));
    let sig = p.doc.add_object(widget(dictionary! {
        "FT" => "Sig", "T" => text("sig"), "Rect" => rect(150, 120),
    }));

    on_page(
        &mut p,
        0,
        &[name, street, city, agree, r0, size, pets, go, sig],
    );
    on_page(&mut p, 1, &[r1]);
    set_form(&mut p, &[name, address, agree, colour, size, pets, go, sig]);
    let fields = p.form_fields();
    let names: Vec<&str> = fields.iter().map(|f| f.name.as_str()).collect();
    assert_eq!(
        names,
        [
            "name",
            "address.street",
            "address.city",
            "agree",
            "colour",
            "size",
            "pets",
            "go",
            "sig"
        ]
    );

    let f = find(&fields, "name");
    assert_eq!(
        f.kind,
        FieldKind::Text {
            multiline: true,
            password: false,
            comb: false,
            max_len: Some(40),
        }
    );
    assert_eq!(f.value, FieldValue::Text("Ann".into()));
    assert!(f.required && !f.read_only);
    assert_eq!(f.tooltip.as_deref(), Some("Your name"));
    // 300 x 200 page: y is measured down from the top.
    assert_eq!(f.widgets[0].page, 0);
    assert_eq!(f.widgets[0].bbox, [10.0, 30.0, 110.0, 50.0]);

    assert_eq!(
        find(&fields, "address.street").value,
        FieldValue::Text(String::new())
    );
    let city = find(&fields, "address.city");
    assert!(matches!(city.kind, FieldKind::Text { comb: true, .. }));
    assert_eq!(city.value, FieldValue::Text("Oslo".into()));

    let agree = find(&fields, "agree");
    assert_eq!(agree.kind, FieldKind::Check);
    assert_eq!(agree.value, FieldValue::Check(true));
    assert!(agree.read_only);
    assert_eq!(agree.widgets[0].on_state.as_deref(), Some("Yes"));

    let colour = find(&fields, "colour");
    assert_eq!(colour.kind, FieldKind::Radio);
    assert_eq!(colour.value, FieldValue::Choice(vec!["blue".into()]));
    let opts: Vec<&str> = colour.options.iter().map(|o| o.0.as_str()).collect();
    assert_eq!(opts, ["red", "blue"]);
    let pages: Vec<usize> = colour.widgets.iter().map(|w| w.page).collect();
    assert_eq!(pages, [0, 1]);
    assert!(!colour.widgets[0].hidden && colour.widgets[1].hidden);
    let exports: Vec<Option<&str>> = colour.widgets.iter().map(|w| w.export.as_deref()).collect();
    assert_eq!(exports, [Some("red"), Some("blue")]);

    let size = find(&fields, "size");
    assert!(matches!(
        size.kind,
        FieldKind::Choice {
            combo: true,
            multi: false,
            ..
        }
    ));
    assert_eq!(size.value, FieldValue::Choice(vec!["m".into()]));
    assert_eq!(size.options[1], ("m".into(), "Medium".into()));

    let pets = find(&fields, "pets");
    assert!(matches!(
        pets.kind,
        FieldKind::Choice {
            combo: false,
            multi: true,
            ..
        }
    ));
    assert_eq!(
        pets.value,
        FieldValue::Choice(vec!["cat".into(), "dog".into()])
    );

    assert_eq!(find(&fields, "go").kind, FieldKind::Button);
    assert_eq!(find(&fields, "go").value, FieldValue::None);
    assert_eq!(find(&fields, "sig").kind, FieldKind::Signature);
}

#[test]
fn skips_broken_fields() {
    let mut p = two_pages();
    // No field type anywhere in its parents.
    let untyped = p.doc.add_object(widget(
        dictionary! { "T" => text("untyped"), "Rect" => rect(0, 0) },
    ));
    // A parent that lists itself among its kids.
    let looped = p.doc.new_object_id();
    let child = p.doc.add_object(widget(dictionary! {
        "T" => text("child"), "FT" => "Tx", "Rect" => rect(0, 30),
    }));
    p.doc.objects.insert(
        looped,
        dictionary! { "T" => text("loop"), "Kids" => vec![looped.into(), child.into()] }.into(),
    );
    // A field whose widget isn't on any page: listed, without widgets.
    let offpage = p.doc.add_object(widget(dictionary! {
        "T" => text("offpage"), "FT" => "Tx", "Rect" => rect(0, 60),
    }));
    on_page(&mut p, 0, &[untyped, child]);
    let form = set_form(&mut p, &[untyped, looped, offpage]);
    // A dangling reference in the field list.
    let missing = Object::Reference((9999, 0));
    p.doc
        .get_dictionary_mut(form)
        .unwrap()
        .get_mut(b"Fields")
        .unwrap()
        .as_array_mut()
        .unwrap()
        .push(missing);

    let fields = p.form_fields();
    let names: Vec<&str> = fields.iter().map(|f| f.name.as_str()).collect();
    assert_eq!(names, ["loop.child", "offpage"]);
    assert_eq!(fields[0].widgets.len(), 1);
    assert!(fields[1].widgets.is_empty());
}

#[test]
fn tells_xfa_only_forms_from_hybrid_ones() {
    let mut p = two_pages();
    assert!(!p.xfa_only(), "no form at all");
    let xfa = Object::Stream(lopdf::Stream::new(dictionary! {}, b"<xdp:xdp/>".to_vec()));
    let xfa = p.doc.add_object(xfa);

    // XFA with no ordinary fields: only Adobe software can show it.
    let form = set_form(&mut p, &[]);
    p.doc.get_dictionary_mut(form).unwrap().set("XFA", xfa);
    assert!(p.xfa_only());

    // Hybrid: XFA plus ordinary fields on the pages, which MPE fills.
    let f = p.doc.add_object(widget(dictionary! {
        "FT" => "Tx", "T" => text("t"), "Rect" => rect(0, 0),
    }));
    on_page(&mut p, 0, &[f]);
    p.doc
        .get_dictionary_mut(form)
        .unwrap()
        .set("Fields", vec![f.into()]);
    assert!(!p.xfa_only());

    // ...unless the file says its pages must be built from the XFA.
    p.doc.catalog_mut().unwrap().set("NeedsRendering", true);
    assert!(p.xfa_only());
}

#[test]
fn places_widgets_on_rotated_pages() {
    let mut p = two_pages();
    let f = p.doc.add_object(widget(dictionary! {
        "FT" => "Tx", "T" => text("t"), "Rect" => rect(10, 150),
    }));
    on_page(&mut p, 0, &[f]);
    set_form(&mut p, &[f]);
    p.rotate_pages(&[0], 90).unwrap();
    // Turned a quarter clockwise, the page's bottom edge is on the left.
    assert_eq!(
        p.form_fields()[0].widgets[0].bbox,
        [150.0, 10.0, 170.0, 110.0]
    );
}

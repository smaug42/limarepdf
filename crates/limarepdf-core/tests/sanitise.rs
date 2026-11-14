mod common;

use common::compare::{differences, Image};
use common::saved::{tools, Saved};
use limarepdf_core::{render, FieldValue, Hidden, Info, Pdf};
use lopdf::{dictionary, Dictionary, Object, ObjectId, Stream};

/// A one-page 300×200 pt document drawing `content` with `/F1` (Helvetica), with extra
/// resources.
fn make(content: &str, extra: Dictionary) -> Pdf {
    let mut p = Pdf::new();
    p.insert_blank(0, 300.0, 200.0).unwrap();
    let font = p.doc.add_object(dictionary! {
        "Type" => "Font", "Subtype" => "Type1", "BaseFont" => "Helvetica", "Encoding" => "WinAnsiEncoding",
    });
    let page = p.page_ids()[0];
    let s = p
        .doc
        .add_object(Stream::new(Dictionary::new(), content.as_bytes().to_vec()));
    let mut res = dictionary! { "Font" => dictionary! { "F1" => font } };
    for (k, v) in extra.iter() {
        res.set(k.clone(), v.clone());
    }
    let d = p.doc.get_dictionary_mut(page).unwrap();
    d.set("Contents", Object::Reference(s));
    d.set("Resources", res);
    p
}

/// Whether `needle` is anywhere in the saved file: in any string or (decompressed) stream.
fn file_has(p: &mut Pdf, needle: &str) -> bool {
    let bytes = p.to_bytes().unwrap();
    let q = Pdf::from_bytes(&bytes, None).unwrap();
    let n = needle.as_bytes();
    let has = |b: &[u8]| b.windows(n.len()).any(|w| w == n);
    fn strings(o: &Object, has: &dyn Fn(&[u8]) -> bool) -> bool {
        match o {
            Object::String(b, _) => has(b),
            Object::Array(a) => a.iter().any(|x| strings(x, has)),
            Object::Dictionary(d) => d.iter().any(|(_, v)| strings(v, has)),
            Object::Stream(s) => {
                has(&s
                    .decompressed_content()
                    .unwrap_or_else(|_| s.content.clone()))
                    || s.dict.iter().any(|(_, v)| strings(v, has))
            }
            _ => false,
        }
    }
    q.doc.objects.values().any(|o| strings(o, &has))
}

fn catalog_set(p: &mut Pdf, key: &str, v: Object) {
    p.doc.catalog_mut().unwrap().set(key, v);
}

#[test]
fn finding_changes_nothing_and_defaults_are_the_safe_kinds() {
    let mut p = make("", Dictionary::new());
    p.set_info(&Info {
        title: Some("Secret title".into()),
        ..Default::default()
    })
    .unwrap();
    let found = p.find_hidden().unwrap();
    assert_eq!(found.count(Hidden::Metadata), 1);
    assert_eq!(p.info().title.as_deref(), Some("Secret title"));
    let on: Vec<Hidden> = Hidden::ALL.into_iter().filter(|h| h.default_on()).collect();
    assert!(on.contains(&Hidden::Scripts) && !on.contains(&Hidden::Comments));
    assert_eq!(Hidden::parse("hidden-layers"), Some(Hidden::HiddenLayers));
}

#[test]
fn removes_metadata_and_xmp() {
    let mut p = make("", Dictionary::new());
    p.set_info(&Info {
        author: Some("Walrus Author".into()),
        ..Default::default()
    })
    .unwrap();
    let xmp = p.doc.add_object(Stream::new(
        dictionary! { "Type" => "Metadata", "Subtype" => "XML" },
        b"<x:xmpmeta>Walrus XMP</x:xmpmeta>".to_vec(),
    ));
    catalog_set(&mut p, "Metadata", Object::Reference(xmp));
    let rep = p.remove_hidden(&[Hidden::Metadata]).unwrap();
    assert_eq!(rep.count(Hidden::Metadata), 2);
    assert!(!file_has(&mut p, "Walrus"));
}

#[test]
fn removes_scripts_but_keeps_web_links() {
    let mut p = make("", Dictionary::new());
    let js = p.doc.add_object(
        dictionary! { "S" => "JavaScript", "JS" => Object::string_literal("app.alert('boo')") },
    );
    catalog_set(&mut p, "OpenAction", Object::Reference(js));
    let named = p.doc.add_object(
        dictionary! { "S" => "JavaScript", "JS" => Object::string_literal("app.beep()") },
    );
    let tree =
        dictionary! { "Names" => vec![Object::string_literal("x"), Object::Reference(named)] };
    catalog_set(&mut p, "Names", dictionary! { "JavaScript" => tree }.into());
    let page = p.page_ids()[0];
    let r = vec![0.into(), 0.into(), 50.into(), 50.into()];
    let bad = p.doc.add_object(dictionary! {
        "Type" => "Annot", "Subtype" => "Link", "Rect" => r.clone(),
        "A" => dictionary! { "S" => "JavaScript", "JS" => Object::string_literal("app.launchURL('x')") },
    });
    let good = p.doc.add_object(dictionary! {
        "Type" => "Annot", "Subtype" => "Link", "Rect" => r,
        "A" => dictionary! { "S" => "URI", "URI" => Object::string_literal("https://example.org") },
    });
    let d = p.doc.get_dictionary_mut(page).unwrap();
    d.set(
        "Annots",
        vec![Object::Reference(bad), Object::Reference(good)],
    );
    d.set("AA", dictionary! { "O" => Object::Reference(js) });
    let rep = p.remove_hidden(&[Hidden::Scripts]).unwrap();
    assert!(rep.count(Hidden::Scripts) >= 4, "{rep:?}");
    assert!(!file_has(&mut p, "app."));
    assert!(file_has(&mut p, "https://example.org"));
}

#[test]
fn removes_attachments() {
    let mut p = make("", Dictionary::new());
    let file = p.doc.add_object(Stream::new(
        dictionary! { "Type" => "EmbeddedFile" },
        b"Walrus attachment".to_vec(),
    ));
    let spec = p.doc.add_object(dictionary! {
        "Type" => "Filespec", "F" => Object::string_literal("notes.txt"),
        "EF" => dictionary! { "F" => file },
    });
    let tree = dictionary! { "Names" => vec![Object::string_literal("notes.txt"), Object::Reference(spec)] };
    catalog_set(
        &mut p,
        "Names",
        dictionary! { "EmbeddedFiles" => tree }.into(),
    );
    assert!(file_has(&mut p, "Walrus attachment"));
    let rep = p
        .remove_hidden(
            &Hidden::ALL
                .map(|h| h)
                .into_iter()
                .filter(|h| h.default_on())
                .collect::<Vec<_>>(),
        )
        .unwrap();
    assert_eq!(rep.count(Hidden::Attachments), 1);
    assert!(!file_has(&mut p, "Walrus attachment"));
}

#[test]
fn removes_the_content_of_hidden_layers() {
    let mut p = make(
        "/OC /L1 BDC BT /F1 20 Tf 30 100 Td (Walrus hidden) Tj ET EMC \
         /OC /L2 BDC BT /F1 20 Tf 30 50 Td (Shown text) Tj ET EMC q /Fm1 Do Q",
        Dictionary::new(),
    );
    let off = p
        .doc
        .add_object(dictionary! { "Type" => "OCG", "Name" => Object::string_literal("Off") });
    let on = p
        .doc
        .add_object(dictionary! { "Type" => "OCG", "Name" => Object::string_literal("On") });
    let font = p.doc.add_object(
        dictionary! { "Type" => "Font", "Subtype" => "Type1", "BaseFont" => "Helvetica" },
    );
    let form = p.doc.add_object(Stream::new(
        dictionary! {
            "Type" => "XObject", "Subtype" => "Form", "OC" => off,
            "BBox" => vec![0.into(), 0.into(), 300.into(), 200.into()],
            "Resources" => dictionary! { "Font" => dictionary! { "F1" => font } },
        },
        b"BT /F1 12 Tf 30 150 Td (Walrus in a form) Tj ET".to_vec(),
    ));
    let page = p.page_ids()[0];
    let mut res = p
        .doc
        .get_dictionary(page)
        .unwrap()
        .get(b"Resources")
        .unwrap()
        .as_dict()
        .unwrap()
        .clone();
    res.set("Properties", dictionary! { "L1" => off, "L2" => on });
    res.set("XObject", dictionary! { "Fm1" => form });
    p.doc
        .get_dictionary_mut(page)
        .unwrap()
        .set("Resources", res);
    catalog_set(
        &mut p,
        "OCProperties",
        dictionary! {
            "OCGs" => vec![Object::Reference(off), Object::Reference(on)],
            "D" => dictionary! { "OFF" => vec![Object::Reference(off)], "Order" => vec![Object::Reference(off), Object::Reference(on)] },
        }
        .into(),
    );
    let rep = p.remove_hidden(&[Hidden::HiddenLayers]).unwrap();
    assert_eq!(rep.count(Hidden::HiddenLayers), 1);
    let text = p.page_text(0);
    assert!(text.contains("Shown") && !text.contains("Walrus"), "{text}");
    assert!(!file_has(&mut p, "Walrus"));
    let props = p
        .doc
        .catalog()
        .unwrap()
        .get(b"OCProperties")
        .unwrap()
        .as_dict()
        .unwrap()
        .clone();
    assert_eq!(
        props.get(b"OCGs").unwrap().as_array().unwrap(),
        &vec![Object::Reference(on)]
    );
}

#[test]
fn cuts_what_lies_outside_the_crop_box() {
    let mut p = make(
        "BT /F1 12 Tf 5 100 Td (Walrus) Tj ET BT /F1 12 Tf 100 100 Td (Inside) Tj ET",
        Dictionary::new(),
    );
    p.crop_page(0, [50.0, 20.0, 250.0, 180.0]).unwrap();
    let rep = p.remove_hidden(&[Hidden::OutsideCrop]).unwrap();
    assert_eq!(rep.count(Hidden::OutsideCrop), 1);
    assert!(p.page_text(0).contains("Inside"));
    assert!(!file_has(&mut p, "Walrus"));
    assert_eq!(p.page_size(0).unwrap(), (200.0, 160.0));
}

fn s(t: &str) -> Object {
    Object::string_literal(t)
}

fn rect(r: [i64; 4]) -> Object {
    r.into_iter()
        .map(Object::Integer)
        .collect::<Vec<_>>()
        .into()
}

/// Set `key` in the first page's resources.
fn page_res_set(p: &mut Pdf, key: &str, v: Object) {
    let page = p.page_ids()[0];
    let d = p.doc.get_dictionary_mut(page).unwrap();
    let res = d.get_mut(b"Resources").unwrap().as_dict_mut().unwrap();
    res.set(key, v);
}

/// Add `id` to the first page's annotations.
fn add_annot(p: &mut Pdf, id: ObjectId) {
    let page = p.page_ids()[0];
    let d = p.doc.get_dictionary_mut(page).unwrap();
    match d.get_mut(b"Annots") {
        Ok(Object::Array(a)) => a.push(id.into()),
        _ => d.set("Annots", vec![Object::Reference(id)]),
    }
}

/// A widget on the first page, its own field, with `extra` entries.
fn widget(p: &mut Pdf, extra: Dictionary) -> ObjectId {
    let page = p.page_ids()[0];
    let mut d = dictionary! {
        "Type" => "Annot", "Subtype" => "Widget", "P" => page, "F" => 4,
        "Rect" => rect([10, 10, 150, 30]),
    };
    for (k, v) in extra.iter() {
        d.set(k.clone(), v.clone());
    }
    let id = p.doc.add_object(d);
    add_annot(p, id);
    id
}

/// A form holding values in every place a value can be: a text field's value, default and
/// rich text; a read-only field; a value inherited from a parent; a ticked checkbox; a chosen
/// and a typed drop-down value; and a signed signature field. Options and names aren't values.
fn filled_form() -> Pdf {
    let mut p = make(
        "BT /F1 12 Tf 10 180 Td (Page text) Tj ET",
        Dictionary::new(),
    );
    let helv = p.doc.add_object(dictionary! {
        "Type" => "Font", "Subtype" => "Type1", "BaseFont" => "Helvetica", "Encoding" => "WinAnsiEncoding",
    });
    let name = widget(&mut p, dictionary! { "FT" => "Tx", "T" => s("name") });
    let fixed = widget(&mut p, dictionary! { "FT" => "Tx", "T" => s("fixed") });
    let street = widget(&mut p, dictionary! { "T" => s("street") });
    let addr = p.doc.add_object(dictionary! {
        "FT" => "Tx", "T" => s("addr"), "V" => s("WALRUS-INHERITED"),
        "DV" => s("WALRUS-INHERITED-DEFAULT"), "Kids" => vec![Object::Reference(street)],
    });
    p.doc
        .get_dictionary_mut(street)
        .unwrap()
        .set("Parent", addr);
    let agree = widget(
        &mut p,
        dictionary! { "FT" => "Btn", "T" => s("agree"), "V" => "Yes", "AS" => "Yes" },
    );
    let opts = vec![
        Object::from(vec![s("red"), s("WALRUS-OPTION-Red")]),
        Object::from(vec![s("blue"), s("Blue")]),
    ];
    let colour = widget(
        &mut p,
        dictionary! { "FT" => "Ch", "Ff" => 1 << 17, "T" => s("colour"), "Opt" => opts },
    );
    let typed = widget(
        &mut p,
        dictionary! { "FT" => "Ch", "Ff" => (1 << 17) | (1 << 18), "T" => s("typed"), "Opt" => vec![s("One")] },
    );
    let sig = widget(
        &mut p,
        dictionary! {
            "FT" => "Sig", "T" => s("signature"),
            "V" => dictionary! { "Type" => "Sig", "Name" => s("WALRUS-SIGNER") },
        },
    );
    let fields: Vec<Object> = [name, fixed, addr, agree, colour, typed, sig]
        .into_iter()
        .map(Object::Reference)
        .collect();
    catalog_set(
        &mut p,
        "AcroForm",
        dictionary! {
            "Fields" => fields, "DA" => s("/Helv 0 Tf 0 g"),
            "DR" => dictionary! { "Font" => dictionary! { "Helv" => helv } },
        }
        .into(),
    );
    p.set_field("name", FieldValue::Text("WALRUS-CURRENT".into()))
        .unwrap();
    p.set_field("fixed", FieldValue::Text("WALRUS-READONLY".into()))
        .unwrap();
    p.set_field("colour", FieldValue::Choice(vec!["red".into()]))
        .unwrap();
    p.set_field("typed", FieldValue::Choice(vec!["WALRUS-TYPED".into()]))
        .unwrap();
    p.refresh_appearances();
    let d = p.doc.get_dictionary_mut(name).unwrap();
    d.set("DV", s("WALRUS-DEFAULT"));
    d.set("RV", s("<body><p>WALRUS-RICH</p></body>"));
    p.doc.get_dictionary_mut(fixed).unwrap().set("Ff", 1);
    // A widget on the page that the form's field list leaves out, showing its value.
    let shown = p.doc.add_object(Stream::new(
        dictionary! {
            "Type" => "XObject", "Subtype" => "Form", "BBox" => rect([0, 0, 140, 20]),
            "Resources" => dictionary! { "Font" => dictionary! { "Helv" => helv } },
        },
        b"BT /Helv 10 Tf 2 6 Td (WALRUS-ORPHAN) Tj ET".to_vec(),
    ));
    widget(
        &mut p,
        dictionary! {
            "FT" => "Tx", "T" => s("orphan"), "V" => s("WALRUS-ORPHAN"),
            "AP" => dictionary! { "N" => shown },
        },
    );
    p
}

const FORM_VALUES: [&str; 7] = [
    "WALRUS-CURRENT",
    "WALRUS-DEFAULT",
    "WALRUS-RICH",
    "WALRUS-READONLY",
    "WALRUS-INHERITED",
    "WALRUS-TYPED",
    "WALRUS-ORPHAN",
];

#[test]
fn form_values_are_cleared_everywhere_they_are_kept() {
    let mut p = filled_form();
    Saved::of(&mut p).assert_kept(&FORM_VALUES);
    let rep = p.remove_hidden(&[Hidden::FormValues]).unwrap();
    assert_eq!(rep.count(Hidden::FormValues), 7, "{rep:?}");
    let saved = Saved::of(&mut p);
    saved.assert_gone(&FORM_VALUES);
    let fields = saved.fields();
    let held: Vec<_> = fields
        .iter()
        .filter(|f| f.name != "signature" && !f.values.is_empty())
        .collect();
    assert!(held.is_empty(), "{held:?}");
    let colour = fields.iter().find(|f| f.name == "colour").unwrap();
    assert!(!colour.appearance.contains("WALRUS"), "{colour:?}");
    // Kept: the signature, the options, every field and the page.
    saved.assert_kept(&["WALRUS-SIGNER", "WALRUS-OPTION-Red", "Page text"]);
    let names: Vec<&str> = fields.iter().map(|f| f.name.as_str()).collect();
    assert_eq!(
        names,
        [
            "name",
            "fixed",
            "addr",
            "addr.street",
            "agree",
            "colour",
            "typed",
            "signature"
        ]
    );
    // Reset form has nothing to bring back.
    let mut q = Pdf::from_bytes(&saved.bytes, None).unwrap();
    assert!(!q.repaired());
    q.reset_form().unwrap();
    for f in q.form_fields() {
        let empty = match &f.value {
            FieldValue::Text(t) => t.is_empty(),
            FieldValue::Check(c) => !c,
            FieldValue::Choice(c) => c.is_empty(),
            FieldValue::None => true,
        };
        assert!(empty, "{f:?}");
    }
}

/// A text widget on the page that the form's field list leaves out, `parents` fields below
/// one holding a value. With `looped`, the top field's parent is the widget's own, so the
/// parents go round in a loop.
fn orphan_below(parents: usize, looped: bool) -> Pdf {
    let mut p = make("BT /F1 12 Tf 10 20 Td (Page text) Tj ET", Dictionary::new());
    let top = p
        .doc
        .add_object(dictionary! { "V" => s("WALRUS-ORPHAN-TOP") });
    let mut parent = top;
    for _ in 1..parents {
        parent = p.doc.add_object(dictionary! { "Parent" => parent });
    }
    if looped {
        p.doc.get_dictionary_mut(top).unwrap().set("Parent", parent);
    }
    widget(&mut p, dictionary! { "FT" => "Tx", "Parent" => parent });
    p
}

/// A text widget `levels` fields below the form's listed top field, which holds a value.
fn listed_below(levels: usize) -> Pdf {
    let mut p = make("BT /F1 12 Tf 10 20 Td (Page text) Tj ET", Dictionary::new());
    let mut kid = widget(&mut p, Dictionary::new());
    for d in (0..levels).rev() {
        let node = p.doc.add_object(dictionary! {
            "T" => s(&format!("level-{d}")), "Kids" => vec![Object::Reference(kid)],
        });
        p.doc.get_dictionary_mut(kid).unwrap().set("Parent", node);
        kid = node;
    }
    let top = p.doc.get_dictionary_mut(kid).unwrap();
    top.set("FT", "Tx");
    top.set("V", s("WALRUS-LISTED-TOP"));
    p.doc.catalog_mut().unwrap().set(
        "AcroForm",
        dictionary! { "Fields" => vec![Object::Reference(kid)] },
    );
    p
}

/// Fields up to 32 deep below the top one, with their widgets one level further, are cleared,
/// as page removal reaches them (PB-05); a field deeper than that is refused with nothing
/// changed.
#[test]
fn form_values_of_fields_nested_32_deep_are_cleared() {
    let mut p = listed_below(33);
    Saved::of(&mut p.clone()).assert_kept(&["WALRUS-LISTED-TOP"]);
    let rep = p.remove_hidden(&[Hidden::FormValues]).unwrap();
    assert_eq!(rep.count(Hidden::FormValues), 1, "{rep:?}");
    Saved::of(&mut p).assert_gone(&["WALRUS"]);

    let mut p = listed_below(34);
    let before = p.clone().snapshot().unwrap();
    let err = format!("{:#}", p.remove_hidden(&[Hidden::FormValues]).unwrap_err());
    assert!(err.contains("nested more than 32 deep"), "{err}");
    assert_eq!(p.clone().snapshot().unwrap(), before);
}

/// An orphan widget's parents more than 32 deep below the top one can't all be followed, so
/// clearing form values refuses with nothing changed rather than leave a value behind (audit
/// R07); up to 32 deep (33 parents, as for a listed field, PB-05), and in a loop, every
/// parent's value goes.
#[test]
fn form_values_above_an_orphan_widget_are_cleared_or_refused() {
    let mut p = orphan_below(34, false);
    let before = p.clone().snapshot().unwrap();
    for err in [
        p.check_remove_hidden(&[Hidden::FormValues]).unwrap_err(),
        p.remove_hidden(&[Hidden::FormValues]).unwrap_err(),
    ] {
        let err = format!("{err:#}");
        assert!(err.contains("nested more than 32 deep"), "{err}");
    }
    assert_eq!(p.clone().snapshot().unwrap(), before);

    for mut p in [orphan_below(33, false), orphan_below(3, true)] {
        Saved::of(&mut p).assert_kept(&["WALRUS-ORPHAN-TOP"]);
        let rep = p.remove_hidden(&[Hidden::FormValues]).unwrap();
        assert_eq!(rep.count(Hidden::FormValues), 1, "{rep:?}");
        let saved = Saved::of(&mut p);
        saved.assert_gone(&["WALRUS"]);
        saved.assert_kept(&["Page text"]);
    }
}

/// Links, a button, the opening action and a bookmark whose ordinary actions are followed,
/// through direct dictionaries, by actions that run something.
fn nested_actions() -> Pdf {
    let mut p = make(
        "BT /F1 12 Tf 10 180 Td (Page text) Tj ET",
        Dictionary::new(),
    );
    let page = p.page_ids()[0];
    let fit = || vec![Object::Reference(page), "Fit".into()];
    let link = p.doc.add_object(dictionary! {
        "Type" => "Annot", "Subtype" => "Link", "Rect" => rect([0, 0, 50, 50]),
        "A" => dictionary! {
            "S" => "URI", "URI" => s("https://example.org/kept"),
            "Next" => dictionary! { "S" => "JavaScript", "JS" => s("app.alert('WALRUS-NEXT')") },
        },
    });
    let chain = p.doc.add_object(dictionary! {
        "Type" => "Annot", "Subtype" => "Link", "Rect" => rect([60, 0, 110, 50]),
        "A" => dictionary! {
            "S" => "GoTo", "D" => fit(),
            "Next" => vec![Object::from(dictionary! {
                "S" => "URI", "URI" => s("https://example.org/also"),
                "Next" => dictionary! { "S" => "Launch", "F" => s("WALRUS-LAUNCH.exe") },
            })],
        },
    });
    add_annot(&mut p, link);
    add_annot(&mut p, chain);
    // An annotation written directly in the page's list, with actions of its own.
    let d = p.doc.get_dictionary_mut(page).unwrap();
    if let Ok(Object::Array(a)) = d.get_mut(b"Annots") {
        a.push(
            dictionary! {
                "Type" => "Annot", "Subtype" => "Square", "Rect" => rect([120, 0, 170, 50]),
                "AA" => dictionary! { "E" => dictionary! { "S" => "JavaScript", "JS" => s("WALRUS-DIRECT-AA") } },
            }
            .into(),
        );
    }
    let item = p.doc.add_object(dictionary! {
        "Title" => s("Chapter"),
        "A" => dictionary! {
            "S" => "GoTo", "D" => fit(),
            "Next" => dictionary! { "S" => "JavaScript", "JS" => s("WALRUS-OUTLINE") },
        },
    });
    let root = p.doc.add_object(
        dictionary! { "Type" => "Outlines", "First" => item, "Last" => item, "Count" => 1 },
    );
    p.doc.get_dictionary_mut(item).unwrap().set("Parent", root);
    catalog_set(&mut p, "Outlines", Object::Reference(root));
    // A push button (poppler lists a widget's additional actions), and the opening action.
    let button = widget(
        &mut p,
        dictionary! {
            "FT" => "Btn", "Ff" => 1 << 16, "T" => s("go"), "Rect" => rect([180, 0, 230, 50]),
            "A" => dictionary! {
                "S" => "URI", "URI" => s("https://example.org/button"),
                "Next" => dictionary! { "S" => "JavaScript", "JS" => s("WALRUS-BUTTON-NEXT") },
            },
            "AA" => dictionary! { "D" => dictionary! { "S" => "JavaScript", "JS" => s("WALRUS-BUTTON-AA") } },
        },
    );
    catalog_set(
        &mut p,
        "AcroForm",
        dictionary! { "Fields" => vec![Object::Reference(button)] }.into(),
    );
    catalog_set(
        &mut p,
        "OpenAction",
        dictionary! {
            "S" => "GoTo", "D" => fit(),
            "Next" => dictionary! { "S" => "JavaScript", "JS" => s("WALRUS-OPEN-NEXT") },
        }
        .into(),
    );
    p
}

#[test]
fn scripts_in_nested_direct_actions_are_removed() {
    let mut p = nested_actions();
    assert!(!Saved::of(&mut p).actions().is_empty());
    let rep = p.remove_hidden(&[Hidden::Scripts]).unwrap();
    assert_eq!(rep.count(Hidden::Scripts), 7, "{rep:?}");
    let saved = Saved::of(&mut p);
    let left = saved.actions();
    assert!(left.is_empty(), "{left:?}");
    saved.assert_gone(&[
        "WALRUS-NEXT",
        "WALRUS-LAUNCH",
        "WALRUS-DIRECT-AA",
        "WALRUS-OUTLINE",
        "WALRUS-BUTTON-NEXT",
        "WALRUS-BUTTON-AA",
        "WALRUS-OPEN-NEXT",
    ]);
    // Kept: the ordinary actions they followed, the bookmark and the opening page.
    saved.assert_kept(&[
        "https://example.org/kept",
        "https://example.org/also",
        "https://example.org/button",
    ]);
    assert!(saved.doc.catalog().unwrap().has(b"OpenAction"));
    let q = Pdf::from_bytes(&saved.bytes, None).unwrap();
    let targets: Vec<_> = q
        .page_links(0)
        .unwrap()
        .into_iter()
        .map(|l| l.target)
        .collect();
    assert_eq!(
        targets,
        [
            limarepdf_core::LinkTarget::Uri("https://example.org/kept".into()),
            limarepdf_core::LinkTarget::Page(0),
        ]
    );
    assert_eq!(q.outline()[0].page, Some(0));
}

/// Add a layer that is switched off and one that is on; returns the one that is off.
fn add_layers(p: &mut Pdf) -> ObjectId {
    let off = p
        .doc
        .add_object(dictionary! { "Type" => "OCG", "Name" => s("Off") });
    let on = p
        .doc
        .add_object(dictionary! { "Type" => "OCG", "Name" => s("On") });
    catalog_set(
        p,
        "OCProperties",
        dictionary! {
            "OCGs" => vec![Object::Reference(off), Object::Reference(on)],
            "D" => dictionary! { "OFF" => vec![Object::Reference(off)] },
        }
        .into(),
    );
    off
}

fn helvetica(p: &mut Pdf) -> ObjectId {
    p.doc.add_object(
        dictionary! { "Type" => "Font", "Subtype" => "Type1", "BaseFont" => "Helvetica" },
    )
}

/// A page drawing a chain of `depth` forms, each drawing the next; the last draws text in a
/// hidden layer and text outside it.
fn nested_layers(depth: usize) -> Pdf {
    let mut p = make(
        "BT /F1 12 Tf 10 20 Td (Page text) Tj ET q /Fm Do Q",
        Dictionary::new(),
    );
    let off = add_layers(&mut p);
    let font = helvetica(&mut p);
    let mut next: Option<ObjectId> = None;
    for _ in 0..depth {
        let mut res = dictionary! {
            "Font" => dictionary! { "F1" => font }, "Properties" => dictionary! { "L1" => off },
        };
        let content = match next {
            None => "/OC /L1 BDC BT /F1 12 Tf 30 150 Td (WALRUS-DEEP) Tj ET EMC \
                     BT /F1 12 Tf 30 120 Td (Visible deep) Tj ET"
                .to_string(),
            Some(n) => {
                res.set("XObject", dictionary! { "Fm" => n });
                "q /Fm Do Q".to_string()
            }
        };
        let form = Stream::new(
            dictionary! {
                "Type" => "XObject", "Subtype" => "Form", "BBox" => rect([0, 0, 300, 200]),
                "Resources" => res,
            },
            content.into_bytes(),
        );
        next = Some(p.doc.add_object(form));
    }
    page_res_set(
        &mut p,
        "XObject",
        dictionary! { "Fm" => next.unwrap() }.into(),
    );
    page_res_set(&mut p, "Properties", dictionary! { "L1" => off }.into());
    p
}

#[test]
fn hidden_layers_in_deeply_nested_forms_are_removed() {
    let mut p = nested_layers(12);
    let rep = p.remove_hidden(&[Hidden::HiddenLayers]).unwrap();
    assert_eq!(rep.count(Hidden::HiddenLayers), 1);
    let saved = Saved::of(&mut p);
    saved.assert_gone(&["WALRUS-DEEP"]);
    saved.assert_kept(&["Visible deep", "Page text"]);
    let users = saved.hidden_layer_users();
    assert!(users.is_empty(), "{users:?}");
}

/// A picture in a hidden layer (its own `/OC`), drawn by a form inside a form and listed only
/// in the inner form's resources (PB-84): it leaves the file, not only the content.
#[test]
fn a_hidden_layer_picture_listed_by_a_nested_form_is_removed() {
    let mut p = make(
        "BT /F1 12 Tf 10 20 Td (Page text) Tj ET q /Fm Do Q",
        Dictionary::new(),
    );
    let off = add_layers(&mut p);
    let samples = b"HiddenPicSecret!".to_vec();
    let pic = p.doc.add_object(Stream::new(
        dictionary! {
            "Type" => "XObject", "Subtype" => "Image", "Width" => samples.len() as i64,
            "Height" => 1, "ColorSpace" => "DeviceGray", "BitsPerComponent" => 8, "OC" => off,
        },
        samples,
    ));
    let form = |res: Dictionary, content: &str| {
        Stream::new(
            dictionary! {
                "Type" => "XObject", "Subtype" => "Form", "BBox" => rect([0, 0, 300, 200]),
                "Resources" => res,
            },
            content.as_bytes().to_vec(),
        )
    };
    let inner = p.doc.add_object(form(
        dictionary! { "XObject" => dictionary! { "Im" => pic } },
        "q 100 0 0 10 30 150 cm /Im Do Q",
    ));
    let outer = p.doc.add_object(form(
        dictionary! { "XObject" => dictionary! { "Fm" => inner } },
        "q /Fm Do Q",
    ));
    page_res_set(&mut p, "XObject", dictionary! { "Fm" => outer }.into());
    let rep = p.remove_hidden(&[Hidden::HiddenLayers]).unwrap();
    assert_eq!(rep.count(Hidden::HiddenLayers), 1);
    let saved = Saved::of(&mut p);
    saved.assert_gone(&["HiddenPicSecret"]);
    saved.assert_kept(&["Page text"]);
}

/// A picture in a hidden layer that a pattern nothing paints with lists too (PB-111): the
/// drawings don't reach the pattern, but the picture leaves its resources and the file.
#[test]
fn a_hidden_layer_picture_listed_by_an_unused_pattern_is_removed() {
    let mut p = make(
        "BT /F1 12 Tf 10 20 Td (Page text) Tj ET q 100 0 0 10 30 150 cm /Im Do Q",
        Dictionary::new(),
    );
    let off = add_layers(&mut p);
    let samples = b"HiddenPicSecret!".to_vec();
    let pic = p.doc.add_object(Stream::new(
        dictionary! {
            "Type" => "XObject", "Subtype" => "Image", "Width" => samples.len() as i64,
            "Height" => 1, "ColorSpace" => "DeviceGray", "BitsPerComponent" => 8, "OC" => off,
        },
        samples,
    ));
    let pattern = p.doc.add_object(Stream::new(
        dictionary! {
            "Type" => "Pattern", "PatternType" => 1, "PaintType" => 1, "TilingType" => 1,
            "BBox" => rect([0, 0, 300, 200]), "XStep" => 300, "YStep" => 200,
            "Resources" => dictionary! { "XObject" => dictionary! { "Im" => pic } },
        },
        b"q 100 0 0 10 30 150 cm /Im Do Q".to_vec(),
    ));
    page_res_set(&mut p, "XObject", dictionary! { "Im" => pic }.into());
    page_res_set(&mut p, "Pattern", dictionary! { "P1" => pattern }.into());
    let rep = p.remove_hidden(&[Hidden::HiddenLayers]).unwrap();
    assert_eq!(rep.count(Hidden::HiddenLayers), 1);
    let saved = Saved::of(&mut p);
    saved.assert_gone(&["HiddenPicSecret"]);
    saved.assert_kept(&["Page text"]);
}

/// A page drawing, only from its content in the hidden layer `/L1`, a form, a picture, a
/// tiling pattern and a soft mask, each with a marker of its own; and a visible form drawing
/// another form only from its content in `/L1` (PB-153). With `shown`, the page draws the
/// form outside the layer too.
fn drawn_from_hidden(shown: bool) -> Pdf {
    let shown = if shown { " q /Fm Do Q" } else { "" };
    let mut p = make(
        &format!(
            "BT /F1 12 Tf 10 20 Td (Page text) Tj ET q /Outer Do Q{shown} \
             /OC /L1 BDC q /Fm Do Q q 100 0 0 10 30 150 cm /Im Do Q \
             /Pattern cs /P1 scn 0 100 100 50 re f /GS1 gs 0 0 50 50 re f EMC"
        ),
        Dictionary::new(),
    );
    let off = add_layers(&mut p);
    let font = helvetica(&mut p);
    let fonts = || dictionary! { "Font" => dictionary! { "F1" => font } };
    let text = |marker: &str| format!("BT /F1 12 Tf 30 150 Td ({marker}) Tj ET");
    let form = |p: &mut Pdf, res: Dictionary, content: String| {
        p.doc.add_object(Stream::new(
            dictionary! {
                "Type" => "XObject", "Subtype" => "Form", "BBox" => rect([0, 0, 300, 200]),
                "Resources" => res,
            },
            content.into_bytes(),
        ))
    };
    let fm = form(&mut p, fonts(), text("WALRUS-FORM"));
    let inner = form(&mut p, fonts(), text("WALRUS-NESTED"));
    let mut res = fonts();
    res.set("XObject", dictionary! { "Inner" => inner });
    res.set("Properties", dictionary! { "L1" => off });
    let content = format!("/OC /L1 BDC q /Inner Do Q EMC {}", text("Visible outer"));
    let outer = form(&mut p, res, content);
    let samples = b"WALRUS-PICTURE".to_vec();
    let im = p.doc.add_object(Stream::new(
        dictionary! {
            "Type" => "XObject", "Subtype" => "Image", "Width" => samples.len() as i64,
            "Height" => 1, "ColorSpace" => "DeviceGray", "BitsPerComponent" => 8,
        },
        samples,
    ));
    let pattern = p.doc.add_object(Stream::new(
        dictionary! {
            "Type" => "Pattern", "PatternType" => 1, "PaintType" => 1, "TilingType" => 1,
            "BBox" => rect([0, 0, 300, 200]), "XStep" => 300, "YStep" => 200,
            "Resources" => fonts(),
        },
        text("WALRUS-PATTERN").into_bytes(),
    ));
    let mask = p.doc.add_object(Stream::new(
        dictionary! {
            "Type" => "XObject", "Subtype" => "Form", "BBox" => rect([0, 0, 300, 200]),
            "Group" => dictionary! { "S" => "Transparency", "CS" => "DeviceGray" },
            "Resources" => fonts(),
        },
        text("WALRUS-MASK").into_bytes(),
    ));
    let xobjects = dictionary! { "Fm" => fm, "Im" => im, "Outer" => outer };
    page_res_set(&mut p, "XObject", xobjects.into());
    page_res_set(&mut p, "Pattern", dictionary! { "P1" => pattern }.into());
    let gs = dictionary! { "SMask" => dictionary! { "S" => "Luminosity", "G" => mask } };
    page_res_set(&mut p, "ExtGState", dictionary! { "GS1" => gs }.into());
    page_res_set(&mut p, "Properties", dictionary! { "L1" => off }.into());
    p
}

/// What only hidden layers draw goes with them (PB-153); what is drawn outside them too
/// stays.
#[test]
fn what_only_hidden_layers_draw_is_removed() {
    for shown in [false, true] {
        let mut p = drawn_from_hidden(shown);
        let rep = p.remove_hidden(&[Hidden::HiddenLayers]).unwrap();
        assert_eq!(rep.count(Hidden::HiddenLayers), 1);
        let saved = Saved::of(&mut p);
        saved.assert_gone(&[
            "WALRUS-NESTED",
            "WALRUS-PICTURE",
            "WALRUS-PATTERN",
            "WALRUS-MASK",
        ]);
        saved.assert_kept(&["Page text", "Visible outer"]);
        match shown {
            true => saved.assert_kept(&["WALRUS-FORM"]),
            false => saved.assert_gone(&["WALRUS-FORM"]),
        }
        assert_eq!(saved.hidden_layer_users(), Vec::<String>::new());
        assert_eq!(saved.unreachable(), []);
    }
}

/// A pattern and a soft mask hidden content sets are what is painted after it with them,
/// so they stay; one it sets and restores before anything is painted goes.
#[test]
fn a_pattern_hidden_content_sets_for_what_follows_is_kept() {
    let mut p = make(
        "BT /F1 12 Tf 10 20 Td (Page text) Tj ET \
         /OC /L1 BDC q /Pattern cs /P2 scn 0 0 9 9 re f Q /Pattern cs /P1 scn /GS1 gs EMC \
         0 100 100 50 re f",
        Dictionary::new(),
    );
    let off = add_layers(&mut p);
    let font = helvetica(&mut p);
    let fonts = || dictionary! { "Font" => dictionary! { "F1" => font } };
    let text = |marker: &str| format!("BT /F1 12 Tf 30 150 Td ({marker}) Tj ET").into_bytes();
    let mut pattern = |marker: &str| {
        p.doc.add_object(Stream::new(
            dictionary! {
                "Type" => "Pattern", "PatternType" => 1, "PaintType" => 1, "TilingType" => 1,
                "BBox" => rect([0, 0, 300, 200]), "XStep" => 300, "YStep" => 200,
                "Resources" => fonts(),
            },
            text(marker),
        ))
    };
    let (shown, hidden) = (pattern("Visible pattern"), pattern("WALRUS-PATTERN"));
    let mask = p.doc.add_object(Stream::new(
        dictionary! {
            "Type" => "XObject", "Subtype" => "Form", "BBox" => rect([0, 0, 300, 200]),
            "Group" => dictionary! { "S" => "Transparency", "CS" => "DeviceGray" },
            "Resources" => fonts(),
        },
        text("Visible mask"),
    ));
    let patterns = dictionary! { "P1" => shown, "P2" => hidden };
    page_res_set(&mut p, "Pattern", patterns.into());
    let gs = dictionary! { "SMask" => dictionary! { "S" => "Luminosity", "G" => mask } };
    page_res_set(&mut p, "ExtGState", dictionary! { "GS1" => gs }.into());
    page_res_set(&mut p, "Properties", dictionary! { "L1" => off }.into());
    let rep = p.remove_hidden(&[Hidden::HiddenLayers]).unwrap();
    assert_eq!(rep.count(Hidden::HiddenLayers), 1);
    let saved = Saved::of(&mut p);
    saved.assert_gone(&["WALRUS-PATTERN"]);
    saved.assert_kept(&["Page text", "Visible pattern", "Visible mask"]);
    assert_eq!(saved.hidden_layer_users(), Vec::<String>::new());
    assert_eq!(saved.unreachable(), []);
}

/// A pattern and a soft mask set before a hidden layer and painted with only inside it go
/// with it, though what sets them is outside (PB-177); painted with after it too, they stay.
#[test]
fn a_pattern_set_for_hidden_content_only_goes_with_it() {
    for shown in [false, true] {
        let after = if shown { "0 100 100 50 re f" } else { "" };
        let mut p = make(
            &format!(
                "BT /F1 12 Tf 10 20 Td (Page text) Tj ET /Pattern cs /P1 scn /GS1 gs \
                 /OC /L1 BDC 0 0 9 9 re f EMC {after}"
            ),
            Dictionary::new(),
        );
        let off = add_layers(&mut p);
        let font = helvetica(&mut p);
        let fonts = || dictionary! { "Font" => dictionary! { "F1" => font } };
        let text = |marker: &str| format!("BT /F1 12 Tf 30 150 Td ({marker}) Tj ET").into_bytes();
        let pattern = p.doc.add_object(Stream::new(
            dictionary! {
                "Type" => "Pattern", "PatternType" => 1, "PaintType" => 1, "TilingType" => 1,
                "BBox" => rect([0, 0, 300, 200]), "XStep" => 300, "YStep" => 200,
                "Resources" => fonts(),
            },
            text("WALRUS-PATTERN"),
        ));
        let mask = p.doc.add_object(Stream::new(
            dictionary! {
                "Type" => "XObject", "Subtype" => "Form", "BBox" => rect([0, 0, 300, 200]),
                "Group" => dictionary! { "S" => "Transparency", "CS" => "DeviceGray" },
                "Resources" => fonts(),
            },
            text("WALRUS-MASK"),
        ));
        page_res_set(&mut p, "Pattern", dictionary! { "P1" => pattern }.into());
        let gs = dictionary! { "SMask" => dictionary! { "S" => "Luminosity", "G" => mask } };
        page_res_set(&mut p, "ExtGState", dictionary! { "GS1" => gs }.into());
        page_res_set(&mut p, "Properties", dictionary! { "L1" => off }.into());
        let rep = p.remove_hidden(&[Hidden::HiddenLayers]).unwrap();
        assert_eq!(rep.count(Hidden::HiddenLayers), 1);
        let saved = Saved::of(&mut p);
        match shown {
            true => saved.assert_kept(&["WALRUS-PATTERN", "WALRUS-MASK"]),
            false => saved.assert_gone(&["WALRUS-PATTERN", "WALRUS-MASK"]),
        }
        saved.assert_kept(&["Page text"]);
        assert_eq!(saved.hidden_layer_users(), Vec::<String>::new());
        assert_eq!(saved.unreachable(), []);
    }
}

/// A form only a hidden layer draws that the tag tree points at too would stay in the file
/// that way: the removal refuses, as redaction does, with nothing changed.
#[test]
fn what_only_hidden_layers_draw_that_the_tag_tree_keeps_is_refused() {
    let mut p = drawn_from_hidden(false);
    let page = p.page_ids()[0];
    let res = p.doc.get_dictionary(page).unwrap().get(b"Resources");
    let xobjects = res.unwrap().as_dict().unwrap().get(b"XObject").unwrap();
    let fm = xobjects.as_dict().unwrap().get(b"Fm").unwrap().clone();
    let figure = dictionary! {
        "Type" => "StructElem", "S" => "Figure", "Pg" => page,
        "K" => dictionary! { "Type" => "OBJR", "Obj" => fm, "Pg" => page },
    };
    let root = p
        .doc
        .add_object(dictionary! { "Type" => "StructTreeRoot", "K" => figure });
    catalog_set(&mut p, "StructTreeRoot", root.into());
    refuses_unchanged(p, "through the tag tree");
}

#[test]
fn hidden_layers_in_appearances_and_patterns_are_removed() {
    let mut p = make(
        "BT /F1 12 Tf 10 20 Td (Page text) Tj ET /Pattern cs /P1 scn 0 100 300 100 re f",
        Dictionary::new(),
    );
    let off = add_layers(&mut p);
    let font = helvetica(&mut p);
    let res = || {
        dictionary! { "Font" => dictionary! { "F1" => font }, "Properties" => dictionary! { "L1" => off } }
    };
    let pattern = p.doc.add_object(Stream::new(
        dictionary! {
            "Type" => "Pattern", "PatternType" => 1, "PaintType" => 1, "TilingType" => 1,
            "BBox" => rect([0, 0, 100, 100]), "XStep" => 100, "YStep" => 100, "Resources" => res(),
        },
        b"/OC /L1 BDC BT /F1 12 Tf 5 50 Td (WALRUS-PATTERN) Tj ET EMC \
          BT /F1 12 Tf 5 20 Td (Visible pattern) Tj ET"
            .to_vec(),
    ));
    page_res_set(&mut p, "Pattern", dictionary! { "P1" => pattern }.into());
    let ap = p.doc.add_object(Stream::new(
        dictionary! {
            "Type" => "XObject", "Subtype" => "Form", "BBox" => rect([0, 0, 100, 50]),
            "Resources" => res(),
        },
        b"/OC /L1 BDC BT /F1 12 Tf 5 30 Td (WALRUS-APPEARANCE) Tj ET EMC \
          BT /F1 12 Tf 5 10 Td (Visible note) Tj ET"
            .to_vec(),
    ));
    let square = p.doc.add_object(dictionary! {
        "Type" => "Annot", "Subtype" => "Square", "Rect" => rect([100, 100, 200, 150]),
        "AP" => dictionary! { "N" => ap },
    });
    add_annot(&mut p, square);
    let rep = p.remove_hidden(&[Hidden::HiddenLayers]).unwrap();
    assert_eq!(rep.count(Hidden::HiddenLayers), 1);
    let saved = Saved::of(&mut p);
    saved.assert_gone(&["WALRUS-PATTERN", "WALRUS-APPEARANCE"]);
    saved.assert_kept(&["Visible pattern", "Visible note", "Page text"]);
    let users = saved.hidden_layer_users();
    assert!(users.is_empty(), "{users:?}");
}

/// Two pages sharing a form with no resources of its own, so its layer `/L` is whichever
/// layer the page it is drawn from names: the hidden one on the pages marked in `hidden`, the
/// one that is on elsewhere. Pages marked in `draws` draw the form; the others only list it.
/// Page 2 also names a hidden layer `/L9` the form doesn't use.
fn shared_unresourced(hidden: [bool; 2], draws: [bool; 2]) -> Pdf {
    let mut p = make("", Dictionary::new());
    p.insert_blank(1, 300.0, 200.0).unwrap();
    let off = add_layers(&mut p);
    let ocgs = p.doc.catalog().unwrap().get(b"OCProperties").unwrap();
    let on = ocgs
        .as_dict()
        .unwrap()
        .get(b"OCGs")
        .unwrap()
        .as_array()
        .unwrap()[1]
        .clone();
    let font = helvetica(&mut p);
    let form = p.doc.add_object(Stream::new(
        dictionary! { "Type" => "XObject", "Subtype" => "Form", "BBox" => rect([0, 0, 300, 200]) },
        b"/OC /L BDC BT /F1 12 Tf 30 150 Td (WALRUS-SHARED) Tj ET EMC \
          BT /F1 12 Tf 30 120 Td (Visible shared) Tj ET"
            .to_vec(),
    ));
    for (i, page) in p.page_ids().into_iter().enumerate() {
        let mut props = dictionary! { "L" => if hidden[i] { off.into() } else { on.clone() } };
        if i == 1 {
            props.set("L9", off);
        }
        let draw = if draws[i] { " q /Fx Do Q" } else { "" };
        let content = format!("BT /F1 12 Tf 10 20 Td (Page {}) Tj ET{draw}", i + 1);
        let s = p
            .doc
            .add_object(Stream::new(Dictionary::new(), content.into_bytes()));
        let d = p.doc.get_dictionary_mut(page).unwrap();
        d.set("Contents", s);
        d.set(
            "Resources",
            dictionary! {
                "Font" => dictionary! { "F1" => font }, "XObject" => dictionary! { "Fx" => form },
                "Properties" => props,
            },
        );
    }
    p
}

#[test]
fn hidden_layers_in_a_drawing_shared_with_other_layer_names_are_refused() {
    let why = "is shared between places that set its layers or fonts differently";
    for (hidden, draws) in [
        ([false, true], [true, true]),
        // The first page only lists the form.
        ([false, true], [false, true]),
        // The other way round.
        ([true, false], [true, true]),
        ([true, false], [true, false]),
    ] {
        refuses_unchanged(shared_unresourced(hidden, draws), why);
    }
}

#[test]
fn hidden_layers_in_a_drawing_shared_with_the_same_layer_names_are_removed() {
    let mut p = shared_unresourced([true, true], [true, true]);
    let rep = p.remove_hidden(&[Hidden::HiddenLayers]).unwrap();
    assert_eq!(rep.count(Hidden::HiddenLayers), 1);
    let saved = Saved::of(&mut p);
    saved.assert_gone(&["WALRUS-SHARED"]);
    saved.assert_kept(&["Visible shared", "Page 1", "Page 2"]);
    let users = saved.hidden_layer_users();
    assert!(users.is_empty(), "{users:?}");
}

/// The document as saved, from a copy (saving moves lopdf's object counter on), with the
/// highest object number the highest in use: a refusal doesn't lower it, and puts back
/// everything else.
fn saved_bytes(p: &Pdf) -> Vec<u8> {
    let mut copy = p.clone();
    copy.doc.max_id = copy.doc.objects.keys().map(|id| id.0).max().unwrap_or(0);
    copy.snapshot().unwrap()
}

/// A removal that can't look at everything, or finds after removing something that it is
/// still kept, refuses with nothing changed, while finding what there is still works.
fn refuses_unchanged(mut p: Pdf, why: &str) {
    p.set_info(&Info {
        title: Some("Walrus title".into()),
        ..Default::default()
    })
    .unwrap();
    let before = saved_bytes(&p);
    let err = p
        .remove_hidden(&[Hidden::Metadata, Hidden::HiddenLayers])
        .unwrap_err();
    assert!(format!("{err:#}").contains(why), "{err:#}");
    assert_eq!(saved_bytes(&p), before);
    let found = p.find_hidden().unwrap();
    assert_eq!(found.count(Hidden::HiddenLayers), 1);
    assert_eq!(found.count(Hidden::Metadata), 1);
    // Without the hidden layers, the rest goes.
    p.remove_hidden(&[Hidden::Metadata]).unwrap();
    assert_eq!(p.info().title, None);
}

/// Forms 40 deep, past the 32 the scan for hidden layers once stopped at: followed to the
/// end, the hidden layer's content removed; with fewer steps than following them takes,
/// refused with nothing changed.
#[test]
fn hidden_layers_in_forms_nested_40_deep_are_removed() {
    use limarepdf_core::{budget_seam, TooComplex};
    let mut p = nested_layers(40);
    let rep = p.remove_hidden(&[Hidden::HiddenLayers]).unwrap();
    assert_eq!(rep.count(Hidden::HiddenLayers), 1);
    let saved = Saved::of(&mut p);
    saved.assert_gone(&["WALRUS-DEEP"]);
    saved.assert_kept(&["Visible deep", "Page text"]);
    let users = saved.hidden_layer_users();
    assert!(users.is_empty(), "{users:?}");
    let mut p = nested_layers(40);
    let before = p.state_before();
    let (got, _) = budget_seam::with_limit(40, || p.remove_hidden(&[Hidden::HiddenLayers]));
    let e = got.unwrap_err();
    let too = e.downcast_ref::<TooComplex>().expect("too complex");
    assert_eq!(
        (too.what, too.limit),
        ("checking the drawings", 40),
        "{e:#}"
    );
    assert_eq!(p.state_since(&before), []);
}

#[test]
fn hidden_layers_with_content_that_cant_be_read_are_refused() {
    let mut p = make(
        "BT /F1 12 Tf 10 20 Td (Page text) Tj ET q /Fm Do Q",
        Dictionary::new(),
    );
    add_layers(&mut p);
    let broken = p.doc.add_object(Stream::new(
        dictionary! {
            "Type" => "XObject", "Subtype" => "Form", "BBox" => rect([0, 0, 300, 200]),
            "Filter" => "FlateDecode",
        },
        b"not compressed data".to_vec(),
    ));
    page_res_set(&mut p, "XObject", dictionary! { "Fm" => broken }.into());
    refuses_unchanged(p, "Part of this document is damaged");
}

/// Drawing compressed with Flate: text in hidden layer `/L1`, a long run of drawing, then
/// visible text. Long enough that one read of the data doesn't give all of it, so a fault
/// near its end can't be read past by chance.
fn long_layered(dict: Dictionary, hidden: &str, tail: &str) -> Stream {
    let mut content = format!("/OC /L1 BDC BT /F1 12 Tf 30 150 Td ({hidden}) Tj ET EMC\n");
    content.push_str(&"q Q\n".repeat(100_000));
    content.push_str(&format!("BT /F1 12 Tf 30 120 Td ({tail}) Tj ET"));
    let mut s = Stream::new(dict, content.into_bytes());
    s.compress().unwrap();
    assert!(s.dict.has(b"Filter"));
    s
}

/// A page whose content is `s`, with the hidden layer `/L1`.
fn layered_page(s: Stream) -> Pdf {
    let mut p = make("", Dictionary::new());
    let off = add_layers(&mut p);
    page_res_set(&mut p, "Properties", dictionary! { "L1" => off }.into());
    let page = p.page_ids()[0];
    let id = p.doc.get_page_contents(page)[0];
    p.doc.objects.insert(id, Object::Stream(s));
    p
}

#[test]
fn hidden_layers_in_data_with_a_wrong_checksum_are_removed_and_the_fault_reported() {
    let mut s = long_layered(Dictionary::new(), "WALRUS-CHECKSUM", "Visible tail");
    let n = s.content.len();
    for b in &mut s.content[n - 4..] {
        *b ^= 0x5a;
    }
    let mut p = layered_page(s);
    let rep = p.remove_hidden(&[Hidden::HiddenLayers]).unwrap();
    assert_eq!(rep.count(Hidden::HiddenLayers), 1);
    let saved = Saved::of(&mut p);
    saved.assert_gone(&["WALRUS-CHECKSUM"]);
    saved.assert_kept(&["Visible tail"]);
    assert_eq!(
        rep.notes(),
        [
            "Some of this document's data had a small fault in it (other PDF readers overlook \
             it too). It was all read, and nothing was skipped."
        ]
    );
    // Data without faults gives no note.
    let mut p = layered_page(long_layered(Dictionary::new(), "WALRUS", "Visible"));
    let rep = p.remove_hidden(&[Hidden::HiddenLayers]).unwrap();
    assert_eq!(rep.count(Hidden::HiddenLayers), 1);
    assert!(rep.notes().is_empty());
}

#[test]
fn hidden_layers_in_data_cut_off_part_way_are_refused() {
    let why = "Part of this document is damaged, so LimarePDF can't check all of it for hidden \
               layers. Nothing was changed. To remove the other kinds of information, untick \
               Hidden layers and try again.";
    let cut = |mut s: Stream| {
        let n = s.content.len();
        s.set_content(s.content[..n / 2].to_vec());
        s
    };
    // In the page's content.
    let s = long_layered(Dictionary::new(), "WALRUS-CUT", "Visible tail");
    refuses_unchanged(layered_page(cut(s)), why);
    // In a form the page draws.
    let mut p = make(
        "BT /F1 12 Tf 10 20 Td (Page text) Tj ET q /Fm Do Q",
        Dictionary::new(),
    );
    let off = add_layers(&mut p);
    let font = helvetica(&mut p);
    let form = long_layered(
        dictionary! {
            "Type" => "XObject", "Subtype" => "Form", "BBox" => rect([0, 0, 300, 200]),
            "Resources" => dictionary! {
                "Font" => dictionary! { "F1" => font }, "Properties" => dictionary! { "L1" => off },
            },
        },
        "WALRUS-CUT",
        "Visible tail",
    );
    let form = p.doc.add_object(cut(form));
    page_res_set(&mut p, "XObject", dictionary! { "Fm" => form }.into());
    refuses_unchanged(p, why);
}

/// A page drawing `content`, with the hidden layer `/L1`.
fn layered(content: &str) -> Pdf {
    let mut p = make(content, Dictionary::new());
    let off = add_layers(&mut p);
    page_res_set(&mut p, "Properties", dictionary! { "L1" => off }.into());
    p
}

/// The first page as LimarePDF draws it, at 72 dpi.
fn drawn(p: &mut Pdf) -> Image {
    let hp = render::load(p.snapshot().unwrap(), None).unwrap();
    Image::from_bitmap(&render::Renderer::new(&hp).render(0, 1.0).unwrap())
}

/// How many pixels of `a` and `b` differ beyond anti-aliasing.
fn changed(a: &Image, b: &Image) -> usize {
    differences(a, b, 8).0.iter().filter(|&&x| x).count()
}

/// The same document with every layer shown.
fn all_shown(p: &Pdf) -> Pdf {
    let mut p = p.clone();
    p.doc.catalog_mut().unwrap().remove(b"OCProperties");
    p
}

/// Content whose hidden layer `/L1` sets what visible content after it is drawn with, each
/// with what the hidden layer draws.
const STATE_IN_HIDDEN_LAYERS: [&str; 13] = [
    // Colour.
    "/OC /L1 BDC 1 0 0 rg 0 0 1 RG 0 140 50 50 re f EMC 80 30 100 100 re f",
    // Position.
    "/OC /L1 BDC 1 0 0 1 40 0 cm 0 1 0 rg 0 150 40 40 re f EMC 20 20 50 50 re f",
    // A clip, set alone and while filling.
    "/OC /L1 BDC 100 0 100 200 re W n 0 1 0 rg 0 0 300 200 re f EMC 0 0 1 rg 0 0 300 100 re f",
    "/OC /L1 BDC 0 1 0 rg 100 0 100 200 re W f EMC 0 0 1 rg 0 0 300 100 re f",
    // A shape begun before the layer and painted in it, with shapes after it: the hidden
    // painting ended it (the delta audit's DA04). And a clip set before the layer.
    "20 20 50 50 re /OC /L1 BDC f EMC 140 20 50 50 re f",
    "0 0 1 rg 20 20 50 50 re 30 30 m 60 60 l /OC /L1 BDC 0 1 0 rg B EMC 140 20 50 50 re f",
    "100 0 100 200 re W /OC /L1 BDC n 0 1 0 rg 0 0 300 200 re f EMC 0 0 1 rg 0 0 300 100 re f",
    // A saved state restored after the layer.
    "/OC /L1 BDC q 1 0 0 rg 0 150 40 40 re f EMC 20 20 50 50 re f Q 100 20 50 50 re f",
    // Text: its font and spacing, then where it ends, along the line.
    "/OC /L1 BDC BT /F1 30 Tf 2 Tc 150 Tz (WALRUS) Tj ET EMC BT 20 50 Td (Shown text) Tj ET",
    "BT /F1 20 Tf 20 100 Td /OC /L1 BDC (WALRUS hidden ) Tj EMC (Shown) Tj ET",
    "BT /F1 20 Tf 3 Tw 20 100 Td /OC /L1 BDC [(WALRUS) -400 ( more)] TJ EMC (Shown) Tj ET",
    "BT /F1 20 Tf 24 TL 20 150 Td /OC /L1 BDC (WALRUS) ' EMC (Shown) Tj ET",
    "BT /F1 20 Tf 24 TL 20 150 Td /OC /L1 BDC 4 1 (WALRUS ok) \" EMC (Shown a b) Tj ET",
];

#[test]
fn removing_hidden_layers_keeps_what_shows() {
    for content in STATE_IN_HIDDEN_LAYERS {
        let mut p = layered(content);
        let before = drawn(&mut p);
        assert!(
            changed(&drawn(&mut all_shown(&p)), &before) > 0,
            "{content}: the hidden layer draws nothing"
        );
        let rep = p.remove_hidden(&[Hidden::HiddenLayers]).unwrap();
        assert_eq!(rep.count(Hidden::HiddenLayers), 1, "{content}");
        assert_eq!(
            changed(&drawn(&mut p), &before),
            0,
            "{content}: what shows changed"
        );
        // With every layer shown, nothing more shows: what the hidden layer drew is gone.
        let shown = drawn(&mut all_shown(&p));
        assert_eq!(
            changed(&shown, &before),
            0,
            "{content}: hidden drawing left"
        );
        assert!(!file_has(&mut p, "WALRUS"), "{content}: hidden text left");
    }
}

#[test]
fn hidden_layers_whose_removal_would_change_what_shows_are_refused() {
    // Hidden text that clips.
    refuses_unchanged(
        layered(
            "BT /F1 20 Tf 7 Tr 20 100 Td /OC /L1 BDC (WALRUS) Tj EMC ET \
             0 0 1 rg 0 0 300 200 re f",
        ),
        "page 1's content has hidden text that clips what is drawn after it",
    );
    // A shape begun in the layer and painted after it.
    refuses_unchanged(
        layered("/OC /L1 BDC 20 20 50 50 re EMC f"),
        "page 1's content has a hidden shape that is finished outside its layer",
    );
    // A form drawing hidden text in the font of where it is used, then text after it on the
    // same line: how far the hidden text reached isn't known.
    let form = |line: &str| {
        let mut p = make("BT /F1 20 Tf ET /Fm Do", Dictionary::new());
        let off = add_layers(&mut p);
        let form = p.doc.add_object(Stream::new(
            dictionary! {
                "Type" => "XObject", "Subtype" => "Form", "BBox" => rect([0, 0, 300, 200]),
                "Resources" => dictionary! { "Properties" => dictionary! { "L1" => off } },
            },
            format!("BT 20 100 Td /OC /L1 BDC (WALRUS) Tj EMC {line} ET").into_bytes(),
        ));
        page_res_set(&mut p, "XObject", dictionary! { "Fm" => form }.into());
        p
    };
    refuses_unchanged(
        form("(Shown) Tj"),
        "has hidden text whose width LimarePDF can't work out, and visible text on the same \
         line comes after it",
    );
    // Text after it on a line of its own is where it was.
    let mut p = form("0 -30 Td (Shown) Tj");
    let before = drawn(&mut p);
    p.remove_hidden(&[Hidden::HiddenLayers]).unwrap();
    assert_eq!(changed(&drawn(&mut p), &before), 0);
    assert!(!file_has(&mut p, "WALRUS"));
}

/// Poppler draws what shows the same before and after hidden layers are removed:
/// `cargo test -p limarepdf-core --test sanitise -- --ignored cross_check`.
#[test]
#[ignore]
fn cross_check_what_shows_after_removing_hidden_layers_with_poppler() {
    for content in STATE_IN_HIDDEN_LAYERS {
        let mut p = layered(content);
        let before = tools::pdftoppm(&p.to_bytes().unwrap());
        p.remove_hidden(&[Hidden::HiddenLayers]).unwrap();
        let after = tools::pdftoppm(&p.to_bytes().unwrap());
        assert_eq!(changed(&after, &before), 0, "{content}: what shows changed");
        let shown = tools::pdftoppm(&all_shown(&p).to_bytes().unwrap());
        assert_eq!(
            changed(&shown, &before),
            0,
            "{content}: hidden drawing left"
        );
    }
}

/// Poppler and Ghostscript, reading the output on their own:
/// `cargo test -p limarepdf-core --test sanitise -- --ignored cross_check`.
#[test]
#[ignore]
fn cross_check_removed_information_with_poppler_and_ghostscript() {
    // Form values: Ghostscript draws the fields.
    let mut p = filled_form();
    let shown = tools::gs_text(&p.to_bytes().unwrap());
    assert!(shown.contains("WALRUS-CURRENT"), "{shown}");
    p.remove_hidden(&[Hidden::FormValues]).unwrap();
    let shown = tools::gs_text(&p.to_bytes().unwrap());
    for v in FORM_VALUES {
        assert!(!shown.contains(v), "{v} in {shown}");
    }
    assert!(shown.contains("Page text"), "{shown}");

    // Scripts: poppler lists only a widget's additional actions (not actions that follow
    // others, links or the opening action), so the saved-output checker's search is the
    // evidence for those.
    let mut p = nested_actions();
    let js = tools::pdfinfo_js(&p.to_bytes().unwrap());
    assert!(js.contains("WALRUS-BUTTON-AA"), "{js}");
    p.remove_hidden(&[Hidden::Scripts]).unwrap();
    let js = tools::pdfinfo_js(&p.to_bytes().unwrap());
    assert!(!js.contains("WALRUS"), "{js}");

    // Hidden layers: neither reader finds the text, nested or in a drawing two pages share.
    let mut p = nested_layers(12);
    p.remove_hidden(&[Hidden::HiddenLayers]).unwrap();
    let bytes = p.to_bytes().unwrap();
    for text in [tools::pdftotext(&bytes), tools::gs_text(&bytes)] {
        assert!(!text.contains("WALRUS"), "{text}");
        assert!(text.contains("Visible deep"), "{text}");
    }
    let mut p = shared_unresourced([true, true], [true, true]);
    p.remove_hidden(&[Hidden::HiddenLayers]).unwrap();
    let bytes = p.to_bytes().unwrap();
    for text in [tools::pdftotext(&bytes), tools::gs_text(&bytes)] {
        assert!(!text.contains("WALRUS"), "{text}");
        assert_eq!(text.matches("Visible shared").count(), 2, "{text}");
    }
}

/// S1-02 (H-03): actions whose next actions are a list stored as an object of its own,
/// `/Next 12 0 R`: on the opening action, a link (the list holding an ordinary action whose
/// own next actions are another stored list), a bookmark (a stored list of actions written
/// in place) and a button, whose two actions name each other through stored lists.
fn stored_next_lists() -> Pdf {
    let mut p = make(
        "BT /F1 12 Tf 10 180 Td (Page text) Tj ET",
        Dictionary::new(),
    );
    let page = p.page_ids()[0];
    let fit = || vec![Object::Reference(page), "Fit".into()];
    let js = |p: &mut Pdf, what: &str| {
        p.doc.add_object(
            dictionary! { "S" => "JavaScript", "JS" => s(&format!("app.alert('{what}')")) },
        )
    };
    let list = |p: &mut Pdf, items: Vec<Object>| p.doc.add_object(Object::Array(items));
    // The opening action.
    let open = js(&mut p, "WALRUS-OPEN");
    let opened = list(&mut p, vec![open.into()]);
    catalog_set(
        &mut p,
        "OpenAction",
        dictionary! { "S" => "GoTo", "D" => fit(), "Next" => opened }.into(),
    );
    // A link: its list holds a web address, followed by a stored list of its own, and a
    // script.
    let launch = list(
        &mut p,
        vec![dictionary! { "S" => "Launch", "F" => s("WALRUS-LAUNCH.exe") }.into()],
    );
    let listed = p.doc.add_object(dictionary! {
        "S" => "URI", "URI" => s("https://example.org/listed"), "Next" => launch,
    });
    let link_js = js(&mut p, "WALRUS-LINK");
    let next = list(&mut p, vec![listed.into(), link_js.into()]);
    let link = p.doc.add_object(dictionary! {
        "Type" => "Annot", "Subtype" => "Link", "Rect" => rect([0, 0, 50, 50]),
        "A" => dictionary! { "S" => "URI", "URI" => s("https://example.org/link"), "Next" => next },
    });
    add_annot(&mut p, link);
    // A bookmark: a stored list of actions written in place.
    let in_place = list(
        &mut p,
        vec![
            dictionary! { "S" => "JavaScript", "JS" => s("WALRUS-OUTLINE") }.into(),
            dictionary! { "S" => "URI", "URI" => s("https://example.org/outline") }.into(),
        ],
    );
    let item = p.doc.add_object(dictionary! {
        "Title" => s("Chapter"),
        "A" => dictionary! { "S" => "GoTo", "D" => fit(), "Next" => in_place },
    });
    let root = p.doc.add_object(
        dictionary! { "Type" => "Outlines", "First" => item, "Last" => item, "Count" => 1 },
    );
    p.doc.get_dictionary_mut(item).unwrap().set("Parent", root);
    catalog_set(&mut p, "Outlines", Object::Reference(root));
    // A button: action a's next is [b], b's is [a, a script].
    let (a, b) = (p.doc.new_object_id(), p.doc.new_object_id());
    let after_a = list(&mut p, vec![b.into()]);
    let button_js = js(&mut p, "WALRUS-BUTTON");
    let after_b = list(&mut p, vec![a.into(), button_js.into()]);
    p.doc.objects.insert(
        a,
        dictionary! { "S" => "URI", "URI" => s("https://example.org/a"), "Next" => after_a }.into(),
    );
    p.doc.objects.insert(
        b,
        dictionary! { "S" => "URI", "URI" => s("https://example.org/b"), "Next" => after_b }.into(),
    );
    let button = widget(
        &mut p,
        dictionary! {
            "FT" => "Btn", "Ff" => 1 << 16, "T" => s("go"), "Rect" => rect([180, 0, 230, 50]),
            "A" => a,
        },
    );
    catalog_set(
        &mut p,
        "AcroForm",
        dictionary! { "Fields" => vec![Object::Reference(button)] }.into(),
    );
    common::seeds::pdf(&p);
    p
}

#[test]
fn scripts_in_stored_lists_of_next_actions_are_removed() {
    let mut p = stored_next_lists();
    assert!(!Saved::of(&mut p).actions().is_empty());
    // One for each script in a list: the opening action's, two of the link's (one in the
    // list its web address leads to), the bookmark's and the button's.
    let found = p.find_hidden().unwrap();
    assert_eq!(found.count(Hidden::Scripts), 5, "{found:?}");
    let rep = p.remove_hidden(&[Hidden::Scripts]).unwrap();
    assert_eq!(rep.count(Hidden::Scripts), 5, "{rep:?}");
    let saved = Saved::of(&mut p);
    let left = saved.actions();
    assert!(left.is_empty(), "{left:?}");
    saved.assert_gone(&[
        "WALRUS-OPEN",
        "WALRUS-LINK",
        "WALRUS-LAUNCH",
        "WALRUS-OUTLINE",
        "WALRUS-BUTTON",
    ]);
    // Kept: the ordinary actions, also those that come before a removed one.
    saved.assert_kept(&[
        "https://example.org/link",
        "https://example.org/listed",
        "https://example.org/outline",
        "https://example.org/a",
        "https://example.org/b",
    ]);
    assert!(saved.doc.catalog().unwrap().has(b"OpenAction"));
    let q = Pdf::from_bytes(&saved.bytes, None).unwrap();
    assert_eq!(q.outline()[0].page, Some(0));
}

/// Two layers in the layer list, `LAYER-A`, shown, and `LAYER-B`, switched off when
/// `b_off`; as references.
fn two_layers(p: &mut Pdf, b_off: bool) -> (Object, Object) {
    let a = p
        .doc
        .add_object(dictionary! { "Type" => "OCG", "Name" => s("LAYER-A") });
    let b = p
        .doc
        .add_object(dictionary! { "Type" => "OCG", "Name" => s("LAYER-B") });
    let off: Vec<Object> = if b_off { vec![b.into()] } else { vec![] };
    catalog_set(
        p,
        "OCProperties",
        dictionary! {
            "OCGs" => vec![Object::Reference(a), Object::Reference(b)],
            "D" => dictionary! { "OFF" => off, "Order" => vec![Object::Reference(a), Object::Reference(b)] },
        }
        .into(),
    );
    (a.into(), b.into())
}

/// Text in the content of the layer rule `/R`, and text outside it.
const RULED: &str = "/OC /R BDC BT /F1 12 Tf 30 150 Td (RULE-SECRET) Tj ET EMC \
                     BT /F1 12 Tf 30 100 Td (Visible text) Tj ET";

/// A page drawing [`RULED`], its rule `/R` made by `rule` from [`two_layers`]' layers (a
/// layer rule is an optional content membership dictionary, `/Type /OCMD`); and the rule.
fn ruled(
    b_off: bool,
    rule: impl FnOnce(&mut Pdf, Object, Object) -> Dictionary,
) -> (Pdf, ObjectId) {
    let mut p = make(RULED, Dictionary::new());
    let (a, b) = two_layers(&mut p, b_off);
    let mut d = rule(&mut p, a, b);
    d.set("Type", "OCMD");
    let r = p.doc.add_object(d);
    page_res_set(&mut p, "Properties", dictionary! { "R" => r }.into());
    common::seeds::pdf(&p);
    (p, r)
}

/// A visibility expression: `op` (`And`, `Or` or `Not`) over `args`.
fn ve(op: &str, args: Vec<Object>) -> Object {
    let mut a = vec![Object::Name(op.as_bytes().to_vec())];
    a.extend(args);
    Object::Array(a)
}

/// A layer rule made from the layers a and b.
type Rule = fn(&mut Pdf, Object, Object) -> Dictionary;

/// Rules hiding their content with LAYER-B switched off. `/VE` comes first (ISO 32000-1,
/// 8.11.2.2), also over `/OCGs` and `/P` that would show it.
const HIDING_WITH_B_OFF: [(&str, Rule); 6] = [
    ("and", |_, a, b| {
        dictionary! {
            "OCGs" => vec![a.clone(), b.clone()], "P" => "AnyOn", "VE" => ve("And", vec![a, b]),
        }
    }),
    ("or", |_, a, b| {
        dictionary! { "VE" => ve("Or", vec![b, ve("Not", vec![a])]) }
    }),
    ("not", |_, a, b| {
        dictionary! { "VE" => ve("Not", vec![ve("Or", vec![a, b])]) }
    }),
    ("nested", |_, a, b| {
        let inner = ve("And", vec![a.clone(), ve("Not", vec![a.clone()])]);
        dictionary! { "VE" => ve("And", vec![a, ve("Or", vec![b, inner])]) }
    }),
    ("a stored sub-expression", |p, a, b| {
        let e = p.doc.add_object(ve("And", vec![a.clone(), b]));
        dictionary! { "VE" => ve("Or", vec![e.into(), ve("Not", vec![a])]) }
    }),
    ("a stored expression", |p, a, b| {
        let e = p.doc.add_object(ve("And", vec![a, b]));
        dictionary! { "VE" => e }
    }),
];

#[test]
fn content_a_layer_rule_hides_is_removed() {
    for (case, rule) in HIDING_WITH_B_OFF {
        let (mut p, _) = ruled(true, rule);
        let found = p.find_hidden().unwrap();
        assert_eq!(found.count(Hidden::HiddenLayers), 1, "{case}");
        let rep = p.remove_hidden(&[Hidden::HiddenLayers]).unwrap();
        assert_eq!(rep.count(Hidden::HiddenLayers), 1, "{case}");
        let saved = Saved::of(&mut p);
        let left = saved.places("RULE-SECRET");
        assert!(
            left.is_empty(),
            "{case}: the hidden content is left in {left:?}"
        );
        let gone = saved.places("LAYER-B");
        assert!(
            gone.is_empty(),
            "{case}: the removed layer is still named in {gone:?}"
        );
        saved.assert_kept(&["Visible text", "LAYER-A"]);
    }
}

#[test]
fn content_a_layer_rule_hides_on_a_page_and_in_a_form_is_removed_from_both() {
    let (mut p, r) = ruled(true, HIDING_WITH_B_OFF[0].1);
    let font = helvetica(&mut p);
    let form = p.doc.add_object(Stream::new(
        dictionary! {
            "Type" => "XObject", "Subtype" => "Form", "BBox" => rect([0, 0, 300, 200]),
            "Resources" => dictionary! {
                "Font" => dictionary! { "F1" => font }, "Properties" => dictionary! { "R" => r },
            },
        },
        b"/OC /R BDC BT /F1 12 Tf 30 60 Td (FORM-SECRET) Tj ET EMC \
          BT /F1 12 Tf 30 40 Td (Form text) Tj ET"
            .to_vec(),
    ));
    page_res_set(&mut p, "XObject", dictionary! { "Fm" => form }.into());
    let page = p.page_ids()[0];
    let id = p.doc.get_page_contents(page)[0];
    let content = format!("{RULED} q /Fm Do Q");
    p.doc.objects.insert(
        id,
        Stream::new(Dictionary::new(), content.into_bytes()).into(),
    );
    let rep = p.remove_hidden(&[Hidden::HiddenLayers]).unwrap();
    assert_eq!(rep.count(Hidden::HiddenLayers), 1);
    let saved = Saved::of(&mut p);
    saved.assert_gone(&["RULE-SECRET", "FORM-SECRET", "LAYER-B"]);
    saved.assert_kept(&["Visible text", "Form text"]);
}

/// `/VE` shows the content though `/OCGs` and `/P` alone would hide it: it is kept, and the
/// rule, which named the removed layer, now names only the layer that is left.
fn shown_by_ve_over_p() -> (Pdf, ObjectId) {
    ruled(true, |_, a, b| {
        dictionary! { "OCGs" => vec![b.clone()], "P" => "AllOn", "VE" => ve("Or", vec![a, b]) }
    })
}

#[test]
fn content_a_layer_rule_shows_by_its_expression_is_kept() {
    let (mut p, r) = shown_by_ve_over_p();
    let a = p
        .doc
        .get_dictionary(r)
        .unwrap()
        .get(b"VE")
        .unwrap()
        .as_array()
        .unwrap()[1]
        .clone();
    let rep = p.remove_hidden(&[Hidden::HiddenLayers]).unwrap();
    assert_eq!(rep.count(Hidden::HiddenLayers), 1);
    let rule = p.doc.get_dictionary(r).unwrap();
    assert_eq!(rule.get(b"OCGs").ok(), Some(&a), "{rule:?}");
    assert!(!rule.has(b"VE") && !rule.has(b"P"), "{rule:?}");
    let saved = Saved::of(&mut p);
    saved.assert_kept(&["RULE-SECRET", "Visible text", "LAYER-A"]);
    saved.assert_gone(&["LAYER-B"]);
}

/// A removal that can't read a layer rule something uses refuses before changing anything,
/// saying `why`, while finding counts the rule as one found (so the box can be ticked, and
/// Remove says why it stops) and the other kinds are still removed.
fn refuses_rule(mut p: Pdf, why: &str) {
    p.set_info(&Info {
        title: Some("Walrus title".into()),
        ..Default::default()
    })
    .unwrap();
    let before = p.state_before();
    let err = p.check_remove_hidden(&[Hidden::HiddenLayers]).unwrap_err();
    assert!(format!("{err:#}").contains(why), "{err:#}");
    let err = p
        .remove_hidden(&[Hidden::Metadata, Hidden::HiddenLayers])
        .unwrap_err();
    assert!(format!("{err:#}").contains(why), "{err:#}");
    assert_eq!(p.state_since(&before), []);
    let found = p.find_hidden().unwrap();
    assert_eq!(found.count(Hidden::HiddenLayers), 1, "{why}");
    assert_eq!(found.count(Hidden::Metadata), 1);
    p.remove_hidden(&[Hidden::Metadata]).unwrap();
    assert_eq!(p.info().title, None);
}

/// An expression made from the layers a and b.
type Expr = fn(&mut Pdf, Object, Object) -> Object;

/// A rule whose `/VE` is `expr` of the two layers, both shown (so no layer is off).
fn ve_rule(expr: Expr) -> (Pdf, ObjectId) {
    ruled(false, |p, a, b| {
        let e = expr(p, a, b);
        dictionary! { "VE" => e }
    })
}

/// `/VE` of `n` levels: `n - 1` of `/Not` around `[/Or b]`.
fn deep(b: Object, n: usize) -> Object {
    (1..n).fold(ve("Or", vec![b]), |e, _| ve("Not", vec![e]))
}

#[test]
fn layer_rules_that_cant_be_read_are_refused() {
    let cant = |r: ObjectId, why: &str| {
        format!(
            "hidden layers can't be removed for certain: a layer rule (object {}) can't be read \
             ({why}). Nothing was changed; leave out hidden layers to remove the rest",
            r.0
        )
    };
    let cases: [(Expr, &str); 7] = [
        (
            |_, a, b| ve("Xor", vec![a, b]),
            "it combines layers with /Xor, which isn't a way to combine them",
        ),
        (
            |_, a, b| ve("Not", vec![a, b]),
            "a /Not in it doesn't have exactly one operand",
        ),
        (
            |_, _, _| ve("And", vec![]),
            "an /And or /Or in it has nothing to combine",
        ),
        (
            |_, a, _| ve("And", vec![a, Object::Integer(5)]),
            "something in it is neither a layer nor an expression",
        ),
        (
            |p, a, _| {
                let e = p.doc.new_object_id();
                p.doc.objects.insert(e, ve("Or", vec![a, e.into()]));
                e.into()
            },
            "it contains itself",
        ),
        (|_, _, b| deep(b, 33), "it is nested more than 32 deep"),
        (|_, a, _| a, "its /VE isn't an expression"),
    ];
    for (expr, why) in cases {
        let (p, r) = ve_rule(expr);
        refuses_rule(p, &cant(r, why));
    }
    // An expression 32 deep is read: 31 of /Not around b, which is shown, hide.
    let (mut p, _) = ve_rule(|_, _, b| deep(b, 32));
    let rep = p.remove_hidden(&[Hidden::HiddenLayers]).unwrap();
    assert_eq!(rep.count(Hidden::HiddenLayers), 1);
    Saved::of(&mut p).assert_gone(&["RULE-SECRET"]);
    // A rule written into the content itself.
    let mut p = make("", Dictionary::new());
    let (a, _) = two_layers(&mut p, false);
    let page = p.page_ids()[0];
    let id = p.doc.get_page_contents(page)[0];
    let content = format!(
        "/OC << /Type /OCMD /VE [/Xor {} 0 R] >> BDC BT /F1 12 Tf 30 150 Td (RULE-SECRET) Tj \
         ET EMC",
        a.as_reference().unwrap().0
    );
    p.doc.objects.insert(
        id,
        Stream::new(Dictionary::new(), content.into_bytes()).into(),
    );
    refuses_rule(
        p,
        "page 1's content has a layer rule that can't be read (it combines layers with /Xor, \
         which isn't a way to combine them)",
    );
}

/// An expression is worked out once and remembered, but how deep it reaches is checked for
/// each place it is used: here 20 deep, used at the top by one rule and 15 deep by another.
#[test]
fn a_stored_expression_used_deeper_by_another_rule_is_refused() {
    for first in ["R1", "R2"] {
        let mut p = make(
            "/OC /R1 BDC BT /F1 12 Tf 30 150 Td (RULE-ONE) Tj ET EMC \
             /OC /R2 BDC BT /F1 12 Tf 30 120 Td (RULE-TWO) Tj ET EMC",
            Dictionary::new(),
        );
        let (_, b) = two_layers(&mut p, false);
        let e = p
            .doc
            .add_object((1..20).fold(ve("Not", vec![b]), |e, _| ve("Not", vec![e])));
        let r1 = p
            .doc
            .add_object(dictionary! { "Type" => "OCMD", "VE" => e });
        let deeper = (1..14).fold(ve("Not", vec![e.into()]), |x, _| ve("Not", vec![x]));
        let r2 = p
            .doc
            .add_object(dictionary! { "Type" => "OCMD", "VE" => deeper });
        let props = match first {
            "R1" => dictionary! { "R1" => r1, "R2" => r2 },
            _ => dictionary! { "R2" => r2, "R1" => r1 },
        };
        page_res_set(&mut p, "Properties", props.into());
        let why = format!(
            "a layer rule (object {}) can't be read (it is nested more than 32 deep)",
            r2.0
        );
        refuses_rule(p, &why);
    }
}

/// A rule of `n` layer names, `/VE [/Or b b …]`, which shows its content: `n + 1` steps
/// (each operator and each operand is one).
fn wide_rule(n: usize) -> Pdf {
    ruled(
        false,
        |_, _, b| dictionary! { "VE" => ve("Or", vec![b; n]) },
    )
    .0
}

#[test]
fn checking_layer_rules_has_a_step_limit() {
    use limarepdf_core::{budget_seam, TooComplex};
    let p = wide_rule(2000);
    let (got, taken) =
        budget_seam::with_limit(2001, || p.check_remove_hidden(&[Hidden::HiddenLayers]));
    got.unwrap();
    // The rules' steps; following the drawings has a budget of its own.
    assert!(taken.contains(&2001), "{taken:?}");
    let mut p = wide_rule(2000);
    let before = p.state_before();
    let (got, _) = budget_seam::with_limit(2000, || p.remove_hidden(&[Hidden::HiddenLayers]));
    let e = got.unwrap_err();
    let too = e.downcast_ref::<TooComplex>().expect("too complex");
    assert_eq!(
        (too.what, too.limit),
        ("checking the layers", 2000),
        "{e:#}"
    );
    assert_eq!(p.state_since(&before), []);
    // A stored expression two rules share is checked once.
    let mut p = make(
        "/OC /R1 BDC (one) Tj EMC /OC /R2 BDC (two) Tj EMC",
        Dictionary::new(),
    );
    let (_, b) = two_layers(&mut p, false);
    let e = p.doc.add_object(ve("Or", vec![b; 2000]));
    let rules: Vec<ObjectId> = (0..2)
        .map(|_| {
            p.doc
                .add_object(dictionary! { "Type" => "OCMD", "VE" => e })
        })
        .collect();
    page_res_set(
        &mut p,
        "Properties",
        dictionary! { "R1" => rules[0], "R2" => rules[1] }.into(),
    );
    let (got, taken) =
        budget_seam::with_limit(u64::MAX, || p.check_remove_hidden(&[Hidden::HiddenLayers]));
    got.unwrap();
    assert!(taken[0] < 2 * 2001, "{taken:?}");
}

/// Rules hiding their content with every layer shown (V-03).
const HIDING_ALL_ON: [(&str, Rule); 4] = [
    (
        "not a",
        |_, a, _| dictionary! { "VE" => ve("Not", vec![a]) },
    ),
    (
        "any off",
        |_, a, b| dictionary! { "OCGs" => vec![a, b], "P" => "AnyOff" },
    ),
    (
        "all off",
        |_, a, b| dictionary! { "OCGs" => vec![a, b], "P" => "AllOff" },
    ),
    ("nested", |_, a, b| {
        let either = ve("Or", vec![ve("Not", vec![a.clone()]), ve("Not", vec![b])]);
        dictionary! { "VE" => ve("And", vec![a, either]) }
    }),
];

#[test]
fn content_a_layer_rule_hides_with_every_layer_shown_is_removed() {
    for (case, rule) in HIDING_ALL_ON {
        let (mut p, _) = ruled(false, rule);
        let found = p.find_hidden().unwrap();
        assert_eq!(found.count(Hidden::HiddenLayers), 1, "{case}");
        let rep = p.remove_hidden(&[Hidden::HiddenLayers]).unwrap();
        assert_eq!(rep.count(Hidden::HiddenLayers), 1, "{case}");
        let saved = Saved::of(&mut p);
        let left = saved.places("RULE-SECRET");
        assert!(
            left.is_empty(),
            "{case}: the hidden content is left in {left:?}"
        );
        saved.assert_kept(&["Visible text", "LAYER-A", "LAYER-B"]);
    }
    // A rule written into the content itself.
    let mut p = make("", Dictionary::new());
    let (a, _) = two_layers(&mut p, false);
    let page = p.page_ids()[0];
    let id = p.doc.get_page_contents(page)[0];
    let content = format!(
        "/OC << /Type /OCMD /VE [/Not {} 0 R] >> BDC BT /F1 12 Tf 30 150 Td (RULE-SECRET) Tj \
         ET EMC BT /F1 12 Tf 30 100 Td (Visible text) Tj ET",
        a.as_reference().unwrap().0
    );
    p.doc.objects.insert(
        id,
        Stream::new(Dictionary::new(), content.into_bytes()).into(),
    );
    assert_eq!(p.find_hidden().unwrap().count(Hidden::HiddenLayers), 1);
    let rep = p.remove_hidden(&[Hidden::HiddenLayers]).unwrap();
    assert_eq!(rep.count(Hidden::HiddenLayers), 1);
    let saved = Saved::of(&mut p);
    saved.assert_gone(&["RULE-SECRET"]);
    saved.assert_kept(&["Visible text"]);
}

/// The control: rules that show their content with every layer shown, among them the
/// brief's nested example. Nothing is found and nothing changes.
#[test]
fn layer_rules_showing_their_content_with_every_layer_shown_change_nothing() {
    let shown: [Rule; 3] = [
        |_, a, b| {
            let both = ve("And", vec![a.clone(), b]);
            dictionary! { "VE" => ve("Or", vec![ve("Not", vec![a]), both]) }
        },
        |_, a, b| dictionary! { "VE" => ve("Or", vec![a, b]) },
        |_, a, b| dictionary! { "OCGs" => vec![a, b], "P" => "AllOn" },
    ];
    for rule in shown {
        let (mut p, _) = ruled(false, rule);
        assert_eq!(p.find_hidden().unwrap().count(Hidden::HiddenLayers), 0);
        let before = p.state_before();
        let rep = p.remove_hidden(&[Hidden::HiddenLayers]).unwrap();
        assert_eq!(rep.count(Hidden::HiddenLayers), 0);
        assert_eq!(p.state_since(&before), []);
    }
}

/// Q4 (a): with every layer shown and every rule shown, a damaged drawing doesn't stop the
/// removal (there is nothing to remove); with a rule that hides, it does.
#[test]
fn a_damaged_drawing_refuses_only_when_a_layer_rule_hides() {
    let damaged = |hides: bool| {
        let content = "BT /F1 12 Tf 10 20 Td (Page text) Tj ET q /Fm Do Q";
        let (mut p, r) = ruled(
            false,
            |_, a, b| dictionary! { "VE" => ve("Or", vec![a, b]) },
        );
        let page = p.page_ids()[0];
        let id = p.doc.get_page_contents(page)[0];
        let content = match hides {
            true => format!("{RULED} {content} /OC /H BDC (WALRUS) Tj EMC"),
            false => format!("{RULED} {content}"),
        };
        p.doc.objects.insert(
            id,
            Stream::new(Dictionary::new(), content.into_bytes()).into(),
        );
        let broken = p.doc.add_object(Stream::new(
            dictionary! {
                "Type" => "XObject", "Subtype" => "Form", "BBox" => rect([0, 0, 300, 200]),
                "Filter" => "FlateDecode",
            },
            b"not compressed data".to_vec(),
        ));
        page_res_set(&mut p, "XObject", dictionary! { "Fm" => broken }.into());
        let mut props = dictionary! { "R" => r };
        if hides {
            let a = p
                .doc
                .get_dictionary(r)
                .unwrap()
                .get(b"VE")
                .unwrap()
                .as_array()
                .unwrap()[1]
                .clone();
            let h = p
                .doc
                .add_object(dictionary! { "Type" => "OCMD", "VE" => ve("Not", vec![a]) });
            props.set("H", h);
        }
        page_res_set(&mut p, "Properties", props.into());
        p
    };
    let mut p = damaged(false);
    let before = p.state_before();
    assert_eq!(p.find_hidden().unwrap().count(Hidden::HiddenLayers), 0);
    let rep = p.remove_hidden(&[Hidden::HiddenLayers]).unwrap();
    assert_eq!(rep.count(Hidden::HiddenLayers), 0);
    assert_eq!(p.state_since(&before), []);

    let mut p = damaged(true);
    let before = p.state_before();
    let e = p.remove_hidden(&[Hidden::HiddenLayers]).unwrap_err();
    assert!(e.is::<limarepdf_core::LayersDamaged>(), "{e:#}");
    assert_eq!(p.state_since(&before), []);
}

/// The rule `r` in `p`.
fn rule_of(p: &Pdf, r: ObjectId) -> Dictionary {
    p.doc.get_dictionary(r).unwrap().clone()
}

/// LAYER-A, as a reference: the first in the layer list [`two_layers`] made.
fn layer_a(p: &Pdf) -> Object {
    let props = p.doc.catalog().unwrap().get(b"OCProperties").unwrap();
    let ocgs = props.as_dict().unwrap().get(b"OCGs").unwrap();
    ocgs.as_array().unwrap()[0].clone()
}

/// Kept rules that named a removed layer are worked out again with it off for good, so the
/// layers that stay switch the content as before, and no removed layer is named anywhere.
#[test]
fn kept_layer_rules_that_named_a_removed_layer_are_rewritten() {
    // `/VE`: the one layer left.
    let (mut p, r) = ruled(true, |_, a, b| dictionary! { "VE" => ve("Or", vec![b, a]) });
    let a = layer_a(&p);
    let rep = p.remove_hidden(&[Hidden::HiddenLayers]).unwrap();
    assert_eq!(rep.count(Hidden::HiddenLayers), 1);
    let rule = rule_of(&p, r);
    assert_eq!(rule.get(b"OCGs").ok(), Some(&a), "{rule:?}");
    assert!(!rule.has(b"VE") && !rule.has(b"P"), "{rule:?}");
    let saved = Saved::of(&mut p);
    saved.assert_kept(&["RULE-SECRET", "LAYER-A"]);
    saved.assert_gone(&["LAYER-B"]);

    // `/P`: each policy, the rule shown with LAYER-B off, and whether what it becomes is
    // right, given LAYER-A.
    type Becomes = fn(&Dictionary, &Object) -> bool;
    let cases: [(&str, Rule, Becomes); 4] = [
        (
            "any on",
            |_, a, b| dictionary! { "OCGs" => vec![a, b], "P" => "AnyOn" },
            |d, a| d.get(b"OCGs").ok() == Some(&Object::Array(vec![a.clone()])),
        ),
        (
            "all off",
            |_, _, b| dictionary! { "OCGs" => vec![b], "P" => "AllOff" },
            |d, _| {
                d.get(b"OCGs")
                    .map_or(true, |o| o.as_array().is_ok_and(|a| a.is_empty()))
            },
        ),
        (
            "any off",
            |_, a, b| dictionary! { "OCGs" => vec![a, b], "P" => "AnyOff" },
            |d, _| d.len() == 1 && d.has(b"Type"),
        ),
        (
            "shown for good",
            |_, a, b| dictionary! { "VE" => ve("Or", vec![a, ve("Not", vec![b])]) },
            |d, _| d.len() == 1 && d.has(b"Type"),
        ),
    ];
    for (case, rule, rewritten) in cases {
        let (mut p, r) = ruled(true, rule);
        let a = layer_a(&p);
        p.remove_hidden(&[Hidden::HiddenLayers]).unwrap();
        let rule = rule_of(&p, r);
        assert!(rewritten(&rule, &a), "{case}: {rule:?}");
        let saved = Saved::of(&mut p);
        saved.assert_kept(&["RULE-SECRET", "Visible text"]);
        let named = saved.places("LAYER-B");
        assert!(
            named.is_empty(),
            "{case}: the removed layer is named in {named:?}"
        );
    }

    // `/AllOn` with LAYER-B off hides: its content goes, and nothing is left using it.
    let (mut p, _) = ruled(
        true,
        |_, a, b| dictionary! { "OCGs" => vec![a, b], "P" => "AllOn" },
    );
    p.remove_hidden(&[Hidden::HiddenLayers]).unwrap();
    Saved::of(&mut p).assert_gone(&["RULE-SECRET", "LAYER-B"]);
}

/// A stored expression two rules share is rewritten where it is: both rules still use it,
/// and the file doesn't grow.
#[test]
fn a_shared_stored_expression_is_rewritten_once_in_place() {
    let mut p = make(
        "/OC /R1 BDC BT /F1 12 Tf 30 150 Td (RULE-ONE) Tj ET EMC \
         /OC /R2 BDC BT /F1 12 Tf 30 120 Td (RULE-TWO) Tj ET EMC",
        Dictionary::new(),
    );
    let (a, b) = two_layers(&mut p, true);
    let mut ops = vec![b.clone(), ve("Not", vec![a.clone()])];
    ops.extend(std::iter::repeat_n(a.clone(), 200));
    let e = p.doc.add_object(ve("Or", ops));
    let r1 = p
        .doc
        .add_object(dictionary! { "Type" => "OCMD", "VE" => ve("And", vec![a, e.into()]) });
    let r2 = p
        .doc
        .add_object(dictionary! { "Type" => "OCMD", "VE" => ve("Or", vec![e.into(), b.clone()]) });
    page_res_set(
        &mut p,
        "Properties",
        dictionary! { "R1" => r1, "R2" => r2 }.into(),
    );
    let size = p.clone().to_bytes().unwrap().len();
    p.remove_hidden(&[Hidden::HiddenLayers]).unwrap();
    let names = |o: &Object, x: &Object| -> bool {
        fn walk(o: &Object, x: &Object) -> bool {
            o == x || o.as_array().is_ok_and(|a| a.iter().any(|y| walk(y, x)))
        }
        walk(o, x)
    };
    let expr = p.doc.get_object(e).unwrap().clone();
    assert!(!names(&expr, &b), "{expr:?}");
    for r in [r1, r2] {
        let rule = rule_of(&p, r);
        let uses = rule.iter().any(|(_, v)| names(v, &e.into()));
        assert!(uses, "{rule:?}");
    }
    let saved = Saved::of(&mut p);
    saved.assert_kept(&["RULE-ONE", "RULE-TWO"]);
    saved.assert_gone(&["LAYER-B"]);
    assert!(saved.bytes.len() <= size, "{} > {size}", saved.bytes.len());
}

/// Poppler shows and hides the content of a rewritten rule by the layer left in it:
/// `cargo test -p limarepdf-core --test sanitise -- --ignored cross_check`.
#[test]
#[ignore]
fn cross_check_rewritten_and_expression_rules_with_poppler() {
    let visible = tools::pdftoppm(
        &make(
            "BT /F1 12 Tf 30 100 Td (Visible text) Tj ET",
            Dictionary::new(),
        )
        .to_bytes()
        .unwrap(),
    );
    // `/VE` read before `/OCGs` and `/P`: kept, it shows, and looks as before.
    let (mut p, _) = shown_by_ve_over_p();
    let before = tools::pdftoppm(&p.to_bytes().unwrap());
    assert!(changed(&before, &visible) > 0, "Poppler hides it");
    p.remove_hidden(&[Hidden::HiddenLayers]).unwrap();
    assert_eq!(
        changed(&tools::pdftoppm(&p.to_bytes().unwrap()), &before),
        0
    );
    // The rewritten rule follows LAYER-A: switched off, the content hides.
    let (mut p, _) = ruled(true, |_, a, b| dictionary! { "VE" => ve("Or", vec![b, a]) });
    p.remove_hidden(&[Hidden::HiddenLayers]).unwrap();
    let shown = tools::pdftoppm(&p.to_bytes().unwrap());
    assert!(changed(&shown, &visible) > 0, "hidden with LAYER-A on");
    let a = layer_a(&p);
    let mut off = p.clone();
    let props = off
        .doc
        .catalog_mut()
        .unwrap()
        .get_mut(b"OCProperties")
        .unwrap();
    let d = props
        .as_dict_mut()
        .unwrap()
        .get_mut(b"D")
        .unwrap()
        .as_dict_mut()
        .unwrap();
    d.set("OFF", vec![a]);
    let hidden = tools::pdftoppm(&off.to_bytes().unwrap());
    assert_eq!(changed(&hidden, &visible), 0, "shown with LAYER-A off");
    // Content hidden with every layer shown: Poppler doesn't show it either before.
    for (case, rule) in HIDING_ALL_ON {
        let (mut p, _) = ruled(false, rule);
        let before = tools::pdftoppm(&p.to_bytes().unwrap());
        assert_eq!(changed(&before, &visible), 0, "{case}: Poppler shows it");
        p.remove_hidden(&[Hidden::HiddenLayers]).unwrap();
        let after = tools::pdftoppm(&p.to_bytes().unwrap());
        assert_eq!(changed(&after, &before), 0, "{case}: what shows changed");
        assert!(!tools::pdftotext(&p.to_bytes().unwrap()).contains("RULE-SECRET"));
    }
}

/// The side finding: a comment's drawing with a hidden layer of its own (`/OC` on its
/// appearance form, not on the comment) goes with the layer.
#[test]
fn a_comment_drawing_in_a_hidden_layer_goes_with_it() {
    let mut p = make(
        "BT /F1 12 Tf 30 100 Td (Visible text) Tj ET",
        Dictionary::new(),
    );
    let (_, b) = two_layers(&mut p, true);
    let font = helvetica(&mut p);
    let look = p.doc.add_object(Stream::new(
        dictionary! {
            "Type" => "XObject", "Subtype" => "Form", "BBox" => rect([0, 0, 100, 40]), "OC" => b,
            "Resources" => dictionary! { "Font" => dictionary! { "F1" => font } },
        },
        b"BT /F1 12 Tf 5 15 Td (AP-SECRET) Tj ET".to_vec(),
    ));
    let annot = p.doc.add_object(dictionary! {
        "Type" => "Annot", "Subtype" => "Square", "Rect" => rect([20, 140, 120, 180]),
        "AP" => dictionary! { "N" => look },
    });
    add_annot(&mut p, annot);
    p.remove_hidden(&[Hidden::HiddenLayers]).unwrap();
    Saved::of(&mut p).assert_gone(&["AP-SECRET", "LAYER-B"]);
}

/// A comment going with a hidden layer leaves the form as it was, its XFA too: only form
/// widgets going change it.
#[test]
fn a_hidden_comment_going_leaves_the_form_as_it_was() {
    let mut p = make(
        "BT /F1 12 Tf 30 100 Td (Visible text) Tj ET",
        Dictionary::new(),
    );
    let off = add_layers(&mut p);
    let note = p.doc.add_object(dictionary! {
        "Type" => "Annot", "Subtype" => "Text", "Rect" => rect([20, 140, 40, 160]),
        "Contents" => s("WALRUS-NOTE"), "OC" => off,
    });
    add_annot(&mut p, note);
    let field = p.doc.add_object(dictionary! {
        "FT" => "Tx", "T" => s("name"), "V" => s("Kept value"),
    });
    let form = dictionary! { "Fields" => vec![Object::Reference(field)], "XFA" => s("KEPT-XFA") };
    catalog_set(&mut p, "AcroForm", form.into());
    p.remove_hidden(&[Hidden::HiddenLayers]).unwrap();
    let saved = Saved::of(&mut p);
    saved.assert_gone(&["WALRUS-NOTE"]);
    saved.assert_kept(&["Kept value", "KEPT-XFA"]);
}

/// A form field's box in a hidden layer goes from its field, and a field only such boxes
/// showed goes with its value; a field with a box still shown stays (PB-178). With the form's
/// fields nested more than 32 deep, too deep to take the box out of for certain, nothing
/// changes.
#[test]
fn a_field_box_in_a_hidden_layer_goes_from_its_field() {
    for (levels, refused) in [(33, false), (34, true)] {
        let mut p = listed_below(levels);
        let off = add_layers(&mut p);
        let mut fields = Vec::new();
        for (name, value, shown) in [("only", "WALRUS-ONLY", false), ("both", "Kept value", true)] {
            let field = p.doc.new_object_id();
            let tip = s(&format!("WALRUS-BOX-{name}"));
            let mut kids = vec![Object::Reference(widget(
                &mut p,
                dictionary! { "OC" => off, "Parent" => field, "TU" => tip },
            ))];
            if shown {
                kids.push(widget(&mut p, dictionary! { "Parent" => field }).into());
            }
            let d = dictionary! { "FT" => "Tx", "T" => s(name), "V" => s(value), "Kids" => kids };
            p.doc.objects.insert(field, d.into());
            fields.push(field);
        }
        let catalog = p.doc.catalog_mut().unwrap();
        let form = catalog.get_mut(b"AcroForm").unwrap().as_dict_mut().unwrap();
        let list = form.get_mut(b"Fields").unwrap().as_array_mut().unwrap();
        list.extend(fields.iter().map(|&f| Object::Reference(f)));
        if refused {
            let before = p.clone().snapshot().unwrap();
            let err = p.remove_hidden(&[Hidden::HiddenLayers]).unwrap_err();
            let err = format!("{err:#}");
            assert!(err.contains("nested more than 32 deep"), "{err}");
            assert_eq!(p.clone().snapshot().unwrap(), before);
            continue;
        }
        p.remove_hidden(&[Hidden::HiddenLayers]).unwrap();
        let both = p.doc.get_dictionary(fields[1]).unwrap();
        let kids = both.get(b"Kids").unwrap().as_array().unwrap();
        assert_eq!(kids.len(), 1, "{kids:?}");
        let saved = Saved::of(&mut p);
        saved.assert_gone(&["WALRUS-ONLY", "WALRUS-BOX"]);
        saved.assert_kept(&["Kept value", "WALRUS-LISTED-TOP"]);
    }
}

/// PB-85: a page cropped, with a form partly outside the crop box that the document's tag
/// tree points at, so redaction's check finds the original may still be in the file.
fn crop_with_a_tagged_form() -> Pdf {
    let mut p = make(
        "BT /F1 12 Tf 100 100 Td (Inside) Tj ET q /Fm Do Q",
        Dictionary::new(),
    );
    let font = helvetica(&mut p);
    let form = p.doc.add_object(Stream::new(
        dictionary! {
            "Type" => "XObject", "Subtype" => "Form", "BBox" => rect([0, 0, 300, 200]),
            "Resources" => dictionary! { "Font" => dictionary! { "F1" => font } },
        },
        b"BT /F1 6 Tf 2 100 Td (OutZebra) Tj ET BT /F1 12 Tf 100 60 Td (Kept) Tj ET".to_vec(),
    ));
    page_res_set(&mut p, "XObject", dictionary! { "Fm" => form }.into());
    p.crop_page(0, [50.0, 20.0, 250.0, 180.0]).unwrap();
    // As tests/redact.rs's `tag_tree_pointing_at`.
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
    p
}

#[test]
fn content_outside_the_crop_box_that_redaction_leaves_is_refused() {
    let why = "Content outside the visible page area can't be removed for certain: the check \
               after redacting found removed content may still be in the file, through the tag \
               tree. Nothing was changed; leave out content outside the visible page area to \
               remove the rest.";
    for what in [
        &[Hidden::OutsideCrop][..],
        &[Hidden::Metadata, Hidden::OutsideCrop],
    ] {
        let mut p = crop_with_a_tagged_form();
        p.set_info(&Info {
            title: Some("Walrus title".into()),
            ..Default::default()
        })
        .unwrap();
        let before = p.state_before();
        // Finding counts the page, so it can be chosen; removing then says why it stops.
        let found = p.find_hidden().unwrap();
        assert_eq!(found.count(Hidden::OutsideCrop), 1, "{what:?}");
        let e = p.remove_hidden(what).unwrap_err();
        assert!(format!("{e:#}").contains(why), "{what:?}: {e:#}");
        assert_eq!(p.state_since(&before), [], "{what:?}");
    }
}

/// When redaction warns that an original picture it cut is still drawn, unchanged, elsewhere
/// on the page, the removal says so too.
#[test]
fn a_picture_cut_outside_the_crop_box_and_still_drawn_inside_it_is_reported() {
    let mut p = make(
        "q 40 0 0 40 5 100 cm /Im1 Do Q q 1 0 0 1 110 100 cm /B Do Q",
        Dictionary::new(),
    );
    let im = p.doc.add_object(Stream::new(
        dictionary! {
            "Type" => "XObject", "Subtype" => "Image", "Width" => 1, "Height" => 1,
            "ColorSpace" => "DeviceGray", "BitsPerComponent" => 8,
        },
        vec![255],
    ));
    let b = p.doc.add_object(Stream::new(
        dictionary! { "Type" => "XObject", "Subtype" => "Form", "BBox" => rect([0, 0, 50, 50]) },
        b"q 50 0 0 50 0 0 cm /Im1 Do Q".to_vec(),
    ));
    page_res_set(
        &mut p,
        "XObject",
        dictionary! { "Im1" => im, "B" => b }.into(),
    );
    p.crop_page(0, [50.0, 20.0, 250.0, 180.0]).unwrap();
    let rep = p.remove_hidden(&[Hidden::OutsideCrop]).unwrap();
    assert_eq!(rep.count(Hidden::OutsideCrop), 1);
    assert_eq!(rep.pictures_elsewhere, [0]);
    assert_eq!(
        rep.notes(),
        [
            "An original picture affected by redaction is still drawn elsewhere on page 1 and may \
             retain the removed content."
        ]
    );
}

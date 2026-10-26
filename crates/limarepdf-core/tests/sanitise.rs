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

/// A removal that can't look at everything refuses before changing anything, while finding
/// what there is still works.
fn refuses_unchanged(mut p: Pdf, why: &str) {
    p.set_info(&Info {
        title: Some("Walrus title".into()),
        ..Default::default()
    })
    .unwrap();
    // Serialising moves lopdf's object counter on, so copies are compared.
    let before = p.clone();
    let err = p
        .remove_hidden(&[Hidden::Metadata, Hidden::HiddenLayers])
        .unwrap_err();
    assert!(format!("{err:#}").contains(why), "{err:#}");
    assert_eq!(
        p.clone().snapshot().unwrap(),
        before.clone().snapshot().unwrap()
    );
    let found = p.find_hidden().unwrap();
    assert_eq!(found.count(Hidden::HiddenLayers), 1);
    assert_eq!(found.count(Hidden::Metadata), 1);
    // Without the hidden layers, the rest goes.
    p.remove_hidden(&[Hidden::Metadata]).unwrap();
    assert_eq!(p.info().title, None);
}

#[test]
fn hidden_layers_nested_too_deep_to_look_at_are_refused() {
    refuses_unchanged(nested_layers(40), "nested more than 32 deep");
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
        rep.note(),
        Some(
            "Some of this document's data had a small fault in it (other PDF readers overlook \
             it too). It was all read, and nothing was skipped."
        )
    );
    // Data without faults gives no note.
    let mut p = layered_page(long_layered(Dictionary::new(), "WALRUS", "Visible"));
    let rep = p.remove_hidden(&[Hidden::HiddenLayers]).unwrap();
    assert_eq!(rep.count(Hidden::HiddenLayers), 1);
    assert_eq!(rep.note(), None);
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
        // hayro 0.8.0 doesn't clip with a path painted as it is set (`W f`; backlog PB-11):
        // how the page should look, drawn with the clip on its own.
        let before = match content.contains("W f") {
            true => drawn(&mut make(
                "100 0 100 200 re W n 0 0 1 rg 0 0 300 100 re f",
                Dictionary::new(),
            )),
            false => drawn(&mut p),
        };
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

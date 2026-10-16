use lopdf::{dictionary, Dictionary, Object, Stream};
use mpe_core::{Hidden, Info, Pdf};

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

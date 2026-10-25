//! Filling in made-up forms: values, appearances, fonts, checks and resets.

use limarepdf_core::render::{self, Renderer};
use limarepdf_core::{FieldValue, Pdf};
use lopdf::{dictionary, Dictionary, Object, ObjectId, StringFormat};

const H: f32 = 200.0;

fn page() -> Pdf {
    let mut p = Pdf::new();
    p.insert_blank(0, 300.0, H).unwrap();
    p
}

fn text(s: &str) -> Object {
    Object::String(s.as_bytes().to_vec(), StringFormat::Literal)
}

/// `s` as a text string in UTF-16, for characters beyond ASCII.
fn utf16(s: &str) -> Object {
    let mut b = vec![0xFE, 0xFF];
    b.extend(s.encode_utf16().flat_map(u16::to_be_bytes));
    Object::String(b, StringFormat::Literal)
}

fn rect(x: f32, y: f32, w: f32, h: f32) -> Object {
    vec![x.into(), y.into(), (x + w).into(), (y + h).into()].into()
}

/// Sets every entry of `extra` in `d`. (lopdf's `extend` makes arrays of repeated keys.)
fn merge(mut d: Dictionary, extra: Dictionary) -> Dictionary {
    for (k, v) in extra {
        d.set(k, v);
    }
    d
}

/// Adds fields that are their own widgets to the first page and the form, with
/// Helvetica as `/Helv` in the form's resources.
fn form(p: &mut Pdf, fields: Vec<Dictionary>) -> Vec<ObjectId> {
    let ids: Vec<ObjectId> = fields
        .into_iter()
        .map(|f| {
            let w = dictionary! { "Type" => "Annot", "Subtype" => "Widget" };
            p.doc.add_object(merge(w, f))
        })
        .collect();
    let refs: Vec<Object> = ids.iter().map(|&i| i.into()).collect();
    let pg = p.page_ids()[0];
    p.doc
        .get_dictionary_mut(pg)
        .unwrap()
        .set("Annots", refs.clone());
    let helv = dictionary! {
        "Type" => "Font", "Subtype" => "Type1", "BaseFont" => "Helvetica",
        "Encoding" => "WinAnsiEncoding",
    };
    let helv = p.doc.add_object(helv);
    let form = dictionary! {
        "Fields" => refs,
        "DA" => text("/Helv 0 Tf 0 g"),
        "DR" => dictionary! { "Font" => dictionary! { "Helv" => helv } },
    };
    let form = p.doc.add_object(form);
    p.doc.catalog_mut().unwrap().set("AcroForm", form);
    ids
}

fn value(p: &Pdf, name: &str) -> FieldValue {
    let f = p.form_fields();
    f.iter().find(|f| f.name == name).unwrap().value.clone()
}

/// The normal appearance stream's content, decompressed.
fn appearance(p: &Pdf, widget: ObjectId) -> String {
    let d = p.doc.get_dictionary(widget).unwrap();
    let ap = d.get(b"AP").unwrap().as_dict().unwrap();
    let n = ap.get(b"N").unwrap().as_reference().unwrap();
    let s = p.doc.get_object(n).unwrap().as_stream().unwrap();
    String::from_utf8_lossy(&s.decompressed_content().unwrap_or(s.content.clone())).into_owned()
}

/// The box (left, top, right, bottom, in pixels at 1 px per point, y down) holding
/// every dark pixel of the first page, or `None` when there are none.
fn ink(p: &mut Pdf) -> Option<[usize; 4]> {
    let hp = render::load(p.render_snapshot().unwrap(), None).unwrap();
    let bmp = Renderer::new(&hp).render(0, 1.0).unwrap();
    let mut b: Option<[usize; 4]> = None;
    for y in 0..bmp.height {
        for x in 0..bmp.width {
            let i = (y * bmp.width + x) * 4;
            if bmp.rgba[i] < 128 && bmp.rgba[i + 1] < 128 && bmp.rgba[i + 2] < 128 {
                b = Some(match b {
                    None => [x, y, x, y],
                    Some(b) => [b[0].min(x), b[1].min(y), b[2].max(x), b[3].max(y)],
                });
            }
        }
    }
    b
}

#[test]
fn render_snapshot_draws_fields_without_a_drawing() {
    // A value another app left the reader to draw, as `limarepdf-cli render` sees it (BUG-23).
    let mut p = page();
    let ids = form(
        &mut p,
        vec![dictionary! {
            "FT" => "Tx", "T" => text("name"), "Rect" => rect(10.0, 150.0, 150.0, 20.0),
            "V" => text("Hello"), "F" => 4,
        }],
    );
    let b = ink(&mut p).expect("the value is drawn");
    assert!(
        b[0] >= 10 && b[2] <= 160 && b[1] >= 30 && b[3] <= 50,
        "outside the field: {b:?}"
    );
    // Only the copy drawn is given the drawing.
    assert!(!p.doc.get_dictionary(ids[0]).unwrap().has(b"AP"));
}

#[test]
fn sets_text_and_draws_it_inside_the_field() {
    let mut p = page();
    let ids = form(
        &mut p,
        vec![dictionary! {
            "FT" => "Tx", "T" => text("name"), "Rect" => rect(10.0, 150.0, 150.0, 20.0),
            "RV" => text("<p>old</p>"),
        }],
    );
    let form = p
        .doc
        .catalog()
        .unwrap()
        .get(b"AcroForm")
        .unwrap()
        .as_reference()
        .unwrap();
    p.doc
        .get_dictionary_mut(form)
        .unwrap()
        .set("XFA", text("<xdp/>"));
    p.set_field("name", FieldValue::Text("MMMM".into()))
        .unwrap();

    assert_eq!(value(&p, "name"), FieldValue::Text("MMMM".into()));
    let ap = appearance(&p, ids[0]);
    assert!(ap.contains("/Tx BMC") && ap.contains("(MMMM) Tj"), "{ap}");
    let d = p.doc.get_dictionary(ids[0]).unwrap();
    assert!(!d.has(b"RV"), "rich text removed");
    assert!(
        !p.doc.get_dictionary(form).unwrap().has(b"XFA"),
        "XFA removed"
    );

    // The text is drawn, and only inside the field (x 10-160, y 30-50 from the top).
    let b = ink(&mut p).expect("text drawn");
    assert!(
        b[0] >= 10 && b[2] <= 160 && b[1] >= 30 && b[3] <= 50,
        "{b:?}"
    );

    // It survives saving and opening again.
    let mut q = Pdf::from_bytes(&p.to_bytes().unwrap(), None).unwrap();
    assert_eq!(value(&q, "name"), FieldValue::Text("MMMM".into()));
    assert!(ink(&mut q).is_some());
}

#[test]
fn turns_the_text_of_a_rotated_widget() {
    let mut p = page();
    // A tall, narrow field whose text runs upwards (/MK /R 90).
    form(
        &mut p,
        vec![dictionary! {
            "FT" => "Tx", "T" => text("up"), "Rect" => rect(100.0, 40.0, 20.0, 120.0),
            "MK" => dictionary! { "R" => 90 },
        }],
    );
    p.set_field("up", FieldValue::Text("MMMMMM".into()))
        .unwrap();
    let b = ink(&mut p).expect("text drawn");
    // Inside the field (x 100-120, y 40-160 from the top), and taller than wide.
    assert!(
        b[0] >= 100 && b[2] <= 120 && b[1] >= 40 && b[3] <= 160,
        "{b:?}"
    );
    assert!(b[3] - b[1] > 2 * (b[2] - b[0]), "{b:?}");
}

#[test]
fn ticks_checkboxes_and_chooses_radio_buttons() {
    let mut p = page();
    let states =
        |on: &str| dictionary! { "N" => dictionary! { on => Object::Null, "Off" => Object::Null } };
    let ids = form(
        &mut p,
        vec![
            dictionary! { "FT" => "Btn", "T" => text("agree"), "AP" => states("Yes"),
            "AS" => "Off", "Rect" => rect(10.0, 10.0, 12.0, 12.0) },
            // No appearance at all: LimarePDF draws one.
            dictionary! { "FT" => "Btn", "T" => text("bare"), "Rect" => rect(30.0, 10.0, 12.0, 12.0),
            "MK" => dictionary! { "BC" => vec![0.into()] } },
        ],
    );
    // A radio group whose states index /Opt.
    let r: Vec<ObjectId> = ["0", "1"]
        .iter()
        .map(|s| {
            let w = dictionary! { "Type" => "Annot", "Subtype" => "Widget", "AP" => states(s),
            "Rect" => rect(60.0, 10.0, 12.0, 12.0) };
            p.doc.add_object(w)
        })
        .collect();
    let colour = p.doc.add_object(dictionary! {
        "FT" => "Btn", "Ff" => 1 << 15, "T" => text("colour"),
        "Opt" => vec![text("red"), text("blue")], "Kids" => vec![r[0].into(), r[1].into()],
    });
    let pg = p.page_ids()[0];
    let annots = p
        .doc
        .get_dictionary_mut(pg)
        .unwrap()
        .get_mut(b"Annots")
        .unwrap();
    annots
        .as_array_mut()
        .unwrap()
        .extend([r[0].into(), r[1].into()]);
    let form = p
        .doc
        .catalog()
        .unwrap()
        .get(b"AcroForm")
        .unwrap()
        .as_reference()
        .unwrap();
    let fields = p
        .doc
        .get_dictionary_mut(form)
        .unwrap()
        .get_mut(b"Fields")
        .unwrap();
    fields.as_array_mut().unwrap().push(colour.into());

    p.set_field("agree", FieldValue::Check(true)).unwrap();
    assert_eq!(value(&p, "agree"), FieldValue::Check(true));
    let as_of = |p: &Pdf, id: ObjectId| {
        let d = p.doc.get_dictionary(id).unwrap();
        d.get(b"AS").unwrap().as_name().unwrap().to_vec()
    };
    assert_eq!(as_of(&p, ids[0]), b"Yes");
    p.set_field("agree", FieldValue::Check(false)).unwrap();
    assert_eq!(as_of(&p, ids[0]), b"Off");
    assert_eq!(value(&p, "agree"), FieldValue::Check(false));

    p.set_field("bare", FieldValue::Check(true)).unwrap();
    assert_eq!(value(&p, "bare"), FieldValue::Check(true));
    assert_eq!(as_of(&p, ids[1]), b"Yes");
    let b = ink(&mut p).expect("check mark and border drawn");
    assert!(b[0] >= 30 && b[2] <= 42, "{b:?}");

    p.set_field("colour", FieldValue::Choice(vec!["blue".into()]))
        .unwrap();
    assert_eq!(value(&p, "colour"), FieldValue::Choice(vec!["blue".into()]));
    assert_eq!(as_of(&p, r[0]), b"Off");
    assert_eq!(as_of(&p, r[1]), b"1");
    p.set_field("colour", FieldValue::Choice(vec![])).unwrap();
    assert_eq!(value(&p, "colour"), FieldValue::Choice(vec![]));
}

#[test]
fn chooses_from_lists() {
    let mut p = page();
    let ids = form(
        &mut p,
        vec![
            dictionary! { "FT" => "Ch", "Ff" => 1 << 17, "T" => text("size"),
            "Opt" => vec![vec![text("s"), text("Small")].into(), vec![text("m"), text("Medium")].into()],
            "Rect" => rect(10.0, 150.0, 100.0, 20.0) },
            dictionary! { "FT" => "Ch", "Ff" => 1 << 21, "T" => text("pets"),
            "Opt" => vec![text("cat"), text("dog"), text("eel")],
            "Rect" => rect(10.0, 60.0, 100.0, 60.0) },
            dictionary! { "FT" => "Ch", "Ff" => (1 << 17) | (1 << 18), "T" => text("town"),
            "Opt" => vec![text("Oslo")], "Rect" => rect(150.0, 150.0, 100.0, 20.0) },
        ],
    );
    // By shown text: stored as the export value, drawn as the shown text.
    p.set_field("size", FieldValue::Choice(vec!["Medium".into()]))
        .unwrap();
    assert_eq!(value(&p, "size"), FieldValue::Choice(vec!["m".into()]));
    assert!(appearance(&p, ids[0]).contains("(Medium) Tj"));

    p.set_field("pets", FieldValue::Choice(vec!["cat".into(), "eel".into()]))
        .unwrap();
    assert_eq!(
        value(&p, "pets"),
        FieldValue::Choice(vec!["cat".into(), "eel".into()])
    );
    let d = p.doc.get_dictionary(ids[1]).unwrap();
    let i: Vec<i64> = d
        .get(b"I")
        .unwrap()
        .as_array()
        .unwrap()
        .iter()
        .map(|o| o.as_i64().unwrap())
        .collect();
    assert_eq!(i, [0, 2]);
    let ap = appearance(&p, ids[1]);
    assert!(ap.contains("(cat) Tj") && ap.contains("(dog) Tj") && ap.contains("(eel) Tj"));

    // An editable drop-down takes any text.
    p.set_field("town", FieldValue::Choice(vec!["Bergen".into()]))
        .unwrap();
    assert_eq!(value(&p, "town"), FieldValue::Choice(vec!["Bergen".into()]));
}

#[test]
fn refuses_values_that_dont_fit() {
    let mut p = page();
    form(
        &mut p,
        vec![
            dictionary! { "FT" => "Tx", "T" => text("one"), "MaxLen" => 3, "Rect" => rect(0.0, 0.0, 50.0, 20.0) },
            dictionary! { "FT" => "Tx", "T" => text("locked"), "Ff" => 1, "Rect" => rect(0.0, 30.0, 50.0, 20.0) },
            dictionary! { "FT" => "Ch", "T" => text("pick"), "Opt" => vec![text("a")], "Rect" => rect(0.0, 60.0, 50.0, 20.0) },
            dictionary! { "FT" => "Btn", "Ff" => 1 << 16, "T" => text("go"), "Rect" => rect(0.0, 90.0, 50.0, 20.0) },
        ],
    );
    let err = |r: limarepdf_core::Result<()>| r.unwrap_err().to_string();
    let t = |s: &str| FieldValue::Text(s.into());
    assert!(err(p.set_field("nope", t("x"))).contains("no form field"));
    assert!(err(p.set_field("one", t("abcd"))).contains("at most 3"));
    assert!(err(p.set_field("one", t("a\nb"))).contains("line breaks"));
    assert!(err(p.set_field("one", t("日本"))).contains("Latin, Greek and Cyrillic"));
    assert!(err(p.set_field("locked", t("x"))).contains("read-only"));
    assert!(
        err(p.set_field("pick", FieldValue::Choice(vec!["b".into()]))).contains("isn't one of")
    );
    assert!(err(p.set_field("go", FieldValue::None)).contains("push button"));
    assert!(err(p.set_field("one", FieldValue::Check(true))).contains("doesn't suit"));
    // Nothing was written.
    assert_eq!(value(&p, "one"), FieldValue::Text(String::new()));
}

#[test]
fn passwords_are_drawn_hidden_and_not_kept() {
    let mut p = page();
    let ids = form(
        &mut p,
        vec![
            dictionary! { "FT" => "Tx", "Ff" => 1 << 13, "T" => text("pw"), "Rect" => rect(0.0, 0.0, 80.0, 20.0) },
        ],
    );
    p.set_field("pw", FieldValue::Text("secret".into()))
        .unwrap();
    assert_eq!(value(&p, "pw"), FieldValue::Text(String::new()));
    let ap = appearance(&p, ids[0]);
    assert!(ap.contains("(******) Tj") && !ap.contains("secret"));
}

#[test]
fn replaces_fonts_it_cannot_write_with() {
    let mut p = page();
    let ids = form(
        &mut p,
        vec![
            dictionary! { "FT" => "Tx", "T" => text("t"), "DA" => text("/Sub 10 Tf 0 0 1 rg"),
            "Rect" => rect(0.0, 0.0, 80.0, 20.0) },
        ],
    );
    // An embedded subset font: its glyphs may be missing, so a standard font is used.
    let sub = p.doc.add_object(dictionary! {
        "Type" => "Font", "Subtype" => "TrueType", "BaseFont" => "ABCDEF+Times-Bold",
    });
    let form = p
        .doc
        .catalog()
        .unwrap()
        .get(b"AcroForm")
        .unwrap()
        .as_reference()
        .unwrap();
    let dr = p
        .doc
        .get_dictionary_mut(form)
        .unwrap()
        .get_mut(b"DR")
        .unwrap();
    let fonts = dr.as_dict_mut().unwrap().get_mut(b"Font").unwrap();
    fonts.as_dict_mut().unwrap().set("Sub", sub);

    p.set_field("t", FieldValue::Text("café".into())).unwrap();
    let ap = appearance(&p, ids[0]);
    assert!(ap.contains("/LimareTimesBold 10 Tf"), "{ap}");
    assert!(ap.contains("0 0 1 rg"), "{ap}");
    let d = p.doc.get_dictionary(ids[0]).unwrap();
    let da = d.get(b"DA").unwrap().as_str().unwrap();
    assert!(String::from_utf8_lossy(da).contains("/LimareTimesBold"));
    // The new font is in the form's resources, and the text reads back.
    let fields = p.form_fields();
    assert_eq!(fields[0].value, FieldValue::Text("café".into()));
    let dr = p.doc.get_dictionary(form).unwrap().get(b"DR").unwrap();
    let dr = p.doc.dereference(dr).unwrap().1.as_dict().unwrap();
    let fonts = p.doc.dereference(dr.get(b"Font").unwrap()).unwrap().1;
    assert!(fonts.as_dict().unwrap().has(b"LimareTimesBold"));
}

#[test]
fn replaces_a_font_that_lacks_a_character() {
    let mut p = page();
    let ids = form(
        &mut p,
        vec![
            dictionary! { "FT" => "Tx", "T" => text("t"), "DA" => text("/F3 10 Tf 0 g"),
            "Rect" => rect(0.0, 0.0, 120.0, 20.0) },
        ],
    );
    // Like LibreOffice's fonts: WinAnsi, with zero widths where it has no characters.
    let mut widths = vec![Object::Integer(500); 224];
    widths[0x96 - 32] = 0.into();
    let arial = p.doc.add_object(dictionary! {
        "Type" => "Font", "Subtype" => "TrueType", "BaseFont" => "ArialMT",
        "Encoding" => "WinAnsiEncoding", "FirstChar" => 32, "LastChar" => 255, "Widths" => widths,
    });
    let form = p
        .doc
        .catalog()
        .unwrap()
        .get(b"AcroForm")
        .unwrap()
        .as_reference()
        .unwrap();
    let dr = p
        .doc
        .get_dictionary_mut(form)
        .unwrap()
        .get_mut(b"DR")
        .unwrap();
    let fonts = dr.as_dict_mut().unwrap().get_mut(b"Font").unwrap();
    fonts.as_dict_mut().unwrap().set("F3", arial);

    p.set_field("t", FieldValue::Text("Café".into())).unwrap();
    assert!(
        appearance(&p, ids[0]).contains("/F3 10 Tf"),
        "its own font when it can"
    );
    p.set_field("t", FieldValue::Text("a – b".into())).unwrap();
    assert!(appearance(&p, ids[0]).contains("/LimareHelvetica 10 Tf"));
}

#[test]
fn resets_to_defaults() {
    let mut p = page();
    form(
        &mut p,
        vec![
            dictionary! { "FT" => "Tx", "T" => text("a"), "DV" => text("start"), "Rect" => rect(0.0, 0.0, 80.0, 20.0) },
            dictionary! { "FT" => "Tx", "T" => text("b"), "Rect" => rect(0.0, 30.0, 80.0, 20.0) },
        ],
    );
    p.set_field("a", FieldValue::Text("x".into())).unwrap();
    p.set_field("b", FieldValue::Text("y".into())).unwrap();
    p.reset_form().unwrap();
    assert_eq!(value(&p, "a"), FieldValue::Text("start".into()));
    assert_eq!(value(&p, "b"), FieldValue::Text(String::new()));
}

#[test]
fn draws_missing_appearances() {
    let mut p = page();
    let ids = form(
        &mut p,
        vec![
            dictionary! { "FT" => "Tx", "T" => text("a"), "V" => text("shown"), "Rect" => rect(0.0, 0.0, 80.0, 20.0) },
            dictionary! { "FT" => "Tx", "T" => text("b"), "V" => utf16("שלום"), "Rect" => rect(0.0, 30.0, 80.0, 20.0) },
        ],
    );
    // A ticked checkbox without drawings gets them, showing it ticked.
    let check = p.doc.add_object(dictionary! {
        "Type" => "Annot", "Subtype" => "Widget", "FT" => "Btn", "T" => text("c"), "V" => "Yes",
        "Rect" => rect(100.0, 0.0, 12.0, 12.0),
    });
    let pg = p.page_ids()[0];
    let annots = p
        .doc
        .get_dictionary_mut(pg)
        .unwrap()
        .get_mut(b"Annots")
        .unwrap();
    annots.as_array_mut().unwrap().push(check.into());
    let form = p
        .doc
        .catalog()
        .unwrap()
        .get(b"AcroForm")
        .unwrap()
        .as_reference()
        .unwrap();
    let fields = p
        .doc
        .get_dictionary_mut(form)
        .unwrap()
        .get_mut(b"Fields")
        .unwrap();
    fields.as_array_mut().unwrap().push(check.into());
    assert_eq!(value(&p, "c"), FieldValue::Check(true));

    // Two drawn; the field with text LimarePDF can't write is left alone.
    assert_eq!(p.refresh_appearances(), 2);
    assert_eq!(value(&p, "c"), FieldValue::Check(true));
    let d = p.doc.get_dictionary(check).unwrap();
    assert_eq!(d.get(b"AS").unwrap().as_name().unwrap(), b"Yes");
    assert!(appearance(&p, ids[0]).contains("(shown) Tj"));
    assert!(!p.doc.get_dictionary(ids[1]).unwrap().has(b"AP"));
    assert_eq!(p.refresh_appearances(), 0);
    // A form that asks for its appearances to be redrawn gets all of them redrawn.
    let form = p
        .doc
        .catalog()
        .unwrap()
        .get(b"AcroForm")
        .unwrap()
        .as_reference()
        .unwrap();
    p.doc
        .get_dictionary_mut(form)
        .unwrap()
        .set("NeedAppearances", true);
    assert_eq!(p.refresh_appearances(), 1);
}

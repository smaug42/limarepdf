//! Filling in made-up forms: values, appearances, fonts, checks and resets.

mod common;

use common::saved::Saved;
use common::tool;
use limarepdf_core::render::{self, Renderer};
use limarepdf_core::{FieldValue, Pdf};
use lopdf::content::Content;
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

fn acro_form(p: &Pdf) -> ObjectId {
    let form = p.doc.catalog().unwrap().get(b"AcroForm").unwrap();
    form.as_reference().unwrap()
}

/// Adds the field `parent` to the form (made by [`form`]), with `kids` (its widgets, or fields
/// that are their own widgets) on the first page: the kids' ids.
fn family(p: &mut Pdf, parent: Dictionary, kids: Vec<Dictionary>) -> Vec<ObjectId> {
    let parent = p.doc.add_object(parent);
    let ids: Vec<ObjectId> = kids
        .into_iter()
        .map(|k| {
            let w = dictionary! { "Type" => "Annot", "Subtype" => "Widget", "Parent" => parent };
            p.doc.add_object(merge(w, k))
        })
        .collect();
    let refs: Vec<Object> = ids.iter().map(|&i| i.into()).collect();
    p.doc
        .get_dictionary_mut(parent)
        .unwrap()
        .set("Kids", refs.clone());
    let pg = p.page_ids()[0];
    let annots = p.doc.get_dictionary_mut(pg).unwrap().get_mut(b"Annots");
    annots.unwrap().as_array_mut().unwrap().extend(refs);
    let form = acro_form(p);
    let fields = p.doc.get_dictionary_mut(form).unwrap().get_mut(b"Fields");
    fields.unwrap().as_array_mut().unwrap().push(parent.into());
    ids
}

fn reopened(p: &mut Pdf) -> Pdf {
    Pdf::from_bytes(&p.to_bytes().unwrap(), None).unwrap()
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
            dictionary! { "FT" => "Sig", "T" => text("sig"), "Rect" => rect(0.0, 120.0, 50.0, 20.0) },
        ],
    );
    let err = |r: limarepdf_core::Result<()>| r.unwrap_err().to_string();
    let t = |s: &str| FieldValue::Text(s.into());
    assert!(err(p.set_field("nope", t("x"))).contains("no form field"));
    assert!(err(p.set_field("one", t("abcd"))).contains("at most 3"));
    assert!(err(p.set_field("one", t("a\nb"))).contains("line breaks"));
    assert!(err(p.set_field("one", t("של"))).contains("right-to-left"));
    assert!(err(p.set_field("one", t("\u{2FFFE}"))).contains("No font on this computer has"));
    assert!(err(p.set_field("locked", t("x"))).contains("read-only"));
    assert!(
        err(p.set_field("pick", FieldValue::Choice(vec!["b".into()]))).contains("isn't one of")
    );
    assert!(err(p.set_field("go", FieldValue::None)).contains("push button"));
    // A signature field points to signing (BUG-33).
    let sig = err(p.set_field("sig", t("x")));
    assert!(
        sig.contains("Document > Digital signatures > Sign with digital ID")
            && sig.contains("`limarepdf-cli sign --field \"sig\"`"),
        "{sig}"
    );
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

/// The form's fonts (`/DR /Font`), as made by [`form`].
fn form_fonts(p: &Pdf) -> Dictionary {
    let form = p.doc.get_dictionary(acro_form(p)).unwrap();
    let dr = p.doc.dereference(form.get(b"DR").unwrap()).unwrap().1;
    font_of(p, dr.as_dict().unwrap().get(b"Font").unwrap())
}

fn form_fonts_mut(p: &mut Pdf) -> &mut Dictionary {
    let form = acro_form(p);
    let dr = p.doc.get_dictionary_mut(form).unwrap().get_mut(b"DR");
    let fonts = dr.unwrap().as_dict_mut().unwrap().get_mut(b"Font");
    fonts.unwrap().as_dict_mut().unwrap()
}

/// The font a resource entry holds (a reference, or a dictionary written in place).
fn font_of(p: &Pdf, entry: &Object) -> Dictionary {
    let (_, font) = p.doc.dereference(entry).unwrap();
    font.as_dict().unwrap().clone()
}

/// The standard font `base` as LimarePDF adds it to a form.
fn standard(base: &str) -> Dictionary {
    let mut d = dictionary! { "Type" => "Font", "Subtype" => "Type1", "BaseFont" => base };
    if base != "ZapfDingbats" {
        d.set("Encoding", "WinAnsiEncoding");
    }
    d
}

/// A form ([`form`]) whose text fields `names` are in a Symbol font, `/Orig`, which can't write
/// Latin letters, so their values go in Helvetica: the fields' ids and the Symbol font's.
fn in_symbol(p: &mut Pdf, names: &[&str]) -> (Vec<ObjectId>, ObjectId) {
    let fields = (names.iter().enumerate())
        .map(|(i, n)| {
            dictionary! { "FT" => "Tx", "T" => text(n), "DA" => text("/Orig 12 Tf 0 g"),
            "Rect" => rect(10.0, 10.0 + 30.0 * i as f32, 120.0, 20.0) }
        })
        .collect();
    let ids = form(p, fields);
    let symbol = dictionary! { "Type" => "Font", "Subtype" => "Type1", "BaseFont" => "Symbol" };
    let symbol = p.doc.add_object(symbol);
    form_fonts_mut(p).set("Orig", symbol);
    (ids, symbol)
}

/// The field `name` (its own widget) on the first page.
fn widget_of(p: &Pdf, name: &str) -> ObjectId {
    let page = p.doc.get_dictionary(p.page_ids()[0]).unwrap();
    let annots = p.doc.dereference(page.get(b"Annots").unwrap()).unwrap().1;
    let annots = annots.as_array().unwrap().iter();
    annots
        .map(|a| a.as_reference().unwrap())
        .find(|&id| {
            let t = p.doc.get_dictionary(id).unwrap().get(b"T").ok();
            t.and_then(|t| t.as_str().ok()) == Some(name.as_bytes())
        })
        .unwrap()
}

/// A widget's normal drawing, the one for `state` when it has one for each.
fn drawing(p: &Pdf, widget: ObjectId, state: Option<&str>) -> ObjectId {
    let d = p.doc.get_dictionary(widget).unwrap();
    let n = d.get(b"AP").unwrap().as_dict().unwrap().get(b"N").unwrap();
    match state {
        Some(s) => {
            let n = p.doc.dereference(n).unwrap().1.as_dict().unwrap();
            n.get(s.as_bytes()).unwrap().as_reference().unwrap()
        }
        None => n.as_reference().unwrap(),
    }
}

/// The fonts the drawing `ap` sets text in: for each `Tf`, the name it gives, the entry the
/// drawing's own resources hold under it, and that font.
fn drawn_in(p: &Pdf, ap: ObjectId) -> Vec<(String, Object, Dictionary)> {
    let s = p.doc.get_object(ap).unwrap().as_stream().unwrap();
    let content = s.decompressed_content().unwrap_or(s.content.clone());
    let res = s.dict.get(b"Resources").unwrap();
    let res = p.doc.dereference(res).unwrap().1;
    let fonts = res.as_dict().unwrap().get(b"Font").unwrap();
    let fonts = p.doc.dereference(fonts).unwrap().1.as_dict().unwrap();
    let ops = Content::decode(&content).unwrap().operations;
    (ops.iter().filter(|o| o.operator == "Tf"))
        .map(|o| {
            let name = o.operands[0].as_name().unwrap();
            let entry = fonts.get(name).unwrap().clone();
            let name = String::from_utf8_lossy(name).into_owned();
            (name, entry.clone(), font_of(p, &entry))
        })
        .collect()
}

/// That field `name`'s drawing sets its text only in Helvetica as LimarePDF adds it, the font
/// the form holds under `want`, named `want` there and by the field's `/DA`.
fn written_in_helvetica(p: &Pdf, name: &str, want: &str) {
    let w = widget_of(p, name);
    let drawn = drawn_in(p, drawing(p, w, None));
    assert!(!drawn.is_empty(), "{name}");
    let held = form_fonts(p).get(want.as_bytes()).ok().cloned();
    let helvetica = standard("Helvetica");
    for (n, entry, font) in drawn {
        assert_eq!((n.as_str(), &font), (want, &helvetica), "{name}");
        assert_eq!(Some(entry), held, "{name}");
    }
    let da = p.doc.get_dictionary(w).unwrap().get(b"DA").unwrap();
    let da = String::from_utf8_lossy(da.as_str().unwrap()).into_owned();
    assert!(da.contains(&format!("/{want} ")), "{name}: {da}");
}

/// A field whose own font can't write its value is written in Helvetica under the name
/// LimarePDF gives it, `LimareHelvetica`, only when the form holds Helvetica there as LimarePDF
/// adds it, or nothing (A-05). The full audit's case: a form whose `LimareHelvetica` is the
/// field's own Symbol font, in which "HELLO" was drawn in Greek letters. That stays the form's
/// `LimareHelvetica`, and Helvetica goes under the next name.
#[test]
fn a_fallback_name_holding_another_font_isnt_used() {
    let mut p = page();
    let (_, symbol) = in_symbol(&mut p, &["Name"]);
    form_fonts_mut(&mut p).set("LimareHelvetica", symbol);
    common::seeds::pdf(&p);
    let hello = FieldValue::Text("HELLO".into());
    p.set_field("Name", hello.clone()).unwrap();
    let q = reopened(&mut p);
    for p in [&p, &q] {
        written_in_helvetica(p, "Name", "LimareHelvetica1");
        let kept = font_of(p, form_fonts(p).get(b"LimareHelvetica").unwrap());
        assert_eq!(kept.get(b"BaseFont").unwrap().as_name().unwrap(), b"Symbol");
        assert_eq!(value(p, "Name"), hello);
    }
}

/// [`a_fallback_name_holding_another_font_isnt_used`] with `taken` under `LimareHelvetica`, as
/// an object of its own or written in place: "Café" (é, so the encoding shows) goes in
/// Helvetica under `LimareHelvetica1`, and the form's `LimareHelvetica` stays as it was.
fn fallback_name_holding(taken: Dictionary, in_place: bool) {
    let mut p = page();
    in_symbol(&mut p, &["Name"]);
    let entry = match in_place {
        true => Object::Dictionary(taken.clone()),
        false => p.doc.add_object(taken.clone()).into(),
    };
    form_fonts_mut(&mut p).set("LimareHelvetica", entry);
    common::seeds::pdf(&p);
    let cafe = FieldValue::Text("Café".into());
    p.set_field("Name", cafe.clone()).unwrap();
    let q = reopened(&mut p);
    for p in [&p, &q] {
        written_in_helvetica(p, "Name", "LimareHelvetica1");
        let kept = form_fonts(p).get(b"LimareHelvetica").unwrap().clone();
        assert_eq!(matches!(kept, Object::Dictionary(_)), in_place);
        assert_eq!(font_of(p, &kept), taken);
        assert_eq!(value(p, "Name"), cafe);
    }
}

#[test]
fn a_fallback_name_holding_helvetica_without_an_encoding_isnt_used() {
    let mut f = standard("Helvetica");
    f.remove(b"Encoding");
    fallback_name_holding(f, false);
}

#[test]
fn a_fallback_name_holding_helvetica_in_another_encoding_isnt_used() {
    let mut f = standard("Helvetica");
    f.set("Encoding", "MacRomanEncoding");
    fallback_name_holding(f, false);
}

#[test]
fn a_fallback_name_holding_helvetica_with_differences_isnt_used() {
    let mut f = standard("Helvetica");
    let enc = dictionary! { "Type" => "Encoding", "BaseEncoding" => "WinAnsiEncoding",
    "Differences" => vec![233.into(), Object::Name(b"egrave".to_vec())] };
    f.set("Encoding", enc);
    fallback_name_holding(f, false);
}

#[test]
fn a_fallback_name_holding_a_truetype_helvetica_isnt_used() {
    let mut f = standard("Helvetica");
    f.set("Subtype", "TrueType");
    fallback_name_holding(f, false);
}

#[test]
fn a_fallback_name_holding_helvetica_with_a_descriptor_isnt_used() {
    let mut f = standard("Helvetica");
    // Written in place, so that it reads the same after reopening.
    let desc = dictionary! { "Type" => "FontDescriptor", "FontName" => "Helvetica",
    "Flags" => 32, "ItalicAngle" => 0, "Ascent" => 718, "Descent" => -207, "StemV" => 88 };
    f.set("FontDescriptor", desc);
    fallback_name_holding(f, false);
}

#[test]
fn a_fallback_name_holding_helvetica_with_widths_isnt_used() {
    let mut f = standard("Helvetica");
    f.set("FirstChar", 32);
    f.set("LastChar", 255);
    f.set("Widths", vec![Object::Integer(600); 224]);
    fallback_name_holding(f, false);
}

#[test]
fn a_fallback_name_holding_a_font_in_place_isnt_used() {
    let symbol = dictionary! { "Type" => "Font", "Subtype" => "Type1", "BaseFont" => "Symbol" };
    fallback_name_holding(symbol, true);
}

/// The Helvetica LimarePDF added for a field is used again for another, after saving and
/// reopening: no second font (a control: so it was before A-05's fix).
#[test]
fn a_fallback_font_added_before_is_used_again() {
    let mut p = page();
    in_symbol(&mut p, &["a", "b"]);
    p.set_field("a", FieldValue::Text("HELLO".into())).unwrap();
    let mut q = reopened(&mut p);
    q.set_field("b", FieldValue::Text("WORLD".into())).unwrap();
    let r = reopened(&mut q);
    for p in [&q, &r] {
        written_in_helvetica(p, "a", "LimareHelvetica");
        written_in_helvetica(p, "b", "LimareHelvetica");
        assert_eq!(form_fonts(p).len(), 3, "Helv, Orig and LimareHelvetica");
    }
}

/// The Helvetica LimarePDF added under a numbered name is used again too, after saving and
/// reopening: no further name, no second font.
#[test]
fn a_fallback_font_under_a_numbered_name_is_used_again() {
    let mut p = page();
    let (_, symbol) = in_symbol(&mut p, &["a", "b"]);
    form_fonts_mut(&mut p).set("LimareHelvetica", symbol);
    p.set_field("a", FieldValue::Text("HELLO".into())).unwrap();
    let mut q = reopened(&mut p);
    q.set_field("b", FieldValue::Text("WORLD".into())).unwrap();
    let r = reopened(&mut q);
    for p in [&q, &r] {
        written_in_helvetica(p, "a", "LimareHelvetica1");
        written_in_helvetica(p, "b", "LimareHelvetica1");
        assert_eq!(form_fonts(p).len(), 4, "Helv, Orig and two LimareHelvetica");
    }
}

/// The fields a reset writes in Helvetica share the one font it adds.
#[test]
fn one_reset_adds_one_fallback_font() {
    let mut p = page();
    let (ids, symbol) = in_symbol(&mut p, &["a", "b"]);
    form_fonts_mut(&mut p).set("LimareHelvetica", symbol);
    for (id, dv) in ids.into_iter().zip(["HELLO", "WORLD"]) {
        p.doc.get_dictionary_mut(id).unwrap().set("DV", text(dv));
    }
    p.reset_form().unwrap();
    let q = reopened(&mut p);
    for p in [&p, &q] {
        written_in_helvetica(p, "a", "LimareHelvetica1");
        written_in_helvetica(p, "b", "LimareHelvetica1");
        assert_eq!(form_fonts(p).len(), 4, "Helv, Orig and two LimareHelvetica");
        assert_eq!(value(p, "b"), FieldValue::Text("WORLD".into()));
    }
}

/// A fill that falls back to Helvetica where its name is taken, and can't be drawn in full,
/// changes nothing (M-09): no name is taken, no font added (a control).
#[test]
fn a_fill_where_the_fallback_name_is_taken_that_cant_be_drawn_changes_nothing() {
    let mut p = page();
    let (_, symbol) = in_symbol(&mut p, &[]);
    form_fonts_mut(&mut p).set("LimareHelvetica", symbol);
    let kids = vec![
        dictionary! { "Rect" => rect(10.0, 150.0, 100.0, 20.0) },
        dictionary! { "Rect" => Object::Null },
    ];
    let field = dictionary! { "FT" => "Tx", "T" => text("two"), "DA" => text("/Orig 12 Tf 0 g") };
    family(&mut p, field, kids);
    common::seeds::pdf(&p);
    let before = p.state_before();
    let e = p.set_field("two", FieldValue::Text("HELLO".into()));
    assert!(e.unwrap_err().to_string().contains("rectangle"));
    assert_eq!(p.state_since(&before), []);
}

/// A form with a checkbox "c" that has no drawing, and `zadb` as its `/ZaDb`.
fn check_with_zadb(p: &mut Pdf, zadb: Object) {
    let c = dictionary! { "FT" => "Btn", "T" => text("c"), "Rect" => rect(30.0, 10.0, 12.0, 12.0) };
    form(p, vec![c]);
    form_fonts_mut(p).set("ZaDb", zadb);
    common::seeds::pdf(p);
}

/// A checkbox LimarePDF draws puts its mark in ZapfDingbats, `/ZaDb` in its own drawing, even
/// when the form's `/ZaDb` is another font, which stays as it is (A-05).
#[test]
fn a_check_mark_isnt_drawn_in_another_font() {
    let mut p = page();
    let helv = p.doc.add_object(standard("Helvetica"));
    check_with_zadb(&mut p, helv.into());
    p.set_field("c", FieldValue::Check(true)).unwrap();
    let q = reopened(&mut p);
    for p in [&p, &q] {
        let drawn = drawn_in(p, drawing(p, widget_of(p, "c"), Some("Yes")));
        assert!(!drawn.is_empty());
        for (n, _, font) in drawn {
            assert_eq!((n.as_str(), font), ("ZaDb", standard("ZapfDingbats")));
        }
        let kept = font_of(p, form_fonts(p).get(b"ZaDb").unwrap());
        assert_eq!(kept, standard("Helvetica"));
        assert_eq!(value(p, "c"), FieldValue::Check(true));
    }
}

/// The form's `/ZaDb`, when it is ZapfDingbats, is what a check mark is drawn in (a control).
#[test]
fn the_forms_zapfdingbats_is_used_for_check_marks() {
    let mut p = page();
    let mut zapf = standard("ZapfDingbats");
    zapf.set("Name", "ZaDb");
    let zapf = p.doc.add_object(zapf);
    check_with_zadb(&mut p, zapf.into());
    p.set_field("c", FieldValue::Check(true)).unwrap();
    let q = reopened(&mut p);
    for p in [&p, &q] {
        let drawn = drawn_in(p, drawing(p, widget_of(p, "c"), Some("Yes")));
        assert!(!drawn.is_empty());
        let held = form_fonts(p).get(b"ZaDb").unwrap().clone();
        for (n, entry, _) in drawn {
            assert_eq!((n.as_str(), entry), ("ZaDb", held.clone()));
        }
        assert_eq!(form_fonts(p).len(), 2, "Helv and ZaDb");
    }
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

/// A value that can't be written in full is refused with nothing written (M-09): the value,
/// the drawings and the form as they were, its XFA part too, which goes only when the value is
/// written.
#[test]
fn a_value_that_cant_be_written_changes_nothing() {
    // A text field with two widgets, the first or the second without a rectangle.
    for broken in [0, 1] {
        let mut p = page();
        form(&mut p, vec![]);
        let form = acro_form(&p);
        p.doc
            .get_dictionary_mut(form)
            .unwrap()
            .set("XFA", text("<xdp/>"));
        let mut kids = vec![
            dictionary! { "Rect" => rect(10.0, 150.0, 100.0, 20.0) },
            dictionary! { "Rect" => rect(10.0, 100.0, 100.0, 20.0) },
        ];
        kids[broken].set("Rect", Object::Null);
        let field = dictionary! { "FT" => "Tx", "T" => text("two"), "V" => text("old") };
        let ids = family(&mut p, field, kids);
        common::seeds::pdf(&p);
        let before = p.state_before();
        let e = p.set_field("two", FieldValue::Text("new".into()));
        assert!(e.unwrap_err().to_string().contains("rectangle"));
        assert_eq!(p.state_since(&before), [], "widget {broken} broken");
        assert_eq!(value(&p, "two"), FieldValue::Text("old".into()));
        // Mended, it is written, and the XFA part goes.
        let r = rect(10.0, 100.0, 100.0, 20.0);
        p.doc
            .get_dictionary_mut(ids[broken])
            .unwrap()
            .set("Rect", r);
        p.set_field("two", FieldValue::Text("new".into())).unwrap();
        assert_eq!(value(&p, "two"), FieldValue::Text("new".into()));
        assert!(!p.doc.get_dictionary(form).unwrap().has(b"XFA"));
    }
}

/// A list box whose options hold a character no font has, set to the one that can be written,
/// is refused with nothing written: the list shows them all.
#[test]
fn a_list_that_cant_be_drawn_changes_nothing() {
    let mut p = page();
    form(
        &mut p,
        vec![dictionary! { "FT" => "Ch", "T" => text("list"),
        "Opt" => vec![text("a"), utf16("\u{2FFFE}")], "Rect" => rect(10.0, 60.0, 100.0, 60.0) }],
    );
    common::seeds::pdf(&p);
    let before = p.state_before();
    let e = p.set_field("list", FieldValue::Choice(vec!["a".into()]));
    let e = e.unwrap_err().to_string();
    assert!(e.contains("No font on this computer has"), "{e}");
    assert_eq!(p.state_since(&before), []);
    assert_eq!(value(&p, "list"), FieldValue::Choice(vec![]));
}

/// A reset whose second field of three can't be drawn resets none of them.
#[test]
fn a_reset_that_cant_be_drawn_changes_nothing() {
    let mut p = page();
    form(
        &mut p,
        vec![
            dictionary! { "FT" => "Tx", "T" => text("a"), "V" => text("x"), "Rect" => rect(0.0, 0.0, 80.0, 20.0) },
            dictionary! { "FT" => "Tx", "T" => text("b"), "V" => text("y"), "Rect" => Object::Null },
            dictionary! { "FT" => "Tx", "T" => text("c"), "V" => text("z"), "Rect" => rect(0.0, 60.0, 80.0, 20.0) },
        ],
    );
    common::seeds::pdf(&p);
    let before = p.state_before();
    assert!(p.reset_form().is_err());
    assert_eq!(p.state_since(&before), []);
    assert_eq!(value(&p, "a"), FieldValue::Text("x".into()));
}

/// A choice field under a parent holding "old", as its own widget.
fn under_old(p: &mut Pdf, dv: Option<&str>) -> ObjectId {
    form(p, vec![]);
    let mut parent = dictionary! {
        "FT" => "Ch", "Ff" => 1 << 17, "T" => text("p"), "V" => text("old"),
        "Opt" => vec![text("old"), text("new")],
    };
    if let Some(dv) = dv {
        parent.set("DV", text(dv));
    }
    let child =
        dictionary! { "T" => text("c"), "Rect" => rect(10.0, 150.0, 100.0, 20.0), "F" => 4 };
    let id = family(p, parent, vec![child])[0];
    common::seeds::pdf(p);
    id
}

/// A choice emptied under a parent field holding a value stays empty, rather than the
/// parent's showing through (M-10).
#[test]
fn an_emptied_choice_stays_empty() {
    let mut p = page();
    under_old(&mut p, None);
    assert_eq!(value(&p, "p.c"), FieldValue::Choice(vec!["old".into()]));
    p.set_field("p.c", FieldValue::Choice(vec![])).unwrap();
    assert_eq!(value(&p, "p.c"), FieldValue::Choice(vec![]));
    let bytes = p.to_bytes().unwrap();
    let q = Pdf::from_bytes(&bytes, None).unwrap();
    assert_eq!(value(&q, "p.c"), FieldValue::Choice(vec![]));
    // The saved file holds the child's own empty value, which the parent's can't show through.
    let saved = Saved::new(bytes);
    let fields = saved.fields();
    let child = fields.iter().find(|f| f.name == "p.c").unwrap();
    assert_eq!(child.values, ["/V []"]);
    assert!(!child.appearance.contains("old"), "{}", child.appearance);
}

/// So does one reset to an empty default; one reset to a default it inherits takes it.
#[test]
fn a_choice_reset_to_an_empty_default_stays_empty() {
    for (dv, want) in [(None, vec![]), (Some("new"), vec!["new".to_string()])] {
        let mut p = page();
        under_old(&mut p, dv);
        p.reset_form().unwrap();
        assert_eq!(value(&reopened(&mut p), "p.c"), FieldValue::Choice(want));
    }
}

/// Poppler, drawing the field from its value, shows the emptied choice empty, and the parent's
/// value when the child is left alone or set to it. Opt-in: needs `pdftoppm`.
#[test]
#[ignore]
fn cross_check_cleared_choice_with_poppler() {
    let dir = tool::Scratch::new("cleared");
    let poppler = |set: Option<Vec<String>>, name: &str| {
        let mut p = page();
        under_old(&mut p, None);
        if let Some(v) = set {
            p.set_field("p.c", FieldValue::Choice(v)).unwrap();
        }
        let form = acro_form(&p);
        p.doc
            .get_dictionary_mut(form)
            .unwrap()
            .set("NeedAppearances", true);
        let path = dir.join(format!("{name}.pdf"));
        std::fs::write(&path, p.to_bytes().unwrap()).unwrap();
        let stem = dir.join(name);
        let ok = tool::output(
            std::process::Command::new("pdftoppm")
                .args(["-r", "72", "-singlefile", "-png"])
                .arg(&path)
                .arg(&stem),
        )
        .expect("pdftoppm")
        .status
        .success();
        assert!(ok);
        let img = image::open(stem.with_extension("png")).unwrap().to_rgb8();
        // The field, 10 to 110 across and 30 to 50 down.
        (30..50)
            .flat_map(|y| (10..110).map(move |x| (x, y)))
            .filter(|&(x, y)| img.get_pixel(x, y).0.iter().any(|&c| c < 160))
            .count()
    };
    assert_eq!(poppler(Some(vec![]), "emptied"), 0);
    assert!(poppler(None, "inherited") > 20);
    assert!(poppler(Some(vec!["old".into()]), "set") > 20);
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

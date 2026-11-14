//! Forms with more fields than LimarePDF lists (10,000): setting, resetting and removing
//! values, counting signatures, redaction's report, the pages' pictures and telling
//! XFA-only forms apart still deal with every field.

use limarepdf_core::{FieldValue, Hidden, Pdf};
use lopdf::{dictionary, Dictionary, Object, ObjectId, Stream, StringFormat};

/// How many fields LimarePDF lists.
const LISTED: usize = 10_000;

fn text(s: &str) -> Object {
    Object::string_literal(s)
}

fn rect(r: [i64; 4]) -> Object {
    r.map(Object::from).to_vec().into()
}

/// Two 300 × 200 pt pages with a form of [`LISTED`] empty text fields sharing one drawing,
/// on the first page when `placed` (on none otherwise), then `last`, a field that is its
/// own widget, on the second page. Returns `last`'s object.
fn crowded(last: Dictionary, placed: bool) -> (Pdf, ObjectId) {
    let mut p = Pdf::new();
    p.insert_blank(0, 300.0, 200.0).unwrap();
    p.insert_blank(0, 300.0, 200.0).unwrap();
    let helv = p.doc.add_object(dictionary! {
        "Type" => "Font", "Subtype" => "Type1", "BaseFont" => "Helvetica",
        "Encoding" => "WinAnsiEncoding",
    });
    let look = p.doc.add_object(Stream::new(
        dictionary! { "Type" => "XObject", "Subtype" => "Form", "BBox" => rect([0, 0, 10, 10]) },
        Vec::new(),
    ));
    let mut fields: Vec<Object> = (0..LISTED)
        .map(|i| {
            let d = dictionary! {
                "Type" => "Annot", "Subtype" => "Widget", "FT" => "Tx",
                "T" => text(&format!("f{i}")), "F" => 4, "Rect" => rect([0, 0, 10, 10]),
                "AP" => dictionary! { "N" => look },
            };
            p.doc.add_object(d).into()
        })
        .collect();
    let mut d = dictionary! {
        "Type" => "Annot", "Subtype" => "Widget", "F" => 4, "Rect" => rect([10, 150, 160, 170]),
    };
    for (k, v) in last {
        d.set(k, v);
    }
    let last = p.doc.add_object(d);
    let pages = p.page_ids();
    if placed {
        let first = p.doc.get_dictionary_mut(pages[0]).unwrap();
        first.set("Annots", fields.clone());
    }
    let second = p.doc.get_dictionary_mut(pages[1]).unwrap();
    second.set("Annots", vec![last.into()]);
    fields.push(last.into());
    let form = p.doc.add_object(dictionary! {
        "Fields" => fields,
        "DA" => text("/Helv 10 Tf 0 g"),
        "DR" => dictionary! { "Font" => dictionary! { "Helv" => helv } },
    });
    p.doc.catalog_mut().unwrap().set("AcroForm", form);
    (p, last)
}

/// The text field `last` holding `value`, with no drawing.
fn last_text(value: &str) -> Dictionary {
    dictionary! { "FT" => "Tx", "T" => text("last"), "V" => text(value) }
}

/// `id`'s `/V`, as text.
fn value(p: &Pdf, id: ObjectId) -> Option<String> {
    let d = p.doc.get_dictionary(id).unwrap();
    let v = d.get(b"V").ok()?.as_str().ok()?;
    Some(String::from_utf8_lossy(v).into_owned())
}

/// The content of `id`'s normal appearance, if it has one.
fn drawing(p: &Pdf, id: ObjectId) -> Option<String> {
    let ap = p.doc.get_dictionary(id).unwrap().get(b"AP").ok()?;
    let n = ap.as_dict().unwrap().get(b"N").unwrap().as_reference();
    let s = p.doc.get_object(n.unwrap()).unwrap().as_stream().unwrap();
    let data = s.decompressed_content().unwrap_or(s.content.clone());
    Some(String::from_utf8_lossy(&data).into_owned())
}

fn need_appearances(p: &Pdf) -> bool {
    let form = p.doc.catalog().unwrap().get(b"AcroForm").unwrap();
    let form = p.doc.get_dictionary(form.as_reference().unwrap()).unwrap();
    let asks = form.get(b"NeedAppearances");
    asks.is_ok_and(|o| o.as_bool().unwrap())
}

/// A field past those listed is set by name, as any other. It wasn't found.
#[test]
fn a_field_past_those_listed_is_set_by_name() {
    let (mut p, last) = crowded(last_text(""), true);
    assert_eq!(p.form_fields().len(), LISTED);
    let every = p.every_form_field();
    assert_eq!(every.len(), LISTED + 1);
    assert_eq!(every[LISTED].name, "last");
    p.set_field("last", FieldValue::Text("Jane Doe".into()))
        .unwrap();
    assert_eq!(value(&p, last).as_deref(), Some("Jane Doe"));
    assert!(drawing(&p, last).is_some());
}

/// Reset form puts a field past those listed back to its default too. It was left as it
/// was.
#[test]
fn reset_form_resets_a_field_past_those_listed() {
    let mut d = last_text("filled");
    d.set("DV", text("default"));
    let (mut p, last) = crowded(d, true);
    p.reset_form().unwrap();
    assert_eq!(value(&p, last).as_deref(), Some("default"));
}

/// PB-174: removing form values counts a field past those listed and redraws it empty, as
/// the others. It wasn't counted, and its drawing was removed with the form set to have
/// readers draw every field.
#[test]
fn removing_form_values_redraws_a_field_past_those_listed() {
    let (mut p, last) = crowded(last_text("SECRET"), true);
    let shown = p.doc.add_object(Stream::new(
        dictionary! { "Type" => "XObject", "Subtype" => "Form", "BBox" => rect([0, 0, 150, 20]) },
        b"BT /Helv 10 Tf 2 6 Td (SECRET) Tj ET".to_vec(),
    ));
    let d = p.doc.get_dictionary_mut(last).unwrap();
    d.set("AP", dictionary! { "N" => shown });

    let rep = p.remove_hidden(&[Hidden::FormValues]).unwrap();
    assert_eq!(value(&p, last), None);
    let drawn = drawing(&p, last).expect("redrawn");
    assert!(!drawn.contains("SECRET"), "{drawn}");
    assert!(!need_appearances(&p));
    assert_eq!(rep.count(Hidden::FormValues), 1, "{rep:?}");
}

/// A signature in a field past those listed is counted, so saving warns that it breaks
/// it. It wasn't.
#[test]
fn a_signature_past_those_listed_is_counted() {
    let (mut p, last) = crowded(dictionary! { "FT" => "Sig", "T" => text("last") }, true);
    assert_eq!(p.signature_count(), 0);
    let sig = p.doc.add_object(dictionary! { "Type" => "Sig" });
    p.doc.get_dictionary_mut(last).unwrap().set("V", sig);
    assert_eq!(p.signature_count(), 1);
}

/// Redaction's report names a field past those listed that still holds the removed
/// text. It was left out.
#[test]
fn redaction_names_a_field_past_those_listed_holding_the_text() {
    let (mut p, _) = crowded(last_text("Walrus"), true);
    let font = p.doc.add_object(dictionary! {
        "Type" => "Font", "Subtype" => "Type1", "BaseFont" => "Helvetica",
        "Encoding" => "WinAnsiEncoding",
    });
    let content = p.doc.add_object(Stream::new(
        Dictionary::new(),
        b"BT /F1 12 Tf 20 100 Td (Walrus) Tj ET".to_vec(),
    ));
    let pg = p.page_ids()[0];
    let d = p.doc.get_dictionary_mut(pg).unwrap();
    d.set("Contents", content);
    d.set(
        "Resources",
        dictionary! { "Font" => dictionary! { "F1" => font } },
    );

    let marks = p.find_redaction_marks("walrus").unwrap();
    assert_eq!(marks.len(), 1);
    let rep = p.redact(&marks).unwrap();
    assert_eq!(
        rep.elsewhere,
        [(
            "walrus".to_string(),
            vec!["form field \u{201c}last\u{201d}".to_string()]
        )]
    );
}

/// The pages' pictures (exported, and looked at for blank pages) draw a field past those
/// listed that has no drawing, as the others, and name it when they can't. The page
/// holding only it was taken for blank, and a value they couldn't draw went unnamed.
#[test]
fn page_pictures_draw_a_field_past_those_listed() {
    let (mut p, _) = crowded(last_text("PASTLIMIT"), true);
    assert!(p.unshown_fields(&[1]).is_empty());
    assert_eq!(p.blank_pages(0.0).unwrap(), [0]);

    let mut hebrew = vec![0xFE, 0xFF];
    hebrew.extend("שלום".encode_utf16().flat_map(u16::to_be_bytes));
    let mut d = last_text("");
    d.set("V", Object::String(hebrew, StringFormat::Hexadecimal));
    let (p, _) = crowded(d, true);
    assert_eq!(p.unshown_fields(&[1]), ["last"]);
}

/// A form with XFA whose only fields on a page are past those listed is a hybrid one,
/// filled as an ordinary form, not XFA-only. It was taken for XFA-only.
#[test]
fn a_hybrid_form_shown_only_past_those_listed_is_not_xfa_only() {
    let (mut p, _) = crowded(last_text(""), false);
    let xfa = Stream::new(Dictionary::new(), b"<xdp:xdp/>".to_vec());
    let xfa = p.doc.add_object(xfa);
    let form = p.doc.catalog().unwrap().get(b"AcroForm").unwrap();
    let form = form.as_reference().unwrap();
    p.doc.get_dictionary_mut(form).unwrap().set("XFA", xfa);
    assert!(!p.xfa_only());
}

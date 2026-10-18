//! Flattening made-up forms: the drawing stays where it was, and the fields go.

use limarepdf_core::render::{self, Renderer};
use limarepdf_core::{FieldValue, Pdf};
use lopdf::{dictionary, Dictionary, Object, ObjectId, StringFormat};

fn text(s: &str) -> Object {
    Object::String(s.as_bytes().to_vec(), StringFormat::Literal)
}

fn rect(x: f32, y: f32, w: f32, h: f32) -> Object {
    vec![x.into(), y.into(), (x + w).into(), (y + h).into()].into()
}

/// Adds the widget to the page's `/Annots`.
fn place(p: &mut Pdf, page: usize, w: ObjectId) {
    let pg = p.page_ids()[page];
    let d = p.doc.get_dictionary_mut(pg).unwrap();
    let mut annots = d
        .get(b"Annots")
        .ok()
        .and_then(|a| a.as_array().ok())
        .cloned()
        .unwrap_or_default();
    annots.push(w.into());
    d.set("Annots", annots);
}

/// Two 300 × 200 pages with a form of `fields`, which are their own widgets, all on
/// the first page.
fn doc(fields: Vec<Dictionary>) -> (Pdf, Vec<ObjectId>) {
    let mut p = Pdf::new();
    p.insert_blank(0, 300.0, 200.0).unwrap();
    p.insert_blank(0, 300.0, 200.0).unwrap();
    let ids: Vec<ObjectId> = fields.into_iter().map(|f| p.doc.add_object(f)).collect();
    for &id in &ids {
        place(&mut p, 0, id);
    }
    let helv = dictionary! {
        "Type" => "Font", "Subtype" => "Type1", "BaseFont" => "Helvetica",
        "Encoding" => "WinAnsiEncoding",
    };
    let helv = p.doc.add_object(helv);
    let form = dictionary! {
        "Fields" => ids.iter().map(|&i| i.into()).collect::<Vec<Object>>(),
        "DA" => text("/Helv 0 Tf 0 g"),
        "DR" => dictionary! { "Font" => dictionary! { "Helv" => helv } },
        "CO" => ids.iter().map(|&i| i.into()).collect::<Vec<Object>>(),
    };
    let form = p.doc.add_object(form);
    p.doc.catalog_mut().unwrap().set("AcroForm", form);
    (p, ids)
}

fn widget(extra: Dictionary) -> Dictionary {
    let mut d = dictionary! { "Type" => "Annot", "Subtype" => "Widget" };
    for (k, v) in extra {
        d.set(k, v);
    }
    d
}

/// The box (left, top, right, bottom, in pixels at 1 px per point, y down) holding
/// every dark pixel of a page, or `None` when there are none.
fn ink(p: &mut Pdf, page: usize) -> Option<[usize; 4]> {
    let hp = render::load(p.render_snapshot().unwrap(), None).unwrap();
    let bmp = Renderer::new(&hp).render(page, 1.0).unwrap();
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

fn has_form(p: &Pdf) -> bool {
    p.doc.catalog().unwrap().has(b"AcroForm")
}

fn annots(p: &Pdf, page: usize) -> usize {
    let d = p.doc.get_dictionary(p.page_ids()[page]).unwrap();
    d.get(b"Annots").map_or(0, |a| a.as_array().unwrap().len())
}

#[test]
fn draws_the_fields_where_they_were_and_removes_the_form() {
    let (mut p, _) = doc(vec![
        widget(dictionary! {
            "FT" => "Tx", "T" => text("name"), "Rect" => rect(10.0, 150.0, 150.0, 20.0),
        }),
        // Turned a quarter: its appearance has a /Matrix.
        widget(dictionary! {
            "FT" => "Tx", "T" => text("up"), "Rect" => rect(200.0, 20.0, 20.0, 120.0),
            "MK" => dictionary! { "R" => 90 },
        }),
        // A value with no appearance yet: drawn before flattening.
        widget(dictionary! {
            "FT" => "Tx", "T" => text("bare"), "Rect" => rect(10.0, 20.0, 150.0, 20.0),
            "V" => text("MMMM"),
        }),
        // Hidden: removed, not drawn.
        widget(dictionary! {
            "FT" => "Tx", "T" => text("hidden"), "Rect" => rect(10.0, 80.0, 150.0, 20.0),
            "V" => text("MMMM"), "F" => 2,
        }),
    ]);
    p.set_field("name", FieldValue::Text("MMMM".into()))
        .unwrap();
    p.set_field("up", FieldValue::Text("MMMMMM".into()))
        .unwrap();
    p.refresh_appearances();
    let before = ink(&mut p, 0).expect("fields drawn");

    assert_eq!(p.flatten_form(None).unwrap(), 3);
    assert!(p.form_fields().is_empty());
    assert!(!has_form(&p));
    assert_eq!(annots(&p, 0), 0);

    // Saved and opened again, the page looks the same, with no form.
    let mut q = Pdf::from_bytes(&p.to_bytes().unwrap(), None).unwrap();
    assert!(q.form_fields().is_empty());
    let after = ink(&mut q, 0).expect("still drawn");
    for (a, b) in before.iter().zip(after) {
        assert!(a.abs_diff(b) <= 1, "{before:?} {after:?}");
    }
}

#[test]
fn draws_the_chosen_state_of_a_checkbox() {
    let mark = |p: &mut Pdf, op: &str| {
        let d = dictionary! {
            "Type" => "XObject", "Subtype" => "Form", "BBox" => rect(0.0, 0.0, 20.0, 20.0),
        };
        p.doc
            .add_object(lopdf::Stream::new(d, op.as_bytes().to_vec()))
    };
    let mut p = Pdf::new();
    p.insert_blank(0, 300.0, 200.0).unwrap();
    let on = mark(&mut p, "0 g 0 0 20 20 re f");
    let off = mark(&mut p, "");
    let boxes: Vec<Dictionary> = [(10.0, "Yes"), (100.0, "Off")]
        .into_iter()
        .enumerate()
        .map(|(i, (x, state))| {
            widget(dictionary! {
                "FT" => "Btn", "T" => text(&format!("c{i}")), "Rect" => rect(x, 100.0, 20.0, 20.0),
                "AP" => dictionary! { "N" => dictionary! { "Yes" => on, "Off" => off } },
                "AS" => state, "V" => state,
            })
        })
        .collect();
    let ids: Vec<ObjectId> = boxes.into_iter().map(|b| p.doc.add_object(b)).collect();
    let refs: Vec<Object> = ids.iter().map(|&i| i.into()).collect();
    let pg = p.page_ids()[0];
    p.doc
        .get_dictionary_mut(pg)
        .unwrap()
        .set("Annots", refs.clone());
    let form = p.doc.add_object(dictionary! { "Fields" => refs });
    p.doc.catalog_mut().unwrap().set("AcroForm", form);

    assert_eq!(p.flatten_form(None).unwrap(), 2);
    // Only the ticked box is dark: x 10-30, y 80-100 from the top.
    let b = ink(&mut p, 0).expect("ticked box drawn");
    assert!(
        b[0] >= 9 && b[2] <= 31 && b[1] >= 79 && b[3] <= 101,
        "{b:?}"
    );
}

#[test]
fn flattens_some_pages_and_keeps_the_rest_fillable() {
    // "both" has a widget on each page; "first" is only on the first page.
    let (mut p, ids) = doc(vec![widget(dictionary! {
        "FT" => "Tx", "T" => text("first"), "Rect" => rect(10.0, 150.0, 150.0, 20.0),
    })]);
    let kids = [0, 1].map(|_| {
        let w = widget(dictionary! { "Rect" => rect(10.0, 100.0, 150.0, 20.0) });
        p.doc.add_object(w)
    });
    let both = p.doc.add_object(dictionary! {
        "FT" => "Tx", "T" => text("both"),
        "Kids" => kids.iter().map(|&k| k.into()).collect::<Vec<Object>>(),
    });
    for (page, k) in kids.into_iter().enumerate() {
        p.doc.get_dictionary_mut(k).unwrap().set("Parent", both);
        place(&mut p, page, k);
    }
    let form = p
        .doc
        .catalog()
        .unwrap()
        .get(b"AcroForm")
        .unwrap()
        .as_reference()
        .unwrap();
    let fields: Vec<Object> = vec![ids[0].into(), both.into()];
    p.doc
        .get_dictionary_mut(form)
        .unwrap()
        .set("Fields", fields);
    p.set_field("first", FieldValue::Text("MMMM".into()))
        .unwrap();
    p.set_field("both", FieldValue::Text("MMMM".into()))
        .unwrap();

    assert!(p.flatten_form(Some(&[5])).is_err());
    assert_eq!(p.flatten_form(Some(&[0])).unwrap(), 2);
    assert!(has_form(&p));
    let fields = p.form_fields();
    assert_eq!(fields.len(), 1, "{fields:?}");
    assert_eq!(fields[0].name, "both");
    assert_eq!(fields[0].widgets.len(), 1);
    assert_eq!(fields[0].widgets[0].page, 1);
    assert_eq!(annots(&p, 0), 0);
    assert!(ink(&mut p, 0).is_some());
    // The calculation order loses the removed field.
    let co = p.doc.get_dictionary(form).unwrap().get(b"CO").unwrap();
    assert!(co.as_array().unwrap().is_empty());

    // The rest is still fillable, and flattening it removes the form.
    p.set_field("both", FieldValue::Text("MM".into())).unwrap();
    assert_eq!(p.flatten_form(Some(&[1])).unwrap(), 1);
    assert!(!has_form(&p));
}

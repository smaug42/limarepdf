//! Batch 42's stage 3, journey A: finishing a form. Typing kept by Save and Save as, and a
//! form of every kind of field filled, saved and opened again, with app.rs's helpers.

use super::*;
use limarepdf_core::{FieldKind, FieldValue};
use lopdf::{dictionary, Dictionary, Object, ObjectId};

const NONE: egui::Modifiers = egui::Modifiers::NONE;

fn test_ctx() -> egui::Context {
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    theme::install(&ctx);
    ctx
}

/// Takes in the results of the saves being written, then lets the window run on: the
/// whole window, so a field with the keyboard is drawn in every frame (`saves_done` draws
/// the dialogs alone, and egui lets go of a focused widget missing from a frame).
fn saved(app: &mut App, ty: &Typist) {
    wait_for(app, ty.ctx, |a| a.saves.is_empty());
    ty.pause(app);
}

/// Field `name` of the tab in front still has its text box open, holding `text`, with the
/// keyboard in it.
fn still_typing(app: &mut App, ty: &Typist, name: &str, text: &str) {
    let tree = ty.pause(app);
    let open = app.tabs[app.active].ed.form.typing_in();
    assert_eq!(open, Some((name.to_string(), 0)), "the field was closed");
    assert_eq!(
        focused_text(&tree).as_deref(),
        Some(text),
        "the keyboard isn't in the field"
    );
}

/// Typing in a form field is saved by Save (Ctrl+S) and by Save as (Ctrl+Shift+S) without
/// leaving the field first: each file has the value as typed, the tab counts as saved, the
/// file saved before keeps its own value, and the field stays open with the keyboard after
/// each, so the typing goes on there and is kept when the field is left.
#[test]
fn save_keeps_a_field_still_being_typed_in() {
    let ctx = test_ctx();
    let ty = Typist::new(&ctx);
    let mut app = App::with(Settings::from_file(None), Recent::from_file(None));
    let dir = test_dir("save-typing");
    let (path, copy) = (dir.join("form.pdf"), dir.join("copy.pdf"));
    form_file(&path, "");
    open_and_wait(&mut app, &ctx, &path, 1);
    ty.click_field(&mut app, "Name", 0);
    ty.key(&mut app, Key::End, NONE);
    ty.text(&mut app, "Ann typed");
    ty.key(&mut app, Key::S, CTRL);
    saved(&mut app, &ty);
    assert_eq!(field_in(&path, "Name"), "Ann typed", "Save");
    assert!(!app.tabs[0].unsaved(), "saved, yet unsaved");
    still_typing(&mut app, &ty, "Name", "Ann typed");

    ty.text(&mut app, " more");
    SAVE_AS.with(|s| s.set(Some(copy.clone())));
    ty.key(&mut app, Key::S, CTRL_SHIFT);
    saved(&mut app, &ty);
    assert_eq!(field_in(&copy, "Name"), "Ann typed more", "Save as");
    assert_eq!(field_in(&path, "Name"), "Ann typed", "the first changed");
    let t = &app.tabs[0];
    assert_eq!(t.path.as_deref(), Some(copy.as_path()));
    assert!(!t.unsaved(), "saved as, yet unsaved");
    still_typing(&mut app, &ty, "Name", "Ann typed more");

    ty.text(&mut app, "!");
    ty.key(&mut app, Key::Escape, NONE);
    ty.pause(&mut app);
    assert_eq!(saved_field(&app.tabs[0], "Name"), "Ann typed more!");
    std::fs::remove_dir_all(&dir).unwrap();
}

/// [`mixed_form`]'s text fields, top to bottom, with what is typed in each.
const TYPED: [(&str, &str); 6] = [
    ("First name", "Ann"),
    ("Last name", "Lee"),
    ("Street", "1 Shore Road"),
    ("Town", "Tofino"),
    ("Email", "ann@example.org"),
    ("Phone", "250 555 0100"),
];

/// [`mixed_form`]'s checkboxes, left to right.
const CHECKS: [&str; 3] = ["Newsletter", "Terms", "Member"];

/// Adds a widget for page `page` from `d` (its field's entries, or its parent's), shown and
/// printed.
fn widget(p: &mut Pdf, page: ObjectId, mut d: Dictionary) -> ObjectId {
    d.set("Type", "Annot");
    d.set("Subtype", "Widget");
    d.set("F", 4);
    d.set("P", page);
    p.doc.add_object(d)
}

/// A box `w` by `h` points with its top left at (`x`, `y`), y down on a US Letter page, as
/// a `/Rect`.
fn rect(x: f32, y: f32, w: f32, h: f32) -> Object {
    let top = 792.0 - y;
    vec![x.into(), (top - h).into(), (x + w).into(), top.into()].into()
}

/// A one-page form of 12 fields, all empty and within the test window's view of the page:
/// the text fields of [`TYPED`], one under another from the top; the drop-down "Colour"
/// (Walrus grey, Otter brown, Seal blue) beside the list box "Pets" (Newt, Toad, Frog);
/// the checkboxes of [`CHECKS`] in a row; and the radio group "Size" (S, M and L) in a
/// row under them.
fn mixed_form() -> Pdf {
    let mut p = Pdf::new();
    p.insert_blank(0, 612.0, 792.0).unwrap();
    let page = p.page_ids()[0];
    let text = |s: &str| Object::string_literal(s);
    let states = |on: &str| {
        let n = dictionary! { on => Object::Null, "Off" => Object::Null };
        dictionary! { "N" => n }
    };
    let mut ids = Vec::new();
    for (i, (name, _)) in TYPED.iter().enumerate() {
        let r = rect(72.0, 40.0 + 30.0 * i as f32, 260.0, 22.0);
        let d = dictionary! { "FT" => "Tx", "T" => text(name), "Rect" => r };
        ids.push(widget(&mut p, page, d));
    }
    let opts = |o: [&str; 3]| -> Vec<Object> { o.into_iter().map(text).collect() };
    let colour = dictionary! {
        "FT" => "Ch", "Ff" => 1 << 17, "T" => text("Colour"),
        "Opt" => opts(["Walrus grey", "Otter brown", "Seal blue"]),
        "Rect" => rect(72.0, 230.0, 200.0, 22.0),
    };
    ids.push(widget(&mut p, page, colour));
    let pets = dictionary! {
        "FT" => "Ch", "T" => text("Pets"), "Opt" => opts(["Newt", "Toad", "Frog"]),
        "Rect" => rect(320.0, 230.0, 200.0, 60.0),
    };
    ids.push(widget(&mut p, page, pets));
    for (i, name) in CHECKS.iter().enumerate() {
        let d = dictionary! {
            "FT" => "Btn", "T" => text(name), "AP" => states("Yes"), "AS" => "Off",
            "Rect" => rect(72.0 + 80.0 * i as f32, 310.0, 14.0, 14.0),
        };
        ids.push(widget(&mut p, page, d));
    }
    let size = p.doc.new_object_id();
    let mut fields: Vec<Object> = ids.iter().map(|&id| id.into()).collect();
    fields.push(size.into());
    let mut kids: Vec<Object> = Vec::new();
    for (i, s) in ["S", "M", "L"].into_iter().enumerate() {
        let d = dictionary! {
            "Parent" => size, "AP" => states(s), "AS" => "Off",
            "Rect" => rect(72.0 + 80.0 * i as f32, 340.0, 14.0, 14.0),
        };
        kids.push(widget(&mut p, page, d).into());
    }
    let group = dictionary! {
        "FT" => "Btn", "Ff" => 1 << 15, "T" => text("Size"), "Kids" => kids.clone(),
    };
    p.doc.objects.insert(size, Object::Dictionary(group));
    let annots: Vec<Object> = ids.iter().map(|&id| id.into()).chain(kids).collect();
    let d = p.doc.get_dictionary_mut(page).unwrap();
    d.set("Annots", annots);
    let helv = p.doc.add_object(dictionary! {
        "Type" => "Font", "Subtype" => "Type1", "BaseFont" => "Helvetica",
        "Encoding" => "WinAnsiEncoding",
    });
    let form = dictionary! {
        "Fields" => fields,
        "DA" => text("/Helv 0 Tf 0 g"),
        "DR" => dictionary! { "Font" => dictionary! { "Helv" => helv } },
    };
    p.doc.catalog_mut().unwrap().set("AcroForm", form);
    p
}

/// Every field of `pdf`: its name, kind and value, in the form's order.
fn fields_of(pdf: &Pdf) -> Vec<(String, FieldKind, FieldValue)> {
    (pdf.form_fields().into_iter())
        .map(|f| (f.name, f.kind, f.value))
        .collect()
}

/// [`mixed_form`]'s fields as [`a_mixed_form_is_filled_saved_and_stays_fillable`] fills
/// them, by name: [`TYPED`], Newsletter and Member ticked (Terms not), Size M, Colour
/// Otter brown and Pets Toad.
fn filled() -> Vec<(String, FieldValue)> {
    let pick = |s: &str| FieldValue::Choice(vec![s.to_string()]);
    let mut v: Vec<(String, FieldValue)> = (TYPED.iter())
        .map(|(n, t)| (n.to_string(), FieldValue::Text(t.to_string())))
        .collect();
    v.push(("Colour".into(), pick("Otter brown")));
    v.push(("Pets".into(), pick("Toad")));
    for (n, on) in CHECKS.iter().zip([true, false, true]) {
        v.push((n.to_string(), FieldValue::Check(on)));
    }
    v.push(("Size".into(), pick("M")));
    v
}

/// Click the radio button of group `name` whose choice is `export`.
fn click_radio(app: &mut App, ty: &Typist, name: &str, export: &str) {
    let fields = app.tabs[app.active].pdf.form_fields();
    let f = fields.iter().find(|f| f.name == name).expect(name);
    let w = (f.widgets.iter()).position(|w| w.export.as_deref() == Some(export));
    let w = w.unwrap_or_else(|| panic!("no {export} in {name}"));
    ty.click_field(app, name, w);
}

/// Open the list of choice field `name` with a click, and click its option `option`.
fn choose(app: &mut App, ty: &Typist, name: &str, option: &str) {
    ty.click_field(app, name, 0);
    let tree = ty.pause(app);
    ty.click(app, middle(&tree, option));
}

/// Journey A: a form of 12 fields of every kind filled in through the window, as a person
/// fills one (clicks on the fields, typing, Tab from one text field to the next, the
/// options of a drop-down and a list box clicked), saved as a new file, the tab closed and
/// the file opened again. Every value is as it was filled in, none blank or lost; the form
/// and its fields are still there, of the same kinds and on the page (not flattened); and
/// the reopened tab takes typing and clicks again.
#[test]
fn a_mixed_form_is_filled_saved_and_stays_fillable() {
    let ctx = test_ctx();
    let ty = Typist::new(&ctx);
    let mut app = App::with(Settings::from_file(None), Recent::from_file(None));
    let dir = test_dir("mixed-form");
    let (path, out) = (dir.join("form.pdf"), dir.join("filled.pdf"));
    mixed_form().save(&path).unwrap();
    open_and_wait(&mut app, &ctx, &path, 1);
    let made = fields_of(&app.tabs[0].pdf);
    assert_eq!(made.len(), 12, "{made:?}");

    // The text fields, with Tab from one to the next; Esc keeps the last.
    ty.click_field(&mut app, TYPED[0].0, 0);
    for (i, (name, value)) in TYPED.iter().enumerate() {
        if i > 0 {
            ty.key(&mut app, Key::Tab, NONE);
            ty.pause(&mut app);
        }
        let open = app.tabs[0].ed.form.typing_in();
        assert_eq!(open, Some((name.to_string(), 0)), "Tab to {name}");
        ty.text(&mut app, value);
    }
    ty.key(&mut app, Key::Escape, NONE);
    ty.pause(&mut app);
    choose(&mut app, &ty, "Colour", "Otter brown");
    choose(&mut app, &ty, "Pets", "Toad");
    for name in ["Newsletter", "Member"] {
        ty.click_field(&mut app, name, 0);
    }
    click_radio(&mut app, &ty, "Size", "M");
    let values = |pdf: &Pdf| -> Vec<(String, FieldValue)> {
        (fields_of(pdf).into_iter())
            .map(|(n, _, v)| (n, v))
            .collect()
    };
    let mut want = filled();
    want.sort_by(|a, b| a.0.cmp(&b.0));
    let sorted = |mut v: Vec<(String, FieldValue)>| {
        v.sort_by(|a, b| a.0.cmp(&b.0));
        v
    };
    assert_eq!(sorted(values(&saved_pdf(&app.tabs[0]))), want, "as filled");

    SAVE_AS.with(|s| s.set(Some(out.clone())));
    ty.key(&mut app, Key::S, CTRL_SHIFT);
    saved(&mut app, &ty);
    let back = Pdf::open(&out, None).expect("the saved file");
    assert_eq!(sorted(values(&back)), want, "saved");
    let kinds = |f: &[(String, FieldKind, FieldValue)]| -> Vec<(String, FieldKind)> {
        f.iter().map(|(n, k, _)| (n.clone(), k.clone())).collect()
    };
    assert_eq!(kinds(&fields_of(&back)), kinds(&made), "the fields changed");
    assert!(
        back.doc.catalog().unwrap().get(b"AcroForm").is_ok(),
        "no form"
    );
    let widgets: usize = (back.form_fields().iter()).map(|f| f.widgets.len()).sum();
    assert_eq!(widgets, 14, "widgets lost");

    // Closed and opened again: as saved, and fillable.
    close_by_button(&mut app, &ctx, "filled.pdf");
    assert!(app.tabs.is_empty(), "not closed");
    open_and_wait(&mut app, &ctx, &out, 1);
    // `close_by_button` ran egui's clock past the typist's.
    ty.time.set(ty.time.get() + 10.0);
    assert_eq!(sorted(values(&app.tabs[0].pdf)), want, "reopened");
    ty.click_field(&mut app, "Last name", 0);
    ty.key(&mut app, Key::End, NONE);
    ty.text(&mut app, "-Smith");
    ty.key(&mut app, Key::Escape, NONE);
    ty.pause(&mut app);
    ty.click_field(&mut app, "Terms", 0);
    let again = saved_pdf(&app.tabs[0]);
    let value = |name: &str| {
        let f = fields_of(&again).into_iter().find(|f| f.0 == name);
        f.map(|f| f.2).unwrap_or_else(|| panic!("no {name}"))
    };
    assert_eq!(value("Last name"), FieldValue::Text("Lee-Smith".into()));
    assert_eq!(value("Terms"), FieldValue::Check(true));
    assert_eq!(value("Town"), FieldValue::Text("Tofino".into()));
    std::fs::remove_dir_all(&dir).unwrap();
}

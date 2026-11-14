//! Property tests for the operations that remove things (cycle plan WP-11 step 4, shared with
//! WP-03 step 5): small documents made at random from a seed, each changed by one operation,
//! saved and read again. Then what the operation removes must be gone from the saved file and
//! everything else kept, and an operation that refuses must leave the document as it was.
//!
//! A document has one to four 200×200 pt pages, each a grid of cells 50 pt wide and 25 pt high.
//! Each piece of text is a marker of its own (`WALRUS-001`) drawn in one cell: in a page's
//! content, in a form or in the tile of a tiling pattern filling the cell, forms drawn in forms
//! and shared between places and pages, patterns painted in pages and forms, in layers
//! switched on or off, as a plain, hex or split string, sometimes invisible. Notes with their
//! own drawings and links (to a web address, or running JavaScript) sit in cells too, and
//! JavaScript is also where a page or the document runs it by itself. Form fields hold a value
//! their widgets in cells show, perhaps a default and a script; some sit under a field of
//! their own, and some aren't in the form's list, found only from their widgets. A redaction
//! mark covers whole cells, so what is under it is known from the cells alone.
//!
//! Each test runs a bounded number of cases, seeded 0, 1, 2…; a failure names its seed.
//! `DESTRUCTIVE_CASES=<n>` runs n cases instead, `DESTRUCTIVE_SEED=<seed>` only that one.

mod common;

use common::saved::Saved;
use limarepdf_core::{Hidden, Pdf};
use lopdf::{dictionary, Dictionary, Object, ObjectId, Stream};
use std::collections::BTreeSet;
use std::panic::{catch_unwind, AssertUnwindSafe};

/// Cases each test runs unless `DESTRUCTIVE_CASES` says otherwise.
const CASES: u64 = 64;
/// A page's grid: columns of cells 50 pt wide and rows 25 pt high, row 0 at the bottom.
const COLUMNS: usize = 4;
const ROWS: usize = 8;
const CELL_W: f32 = 50.0;
const CELL_H: f32 = 25.0;
const SIDE: f32 = 200.0;

/// Splitmix64: the same numbers for a seed everywhere.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// From 0 to `n` − 1.
    fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }

    /// True one time in `n`.
    fn one_in(&mut self, n: usize) -> bool {
        self.below(n) == 0
    }
}

/// A cell: its column, then its row from the bottom.
type Cell = (usize, usize);

/// What a marker is in.
#[derive(Clone, Copy, Debug, PartialEq)]
enum What {
    /// A page's or a form's content.
    Text,
    /// The drawing a note has of its own.
    NoteDrawing,
    /// A note's text (`/Contents`).
    NoteText,
    /// A link's web address.
    Address,
    /// JavaScript.
    Script,
    /// A form field's value, which its widgets' drawings show.
    Value,
    /// A form field's default value.
    Default,
}

/// Where a marker is.
#[derive(Clone, Debug)]
enum Place {
    Drawn(Vec<Drawing>),
    /// In the annotation [`Made::annots`] has at this index.
    Annot(usize),
    /// In the form field [`Made::fields`] has at this index, or its widgets' drawings.
    Field(usize),
    /// In the script a page runs when it is opened.
    PageScript(usize),
    /// In a script the document runs by itself, or lists.
    DocScript,
}

/// One place a marker is drawn.
#[derive(Clone, Debug)]
struct Drawing {
    page: usize,
    cell: Cell,
    /// Hidden by a layer switched off, its own or that of what draws it.
    hidden: bool,
}

#[derive(Debug)]
struct Marker {
    name: String,
    what: What,
    place: Place,
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Kind {
    Note,
    Link,
    Widget,
}

struct Annot {
    page: usize,
    cell: Cell,
    kind: Kind,
    /// Hidden by its flags.
    hidden: bool,
    /// Whether the layer it is in is switched on, when it is in one.
    layer: Option<bool>,
}

/// A document made from a seed, and where each marker in it is.
struct Made {
    pdf: Pdf,
    markers: Vec<Marker>,
    annots: Vec<Annot>,
    /// Each form field's widgets, by their index in `annots`.
    fields: Vec<Vec<usize>>,
}

/// Something a page's or a form's content draws at a cell of it, perhaps in a layer.
struct Item {
    cell: Cell,
    layer: Option<usize>,
    draws: Draws,
}

enum Draws {
    /// A marker, written as `style` picks.
    Text { marker: usize, style: usize },
    /// A form, by its index.
    Form(usize),
    /// The cell filled with a tiling pattern, by its index, or the marker's text painted
    /// with it.
    Pattern(usize, Option<usize>),
}

struct Form {
    /// In cells.
    size: Cell,
    /// The layer the form itself is in.
    layer: Option<usize>,
    items: Vec<Item>,
}

/// A tiling pattern whose tile is a cell, each tile drawing `items`: text, in cell (0, 0).
struct Pattern {
    items: Vec<Item>,
}

fn new_marker(markers: &mut Vec<Marker>, what: What, place: Place) -> usize {
    let name = format!("WALRUS-{:03}", markers.len());
    markers.push(Marker { name, what, place });
    markers.len() - 1
}

/// Something to draw in an area `size` cells big: text, or a form made earlier that fits.
fn item(r: &mut Rng, size: Cell, forms: &[Form], markers: &mut Vec<Marker>, layers: usize) -> Item {
    let layer = r.one_in(3).then(|| r.below(layers));
    let fits: Vec<usize> = (0..forms.len())
        .filter(|&f| forms[f].size.0 <= size.0 && forms[f].size.1 <= size.1)
        .collect();
    if !fits.is_empty() && r.one_in(2) {
        let f = fits[r.below(fits.len())];
        let cell = (
            r.below(size.0 - forms[f].size.0 + 1),
            r.below(size.1 - forms[f].size.1 + 1),
        );
        Item {
            cell,
            layer,
            draws: Draws::Form(f),
        }
    } else {
        let marker = new_marker(markers, What::Text, Place::Drawn(Vec::new()));
        Item {
            cell: (r.below(size.0), r.below(size.1)),
            layer,
            draws: Draws::Text {
                marker,
                style: r.below(4),
            },
        }
    }
}

/// Marks form `f` and the forms it draws as drawn.
fn mark_drawn(f: usize, forms: &[Form], drawn: &mut [bool]) {
    if !drawn[f] {
        drawn[f] = true;
        for item in &forms[f].items {
            if let Draws::Form(g) = item.draws {
                mark_drawn(g, forms, drawn);
            }
        }
    }
}

/// Records where each marker `items` draw is drawn, `from` the cell, page and state given.
fn record(
    items: &[Item],
    forms: &[Form],
    patterns: &[Pattern],
    layers: &[bool],
    from: &Drawing,
    markers: &mut [Marker],
) {
    for item in items {
        let mut at = Drawing {
            cell: (from.cell.0 + item.cell.0, from.cell.1 + item.cell.1),
            hidden: from.hidden || item.layer.is_some_and(|l| !layers[l]),
            ..*from
        };
        if let Draws::Text { marker, .. } | Draws::Pattern(_, Some(marker)) = item.draws {
            if let Place::Drawn(list) = &mut markers[marker].place {
                list.push(at.clone());
            }
        }
        match item.draws {
            Draws::Text { .. } => {}
            Draws::Form(f) => {
                let form = &forms[f];
                at.hidden |= form.layer.is_some_and(|l| !layers[l]);
                record(&form.items, forms, patterns, layers, &at, markers);
            }
            Draws::Pattern(p, _) => {
                record(&patterns[p].items, forms, patterns, layers, &at, markers)
            }
        }
    }
}

/// The content drawing `items`, each in its cell; text in a saved state, as `3 Tr` lasts.
fn content(items: &[Item], markers: &[Marker]) -> String {
    let mut out = String::new();
    for item in items {
        let (x, y) = (item.cell.0 as f32 * CELL_W, item.cell.1 as f32 * CELL_H);
        if let Some(l) = item.layer {
            out += &format!("/OC /L{l} BDC ");
        }
        match item.draws {
            Draws::Text { marker, style } => {
                let name = &markers[marker].name;
                let shown = match style {
                    0 => format!("({name}) Tj"),
                    1 => format!("<{}> Tj", hex(name)),
                    2 => {
                        let (a, b) = name.split_at(4);
                        format!("[({a}) -20 ({b})] TJ")
                    }
                    _ => format!("3 Tr ({name}) Tj"),
                };
                out += &format!("q BT /F1 6 Tf {} {} Td {shown} ET Q ", x + 4.0, y + 9.0);
            }
            Draws::Form(f) => out += &format!("q 1 0 0 1 {x} {y} cm /Fm{f} Do Q "),
            Draws::Pattern(p, None) => {
                out += &format!("q /Pattern cs /P{p} scn {x} {y} {CELL_W} {CELL_H} re f Q ")
            }
            Draws::Pattern(p, Some(marker)) => {
                let name = &markers[marker].name;
                out += &format!(
                    "q /Pattern cs /P{p} scn BT /F1 6 Tf {} {} Td ({name}) Tj ET Q ",
                    x + 4.0,
                    y + 9.0
                );
            }
        }
        if item.layer.is_some() {
            out += "EMC ";
        }
    }
    out
}

/// The resources for content drawing `items`: the font, every layer, and the forms and
/// patterns it draws.
fn resources(
    items: &[Item],
    font: ObjectId,
    ocgs: &[ObjectId],
    forms: &[ObjectId],
    patterns: &[ObjectId],
) -> Dictionary {
    let (mut xobjects, mut fills) = (Dictionary::new(), Dictionary::new());
    for item in items {
        match item.draws {
            Draws::Form(f) => xobjects.set(format!("Fm{f}"), forms[f]),
            Draws::Pattern(p, _) => fills.set(format!("P{p}"), patterns[p]),
            Draws::Text { .. } => {}
        }
    }
    let mut properties = Dictionary::new();
    for (l, &id) in ocgs.iter().enumerate() {
        properties.set(format!("L{l}"), id);
    }
    dictionary! {
        "Font" => dictionary! { "F1" => font }, "XObject" => xobjects, "Pattern" => fills,
        "Properties" => properties,
    }
}

fn hex(s: &str) -> String {
    s.bytes().map(|b| format!("{b:02X}")).collect()
}

fn s(t: &str) -> Object {
    Object::string_literal(t)
}

fn rect(r: [f32; 4]) -> Object {
    r.into_iter().map(Object::Real).collect::<Vec<_>>().into()
}

/// A JavaScript action showing `marker`.
fn script(marker: &str) -> Dictionary {
    dictionary! { "S" => "JavaScript", "JS" => s(&format!("app.alert('{marker}');")) }
}

/// The document `seed` makes.
fn make(seed: u64) -> Made {
    let mut r = Rng(seed);
    let mut markers = Vec::new();
    let pages = 1 + r.below(4);
    let layers: Vec<bool> = (0..1 + r.below(3)).map(|_| r.one_in(2)).collect();

    // Forms, each drawing only forms made before it, then what the pages draw.
    let mut forms: Vec<Form> = Vec::new();
    for _ in 0..r.below(5) {
        let size = (1 + r.below(2), 1 + r.below(3));
        let layer = r.one_in(4).then(|| r.below(layers.len()));
        let mut items = Vec::new();
        for _ in 0..1 + r.below(3) {
            items.push(item(&mut r, size, &forms, &mut markers, layers.len()));
        }
        forms.push(Form { size, layer, items });
    }
    let mut page_items: Vec<Vec<Item>> = Vec::new();
    for _ in 0..pages {
        let mut items = Vec::new();
        for _ in 0..2 + r.below(5) {
            items.push(item(
                &mut r,
                (COLUMNS, ROWS),
                &forms,
                &mut markers,
                layers.len(),
            ));
        }
        page_items.push(items);
    }
    // A form nothing draws is drawn on a page; the last first, as it can draw the others.
    let mut drawn = vec![false; forms.len()];
    for item in page_items.iter().flatten() {
        if let Draws::Form(f) = item.draws {
            mark_drawn(f, &forms, &mut drawn);
        }
    }
    for f in (0..forms.len()).rev() {
        if !drawn[f] {
            let cell = (
                r.below(COLUMNS - forms[f].size.0 + 1),
                r.below(ROWS - forms[f].size.1 + 1),
            );
            page_items[r.below(pages)].push(Item {
                cell,
                layer: None,
                draws: Draws::Form(f),
            });
            mark_drawn(f, &forms, &mut drawn);
        }
    }
    // Tiling patterns, painted in cells of pages and forms, and the form fields below, from
    // numbers of their own: what the rest of a seed makes is as it was without them.
    let mut more = Rng(seed ^ 0xA5A5_A5A5_A5A5_A5A5);
    let mut patterns: Vec<Pattern> = Vec::new();
    for _ in 0..more.below(3) {
        let mut items = Vec::new();
        for _ in 0..1 + more.below(2) {
            let layer = more.one_in(3).then(|| more.below(layers.len()));
            let marker = new_marker(&mut markers, What::Text, Place::Drawn(Vec::new()));
            let draws = Draws::Text {
                marker,
                style: more.below(4),
            };
            items.push(Item {
                cell: (0, 0),
                layer,
                draws,
            });
        }
        patterns.push(Pattern { items });
    }
    for p in 0..patterns.len() {
        for _ in 0..1 + more.below(2) {
            let layer = more.one_in(3).then(|| more.below(layers.len()));
            let (size, items) = if !forms.is_empty() && more.one_in(3) {
                let f = more.below(forms.len());
                (forms[f].size, &mut forms[f].items)
            } else {
                ((COLUMNS, ROWS), &mut page_items[more.below(pages)])
            };
            let cell = (more.below(size.0), more.below(size.1));
            let text = (more.one_in(2))
                .then(|| new_marker(&mut markers, What::Text, Place::Drawn(Vec::new())));
            items.push(Item {
                cell,
                layer,
                draws: Draws::Pattern(p, text),
            });
        }
    }
    for (page, items) in page_items.iter().enumerate() {
        let start = Drawing {
            page,
            cell: (0, 0),
            hidden: false,
        };
        record(items, &forms, &patterns, &layers, &start, &mut markers);
    }

    let mut pdf = Pdf::new();
    for i in 0..pages {
        pdf.insert_blank(i, SIDE, SIDE).unwrap();
    }
    let page_ids = pdf.page_ids();
    let font = pdf.doc.add_object(dictionary! {
        "Type" => "Font", "Subtype" => "Type1", "BaseFont" => "Helvetica", "Encoding" => "WinAnsiEncoding",
    });
    let ocgs: Vec<ObjectId> = (0..layers.len())
        .map(|l| {
            pdf.doc
                .add_object(dictionary! { "Type" => "OCG", "Name" => s(&format!("Layer {l}")) })
        })
        .collect();
    let off: Vec<Object> = (ocgs.iter().zip(&layers))
        .filter(|(_, on)| !**on)
        .map(|(&id, _)| id.into())
        .collect();
    pdf.doc.catalog_mut().unwrap().set(
        "OCProperties",
        dictionary! {
            "OCGs" => ocgs.iter().map(|&id| id.into()).collect::<Vec<Object>>(),
            "D" => dictionary! { "OFF" => off },
        },
    );
    let mut pattern_ids: Vec<ObjectId> = Vec::new();
    for pattern in &patterns {
        let dict = dictionary! {
            "Type" => "Pattern", "PatternType" => 1, "PaintType" => 1, "TilingType" => 1,
            "BBox" => rect([0.0, 0.0, CELL_W, CELL_H]), "XStep" => CELL_W, "YStep" => CELL_H,
            "Resources" => resources(&pattern.items, font, &ocgs, &[], &[]),
        };
        let body = content(&pattern.items, &markers).into_bytes();
        pattern_ids.push(pdf.doc.add_object(Stream::new(dict, body)));
    }
    let mut form_ids: Vec<ObjectId> = Vec::new();
    for form in &forms {
        let (w, h) = (form.size.0 as f32 * CELL_W, form.size.1 as f32 * CELL_H);
        let mut dict = dictionary! {
            "Type" => "XObject", "Subtype" => "Form", "BBox" => rect([0.0, 0.0, w, h]),
            "Resources" => resources(&form.items, font, &ocgs, &form_ids, &pattern_ids),
        };
        if let Some(l) = form.layer {
            dict.set("OC", ocgs[l]);
        }
        let body = content(&form.items, &markers).into_bytes();
        form_ids.push(pdf.doc.add_object(Stream::new(dict, body)));
    }
    for (items, &page) in page_items.iter().zip(&page_ids) {
        let body = content(items, &markers).into_bytes();
        let c = pdf.doc.add_object(Stream::new(Dictionary::new(), body));
        let res = resources(items, font, &ocgs, &form_ids, &pattern_ids);
        let d = pdf.doc.get_dictionary_mut(page).unwrap();
        d.set("Contents", c);
        d.set("Resources", res);
    }

    // Notes and links in cells, and the scripts pages and the document run by themselves.
    let mut annots = Vec::new();
    for (page, &page_id) in page_ids.iter().enumerate() {
        let mut list: Vec<Object> = Vec::new();
        for _ in 0..r.below(4) {
            let cell = (r.below(COLUMNS), r.below(ROWS));
            let (x, y) = (cell.0 as f32 * CELL_W, cell.1 as f32 * CELL_H);
            let place = Place::Annot(annots.len());
            let layer = r.one_in(4).then(|| r.below(layers.len()));
            let (kind, hidden, mut dict) = if r.one_in(2) {
                let hidden = r.one_in(4);
                let drawing = new_marker(&mut markers, What::NoteDrawing, place.clone());
                let text = new_marker(&mut markers, What::NoteText, place);
                let ap = pdf.doc.add_object(Stream::new(
                    dictionary! {
                        "Type" => "XObject", "Subtype" => "Form", "BBox" => rect([0.0, 0.0, 46.0, 21.0]),
                        "Resources" => dictionary! { "Font" => dictionary! { "F1" => font } },
                    },
                    format!("BT /F1 6 Tf 2 7 Td ({}) Tj ET", markers[drawing].name).into_bytes(),
                ));
                let note = dictionary! {
                    "Type" => "Annot", "Subtype" => "Square", "F" => if hidden { 6 } else { 4 },
                    "Contents" => s(&markers[text].name), "AP" => dictionary! { "N" => ap },
                };
                (Kind::Note, hidden, note)
            } else {
                let action = if r.one_in(3) {
                    let code = new_marker(&mut markers, What::Script, place);
                    script(&markers[code].name)
                } else {
                    let address = new_marker(&mut markers, What::Address, place.clone());
                    let mut uri = dictionary! {
                        "S" => "URI", "URI" => s(&format!("https://example.org/{}", markers[address].name)),
                    };
                    if r.one_in(3) {
                        let next = new_marker(&mut markers, What::Script, place);
                        uri.set("Next", script(&markers[next].name));
                    }
                    uri
                };
                let action: Object = if r.one_in(2) {
                    pdf.doc.add_object(action).into()
                } else {
                    action.into()
                };
                let link = dictionary! { "Type" => "Annot", "Subtype" => "Link", "A" => action };
                (Kind::Link, false, link)
            };
            dict.set("Rect", rect([x + 2.0, y + 2.0, x + 48.0, y + 23.0]));
            dict.set("P", page_id);
            if let Some(l) = layer {
                dict.set("OC", ocgs[l]);
            }
            list.push(pdf.doc.add_object(dict).into());
            annots.push(Annot {
                page,
                cell,
                kind,
                hidden,
                layer: layer.map(|l| layers[l]),
            });
        }
        let opens = r
            .one_in(3)
            .then(|| new_marker(&mut markers, What::Script, Place::PageScript(page)));
        let d = pdf.doc.get_dictionary_mut(page_id).unwrap();
        if !list.is_empty() {
            d.set("Annots", list);
        }
        if let Some(m) = opens {
            d.set("AA", dictionary! { "O" => script(&markers[m].name) });
        }
    }
    if r.one_in(2) {
        let m = new_marker(&mut markers, What::Script, Place::DocScript);
        let code = format!("app.alert('{}');", markers[m].name);
        let js: Object = if r.one_in(2) {
            s(&code)
        } else {
            let stream = Stream::new(Dictionary::new(), code.into_bytes());
            pdf.doc.add_object(stream).into()
        };
        let open = dictionary! { "S" => "JavaScript", "JS" => js };
        pdf.doc.catalog_mut().unwrap().set("OpenAction", open);
    }
    if r.one_in(2) {
        let m = new_marker(&mut markers, What::Script, Place::DocScript);
        let action = pdf.doc.add_object(script(&markers[m].name));
        let names = dictionary! { "JavaScript" => dictionary! { "Names" => vec![s("Opening"), action.into()] } };
        pdf.doc.catalog_mut().unwrap().set("Names", names);
    }

    // Text fields, each with a widget or more in cells; one with a single widget is often
    // the widget itself.
    let mut fields = Vec::new();
    let mut listed: Vec<Object> = Vec::new();
    for i in 0..more.below(4) {
        let value = new_marker(&mut markers, What::Value, Place::Field(i));
        let mut field = dictionary! {
            "FT" => "Tx", "T" => s(&format!("f{i}")), "V" => s(&markers[value].name),
        };
        if more.one_in(3) {
            let m = new_marker(&mut markers, What::Default, Place::Field(i));
            field.set("DV", s(&markers[m].name));
        }
        if more.one_in(4) {
            let m = new_marker(&mut markers, What::Script, Place::Field(i));
            field.set("AA", dictionary! { "K" => script(&markers[m].name) });
        }
        let mut widgets = Vec::new();
        for _ in 0..1 + more.below(3) {
            let (page, cell) = (more.below(pages), (more.below(COLUMNS), more.below(ROWS)));
            let (x, y) = (cell.0 as f32 * CELL_W, cell.1 as f32 * CELL_H);
            let hidden = more.one_in(5);
            let layer = more.one_in(4).then(|| more.below(layers.len()));
            let ap = pdf.doc.add_object(Stream::new(
                dictionary! {
                    "Type" => "XObject", "Subtype" => "Form", "BBox" => rect([0.0, 0.0, 46.0, 21.0]),
                    "Resources" => dictionary! { "Font" => dictionary! { "F1" => font } },
                },
                format!("/Tx BMC BT /F1 6 Tf 2 7 Td ({}) Tj ET EMC", markers[value].name).into_bytes(),
            ));
            let mut widget = dictionary! {
                "Type" => "Annot", "Subtype" => "Widget", "F" => if hidden { 6 } else { 4 },
                "Rect" => rect([x + 2.0, y + 2.0, x + 48.0, y + 23.0]), "P" => page_ids[page],
                "AP" => dictionary! { "N" => ap },
            };
            if let Some(l) = layer {
                widget.set("OC", ocgs[l]);
            }
            widgets.push((page_ids[page], widget));
            annots.push(Annot {
                page,
                cell,
                kind: Kind::Widget,
                hidden,
                layer: layer.map(|l| layers[l]),
            });
        }
        fields.push((annots.len() - widgets.len()..annots.len()).collect());
        // Under a field of no value of its own, sometimes.
        let parent = more.one_in(3).then(|| pdf.doc.new_object_id());
        if let Some(parent) = parent {
            field.set("Parent", parent);
        }
        let id = if widgets.len() == 1 && more.one_in(2) {
            let (page, widget) = widgets.pop().unwrap();
            field.extend(&widget);
            let id = pdf.doc.add_object(field);
            add_annot(&mut pdf, page, id);
            id
        } else {
            let id = pdf.doc.new_object_id();
            let mut kids: Vec<Object> = Vec::new();
            for (page, mut widget) in widgets {
                widget.set("Parent", id);
                let w = pdf.doc.add_object(widget);
                add_annot(&mut pdf, page, w);
                kids.push(w.into());
            }
            field.set("Kids", kids);
            pdf.doc.objects.insert(id, field.into());
            id
        };
        if let Some(parent) = parent {
            let kids: Vec<Object> = vec![id.into()];
            let node = dictionary! { "T" => s(&format!("p{i}")), "Kids" => kids };
            pdf.doc.objects.insert(parent, node.into());
        }
        // In the form's list, or found only from its widgets.
        if !more.one_in(6) {
            listed.push(parent.unwrap_or(id).into());
        }
    }
    if !fields.is_empty() {
        let form = dictionary! {
            "Fields" => listed, "DA" => s("/F1 6 Tf 0 g"),
            "DR" => dictionary! { "Font" => dictionary! { "F1" => font } },
        };
        let form: Object = if more.one_in(2) {
            pdf.doc.add_object(form).into()
        } else {
            form.into()
        };
        pdf.doc.catalog_mut().unwrap().set("AcroForm", form);
    }
    Made {
        pdf,
        markers,
        annots,
        fields,
    }
}

/// Adds annotation `annot` to the end of `page`'s list.
fn add_annot(pdf: &mut Pdf, page: ObjectId, annot: ObjectId) {
    let d = pdf.doc.get_dictionary_mut(page).unwrap();
    let mut list = (d.get(b"Annots").and_then(Object::as_array).cloned()).unwrap_or_default();
    list.push(annot.into());
    d.set("Annots", list);
}

/// The document `seed` makes, checked first: saved as it is, it keeps every marker.
fn made(seed: u64) -> Made {
    let made = make(seed);
    let pages = made.pdf.page_count();
    check(&mut made.pdf.clone(), &made.markers, pages, |_| Some(false));
    made
}

/// Runs `case` with each seed (see the notes at the top), naming the seed of a failure. With
/// only one seed, its markers are listed first.
fn each_case(operation: &str, case: impl Fn(u64)) {
    let seeds: Vec<u64> = match std::env::var("DESTRUCTIVE_SEED") {
        Ok(s) => {
            let seed = s
                .parse()
                .unwrap_or_else(|_| panic!("DESTRUCTIVE_SEED is a whole number, not {s:?}"));
            for m in make(seed).markers {
                eprintln!("{m:?}");
            }
            vec![seed]
        }
        Err(_) => (0..cases()).collect(),
    };
    for seed in seeds {
        if let Err(e) = catch_unwind(AssertUnwindSafe(|| case(seed))) {
            let why = (e.downcast_ref::<String>().map(String::as_str))
                .or_else(|| e.downcast_ref::<&str>().copied())
                .unwrap_or("a panic");
            panic!(
                "{operation}, seed {seed}: {why}\n(DESTRUCTIVE_SEED={seed} runs only this case)"
            );
        }
    }
}

fn cases() -> u64 {
    match std::env::var("DESTRUCTIVE_CASES") {
        Ok(n) => match n.parse() {
            Ok(n) if n > 0 => n,
            _ => panic!("DESTRUCTIVE_CASES is a whole number above 0, not {n:?}"),
        },
        Err(_) => CASES,
    }
}

/// `operation` applied to `pdf`. None of these documents is refused; a refusal is checked to
/// have changed nothing all the same before it fails the case.
fn change<T>(pdf: &mut Pdf, operation: impl FnOnce(&mut Pdf) -> limarepdf_core::Result<T>) -> T {
    let before = pdf.state_before();
    match operation(pdf) {
        Ok(t) => t,
        Err(e) => {
            assert_eq!(
                pdf.state_since(&before),
                [],
                "refused ({e:#}), with changes"
            );
            panic!("refused: {e:#}");
        }
    }
}

/// `pdf` saved, opened again and checked: `pages` pages, nothing that isn't reachable or
/// refers to a page that isn't there, and each marker gone (`Some(true)`), kept
/// (`Some(false)`) or either (`None`) as `expect` says.
fn check(
    pdf: &mut Pdf,
    markers: &[Marker],
    pages: usize,
    expect: impl Fn(&Marker) -> Option<bool>,
) -> Saved {
    let saved = Saved::of(pdf);
    let again = Pdf::from_bytes(&saved.bytes, None).expect("the saved file opens");
    assert_eq!(again.page_count(), pages);
    assert_eq!(saved.stray_pages(), Vec::<String>::new());
    assert_eq!(saved.unreachable(), []);
    let (mut gone, mut kept) = (Vec::new(), Vec::new());
    for m in markers {
        match expect(m) {
            Some(true) => gone.push(m.name.as_str()),
            Some(false) => kept.push(m.name.as_str()),
            None => {}
        }
    }
    saved.assert_gone(&gone);
    saved.assert_kept(&kept);
    saved
}

/// Some of the `pages` (one at least), or all of them (`None`), as `r` picks.
fn some_pages(r: &mut Rng, pages: usize) -> Option<Vec<usize>> {
    (!r.one_in(2)).then(|| {
        let on: Vec<usize> = (0..pages).filter(|_| r.one_in(2)).collect();
        if on.is_empty() {
            vec![r.below(pages)]
        } else {
            on
        }
    })
}

#[test]
fn redaction_removes_what_is_under_its_marks_and_keeps_the_rest() {
    each_case("redact", |seed| {
        let Made {
            mut pdf,
            markers,
            annots,
            fields,
        } = made(seed);
        let mut r = Rng(!seed);
        let pages = pdf.page_count();
        let mut marked = BTreeSet::new();
        for page in 0..pages {
            for c in 0..COLUMNS {
                for row in 0..ROWS {
                    if r.one_in(4) {
                        marked.insert((page, (c, row)));
                    }
                }
            }
        }
        // In displayed-page points, y down: inside the cell, so they touch nothing next to it.
        let marks: Vec<(usize, [f32; 4])> = (marked.iter())
            .map(|&(page, (c, row))| {
                let (x, top) = (c as f32 * CELL_W, SIDE - (row + 1) as f32 * CELL_H);
                (
                    page,
                    [x + 1.0, top + 1.0, x + CELL_W - 1.0, top + CELL_H - 1.0],
                )
            })
            .collect();
        change(&mut pdf, |p| p.redact(&marks));
        let under = |a: usize| marked.contains(&(annots[a].page, annots[a].cell));
        check(&mut pdf, &markers, pages, |m| {
            Some(match &m.place {
                Place::Drawn(at) => (at.iter()).all(|d| marked.contains(&(d.page, d.cell))),
                Place::Annot(a) => under(*a),
                Place::Field(f) => fields[*f].iter().all(|&a| under(a)),
                Place::PageScript(_) | Place::DocScript => false,
            })
        });
    });
}

#[test]
fn removing_hidden_layers_removes_only_what_they_alone_hold() {
    each_case("remove hidden layers", |seed| {
        let Made {
            mut pdf,
            markers,
            annots,
            fields,
        } = made(seed);
        let pages = pdf.page_count();
        change(&mut pdf, |p| p.remove_hidden(&[Hidden::HiddenLayers]));
        let off = |a: usize| annots[a].layer == Some(false);
        let saved = check(&mut pdf, &markers, pages, |m| match &m.place {
            Place::Drawn(at) => Some(at.iter().all(|d| d.hidden)),
            Place::Annot(a) => Some(off(*a)),
            Place::Field(f) => Some(fields[*f].iter().all(|&a| off(a))),
            Place::PageScript(_) | Place::DocScript => Some(false),
        });
        assert_eq!(saved.hidden_layer_users(), Vec::<String>::new());
    });
}

#[test]
fn removing_scripts_removes_every_script_and_keeps_the_rest() {
    each_case("remove scripts", |seed| {
        let Made {
            mut pdf, markers, ..
        } = made(seed);
        let pages = pdf.page_count();
        change(&mut pdf, |p| p.remove_hidden(&[Hidden::Scripts]));
        let saved = check(&mut pdf, &markers, pages, |m| Some(m.what == What::Script));
        assert_eq!(saved.actions(), Vec::<String>::new());
    });
}

#[test]
fn deleting_pages_removes_what_only_they_hold() {
    each_case("delete pages", |seed| {
        let Made {
            mut pdf,
            markers,
            annots,
            fields,
        } = made(seed);
        let mut r = Rng(!seed);
        let pages = pdf.page_count();
        if pages == 1 {
            let before = pdf.state_before();
            assert!(pdf.delete_pages(&[0]).is_err(), "the only page was deleted");
            assert_eq!(pdf.state_since(&before), []);
            return;
        }
        let mut gone: BTreeSet<usize> = (0..pages).filter(|_| r.one_in(2)).collect();
        if gone.is_empty() {
            gone.insert(r.below(pages));
        } else if gone.len() == pages {
            gone.remove(&r.below(pages));
        }
        let list: Vec<usize> = gone.iter().copied().collect();
        change(&mut pdf, |p| p.delete_pages(&list));
        check(&mut pdf, &markers, pages - gone.len(), |m| {
            Some(match &m.place {
                Place::Drawn(at) => at.iter().all(|d| gone.contains(&d.page)),
                Place::Annot(a) => gone.contains(&annots[*a].page),
                Place::Field(f) => fields[*f].iter().all(|&a| gone.contains(&annots[a].page)),
                Place::PageScript(page) => gone.contains(page),
                Place::DocScript => false,
            })
        });
    });
}

#[test]
fn extracting_pages_keeps_only_what_they_hold() {
    each_case("extract pages", |seed| {
        let Made {
            pdf,
            markers,
            annots,
            fields,
        } = made(seed);
        let mut r = Rng(!seed);
        let pages = pdf.page_count();
        let mut keep: Vec<usize> = (0..pages).filter(|_| r.one_in(2)).collect();
        if keep.is_empty() {
            keep.push(r.below(pages));
        }
        if r.one_in(2) {
            keep.reverse();
        }
        let before = pdf.state_before();
        let mut out = pdf
            .extract(&keep)
            .unwrap_or_else(|e| panic!("refused: {e:#}"));
        assert_eq!(
            pdf.state_since(&before),
            [],
            "the document taken from changed"
        );
        // What the document runs by itself isn't any page's: either way.
        check(&mut out, &markers, keep.len(), |m| match &m.place {
            Place::Drawn(at) => Some(at.iter().all(|d| !keep.contains(&d.page))),
            Place::Annot(a) => Some(!keep.contains(&annots[*a].page)),
            Place::Field(f) => Some(fields[*f].iter().all(|&a| !keep.contains(&annots[a].page))),
            Place::PageScript(page) => Some(!keep.contains(page)),
            Place::DocScript => None,
        });
    });
}

#[test]
fn flattening_notes_keeps_what_they_show() {
    each_case("flatten annotations", |seed| {
        let Made {
            mut pdf,
            markers,
            annots,
            ..
        } = made(seed);
        let pages = pdf.page_count();
        let on = some_pages(&mut Rng(!seed), pages);
        let flattened = |page: usize| on.as_ref().is_none_or(|on| on.contains(&page));
        let done = change(&mut pdf, |p| p.flatten_annotations(on.as_deref()));
        assert_eq!(done.kept, [], "every note has a drawing to draw");
        // A hidden note goes undrawn; how a note in a layer is drawn, and whether a note's
        // text is kept anywhere once it is drawn, aren't settled here.
        check(&mut pdf, &markers, pages, |m| {
            let Place::Annot(a) = &m.place else {
                return Some(false);
            };
            let a = &annots[*a];
            if a.kind == Kind::Link || !flattened(a.page) {
                return Some(false);
            }
            match (m.what, a.hidden, a.layer) {
                (_, true, _) => Some(true),
                (What::NoteDrawing, false, None) => Some(false),
                _ => None,
            }
        });
    });
}

#[test]
fn flattening_the_form_keeps_what_its_widgets_show() {
    each_case("flatten form", |seed| {
        let Made {
            mut pdf,
            markers,
            annots,
            fields,
        } = made(seed);
        let pages = pdf.page_count();
        let on = some_pages(&mut Rng(!seed), pages);
        let flattened = |page: usize| on.as_ref().is_none_or(|on| on.contains(&page));
        change(&mut pdf, |p| p.flatten_form(on.as_deref()));
        // A field with a widget left stays as it was. One without goes, its value kept where
        // a widget drew it; a hidden widget goes undrawn, and how one in a layer is drawn
        // isn't settled here.
        let saved = check(&mut pdf, &markers, pages, |m| {
            let Place::Field(f) = &m.place else {
                return Some(false);
            };
            let widgets: Vec<&Annot> = fields[*f].iter().map(|&a| &annots[a]).collect();
            if widgets.iter().any(|w| !flattened(w.page)) {
                Some(false)
            } else if m.what != What::Value || widgets.iter().all(|w| w.hidden) {
                Some(true)
            } else if widgets.iter().any(|w| !w.hidden && w.layer.is_none()) {
                Some(false)
            } else {
                None
            }
        });
        if on.is_none() {
            assert_eq!(saved.fields().len(), 0, "fields left");
        }
    });
}

#[test]
fn removing_form_values_removes_every_value_and_keeps_the_rest() {
    each_case("remove form values", |seed| {
        let Made {
            mut pdf, markers, ..
        } = made(seed);
        let pages = pdf.page_count();
        change(&mut pdf, |p| p.remove_hidden(&[Hidden::FormValues]));
        let saved = check(&mut pdf, &markers, pages, |m| {
            Some(matches!(m.what, What::Value | What::Default))
        });
        let held: Vec<_> = (saved.fields().into_iter())
            .filter(|f| !f.values.is_empty())
            .collect();
        assert!(held.is_empty(), "values left: {held:?}");
    });
}

#[test]
fn removing_comments_removes_every_note_and_keeps_the_rest() {
    each_case("remove comments", |seed| {
        let Made {
            mut pdf, markers, ..
        } = made(seed);
        let pages = pdf.page_count();
        change(&mut pdf, |p| p.remove_hidden(&[Hidden::Comments]));
        check(&mut pdf, &markers, pages, |m| {
            Some(matches!(m.what, What::NoteDrawing | What::NoteText))
        });
    });
}

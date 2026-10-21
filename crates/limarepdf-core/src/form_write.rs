//! Writing form values, and the appearance streams that show them.

use crate::doc::encode_text;
use crate::fonts::{winansi_bytes, FontChoice, FontCtx};
use crate::form_ap::{self, Body, Border, Look, Metrics};
use crate::forms::{button_export, Node, MAX_DEPTH};
use crate::{std14, Field, FieldKind, FieldValue, Pdf, Result};
use anyhow::{anyhow, bail};
use lopdf::content::{Content, Operation};
use lopdf::{dictionary, Dictionary, Object, ObjectId, Stream, StringFormat};

/// The check mark in ZapfDingbats, drawn when a checkbox has no appearance of its own.
const CHECK: u8 = b'4';

impl Pdf {
    /// Set a field's value and redraw it. The value must suit the field: text for a text
    /// field, `Check` for a checkbox, and for radio groups and choices the export value of
    /// an option (a choice's shown text is accepted too). Read-only fields, push buttons
    /// and signature fields can't be set, and text LimarePDF can't write (characters outside
    /// Latin-1) is refused rather than saved wrongly.
    pub fn set_field(&mut self, name: &str, value: FieldValue) -> Result<()> {
        let (field, node) = self
            .form_nodes()
            .into_iter()
            .find(|(f, _)| f.name == name)
            .ok_or_else(|| anyhow!("no form field named \"{name}\""))?;
        if field.read_only {
            bail!("the field \"{name}\" is read-only");
        }
        let value = check_value(&field, value)?;
        self.write_value(&field, &node, &value)?;
        self.form_changed();
        Ok(())
    }

    /// Put every field back to its default value (`/DV`), or empty when it has none.
    /// Read-only fields are left as they are.
    pub fn reset_form(&mut self) -> Result<()> {
        for (field, node) in self.form_nodes() {
            if field.read_only || matches!(field.kind, FieldKind::Button | FieldKind::Signature) {
                continue;
            }
            let dv = self.value_of(&field.kind, node.dv.as_ref(), &node.opt, &node.widgets);
            let value = check_value(&field, dv).unwrap_or_else(|_| empty(&field.kind));
            self.write_value(&field, &node, &value)?;
        }
        self.form_changed();
        Ok(())
    }

    /// Clear every form field's value for good, as removing hidden information does: current,
    /// default and inherited values, rich text and chosen options, read-only fields too, and
    /// widgets on pages that the field tree doesn't list. Checkboxes and radio buttons go
    /// Off; text fields and choices are redrawn empty, or lose their appearance where
    /// LimarePDF can't draw them (readers then draw them, empty). Signature fields are left
    /// alone. Returns how many fields held a value. Unlike [`Pdf::reset_form`], nothing can
    /// bring the values back.
    pub(crate) fn clear_form_values(&mut self) -> Result<usize> {
        let nodes = self.value_nodes()?;
        let orphans = self.orphan_widgets(&nodes)?;
        let all = self.form_nodes();
        // The widgets of every field read, buttons and signatures included.
        let reached: std::collections::HashSet<ObjectId> = all
            .iter()
            .flat_map(|(_, n)| n.widgets.iter().copied())
            .collect();
        let fields: Vec<(Field, Node)> = all
            .into_iter()
            .filter(|(f, _)| !matches!(f.kind, FieldKind::Button | FieldKind::Signature))
            .collect();
        let filled = |v: &FieldValue| match v {
            FieldValue::Text(t) => !t.is_empty(),
            FieldValue::Check(c) => *c,
            FieldValue::Choice(c) => !c.is_empty(),
            FieldValue::None => false,
        };
        let mut held = fields
            .iter()
            .filter(|(f, n)| {
                let dv = self.value_of(&f.kind, n.dv.as_ref(), &n.opt, &n.widgets);
                let rich = self.doc.get_dictionary(n.id).is_ok_and(|d| d.has(b"RV"));
                filled(&f.value) || filled(&dv) || rich
            })
            .count();
        const VALUES: [&[u8]; 4] = [b"V", b"DV", b"RV", b"I"];
        for (id, sig) in &nodes {
            if !sig {
                if let Ok(d) = self.doc.get_dictionary_mut(*id) {
                    for k in VALUES {
                        d.remove(k);
                    }
                }
            }
        }
        let mut undrawn = Vec::new();
        for (field, node) in &fields {
            for &w in &node.widgets {
                match field.kind {
                    FieldKind::Check | FieldKind::Radio => {
                        if field.kind == FieldKind::Check && !self.has_appearance(w) {
                            let _ = self.draw_check(node, w, false);
                        }
                        if let Ok(d) = self.doc.get_dictionary_mut(w) {
                            d.set("AS", "Off");
                        }
                    }
                    _ => {
                        if self
                            .draw_widget(field, node, w, &empty(&field.kind))
                            .is_err()
                        {
                            undrawn.push(w);
                        }
                    }
                }
            }
        }
        // Widgets the tree reaches beyond the fields read above (a form of more fields than
        // [`Pdf::form_fields`] reads): their drawings go, as they can't be redrawn.
        for &(id, sig) in &nodes {
            let widget = self.doc.get_dictionary(id).is_ok_and(|d| d.has(b"Rect"));
            if !sig && widget && !reached.contains(&id) {
                self.clear_unread_widget(id, &mut undrawn);
            }
        }
        // Widgets on pages that the tree doesn't reach: their values, and their parents',
        // go; their drawings can't be redrawn without a field, so they go too.
        for chain in orphans {
            let had = chain.iter().any(|&c| {
                self.doc
                    .get_dictionary(c)
                    .is_ok_and(|d| VALUES.iter().any(|k| d.has(k)))
            });
            held += usize::from(had);
            for &c in &chain {
                if let Ok(d) = self.doc.get_dictionary_mut(c) {
                    for k in VALUES {
                        d.remove(k);
                    }
                }
            }
            self.clear_unread_widget(chain[0], &mut undrawn);
        }
        if !undrawn.is_empty() {
            for w in undrawn {
                if let Ok(d) = self.doc.get_dictionary_mut(w) {
                    d.remove(b"AP");
                }
            }
            let form = self.acro_form_id()?;
            self.doc
                .get_dictionary_mut(form)?
                .set("NeedAppearances", true);
        }
        self.form_changed();
        Ok(held)
    }

    /// The widgets on pages that the field tree (`nodes`, as [`Pdf::value_nodes`] reads it)
    /// doesn't reach, signature fields' left out, each followed by its parents. Fails rather
    /// than leave a parent out, as it could hold a value, when they are nested more than
    /// [`MAX_DEPTH`] deep.
    pub(crate) fn orphan_widgets(&self, nodes: &[(ObjectId, bool)]) -> Result<Vec<Vec<ObjectId>>> {
        let listed: std::collections::HashSet<ObjectId> = nodes.iter().map(|n| n.0).collect();
        let mut out = Vec::new();
        for page in self.page_ids() {
            for a in self.all_annot_refs(page) {
                let Ok(d) = self.doc.get_dictionary(a) else {
                    continue;
                };
                let widget = d.get(b"Subtype").and_then(|o| o.as_name()).ok() == Some(b"Widget");
                if !widget || listed.contains(&a) {
                    continue;
                }
                let ft = self.inherited(a, b"FT");
                if ft.as_ref().and_then(|o| o.as_name().ok()) == Some(b"Sig") {
                    continue;
                }
                let mut chain = vec![a];
                while let Some(p) = chain.last().and_then(|&c| {
                    let d = self.doc.get_dictionary(c).ok()?;
                    d.get(b"Parent").ok()?.as_reference().ok()
                }) {
                    if chain.contains(&p) {
                        break;
                    }
                    if chain.len() > MAX_DEPTH {
                        bail!("the form's fields are nested more than {MAX_DEPTH} deep");
                    }
                    chain.push(p);
                }
                out.push(chain);
            }
        }
        Ok(out)
    }

    /// For [`Pdf::clear_form_values`], a widget of no field it read: a checkbox or radio
    /// button goes Off, and a text field's or choice's drawing goes into `undrawn`, to be
    /// removed. Push buttons have no value and stay.
    fn clear_unread_widget(&mut self, id: ObjectId, undrawn: &mut Vec<ObjectId>) {
        let ff = self
            .inherited(id, b"Ff")
            .and_then(|o| self.int_value(&o))
            .unwrap_or(0);
        match self.inherited(id, b"FT").as_ref().map(|o| o.as_name()) {
            Some(Ok(b"Btn")) if ff & (1 << 16) == 0 => {
                if let Ok(d) = self.doc.get_dictionary_mut(id) {
                    d.set("AS", "Off");
                }
            }
            Some(Ok(b"Tx" | b"Ch")) => undrawn.push(id),
            _ => {}
        }
    }

    /// Draw the appearances that are missing, so that every field shows its value; all
    /// of the text and choice fields' appearances when the form asks for that
    /// (`/NeedAppearances`). Fields LimarePDF can't draw are left as they are. Returns how
    /// many widgets were drawn.
    pub fn refresh_appearances(&mut self) -> usize {
        let mut n = 0;
        for (field, node, todo) in self.appearances_to_draw() {
            for w in todo {
                let drawn = match &field.kind {
                    FieldKind::Text { .. } | FieldKind::Choice { .. } => {
                        self.draw_widget(&field, &node, w, &field.value)
                    }
                    FieldKind::Check => {
                        self.draw_check(&node, w, field.value == FieldValue::Check(true))
                    }
                    _ => continue,
                };
                if drawn.is_ok() {
                    n += 1;
                }
            }
        }
        n
    }

    /// The widgets [`Pdf::refresh_appearances`] would draw, by field: only fields LimarePDF can
    /// draw.
    pub(crate) fn appearances_to_draw(&self) -> Vec<(Field, Node, Vec<ObjectId>)> {
        let all = self
            .acro_form()
            .and_then(|f| f.get(b"NeedAppearances").ok())
            .and_then(|o| o.as_bool().ok())
            .unwrap_or(false);
        let mut out = Vec::new();
        for (field, node) in self.form_nodes() {
            let drawable = matches!(
                field.kind,
                FieldKind::Text { .. } | FieldKind::Choice { .. } | FieldKind::Check
            );
            let todo: Vec<ObjectId> = node
                .widgets
                .iter()
                .copied()
                .filter(|&w| !self.has_appearance(w) || (all && is_text_like(&field.kind)))
                .collect();
            if drawable && !todo.is_empty() && check_value(&field, field.value.clone()).is_ok() {
                out.push((field, node, todo));
            }
        }
        out
    }

    fn write_value(&mut self, field: &Field, node: &Node, value: &FieldValue) -> Result<()> {
        match (&field.kind, value) {
            (FieldKind::Text { password, .. }, FieldValue::Text(t)) => {
                // A password is never kept in the file. An empty string, not a missing
                // /V, so that a parent's value doesn't show through.
                let kept = if *password { "" } else { t.as_str() };
                let v = Object::String(encode_text(kept), StringFormat::Literal);
                self.doc.get_dictionary_mut(node.id)?.set("V", v);
            }
            (FieldKind::Check, FieldValue::Check(on)) => {
                let on_name = node
                    .widgets
                    .iter()
                    .find_map(|&w| self.on_state(w))
                    .unwrap_or_else(|| "Yes".into());
                let v = if *on { on_name.as_str() } else { "Off" };
                self.doc
                    .get_dictionary_mut(node.id)?
                    .set("V", Object::Name(v.as_bytes().to_vec()));
                for &w in &node.widgets {
                    if !self.has_appearance(w) {
                        self.draw_check(node, w, *on)?;
                    }
                    let state = self.on_state(w).filter(|s| *on && *s == on_name);
                    let state = state.unwrap_or_else(|| "Off".into());
                    self.doc
                        .get_dictionary_mut(w)?
                        .set("AS", Object::Name(state.into_bytes()));
                }
            }
            (FieldKind::Radio, FieldValue::Choice(chosen)) => {
                let target = chosen.first();
                let states: Vec<(ObjectId, Option<String>)> = node
                    .widgets
                    .iter()
                    .map(|&w| (w, self.on_state(w)))
                    .collect();
                let on = target.and_then(|t| {
                    states
                        .iter()
                        .filter_map(|(_, s)| s.as_ref())
                        .find(|s| button_export(s, &node.opt) == *t)
                        .cloned()
                });
                let v = on.clone().unwrap_or_else(|| "Off".into());
                self.doc
                    .get_dictionary_mut(node.id)?
                    .set("V", Object::Name(v.into_bytes()));
                for (w, s) in states {
                    let state = match (&on, s) {
                        (Some(on), Some(s)) if *on == s => s,
                        _ => "Off".into(),
                    };
                    self.doc
                        .get_dictionary_mut(w)?
                        .set("AS", Object::Name(state.into_bytes()));
                }
            }
            (FieldKind::Choice { multi, .. }, FieldValue::Choice(chosen)) => {
                let text = |s: &String| Object::String(encode_text(s), StringFormat::Literal);
                let d = self.doc.get_dictionary_mut(node.id)?;
                match chosen.as_slice() {
                    [] => {
                        d.remove(b"V");
                    }
                    [one] => d.set("V", text(one)),
                    many => d.set("V", many.iter().map(text).collect::<Vec<_>>()),
                }
                // Multi-select lists also give the chosen options' positions.
                let indices: Vec<Object> = node
                    .opt
                    .iter()
                    .enumerate()
                    .filter(|(_, o)| chosen.contains(&o.0))
                    .map(|(i, _)| Object::Integer(i as i64))
                    .collect();
                if *multi && !indices.is_empty() {
                    d.set("I", indices);
                } else {
                    d.remove(b"I");
                }
            }
            _ => bail!("the value doesn't suit the field \"{}\"", field.name),
        }
        // A rich-text value would contradict the new plain one.
        self.doc.get_dictionary_mut(node.id)?.remove(b"RV");
        if is_text_like(&field.kind) {
            for &w in &node.widgets {
                self.draw_widget(field, node, w, value)?;
            }
        }
        Ok(())
    }

    /// After a change: a hybrid form's XFA part still holds the old values, and readers
    /// that prefer XFA would show those, so it goes.
    pub(crate) fn form_changed(&mut self) {
        if let Ok(form) = self.acro_form_id() {
            if let Ok(d) = self.doc.get_dictionary_mut(form) {
                d.remove(b"XFA");
            }
        }
    }

    /// Whether a widget has a normal appearance.
    fn has_appearance(&self, widget: ObjectId) -> bool {
        self.doc
            .get_dictionary(widget)
            .ok()
            .and_then(|d| d.get(b"AP").ok())
            .and_then(|o| self.doc.dereference(o).ok())
            .and_then(|(_, o)| o.as_dict().ok())
            .is_some_and(|ap| ap.has(b"N"))
    }

    /// Draw a text field's or a choice's widget showing `value`.
    fn draw_widget(
        &mut self,
        field: &Field,
        node: &Node,
        widget: ObjectId,
        value: &FieldValue,
    ) -> Result<()> {
        let shown_text = |export: &str| -> String {
            node.opt
                .iter()
                .find(|o| o.0 == export)
                .map_or_else(|| export.to_string(), |o| o.1.clone())
        };
        let chosen: Vec<String> = match value {
            FieldValue::Choice(c) => c.clone(),
            _ => Vec::new(),
        };
        let items: Vec<String> = node.opt.iter().map(|o| o.1.clone()).collect();
        let indices: Vec<usize> = node
            .opt
            .iter()
            .enumerate()
            .filter(|(_, o)| chosen.contains(&o.0))
            .map(|(i, _)| i)
            .collect();
        let text = match (&field.kind, value) {
            (FieldKind::Text { password: true, .. }, FieldValue::Text(t)) => {
                "*".repeat(t.chars().count())
            }
            (FieldKind::Text { .. }, FieldValue::Text(t)) => t.clone(),
            (FieldKind::Choice { combo: true, .. }, _) => {
                chosen.first().map(|c| shown_text(c)).unwrap_or_default()
            }
            (FieldKind::Choice { .. }, _) => items.concat(),
            _ => bail!("not a text or choice field"),
        };
        let top = self
            .doc
            .get_dictionary(node.id)?
            .get(b"TI")
            .ok()
            .and_then(|o| self.int_value(o))
            .and_then(|t| usize::try_from(t).ok())
            .unwrap_or(0);
        let body = match &field.kind {
            FieldKind::Text {
                comb: true,
                max_len: Some(n),
                ..
            } => Body::Comb(&text, *n),
            FieldKind::Text {
                multiline: true, ..
            } => Body::Lines(&text),
            FieldKind::Choice { combo: false, .. } => Body::List {
                items: &items,
                chosen: &indices,
                top,
            },
            _ => Body::Line(&text),
        };
        let da = self.widget_da(widget, node);
        let (font, size, color) = parse_da(&da);
        let (font, font_ref, metrics, replaced) = self.pick_font(font.as_deref(), &text)?;
        if replaced {
            // Keep /DA in step, so readers that redraw fields use the same font.
            let da = encode_da(&font, size, color.as_ref());
            let da = Object::String(da, StringFormat::Literal);
            self.doc.get_dictionary_mut(node.id)?.set("DA", da.clone());
            let w = self.doc.get_dictionary_mut(widget)?;
            if w.has(b"DA") {
                w.set("DA", da);
            }
        }
        let (mut look, matrix) = self.widget_look(widget)?;
        look.font = font.clone();
        look.size = size;
        look.color = color;
        look.q = self
            .doc
            .get_dictionary(widget)?
            .get(b"Q")
            .ok()
            .and_then(|o| self.int_value(o))
            .unwrap_or(node.q);
        let content = form_ap::draw(&look, &body, &metrics);
        let resources = dictionary! { "Font" => dictionary! { font => font_ref } };
        let ap = self.add_appearance(&look, matrix, resources, content);
        self.doc
            .get_dictionary_mut(widget)?
            .set("AP", dictionary! { "N" => ap });
        Ok(())
    }

    /// Draw a checkbox widget that has no appearances: a check mark (or the widget's own
    /// `/MK /CA` character) when on, and only the background and border when off.
    fn draw_check(&mut self, node: &Node, widget: ObjectId, on: bool) -> Result<()> {
        let (mut look, matrix) = self.widget_look(widget)?;
        let da = self.widget_da(widget, node);
        let (_, size, color) = parse_da(&da);
        let mark = self
            .doc
            .get_dictionary(widget)?
            .get(b"MK")
            .ok()
            .and_then(|o| self.doc.dereference(o).ok())
            .and_then(|(_, o)| o.as_dict().ok())
            .and_then(|mk| mk.get(b"CA").ok())
            .and_then(|o| self.string_bytes(o))
            .and_then(|b| b.first().copied())
            .unwrap_or(CHECK);
        let font_ref = self.dr_zapf()?;
        look.font = b"ZaDb".to_vec();
        look.size = size;
        look.color = color;
        let zapf = Metrics(std::array::from_fn(|c| {
            std14::symbolic_width("ZapfDingbats", c as u32).unwrap_or(0.0)
        }));
        let resources = || dictionary! { "Font" => dictionary! { "ZaDb" => font_ref } };
        let on_content = form_ap::draw(&look, &Body::Mark(mark), &zapf);
        let off_content = form_ap::draw(&look, &Body::Empty, &zapf);
        let on_ap = self.add_appearance(&look, matrix, resources(), on_content);
        let off_ap = self.add_appearance(&look, matrix, resources(), off_content);
        let state = self.on_state(widget).unwrap_or_else(|| "Yes".into());
        let n = dictionary! { state.clone() => on_ap, "Off" => off_ap };
        let w = self.doc.get_dictionary_mut(widget)?;
        w.set("AP", dictionary! { "N" => n });
        w.set("AS", if on { state.as_str() } else { "Off" });
        Ok(())
    }

    /// A form XObject holding an appearance.
    fn add_appearance(
        &mut self,
        look: &Look,
        matrix: Option<[f32; 6]>,
        resources: Dictionary,
        content: Vec<u8>,
    ) -> ObjectId {
        let mut d = dictionary! {
            "Type" => "XObject",
            "Subtype" => "Form",
            "BBox" => vec![0.into(), 0.into(), look.w.into(), look.h.into()],
            "Resources" => resources,
        };
        if let Some(m) = matrix {
            d.set(
                "Matrix",
                m.iter().map(|&x| Object::Real(x)).collect::<Vec<_>>(),
            );
        }
        self.doc.add_object(Stream::new(d, content))
    }

    /// The widget's size, background, border and rotation; font, size, colour and
    /// alignment are left for the caller. Also the matrix that turns a rotated widget's
    /// appearance.
    fn widget_look(&self, widget: ObjectId) -> Result<(Look, Option<[f32; 6]>)> {
        let d = self.doc.get_dictionary(widget)?;
        let rect = d
            .get(b"Rect")
            .ok()
            .and_then(|o| self.rect(o))
            .ok_or_else(|| anyhow!("a widget without a rectangle"))?;
        let (w, h) = (rect[2] - rect[0], rect[3] - rect[1]);
        let dict = |o| self.dict_of(o);
        let colors = |o: &Object| -> Option<Vec<f32>> {
            let a = self.doc.dereference(o).ok()?.1.as_array().ok()?;
            let c: Vec<f32> = a.iter().filter_map(|x| x.as_float().ok()).collect();
            (!c.is_empty()).then_some(c)
        };
        let mk = d.get(b"MK").ok().and_then(dict);
        let rotation = mk
            .and_then(|m| m.get(b"R").ok())
            .and_then(|o| self.int_value(o))
            .unwrap_or(0)
            .rem_euclid(360);
        let bg = mk.and_then(|m| m.get(b"BG").ok()).and_then(colors);
        let bc = mk.and_then(|m| m.get(b"BC").ok()).and_then(colors);
        let bs = d.get(b"BS").ok().and_then(dict);
        let width = bs
            .and_then(|b| b.get(b"W").ok())
            .and_then(|o| self.doc.dereference(o).ok())
            .and_then(|(_, o)| o.as_float().ok())
            .or_else(|| {
                let a = d.get(b"Border").ok()?.as_array().ok()?;
                a.get(2)?.as_float().ok()
            })
            .unwrap_or(1.0);
        let style = bs
            .and_then(|b| b.get(b"S").ok())
            .and_then(|o| o.as_name().ok())
            .and_then(|n| n.first().copied())
            .unwrap_or(b'S');
        let border = bc.map(|color| Border {
            color,
            width,
            style,
        });
        let (w, h, matrix) = match rotation {
            90 => (h, w, Some([0.0, 1.0, -1.0, 0.0, 0.0, 0.0])),
            180 => (w, h, Some([-1.0, 0.0, 0.0, -1.0, 0.0, 0.0])),
            270 => (h, w, Some([0.0, -1.0, 1.0, 0.0, 0.0, 0.0])),
            _ => (w, h, None),
        };
        let look = Look {
            w,
            h,
            bg,
            border,
            font: Vec::new(),
            size: 0.0,
            color: None,
            q: 0,
        };
        Ok((look, matrix))
    }

    pub(crate) fn dict_of<'a>(&'a self, o: &'a Object) -> Option<&'a Dictionary> {
        self.doc
            .dereference(o)
            .ok()
            .and_then(|(_, o)| o.as_dict().ok())
    }

    /// The widget's own `/DA`, else the field's (inherited, or the form's).
    fn widget_da(&self, widget: ObjectId, node: &Node) -> Vec<u8> {
        self.doc
            .get_dictionary(widget)
            .ok()
            .and_then(|d| d.get(b"DA").ok())
            .and_then(|o| self.string_bytes(o))
            .or_else(|| node.da.clone())
            .unwrap_or_default()
    }

    /// The font to draw `text` with: the one `/DA` names when LimarePDF can write with it,
    /// else the closest standard font, added to the form's resources. Returns its name,
    /// a reference to it, its widths, and whether it replaced the named one.
    fn pick_font(
        &mut self,
        name: Option<&[u8]>,
        text: &str,
    ) -> Result<(Vec<u8>, ObjectId, Metrics, bool)> {
        let found = name.and_then(|n| Some((n.to_vec(), self.dr_font(n)?)));
        let mut base = String::from("Helvetica");
        if let Some((name, obj)) = &found {
            let (id, dict) = match obj {
                Object::Reference(id) => (Some(*id), self.doc.get_dictionary(*id).ok()),
                Object::Dictionary(d) => (None, Some(d)),
                _ => (None, None),
            };
            if let Some(dict) = dict {
                let ctx = FontCtx::new(&self.doc, dict);
                let winansi = dict
                    .get(b"Encoding")
                    .ok()
                    .and_then(|o| o.as_name().ok())
                    .is_some_and(|n| n == b"WinAnsiEncoding");
                let metrics = Metrics(std::array::from_fn(|c| ctx.width(c as u32)));
                // A zero width is how some makers (LibreOffice, for one) mark a character
                // the font in the file doesn't have.
                let has_all = winansi_bytes(text)
                    .iter()
                    .all(|&b| b <= b' ' || metrics.0[b as usize] > 0.0);
                if ctx.reusable() && (text.is_ascii() || winansi) && has_all {
                    let dict = dict.clone();
                    let id = id.unwrap_or_else(|| self.doc.add_object(dict));
                    return Ok((name.clone(), id, metrics, false));
                }
                base = ctx.base.clone();
            }
        }
        let base = FontChoice::from_name(&base).base_font();
        let name = format!("Limare{}", base.replace('-', "")).into_bytes();
        let id = match self.dr_font(&name) {
            Some(Object::Reference(id)) => id,
            _ => {
                let font = dictionary! {
                    "Type" => "Font",
                    "Subtype" => "Type1",
                    "BaseFont" => base,
                    "Encoding" => "WinAnsiEncoding",
                };
                let id = self.doc.add_object(font);
                self.add_dr_font(&name, id)?;
                id
            }
        };
        Ok((name, id, standard_metrics(base), true))
    }

    /// The form's ZapfDingbats font (`/ZaDb`), added when it has none.
    fn dr_zapf(&mut self) -> Result<ObjectId> {
        match self.dr_font(b"ZaDb") {
            Some(Object::Reference(id)) => Ok(id),
            Some(Object::Dictionary(d)) => Ok(self.doc.add_object(d)),
            _ => {
                let font = dictionary! {
                    "Type" => "Font", "Subtype" => "Type1", "BaseFont" => "ZapfDingbats",
                };
                let id = self.doc.add_object(font);
                self.add_dr_font(b"ZaDb", id)?;
                Ok(id)
            }
        }
    }

    /// A font in the form's default resources (`/DR /Font`), as stored there.
    fn dr_font(&self, name: &[u8]) -> Option<Object> {
        let dr = self.acro_form()?.get(b"DR").ok()?;
        let dr = self.doc.dereference(dr).ok()?.1.as_dict().ok()?;
        let fonts = self.doc.dereference(dr.get(b"Font").ok()?).ok()?.1;
        fonts.as_dict().ok()?.get(name).ok().cloned()
    }

    fn add_dr_font(&mut self, name: &[u8], font: ObjectId) -> Result<()> {
        let form = self.acro_form_id()?;
        let dr = self.indirect_entry(form, b"DR")?;
        let fonts = self.indirect_entry(dr, b"Font")?;
        self.doc.get_dictionary_mut(fonts)?.set(name, font);
        Ok(())
    }

    /// The `/AcroForm` dictionary's object, made indirect if it was written in the
    /// catalog.
    pub(crate) fn acro_form_id(&mut self) -> Result<ObjectId> {
        let catalog = self.doc.trailer.get(b"Root")?.as_reference()?;
        self.indirect_entry(catalog, b"AcroForm")
    }

    /// The dictionary under `key` in the dictionary `parent`, as an object of its own:
    /// moved out when it is written in place, created empty when missing.
    fn indirect_entry(&mut self, parent: ObjectId, key: &[u8]) -> Result<ObjectId> {
        let current = self.doc.get_dictionary(parent)?.get(key).ok().cloned();
        let d = match current {
            Some(Object::Reference(id)) if self.doc.get_dictionary(id).is_ok() => return Ok(id),
            Some(Object::Dictionary(d)) => d,
            _ => Dictionary::new(),
        };
        let id = self.doc.add_object(d);
        self.doc.get_dictionary_mut(parent)?.set(key, id);
        Ok(id)
    }
}

/// Text, choice and list fields are drawn by LimarePDF; buttons have drawings of their own.
fn is_text_like(kind: &FieldKind) -> bool {
    matches!(kind, FieldKind::Text { .. } | FieldKind::Choice { .. })
}

/// The empty value for a kind of field.
fn empty(kind: &FieldKind) -> FieldValue {
    match kind {
        FieldKind::Text { .. } => FieldValue::Text(String::new()),
        FieldKind::Check => FieldValue::Check(false),
        FieldKind::Radio | FieldKind::Choice { .. } => FieldValue::Choice(Vec::new()),
        FieldKind::Button | FieldKind::Signature => FieldValue::None,
    }
}

/// Check that a value suits a field, and normalise it: a choice given by its shown text
/// becomes its export value.
fn check_value(field: &Field, value: FieldValue) -> Result<FieldValue> {
    let name = &field.name;
    let writable = |s: &str| -> Result<()> {
        let bad = s
            .chars()
            .find(|&c| !matches!(c, '\n' | '\r' | '?') && winansi_bytes(&c.to_string()) == b"?");
        match bad {
            Some(c) => bail!(
                "LimarePDF can't write \"{c}\" in a form field yet; only Latin-1 (Western European) \
                 characters are supported"
            ),
            None => Ok(()),
        }
    };
    let exports = || -> Vec<&str> { field.options.iter().map(|o| o.0.as_str()).collect() };
    match (&field.kind, value) {
        (
            FieldKind::Text {
                multiline, max_len, ..
            },
            FieldValue::Text(t),
        ) => {
            if !multiline && t.contains(['\n', '\r']) {
                bail!("\"{name}\" is a single-line field; its value can't have line breaks");
            }
            if let Some(n) = max_len {
                if t.chars().count() > *n {
                    bail!("\"{name}\" takes at most {n} characters");
                }
            }
            writable(&t)?;
            Ok(FieldValue::Text(t))
        }
        (FieldKind::Check, v @ FieldValue::Check(_)) => Ok(v),
        (FieldKind::Radio, FieldValue::Choice(c)) => match c.as_slice() {
            [] => Ok(FieldValue::Choice(c)),
            [one] if exports().contains(&one.as_str()) => Ok(FieldValue::Choice(c)),
            [one] => bail!(
                "\"{one}\" isn't one of the choices for \"{name}\": {}",
                exports().join(", ")
            ),
            _ => bail!("\"{name}\" is a radio group; only one choice can be made"),
        },
        (
            FieldKind::Choice {
                combo,
                editable,
                multi,
            },
            FieldValue::Choice(c),
        ) => {
            if !multi && c.len() > 1 {
                bail!("\"{name}\" takes only one choice");
            }
            let mut out = Vec::new();
            for v in c {
                let by_shown = field.options.iter().find(|o| o.1 == v).map(|o| o.0.clone());
                if exports().contains(&v.as_str()) {
                    out.push(v);
                } else if let Some(e) = by_shown {
                    out.push(e);
                } else if *combo && *editable {
                    writable(&v)?;
                    out.push(v);
                } else {
                    bail!(
                        "\"{v}\" isn't one of the choices for \"{name}\": {}",
                        exports().join(", ")
                    );
                }
            }
            Ok(FieldValue::Choice(out))
        }
        (FieldKind::Button, _) => bail!("\"{name}\" is a push button; it has no value to set"),
        (FieldKind::Signature, _) => {
            bail!("\"{name}\" is a signature field; LimarePDF can't sign yet")
        }
        _ => bail!("the value doesn't suit the field \"{name}\""),
    }
}

/// The font name, size and text colour from a `/DA` string.
pub(crate) fn parse_da(da: &[u8]) -> (Option<Vec<u8>>, f32, Option<Operation>) {
    let ops = Content::decode(da)
        .map(|c| c.operations)
        .unwrap_or_default();
    let (mut font, mut size, mut color) = (None, 0.0, None);
    for o in ops {
        match o.operator.as_str() {
            "Tf" if o.operands.len() == 2 => {
                font = o.operands[0].as_name().ok().map(<[u8]>::to_vec);
                size = o.operands[1].as_float().unwrap_or(0.0).max(0.0);
            }
            "g" | "rg" | "k" => color = Some(o),
            _ => {}
        }
    }
    (font, size, color)
}

fn encode_da(font: &[u8], size: f32, color: Option<&Operation>) -> Vec<u8> {
    let mut ops = vec![Operation::new(
        "Tf",
        vec![Object::Name(font.to_vec()), Object::Real(size)],
    )];
    ops.extend(color.cloned());
    Content { operations: ops }.encode().unwrap_or_default()
}

/// Widths of a standard font by WinAnsi code.
pub(crate) fn standard_metrics(base: &str) -> Metrics {
    let mut w = [0.0; 256];
    for c in (0x20..=0x2122).filter_map(char::from_u32) {
        let b = winansi_bytes(&c.to_string())[0];
        if b != b'?' || c == '?' {
            w[b as usize] = std14::width(base, c).unwrap_or(556.0);
        }
    }
    Metrics(w)
}

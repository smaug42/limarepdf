//! Writing form values, and the appearance streams that show them.

use crate::doc::encode_text;
use crate::edit::numbered;
use crate::embed::{cant_write, unwritable};
use crate::fonts::{FontChoice, FontCtx};
use crate::form_ap::{self, Body, Border, BoxSetting, Look, Metrics, OwnFont};
use crate::forms::{button_export, Node, Reach, MAX_DEPTH};
use crate::layout::SetIn;
use crate::{std14, Field, FieldKind, FieldValue, Pdf, Result};
use anyhow::{anyhow, bail};
use lopdf::content::{Content, Operation};
use lopdf::{dictionary, Dictionary, Document, Object, ObjectId, Stream, StringFormat};

/// The check mark in ZapfDingbats, drawn when a checkbox has no appearance of its own.
const CHECK: u8 = b'4';
/// The name of an embedded font in the appearances that use one.
const EMBEDDED: &[u8] = b"LimareEm";

/// The font a widget's text is drawn with ([`Pdf::pick_font`]).
struct Picked {
    /// Its name in the appearance's resources.
    name: Vec<u8>,
    /// The field's own font, written as it is, or the standard font that replaces it; `None`
    /// for an embedded face.
    font: Option<FontObj>,
    metrics: Metrics,
    /// Whether `/DA` should name it.
    replaced: bool,
    /// The font chosen, when it isn't the field's own.
    choice: FontChoice,
}

/// A font object an appearance names: one in the document, or one to add when the
/// appearance is written ([`Pdf::font_object`]).
enum FontObj {
    Is(ObjectId),
    /// Added to the form's resources too when `to_form`, unless one added since is there.
    Add {
        dict: Dictionary,
        to_form: bool,
    },
}

/// All a field's new value writes, worked out before anything is written
/// ([`Pdf::stage_value`]), so that a value that can't be written changes nothing.
struct Staged {
    node: ObjectId,
    /// The field's entries, each set, or removed when `None`.
    entries: Vec<(&'static [u8], Option<Object>)>,
    /// Checkbox widgets drawn for the first time, before their states are set.
    checks: Vec<CheckDrawing>,
    /// Button widgets' states (`/AS`).
    states: Vec<(ObjectId, Vec<u8>)>,
    drawings: Vec<Drawing>,
}

impl Staged {
    /// Whether writing it can still fail part way: when text is set by the text pipeline,
    /// which can embed glyphs, or a font is added to the form's resources.
    fn can_fail(&self) -> bool {
        let to_form = |f: &FontObj| matches!(f, FontObj::Add { to_form: true, .. });
        let set = |d: &Drawing| matches!(d.picked.metrics, Metrics::Set(_));
        (self.drawings.iter()).any(|d| set(d) || d.picked.font.as_ref().is_some_and(to_form))
            || self.checks.iter().any(|c| to_form(&c.font))
    }
}

/// A text or choice widget's appearance, to write ([`Pdf::stage_widget`]).
struct Drawing {
    /// The widget, and its field's object.
    widget: ObjectId,
    node: ObjectId,
    look: Look,
    matrix: Option<[f32; 6]>,
    drawn: form_ap::Drawn,
    picked: Picked,
    /// The `/DA` naming the font that replaces the field's own.
    da: Option<Object>,
}

/// A checkbox widget's appearances, on and off, to write ([`Pdf::stage_check`]).
struct CheckDrawing {
    widget: ObjectId,
    look: Look,
    matrix: Option<[f32; 6]>,
    on: Vec<u8>,
    off: Vec<u8>,
    /// The on appearance's name.
    state: String,
    /// The form's ZapfDingbats.
    font: FontObj,
}

impl Pdf {
    /// Set a field's value and redraw it. The value must suit the field: text for a text
    /// field, `Check` for a checkbox, and for radio groups and choices the export value of
    /// an option (a choice's shown text is accepted too). Read-only fields, push buttons
    /// and signature fields can't be set, and text LimarePDF can't write (right-to-left or
    /// shaped scripts, emoji of several characters, characters no installed font has) is
    /// refused rather than saved wrongly.
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
        let staged = self.stage_value(&field, &node, &value)?;
        self.commit_values(vec![staged])?;
        self.form_changed();
        Ok(())
    }

    /// Put every field back to its default value (`/DV`), or empty when it has none.
    /// Read-only fields are left as they are. When a field can't be reset, none is.
    pub fn reset_form(&mut self) -> Result<()> {
        let mut staged = Vec::new();
        for (field, node) in self.form_nodes() {
            if field.read_only || matches!(field.kind, FieldKind::Button | FieldKind::Signature) {
                continue;
            }
            let dv = self.value_of(&field.kind, node.dv.as_ref(), &node.opt, &node.widgets);
            let value = check_value(&field, dv).unwrap_or_else(|_| empty(&field.kind));
            staged.push(self.stage_value(&field, &node, &value)?);
        }
        self.commit_values(staged)?;
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
                    // The widget and its parents, the top one at most MAX_DEPTH above the
                    // widget's own field, as for a listed field (PB-05).
                    if chain.len() > MAX_DEPTH + 1 {
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
        self.refresh_reaching(Reach::Listed)
    }

    /// [`Pdf::refresh_appearances`] for the fields `reach` takes: every field for
    /// flattening and printing.
    pub(crate) fn refresh_reaching(&mut self, reach: Reach) -> usize {
        let mut n = 0;
        let nodes = self.field_nodes(reach).0;
        for (field, node, todo) in self.appearances_to_draw(nodes) {
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

    /// The widgets [`Pdf::refresh_appearances`] would draw of `nodes` (as
    /// [`Pdf::field_nodes`] gives them), by field: only fields LimarePDF can draw.
    pub(crate) fn appearances_to_draw(
        &self,
        nodes: Vec<(Field, Node)>,
    ) -> Vec<(Field, Node, Vec<ObjectId>)> {
        let all = self
            .acro_form()
            .and_then(|f| f.get(b"NeedAppearances").ok())
            .and_then(|o| o.as_bool().ok())
            .unwrap_or(false);
        let mut out = Vec::new();
        for (field, node) in nodes {
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

    /// What writing `value` in the field changes: its value, its widgets' states and
    /// drawings. Fails, having changed nothing, when it can't be written in full.
    fn stage_value(&self, field: &Field, node: &Node, value: &FieldValue) -> Result<Staged> {
        self.doc.get_dictionary(node.id)?;
        for &w in &node.widgets {
            self.doc.get_dictionary(w)?;
        }
        let mut s = Staged {
            node: node.id,
            entries: Vec::new(),
            checks: Vec::new(),
            states: Vec::new(),
            drawings: Vec::new(),
        };
        let name = |n: &str| Some(Object::Name(n.as_bytes().to_vec()));
        match (&field.kind, value) {
            (FieldKind::Text { password, .. }, FieldValue::Text(t)) => {
                // A password is never kept in the file. An empty string, not a missing
                // /V, so that a parent's value doesn't show through.
                let kept = if *password { "" } else { t.as_str() };
                let v = Object::String(encode_text(kept), StringFormat::Literal);
                s.entries.push((b"V", Some(v)));
            }
            (FieldKind::Check, FieldValue::Check(on)) => {
                let on_name = node
                    .widgets
                    .iter()
                    .find_map(|&w| self.on_state(w))
                    .unwrap_or_else(|| "Yes".into());
                s.entries
                    .push((b"V", name(if *on { &on_name } else { "Off" })));
                for &w in &node.widgets {
                    // A widget without drawings gets them, its on state named as drawn.
                    let state = if self.has_appearance(w) {
                        self.on_state(w)
                    } else {
                        let check = self.stage_check(node, w)?;
                        let state = check.state.clone();
                        s.checks.push(check);
                        Some(state)
                    };
                    let state = state.filter(|s| *on && *s == on_name);
                    let state = state.unwrap_or_else(|| "Off".into());
                    s.states.push((w, state.into_bytes()));
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
                s.entries.push((b"V", name(on.as_deref().unwrap_or("Off"))));
                for (w, st) in states {
                    let state = match (&on, st) {
                        (Some(on), Some(st)) if *on == st => st,
                        _ => "Off".into(),
                    };
                    s.states.push((w, state.into_bytes()));
                }
            }
            (FieldKind::Choice { multi, .. }, FieldValue::Choice(chosen)) => {
                let text = |s: &String| Object::String(encode_text(s), StringFormat::Literal);
                // An empty list, not a missing /V, so that a parent's value doesn't show
                // through.
                let v = match chosen.as_slice() {
                    [one] => text(one),
                    many => Object::Array(many.iter().map(text).collect()),
                };
                s.entries.push((b"V", Some(v)));
                // Multi-select lists also give the chosen options' positions.
                let indices: Vec<Object> = node
                    .opt
                    .iter()
                    .enumerate()
                    .filter(|(_, o)| chosen.contains(&o.0))
                    .map(|(i, _)| Object::Integer(i as i64))
                    .collect();
                let i = (*multi && !indices.is_empty()).then(|| Object::Array(indices));
                s.entries.push((b"I", i));
            }
            _ => bail!("the value doesn't suit the field \"{}\"", field.name),
        }
        // A rich-text value would contradict the new plain one.
        s.entries.push((b"RV", None));
        if is_text_like(&field.kind) {
            for &w in &node.widgets {
                s.drawings.push(self.stage_widget(field, node, w, value)?);
            }
        }
        Ok(s)
    }

    /// Write `staged` ([`Pdf::stage_value`]), all of it or, when a step fails, none: the
    /// document is copied first when a step can fail ([`Staged::can_fail`]).
    fn commit_values(&mut self, staged: Vec<Staged>) -> Result<()> {
        let before = staged.iter().any(Staged::can_fail).then(|| self.before());
        let put = staged.into_iter().try_for_each(|s| self.put_value(s));
        if let (Err(_), Some(before)) = (&put, before) {
            self.go_back(before);
        }
        put
    }

    fn put_value(&mut self, s: Staged) -> Result<()> {
        let d = self.doc.get_dictionary_mut(s.node)?;
        for (key, v) in s.entries {
            match v {
                Some(v) => d.set(key, v),
                None => {
                    d.remove(key);
                }
            }
        }
        for c in s.checks {
            self.put_check(c)?;
        }
        for (w, state) in s.states {
            self.doc
                .get_dictionary_mut(w)?
                .set("AS", Object::Name(state));
        }
        for d in s.drawings {
            self.put_widget(d)?;
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
        let d = self.stage_widget(field, node, widget, value)?;
        self.put_widget(d)
    }

    /// The drawing of a text field's or a choice's widget showing `value`, to write.
    fn stage_widget(
        &self,
        field: &Field,
        node: &Node,
        widget: ObjectId,
        value: &FieldValue,
    ) -> Result<Drawing> {
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
        let picked = self.pick_font(font.as_deref(), &text)?;
        // Keep /DA in step, so readers that redraw fields use the same font.
        let da = picked.replaced.then(|| {
            let da = encode_da(&picked.name, size, color.as_ref());
            Object::String(da, StringFormat::Literal)
        });
        let (mut look, matrix) = self.widget_look(widget)?;
        look.font = picked.name.clone();
        look.size = size;
        look.color = color;
        look.q = self.widget_q(widget, node.q);
        let drawn = form_ap::draw(&look, &body, &picked.metrics);
        Ok(Drawing {
            widget,
            node: node.id,
            look,
            matrix,
            drawn,
            picked,
            da,
        })
    }

    /// Write a widget's drawing ([`Pdf::stage_widget`]). Only setting text in the text
    /// pipeline and adding a font to the form can fail, before the field's `/DA` is changed.
    fn put_widget(&mut self, d: Drawing) -> Result<()> {
        let Drawing {
            widget,
            node,
            look,
            matrix,
            drawn,
            picked,
            da,
        } = d;
        let name = picked.name;
        let font = picked.font.map(|f| self.font_object(&name, f));
        let (content, mut fonts) = match &picked.metrics {
            Metrics::Bytes(_) => (drawn.bytes_content(), Dictionary::new()),
            Metrics::Set(setter) => {
                let lines: Vec<&str> = drawn.lines.iter().map(|(s, ..)| s.as_str()).collect();
                let reuse: Vec<(FontChoice, ObjectId)> =
                    font.iter().map(|&(id, _)| (picked.choice, id)).collect();
                let pen = self.pen_reusing(setter, &lines, &reuse)?;
                let names = pen.names(&String::from_utf8_lossy(&name));
                let content = drawn.content(|i, _| pen.ops(i, drawn.size, 0.0, &names));
                (content, pen.font_dict(&names))
            }
        };
        if let Some((id, true)) = font {
            self.add_dr_font(&name, id)?;
        }
        // The font `/DA` names stays in the resources, though no text is in it.
        if let Some((id, _)) = font.filter(|_| !fonts.has(&name)) {
            fonts.set(name, id);
        }
        let resources = dictionary! { "Font" => fonts };
        let ap = self.add_appearance(&look, matrix, resources, content);
        self.doc
            .get_dictionary_mut(widget)?
            .set("AP", dictionary! { "N" => ap });
        if let Some(da) = da {
            self.doc.get_dictionary_mut(node)?.set("DA", da.clone());
            let w = self.doc.get_dictionary_mut(widget)?;
            if w.has(b"DA") {
                w.set("DA", da);
            }
        }
        Ok(())
    }

    /// The object of a font an appearance names ([`FontObj`]), and whether it is still to be
    /// added to the form's resources: one already there under its name is used only when
    /// it is the same font (one an earlier widget of the same change added).
    fn font_object(&mut self, name: &[u8], f: FontObj) -> (ObjectId, bool) {
        match f {
            FontObj::Is(id) => (id, false),
            FontObj::Add { dict, to_form } => match self.dr_font(name) {
                Some(Object::Reference(id))
                    if to_form && self.doc.get_dictionary(id).is_ok_and(|d| *d == dict) =>
                {
                    (id, false)
                }
                _ => (self.doc.add_object(dict), to_form),
            },
        }
    }

    /// Draw a checkbox widget that has no appearances: a check mark (or the widget's own
    /// `/MK /CA` character) when on, and only the background and border when off.
    fn draw_check(&mut self, node: &Node, widget: ObjectId, on: bool) -> Result<()> {
        let c = self.stage_check(node, widget)?;
        let state = c.state.clone();
        self.put_check(c)?;
        let w = self.doc.get_dictionary_mut(widget)?;
        w.set("AS", if on { state.as_str() } else { "Off" });
        Ok(())
    }

    /// A checkbox widget's drawings ([`Pdf::draw_check`]), to write.
    fn stage_check(&self, node: &Node, widget: ObjectId) -> Result<CheckDrawing> {
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
        look.font = b"ZaDb".to_vec();
        look.size = size;
        look.color = color;
        let zapf = Metrics::Bytes(Box::new(std::array::from_fn(|c| {
            std14::symbolic_width("ZapfDingbats", c as u32).unwrap_or(0.0)
        })));
        let on = form_ap::draw(&look, &Body::Mark(mark), &zapf).bytes_content();
        let off = form_ap::draw(&look, &Body::Empty, &zapf).bytes_content();
        Ok(CheckDrawing {
            widget,
            look,
            matrix,
            on,
            off,
            state: self.on_state(widget).unwrap_or_else(|| "Yes".into()),
            font: self.zapf(),
        })
    }

    /// Write a checkbox widget's drawings ([`Pdf::stage_check`]); its state is left as it is.
    fn put_check(&mut self, c: CheckDrawing) -> Result<()> {
        let (font, add) = self.font_object(b"ZaDb", c.font);
        if add {
            self.add_dr_font(b"ZaDb", font)?;
        }
        let resources = || dictionary! { "Font" => dictionary! { "ZaDb" => font } };
        let on = self.add_appearance(&c.look, c.matrix, resources(), c.on);
        let off = self.add_appearance(&c.look, c.matrix, resources(), c.off);
        let n = dictionary! { c.state => on, "Off" => off };
        self.doc
            .get_dictionary_mut(c.widget)?
            .set("AP", dictionary! { "N" => n });
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

    /// How the text field `name`'s value is set in its widget `widget` ([`Widget::id`]), to
    /// lay text out while it is typed as it will be written ([`BoxSetting::lines`]): in the
    /// font its `/DA` names when that takes the text, as [`Pdf::set_field`] writes it; `None`
    /// when it isn't a text field or has no such widget.
    ///
    /// [`Widget::id`]: crate::Widget::id
    pub fn field_setting(&self, name: &str, widget: (u32, u16)) -> Option<BoxSetting> {
        let (field, node) = (self.form_nodes().into_iter()).find(|(f, _)| f.name == name)?;
        if !matches!(field.kind, FieldKind::Text { .. }) || !node.widgets.contains(&widget) {
            return None;
        }
        let (font, size, _) = parse_da(&self.widget_da(widget, &node));
        let (mut look, _) = self.widget_look(widget).ok()?;
        look.size = size;
        look.q = self.widget_q(widget, node.q);
        let found = font.and_then(|n| self.dr_font(&n));
        let (own, base) = match found.as_ref().and_then(|o| self.dict_of(o)) {
            Some(dict) => {
                let (own, base) = own_font(&self.doc, dict);
                (Some(own), base)
            }
            None => (None, String::from("Helvetica")),
        };
        Some(BoxSetting::field(look, own, FontChoice::from_name(&base)))
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

    /// The widget's own `/Q` (0 left, 1 centred, 2 right), else its field's, `field`.
    pub(crate) fn widget_q(&self, widget: ObjectId, field: i64) -> i64 {
        (self.doc.get_dictionary(widget).ok())
            .and_then(|d| d.get(b"Q").ok())
            .and_then(|o| self.int_value(o))
            .unwrap_or(field)
    }

    /// The font to draw `text` with: the one `/DA` names when LimarePDF can write with it,
    /// else the closest standard font, added to the form's resources, or for characters
    /// the standard font lacks, its embedded face and the fonts it falls back to, named
    /// only in the appearance's own resources (`/DA` keeps the field's font: the Unicode
    /// text design, question 8). In a signed document the standard font too is named only
    /// in the appearance's resources, so that filling in a field changes only its value
    /// and drawing, as a signature allows (D-16).
    fn pick_font(&self, name: Option<&[u8]>, text: &str) -> Result<Picked> {
        let found = name.and_then(|n| Some((n.to_vec(), self.dr_font(n)?)));
        let mut base = String::from("Helvetica");
        if let Some((name, obj)) = &found {
            let (id, dict) = match obj {
                Object::Reference(id) => (Some(*id), self.doc.get_dictionary(*id).ok()),
                Object::Dictionary(d) => (None, Some(d)),
                _ => (None, None),
            };
            if let Some(dict) = dict {
                let (own, nearest) = own_font(&self.doc, dict);
                if own.takes(text) {
                    let font = match id {
                        Some(id) => FontObj::Is(id),
                        None => FontObj::Add {
                            dict: dict.clone(),
                            to_form: false,
                        },
                    };
                    return Ok(Picked {
                        name: name.clone(),
                        font: Some(font),
                        metrics: Metrics::Bytes(own.widths),
                        replaced: false,
                        choice: FontChoice::default(),
                    });
                }
                base = nearest;
            }
        }
        let choice = FontChoice::from_name(&base);
        let setter = self.setter(choice, text);
        let missing = setter.missing(&text.replace(['\n', '\r'], ""));
        if !missing.is_empty() {
            bail!("{}", cant_write(&missing));
        }
        if !matches!(setter.own(), SetIn::Standard(_)) {
            return Ok(Picked {
                name: EMBEDDED.to_vec(),
                font: None,
                metrics: Metrics::Set(setter),
                replaced: false,
                choice,
            });
        }
        let base = choice.base_font();
        let prefix = format!("Limare{}", base.replace('-', ""));
        let signed = self.holds_signature();
        // Its name, or the first numbered one, that is free or holds this font (A-05).
        let (name, font) = std::iter::once(prefix.clone())
            .chain(numbered(&prefix))
            .find_map(|n| Some((n.clone(), self.standard_under(n.as_bytes(), base, !signed)?)))
            .ok_or_else(|| anyhow!("no free name for {base} in the form's fonts"))?;
        Ok(Picked {
            name: name.into_bytes(),
            font: Some(font),
            metrics: Metrics::Set(setter),
            replaced: !signed,
            choice,
        })
    }

    /// The form's ZapfDingbats font (`/ZaDb`), added to it when it has none. When `/ZaDb` is
    /// another font, the check mark's drawing has a ZapfDingbats of its own (A-05).
    fn zapf(&self) -> FontObj {
        let base = "ZapfDingbats";
        self.standard_under(b"ZaDb", base, true)
            .unwrap_or_else(|| FontObj::Add {
                dict: standard_font(base),
                to_form: false,
            })
    }

    /// The standard font `base` under `name` in the form's fonts, when the name is free or
    /// holds exactly that font ([`is_standard`]), and `None` when it holds another: a font
    /// is never used for its name alone (A-05). A free name's font is added to the form's
    /// fonts too when `to_form`.
    fn standard_under(&self, name: &[u8], base: &str, to_form: bool) -> Option<FontObj> {
        match self.dr_font(name) {
            None | Some(Object::Null) => Some(FontObj::Add {
                dict: standard_font(base),
                to_form,
            }),
            Some(Object::Reference(id)) => {
                let dict = self.doc.get_dictionary(id).ok()?;
                is_standard(dict, base).then_some(FontObj::Is(id))
            }
            Some(Object::Dictionary(dict)) => is_standard(&dict, base).then_some(FontObj::Add {
                dict,
                to_form: false,
            }),
            Some(_) => None,
        }
    }

    /// A font in the form's default resources (`/DR /Font`), as stored there.
    pub(crate) fn dr_font(&self, name: &[u8]) -> Option<Object> {
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

/// The font `dict` (in `doc`) as a field's own font, and the name of the standard font
/// nearest it, which sets the text it doesn't take.
fn own_font(doc: &Document, dict: &Dictionary) -> (OwnFont, String) {
    let ctx = FontCtx::new(doc, dict);
    let winansi = (dict.get(b"Encoding").ok())
        .and_then(|o| o.as_name().ok())
        .is_some_and(|n| n == b"WinAnsiEncoding");
    let own = OwnFont {
        widths: Box::new(std::array::from_fn(|c| ctx.width(c as u32, 1))),
        winansi,
        reusable: ctx.reusable(),
    };
    (own, ctx.base.clone())
}

/// The standard font `base` as LimarePDF adds it to a form: in the Windows Latin encoding,
/// but for ZapfDingbats, which has its own.
fn standard_font(base: &str) -> Dictionary {
    let mut font = dictionary! { "Type" => "Font", "Subtype" => "Type1", "BaseFont" => base };
    if base != "ZapfDingbats" {
        font.set("Encoding", "WinAnsiEncoding");
    }
    font
}

/// Whether `dict` is exactly [`standard_font`]`(base)`, which may leave out `/Type` and have
/// the old `/Name`: not embedded, and no encoding, widths or text mapping of its own (S1-31).
fn is_standard(dict: &Dictionary, base: &str) -> bool {
    let mut font = dict.clone();
    font.remove(b"Name");
    if !font.has(b"Type") {
        font.set("Type", "Font");
    }
    font == standard_font(base)
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
        let s: String = s.chars().filter(|c| !matches!(c, '\n' | '\r')).collect();
        let bad = unwritable(FontChoice::default(), &s);
        if bad.is_empty() {
            return Ok(());
        }
        bail!("{}", cant_write(&bad))
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
        (FieldKind::Signature, _) => bail!(
            "\"{name}\" is a signature field, which takes a digital signature rather than a \
             value: sign it with Document > Digital signatures > Sign with digital ID in the \
             app, or `limarepdf-cli sign --field \"{name}\"`"
        ),
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

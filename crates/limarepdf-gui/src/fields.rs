//! Filling in form fields on the page: tints, the text box over a field being typed in,
//! the list of a choice's options, and moving between fields with Tab.

use crate::editor::Action;
use eframe::egui::{self, Color32, CursorIcon, Key, Rect, Stroke, StrokeKind};
use limarepdf_core::{Field, FieldKind, FieldValue, Pdf};

/// Required fields' outline while fields are highlighted.
const REQUIRED: Color32 = Color32::from_rgb(220, 50, 50);

/// The field with the keyboard. Kept by name, so it survives the fields being read anew
/// after a change.
struct Focus {
    name: String,
    /// Which of the field's widgets.
    widget: usize,
    edit: Edit,
}

enum Edit {
    /// Shown with a ring; Space or Enter acts on it.
    Ring,
    /// A text box over the field. `fresh` until it has taken the keyboard; `back` while a
    /// dialog has it, to take it back when the dialog closes.
    Text {
        text: String,
        original: String,
        fresh: bool,
        back: bool,
    },
    /// A choice's options in a list below the field; `text` is what's typed in an
    /// editable drop-down.
    Options { text: String, fresh: bool },
}

/// The typing a text box kept as its field was left, until the change is made: given
/// back if it is refused ([`FormUi::kept`]).
struct Left {
    name: String,
    widget: usize,
    text: String,
    original: String,
}

pub struct FormUi {
    /// The document's fields, read again after every change.
    fields: Option<Vec<Field>>,
    /// Tab order: (field, widget), by page, then top to bottom and left to right.
    order: Vec<(usize, usize)>,
    /// Tint every field, and outline the required ones.
    pub highlight: bool,
    focus: Option<Focus>,
    left: Option<Left>,
    /// Bring the focused field into view: set when the keyboard moves to it.
    scroll: bool,
    /// The tab's editor ids ([`crate::editor::Editor::scope`]): a text box's id is this
    /// with its field and widget.
    pub scope: egui::Id,
}

impl Default for FormUi {
    fn default() -> Self {
        Self {
            fields: None,
            order: Vec::new(),
            highlight: false,
            focus: None,
            left: None,
            scroll: false,
            scope: egui::Id::unique("form"),
        }
    }
}

impl FormUi {
    /// Forget the fields after a change to the document. The focus stays.
    pub fn invalidate(&mut self) {
        self.fields = None;
        self.order.clear();
    }

    /// The fields, once they have been read (on drawing the first page).
    pub fn fields(&self) -> &[Field] {
        self.fields.as_deref().unwrap_or(&[])
    }

    fn load(&mut self, pdf: &Pdf) {
        if self.fields.is_some() {
            return;
        }
        let fields = pdf.form_fields();
        let mut order: Vec<(usize, usize)> = Vec::new();
        for (fi, f) in fields.iter().enumerate() {
            for (wi, w) in f.widgets.iter().enumerate() {
                if !w.hidden {
                    order.push((fi, wi));
                }
            }
        }
        let key = |&(fi, wi): &(usize, usize)| {
            let w = &fields[fi].widgets[wi];
            (w.page, (w.bbox[1] / 4.0) as i64, (w.bbox[0] * 10.0) as i64)
        };
        order.sort_by_key(key);
        self.order = order;
        self.fields = Some(fields);
        if self.focused().is_none() {
            self.focus = None;
        }
    }

    /// The focused field and widget, as indices.
    fn focused(&self) -> Option<(usize, usize)> {
        let f = self.focus.as_ref()?;
        let fi = self.fields().iter().position(|x| x.name == f.name)?;
        (f.widget < self.fields()[fi].widgets.len()).then_some((fi, f.widget))
    }

    /// The focused field, for the inspector.
    pub fn focused_field(&self) -> Option<&Field> {
        self.focused().map(|(fi, _)| &self.fields()[fi])
    }

    /// A page the focused field is on that should be brought into view, until it is.
    pub fn scroll_page(&self) -> Option<usize> {
        let (fi, wi) = self.focused().filter(|_| self.scroll)?;
        Some(self.fields()[fi].widgets[wi].page)
    }

    pub fn clear_focus(&mut self) {
        self.focus = None;
    }

    /// What the text box of the field with the keyboard holds, as typed so far.
    pub fn typed_text(&self) -> Option<&str> {
        match &self.focus.as_ref()?.edit {
            Edit::Text { text, .. } => Some(text),
            _ => None,
        }
    }

    /// The field whose text box has the keyboard: its name and widget.
    pub fn typing_in(&self) -> Option<(String, usize)> {
        let f = self.focus.as_ref()?;
        matches!(f.edit, Edit::Text { .. }).then(|| (f.name.clone(), f.widget))
    }

    /// Whether a field that isn't typed in (a checkbox, a button, a list closed) has the
    /// keys, shown by a ring around it.
    pub fn ringed(&self) -> bool {
        self.focused().is_some() && matches!(self.focus.as_ref().map(|f| &f.edit), Some(Edit::Ring))
    }

    /// Give the keyboard back to widget `widget` of the field `name`, as Tab gives it.
    pub fn focus_field(&mut self, name: &str, widget: usize) {
        let at = self.fields().iter().position(|f| f.name == name);
        if let Some(fi) = at.filter(|&fi| widget < self.fields()[fi].widgets.len()) {
            self.focus_on(fi, widget, false);
        }
    }

    /// Move the keyboard to the next (or previous) field that can be filled in, starting
    /// with the first when none has it.
    pub fn step(&mut self, forward: bool) {
        let fillable: Vec<(usize, usize)> = self
            .order
            .iter()
            .copied()
            .filter(|&(fi, _)| why_not(&self.fields()[fi]).is_none())
            .collect();
        if fillable.is_empty() {
            return;
        }
        let at = self
            .focused()
            .and_then(|f| fillable.iter().position(|&x| x == f));
        let n = fillable.len();
        let next = match (at, forward) {
            (Some(i), true) => (i + 1) % n,
            (Some(i), false) => (i + n - 1) % n,
            (None, true) => 0,
            (None, false) => n - 1,
        };
        let (fi, wi) = fillable[next];
        self.focus_on(fi, wi, false);
        self.scroll = true;
    }

    /// Give a field the keyboard: a text field opens its text box, a choice its options
    /// when `open`, anything else gets a ring.
    fn focus_on(&mut self, fi: usize, wi: usize, open: bool) {
        let f = &self.fields()[fi];
        let edit = match (&f.kind, &f.value) {
            (FieldKind::Text { .. }, FieldValue::Text(t)) => Edit::Text {
                text: t.clone(),
                original: t.clone(),
                fresh: true,
                back: false,
            },
            (FieldKind::Choice { .. }, v) if open => Edit::Options {
                text: match v {
                    FieldValue::Choice(c) => c.first().cloned().unwrap_or_default(),
                    _ => String::new(),
                },
                fresh: true,
            },
            _ => Edit::Ring,
        };
        self.focus = Some(Focus {
            name: f.name.clone(),
            widget: wi,
            edit,
        });
    }

    /// Clear the focused field's value.
    pub fn clear_value(&self) -> Option<Action> {
        let f = self.focused_field()?;
        let v = match f.kind {
            FieldKind::Text { .. } => FieldValue::Text(String::new()),
            FieldKind::Check => FieldValue::Check(false),
            FieldKind::Radio | FieldKind::Choice { .. } => FieldValue::Choice(Vec::new()),
            _ => return None,
        };
        Some(set(f, v))
    }

    /// Keys for a field with a ring, or Tab to the first field; outside any text box.
    pub fn keys(&mut self, ctx: &egui::Context, acts: &mut Vec<Action>) {
        if self.fields().is_empty() || ctx.egui_wants_keyboard_input() {
            return;
        }
        let ring = matches!(self.focus.as_ref().map(|f| &f.edit), Some(Edit::Ring));
        let (tab, back, space, enter, esc) = ctx.input_mut(|i| {
            let back = i.consume_key(egui::Modifiers::SHIFT, Key::Tab);
            let tab = i.consume_key(egui::Modifiers::NONE, Key::Tab);
            let mut take = |k| ring && i.consume_key(egui::Modifiers::NONE, k);
            (
                tab,
                back,
                take(Key::Space),
                take(Key::Enter),
                take(Key::Escape),
            )
        });
        if tab || back {
            self.step(tab);
        }
        let Some((fi, wi)) = self.focused().filter(|_| ring) else {
            return;
        };
        if esc {
            self.focus = None;
        } else if space || enter {
            self.activate(fi, wi, acts);
        }
    }

    /// Whether text typed into a field isn't kept yet.
    pub fn typing(&self) -> bool {
        self.focused().is_some()
            && matches!(
                &self.focus,
                Some(Focus { edit: Edit::Text { text, original, .. }, .. }) if text != original
            )
    }

    /// Keep what was typed in the field being left, when the keyboard moves on before its
    /// text box has noticed, or before saving. Returns the value the field had, for
    /// [`Self::uncommit`].
    pub fn commit_typing(&mut self, acts: &mut Vec<Action>) -> Option<String> {
        let (fi, _) = self.focused()?;
        let Some(Focus {
            edit: Edit::Text { text, original, .. },
            ..
        }) = &mut self.focus
        else {
            return None;
        };
        if text == original {
            return None;
        }
        let was = std::mem::replace(original, text.clone());
        let v = FieldValue::Text(text.clone());
        acts.push(set(&self.fields.as_ref().unwrap()[fi], v));
        Some(was)
    }

    /// The field's typing is still to keep: what [`Self::commit_typing`] queued wasn't
    /// made, and `original` is the value the field has.
    pub fn uncommit(&mut self, original: String) {
        if let Some(Focus {
            edit: Edit::Text { original: o, .. },
            ..
        }) = &mut self.focus
        {
            *o = original;
        }
    }

    /// The change queued for field `name` was made, or refused when not `ok`. Typing its
    /// text box kept as the field was left and then refused goes back into the box, which
    /// takes the keyboard back once the reason has been read (BUG-37).
    pub fn kept(&mut self, name: &str, ok: bool) {
        let Some(l) = self.left.take().filter(|l| !ok && l.name == name) else {
            return;
        };
        self.focus = Some(Focus {
            name: l.name,
            widget: l.widget,
            edit: Edit::Text {
                text: l.text,
                original: l.original,
                fresh: false,
                back: true,
            },
        });
    }

    /// What a click on (or Space over) a field does.
    fn activate(&mut self, fi: usize, wi: usize, acts: &mut Vec<Action>) {
        if self.focused() != Some((fi, wi)) {
            self.commit_typing(acts);
        }
        let f = &self.fields()[fi];
        match (&f.kind, &f.value) {
            (FieldKind::Check, FieldValue::Check(on)) => {
                acts.push(set(f, FieldValue::Check(!on)));
                self.focus_on(fi, wi, false);
            }
            (FieldKind::Radio, _) => {
                if let Some(e) = f.widgets[wi].export.clone() {
                    acts.push(set(f, FieldValue::Choice(vec![e])));
                }
                self.focus_on(fi, wi, false);
            }
            _ => self.focus_on(fi, wi, true),
        }
    }

    /// Tints, focus, text box and options for one page; `k` is the screen size of a point.
    /// When `active` (the tool fills in fields), hovering shows what a field is and a
    /// click fills it in. Returns whether the pointer is over a field, so the tool
    /// leaves the click alone.
    #[allow(clippy::too_many_arguments)]
    pub fn page_ui(
        &mut self,
        ui: &mut egui::Ui,
        pdf: &Pdf,
        page: usize,
        r: Rect,
        k: f32,
        resp: &egui::Response,
        active: bool,
        acts: &mut Vec<Action>,
    ) -> bool {
        self.load(pdf);
        let screen = |b: [f32; 4]| {
            Rect::from_min_max(
                egui::pos2(r.min.x + b[0] * k, r.min.y + b[1] * k),
                egui::pos2(r.min.x + b[2] * k, r.min.y + b[3] * k),
            )
        };
        let here: Vec<(usize, usize, Rect)> = self
            .order
            .iter()
            .filter(|&&(fi, wi)| self.fields()[fi].widgets[wi].page == page)
            .map(|&(fi, wi)| (fi, wi, screen(self.fields()[fi].widgets[wi].bbox)))
            .collect();
        if here.is_empty() {
            return false;
        }
        let painter = ui.painter().clone();
        let accent = ui.visuals().selection.bg_fill;
        let hover = resp.hover_pos();
        let over = hover.and_then(|p| here.iter().rev().find(|w| w.2.contains(p)).copied());
        let focused = self.focused();
        for &(fi, wi, rect) in &here {
            let f = &self.fields()[fi];
            let hovered = active && over.is_some_and(|o| (o.0, o.1) == (fi, wi));
            // A light tint while fields are highlighted (on by default), stronger under the
            // pointer.
            if hovered && why_not(f).is_none() {
                painter.rect_filled(rect, 0.0, accent.gamma_multiply(0.25));
            } else if self.highlight {
                painter.rect_filled(rect, 0.0, accent.gamma_multiply(0.15));
            }
            if self.highlight && f.required {
                painter.rect_stroke(rect, 0.0, Stroke::new(1.0, REQUIRED), StrokeKind::Outside);
            }
            if focused == Some((fi, wi)) {
                if self.scroll {
                    ui.scroll_to_rect(rect.expand(24.0), Some(egui::Align::Center));
                    self.scroll = false;
                }
                let ring = Stroke::new(2.0, ui.visuals().selection.stroke.color);
                painter.rect_stroke(rect, 0.0, ring, StrokeKind::Outside);
            }
        }
        if let Some((fi, wi)) = focused.filter(|f| here.iter().any(|h| (h.0, h.1) == *f)) {
            let rect = screen(self.fields()[fi].widgets[wi].bbox);
            self.text_box(ui, fi, rect, k, acts);
            self.options(ui, fi, rect, acts);
        }
        let Some((fi, wi, _)) = over.filter(|_| active) else {
            // A click away from the fields lets go of one with a ring.
            let ring = matches!(self.focus.as_ref().map(|f| &f.edit), Some(Edit::Ring));
            if active && ring && resp.clicked() {
                self.focus = None;
            }
            return false;
        };
        let f = &self.fields()[fi];
        // An empty signature field: a click signs in it with the Sign tool. Any other
        // signature field holds a digital signature: a click shows its details.
        let sign_box = (f.kind == FieldKind::Signature)
            .then(|| f.widgets[wi].bbox)
            .filter(|b| pdf.empty_signature_boxes(page).contains(b));
        let signed = f.kind == FieldKind::Signature && sign_box.is_none();
        let tip = match why_not(f) {
            _ if signed => format!(
                "{}\nA digital signature: click to see who signed and whether it is valid",
                label(f)
            ),
            Some(why) => format!("{}\n{why}", label(f)),
            None => label(f),
        };
        if why_not(f).is_none() {
            let icon = match f.kind {
                FieldKind::Text { .. } => CursorIcon::Text,
                _ => CursorIcon::PointingHand,
            };
            ui.ctx().set_cursor_icon(icon);
        }
        if sign_box.is_some() || signed {
            ui.ctx().set_cursor_icon(CursorIcon::PointingHand);
        }
        resp.clone().on_hover_text_at_pointer(tip);
        if resp.clicked() {
            if let Some(b) = sign_box {
                acts.push(Action::SignHere(page, b));
            } else if signed {
                acts.push(Action::SignatureDetails(f.name.clone()));
            } else if why_not(f).is_none() {
                self.activate(fi, wi, acts);
            }
        }
        true
    }

    /// The text box over a text field being typed in.
    fn text_box(
        &mut self,
        ui: &mut egui::Ui,
        fi: usize,
        rect: Rect,
        k: f32,
        acts: &mut Vec<Action>,
    ) {
        let f = &self.fields()[fi];
        let FieldKind::Text {
            multiline,
            password,
            max_len,
            ..
        } = f.kind
        else {
            return;
        };
        let field = f.clone();
        let Some(Focus {
            widget,
            edit:
                Edit::Text {
                    text,
                    original,
                    fresh,
                    back,
                },
            ..
        }) = &mut self.focus
        else {
            return;
        };
        let id = self.scope.with(("field-text", &field.name, *widget));
        // The size the field is saved with, so typing looks like the result; a field that
        // fits its text to the box gets about what that gives.
        let size = if field.text_size > 0.0 {
            field.text_size * k
        } else if multiline {
            12.0 * k
        } else {
            (rect.height() * 0.6).min(12.0 * k)
        }
        .clamp(8.0, 48.0);
        // A multi-line box would otherwise be four rows high, whatever the field's size.
        let held = egui::TextEdit::load_state(ui.ctx(), id).and_then(|s| s.cursor.char_range());
        let mut buffer = crate::page_text::Clusters(text);
        let mut te = if multiline {
            egui::TextEdit::multiline(&mut buffer)
                .desired_rows(1)
                .min_size(rect.size())
        } else {
            egui::TextEdit::singleline(&mut buffer)
        }
        .id(id)
        .font(egui::FontId::proportional(size))
        .desired_width(rect.width())
        .margin(egui::Margin::same(2))
        .password(password);
        if let Some(n) = max_len {
            te = te.char_limit(n);
        }
        // Light colours in either theme: the box sits on the paper.
        let theme = ui.visuals().clone();
        *ui.visuals_mut() = egui::Visuals::light();
        let out = ui.put(rect, te);
        crate::page_text::snap_to_clusters(ui.ctx(), id, text, held);
        *ui.visuals_mut() = theme;
        if *fresh {
            out.request_focus();
            *fresh = false;
            return;
        }
        let dialog = crate::dialogs::any_open(ui.ctx());
        if *back && !dialog {
            crate::page_text::hand_back(ui.ctx(), id);
            *back = false;
            return;
        }
        if !out.lost_focus() {
            return;
        }
        // A dialog took the keyboard (a command's question, or why the typing couldn't be
        // kept): the typing stays until it closes.
        if dialog {
            *back = true;
            return;
        }
        // Tab is taken here, or `keys` would see it too and move on a second field. Esc,
        // like Enter or a click away, keeps what was typed.
        let (tab, shift) = ui.input_mut(|i| {
            let shift = i.modifiers.shift;
            (
                i.consume_key(egui::Modifiers::SHIFT, Key::Tab)
                    || i.consume_key(egui::Modifiers::NONE, Key::Tab),
                shift,
            )
        });
        if text != original {
            self.left = Some(Left {
                name: field.name.clone(),
                widget: *widget,
                text: text.clone(),
                original: std::mem::replace(original, text.clone()),
            });
            acts.push(set(&field, FieldValue::Text(text.clone())));
        }
        if tab {
            self.step(!shift);
        } else {
            self.focus = None;
        }
    }

    /// A choice's options in a list below the field. A click on one chooses it; in a
    /// multi-select list it adds or removes it, and the list stays open until a click
    /// outside it, Esc or Enter.
    fn options(&mut self, ui: &mut egui::Ui, fi: usize, rect: Rect, acts: &mut Vec<Action>) {
        let f = self.fields()[fi].clone();
        let FieldKind::Choice {
            combo,
            editable,
            multi,
        } = f.kind
        else {
            return;
        };
        let Some(Focus {
            edit: Edit::Options { text, fresh },
            ..
        }) = &mut self.focus
        else {
            return;
        };
        let chosen = match &f.value {
            FieldValue::Choice(c) => c.clone(),
            _ => Vec::new(),
        };
        let mut pick: Option<Vec<String>> = None;
        // Enter closes a multi-select list. It is taken first, or an option with the
        // keyboard would take it as a click (Space chooses one).
        let mut close =
            multi && !*fresh && ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, Key::Enter));
        let area = egui::Area::new(self.scope.with(("field-options", &f.name)))
            .order(egui::Order::Foreground)
            .fixed_pos(rect.left_bottom())
            .show(ui.ctx(), |ui| {
                egui::Frame::popup(ui.style()).show(ui, |ui| {
                    ui.set_min_width(rect.width().max(120.0));
                    if combo && editable {
                        let r = ui.text_edit_singleline(text);
                        if *fresh {
                            r.request_focus();
                        }
                        if r.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter)) {
                            pick = Some(vec![text.clone()]);
                        }
                    }
                    egui::ScrollArea::vertical()
                        .max_height(240.0)
                        .show(ui, |ui| {
                            for (export, shown) in &f.options {
                                let on = chosen.contains(export);
                                if ui.add(egui::Button::selectable(on, shown)).clicked() {
                                    pick = Some(if multi {
                                        let mut c = chosen.clone();
                                        if on {
                                            c.retain(|x| x != export);
                                        } else {
                                            c.push(export.clone());
                                        }
                                        c
                                    } else {
                                        vec![export.clone()]
                                    });
                                }
                            }
                        });
                });
            });
        let first = std::mem::take(fresh);
        let esc = ui.input(|i| i.key_pressed(Key::Escape));
        if !first && (area.response.clicked_elsewhere() || esc) {
            close = true;
        }
        if let Some(c) = pick {
            if c != chosen {
                acts.push(set(&f, FieldValue::Choice(c)));
            }
            close |= !multi;
        }
        if close {
            if let Some(focus) = &mut self.focus {
                focus.edit = Edit::Ring;
            }
        }
    }
}

/// The change that sets a field, on the pages it is drawn on.
fn set(f: &Field, v: FieldValue) -> Action {
    let mut pages: Vec<usize> = f.widgets.iter().map(|w| w.page).collect();
    pages.sort_unstable();
    pages.dedup();
    Action::SetField(pages, f.name.clone(), v)
}

/// The field's description, or its name, for hovering.
fn label(f: &Field) -> String {
    let mut s = f.tooltip.clone().unwrap_or_else(|| f.name.clone());
    if f.required {
        s.push_str(" (required)");
    }
    s
}

/// Why a field can't be filled in, if it can't.
pub fn why_not(f: &Field) -> Option<&'static str> {
    match f.kind {
        _ if f.read_only => Some("Read-only"),
        FieldKind::Button => Some("A button: it runs a script, which LimarePDF doesn't do"),
        FieldKind::Signature => Some(
            "A signature field: click to put your signature in it (a picture of it, not a \
             digital signature)",
        ),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use lopdf::{dictionary, Dictionary, Object, StringFormat};

    fn text(s: &str) -> Object {
        Object::String(s.as_bytes().to_vec(), StringFormat::Literal)
    }

    /// Two pages; each field is its own widget, on the page and at the height (from the
    /// bottom) given.
    fn form(fields: Vec<(usize, i64, Dictionary)>) -> Pdf {
        let mut p = Pdf::new();
        for i in 0..2 {
            p.insert_blank(i, 300.0, 300.0).unwrap();
        }
        let pages = p.page_ids();
        let mut top = Vec::new();
        let mut annots = vec![Vec::new(), Vec::new()];
        for (page, y, mut d) in fields {
            d.set("Type", "Annot");
            d.set("Subtype", "Widget");
            d.set(
                "Rect",
                vec![10.into(), y.into(), 100.into(), (y + 20).into()],
            );
            let id = p.doc.add_object(d);
            top.push(Object::from(id));
            annots[page].push(Object::from(id));
        }
        for (pg, a) in pages.iter().zip(annots) {
            p.doc.get_dictionary_mut(*pg).unwrap().set("Annots", a);
        }
        let f = p.doc.add_object(dictionary! { "Fields" => top });
        p.doc.catalog_mut().unwrap().set("AcroForm", f);
        p
    }

    fn loaded(p: &Pdf) -> FormUi {
        let mut ui = FormUi::default();
        ui.load(p);
        ui
    }

    fn focused_name(ui: &FormUi) -> Option<&str> {
        ui.focused_field().map(|f| f.name.as_str())
    }

    #[test]
    fn tab_goes_through_fillable_fields_in_page_order() {
        let p = form(vec![
            (1, 200, dictionary! { "FT" => "Tx", "T" => text("p2") }),
            (0, 100, dictionary! { "FT" => "Tx", "T" => text("low") }),
            (0, 250, dictionary! { "FT" => "Tx", "T" => text("high") }),
            (
                0,
                200,
                dictionary! { "FT" => "Tx", "T" => text("locked"), "Ff" => 1 },
            ),
            (
                0,
                150,
                dictionary! { "FT" => "Btn", "Ff" => 1 << 16, "T" => text("go") },
            ),
        ]);
        let mut ui = loaded(&p);
        let mut seen = Vec::new();
        for _ in 0..4 {
            ui.step(true);
            seen.push(focused_name(&ui).unwrap().to_string());
        }
        // Top to bottom on page 1 (the top is the larger height), then page 2, then
        // round again; read-only fields and buttons are skipped.
        assert_eq!(seen, ["high", "low", "p2", "high"]);
        ui.step(false);
        assert_eq!(focused_name(&ui), Some("p2"));
        assert_eq!(ui.scroll_page(), Some(1));
    }

    #[test]
    fn clicks_tick_boxes_choose_radios_and_open_text_and_lists() {
        let on = |s: &str| dictionary! { "N" => dictionary! { s => Object::Null, "Off" => Object::Null } };
        let p = form(vec![
            (
                0,
                250,
                dictionary! { "FT" => "Btn", "T" => text("agree"), "AP" => on("Yes") },
            ),
            (
                0,
                200,
                dictionary! { "FT" => "Btn", "Ff" => 1 << 15, "T" => text("r"), "AP" => on("a") },
            ),
            (
                0,
                150,
                dictionary! { "FT" => "Tx", "T" => text("name"), "V" => text("Ann") },
            ),
            (
                0,
                100,
                dictionary! { "FT" => "Ch", "T" => text("pick"), "Opt" => vec![text("x")] },
            ),
        ]);
        let mut ui = loaded(&p);
        let mut acts = Vec::new();
        let at = |ui: &FormUi, name: &str| ui.fields().iter().position(|f| f.name == name).unwrap();

        ui.activate(at(&ui, "agree"), 0, &mut acts);
        ui.activate(at(&ui, "r"), 0, &mut acts);
        let got: Vec<(String, FieldValue)> = acts
            .drain(..)
            .map(|a| match a {
                Action::SetField(pages, name, v) => {
                    assert_eq!(pages, [0]);
                    (name, v)
                }
                _ => panic!("not a field change"),
            })
            .collect();
        assert_eq!(
            got,
            [
                ("agree".to_string(), FieldValue::Check(true)),
                ("r".to_string(), FieldValue::Choice(vec!["a".into()])),
            ]
        );

        // A text field opens its text box with the value; a list opens its options.
        ui.activate(at(&ui, "name"), 0, &mut acts);
        assert!(matches!(
            &ui.focus.as_ref().unwrap().edit,
            Edit::Text { text, .. } if text == "Ann"
        ));
        ui.activate(at(&ui, "pick"), 0, &mut acts);
        assert!(matches!(
            ui.focus.as_ref().unwrap().edit,
            Edit::Options { .. }
        ));
        assert!(acts.is_empty(), "nothing typed, nothing set");

        // Clearing gives the empty value of the field's kind.
        match ui.clear_value() {
            Some(Action::SetField(_, name, v)) => {
                assert_eq!(name, "pick");
                assert_eq!(v, FieldValue::Choice(vec![]));
            }
            _ => panic!("no clear"),
        }
    }

    #[test]
    fn typing_is_kept_when_another_field_is_clicked() {
        let p = form(vec![
            (0, 250, dictionary! { "FT" => "Tx", "T" => text("a") }),
            (0, 200, dictionary! { "FT" => "Tx", "T" => text("b") }),
        ]);
        let mut ui = loaded(&p);
        let mut acts = Vec::new();
        ui.activate(0, 0, &mut acts);
        if let Some(Focus {
            edit: Edit::Text { text, .. },
            ..
        }) = &mut ui.focus
        {
            text.push_str("typed");
        }
        ui.activate(1, 0, &mut acts);
        assert!(matches!(
            acts.as_slice(),
            [Action::SetField(_, name, FieldValue::Text(t))] if name == "a" && t == "typed"
        ));
        assert_eq!(focused_name(&ui), Some("b"));
    }

    #[test]
    fn typing_counts_until_it_is_kept() {
        let p = form(vec![(
            0,
            250,
            dictionary! { "FT" => "Tx", "T" => text("a") },
        )]);
        let mut ui = loaded(&p);
        let mut acts = Vec::new();
        ui.activate(0, 0, &mut acts);
        // In the field, nothing typed yet.
        assert!(!ui.typing());
        if let Some(Focus {
            edit: Edit::Text { text, .. },
            ..
        }) = &mut ui.focus
        {
            text.push_str("typed");
        }
        assert!(ui.typing());
        ui.commit_typing(&mut acts);
        assert!(matches!(
            acts.as_slice(),
            [Action::SetField(_, name, FieldValue::Text(t))] if name == "a" && t == "typed"
        ));
        assert!(!ui.typing(), "still counted once kept");
    }

    /// Draw the open text box or list of field `fi`, over a field at a fixed place, with
    /// `events`; returns the AccessKit tree.
    fn shown(
        ui: &mut FormUi,
        ctx: &egui::Context,
        fi: usize,
        events: Vec<egui::Event>,
        acts: &mut Vec<Action>,
    ) -> egui::accesskit::TreeUpdate {
        let raw = egui::RawInput {
            screen_rect: Some(Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(800.0, 600.0),
            )),
            events,
            ..Default::default()
        };
        let rect = Rect::from_min_size(egui::pos2(100.0, 100.0), egui::vec2(90.0, 20.0));
        let mut out = ctx.run_ui(raw, |eui| {
            ui.text_box(eui, fi, rect, 1.0, acts);
            ui.options(eui, fi, rect, acts);
        });
        out.textures_delta.clear();
        out.platform_output.accesskit_update.expect("the widgets")
    }

    fn key(key: Key) -> egui::Event {
        egui::Event::Key {
            key,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::NONE,
        }
    }

    fn click(at: egui::Pos2) -> Vec<egui::Event> {
        let press = |pressed| egui::Event::PointerButton {
            pos: at,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: Default::default(),
        };
        vec![egui::Event::PointerMoved(at), press(true), press(false)]
    }

    #[test]
    fn esc_keeps_what_was_typed_in_a_text_field() {
        let p = form(vec![(
            0,
            250,
            dictionary! { "FT" => "Tx", "T" => text("a") },
        )]);
        let mut ui = loaded(&p);
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        let mut acts = Vec::new();
        ui.activate(0, 0, &mut acts);
        shown(&mut ui, &ctx, 0, vec![], &mut acts);
        let typed = vec![egui::Event::Text("Walrus".into())];
        shown(&mut ui, &ctx, 0, typed, &mut acts);
        assert!(ui.typing() && acts.is_empty());
        shown(&mut ui, &ctx, 0, vec![key(Key::Escape)], &mut acts);
        shown(&mut ui, &ctx, 0, vec![], &mut acts);
        assert!(matches!(
            acts.as_slice(),
            [Action::SetField(_, name, FieldValue::Text(t))] if name == "a" && t == "Walrus"
        ));
        assert!(!ui.typing());
    }

    /// A multi-select list has no Done: each click applies, the list stays open, and a
    /// click outside it, Esc or Enter closes it with the choices kept.
    #[test]
    fn a_multi_select_list_applies_as_clicked_and_closes_without_done() {
        let opts = vec![text("Walrus"), text("Seal"), text("Puffin")];
        let mut p = form(vec![(
            0,
            250,
            dictionary! { "FT" => "Ch", "T" => text("pick"), "Ff" => 1 << 21, "Opt" => opts },
        )]);
        let mut ui = loaded(&p);
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        let mut acts = Vec::new();
        let open = |ui: &mut FormUi| {
            ui.focus = None;
            ui.activate(0, 0, &mut Vec::new());
            assert!(matches!(
                ui.focus.as_ref().unwrap().edit,
                Edit::Options { .. }
            ));
        };
        let is_open = |ui: &FormUi| matches!(ui.focus.as_ref().unwrap().edit, Edit::Options { .. });
        let option = |tree: &egui::accesskit::TreeUpdate, name: &str| {
            let (_, n) = tree
                .nodes
                .iter()
                .find(|(_, n)| n.label() == Some(name) || n.value() == Some(name))
                .unwrap_or_else(|| panic!("no {name}"));
            let b = n.bounds().expect("placed");
            egui::pos2((b.x0 + b.x1) as f32 / 2.0, (b.y0 + b.y1) as f32 / 2.0)
        };
        open(&mut ui);
        shown(&mut ui, &ctx, 0, vec![], &mut acts);
        let mut tree = shown(&mut ui, &ctx, 0, vec![], &mut acts);
        assert!(
            !tree.nodes.iter().any(|(_, n)| n.label() == Some("Done")),
            "the list still has Done"
        );
        // Each click applies at once, and the list stays open for the next.
        for (name, want) in [
            ("Walrus", vec!["Walrus"]),
            ("Puffin", vec!["Walrus", "Puffin"]),
        ] {
            let at = option(&tree, name);
            shown(&mut ui, &ctx, 0, click(at), &mut acts);
            let Some(Action::SetField(_, _, v)) = acts.pop() else {
                panic!("{name} not applied");
            };
            let want: Vec<String> = want.into_iter().map(String::from).collect();
            assert_eq!(v, FieldValue::Choice(want));
            assert!(is_open(&ui), "closed by choosing {name}");
            p.set_field("pick", v).unwrap();
            ui.invalidate();
            ui.load(&p);
            tree = shown(&mut ui, &ctx, 0, vec![], &mut acts);
        }
        // Enter, Esc and a click outside each close it, leaving the field's ring and the
        // choices as they are.
        let closers = [
            vec![key(Key::Enter)],
            vec![key(Key::Escape)],
            click(egui::pos2(600.0, 500.0)),
        ];
        for events in closers {
            open(&mut ui);
            shown(&mut ui, &ctx, 0, vec![], &mut acts);
            shown(&mut ui, &ctx, 0, events, &mut acts);
            assert!(matches!(ui.focus.as_ref().unwrap().edit, Edit::Ring));
        }
        assert!(acts.is_empty(), "closing changed the choices");
        assert_eq!(
            ui.fields()[0].value,
            FieldValue::Choice(vec!["Walrus".into(), "Puffin".into()])
        );
    }

    #[test]
    fn focus_survives_the_fields_being_read_again() {
        let p = form(vec![(
            0,
            250,
            dictionary! { "FT" => "Tx", "T" => text("a") },
        )]);
        let mut ui = loaded(&p);
        ui.step(true);
        ui.invalidate();
        ui.load(&p);
        assert_eq!(focused_name(&ui), Some("a"));
    }
}

//! Filling in form fields on the page: tints, the text box over a field being typed in,
//! the list of a choice's options, and moving between fields with Tab.

use crate::editor::Action;
use eframe::egui::{self, Color32, CursorIcon, Key, Rect, Stroke, StrokeKind};
use mpe_core::{Field, FieldKind, FieldValue, Pdf};

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
    /// A text box over the field. `fresh` until it has taken the keyboard.
    Text {
        text: String,
        original: String,
        fresh: bool,
    },
    /// A choice's options in a list below the field; `text` is what's typed in an
    /// editable drop-down.
    Options { text: String, fresh: bool },
}

#[derive(Default)]
pub struct FormUi {
    /// The document's fields, read again after every change.
    fields: Option<Vec<Field>>,
    /// Tab order: (field, widget), by page, then top to bottom and left to right.
    order: Vec<(usize, usize)>,
    /// Tint every field, and outline the required ones.
    pub highlight: bool,
    focus: Option<Focus>,
    /// Bring the focused field into view: set when the keyboard moves to it.
    scroll: bool,
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

    /// Keep what was typed in the field being left, when the keyboard moves on before its
    /// text box has noticed.
    fn commit_typing(&mut self, acts: &mut Vec<Action>) {
        let Some((fi, _)) = self.focused() else {
            return;
        };
        if let Some(Focus {
            edit: Edit::Text { text, original, .. },
            ..
        }) = &mut self.focus
        {
            if text != original {
                *original = text.clone();
                let v = FieldValue::Text(text.clone());
                acts.push(set(&self.fields.as_ref().unwrap()[fi], v));
            }
        }
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
            if self.highlight || (hovered && why_not(f).is_none()) {
                painter.rect_filled(rect, 0.0, accent.gamma_multiply(0.25));
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
        let tip = match why_not(f) {
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
        resp.clone().on_hover_text_at_pointer(tip);
        if resp.clicked() && why_not(f).is_none() {
            self.activate(fi, wi, acts);
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
            edit:
                Edit::Text {
                    text,
                    original,
                    fresh,
                },
            ..
        }) = &mut self.focus
        else {
            return;
        };
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
        let mut te = if multiline {
            egui::TextEdit::multiline(text)
                .desired_rows(1)
                .min_size(rect.size())
        } else {
            egui::TextEdit::singleline(text)
        }
        .id(egui::Id::new(("field-text", &field.name)))
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
        *ui.visuals_mut() = theme;
        if *fresh {
            out.request_focus();
            *fresh = false;
            return;
        }
        if !out.lost_focus() {
            return;
        }
        // Tab is taken here, or `keys` would see it too and move on a second field.
        let (esc, tab, shift) = ui.input_mut(|i| {
            let shift = i.modifiers.shift;
            (
                i.key_pressed(Key::Escape),
                i.consume_key(egui::Modifiers::SHIFT, Key::Tab)
                    || i.consume_key(egui::Modifiers::NONE, Key::Tab),
                shift,
            )
        });
        if esc {
            self.focus = None;
            return;
        }
        if text != original {
            *original = text.clone();
            acts.push(set(&field, FieldValue::Text(text.clone())));
        }
        if tab {
            self.step(!shift);
        } else {
            self.focus = None;
        }
    }

    /// A choice's options in a list below the field. A click on one chooses it; in a
    /// multi-select list it adds or removes it, and the list stays open.
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
        let mut close = false;
        let area = egui::Area::new(egui::Id::new(("field-options", &f.name)))
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
                    if multi && ui.button("Done").clicked() {
                        close = true;
                    }
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
        FieldKind::Button => Some("A button: it runs a script, which MPE doesn't do"),
        FieldKind::Signature => Some("A signature field: MPE can't sign yet"),
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

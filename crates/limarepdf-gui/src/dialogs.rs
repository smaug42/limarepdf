//! The dialogs that remain: printing, the password prompt, messages and confirmations.
//! Document and page settings live in the inspector (see `inspector.rs`).

use eframe::egui::{self, Align2, Context, Ui};
use limarepdf_core::print::{Printer, Sides};
use std::path::PathBuf;

#[derive(PartialEq, Eq)]
pub enum Outcome {
    Open,
    Cancel,
    Apply,
}

fn modal(ctx: &Context, title: &str, body: impl FnOnce(&mut Ui) -> bool) -> Outcome {
    let mut out = Outcome::Open;
    let mut open = true;
    egui::Window::new(title)
        .collapsible(false)
        .resizable(false)
        .anchor(Align2::CENTER_CENTER, [0.0, 0.0])
        .open(&mut open)
        .show(ctx, |ui| {
            if body(ui) {
                out = Outcome::Apply;
            }
        });
    if !open {
        return Outcome::Cancel;
    }
    out
}

fn buttons(ui: &mut Ui, apply: &str) -> bool {
    ui.add_space(8.0);
    ui.separator();
    ui.horizontal(|ui| ui.button(apply).clicked()).inner
}

fn range_field(ui: &mut Ui, text: &mut String) {
    ui.horizontal(|ui| {
        ui.label("Pages:");
        pages_box(ui, text, None);
    });
}

/// A box for a page range, such as `1-3,5,8-` or `all`, `width` wide if given. Going into
/// it while it says `all` selects that, so typing page numbers replaces it.
pub fn pages_box(ui: &mut Ui, text: &mut String, width: Option<f32>) -> egui::Response {
    let all = text.trim() == "all";
    let mut edit = egui::TextEdit::singleline(text);
    if let Some(w) = width {
        edit = edit.desired_width(w);
    }
    let mut out = edit.show(ui);
    if all && out.response.response.gained_focus() {
        let range = egui::text::CCursorRange::select_all(&out.galley);
        out.state.cursor.set_char_range(Some(range));
        out.state.store(ui.ctx(), out.response.response.id);
    }
    out.response
        .response
        .on_hover_text("e.g. 1-3,5,8-  or  all")
}

// ------------------------------------------------------------------ print

/// How selected text is printed or saved.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Layout {
    /// The pages cropped to the selection, as they look.
    Original,
    /// The words alone, laid out afresh in Helvetica.
    Plain,
    /// A .txt file (saving only).
    TextFile,
}
impl Layout {
    fn label(self) -> &'static str {
        match self {
            Layout::Original => "As it looks (cropped pages)",
            Layout::Plain => "Plain text (PDF)",
            Layout::TextFile => "Text file (.txt)",
        }
    }
    fn tip(self) -> &'static str {
        match self {
            Layout::Original => {
                "Each page at its full width, cut down from top and bottom to the selected \
                 lines. Fonts, pictures and layout stay; anything else beside those lines \
                 shows too."
            }
            Layout::Plain => {
                "Just the selected words, set in Helvetica. Fonts and pictures are lost, and \
                 characters outside Western European alphabets come out as ?."
            }
            Layout::TextFile => "Just the selected words, as plain text.",
        }
    }
}

fn layout_choice(ui: &mut Ui, layout: &mut Layout, choices: &[Layout]) {
    ui.label("Layout");
    ui.vertical(|ui| {
        for &x in choices {
            ui.radio_value(layout, x, x.label()).on_hover_text(x.tip());
        }
    });
    ui.end_row();
}

pub struct Print {
    pub printers: Vec<Printer>,
    pub printer: usize,
    pub pages: String,
    pub copies: u32,
    pub sides: Sides,
    pub as_image: bool,
    /// Printing the selected text, in this layout, instead of pages.
    pub selection: Option<Layout>,
}
impl Print {
    /// `printers` has the default first.
    pub fn new(printers: Vec<Printer>) -> Self {
        Self {
            printers,
            printer: 0,
            pages: "all".into(),
            copies: 1,
            sides: Sides::One,
            as_image: false,
            selection: None,
        }
    }
}
pub fn print(ctx: &Context, s: &mut Print) -> Outcome {
    let title = match s.selection {
        Some(_) => "Print selected text",
        None => "Print",
    };
    modal(ctx, title, |ui| {
        if s.printers.is_empty() {
            ui.label("No printers are set up. Add one in the system's printer settings.");
            return buttons(ui, "OK");
        }
        egui::Grid::new("print")
            .num_columns(2)
            .spacing([10.0, 6.0])
            .show(ui, |ui| {
                ui.label("Printer");
                let name = |p: &Printer| {
                    if p.default {
                        format!("{} (default)", p.name)
                    } else {
                        p.name.clone()
                    }
                };
                egui::ComboBox::from_id_salt("printer")
                    .selected_text(name(&s.printers[s.printer]))
                    .show_ui(ui, |ui| {
                        for (i, p) in s.printers.iter().enumerate() {
                            ui.selectable_value(&mut s.printer, i, name(p));
                        }
                    });
                ui.end_row();
                ui.label("Copies");
                ui.add(egui::DragValue::new(&mut s.copies).range(1..=999));
                ui.end_row();
                ui.label("Two-sided");
                let label = |x: Sides| match x {
                    Sides::One => "Off",
                    Sides::LongEdge => "Flip on long edge",
                    Sides::ShortEdge => "Flip on short edge",
                };
                egui::ComboBox::from_id_salt("sides")
                    .selected_text(label(s.sides))
                    .show_ui(ui, |ui| {
                        for x in [Sides::One, Sides::LongEdge, Sides::ShortEdge] {
                            ui.selectable_value(&mut s.sides, x, label(x));
                        }
                    });
                ui.end_row();
                if let Some(layout) = &mut s.selection {
                    layout_choice(ui, layout, &[Layout::Original, Layout::Plain]);
                }
            });
        if s.selection.is_none() {
            range_field(ui, &mut s.pages);
        }
        // Windows always prints pages drawn by LimarePDF.
        if !cfg!(windows) {
            let tip = "LimarePDF draws each page at 300 dpi and sends pictures of them, so the \
                       printout matches the screen. Use this if the printout looks wrong; \
                       text is less sharp and the job is larger.";
            ui.checkbox(&mut s.as_image, "Print as image")
                .on_hover_text(tip);
        }
        buttons(ui, "Print")
    })
}

/// Save the selected text: choose the layout; the file dialog follows.
pub fn save_selection(ctx: &Context, layout: &mut Layout) -> Outcome {
    modal(ctx, "Save selected text", |ui| {
        egui::Grid::new("save_selection")
            .num_columns(2)
            .spacing([10.0, 6.0])
            .show(ui, |ui| {
                let all = [Layout::Original, Layout::Plain, Layout::TextFile];
                layout_choice(ui, layout, &all);
            });
        buttons(ui, "Save…")
    })
}

/// Remove hidden information: each kind, how many the document has, and whether it is
/// chosen.
pub struct RemoveHidden {
    pub kinds: Vec<(limarepdf_core::Hidden, usize, bool)>,
}

pub fn remove_hidden(ctx: &Context, s: &mut RemoveHidden) -> Outcome {
    modal(ctx, "Remove hidden information", |ui| {
        ui.label(
            "Information that travels with the file but isn't on its pages. What is ticked is \
             removed; once saved, it is gone from the file. Undo brings it back until you close \
             the document.",
        );
        ui.add_space(6.0);
        for (h, n, on) in &mut s.kinds {
            if *h == limarepdf_core::Hidden::Comments {
                ui.add_space(4.0);
                ui.label("Often wanted, so not ticked at first:");
            }
            let found = if *n == 0 {
                "none".to_string()
            } else {
                format!("{n} found")
            };
            ui.add_enabled(
                *n > 0,
                egui::Checkbox::new(on, format!("{} ({found})", h.label())),
            );
        }
        let any = s.kinds.iter().any(|(_, n, on)| *on && *n > 0);
        ui.add_space(8.0);
        ui.separator();
        ui.horizontal(|ui| ui.add_enabled(any, egui::Button::new("Remove")).clicked())
            .inner
    })
}

pub struct Password {
    pub path: PathBuf,
    pub input: String,
    pub error: Option<String>,
}
pub fn password(ctx: &Context, s: &mut Password) -> Outcome {
    modal(ctx, "Password required", |ui| {
        ui.label(format!(
            "{} is protected.",
            s.path
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default()
        ));
        let r = ui.add(
            egui::TextEdit::singleline(&mut s.input)
                .password(true)
                .hint_text("Password"),
        );
        if let Some(e) = &s.error {
            ui.colored_label(egui::Color32::LIGHT_RED, e);
        }
        let go = buttons(ui, "Open");
        go || (r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)))
    })
}

/// The name written into comments and markup; empty for none.
pub fn comment_name(ctx: &Context, name: &mut String) -> Outcome {
    modal(ctx, "Your name for comments", |ui| {
        ui.label(
            "Comments and markup you add are saved with this name, so others see who made \
             them. Leave it empty to save no name.",
        );
        let r = ui.add(egui::TextEdit::singleline(name).hint_text("No name"));
        let go = buttons(ui, "Save");
        go || (r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)))
    })
}

pub fn message(ctx: &Context, title: &str, text: &str) -> Outcome {
    modal(ctx, title, |ui| {
        ui.label(text);
        buttons(ui, "OK")
    })
}

pub fn confirm(ctx: &Context, title: &str, text: &str, yes: &str) -> Outcome {
    modal(ctx, title, |ui| {
        ui.label(text);
        buttons(ui, yes)
    })
}

// ------------------------------------------------------------------ recovery

/// A document offered in the restore window (the recovery design's section 7).
pub struct Offer {
    /// Its title, or `None` for a copy that can't be restored.
    pub title: Option<String>,
    /// Where from, when, how many pages, and until when it is kept; or why it can't be
    /// restored.
    pub detail: String,
}

/// What was pressed in the restore window: a button for the offer at an index, or Not now.
#[derive(PartialEq, Eq, Debug)]
pub enum RestoreOut {
    Open,
    Restore(usize),
    Discard(usize),
    ShowFile(usize),
    NotNow,
}

/// At start, the documents LimarePDF kept recovery copies of when it last stopped without
/// closing normally, each with Restore and Discard (Show file and Discard for a copy that
/// can't be restored); Not now keeps them all for the next start.
pub fn restore(ctx: &Context, offers: &[Offer]) -> RestoreOut {
    let mut out = RestoreOut::Open;
    egui::Window::new("Recover unsaved changes")
        .collapsible(false)
        .resizable(false)
        .anchor(Align2::CENTER_CENTER, [0.0, 0.0])
        .show(ctx, |ui| {
            ui.set_max_width(520.0);
            ui.label(
                "LimarePDF stopped without closing normally. It kept copies of these documents \
                 with their unsaved changes, as they were at the time shown. A restored \
                 document opens as a new, unsaved document; the original file isn't changed.",
            );
            ui.add_space(6.0);
            egui::ScrollArea::vertical()
                .max_height(360.0)
                .show(ui, |ui| {
                    for (i, o) in offers.iter().enumerate() {
                        ui.separator();
                        match &o.title {
                            Some(t) => ui.strong(t),
                            None => ui.strong("A copy that can't be restored"),
                        };
                        ui.add(egui::Label::new(&o.detail).wrap());
                        ui.horizontal(|ui| {
                            if o.title.is_some() {
                                if ui.button("Restore").clicked() {
                                    out = RestoreOut::Restore(i);
                                }
                            } else if ui.button("Show file").clicked() {
                                out = RestoreOut::ShowFile(i);
                            }
                            if ui.button("Discard").clicked() {
                                out = RestoreOut::Discard(i);
                            }
                        });
                    }
                });
            ui.add_space(8.0);
            ui.separator();
            let later = ui.button("Not now").on_hover_text(
                "Keep the copies, and ask again the next time LimarePDF starts (until the date \
                 shown).",
            );
            if later.clicked() {
                out = RestoreOut::NotNow;
            }
        });
    out
}

/// What was picked in [`choose`].
#[derive(PartialEq, Eq, Debug)]
pub enum Choice {
    Open,
    Cancel,
    First,
    Second,
}

/// A question with two answers, `first` the safer one; closing the window cancels.
pub fn choose(ctx: &Context, title: &str, text: &str, first: &str, second: &str) -> Choice {
    let mut out = Choice::Open;
    let mut open = true;
    egui::Window::new(title)
        .collapsible(false)
        .resizable(false)
        .anchor(Align2::CENTER_CENTER, [0.0, 0.0])
        .open(&mut open)
        .show(ctx, |ui| {
            ui.label(text);
            ui.add_space(8.0);
            ui.separator();
            ui.horizontal(|ui| {
                if ui.button(first).clicked() {
                    out = Choice::First;
                }
                if ui.button(second).clicked() {
                    out = Choice::Second;
                }
            });
        });
    if !open {
        return Choice::Cancel;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Shows a pages box holding `text` for one frame, given `events`; returns where it is.
    fn frame(ctx: &Context, text: &mut String, events: Vec<egui::Event>) -> egui::Rect {
        let raw = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(400.0, 200.0),
            )),
            events,
            ..Default::default()
        };
        let mut rect = egui::Rect::NOTHING;
        let mut out = ctx.run_ui(raw, |ui| rect = pages_box(ui, text, None).rect);
        out.textures_delta.clear();
        rect
    }

    /// Clicks in the middle of a pages box holding `text`, then types `typed`.
    fn click_and_type(text: &str, typed: &str) -> String {
        let ctx = Context::default();
        let mut text = text.to_string();
        let at = frame(&ctx, &mut text, vec![]).center();
        let click = |pressed| egui::Event::PointerButton {
            pos: at,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: Default::default(),
        };
        frame(
            &ctx,
            &mut text,
            vec![egui::Event::PointerMoved(at), click(true)],
        );
        frame(&ctx, &mut text, vec![click(false)]);
        frame(&ctx, &mut text, vec![]);
        frame(&ctx, &mut text, vec![egui::Event::Text(typed.into())]);
        text
    }

    #[test]
    fn typing_into_a_pages_box_saying_all_replaces_it() {
        assert_eq!(click_and_type("all", "2-4"), "2-4");
    }

    #[test]
    fn other_page_ranges_are_edited_where_clicked() {
        // The click lands past the end of the text, so typing adds to it.
        assert_eq!(click_and_type("1-3", ",5"), "1-3,5");
    }
}

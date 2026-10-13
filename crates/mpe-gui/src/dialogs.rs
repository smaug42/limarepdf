//! The dialogs that remain: printing, the password prompt, messages and confirmations.
//! Document and page settings live in the inspector (see `inspector.rs`).

use eframe::egui::{self, Align2, Context, Ui};
use mpe_core::print::{Printer, Sides};
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
        ui.text_edit_singleline(text)
            .on_hover_text("e.g. 1-3,5,8-  or  all");
    });
}

// ------------------------------------------------------------------ print

pub struct Print {
    pub printers: Vec<Printer>,
    pub printer: usize,
    pub pages: String,
    pub copies: u32,
    pub sides: Sides,
    pub as_image: bool,
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
        }
    }
}
pub fn print(ctx: &Context, s: &mut Print) -> Outcome {
    modal(ctx, "Print", |ui| {
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
            });
        range_field(ui, &mut s.pages);
        // Windows always prints pages drawn by MPE.
        if !cfg!(windows) {
            let tip = "MPE draws each page at 300 dpi and sends pictures of them, so the \
                       printout matches the screen. Use this if the printout looks wrong; \
                       text is less sharp and the job is larger.";
            ui.checkbox(&mut s.as_image, "Print as image")
                .on_hover_text(tip);
        }
        buttons(ui, "Print")
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

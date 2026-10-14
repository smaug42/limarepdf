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

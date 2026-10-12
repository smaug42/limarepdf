use eframe::egui::{self, Align2, Context, Ui};
use mpe_core::{Cipher, Info, Restrictions};
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

// -------------------------------------------------------------- watermark

pub struct Wm {
    pub text: String,
    pub size: f32,
    pub opacity: f32,
    pub rotation: f32,
    pub color: [f32; 3],
    pub foreground: bool,
    pub range: String,
}
impl Default for Wm {
    fn default() -> Self {
        Self {
            text: "DRAFT".into(),
            size: 96.0,
            opacity: 0.25,
            rotation: 45.0,
            color: [0.6, 0.0, 0.0],
            foreground: true,
            range: "all".into(),
        }
    }
}
pub fn watermark(ctx: &Context, s: &mut Wm) -> Outcome {
    modal(ctx, "Watermark", |ui| {
        egui::Grid::new("wm")
            .num_columns(2)
            .spacing([10.0, 6.0])
            .show(ui, |ui| {
                ui.label("Text");
                ui.text_edit_singleline(&mut s.text);
                ui.end_row();
                ui.label("Size");
                ui.add(egui::Slider::new(&mut s.size, 12.0..=300.0));
                ui.end_row();
                ui.label("Opacity");
                ui.add(egui::Slider::new(&mut s.opacity, 0.02..=1.0));
                ui.end_row();
                ui.label("Rotation");
                ui.add(egui::Slider::new(&mut s.rotation, -90.0..=90.0).suffix("°"));
                ui.end_row();
                ui.label("Colour");
                ui.color_edit_button_rgb(&mut s.color);
                ui.end_row();
                ui.label("Placement");
                ui.horizontal(|ui| {
                    ui.radio_value(&mut s.foreground, true, "Above content");
                    ui.radio_value(&mut s.foreground, false, "Behind content");
                });
                ui.end_row();
            });
        range_field(ui, &mut s.range);
        buttons(ui, "Apply")
    })
}

// ---------------------------------------------------- header / footer / Bates

pub struct Hf {
    pub header: [String; 3],
    pub footer: [String; 3],
    pub size: f32,
    pub margin: f32,
    pub bates: bool,
    pub prefix: String,
    pub suffix: String,
    pub digits: u32,
    pub start: u64,
    pub range: String,
}
impl Default for Hf {
    fn default() -> Self {
        Self {
            header: Default::default(),
            footer: ["".into(), "Page {page} of {pages}".into(), "".into()],
            size: 10.0,
            margin: 36.0,
            bates: false,
            prefix: String::new(),
            suffix: String::new(),
            digits: 6,
            start: 1,
            range: "all".into(),
        }
    }
}
pub fn header_footer(ctx: &Context, s: &mut Hf) -> Outcome {
    modal(ctx, "Headers, footers and Bates numbers", |ui| {
        ui.label("Tokens: {page}  {pages}  {date}  {bates}");
        egui::Grid::new("hf")
            .num_columns(4)
            .spacing([8.0, 6.0])
            .show(ui, |ui| {
                ui.label("");
                ui.label("Left");
                ui.label("Centre");
                ui.label("Right");
                ui.end_row();
                ui.label("Header");
                for t in s.header.iter_mut() {
                    ui.add(egui::TextEdit::singleline(t).desired_width(150.0));
                }
                ui.end_row();
                ui.label("Footer");
                for t in s.footer.iter_mut() {
                    ui.add(egui::TextEdit::singleline(t).desired_width(150.0));
                }
                ui.end_row();
            });
        ui.horizontal(|ui| {
            ui.label("Font size");
            ui.add(egui::DragValue::new(&mut s.size).range(5.0..=48.0));
            ui.label("Margin (pt)");
            ui.add(egui::DragValue::new(&mut s.margin).range(10.0..=144.0));
        });
        ui.checkbox(&mut s.bates, "Bates numbering");
        ui.add_enabled_ui(s.bates, |ui| {
            ui.horizontal(|ui| {
                ui.label("Prefix");
                ui.add(egui::TextEdit::singleline(&mut s.prefix).desired_width(80.0));
                ui.label("Suffix");
                ui.add(egui::TextEdit::singleline(&mut s.suffix).desired_width(80.0));
                ui.label("Digits");
                ui.add(egui::DragValue::new(&mut s.digits).range(1..=12));
                ui.label("Start");
                ui.add(egui::DragValue::new(&mut s.start).range(0..=999_999_999));
            });
            ui.label("Without a {bates} token the number is placed at the bottom right.");
        });
        range_field(ui, &mut s.range);
        buttons(ui, "Apply")
    })
}

// ---------------------------------------------------------------- metadata

pub struct Meta(pub Info);
pub fn metadata(ctx: &Context, s: &mut Meta) -> Outcome {
    modal(ctx, "Document properties", |ui| {
        egui::Grid::new("meta")
            .num_columns(2)
            .spacing([10.0, 6.0])
            .show(ui, |ui| {
                for (label, f) in [
                    ("Title", &mut s.0.title),
                    ("Author", &mut s.0.author),
                    ("Subject", &mut s.0.subject),
                    ("Keywords", &mut s.0.keywords),
                    ("Creator", &mut s.0.creator),
                    ("Producer", &mut s.0.producer),
                ] {
                    ui.label(label);
                    let mut v = f.clone().unwrap_or_default();
                    if ui
                        .add(egui::TextEdit::singleline(&mut v).desired_width(300.0))
                        .changed()
                    {
                        *f = Some(v);
                    }
                    ui.end_row();
                }
            });
        buttons(ui, "Apply")
    })
}

// ---------------------------------------------------------------- security

pub struct Sec {
    pub enabled: bool,
    pub user: String,
    pub owner: String,
    pub cipher: Cipher,
    pub r: Restrictions,
}
impl Default for Sec {
    fn default() -> Self {
        Self {
            enabled: true,
            user: String::new(),
            owner: String::new(),
            cipher: Cipher::Aes256,
            r: Restrictions::default(),
        }
    }
}
pub fn security(ctx: &Context, s: &mut Sec) -> Outcome {
    modal(ctx, "Password security", |ui| {
        ui.checkbox(
            &mut s.enabled,
            "Protect this document with passwords when saving",
        );
        ui.add_enabled_ui(s.enabled, |ui| {
            egui::Grid::new("sec")
                .num_columns(2)
                .spacing([10.0, 6.0])
                .show(ui, |ui| {
                    ui.label("Open password");
                    ui.add(egui::TextEdit::singleline(&mut s.user).password(true));
                    ui.end_row();
                    ui.label("Permissions password");
                    ui.add(egui::TextEdit::singleline(&mut s.owner).password(true));
                    ui.end_row();
                    ui.label("Encryption");
                    egui::ComboBox::from_id_salt("cipher")
                        .selected_text(match s.cipher {
                            Cipher::Rc4_40 => "40-bit RC4 (weak)",
                            Cipher::Rc4_128 => "128-bit RC4 (weak)",
                            Cipher::Aes128 => "128-bit AES",
                            Cipher::Aes256 => "256-bit AES",
                        })
                        .show_ui(ui, |ui| {
                            ui.selectable_value(&mut s.cipher, Cipher::Aes256, "256-bit AES");
                            ui.selectable_value(&mut s.cipher, Cipher::Aes128, "128-bit AES");
                            ui.selectable_value(
                                &mut s.cipher,
                                Cipher::Rc4_128,
                                "128-bit RC4 (weak)",
                            );
                            ui.selectable_value(&mut s.cipher, Cipher::Rc4_40, "40-bit RC4 (weak)");
                        });
                    ui.end_row();
                });
            ui.label("Allow:");
            ui.checkbox(&mut s.r.print, "Printing");
            ui.checkbox(&mut s.r.print_high_quality, "High-resolution printing");
            ui.checkbox(&mut s.r.modify, "Modifying");
            ui.checkbox(&mut s.r.copy, "Copying content");
            ui.checkbox(&mut s.r.annotate, "Commenting");
            ui.checkbox(&mut s.r.fill_forms, "Filling form fields");
            ui.checkbox(&mut s.r.assemble, "Page assembly");
            ui.label("Permission flags are advisory: they rely on the viewer honouring them.");
        });
        buttons(ui, "Apply")
    })
}

// ---------------------------------------------------------- simple prompts

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum RangeKind {
    Extract,
    Delete,
    SplitEvery,
}
pub struct Range {
    pub kind: RangeKind,
    pub text: String,
}
pub fn range(ctx: &Context, s: &mut Range) -> Outcome {
    let (title, label) = match s.kind {
        RangeKind::Extract => ("Extract pages", "Pages to extract (e.g. 1-3,7):"),
        RangeKind::Delete => ("Delete pages", "Pages to delete (e.g. 2,4-6):"),
        RangeKind::SplitEvery => ("Split document", "Pages per file:"),
    };
    modal(ctx, title, |ui| {
        ui.label(label);
        ui.text_edit_singleline(&mut s.text);
        buttons(ui, "OK")
    })
}

pub struct Blank {
    pub after: usize,
    pub w: f32,
    pub h: f32,
}
pub fn blank(ctx: &Context, s: &mut Blank, pages: usize) -> Outcome {
    modal(ctx, "Insert blank page", |ui| {
        ui.horizontal(|ui| {
            ui.label("After page (0 = first)");
            ui.add(egui::DragValue::new(&mut s.after).range(0..=pages));
        });
        ui.horizontal(|ui| {
            if ui.button("Letter").clicked() {
                (s.w, s.h) = (612.0, 792.0);
            }
            if ui.button("A4").clicked() {
                (s.w, s.h) = (595.0, 842.0);
            }
            if ui.button("Legal").clicked() {
                (s.w, s.h) = (612.0, 1008.0);
            }
            ui.add(
                egui::DragValue::new(&mut s.w)
                    .range(36.0..=14400.0)
                    .suffix(" pt"),
            );
            ui.label("×");
            ui.add(
                egui::DragValue::new(&mut s.h)
                    .range(36.0..=14400.0)
                    .suffix(" pt"),
            );
        });
        buttons(ui, "Insert")
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

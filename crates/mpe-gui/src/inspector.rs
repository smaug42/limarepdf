//! The inspector's page and document sections: the settings that used to be dialogs.
//!
//! With pages selected it shows page actions and the tools that work on a page range,
//! with the range filled in from the selection. With nothing selected it shows the
//! document: properties, security, the same tools set to all pages, export and split.
//! (A selected object or words, or a drawing tool in use, show the editor's own
//! section instead; see `Editor::owns_panel`.) The panel reports which button was
//! pressed as an [`Apply`]; the app carries it out.

use crate::actions::{self, Act, Ctx};
use crate::selection::Selection;
use crate::tab::Tab;
use crate::theme;
use eframe::egui::{self, Ui};
use mpe_core::render::{ImageFormat, MAX_DPI};
use mpe_core::{Cipher, Info, Restrictions, ScanMode, ScanOptions};
use std::collections::BTreeSet;

/// A section of the panel, for opening it from a menu command.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Section {
    Properties,
    Security,
    HeaderFooter,
    Watermark,
    Ocr,
    Scans,
    Export,
    Split,
}

impl Section {
    /// Shown only with nothing selected (the others also act on selected pages).
    pub fn document_only(self) -> bool {
        matches!(
            self,
            Section::Properties | Section::Security | Section::Export | Section::Split
        )
    }
}

/// A button pressed in the panel.
pub enum Apply {
    Act(Act),
    Props,
    Security,
    HeaderFooter,
    Watermark,
    Ocr,
    Scans,
    Export,
    Split,
    /// Insert a blank page after this page (0-based), at the form's size.
    Blank(usize),
}

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
            enabled: false,
            user: String::new(),
            owner: String::new(),
            cipher: Cipher::Aes256,
            r: Restrictions::default(),
        }
    }
}

pub struct Export {
    pub pages: String,
    pub jpeg: bool,
    pub dpi: f32,
    pub quality: u8,
}
impl Default for Export {
    fn default() -> Self {
        Self {
            pages: "all".into(),
            jpeg: false,
            dpi: 150.0,
            quality: 90,
        }
    }
}
impl Export {
    pub fn format(&self) -> ImageFormat {
        if self.jpeg {
            ImageFormat::Jpeg(self.quality)
        } else {
            ImageFormat::Png
        }
    }
}

/// The panel's form state, kept per tab.
#[derive(Default)]
pub struct Forms {
    pub meta: Info,
    pub sec: Sec,
    pub wm: Wm,
    pub hf: Hf,
    pub ocr: String,
    pub scan_pages: String,
    pub scan: ScanOptions,
    pub export: Export,
    /// Pages per file when splitting.
    pub split: usize,
    /// Size of a new blank page, in points.
    pub blank: (f32, f32),
    /// Section to open and scroll to on the next frame.
    pub reveal: Option<Section>,
    /// Document revision the properties were read at.
    meta_rev: Option<u64>,
    /// Selection the page ranges were filled in for.
    filled_for: Option<Selection>,
}

/// Page numbers as a range, consecutive runs collapsed: "1-3,7".
pub fn range_text(pages: &BTreeSet<usize>) -> String {
    let v: Vec<usize> = pages.iter().copied().collect();
    let mut out: Vec<String> = Vec::new();
    let mut i = 0;
    while i < v.len() {
        let mut j = i;
        while j + 1 < v.len() && v[j + 1] == v[j] + 1 {
            j += 1;
        }
        out.push(if i == j {
            format!("{}", v[i] + 1)
        } else {
            format!("{}-{}", v[i] + 1, v[j] + 1)
        });
        i = j + 1;
    }
    out.join(",")
}

impl Tab {
    /// Bring the forms up to date: properties after the document changed, page ranges
    /// and blank-page size after the selection changed.
    fn sync_forms(&mut self) {
        if self.forms.meta_rev != Some(self.revision) {
            if self.forms.meta_rev.is_none() {
                self.forms.sec = match &self.encryption {
                    Some(e) => Sec {
                        enabled: true,
                        user: e.user_password.clone(),
                        owner: e.owner_password.clone(),
                        cipher: e.cipher,
                        r: e.restrictions,
                    },
                    None => Sec::default(),
                };
                self.forms.split = 1;
            }
            self.forms.meta = self.pdf.info();
            self.forms.meta_rev = Some(self.revision);
        }
        let sel = self.ed.selection.clone();
        if self.forms.filled_for.as_ref() == Some(&sel) {
            return;
        }
        let range = sel.pages().map_or("all".into(), range_text);
        let f = &mut self.forms;
        for r in [
            &mut f.wm.range,
            &mut f.hf.range,
            &mut f.ocr,
            &mut f.scan_pages,
            &mut f.export.pages,
        ] {
            *r = range.clone();
        }
        let first = sel.pages().and_then(|s| s.first().copied());
        let page = first.unwrap_or(self.current);
        f.blank = self.sizes.get(page).copied().unwrap_or((612.0, 792.0));
        f.filled_for = Some(sel);
    }
}

/// Draw the page or document section for `t`. Returns the button pressed, if any.
pub fn show(ui: &mut Ui, t: &mut Tab, c: &Ctx) -> Option<Apply> {
    t.sync_forms();
    let mut apply = None;
    let reveal = t.forms.reveal.take();
    let pages = t.ed.selection.pages().cloned();
    let f = &mut t.forms;
    match &pages {
        Some(pages) => {
            let title = if pages.len() == 1 {
                format!("Page {}", range_text(pages))
            } else {
                format!("{} pages", pages.len())
            };
            ui.heading(title);
            if pages.len() > 1 {
                ui.weak(range_text(pages));
            }
            ui.add_space(4.0);
            for a in [
                Act::RotR,
                Act::RotL,
                Act::Delete,
                Act::Extract,
                Act::InsertImages,
            ] {
                if act_button(ui, a, c) {
                    apply = Some(Apply::Act(a));
                }
            }
            let last = pages.last().copied().unwrap_or(0);
            ui.add_space(4.0);
            section(ui, reveal, None, "Insert blank page", |ui| {
                if blank_ui(ui, &mut f.blank, last) {
                    apply = Some(Apply::Blank(last));
                }
            });
        }
        None => {
            ui.heading("Document");
            ui.weak("Select pages in the thumbnails, or pick a tool, for more options.");
            ui.add_space(4.0);
            section(ui, reveal, Some(Section::Properties), "Properties", |ui| {
                if meta_ui(ui, &mut f.meta) {
                    apply = Some(Apply::Props);
                }
            });
            let sec = "Password security";
            section(ui, reveal, Some(Section::Security), sec, |ui| {
                if security_ui(ui, &mut f.sec) {
                    apply = Some(Apply::Security);
                }
            });
        }
    }
    // Tools that work on a page range: the selected pages, or all.
    section(ui, reveal, Some(Section::Watermark), "Watermark", |ui| {
        if watermark_ui(ui, &mut f.wm) {
            apply = Some(Apply::Watermark);
        }
    });
    let hf_title = "Headers, footers, Bates";
    section(ui, reveal, Some(Section::HeaderFooter), hf_title, |ui| {
        if header_footer_ui(ui, &mut f.hf) {
            apply = Some(Apply::HeaderFooter);
        }
    });
    let ocr = "Recognise text (OCR)";
    section(ui, reveal, Some(Section::Ocr), ocr, |ui| {
        ui.weak("Pages without text; Latin script. Runs in the background.");
        range_row(ui, &mut f.ocr);
        let run = egui::Button::new("Recognise");
        if ui.add_enabled(!c.busy, run).clicked() {
            apply = Some(Apply::Ocr);
        }
    });
    let scans = "Optimise scanned pages";
    section(ui, reveal, Some(Section::Scans), scans, |ui| {
        if scan_ui(ui, &mut f.scan_pages, &mut f.scan, c.busy) {
            apply = Some(Apply::Scans);
        }
    });
    if pages.is_none() {
        let export = "Export pages as images";
        section(ui, reveal, Some(Section::Export), export, |ui| {
            if export_ui(ui, &mut f.export, c.busy) {
                apply = Some(Apply::Export);
            }
        });
        section(ui, reveal, Some(Section::Split), "Split into files", |ui| {
            ui.horizontal(|ui| {
                ui.label("Pages per file");
                ui.add(egui::DragValue::new(&mut f.split).range(1..=100_000));
            });
            if ui.button("Choose folder…").clicked() {
                apply = Some(Apply::Split);
            }
        });
        ui.add_space(8.0);
        for a in [Act::ExportText, Act::Print, Act::Merge] {
            if act_button(ui, a, c) {
                apply = Some(Apply::Act(a));
            }
        }
    }
    apply
}

/// A collapsible section; opened and scrolled into view when it is `reveal`.
fn section(
    ui: &mut Ui,
    reveal: Option<Section>,
    this: Option<Section>,
    title: &str,
    body: impl FnOnce(&mut Ui),
) {
    let open = this.is_some() && reveal == this;
    let r = egui::CollapsingHeader::new(title)
        .id_salt(("inspector", title))
        .open(open.then_some(true))
        .show(ui, body);
    if open {
        r.header_response.scroll_to_me(Some(egui::Align::TOP));
    }
}

/// A full-width button for a registry command: icon and label, disabled when the
/// command is not available.
fn act_button(ui: &mut Ui, act: Act, c: &Ctx) -> bool {
    let a = actions::get(act);
    let glyph = theme::icon(if a.icon.is_empty() { " " } else { a.icon }).size(16.0);
    let b = egui::Button::new((glyph, a.label))
        .min_size(egui::vec2(ui.available_width(), 26.0))
        .frame_when_inactive(false);
    ui.add_enabled((a.enabled)(c), b).clicked()
}

fn range_row(ui: &mut Ui, text: &mut String) {
    ui.horizontal(|ui| {
        ui.label("Pages");
        ui.add(egui::TextEdit::singleline(text).desired_width(f32::INFINITY))
            .on_hover_text("e.g. 1-3,5,8-  or  all");
    });
}

/// A two-column grid of labels and controls, sized for the panel.
fn grid(ui: &mut Ui, id: &str, body: impl FnOnce(&mut Ui)) {
    egui::Grid::new(id)
        .num_columns(2)
        .spacing([8.0, 6.0])
        .show(ui, body);
}

/// A page dimension in points.
fn points(v: &mut f32) -> egui::DragValue<'_> {
    egui::DragValue::new(v).range(36.0..=14400.0).suffix(" pt")
}

fn blank_ui(ui: &mut Ui, size: &mut (f32, f32), after: usize) -> bool {
    ui.horizontal(|ui| {
        for (name, w, h) in [
            ("Letter", 612.0, 792.0),
            ("A4", 595.0, 842.0),
            ("Legal", 612.0, 1008.0),
        ] {
            if ui.small_button(name).clicked() {
                *size = (w, h);
            }
        }
    });
    ui.horizontal(|ui| {
        ui.add(points(&mut size.0));
        ui.label("×");
        ui.add(points(&mut size.1));
    });
    ui.button(format!("Insert after page {}", after + 1))
        .clicked()
}

fn meta_ui(ui: &mut Ui, s: &mut Info) -> bool {
    grid(ui, "meta", |ui| {
        for (label, f) in [
            ("Title", &mut s.title),
            ("Author", &mut s.author),
            ("Subject", &mut s.subject),
            ("Keywords", &mut s.keywords),
            ("Creator", &mut s.creator),
            ("Producer", &mut s.producer),
        ] {
            ui.label(label);
            let mut v = f.clone().unwrap_or_default();
            let field = egui::TextEdit::singleline(&mut v).desired_width(f32::INFINITY);
            if ui.add(field).changed() {
                *f = Some(v);
            }
            ui.end_row();
        }
    });
    ui.button("Apply").clicked()
}

fn security_ui(ui: &mut Ui, s: &mut Sec) -> bool {
    ui.checkbox(&mut s.enabled, "Protect with passwords when saving");
    ui.add_enabled_ui(s.enabled, |ui| {
        grid(ui, "sec", |ui| {
            ui.label("Open");
            ui.add(egui::TextEdit::singleline(&mut s.user).password(true))
                .on_hover_text("Needed to open the document");
            ui.end_row();
            ui.label("Permissions");
            ui.add(egui::TextEdit::singleline(&mut s.owner).password(true))
                .on_hover_text("Needed to change the permissions");
            ui.end_row();
            ui.label("Encryption");
            let name = |c: Cipher| match c {
                Cipher::Rc4_40 => "40-bit RC4 (weak)",
                Cipher::Rc4_128 => "128-bit RC4 (weak)",
                Cipher::Aes128 => "128-bit AES",
                Cipher::Aes256 => "256-bit AES",
            };
            egui::ComboBox::from_id_salt("cipher")
                .selected_text(name(s.cipher))
                .show_ui(ui, |ui| {
                    for c in [
                        Cipher::Aes256,
                        Cipher::Aes128,
                        Cipher::Rc4_128,
                        Cipher::Rc4_40,
                    ] {
                        ui.selectable_value(&mut s.cipher, c, name(c));
                    }
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
        ui.weak("Permission flags are advisory: they rely on the viewer honouring them.");
    });
    ui.weak("Applies when you save.");
    ui.button("Apply").clicked()
}

fn watermark_ui(ui: &mut Ui, s: &mut Wm) -> bool {
    grid(ui, "wm", |ui| {
        ui.label("Text");
        ui.add(egui::TextEdit::singleline(&mut s.text).desired_width(f32::INFINITY));
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
    });
    ui.radio_value(&mut s.foreground, true, "Above the content");
    ui.radio_value(&mut s.foreground, false, "Behind the content");
    range_row(ui, &mut s.range);
    ui.button("Apply").clicked()
}

fn header_footer_ui(ui: &mut Ui, s: &mut Hf) -> bool {
    ui.weak("Tokens: {page}  {pages}  {date}  {bates}");
    grid(ui, "hf", |ui| {
        let sides = ["left", "centre", "right"];
        for (name, row) in [("Header", &mut s.header), ("Footer", &mut s.footer)] {
            for (side, text) in sides.iter().zip(row.iter_mut()) {
                ui.label(format!("{name}, {side}"));
                ui.add(egui::TextEdit::singleline(text).desired_width(f32::INFINITY));
                ui.end_row();
            }
        }
        ui.label("Font size");
        ui.add(egui::DragValue::new(&mut s.size).range(5.0..=48.0));
        ui.end_row();
        ui.label("Margin");
        ui.add(
            egui::DragValue::new(&mut s.margin)
                .range(10.0..=144.0)
                .suffix(" pt"),
        );
        ui.end_row();
    });
    ui.checkbox(&mut s.bates, "Bates numbering");
    ui.add_enabled_ui(s.bates, |ui| {
        grid(ui, "bates", |ui| {
            ui.label("Prefix");
            ui.add(egui::TextEdit::singleline(&mut s.prefix).desired_width(f32::INFINITY));
            ui.end_row();
            ui.label("Suffix");
            ui.add(egui::TextEdit::singleline(&mut s.suffix).desired_width(f32::INFINITY));
            ui.end_row();
            ui.label("Digits");
            ui.add(egui::DragValue::new(&mut s.digits).range(1..=12));
            ui.end_row();
            ui.label("Start");
            ui.add(egui::DragValue::new(&mut s.start).range(0..=999_999_999));
            ui.end_row();
        });
        ui.weak("Without a {bates} token the number goes at the bottom right.");
    });
    range_row(ui, &mut s.range);
    ui.button("Apply").clicked()
}

fn scan_ui(ui: &mut Ui, pages: &mut String, o: &mut ScanOptions, busy: bool) -> bool {
    ui.weak("Pages that are one image covering the page. Runs in the background.");
    range_row(ui, pages);
    grid(ui, "scan", |ui| {
        ui.label("Store as");
        let label = |m: ScanMode| match m {
            ScanMode::Auto => "Automatic",
            ScanMode::BlackWhite => "Black and white",
            ScanMode::Gray => "Greyscale",
            ScanMode::Color => "Colour",
        };
        egui::ComboBox::from_id_salt("scan-mode")
            .selected_text(label(o.mode))
            .show_ui(ui, |ui| {
                for m in [
                    ScanMode::Auto,
                    ScanMode::BlackWhite,
                    ScanMode::Gray,
                    ScanMode::Color,
                ] {
                    ui.selectable_value(&mut o.mode, m, label(m));
                }
            });
        ui.end_row();
        ui.label("Image resolution")
            .on_hover_text("Greyscale and colour images above this are downsampled");
        ui.add(
            egui::DragValue::new(&mut o.max_dpi)
                .range(72.0..=600.0)
                .suffix(" dpi"),
        );
        ui.end_row();
        ui.label("JPEG quality");
        ui.add(egui::Slider::new(&mut o.jpeg_quality, 10..=100));
        ui.end_row();
    });
    ui.checkbox(&mut o.deskew, "Straighten tilted pages");
    ui.checkbox(&mut o.despeckle, "Remove specks (black and white)");
    ui.add_enabled(!busy, egui::Button::new("Optimise"))
        .clicked()
}

fn export_ui(ui: &mut Ui, s: &mut Export, busy: bool) -> bool {
    ui.weak("One file per page, named after the document and the page number.");
    range_row(ui, &mut s.pages);
    grid(ui, "export", |ui| {
        ui.label("Format");
        ui.horizontal(|ui| {
            ui.radio_value(&mut s.jpeg, false, "PNG");
            ui.radio_value(&mut s.jpeg, true, "JPEG");
        });
        ui.end_row();
        ui.label("Resolution");
        ui.add(
            egui::DragValue::new(&mut s.dpi)
                .range(36.0..=MAX_DPI)
                .suffix(" dpi"),
        );
        ui.end_row();
        ui.label("JPEG quality");
        ui.add_enabled(s.jpeg, egui::Slider::new(&mut s.quality, 10..=100));
        ui.end_row();
    });
    ui.add_enabled(!busy, egui::Button::new("Choose folder…"))
        .clicked()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ranges_collapse_runs() {
        let set: BTreeSet<usize> = [0, 1, 2, 4, 6, 7].into();
        assert_eq!(range_text(&set), "1-3,5,7-8");
        assert_eq!(range_text(&[3].into()), "4");
    }
}

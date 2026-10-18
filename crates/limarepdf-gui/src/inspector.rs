//! The inspector's page and document sections: the settings that used to be dialogs.
//!
//! With pages selected it shows page actions and the tools that work on a page range,
//! with the range filled in from the selection. With nothing selected it shows the
//! document: properties, security, the same tools set to all pages, export and split.
//! (A selected object or words, or a drawing tool in use, show the editor's own
//! section instead; see `Editor::owns_panel`.) The panel reports which button was
//! pressed as an [`Apply`]; the app carries it out.

use crate::actions::{self, Act, Ctx};
use crate::app::IMAGE_EXTS;
use crate::fields::{self, FormUi};
use crate::selection::Selection;
use crate::tab::{Side, Stamp, Tab};
use crate::theme;
use eframe::egui::{self, Ui};
use limarepdf_core::render::{ImageFormat, MAX_DPI};
use limarepdf_core::{
    Cipher, Encryption, Info, LabelStyle, OpenLayout, OpenPanel, Restrictions, ScanMode,
    ScanOptions, ViewPrefs,
};
use std::collections::BTreeSet;
use std::path::PathBuf;

/// A section of the panel, for opening it from a menu command.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Section {
    Properties,
    Security,
    Crop,
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
    Crop,
    /// `replace`: update the headers and footers just added instead of adding more.
    HeaderFooter {
        replace: bool,
    },
    /// `replace`: update the watermark just added instead of adding another.
    Watermark {
        replace: bool,
    },
    Ocr,
    Scans,
    Export,
    Split,
    /// Insert a blank page after this page (0-based), at the form's size.
    Blank(usize),
    /// Number the pages from this page (0-based) on, as the form says.
    Label(usize),
    RemoveLabels,
    ViewPrefs,
}

pub struct Wm {
    /// A picture instead of text.
    pub picture: bool,
    pub text: String,
    pub size: f32,
    pub image: Option<PathBuf>,
    /// Picture size, percent of the page.
    pub scale: f32,
    pub opacity: f32,
    pub rotation: f32,
    pub color: [f32; 3],
    pub foreground: bool,
    pub range: String,
}
impl Default for Wm {
    fn default() -> Self {
        Self {
            picture: false,
            text: "DRAFT".into(),
            size: 96.0,
            image: None,
            scale: 50.0,
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
impl Sec {
    /// The form filled in from the document's password settings.
    pub fn of(e: Option<&Encryption>) -> Self {
        match e {
            Some(e) => Sec {
                enabled: true,
                user: e.user_password.clone(),
                owner: e.owner_password.clone(),
                cipher: e.cipher,
                r: e.restrictions,
            },
            None => Sec::default(),
        }
    }

    /// The password settings the form describes. An empty owner password is the open
    /// password.
    pub fn encryption(&self) -> Option<Encryption> {
        self.enabled.then(|| Encryption {
            user_password: self.user.clone(),
            owner_password: if self.owner.is_empty() {
                self.user.clone()
            } else {
                self.owner.clone()
            },
            cipher: self.cipher,
            restrictions: self.r,
        })
    }
}

#[derive(Default)]
pub struct Crop {
    pub pages: String,
    /// Points trimmed from the full page: top, right, bottom, left as displayed.
    pub margins: [f32; 4],
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

/// Page numbering to set from the first selected page.
pub struct Label {
    /// `None`: the prefix alone.
    pub style: Option<LabelStyle>,
    pub prefix: String,
    pub start: u32,
}
impl Default for Label {
    fn default() -> Self {
        Self {
            style: Some(LabelStyle::Decimal),
            prefix: String::new(),
            start: 1,
        }
    }
}

/// The panel's form state, kept per tab.
#[derive(Default)]
pub struct Forms {
    pub meta: Info,
    pub sec: Sec,
    pub crop: Crop,
    pub wm: Wm,
    pub hf: Hf,
    pub ocr: String,
    pub scan_pages: String,
    pub scan: ScanOptions,
    pub export: Export,
    /// Pages per file when splitting.
    pub split: usize,
    /// Split at bookmarks instead, down to this level (1 = top level only).
    pub split_at_marks: bool,
    pub split_level: usize,
    /// Size of a new blank page, in points.
    pub blank: (f32, f32),
    pub label: Label,
    /// The viewer preferences being edited, and the page shown in the page box (0-based)
    /// while no opening page is set.
    pub view: ViewPrefs,
    pub view_page: usize,
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
    /// and blank-page size after the selection changed, and the crop margins after
    /// either.
    fn sync_forms(&mut self) {
        let changed = self.forms.meta_rev != Some(self.revision);
        if changed {
            if self.forms.meta_rev.is_none() {
                self.forms.sec = Sec::of(self.encryption.as_ref());
                self.forms.split = 1;
                self.forms.split_level = 1;
            }
            self.forms.meta = self.pdf.info();
            self.forms.view = self.pdf.view_prefs();
            self.forms.view_page = self.forms.view.page.unwrap_or(0);
            self.forms.meta_rev = Some(self.revision);
        }
        let sel = self.ed.selection.clone();
        let first = sel.pages().and_then(|s| s.first().copied());
        if changed || self.forms.filled_for.as_ref() != Some(&sel) {
            let margins = self.pdf.crop_margins(first.unwrap_or(0));
            self.forms.crop.margins = margins.unwrap_or_default();
        }
        if self.forms.filled_for.as_ref() == Some(&sel) {
            return;
        }
        let range = sel.pages().map_or("all".into(), range_text);
        let f = &mut self.forms;
        for r in [
            &mut f.crop.pages,
            &mut f.wm.range,
            &mut f.hf.range,
            &mut f.ocr,
            &mut f.scan_pages,
            &mut f.export.pages,
        ] {
            *r = range.clone();
        }
        let page = first.unwrap_or(self.current);
        f.blank = self.sizes.get(page).copied().unwrap_or((612.0, 792.0));
        f.filled_for = Some(sel);
    }
}

/// The panel's heading, drawn by the app beside the button that hides the panel: "Edit"
/// for the editor's section, else the selected pages or "Document".
pub fn title(t: &Tab) -> String {
    if t.ed.owns_panel() {
        return "Edit".into();
    }
    match t.ed.selection.pages() {
        Some(p) if p.len() == 1 => format!("Page {}", range_text(p)),
        Some(p) => format!("{} pages", p.len()),
        None => "Document".into(),
    }
}

/// Draw the page or document section for `t`. Returns the button pressed, if any.
pub fn show(ui: &mut Ui, t: &mut Tab, c: &Ctx) -> Option<Apply> {
    t.sync_forms();
    let mut apply = None;
    let reveal = t.forms.reveal.take();
    let pages = t.ed.selection.pages().cloned();
    let wm_again = t.replaces_stamp(Stamp::Watermark);
    let hf_again = t.replaces_stamp(Stamp::HeaderFooter);
    let here = Here {
        layout: t.layout.to_open(t.cover),
        panel: match (c.side, t.side) {
            (true, Side::Pages) => Some(OpenPanel::Pages),
            (true, Side::Bookmarks) => Some(OpenPanel::Bookmarks),
            _ => None,
        },
        page: t.current,
        pages: t.page_count(),
    };
    let f = &mut t.forms;
    match &pages {
        Some(pages) => {
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
            let first = pages.first().copied().unwrap_or(0);
            let labelled = t.labels.is_some();
            section(ui, reveal, None, "Page labels", |ui| {
                if let Some(a) = label_ui(ui, &mut f.label, first, labelled) {
                    apply = Some(a);
                }
            });
        }
        None => {
            ui.weak("Select pages in the thumbnails, or pick a tool, for more options.");
            ui.add_space(4.0);
            section(ui, reveal, Some(Section::Properties), "Properties", |ui| {
                if meta_ui(ui, &mut f.meta) {
                    apply = Some(Apply::Props);
                }
            });
            section(ui, reveal, None, "Opening view", |ui| {
                if view_ui(ui, &mut f.view, &mut f.view_page, &here) {
                    apply = Some(Apply::ViewPrefs);
                }
            });
            let sec = "Password security";
            section(ui, reveal, Some(Section::Security), sec, |ui| {
                if security_ui(ui, &mut f.sec) {
                    apply = Some(Apply::Security);
                }
            });
            if c.form {
                section(ui, reveal, None, "Form", |ui| {
                    if let Some(a) = form_ui(ui, &t.ed.form, c) {
                        apply = Some(Apply::Act(a));
                    }
                });
            }
        }
    }
    // Tools that work on a page range: the selected pages, or all.
    section(ui, reveal, Some(Section::Crop), "Crop", |ui| {
        if crop_ui(ui, &mut f.crop) {
            apply = Some(Apply::Crop);
        }
    });
    section(ui, reveal, Some(Section::Watermark), "Watermark", |ui| {
        if let Some(replace) = watermark_ui(ui, &mut f.wm, wm_again) {
            apply = Some(Apply::Watermark { replace });
        }
    });
    let hf_title = "Headers, footers, Bates";
    section(ui, reveal, Some(Section::HeaderFooter), hf_title, |ui| {
        if let Some(replace) = header_footer_ui(ui, &mut f.hf, hf_again) {
            apply = Some(Apply::HeaderFooter { replace });
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
                ui.radio_value(&mut f.split_at_marks, false, "Pages per file");
                ui.add_enabled(
                    !f.split_at_marks,
                    egui::DragValue::new(&mut f.split).range(1..=100_000),
                );
            });
            ui.horizontal(|ui| {
                ui.radio_value(&mut f.split_at_marks, true, "At bookmarks, levels");
                ui.add_enabled(
                    f.split_at_marks,
                    egui::DragValue::new(&mut f.split_level).range(1..=9),
                );
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

/// The form: the field with the keyboard, if any, and commands for the whole form.
fn form_ui(ui: &mut Ui, form: &FormUi, c: &Ctx) -> Option<Act> {
    let mut act = None;
    let n = form.fields().len();
    ui.weak(format!("{n} field{}", if n == 1 { "" } else { "s" }));
    match form.focused_field() {
        Some(f) => {
            ui.strong(&f.name);
            if let Some(tip) = &f.tooltip {
                ui.label(tip);
            }
            let mut flags = Vec::new();
            if f.required {
                flags.push("Required");
            }
            if let Some(why) = fields::why_not(f) {
                flags.push(why);
            }
            if !flags.is_empty() {
                ui.weak(flags.join(" · "));
            }
            if act_button(ui, Act::ClearField, c) {
                act = Some(Act::ClearField);
            }
        }
        None => {
            ui.weak("Click a field to fill it in, or press Tab.");
        }
    }
    let mut shown = c.fields_shown;
    if ui.checkbox(&mut shown, "Highlight fields").changed() {
        act = Some(Act::ToggleFields);
    }
    for a in [Act::ResetForm, Act::FlattenForm] {
        if act_button(ui, a, c) {
            act = Some(a);
        }
    }
    act
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

/// The numbering to set from page `first` on, until the next numbering the document has after it.
fn label_ui(ui: &mut Ui, s: &mut Label, first: usize, labelled: bool) -> Option<Apply> {
    let name = |st: Option<LabelStyle>| st.map_or("Prefix only", LabelStyle::name);
    ui.weak("The page numbers viewers show, such as i, ii, 1, 2. Nothing is printed on the pages.");
    ui.horizontal(|ui| {
        ui.label("Style");
        egui::ComboBox::from_id_salt("label-style")
            .selected_text(name(s.style))
            .show_ui(ui, |ui| {
                for st in LabelStyle::ALL.map(Some).into_iter().chain([None]) {
                    ui.selectable_value(&mut s.style, st, name(st));
                }
            });
    });
    ui.horizontal(|ui| {
        ui.label("Prefix");
        ui.add(egui::TextEdit::singleline(&mut s.prefix).desired_width(80.0));
        ui.label("Start at");
        ui.add_enabled(
            s.style.is_some(),
            egui::DragValue::new(&mut s.start).range(1..=99_999),
        );
    });
    let preview = match s.style {
        Some(st) => format!("{}{}", s.prefix, st.format(s.start)),
        None => s.prefix.clone(),
    };
    ui.weak(format!("Page {} will be labelled “{preview}”.", first + 1));
    if ui
        .button(format!("Number from page {}", first + 1))
        .clicked()
    {
        return Some(Apply::Label(first));
    }
    if labelled && ui.button("Remove all page labels").clicked() {
        return Some(Apply::RemoveLabels);
    }
    None
}

/// Margins to trim, and the buttons to crop or show the whole page. Removing the crop
/// sets the margins to zero.
fn crop_ui(ui: &mut Ui, s: &mut Crop) -> bool {
    ui.weak("Trimmed from each side of the full page, as shown.");
    range_row(ui, &mut s.pages);
    grid(ui, "crop", |ui| {
        for (name, i) in [("Top", 0), ("Bottom", 2), ("Left", 3), ("Right", 1)] {
            ui.label(name);
            let v = &mut s.margins[i];
            ui.add(egui::DragValue::new(v).range(0.0..=14400.0).suffix(" pt"));
            ui.end_row();
        }
    });
    let mut apply = false;
    ui.horizontal(|ui| {
        apply = ui.button("Crop").clicked();
        if ui.button("Remove crop").clicked() {
            s.margins = [0.0; 4];
            apply = true;
        }
    });
    apply
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

/// How the document is being viewed now, for "Use current view".
struct Here {
    layout: OpenLayout,
    /// The side tab shown, when the side panel is open and on a tab a document can ask for.
    panel: Option<OpenPanel>,
    page: usize,
    pages: usize,
}

fn view_ui(ui: &mut Ui, v: &mut ViewPrefs, page: &mut usize, here: &Here) -> bool {
    ui.weak("How viewers show this document when it opens, if they follow it. LimarePDF does.");
    let layout_name = |l: Option<OpenLayout>| match l {
        None => "Not set",
        Some(OpenLayout::Continuous) => "Continuous pages",
        Some(OpenLayout::Facing) => "Facing pages",
        Some(OpenLayout::FacingCover) => "Facing, cover alone",
        Some(OpenLayout::Single) => "Single page",
    };
    let panel_name = |p: Option<OpenPanel>| match p {
        None => "Not set",
        Some(OpenPanel::Pages) => "Pages",
        Some(OpenPanel::Bookmarks) => "Bookmarks",
    };
    grid(ui, "view", |ui| {
        ui.label("Layout");
        egui::ComboBox::from_id_salt("view-layout")
            .selected_text(layout_name(v.layout))
            .show_ui(ui, |ui| {
                for l in [None].into_iter().chain(OpenLayout::ALL.map(Some)) {
                    ui.selectable_value(&mut v.layout, l, layout_name(l));
                }
            });
        ui.end_row();
        ui.label("Side panel");
        egui::ComboBox::from_id_salt("view-panel")
            .selected_text(panel_name(v.panel))
            .show_ui(ui, |ui| {
                for p in [None].into_iter().chain(OpenPanel::ALL.map(Some)) {
                    ui.selectable_value(&mut v.panel, p, panel_name(p));
                }
            });
        ui.end_row();
        ui.label("Page");
        ui.horizontal(|ui| {
            let mut set = v.page.is_some();
            ui.checkbox(&mut set, "Open at");
            let mut n = *page + 1;
            let field = egui::DragValue::new(&mut n).range(1..=here.pages.max(1));
            ui.add_enabled(set, field);
            *page = n - 1;
            v.page = set.then_some(*page);
        });
        ui.end_row();
    });
    ui.horizontal(|ui| {
        if ui
            .button("Use current view")
            .on_hover_text(
                "Fill in this document's layout, side panel and page as you see them now.",
            )
            .clicked()
        {
            *page = here.page;
            *v = ViewPrefs {
                layout: Some(here.layout),
                panel: here.panel,
                page: Some(here.page),
            };
        }
        ui.button("Apply").clicked()
    })
    .inner
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

/// The buttons of a stamp section. `again`: the stamp was just added, so it can be updated or
/// another added. Returns whether to replace it, when a button was pressed.
fn stamp_apply(ui: &mut Ui, again: bool, what: &str) -> Option<bool> {
    if !again {
        return ui.button("Apply").clicked().then_some(false);
    }
    ui.weak(format!("Update changes the {what} just added."));
    ui.horizontal(|ui| {
        if ui.button("Update").clicked() {
            Some(true)
        } else if ui.button("Add another").clicked() {
            Some(false)
        } else {
            None
        }
    })
    .inner
}

fn watermark_ui(ui: &mut Ui, s: &mut Wm, again: bool) -> Option<bool> {
    ui.horizontal(|ui| {
        ui.radio_value(&mut s.picture, false, "Text");
        ui.radio_value(&mut s.picture, true, "Picture");
    });
    grid(ui, "wm", |ui| {
        if s.picture {
            ui.label("Picture");
            ui.horizontal(|ui| {
                if ui.button("Choose…").clicked() {
                    if let Some(p) = rfd::FileDialog::new()
                        .add_filter("Images (PNG, JPEG, TIFF)", &IMAGE_EXTS)
                        .pick_file()
                    {
                        s.image = Some(p);
                    }
                }
                match s.image.as_ref().and_then(|p| p.file_name()) {
                    Some(n) => ui.label(n.to_string_lossy()),
                    None => ui.weak("none chosen"),
                };
            });
            ui.end_row();
            ui.label("Size");
            ui.add(egui::Slider::new(&mut s.scale, 5.0..=100.0).suffix("% of page"));
            ui.end_row();
        } else {
            ui.label("Text");
            ui.add(egui::TextEdit::singleline(&mut s.text).desired_width(f32::INFINITY));
            ui.end_row();
            ui.label("Size");
            ui.add(egui::Slider::new(&mut s.size, 12.0..=300.0));
            ui.end_row();
        }
        ui.label("Opacity");
        ui.add(egui::Slider::new(&mut s.opacity, 0.02..=1.0));
        ui.end_row();
        ui.label("Rotation");
        ui.add(egui::Slider::new(&mut s.rotation, -90.0..=90.0).suffix("°"));
        ui.end_row();
        if !s.picture {
            ui.label("Colour");
            ui.color_edit_button_rgb(&mut s.color);
            ui.end_row();
        }
    });
    ui.radio_value(&mut s.foreground, true, "Above the content");
    ui.radio_value(&mut s.foreground, false, "Behind the content");
    range_row(ui, &mut s.range);
    stamp_apply(ui, again, "watermark")
}

fn header_footer_ui(ui: &mut Ui, s: &mut Hf, again: bool) -> Option<bool> {
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
    stamp_apply(ui, again, "headers and footers")
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

    #[test]
    fn security_form_round_trips() {
        assert!(Sec::default().encryption().is_none());
        let form = Sec {
            enabled: true,
            user: "open".into(),
            owner: String::new(),
            cipher: Cipher::Aes128,
            r: Restrictions::default(),
        };
        let e = form.encryption().unwrap();
        // An empty owner password is the open password.
        assert_eq!(e.owner_password, "open");
        let back = Sec::of(Some(&e));
        assert!(back.enabled);
        assert_eq!((back.user.as_str(), back.cipher), ("open", Cipher::Aes128));
        assert!(!Sec::of(None).enabled);
    }
}

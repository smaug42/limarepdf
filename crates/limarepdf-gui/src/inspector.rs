//! The inspector's page and document sections: the settings that used to be dialogs.
//!
//! With pages selected it shows page actions and the tools that work on a page range,
//! with the range filled in from the selection. With nothing selected it shows the
//! document: properties, security, the same tools set to all pages, export and split.
//! (A selected object or words, or a drawing tool in use, show the editor's own
//! section instead; see `Editor::owns_panel`.) The panel reports which button was
//! pressed, or which setting changed, as an [`Apply`]; the app carries it out.
//!
//! Settings apply as they are changed, as in a word processor, one undo step each: text
//! when Enter is pressed or the field is left, choices at once, numbers once let go (a
//! number nudged again soon joins the same step). Watermarks and headers and footers are
//! shown on the pages until Add to pages, then each change updates the one added.

use crate::actions::{self, Act, Ctx};
use crate::app::IMAGE_EXTS;
use crate::dialogs::{set_name, slider};
use crate::fields::{self, FormUi};
use crate::selection::Selection;
use crate::tab::{Side, Stamp, StampForm, Tab};
use crate::theme;
use eframe::egui::{self, Ui};
use limarepdf_core::render::{ImageFormat, MAX_DPI};
use limarepdf_core::{
    parse_page_range, Bates, Cipher, Encryption, HeaderFooter, Info, LabelRange, LabelStyle,
    OpenLayout, OpenPanel, Restrictions, ScanMode, ScanOptions, ViewPrefs, Watermark,
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
    /// Page labels, shown with selected pages.
    Labels,
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

/// A button pressed in the panel, or a setting changed. `Option<&'static str>`: the
/// setting's name when changing it again soon joins the same undo step (see
/// [`Tab::edit_setting`]).
pub enum Apply {
    Act(Act),
    Props,
    Security,
    Crop(Option<&'static str>),
    /// `update`: put the form's settings in place of the stamp just added (see
    /// [`Tab::linked`]) instead of adding one.
    Stamp {
        kind: Stamp,
        update: bool,
    },
    Ocr,
    Scans,
    Export,
    Split,
    /// Insert a blank page after this page (0-based), at the form's size.
    Blank(usize),
    /// Number the pages from the form's page on, as it says.
    Label(Option<&'static str>),
    RemoveLabels,
    ViewPrefs(Option<&'static str>),
}

impl Apply {
    /// The button's name for messages, as its section is titled.
    pub fn name(&self) -> &'static str {
        match self {
            Apply::Act(a) => crate::actions::name(*a),
            Apply::Props => "Properties",
            Apply::Security => "Password security",
            Apply::Crop(_) => "Crop",
            Apply::Stamp {
                kind: Stamp::HeaderFooter,
                ..
            } => "Headers, footers, Bates",
            Apply::Stamp {
                kind: Stamp::Watermark,
                ..
            } => "Watermark",
            Apply::Ocr => "Recognise text (OCR)",
            Apply::Scans => "Optimise scanned pages",
            Apply::Export => "Export pages as images",
            Apply::Split => "Split into files",
            Apply::Blank(_) => "Insert blank page",
            Apply::Label(..) | Apply::RemoveLabels => "Page labels",
            Apply::ViewPrefs(_) => "Opening view",
        }
    }
}

#[derive(Clone, PartialEq)]
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
impl Wm {
    /// The text watermark the form describes.
    pub fn watermark(&self) -> Watermark {
        Watermark {
            text: self.text.clone(),
            font_size: self.size,
            opacity: self.opacity,
            rotation: self.rotation,
            color: (self.color[0], self.color[1], self.color[2]),
            foreground: self.foreground,
        }
    }

    /// Whether there is something to add: some text, or a picture chosen.
    pub fn ready(&self) -> bool {
        if self.picture {
            self.image.is_some()
        } else {
            !self.text.trim().is_empty()
        }
    }
}

#[derive(Clone, PartialEq)]
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
impl Hf {
    /// The headers and footers the form describes.
    pub fn header_footer(&self) -> HeaderFooter {
        HeaderFooter {
            header: self.header.clone(),
            footer: self.footer.clone(),
            font_size: self.size,
            margin: self.margin,
            color: (0.0, 0.0, 0.0),
            bates: self.bates.then(|| Bates {
                prefix: self.prefix.clone(),
                suffix: self.suffix.clone(),
                digits: self.digits as usize,
                start: self.start,
            }),
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
    /// A margin is being dragged in the form: the pages show the margins as they are now.
    pub dragged: bool,
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

/// Page numbering from the first selected page on: as the document has it, until changed.
#[derive(Clone, PartialEq, Debug)]
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
impl Label {
    /// The numbering in effect at `page` (0-based): that of the range it is in, carried on
    /// to it. Pages before the first range, or in a document without labels, are numbered
    /// plainly from 1.
    pub fn at(ranges: &[LabelRange], page: usize) -> Label {
        match ranges.iter().rev().find(|r| r.page <= page) {
            Some(r) => Label {
                style: r.style,
                prefix: r.prefix.clone(),
                start: r
                    .first
                    .saturating_add(u32::try_from(page - r.page).unwrap_or(u32::MAX)),
            },
            None => Label {
                start: u32::try_from(page + 1).unwrap_or(u32::MAX),
                ..Label::default()
            },
        }
    }
}

/// The document properties form's fields, by name.
fn meta_fields(i: &mut Info) -> [(&'static str, &mut Option<String>); 6] {
    [
        ("Title", &mut i.title),
        ("Author", &mut i.author),
        ("Subject", &mut i.subject),
        ("Keywords", &mut i.keywords),
        ("Creator", &mut i.creator),
        ("Producer", &mut i.producer),
    ]
}

/// Whether the properties form says something the document doesn't. Fields left alone are
/// `None`, and an empty field is the same as one the document doesn't have.
pub fn props_changed(form: &Info, doc: &Info) -> bool {
    let (mut form, mut doc) = (form.clone(), doc.clone());
    let changed = meta_fields(&mut form)
        .into_iter()
        .zip(meta_fields(&mut doc))
        .any(|((_, f), (_, d))| {
            f.as_ref()
                .is_some_and(|f| f != d.as_deref().unwrap_or_default())
        });
    changed
}

/// The page label prefix's name as the field being typed in (see `Forms::typing`).
const LABEL_PREFIX: &str = "label-prefix";

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
    /// The page (0-based) `label` is for: the first selected page.
    pub label_page: usize,
    /// The viewer preferences being edited, and the page shown in the page box (0-based)
    /// while no opening page is set.
    pub view: ViewPrefs,
    pub view_page: usize,
    /// Section to open and scroll to on the next frame.
    pub reveal: Option<Section>,
    /// Section to close on the next frame.
    pub close: Option<Section>,
    /// The sections open, and the frame (`egui`'s pass number) the panel was last drawn in:
    /// the pages show the crop box and stamp previews while their section is open.
    shown: Vec<Section>,
    shown_in: u64,
    /// The text field that had the keyboard when the panel was last drawn (see
    /// [`meta_fields`] and [`LABEL_PREFIX`]).
    typing: Option<&'static str>,
    /// The watermark and the headers and footers aren't previewed on the pages: one was
    /// just added, and the form hasn't changed since (so a stamp the form is no longer
    /// linked to isn't shown twice).
    pub wm_added: bool,
    pub hf_added: bool,
    /// Settings a stamp couldn't be updated to, so they aren't tried again every frame.
    pub refused: Option<StampForm>,
    /// Document revision the properties were read at.
    meta_rev: Option<u64>,
    /// Selection the page ranges were filled in for.
    filled_for: Option<Selection>,
}

impl Forms {
    /// Whether section `s` was open when the panel was last drawn, in pass `pass` or the one
    /// before (the panel is drawn before the pages, so normally in this one).
    pub fn open(&self, s: Section, pass: u64) -> bool {
        self.drawn(pass) && self.shown.contains(&s)
    }

    /// Whether the panel was drawn in pass `pass` or the one before.
    pub fn drawn(&self, pass: u64) -> bool {
        pass.saturating_sub(self.shown_in) <= 1
    }

    /// Whether a text field had the keyboard when the panel was last drawn.
    pub fn typing(&self) -> bool {
        self.typing.is_some()
    }
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
    /// and blank-page size after the selection changed, and the crop margins and page
    /// labels after either. A text field being typed in keeps what was typed.
    fn sync_forms(&mut self, ctx: &egui::Context) {
        let changed = self.forms.meta_rev != Some(self.revision);
        let typing = self.forms.typing;
        if changed {
            if self.forms.meta_rev.is_none() {
                self.forms.sec = Sec::of(self.encryption.as_ref());
                self.forms.split = 1;
                self.forms.split_level = 1;
            }
            let mut typed = std::mem::replace(&mut self.forms.meta, self.pdf.info());
            for ((name, now), (_, was)) in meta_fields(&mut self.forms.meta)
                .into_iter()
                .zip(meta_fields(&mut typed))
            {
                if typing == Some(name) {
                    *now = was.take();
                }
            }
            self.forms.view = self.pdf.view_prefs();
            self.forms.view_page = self.forms.view.page.unwrap_or(0);
            self.forms.meta_rev = Some(self.revision);
        }
        let sel = self.ed.selection.clone();
        let first = sel.pages().and_then(|s| s.first().copied());
        let moved = self.forms.filled_for.as_ref() != Some(&sel);
        if moved && typing == Some(LABEL_PREFIX) {
            // Typing for other pages, left by choosing these: kept for those pages.
            self.keep_label(ctx, None);
        }
        if changed || moved {
            let margins = self.pdf.crop_margins(first.unwrap_or(0));
            self.forms.crop.margins = margins.unwrap_or_default();
            let page = first.unwrap_or(0);
            let label = Label::at(&self.pdf.label_ranges(), page);
            let typed = std::mem::replace(&mut self.forms.label, label);
            if typing == Some(LABEL_PREFIX) && !moved {
                self.forms.label.prefix = typed.prefix;
            }
            self.forms.label_page = page;
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

    /// Keep the properties form's changes, one undo step.
    pub fn keep_props(&mut self, ctx: &egui::Context) {
        if props_changed(&self.forms.meta, &self.pdf.info()) {
            let info = self.forms.meta.clone();
            self.edit_setting(ctx, None, |p| p.set_info(&info));
        }
    }

    /// Keep the opening view form's changes, one undo step; `joins` as for
    /// [`Tab::edit_setting`].
    pub fn keep_view(&mut self, ctx: &egui::Context, joins: Option<&'static str>) {
        let prefs = self.forms.view;
        if prefs != self.pdf.view_prefs() {
            self.edit_setting(ctx, joins, |p| p.set_view_prefs(&prefs));
        }
    }

    /// Number the pages from the label form's page on as it says, unless that is the
    /// numbering they have; `joins` as for [`Tab::edit_setting`].
    pub fn keep_label(&mut self, ctx: &egui::Context, joins: Option<&'static str>) {
        let page = self.forms.label_page;
        let l = &self.forms.label;
        if page >= self.pdf.page_count() || Label::at(&self.pdf.label_ranges(), page) == *l {
            return;
        }
        let range = LabelRange {
            page,
            style: l.style,
            prefix: l.prefix.clone(),
            first: l.start.max(1),
        };
        self.edit_setting(ctx, joins, |p| p.set_label_range(range));
    }

    /// Add the stamp the form for `kind` describes to its pages, or, with `update`, put it
    /// in place of the one the form is linked to.
    pub fn stamp_from_form(&mut self, ctx: &egui::Context, kind: Stamp, update: bool) {
        let (form, range) = match kind {
            Stamp::Watermark => (StampForm::Wm(self.forms.wm.clone()), &self.forms.wm.range),
            Stamp::HeaderFooter => (StampForm::Hf(self.forms.hf.clone()), &self.forms.hf.range),
        };
        let pages = match parse_page_range(range, self.page_count()) {
            Ok(p) => p,
            Err(e) => {
                self.error = Some(e.to_string());
                self.forms.refused = Some(form);
                return;
            }
        };
        if !self.put_stamp(ctx, form.clone(), &pages, update) {
            self.forms.refused = Some(form);
            return;
        }
        match kind {
            Stamp::Watermark => self.forms.wm_added = true,
            Stamp::HeaderFooter => self.forms.hf_added = true,
        }
    }

    /// Whether a text field of the forms holds typing the document doesn't have yet.
    pub fn typing_in_forms(&self) -> bool {
        match self.forms.typing {
            None => false,
            Some(LABEL_PREFIX) => {
                Label::at(&self.pdf.label_ranges(), self.forms.label_page) != self.forms.label
            }
            Some(_) => props_changed(&self.forms.meta, &self.pdf.info()),
        }
    }

    /// Keep what is being typed in the forms, as leaving the field would (before saving or
    /// closing), and a linked stamp's changes not yet in it.
    pub fn keep_forms(&mut self, ctx: &egui::Context) {
        match self.forms.typing.take() {
            Some(LABEL_PREFIX) => self.keep_label(ctx, None),
            Some(_) => self.keep_props(ctx),
            None => {}
        }
        for kind in [Stamp::Watermark, Stamp::HeaderFooter] {
            if self.stamp_behind(kind) {
                self.stamp_from_form(ctx, kind, true);
            }
        }
    }

    /// Whether the form for `kind` is linked to a stamp and says something else, that it
    /// can be updated to.
    fn stamp_behind(&self, kind: Stamp) -> bool {
        let form = match kind {
            Stamp::Watermark if self.forms.wm.ready() => StampForm::Wm(self.forms.wm.clone()),
            Stamp::HeaderFooter => StampForm::Hf(self.forms.hf.clone()),
            Stamp::Watermark => return false,
        };
        self.linked(kind).is_some_and(|s| s.form != form)
            && self.forms.refused.as_ref() != Some(&form)
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
    t.sync_forms(ui.ctx());
    let mut apply = None;
    let reveal = t.forms.reveal.take();
    let close = t.forms.close.take();
    let pages = t.ed.selection.pages().cloned();
    let wm_link = t.linked(Stamp::Watermark).is_some();
    let hf_link = t.linked(Stamp::HeaderFooter).is_some();
    let wm_behind = t.stamp_behind(Stamp::Watermark);
    let hf_behind = t.stamp_behind(Stamp::HeaderFooter);
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
    let was_typing = f.typing.take();
    f.shown.clear();
    f.shown_in = ui.ctx().cumulative_pass_nr();
    let mut sections = Sections {
        reveal,
        close,
        open: Vec::new(),
    };
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
            sections.show(ui, None, "Insert blank page", |ui| {
                if blank_ui(ui, &mut f.blank, last) {
                    apply = Some(Apply::Blank(last));
                }
            });
            let labelled = t.labels.is_some();
            sections.show(ui, Some(Section::Labels), "Page labels", |ui| {
                let page = f.label_page;
                if let Some(a) = label_ui(ui, &mut f.label, page, labelled, &mut f.typing) {
                    apply = Some(a);
                }
            });
        }
        None => {
            sections.show(ui, Some(Section::Properties), "Properties", |ui| {
                if meta_ui(ui, &mut f.meta, &mut f.typing) {
                    apply = Some(Apply::Props);
                }
            });
            sections.show(ui, None, "Opening view", |ui| {
                if let Some(joins) = view_ui(ui, &mut f.view, &mut f.view_page, &here) {
                    apply = Some(Apply::ViewPrefs(joins));
                }
            });
            let sec = "Password security";
            sections.show(ui, Some(Section::Security), sec, |ui| {
                if security_ui(ui, &mut f.sec) {
                    apply = Some(Apply::Security);
                }
            });
            if c.form {
                sections.show(ui, None, "Form", |ui| {
                    if let Some(a) = form_ui(ui, &t.ed.form, c) {
                        apply = Some(Apply::Act(a));
                    }
                });
            }
        }
    }
    // Tools that work on a page range: the selected pages, or all.
    sections.show(ui, Some(Section::Crop), "Crop", |ui| {
        if let Some(joins) = crop_ui(ui, &mut f.crop) {
            apply = Some(Apply::Crop(joins));
        }
    });
    sections.show(ui, Some(Section::Watermark), "Watermark", |ui| {
        let before = f.wm.clone();
        let press = watermark_ui(ui, &mut f.wm, wm_link);
        // Changed since one was added: shown on the pages again until added.
        f.wm_added &= wm_link || f.wm == before;
        if let Some(a) = stamp_change(ui, Stamp::Watermark, press, wm_behind) {
            apply = Some(a);
        }
    });
    let hf_title = "Headers, footers, Bates";
    sections.show(ui, Some(Section::HeaderFooter), hf_title, |ui| {
        let before = f.hf.clone();
        let press = header_footer_ui(ui, &mut f.hf, hf_link);
        f.hf_added &= hf_link || f.hf == before;
        if let Some(a) = stamp_change(ui, Stamp::HeaderFooter, press, hf_behind) {
            apply = Some(a);
        }
    });
    let ocr = "Recognise text (OCR)";
    sections.show(ui, Some(Section::Ocr), ocr, |ui| {
        ui.weak("Pages without text, Latin script only.");
        range_row(ui, &mut f.ocr);
        let run = egui::Button::new("Recognise");
        if ui.add_enabled(!c.busy, run).clicked() {
            apply = Some(Apply::Ocr);
        }
    });
    let scans = "Optimise scanned pages";
    sections.show(ui, Some(Section::Scans), scans, |ui| {
        if scan_ui(ui, &mut f.scan_pages, &mut f.scan, c.busy) {
            apply = Some(Apply::Scans);
        }
    });
    if pages.is_none() {
        let export = "Export pages as images";
        sections.show(ui, Some(Section::Export), export, |ui| {
            if export_ui(ui, &mut f.export, c.busy) {
                apply = Some(Apply::Export);
            }
        });
        sections.show(ui, Some(Section::Split), "Split into files", |ui| {
            ui.horizontal(|ui| {
                ui.radio_value(&mut f.split_at_marks, false, "Pages per file");
                let r = ui.add_enabled(
                    !f.split_at_marks,
                    egui::DragValue::new(&mut f.split).range(1..=100_000),
                );
                set_name(&r, "Pages per file");
            });
            ui.horizontal(|ui| {
                ui.radio_value(&mut f.split_at_marks, true, "At bookmarks, levels");
                let r = ui.add_enabled(
                    f.split_at_marks,
                    egui::DragValue::new(&mut f.split_level).range(1..=9),
                );
                set_name(&r, "Bookmark levels");
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
    f.shown = sections.open;
    // A field typed in that no longer has the keyboard is kept, as leaving it does, also
    // when it is no longer drawn (the panel showing selected pages instead).
    if apply.is_none() && f.typing.is_none() {
        apply = match was_typing {
            Some(LABEL_PREFIX) => Some(Apply::Label(None)),
            Some(_) => Some(Apply::Props),
            None => None,
        };
    }
    apply
}

/// The form: the field with the keyboard, if any, and commands for the whole form.
fn form_ui(ui: &mut Ui, form: &FormUi, c: &Ctx) -> Option<Act> {
    let mut act = None;
    let n = form.fields().len();
    ui.weak(format!("{n} field{}", if n == 1 { "" } else { "s" }));
    if let Some(f) = form.focused_field() {
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

/// The panel's collapsible sections: the one to open and scroll into view (`reveal`), the
/// one to close, and those found open.
struct Sections {
    reveal: Option<Section>,
    close: Option<Section>,
    open: Vec<Section>,
}

impl Sections {
    fn show(
        &mut self,
        ui: &mut Ui,
        this: Option<Section>,
        title: &str,
        body: impl FnOnce(&mut Ui),
    ) {
        let reveal = this.is_some() && self.reveal == this;
        let close = this.is_some() && self.close == this;
        let set = if close {
            Some(false)
        } else {
            reveal.then_some(true)
        };
        let r = egui::CollapsingHeader::new(title)
            .id_salt(("inspector", title))
            .open(set)
            .show(ui, body);
        if reveal {
            r.header_response.scroll_to_me(Some(egui::Align::TOP));
        }
        // Open as the user left it, not while it folds away.
        let id = r.header_response.id;
        let state = egui::collapsing_header::CollapsingState::load(ui.ctx(), id);
        if state.is_some_and(|s| s.is_open()) {
            self.open.extend(this);
        }
    }
}

/// A full-width button for a registry command: icon and label, disabled when the
/// command is not available. Screen readers say the label alone.
fn act_button(ui: &mut Ui, act: Act, c: &Ctx) -> bool {
    let a = actions::get(act);
    let glyph = theme::icon(if a.icon.is_empty() { " " } else { a.icon }).size(16.0);
    let b = egui::Button::new((glyph, a.label))
        .min_size(egui::vec2(ui.available_width(), 26.0))
        .frame_when_inactive(false);
    let r = ui.add_enabled((a.enabled)(c), b);
    set_name(&r, a.label);
    r.clicked()
}

fn range_row(ui: &mut Ui, text: &mut String) -> egui::Response {
    ui.horizontal(|ui| {
        ui.label("Pages");
        let r = crate::dialogs::pages_box(ui, text, Some(f32::INFINITY));
        set_name(&r, "Pages");
        r
    })
    .inner
}

/// A two-column grid of labels and controls, sized for the panel.
fn grid(ui: &mut Ui, id: &str, body: impl FnOnce(&mut Ui)) {
    egui::Grid::new(id)
        .num_columns(2)
        .spacing([8.0, 6.0])
        .show(ui, body);
}

/// A row of a grid: `label`, then a slider with its number, named by the label.
fn slider_row<N: egui::emath::Numeric>(
    ui: &mut Ui,
    label: &str,
    value: &mut N,
    range: std::ops::RangeInclusive<N>,
    suffix: &str,
) -> egui::Response {
    row(ui, label, |ui| {
        ui.horizontal(|ui| slider(ui, value, range, suffix, label))
            .inner
    })
}

/// A row of a grid: `label`, then the control `add` makes, named by the label for screen
/// readers.
fn row(ui: &mut Ui, label: &str, add: impl FnOnce(&mut Ui) -> egui::Response) -> egui::Response {
    ui.label(label);
    let r = add(ui);
    set_name(&r, label);
    ui.end_row();
    r
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
        set_name(&ui.add(points(&mut size.0)), "Width");
        ui.label("×");
        set_name(&ui.add(points(&mut size.1)), "Height");
    });
    ui.button(format!("Insert after page {}", after + 1))
        .clicked()
}

/// Whether a number field's change is to be kept now: once it is let go, not while it is
/// dragged. A number typed in arrives when the field is left or Enter pressed (the
/// fields don't `update_while_editing`).
fn let_go(r: &egui::Response) -> bool {
    (r.changed() && !r.dragged()) || r.drag_stopped()
}

/// The numbering from page `first` on, until the next numbering the document has after it,
/// as it is; a change applies at once. `typing`: set when the prefix has the keyboard.
fn label_ui(
    ui: &mut Ui,
    s: &mut Label,
    first: usize,
    labelled: bool,
    typing: &mut Option<&'static str>,
) -> Option<Apply> {
    let name = |st: Option<LabelStyle>| st.map_or("Prefix only", LabelStyle::name);
    let mut apply = None;
    ui.horizontal(|ui| {
        ui.label("Style");
        let style = s.style;
        let r = egui::ComboBox::from_id_salt("label-style")
            .selected_text(name(s.style))
            .show_ui(ui, |ui| {
                for st in LabelStyle::ALL.map(Some).into_iter().chain([None]) {
                    ui.selectable_value(&mut s.style, st, name(st));
                }
            });
        set_name(&r.response, "Style");
        if s.style != style {
            apply = Some(Apply::Label(None));
        }
    });
    ui.horizontal(|ui| {
        ui.label("Prefix");
        let r = ui.add(egui::TextEdit::singleline(&mut s.prefix).desired_width(80.0));
        set_name(&r, "Prefix");
        if r.has_focus() {
            *typing = Some(LABEL_PREFIX);
        }
        if r.lost_focus() {
            apply = Some(Apply::Label(None));
        }
        ui.label("Start at");
        let start = egui::DragValue::new(&mut s.start)
            .range(1..=999_999_999)
            .update_while_editing(false);
        let r = ui.add_enabled(s.style.is_some(), start);
        set_name(&r, "Start at");
        if let_go(&r) {
            apply = Some(Apply::Label(Some("label-start")));
        }
    });
    let shown = match s.style {
        Some(st) => format!("{}{}", s.prefix, st.format(s.start)),
        None => s.prefix.clone(),
    };
    ui.weak(format!("Page {} is labelled “{shown}”.", first + 1));
    if labelled && ui.button("Remove all page labels").clicked() {
        return Some(Apply::RemoveLabels);
    }
    apply
}

/// The margins trimmed from the pages, as they are; a change crops at once (see
/// [`Tab::crop`]), and Remove crop sets them to zero. While the section is open the pages
/// are shown whole with the crop box on them.
fn crop_ui(ui: &mut Ui, s: &mut Crop) -> Option<Option<&'static str>> {
    range_row(ui, &mut s.pages);
    let mut apply = None;
    s.dragged = false;
    grid(ui, "crop", |ui| {
        for (name, i, key) in [
            ("Top", 0, "crop-top"),
            ("Bottom", 2, "crop-bottom"),
            ("Left", 3, "crop-left"),
            ("Right", 1, "crop-right"),
        ] {
            let v = &mut s.margins[i];
            let field = egui::DragValue::new(v)
                .range(0.0..=14400.0)
                .suffix(" pt")
                .update_while_editing(false);
            let r = row(ui, name, |ui| ui.add(field));
            s.dragged |= r.dragged();
            if let_go(&r) {
                apply = Some(Some(key));
            }
        }
    });
    if ui.button("Remove crop").clicked() {
        s.margins = [0.0; 4];
        apply = Some(None);
    }
    apply
}

/// The document's properties; a field is kept when Enter is pressed or it is left, so
/// returns true then. `typing`: set to the field that has the keyboard.
fn meta_ui(ui: &mut Ui, s: &mut Info, typing: &mut Option<&'static str>) -> bool {
    let mut kept = false;
    grid(ui, "meta", |ui| {
        for (label, f) in meta_fields(s) {
            let mut v = f.clone().unwrap_or_default();
            let field = egui::TextEdit::singleline(&mut v).desired_width(f32::INFINITY);
            let r = row(ui, label, |ui| ui.add(field));
            if r.changed() {
                *f = Some(v);
            }
            if r.has_focus() {
                *typing = Some(label);
            }
            kept |= r.lost_focus();
        }
    });
    kept
}

/// How the document is being viewed now, for "Use current view".
struct Here {
    layout: OpenLayout,
    /// The side tab shown, when the side panel is open and on a tab a document can ask for.
    panel: Option<OpenPanel>,
    page: usize,
    pages: usize,
}

/// The document's opening view, as it is; a choice applies as it is made, so returns
/// `Some` then (with the name to join undo steps by, for the page number).
fn view_ui(
    ui: &mut Ui,
    v: &mut ViewPrefs,
    page: &mut usize,
    here: &Here,
) -> Option<Option<&'static str>> {
    let before = *v;
    let mut number = None;
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
        row(ui, "Layout", |ui| {
            egui::ComboBox::from_id_salt("view-layout")
                .selected_text(layout_name(v.layout))
                .show_ui(ui, |ui| {
                    for l in [None].into_iter().chain(OpenLayout::ALL.map(Some)) {
                        ui.selectable_value(&mut v.layout, l, layout_name(l));
                    }
                })
                .response
        });
        row(ui, "Side panel", |ui| {
            egui::ComboBox::from_id_salt("view-panel")
                .selected_text(panel_name(v.panel))
                .show_ui(ui, |ui| {
                    for p in [None].into_iter().chain(OpenPanel::ALL.map(Some)) {
                        ui.selectable_value(&mut v.panel, p, panel_name(p));
                    }
                })
                .response
        });
        ui.label("Page");
        ui.horizontal(|ui| {
            let mut set = v.page.is_some();
            ui.checkbox(&mut set, "Open at");
            let mut n = *page + 1;
            let field = egui::DragValue::new(&mut n)
                .range(1..=here.pages.max(1))
                .update_while_editing(false);
            let r = ui.add_enabled(set, field);
            set_name(&r, "Page to open at");
            *page = n - 1;
            v.page = set.then_some(*page);
            number = Some(r);
        });
        ui.end_row();
    });
    if ui
        .button("Use current view")
        .on_hover_text("Set this document's layout, side panel and page as you see them now.")
        .clicked()
    {
        *page = here.page;
        *v = ViewPrefs {
            layout: Some(here.layout),
            panel: here.panel,
            page: Some(here.page),
        };
        return Some(None);
    }
    match number {
        Some(r) if r.dragged() => None,
        Some(r) if let_go(&r) => Some(Some("view-page")),
        _ => (*v != before).then_some(None),
    }
}

fn security_ui(ui: &mut Ui, s: &mut Sec) -> bool {
    ui.checkbox(&mut s.enabled, "Protect with passwords when saving");
    ui.add_enabled_ui(s.enabled, |ui| {
        grid(ui, "sec", |ui| {
            ui.label("Open");
            let r = ui
                .add(egui::TextEdit::singleline(&mut s.user).password(true))
                .on_hover_text("Needed to open the document");
            set_name(&r, "Open password");
            ui.end_row();
            ui.label("Permissions");
            let r = ui
                .add(egui::TextEdit::singleline(&mut s.owner).password(true))
                .on_hover_text("Needed to change the permissions");
            set_name(&r, "Permissions password");
            ui.end_row();
            let name = |c: Cipher| match c {
                Cipher::Rc4_40 => "40-bit RC4 (weak)",
                Cipher::Rc4_128 => "128-bit RC4 (weak)",
                Cipher::Aes128 => "128-bit AES",
                Cipher::Aes256 => "256-bit AES",
            };
            row(ui, "Encryption", |ui| {
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
                    })
                    .response
            });
        });
        ui.label("Allow:");
        ui.checkbox(&mut s.r.print, "Printing");
        ui.checkbox(&mut s.r.print_high_quality, "High-resolution printing");
        ui.checkbox(&mut s.r.modify, "Modifying");
        ui.checkbox(&mut s.r.copy, "Copying content");
        ui.checkbox(&mut s.r.annotate, "Commenting");
        ui.checkbox(&mut s.r.fill_forms, "Filling form fields");
        ui.checkbox(&mut s.r.assemble, "Page assembly");
        ui.weak("Other apps may ignore these.");
    });
    ui.weak("Applies when you save.");
    ui.button("Apply").clicked()
}

/// What happened in a stamp section: its button was pressed (Add to pages, or Add
/// another), or a text field was left.
#[derive(Default)]
struct Press {
    add: bool,
    left: bool,
}

/// A text field of a stamp section, which notes in `p` when it is left; a grid row named
/// `label`.
fn stamp_text(ui: &mut Ui, label: &str, text: &mut String, p: &mut Press) {
    let field = egui::TextEdit::singleline(text).desired_width(f32::INFINITY);
    let r = row(ui, label, |ui| ui.add(field));
    p.left |= r.lost_focus();
}

/// The button of a stamp section. Before one is added, it is shown on the pages and Add
/// to pages adds it (`ready`: there is something to add); once added (`linked`), changes
/// update it and Add another adds one more.
fn stamp_button(ui: &mut Ui, linked: bool, ready: bool, what: &str, p: &mut Press) {
    if linked {
        ui.weak(format!("Changes update the {what} just added."));
        p.add |= ui.button("Add another").clicked();
    } else {
        let add = egui::Button::new("Add to pages");
        p.add |= ui.add_enabled(ready, add).clicked();
    }
}

/// The change a stamp section asks for: adding one when its button was pressed; else, when
/// the form is linked to a stamp and says something else (`behind`), updating it once
/// nothing is being dragged, typed or picked (a colour applies when its picker closes).
fn stamp_change(ui: &Ui, kind: Stamp, p: Press, behind: bool) -> Option<Apply> {
    if p.add {
        return Some(Apply::Stamp {
            kind,
            update: false,
        });
    }
    let busy = ui.input(|i| i.pointer.any_down())
        || ui.ctx().egui_wants_keyboard_input()
        || egui::Popup::is_any_open(ui.ctx());
    let settled = p.left || !busy;
    (behind && settled).then_some(Apply::Stamp { kind, update: true })
}

fn watermark_ui(ui: &mut Ui, s: &mut Wm, linked: bool) -> Press {
    let mut p = Press::default();
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
            slider_row(ui, "Size", &mut s.scale, 5.0..=100.0, "% of page");
        } else {
            stamp_text(ui, "Text", &mut s.text, &mut p);
            slider_row(ui, "Size", &mut s.size, 12.0..=300.0, "");
        }
        slider_row(ui, "Opacity", &mut s.opacity, 0.02..=1.0, "");
        slider_row(ui, "Rotation", &mut s.rotation, -90.0..=90.0, "°");
        if !s.picture {
            row(ui, "Colour", |ui| ui.color_edit_button_rgb(&mut s.color));
        }
    });
    ui.radio_value(&mut s.foreground, true, "Above the content");
    ui.radio_value(&mut s.foreground, false, "Behind the content");
    p.left |= range_row(ui, &mut s.range).lost_focus();
    stamp_button(ui, linked, s.ready(), "watermark", &mut p);
    p
}

fn header_footer_ui(ui: &mut Ui, s: &mut Hf, linked: bool) -> Press {
    let mut p = Press::default();
    ui.weak("Tokens: {page}  {pages}  {date}  {bates}");
    grid(ui, "hf", |ui| {
        let sides = ["left", "centre", "right"];
        for (name, line) in [("Header", &mut s.header), ("Footer", &mut s.footer)] {
            for (side, text) in sides.iter().zip(line.iter_mut()) {
                stamp_text(ui, &format!("{name}, {side}"), text, &mut p);
            }
        }
        row(ui, "Font size", |ui| {
            ui.add(egui::DragValue::new(&mut s.size).range(5.0..=48.0))
        });
        row(ui, "Margin", |ui| {
            ui.add(
                egui::DragValue::new(&mut s.margin)
                    .range(10.0..=144.0)
                    .suffix(" pt"),
            )
        });
    });
    ui.checkbox(&mut s.bates, "Bates numbering")
        .on_hover_text("Without a {bates} token the number goes at the bottom right.");
    ui.add_enabled_ui(s.bates, |ui| {
        grid(ui, "bates", |ui| {
            stamp_text(ui, "Prefix", &mut s.prefix, &mut p);
            stamp_text(ui, "Suffix", &mut s.suffix, &mut p);
            row(ui, "Digits", |ui| {
                ui.add(egui::DragValue::new(&mut s.digits).range(1..=12))
            });
            row(ui, "Start", |ui| {
                ui.add(egui::DragValue::new(&mut s.start).range(0..=999_999_999))
            });
        });
    });
    p.left |= range_row(ui, &mut s.range).lost_focus();
    stamp_button(ui, linked, true, "headers and footers", &mut p);
    p
}

fn scan_ui(ui: &mut Ui, pages: &mut String, o: &mut ScanOptions, busy: bool) -> bool {
    ui.weak("Only pages that are one full-page image.");
    range_row(ui, pages);
    grid(ui, "scan", |ui| {
        let label = |m: ScanMode| match m {
            ScanMode::Auto => "Automatic",
            ScanMode::BlackWhite => "Black and white",
            ScanMode::Gray => "Greyscale",
            ScanMode::Color => "Colour",
        };
        row(ui, "Store as", |ui| {
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
                })
                .response
        });
        ui.label("Image resolution")
            .on_hover_text("Greyscale and colour images above this are downsampled");
        let r = ui.add(
            egui::DragValue::new(&mut o.max_dpi)
                .range(72.0..=600.0)
                .suffix(" dpi"),
        );
        set_name(&r, "Image resolution");
        ui.end_row();
        slider_row(ui, "JPEG quality", &mut o.jpeg_quality, 10..=100, "");
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
        row(ui, "Resolution", |ui| {
            ui.add(
                egui::DragValue::new(&mut s.dpi)
                    .range(36.0..=MAX_DPI)
                    .suffix(" dpi"),
            )
        });
        ui.label("JPEG quality");
        ui.add_enabled_ui(s.jpeg, |ui| {
            ui.horizontal(|ui| slider(ui, &mut s.quality, 10..=100, "", "JPEG quality"));
        });
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

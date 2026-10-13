use crate::dialogs::{self as dlg, Outcome};
use crate::editor::Tool;
use crate::tab::{Side, Tab};
use eframe::egui::{self, Key, KeyboardShortcut, Modifiers};
use egui_phosphor::regular as icon;
use mpe_core::{
    parse_page_range, Bates, Encryption, HeaderFooter, ImageImport, OcrResult, Pdf, ScanResult,
    Watermark,
};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{channel, Receiver, Sender};

const IMAGE_EXTS: [&str; 5] = ["png", "jpg", "jpeg", "tif", "tiff"];

fn is_image(p: &Path) -> bool {
    p.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| IMAGE_EXTS.contains(&e.to_ascii_lowercase().as_str()))
}

/// Insert image files as pages at `at`, in the order given, naming the file that fails.
fn insert_images(p: &mut Pdf, mut at: usize, files: &[(PathBuf, Vec<u8>)]) -> mpe_core::Result<()> {
    for (path, bytes) in files {
        at += p
            .insert_image_pages(at, std::slice::from_ref(bytes), &ImageImport::default())
            .map_err(|e| e.context(path.display().to_string()))?;
    }
    Ok(())
}

enum Loaded {
    Ok {
        path: PathBuf,
        pdf: Pdf,
        page: Option<usize>,
    },
    NeedPassword {
        path: PathBuf,
        error: Option<String>,
    },
    Err(String),
    /// A background job on a copy of tab `tab` at `revision` finished. On success: the
    /// changed document, whether anything changed, and a summary for the user.
    Job {
        tab: u64,
        revision: u64,
        title: &'static str,
        result: Result<(Pdf, bool, String), String>,
    },
}

/// Summary of an OCR run for the user, and whether any page changed.
fn ocr_summary(report: &[(usize, OcrResult)]) -> (bool, String) {
    let (mut pages, mut words) = (0, 0);
    let mut skipped = Vec::new();
    for (p, r) in report {
        match r {
            OcrResult::Added { words: w, .. } => {
                pages += 1;
                words += w;
            }
            OcrResult::Skipped(why) => skipped.push(format!("Page {}: {why}", p + 1)),
        }
    }
    let mut text = format!("Text added to {pages} page(s): {words} words.");
    if pages > 0 {
        text.push_str(
            "\n\nThe text is invisible. To see it, search for a word, or turn on \
             View > Show recognised text; misread words can be corrected with the arrow tool.",
        );
    }
    push_skipped(&mut text, &skipped);
    (pages > 0, text)
}

fn push_skipped(text: &mut String, skipped: &[String]) {
    if skipped.is_empty() {
        return;
    }
    text.push_str(&format!("\n\n{} page(s) left as they were:", skipped.len()));
    for s in skipped.iter().take(12) {
        text.push('\n');
        text.push_str(s);
    }
    if skipped.len() > 12 {
        text.push_str("\n…");
    }
}

/// Summary of a scan optimisation for the user, and whether any page changed.
fn scan_summary(report: &[(usize, ScanResult)]) -> (bool, String) {
    let (mut n, mut before, mut after) = (0, 0, 0);
    let mut skipped = Vec::new();
    for (p, r) in report {
        match r {
            ScanResult::Optimized {
                before: b,
                after: a,
                ..
            } => {
                n += 1;
                before += b;
                after += a;
            }
            ScanResult::Skipped(why) => skipped.push(format!("Page {}: {why}", p + 1)),
        }
    }
    let mb = |b: usize| b as f64 / 1e6;
    let mut text = format!(
        "{n} page(s) optimised: images {:.1} MB -> {:.1} MB.",
        mb(before),
        mb(after)
    );
    push_skipped(&mut text, &skipped);
    (n > 0, text)
}

enum Dialog {
    None,
    Watermark(dlg::Wm),
    HeaderFooter(dlg::Hf),
    Meta(dlg::Meta),
    Security(dlg::Sec),
    Range(dlg::Range),
    Blank(dlg::Blank),
    Scan(dlg::Scan),
    Password(dlg::Password),
    Message(String, String),
    ConfirmClose(usize),
    ConfirmQuit,
    About,
}

#[derive(Clone, Copy)]
enum Act {
    Open,
    NewFromImages,
    InsertImages,
    Scans,
    Ocr,
    Save,
    SaveAs,
    CloseTab,
    Quit,
    Undo,
    Redo,
    Find,
    Copy,
    ZoomIn,
    ZoomOut,
    FitWidth,
    Actual,
    RotL,
    RotR,
    Delete,
    Extract,
    Blank,
    Merge,
    Split,
    HeaderFooter,
    Watermark,
    Props,
    Security,
    About,
    Theme(egui::ThemePreference),
}

pub struct App {
    tabs: Vec<Tab>,
    active: usize,
    next_id: u64,
    dialog: Dialog,
    tx: Sender<Loaded>,
    rx: Receiver<Loaded>,
    show_side: bool,
    status: String,
    last_title: String,
    quit_ok: bool,
    /// A background job (scan optimisation, OCR) is running.
    busy: bool,
}

impl App {
    pub fn new(cc: &eframe::CreationContext<'_>, files: Vec<PathBuf>, page: Option<usize>) -> Self {
        let mut fonts = egui::FontDefinitions::default();
        egui_phosphor::add_to_fonts(&mut fonts, egui_phosphor::Variant::Regular);
        cc.egui_ctx.set_fonts(fonts);
        let (tx, rx) = channel();
        let mut app = Self {
            tabs: Vec::new(),
            active: 0,
            next_id: 1,
            dialog: Dialog::None,
            tx,
            rx,
            show_side: true,
            status: String::new(),
            last_title: String::new(),
            quit_ok: false,
            busy: false,
        };
        let (images, pdfs): (Vec<_>, Vec<_>) = files.into_iter().partition(|p| is_image(p));
        for f in pdfs {
            app.spawn_open(&cc.egui_ctx, f, None, page);
        }
        if !images.is_empty() {
            app.new_from_images(&cc.egui_ctx, &images);
        }
        app
    }

    fn spawn_open(
        &self,
        ctx: &egui::Context,
        path: PathBuf,
        password: Option<String>,
        page: Option<usize>,
    ) {
        let tx = self.tx.clone();
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let msg = match Pdf::open(&path, password.as_deref()) {
                Ok(pdf) => Loaded::Ok { path, pdf, page },
                Err(e) => {
                    let s = format!("{e:#}");
                    if s.to_lowercase().contains("password") {
                        Loaded::NeedPassword {
                            path,
                            error: password.map(|_| "Incorrect password.".to_string()),
                        }
                    } else {
                        Loaded::Err(format!("{}: {s}", path.display()))
                    }
                }
            };
            let _ = tx.send(msg);
            ctx.request_repaint();
        });
    }

    fn add_tab(
        &mut self,
        ctx: &egui::Context,
        pdf: Pdf,
        path: Option<PathBuf>,
        page: Option<usize>,
        dirty: bool,
    ) {
        match Tab::new(ctx, self.next_id, pdf, path) {
            Ok(mut t) => {
                self.next_id += 1;
                t.dirty = dirty;
                if let Some(p) = page {
                    t.go_to(p.saturating_sub(1));
                }
                self.tabs.push(t);
                self.active = self.tabs.len() - 1;
            }
            Err(e) => self.dialog = Dialog::Message("Cannot open".into(), e),
        }
    }

    fn poll_loads(&mut self, ctx: &egui::Context) {
        while let Ok(m) = self.rx.try_recv() {
            match m {
                Loaded::Ok { path, pdf, page } => self.add_tab(ctx, pdf, Some(path), page, false),
                Loaded::NeedPassword { path, error } => {
                    self.dialog = Dialog::Password(dlg::Password {
                        path,
                        input: String::new(),
                        error,
                    });
                }
                Loaded::Err(e) => self.dialog = Dialog::Message("Cannot open file".into(), e),
                Loaded::Job {
                    tab,
                    revision,
                    title,
                    result,
                } => {
                    self.busy = false;
                    self.status.clear();
                    let Some(t) = self.tabs.iter_mut().find(|t| t.id == tab) else {
                        continue;
                    };
                    let (pdf, changed, text) = match result {
                        Ok(r) => r,
                        Err(e) => {
                            self.dialog = Dialog::Message(title.into(), e);
                            continue;
                        }
                    };
                    if t.revision != revision {
                        self.dialog = Dialog::Message(
                            title.into(),
                            "The document changed while this was running, so the result \
                             was discarded. Run it again."
                                .into(),
                        );
                        continue;
                    }
                    if changed {
                        t.edit(ctx, move |p| {
                            *p = pdf;
                            Ok(())
                        });
                    }
                    self.dialog = Dialog::Message(title.into(), text);
                }
            }
        }
    }

    fn pdf_dialog() -> rfd::FileDialog {
        rfd::FileDialog::new().add_filter("PDF documents", &["pdf"])
    }

    /// Ask for image files. Returned sorted by path, since scans are usually numbered.
    fn pick_images() -> Option<Vec<PathBuf>> {
        let mut files = rfd::FileDialog::new()
            .add_filter("Images (PNG, JPEG, TIFF)", &IMAGE_EXTS)
            .pick_files()?;
        files.sort();
        Some(files)
    }

    /// Read image files, reporting the first that cannot be read.
    fn read_images(&mut self, files: &[PathBuf]) -> Option<Vec<(PathBuf, Vec<u8>)>> {
        let mut out = Vec::new();
        for f in files {
            match std::fs::read(f) {
                Ok(b) => out.push((f.clone(), b)),
                Err(e) => {
                    self.dialog = Dialog::Message(
                        "Cannot read image".into(),
                        format!("{}: {e}", f.display()),
                    );
                    return None;
                }
            }
        }
        Some(out)
    }

    /// Run `work` on a copy of the active document on a background thread. The result is
    /// applied (undoably) when it arrives, if the document has not changed meanwhile.
    fn spawn_job(
        &mut self,
        ctx: &egui::Context,
        title: &'static str,
        work: impl FnOnce(&mut Pdf) -> mpe_core::Result<(bool, String)> + Send + 'static,
    ) {
        let t = &self.tabs[self.active];
        let (tab, revision, mut pdf) = (t.id, t.revision, t.pdf.clone());
        let (tx, ctx) = (self.tx.clone(), ctx.clone());
        std::thread::spawn(move || {
            let result = work(&mut pdf)
                .map(|(changed, text)| (pdf, changed, text))
                .map_err(|e| format!("{e:#}"));
            let _ = tx.send(Loaded::Job {
                tab,
                revision,
                title,
                result,
            });
            ctx.request_repaint();
        });
        self.busy = true;
        self.status = format!("{title}…");
    }

    fn new_from_images(&mut self, ctx: &egui::Context, files: &[PathBuf]) {
        let Some(data) = self.read_images(files) else {
            return;
        };
        let mut pdf = Pdf::new();
        match insert_images(&mut pdf, 0, &data) {
            Ok(()) => self.add_tab(ctx, pdf, None, None, true),
            Err(e) => {
                self.dialog = Dialog::Message("Cannot import images".into(), format!("{e:#}"))
            }
        }
    }

    fn save(&mut self, as_new: bool) {
        let Some(tab) = self.tabs.get_mut(self.active) else {
            return;
        };
        let path = if as_new || tab.path.is_none() {
            let mut d = Self::pdf_dialog();
            if let Some(n) = tab.path.as_ref().and_then(|p| p.file_name()) {
                d = d.set_file_name(n.to_string_lossy());
            }
            match d.save_file() {
                Some(mut p) => {
                    if p.extension().is_none() {
                        p.set_extension("pdf");
                    }
                    p
                }
                None => return,
            }
        } else {
            tab.path.clone().unwrap()
        };
        tab.pdf.set_encryption(tab.encryption.clone());
        match tab.pdf.save(&path) {
            Ok(()) => {
                self.status = format!("Saved {}", path.display());
                tab.path = Some(path);
                tab.dirty = false;
            }
            Err(e) => self.dialog = Dialog::Message("Save failed".into(), format!("{e:#}")),
        }
    }

    fn act(&mut self, ctx: &egui::Context, a: Act) {
        let has = !self.tabs.is_empty();
        match a {
            Act::Open => {
                if let Some(files) = Self::pdf_dialog().pick_files() {
                    for f in files {
                        self.spawn_open(ctx, f, None, None);
                    }
                }
            }
            Act::NewFromImages => {
                if let Some(files) = Self::pick_images() {
                    self.new_from_images(ctx, &files);
                }
            }
            Act::Save => self.save(false),
            Act::SaveAs => self.save(true),
            Act::CloseTab if has => {
                if self.tabs[self.active].dirty {
                    self.dialog = Dialog::ConfirmClose(self.active);
                } else {
                    self.close_tab(self.active);
                }
            }
            Act::Quit => {
                if self.tabs.iter().any(|t| t.dirty) {
                    self.dialog = Dialog::ConfirmQuit;
                } else {
                    self.quit_ok = true;
                    ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                }
            }
            Act::About => self.dialog = Dialog::About,
            Act::Theme(p) => ctx.set_theme(p),
            _ if !has => {}
            Act::Undo => self.tabs[self.active].undo(ctx),
            Act::Redo => self.tabs[self.active].redo(ctx),
            Act::Copy => {
                if let Some(text) = self.tabs[self.active].ed.selected_text() {
                    ctx.copy_text(text);
                    self.status = "Copied the selected text.".into();
                }
            }
            Act::Find => {
                self.show_side = true;
                self.tabs[self.active].side = Side::Search;
            }
            Act::ZoomIn => {
                let t = &mut self.tabs[self.active];
                t.set_zoom(t.zoom * 1.2);
            }
            Act::ZoomOut => {
                let t = &mut self.tabs[self.active];
                t.set_zoom(t.zoom / 1.2);
            }
            Act::Actual => self.tabs[self.active].set_zoom(1.0),
            Act::FitWidth => {
                let w = ctx.content_rect().width() - if self.show_side { 190.0 } else { 0.0 };
                self.tabs[self.active].fit_width(w);
            }
            Act::RotL | Act::RotR => {
                let deg = if matches!(a, Act::RotR) { 90 } else { 270 };
                let t = &mut self.tabs[self.active];
                let idx = t.targets();
                t.edit(ctx, |p| p.rotate_pages(&idx, deg));
            }
            Act::Delete => {
                self.dialog = Dialog::Range(dlg::Range {
                    kind: dlg::RangeKind::Delete,
                    text: self.default_range(),
                })
            }
            Act::Extract => {
                self.dialog = Dialog::Range(dlg::Range {
                    kind: dlg::RangeKind::Extract,
                    text: self.default_range(),
                })
            }
            Act::Split => {
                self.dialog = Dialog::Range(dlg::Range {
                    kind: dlg::RangeKind::SplitEvery,
                    text: "1".into(),
                })
            }
            Act::Blank => {
                let t = &self.tabs[self.active];
                let (w, h) = t.sizes.get(t.current).copied().unwrap_or((612.0, 792.0));
                self.dialog = Dialog::Blank(dlg::Blank {
                    after: t.current + 1,
                    w,
                    h,
                });
            }
            Act::Merge => {
                let Some(files) = Self::pdf_dialog().pick_files() else {
                    return;
                };
                let mut docs = Vec::new();
                for f in &files {
                    match Pdf::open(f, None) {
                        Ok(d) => docs.push(d),
                        Err(e) => {
                            self.dialog = Dialog::Message(
                                "Merge failed".into(),
                                format!("{}: {e:#}", f.display()),
                            );
                            return;
                        }
                    }
                }
                self.tabs[self.active].edit(ctx, |p| p.append(docs));
            }
            Act::InsertImages => {
                let Some(files) = Self::pick_images() else {
                    return;
                };
                let Some(data) = self.read_images(&files) else {
                    return;
                };
                let t = &mut self.tabs[self.active];
                let (at, before) = (t.current + 1, t.page_count());
                t.edit(ctx, |p| insert_images(p, at, &data));
                if t.page_count() > before {
                    t.go_to(at);
                }
            }
            Act::Scans => self.dialog = Dialog::Scan(dlg::Scan::default()),
            Act::Ocr => {
                self.dialog = Dialog::Range(dlg::Range {
                    kind: dlg::RangeKind::Ocr,
                    text: "all".into(),
                })
            }
            Act::HeaderFooter => self.dialog = Dialog::HeaderFooter(dlg::Hf::default()),
            Act::Watermark => self.dialog = Dialog::Watermark(dlg::Wm::default()),
            Act::Props => self.dialog = Dialog::Meta(dlg::Meta(self.tabs[self.active].pdf.info())),
            Act::Security => {
                let mut s = dlg::Sec::default();
                if let Some(e) = &self.tabs[self.active].encryption {
                    s = dlg::Sec {
                        enabled: true,
                        user: e.user_password.clone(),
                        owner: e.owner_password.clone(),
                        cipher: e.cipher,
                        r: e.restrictions,
                    };
                }
                self.dialog = Dialog::Security(s);
            }
            _ => {}
        }
    }

    fn default_range(&self) -> String {
        let t = &self.tabs[self.active];
        let mut v = t.targets();
        v.sort_unstable();
        // Collapse consecutive runs into a-b.
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

    fn close_tab(&mut self, i: usize) {
        if i < self.tabs.len() {
            self.tabs.remove(i);
            self.active = self.active.min(self.tabs.len().saturating_sub(1));
        }
    }

    fn dialogs(&mut self, ctx: &egui::Context) {
        let mut dialog = std::mem::replace(&mut self.dialog, Dialog::None);
        let n = self
            .tabs
            .get(self.active)
            .map(|t| t.page_count())
            .unwrap_or(0);
        let mut done = false;
        match &mut dialog {
            Dialog::None => done = true,
            Dialog::About => {
                done = dlg::message(
                    ctx,
                    "About MPE",
                    "MPE - a fast, pure-Rust PDF viewer and editor.\nLicensed under MIT OR Apache-2.0.\nRendering: hayro. Document engine: lopdf. UI: egui.\n\nOCR uses the ocrs text recognition model by Robert Knight\n(huggingface.co/robertknight/ocrs), licensed CC BY-SA 4.0\n(creativecommons.org/licenses/by-sa/4.0), included unmodified.",
                ) != Outcome::Open;
            }
            Dialog::Message(title, text) => done = dlg::message(ctx, title, text) != Outcome::Open,
            Dialog::ConfirmClose(i) => match dlg::confirm(
                ctx,
                "Unsaved changes",
                "This document has unsaved changes. Close it anyway?",
                "Discard changes",
            ) {
                Outcome::Apply => {
                    let i = *i;
                    self.close_tab(i);
                    done = true;
                }
                Outcome::Cancel => done = true,
                Outcome::Open => {}
            },
            Dialog::ConfirmQuit => match dlg::confirm(
                ctx,
                "Unsaved changes",
                "Some documents have unsaved changes. Quit anyway?",
                "Quit without saving",
            ) {
                Outcome::Apply => {
                    self.quit_ok = true;
                    ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                    done = true;
                }
                Outcome::Cancel => done = true,
                Outcome::Open => {}
            },
            Dialog::Password(s) => match dlg::password(ctx, s) {
                Outcome::Apply => {
                    self.spawn_open(ctx, s.path.clone(), Some(s.input.clone()), None);
                    done = true;
                }
                Outcome::Cancel => done = true,
                Outcome::Open => {}
            },
            _ if self.tabs.is_empty() => done = true,
            Dialog::Watermark(s) => match dlg::watermark(ctx, s) {
                Outcome::Apply => {
                    let t = &mut self.tabs[self.active];
                    match parse_page_range(&s.range, t.page_count()) {
                        Ok(idx) => {
                            let wm = Watermark {
                                text: s.text.clone(),
                                font_size: s.size,
                                opacity: s.opacity,
                                rotation: s.rotation,
                                color: (s.color[0], s.color[1], s.color[2]),
                                foreground: s.foreground,
                            };
                            t.edit(ctx, |p| p.add_watermark(&idx, &wm));
                            done = true;
                        }
                        Err(e) => t.error = Some(e.to_string()),
                    }
                }
                Outcome::Cancel => done = true,
                Outcome::Open => {}
            },
            Dialog::HeaderFooter(s) => match dlg::header_footer(ctx, s) {
                Outcome::Apply => {
                    let t = &mut self.tabs[self.active];
                    match parse_page_range(&s.range, t.page_count()) {
                        Ok(idx) => {
                            let hf = HeaderFooter {
                                header: s.header.clone(),
                                footer: s.footer.clone(),
                                font_size: s.size,
                                margin: s.margin,
                                color: (0.0, 0.0, 0.0),
                                bates: s.bates.then(|| Bates {
                                    prefix: s.prefix.clone(),
                                    suffix: s.suffix.clone(),
                                    digits: s.digits as usize,
                                    start: s.start,
                                }),
                            };
                            t.edit(ctx, |p| p.add_header_footer(&idx, &hf));
                            done = true;
                        }
                        Err(e) => t.error = Some(e.to_string()),
                    }
                }
                Outcome::Cancel => done = true,
                Outcome::Open => {}
            },
            Dialog::Meta(s) => match dlg::metadata(ctx, s) {
                Outcome::Apply => {
                    let info = s.0.clone();
                    self.tabs[self.active].edit(ctx, |p| p.set_info(&info));
                    done = true;
                }
                Outcome::Cancel => done = true,
                Outcome::Open => {}
            },
            Dialog::Security(s) => match dlg::security(ctx, s) {
                Outcome::Apply => {
                    let t = &mut self.tabs[self.active];
                    t.encryption = s.enabled.then(|| Encryption {
                        user_password: s.user.clone(),
                        owner_password: if s.owner.is_empty() {
                            s.user.clone()
                        } else {
                            s.owner.clone()
                        },
                        cipher: s.cipher,
                        restrictions: s.r,
                    });
                    t.dirty = true;
                    self.status = "Security settings apply when you save.".into();
                    done = true;
                }
                Outcome::Cancel => done = true,
                Outcome::Open => {}
            },
            Dialog::Blank(s) => match dlg::blank(ctx, s, n) {
                Outcome::Apply => {
                    let (after, w, h) = (s.after, s.w, s.h);
                    self.tabs[self.active].edit(ctx, |p| p.insert_blank(after, w, h));
                    done = true;
                }
                Outcome::Cancel => done = true,
                Outcome::Open => {}
            },
            Dialog::Scan(s) => match dlg::scan(ctx, s) {
                Outcome::Apply => {
                    let t = &mut self.tabs[self.active];
                    match parse_page_range(&s.pages, t.page_count()) {
                        Ok(idx) => {
                            let opts = s.o;
                            self.spawn_job(ctx, "Optimise scanned pages", move |p| {
                                Ok(scan_summary(&p.optimize_scans(&idx, &opts)?))
                            });
                            done = true;
                        }
                        Err(e) => t.error = Some(e.to_string()),
                    }
                }
                Outcome::Cancel => done = true,
                Outcome::Open => {}
            },
            Dialog::Range(s) => match dlg::range(ctx, s) {
                Outcome::Apply => {
                    done = self.apply_range(ctx, s.kind, &s.text);
                }
                Outcome::Cancel => done = true,
                Outcome::Open => {}
            },
        }
        if !done {
            self.dialog = dialog;
        }
    }

    /// Returns true when the dialog should close.
    fn apply_range(&mut self, ctx: &egui::Context, kind: dlg::RangeKind, text: &str) -> bool {
        let t = &mut self.tabs[self.active];
        match kind {
            dlg::RangeKind::SplitEvery => {
                let Ok(every) = text.trim().parse::<usize>() else {
                    t.error = Some("Enter a whole number of pages per file.".into());
                    return false;
                };
                let Some(dir) = rfd::FileDialog::new()
                    .set_title("Choose output folder")
                    .pick_folder()
                else {
                    return true;
                };
                let stem = t
                    .path
                    .as_ref()
                    .and_then(|p| p.file_stem())
                    .map(|s| s.to_string_lossy().into_owned())
                    .unwrap_or_else(|| "part".into());
                match t.pdf.split_every(every) {
                    Ok(parts) => {
                        let count = parts.len();
                        for (i, mut part) in parts.into_iter().enumerate() {
                            if let Err(e) = part.save(dir.join(format!("{stem}-{:03}.pdf", i + 1)))
                            {
                                self.dialog =
                                    Dialog::Message("Split failed".into(), format!("{e:#}"));
                                return true;
                            }
                        }
                        self.status = format!("Wrote {count} files to {}", dir.display());
                    }
                    Err(e) => t.error = Some(format!("{e:#}")),
                }
                true
            }
            _ => {
                let idx = match parse_page_range(text, t.page_count()) {
                    Ok(v) => v,
                    Err(e) => {
                        t.error = Some(e.to_string());
                        return false;
                    }
                };
                if kind == dlg::RangeKind::Ocr {
                    self.spawn_job(ctx, "Recognise text", move |p| {
                        let ocr = mpe_core::Ocr::bundled()?;
                        Ok(ocr_summary(&p.ocr_pages(&idx, &ocr)?))
                    });
                } else if kind == dlg::RangeKind::Delete {
                    t.edit(ctx, |p| p.delete_pages(&idx));
                } else {
                    match t.pdf.extract(&idx) {
                        Ok(p) => self.add_tab(ctx, p, None, None, true),
                        Err(e) => t.error = Some(format!("{e:#}")),
                    }
                }
                true
            }
        }
    }

    fn shortcuts(&mut self, ctx: &egui::Context) {
        let mut acts = Vec::new();
        ctx.input_mut(|i| {
            let c = |k| KeyboardShortcut::new(Modifiers::COMMAND, k);
            let cs = |k| KeyboardShortcut::new(Modifiers::COMMAND | Modifiers::SHIFT, k);
            for (sc, a) in [
                (c(Key::O), Act::Open),
                (cs(Key::S), Act::SaveAs),
                (c(Key::S), Act::Save),
                (c(Key::W), Act::CloseTab),
                (cs(Key::Z), Act::Redo),
                (c(Key::Z), Act::Undo),
                (c(Key::Y), Act::Redo),
                (c(Key::F), Act::Find),
                (c(Key::Num0), Act::FitWidth),
                (c(Key::Num1), Act::Actual),
                (c(Key::Equals), Act::ZoomIn),
                (c(Key::Minus), Act::ZoomOut),
            ] {
                if i.consume_shortcut(&sc) {
                    acts.push(a);
                }
            }
        });
        for a in acts {
            self.act(ctx, a);
        }
        if ctx.egui_wants_keyboard_input() || self.tabs.is_empty() {
            return;
        }
        let (down, up, home, end) = ctx.input(|i| {
            (
                i.key_pressed(Key::PageDown),
                i.key_pressed(Key::PageUp),
                i.key_pressed(Key::Home),
                i.key_pressed(Key::End),
            )
        });
        let t = &mut self.tabs[self.active];
        let last = t.page_count().saturating_sub(1);
        if down {
            t.go_to((t.current + 1).min(last));
        }
        if up {
            t.go_to(t.current.saturating_sub(1));
        }
        if home {
            t.go_to(0);
        }
        if end {
            t.go_to(last);
        }
    }
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        self.poll_loads(&ctx);

        // Files dropped onto the window.
        let dropped: Vec<PathBuf> = ctx.input(|i| {
            i.raw
                .dropped_files
                .iter()
                .map(|f| f.path().to_path_buf())
                .collect()
        });
        let (mut images, pdfs): (Vec<_>, Vec<_>) = dropped.into_iter().partition(|p| is_image(p));
        for p in pdfs {
            self.spawn_open(&ctx, p, None, None);
        }
        if !images.is_empty() {
            images.sort();
            self.new_from_images(&ctx, &images);
        }

        // Guard against closing the window with unsaved work.
        if ctx.input(|i| i.viewport().close_requested())
            && !self.quit_ok
            && self.tabs.iter().any(|t| t.dirty)
        {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            self.dialog = Dialog::ConfirmQuit;
        }

        self.shortcuts(&ctx);

        let mut acts: Vec<Act> = Vec::new();
        let has = !self.tabs.is_empty();
        let (can_undo, can_redo) = self
            .tabs
            .get(self.active)
            .map(|t| (t.can_undo(), t.can_redo()))
            .unwrap_or((false, false));
        let can_copy = self
            .tabs
            .get(self.active)
            .is_some_and(|t| t.ed.selected_text().is_some());

        egui::Panel::top("menu").show(ui, |ui| {
            egui::MenuBar::new().ui(ui, |ui| {
                let mut item =
                    |ui: &mut egui::Ui, label: &str, key: &str, enabled: bool, act: Act| {
                        let b = egui::Button::new(label).shortcut_text(key);
                        if ui.add_enabled(enabled, b).clicked() {
                            acts.push(act);
                            ui.close();
                        }
                    };
                ui.menu_button("File", |ui| {
                    item(ui, "Open…", "Ctrl+O", true, Act::Open);
                    item(ui, "New from images…", "", true, Act::NewFromImages);
                    item(ui, "Save", "Ctrl+S", has, Act::Save);
                    item(ui, "Save As…", "Ctrl+Shift+S", has, Act::SaveAs);
                    item(ui, "Close tab", "Ctrl+W", has, Act::CloseTab);
                    ui.separator();
                    item(ui, "Quit", "", true, Act::Quit);
                });
                ui.menu_button("Edit", |ui| {
                    item(ui, "Undo", "Ctrl+Z", can_undo, Act::Undo);
                    item(ui, "Redo", "Ctrl+Y", can_redo, Act::Redo);
                    ui.separator();
                    item(ui, "Copy text", "Ctrl+C", can_copy, Act::Copy);
                    item(ui, "Find…", "Ctrl+F", has, Act::Find);
                });
                ui.menu_button("View", |ui| {
                    item(ui, "Zoom in", "Ctrl+=", has, Act::ZoomIn);
                    item(ui, "Zoom out", "Ctrl+-", has, Act::ZoomOut);
                    item(ui, "Fit width", "Ctrl+0", has, Act::FitWidth);
                    item(ui, "Actual size", "Ctrl+1", has, Act::Actual);
                    ui.separator();
                    ui.checkbox(&mut self.show_side, "Side panel");
                    if let Some(t) = self.tabs.get_mut(self.active) {
                        ui.checkbox(&mut t.ed.show_hidden, "Show recognised text")
                            .on_hover_text(
                                "Outline the invisible text added by OCR. Hover to read it; \
                                 select it with the arrow tool to correct it.",
                            );
                    }
                    ui.menu_button("Theme", |ui| {
                        item(
                            ui,
                            "System",
                            "",
                            true,
                            Act::Theme(egui::ThemePreference::System),
                        );
                        item(
                            ui,
                            "Light",
                            "",
                            true,
                            Act::Theme(egui::ThemePreference::Light),
                        );
                        item(
                            ui,
                            "Dark",
                            "",
                            true,
                            Act::Theme(egui::ThemePreference::Dark),
                        );
                    });
                });
                ui.menu_button("Document", |ui| {
                    item(ui, "Rotate clockwise", "", has, Act::RotR);
                    item(ui, "Rotate counter-clockwise", "", has, Act::RotL);
                    item(ui, "Delete pages…", "", has, Act::Delete);
                    item(ui, "Extract pages to new tab…", "", has, Act::Extract);
                    item(ui, "Insert blank page…", "", has, Act::Blank);
                    item(ui, "Insert images…", "", has, Act::InsertImages);
                    let idle = has && !self.busy;
                    item(ui, "Optimise scanned pages…", "", idle, Act::Scans);
                    item(ui, "Recognise text (OCR)…", "", idle, Act::Ocr);
                    item(ui, "Append PDFs…", "", has, Act::Merge);
                    item(ui, "Split into files…", "", has, Act::Split);
                    ui.separator();
                    item(ui, "Headers, footers, Bates…", "", has, Act::HeaderFooter);
                    item(ui, "Watermark…", "", has, Act::Watermark);
                    ui.separator();
                    item(ui, "Properties…", "", has, Act::Props);
                    item(ui, "Password security…", "", has, Act::Security);
                });
                ui.menu_button("Help", |ui| item(ui, "About", "", true, Act::About));
            });
        });

        if has {
            egui::Panel::top("toolbar").show(ui, |ui| {
                ui.horizontal(|ui| {
                    let icon_btn = |ui: &mut egui::Ui,
                                        glyph: &str,
                                        tip: &str,
                                        on: bool,
                                        enabled: bool|
                     -> bool {
                        let b = egui::Button::new(egui::RichText::new(glyph).size(18.0))
                            .selected(on)
                            .min_size(egui::vec2(30.0, 28.0));
                        ui.add_enabled(enabled, b).on_hover_text(tip).clicked()
                    };
                    if icon_btn(ui, icon::FOLDER_OPEN, "Open (Ctrl+O)", false, true) {
                        acts.push(Act::Open);
                    }
                    if icon_btn(ui, icon::FLOPPY_DISK, "Save (Ctrl+S)", false, true) {
                        acts.push(Act::Save);
                    }
                    if icon_btn(
                        ui,
                        icon::ARROW_COUNTER_CLOCKWISE,
                        "Undo (Ctrl+Z)",
                        false,
                        can_undo,
                    ) {
                        acts.push(Act::Undo);
                    }
                    if icon_btn(ui, icon::ARROW_CLOCKWISE, "Redo (Ctrl+Y)", false, can_redo) {
                        acts.push(Act::Redo);
                    }
                    ui.separator();
                    if icon_btn(
                        ui,
                        icon::MAGNIFYING_GLASS_MINUS,
                        "Zoom out (Ctrl+-)",
                        false,
                        true,
                    ) {
                        acts.push(Act::ZoomOut);
                    }
                    if icon_btn(
                        ui,
                        icon::MAGNIFYING_GLASS_PLUS,
                        "Zoom in (Ctrl+=)",
                        false,
                        true,
                    ) {
                        acts.push(Act::ZoomIn);
                    }
                    if icon_btn(ui, icon::ARROWS_OUT, "Fit width (Ctrl+0)", false, true) {
                        acts.push(Act::FitWidth);
                    }
                    ui.separator();
                    let t = &mut self.tabs[self.active];
                    let cur = t.ed.tool;
                    for (tool, glyph, tip) in [
                        (Tool::View, icon::HAND, "Read only (no editing)"),
                        (
                            Tool::Select,
                            icon::CURSOR,
                            "Select, move and resize objects",
                        ),
                        (Tool::SelectText, icon::CURSOR_TEXT, "Select and copy text"),
                        (Tool::Text, icon::TEXT_T, "Add text"),
                        (Tool::Rect, icon::SQUARE, "Add rectangle"),
                        (Tool::Ellipse, icon::CIRCLE, "Add ellipse"),
                        (Tool::Line, icon::LINE_SEGMENT, "Add line"),
                    ] {
                        if icon_btn(ui, glyph, tip, cur == tool, true) {
                            t.ed.set_tool(tool);
                        }
                    }
                    if icon_btn(ui, icon::IMAGE, "Add image…", cur == Tool::Image, true) {
                        t.choose_image();
                    }
                });
            });
        }

        if has {
            egui::Panel::top("tabs").show(ui, |ui| {
                ui.horizontal_wrapped(|ui| {
                    let mut close = None;
                    for i in 0..self.tabs.len() {
                        if ui
                            .selectable_label(i == self.active, self.tabs[i].title())
                            .clicked()
                        {
                            self.active = i;
                        }
                        if ui.small_button("×").clicked() {
                            close = Some(i);
                        }
                        ui.add_space(6.0);
                    }
                    if let Some(i) = close {
                        self.active = i;
                        acts.push(Act::CloseTab);
                    }
                });
            });
        }

        egui::Panel::bottom("status").show(ui, |ui| {
            ui.horizontal(|ui| {
                if let Some(t) = self.tabs.get(self.active) {
                    ui.label(format!("Page {} of {}", t.current + 1, t.page_count()));
                    ui.separator();
                    ui.label(format!("{:.0}%", t.zoom * 100.0));
                    if t.encryption.is_some() {
                        ui.separator();
                        ui.label("will be encrypted on save");
                    }
                }
                if !self.status.is_empty() {
                    ui.separator();
                    ui.label(&self.status);
                }
            });
        });

        if has && self.show_side {
            let active = self.active;
            egui::Panel::left("side")
                .resizable(true)
                .default_size(170.0)
                .show(ui, |ui| {
                    let t = &mut self.tabs[active];
                    ui.horizontal(|ui| {
                        ui.selectable_value(&mut t.side, Side::Pages, "Pages");
                        ui.selectable_value(&mut t.side, Side::Search, "Search");
                    });
                    ui.separator();
                    match t.side {
                        Side::Pages => t.show_thumbs(ui),
                        Side::Search => {
                            let r = ui.text_edit_singleline(&mut t.query);
                            let enter = r.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter));
                            ui.horizontal(|ui| {
                                if ui.button("Find").clicked() || enter {
                                    t.run_search();
                                }
                                if ui.button("◀").clicked() {
                                    t.step_hit(-1);
                                }
                                if ui.button("▶").clicked() {
                                    t.step_hit(1);
                                }
                            });
                            ui.label(format!("{} match(es)", t.hits.len()));
                            let mut jump = None;
                            egui::ScrollArea::vertical().show(ui, |ui| {
                                for (i, h) in t.hits.iter().enumerate() {
                                    let text = format!("p{}  {}", h.page + 1, h.snippet);
                                    if ui.selectable_label(i == t.hit_idx, text).clicked() {
                                        jump = Some(i);
                                    }
                                }
                            });
                            if let Some(i) = jump {
                                t.hit_idx = i;
                                let p = t.hits[i].page;
                                t.go_to(p);
                            }
                        }
                    }
                });
        }

        if has && self.tabs[self.active].ed.tool != Tool::View {
            let active = self.active;
            egui::Panel::right("inspector")
                .resizable(true)
                .default_size(240.0)
                .show(ui, |ui| {
                    egui::ScrollArea::vertical()
                        .auto_shrink([false, false])
                        .show(ui, |ui| self.tabs[active].ed.inspector(ui));
                });
        }

        egui::CentralPanel::default().show(ui, |ui| {
            if let Some(t) = self.tabs.get_mut(self.active) {
                t.show_pages(ui);
            } else {
                ui.centered_and_justified(|ui| {
                    ui.label("Open a PDF with File ▸ Open, Ctrl+O, or drop a file here.");
                });
            }
        });

        // Surface errors raised by tab operations.
        if let Some(t) = self.tabs.get_mut(self.active) {
            if let Some(e) = t.error.take() {
                self.dialog = Dialog::Message("Error".into(), e);
            }
        }

        for a in acts {
            self.act(&ctx, a);
        }
        self.dialogs(&ctx);

        let title = match self.tabs.get(self.active) {
            Some(t) => format!("{} - MPE", t.title()),
            None => "MPE".into(),
        };
        if title != self.last_title {
            ctx.send_viewport_cmd(egui::ViewportCommand::Title(title.clone()));
            self.last_title = title;
        }
    }
}

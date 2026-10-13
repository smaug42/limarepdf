use crate::actions::{self, Act, Action, Ctx, Group, ACTIONS};
use crate::dialogs::{self as dlg, Outcome};
use crate::editor::Tool;
use crate::tab::{Side, Tab};
use crate::theme;
use eframe::egui::{self, Key};
use egui_phosphor::regular as icon;
use mpe_core::render::{image_names, ImageFormat};
use mpe_core::{
    parse_page_range, print, Bates, Encryption, HeaderFooter, ImageImport, OcrResult, Pdf,
    ScanResult, Watermark,
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

/// Page images to write, once the folder is chosen.
#[derive(Clone)]
struct ImageExport {
    pages: Vec<usize>,
    dpi: f32,
    format: ImageFormat,
    dir: PathBuf,
    stem: String,
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
    Export(dlg::Export),
    Password(dlg::Password),
    Message(String, String),
    ConfirmClose(usize),
    ConfirmQuit,
    /// Export would replace this many files.
    ConfirmReplace(ImageExport, usize),
    Print(dlg::Print),
    About,
}

/// The command palette (Ctrl+K) while it is open.
#[derive(Default)]
struct Palette {
    query: String,
    /// Highlighted row among the matches.
    row: usize,
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
    /// Put the keyboard focus in the top bar's search field (Ctrl+F).
    focus_search: bool,
    palette: Option<Palette>,
}

impl App {
    pub fn new(cc: &eframe::CreationContext<'_>, files: Vec<PathBuf>, page: Option<usize>) -> Self {
        theme::install(&cc.egui_ctx);
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
            focus_search: false,
            palette: None,
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
                    if changed && t.revision != revision {
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

    /// Write page images on a background thread; the document is not changed.
    fn start_export(&mut self, ctx: &egui::Context, e: ImageExport) {
        self.spawn_job(ctx, "Export pages as images", move |p| {
            let files = p.export_images(&e.pages, e.dpi, e.format, &e.dir, &e.stem)?;
            let n = files.len();
            Ok((false, format!("Wrote {n} image(s) to {}.", e.dir.display())))
        });
    }

    /// Write the text of every page to a file, pages separated by form feeds.
    fn export_text(&mut self) {
        let t = &self.tabs[self.active];
        let all: Vec<usize> = (0..t.page_count()).collect();
        let text = t.pdf.pages_text(&all);
        if text.trim_matches(|c: char| c.is_whitespace()).is_empty() {
            self.dialog = Dialog::Message(
                "Export text".into(),
                "This document has no text to export. Scanned pages need \
                 Document > Recognise text (OCR) first."
                    .into(),
            );
            return;
        }
        let Some(mut path) = rfd::FileDialog::new()
            .add_filter("Text", &["txt"])
            .set_file_name(format!("{}.txt", t.stem("document")))
            .save_file()
        else {
            return;
        };
        if path.extension().is_none() {
            path.set_extension("txt");
        }
        match std::fs::write(&path, text) {
            Ok(()) => self.status = format!("Wrote the text to {}", path.display()),
            Err(e) => {
                self.dialog =
                    Dialog::Message("Export failed".into(), format!("{}: {e}", path.display()))
            }
        }
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
            Act::ExportImages if has => self.dialog = Dialog::Export(dlg::Export::default()),
            Act::ExportText if has => self.export_text(),
            Act::Print if has && !self.busy => {
                self.dialog = match print::printers() {
                    Ok(list) => Dialog::Print(dlg::Print::new(list)),
                    Err(e) => Dialog::Message("Print".into(), format!("{e:#}")),
                }
            }
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
            Act::ToggleSide => self.show_side = !self.show_side,
            Act::Palette => self.palette = Some(Palette::default()),
            _ if !has => {}
            Act::ToggleHidden => {
                let ed = &mut self.tabs[self.active].ed;
                ed.show_hidden = !ed.show_hidden;
            }
            Act::Tool(Tool::Image) => self.tabs[self.active].choose_image(),
            Act::Tool(tool) => self.tabs[self.active].ed.set_tool(tool),
            Act::SelectAllText => {
                let t = &mut self.tabs[self.active];
                t.ed.select_all_words(&t.pdf, t.current);
            }
            Act::DeleteObject => self.tabs[self.active].ed.delete_selected(),
            Act::Nudge(dx, dy) => {
                let shift = ctx.input(|i| i.modifiers.shift);
                let step = if shift { 10.0 } else { 1.0 };
                let (dx, dy) = (dx as f32 * step, dy as f32 * step);
                self.tabs[self.active].ed.nudge(dx, dy);
            }
            Act::Escape => self.tabs[self.active].ed.escape(),
            Act::NextPage | Act::PrevPage | Act::FirstPage | Act::LastPage => {
                let t = &mut self.tabs[self.active];
                let last = t.page_count().saturating_sub(1);
                let to = match a {
                    Act::NextPage => (t.current + 1).min(last),
                    Act::PrevPage => t.current.saturating_sub(1),
                    Act::FirstPage => 0,
                    _ => last,
                };
                t.go_to(to);
            }
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
                self.focus_search = true;
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
                    "MPE - a fast, pure-Rust PDF viewer and editor.\nLicensed under MIT OR Apache-2.0.\nRendering: hayro. Document engine: lopdf. UI: egui.\n\nOCR uses the ocrs text recognition model by Robert Knight\n(huggingface.co/robertknight/ocrs), licensed CC BY-SA 4.0\n(creativecommons.org/licenses/by-sa/4.0), included unmodified.\n\nText font: Inter by The Inter Project Authors (rsms.me/inter),\nlicensed under the SIL Open Font License 1.1, included unmodified.",
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
            Dialog::Export(s) => match dlg::export(ctx, s) {
                Outcome::Apply => {
                    let t = &mut self.tabs[self.active];
                    match parse_page_range(&s.pages, t.page_count()) {
                        Ok(pages) => {
                            // Cancelling the folder choice leaves the dialog open.
                            if let Some(dir) = rfd::FileDialog::new()
                                .set_title("Choose a folder for the images")
                                .pick_folder()
                            {
                                let e = ImageExport {
                                    stem: t.stem("page"),
                                    format: s.format(),
                                    dpi: s.dpi,
                                    pages,
                                    dir,
                                };
                                let existing = image_names(&e.pages, &e.stem, e.format)
                                    .iter()
                                    .filter(|n| e.dir.join(n).exists())
                                    .count();
                                if existing > 0 {
                                    self.dialog = Dialog::ConfirmReplace(e, existing);
                                } else {
                                    self.start_export(ctx, e);
                                }
                                done = true;
                            }
                        }
                        Err(e) => t.error = Some(e.to_string()),
                    }
                }
                Outcome::Cancel => done = true,
                Outcome::Open => {}
            },
            Dialog::ConfirmReplace(e, n) => match dlg::confirm(
                ctx,
                "Replace files?",
                &format!(
                    "{n} file(s) in {} already have the names these images would get. \
                     Replace them?",
                    e.dir.display()
                ),
                "Replace",
            ) {
                Outcome::Apply => {
                    self.start_export(ctx, e.clone());
                    done = true;
                }
                Outcome::Cancel => done = true,
                Outcome::Open => {}
            },
            Dialog::Print(s) => match dlg::print(ctx, s) {
                Outcome::Apply if s.printers.is_empty() => done = true,
                Outcome::Apply => {
                    let t = &mut self.tabs[self.active];
                    match parse_page_range(&s.pages, t.page_count()) {
                        Ok(pages) => {
                            let printer = s.printers[s.printer].name.clone();
                            let opts = print::PrintOptions {
                                printer: Some(printer.clone()),
                                copies: s.copies,
                                sides: s.sides,
                                as_image: s.as_image,
                                title: t.stem("Untitled"),
                            };
                            self.spawn_job(ctx, "Print", move |p| {
                                let reply = p.print(&pages, &opts)?;
                                let n = pages.len();
                                Ok((false, format!("Sent {n} page(s) to {printer}.\n{reply}")))
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
                let stem = t.stem("part");
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

    /// The state the commands' conditions are checked against.
    fn state(&self, ctx: &egui::Context) -> Ctx {
        let t = self.tabs.get(self.active);
        Ctx {
            doc: t.is_some(),
            busy: self.busy,
            can_undo: t.is_some_and(|t| t.can_undo()),
            can_redo: t.is_some_and(|t| t.can_redo()),
            words: t.is_some_and(|t| t.ed.selected_text().is_some()),
            object: t.is_some_and(|t| t.ed.has_object()),
            tool: t.map(|t| t.ed.tool),
            side: self.show_side,
            hidden_text: t.is_some_and(|t| t.ed.show_hidden),
            theme: ctx.options(|o| o.theme_preference),
        }
    }

    /// The menu: one submenu per group, built from the registry.
    fn menu(ui: &mut egui::Ui, c: &Ctx, acts: &mut Vec<Act>) {
        for g in Group::MENUS {
            ui.menu_button(g.name(), |ui| {
                let items: Vec<&Action> =
                    ACTIONS.iter().filter(|a| a.group == g && a.menu).collect();
                let mut i = 0;
                while i < items.len() {
                    let a = items[i];
                    if a.sep && i > 0 {
                        ui.separator();
                    }
                    if a.submenu.is_empty() {
                        menu_item(ui, a, c, acts);
                        i += 1;
                        continue;
                    }
                    // A run of items with the same submenu becomes one submenu.
                    let n = items[i..]
                        .iter()
                        .take_while(|b| b.submenu == a.submenu)
                        .count();
                    ui.menu_button(a.submenu, |ui| {
                        for b in &items[i..i + n] {
                            menu_item(ui, b, c, acts);
                        }
                    });
                    i += n;
                }
            });
        }
    }

    /// The command palette (Ctrl+K): type to filter the commands, Up and Down to move,
    /// Enter or a click to run, Esc or a click outside to close.
    fn palette_ui(&mut self, ctx: &egui::Context, c: &Ctx, acts: &mut Vec<Act>) {
        let Some(p) = self.palette.as_mut() else {
            return;
        };
        let items: Vec<&Action> = ACTIONS
            .iter()
            .filter(|a| a.palette && actions::matches(a, &p.query))
            .collect();
        let (up, down, enter) = ctx.input_mut(|i| {
            let none = egui::Modifiers::NONE;
            (
                i.consume_key(none, Key::ArrowUp),
                i.consume_key(none, Key::ArrowDown),
                i.consume_key(none, Key::Enter),
            )
        });
        let last = items.len().saturating_sub(1);
        if up {
            p.row = p.row.saturating_sub(1);
        }
        if down {
            p.row = (p.row + 1).min(last);
        }
        p.row = p.row.min(last);
        let mut run = None;
        let modal = egui::Modal::new(egui::Id::new("palette")).show(ctx, |ui| {
            ui.set_width(480.0);
            let field = egui::TextEdit::singleline(&mut p.query)
                .hint_text("Type a command")
                .desired_width(f32::INFINITY);
            let r = ui.add(field);
            r.request_focus();
            if r.changed() {
                p.row = 0;
            }
            ui.separator();
            let list = egui::ScrollArea::vertical().max_height(360.0);
            list.show(ui, |ui| {
                if items.is_empty() {
                    ui.weak("No matching command.");
                }
                for (n, a) in items.iter().enumerate() {
                    let on = n == p.row;
                    let r = palette_row(ui, a, n, on, (a.enabled)(c));
                    if on && (up || down) {
                        r.scroll_to_me(None);
                    }
                    if r.clicked() {
                        run = Some(a.act);
                    }
                }
            });
        });
        if enter {
            if let Some(a) = items.get(p.row).filter(|a| (a.enabled)(c)) {
                run = Some(a.act);
            }
        }
        if run.is_some() || modal.should_close() {
            self.palette = None;
        }
        acts.extend(run);
    }

    /// Keyboard shortcuts, from the registry. Ctrl/Cmd shortcuts work anywhere and are
    /// consumed. Plain keys (Delete, arrows, Esc, PageUp…), Ctrl+A and Ctrl+C act only
    /// when no text field has focus, so they never take over typing, and are not
    /// consumed, as before (Esc also closes menus). Nothing runs while the palette is
    /// open; it handles its own keys.
    fn shortcuts(&mut self, ctx: &egui::Context) {
        if self.palette.is_some() {
            return;
        }
        let c = self.state(ctx);
        let typing = ctx.egui_wants_keyboard_input();
        let mut acts = Vec::new();
        ctx.input_mut(|i| {
            for (a, k) in actions::shortcuts() {
                let page_only = matches!(a.act, Act::Copy | Act::SelectAllText);
                let global = k.modifiers.command && !page_only;
                if (typing && !global) || !(a.enabled)(&c) {
                    continue;
                }
                let hit = if a.act == Act::Copy {
                    // Ctrl+C arrives as a Copy event, not as a key press.
                    i.events.iter().any(|e| matches!(e, egui::Event::Copy))
                } else if global {
                    i.consume_shortcut(&k)
                } else {
                    i.modifiers.matches_logically(k.modifiers) && i.key_pressed(k.logical_key)
                };
                if hit {
                    acts.push(a.act);
                }
            }
        });
        for a in acts {
            self.act(ctx, a);
        }
    }
}

/// One menu entry: a checkbox for on/off commands, else a button with its shortcut.
fn menu_item(ui: &mut egui::Ui, a: &Action, c: &Ctx, acts: &mut Vec<Act>) {
    let enabled = (a.enabled)(c);
    let r = match a.checked {
        Some(on) => {
            let mut v = on(c);
            ui.add_enabled(enabled, egui::Checkbox::new(&mut v, a.label))
        }
        None => {
            let key = a.keys.first().map(|k| ui.ctx().format_shortcut(k));
            let b = egui::Button::new(a.label).shortcut_text(key.unwrap_or_default());
            ui.add_enabled(enabled, b)
        }
    };
    let r = if a.hint.is_empty() {
        r
    } else {
        r.on_hover_text(a.hint)
    };
    if r.clicked() {
        acts.push(a.act);
        if a.checked.is_none() {
            ui.close();
        }
    }
}

/// One row of the command palette: icon, label, then group and shortcut on the right.
/// `n` is the row number; `on` highlights it.
fn palette_row(ui: &mut egui::Ui, a: &Action, n: usize, on: bool, enabled: bool) -> egui::Response {
    let glyph = if a.icon.is_empty() {
        // Keeps labels aligned with the rows that have an icon.
        egui::Atom::custom(egui::Id::new(("palette-icon", n)), [18.0, 18.0])
    } else {
        theme::icon(a.icon).size(16.0).into()
    };
    let right = match a.keys.first() {
        Some(k) => format!("{}    {}", a.group.name(), ui.ctx().format_shortcut(k)),
        None => a.group.name().to_string(),
    };
    let b = egui::Button::new((glyph, a.label))
        .shortcut_text(right)
        .selected(on)
        .frame_when_inactive(on)
        .min_size(egui::vec2(ui.available_width(), 28.0));
    ui.add_enabled(enabled, b)
}

/// The window with no document: a card to open a PDF or start from images. The card
/// is a drop target and is clickable as a whole; it lights up while files are dragged
/// over the window.
fn empty_state(ui: &mut egui::Ui, acts: &mut Vec<Act>) {
    let dragging = ui.input(|i| !i.raw.hovered_files.is_empty());
    let room = ui.max_rect();
    let size = egui::vec2(440.0, 280.0).min(room.size() - egui::vec2(32.0, 32.0));
    let card = egui::Rect::from_center_size(room.center(), size.max(egui::vec2(1.0, 1.0)));
    let id = egui::Id::new("empty-card");
    let r = ui
        .interact(card, id, egui::Sense::click())
        .on_hover_cursor(egui::CursorIcon::PointingHand);
    let v = ui.visuals();
    let accent = theme::accent(v.dark_mode);
    let stroke = if dragging || r.hovered() {
        egui::Stroke::new(2.0, accent)
    } else {
        egui::Stroke::new(1.0, v.widgets.noninteractive.bg_stroke.color)
    };
    let inside = egui::StrokeKind::Inside;
    ui.painter().rect(card, 12, v.panel_fill, stroke, inside);

    let layout = egui::Layout::top_down(egui::Align::Center);
    let inner = card.shrink(24.0);
    let mut ui = ui.new_child(egui::UiBuilder::new().max_rect(inner).layout(layout));
    ui.add_space(8.0);
    ui.label(theme::icon(icon::FILE_PDF).size(44.0).color(accent));
    ui.add_space(8.0);
    let heading = if dragging {
        "Drop to open"
    } else {
        "Open a PDF to get started"
    };
    ui.heading(heading);
    ui.add_space(16.0);
    let open = theme::primary_button("Open PDF…").min_size(egui::vec2(180.0, 32.0));
    if ui.add(open).clicked() {
        acts.push(Act::Open);
    }
    ui.add_space(4.0);
    let images = egui::Button::new("New from images…").min_size(egui::vec2(180.0, 28.0));
    if ui.add(images).clicked() {
        acts.push(Act::NewFromImages);
    }
    ui.add_space(16.0);
    ui.weak("or drop PDF and image files anywhere in this window");
    let keys = |a: Act| {
        let k = actions::get(a).keys[0];
        ui.ctx().format_shortcut(&k)
    };
    ui.weak(format!(
        "{} to open  ·  {} for all commands",
        keys(Act::Open),
        keys(Act::Palette)
    ));
    if r.clicked() {
        acts.push(Act::Open);
    }
}

/// Tooltip for a command: its label and first shortcut.
fn tooltip(ctx: &egui::Context, a: &Action) -> String {
    match a.keys.first() {
        Some(k) => format!("{} ({})", a.label, ctx.format_shortcut(k)),
        None => a.label.to_string(),
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
        let c = self.state(&ctx);

        // A top-bar button for a command: its icon, no frame until hovered except when
        // `on` (the active tool), its label and shortcut as the tooltip.
        let bar = |ui: &mut egui::Ui, act: Act, on: bool, acts: &mut Vec<Act>| {
            let a = actions::get(act);
            let b = egui::Button::new(theme::icon(a.icon).size(18.0))
                .selected(on)
                .frame_when_inactive(on)
                .min_size(egui::vec2(28.0, 28.0));
            let tip = tooltip(ui.ctx(), a);
            let r = ui.add_enabled((a.enabled)(&c), b);
            if r.on_hover_text(tip).clicked() {
                acts.push(act);
            }
        };
        let menu_icon = |glyph: &str| theme::icon(glyph).size(18.0);

        egui::Panel::top("topbar")
            .frame(theme::bar_frame(ui.style()))
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.menu_button(menu_icon(icon::LIST), |ui| {
                        Self::menu(ui, &c, &mut acts);
                    })
                    .response
                    .on_hover_text("Menu");
                    bar(ui, Act::Open, false, &mut acts);

                    let title = match self.tabs.get(self.active) {
                        Some(t) => t.title(),
                        None => "MPE".into(),
                    };
                    let title = egui::RichText::new(title).family(theme::semibold());
                    ui.allocate_ui(egui::vec2(240.0, 28.0), |ui| {
                        ui.add(egui::Label::new(title).truncate())
                    });
                    if !has {
                        return;
                    }
                    bar(ui, Act::Save, false, &mut acts);
                    ui.separator();
                    bar(ui, Act::Undo, false, &mut acts);
                    bar(ui, Act::Redo, false, &mut acts);
                    ui.separator();
                    let wide = ui.available_width() > 640.0;
                    for a in ACTIONS.iter().filter(|a| a.group == Group::Tools) {
                        let on = matches!(a.act, Act::Tool(t) if c.tool == Some(t));
                        bar(ui, a.act, on, &mut acts);
                    }

                    // Right-hand side, laid out from the right edge.
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.menu_button(menu_icon(icon::EXPORT), |ui| {
                            for act in [Act::ExportImages, Act::ExportText, Act::Print] {
                                menu_item(ui, actions::get(act), &c, &mut acts);
                            }
                        })
                        .response
                        .on_hover_text("Export and print");
                        let t = &mut self.tabs[self.active];
                        if wide {
                            let r = ui.add(
                                egui::TextEdit::singleline(&mut t.query)
                                    .hint_text("Search")
                                    .desired_width(180.0),
                            );
                            // Laid out right to left, so this lands left of the field.
                            ui.label(theme::icon(icon::MAGNIFYING_GLASS).weak());
                            if std::mem::take(&mut self.focus_search) {
                                r.request_focus();
                            }
                            if r.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter)) {
                                t.run_search();
                                t.side = Side::Search;
                                self.show_side = true;
                            }
                        } else {
                            bar(ui, Act::Find, false, &mut acts);
                        }
                        ui.separator();
                        bar(ui, Act::FitWidth, false, &mut acts);
                        bar(ui, Act::ZoomIn, false, &mut acts);
                        // The zoom level; clicking it resets to actual size.
                        let pct = format!("{:.0}%", t.zoom * 100.0);
                        let b = egui::Button::new(pct).frame_when_inactive(false);
                        let tip = tooltip(ui.ctx(), actions::get(Act::Actual));
                        if ui.add(b).on_hover_text(tip).clicked() {
                            acts.push(Act::Actual);
                        }
                        bar(ui, Act::ZoomOut, false, &mut acts);
                    });
                });
            });

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

        let canvas = egui::Frame::central_panel(ui.style())
            .fill(theme::canvas(ui.visuals().dark_mode))
            .inner_margin(0);
        egui::CentralPanel::default().frame(canvas).show(ui, |ui| {
            if let Some(t) = self.tabs.get_mut(self.active) {
                t.show_pages(ui);
            } else {
                empty_state(ui, &mut acts);
            }
        });

        // Surface errors raised by tab operations.
        if let Some(t) = self.tabs.get_mut(self.active) {
            if let Some(e) = t.error.take() {
                self.dialog = Dialog::Message("Error".into(), e);
            }
        }

        self.palette_ui(&ctx, &c, &mut acts);
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

use crate::actions::{self, menu_item, tooltip, Act, Action, Ctx, Group, ACTIONS};
use crate::banners::Kind as BannerKind;
use crate::dialogs::{self as dlg, Outcome};
use crate::editor::{Action as EdAction, Tool};
use crate::inspector::{self, Apply, Section};
use crate::markup;
use crate::recent::Recent;
use crate::selection::Selection;
use crate::settings::Settings;
use crate::tab::{Side, Tab};
use crate::theme;
use eframe::egui::{self, AtomExt as _, Key};
use egui_phosphor::regular as icon;
use mpe_core::render::{image_names, ImageFormat};
use mpe_core::{
    parse_page_range, print, Bates, HeaderFooter, ImageImport, OcrResult, Pdf, ScanResult,
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

/// What to show for a clicked link to an address outside the document. Only web and
/// email addresses are offered for opening; anything else (a local file, a script) is
/// shown but not opened.
fn link_dialog(url: String) -> Dialog {
    let scheme = url.split_once(':').map(|(s, _)| s.to_ascii_lowercase());
    if matches!(scheme.as_deref(), Some("http" | "https" | "mailto")) {
        Dialog::OpenLink(url)
    } else {
        Dialog::Message(
            "Link not opened".into(),
            format!("This link goes to:\n\n{url}\n\nMPE only opens web and email links."),
        )
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

/// The dialogs left: settings live in the inspector.
enum Dialog {
    None,
    Password(dlg::Password),
    Message(String, String),
    ConfirmClose(usize),
    ConfirmQuit,
    /// Export would replace this many files.
    ConfirmReplace(ImageExport, usize),
    ConfirmFlatten,
    ConfirmFlattenAnnots,
    /// Save (`true`: as a new file) although it breaks the document's signatures?
    ConfirmSaveSigned(bool),
    Print(dlg::Print),
    SaveSelection(dlg::Layout),
    /// Open this web or email address from a link?
    OpenLink(String),
    /// The name for comments, being edited.
    CommentName(String),
    About,
}

/// What selected text is printed or saved from, taken from the tab so a background job
/// can build the new document.
enum Excerpt {
    /// Pages cropped to these rectangles.
    Pages(Vec<(usize, [f32; 4])>),
    /// The words, on pages of this size.
    Text(String, (f32, f32)),
}
impl Excerpt {
    fn new(t: &Tab, layout: dlg::Layout) -> Self {
        let rects = t.ed.selected_rects();
        match layout {
            // The full page width, margins included; `excerpt` clips to the page.
            dlg::Layout::Original => Excerpt::Pages(
                rects
                    .into_iter()
                    .map(|(p, r)| (p, [0.0, r[1], f32::MAX, r[3]]))
                    .collect(),
            ),
            _ => {
                let page = rects.first().map_or(0, |r| r.0);
                let size = t.pdf.page_size(page).unwrap_or((612.0, 792.0));
                Excerpt::Text(t.ed.selected_text().unwrap_or_default(), size)
            }
        }
    }

    fn build(&self, p: &Pdf) -> mpe_core::Result<Pdf> {
        match self {
            Excerpt::Pages(rects) => p.excerpt(rects),
            Excerpt::Text(text, size) => Pdf::from_text(text, *size),
        }
    }
}

/// Title of the OCR background job, also used to recognise its result.
const OCR_JOB: &str = "Recognise text";

/// Width of the inspector panel, and of its rail when collapsed.
const INSPECTOR_WIDTH: f32 = 250.0;
const RAIL_WIDTH: f32 = 36.0;
/// Below this window width the inspector collapses to its rail by itself.
const NARROW_WIDTH: f32 = 960.0;

/// How long the undo toast stays, in seconds.
const TOAST_SECONDS: f64 = 6.0;

/// A brief notice at the bottom of the window with an Undo button.
struct Toast {
    text: String,
    /// When it goes away (egui input time).
    until: f64,
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
    /// The inspector is open (else collapsed to its rail).
    show_inspector: bool,
    /// Whether the user wants it open when the window is wide enough.
    inspector_pref: bool,
    /// The window was narrower than `NARROW_WIDTH` last frame.
    narrow: bool,
    toast: Option<Toast>,
    /// Recently opened files, kept between runs.
    recent: Recent,
    /// Settings kept between runs (the theme).
    settings: Settings,
}

impl App {
    pub fn new(cc: &eframe::CreationContext<'_>, files: Vec<PathBuf>, page: Option<usize>) -> Self {
        theme::install(&cc.egui_ctx);
        let settings = Settings::load();
        cc.egui_ctx.set_theme(settings.theme);
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
            show_inspector: true,
            inspector_pref: true,
            narrow: false,
            toast: None,
            recent: Recent::load(),
            settings,
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
                Loaded::Ok { path, pdf, page } => {
                    self.recent.add(&path);
                    self.add_tab(ctx, pdf, Some(path), page, false);
                }
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
                        // Text was recognised: the scan banner has done its job.
                        if title == OCR_JOB {
                            t.banners.retain(|b| b.kind != BannerKind::Scanned);
                        }
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

    /// Save the selected text as cropped pages, as plain text in a PDF, or as a text file.
    fn save_selection(&mut self, layout: dlg::Layout) {
        let t = &self.tabs[self.active];
        let Some(text) = t.ed.selected_text() else {
            return;
        };
        let (filter, ext) = match layout {
            dlg::Layout::TextFile => ("Text", "txt"),
            _ => ("PDF", "pdf"),
        };
        let Some(mut path) = rfd::FileDialog::new()
            .add_filter(filter, &[ext])
            .set_file_name(format!("{} (selection).{ext}", t.stem("document")))
            .save_file()
        else {
            return;
        };
        if path.extension().is_none() {
            path.set_extension(ext);
        }
        let result: mpe_core::Result<()> = match layout {
            dlg::Layout::TextFile => std::fs::write(&path, text).map_err(Into::into),
            _ => Excerpt::new(t, layout)
                .build(&t.pdf)
                .and_then(|mut d| d.save(&path)),
        };
        match result {
            Ok(()) => self.status = format!("Saved the selected text to {}", path.display()),
            Err(e) => {
                self.dialog = Dialog::Message("Save failed".into(), format!("{e:#}"));
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

    /// Saves, first asking when that would break the document's digital signatures.
    fn save(&mut self, as_new: bool) {
        if self.tabs.get(self.active).is_some_and(|t| t.signed) {
            self.dialog = Dialog::ConfirmSaveSigned(as_new);
        } else {
            self.save_now(as_new);
        }
    }

    fn save_now(&mut self, as_new: bool) {
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
                self.recent.add(&path);
                tab.path = Some(path);
                tab.dirty = false;
                // The saved copy's signatures are already broken: no need to ask again.
                tab.signed = false;
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
            Act::OpenRecent(i) => {
                if let Some(path) = self.recent.existing().get(i).map(|p| (*p).clone()) {
                    self.spawn_open(ctx, path, None, None);
                }
            }
            Act::ClearRecent => self.recent.clear(),
            Act::NewFromImages => {
                if let Some(files) = Self::pick_images() {
                    self.new_from_images(ctx, &files);
                }
            }
            Act::Save => self.save(false),
            Act::SaveAs => self.save(true),
            Act::ExportImages if has => self.reveal(Section::Export),
            Act::ExportText if has => self.export_text(),
            Act::Print | Act::PrintSelection if has && !self.busy => {
                let selected = self.tabs[self.active].ed.selection.pages();
                let pages = selected.map_or("all".into(), inspector::range_text);
                let selection = (a == Act::PrintSelection).then_some(dlg::Layout::Original);
                self.dialog = match print::printers() {
                    Ok(list) => Dialog::Print(dlg::Print {
                        pages,
                        selection,
                        ..dlg::Print::new(list)
                    }),
                    Err(e) => Dialog::Message("Print".into(), format!("{e:#}")),
                }
            }
            Act::SaveSelection if has => {
                self.dialog = Dialog::SaveSelection(dlg::Layout::Original);
            }
            Act::Mark(m) if has => {
                let color = markup::color(m, &self.state(ctx));
                let author = self.settings.author();
                self.tabs[self.active].add_markup(ctx, m, color, &author);
            }
            Act::MarkColor { line, index } => {
                if line {
                    self.settings.line = index;
                } else {
                    self.settings.highlight = index;
                }
                self.settings.save();
            }
            Act::CommentName => {
                self.dialog = Dialog::CommentName(self.settings.author());
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
            Act::Theme(p) => {
                ctx.set_theme(p);
                self.settings.theme = p;
                self.settings.save();
            }
            Act::ToggleSide => self.show_side = !self.show_side,
            Act::Bookmarks => {
                self.show_side = true;
                self.tabs[self.active].side = Side::Bookmarks;
            }
            Act::Comments => {
                self.show_side = true;
                self.tabs[self.active].side = Side::Comments;
            }
            Act::ToggleInspector => {
                self.show_inspector = !self.show_inspector;
                if !self.narrow {
                    self.inspector_pref = self.show_inspector;
                }
            }
            Act::Palette => self.palette = Some(Palette::default()),
            _ if !has => {}
            Act::ToggleHidden => {
                let ed = &mut self.tabs[self.active].ed;
                ed.show_hidden = !ed.show_hidden;
            }
            Act::ToggleFields => {
                let form = &mut self.tabs[self.active].ed.form;
                form.highlight = !form.highlight;
            }
            Act::NextField => self.tabs[self.active].ed.form.step(true),
            Act::PrevField => self.tabs[self.active].ed.form.step(false),
            Act::ClearField => {
                let ed = &mut self.tabs[self.active].ed;
                if let Some(a) = ed.form.clear_value() {
                    ed.actions.push(a);
                }
            }
            Act::ResetForm => {
                let t = &mut self.tabs[self.active];
                t.ed.form.clear_focus();
                t.edit(ctx, |p| p.reset_form());
                t.notice = Some("Reset the form".into());
            }
            Act::FlattenForm => self.dialog = Dialog::ConfirmFlatten,
            Act::FlattenAnnots => {
                if self.tabs[self.active].pdf.annotations().is_empty() {
                    self.tabs[self.active].notice =
                        Some("There are no annotations to flatten".into());
                } else {
                    self.dialog = Dialog::ConfirmFlattenAnnots;
                }
            }
            Act::Tool(Tool::Image) => self.tabs[self.active].choose_image(),
            Act::Tool(tool) => self.tabs[self.active].ed.set_tool(tool),
            Act::SelectAllText => {
                let t = &mut self.tabs[self.active];
                t.ed.select_all_words(&t.pdf, t.current);
            }
            Act::DeleteObject => self.tabs[self.active].ed.delete_selected(),
            Act::DeleteSelection => {
                if self.tabs[self.active].ed.has_object() {
                    self.act(ctx, Act::DeleteObject);
                } else if self.tabs[self.active].ed.selection.pages().is_some() {
                    self.act(ctx, Act::Delete);
                } else {
                    self.tabs[self.active].ed.delete_selected();
                }
            }
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
            Act::CopyObjectText => {
                let o = self.tabs[self.active].ed.selected_object();
                if let Some(text) = o.and_then(|o| o.text.clone()) {
                    ctx.copy_text(text);
                    self.status = "Copied the object's text.".into();
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
            // On: the zoom follows the view's width (see Tab::show_pages); off: it stays.
            Act::FitWidth => {
                let t = &mut self.tabs[self.active];
                t.fit = !t.fit;
            }
            Act::RotL | Act::RotR => {
                let deg = if matches!(a, Act::RotR) { 90 } else { 270 };
                let t = &mut self.tabs[self.active];
                let idx = t.targets();
                t.edit(ctx, |p| p.rotate_pages(&idx, deg));
            }
            Act::Delete => {
                let t = &mut self.tabs[self.active];
                let idx = t.targets();
                let before = t.revision;
                t.edit(ctx, |p| p.delete_pages(&idx));
                if t.revision != before {
                    let n = idx.len();
                    let what = if n == 1 {
                        format!("page {}", idx[0] + 1)
                    } else {
                        format!("{n} pages")
                    };
                    self.notify(ctx, format!("Deleted {what}"));
                }
            }
            Act::Extract => {
                let t = &mut self.tabs[self.active];
                match t.pdf.extract(&t.targets()) {
                    Ok(p) => self.add_tab(ctx, p, None, None, true),
                    Err(e) => t.error = Some(format!("{e:#}")),
                }
            }
            Act::Split => self.reveal(Section::Split),
            Act::Blank => {
                let t = &self.tabs[self.active];
                let after = t.targets().last().copied().unwrap_or(t.current);
                let size = t.sizes.get(after).copied().unwrap_or((612.0, 792.0));
                self.insert_blank(ctx, after, size);
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
                // After the selected pages, or the current page.
                let after = t.targets().last().copied().unwrap_or(t.current);
                let (at, before) = (after + 1, t.page_count());
                t.edit(ctx, |p| insert_images(p, at, &data));
                if t.page_count() > before {
                    t.go_to(at);
                }
            }
            Act::Scans => self.reveal(Section::Scans),
            Act::Ocr => self.reveal(Section::Ocr),
            Act::HeaderFooter => self.reveal(Section::HeaderFooter),
            Act::Watermark => self.reveal(Section::Watermark),
            Act::Crop => self.reveal(Section::Crop),
            Act::Props => self.reveal(Section::Properties),
            Act::Security => self.reveal(Section::Security),
            Act::EditObjectText => {
                self.show_inspector = true;
                self.tabs[self.active].ed.focus_text_field();
            }
            Act::ReplaceImage => {
                let t = &mut self.tabs[self.active];
                let Some((page, id)) = t.ed.sel() else {
                    return;
                };
                if t.ed.selected_object().is_some_and(|o| o.inline) {
                    t.error = Some(
                        "This image is stored inside the page's content (an inline image): \
                         it can be moved, resized or deleted, but not replaced."
                            .into(),
                    );
                } else if let Some(bytes) = crate::editor::pick_image() {
                    t.ed.actions.push(EdAction::ReplaceImage(page, id, bytes));
                }
            }
            _ => {}
        }
    }

    /// Open section `s` of the inspector and scroll to it. The panel must show the page
    /// or document sections, so an object or word selection is dropped and a tool that
    /// adds content gives way to the arrow; document-only sections also drop a page
    /// selection.
    fn reveal(&mut self, s: Section) {
        self.show_inspector = true;
        let ed = &mut self.tabs[self.active].ed;
        ed.selection.clear_object();
        ed.selection.clear_words();
        if ed.owns_panel() {
            ed.set_tool(Tool::Select);
        }
        if s.document_only() {
            ed.selection = Selection::None;
        }
        self.tabs[self.active].forms.reveal = Some(s);
    }

    /// The undo toast, bottom centre above the status bar, until it times out, is
    /// dismissed, or Undo is pressed.
    fn toast_ui(&mut self, ctx: &egui::Context, acts: &mut Vec<Act>) {
        let Some(toast) = &self.toast else {
            return;
        };
        let now = ctx.input(|i| i.time);
        if now >= toast.until || self.tabs.is_empty() {
            self.toast = None;
            return;
        }
        ctx.request_repaint_after(std::time::Duration::from_secs_f64(toast.until - now));
        let mut close = false;
        egui::Area::new(egui::Id::new("toast"))
            .anchor(egui::Align2::CENTER_BOTTOM, [0.0, -40.0])
            .order(egui::Order::Foreground)
            .show(ctx, |ui| {
                egui::Frame::popup(ui.style())
                    .inner_margin(egui::Margin::symmetric(12, 8))
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            ui.label(&toast.text);
                            ui.add_space(8.0);
                            let undo = actions::get(Act::Undo);
                            let b = egui::Button::new(egui::RichText::new("Undo").strong());
                            let tip = tooltip(ctx, undo);
                            if ui.add(b).on_hover_text(tip).clicked() {
                                acts.push(Act::Undo);
                                close = true;
                            }
                            let x = egui::Button::new(theme::icon(icon::X));
                            let x = x.frame_when_inactive(false);
                            if ui.add(x).on_hover_text("Dismiss").clicked() {
                                close = true;
                            }
                        });
                    });
            });
        if close {
            self.toast = None;
        }
    }

    /// Show the undo toast for something that just happened.
    fn notify(&mut self, ctx: &egui::Context, text: String) {
        let until = ctx.input(|i| i.time) + TOAST_SECONDS;
        self.toast = Some(Toast { text, until });
    }

    /// Insert a blank page of `size` points after page `after` (0-based) and go to it.
    fn insert_blank(&mut self, ctx: &egui::Context, after: usize, size: (f32, f32)) {
        let t = &mut self.tabs[self.active];
        let before = t.page_count();
        t.edit(ctx, |p| p.insert_blank(after + 1, size.0, size.1));
        if t.page_count() > before {
            t.go_to(after + 1);
        }
    }

    /// Carry out a button pressed in the inspector's page or document section.
    fn apply_panel(&mut self, ctx: &egui::Context, apply: Apply) {
        let active = self.active;
        let n = self.tabs[active].page_count();
        match apply {
            Apply::Act(a) => self.act(ctx, a),
            Apply::Blank(after) => {
                let size = self.tabs[active].forms.blank;
                self.insert_blank(ctx, after, size);
            }
            Apply::Props => {
                let t = &mut self.tabs[active];
                let info = t.forms.meta.clone();
                t.edit(ctx, |p| p.set_info(&info));
            }
            Apply::Security => {
                let t = &mut self.tabs[active];
                t.banners.retain(|b| b.kind != BannerKind::Protected);
                t.set_encryption(t.forms.sec.encryption());
                self.status = "Security settings apply when you save.".into();
                self.notify(ctx, "Changed password security".into());
            }
            Apply::Crop => {
                let t = &mut self.tabs[active];
                let s = &t.forms.crop;
                match parse_page_range(&s.pages, n) {
                    Ok(idx) => {
                        let m = s.margins;
                        t.edit(ctx, |p| {
                            idx.iter().try_for_each(|&i| p.set_crop_margins(i, m))
                        });
                    }
                    Err(e) => t.error = Some(e.to_string()),
                }
            }
            Apply::Watermark => {
                let t = &mut self.tabs[active];
                let s = &t.forms.wm;
                match parse_page_range(&s.range, n) {
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
                    }
                    Err(e) => t.error = Some(e.to_string()),
                }
            }
            Apply::HeaderFooter => {
                let t = &mut self.tabs[active];
                let s = &t.forms.hf;
                match parse_page_range(&s.range, n) {
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
                    }
                    Err(e) => t.error = Some(e.to_string()),
                }
            }
            Apply::Ocr => match parse_page_range(&self.tabs[active].forms.ocr, n) {
                Ok(idx) => self.spawn_job(ctx, OCR_JOB, move |p| {
                    let ocr = mpe_core::Ocr::bundled()?;
                    Ok(ocr_summary(&p.ocr_pages(&idx, &ocr)?))
                }),
                Err(e) => self.tabs[active].error = Some(e.to_string()),
            },
            Apply::Scans => {
                let f = &self.tabs[active].forms;
                let opts = f.scan;
                match parse_page_range(&f.scan_pages, n) {
                    Ok(idx) => self.spawn_job(ctx, "Optimise scanned pages", move |p| {
                        Ok(scan_summary(&p.optimize_scans(&idx, &opts)?))
                    }),
                    Err(e) => self.tabs[active].error = Some(e.to_string()),
                }
            }
            Apply::Export => {
                let t = &mut self.tabs[active];
                let s = &t.forms.export;
                let (format, dpi) = (s.format(), s.dpi);
                let pages = match parse_page_range(&s.pages, n) {
                    Ok(p) => p,
                    Err(e) => {
                        t.error = Some(e.to_string());
                        return;
                    }
                };
                let stem = t.stem("page");
                let Some(dir) = rfd::FileDialog::new()
                    .set_title("Choose a folder for the images")
                    .pick_folder()
                else {
                    return;
                };
                let e = ImageExport {
                    stem,
                    format,
                    dpi,
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
            }
            Apply::Split => {
                let every = self.tabs[active].forms.split.max(1);
                let Some(dir) = rfd::FileDialog::new()
                    .set_title("Choose output folder")
                    .pick_folder()
                else {
                    return;
                };
                let t = &mut self.tabs[active];
                let stem = t.stem("part");
                match t.pdf.split_every(every) {
                    Ok(parts) => {
                        let count = parts.len();
                        for (i, mut part) in parts.into_iter().enumerate() {
                            let path = dir.join(format!("{stem}-{:03}.pdf", i + 1));
                            if let Err(e) = part.save(path) {
                                let text = format!("{e:#}");
                                self.dialog = Dialog::Message("Split failed".into(), text);
                                return;
                            }
                        }
                        self.status = format!("Wrote {count} files to {}", dir.display());
                    }
                    Err(e) => t.error = Some(format!("{e:#}")),
                }
            }
        }
    }

    fn close_tab(&mut self, i: usize) {
        if i < self.tabs.len() {
            self.tabs.remove(i);
            self.active = self.active.min(self.tabs.len().saturating_sub(1));
        }
    }

    fn dialogs(&mut self, ctx: &egui::Context) {
        let mut dialog = std::mem::replace(&mut self.dialog, Dialog::None);
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
            Dialog::CommentName(name) => match dlg::comment_name(ctx, name) {
                Outcome::Apply => {
                    self.settings.author = Some(name.trim().to_string());
                    self.settings.save();
                    done = true;
                }
                Outcome::Cancel => done = true,
                Outcome::Open => {}
            },
            Dialog::OpenLink(url) => match dlg::confirm(
                ctx,
                "Open link",
                &format!(
                    "This link goes to:\n\n{url}\n\nOpen it in your browser or email program?"
                ),
                "Open link",
            ) {
                Outcome::Apply => {
                    ctx.open_url(egui::OpenUrl::new_tab(url.as_str()));
                    done = true;
                }
                Outcome::Cancel => done = true,
                Outcome::Open => {}
            },
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
            Dialog::ConfirmFlatten => match dlg::confirm(
                ctx,
                "Flatten form?",
                "The fields become part of the pages, as they look now, and can no longer be \
                 filled in, here or in other readers. Undo brings them back until you close \
                 the document.",
                "Flatten",
            ) {
                Outcome::Apply => {
                    let t = &mut self.tabs[self.active];
                    t.ed.form.clear_focus();
                    if t.edit(ctx, |p| p.flatten_form(None).map(|_| ())) {
                        t.banners.retain(|b| b.kind != BannerKind::Forms);
                        t.notice = Some("Flattened the form".into());
                    }
                    done = true;
                }
                Outcome::Cancel => done = true,
                Outcome::Open => {}
            },
            Dialog::ConfirmFlattenAnnots => match dlg::confirm(
                ctx,
                "Flatten annotations?",
                "Comments, notes, highlights and drawings become part of the pages, as they \
                 look now, and can no longer be changed or removed, here or in other readers. \
                 Their comment text is dropped. Links and form fields stay. Undo brings them \
                 back until you close the document.",
                "Flatten",
            ) {
                Outcome::Apply => {
                    let t = &mut self.tabs[self.active];
                    if t.edit(ctx, |p| p.flatten_annotations(None).map(|_| ())) {
                        t.notice = Some("Flattened the annotations".into());
                    }
                    done = true;
                }
                Outcome::Cancel => done = true,
                Outcome::Open => {}
            },
            Dialog::ConfirmSaveSigned(as_new) => {
                let as_new = *as_new;
                match dlg::confirm(
                    ctx,
                    "Break the signatures?",
                    "This document is digitally signed. MPE saves by rewriting the whole file, so \
                     the saved copy's signatures will show as invalid in every reader. Save as a \
                     new file to keep the signed original.",
                    "Save anyway",
                ) {
                    Outcome::Apply => {
                        self.save_now(as_new);
                        done = true;
                    }
                    Outcome::Cancel => done = true,
                    Outcome::Open => {}
                }
            }
            Dialog::Print(s) => match dlg::print(ctx, s) {
                Outcome::Apply if s.printers.is_empty() => done = true,
                Outcome::Apply => {
                    let t = &mut self.tabs[self.active];
                    let printer = s.printers[s.printer].name.clone();
                    let opts = print::PrintOptions {
                        printer: Some(printer.clone()),
                        copies: s.copies,
                        sides: s.sides,
                        as_image: s.as_image,
                        title: t.stem("Untitled"),
                    };
                    if let Some(layout) = s.selection {
                        let ex = Excerpt::new(t, layout);
                        self.spawn_job(ctx, "Print", move |p| {
                            let mut out = ex.build(p)?;
                            let pages: Vec<usize> = (0..out.page_count()).collect();
                            let reply = out.print(&pages, &opts)?;
                            let msg = format!("Sent the selected text to {printer}.\n{reply}");
                            Ok((false, msg))
                        });
                        done = true;
                    } else {
                        match parse_page_range(&s.pages, t.page_count()) {
                            Ok(pages) => {
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
                }
                Outcome::Cancel => done = true,
                Outcome::Open => {}
            },
            Dialog::SaveSelection(layout) => match dlg::save_selection(ctx, layout) {
                Outcome::Apply => {
                    let layout = *layout;
                    done = true;
                    self.save_selection(layout);
                }
                Outcome::Cancel => done = true,
                Outcome::Open => {}
            },
        }
        if !done {
            self.dialog = dialog;
        }
    }

    /// The state the commands' conditions are checked against.
    fn state(&self, ctx: &egui::Context) -> Ctx {
        let mut c = Ctx {
            busy: self.busy,
            side: self.show_side,
            inspector: self.show_inspector,
            theme: ctx.options(|o| o.theme_preference),
            highlight: self.settings.highlight,
            line: self.settings.line,
            ..Default::default()
        };
        if let Some(t) = self.tabs.get(self.active) {
            t.fill_ctx(&mut c);
        }
        c
    }

    /// The menu: one submenu per group, built from the registry, plus File > Open recent
    /// from `recent` (the recent files that still exist).
    fn menu(ui: &mut egui::Ui, c: &Ctx, recent: &[&PathBuf], acts: &mut Vec<Act>) {
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
                        if a.act == Act::Open {
                            recent_menu(ui, recent, acts);
                        }
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

/// A toolbar button for a command: (ui, command, shown as on, where clicks go).
type BarButton<'a> = &'a dyn Fn(&mut egui::Ui, Act, bool, &mut Vec<Act>);

/// The Annotate button: the annotation tool last used, and a small menu beside it for
/// the others.
fn annotate_button(
    ui: &mut egui::Ui,
    tools: Vec<&Action>,
    c: &Ctx,
    bar: BarButton,
    acts: &mut Vec<Act>,
) {
    let last = c.annot_tool.unwrap_or(Tool::Note);
    let on = |a: &Action| matches!(a.act, Act::Tool(t) if c.tool == Some(t));
    if let Some(a) = tools.iter().find(|a| a.act == Act::Tool(last)) {
        bar(ui, a.act, on(a), acts);
    }
    ui.add_space(-ui.spacing().item_spacing.x);
    ui.menu_button(theme::icon(icon::CARET_DOWN).size(10.0), |ui| {
        for a in tools {
            let b = egui::Button::new((theme::icon(a.icon).size(16.0), a.label)).selected(on(a));
            if ui.add(b).clicked() {
                acts.push(a.act);
                ui.close();
            }
        }
    })
    .response
    .on_hover_text("Annotate");
}

/// The tools as one menu button showing the current tool, for narrow windows.
fn tool_menu(
    ui: &mut egui::Ui,
    tools: Vec<&Action>,
    is_on: &dyn Fn(&Action) -> bool,
    acts: &mut Vec<Act>,
) {
    let on = tools.iter().find(|a| is_on(a));
    let current = on.map_or(icon::HAND, |a| a.icon);
    let label = (
        theme::icon(current).size(18.0),
        theme::icon(icon::CARET_DOWN).size(10.0),
    );
    ui.menu_button(label, |ui| {
        for a in tools {
            let b = egui::Button::new((theme::icon(a.icon).size(16.0), a.label)).selected(is_on(a));
            if ui.add(b).clicked() {
                acts.push(a.act);
                ui.close();
            }
        }
    })
    .response
    .on_hover_text("Tools");
}

/// The collapsed inspector: a rail with an icon and an arrow, clickable as a whole, so
/// it is clear that a panel slides out here. Returns true when clicked.
fn inspector_rail(ui: &mut egui::Ui) -> bool {
    let rect = ui.max_rect();
    let id = egui::Id::new("inspector-rail-click");
    let r = ui
        .interact(rect, id, egui::Sense::click())
        .on_hover_cursor(egui::CursorIcon::PointingHand)
        .on_hover_text("Show the inspector: options for the selection or the document");
    if r.hovered() {
        let fill = ui.visuals().widgets.hovered.weak_bg_fill;
        ui.painter().rect_filled(rect, 0, fill);
    }
    ui.vertical_centered(|ui| {
        ui.add_space(6.0);
        ui.label(theme::icon(icon::SLIDERS_HORIZONTAL).size(18.0));
        ui.label(theme::icon(icon::CARET_DOUBLE_LEFT).size(12.0).weak());
    });
    r.clicked()
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

/// File > Open recent: the recent files that still exist, then Clear recent files.
fn recent_menu(ui: &mut egui::Ui, recent: &[&PathBuf], acts: &mut Vec<Act>) {
    ui.add_enabled_ui(!recent.is_empty(), |ui| {
        ui.menu_button("Open recent", |ui| {
            for (i, path) in recent.iter().enumerate() {
                let tip = path.display().to_string();
                let row = egui::Button::new(file_name(path)).truncate();
                if ui.add(row).on_hover_text(tip).clicked() {
                    acts.push(Act::OpenRecent(i));
                    ui.close();
                }
            }
            ui.separator();
            if ui.button("Clear recent files").clicked() {
                acts.push(Act::ClearRecent);
                ui.close();
            }
        });
    });
}

/// A path's file name, or the whole path when it has none.
fn file_name(path: &Path) -> String {
    match path.file_name() {
        Some(n) => n.to_string_lossy().into(),
        None => path.display().to_string(),
    }
}

/// The window with no document: a card to open a PDF or start from images. The card
/// is a drop target and is clickable as a whole; it lights up while files are dragged
/// over the window.
fn empty_state(ui: &mut egui::Ui, recent: &[&PathBuf], acts: &mut Vec<Act>) {
    let dragging = ui.input(|i| !i.raw.hovered_files.is_empty());
    let room = ui.max_rect();
    // Up to five recent files under the buttons, with a heading.
    let shown = recent.len().min(5);
    let extra = if shown > 0 {
        36.0 + 30.0 * shown as f32
    } else {
        0.0
    };
    let size = egui::vec2(440.0, 280.0 + extra).min(room.size() - egui::vec2(32.0, 32.0));
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
    let images = egui::Button::new("Make a PDF from images…").min_size(egui::vec2(180.0, 28.0));
    let images = ui
        .add(images)
        .on_hover_text("Turn photos or scanned pages into a new PDF");
    if images.clicked() {
        acts.push(Act::NewFromImages);
    }
    if shown > 0 {
        ui.add_space(16.0);
        ui.label(egui::RichText::new("Recent").family(theme::semibold()));
        ui.add_space(4.0);
        for (i, path) in recent.iter().take(shown).enumerate() {
            // The name, not the icon, is the part that shrinks: cut short with "…".
            // The growing space after it keeps the rows left-aligned.
            let name = file_name(path).atom_shrink(true);
            let glyph = theme::icon(icon::FILE_PDF).size(14.0);
            let row = egui::Button::new((glyph, name, egui::Atom::grow()))
                .truncate()
                .frame_when_inactive(false)
                .min_size(egui::vec2(ui.available_width(), 26.0));
            let tip = path.display().to_string();
            if ui.add(row).on_hover_text(tip).clicked() {
                acts.push(Act::OpenRecent(i));
            }
        }
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

        // Collapse the inspector when the window becomes narrow; when it becomes wide
        // again, restore what the user last chose at full width.
        let narrow = ctx.content_rect().width() < NARROW_WIDTH;
        if narrow != self.narrow {
            self.show_inspector = !narrow && self.inspector_pref;
            self.narrow = narrow;
        }

        self.shortcuts(&ctx);

        let mut acts: Vec<Act> = Vec::new();
        let has = !self.tabs.is_empty();
        let c = self.state(&ctx);
        // Recent files that still exist, for the menu and the start screen. Copied, so
        // the frame can change the app while showing them.
        let recent_files: Vec<PathBuf> = self.recent.existing().into_iter().cloned().collect();
        let recent: Vec<&PathBuf> = recent_files.iter().collect();

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

        let mut right_acts: Vec<Act> = Vec::new();

        egui::Panel::top("topbar")
            .frame(theme::bar_frame(ui.style()))
            .show(ui, |ui| {
                // Width budget, so the two ends never overlap: on narrower windows the
                // search field becomes an icon, and then the tools fold into one menu.
                // (The file name is on the tab and in the window title.)
                let sp = ui.spacing();
                let btn = (18.0 + 2.0 * sp.button_padding.x).max(28.0) + sp.item_spacing.x;
                let sep = 2.0 * sp.item_spacing.x;
                // Search field with its icon, and the zoom percentage.
                let (field, pct) = (180.0 + 18.0 + 2.0 * sp.item_spacing.x, 64.0);
                // Theme, Export menu, Print, search, then fit, zoom in and zoom out.
                let right = |with_field: bool| {
                    let search = if with_field { field } else { btn };
                    3.0 * btn + search + sep + 3.0 * btn + pct
                };
                // Menu, Open, Save, Undo, Redo and three separators, then the tools.
                let left = |tools: f32| 5.0 * btn + 3.0 * sep + tools;
                // Seven tools, then the Annotate button with its menu.
                let inline = 10.0 * btn;
                let w = ui.available_width() - 16.0;
                let search_field = w >= left(inline) + right(true);
                let tools_inline = w >= left(inline) + right(search_field);

                let left_side = |ui: &mut egui::Ui| {
                    ui.menu_button(menu_icon(icon::LIST), |ui| {
                        Self::menu(ui, &c, &recent, &mut acts);
                    })
                    .response
                    .on_hover_text("Menu");
                    bar(ui, Act::Open, false, &mut acts);
                    if !has {
                        return;
                    }
                    ui.separator();
                    bar(ui, Act::Save, false, &mut acts);
                    ui.separator();
                    bar(ui, Act::Undo, false, &mut acts);
                    bar(ui, Act::Redo, false, &mut acts);
                    ui.separator();
                    let tools = ACTIONS.iter().filter(|a| a.group == Group::Tools);
                    let is_on = |a: &Action| matches!(a.act, Act::Tool(t) if c.tool == Some(t));
                    let annotate =
                        |a: &&Action| matches!(a.act, Act::Tool(t) if Tool::ANNOTATE.contains(&t));
                    if tools_inline {
                        for a in tools.clone().filter(|a| !annotate(a)) {
                            bar(ui, a.act, is_on(a), &mut acts);
                        }
                        ui.separator();
                        let annot_tools = tools.filter(annotate).collect();
                        annotate_button(ui, annot_tools, &c, &bar, &mut acts);
                    } else {
                        tool_menu(ui, tools.collect(), &is_on, &mut acts);
                    }
                };
                let right_side = |ui: &mut egui::Ui| {
                    let acts = &mut right_acts;
                    // The theme, at the far right, with or without a document.
                    let glyph = match c.theme {
                        egui::ThemePreference::Light => icon::SUN,
                        egui::ThemePreference::Dark => icon::MOON,
                        egui::ThemePreference::System => icon::CIRCLE_HALF,
                    };
                    ui.menu_button(menu_icon(glyph), |ui| {
                        let themes = ACTIONS.iter().filter(|a| matches!(a.act, Act::Theme(_)));
                        for a in themes {
                            menu_item(ui, a, &c, acts);
                        }
                    })
                    .response
                    .on_hover_text("Theme: follow the system, light or dark");
                    if !has {
                        return;
                    }
                    ui.menu_button(menu_icon(icon::EXPORT), |ui| {
                        for act in [Act::ExportImages, Act::ExportText] {
                            menu_item(ui, actions::get(act), &c, acts);
                        }
                    })
                    .response
                    .on_hover_text("Export");
                    bar(ui, Act::Print, false, acts);
                    let t = &mut self.tabs[self.active];
                    if search_field {
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
                        bar(ui, Act::Find, false, acts);
                    }
                    ui.separator();
                    // Highlighted while the zoom follows the width.
                    bar(ui, Act::FitWidth, c.fit, acts);
                    bar(ui, Act::ZoomIn, false, acts);
                    // The zoom level; clicking it resets to actual size.
                    let level = format!("{:.0}%", t.zoom * 100.0);
                    let b = egui::Button::new(level).frame_when_inactive(false);
                    let tip = tooltip(ui.ctx(), actions::get(Act::Actual));
                    if ui.add(b).on_hover_text(tip).clicked() {
                        acts.push(Act::Actual);
                    }
                    bar(ui, Act::ZoomOut, false, acts);
                };
                // The right side is laid out first; the left gets what remains.
                egui::Sides::new()
                    .shrink_left()
                    .height(28.0)
                    .show(ui, left_side, right_side);
            });
        acts.append(&mut right_acts);

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
                    ui.horizontal_wrapped(|ui| {
                        ui.selectable_value(&mut t.side, Side::Pages, "Pages");
                        ui.selectable_value(&mut t.side, Side::Search, "Search");
                        ui.selectable_value(&mut t.side, Side::Bookmarks, "Bookmarks");
                        ui.selectable_value(&mut t.side, Side::Comments, "Comments");
                    });
                    ui.separator();
                    match t.side {
                        Side::Pages => t.show_thumbs(ui, &c),
                        Side::Bookmarks => t.show_outline(ui),
                        Side::Comments => t.show_comments(ui),
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

        // The inspector: the editor's section for a selected object or words or a tool
        // that adds content; otherwise the selected pages or the document.
        // Collapsed, it is a thin rail at the right edge that slides the panel back out.
        let mut panel_apply = None;
        if has {
            let t = &mut self.tabs[self.active];
            // "Reply…" in the Comments tab types in the inspector.
            if t.ed.reply_focus {
                self.show_inspector = true;
            }
            let mut open = self.show_inspector;
            let mut toggle = false;
            let rail = egui::Panel::right("inspector-rail")
                .resizable(false)
                .exact_size(RAIL_WIDTH);
            let full = egui::Panel::right("inspector")
                .resizable(true)
                .default_size(INSPECTOR_WIDTH);
            egui::Panel::show_switched(ui, &mut open, rail, full, |ui, expanded| {
                if !expanded {
                    toggle = inspector_rail(ui);
                    return;
                }
                // The heading stays put above the scrolling content, with the
                // button that hides the panel at its right.
                let title = inspector::title(t);
                egui::Sides::new().shrink_left().truncate().show(
                    ui,
                    |ui| ui.heading(title),
                    |ui| {
                        let hide = egui::Button::new(theme::icon(icon::CARET_DOUBLE_RIGHT))
                            .frame_when_inactive(false);
                        if ui.add(hide).on_hover_text("Hide the inspector").clicked() {
                            toggle = true;
                        }
                    },
                );
                ui.add_space(4.0);
                egui::ScrollArea::vertical()
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        if t.ed.owns_panel() {
                            t.ed.inspector(ui);
                        } else {
                            panel_apply = inspector::show(ui, t, &c);
                        }
                    });
            });
            if toggle {
                open = !open;
            }
            // Also catches the panel being dragged closed or open.
            if open != self.show_inspector {
                self.show_inspector = open;
                if !self.narrow {
                    self.inspector_pref = open;
                }
            }
        }
        if let Some(apply) = panel_apply {
            self.apply_panel(&ctx, apply);
        }

        let canvas = egui::Frame::central_panel(ui.style())
            .fill(theme::canvas(ui.visuals().dark_mode))
            .inner_margin(0);
        let author = self.settings.author();
        egui::CentralPanel::default().frame(canvas).show(ui, |ui| {
            if let Some(t) = self.tabs.get_mut(self.active) {
                t.ed.author = author;
                t.show_pages(ui, &c);
            } else {
                empty_state(ui, &recent, &mut acts);
            }
        });

        // Surface errors raised by tab operations.
        if let Some(t) = self.tabs.get_mut(self.active) {
            if let Some(e) = t.error.take() {
                self.dialog = Dialog::Message("Error".into(), e);
            }
            if let Some(url) = t.open_link.take() {
                self.dialog = link_dialog(url);
            }
        }

        // Commands chosen from right-click menus on pages and thumbnails, and undoable
        // changes the editor reports.
        let mut notice = None;
        if let Some(t) = self.tabs.get_mut(self.active) {
            acts.append(&mut t.requested);
            notice = t.notice.take();
        }
        if let Some(text) = notice {
            self.notify(&ctx, text);
        }
        self.toast_ui(&ctx, &mut acts);
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

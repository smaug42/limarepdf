use crate::about;
use crate::actions::{self, menu_item, tooltip, Act, Action, Ctx, Group, ACTIONS};
use crate::banners::Kind as BannerKind;
use crate::dialogs::{self as dlg, Ask, Btn, Choice, Outcome, Shell};
use crate::digital::{self, IdsWindow, SignOut, SignWindow};
use crate::editor::{Action as EdAction, Tool};
use crate::inspector::{self, Apply, Section};
use crate::markup;
use crate::panels::Panel;
use crate::recent::Recent;
use crate::selection::Selection;
use crate::settings::{Layout, Settings};
use crate::signatures::{self as sigs, Check, SigState, TrustAsk, TrustedOut, TrustedWindow};
use crate::tab::{Side, State, Tab};
use crate::theme;
use crate::worker::Colours;
use eframe::egui::{self, AtomExt as _, Key};
use egui_phosphor::regular as icon;
use limarepdf_core::render::{image_names, ImageFormat};
use limarepdf_core::{
    parse_page_range, part_file_name, print, ImageImport, OcrResult, OpenPanel, Pdf, ScanResult,
    SplitBy, TrustRole,
};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::Arc;

mod recover;

pub const IMAGE_EXTS: [&str; 5] = ["png", "jpg", "jpeg", "tif", "tiff"];

fn is_image(p: &Path) -> bool {
    p.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| IMAGE_EXTS.contains(&e.to_ascii_lowercase().as_str()))
}

/// Insert image files as pages at `at`, in the order given, naming the file that fails.
fn insert_images(
    p: &mut Pdf,
    mut at: usize,
    files: &[(PathBuf, Vec<u8>)],
) -> limarepdf_core::Result<()> {
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
        /// For a file with signature fields: its bytes and password, to check them.
        signed: Option<(Arc<[u8]>, Option<String>)>,
    },
    /// The signature check of tab `tab` (its `generation`th) finished.
    Signatures {
        tab: u64,
        generation: u64,
        check: Check,
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
            format!("This link goes to:\n\n{url}\n\nLimarePDF only opens web and email links."),
        )
    }
}

/// Summary of a scan optimisation for the user, and whether any page changed.
fn scan_summary(report: &[(usize, ScanResult)]) -> (bool, String) {
    let (mut n, mut before, mut after) = (0, 0, 0);
    let (mut skipped, mut same, mut kept) = (Vec::new(), Vec::new(), Vec::new());
    for (p, r) in report {
        match r {
            ScanResult::Optimized {
                before: b,
                after: a,
                original_kept,
                ..
            } => {
                n += 1;
                before += b;
                after += a;
                if *original_kept {
                    kept.push((p + 1).to_string());
                }
            }
            ScanResult::SameImage(first) => {
                n += 1;
                same.push(format!(
                    "Page {}: the same picture as page {}, optimised with it.",
                    p + 1,
                    first + 1
                ));
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
    for s in same.iter().take(12) {
        text.push('\n');
        text.push_str(s);
    }
    if same.len() > 12 {
        text.push_str("\n…");
    }
    match kept.len() {
        0 => {}
        1 => text.push_str(&format!(
            "\n\nPage {}'s original picture is also used elsewhere in the document (on a page \
             that wasn't selected, for example). It stays there, so the file holds both.",
            kept[0]
        )),
        _ => text.push_str(&format!(
            "\n\nThe original pictures of pages {} are also used elsewhere in the document \
             (on pages that weren't selected, for example). They stay there, so the file \
             holds both versions.",
            kept.join(", ")
        )),
    }
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

/// The tab a question is about, as it was when asked. The answer acts on that tab only
/// while it is open and unchanged ([`App::still`]): work in the background, such as a file
/// finishing opening, goes on while a question is open.
struct Asked {
    tab: u64,
    /// Boxed, to keep [`Dialog`] small.
    state: Box<State>,
}

impl Asked {
    fn of(t: &Tab) -> Self {
        Self {
            tab: t.id,
            state: Box::new(t.state()),
        }
    }
}

/// The dialogs left: settings live in the inspector.
enum Dialog {
    None,
    Password(dlg::Password),
    Message(String, String),
    ConfirmClose(Asked),
    ConfirmQuit,
    /// Export would replace this many files.
    ConfirmReplace(Asked, ImageExport, usize),
    ConfirmFlatten(Asked),
    ConfirmFlattenAnnots(Asked),
    /// Save (`true`: as a new file) by writing the whole file, which breaks the
    /// document's signatures? With why the edits can't be added after what was signed.
    ConfirmSaveSigned(Asked, bool, String),
    /// Save over a file that was repaired to open it: replace it, or save as a new file.
    ConfirmSaveRepaired(Asked),
    /// Save edits the document's signatures don't allow?
    ConfirmSaveDisallowed(Box<SaveDisallowed>),
    /// Remove everything under this many marked areas?
    ConfirmApplyRedactions(Asked, usize),
    /// Draw this many fill-and-sign items into the pages?
    ConfirmFinishSigning(Asked, usize),
    RemoveHidden(Asked, dlg::RemoveHidden),
    /// Save (`true`: as a new file) with this many marked areas not yet applied?
    ConfirmSaveMarks(Asked, bool, usize),
    Print(Asked, Box<dlg::Print>),
    /// Print with fields whose values won't show on paper?
    ConfirmPrintFields(Box<PrintFields>),
    SaveSelection(Asked, dlg::Layout),
    /// Open this web or email address from a link?
    OpenLink(String),
    /// The name for comments, being edited.
    CommentName(String),
    About(about::Page),
    /// A message about a recovery copy, with Show file for it (or its folder).
    RecoveryFile {
        title: String,
        text: String,
        file: PathBuf,
    },
    /// Sign a recovered copy of a signed document, which writes the whole file?
    ConfirmSignRecovered(Asked),
}

impl Dialog {
    /// The tab the dialog is about, if it is about one.
    fn asked(&self) -> Option<&Asked> {
        match self {
            Dialog::ConfirmClose(a)
            | Dialog::ConfirmReplace(a, ..)
            | Dialog::ConfirmFlatten(a)
            | Dialog::ConfirmFlattenAnnots(a)
            | Dialog::ConfirmSaveSigned(a, ..)
            | Dialog::ConfirmSaveRepaired(a)
            | Dialog::ConfirmApplyRedactions(a, _)
            | Dialog::ConfirmFinishSigning(a, _)
            | Dialog::RemoveHidden(a, _)
            | Dialog::ConfirmSaveMarks(a, ..)
            | Dialog::Print(a, _)
            | Dialog::SaveSelection(a, _)
            | Dialog::ConfirmSignRecovered(a) => Some(a),
            Dialog::ConfirmSaveDisallowed(s) => Some(&s.asked),
            Dialog::ConfirmPrintFields(q) => Some(&q.asked),
            _ => None,
        }
    }
}

/// Why a signed document with redactions applied or hidden information removed is saved
/// by writing the whole file.
const SCRUBBED: &str = "you applied redactions or removed hidden information, and the version \
                        that was signed would still hold everything removed";

/// Why a signed document that was repaired to open it is saved by writing the whole file.
const REPAIRED: &str = "the file is damaged, and LimarePDF repaired it to open it";

/// Why a signed document restored from a recovery copy is saved by writing the whole file.
const RECOVERED: &str = "this is a recovered copy, and LimarePDF doesn't have the file as it \
                         was signed";

/// What a save that replaced the file but couldn't confirm it on disk says (decision D7).
pub const NOT_CONFIRMED: &str = "The file was saved, but your system couldn't confirm it is \
                                 safely on disk. If the computer lost power just now, save it \
                                 again.";

/// Whether `e` is a save that replaced the file but couldn't confirm it on disk: the file
/// is the new one, never the old one kept.
pub fn not_confirmed(e: &anyhow::Error) -> bool {
    e.downcast_ref::<limarepdf_core::NotConfirmed>().is_some()
}

/// Why a document set to be saved with a password isn't signed: the signed file would be
/// written without it.
const PASSWORD_NOT_SIGNED: &str = "You set a password that applies when you save, and \
    LimarePDF can't sign password-protected documents, so the signed file would be saved \
    without it. To sign, take the password off first (Document > Password security). To keep \
    the password, save without signing.";

/// A signed document with its edits added after what was signed, ready to write once the
/// user agrees that the signatures will report changes they don't allow.
struct SaveDisallowed {
    as_new: bool,
    /// The tab, as it was when the file was made from it.
    asked: Asked,
    bytes: Vec<u8>,
    disallowed: limarepdf_core::DisallowedChanges,
}

/// A print job waiting for the user to agree that these fields won't show on paper:
/// LimarePDF can't draw their values, and they have no drawing of their own.
struct PrintFields {
    asked: Asked,
    fields: Vec<String>,
    pages: Vec<usize>,
    opts: print::PrintOptions,
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

    fn build(&self, p: &Pdf) -> limarepdf_core::Result<Pdf> {
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
/// Below this window width the inspector collapses to its strip by itself.
const NARROW_WIDTH: f32 = 960.0;
/// Below this window width the navigation panel folds away to its rail by itself.
const SIDE_NARROW_WIDTH: f32 = 760.0;

/// The navigation rail's views, top to bottom. The digital signatures view is opened
/// from the menu only.
const NAV_VIEWS: [(Side, Act); 4] = [
    (Side::Pages, Act::PagesView),
    (Side::Search, Act::SearchView),
    (Side::Bookmarks, Act::Bookmarks),
    (Side::Comments, Act::Comments),
];

/// A top-bar control that leaves the bar when it is short of room: a tool, into the Tools
/// menu, or Export or Undo and Redo, which are in the menu (☰) anyway.
#[derive(Clone, Copy, PartialEq, Debug)]
enum Fold {
    /// The Comment button and its menu.
    Commenting,
    Tool(Tool),
    Export,
    /// Undo and Redo.
    History,
}

impl Fold {
    /// Its width on the bar, from a button's, a menu caret's and a separator's.
    fn width(self, btn: f32, caret: f32, sep: f32) -> f32 {
        match self {
            Fold::Commenting => sep + btn + caret,
            // Fill and sign has a menu beside it.
            Fold::Tool(Tool::Sign) => btn + caret,
            Fold::Tool(_) | Fold::Export => btn,
            Fold::History => sep + 2.0 * btn,
        }
    }
}

/// The order controls leave the bar; they come back in reverse. The tools go first,
/// and only in very narrow windows Export and then Undo and Redo. Read and select text,
/// the Tools menu, Menu, Open, Save, Search and Print stay.
const FOLDS: [Fold; 7] = [
    Fold::Commenting,
    Fold::Tool(Tool::Image),
    Fold::Tool(Tool::Redact),
    Fold::Tool(Tool::Text),
    Fold::Tool(Tool::Sign),
    Fold::Export,
    Fold::History,
];

/// Widths of the top bar's parts as last drawn, in points; `None` until drawn once.
#[derive(Clone, Copy, Default)]
struct BarWidths {
    /// The left end without the tools of [`FOLDS`].
    fixed: Option<f32>,
    /// Each of [`FOLDS`], in its order.
    folds: [Option<f32>; FOLDS.len()],
    /// The right end, with the search icon and with the search field.
    right: [Option<f32>; 2],
    /// The page layout and zoom controls at the right of the status bar.
    view: Option<f32>,
}

/// Extra room a folded control needs before it comes back, so that a width at the
/// boundary doesn't fold and unfold it by turns.
const UNFOLD_MARGIN: f32 = 12.0;

/// How many of [`FOLDS`] to fold, given last frame's count `prev`, the room for the left
/// end of the bar and the width it needs with each count folded.
fn fold_level(prev: usize, room: f32, need: impl Fn(usize) -> f32) -> usize {
    let mut n = prev.min(FOLDS.len());
    while n > 0 && need(n - 1) + UNFOLD_MARGIN <= room {
        n -= 1;
    }
    while n < FOLDS.len() && need(n) > room {
        n += 1;
    }
    n
}

/// An accessible name for a control showing only an icon.
fn named(label: &str) -> egui::WidgetInfo {
    egui::WidgetInfo::labeled(egui::WidgetType::Button, true, label)
}

/// Names the resize handle egui gives `panel` (by its id) and leaves unnamed, once egui
/// has added it this frame, so no stray node is made. egui keeps the handle under the
/// panel's id with "__resize"; the naming tests fail if that changes. It can't be moved
/// with the keyboard (egui's limit).
fn name_resize_handle(ctx: &egui::Context, panel: &str, name: &str) {
    let id = egui::Id::new(panel).with("__resize");
    if ctx.viewport(|v| v.this_pass.widgets.get(id).is_some()) {
        ctx.accesskit_node_builder(id, |n| {
            n.set_role(egui::accesskit::Role::Splitter);
            n.set_label(name);
        });
    }
}

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
    /// The navigation panel at the left (its rail of icons is always shown).
    side: Panel,
    status: String,
    last_title: String,
    quit_ok: bool,
    /// A background job (scan optimisation, OCR) is running.
    busy: bool,
    /// Put the keyboard focus in the top bar's search field (Ctrl+F).
    focus_search: bool,
    palette: Option<Palette>,
    /// The inspector at the right (else collapsed to its strip).
    inspector: Panel,
    /// The collapsed inspector's contents are shown in a pop-out over the pages.
    popout: bool,
    /// How many tool controls are folded into the Tools menu, from [`FOLDS`].
    fold: usize,
    /// The top bar's controls' widths as last drawn.
    bar_widths: BarWidths,
    /// The window has been fitted to its monitor, at the first frame the monitor's size was known.
    fitted: bool,
    toast: Option<Toast>,
    /// Recently opened files, kept between runs.
    recent: Recent,
    /// Settings kept between runs (the theme).
    settings: Settings,
    /// The Details window: the tab's id and the signature's place in its report.
    sig_details: Option<(u64, usize)>,
    /// Asking before trusting a certificate.
    trust_ask: Option<TrustAsk>,
    /// The Trusted certificates window, when open.
    trusted: Option<TrustedWindow>,
    /// The Digital IDs window, when open.
    ids_window: Option<IdsWindow>,
    /// The signing window, when open.
    sign_window: Option<SignWindow>,
    /// Crash recovery: this run's copies, and the copies an earlier run left.
    rescue: recover::Rescue,
}

impl App {
    pub fn new(
        cc: &eframe::CreationContext<'_>,
        settings: Settings,
        files: Vec<PathBuf>,
        page: Option<usize>,
    ) -> Self {
        theme::install(&cc.egui_ctx);
        cc.egui_ctx.set_theme(settings.theme);
        let mut app = Self::with(settings, Recent::load());
        app.start_recovery(&cc.egui_ctx, files, page);
        app
    }

    /// Opens the files given on the command line: PDFs in tabs of their own, images as one
    /// new document.
    fn open_startup(&mut self, ctx: &egui::Context, files: Vec<PathBuf>, page: Option<usize>) {
        let (images, pdfs): (Vec<_>, Vec<_>) = files.into_iter().partition(|p| is_image(p));
        for f in pdfs {
            self.spawn_open(ctx, f, None, page);
        }
        if !images.is_empty() {
            self.new_from_images(ctx, &images);
        }
    }

    /// An app with no tabs.
    fn with(settings: Settings, recent: Recent) -> Self {
        let (tx, rx) = channel();
        Self {
            tabs: Vec::new(),
            active: 0,
            next_id: 1,
            dialog: Dialog::None,
            tx,
            rx,
            side: Panel::new(settings.navigation_panel),
            status: String::new(),
            last_title: String::new(),
            quit_ok: false,
            busy: false,
            focus_search: false,
            palette: None,
            inspector: Panel::new(settings.inspector_panel),
            popout: false,
            fold: 0,
            bar_widths: BarWidths::default(),
            fitted: false,
            toast: None,
            recent,
            settings,
            sig_details: None,
            trust_ask: None,
            trusted: None,
            ids_window: None,
            sign_window: None,
            rescue: recover::Rescue::default(),
        }
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
            let open = || -> limarepdf_core::Result<(Pdf, Option<Arc<[u8]>>)> {
                use anyhow::Context as _;
                let bytes =
                    std::fs::read(&path).with_context(|| format!("reading {}", path.display()))?;
                let pdf = Pdf::from_bytes(&bytes, password.as_deref())?;
                // Signed files keep their bytes, to check and for View signed version.
                let fields = pdf.form_fields();
                let signed = fields
                    .iter()
                    .any(|f| f.kind == limarepdf_core::FieldKind::Signature)
                    .then(|| Arc::from(bytes));
                Ok((pdf, signed))
            };
            let msg = match crate::guard(open) {
                Ok((pdf, signed)) => Loaded::Ok {
                    path,
                    pdf,
                    page,
                    signed: signed.map(|b| (b, password)),
                },
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
    ) -> bool {
        let colours = self.colours(ctx);
        let tab = crate::guard(|| {
            Tab::new(ctx, self.next_id, pdf, path, colours).map_err(anyhow::Error::msg)
        });
        match tab {
            Ok(mut t) => {
                self.next_id += 1;
                // A repaired file counts as changed until the repaired copy is saved.
                t.dirty = dirty || t.pdf.repaired();
                t.ed.form.highlight = self.settings.fields_shown;
                t.ed.keep.clone_from(&self.settings.keep_tools);
                // The document's own viewer preferences, else the app's defaults. A page
                // asked for on the command line wins over the document's opening page.
                let prefs = t.pdf.view_prefs();
                (t.layout, t.cover) = match prefs.layout {
                    Some(l) => Layout::from_open(l, self.settings.cover),
                    None => (self.settings.layout, self.settings.cover),
                };
                if let Some(panel) = prefs.panel {
                    t.side = match panel {
                        OpenPanel::Pages => Side::Pages,
                        OpenPanel::Bookmarks => Side::Bookmarks,
                    };
                    // As the file asks; not saved as the user's choice unless they then
                    // change a panel themselves.
                    self.side.set_open(true);
                }
                if let Some(p) = page.map(|p| p.saturating_sub(1)).or(prefs.page) {
                    t.go_to(p);
                }
                self.tabs.push(t);
                self.active = self.tabs.len() - 1;
                true
            }
            Err(e) => {
                self.dialog = Dialog::Message("Cannot open".into(), format!("{e:#}"));
                false
            }
        }
    }

    /// Checks the digital signatures of tab `i`, whose file is `bytes`, in the
    /// background; its banner says so meanwhile.
    fn check_signatures(
        &mut self,
        ctx: &egui::Context,
        i: usize,
        bytes: Arc<[u8]>,
        password: Option<String>,
    ) {
        let Some(t) = self.tabs.get_mut(i) else {
            return;
        };
        let generation = t.sigs.as_ref().map_or(0, |s| s.generation + 1);
        t.sigs = Some(SigState {
            bytes: bytes.clone(),
            password: password.clone(),
            check: Check::Pending,
            generation,
        });
        // "Checking…" only when there is something to check, not for empty fields alone.
        if t.pdf.signature_count() > 0 {
            t.sig_banner();
        } else {
            t.banners.retain(|b| b.kind != BannerKind::Signatures);
        }
        let (tx, ctx, tab) = (self.tx.clone(), ctx.clone(), t.id);
        std::thread::spawn(move || {
            let check = sigs::run(&bytes, password.as_deref());
            let _ = tx.send(Loaded::Signatures {
                tab,
                generation,
                check,
            });
            ctx.request_repaint();
        });
    }

    /// After the trust list changed: judges every open document's signatures again and
    /// refreshes their banners.
    fn retrust(&mut self) {
        let (list, status) = sigs::load_trust();
        for t in &mut self.tabs {
            if let Some(SigState {
                check: Check::Done(c),
                ..
            }) = &mut t.sigs
            {
                c.report.apply_trust(&list);
                c.trust = status.clone();
                t.sig_banner();
            }
        }
        if let Some(w) = &mut self.trusted {
            *w = TrustedWindow::load();
        }
    }

    /// Adds the certificate `ask` settled on to the trust list file. On failure the
    /// reason is kept in `ask` for its window.
    fn trust_now(&mut self, ask: &mut TrustAsk) -> bool {
        let Some(cert) = ask.cert().cloned() else {
            return false;
        };
        let result = (|| -> limarepdf_core::Result<limarepdf_core::Added> {
            let path = limarepdf_core::TrustList::default_path().ok_or_else(|| {
                anyhow::anyhow!(
                    "LimarePDF can't find its settings folder (HOME or APPDATA isn't set)"
                )
            })?;
            let mut list = limarepdf_core::TrustList::load(&path)?;
            let label = ask.label.trim();
            let added = list.add(&cert.der, ask.role, (!label.is_empty()).then_some(label))?;
            list.save(&path)?;
            Ok(added)
        })();
        match result {
            Ok(added) => {
                let was = match added {
                    limarepdf_core::Added::RoleChanged(old) => format!(" (instead of as {old})"),
                    limarepdf_core::Added::New => String::new(),
                };
                let name = limarepdf_core::display_line(ask.label.trim(), 80);
                self.status = format!(
                    "Now trusting \"{name}\" as {}{was}. {}",
                    ask.role,
                    sigs::TRUST_NOTE
                );
                self.retrust();
                true
            }
            Err(e) => {
                ask.error = Some(format!("{e:#}"));
                false
            }
        }
    }

    /// The Details, Trusted certificates and trust question windows, when open; the
    /// question last, so it is drawn over the others.
    fn signature_windows(&mut self, ctx: &egui::Context) {
        if let Some((id, i)) = self.sig_details {
            let found = self.tabs.iter().position(|t| t.id == id);
            let shown = found.and_then(|ti| {
                let t = &self.tabs[ti];
                let c = t.sig_report()?;
                let out = sigs::details(ctx, &t.title(), c, i);
                let s = c.report.signatures.get(i);
                let signer = s.and_then(|s| s.signer.clone());
                let cas = s.map(sigs::authorities).unwrap_or_default();
                Some((ti, out, signer, cas))
            });
            match shown {
                None => self.sig_details = None,
                Some((ti, out, signer, cas)) => {
                    if out.close {
                        self.sig_details = None;
                    }
                    if out.trust_signer {
                        if let Some(cert) = signer {
                            self.trust_ask =
                                Some(TrustAsk::new(TrustRole::Signer, vec![cert], false));
                        }
                    }
                    if out.trust_authority && !cas.is_empty() {
                        self.trust_ask = Some(TrustAsk::new(TrustRole::Authority, cas, false));
                    }
                    if let Some((p, y)) = out.go_to {
                        self.active = ti;
                        self.tabs[ti].go_to_at(p, y);
                    }
                    if out.signed_version {
                        self.active = ti;
                        self.sig_details = None;
                        self.open_signed_version(ctx, i);
                    }
                }
            }
        }
        if let Some(mut w) = self.trusted.take() {
            match w.ui(ctx) {
                TrustedOut::Open => self.trusted = Some(w),
                TrustedOut::Close => {}
                TrustedOut::Add => {
                    self.trusted = Some(w);
                    self.add_certificate_file();
                }
                TrustedOut::Remove(i) => {
                    let sha = w.list.entries.get(i).map(|e| e.cert.sha256.clone());
                    self.trusted = Some(w);
                    if let Some(sha) = sha {
                        self.untrust(&sha);
                    }
                }
            }
        }
        if let Some(mut ask) = self.trust_ask.take() {
            match ask.ui(ctx) {
                Outcome::Apply if self.trust_now(&mut ask) => {}
                Outcome::Cancel => {}
                _ => self.trust_ask = Some(ask),
            }
        }
    }

    /// Asks for a certificate file and what to trust it as.
    fn add_certificate_file(&mut self) {
        let Some(file) = rfd::FileDialog::new()
            .add_filter("Certificates", &["cer", "crt", "pem", "der", "p7c", "p7b"])
            .pick_file()
        else {
            return;
        };
        let read = std::fs::read(&file)
            .map_err(anyhow::Error::from)
            .and_then(|b| crate::guard(|| limarepdf_core::read_certificates(&b)));
        match read {
            Ok(certs) if certs.is_empty() => {
                self.dialog = Dialog::Message(
                    "No certificate".into(),
                    format!("{} holds no certificates.", file.display()),
                )
            }
            // As a signer unless chosen otherwise: the narrower trust.
            Ok(certs) => self.trust_ask = Some(TrustAsk::new(TrustRole::Signer, certs, true)),
            Err(e) => {
                self.dialog = Dialog::Message(
                    "Can't read the certificate".into(),
                    format!("{}: {e:#}", file.display()),
                )
            }
        }
    }

    /// Takes the certificate with this SHA-256 fingerprint out of the trust list file.
    fn untrust(&mut self, sha256: &str) {
        let result = (|| -> limarepdf_core::Result<limarepdf_core::TrustEntry> {
            let path = limarepdf_core::TrustList::default_path().ok_or_else(|| {
                anyhow::anyhow!(
                    "LimarePDF can't find its settings folder (HOME or APPDATA isn't set)"
                )
            })?;
            let mut list = limarepdf_core::TrustList::load(&path)?;
            let i = list
                .find(sha256)
                .ok_or_else(|| anyhow::anyhow!("it is no longer in the trust list"))?;
            let e = list
                .remove(i)
                .ok_or_else(|| anyhow::anyhow!("it is no longer in the trust list"))?;
            list.save(&path)?;
            Ok(e)
        })();
        match result {
            Ok(e) => {
                self.status = format!(
                    "No longer trusting \"{}\". {}",
                    limarepdf_core::display_line(&e.label, 80),
                    sigs::TRUST_NOTE
                );
            }
            Err(e) => {
                self.dialog =
                    Dialog::Message("Can't change the trust list".into(), format!("{e:#}"))
            }
        }
        self.retrust();
    }

    /// Opens the version of the active document that its signature `i` signed, in a new
    /// tab of its own.
    fn open_signed_version(&mut self, ctx: &egui::Context, i: usize) {
        let Some(t) = self.tabs.get(self.active) else {
            return;
        };
        let (Some(state), Some(c)) = (t.sigs.as_ref(), t.sig_report()) else {
            return;
        };
        let Some(len) = c.report.signatures.get(i).and_then(|s| s.signed_len) else {
            return;
        };
        let Some(part) = state.bytes.get(..len) else {
            return;
        };
        let bytes: Arc<[u8]> = Arc::from(part);
        let password = state.password.clone();
        let label = format!("{} (signed version {})", t.stem("Untitled"), i + 1);
        match crate::guard(|| Pdf::from_bytes(&bytes, password.as_deref())) {
            Ok(pdf) => {
                if self.add_tab(ctx, pdf, None, None, false) {
                    self.tabs[self.active].label = Some(label);
                    self.check_signatures(ctx, self.active, bytes, password);
                }
            }
            Err(e) => {
                self.dialog =
                    Dialog::Message("Can't open the signed version".into(), format!("{e:#}"))
            }
        }
    }

    /// Chooses the digital signature tool for the active tab; the place comes next.
    fn sign_tool(&mut self, certify: bool) {
        if let Some(t) = self.tabs.get_mut(self.active) {
            t.ed.set_tool(Tool::DigitalSign);
            t.ed.digital_certify = certify;
            self.open_inspector();
        }
    }

    /// The digital signature tool's place was chosen: opens the signing window for it.
    fn open_sign_window(&mut self) {
        let Some(t) = self.tabs.get_mut(self.active) else {
            return;
        };
        let Some(place) = t.ed.digital_place.take() else {
            return;
        };
        let certify = t.ed.digital_certify;
        t.ed.set_tool(Tool::Read);
        if self.sign_window.is_some() {
            return;
        }
        let where_text = digital::where_text(&t.pdf, &place);
        let has_saved = !self
            .settings
            .sign
            .list(limarepdf_core::SignRole::Signature)
            .is_empty();
        self.sign_window = Some(SignWindow::new(
            t.id, t.revision, place, where_text, certify, has_saved,
        ));
    }

    /// The signing window and the Digital IDs window, while open. The IDs window, opened from
    /// the signing window, is drawn after it, so it is the one that takes input.
    fn digital_windows(&mut self, ctx: &egui::Context) {
        self.digital_sign_window(ctx);
        if let Some(mut w) = self.ids_window.take() {
            let out = w.ui(ctx, &self.settings.author());
            if out.changed {
                if let Some(s) = self.sign_window.as_mut() {
                    s.reload();
                }
            }
            if !out.close {
                self.ids_window = Some(w);
            }
        }
    }

    fn digital_sign_window(&mut self, ctx: &egui::Context) {
        let Some(mut w) = self.sign_window.take() else {
            return;
        };
        match w.ui(ctx) {
            SignOut::Open => self.sign_window = Some(w),
            SignOut::Cancel => {}
            SignOut::ManageIds => {
                if self.ids_window.is_none() {
                    self.ids_window = Some(IdsWindow::new());
                }
                self.sign_window = Some(w);
            }
            SignOut::Go => {
                self.start_signing(ctx, &mut w);
                self.sign_window = Some(w);
            }
            SignOut::Done(s) => self.signed(ctx, *s, w.tab, w.revision),
        }
    }

    /// Gathers what signing needs from the window's tab, asks where to save the signed
    /// document, and starts signing in the background.
    fn start_signing(&mut self, ctx: &egui::Context, w: &mut SignWindow) {
        let Some(t) = self.tabs.iter().find(|t| t.id == w.tab) else {
            w.error = Some("The document's tab was closed.".into());
            return;
        };
        if t.revision != w.revision {
            w.error = Some(
                "The document changed after you chose where to sign. Cancel, and choose the \
                 place again."
                    .into(),
            );
            return;
        }
        let gathered = crate::guard(|| {
            if t.pdf.encrypted() {
                anyhow::bail!("LimarePDF can't sign encrypted (password-protected) documents");
            }
            if t.encryption.is_some() {
                anyhow::bail!(PASSWORD_NOT_SIGNED);
            }
            // A document opened with signature fields kept its bytes: the signature is
            // added after them, with the edits made since. Any other document has no
            // signatures to keep, so it is written out as it is now and signed. So is one
            // that had something removed for good, which its opened bytes still hold.
            if t.scrubbed && t.signed {
                anyhow::bail!(
                    "You applied redactions or removed hidden information, and the version \
                     already signed still holds everything removed. Save the document first \
                     (which writes the whole file again and breaks the earlier signatures), \
                     then sign it."
                );
            }
            let (base, edited): (Arc<[u8]>, _) = match &t.sigs {
                Some(s) if !t.scrubbed => (s.bytes.clone(), t.dirty.then(|| t.pdf.clone())),
                _ => (Arc::from(t.pdf.clone().to_bytes()?), None),
            };
            let art = match self
                .settings
                .sign
                .list(limarepdf_core::SignRole::Signature)
                .first()
            {
                Some(e) if w.use_saved => {
                    let bytes = e.saved.bytes().map_err(|e| {
                        anyhow::anyhow!("LimarePDF can't read your saved signature's picture: {e}")
                    })?;
                    Some((e.saved.clone(), bytes))
                }
                _ => None,
            };
            Ok((base, edited, art))
        });
        let (base, edited, art) = match gathered {
            Ok(g) => g,
            Err(e) => {
                w.error = Some(format!("{e:#}"));
                return;
            }
        };
        let mut d = Self::pdf_dialog().set_file_name(format!("{}-signed.pdf", t.stem("document")));
        if let Some(dir) = t.path.as_ref().and_then(|p| p.parent()) {
            d = d.set_directory(dir);
        }
        let Some(mut out) = d.save_file() else {
            return;
        };
        if out.extension().is_none() {
            out.set_extension("pdf");
        }
        w.start(
            ctx,
            digital::Input {
                base,
                edited,
                art,
                out,
            },
        );
    }

    /// A document was signed and written: it opens in place of the tab it was signed
    /// from, unless that tab changed or closed meanwhile. Replacing it would lose a change
    /// to the document, a password set (signing never starts with one set, so any is
    /// new), or text being typed.
    fn signed(&mut self, ctx: &egui::Context, s: digital::Signed, tab: u64, revision: u64) {
        self.recent.add(&s.path);
        let replace = self.tabs.iter().position(|t| {
            t.id == tab && t.revision == revision && t.encryption.is_none() && !t.ed.pending()
        });
        let page = replace.map(|i| self.tabs[i].current);
        let what = if s.certified { "Certified" } else { "Signed" };
        let mut text = format!(
            "{what} as \"{}\" in the field \"{}\", and saved to {}.",
            s.signer,
            limarepdf_core::display_line(&s.field, 60),
            s.path.display()
        );
        if s.rewritten {
            text.push_str(
                "\n\nThe file's structure was damaged, or other PDF apps may show it \
                 differently, so LimarePDF wrote it out in full, as shown here, before signing.",
            );
        }
        if s.finished_items > 0 {
            text.push_str(&format!(
                "\n\n{} were drawn into the pages first.",
                plural(
                    s.finished_items,
                    "1 fill-and-sign item",
                    "{n} fill-and-sign items"
                )
            ));
        }
        if !s.confirmed {
            text.push_str(&format!("\n\n{NOT_CONFIRMED}"));
        }
        if s.self_issued {
            text.push_str(
                "\n\nOthers see you as an unknown signer until they trust your certificate: \
                 send it to them from Edit > Digital IDs (Export, certificate only). LimarePDF shows \
                 the same until you trust it.",
            );
        }
        let bytes: Arc<[u8]> = Arc::from(s.bytes);
        match crate::guard(|| Pdf::from_bytes(&bytes, None)) {
            Ok(pdf) => {
                if self.add_tab(ctx, pdf, Some(s.path), None, false) {
                    if let Some(i) = replace {
                        // add_tab put it last: it takes the signed tab's place.
                        if let Some(new) = self.tabs.pop() {
                            let old = std::mem::replace(&mut self.tabs[i], new);
                            self.active = i;
                            self.closed_copy(&old);
                        }
                    }
                    if let Some(p) = page {
                        self.tabs[self.active].go_to(p);
                    }
                    // Saving again is one click (decision D7).
                    self.tabs[self.active].dirty = !s.confirmed;
                    self.check_signatures(ctx, self.active, bytes, None);
                }
                if replace.is_none() {
                    text.push_str(
                        "\n\nThe document changed while it was being signed, so the signed \
                         file opened in a new tab; the changes made meanwhile aren't in it.",
                    );
                }
            }
            Err(e) => text.push_str(&format!("\n\nLimarePDF can't open the signed file: {e:#}")),
        }
        self.dialog = Dialog::Message(what.into(), text);
    }

    fn poll_loads(&mut self, ctx: &egui::Context) {
        while let Ok(m) = self.rx.try_recv() {
            match m {
                Loaded::Ok {
                    path,
                    pdf,
                    page,
                    signed,
                } => {
                    self.recent.add(&path);
                    if self.add_tab(ctx, pdf, Some(path), page, false) {
                        if let Some((bytes, password)) = signed {
                            self.check_signatures(ctx, self.active, bytes, password);
                        }
                    }
                }
                Loaded::Signatures {
                    tab,
                    generation,
                    check,
                } => {
                    if let Some(t) = self.tabs.iter_mut().find(|t| t.id == tab) {
                        t.sig_checked(generation, check);
                    }
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
        work: impl FnOnce(&mut Pdf) -> limarepdf_core::Result<(bool, String)> + Send + 'static,
    ) {
        let t = &self.tabs[self.active];
        let (tab, revision, mut pdf) = (t.id, t.revision, t.pdf.clone());
        let (tx, ctx) = (self.tx.clone(), ctx.clone());
        std::thread::spawn(move || {
            let result = crate::guard(|| work(&mut pdf))
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

    /// Send `pages` (zero-based) of the active tab to the printer on a background thread.
    fn start_print(&mut self, ctx: &egui::Context, pages: Vec<usize>, opts: print::PrintOptions) {
        let printer = opts.printer.clone().unwrap_or_default();
        self.spawn_job(ctx, "Print", move |p| {
            let reply = p.print(&pages, &opts)?;
            let n = pages.len();
            Ok((false, format!("Sent {n} page(s) to {printer}.\n{reply}")))
        });
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
        let result: limarepdf_core::Result<()> = match layout {
            dlg::Layout::TextFile => std::fs::write(&path, text).map_err(Into::into),
            _ => crate::guard(|| {
                Excerpt::new(t, layout)
                    .build(&t.pdf)
                    .and_then(|mut d| d.save(&path))
            }),
        };
        match result {
            Ok(()) => self.status = format!("Saved the selected text to {}", path.display()),
            Err(e) if not_confirmed(&e) => {
                self.dialog = Dialog::Message("Saved, not confirmed".into(), NOT_CONFIRMED.into())
            }
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
            Ok(()) => {
                self.add_tab(ctx, pdf, None, None, true);
            }
            Err(e) => {
                self.dialog = Dialog::Message("Cannot import images".into(), format!("{e:#}"))
            }
        }
    }

    /// Saves, with what is being typed kept first. Saving over a file that was repaired to
    /// open it asks first, offering Save as instead: the original is only replaced when the
    /// user says so.
    fn save(&mut self, ctx: &egui::Context, as_new: bool) {
        if let Some(t) = self.tabs.get_mut(self.active) {
            t.keep_typing(ctx);
        }
        let over_repaired = self
            .tabs
            .get(self.active)
            .filter(|t| t.repaired && t.path.is_some());
        match over_repaired {
            Some(t) if !as_new => self.dialog = Dialog::ConfirmSaveRepaired(Asked::of(t)),
            _ => self.save_unmarked(ctx, as_new),
        }
    }

    /// Saves, first asking when the document has redaction marks not yet applied; a
    /// signed document then goes through [`Self::save_signed`].
    fn save_unmarked(&mut self, ctx: &egui::Context, as_new: bool) {
        let Some(t) = self.tabs.get(self.active) else {
            return;
        };
        let marks = t.pdf.redaction_marks().len();
        if marks > 0 {
            self.dialog = Dialog::ConfirmSaveMarks(Asked::of(t), as_new, marks);
        } else {
            self.save_signed(ctx, as_new);
        }
    }

    /// A signed document is saved by adding the edits after what was signed, so that its
    /// signatures stay valid; this asks first when they don't allow the edits. When the
    /// edits can't be added that way, it asks before writing the whole file, which breaks
    /// the signatures.
    fn save_signed(&mut self, ctx: &egui::Context, as_new: bool) {
        let Some(tab) = self.tabs.get_mut(self.active) else {
            return;
        };
        if !tab.signed {
            return self.save_now(ctx, as_new);
        }
        let appended = if tab.scrubbed {
            Err(SCRUBBED.to_string())
        } else if tab.recovered.is_some() {
            Err(RECOVERED.to_string())
        } else if tab.repaired {
            Err(REPAIRED.to_string())
        } else if let Some(s) = &tab.sigs {
            tab.pdf.set_encryption(tab.encryption.clone());
            crate::guard(|| limarepdf_core::append_edits(&s.bytes, &tab.pdf))
                .map_err(|e| format!("{e:#}"))
        } else {
            Err("LimarePDF doesn't have the file as it was signed".to_string())
        };
        match appended {
            Ok(a) => match a.disallowed {
                None => self.save_appended(ctx, as_new, a.bytes),
                Some(disallowed) => {
                    self.dialog = Dialog::ConfirmSaveDisallowed(Box::new(SaveDisallowed {
                        as_new,
                        asked: Asked::of(tab),
                        bytes: a.bytes,
                        disallowed,
                    }))
                }
            },
            Err(why) => self.dialog = Dialog::ConfirmSaveSigned(Asked::of(tab), as_new, why),
        }
    }

    /// Where to save the tab: its file, or one the user picks (for Save as, or a document
    /// with no file yet).
    fn save_path(tab: &Tab, as_new: bool) -> Option<PathBuf> {
        if !as_new {
            if let Some(p) = &tab.path {
                return Some(p.clone());
            }
        }
        let mut d = Self::pdf_dialog();
        if let Some(n) = tab.path.as_ref().and_then(|p| p.file_name()) {
            d = d.set_file_name(n.to_string_lossy());
        } else if let Some(l) = &tab.label {
            d = d.set_file_name(format!("{l}.pdf"));
        }
        // A recovered copy is suggested beside its original.
        let original = tab.recovered.as_ref().and_then(|r| r.original.as_ref());
        if let Some(dir) = original.and_then(|p| p.parent()).filter(|d| d.is_dir()) {
            d = d.set_directory(dir);
        }
        let mut p = d.save_file()?;
        if p.extension().is_none() {
            p.set_extension("pdf");
        }
        Some(p)
    }

    /// Writes a signed document whose edits were added after what was signed (`bytes`).
    /// The signatures stay, and the saved file becomes what later saves add to.
    fn save_appended(&mut self, ctx: &egui::Context, as_new: bool, bytes: Vec<u8>) {
        let Some(tab) = self.tabs.get_mut(self.active) else {
            return;
        };
        let Some(path) = Self::save_path(tab, as_new) else {
            return;
        };
        let confirmed = match limarepdf_core::write_file(&path, &bytes) {
            Ok(()) => true,
            Err(e) if not_confirmed(&e) => false,
            Err(e) => {
                self.dialog = Dialog::Message("Save failed".into(), format!("{e:#}"));
                return;
            }
        };
        self.status = format!(
            "Saved {}; the changes were added after what was signed, so the signatures stay",
            path.display()
        );
        self.recent.add(&path);
        tab.dirty = !confirmed;
        tab.scrubbed = false;
        tab.path = Some(path);
        self.check_signatures(ctx, self.active, Arc::from(bytes), None);
        self.saved_copy(self.active, confirmed);
        if !confirmed {
            self.dialog = Dialog::Message("Saved, not confirmed".into(), NOT_CONFIRMED.into());
        }
    }

    fn save_now(&mut self, ctx: &egui::Context, as_new: bool) {
        let Some(tab) = self.tabs.get_mut(self.active) else {
            return;
        };
        let Some(path) = Self::save_path(tab, as_new) else {
            return;
        };
        tab.pdf.set_encryption(tab.encryption.clone());
        let confirmed = match crate::guard(|| tab.pdf.save(&path)) {
            Ok(()) => true,
            Err(e) if not_confirmed(&e) => false,
            Err(e) => {
                self.dialog = Dialog::Message("Save failed".into(), format!("{e:#}"));
                return;
            }
        };
        self.status = format!("Saved {}", path.display());
        self.recent.add(&path);
        tab.dirty = !confirmed;
        tab.scrubbed = false;
        // The saved file is the repaired copy, with nothing left to repair, and it reads the
        // same in every app.
        tab.repaired = false;
        tab.banners
            .retain(|b| !matches!(b.kind, BannerKind::Repaired | BannerKind::OtherApps));
        // The saved copy's signatures are already broken: no need to ask again.
        tab.signed = false;
        // The Signatures tab describes the file, so check the saved one (a saved recovered
        // copy's too, which was never checked).
        let password = tab.encryption.as_ref().map(|e| e.user_password.clone());
        let recheck =
            tab.sigs.is_some() || (tab.recovered.is_some() && tab.pdf.signature_count() > 0);
        let saved = recheck.then(|| std::fs::read(&path));
        tab.path = Some(path);
        match saved {
            Some(Ok(bytes)) => self.check_signatures(ctx, self.active, Arc::from(bytes), password),
            Some(Err(e)) => {
                let t = &mut self.tabs[self.active];
                if let Some(s) = t.sigs.as_mut() {
                    s.check =
                        Check::Failed(format!("LimarePDF can't read the saved file again: {e}"));
                    t.sig_banner();
                }
            }
            None => {}
        }
        self.saved_copy(self.active, confirmed);
        if !confirmed {
            self.dialog = Dialog::Message("Saved, not confirmed".into(), NOT_CONFIRMED.into());
        }
    }

    /// Run a command (`name` is for messages) so that a panic in it can't close the app with
    /// every tab's unsaved work. Afterwards the app's state is made valid again, and the active
    /// tab is checked against how it was before: unchanged, the failure is only reported;
    /// changed, the tab can't be trusted and shows its error page (Save a copy, Close). Other
    /// tabs are never touched.
    fn run_command(
        &mut self,
        ctx: &egui::Context,
        name: &str,
        f: impl FnOnce(&mut Self, &egui::Context),
    ) {
        let before = self.tabs.get(self.active).map(|t| (t.id, t.state()));
        let Err(e) = crate::guard(|| {
            f(self, ctx);
            Ok(())
        }) else {
            return;
        };
        self.active = self.active.min(self.tabs.len().saturating_sub(1));
        // A dialog the command opened may hold half-built data.
        self.dialog = Dialog::None;
        let tab = before.and_then(|(id, state)| {
            let t = self.tabs.iter_mut().find(|t| t.id == id)?;
            t.reset_interaction();
            Some((t.state() != state, t))
        });
        match tab {
            Some((true, t)) => {
                t.broken = Some(format!(
                    "{name} failed part way, so this tab may not show the document as it is \
                     now. {e}"
                ));
            }
            _ => {
                self.dialog = Dialog::Message(
                    format!("{name} failed"),
                    format!("{e} Nothing was changed."),
                );
            }
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
            Act::Save => self.save(ctx, false),
            Act::SaveAs => self.save(ctx, true),
            Act::ExportImages if has => self.reveal(Section::Export),
            Act::ExportText if has => self.export_text(),
            Act::Print | Act::PrintSelection if has && !self.busy => {
                let t = &self.tabs[self.active];
                let selected = t.ed.selection.pages();
                let file = t.title();
                let selection = (a == Act::PrintSelection).then_some(dlg::Layout::Original);
                self.dialog = match print::printers() {
                    Ok(list) => Dialog::Print(
                        self.asked(),
                        Box::new(dlg::Print {
                            all: selected.is_none(),
                            pages: selected.map(inspector::range_text).unwrap_or_default(),
                            selection,
                            ..dlg::Print::new(list, file)
                        }),
                    ),
                    Err(e) => Dialog::Message("Print".into(), format!("{e:#}")),
                }
            }
            Act::SaveSelection if has => {
                self.dialog = Dialog::SaveSelection(self.asked(), dlg::Layout::Original);
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
            Act::MarkRedact if has => {
                let author = self.settings.author();
                self.tabs[self.active].mark_selection_for_redaction(ctx, &author);
            }
            Act::MarkResultsRedact if has => {
                let author = self.settings.author();
                self.tabs[self.active].mark_results_for_redaction(ctx, &author);
            }
            Act::RemoveHidden if has => match self.tabs[self.active].pdf.find_hidden() {
                Ok(found) => {
                    let kinds = limarepdf_core::Hidden::ALL
                        .into_iter()
                        .map(|h| (h, found.count(h), h.default_on()))
                        .collect();
                    self.dialog = Dialog::RemoveHidden(self.asked(), dlg::RemoveHidden { kinds });
                }
                Err(e) => {
                    self.dialog =
                        Dialog::Message("Remove hidden information".into(), format!("{e:#}"))
                }
            },
            Act::ApplyRedactions if has => {
                let n = self.tabs[self.active].pdf.redaction_marks().len();
                if n == 0 {
                    self.tabs[self.active].notice = Some(
                        "There are no redaction marks: mark areas with Annotate > Mark areas for \
                         redaction, or mark selected text or search results"
                            .into(),
                    );
                } else {
                    self.dialog = Dialog::ConfirmApplyRedactions(self.asked(), n);
                }
            }
            Act::FinishSigning if has => {
                let n = self.tabs[self.active].pdf.sign_items();
                if n == 0 {
                    self.tabs[self.active].notice = Some(
                        "There is nothing to finish: put down a signature, the date or a tick \
                         with the Sign tool first"
                            .into(),
                    );
                } else {
                    self.dialog = Dialog::ConfirmFinishSigning(self.asked(), n);
                }
            }
            Act::SignDigital | Act::Certify if has => {
                let certify = a == Act::Certify;
                let title = if certify {
                    "Certify"
                } else {
                    "Sign with digital ID"
                };
                let t = &mut self.tabs[self.active];
                if self.sign_window.is_some() {
                    // One signing at a time; the open window says where it is.
                } else if t.pdf.encrypted() {
                    self.dialog = Dialog::Message(
                        title.into(),
                        "LimarePDF can't sign encrypted (password-protected) documents. Save a copy \
                         without the password (Document > Password security) and sign that \
                         instead."
                            .into(),
                    );
                } else if t.encryption.is_some() {
                    self.dialog = Dialog::Message(title.into(), PASSWORD_NOT_SIGNED.into());
                } else if certify && t.pdf.signature_count() > 0 {
                    self.dialog = Dialog::Message(
                        title.into(),
                        "Only a document no one has signed yet can be certified. Use Sign \
                         with digital ID instead."
                            .into(),
                    );
                } else if t.recovered.is_some() && t.signed {
                    self.dialog = Dialog::ConfirmSignRecovered(Asked::of(t));
                } else {
                    self.sign_tool(certify);
                }
            }
            Act::DigitalPlaced if has => self.open_sign_window(),
            Act::DigitalIds => {
                if self.ids_window.is_none() {
                    self.ids_window = Some(IdsWindow::new());
                }
            }
            Act::CloseTab if has => {
                if self.tabs[self.active].unsaved() {
                    self.dialog = Dialog::ConfirmClose(self.asked());
                } else {
                    self.close_tab(self.active);
                }
            }
            Act::ToggleRecovery => self.toggle_recovery(),
            Act::ShowRecoveryFile => self.show_leftover(),
            Act::Quit => {
                if self.tabs.iter().any(Tab::unsaved) {
                    self.dialog = Dialog::ConfirmQuit;
                } else {
                    self.quit_ok = true;
                    ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                }
            }
            Act::About => self.dialog = Dialog::About(about::Page::Main),
            Act::Theme(p) => {
                ctx.set_theme(p);
                self.settings.theme = p;
                self.settings.save();
            }
            // Open tabs pick the change up at the start of the next frame.
            Act::PageColours(look) => {
                self.settings.page_look = look;
                self.settings.save();
            }
            Act::TogglePicturesToo => {
                self.settings.pictures_too = !self.settings.pictures_too;
                self.settings.save();
            }
            Act::ToggleSide => {
                self.side.toggle();
                self.keep_panels();
            }
            Act::ToggleInspector => {
                self.inspector.toggle();
                self.keep_panels();
            }
            Act::ToggleDock => {
                self.settings.controls_docked = !self.settings.controls_docked;
                self.settings.save();
            }
            Act::Palette => self.palette = Some(Palette::default()),
            Act::TrustedCertificates => self.trusted = Some(TrustedWindow::load()),
            _ if !has => {}
            Act::PageLabels => self.reveal_labels(),
            Act::PagesView => self.show_view(Side::Pages),
            Act::SearchView => self.show_view(Side::Search),
            Act::Bookmarks => self.show_view(Side::Bookmarks),
            Act::Comments => self.show_view(Side::Comments),
            Act::Signatures => self.show_view(Side::Signatures),
            Act::SignatureDetails(i) => {
                self.sig_details = Some((self.tabs[self.active].id, i));
            }
            Act::SignedVersion(i) => self.open_signed_version(ctx, i),
            Act::ToggleHidden => {
                let ed = &mut self.tabs[self.active].ed;
                ed.show_hidden = !ed.show_hidden;
            }
            // For every document, and remembered between runs.
            Act::ToggleFields => {
                let s = &mut self.settings;
                s.fields_shown = !s.fields_shown;
                s.save();
                for t in &mut self.tabs {
                    t.ed.form.highlight = s.fields_shown;
                }
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
            Act::FlattenForm => self.dialog = Dialog::ConfirmFlatten(self.asked()),
            Act::FlattenAnnots => {
                if self.tabs[self.active].pdf.annotations().is_empty() {
                    self.tabs[self.active].notice =
                        Some("There are no annotations to flatten".into());
                } else {
                    self.dialog = Dialog::ConfirmFlattenAnnots(self.asked());
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
            // Setting the crop on the pages: Esc closes the Crop section, showing the pages
            // as cropped again.
            Act::Escape if self.tabs[self.active].crop_view() => {
                self.tabs[self.active].forms.close = Some(inspector::Section::Crop);
            }
            Act::Escape => self.tabs[self.active].ed.escape(),
            Act::NextPage | Act::PrevPage | Act::FirstPage | Act::LastPage => {
                let t = &mut self.tabs[self.active];
                let (layout, cover) = (t.layout, t.cover);
                let last = t.page_count().saturating_sub(1);
                let to = match a {
                    Act::NextPage => t.step_page(1, layout, cover),
                    Act::PrevPage => t.step_page(-1, layout, cover),
                    Act::FirstPage => 0,
                    _ => last,
                };
                t.go_to(to);
            }
            // The active document changes, kept at the page it was on; the choice also
            // becomes the default for documents opened later without a preference.
            Act::Layout(l) => {
                self.settings.layout = l;
                self.settings.save();
                if let Some(t) = self.tabs.get_mut(self.active) {
                    t.layout = l;
                    t.go_to(t.current);
                }
            }
            Act::ToggleCover => {
                let cover = match self.tabs.get(self.active) {
                    Some(t) => !t.cover,
                    None => !self.settings.cover,
                };
                self.settings.cover = cover;
                self.settings.save();
                if let Some(t) = self.tabs.get_mut(self.active) {
                    t.cover = cover;
                    t.go_to(t.current);
                }
            }
            Act::Undo => self.tabs[self.active].undo(ctx),
            Act::Redo => self.redo(ctx),
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
                self.show_view(Side::Search);
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
            Act::RemoveBlank => self.spawn_job(ctx, "Remove blank pages", |p| {
                let gone = p.remove_blank_pages(limarepdf_core::BLANK_TOLERANCE)?;
                let text = match gone.len() {
                    0 => "No blank pages were found.".to_string(),
                    n => format!(
                        "Removed {n} blank page(s): {}. Undo brings them back.",
                        limarepdf_core::format_page_range(&gone)
                    ),
                };
                Ok((!gone.is_empty(), text))
            }),
            Act::Extract => {
                let t = &mut self.tabs[self.active];
                // Pages from a password-protected document get no recovery copies either.
                let protected = t.copy.protected || t.encryption.is_some() || t.pdf.encrypted();
                match t.pdf.extract(&t.targets()) {
                    Ok(p) => {
                        if self.add_tab(ctx, p, None, None, true) {
                            self.tabs[self.active].copy.protected = protected;
                        }
                    }
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
                if let Some(files) = Self::pdf_dialog().pick_files() {
                    self.merge_files(ctx, &files);
                }
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
                self.open_inspector();
                self.tabs[self.active].ed.focus_text_field();
            }
            Act::MoveText => self.tabs[self.active].ed.move_text(),
            Act::SignWith(pick) => {
                let ed = &mut self.tabs[self.active].ed;
                ed.set_tool(Tool::Sign);
                ed.sign.pick = pick;
            }
            Act::CreateSign(role) => {
                let ed = &mut self.tabs[self.active].ed;
                ed.set_tool(Tool::Sign);
                let author = self.settings.author();
                ed.sign.start_create(role, &author, None);
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
    /// Open the inspector's page labels for the selected pages, or the current page.
    fn reveal_labels(&mut self) {
        self.reveal(Section::Labels);
        let t = &mut self.tabs[self.active];
        if t.ed.selection.pages().is_none() {
            t.ed.selection = Selection::pages_from([t.current].into());
        }
    }

    /// Show one of the navigation panel's views, opening the panel.
    fn show_view(&mut self, side: Side) {
        self.side.set_open(true);
        self.keep_panels();
        self.tabs[self.active].side = side;
    }

    /// Open the inspector, as the user's choice.
    fn open_inspector(&mut self) {
        self.inspector.set_open(true);
        self.keep_panels();
    }

    /// Remember the user's choice of open or closed panels, when it changed.
    fn keep_panels(&mut self) {
        let s = &mut self.settings;
        let now = (self.side.wanted(), self.inspector.wanted());
        if (s.navigation_panel, s.inspector_panel) != now {
            (s.navigation_panel, s.inspector_panel) = now;
            s.save();
        }
    }

    fn reveal(&mut self, s: Section) {
        self.open_inspector();
        let ed = &mut self.tabs[self.active].ed;
        ed.selection.clear_object();
        ed.selection.clear_words();
        if ed.owns_panel() {
            ed.set_tool(Tool::Read);
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

    /// Removes everything under the active tab's redaction marks, once its recovery copies
    /// are withdrawn (recovery design, section 4).
    fn apply_redactions(&mut self, ctx: &egui::Context) {
        if let Err(stop) = self.withdraw_copies(self.active, "applied the redactions") {
            self.dialog = stop;
            return;
        }
        let t = &mut self.tabs[self.active];
        let mut report = None;
        let ok = t.edit_removing(ctx, |p| {
            let r = p.apply_redactions()?;
            if !r.left.is_empty() {
                anyhow::bail!(
                    "Nothing was changed: the check after redacting found {}",
                    r.left.join("; ")
                );
            }
            report = Some(r);
            Ok(())
        });
        if let Some(r) = report.filter(|_| ok) {
            self.dialog = Dialog::Message("Redactions applied".into(), report_text(&r));
        }
    }

    /// Redoes the active tab's last undone change. Redoing a redaction or a removal of
    /// hidden information removes the content again, so its recovery copies are withdrawn
    /// first, as before the command itself: a copy written after the undo holds the content.
    fn redo(&mut self, ctx: &egui::Context) {
        if self.tabs[self.active].redo_removes() {
            if let Err(stop) = self.withdraw_copies(self.active, "redone the change") {
                self.dialog = stop;
                return;
            }
        }
        self.tabs[self.active].redo(ctx);
    }

    /// Adds the pages of `files` to the end of the active tab. Pages from a
    /// password-protected document (one that opens without asking) make the tab protected:
    /// its recovery copies are withdrawn first, as for a password, and it gets no more.
    fn merge_files(&mut self, ctx: &egui::Context, files: &[PathBuf]) {
        let mut docs = Vec::new();
        for f in files {
            match crate::guard(|| Pdf::open(f, None)) {
                Ok(d) => docs.push(d),
                Err(e) => {
                    self.dialog =
                        Dialog::Message("Merge failed".into(), format!("{}: {e:#}", f.display()));
                    return;
                }
            }
        }
        let protected = docs.iter().any(Pdf::encrypted);
        if protected {
            let done = "added the pages of the password-protected document";
            if let Err(stop) = self.withdraw_copies(self.active, done) {
                self.dialog = stop;
                return;
            }
        }
        let t = &mut self.tabs[self.active];
        if t.edit(ctx, |p| p.append(docs)) && protected {
            t.copy.protected = true;
        }
    }

    /// Removes the kinds of hidden information `what` from the active tab, once its
    /// recovery copies are withdrawn. A removal that would be refused (it couldn't remove
    /// all of a kind) is refused first, with nothing changed and the copies kept.
    fn remove_hidden(&mut self, ctx: &egui::Context, what: &[limarepdf_core::Hidden]) {
        if let Err(e) = self.tabs[self.active].pdf.check_remove_hidden(what) {
            self.dialog = Dialog::Message("Remove hidden information".into(), format!("{e:#}"));
            return;
        }
        if let Err(stop) = self.withdraw_copies(self.active, "removed the hidden information") {
            self.dialog = stop;
            return;
        }
        let t = &mut self.tabs[self.active];
        let mut report = None;
        let ok = t.edit_removing(ctx, |p| {
            report = Some(p.remove_hidden(what)?);
            Ok(())
        });
        if let Some(r) = report.filter(|_| ok) {
            let mut text = String::from("Removed:\n");
            for (h, n) in &r.removed {
                text.push_str(&format!("• {} ({n})\n", h.label()));
            }
            if let Some(note) = r.note() {
                text.push_str(&format!("\n{note}\n"));
            }
            text.push_str("\nSave the document to keep the changes; Undo brings it all back.");
            self.dialog = Dialog::Message("Hidden information removed".into(), text);
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
            Apply::Label(joins) => self.tabs[active].keep_label(ctx, joins),
            Apply::RemoveLabels => {
                self.tabs[active].edit_setting(ctx, None, |p| p.set_label_ranges(&[]));
            }
            Apply::Props => self.tabs[active].keep_props(ctx),
            Apply::ViewPrefs(joins) => self.tabs[active].keep_view(ctx, joins),
            Apply::Security => {
                let encryption = self.tabs[active].forms.sec.encryption();
                // No recovery copy made before may outlast the password (recovery design,
                // section 4), and none is made after it.
                if encryption.is_some() {
                    if let Err(stop) = self.withdraw_copies(active, "added the password") {
                        self.dialog = stop;
                        return;
                    }
                }
                let t = &mut self.tabs[active];
                t.copy.protected |= encryption.is_some();
                t.banners.retain(|b| b.kind != BannerKind::Protected);
                t.set_encryption(encryption);
                self.status = "Security settings apply when you save.".into();
                self.notify(ctx, "Changed password security".into());
            }
            Apply::Crop(joins) => {
                let t = &mut self.tabs[active];
                let s = &t.forms.crop;
                match parse_page_range(&s.pages, n) {
                    Ok(idx) => {
                        let m = s.margins;
                        t.crop(ctx, &idx, m, joins);
                    }
                    Err(e) => t.error = Some(e.to_string()),
                }
            }
            Apply::Stamp { kind, update } => {
                self.tabs[active].stamp_from_form(ctx, kind, update);
            }
            Apply::Ocr => match parse_page_range(&self.tabs[active].forms.ocr, n) {
                Ok(idx) => self.spawn_job(ctx, OCR_JOB, move |p| {
                    let ocr = limarepdf_core::Ocr::bundled()?;
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
                    self.dialog =
                        Dialog::ConfirmReplace(Asked::of(&self.tabs[active]), e, existing);
                } else {
                    self.start_export(ctx, e);
                }
            }
            Apply::Split => {
                let f = &self.tabs[active].forms;
                let level = f.split_at_marks.then_some(f.split_level.max(1));
                let every = f.split.max(1);
                let Some(dir) = rfd::FileDialog::new()
                    .set_title("Choose output folder")
                    .pick_folder()
                else {
                    return;
                };
                let stem = self.tabs[active].stem("part");
                self.spawn_job(ctx, "Split into files", move |p| {
                    let by = match level {
                        Some(level) => SplitBy::Bookmarks(level),
                        None => SplitBy::Every(every),
                    };
                    match p.split(by, |n, title| dir.join(part_file_name(&stem, n, title))) {
                        Ok(n) => Ok((false, format!("Wrote {n} files to {}.", dir.display()))),
                        Err(e) if not_confirmed(&e) => Ok((
                            false,
                            format!(
                                "Wrote the files to {}, but your system couldn't confirm they \
                                 are safely on disk. If the computer lost power just now, \
                                 split the document again.",
                                dir.display()
                            ),
                        )),
                        Err(e) => Err(e),
                    }
                });
            }
        }
    }

    /// What a question about the tab in front is tied to.
    fn asked(&self) -> Asked {
        Asked::of(&self.tabs[self.active])
    }

    /// Whether the tab a question was asked about is still open and unchanged. If it is,
    /// it comes to the front for the answer to act on; if not, nothing is done, and a
    /// message says why.
    fn still(&mut self, a: &Asked) -> bool {
        let found = self.tabs.iter().position(|t| t.id == a.tab);
        if let Some(i) = found.filter(|&i| self.tabs[i].state() == *a.state) {
            self.active = i;
            return true;
        }
        let what = if found.is_some() {
            "changed"
        } else {
            "was closed"
        };
        self.dialog = Dialog::Message(
            "Nothing was done".into(),
            format!(
                "The document {what} while the question was open, so nothing was done. \
                 Choose the command again if you still want it."
            ),
        );
        false
    }

    fn close_tab(&mut self, i: usize) {
        if i < self.tabs.len() {
            let tab = self.tabs.remove(i);
            self.active = self.active.min(self.tabs.len().saturating_sub(1));
            self.closed_copy(&tab);
            // Dropped last (stopping its render worker), with the app's state already valid.
            drop(tab);
        }
    }

    fn dialogs(&mut self, ctx: &egui::Context) {
        let mut dialog = std::mem::replace(&mut self.dialog, Dialog::None);
        let mut done = false;
        // The document a question is about, named under its title.
        let file = dialog
            .asked()
            .and_then(|a| self.tabs.iter().find(|t| t.id == a.tab))
            .map(Tab::title);
        let file = file.as_deref();
        match &mut dialog {
            Dialog::None => done = true,
            Dialog::About(page) => done = about::show(ctx, page),
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
                Ask::new(
                    "Open link?",
                    &format!(
                        "This link goes to:\n\n{}\n\nOpen it in your browser or email program?",
                        limarepdf_core::display_line(url, 300)
                    ),
                    Btn::primary("Open link"),
                ),
            ) {
                Outcome::Apply => {
                    ctx.open_url(egui::OpenUrl::new_tab(url.as_str()));
                    done = true;
                }
                Outcome::Cancel => done = true,
                Outcome::Open => {}
            },
            Dialog::ConfirmClose(asked) => match dlg::confirm(
                ctx,
                Ask::new(
                    "Close without saving?",
                    "This document has changes that aren't saved. Closing it loses them.",
                    Btn::danger("Discard changes"),
                )
                .file(file),
            ) {
                Outcome::Apply => {
                    if self.still(asked) {
                        self.close_tab(self.active);
                    }
                    done = true;
                }
                Outcome::Cancel => done = true,
                Outcome::Open => {}
            },
            Dialog::ConfirmQuit => match dlg::confirm(
                ctx,
                Ask::new(
                    "Quit without saving?",
                    "Some documents have changes that aren't saved. Quitting loses them.",
                    Btn::danger("Quit without saving"),
                ),
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
            Dialog::RecoveryFile { title, text, file } => {
                let (ok, show) = (Btn::primary("OK"), Btn::plain("Show file"));
                match dlg::choose(ctx, Shell::new(title), text, ok, show, false) {
                    Choice::Second => {
                        let file = file.clone();
                        self.show_file(&file);
                    }
                    Choice::Open => {}
                    Choice::First | Choice::Cancel => done = true,
                }
            }
            _ if self.tabs.is_empty() => done = true,
            Dialog::ConfirmReplace(asked, e, n) => match dlg::confirm(
                ctx,
                Ask::new(
                    "Replace files?",
                    &format!(
                        "{} in {} already {} the names these images would get. Replacing \
                         {} can't be undone.",
                        plural(*n, "A file", "{n} files"),
                        e.dir.display(),
                        if *n == 1 { "has" } else { "have" },
                        if *n == 1 { "it" } else { "them" },
                    ),
                    Btn::danger("Replace"),
                )
                .file(file),
            ) {
                Outcome::Apply => {
                    if self.still(asked) {
                        self.start_export(ctx, e.clone());
                    }
                    done = true;
                }
                Outcome::Cancel => done = true,
                Outcome::Open => {}
            },
            Dialog::ConfirmFlatten(asked) => match dlg::confirm(
                ctx,
                Ask::new(
                    "Flatten form?",
                    "The fields become part of the pages, as they look now, and can no longer \
                     be filled in, here or in other readers. Undo brings them back until you \
                     close the document.",
                    Btn::primary("Flatten form"),
                )
                .file(file),
            ) {
                Outcome::Apply => {
                    if self.still(asked) {
                        let t = &mut self.tabs[self.active];
                        t.ed.form.clear_focus();
                        if t.edit(ctx, |p| p.flatten_form(None).map(|_| ())) {
                            t.banners.retain(|b| b.kind != BannerKind::Forms);
                            t.notice = Some("Flattened the form".into());
                        }
                    }
                    done = true;
                }
                Outcome::Cancel => done = true,
                Outcome::Open => {}
            },
            Dialog::ConfirmFlattenAnnots(asked) => match dlg::confirm(
                ctx,
                Ask::new(
                    "Flatten annotations?",
                    "Comments, notes, highlights and drawings become part of the pages, as \
                     they look now, and can no longer be changed or removed, here or in other \
                     readers. Their comment text is dropped. Links and form fields stay. Undo \
                     brings them back until you close the document.",
                    Btn::primary("Flatten annotations"),
                )
                .file(file),
            ) {
                Outcome::Apply => {
                    if self.still(asked) {
                        let t = &mut self.tabs[self.active];
                        if t.edit(ctx, |p| p.flatten_annotations(None).map(|_| ())) {
                            t.notice = Some("Flattened the annotations".into());
                        }
                    }
                    done = true;
                }
                Outcome::Cancel => done = true,
                Outcome::Open => {}
            },
            Dialog::ConfirmApplyRedactions(asked, n) => match dlg::confirm(
                ctx,
                Ask::new(
                    &plural(*n, "Apply 1 redaction?", "Apply {n} redactions?"),
                    &format!(
                        "Everything under {} is removed from the document: text (hidden text \
                         too), the covered parts of pictures, drawings, and comments, links \
                         and form fields. The areas are painted black, and the marks go. \
                         LimarePDF then checks that nothing readable is left under them. Undo \
                         brings it all back until you close the document; once saved, it is \
                         gone from the file.",
                        plural(*n, "the marked area", "the {n} marked areas")
                    ),
                    Btn::danger("Apply redactions"),
                )
                .file(file),
            ) {
                Outcome::Apply => {
                    if self.still(asked) {
                        self.apply_redactions(ctx);
                    }
                    done = true;
                }
                Outcome::Cancel => done = true,
                Outcome::Open => {}
            },
            Dialog::ConfirmFinishSigning(asked, n) => match dlg::confirm(
                ctx,
                Ask::new(
                    "Finish signing?",
                    &format!(
                        "{} drawn into the pages, as {} now. After that {} can't be moved or \
                         removed like a comment. Undo brings {} back until you close the \
                         document.",
                        plural(
                            *n,
                            "The item you put down with the Sign tool is",
                            "The {n} items you put down with the Sign tool are"
                        ),
                        if *n == 1 { "it looks" } else { "they look" },
                        if *n == 1 { "it" } else { "they" },
                        if *n == 1 { "it" } else { "them" },
                    ),
                    Btn::primary("Finish signing"),
                )
                .file(file),
            ) {
                Outcome::Apply => {
                    if self.still(asked) {
                        let t = &mut self.tabs[self.active];
                        t.edit(ctx, |p| p.finish_signing().map(drop));
                    }
                    done = true;
                }
                Outcome::Cancel => done = true,
                Outcome::Open => {}
            },
            Dialog::RemoveHidden(asked, s) => match dlg::remove_hidden(ctx, file, s) {
                Outcome::Apply => {
                    let what: Vec<limarepdf_core::Hidden> = s
                        .kinds
                        .iter()
                        .filter(|(_, n, on)| *on && *n > 0)
                        .map(|(h, _, _)| *h)
                        .collect();
                    if self.still(asked) {
                        self.remove_hidden(ctx, &what);
                    }
                    done = true;
                }
                Outcome::Cancel => done = true,
                Outcome::Open => {}
            },
            Dialog::ConfirmSaveMarks(asked, as_new, n) => {
                let as_new = *as_new;
                match dlg::confirm(
                    ctx,
                    Ask::new(
                        "Save without applying the redactions?",
                        &format!(
                            "This document has {} marked for redaction but not applied: \
                             nothing under the marks has been removed, and anyone can still \
                             read it. Apply them first with Document > Apply redactions, or \
                             save the marks to finish or review later.",
                            plural(*n, "an area", "{n} areas")
                        ),
                        Btn::danger("Save with marks"),
                    )
                    .file(file),
                ) {
                    Outcome::Apply => {
                        if self.still(asked) {
                            self.save_signed(ctx, as_new);
                        }
                        done = true;
                    }
                    Outcome::Cancel => done = true,
                    Outcome::Open => {}
                }
            }
            Dialog::ConfirmSaveRepaired(asked) => match dlg::choose(
                ctx,
                Shell::new("Replace the damaged original?").file(file),
                "LimarePDF repaired this file to open it, and some of its content may be missing \
                 from the repair. Saving over it replaces the original for good. Save as a new \
                 file to keep the original as it is.",
                Btn::primary("Save as…"),
                Btn::danger("Replace original"),
                true,
            ) {
                Choice::First => {
                    if self.still(asked) {
                        self.save_unmarked(ctx, true);
                    }
                    done = true;
                }
                Choice::Second => {
                    if self.still(asked) {
                        self.save_unmarked(ctx, false);
                    }
                    done = true;
                }
                Choice::Cancel => done = true,
                Choice::Open => {}
            },
            Dialog::ConfirmSaveSigned(asked, as_new, why) => {
                let as_new = *as_new;
                match dlg::confirm(
                    ctx,
                    Ask::new(
                        "Break the signatures?",
                        "This document is digitally signed, and saving your changes writes the \
                         whole file again, so the saved copy's signatures will show as invalid \
                         in every reader. Save as a new file to keep the signed original.",
                        Btn::danger("Save anyway"),
                    )
                    .file(file)
                    .details(&format!(
                        "LimarePDF can't add your changes after what was signed: {}.",
                        limarepdf_core::display_line(why, 300)
                    )),
                ) {
                    Outcome::Apply => {
                        if self.still(asked) {
                            self.save_now(ctx, as_new);
                        }
                        done = true;
                    }
                    Outcome::Cancel => done = true,
                    Outcome::Open => {}
                }
            }
            Dialog::ConfirmSaveDisallowed(s) => {
                match dlg::confirm(
                    ctx,
                    Ask::new(
                        "Save changes the signers didn't allow?",
                        "Your changes are ones this document's signatures don't allow. The \
                         signatures stay valid for what was signed, but anyone who checks them \
                         will be told about these changes. Save as a new file to keep the \
                         signed original as it is.",
                        Btn::danger("Save anyway"),
                    )
                    .file(file)
                    .details(&format!(
                        "What checking the signatures will report:\n{}",
                        digital::disallowed_list(&s.disallowed)
                    )),
                ) {
                    Outcome::Apply => {
                        let as_new = s.as_new;
                        match self.tabs.iter().position(|t| t.id == s.asked.tab) {
                            // Closed meanwhile: says so.
                            None => {
                                self.still(&s.asked);
                            }
                            Some(i) => {
                                self.active = i;
                                if self.tabs[i].state() == *s.asked.state {
                                    let bytes = std::mem::take(&mut s.bytes);
                                    self.save_appended(ctx, as_new, bytes);
                                } else {
                                    // The document changed meanwhile: work it out again.
                                    self.save_signed(ctx, as_new);
                                }
                            }
                        }
                        done = true;
                    }
                    Outcome::Cancel => done = true,
                    Outcome::Open => {}
                }
            }
            Dialog::Print(asked, s) => match dlg::print(ctx, s) {
                Outcome::Apply if s.printers.is_empty() => done = true,
                Outcome::Apply if !self.still(asked) => done = true,
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
                        match parse_page_range(s.range(), t.page_count()) {
                            Ok(pages) => {
                                let fields = t.pdf.unprintable_fields(&pages);
                                if fields.is_empty() {
                                    self.start_print(ctx, pages, opts);
                                } else {
                                    self.dialog =
                                        Dialog::ConfirmPrintFields(Box::new(PrintFields {
                                            asked: Asked::of(t),
                                            fields,
                                            pages,
                                            opts,
                                        }));
                                }
                                done = true;
                            }
                            Err(e) => s.error = Some(e.to_string()),
                        }
                    }
                }
                Outcome::Cancel => done = true,
                Outcome::Open => {}
            },
            Dialog::ConfirmPrintFields(q) => {
                let (these, values) = match q.fields.len() {
                    1 => ("This field", "its value"),
                    _ => ("These fields", "their values"),
                };
                match dlg::confirm(
                    ctx,
                    Ask::new(
                        "Print without these fields?",
                        &format!(
                            "{these} won't show on paper, because LimarePDF can't draw \
                             {values}: {}.",
                            limarepdf_core::quoted_list(&q.fields)
                        ),
                        Btn::primary("Print anyway"),
                    )
                    .file(file),
                ) {
                    Outcome::Apply => {
                        if self.still(&q.asked) {
                            let pages = std::mem::take(&mut q.pages);
                            self.start_print(ctx, pages, q.opts.clone());
                        }
                        done = true;
                    }
                    Outcome::Cancel => done = true,
                    Outcome::Open => {}
                }
            }
            Dialog::SaveSelection(asked, layout) => match dlg::save_selection(ctx, file, layout) {
                Outcome::Apply => {
                    let layout = *layout;
                    done = true;
                    if self.still(asked) {
                        self.save_selection(layout);
                    }
                }
                Outcome::Cancel => done = true,
                Outcome::Open => {}
            },
            Dialog::ConfirmSignRecovered(asked) => match dlg::confirm(
                ctx,
                Ask::new(
                    "Break the signatures?",
                    "This document is digitally signed, and signing this copy writes the whole \
                     file again, so the earlier signatures will show as invalid in every \
                     reader. Open the original to sign it with its signatures kept.",
                    Btn::danger("Sign anyway"),
                )
                .file(file)
                .details(&format!(
                    "LimarePDF can't add your signature after what was signed: {RECOVERED}."
                )),
            ) {
                Outcome::Apply => {
                    if self.still(asked) {
                        self.sign_tool(false);
                    }
                    done = true;
                }
                Outcome::Cancel => done = true,
                Outcome::Open => {}
            },
        }
        if !done {
            self.dialog = dialog;
        }
    }

    /// The colours pages are drawn in on screen now.
    fn colours(&self, ctx: &egui::Context) -> Colours {
        Colours {
            mode: self
                .settings
                .page_look
                .mode(ctx.theme() == egui::Theme::Dark),
            pictures_too: self.settings.pictures_too,
        }
    }

    /// The state the commands' conditions are checked against.
    fn state(&self, ctx: &egui::Context) -> Ctx {
        let mut c = Ctx {
            busy: self.busy || self.sign_window.as_ref().is_some_and(SignWindow::busy),
            side: self.side.shown(),
            inspector: self.inspector.shown(),
            docked: self.settings.controls_docked,
            theme: ctx.options(|o| o.theme_preference),
            page_look: self.settings.page_look,
            pictures_too: self.settings.pictures_too,
            recovery: self.settings.recovery,
            layout: self.settings.layout,
            cover: self.settings.cover,
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
                        for (j, b) in items[i..i + n].iter().enumerate() {
                            if b.sep && j > 0 {
                                ui.separator();
                            }
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
        // In the dialogs' look, as a launcher: no title or buttons (option A of the mockup).
        let frame = egui::Frame::window(&ctx.global_style()).inner_margin(16);
        let width = (ctx.content_rect().width() - 32.0).clamp(240.0, 480.0);
        let modal = egui::Modal::new(egui::Id::new("palette"))
            .frame(frame)
            .show(ctx, |ui| {
                ui.set_width(width);
                theme::dialog_style(ui.style_mut());
                let field = theme::field(
                    egui::TextEdit::singleline(&mut p.query)
                        .hint_text("Type a command")
                        .desired_width(f32::INFINITY),
                )
                .margin(egui::Margin {
                    left: 36,
                    right: 12,
                    top: 0,
                    bottom: 0,
                });
                let r = ui.add(field);
                // The search icon, inside the field before the text.
                let at = r.rect.left_center() + egui::vec2(12.0, 0.0);
                let weak = ui.visuals().weak_text_color();
                let font = egui::FontId::new(16.0, theme::icons());
                let glass = icon::MAGNIFYING_GLASS;
                ui.painter()
                    .text(at, egui::Align2::LEFT_CENTER, glass, font, weak);
                dlg::set_name(&r, "Command");
                r.request_focus();
                if r.changed() {
                    p.row = 0;
                }
                ui.add_space(12.0);
                ui.add(egui::Separator::default().spacing(0.0));
                ui.add_space(8.0);
                let columns = palette_columns(ui, &items);
                let list = egui::ScrollArea::vertical().max_height(360.0);
                list.show(ui, |ui| {
                    if items.is_empty() {
                        ui.weak("No matching command.");
                    }
                    for (n, a) in items.iter().enumerate() {
                        let on = n == p.row;
                        let r = palette_row(ui, a, on, (a.enabled)(c), columns);
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
        // A dialog handles its own keys.
        if self.palette.is_some() || dlg::any_open(ctx) {
            return;
        }
        // Esc closes the inspector's pop-out first, keeping what was changed in it, and
        // the keyboard goes back to the button that opened it.
        if self.popout && ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, Key::Escape)) {
            self.popout = false;
            if let Some(id) = ctx.data(|d| d.get_temp(egui::Id::new(POPOUT_OPENER))) {
                ctx.memory_mut(|m| m.request_focus(id));
            }
            return;
        }
        let c = self.state(ctx);
        let typing = ctx.egui_wants_keyboard_input();
        // Undo and Redo while a line of page text is edited, or new text typed, on the page
        // take back typing there; Undo takes back the whole edit once it is kept.
        let line = self
            .tabs
            .get(self.active)
            .is_some_and(|t| t.ed.typing_on_page());
        // Esc with a list, menu or colour picker open only closes it (a note being typed
        // stays open while its colour is picked).
        let picking = egui::Popup::is_any_open(ctx);
        let mut acts = Vec::new();
        ctx.input_mut(|i| {
            for (a, k) in actions::shortcuts() {
                let page_only = matches!(a.act, Act::Copy | Act::SelectAllText);
                let global = k.modifiers.command && !page_only;
                let in_box = line && typing && matches!(a.act, Act::Undo | Act::Redo);
                let closes = picking && a.act == Act::Escape;
                if (typing && !global) || in_box || closes || !(a.enabled)(&c) {
                    continue;
                }
                let hit = if a.act == Act::Copy {
                    // Ctrl+C arrives as a Copy event, not as a key press.
                    i.events.iter().any(|e| matches!(e, egui::Event::Copy))
                } else if global {
                    i.consume_shortcut(&k)
                } else {
                    // The key's own modifiers, not those held at the end of the frame: a
                    // quick Ctrl+A can arrive with Ctrl already let go.
                    i.events.iter().any(|e| {
                        matches!(e, egui::Event::Key { key, pressed: true, modifiers, .. }
                            if *key == k.logical_key && modifiers.matches_logically(k.modifiers))
                    })
                };
                if hit {
                    acts.push(a.act);
                }
            }
        });
        for a in acts {
            self.run_command(ctx, actions::name(a), |app, ctx| app.act(ctx, a));
        }
    }
}

/// A toolbar button for a command: (ui, command, shown as on, where clicks go).
type BarButton<'a> = &'a dyn Fn(&mut egui::Ui, Act, bool, &mut Vec<Act>);

/// The small menu beside the Fill and sign button: signatures and initials (saved ones
/// and Create), the date, your name, text and marks.
fn sign_button_menu(ui: &mut egui::Ui, menu: &crate::signing::SignMenu, acts: &mut Vec<Act>) {
    ui.add_space(-ui.spacing().item_spacing.x);
    ui.menu_button(theme::icon(icon::CARET_DOWN).size(10.0), |ui| {
        crate::signing::menu_ui(ui, menu, acts);
    })
    .response
    .on_hover_text("Signatures, initials, date, name, text and marks")
    .widget_info(|| named("Fill and sign choices"));
}

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
            let r = ui.add(b);
            dlg::set_name(&r, a.label);
            if r.clicked() {
                acts.push(a.act);
                ui.close();
            }
        }
    })
    .response
    .on_hover_text("Commenting tools")
    .widget_info(|| named("Commenting tools"));
}

/// The Tools menu button: every tool, those folded off the bar among them. It shows
/// `hidden`, the current tool when that one isn't on the bar, highlighted.
fn tool_menu(
    ui: &mut egui::Ui,
    tools: Vec<&Action>,
    hidden: Option<Tool>,
    is_on: &dyn Fn(&Action) -> bool,
    acts: &mut Vec<Act>,
) {
    let current = tools
        .iter()
        .find(|a| hidden.is_some_and(|t| a.act == Act::Tool(t)))
        .map_or(icon::TOOLBOX, |a| a.icon);
    let label = (
        theme::icon(current).size(18.0),
        theme::icon(icon::CARET_DOWN).size(10.0),
    );
    let button = egui::Button::new(label)
        .selected(hidden.is_some())
        .frame_when_inactive(hidden.is_some());
    egui::containers::menu::MenuButton::from_button(button)
        .ui(ui, |ui| {
            for a in tools {
                let b =
                    egui::Button::new((theme::icon(a.icon).size(16.0), a.label)).selected(is_on(a));
                let r = ui.add(b);
                dlg::set_name(&r, a.label);
                if r.clicked() {
                    acts.push(a.act);
                    ui.close();
                }
            }
        })
        .0
        .on_hover_text("Tools: every tool, including those without room on the bar")
        .widget_info(|| named("Tools"));
}

/// A document tab: its title, and a close button inside its frame. Returns whether the
/// title or the close button was clicked.
fn tab_button(ui: &mut egui::Ui, title: &str, on: bool) -> (bool, bool) {
    let v = ui.visuals();
    let fill = if on {
        v.selection.bg_fill.gamma_multiply(0.35)
    } else {
        v.faint_bg_color
    };
    let stroke = if on {
        egui::Stroke::new(1.0, v.selection.stroke.color)
    } else {
        v.widgets.noninteractive.bg_stroke
    };
    egui::Frame::new()
        .fill(fill)
        .stroke(stroke)
        .corner_radius(5)
        .inner_margin(egui::Margin {
            left: 8,
            right: 2,
            top: 1,
            bottom: 1,
        })
        .show(ui, |ui| {
            ui.spacing_mut().item_spacing.x = 2.0;
            let text = egui::RichText::new(title);
            let text = if on { text.strong() } else { text };
            let pick = ui.add(egui::Button::new(text).frame(false)).clicked();
            let x = egui::Button::new(theme::icon(icon::X).size(12.0))
                .frame_when_inactive(false)
                .min_size(egui::vec2(20.0, 20.0));
            let x = ui.add(x);
            x.widget_info(|| named(&format!("Close {title}")));
            (pick, x.on_hover_text("Close tab").clicked())
        })
        .inner
}

/// An icon button with its name as tooltip and accessible name.
fn icon_button(ui: &mut egui::Ui, glyph: &str, name: &str) -> egui::Response {
    let b = egui::Button::new(theme::icon(glyph).size(16.0))
        .frame_when_inactive(false)
        .min_size(egui::vec2(26.0, 26.0));
    let r = ui.add(b);
    r.widget_info(|| named(name));
    r.on_hover_text(name)
}

/// The button that takes docked selection buttons back to a bar by the selection.
fn float_button(ui: &mut egui::Ui, acts: &mut Vec<Act>) {
    let name = "Float controls near selection";
    if icon_button(ui, icon::PICTURE_IN_PICTURE, name).clicked() {
        acts.push(Act::ToggleDock);
    }
}

/// The collapsed inspector: a strip with a button to expand it at the top, the docked
/// selection buttons (`groups`, empty when floating or nothing is selected) with the
/// float button, and a button for the inspector's contents in a pop-out. Returns
/// whether expand, and the pop-out button, were clicked.
fn inspector_strip(
    ui: &mut egui::Ui,
    groups: &[Vec<&'static Action>],
    c: &Ctx,
    acts: &mut Vec<Act>,
) -> (bool, bool) {
    let mut clicked = (false, false);
    ui.vertical_centered(|ui| {
        ui.add_space(4.0);
        clicked.0 = icon_button(ui, icon::CARET_DOUBLE_LEFT, "Expand the inspector").clicked();
        ui.separator();
        if !groups.is_empty() {
            actions::context_buttons(ui, groups, c, acts);
            ui.separator();
            float_button(ui, acts);
            ui.separator();
        }
        let options = "Options for the selection or the document";
        let r = icon_button(ui, icon::SLIDERS_HORIZONTAL, options);
        ui.data_mut(|d| d.insert_temp(egui::Id::new(POPOUT_OPENER), r.id));
        clicked.1 = r.clicked();
    });
    clicked
}

/// The inspector's contents: the editor's section for a selected object or words or a
/// tool that adds content; otherwise the selected pages or the document. Returns a
/// panel command pressed.
fn inspector_body(
    s: &mut Settings,
    t: &mut Tab,
    ui: &mut egui::Ui,
    c: &Ctx,
) -> Option<inspector::Apply> {
    let mut apply = None;
    t.guarded(|t| {
        if !t.ed.owns_panel() {
            apply = inspector::show(ui, t, c);
            return;
        }
        // Stamp choices and the author's name are app-wide: lent to the tab's inspector
        // and saved when it changes them.
        let author = s.author();
        t.ed.author = author.clone();
        t.ed.stamp_name_date = s.stamp_name_date;
        t.ed.stamp_pictures = s.stamp_pictures.clone();
        t.ed.stamp_wordings = s.stamp_wordings.clone();
        t.ed.sign_kit = s.sign.clone();
        t.ed.keep.clone_from(&s.keep_tools);
        t.ed.inspector(ui);
        if t.ed.keep != s.keep_tools {
            s.keep_tools.clone_from(&t.ed.keep);
            s.save();
        }
        if std::mem::take(&mut t.ed.apply_redactions) {
            t.requested.push(Act::ApplyRedactions);
        }
        if std::mem::take(&mut t.ed.finish_signing) {
            t.requested.push(Act::FinishSigning);
        }
        if t.ed.sign_kit != s.sign {
            s.sign = t.ed.sign_kit.clone();
            s.sign.save();
        }
        if (
            t.ed.stamp_name_date,
            &t.ed.stamp_pictures,
            &t.ed.stamp_wordings,
            &t.ed.author,
        ) != (
            s.stamp_name_date,
            &s.stamp_pictures,
            &s.stamp_wordings,
            &author,
        ) {
            s.stamp_name_date = t.ed.stamp_name_date;
            s.stamp_pictures = t.ed.stamp_pictures.clone();
            s.stamp_wordings = t.ed.stamp_wordings.clone();
            if t.ed.author != author {
                s.author = Some(t.ed.author.clone());
            }
            s.save();
        }
    });
    apply
}

/// Where the inspector strip keeps the id of its button that opens the pop-out.
const POPOUT_OPENER: &str = "inspector-popout-opener";

/// The inspector's contents in a pop-out over the pages, left of its strip (`strip`),
/// for windows too narrow for the panel. Changes made in it stay when it closes. Returns
/// a panel command pressed, and whether it stays open: a click outside it (and outside
/// the strip, whose button opens and closes it) closes it, as Esc does (see
/// `App::shortcuts`).
fn inspector_popout(
    ctx: &egui::Context,
    strip: egui::Rect,
    s: &mut Settings,
    t: &mut Tab,
    c: &Ctx,
) -> (Option<inspector::Apply>, bool) {
    let mut apply = None;
    let height = (strip.height() - 16.0).max(120.0);
    let area = egui::Area::new(egui::Id::new("inspector-popout"))
        .order(egui::Order::Foreground)
        .fixed_pos(egui::pos2(strip.left() - 4.0, strip.top() + 4.0))
        .pivot(egui::Align2::RIGHT_TOP)
        .constrain(true)
        .show(ctx, |ui| {
            egui::Frame::popup(ui.style()).show(ui, |ui| {
                ui.set_width(INSPECTOR_WIDTH);
                ui.heading(inspector::title(t));
                ui.add_space(4.0);
                egui::ScrollArea::vertical()
                    .max_height(height)
                    .show(ui, |ui| apply = inspector_body(s, t, ui, c));
            });
        });
    let clicked_outside = ctx.input(|i| {
        let at = i.pointer.interact_pos();
        i.pointer.any_click()
            && at.is_some_and(|p| !area.response.rect.contains(p) && !strip.contains(p))
    });
    // A menu or colour picker opened from the pop-out is outside it, but part of it.
    let child_open = egui::Popup::is_any_open(ctx);
    (apply, !(clicked_outside && !child_open))
}

/// The widths of the command search's category and shortcut columns: the widest of each
/// among `items`.
fn palette_columns(ui: &egui::Ui, items: &[&Action]) -> (f32, f32) {
    let font = egui::TextStyle::Body.resolve(ui.style());
    let width = |text: String| {
        let g = ui
            .painter()
            .layout_no_wrap(text, font.clone(), egui::Color32::PLACEHOLDER);
        g.size().x
    };
    let mut columns = (0.0_f32, 0.0_f32);
    for a in items {
        columns.0 = columns.0.max(width(a.group.name().to_string()));
        if let Some(k) = a.keys.first() {
            columns.1 = columns.1.max(width(ui.ctx().format_shortcut(k)));
        }
    }
    columns
}

/// A row of the command search: the command's icon and name, then its category and
/// shortcut in columns `columns` wide, aligned down the list.
fn palette_row(
    ui: &mut egui::Ui,
    a: &Action,
    on: bool,
    enabled: bool,
    columns: (f32, f32),
) -> egui::Response {
    let sense = if enabled {
        egui::Sense::click()
    } else {
        egui::Sense::hover()
    };
    let size = egui::vec2(ui.available_width(), theme::DIALOG_CONTROL);
    let (rect, r) = ui.allocate_exact_size(size, sense);
    r.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, enabled, a.label));
    dlg::set_name(&r, a.label);
    if !ui.is_rect_visible(rect) {
        return r;
    }
    let v = ui.visuals();
    let radius = f32::from(theme::RADIUS);
    if on {
        ui.painter().rect_filled(rect, radius, v.selection.bg_fill);
    } else if r.hovered() && enabled {
        ui.painter()
            .rect_filled(rect, radius, v.widgets.hovered.weak_bg_fill);
    }
    let (mut text, mut weak) = (v.text_color(), v.weak_text_color());
    if on {
        text = v.selection.stroke.color;
    }
    if !enabled {
        text = weak;
        weak = weak.gamma_multiply(0.7);
    }
    let font = egui::TextStyle::Body.resolve(ui.style());
    let pad = 10.0;
    let gap = 24.0;
    let y = rect.center().y;
    let painter = ui.painter_at(rect);
    if !a.icon.is_empty() {
        let at = egui::pos2(rect.left() + pad, y);
        let icons = egui::FontId::new(16.0, theme::icons());
        painter.text(at, egui::Align2::LEFT_CENTER, a.icon, icons, text);
    }
    // The shortcut flush right, the category in its column before it.
    let key_left = rect.right() - pad - columns.1;
    if let Some(k) = a.keys.first() {
        let at = egui::pos2(rect.right() - pad, y);
        let key = ui.ctx().format_shortcut(k);
        painter.text(at, egui::Align2::RIGHT_CENTER, key, font.clone(), weak);
    }
    let cat_left = key_left - if columns.1 > 0.0 { gap } else { 0.0 } - columns.0;
    let at = egui::pos2(cat_left, y);
    let category = a.group.name();
    painter.text(at, egui::Align2::LEFT_CENTER, category, font.clone(), weak);
    // The name, cut short with … if it would reach the category.
    let left = rect.left() + pad + 26.0;
    let mut job = egui::text::LayoutJob::simple_singleline(a.label.to_string(), font, text);
    job.wrap = egui::text::TextWrapping::truncate_at_width((cat_left - gap - left).max(0.0));
    let galley = ui.painter().layout_job(job);
    let at = egui::pos2(left, y - galley.size().y / 2.0);
    painter.galley(at, galley, text);
    r
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

/// A tab whose document could not be shown: what happened, and ways out that don't
/// need the pages drawn.
fn broken_state(ui: &mut egui::Ui, why: &str, acts: &mut Vec<Act>) {
    ui.vertical_centered(|ui| {
        ui.add_space(ui.available_height() / 3.0);
        ui.heading("This document can't be shown");
        ui.add_space(8.0);
        ui.label(why);
        ui.weak("Your other tabs are not affected. Changes made here can still be saved.");
        ui.add_space(16.0);
        if ui.button("Save a copy…").clicked() {
            acts.push(Act::SaveAs);
        }
        if ui.button("Close this tab").clicked() {
            acts.push(Act::CloseTab);
        }
    });
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
    let size = egui::vec2(440.0, 310.0 + extra).min(room.size() - egui::vec2(32.0, 32.0));
    let card = egui::Rect::from_center_size(room.center(), size.max(egui::vec2(1.0, 1.0)));
    let id = egui::Id::new("empty-card");
    let r = ui
        .interact(card, id, egui::Sense::click())
        .on_hover_cursor(egui::CursorIcon::PointingHand);
    let v = ui.visuals();
    let accent = theme::accent(v.dark_mode);
    // Lit while the pointer is anywhere on the card: the buttons inside take the hover
    // from the card, so `r.hovered()` would flicker as the pointer crosses them.
    let stroke = if dragging || ui.rect_contains_pointer(card) {
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
            let label = file_name(path);
            let name = label.clone().atom_shrink(true);
            let glyph = theme::icon(icon::FILE_PDF).size(14.0);
            let row = egui::Button::new((glyph, name, egui::Atom::grow()))
                .truncate()
                .frame_when_inactive(false)
                .min_size(egui::vec2(ui.available_width(), 26.0));
            let tip = path.display().to_string();
            let r = ui.add(row);
            dlg::set_name(&r, &label);
            if r.on_hover_text(tip).clicked() {
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
    ui.add_space(10.0);
    about::made_in_canada(&mut ui, "start-canada", true);
    if r.clicked() {
        acts.push(Act::Open);
    }
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        self.poll_loads(&ctx);
        self.recovery_tick(&ctx);
        // Page colours follow the setting and, when automatic, the app's theme.
        let colours = self.colours(&ctx);
        for t in &mut self.tabs {
            t.set_colours(colours);
        }

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
            && self.tabs.iter().any(Tab::unsaved)
        {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            self.dialog = Dialog::ConfirmQuit;
        }

        // The window's size, saved when the app closes; not while minimized or full screen.
        let (size, maximized, hidden, monitor) = ctx.input(|i| {
            let v = i.viewport();
            let size = window_size(i);
            let hidden = v.minimized == Some(true) || v.fullscreen == Some(true);
            (size, v.maximized, hidden, v.monitor_size)
        });
        if !hidden {
            let maximized = maximized.unwrap_or(self.settings.window_maximized);
            self.settings.keep_window(Some([size.x, size.y]), maximized);
            // A size kept on a bigger monitor is fitted to this one, once it is known.
            if let (false, false, Some(monitor)) = (self.fitted, maximized, monitor) {
                self.fitted = true;
                let fit = size.min(monitor);
                if fit != size && fit.x > 0.0 && fit.y > 0.0 {
                    ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(fit));
                }
            }
        }

        // Narrow windows fold the panels away for now; they come back with room, unless the
        // user closed them (see `Panel`).
        let width = ctx.content_rect().width();
        self.side.set_narrow(width < SIDE_NARROW_WIDTH);
        self.inspector.set_narrow(width < NARROW_WIDTH);

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
            let enabled = (a.enabled)(&c);
            let r = ui.add_enabled(enabled, b);
            let name = a.label.trim_end_matches('…');
            r.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, enabled, name));
            if r.on_hover_text(tip).clicked() {
                acts.push(act);
            }
        };
        let menu_icon = |glyph: &str| theme::icon(glyph).size(18.0);
        // The saved signatures for the Fill and sign button's menu.
        let sign_menu = self
            .tabs
            .get_mut(self.active)
            .map(|t| t.ed.sign.menu(&ctx, &self.settings.sign));

        let mut right_acts: Vec<Act> = Vec::new();
        // A search was run from the top bar: its results show in the navigation panel.
        let mut searched = false;

        egui::Panel::top("topbar")
            .frame(theme::bar_frame(ui.style()))
            .show(ui, |ui| {
                // Width budget, so the two ends never overlap: on narrower windows the
                // search field becomes an icon, and then the tools fold one by one into
                // the Tools menu, in the order of FOLDS; they come back as room returns.
                // The widths are the controls' own, measured as they were last drawn
                // (in points, so at any display scale), and estimated until then. (The
                // file name is on the tab and in the window title.)
                let sp = ui.spacing();
                let btn = (18.0 + 2.0 * sp.button_padding.x).max(28.0) + sp.item_spacing.x;
                let caret = 10.0 + 2.0 * sp.button_padding.x + sp.item_spacing.x;
                let sep = 2.0 * sp.item_spacing.x;
                let gap = sp.item_spacing.x;
                let widths = std::cell::RefCell::new(self.bar_widths);
                let m = self.bar_widths;
                // Print, then the search field (with its icon) or the search icon.
                let right = |with_field: bool| {
                    let guess = btn + if with_field { 180.0 + 18.0 + 2.0 * gap } else { btn };
                    m.right[usize::from(with_field)].unwrap_or(guess)
                };
                // Menu, Open, Save, Read, the Tools menu and two separators, then the
                // controls not folded away.
                let need = |folded: usize| {
                    let tools: f32 = (folded..FOLDS.len())
                        .map(|i| m.folds[i].unwrap_or(FOLDS[i].width(btn, caret, sep)))
                        .sum();
                    m.fixed.unwrap_or(4.0 * btn + caret + 2.0 * sep) + tools
                };
                // The room between the two sides, which `Sides` keeps apart.
                let w = ui.available_width() - 2.0 * gap;
                let search_field = w >= need(0) + right(true);
                self.fold = fold_level(self.fold, w - right(search_field), need);
                let (folded, shown) = FOLDS.split_at(self.fold);
                // A tool control's width on the bar: what drawing it added.
                let measure = |ui: &mut egui::Ui, f: Fold, draw: &mut dyn FnMut(&mut egui::Ui)| {
                    let before = ui.min_rect().width();
                    draw(ui);
                    let i = FOLDS.iter().position(|g| *g == f).unwrap_or(0);
                    widths.borrow_mut().folds[i] = Some(ui.min_rect().width() - before);
                };

                let left_side = |ui: &mut egui::Ui| {
                    ui.menu_button(menu_icon(icon::LIST), |ui| {
                        Self::menu(ui, &c, &recent, &mut acts);
                    })
                    .response
                    .on_hover_text("Menu")
                    .widget_info(|| named("Menu"));
                    bar(ui, Act::Open, false, &mut acts);
                    if !has {
                        return;
                    }
                    ui.separator();
                    bar(ui, Act::Save, false, &mut acts);
                    if !folded.contains(&Fold::Export) {
                        measure(ui, Fold::Export, &mut |ui| {
                            let export = egui::Button::new(menu_icon(icon::EXPORT))
                                .frame_when_inactive(false)
                                .min_size(egui::vec2(28.0, 28.0));
                            egui::containers::menu::MenuButton::from_button(export)
                                .ui(ui, |ui| {
                                    for act in [Act::ExportImages, Act::ExportText] {
                                        menu_item(ui, actions::get(act), &c, &mut acts);
                                    }
                                })
                                .0
                                .on_hover_text("Export")
                                .widget_info(|| named("Export"));
                        });
                    }
                    if !folded.contains(&Fold::History) {
                        measure(ui, Fold::History, &mut |ui| {
                            ui.separator();
                            bar(ui, Act::Undo, false, &mut acts);
                            bar(ui, Act::Redo, false, &mut acts);
                        });
                    }
                    ui.separator();
                    let is_on = |a: &Action| matches!(a.act, Act::Tool(t) if c.tool == Some(t));
                    bar(ui, Act::Tool(Tool::Read), c.tool == Some(Tool::Read), &mut acts);
                    for t in [Tool::Text, Tool::Image, Tool::Redact, Tool::Sign] {
                        if !folded.contains(&Fold::Tool(t)) {
                            measure(ui, Fold::Tool(t), &mut |ui| {
                                bar(ui, Act::Tool(t), c.tool == Some(t), &mut acts);
                                if let Some(menu) = sign_menu.as_ref().filter(|_| t == Tool::Sign) {
                                    sign_button_menu(ui, menu, &mut acts);
                                }
                            });
                        }
                    }
                    if !folded.contains(&Fold::Commenting) {
                        measure(ui, Fold::Commenting, &mut |ui| {
                            ui.separator();
                            let commenting = ACTIONS
                                .iter()
                                .filter(|a| matches!(a.act, Act::Tool(t) if Tool::COMMENTING.contains(&t)))
                                .collect();
                            annotate_button(ui, commenting, &c, &bar, &mut acts);
                        });
                    }
                    // Every tool, the folded ones among them; it shows the current tool
                    // when that one isn't on the bar.
                    let inline = |t: Tool| {
                        t == Tool::Read
                            || shown.iter().any(|f| match f {
                                Fold::Tool(u) => *u == t,
                                Fold::Commenting => Tool::COMMENTING.contains(&t),
                                Fold::Export | Fold::History => false,
                            })
                    };
                    let hidden = c.tool.filter(|t| !inline(*t));
                    let tools = ACTIONS.iter().filter(|a| a.group == Group::Tools).collect();
                    tool_menu(ui, tools, hidden, &is_on, &mut acts);
                    // Plus the spacing before the next item, as the measured controls have.
                    let mut w = widths.borrow_mut();
                    let tools: f32 = (folded.len()..FOLDS.len()).filter_map(|i| w.folds[i]).sum();
                    w.fixed = Some(ui.min_rect().width() + gap - tools);
                };
                let right_side = |ui: &mut egui::Ui| {
                    let acts = &mut right_acts;
                    if !has {
                        return;
                    }
                    let t = &mut self.tabs[self.active];
                    let last = m.right[usize::from(search_field)].map(|w| w - gap);
                    let used = dlg::reading_order(ui, last, |ui| {
                        if search_field {
                            ui.label(theme::icon(icon::MAGNIFYING_GLASS).weak());
                            let r = ui.add(
                                egui::TextEdit::singleline(&mut t.query)
                                    .hint_text("Search")
                                    .desired_width(180.0),
                            );
                            dlg::set_name(&r, "Search");
                            if std::mem::take(&mut self.focus_search) {
                                r.request_focus();
                            }
                            if r.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter)) {
                                t.run_search();
                                searched = true;
                            }
                        } else {
                            bar(ui, Act::Find, false, acts);
                        }
                        bar(ui, Act::Print, false, acts);
                    });
                    widths.borrow_mut().right[usize::from(search_field)] = Some(used + gap);
                };
                // The left side is laid out first, so Tab goes through the bar from left
                // to right; the folding above keeps it out of the right side's room.
                egui::Sides::new()
                    .shrink_right()
                    .height(28.0)
                    .show(ui, left_side, right_side);
                self.bar_widths = widths.into_inner();
            });
        acts.append(&mut right_acts);
        if searched {
            self.show_view(Side::Search);
        }

        if has {
            egui::Panel::top("tabs").show(ui, |ui| {
                ui.horizontal_wrapped(|ui| {
                    let mut close = None;
                    for i in 0..self.tabs.len() {
                        let (pick, x) = tab_button(ui, &self.tabs[i].title(), i == self.active);
                        if pick {
                            self.active = i;
                        }
                        if x {
                            close = Some(i);
                        }
                    }
                    if let Some(i) = close {
                        self.active = i;
                        acts.push(Act::CloseTab);
                    }
                });
            });
        }

        // The status bar: the page and messages at the left; the page layout, Fit width
        // and the zoom at the right.
        egui::Panel::bottom("status").show(ui, |ui| {
            let t = self.tabs.get(self.active);
            let status = |ui: &mut egui::Ui| {
                if let Some(t) = t {
                    let (n, name) = (t.current + 1, t.page_name(t.current));
                    ui.label(if name == n.to_string() {
                        format!("Page {n} of {}", t.page_count())
                    } else {
                        format!("Page {name} ({n} of {})", t.page_count())
                    });
                    if t.encryption.is_some() {
                        ui.separator();
                        ui.label("will be encrypted on save");
                    }
                }
                if !self.status.is_empty() {
                    ui.separator();
                    ui.label(&self.status);
                }
            };
            let last = &mut self.bar_widths.view;
            let view = |ui: &mut egui::Ui| {
                let Some(t) = t else { return };
                *last = Some(dlg::reading_order(ui, *last, |ui| {
                    // The page layout, its icon showing the current one.
                    let glyph = actions::get(Act::Layout(c.layout)).icon;
                    ui.menu_button(menu_icon(glyph), |ui| {
                        let layouts = ACTIONS.iter().filter(|a| a.submenu == "Page layout");
                        for a in layouts {
                            if a.sep {
                                ui.separator();
                            }
                            menu_item(ui, a, &c, &mut acts);
                        }
                    })
                    .response
                    .on_hover_text("Page layout")
                    .widget_info(|| named("Page layout"));
                    // Highlighted while the zoom follows the width.
                    bar(ui, Act::FitWidth, c.fit, &mut acts);
                    bar(ui, Act::ZoomOut, false, &mut acts);
                    // The zoom level; clicking it resets to actual size.
                    let level = format!("{:.0}%", t.zoom * 100.0);
                    let b = egui::Button::new(&level).frame_when_inactive(false);
                    let tip = tooltip(ui.ctx(), actions::get(Act::Actual));
                    let r = ui.add(b);
                    dlg::set_name(&r, &format!("Zoom {level}"));
                    if r.on_hover_text(tip).clicked() {
                        acts.push(Act::Actual);
                    }
                    bar(ui, Act::ZoomIn, false, &mut acts);
                }));
            };
            egui::Sides::new()
                .shrink_left()
                .truncate()
                .show(ui, status, view);
        });

        // The navigation rail at the left edge: one icon per view. Clicking another view
        // opens it; clicking the open one folds the panel away, and again brings it back.
        if has {
            let t = &self.tabs[self.active];
            let open = self.side.shown();
            let mut pick = None;
            egui::Panel::left("nav-rail")
                .resizable(false)
                .exact_size(RAIL_WIDTH)
                .show(ui, |ui| {
                    ui.vertical_centered(|ui| {
                        ui.add_space(4.0);
                        for (side, act) in NAV_VIEWS {
                            let a = actions::get(act);
                            let on = open && t.side == side;
                            let b = egui::Button::new(theme::icon(a.icon).size(18.0))
                                .selected(on)
                                .frame_when_inactive(on)
                                .min_size(egui::vec2(28.0, 28.0));
                            let r = ui.add(b);
                            r.widget_info(|| named(a.label));
                            if r.on_hover_text(a.label).clicked() {
                                pick = Some(side);
                            }
                        }
                    });
                });
            if let Some(side) = pick {
                if open && t.side == side {
                    self.side.set_open(false);
                    self.keep_panels();
                } else {
                    self.show_view(side);
                }
            }
        }

        if has && self.side.shown() {
            let active = self.active;
            egui::Panel::left("side")
                .resizable(true)
                .default_size(170.0)
                .show(ui, |ui| {
                    let t = &mut self.tabs[active];
                    let heading = match t.side {
                        Side::Pages => "Pages",
                        Side::Search => "Search results",
                        Side::Bookmarks => "Bookmarks",
                        Side::Comments => "Comments",
                        Side::Signatures => "Digital signatures",
                    };
                    ui.add_space(4.0);
                    ui.label(egui::RichText::new(heading).family(theme::semibold()));
                    ui.separator();
                    t.guarded(|t| match t.side {
                        Side::Pages => t.show_thumbs(ui, &c),
                        Side::Bookmarks => t.show_outline(ui),
                        Side::Comments => t.show_comments(ui),
                        Side::Signatures => t.show_signatures(ui),
                        Side::Search => {
                            let r = ui.text_edit_singleline(&mut t.query);
                            dlg::set_name(&r, "Search for");
                            let enter = r.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter));
                            ui.horizontal(|ui| {
                                if ui.button("Find").clicked() || enter {
                                    t.run_search();
                                }
                                for (glyph, name, step) in
                                    [("◀", "Previous match", -1), ("▶", "Next match", 1)]
                                {
                                    let r = ui.button(glyph);
                                    dlg::set_name(&r, name);
                                    if r.on_hover_text(name).clicked() {
                                        t.step_hit(step);
                                    }
                                }
                            });
                            ui.label(format!("{} match(es)", t.hits.len()));
                            if !t.hits.is_empty() {
                                let a = actions::get(Act::MarkResultsRedact);
                                let label = "Mark all for redaction";
                                if theme::glyph_button(ui, a.icon, label)
                                    .on_hover_text(a.hint)
                                    .clicked()
                                {
                                    acts.push(Act::MarkResultsRedact);
                                }
                            }
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
                    });
                });
            name_resize_handle(&ctx, "side", "Side panel width");
        }

        // The inspector: the editor's section for a selected object or words or a tool
        // that adds content; otherwise the selected pages or the document.
        // Docked, the selection's buttons head the panel, with one to float them again.
        // Collapsed, it is a strip at the right edge: a button that brings the panel back,
        // the docked buttons, and one for the panel's contents in a pop-out over the pages.
        let mut panel_apply = None;
        if has {
            // "Reply…" in the Comments view types in the inspector.
            if self.tabs[self.active].ed.reply_focus && !self.inspector.shown() {
                self.open_inspector();
                self.popout = false;
            }
            let t = &mut self.tabs[self.active];
            // As the floating bar: words, a comment or an object, not pages or nothing.
            let selected = matches!(
                t.ed.selection,
                Selection::Words { .. } | Selection::Annot { .. } | Selection::Object { .. }
            );
            let groups = if self.settings.controls_docked && selected {
                actions::toolbar_items(&t.ed.selection, &c)
            } else {
                Vec::new()
            };
            let mut open = self.inspector.shown();
            let (mut toggle, mut pop) = (false, false);
            let mut strip = None;
            let mut panel_acts = Vec::new();
            let s = &mut self.settings;
            let rail = egui::Panel::right("inspector-rail")
                .resizable(false)
                .exact_size(RAIL_WIDTH);
            let full = egui::Panel::right("inspector")
                .resizable(true)
                .default_size(INSPECTOR_WIDTH);
            egui::Panel::show_switched(ui, &mut open, rail, full, |ui, expanded| {
                if !expanded {
                    (toggle, pop) = inspector_strip(ui, &groups, &c, &mut panel_acts);
                    strip = Some(ui.max_rect());
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
                        let r = ui.add(hide);
                        r.widget_info(|| named("Collapse the inspector"));
                        if r.on_hover_text("Collapse the inspector").clicked() {
                            toggle = true;
                        }
                    },
                );
                if !groups.is_empty() {
                    ui.horizontal_wrapped(|ui| {
                        actions::context_buttons(ui, &groups, &c, &mut panel_acts);
                        ui.separator();
                        float_button(ui, &mut panel_acts);
                    });
                    ui.separator();
                }
                ui.add_space(4.0);
                egui::ScrollArea::vertical()
                    .auto_shrink([false, false])
                    .show(ui, |ui| panel_apply = inspector_body(s, t, ui, &c));
            });
            name_resize_handle(&ctx, "inspector", "Inspector width");
            if toggle {
                open = !open;
            }
            // Also catches the panel being dragged closed or open.
            if open != self.inspector.shown() {
                self.inspector.set_open(open);
                self.keep_panels();
            }
            if pop {
                self.popout = !self.popout;
            }
            if self.inspector.shown() {
                self.popout = false;
            }
            if let Some(strip) = strip.filter(|_| self.popout) {
                let t = &mut self.tabs[self.active];
                let (apply, keep) = inspector_popout(&ctx, strip, &mut self.settings, t, &c);
                panel_apply = panel_apply.or(apply);
                self.popout = keep;
            }
            acts.extend(panel_acts);
        }
        if let Some(apply) = panel_apply {
            let name = apply.name();
            self.run_command(&ctx, name, |app, ctx| app.apply_panel(ctx, apply));
        }

        let canvas = egui::Frame::central_panel(ui.style())
            .fill(theme::canvas(ui.visuals().dark_mode))
            .inner_margin(0);
        let author = self.settings.author();
        let (keep, fields) = (&self.settings.keep_tools, self.settings.fields_shown);
        egui::CentralPanel::default().frame(canvas).show(ui, |ui| {
            if let Some(t) = self.tabs.get_mut(self.active) {
                // App-wide choices, lent to the tab.
                t.ed.author = author;
                t.ed.keep.clone_from(keep);
                t.ed.form.highlight = fields;
                t.guarded(|t| t.show_pages(ui, &c));
                if let Some(why) = &t.broken {
                    broken_state(ui, why, &mut acts);
                }
            } else {
                empty_state(ui, &recent, &mut acts);
            }
        });

        // The Create signature window, whether or not the inspector is showing; a
        // signature made is saved with the others.
        if let Some(t) = self.tabs.get_mut(self.active) {
            t.ed.sign_kit.clone_from(&self.settings.sign);
            t.ed.sign_window(ui.ctx());
            if t.ed.sign_kit != self.settings.sign {
                self.settings.sign = t.ed.sign_kit.clone();
                self.settings.sign.save();
            }
        }

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
            // The digital signature tool lists the empty signature fields, and opens the
            // signing window once its place is chosen.
            if t.ed.tool == Tool::DigitalSign {
                t.ed.digital_fields = t
                    .pdf
                    .empty_signature_fields()
                    .into_iter()
                    .filter_map(|f| {
                        let w = f.widgets.iter().find(|w| !w.hidden)?;
                        Some((f.name, w.page))
                    })
                    .collect();
                if t.ed.digital_place.is_some() {
                    t.requested.push(Act::DigitalPlaced);
                }
            }
            acts.append(&mut t.requested);
            notice = t.notice.take();
        }
        if let Some(text) = notice {
            self.notify(&ctx, text);
        }
        self.toast_ui(&ctx, &mut acts);
        self.palette_ui(&ctx, &c, &mut acts);
        for a in acts {
            self.run_command(&ctx, actions::name(a), |app, ctx| app.act(ctx, a));
        }
        // The dialog drawn last takes input: messages and questions go last, as they can
        // come from the windows before them.
        self.restore_window(&ctx);
        self.signature_windows(&ctx);
        self.digital_windows(&ctx);
        self.dialogs(&ctx);
        dlg::name_popups(&ctx);

        let title = match self.tabs.get(self.active) {
            Some(t) => format!("{} - LimarePDF", t.title()),
            None => "LimarePDF".into(),
        };
        if title != self.last_title {
            ctx.send_viewport_cmd(egui::ViewportCommand::Title(title.clone()));
            self.last_title = title;
        }
    }

    /// A normal exit (not a crash, which never gets here): no recovery copies are left.
    fn on_exit(&mut self) {
        self.end_recovery();
        // The window's size, for the next start.
        self.settings.save();
    }
}

/// The window's inner size in points. Not from the viewport's `inner_rect`, which egui
/// leaves out when the window's position isn't known, as on Wayland.
fn window_size(i: &egui::InputState) -> egui::Vec2 {
    i.viewport_rect().size()
}

/// `one` when `n` is 1, else `many` with `{n}` replaced by the number.
fn plural(n: usize, one: &str, many: &str) -> String {
    if n == 1 {
        one.to_string()
    } else {
        many.replace("{n}", &n.to_string())
    }
}

/// What applying redactions removed, page by page, and where the removed text still is.
fn report_text(r: &limarepdf_core::RedactReport) -> String {
    let mut out = String::new();
    for p in &r.pages {
        let mut parts = Vec::new();
        for (n, what) in [
            (p.chars, "characters"),
            (p.images_cut, "pictures blacked out"),
            (p.images, "unreadable pictures removed whole"),
            (p.paths, "parts of drawings"),
            (p.clipped, "drawings cut around the area"),
            (p.annots, "comments, links or fields"),
        ] {
            if n > 0 {
                parts.push(format!("{n} {what}"));
            }
        }
        let what = if parts.is_empty() {
            "nothing was under the marks".to_string()
        } else {
            parts.join(", ")
        };
        out.push_str(&format!("Page {}: {what}\n", p.page + 1));
    }
    out.push_str("\nChecked: nothing readable is left under the marked areas.");
    if !r.elsewhere.is_empty() {
        out.push_str("\n\nThe removed text can still be found elsewhere:");
        for (t, places) in &r.elsewhere {
            out.push_str(&format!(
                "\n• \u{201c}{}\u{201d}: {}",
                limarepdf_core::display_line(t, 60),
                places.join(", ")
            ));
        }
    }
    out.push_str(
        "\n\nTo also remove metadata, attachments and other hidden information, choose Document > \
         Remove hidden information.\n\nSave the document to keep the changes; Undo brings \
         everything back.",
    );
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::page_text;
    use limarepdf_core::{Cipher, Encryption, FontChoice, Restrictions, SignPlace};

    #[test]
    fn window_size_is_known_without_its_position() {
        // On Wayland apps aren't told where their window is, so egui has no `inner_rect`.
        let ctx = egui::Context::default();
        let raw = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(900.0, 600.0),
            )),
            ..Default::default()
        };
        let mut size = egui::Vec2::ZERO;
        let mut out = ctx.run_ui(raw, |ui| size = ui.ctx().input(window_size));
        out.textures_delta.clear();
        assert_eq!(size, egui::vec2(900.0, 600.0));
    }

    /// An app with `n` tabs of one blank page, each with an unsaved change (a second page),
    /// and the last one active. Nothing is read from or written to the user's settings.
    /// A bar where each folded control saves 40 points: 400 with none folded.
    fn need(folded: usize) -> f32 {
        400.0 - 40.0 * folded as f32
    }

    #[test]
    fn tools_fold_in_order_as_room_shrinks_and_return_as_it_grows() {
        assert_eq!(fold_level(0, 400.0, need), 0);
        assert_eq!(fold_level(0, 399.0, need), 1);
        assert_eq!(fold_level(0, 300.0, need), 3);
        // Read and select text never folds: the count stops at every control in FOLDS.
        assert_eq!(fold_level(0, 10.0, need), FOLDS.len());
        assert_eq!(fold_level(FOLDS.len(), 1000.0, need), 0);
        assert_eq!(FOLDS[0], Fold::Commenting);
        // The tools go before Export, and Undo and Redo last.
        assert_eq!(FOLDS[4], Fold::Tool(Tool::Sign));
        assert_eq!(FOLDS[FOLDS.len() - 1], Fold::History);
    }

    #[test]
    fn a_width_at_the_boundary_does_not_fold_and_unfold_by_turns() {
        // Folded one at 399; at 400 to 405 it stays folded, coming back with the margin.
        let mut n = fold_level(0, 399.0, need);
        for room in [400.0, 405.0, 399.5, 404.0] {
            n = fold_level(n, room, need);
            assert_eq!(n, 1, "at {room}");
        }
        assert_eq!(fold_level(n, 400.0 + UNFOLD_MARGIN, need), 0);
    }

    fn app_with(ctx: &egui::Context, n: usize) -> App {
        let mut app = App::with(Settings::from_file(None), Recent::from_file(None));
        for _ in 0..n {
            let mut pdf = Pdf::new();
            pdf.insert_blank(0, 612.0, 792.0).unwrap();
            assert!(app.add_tab(ctx, pdf, None, None, false));
            let t = app.tabs.last_mut().unwrap();
            assert!(t.edit(ctx, |p| p.insert_blank(1, 612.0, 792.0)));
        }
        app.active = n - 1;
        app
    }

    /// Draw the whole window `width` points wide with `events`, and return its AccessKit tree.
    fn window(
        app: &mut App,
        ctx: &egui::Context,
        width: f32,
        events: Vec<egui::Event>,
    ) -> egui::accesskit::TreeUpdate {
        let raw = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(width, 800.0),
            )),
            events,
            ..Default::default()
        };
        let mut frame = eframe::Frame::_new_kittest();
        let mut out = ctx.run_ui(raw, |ui| eframe::App::ui(app, ui, &mut frame));
        out.textures_delta.clear();
        out.platform_output
            .accesskit_update
            .expect("the window's widgets")
    }

    /// The window drawn `width` points wide once settled.
    fn settled_at(app: &mut App, ctx: &egui::Context, width: f32) -> egui::accesskit::TreeUpdate {
        for _ in 0..4 {
            window(app, ctx, width, vec![]);
        }
        window(app, ctx, width, vec![])
    }

    /// The names of the controls Tab can reach, drawn `width` points wide once settled.
    fn focusable(app: &mut App, ctx: &egui::Context, width: f32) -> Vec<Option<String>> {
        let tree = settled_at(app, ctx, width);
        tree.nodes
            .iter()
            .filter(|(_, n)| n.supports_action(egui::accesskit::Action::Focus))
            .map(|(_, n)| n.label().map(str::to_string))
            .collect()
    }

    #[test]
    fn every_control_has_a_name_at_every_width() {
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        theme::install(&ctx);
        let mut app = app_with(&ctx, 1);
        for width in [1400.0, 1000.0, 760.0, 560.0, 400.0] {
            // The panels' resize handles too.
            let tree = settled_at(&mut app, &ctx, width);
            assert_eq!(dlg::unclear_names(&tree), [""; 0], "at {width}");
            let names = focusable(&mut app, &ctx, width);
            for name in [
                "Menu",
                "Open",
                "Save",
                "Read and select text",
                "Tools",
                "Print",
            ] {
                assert!(names.contains(&Some(name.into())), "no {name} at {width}");
            }
        }
    }

    /// Adds to `bad` the controls in `tree` whose names don't say what they are, in
    /// `what`'s state.
    fn note_unclear(tree: &egui::accesskit::TreeUpdate, what: &str, bad: &mut Vec<String>) {
        bad.extend(
            dlg::unclear_names(tree)
                .into_iter()
                .map(|n| format!("{what}: {n}")),
        );
    }

    /// Adds to `bad` the controls whose names don't say what they are in `what`'s state,
    /// wide and narrow.
    fn clear_names(app: &mut App, ctx: &egui::Context, what: &str, bad: &mut Vec<String>) {
        for width in [1400.0, 560.0] {
            let tree = settled_at(app, ctx, width);
            note_unclear(&tree, &format!("{what} at {width}"), bad);
        }
    }

    /// Clicks the control named `name` in the whole window, 1400 points wide.
    fn click_named(app: &mut App, ctx: &egui::Context, name: &str) {
        let tree = settled_at(app, ctx, 1400.0);
        let p = middle(&tree, name);
        window(app, ctx, 1400.0, vec![egui::Event::PointerMoved(p)]);
        window(app, ctx, 1400.0, vec![press(p, true)]);
        window(app, ctx, 1400.0, vec![press(p, false)]);
    }

    /// G06: contextual controls are named too: each tool's inspector, text being added and
    /// a line edited on the page, each kind of comment selected, each side view, Fill and
    /// sign with saved signatures, the lists, menus and command search opened, and Print's
    /// states (V01).
    #[test]
    fn controls_have_clear_names_in_every_state() {
        use crate::signing::{Entry, Saved};
        use limarepdf_core::{BookmarkPlace, Shape, ShapeLook, SignRole, SignStyle, TextStyle};
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        theme::install(&ctx);
        let mut app = app_with(&ctx, 1);
        let mut comments = Vec::new();
        assert!(app.tabs[0].edit(&ctx, |p| {
            p.add_text(
                0,
                (100.0, 200.0),
                "Walrus",
                FontChoice::default(),
                40.0,
                [0.0; 3],
            )?;
            let yellow = [1.0, 1.0, 0.0];
            comments.push(p.add_note(0, (300.0, 600.0), yellow, "Walrus", "Tester")?);
            let style = TextStyle::default();
            let rect = [300.0, 300.0, 500.0, 360.0];
            comments.push(p.add_text_box(0, rect, "Walrus", &style, "Tester")?);
            for (shape, rect) in [
                (Shape::Rect, [100.0, 400.0, 200.0, 500.0]),
                (Shape::Line, [100.0, 520.0, 200.0, 560.0]),
            ] {
                let look = ShapeLook::default();
                let red = Some([1.0, 0.0, 0.0]);
                comments.push(p.add_shape_annotation(0, shape, rect, red, &look, "Tester")?);
            }
            p.add_bookmark("Walrus", 0, BookmarkPlace::End).map(drop)
        }));
        app.tabs[0].query = "Walrus".into();
        app.tabs[0].run_search();
        assert!(!app.tabs[0].hits.is_empty());
        let mut bad = Vec::new();
        clear_names(&mut app, &ctx, "Read", &mut bad);

        for tool in [
            Tool::Select,
            Tool::Text,
            Tool::Image,
            Tool::Rect,
            Tool::Ellipse,
            Tool::Line,
            Tool::Note,
            Tool::TextBox,
            Tool::Pen,
            Tool::Stamp,
            Tool::Redact,
            Tool::Sign,
            Tool::DigitalSign,
        ] {
            // Not through the command, which asks for a picture for Image.
            app.tabs[0].ed.set_tool(tool);
            clear_names(&mut app, &ctx, &format!("{tool:?}"), &mut bad);
        }

        // Text being added on the page, and a line of page text edited there.
        app.act(&ctx, Act::Tool(Tool::Text));
        let (page, k) = page_on_screen(&mut app, &ctx);
        let p = page + egui::vec2(300.0, 100.0) * k;
        window(&mut app, &ctx, 1400.0, click_at(p));
        assert!(app.tabs[0].ed.adding.is_some());
        clear_names(&mut app, &ctx, "Adding text", &mut bad);
        app.act(&ctx, Act::Tool(Tool::Read));
        app.act(&ctx, Act::Tool(Tool::Text));
        let (page, k) = page_on_screen(&mut app, &ctx);
        let p = page + egui::vec2(160.0, 190.0) * k;
        window(&mut app, &ctx, 1400.0, click_at(p));
        assert!(app.tabs[0].ed.typing_on_page());
        clear_names(&mut app, &ctx, "Editing a line", &mut bad);
        app.act(&ctx, Act::Tool(Tool::Read));

        for (n, id) in comments.into_iter().enumerate() {
            app.tabs[0].ed.selection = Selection::Annot { page: 0, id };
            clear_names(&mut app, &ctx, &format!("Comment {n}"), &mut bad);
        }
        app.tabs[0].ed.selection = Selection::None;

        // Each section of the document's inspector opened, then with pages selected.
        for s in [
            Section::Properties,
            Section::Security,
            Section::Crop,
            Section::HeaderFooter,
            Section::Watermark,
            Section::Ocr,
            Section::Scans,
            Section::Export,
            Section::Split,
        ] {
            app.reveal(s);
            clear_names(&mut app, &ctx, &format!("{s:?}"), &mut bad);
        }
        app.reveal_labels();
        clear_names(&mut app, &ctx, "Selected pages", &mut bad);
        app.tabs[0].ed.selection = Selection::None;

        app.side.set_open(true);
        for (side, what) in [
            (Side::Pages, "Pages"),
            (Side::Search, "Search results"),
            (Side::Bookmarks, "Bookmarks"),
            (Side::Comments, "Comments"),
        ] {
            app.tabs[0].side = side;
            clear_names(&mut app, &ctx, what, &mut bad);
        }

        // Fill and sign with saved signatures and initials, and its choices menu open.
        for role in [SignRole::Signature, SignRole::Initials] {
            let saved = Saved::Typed {
                text: "Sam Lee".into(),
                style: SignStyle::ALL[0],
            };
            let e = Entry { saved, keep: false };
            app.settings.sign.add(role, e).unwrap();
        }
        app.act(&ctx, Act::Tool(Tool::Sign));
        clear_names(&mut app, &ctx, "Fill and sign", &mut bad);
        let tree = settled_at(&mut app, &ctx, 1400.0);
        let initials = node(&tree, "Initials 1");
        assert!(initials.is_some(), "no saved initials shown");
        click_named(&mut app, &ctx, "Fill and sign choices");
        let tree = settled_at(&mut app, &ctx, 1400.0);
        assert!(egui::Popup::is_any_open(&ctx), "the menu isn't open");
        note_unclear(&tree, "Fill and sign choices", &mut bad);
        app.act(&ctx, Act::Tool(Tool::Read));

        // The font and size lists, the menu and the command search, open.
        let esc = vec![key_with(Key::Escape, egui::Modifiers::NONE)];
        app.act(&ctx, Act::Tool(Tool::Text));
        for list in ["Font", "Font sizes"] {
            click_named(&mut app, &ctx, list);
            let tree = settled_at(&mut app, &ctx, 1400.0);
            assert!(egui::Popup::is_any_open(&ctx), "{list} isn't open");
            note_unclear(&tree, &format!("{list} open"), &mut bad);
            window(&mut app, &ctx, 1400.0, esc.clone());
        }
        app.act(&ctx, Act::Tool(Tool::Read));
        click_named(&mut app, &ctx, "Menu");
        let tree = settled_at(&mut app, &ctx, 1400.0);
        assert!(egui::Popup::is_any_open(&ctx), "the menu isn't open");
        note_unclear(&tree, "Menu open", &mut bad);
        window(&mut app, &ctx, 1400.0, esc.clone());

        // Print with all pages, with a page range it can't print, with its Print on list
        // open, and for the selected text.
        let printer = print::Printer {
            name: "limarepdf-test-no-such-printer".into(),
            default: true,
        };
        let print = || Box::new(dlg::Print::new(vec![printer.clone()], "test.pdf".into()));
        app.dialog = Dialog::Print(app.asked(), print());
        clear_names(&mut app, &ctx, "Print", &mut bad);
        let mut range = print();
        range.all = false;
        range.pages = "9".into();
        range.error = Some("There is no page 9.".into());
        app.dialog = Dialog::Print(app.asked(), range);
        clear_names(&mut app, &ctx, "Print a page range", &mut bad);
        click_named(&mut app, &ctx, "Print on");
        let tree = settled_at(&mut app, &ctx, 1400.0);
        assert!(egui::Popup::is_any_open(&ctx), "Print on isn't open");
        note_unclear(&tree, "Print on open", &mut bad);
        window(&mut app, &ctx, 1400.0, esc);
        let mut selected = print();
        selected.selection = Some(dlg::Layout::Original);
        app.dialog = Dialog::Print(app.asked(), selected);
        clear_names(&mut app, &ctx, "Print selected text", &mut bad);
        app.dialog = Dialog::None;

        app.act(&ctx, Act::Palette);
        clear_names(&mut app, &ctx, "Command search", &mut bad);
        assert_eq!(bad, [""; 0]);
    }

    /// Batch 7b: the command search in the dialogs' look, as a launcher (option A of the
    /// mockup): 16 points around it, its field as tall as the dialogs' and holding the
    /// keyboard, a rule, then rows as tall, lined up with the field; Enter runs the row
    /// picked and Esc closes it, wide and narrow.
    #[test]
    fn command_search_has_the_dialog_look() {
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        theme::install(&ctx);
        let mut app = app_with(&ctx, 1);
        for width in [1400.0, 400.0] {
            app.act(&ctx, Act::Palette);
            let tree = settled_at(&mut app, &ctx, width);
            let (field, focused) = node(&tree, "Command").expect("the field");
            assert!(focused, "{width}: the keyboard isn't in the field");
            assert!(field.height() >= theme::DIALOG_CONTROL - 0.5, "{width}");
            let id = egui::Id::new("palette");
            let at = ctx.memory(|m| m.area_rect(id)).expect("the window");
            let inset = 16.0 + ctx.global_style().visuals.window_stroke.width;
            assert!((field.left() - at.left() - inset).abs() < 1.0, "{width}");
            assert!((field.top() - at.top() - inset).abs() < 1.0, "{width}");
            let (row, _) = node(&tree, "Open…").expect("a command's row");
            assert!(row.height() >= theme::DIALOG_CONTROL - 0.5, "{width}");
            assert!(row.top() >= field.bottom() + 12.0, "{width}: no rule");
            assert!((row.left() - field.left()).abs() < 1.0, "{width}");
            assert!(at.contains_rect(row), "{width}");
            let esc = key_with(Key::Escape, egui::Modifiers::NONE);
            window(&mut app, &ctx, width, vec![esc]);
            assert!(app.palette.is_none(), "{width}: Esc didn't close it");
        }
        app.act(&ctx, Act::Palette);
        settled_at(&mut app, &ctx, 1400.0);
        let typed = egui::Event::Text("About".into());
        window(&mut app, &ctx, 1400.0, vec![typed]);
        window(&mut app, &ctx, 1400.0, vec![]);
        let enter = key_with(Key::Enter, egui::Modifiers::NONE);
        window(&mut app, &ctx, 1400.0, vec![enter]);
        assert!(app.palette.is_none());
        let about = matches!(app.dialog, Dialog::About(_));
        assert!(about, "Enter didn't run About");
    }

    /// G06: Bold and Italic say whether they are on, and Tab goes through the new text's
    /// look in the order shown.
    #[test]
    fn text_look_controls_say_their_state_in_reading_order() {
        use egui::accesskit::Toggled;
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        theme::install(&ctx);
        let mut app = app_with(&ctx, 1);
        app.act(&ctx, Act::Tool(Tool::Text));
        let toggled = |tree: &egui::accesskit::TreeUpdate, name: &str| {
            let n = tree.nodes.iter().find(|(_, n)| n.label() == Some(name));
            n.and_then(|(_, n)| n.toggled())
        };
        let tree = settled_at(&mut app, &ctx, 1400.0);
        assert_eq!(toggled(&tree, "Bold"), Some(Toggled::False));
        assert_eq!(toggled(&tree, "Italic"), Some(Toggled::False));
        app.tabs[0].ed.tfont.bold = true;
        let tree = settled_at(&mut app, &ctx, 1400.0);
        assert_eq!(toggled(&tree, "Bold"), Some(Toggled::True));
        assert_eq!(toggled(&tree, "Italic"), Some(Toggled::False));
        let font = tree.nodes.iter().find(|(_, n)| n.label() == Some("Font"));
        let value = font.and_then(|(_, n)| n.value());
        assert_eq!(value, Some("Helvetica"));

        let reached = tab_walk(&mut app, &ctx, 1400.0, 80);
        let look = [
            "Font",
            "Bold",
            "Italic",
            "Font size",
            "Font sizes",
            "Text colour",
        ];
        let at = reached.iter().position(|n| n == "Font").expect("no Font");
        assert_eq!(reached[at..at + look.len()], look);

        // The list, once open, is named after the control that opened it.
        click_named(&mut app, &ctx, "Font");
        let tree = settled_at(&mut app, &ctx, 1400.0);
        let menu = tree
            .nodes
            .iter()
            .find(|(_, n)| n.role() == egui::accesskit::Role::Menu);
        assert_eq!(menu.and_then(|(_, n)| n.label()), Some("Font"));
    }

    /// G06: Fill and sign's saved signatures and marks say which one is picked.
    #[test]
    fn fill_and_sign_says_which_choice_is_picked() {
        use crate::signing::{Entry, Saved, SignPick};
        use egui::accesskit::Toggled;
        use limarepdf_core::{Mark, SignRole, SignStyle};
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        theme::install(&ctx);
        let mut app = app_with(&ctx, 1);
        for text in ["Sam Lee", "S. Lee"] {
            let saved = Saved::Typed {
                text: text.into(),
                style: SignStyle::ALL[0],
            };
            let e = Entry { saved, keep: false };
            app.settings.sign.add(SignRole::Signature, e).unwrap();
        }
        app.act(&ctx, Act::Tool(Tool::Sign));
        let toggled = |tree: &egui::accesskit::TreeUpdate, name: &str| {
            let n = tree.nodes.iter().find(|(_, n)| n.label() == Some(name));
            n.and_then(|(_, n)| n.toggled())
        };
        app.tabs[0].ed.sign.pick = SignPick::Saved(SignRole::Signature, 1);
        let tree = settled_at(&mut app, &ctx, 1400.0);
        assert_eq!(toggled(&tree, "Signature 1"), Some(Toggled::False));
        assert_eq!(toggled(&tree, "Signature 2"), Some(Toggled::True));
        assert_eq!(toggled(&tree, "Tick"), Some(Toggled::False));
        app.tabs[0].ed.sign.pick = SignPick::Mark(Mark::Tick);
        let tree = settled_at(&mut app, &ctx, 1400.0);
        assert_eq!(toggled(&tree, "Signature 2"), Some(Toggled::False));
        assert_eq!(toggled(&tree, "Tick"), Some(Toggled::True));
        assert_eq!(toggled(&tree, "Cross"), Some(Toggled::False));
    }

    #[test]
    fn fill_and_sign_leaves_digital_signing_to_the_document_menu() {
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        theme::install(&ctx);
        let mut app = app_with(&ctx, 1);
        app.act(&ctx, Act::Tool(Tool::Sign));
        let names = focusable(&mut app, &ctx, 1400.0);
        assert!(names.contains(&Some("Finish signing…".into())));
        let digital = names
            .iter()
            .flatten()
            .find(|n| n.to_lowercase().contains("digital"));
        assert_eq!(digital, None);
        // Signing and certifying are still in Document > Digital signatures.
        for act in [Act::SignDigital, Act::Certify] {
            let a = actions::get(act);
            assert_eq!(
                (a.group, a.submenu),
                (Group::Document, "Digital signatures")
            );
            app.act(&ctx, act);
            assert_eq!(app.tabs[0].ed.tool, Tool::DigitalSign);
            assert_eq!(app.tabs[0].ed.digital_certify, act == Act::Certify);
            app.act(&ctx, Act::Tool(Tool::Read));
        }
    }

    #[test]
    fn tab_goes_through_the_bars_in_reading_order() {
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        theme::install(&ctx);
        let mut app = app_with(&ctx, 1);
        let mut reached = tab_walk(&mut app, &ctx, 1400.0, 30);
        // Not the zoom level, which depends on the window.
        reached.retain(|name| !name.ends_with('%'));
        let bars = [
            "Menu",
            "Open",
            "Save",
            "Export",
            "Undo", // Redo is disabled: nothing was undone.
            "Read and select text",
            "Add text",
            "Add image",
            "Mark areas for redaction",
            "Fill and sign",
            "Fill and sign choices",
            "Add a note",
            "Commenting tools",
            "Tools",
            "Search",
            "Print",
            "Untitled *",
            "Close Untitled *",
            "Page layout",
            "Fit width",
            "Zoom out",
            "Zoom in",
            "Pages",
            "Search results",
            "Bookmarks",
            "Comments",
            "Page 1",
        ];
        assert_eq!(reached[..bars.len()], bars);
    }

    /// The screen bounds of the node named `name` in `tree`, and whether it has the keyboard.
    fn node(tree: &egui::accesskit::TreeUpdate, name: &str) -> Option<(egui::Rect, bool)> {
        let (id, n) = tree.nodes.iter().find(|(_, n)| n.label() == Some(name))?;
        let b = n.bounds()?;
        let r = egui::Rect::from_min_max(
            egui::pos2(b.x0 as f32, b.y0 as f32),
            egui::pos2(b.x1 as f32, b.y1 as f32),
        );
        Some((r, tree.focus == *id))
    }

    #[test]
    fn new_text_is_typed_on_the_page_and_added_in_one_step() {
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        theme::install(&ctx);
        let mut app = app_with(&ctx, 1);
        app.act(&ctx, Act::Tool(Tool::Text));
        app.tabs[0].ed.tsize = 14.0;
        let mut tree = window(&mut app, &ctx, 1400.0, vec![]);
        for _ in 0..4 {
            tree = window(&mut app, &ctx, 1400.0, vec![]);
        }
        // The page's place on screen, and a point on it in page points.
        let (page, _) = node(&tree, "Page 1").expect("the page");
        let k = page.width() / 612.0;
        let at = |x: f32, y: f32| page.min + egui::vec2(x, y) * k;
        let click = |p: egui::Pos2| {
            let press = |pressed| egui::Event::PointerButton {
                pos: p,
                button: egui::PointerButton::Primary,
                pressed,
                modifiers: Default::default(),
            };
            vec![egui::Event::PointerMoved(p), press(true), press(false)]
        };
        let key = |key, modifiers| egui::Event::Key {
            key,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers,
        };
        window(&mut app, &ctx, 1400.0, click(at(100.0, 200.0)));
        for _ in 0..2 {
            tree = window(&mut app, &ctx, 1400.0, vec![]);
        }
        // The editor is on the page, named, with the keyboard, its first baseline on the
        // click: the text's top is Liberation Sans's ascent (0.905 em) above it.
        let (r, focused) = node(&tree, page_text::NAME).expect("the editor");
        assert!(focused, "the editor doesn't have the keyboard");
        let click_y = at(100.0, 200.0).y;
        assert!((r.min.x - at(100.0, 200.0).x).abs() < 1.5, "{r:?}");
        assert!(
            (r.min.y - (click_y - 0.905 * 14.0 * k)).abs() < 1.5,
            "{r:?}"
        );
        // Typing, Enter for a second line, a character that can't be written yet, and
        // Ctrl+B. Nothing is in the document yet.
        let none = egui::Modifiers::NONE;
        let typed = vec![
            egui::Event::Text("Walrus".into()),
            key(Key::Enter, none),
            egui::Event::Text("Seal Ω".into()),
            key(Key::B, egui::Modifiers::COMMAND),
        ];
        window(&mut app, &ctx, 1400.0, typed);
        let t = &app.tabs[0];
        assert_eq!(
            t.ed.adding.as_ref().map(|a| a.text.as_str()),
            Some("Walrus\nSeal ")
        );
        assert!(t.ed.tfont.bold);
        assert!(t.unsaved() && t.ed.pending());
        let lines = |app: &App| -> Vec<(String, [f32; 4], Option<String>)> {
            let objs = app.tabs[0].pdf.page_objects(0).unwrap();
            objs.into_iter()
                .filter_map(|o| Some((o.text?, o.bbox, o.font_name)))
                .collect()
        };
        assert!(lines(&app).is_empty());
        tree = window(&mut app, &ctx, 1400.0, vec![]);
        assert!(
            tree.nodes
                .iter()
                .any(|(_, n)| n.value() == Some(page_text::REFUSED)),
            "no note about the refused character"
        );
        // Bold switched off in the inspector applies to the text being typed, which takes
        // the keyboard back: Ctrl+B there switches it on again.
        let (b, _) = node(&tree, "Bold").expect("the inspector's Bold");
        window(&mut app, &ctx, 1400.0, click(b.center()));
        assert!(!app.tabs[0].ed.tfont.bold);
        for _ in 0..2 {
            tree = window(&mut app, &ctx, 1400.0, vec![]);
        }
        assert!(
            node(&tree, page_text::NAME).is_some_and(|(_, f)| f),
            "no keyboard back"
        );
        window(
            &mut app,
            &ctx,
            1400.0,
            vec![key(Key::B, egui::Modifiers::COMMAND)],
        );
        assert!(app.tabs[0].ed.tfont.bold);
        // Dragging the outline (just left of the text) moves the text being typed.
        let (r, _) = node(&tree, page_text::NAME).expect("the editor");
        let grip = egui::pos2(r.min.x - 4.5, r.center().y);
        let button = |pressed| egui::Event::PointerButton {
            pos: grip + egui::vec2(if pressed { 0.0 } else { 60.0 }, 0.0),
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: Default::default(),
        };
        window(
            &mut app,
            &ctx,
            1400.0,
            vec![egui::Event::PointerMoved(grip), button(true)],
        );
        for dx in [20.0, 40.0, 60.0] {
            let to = grip + egui::vec2(dx, 0.0);
            window(&mut app, &ctx, 1400.0, vec![egui::Event::PointerMoved(to)]);
        }
        window(&mut app, &ctx, 1400.0, vec![button(false)]);
        let x = app.tabs[0].ed.adding.as_ref().map_or(0.0, |a| a.at.0);
        assert!((x - (100.0 + 60.0 / k)).abs() < 0.01, "moved to {x}");
        assert_eq!(app.tabs[0].ed.adding.as_ref().map(|a| a.at.1), Some(200.0));
        // A click elsewhere on the page adds both lines, in bold, the second 1.2 times the
        // size below; the read tool comes back.
        window(&mut app, &ctx, 1400.0, click(at(300.0, 300.0)));
        for _ in 0..2 {
            window(&mut app, &ctx, 1400.0, vec![]);
        }
        let got = lines(&app);
        assert_eq!(got.len(), 2, "{got:?}");
        assert_eq!((got[0].0.as_str(), got[1].0.as_str()), ("Walrus", "Seal"));
        for (_, b, font) in &got {
            assert!((b[0] - x).abs() < 1.0, "{b:?}");
            assert!(
                font.as_deref()
                    .is_some_and(|f| f.contains("Helvetica-Bold")),
                "{font:?}"
            );
        }
        assert!(got[0].1[1] < 200.0 && got[0].1[3] > 200.0, "{:?}", got[0].1);
        assert!(((got[1].1[1] - got[0].1[1]) - 16.8).abs() < 0.5);
        let t = &app.tabs[0];
        assert!(t.ed.adding.is_none() && !t.ed.pending());
        assert_eq!(t.ed.tool, Tool::Read);
        // One step: Undo takes both lines back.
        app.act(&ctx, Act::Undo);
        assert!(lines(&app).is_empty());
    }

    #[test]
    fn esc_keeps_new_text_and_empty_text_adds_nothing() {
        let ctx = egui::Context::default();
        let mut app = app_with(&ctx, 1);
        app.act(&ctx, Act::Tool(Tool::Text));
        let ed = &mut app.tabs[0].ed;
        // Nothing typed: Esc closes the editor, adding nothing; Esc again leaves the tool.
        ed.adding = Some(page_text::PageText::new(0, (72.0, 72.0)));
        ed.escape();
        assert!(ed.adding.is_none() && ed.actions.is_empty());
        assert_eq!(ed.tool, Tool::Text);
        // Spaces and blank lines are nothing too.
        ed.adding = Some(page_text::PageText::new(0, (72.0, 72.0)));
        if let Some(a) = ed.adding.as_mut() {
            a.text = " \n ".into();
        }
        assert!(!ed.pending());
        ed.escape();
        assert!(ed.actions.is_empty());
        // Typed text: Esc keeps it, in the inspector's look at that moment.
        ed.adding = Some(page_text::PageText::new(0, (72.0, 90.0)));
        if let Some(a) = ed.adding.as_mut() {
            a.text = "Walrus".into();
        }
        ed.tsize = 20.0;
        ed.tcolor = [1.0, 0.0, 0.0];
        ed.escape();
        assert!(matches!(
            ed.actions.as_slice(),
            [EdAction::AddText(0, (72.0, 90.0), t, look)]
                if t == "Walrus" && look.size == 20.0 && look.color == [1.0, 0.0, 0.0]
        ));
        app.tabs[0].apply_actions(&ctx);
        let objs = app.tabs[0].pdf.page_objects(0).unwrap();
        assert!(objs.iter().any(|o| o.text.as_deref() == Some("Walrus")
            && o.font_size == Some(20.0)
            && o.fill == Some([1.0, 0.0, 0.0])));
        // Choosing another tool while typing keeps the text, and that tool.
        let ed = &mut app.tabs[0].ed;
        ed.set_tool(Tool::Text);
        ed.adding = Some(page_text::PageText::new(0, (72.0, 400.0)));
        if let Some(a) = ed.adding.as_mut() {
            a.text = "Seal".into();
        }
        ed.set_tool(Tool::Rect);
        app.tabs[0].apply_actions(&ctx);
        assert_eq!(app.tabs[0].ed.tool, Tool::Rect);
        let objs = app.tabs[0].pdf.page_objects(0).unwrap();
        assert!(objs.iter().any(|o| o.text.as_deref() == Some("Seal")));
    }

    #[test]
    fn a_click_on_page_text_with_the_text_tool_edits_its_line() {
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        theme::install(&ctx);
        let mut app = app_with(&ctx, 1);
        assert!(app.tabs[0].edit(&ctx, |p| {
            p.add_text(
                0,
                (100.0, 200.0),
                "Walrus",
                FontChoice::default(),
                40.0,
                [0.0; 3],
            )
        }));
        app.act(&ctx, Act::Tool(Tool::Text));
        let mut tree = window(&mut app, &ctx, 1400.0, vec![]);
        for _ in 0..4 {
            tree = window(&mut app, &ctx, 1400.0, vec![]);
        }
        let (page, _) = node(&tree, "Page 1").expect("the page");
        let k = page.width() / 612.0;
        // In the middle of the word, a little above the baseline.
        let p = page.min + egui::vec2(100.0 + 3.0 * 40.0 * 0.5, 190.0) * k;
        let press = |pressed| egui::Event::PointerButton {
            pos: p,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: Default::default(),
        };
        window(
            &mut app,
            &ctx,
            1400.0,
            vec![egui::Event::PointerMoved(p), press(true), press(false)],
        );
        let ed = &app.tabs[0].ed;
        assert!(ed.adding.is_none(), "new text started on page text");
        assert!(ed.typing_on_page(), "the line isn't being edited");
    }

    /// A line of page text is edited where it is, drawn as it will be written, over the
    /// line hidden in the colour around it; Esc keeps the change, one Undo takes it back.
    #[test]
    fn a_line_is_edited_in_place_and_esc_keeps_it() {
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        theme::install(&ctx);
        let mut app = app_with(&ctx, 1);
        // White Courier on a blue bar: each letter is 0.6 em, 24 points at 40.
        let courier = FontChoice::from_name("Courier");
        assert!(app.tabs[0].edit(&ctx, |p| {
            let blue = limarepdf_core::PathStyle {
                fill: Some([0.0, 0.0, 1.0]),
                stroke: None,
                line_width: 1.0,
            };
            p.add_shape(
                0,
                limarepdf_core::Shape::Rect,
                [80.0, 150.0, 400.0, 230.0],
                &blue,
            )?;
            p.add_text(0, (100.0, 200.0), "Walrus", courier, 40.0, [1.0; 3])
        }));
        app.act(&ctx, Act::Tool(Tool::Text));
        let mut tree = window(&mut app, &ctx, 1400.0, vec![]);
        for _ in 0..4 {
            tree = window(&mut app, &ctx, 1400.0, vec![]);
        }
        let (page, _) = node(&tree, "Page 1").expect("the page");
        let k = page.width() / 612.0;
        let at = |x: f32, y: f32| page.min + egui::vec2(x, y) * k;
        let press = |p: egui::Pos2, pressed| egui::Event::PointerButton {
            pos: p,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: Default::default(),
        };
        let key = |key, modifiers| egui::Event::Key {
            key,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers,
        };
        // A click just past the third letter puts the caret after it.
        let p = at(100.0 + 3.0 * 24.0 + 4.0, 190.0);
        window(
            &mut app,
            &ctx,
            1400.0,
            vec![
                egui::Event::PointerMoved(p),
                press(p, true),
                press(p, false),
            ],
        );
        for _ in 0..2 {
            tree = window(&mut app, &ctx, 1400.0, vec![]);
        }
        let name = "Editing text: Walrus";
        let (r, focused) = node(&tree, name).expect("the line editor");
        assert!(focused, "the line editor doesn't have the keyboard");
        // On the line's baseline, from its left end.
        assert!((r.min.x - at(100.0, 200.0).x).abs() < 1.5, "{r:?}");
        assert!(
            r.min.y < at(100.0, 200.0).y && r.max.y > at(100.0, 200.0).y,
            "{r:?}"
        );
        let id = egui::Id::new(("line-edit", 0));
        let caret = egui::TextEdit::load_state(&ctx, id)
            .and_then(|s| s.cursor.char_range())
            .map(|c| c.primary.index.0);
        assert_eq!(caret, Some(3));
        // The line under the editor is hidden in the bar's blue.
        for _ in 0..500 {
            if app.tabs[0].ed.cover.is_some() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
            window(&mut app, &ctx, 1400.0, vec![]);
        }
        let c = app.tabs[0]
            .ed
            .cover
            .expect("no colour to hide the line with");
        assert!(c.r() < 8 && c.g() < 8 && c.b() > 247, "{c:?}");
        // Typing, a character that can't be written, and Ctrl+B for the whole line.
        let typed = vec![
            egui::Event::Text("XΩ".into()),
            key(Key::B, egui::Modifiers::COMMAND),
        ];
        window(&mut app, &ctx, 1400.0, typed);
        tree = window(&mut app, &ctx, 1400.0, vec![]);
        let value = |tree: &egui::accesskit::TreeUpdate| {
            let n = tree.nodes.iter().find(|(_, n)| n.label() == Some(name));
            n.and_then(|(_, n)| n.value().map(str::to_string))
        };
        assert_eq!(value(&tree).as_deref(), Some("WalXrus"));
        assert!(
            tree.nodes
                .iter()
                .any(|(_, n)| n.value() == Some(page_text::REFUSED)),
            "no note about the refused character"
        );
        let line = |app: &App| {
            let objs = app.tabs[0].pdf.page_objects(0).unwrap();
            objs.into_iter()
                .find_map(|o| Some((o.text?, o.font_name?, o.baseline?)))
        };
        assert_eq!(line(&app).map(|l| l.0).as_deref(), Some("Walrus"));
        // Esc keeps it, in Courier Bold now, on the same baseline.
        window(
            &mut app,
            &ctx,
            1400.0,
            vec![key(Key::Escape, egui::Modifiers::NONE)],
        );
        for _ in 0..2 {
            window(&mut app, &ctx, 1400.0, vec![]);
        }
        let (text, font, (x, y)) = line(&app).expect("the line");
        assert_eq!((text.as_str(), font.as_str()), ("WalXrus", "Courier-Bold"));
        assert!(
            (x - 100.0).abs() < 0.01 && (y - 200.0).abs() < 0.01,
            "{x}, {y}"
        );
        let ed = &app.tabs[0].ed;
        assert!(!ed.typing_on_page() && !ed.pending());
        app.act(&ctx, Act::Undo);
        let (text, font, _) = line(&app).expect("the line");
        assert_eq!((text.as_str(), font.as_str()), ("Walrus", "Courier"));
    }

    #[test]
    fn a_double_click_with_the_read_tool_edits_its_line() {
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        theme::install(&ctx);
        let mut app = app_with(&ctx, 1);
        let times = FontChoice::from_name("Times-Roman");
        assert!(app.tabs[0].edit(&ctx, |p| {
            p.add_text(
                0,
                (100.0, 200.0),
                "Walrus like cold water",
                times,
                20.0,
                [0.0; 3],
            )
        }));
        let mut tree = window(&mut app, &ctx, 1400.0, vec![]);
        for _ in 0..4 {
            tree = window(&mut app, &ctx, 1400.0, vec![]);
        }
        let (page, _) = node(&tree, "Page 1").expect("the page");
        let k = page.width() / 612.0;
        let words = app.tabs[0].pdf.page_words(0).unwrap();
        let cold = words.iter().find(|w| w.text == "cold").unwrap().bbox;
        let p = page.min + egui::vec2((cold[0] + cold[2]) / 2.0, 195.0) * k;
        let press = |pressed| egui::Event::PointerButton {
            pos: p,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: Default::default(),
        };
        let mut raw = |events, time| {
            let raw = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1400.0, 800.0),
                )),
                time: Some(time),
                events,
                ..Default::default()
            };
            let mut frame = eframe::Frame::_new_kittest();
            let mut out = ctx.run_ui(raw, |ui| eframe::App::ui(&mut app, ui, &mut frame));
            out.textures_delta.clear();
            out.platform_output.accesskit_update.expect("the window")
        };
        raw(vec![egui::Event::PointerMoved(p)], 10.0);
        raw(vec![press(true)], 10.02);
        raw(vec![press(false)], 10.08);
        raw(vec![press(true)], 10.16);
        raw(vec![press(false)], 10.22);
        let mut tree = raw(vec![], 10.3);
        for i in 0..3 {
            tree = raw(vec![], 10.4 + i as f64 * 0.1);
        }
        let name = "Editing text: Walrus like cold water";
        let (_, focused) = node(&tree, name).expect("the line editor");
        assert!(focused, "the line editor doesn't have the keyboard");
        // The word double-clicked is selected.
        let id = egui::Id::new(("line-edit", 0));
        let picked = egui::TextEdit::load_state(&ctx, id)
            .and_then(|s| s.cursor.char_range())
            .map(|c| c.as_sorted_char_range())
            .map(|r| (r.start.0, r.end.0));
        assert_eq!(picked, Some((12, 16)));
    }

    /// Move or resize text lets the line move with the read tool, which stays on: a click
    /// away ends it, with no other tool left behind.
    #[test]
    fn move_or_resize_text_stays_in_the_read_tool() {
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        theme::install(&ctx);
        let mut app = app_with(&ctx, 1);
        let font = FontChoice::default();
        assert!(app.tabs[0].edit(&ctx, |p| {
            p.add_text(0, (100.0, 200.0), "Walrus", font, 20.0, [0.0; 3])
        }));
        let mut tree = window(&mut app, &ctx, 1400.0, vec![]);
        for _ in 0..4 {
            tree = window(&mut app, &ctx, 1400.0, vec![]);
        }
        let (page, _) = node(&tree, "Page 1").expect("the page");
        let k = page.width() / 612.0;
        let at = |x: f32, y: f32| page.min + egui::vec2(x, y) * k;
        let mut time = 10.0;
        let mut frame = |app: &mut App, events| {
            time += 0.05;
            let raw = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1400.0, 800.0),
                )),
                time: Some(time),
                events,
                ..Default::default()
            };
            let mut f = eframe::Frame::_new_kittest();
            let mut out = ctx.run_ui(raw, |ui| eframe::App::ui(app, ui, &mut f));
            out.textures_delta.clear();
        };
        let button = |p: egui::Pos2, pressed| egui::Event::PointerButton {
            pos: p,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: Default::default(),
        };
        // Double-click the line, then Move or resize text.
        let p = at(130.0, 195.0);
        frame(&mut app, vec![egui::Event::PointerMoved(p)]);
        for pressed in [true, false, true, false] {
            frame(&mut app, vec![button(p, pressed)]);
        }
        frame(&mut app, vec![]);
        assert!(
            app.tabs[0].ed.typing_on_page(),
            "the line isn't being edited"
        );
        app.act(&ctx, Act::MoveText);
        frame(&mut app, vec![]);
        let ed = &app.tabs[0].ed;
        assert_eq!(ed.tool, Tool::Read);
        assert!(ed.object_moves() && !ed.typing_on_page());
        // Dragged 50 points right.
        frame(
            &mut app,
            vec![egui::Event::PointerMoved(p), button(p, true)],
        );
        for dx in [10.0, 30.0, 50.0] {
            let to = p + egui::vec2(dx * k, 0.0);
            frame(&mut app, vec![egui::Event::PointerMoved(to)]);
        }
        frame(&mut app, vec![button(p + egui::vec2(50.0 * k, 0.0), false)]);
        for _ in 0..2 {
            frame(&mut app, vec![]);
        }
        let x = app.tabs[0].pdf.page_objects(0).unwrap()[0].bbox[0];
        assert!((x - 150.0).abs() < 0.5, "moved to {x}");
        assert_eq!(app.tabs[0].ed.tool, Tool::Read);
        // A click away: still the read tool, and the line no longer moves.
        for _ in 0..20 {
            frame(&mut app, vec![]);
        }
        let away = at(300.0, 300.0);
        frame(&mut app, vec![egui::Event::PointerMoved(away)]);
        frame(&mut app, vec![button(away, true)]);
        frame(&mut app, vec![button(away, false)]);
        frame(&mut app, vec![]);
        let ed = &mut app.tabs[0].ed;
        assert_eq!(ed.tool, Tool::Read);
        assert_eq!(ed.sel(), None);
        ed.select_object(0, 0);
        assert!(
            !ed.object_moves(),
            "page text still moves with the read tool"
        );
    }

    #[test]
    fn a_drag_with_the_text_tool_selects_text() {
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        theme::install(&ctx);
        let mut app = app_with(&ctx, 1);
        let courier = FontChoice::from_name("Courier");
        assert!(app.tabs[0].edit(&ctx, |p| {
            p.add_text(0, (100.0, 200.0), "Walrus seal", courier, 40.0, [0.0; 3])
        }));
        app.act(&ctx, Act::Tool(Tool::Text));
        let mut tree = window(&mut app, &ctx, 1400.0, vec![]);
        for _ in 0..4 {
            tree = window(&mut app, &ctx, 1400.0, vec![]);
        }
        let (page, _) = node(&tree, "Page 1").expect("the page");
        let k = page.width() / 612.0;
        let at = |x: f32| page.min + egui::vec2(x, 190.0) * k;
        let press = |p: egui::Pos2, pressed| egui::Event::PointerButton {
            pos: p,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: Default::default(),
        };
        let (from, to) = (at(102.0), at(100.0 + 10.0 * 24.0));
        window(
            &mut app,
            &ctx,
            1400.0,
            vec![egui::Event::PointerMoved(from), press(from, true)],
        );
        for x in [150.0, 220.0, 100.0 + 10.0 * 24.0] {
            window(
                &mut app,
                &ctx,
                1400.0,
                vec![egui::Event::PointerMoved(at(x))],
            );
        }
        window(&mut app, &ctx, 1400.0, vec![press(to, false)]);
        let ed = &app.tabs[0].ed;
        assert!(ed.selection.words().is_some(), "{:?}", ed.selection);
        assert!(!ed.typing_on_page() && ed.adding.is_none());
        assert_eq!(ed.tool, Tool::Text);
    }

    /// A window with one page, drawn until settled: the page's screen place for page
    /// points, and its pixels per point.
    fn page_on_screen(app: &mut App, ctx: &egui::Context) -> (egui::Pos2, f32) {
        let mut tree = window(app, ctx, 1400.0, vec![]);
        for _ in 0..4 {
            tree = window(app, ctx, 1400.0, vec![]);
        }
        let (page, _) = node(&tree, "Page 1").expect("the page");
        (page.min, page.width() / 612.0)
    }

    fn press(p: egui::Pos2, pressed: bool) -> egui::Event {
        egui::Event::PointerButton {
            pos: p,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: Default::default(),
        }
    }

    fn click_at(p: egui::Pos2) -> Vec<egui::Event> {
        vec![
            egui::Event::PointerMoved(p),
            press(p, true),
            press(p, false),
        ]
    }

    fn key_with(key: Key, modifiers: egui::Modifiers) -> egui::Event {
        egui::Event::Key {
            key,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers,
        }
    }

    /// The comments of the document's annotations.
    fn comments(app: &App) -> Vec<String> {
        let annots = app.tabs[0].pdf.annotations();
        annots.into_iter().map(|a| a.contents).collect()
    }

    /// A note being typed has no buttons: Esc, a click outside it and Ctrl+Enter each keep
    /// what was typed, as one undo step; an empty one adds nothing; another tool stays.
    #[test]
    fn a_note_is_kept_by_esc_a_click_outside_or_ctrl_enter() {
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        theme::install(&ctx);
        let mut app = app_with(&ctx, 1);
        let (min, k) = page_on_screen(&mut app, &ctx);
        let at = |x: f32, y: f32| min + egui::vec2(x, y) * k;
        let typing =
            |ctx: &egui::Context| ctx.memory(|m| m.has_focus(egui::Id::new(("note-text", 0usize))));
        // Put a note down at (x, y) with the Note tool and type `text` into it.
        let note = |app: &mut App, x: f32, y: f32, text: &str| {
            app.act(&ctx, Act::Tool(Tool::Note));
            window(app, &ctx, 1400.0, click_at(at(x, y)));
            let tree = window(app, &ctx, 1400.0, vec![]);
            assert!(typing(&ctx), "the note doesn't have the keyboard");
            for name in ["Done", "Cancel"] {
                assert!(node(&tree, name).is_none(), "the note has {name}");
            }
            window(app, &ctx, 1400.0, vec![egui::Event::Text(text.into())]);
            assert_eq!(
                app.tabs[0].ed.writing.as_ref().map(|w| w.text.as_str()),
                Some(text)
            );
        };
        let settle = |app: &mut App| {
            for _ in 0..2 {
                window(app, &ctx, 1400.0, vec![]);
            }
        };
        let none = egui::Modifiers::NONE;
        // Esc keeps it; the read tool comes back with it selected; one Undo removes it.
        note(&mut app, 100.0, 100.0, "Walrus");
        window(&mut app, &ctx, 1400.0, vec![key_with(Key::Escape, none)]);
        settle(&mut app);
        assert_eq!(comments(&app), ["Walrus"]);
        let ed = &app.tabs[0].ed;
        assert!(ed.writing.is_none() && !ed.pending());
        assert_eq!(ed.tool, Tool::Read);
        assert!(matches!(ed.selection, Selection::Annot { page: 0, .. }));
        app.act(&ctx, Act::Undo);
        assert!(comments(&app).is_empty());
        // A click on the page outside the note keeps it and puts down nothing else; the
        // read tool comes back. Empty, the same click adds nothing and starts no note.
        // Ctrl+Enter keeps the next.
        note(&mut app, 100.0, 100.0, "Seal");
        window(&mut app, &ctx, 1400.0, click_at(at(300.0, 400.0)));
        settle(&mut app);
        assert_eq!(comments(&app), ["Seal"]);
        assert!(app.tabs[0].ed.writing.is_none(), "the click started a note");
        assert_eq!(app.tabs[0].ed.tool, Tool::Read);
        note(&mut app, 100.0, 100.0, "");
        window(&mut app, &ctx, 1400.0, click_at(at(300.0, 400.0)));
        settle(&mut app);
        assert_eq!(comments(&app), ["Seal"]);
        assert!(app.tabs[0].ed.writing.is_none(), "the click started a note");
        assert_eq!(app.tabs[0].ed.tool, Tool::Note);
        note(&mut app, 200.0, 200.0, "Puffin");
        let ctrl_enter = key_with(Key::Enter, egui::Modifiers::COMMAND);
        window(&mut app, &ctx, 1400.0, vec![ctrl_enter]);
        settle(&mut app);
        assert_eq!(comments(&app), ["Seal", "Puffin"]);
        // Ctrl+Z while typing undoes the typing, not the document (its second page stays).
        note(&mut app, 300.0, 300.0, "");
        let ctrl_z = key_with(Key::Z, egui::Modifiers::COMMAND);
        window(&mut app, &ctx, 1400.0, vec![ctrl_z]);
        assert_eq!(app.tabs[0].page_count(), 2);
        assert_eq!(comments(&app).len(), 2);
        // Empty: Esc adds nothing.
        window(&mut app, &ctx, 1400.0, vec![key_with(Key::Escape, none)]);
        settle(&mut app);
        assert_eq!(comments(&app).len(), 2);
        assert!(app.tabs[0].ed.writing.is_none());
        // Finished by choosing another tool: the note is kept and that tool stays.
        note(&mut app, 400.0, 400.0, "Krill");
        app.act(&ctx, Act::Tool(Tool::Rect));
        settle(&mut app);
        assert_eq!(comments(&app), ["Seal", "Puffin", "Krill"]);
        assert_eq!(app.tabs[0].ed.tool, Tool::Rect);
    }

    /// The colour picked in the inspector while a note is typed colours that note, which
    /// stays open (clicks in the inspector and in the picker, even over the pages, are not
    /// outside it) and gets the keyboard back. The Note tool has no pin.
    #[test]
    fn the_colour_picked_while_typing_a_note_colours_it() {
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        theme::install(&ctx);
        let mut app = app_with(&ctx, 1);
        let (min, k) = page_on_screen(&mut app, &ctx);
        let at = |x: f32, y: f32| min + egui::vec2(x, y) * k;
        let draw = |app: &mut App, events| window(app, &ctx, 1400.0, events);
        let typing = || ctx.memory(|m| m.has_focus(egui::Id::new(("note-text", 0usize))));
        let picker = || egui::Popup::is_any_open(&ctx);
        let open = |app: &App| app.tabs[0].ed.writing.is_some();
        let settle = |app: &mut App| {
            for _ in 0..2 {
                draw(app, vec![]);
            }
        };
        let rect = |b: egui::accesskit::Rect| {
            let (x0, y0, x1, y1) = (b.x0 as f32, b.y0 as f32, b.x1 as f32, b.y1 as f32);
            egui::Rect::from_min_max(egui::pos2(x0, y0), egui::pos2(x1, y1))
        };
        // The inspector's colour button.
        let well = |tree: &egui::accesskit::TreeUpdate| {
            let role = egui::accesskit::Role::ColorWell;
            let (_, n) = tree.nodes.iter().find(|(_, n)| n.role() == role).unwrap();
            rect(n.bounds().unwrap())
        };
        // The document keeps colours rounded.
        let same_color =
            |a: [f32; 3], b: [f32; 3]| a.iter().zip(&b).all(|(x, y)| (x - y).abs() < 0.01);
        let pin = "Keep tool active";
        app.act(&ctx, Act::Tool(Tool::TextBox));
        let tree = draw(&mut app, vec![]);
        assert!(node(&tree, pin).is_some(), "no pin on the text box");
        app.act(&ctx, Act::Tool(Tool::Note));
        let tree = draw(&mut app, vec![]);
        assert!(node(&tree, pin).is_none(), "a pin on the Note tool");
        draw(&mut app, click_at(at(100.0, 100.0)));
        draw(&mut app, vec![]);
        draw(&mut app, vec![egui::Event::Text("Walrus".into())]);
        let tree = draw(&mut app, vec![]);
        let was = app.tabs[0].ed.note_color;
        // The colour button opens the picker; the note stays open.
        draw(&mut app, click_at(well(&tree).center()));
        let tree = draw(&mut app, vec![]);
        assert!(picker(), "no colour picker");
        assert!(open(&app), "the colour button closed the note");
        // A click in the picker's colour square, over the page view, changes the colour.
        let square = tree
            .nodes
            .iter()
            .filter(|(_, n)| n.role() == egui::accesskit::Role::Unknown && n.label().is_none())
            .filter_map(|(_, n)| n.bounds().map(rect))
            .find(|r| (r.width() - r.height()).abs() < 1.0 && r.width() > 100.0)
            .expect("the colour square");
        let page = egui::Rect::from_min_max(at(0.0, 0.0), at(612.0, 792.0));
        let p = square.left_top() + square.size() * 0.2;
        assert!(page.contains(p), "the square isn't over the page");
        draw(&mut app, click_at(p));
        draw(&mut app, vec![]);
        let picked = app.tabs[0].ed.note_color;
        assert!(!same_color(picked, was), "the colour didn't change");
        assert!(open(&app), "the picker closed the note");
        // Esc closes the picker only; the note gets the keyboard back for more typing.
        draw(&mut app, vec![key_with(Key::Escape, egui::Modifiers::NONE)]);
        settle(&mut app);
        assert!(!picker());
        assert!(open(&app), "Esc in the picker closed the note");
        assert!(typing(), "the note didn't get the keyboard back");
        draw(&mut app, vec![egui::Event::Text(" pod".into())]);
        // A click on the page that only closes the picker keeps the note open too.
        let tree = draw(&mut app, vec![]);
        draw(&mut app, click_at(well(&tree).center()));
        draw(&mut app, vec![]);
        assert!(picker(), "no colour picker");
        draw(&mut app, click_at(at(300.0, 400.0)));
        settle(&mut app);
        assert!(!picker());
        assert!(comments(&app).is_empty(), "the note was added");
        assert!(open(&app), "closing the picker closed the note");
        // The note is added in the colour picked.
        draw(&mut app, click_at(at(300.0, 400.0)));
        settle(&mut app);
        let annots = app.tabs[0].pdf.annotations();
        assert_eq!(comments(&app), ["Walrus pod"]);
        let color = annots[0].color.expect("the note's colour");
        assert!(same_color(color, picked), "{color:?} isn't {picked:?}");
        assert!(app.tabs[0].ed.writing.is_none(), "the click started a note");
        assert_eq!(app.tabs[0].ed.tool, Tool::Read);
    }

    /// Esc keeps a text box being typed and an existing note's changed comment; a text box
    /// finished by choosing another tool leaves that tool on.
    #[test]
    fn esc_keeps_a_text_box_and_a_changed_note() {
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        theme::install(&ctx);
        let mut app = app_with(&ctx, 1);
        let (min, k) = page_on_screen(&mut app, &ctx);
        let at = |x: f32, y: f32| min + egui::vec2(x, y) * k;
        let none = egui::Modifiers::NONE;
        // Put a text box down at (x, y) (a click gives the default size) and type `text`
        // into it.
        let text_box = |app: &mut App, x: f32, y: f32, text: &str| {
            app.act(&ctx, Act::Tool(Tool::TextBox));
            window(app, &ctx, 1400.0, click_at(at(x, y)));
            window(app, &ctx, 1400.0, vec![]);
            let id = egui::Id::new(("box-text", 0usize));
            assert!(ctx.memory(|m| m.has_focus(id)), "the box has no keyboard");
            window(app, &ctx, 1400.0, vec![egui::Event::Text(text.into())]);
        };
        let settle = |app: &mut App| {
            for _ in 0..2 {
                window(app, &ctx, 1400.0, vec![]);
            }
        };
        text_box(&mut app, 100.0, 100.0, "Walrus");
        window(&mut app, &ctx, 1400.0, vec![key_with(Key::Escape, none)]);
        settle(&mut app);
        assert_eq!(comments(&app), ["Walrus"]);
        assert_eq!(app.tabs[0].ed.tool, Tool::Read);
        // Empty: nothing.
        text_box(&mut app, 100.0, 300.0, "");
        window(&mut app, &ctx, 1400.0, vec![key_with(Key::Escape, none)]);
        settle(&mut app);
        assert_eq!(comments(&app), ["Walrus"]);
        // Another tool chosen: the box is kept and the tool stays.
        text_box(&mut app, 100.0, 200.0, "Seal");
        app.act(&ctx, Act::Tool(Tool::Rect));
        settle(&mut app);
        assert_eq!(comments(&app), ["Walrus", "Seal"]);
        assert_eq!(app.tabs[0].ed.tool, Tool::Rect);
        // An existing note opened for typing: Esc keeps the change, in one step.
        let mut app = typing_in_a_note(&ctx, "Walrus typed");
        app.tabs[0].ed.escape();
        app.tabs[0].apply_actions(&ctx);
        assert_eq!(comments(&app), ["Walrus typed"]);
        app.act(&ctx, Act::Undo);
        assert_eq!(comments(&app), ["Old text"]);
    }

    /// The names of the controls Tab reaches in turn, from the window's first one, `n`
    /// presses.
    fn tab_walk(app: &mut App, ctx: &egui::Context, width: f32, n: usize) -> Vec<String> {
        focusable(app, ctx, width);
        let tab = egui::Event::Key {
            key: Key::Tab,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::NONE,
        };
        let mut reached = Vec::new();
        for _ in 0..n {
            let tree = window(app, ctx, width, vec![tab.clone()]);
            let focus = tree.nodes.iter().find(|(id, _)| *id == tree.focus);
            if let Some(name) = focus.and_then(|(_, n)| n.label()) {
                reached.push(name.to_string());
            }
        }
        reached
    }

    /// A dialog's fields are named, and Tab goes through them and then its buttons in the
    /// order they are shown.
    #[test]
    fn tab_goes_through_dialogs_in_reading_order() {
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        theme::install(&ctx);
        let mut app = app_with(&ctx, 1);
        let footer = |action: &'static str| {
            let pair = [action, "Cancel"];
            if dlg::ACTION_FIRST {
                pair
            } else {
                [pair[1], pair[0]]
            }
        };
        let printer = print::Printer {
            name: "limarepdf-test-no-such-printer".into(),
            default: true,
        };
        let print = dlg::Print::new(vec![printer], "test.pdf".into());
        app.dialog = Dialog::Print(app.asked(), Box::new(print));
        let mut want = vec!["Printer", "All pages", "Page range", "Copies", "Print on"];
        if !cfg!(windows) {
            want.push("Advanced");
        }
        want.extend(footer("Print"));
        assert_eq!(tab_walk(&mut app, &ctx, 1200.0, want.len()), want);
        // With Page range chosen, its pages after it, from the start again.
        if let Dialog::Print(_, s) = &mut app.dialog {
            s.all = false;
        }
        if let Some(id) = ctx.memory(|m| m.focused()) {
            ctx.memory_mut(|m| m.surrender_focus(id));
        }
        want.insert(3, "Pages to print");
        assert_eq!(tab_walk(&mut app, &ctx, 1200.0, want.len()), want);

        app.dialog = Dialog::None;
        app.act(&ctx, Act::CreateSign(limarepdf_core::SignRole::Signature));
        let mut want = vec![
            "Draw",
            "Type",
            "Image",
            "Drawing area: sign with the mouse or a pen",
            "Black ink",
            "Blue ink",
            "Save for next time",
        ];
        want.extend(footer("Use signature"));
        assert_eq!(tab_walk(&mut app, &ctx, 1200.0, want.len()), want);
    }

    /// Draws the dialog open in `app` on a screen `size` points big, at `scale` pixels per
    /// point, with `events`; returns its widgets.
    fn dialog_frame(
        app: &mut App,
        ctx: &egui::Context,
        size: egui::Vec2,
        scale: f32,
        events: Vec<egui::Event>,
    ) -> egui::accesskit::TreeUpdate {
        let mut raw = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, size)),
            events,
            ..Default::default()
        };
        raw.viewports
            .entry(egui::ViewportId::ROOT)
            .or_default()
            .native_pixels_per_point = Some(scale);
        let mut out = ctx.run_ui(raw, |ui| app.dialogs(ui.ctx()));
        out.textures_delta.clear();
        out.platform_output
            .accesskit_update
            .expect("the dialog's widgets")
    }

    /// The `i`th of the app's questions and messages, with long texts where they can come
    /// from the document or the system: its title, the buttons it shows, and the dialog.
    fn question(app: &App, long: &str, i: usize) -> Option<(String, Vec<&'static str>, Dialog)> {
        let two = |yes| vec![yes, "Cancel"];
        let printer = || print::Printer {
            name: format!("printer-{long}"),
            default: true,
        };
        let q = match i {
            0 => (
                "Couldn't save".into(),
                vec!["OK", "Copy"],
                Dialog::Message(
                    "Couldn't save".into(),
                    format!("The folder /home/sam/{long} can't be written to. {long}"),
                ),
            ),
            1 => (
                "Close without saving?".into(),
                two("Discard changes"),
                Dialog::ConfirmClose(app.asked()),
            ),
            2 => (
                "Quit without saving?".into(),
                two("Quit without saving"),
                Dialog::ConfirmQuit,
            ),
            3 => (
                "Replace files?".into(),
                two("Replace"),
                Dialog::ConfirmReplace(
                    app.asked(),
                    ImageExport {
                        pages: vec![0, 1],
                        dpi: 150.0,
                        format: ImageFormat::Png,
                        dir: PathBuf::from(format!("/home/sam/{long}")),
                        stem: long.into(),
                    },
                    2,
                ),
            ),
            4 => (
                "Flatten form?".into(),
                two("Flatten form"),
                Dialog::ConfirmFlatten(app.asked()),
            ),
            5 => (
                "Flatten annotations?".into(),
                two("Flatten annotations"),
                Dialog::ConfirmFlattenAnnots(app.asked()),
            ),
            6 => (
                "Break the signatures?".into(),
                vec!["Details", "Save anyway", "Cancel"],
                Dialog::ConfirmSaveSigned(app.asked(), false, long.into()),
            ),
            7 => (
                "Replace the damaged original?".into(),
                vec!["Save as…", "Replace original", "Cancel"],
                Dialog::ConfirmSaveRepaired(app.asked()),
            ),
            8 => (
                "Save changes the signers didn't allow?".into(),
                vec!["Details", "Save anyway", "Cancel"],
                Dialog::ConfirmSaveDisallowed(Box::new(SaveDisallowed {
                    as_new: false,
                    asked: app.asked(),
                    bytes: Vec::new(),
                    disallowed: limarepdf_core::DisallowedChanges {
                        signers: vec![(format!("\"{long}\""), vec![long.into(); 3])],
                    },
                })),
            ),
            9 => (
                "Apply 3 redactions?".into(),
                two("Apply redactions"),
                Dialog::ConfirmApplyRedactions(app.asked(), 3),
            ),
            10 => (
                "Finish signing?".into(),
                two("Finish signing"),
                Dialog::ConfirmFinishSigning(app.asked(), 2),
            ),
            11 => (
                "Remove hidden information".into(),
                two("Remove"),
                Dialog::RemoveHidden(
                    app.asked(),
                    dlg::RemoveHidden {
                        kinds: limarepdf_core::Hidden::ALL
                            .iter()
                            .map(|&h| (h, 2, true))
                            .collect(),
                    },
                ),
            ),
            12 => (
                "Save without applying the redactions?".into(),
                two("Save with marks"),
                Dialog::ConfirmSaveMarks(app.asked(), false, 2),
            ),
            13 => (
                "Print".into(),
                two("Print"),
                Dialog::Print(
                    app.asked(),
                    Box::new(dlg::Print::new(vec![printer()], "test.pdf".into())),
                ),
            ),
            14 => (
                "Print without these fields?".into(),
                two("Print anyway"),
                Dialog::ConfirmPrintFields(Box::new(PrintFields {
                    asked: app.asked(),
                    fields: (0..30).map(|i| format!("Field {i} {long}")).collect(),
                    pages: vec![0],
                    opts: print::PrintOptions {
                        printer: Some(printer().name),
                        copies: 1,
                        sides: print::Sides::One,
                        as_image: false,
                        title: "test".into(),
                    },
                })),
            ),
            15 => (
                "Save selected text".into(),
                two("Save…"),
                Dialog::SaveSelection(app.asked(), dlg::Layout::Original),
            ),
            16 => (
                "Open link?".into(),
                two("Open link"),
                Dialog::OpenLink(format!("https://example.com/{long}")),
            ),
            17 => (
                "Your name for comments".into(),
                two("Save"),
                Dialog::CommentName(long.into()),
            ),
            18 => (
                "About LimarePDF".into(),
                vec!["Close"],
                Dialog::About(about::Page::Main),
            ),
            19 => (
                "Recovery copy left over".into(),
                vec!["OK", "Show file"],
                Dialog::RecoveryFile {
                    title: "Recovery copy left over".into(),
                    text: format!("A copy in /home/sam/{long} couldn't be removed. {long}"),
                    file: PathBuf::from(format!("/home/sam/{long}")),
                },
            ),
            20 => (
                "Break the signatures?".into(),
                vec!["Details", "Sign anyway", "Cancel"],
                Dialog::ConfirmSignRecovered(app.asked()),
            ),
            21 => (
                "Password required".into(),
                two("Open"),
                Dialog::Password(dlg::Password {
                    path: PathBuf::from(format!("/home/sam/{long}.pdf")),
                    input: String::new(),
                    error: Some(format!("That isn't the password. {long}")),
                }),
            ),
            _ => return None,
        };
        Some(q)
    }

    /// G08: a menu open when a dialog opens by a shortcut (Page layout's, here) closes,
    /// instead of showing over the dimmed window.
    #[test]
    fn a_menu_open_behind_closes_when_a_dialog_opens() {
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        theme::install(&ctx);
        let mut app = app_with(&ctx, 1);
        let tree = settled_at(&mut app, &ctx, 400.0);
        let (at, _) = node(&tree, "Page layout").expect("the layout button");
        let press = |pressed| egui::Event::PointerButton {
            pos: at.center(),
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        };
        window(
            &mut app,
            &ctx,
            400.0,
            vec![egui::Event::PointerMoved(at.center()), press(true)],
        );
        window(&mut app, &ctx, 400.0, vec![press(false)]);
        settled_at(&mut app, &ctx, 400.0);
        assert!(egui::Popup::is_any_open(&ctx), "the menu didn't open");
        app.act(&ctx, Act::About);
        settled_at(&mut app, &ctx, 400.0);
        assert!(matches!(app.dialog, Dialog::About(_)));
        assert!(!egui::Popup::is_any_open(&ctx), "the menu stayed open");
    }

    /// G08: every question and message the app asks, wide, narrow and short, at each scale,
    /// with long names and texts: inside the screen with its title and buttons in view and
    /// no text past its sides; every control clearly named; Enter as it opens answering
    /// nothing (but in a dialog's only field); Tab reaching each button; and Esc closing it
    /// without acting.
    #[test]
    fn every_question_fits_and_is_answered_from_the_keyboard() {
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        theme::install(&ctx);
        let mut app = app_with(&ctx, 1);
        let long = "a-name-far-too-long-to-fit-on-one-line-of-a-dialog-".repeat(4);
        app.tabs[0].path = Some(PathBuf::from(format!("/home/sam/{long}.pdf")));
        let state = app.tabs[0].state();
        let key = |key| egui::Event::Key {
            key,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::NONE,
        };
        let rect = |b: egui::accesskit::Rect| {
            egui::Rect::from_min_max(
                egui::pos2(b.x0 as f32, b.y0 as f32),
                egui::pos2(b.x1 as f32, b.y1 as f32),
            )
        };
        let sizes = [
            (1200.0, 800.0),
            (400.0, 800.0),
            (1200.0, 300.0),
            (400.0, 300.0),
        ];
        for i in 0.. {
            let Some((title, _, _)) = question(&app, &long, i) else {
                assert_eq!(i, 22, "a question was left out");
                break;
            };
            for (size, scale) in sizes
                .iter()
                .flat_map(|&s| [1.0, 1.5, 2.0].map(move |k| (egui::Vec2::from(s), k)))
            {
                let at = format!("{title} at {size:?}, scale {scale}");
                let (_, buttons, dialog) = question(&app, &long, i).unwrap();
                if let Some(id) = ctx.memory(|m| m.focused()) {
                    ctx.memory_mut(|m| m.surrender_focus(id));
                }
                app.dialog = dialog;
                let one_field = matches!(app.dialog, Dialog::Password(_) | Dialog::CommentName(_));
                let mut tree = dialog_frame(&mut app, &ctx, size, scale, vec![]);
                for _ in 0..4 {
                    tree = dialog_frame(&mut app, &ctx, size, scale, vec![]);
                }
                let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, size).expand(0.5);
                let window = ctx.memory(|m| m.top_modal_layer().and_then(|l| m.area_rect(l.id)));
                let window = window.unwrap_or_else(|| panic!("{at}: no window"));
                assert!(screen.contains_rect(window), "{at}: window {window:?}");
                let inside = window.expand(0.5);
                // Text shown as is: a one-line field scrolls what it holds within itself.
                for (_, n) in &tree.nodes {
                    let (Some(text), Some(b)) = (n.value(), n.bounds()) else {
                        continue;
                    };
                    if n.role() != egui::accesskit::Role::Label {
                        continue;
                    }
                    let r = rect(b);
                    let across = r.left() >= inside.left() && r.right() <= inside.right();
                    assert!(across, "{at}: {text:?} at {r:?} past {window:?}");
                }
                let shown = |name: &str| {
                    let n = tree
                        .nodes
                        .iter()
                        .find(|(_, n)| n.label() == Some(name) || n.value() == Some(name));
                    n.and_then(|(_, n)| n.bounds()).map(rect)
                };
                let t = shown(&title).unwrap_or_else(|| panic!("{at}: no title"));
                assert!(inside.contains_rect(t), "{at}: title {t:?}");
                for b in &buttons {
                    let r = shown(b).unwrap_or_else(|| panic!("{at}: no {b}"));
                    assert!(inside.contains_rect(r), "{at}: {b} at {r:?}");
                }
                assert_eq!(dlg::unclear_names(&tree), [""; 0], "{at}");

                if !one_field {
                    dialog_frame(&mut app, &ctx, size, scale, vec![key(Key::Enter)]);
                    assert!(!matches!(app.dialog, Dialog::None), "{at}: Enter answered");
                }
                let mut reached = Vec::new();
                for _ in 0..buttons.len() + 12 {
                    let tree = dialog_frame(&mut app, &ctx, size, scale, vec![key(Key::Tab)]);
                    let focus = tree.nodes.iter().find(|(id, _)| *id == tree.focus);
                    if let Some(name) = focus.and_then(|(_, n)| n.label()) {
                        reached.push(name.to_string());
                    }
                }
                for b in &buttons {
                    assert!(reached.iter().any(|r| r == b), "{at}: Tab missed {b}");
                }
                // Tab may have opened Details; Esc closes the dialog all the same.
                if let Some(id) = ctx.memory(|m| m.focused()) {
                    ctx.memory_mut(|m| m.surrender_focus(id));
                }
                dialog_frame(&mut app, &ctx, size, scale, vec![key(Key::Escape)]);
                assert!(
                    matches!(app.dialog, Dialog::None),
                    "{at}: Esc didn't close it"
                );
                assert_eq!(app.tabs.len(), 1, "{at}");
                assert!(
                    app.tabs[0].state() == state && !app.quit_ok,
                    "{at}: Esc acted"
                );
            }
        }
    }

    #[test]
    fn esc_closes_the_inspector_pop_out_and_focuses_its_button() {
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        theme::install(&ctx);
        let mut app = app_with(&ctx, 1);
        // Narrow enough to fold the inspector into its strip.
        focusable(&mut app, &ctx, 700.0);
        app.popout = true;
        focusable(&mut app, &ctx, 700.0);
        assert!(app.popout);
        let esc = egui::Event::Key {
            key: Key::Escape,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::NONE,
        };
        window(&mut app, &ctx, 700.0, vec![esc]);
        assert!(!app.popout);
        let tree = window(&mut app, &ctx, 700.0, vec![]);
        let focus = tree.nodes.iter().find(|(id, _)| *id == tree.focus);
        assert_eq!(
            focus.and_then(|(_, n)| n.label()),
            Some("Options for the selection or the document")
        );
    }

    /// Docked selection controls take the floating bar's place and stay docked whatever
    /// the width, in the panel or, when it folds away, in its strip.
    #[test]
    fn docked_controls_stay_docked_through_resizing() {
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        theme::install(&ctx);
        let mut app = app_with(&ctx, 1);
        with_notes(&mut app, &ctx);
        let id = app.tabs[0].pdf.annotations()[0].id;
        app.tabs[0].ed.selection = Selection::Annot { page: 0, id };
        let (dock, float) = (
            Some("Dock controls on the right".to_string()),
            Some("Float controls near selection".to_string()),
        );
        let names = focusable(&mut app, &ctx, 1400.0);
        assert!(names.contains(&dock) && !names.contains(&float));
        app.act(&ctx, Act::ToggleDock);
        for width in [1400.0, 700.0, 1400.0] {
            let names = focusable(&mut app, &ctx, width);
            assert!(
                names.contains(&float) && !names.contains(&dock),
                "at {width}"
            );
        }
        assert!(app.settings.controls_docked);
    }

    fn message(app: &App) -> (&str, &str) {
        match &app.dialog {
            Dialog::Message(title, text) => (title, text),
            _ => panic!("no message shown"),
        }
    }

    #[test]
    fn a_command_failing_before_any_change_is_only_reported() {
        let ctx = egui::Context::default();
        let mut app = app_with(&ctx, 2);
        app.tabs[1].ed.set_tool(Tool::Note);
        let before: Vec<_> = app.tabs.iter().map(Tab::state).collect();
        app.run_command(&ctx, "Rotate", |_, _| panic!("bad page tree"));
        assert!(app.tabs.iter().zip(&before).all(|(t, s)| t.state() == *s));
        assert!(app.tabs.iter().all(|t| t.broken.is_none() && t.dirty));
        assert!(app.tabs[1].ed.tool == Tool::Read);
        let (title, text) = message(&app);
        assert_eq!(title, "Rotate failed");
        assert!(text.contains("bad page tree") && text.ends_with("Nothing was changed."));
    }

    #[test]
    fn a_command_failing_after_a_change_marks_only_its_tab_broken() {
        let ctx = egui::Context::default();
        let mut app = app_with(&ctx, 2);
        let other = app.tabs[0].state();
        app.run_command(&ctx, "Crop", |app, ctx| {
            let t = &mut app.tabs[app.active];
            assert!(t.edit(ctx, |p| p.insert_blank(0, 100.0, 100.0)));
            app.dialog = Dialog::ConfirmFlattenAnnots(app.asked());
            panic!("half done");
        });
        let why = app.tabs[1].broken.as_deref().expect("tab marked broken");
        assert!(why.starts_with("Crop failed part way") && why.contains("half done"));
        // The half-built dialog is gone, and the error page says what happened instead.
        assert!(matches!(app.dialog, Dialog::None));
        assert!(app.tabs[0].broken.is_none() && app.tabs[0].state() == other);
        // Save a copy: the document as it is now can still be written and read back.
        let path =
            std::env::temp_dir().join(format!("limarepdf-app-{}-copy.pdf", std::process::id()));
        app.tabs[1].pdf.save(&path).unwrap();
        let back = Pdf::from_bytes(&std::fs::read(&path).unwrap(), None).unwrap();
        let _ = std::fs::remove_file(&path);
        assert_eq!(back.page_count(), 3);
    }

    #[test]
    fn a_command_failing_after_removing_a_tab_leaves_a_valid_app() {
        let ctx = egui::Context::default();
        let mut app = app_with(&ctx, 3);
        let first = app.tabs[0].state();
        app.run_command(&ctx, "Close tab", |app, _| {
            app.tabs.remove(app.active);
            panic!("while closing");
        });
        assert_eq!((app.tabs.len(), app.active), (2, 1));
        assert!(app.tabs.iter().all(|t| t.broken.is_none() && t.dirty));
        assert!(app.tabs[0].state() == first);
        assert_eq!(message(&app).0, "Close tab failed");
        // The next command on the active tab works.
        app.run_command(&ctx, "Note", |app, ctx| app.act(ctx, Act::Tool(Tool::Note)));
        assert!(app.tabs[1].ed.tool == Tool::Note);
    }

    /// Draw the open dialog until its window has settled, then click its button labelled
    /// `label`, as a user would.
    fn click(app: &mut App, ctx: &egui::Context, label: &str) {
        ctx.enable_accesskit();
        // Dialogs use the app's fonts.
        theme::install(ctx);
        let frame = |app: &mut App, events: Vec<egui::Event>| {
            let raw = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1000.0, 800.0),
                )),
                events,
                ..Default::default()
            };
            let mut out = ctx.run_ui(raw, |ui| app.dialogs(ui.ctx()));
            out.textures_delta.clear();
            out
        };
        for _ in 0..3 {
            frame(app, vec![]);
        }
        let tree = frame(app, vec![])
            .platform_output
            .accesskit_update
            .expect("the dialog's widgets");
        let at = tree
            .nodes
            .iter()
            .find(|(_, n)| n.role() == egui::accesskit::Role::Button && n.label() == Some(label))
            .and_then(|(_, n)| n.bounds())
            .unwrap_or_else(|| panic!("no {label} button"));
        let pos = egui::pos2(
            ((at.x0 + at.x1) / 2.0) as f32,
            ((at.y0 + at.y1) / 2.0) as f32,
        );
        let press = |pressed| egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: Default::default(),
        };
        frame(app, vec![egui::Event::PointerMoved(pos), press(true)]);
        frame(app, vec![press(false)]);
    }

    /// Put a comment on page 1 of every tab.
    fn with_notes(app: &mut App, ctx: &egui::Context) {
        for t in &mut app.tabs {
            assert!(t.edit(ctx, |p| {
                p.add_note(0, (100.0, 100.0), [1.0, 1.0, 0.0], "Walrus", "Tester")
                    .map(drop)
            }));
        }
    }

    #[test]
    fn a_confirmation_never_acts_on_a_tab_opened_meanwhile() {
        let ctx = egui::Context::default();
        let mut app = app_with(&ctx, 1);
        with_notes(&mut app, &ctx);
        app.act(&ctx, Act::FlattenAnnots);
        assert!(matches!(app.dialog, Dialog::ConfirmFlattenAnnots(..)));
        // A file finishes opening while the question is open, and its tab comes to the
        // front (as clicking another tab does).
        let mut pdf = Pdf::new();
        pdf.insert_blank(0, 612.0, 792.0).unwrap();
        pdf.add_note(0, (100.0, 100.0), [1.0, 1.0, 0.0], "Opened", "Tester")
            .unwrap();
        assert!(app.add_tab(&ctx, pdf, None, None, false));
        assert_eq!(app.active, 1);
        let opened = app.tabs[1].state();
        click(&mut app, &ctx, "Flatten annotations");
        assert!(app.tabs[1].state() == opened, "the new tab was flattened");
        assert_eq!(app.tabs[1].pdf.annotations().len(), 1);
        // The question's own document, unchanged, is flattened and comes to the front.
        assert_eq!(app.active, 0);
        assert!(app.tabs[0].pdf.annotations().is_empty());
    }

    #[test]
    fn a_confirmation_for_a_closed_tab_does_nothing() {
        let ctx = egui::Context::default();
        let mut app = app_with(&ctx, 2);
        with_notes(&mut app, &ctx);
        app.act(&ctx, Act::FlattenAnnots);
        let other = app.tabs[0].state();
        app.close_tab(1);
        click(&mut app, &ctx, "Flatten annotations");
        assert!(app.tabs[0].state() == other, "another tab was flattened");
        assert!(
            matches!(&app.dialog, Dialog::Message(_, text) if text.contains("was closed")),
            "no message saying why"
        );
    }

    #[test]
    fn printing_asks_first_when_fields_would_print_empty() {
        use lopdf::{dictionary, Object, StringFormat};
        let ctx = egui::Context::default();
        let mut app = app_with(&ctx, 1);
        // A printed text field filled in another app with text LimarePDF can't draw, and
        // no drawing of its own.
        assert!(app.tabs[0].edit(&ctx, |p| {
            let mut v = vec![0xFE, 0xFF];
            v.extend("日本語".encode_utf16().flat_map(u16::to_be_bytes));
            let rect: Vec<Object> = [20, 20, 280, 50].map(Object::from).to_vec();
            let w = p.doc.add_object(dictionary! {
                "Type" => "Annot", "Subtype" => "Widget", "FT" => "Tx", "F" => 4,
                "T" => Object::string_literal("Name"), "Rect" => rect,
                "V" => Object::String(v, StringFormat::Hexadecimal),
            });
            let page = p.page_ids()[0];
            p.doc
                .get_dictionary_mut(page)?
                .set("Annots", vec![w.into()]);
            p.doc
                .catalog_mut()?
                .set("AcroForm", dictionary! { "Fields" => vec![w.into()] });
            Ok(())
        }));
        let state = app.tabs[0].state();
        let printer = print::Printer {
            name: "limarepdf-test-no-such-printer".into(),
            default: true,
        };
        let print = dlg::Print::new(vec![printer], "test.pdf".into());
        app.dialog = Dialog::Print(app.asked(), Box::new(print));

        click(&mut app, &ctx, "Print");
        // Nothing sent yet: the question names the field.
        assert!(!app.busy, "a print job started");
        match &app.dialog {
            Dialog::ConfirmPrintFields(q) => assert_eq!(q.fields, ["Name"]),
            _ => panic!("no question asked"),
        }
        click(&mut app, &ctx, "Cancel");
        assert!(matches!(app.dialog, Dialog::None));
        assert!(!app.busy, "a print job started");
        assert!(app.tabs[0].state() == state);
    }

    #[test]
    fn a_confirmation_is_refused_when_its_document_changed() {
        let ctx = egui::Context::default();
        let mut app = app_with(&ctx, 1);
        with_notes(&mut app, &ctx);
        app.act(&ctx, Act::FlattenAnnots);
        // An edit while the question is open (the dialog doesn't block the page, Undo or
        // the shortcuts).
        let t = &mut app.tabs[0];
        assert!(t.edit(&ctx, |p| p.insert_blank(0, 612.0, 792.0)));
        let state = t.state();
        click(&mut app, &ctx, "Flatten annotations");
        let t = &app.tabs[0];
        assert!(t.state() == state, "flattened after the document changed");
        assert_eq!(t.pdf.annotations().len(), 1);
        assert!(
            matches!(&app.dialog, Dialog::Message(_, text) if text.contains("changed while")),
            "no message saying why"
        );
    }

    /// Esc answers a question as Cancel, and the app's shortcuts wait while it is open.
    #[test]
    fn esc_cancels_a_question_and_shortcuts_wait_for_it() {
        let ctx = egui::Context::default();
        theme::install(&ctx);
        let mut app = app_with(&ctx, 1);
        let frame = |app: &mut App, events: Vec<egui::Event>| {
            let raw = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1000.0, 800.0),
                )),
                events,
                ..Default::default()
            };
            let mut out = ctx.run_ui(raw, |ui| {
                app.shortcuts(ui.ctx());
                app.dialogs(ui.ctx());
            });
            out.textures_delta.clear();
        };
        let key = |key, modifiers| egui::Event::Key {
            key,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers,
        };
        app.act(&ctx, Act::CloseTab);
        assert!(matches!(app.dialog, Dialog::ConfirmClose(..)));
        for _ in 0..3 {
            frame(&mut app, vec![]);
        }
        frame(&mut app, vec![key(Key::K, egui::Modifiers::COMMAND)]);
        assert!(
            app.palette.is_none(),
            "Ctrl+K opened the palette behind the question"
        );
        frame(&mut app, vec![key(Key::Escape, egui::Modifiers::NONE)]);
        assert!(matches!(app.dialog, Dialog::None));
        assert_eq!(app.tabs.len(), 1, "Esc closed the tab");
        for _ in 0..2 {
            frame(&mut app, vec![]);
        }
        frame(&mut app, vec![key(Key::K, egui::Modifiers::COMMAND)]);
        assert!(
            app.palette.is_some(),
            "shortcuts stayed off after the question"
        );
    }

    #[test]
    fn discarding_changes_closes_the_tab_that_was_asked_about() {
        let ctx = egui::Context::default();
        let mut app = app_with(&ctx, 3);
        let ids: Vec<u64> = app.tabs.iter().map(|t| t.id).collect();
        app.tabs[0].dirty = false;
        app.active = 1;
        app.act(&ctx, Act::CloseTab);
        assert!(matches!(app.dialog, Dialog::ConfirmClose(..)));
        // The first tab, with nothing unsaved, is closed while the question is open.
        app.close_tab(0);
        click(&mut app, &ctx, "Discard changes");
        let left: Vec<u64> = app.tabs.iter().map(|t| t.id).collect();
        assert_eq!(left, [ids[2]], "the wrong tab was closed");
    }

    /// One tab, saved, with a comment on page 1 open for typing and `typed` typed into it
    /// but not yet kept.
    fn typing_in_a_note(ctx: &egui::Context, typed: &str) -> App {
        let mut app = App::with(Settings::from_file(None), Recent::from_file(None));
        let mut pdf = Pdf::new();
        pdf.insert_blank(0, 612.0, 792.0).unwrap();
        let id = pdf
            .add_note(0, (100.0, 100.0), [1.0, 1.0, 0.0], "Old text", "Tester")
            .unwrap();
        assert!(app.add_tab(ctx, pdf, None, None, false));
        let t = &mut app.tabs[0];
        t.ed.annots(&t.pdf, 0);
        t.ed.selection = Selection::Annot { page: 0, id };
        t.ed.write_selected();
        t.ed.writing.as_mut().expect("open for typing").text = typed.into();
        assert!(!t.dirty);
        app
    }

    #[test]
    fn closing_while_typing_a_comment_asks_first() {
        let ctx = egui::Context::default();
        let mut app = typing_in_a_note(&ctx, "Walrus typed");
        app.act(&ctx, Act::CloseTab);
        assert_eq!(app.tabs.len(), 1, "closed, and the typed text lost");
        assert!(matches!(app.dialog, Dialog::ConfirmClose(..)));
        // Cancel: the typing stays open as it was, and the document unchanged.
        let state = app.tabs[0].state();
        app.dialog = Dialog::None;
        let t = &app.tabs[0];
        assert!(t.state() == state && !t.dirty);
        assert_eq!(
            t.ed.writing.as_ref().map(|w| w.text.as_str()),
            Some("Walrus typed")
        );
        app.act(&ctx, Act::Quit);
        assert!(!app.quit_ok, "quit, and the typed text lost");
        assert!(matches!(app.dialog, Dialog::ConfirmQuit));
        // Discard drops it.
        app.dialog = Dialog::None;
        app.act(&ctx, Act::CloseTab);
        click(&mut app, &ctx, "Discard changes");
        assert!(app.tabs.is_empty());
    }

    #[test]
    fn a_comment_opened_but_not_changed_is_not_unsaved_work() {
        let ctx = egui::Context::default();
        let mut app = typing_in_a_note(&ctx, "Old text");
        app.act(&ctx, Act::CloseTab);
        assert!(app.tabs.is_empty(), "asked about nothing");
    }

    #[test]
    fn saving_keeps_a_comment_being_typed() {
        let ctx = egui::Context::default();
        let mut app = typing_in_a_note(&ctx, "Walrus typed");
        let path =
            std::env::temp_dir().join(format!("limarepdf-app-{}-typed.pdf", std::process::id()));
        app.tabs[0].path = Some(path.clone());
        app.act(&ctx, Act::Save);
        let back = Pdf::from_bytes(&std::fs::read(&path).unwrap(), None).unwrap();
        let _ = std::fs::remove_file(&path);
        let texts: Vec<String> = back.annotations().into_iter().map(|a| a.contents).collect();
        assert_eq!(texts, ["Walrus typed"]);
        assert!(!app.tabs[0].dirty, "saved, yet marked unsaved");
    }

    /// The window drawn 1400 points wide with `events` (see [`window`]), and the text drawn
    /// on the pages: the previews, in Liberation fonts.
    fn drawn_texts(app: &mut App, ctx: &egui::Context, events: Vec<egui::Event>) -> Vec<String> {
        let raw = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(1400.0, 800.0),
            )),
            events,
            ..Default::default()
        };
        let mut frame = eframe::Frame::_new_kittest();
        let mut out = ctx.run_ui(raw, |ui| eframe::App::ui(app, ui, &mut frame));
        out.textures_delta.clear();
        // Only text drawn on the pages, in the fonts of the text written there.
        fn walk(s: &egui::Shape, out: &mut Vec<String>) {
            match s {
                egui::Shape::Text(t) => {
                    let section = t.galley.job.sections.first();
                    if let Some(egui::FontFamily::Name(n)) =
                        section.map(|s| &s.format.font_id.family)
                    {
                        if n.starts_with("Liberation") {
                            out.push(t.galley.text().to_string());
                        }
                    }
                }
                egui::Shape::Vec(v) => v.iter().for_each(|s| walk(s, out)),
                _ => {}
            }
        }
        let mut texts = Vec::new();
        for c in &out.shapes {
            walk(&c.shape, &mut texts);
        }
        texts
    }

    /// The middle of the control called `name`.
    fn middle(tree: &egui::accesskit::TreeUpdate, name: &str) -> egui::Pos2 {
        node(tree, name)
            .unwrap_or_else(|| panic!("no {name}"))
            .0
            .center()
    }

    /// The window settled after a change: a few frames with no input.
    fn settle(app: &mut App, ctx: &egui::Context) -> egui::accesskit::TreeUpdate {
        for _ in 0..3 {
            window(app, ctx, 1400.0, vec![]);
        }
        window(app, ctx, 1400.0, vec![])
    }

    /// Document properties apply as typed, with no Apply button: Enter or leaving a field
    /// keeps it, one undo step each, and saving keeps what is still being typed.
    #[test]
    fn properties_are_kept_by_enter_or_leaving_a_field() {
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        theme::install(&ctx);
        let mut app = app_with(&ctx, 1);
        let info = limarepdf_core::Info {
            title: Some("Walrus".into()),
            ..Default::default()
        };
        assert!(app.tabs[0].edit(&ctx, |p| p.set_info(&info)));
        app.tabs[0].forms.reveal = Some(inspector::Section::Properties);
        let tree = settle(&mut app, &ctx);
        // Password security is folded, so any Apply would be the properties'.
        let buttons = |tree: &egui::accesskit::TreeUpdate| -> Vec<String> {
            tree.nodes
                .iter()
                .filter(|(_, n)| n.role() == egui::accesskit::Role::Button)
                .filter_map(|(_, n)| n.label().map(str::to_string))
                .collect()
        };
        let shown = buttons(&tree);
        assert!(!shown.iter().any(|b| b == "Apply"), "{shown:?}");
        let field = |tree: &egui::accesskit::TreeUpdate, value: &str| {
            let (_, n) = tree
                .nodes
                .iter()
                .find(|(_, n)| {
                    n.role() == egui::accesskit::Role::TextInput && n.value() == Some(value)
                })
                .unwrap_or_else(|| panic!("no field holding {value:?}"));
            let b = n.bounds().unwrap();
            egui::pos2(((b.x0 + b.x1) / 2.0) as f32, ((b.y0 + b.y1) / 2.0) as f32)
        };
        let none = egui::Modifiers::NONE;
        let title = |app: &App| app.tabs[0].pdf.info().title;
        // Enter keeps it.
        window(&mut app, &ctx, 1400.0, click_at(field(&tree, "Walrus")));
        window(&mut app, &ctx, 1400.0, vec![key_with(Key::End, none)]);
        let typed = vec![egui::Event::Text(" notes".into())];
        window(&mut app, &ctx, 1400.0, typed);
        assert_eq!(title(&app).as_deref(), Some("Walrus"), "kept before Enter");
        window(&mut app, &ctx, 1400.0, vec![key_with(Key::Enter, none)]);
        let tree = settle(&mut app, &ctx);
        assert_eq!(title(&app).as_deref(), Some("Walrus notes"));
        // Leaving the field keeps it: a click on the page.
        let at = field(&tree, "Walrus notes");
        window(&mut app, &ctx, 1400.0, click_at(at));
        window(&mut app, &ctx, 1400.0, vec![key_with(Key::End, none)]);
        window(&mut app, &ctx, 1400.0, vec![egui::Event::Text("!".into())]);
        let tree = settle(&mut app, &ctx);
        window(&mut app, &ctx, 1400.0, click_at(middle(&tree, "Page 1")));
        settle(&mut app, &ctx);
        assert_eq!(title(&app).as_deref(), Some("Walrus notes!"));
        // Typing not yet kept is kept by saving.
        app.tabs[0].ed.selection = Selection::default();
        let tree = settle(&mut app, &ctx);
        let at = field(&tree, "Walrus notes!");
        window(&mut app, &ctx, 1400.0, click_at(at));
        window(&mut app, &ctx, 1400.0, vec![key_with(Key::End, none)]);
        window(&mut app, &ctx, 1400.0, vec![egui::Event::Text("?".into())]);
        settle(&mut app, &ctx);
        assert!(app.tabs[0].typing_in_forms());
        let path =
            std::env::temp_dir().join(format!("limarepdf-app-{}-props.pdf", std::process::id()));
        app.tabs[0].path = Some(path.clone());
        app.act(&ctx, Act::Save);
        let back = Pdf::from_bytes(&std::fs::read(&path).unwrap(), None).unwrap();
        let _ = std::fs::remove_file(&path);
        assert_eq!(back.info().title.as_deref(), Some("Walrus notes!?"));
        // One undo step each.
        let t = &mut app.tabs[0];
        for was in ["Walrus notes!", "Walrus notes", "Walrus"] {
            t.undo(&ctx);
            assert_eq!(t.pdf.info().title.as_deref(), Some(was));
        }
    }

    /// A watermark is drawn over the pages while its section is open, and nothing is written
    /// until Add to pages; then changes update it in place. Headers and footers likewise.
    #[test]
    fn stamps_are_shown_on_the_pages_until_added() {
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        theme::install(&ctx);
        let mut app = app_with(&ctx, 1);
        app.tabs[0].forms.reveal = Some(inspector::Section::Watermark);
        let tree = settle(&mut app, &ctx);
        // On the page in view, written nowhere.
        let texts = drawn_texts(&mut app, &ctx, vec![]);
        assert_eq!(texts, ["DRAFT"]);
        let lines = |app: &App| -> Vec<String> {
            let objs = app.tabs[0].pdf.page_objects(0).unwrap();
            objs.into_iter().filter_map(|o| o.text).collect()
        };
        assert!(lines(&app).is_empty());
        let add = middle(&tree, "Add to pages");
        window(&mut app, &ctx, 1400.0, click_at(add));
        let tree = settle(&mut app, &ctx);
        assert_eq!(lines(&app), ["DRAFT"]);
        assert!(node(&tree, "Add another").is_some());
        let texts = drawn_texts(&mut app, &ctx, vec![]);
        assert!(texts.is_empty(), "still previewed: {texts:?}");
        // A change updates it.
        app.tabs[0].forms.wm.text = "COPY".into();
        settle(&mut app, &ctx);
        assert_eq!(lines(&app), ["COPY"]);
        app.tabs[0].undo(&ctx);
        assert_eq!(lines(&app), ["DRAFT"]);
        // Headers and footers, with their tokens filled in for each page.
        app.tabs[0].forms.reveal = Some(inspector::Section::HeaderFooter);
        settle(&mut app, &ctx);
        assert_eq!(drawn_texts(&mut app, &ctx, vec![]), ["Page 1 of 2"]);
        assert_eq!(lines(&app), ["DRAFT"]);
    }

    /// While the Crop section is open the pages are drawn whole with the crop box on them;
    /// dragging its top edge crops the pages, one undo step, and can pull it back out. Esc
    /// closes the section, showing the pages cropped.
    #[test]
    fn the_crop_box_is_dragged_on_the_page() {
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        theme::install(&ctx);
        let mut app = app_with(&ctx, 1);
        app.tabs[0].forms.reveal = Some(inspector::Section::Crop);
        let drag = |app: &mut App, from: egui::Pos2, to: egui::Pos2| {
            window(app, &ctx, 1400.0, vec![egui::Event::PointerMoved(from)]);
            window(app, &ctx, 1400.0, vec![press(from, true)]);
            for i in 1..=4 {
                let at = from + (to - from) * (i as f32 / 4.0);
                window(app, &ctx, 1400.0, vec![egui::Event::PointerMoved(at)]);
            }
            window(app, &ctx, 1400.0, vec![press(to, false)]);
        };
        // The page, inside the room around it to take its edges.
        fn page_in(tree: &egui::accesskit::TreeUpdate) -> egui::Rect {
            let (r, _) = node(tree, "Crop box on page 1").expect("the crop box");
            r.shrink(8.0)
        }
        let tree = settle(&mut app, &ctx);
        assert!(app.tabs[0].crop_view());
        let page = page_in(&tree);
        let k = page.width() / 612.0;
        let top = egui::pos2(page.center().x, page.min.y);
        drag(&mut app, top, top + egui::vec2(0.0, 72.0 * k));
        let margins = |app: &App, p| app.tabs[0].pdf.crop_margins(p).unwrap();
        assert_eq!(margins(&app, 0), [72.0, 0.0, 0.0, 0.0]);
        assert_eq!(margins(&app, 1), [72.0, 0.0, 0.0, 0.0]);
        assert_eq!(app.tabs[0].sizes[0], (612.0, 792.0), "not shown whole");
        // Esc: shown as cropped.
        let esc = key_with(Key::Escape, egui::Modifiers::NONE);
        window(&mut app, &ctx, 1400.0, vec![esc]);
        settle(&mut app, &ctx);
        assert!(!app.tabs[0].crop_view());
        assert_eq!(app.tabs[0].sizes[0], (612.0, 720.0));
        // Open again, and pulled back out past the page's top.
        app.tabs[0].forms.reveal = Some(inspector::Section::Crop);
        let page = page_in(&settle(&mut app, &ctx));
        let edge = egui::pos2(page.center().x, page.min.y + 72.0 * k);
        drag(&mut app, edge, egui::pos2(edge.x, page.min.y - 30.0));
        assert_eq!(margins(&app, 0), [0.0; 4]);
        app.tabs[0].undo(&ctx);
        assert_eq!(margins(&app, 1), [72.0, 0.0, 0.0, 0.0]);
        app.tabs[0].undo(&ctx);
        assert_eq!(margins(&app, 1), [0.0; 4]);
    }

    /// What the signing window hands over once a document was signed and written.
    fn signed_copy() -> digital::Signed {
        let mut pdf = Pdf::from_text("Signed", (612.0, 792.0)).unwrap();
        digital::Signed {
            path: std::env::temp_dir().join("limarepdf-app-signed-copy.pdf"),
            bytes: pdf.to_bytes().unwrap(),
            field: "Signature1".into(),
            finished_items: 0,
            rewritten: false,
            signer: "Tester".into(),
            self_issued: false,
            certified: false,
            confirmed: true,
        }
    }

    /// A password set to apply when the document is saved (Document > Password security).
    fn password() -> Option<Encryption> {
        Some(Encryption {
            user_password: "u".into(),
            owner_password: "o".into(),
            cipher: Cipher::Aes128,
            restrictions: Restrictions::default(),
        })
    }

    #[test]
    fn signing_is_refused_while_a_password_waits_to_be_saved() {
        let ctx = egui::Context::default();
        let mut app = app_with(&ctx, 1);
        app.tabs[0].set_encryption(password());
        app.act(&ctx, Act::SignDigital);
        assert_eq!(message(&app), ("Sign with digital ID", PASSWORD_NOT_SIGNED));
        assert!(app.tabs[0].ed.tool != Tool::DigitalSign);
        // Set after the place was chosen: refused when Sign is pressed, before asking where
        // to save the signed file.
        app.dialog = Dialog::None;
        let t = &app.tabs[0];
        let place = SignPlace::NewBox {
            page: 0,
            rect: [72.0, 72.0, 272.0, 132.0],
        };
        let mut w = SignWindow::new(t.id, t.revision, place, "a box".into(), false, false);
        app.start_signing(&ctx, &mut w);
        assert_eq!(w.error.as_deref(), Some(PASSWORD_NOT_SIGNED));
        assert!(!w.busy());
        assert!(app.tabs[0].encryption.is_some());
    }

    #[test]
    fn the_signed_file_takes_the_place_of_an_unchanged_tab() {
        let ctx = egui::Context::default();
        let mut app = app_with(&ctx, 2);
        let (id, revision) = (app.tabs[0].id, app.tabs[0].revision);
        app.signed(&ctx, signed_copy(), id, revision);
        assert_eq!(app.tabs.len(), 2);
        assert!(
            app.tabs.iter().all(|t| t.id != id),
            "the old tab is still open"
        );
        assert_eq!(app.active, 0);
        assert!(!message(&app).1.contains("changed while"));
        assert!(!app.tabs[0].dirty);
        assert!(!message(&app).1.contains(NOT_CONFIRMED));
    }

    /// A signed file written but not confirmed on disk (decision D7) still takes the tab's
    /// place, as the new file; the message says so and the document stays marked unsaved.
    #[test]
    fn a_signed_file_not_confirmed_on_disk() {
        let ctx = egui::Context::default();
        let mut app = app_with(&ctx, 2);
        let (id, revision) = (app.tabs[0].id, app.tabs[0].revision);
        let s = digital::Signed {
            confirmed: false,
            ..signed_copy()
        };
        let path = s.path.clone();
        app.signed(&ctx, s, id, revision);
        assert!(app.tabs.iter().all(|t| t.id != id));
        assert_eq!(app.tabs[0].path.as_ref(), Some(&path));
        assert!(app.tabs[0].dirty, "marked saved");
        assert!(message(&app).1.contains(NOT_CONFIRMED));
    }

    #[test]
    fn a_password_set_while_signing_is_kept() {
        let ctx = egui::Context::default();
        let mut app = app_with(&ctx, 1);
        let (id, revision) = (app.tabs[0].id, app.tabs[0].revision);
        // While signing runs, a password is set.
        app.tabs[0].set_encryption(password());
        app.signed(&ctx, signed_copy(), id, revision);
        let old = app.tabs.iter().find(|t| t.id == id);
        assert!(
            old.is_some_and(|t| t.encryption.is_some()),
            "the signed file replaced the tab, and the password was lost"
        );
        assert_eq!(app.tabs.len(), 2, "the signed file didn't open");
        assert!(message(&app)
            .1
            .contains("changed while it was being signed"));
    }

    #[test]
    fn typing_while_signing_is_kept() {
        let ctx = egui::Context::default();
        let mut app = typing_in_a_note(&ctx, "Walrus typed");
        let (id, revision) = (app.tabs[0].id, app.tabs[0].revision);
        app.signed(&ctx, signed_copy(), id, revision);
        let old = app.tabs.iter().find(|t| t.id == id);
        assert_eq!(
            old.and_then(|t| t.ed.writing.as_ref())
                .map(|w| w.text.as_str()),
            Some("Walrus typed"),
            "the signed file replaced the tab, and the typed text was lost"
        );
        assert_eq!(app.tabs.len(), 2, "the signed file didn't open");
    }

    #[test]
    fn scan_summary_counts_shared_pictures_and_says_when_originals_stay() {
        let done = |kept| ScanResult::Optimized {
            mode: limarepdf_core::ScanMode::Gray,
            before: 3_000_000,
            after: 1_000_000,
            deskewed: None,
            original_kept: kept,
        };
        let (changed, text) = scan_summary(&[(0, done(true)), (2, ScanResult::SameImage(0))]);
        assert!(changed);
        assert!(text.starts_with("2 page(s) optimised: images 3.0 MB -> 1.0 MB."));
        assert!(text.contains("Page 3: the same picture as page 1, optimised with it."));
        assert!(text.contains("Page 1's original picture is also used elsewhere"));
        let (_, text) = scan_summary(&[(0, done(false))]);
        assert!(!text.contains("elsewhere"), "{text}");
    }

    #[test]
    fn closing_the_last_tab_keeps_the_active_index_valid() {
        let ctx = egui::Context::default();
        let mut app = app_with(&ctx, 2);
        app.close_tab(1);
        assert_eq!((app.tabs.len(), app.active), (1, 0));
        app.close_tab(0);
        assert_eq!((app.tabs.len(), app.active), (0, 0));
    }
}

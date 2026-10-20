use crate::about;
use crate::actions::{self, menu_item, tooltip, Act, Action, Ctx, Group, ACTIONS};
use crate::banners::Kind as BannerKind;
use crate::dialogs::{self as dlg, Choice, Outcome};
use crate::digital::{self, IdsWindow, SignOut, SignWindow};
use crate::editor::{Action as EdAction, Tool};
use crate::inspector::{self, Apply, Section};
use crate::markup;
use crate::recent::Recent;
use crate::selection::Selection;
use crate::settings::{Layout, Settings};
use crate::signatures::{self as sigs, Check, SigState, TrustAsk, TrustedOut, TrustedWindow};
use crate::tab::{Side, Stamp, Tab};
use crate::theme;
use crate::worker::Colours;
use eframe::egui::{self, AtomExt as _, Key};
use egui_phosphor::regular as icon;
use limarepdf_core::render::{image_names, ImageFormat};
use limarepdf_core::{
    parse_page_range, part_file_name, print, Bates, HeaderFooter, ImageImport, ImageWatermark,
    OcrResult, OpenPanel, Pdf, ScanResult, TrustRole, Watermark,
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
    /// Save (`true`: as a new file) by writing the whole file, which breaks the
    /// document's signatures? With why the edits can't be added after what was signed.
    ConfirmSaveSigned(bool, String),
    /// Save over a file that was repaired to open it: replace it, or save as a new file.
    ConfirmSaveRepaired,
    /// Save edits the document's signatures don't allow?
    ConfirmSaveDisallowed(Box<SaveDisallowed>),
    /// Remove everything under this many marked areas?
    ConfirmApplyRedactions(usize),
    /// Draw this many fill-and-sign items into the pages?
    ConfirmFinishSigning(usize),
    RemoveHidden(dlg::RemoveHidden),
    /// Save (`true`: as a new file) with this many marked areas not yet applied?
    ConfirmSaveMarks(bool, usize),
    Print(dlg::Print),
    SaveSelection(dlg::Layout),
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
    ConfirmSignRecovered,
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

/// A signed document with its edits added after what was signed, ready to write once the
/// user agrees that the signatures will report changes they don't allow.
struct SaveDisallowed {
    as_new: bool,
    /// The tab and its revision the file was made from.
    tab: u64,
    revision: u64,
    bytes: Vec<u8>,
    disallowed: limarepdf_core::DisallowedChanges,
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
                    self.show_side = true;
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
            self.show_inspector = true;
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

    /// The Digital IDs window and the signing window, while open.
    fn digital_windows(&mut self, ctx: &egui::Context) {
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
    /// from, unless that tab changed or closed meanwhile.
    fn signed(&mut self, ctx: &egui::Context, s: digital::Signed, tab: u64, revision: u64) {
        self.recent.add(&s.path);
        let replace = self
            .tabs
            .iter()
            .position(|t| t.id == tab && t.revision == revision);
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

    /// Saves. Saving over a file that was repaired to open it asks first, offering Save as
    /// instead: the original is only replaced when the user says so.
    fn save(&mut self, ctx: &egui::Context, as_new: bool) {
        let over_repaired = self
            .tabs
            .get(self.active)
            .is_some_and(|t| t.repaired && t.path.is_some());
        if !as_new && over_repaired {
            self.dialog = Dialog::ConfirmSaveRepaired;
        } else {
            self.save_unmarked(ctx, as_new);
        }
    }

    /// Saves, first asking when the document has redaction marks not yet applied; a
    /// signed document then goes through [`Self::save_signed`].
    fn save_unmarked(&mut self, ctx: &egui::Context, as_new: bool) {
        let marks = self
            .tabs
            .get(self.active)
            .map_or(0, |t| t.pdf.redaction_marks().len());
        if marks > 0 {
            self.dialog = Dialog::ConfirmSaveMarks(as_new, marks);
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
                        tab: tab.id,
                        revision: tab.revision,
                        bytes: a.bytes,
                        disallowed,
                    }))
                }
            },
            Err(why) => self.dialog = Dialog::ConfirmSaveSigned(as_new, why),
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
        match limarepdf_core::write_file(&path, &bytes) {
            Ok(()) => {
                self.status = format!(
                    "Saved {}; the changes were added after what was signed, so the signatures \
                     stay",
                    path.display()
                );
                self.recent.add(&path);
                tab.dirty = false;
                tab.scrubbed = false;
                tab.path = Some(path);
                self.check_signatures(ctx, self.active, Arc::from(bytes), None);
                self.saved_copy(self.active);
            }
            Err(e) => self.dialog = Dialog::Message("Save failed".into(), format!("{e:#}")),
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
        match crate::guard(|| tab.pdf.save(&path)) {
            Ok(()) => {
                self.status = format!("Saved {}", path.display());
                self.recent.add(&path);
                tab.dirty = false;
                tab.scrubbed = false;
                // The saved file is the repaired copy, with nothing left to repair.
                tab.repaired = false;
                tab.banners.retain(|b| b.kind != BannerKind::Repaired);
                // The saved copy's signatures are already broken: no need to ask again.
                tab.signed = false;
                // The Signatures tab describes the file, so check the saved one (a saved
                // recovered copy's too, which was never checked).
                let password = tab.encryption.as_ref().map(|e| e.user_password.clone());
                let recheck = tab.sigs.is_some()
                    || (tab.recovered.is_some() && tab.pdf.signature_count() > 0);
                let saved = recheck.then(|| std::fs::read(&path));
                tab.path = Some(path);
                match saved {
                    Some(Ok(bytes)) => {
                        self.check_signatures(ctx, self.active, Arc::from(bytes), password)
                    }
                    Some(Err(e)) => {
                        let t = &mut self.tabs[self.active];
                        if let Some(s) = t.sigs.as_mut() {
                            s.check = Check::Failed(format!(
                                "LimarePDF can't read the saved file again: {e}"
                            ));
                            t.sig_banner();
                        }
                    }
                    None => {}
                }
                self.saved_copy(self.active);
            }
            Err(e) => self.dialog = Dialog::Message("Save failed".into(), format!("{e:#}")),
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
                    self.dialog = Dialog::RemoveHidden(dlg::RemoveHidden { kinds });
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
                    self.dialog = Dialog::ConfirmApplyRedactions(n);
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
                    self.dialog = Dialog::ConfirmFinishSigning(n);
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
                } else if certify && t.pdf.signature_count() > 0 {
                    self.dialog = Dialog::Message(
                        title.into(),
                        "Only a document no one has signed yet can be certified. Use Sign \
                         with digital ID instead."
                            .into(),
                    );
                } else if t.recovered.is_some() && t.signed {
                    self.dialog = Dialog::ConfirmSignRecovered;
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
                if self.tabs[self.active].dirty {
                    self.dialog = Dialog::ConfirmClose(self.active);
                } else {
                    self.close_tab(self.active);
                }
            }
            Act::ToggleRecovery => self.toggle_recovery(),
            Act::ShowRecoveryFile => self.show_leftover(),
            Act::Quit => {
                if self.tabs.iter().any(|t| t.dirty) {
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
            Act::TrustedCertificates => self.trusted = Some(TrustedWindow::load()),
            _ if !has => {}
            Act::Signatures => {
                self.show_side = true;
                self.tabs[self.active].side = Side::Signatures;
            }
            Act::SignatureDetails(i) => {
                self.sig_details = Some((self.tabs[self.active].id, i));
            }
            Act::SignedVersion(i) => self.open_signed_version(ctx, i),
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
            Apply::Label(page) => {
                let t = &mut self.tabs[active];
                let l = &t.forms.label;
                let range = limarepdf_core::LabelRange {
                    page,
                    style: l.style,
                    prefix: l.prefix.clone(),
                    first: l.start.max(1),
                };
                t.edit(ctx, |p| p.set_label_range(range));
            }
            Apply::RemoveLabels => {
                self.tabs[active].edit(ctx, |p| p.set_label_ranges(&[]));
            }
            Apply::Props => {
                let t = &mut self.tabs[active];
                let info = t.forms.meta.clone();
                t.edit(ctx, |p| p.set_info(&info));
            }
            Apply::ViewPrefs => {
                let t = &mut self.tabs[active];
                let prefs = t.forms.view;
                t.edit(ctx, |p| p.set_view_prefs(&prefs));
            }
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
            Apply::Watermark { replace } => {
                let t = &mut self.tabs[active];
                if !replace {
                    t.forget_stamp();
                }
                let s = &t.forms.wm;
                let image = match (&s.picture, &s.image) {
                    (false, _) => Ok(None),
                    (true, None) => Err(anyhow::anyhow!("choose a picture first")),
                    (true, Some(path)) => std::fs::read(path)
                        .map(Some)
                        .map_err(|e| anyhow::anyhow!("reading {}: {e}", path.display())),
                };
                match image.and_then(|im| Ok((im, parse_page_range(&s.range, n)?))) {
                    Ok((Some(image), idx)) => {
                        let wm = ImageWatermark {
                            image,
                            scale: s.scale / 100.0,
                            opacity: s.opacity,
                            rotation: s.rotation,
                            foreground: s.foreground,
                        };
                        t.edit_stamp(ctx, Stamp::Watermark, |p| p.add_image_watermark(&idx, &wm));
                    }
                    Ok((None, idx)) => {
                        let wm = Watermark {
                            text: s.text.clone(),
                            font_size: s.size,
                            opacity: s.opacity,
                            rotation: s.rotation,
                            color: (s.color[0], s.color[1], s.color[2]),
                            foreground: s.foreground,
                        };
                        t.edit_stamp(ctx, Stamp::Watermark, |p| p.add_watermark(&idx, &wm));
                    }
                    Err(e) => t.error = Some(e.to_string()),
                }
            }
            Apply::HeaderFooter { replace } => {
                let t = &mut self.tabs[active];
                if !replace {
                    t.forget_stamp();
                }
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
                        t.edit_stamp(ctx, Stamp::HeaderFooter, |p| p.add_header_footer(&idx, &hf));
                    }
                    Err(e) => t.error = Some(e.to_string()),
                }
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
                    self.dialog = Dialog::ConfirmReplace(e, existing);
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
                    let parts = match level {
                        Some(level) => p.split_at_bookmarks(level)?,
                        None => p
                            .split_every(every)?
                            .into_iter()
                            .map(|d| (None, d))
                            .collect(),
                    };
                    let n = parts.len();
                    for (i, (title, mut part)) in parts.into_iter().enumerate() {
                        part.save(dir.join(part_file_name(&stem, i + 1, title.as_deref())))?;
                    }
                    Ok((false, format!("Wrote {n} files to {}.", dir.display())))
                });
            }
        }
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
                "Open link",
                &format!(
                    "This link goes to:\n\n{}\n\nOpen it in your browser or email program?",
                    limarepdf_core::display_line(url, 300)
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
            Dialog::RecoveryFile { title, text, file } => {
                match dlg::choose(ctx, title, text, "OK", "Show file") {
                    Choice::Second => {
                        let file = file.clone();
                        self.show_file(&file);
                    }
                    Choice::Open => {}
                    Choice::First | Choice::Cancel => done = true,
                }
            }
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
            Dialog::ConfirmApplyRedactions(n) => match dlg::confirm(
                ctx,
                "Apply redactions?",
                &format!(
                    "Everything under {} is removed from the document for good: text (hidden \
                     text too), the covered parts of pictures, drawings, and comments, links and \
                     form fields. The areas are painted black, and the marks go. LimarePDF then checks \
                     that nothing readable is left under them. Undo brings it all back until you \
                     close the document; once saved, it is gone from the file.",
                    plural(*n, "the marked area", "the {n} marked areas")
                ),
                "Apply",
            ) {
                Outcome::Apply => {
                    self.apply_redactions(ctx);
                    done = true;
                }
                Outcome::Cancel => done = true,
                Outcome::Open => {}
            },
            Dialog::ConfirmFinishSigning(n) => match dlg::confirm(
                ctx,
                "Finish signing?",
                &format!(
                    "{} drawn into the pages, as {} now. After that {} can't be moved or \
                     removed like a comment. Undo brings {} back until you close the document.",
                    plural(
                        *n,
                        "The item you put down with the Sign tool is",
                        "The {n} items you put down with the Sign tool are"
                    ),
                    if *n == 1 { "it looks" } else { "they look" },
                    if *n == 1 { "it" } else { "they" },
                    if *n == 1 { "it" } else { "them" },
                ),
                "Finish signing",
            ) {
                Outcome::Apply => {
                    let t = &mut self.tabs[self.active];
                    t.edit(ctx, |p| p.finish_signing().map(drop));
                    done = true;
                }
                Outcome::Cancel => done = true,
                Outcome::Open => {}
            },
            Dialog::RemoveHidden(s) => match dlg::remove_hidden(ctx, s) {
                Outcome::Apply => {
                    let what: Vec<limarepdf_core::Hidden> = s
                        .kinds
                        .iter()
                        .filter(|(_, n, on)| *on && *n > 0)
                        .map(|(h, _, _)| *h)
                        .collect();
                    self.remove_hidden(ctx, &what);
                    done = true;
                }
                Outcome::Cancel => done = true,
                Outcome::Open => {}
            },
            Dialog::ConfirmSaveMarks(as_new, n) => {
                let as_new = *as_new;
                match dlg::confirm(
                    ctx,
                    "Save without applying the redactions?",
                    &format!(
                        "This document has {} marked for redaction but not applied: nothing \
                         under the marks has been removed, and anyone can still read it. Apply \
                         them first with Document > Apply redactions, or save the marks to \
                         finish or review later.",
                        plural(*n, "an area", "{n} areas")
                    ),
                    "Save with marks",
                ) {
                    Outcome::Apply => {
                        self.save_signed(ctx, as_new);
                        done = true;
                    }
                    Outcome::Cancel => done = true,
                    Outcome::Open => {}
                }
            }
            Dialog::ConfirmSaveRepaired => match dlg::choose(
                ctx,
                "Replace the damaged original?",
                "LimarePDF repaired this file to open it, and some of its content may be missing \
                 from the repair. Saving over it replaces the original for good. Save as a new \
                 file to keep the original as it is.",
                "Save as…",
                "Replace original",
            ) {
                Choice::First => {
                    self.save_unmarked(ctx, true);
                    done = true;
                }
                Choice::Second => {
                    self.save_unmarked(ctx, false);
                    done = true;
                }
                Choice::Cancel => done = true,
                Choice::Open => {}
            },
            Dialog::ConfirmSaveSigned(as_new, why) => {
                let as_new = *as_new;
                match dlg::confirm(
                    ctx,
                    "Break the signatures?",
                    &format!(
                        "This document is digitally signed, but LimarePDF can't add your changes after \
                         what was signed: {}. Saving instead writes the whole file again, so the \
                         saved copy's signatures will show as invalid in every reader. Save as a \
                         new file to keep the signed original.",
                        limarepdf_core::display_line(why, 300)
                    ),
                    "Save anyway",
                ) {
                    Outcome::Apply => {
                        self.save_now(ctx, as_new);
                        done = true;
                    }
                    Outcome::Cancel => done = true,
                    Outcome::Open => {}
                }
            }
            Dialog::ConfirmSaveDisallowed(s) => {
                match dlg::confirm(
                    ctx,
                    "Save changes the signers didn't allow?",
                    &format!(
                        "Your changes are ones this document's signatures don't allow. If you \
                         save, anyone who checks them will be told:\n{}\n\nThe signatures stay \
                         valid for what was signed. Save as a new file to keep the signed \
                         original as it is.",
                        digital::disallowed_list(&s.disallowed)
                    ),
                    "Save anyway",
                ) {
                    Outcome::Apply => {
                        let (as_new, tab, revision) = (s.as_new, s.tab, s.revision);
                        let current = self
                            .tabs
                            .get(self.active)
                            .is_some_and(|t| t.id == tab && t.revision == revision);
                        if current {
                            let bytes = std::mem::take(&mut s.bytes);
                            self.save_appended(ctx, as_new, bytes);
                        } else {
                            // The document changed meanwhile: work it out again.
                            self.save_signed(ctx, as_new);
                        }
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
            Dialog::ConfirmSignRecovered => match dlg::confirm(
                ctx,
                "Break the signatures?",
                &format!(
                    "This document is digitally signed, but LimarePDF can't add your signature \
                     after what was signed: {RECOVERED}. Signing instead writes the whole file \
                     again, so the earlier signatures will show as invalid in every reader. Open \
                     the original to sign it with its signatures kept."
                ),
                "Sign anyway",
            ) {
                Outcome::Apply => {
                    self.sign_tool(false);
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
            side: self.show_side,
            inspector: self.show_inspector,
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
            self.run_command(ctx, actions::name(a), |app, ctx| app.act(ctx, a));
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
            && self.tabs.iter().any(|t| t.dirty)
        {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            self.dialog = Dialog::ConfirmQuit;
        }

        // The window's size, saved when the app closes; not while minimized or full screen.
        let (size, maximized, hidden, monitor) = ctx.input(|i| {
            let v = i.viewport();
            let size = v.inner_rect.map(|r| r.size());
            let hidden = v.minimized == Some(true) || v.fullscreen == Some(true);
            (size, v.maximized, hidden, v.monitor_size)
        });
        if !hidden {
            let maximized = maximized.unwrap_or(self.settings.window_maximized);
            self.settings
                .keep_window(size.map(|s| [s.x, s.y]), maximized);
            // A size kept on a bigger monitor is fitted to this one, once it is known.
            if let (false, false, Some(size), Some(monitor)) =
                (self.fitted, maximized, size, monitor)
            {
                self.fitted = true;
                let fit = size.min(monitor);
                if fit != size && fit.x > 0.0 && fit.y > 0.0 {
                    ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(fit));
                }
            }
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
                // Theme, Export menu, Print, search, then fit, zoom in, zoom out and the
                // page layout menu.
                let right = |with_field: bool| {
                    let search = if with_field { field } else { btn };
                    3.0 * btn + search + sep + 3.0 * btn + pct + sep + btn
                };
                // Menu, Open, Save, Undo, Redo and three separators, then the tools.
                let left = |tools: f32| 5.0 * btn + 3.0 * sep + tools;
                // Eight tools (Sign among them), then the Annotate button with its menu.
                let inline = 11.0 * btn;
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
                    ui.separator();
                    // The page layout, its icon showing the current one.
                    let glyph = actions::get(Act::Layout(c.layout)).icon;
                    ui.menu_button(menu_icon(glyph), |ui| {
                        let layouts = ACTIONS.iter().filter(|a| a.submenu == "Page layout");
                        for a in layouts {
                            if a.sep {
                                ui.separator();
                            }
                            menu_item(ui, a, &c, acts);
                        }
                    })
                    .response
                    .on_hover_text("Page layout");
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
                    let (n, name) = (t.current + 1, t.page_name(t.current));
                    ui.label(if name == n.to_string() {
                        format!("Page {n} of {}", t.page_count())
                    } else {
                        format!("Page {name} ({n} of {})", t.page_count())
                    });
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
                        ui.selectable_value(&mut t.side, Side::Signatures, "Signatures");
                    });
                    ui.separator();
                    t.guarded(|t| match t.side {
                        Side::Pages => t.show_thumbs(ui, &c),
                        Side::Bookmarks => t.show_outline(ui),
                        Side::Comments => t.show_comments(ui),
                        Side::Signatures => t.show_signatures(ui),
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
                            if !t.hits.is_empty() {
                                let a = actions::get(Act::MarkResultsRedact);
                                if ui
                                    .button(format!("{} Mark all for redaction", a.icon))
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
                        t.guarded(|t| {
                            if t.ed.owns_panel() {
                                // Stamp choices and the author's name are app-wide:
                                // lent to the tab's inspector and saved when it
                                // changes them.
                                let s = &mut self.settings;
                                let author = s.author();
                                t.ed.author = author.clone();
                                t.ed.stamp_name_date = s.stamp_name_date;
                                t.ed.stamp_pictures = s.stamp_pictures.clone();
                                t.ed.stamp_wordings = s.stamp_wordings.clone();
                                t.ed.sign_kit = s.sign.clone();
                                t.ed.inspector(ui);
                                if std::mem::take(&mut t.ed.apply_redactions) {
                                    t.requested.push(Act::ApplyRedactions);
                                }
                                if std::mem::take(&mut t.ed.finish_signing) {
                                    t.requested.push(Act::FinishSigning);
                                }
                                if std::mem::take(&mut t.ed.sign_digital) {
                                    t.requested.push(Act::SignDigital);
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
                            } else {
                                panel_apply = inspector::show(ui, t, &c);
                            }
                        })
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
            let name = apply.name();
            self.run_command(&ctx, name, |app, ctx| app.apply_panel(ctx, apply));
        }

        let canvas = egui::Frame::central_panel(ui.style())
            .fill(theme::canvas(ui.visuals().dark_mode))
            .inner_margin(0);
        let author = self.settings.author();
        egui::CentralPanel::default().frame(canvas).show(ui, |ui| {
            if let Some(t) = self.tabs.get_mut(self.active) {
                t.ed.author = author;
                t.guarded(|t| t.show_pages(ui, &c));
                if let Some(why) = &t.broken {
                    broken_state(ui, why, &mut acts);
                }
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
        self.dialogs(&ctx);
        self.restore_window(&ctx);
        self.signature_windows(&ctx);
        self.digital_windows(&ctx);

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

    /// An app with `n` tabs of one blank page, each with an unsaved change (a second page),
    /// and the last one active. Nothing is read from or written to the user's settings.
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
            app.dialog = Dialog::ConfirmFlattenAnnots;
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

use crate::about;
use crate::accent;
use crate::actions::{self, menu_item, tooltip, Act, Action, Ctx, Group, ACTIONS};
use crate::band::{self, Band};
use crate::banners::Kind as BannerKind;
use crate::dialogs::{self as dlg, Ask, Btn, Choice, Outcome, Shell};
use crate::digital::{self, IdsWindow, SignOut, SignWindow};
use crate::editor::{Action as EdAction, Parked, Tool};
use crate::icon;
use crate::incoming::Incoming;
use crate::inspector::{self, Apply, Section};
use crate::markup;
use crate::page_text;
use crate::panels::{self, Panel};
use crate::recent::Recent;
use crate::regions::{self, Region, Regions};
use crate::selection::Selection;
use crate::settings::{Accent, Layout, Settings};
use crate::signatures::{self as sigs, Check, SigState, TrustAsk, TrustedOut, TrustedWindow};
use crate::tab::{Side, State, Tab};
use crate::theme;
use crate::timing;
use crate::worker::Colours;
use eframe::egui::{self, AtomExt as _, Key};
use limarepdf_core::disk::{self, OnDisk};
use limarepdf_core::render::{image_names, ImageFormat};
use limarepdf_core::{
    parse_page_range, part_file_name, print, ImageImport, OcrResult, OpenPanel, Pdf, ScanResult,
    SplitBy, TrustEntry, TrustRole,
};
use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::Arc;

mod recover;
mod save;

use save::{AfterSave, Write};

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

/// A tab to reload: its id, and its revision when asked, so a document changed meanwhile
/// isn't replaced.
type Reloading = Option<(u64, u64)>;

/// What a file's password is asked for.
enum Unlock {
    /// To open the file in a tab of its own, at this page if given, or (`Reloading`) to
    /// read it again into a tab.
    Tab(Option<usize>, Reloading),
    /// To add its pages to a tab's document, with the files after it (BUG-20).
    Append(Box<Appending>),
}

/// PDFs whose pages are being added to a tab's document, read in the background one after
/// another (BUG-20).
struct Appending {
    /// The tab they go to, and where in it.
    tab: u64,
    to: PagesGo,
    /// The files, in order.
    files: Vec<PathBuf>,
    /// The documents read so far, one per file in order; `None` for a file left out.
    read: Vec<Option<Pdf>>,
}

/// Where pages read from files go: after the last page (Append PDFs), or after a page,
/// by its object, or first (Insert PDF pages), wherever that page goes meanwhile.
#[derive(Clone, Copy, PartialEq, Debug)]
enum PagesGo {
    End,
    After(Option<(u32, u16)>),
}

/// The documents read to add to a tab's document, some with form fields, waiting for the
/// go-ahead to draw those fields onto their pages (BUG-19): pages are added without their
/// forms, so the fields couldn't be filled in, and answers with no drawing of their own
/// would show blank.
struct AppendForms {
    asked: Asked,
    to: PagesGo,
    /// The documents, in order.
    docs: Vec<Pdf>,
    /// The names of the files with form fields.
    names: Vec<String>,
}

/// Why reading the files to append stopped at the next one.
enum AppendStop {
    /// It needs a password, or (the message) the one given was wrong.
    Password(Option<String>),
    /// It can't be read: why.
    Failed(String),
}

/// Whether opening a file failed for want of the right password.
fn needs_password(e: &str) -> bool {
    e.to_lowercase().contains("password")
}

/// What a password prompt says when asked again.
const INCORRECT_PASSWORD: &str = "Incorrect password.";

/// The title of the message about a file that can't be opened, and of several together.
const CANNOT_OPEN: &str = "Cannot open file";
const CANNOT_OPEN_SEVERAL: &str = "Cannot open files";

enum Loaded {
    Ok {
        path: PathBuf,
        pdf: Pdf,
        page: Option<usize>,
        /// For a file with signature fields: its bytes and password, to check them.
        signed: Option<(Arc<[u8]>, Option<String>)>,
        /// The file as it was read.
        version: disk::Version,
        into: Reloading,
    },
    /// How the files of tabs compare with the versions they last read or saved, each
    /// tab's id with that version.
    Disk(Vec<(u64, disk::Version, OnDisk)>),
    /// The signature check of tab `tab` (its `generation`th) finished.
    Signatures {
        tab: u64,
        generation: u64,
        check: Check,
    },
    NeedPassword {
        path: PathBuf,
        error: Option<String>,
        page: Option<usize>,
        into: Reloading,
    },
    /// The file at the path couldn't be opened: why.
    Err(PathBuf, String),
    /// Files to append were read: all of them, or as far as one that stopped the reading.
    Append(Box<Appending>, Option<AppendStop>),
    /// Search number `.0` for the printers finished: the printers, default first, or why
    /// they couldn't be listed.
    Printers(u64, Result<Vec<print::Printer>, String>),
    /// A background job on a copy of tab `tab` at `revision` finished. On success: the
    /// changed document, whether anything changed, and a summary for the user.
    Job {
        tab: u64,
        revision: u64,
        title: &'static str,
        result: Result<(Pdf, bool, String), String>,
    },
    /// A save's thread finished writing (ENH-35).
    Saved(Box<save::Saved>),
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

/// The document in front split into files in `dir`, named from `stem`.
#[derive(Clone)]
struct SplitFiles {
    by: SplitBy,
    dir: PathBuf,
    stem: String,
}

/// Files about to be written, replacing files of the same names.
#[derive(Clone)]
enum Replacing {
    Images(ImageExport),
    Split(SplitFiles),
}

impl Replacing {
    fn dir(&self) -> &Path {
        match self {
            Replacing::Images(e) => &e.dir,
            Replacing::Split(s) => &s.dir,
        }
    }
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

/// Closing a tab, or quitting, with Save chosen for a document (ENH-01): what happens once
/// its save is done ([`App::close_step`]). A save cancelled, refused or failed, or not
/// confirmed on disk, leaves the document open with its changes, and closing or quitting
/// stops.
enum Closing {
    /// Tab `tab` closes once saved, and the tab `front` was in front comes back.
    Tab { tab: u64, front: Option<u64> },
    /// Quitting: once `saving` is saved, each of `rest` still unsaved is asked about in
    /// turn, and then the app quits. Nothing closes before that, so stopping keeps every
    /// document open.
    Quit {
        saving: Option<u64>,
        rest: VecDeque<u64>,
    },
}

impl Closing {
    /// The tab whose save it waits for.
    fn saved(&self) -> Option<u64> {
        match self {
            Closing::Tab { tab, .. } => Some(*tab),
            Closing::Quit { saving, .. } => *saving,
        }
    }
}

/// The dialogs left: settings live in the inspector.
enum Dialog {
    None,
    /// The password for a file being opened, reloaded into a tab, or appended to one.
    Password(dlg::Password, Unlock),
    Message(String, String),
    /// Save, or discard, a document's changes before closing it (`true`: before quitting)?
    ConfirmClose(Asked, bool),
    /// Quit with several documents unsaved: go through them, or discard them all?
    ConfirmQuit,
    /// Export or split would replace this many files.
    ConfirmReplace(Asked, Replacing, usize),
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
    /// Add pages with their form fields drawn onto them?
    ConfirmAppendForms(Box<AppendForms>),
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
    /// Saving would replace a file another app or tab changed since the tab read or saved
    /// it, now as this version: Save as, reload, or replace it (BUG-06)?
    ConfirmStale(Asked, Box<disk::Version>),
    /// Reload the file, losing the unsaved changes?
    ConfirmReload(Asked),
}

impl Dialog {
    /// The tab the dialog is about, if it is about one.
    fn asked(&self) -> Option<&Asked> {
        match self {
            Dialog::ConfirmClose(a, _)
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
            | Dialog::ConfirmSignRecovered(a)
            | Dialog::ConfirmStale(a, _)
            | Dialog::ConfirmReload(a) => Some(a),
            Dialog::ConfirmSaveDisallowed(s) => Some(&s.asked),
            Dialog::ConfirmPrintFields(q) => Some(&q.asked),
            Dialog::ConfirmAppendForms(q) => Some(&q.asked),
            _ => None,
        }
    }

    /// The same, to change.
    fn asked_mut(&mut self) -> Option<&mut Asked> {
        match self {
            Dialog::ConfirmClose(a, _)
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
            | Dialog::ConfirmSignRecovered(a)
            | Dialog::ConfirmStale(a, _)
            | Dialog::ConfirmReload(a) => Some(a),
            Dialog::ConfirmSaveDisallowed(s) => Some(&mut s.asked),
            Dialog::ConfirmPrintFields(q) => Some(&mut q.asked),
            Dialog::ConfirmAppendForms(q) => Some(&mut q.asked),
            _ => None,
        }
    }

    /// Whether it is a question about saving a document, so one waiting while the window's
    /// close request is asked about: closing goes on only once it is answered.
    fn saving(&self) -> bool {
        matches!(
            self,
            Dialog::ConfirmClose(..)
                | Dialog::ConfirmStale(..)
                | Dialog::ConfirmSaveRepaired(_)
                | Dialog::ConfirmSaveMarks(..)
                | Dialog::ConfirmSaveSigned(..)
                | Dialog::ConfirmSaveDisallowed(_)
        )
    }

    /// Whether it is the question quitting asks.
    fn quitting(&self) -> bool {
        matches!(self, Dialog::ConfirmQuit | Dialog::ConfirmClose(_, true))
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

/// A write that replaced its file: done, or done but not confirmed on disk, with why the
/// system couldn't confirm it (decision D7). Any other failure is returned.
pub fn unconfirmed(r: anyhow::Result<()>) -> anyhow::Result<Option<String>> {
    match r {
        Ok(()) => Ok(None),
        Err(e) => match e.downcast_ref::<limarepdf_core::NotConfirmed>() {
            Some(n) => Ok(Some(n.why.clone())),
            None => Err(e),
        },
    }
}

/// `why` the system couldn't confirm a file written is safely on disk, said after "but".
pub fn unconfirmed_text(why: &str) -> String {
    format!("your system couldn't confirm it is safely on disk ({why})")
}

/// What a change to the trust list adds to its sentence when the list was written but not
/// confirmed on disk.
fn trust_unsure(why: Option<String>) -> String {
    why.map_or_else(String::new, |why| {
        format!(", but your system couldn't confirm the trust list is safely on disk ({why})")
    })
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

/// Title of the split background job and its messages.
const SPLIT_JOB: &str = "Split into files";

/// Width of the inspector panel, and of its rail when collapsed (build plan 2.2's
/// workspace: 205 points).
const INSPECTOR_WIDTH: f32 = 205.0;
const RAIL_WIDTH: f32 = 36.0;
/// Width of the side panel showing the pages, as the workspace has it.
const SIDE_WIDTH: f32 = 132.0;
/// Width of the side panel showing search results, bookmarks, comments or signatures.
const SIDE_TEXT_WIDTH: f32 = 220.0;
/// Below this window width the inspector collapses to its strip by itself.
const NARROW_WIDTH: f32 = 960.0;
/// Below this window width the side panel folds away by itself (View > Pages panel brings
/// it back).
const SIDE_NARROW_WIDTH: f32 = 760.0;
/// Below this window height the commands are compact by themselves (Compact commands).
const SHORT_HEIGHT: f32 = 600.0;

/// Copy, Cut or Paste in the band, for the text being typed: asked of the system once the
/// typing has the keyboard back ([`App::clipboard`]).
#[derive(Clone, Copy, PartialEq, Debug)]
enum Clip {
    Copy,
    Cut,
    Paste,
}

/// An accessible name for a control showing only an icon.
fn named(label: &str) -> egui::WidgetInfo {
    egui::WidgetInfo::labeled(egui::Role::Button, true, label)
}

/// How long the undo toast stays, in seconds.
const TOAST_SECONDS: f64 = 6.0;
/// How long a toast waits while its tab is behind another, in seconds, and the least it
/// shows for when that tab comes back to the front within the wait.
const TOAST_AWAY: f64 = 30.0;
const TOAST_BACK: f64 = 3.0;

/// A brief notice at the bottom of the window about something that happened in one tab,
/// shown while that tab is in front. Undo takes back the change it tells of, and goes as
/// soon as that change isn't the last thing in the tab (BUG-02).
struct Toast {
    tab: u64,
    text: String,
    /// The tab's [`Tab::history`] just after the change, for a change Undo takes back.
    undo: Option<(u64, usize, usize)>,
    /// When it goes away (egui input time), while its tab is in front.
    until: f64,
    /// While its tab is behind another: since when, and the seconds it had left.
    away: Option<(f64, f64)>,
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
    /// The side panel at the left: pages, search results, bookmarks, comments, signatures.
    side: Panel,
    status: String,
    last_title: String,
    quit_ok: bool,
    /// A background job (scan optimisation, OCR) is running.
    busy: bool,
    palette: Option<Palette>,
    /// The inspector at the right (else collapsed to its strip).
    inspector: Panel,
    /// The collapsed inspector's contents are shown in a pop-out over the pages.
    popout: bool,
    /// The panels whose edge the user is moving: their width is kept once let go.
    resizing: Vec<&'static str>,
    /// The band's name chosen: its commands show under the names (ENH-38).
    band: Band,
    /// Copy, Cut or Paste clicked in the band for the text being typed (its box's id),
    /// waiting for that box to have the keyboard back.
    clip: Option<(Clip, egui::Id)>,
    /// The Undo and Redo buttons, as last drawn: a click on them leaves the keyboard in
    /// the text box that has it.
    undo_buttons: Vec<egui::Rect>,
    /// This frame's click is on Undo or Redo while a text box has the keyboard.
    undo_held: bool,
    /// Undo or Redo clicked while a text box had the keyboard, sent to it as its key on the
    /// next frame.
    undo_in_box: Option<Act>,
    /// This frame's Esc left a box of the band or the inspector, with typing open on the
    /// page: it goes back to the typing, which stays open.
    esc_to_typing: bool,
    /// The commands' state as this frame drew them, while the commands clicked in it run: a
    /// toggle goes by what it showed, as its click may have changed what it shows (a line
    /// being edited ends when Edit text is clicked).
    shown: Option<Ctx>,
    /// The text box the keyboard last left: its id, and its tab's id and revision then.
    left_box: Option<(egui::Id, u64, u64)>,
    /// The id of the tab that was in front last frame.
    front: Option<u64>,
    /// The width of the status bar's controls at its right, as last drawn.
    status_width: Option<f32>,
    /// The width of Print and the compact commands button, at the commands' right, as last
    /// drawn.
    commands_right: Option<f32>,
    /// The desktop's accent colour being read off the window's thread ([`App::read_accent`]).
    accent_read: Option<Receiver<Option<egui::Color32>>>,
    /// The window has been fitted to its monitor, at the first frame the monitor's size was known.
    fitted: bool,
    /// The window's regions, which F6 moves the keyboard between (ENH-10).
    regions: Regions,
    /// The control that had the keyboard last frame (see [`page_text::fresh_history`]).
    keyboard: Option<egui::Id>,
    /// At most one for each tab.
    toasts: Vec<Toast>,
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
    /// The files being read for tabs, so that opening one again while it is read doesn't
    /// give it a second tab.
    opening: Vec<PathBuf>,
    /// The user said to replace this version of the tab's file, which another app or tab
    /// changed (see [`Dialog::ConfirmStale`]).
    replace_ok: Option<(u64, disk::Version)>,
    /// The window had the keyboard last frame: getting it back, the tabs' files are
    /// checked for changes made meanwhile.
    focused: bool,
    /// The tab and selection last offered for the PRIMARY selection ([`App::offer_selection`]).
    offered: Option<(u64, Selection)>,
    /// The settings or recent files couldn't be saved, and that was said: once a session
    /// ([`App::say_unsaved`]).
    said_unsaved: bool,
    /// Dialogs waiting, in turn, for the one open to be answered (BUG-07).
    waiting: VecDeque<Dialog>,
    /// A file being read again with the password given for it: the dialogs waiting wait
    /// for the result, so that a wrong password is asked for again first.
    retrying: Option<PathBuf>,
    /// The number of the last search for printers started (see [`dlg::Search`]).
    searches: u64,
    /// Closing a tab or quitting, once a document is saved (ENH-01).
    closing: Option<Closing>,
    /// Files the system asks to open, from Finder on macOS (BUG-28).
    incoming: Incoming,
    /// Saves being written on their threads (ENH-35), and the number of the last one.
    saves: Vec<save::Running>,
    saves_made: u64,
    /// What waits for a save to end.
    after_saves: Vec<AfterSave>,
    /// Tests: the next save's thread waits for this before writing.
    #[cfg(test)]
    save_gate: Option<Receiver<()>>,
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
        crate::font_list::warm(&cc.egui_ctx);
        let mut app = Self::with(settings, Recent::load());
        // The desktop's accent before the first frame, unless the desktop is slow to say:
        // then it comes in as soon as it is known.
        app.read_accent(&cc.egui_ctx);
        if let Some(rx) = &app.accent_read {
            if let Ok(accent) = rx.recv_timeout(std::time::Duration::from_millis(300)) {
                theme::set_accent(&cc.egui_ctx, accent);
                app.accent_read = None;
            }
        }
        app.incoming = Incoming::system().clone();
        app.incoming.wake(&cc.egui_ctx);
        app.start_recovery(&cc.egui_ctx, files, page);
        timing::mark("app made", String::new);
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
            palette: None,
            inspector: Panel::new(settings.inspector_panel),
            popout: false,
            resizing: Vec::new(),
            band: Band::default(),
            clip: None,
            undo_buttons: Vec::new(),
            undo_held: false,
            undo_in_box: None,
            esc_to_typing: false,
            shown: None,
            left_box: None,
            front: None,
            status_width: None,
            commands_right: None,
            accent_read: None,
            fitted: false,
            regions: Regions::default(),
            keyboard: None,
            toasts: Vec::new(),
            recent,
            settings,
            sig_details: None,
            trust_ask: None,
            trusted: None,
            ids_window: None,
            sign_window: None,
            rescue: recover::Rescue::default(),
            opening: Vec::new(),
            replace_ok: None,
            focused: true,
            offered: None,
            said_unsaved: false,
            waiting: VecDeque::new(),
            retrying: None,
            searches: 0,
            closing: None,
            incoming: Incoming::default(),
            saves: Vec::new(),
            saves_made: 0,
            after_saves: Vec::new(),
            #[cfg(test)]
            save_gate: None,
        }
    }

    /// Opens the PDF at `path` in a tab of its own. A file open already, by any path or
    /// link to it, or being opened, isn't opened again: its tab comes to the front, and its
    /// file is checked for changes made since it was read (BUG-06).
    fn spawn_open(
        &mut self,
        ctx: &egui::Context,
        path: PathBuf,
        password: Option<String>,
        page: Option<usize>,
    ) {
        if let Some(i) = self.tab_of(&path) {
            self.active = i;
            if let Some(p) = page {
                self.tabs[i].go_to(p.saturating_sub(1));
            }
            self.check_disk(ctx, Some(self.tabs[i].id));
            return;
        }
        if !self.reading(&path) {
            self.read_file(ctx, path, password, page, None);
        }
    }

    /// Whether the file at `path`, by any path or link to it, is being read for a tab, or
    /// its password is asked for to read it.
    fn reading(&self, path: &Path) -> bool {
        let asks = |d: &Dialog| match d {
            Dialog::Password(s, Unlock::Tab(..)) => disk::same_file(&s.path, path),
            _ => false,
        };
        let asking = std::iter::once(&self.dialog).chain(&self.waiting).any(asks);
        asking || self.opening.iter().any(|p| disk::same_file(p, path))
    }

    /// Shows `d` once the dialog open now, and those waiting before it, are answered: a
    /// result coming in the background never replaces a question (BUG-07). Files that
    /// can't be opened, waiting together, are told about in one message.
    fn show(&mut self, d: Dialog) {
        if let Dialog::Message(title, text) = &d {
            if title == CANNOT_OPEN {
                let same = self.waiting.iter_mut().find_map(|w| match w {
                    Dialog::Message(t, x) if t == CANNOT_OPEN || t == CANNOT_OPEN_SEVERAL => {
                        Some((t, x))
                    }
                    _ => None,
                });
                if let Some((t, x)) = same {
                    *t = CANNOT_OPEN_SEVERAL.into();
                    x.push_str("\n\n");
                    x.push_str(text);
                    return;
                }
            }
        }
        self.waiting.push_back(d);
        self.next_dialog();
    }

    /// Asks for a password, `again` (after a wrong one, or the file skipped before it)
    /// ahead of the dialogs waiting.
    fn ask_password(&mut self, d: Dialog, again: bool) {
        if again {
            self.waiting.push_front(d);
            self.next_dialog();
        } else {
            self.show(d);
        }
    }

    /// Opens the next dialog waiting, once none is open and no password given is being
    /// tried.
    fn next_dialog(&mut self) {
        if matches!(self.dialog, Dialog::None) && self.retrying.is_none() {
            if let Some(d) = self.waiting.pop_front() {
                self.dialog = d;
            }
        }
    }

    /// The tab whose file is at `path`, by any path or link to it.
    fn tab_of(&self, path: &Path) -> Option<usize> {
        self.tabs
            .iter()
            .position(|t| t.path.as_deref().is_some_and(|p| disk::same_file(p, path)))
    }

    /// The file at `path` was read, or couldn't be: it isn't being opened any more.
    fn opened(&mut self, path: &Path) {
        if let Some(i) = self.opening.iter().position(|p| p == path) {
            self.opening.remove(i);
        }
        if self.retrying.as_deref() == Some(path) {
            self.retrying = None;
        }
    }

    /// Reads the PDF at `path` in the background, for a new tab or (`into`) to reload one.
    fn read_file(
        &mut self,
        ctx: &egui::Context,
        path: PathBuf,
        password: Option<String>,
        page: Option<usize>,
        into: Reloading,
    ) {
        self.opening.push(path.clone());
        let tx = self.tx.clone();
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            timing::mark("open start", || path.display().to_string());
            type Read = (Pdf, Option<Arc<[u8]>>, disk::Version);
            let open = || -> limarepdf_core::Result<Read> {
                use anyhow::Context as _;
                let (bytes, version) =
                    disk::read(&path).with_context(|| format!("reading {}", path.display()))?;
                let pdf = Pdf::from_bytes(&bytes, password.as_deref())?;
                // Signed files keep their bytes, to check and for View signed version.
                let fields = pdf.form_fields();
                let signed = fields
                    .iter()
                    .any(|f| f.kind == limarepdf_core::FieldKind::Signature)
                    .then(|| Arc::from(bytes));
                Ok((pdf, signed, version))
            };
            let read = crate::guard(open);
            timing::mark("open end", || {
                let how = if read.is_ok() { "read" } else { "failed" };
                format!("{} {how}", path.display())
            });
            let msg = match read {
                Ok((pdf, signed, version)) => Loaded::Ok {
                    path,
                    pdf,
                    page,
                    signed: signed.map(|b| (b, password)),
                    version,
                    into,
                },
                Err(e) => {
                    let s = format!("{e:#}");
                    if needs_password(&s) {
                        Loaded::NeedPassword {
                            path,
                            error: password.map(|_| INCORRECT_PASSWORD.to_string()),
                            page,
                            into,
                        }
                    } else {
                        let text = format!("{}: {s}", path.display());
                        Loaded::Err(path, text)
                    }
                }
            };
            let _ = tx.send(msg);
            ctx.request_repaint();
        });
    }

    /// Reads tab `i`'s file again into it, as it now is.
    fn reload(&mut self, ctx: &egui::Context, i: usize) {
        let Some(t) = self.tabs.get(i) else {
            return;
        };
        let (Some(path), into) = (t.path.clone(), Some((t.id, t.revision))) else {
            return;
        };
        if !self.reading(&path) {
            self.read_file(ctx, path, None, None, into);
        }
    }

    /// Checks in the background whether the files of the tabs (or of tab `only`) still hold
    /// what the tabs last read or saved; a tab whose file another app or tab changed shows
    /// a banner about it.
    fn check_disk(&self, ctx: &egui::Context, only: Option<u64>) {
        let files: Vec<_> = self
            .tabs
            .iter()
            .filter(|t| only.is_none_or(|id| id == t.id))
            .filter_map(|t| Some((t.id, t.path.clone()?, t.disk.clone()?)))
            .collect();
        if files.is_empty() {
            return;
        }
        let (tx, ctx) = (self.tx.clone(), ctx.clone());
        std::thread::spawn(move || {
            let found = files
                .into_iter()
                .filter_map(|(id, path, known)| {
                    let now = disk::compare(&path, &known).ok()?;
                    Some((id, known, now))
                })
                .collect();
            let _ = tx.send(Loaded::Disk(found));
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
                t.use_labels = self.settings.page_labels;
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
                self.show(Dialog::Message("Cannot open".into(), format!("{e:#}")));
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
        let result = (|| -> limarepdf_core::Result<(limarepdf_core::Added, Option<String>)> {
            let path = limarepdf_core::TrustList::default_path().ok_or_else(|| {
                anyhow::anyhow!(
                    "LimarePDF can't find its settings folder (HOME or APPDATA isn't set)"
                )
            })?;
            let mut list = limarepdf_core::TrustList::load(&path)?;
            let label = ask.label.trim();
            let added = list.add(&cert.der, ask.role, (!label.is_empty()).then_some(label))?;
            Ok((added, unconfirmed(list.save(&path))?))
        })();
        match result {
            Ok((added, unsure)) => {
                let was = match added {
                    limarepdf_core::Added::RoleChanged(old) => format!(" (instead of as {old})"),
                    limarepdf_core::Added::New => String::new(),
                };
                let name = limarepdf_core::display_line(ask.label.trim(), 80);
                self.status = format!(
                    "Now trusting \"{name}\" as {}{was}{}. {}",
                    ask.role,
                    trust_unsure(unsure),
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
        let result = (|| -> limarepdf_core::Result<(TrustEntry, Option<String>)> {
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
            Ok((e, unconfirmed(list.save(&path))?))
        })();
        match result {
            Ok((e, unsure)) => {
                self.status = format!(
                    "No longer trusting \"{}\"{}. {}",
                    limarepdf_core::display_line(&e.label, 80),
                    trust_unsure(unsure),
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

    /// Says what of LimarePDF's own couldn't be saved: a signature saved for next time
    /// each time, anything else (settings, recent files) once a session.
    fn say_unsaved(&mut self) {
        if let Some(text) = self.settings.take_sign_unsaved() {
            self.show(Dialog::Message("Not saved for next time".into(), text));
        }
        let once = self.settings.take_unsaved();
        if let Some((title, text)) = once.or_else(|| self.recent.take_unsaved()) {
            if !std::mem::replace(&mut self.said_unsaved, true) {
                let text = format!("{text} This isn't said again until LimarePDF starts again.");
                self.show(Dialog::Message(title, text));
            }
        }
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
        // Never over another tab's file, nor over this tab's own when something else
        // changed it since (BUG-06).
        if let Some(why) = self.in_other_tab(&out, Some(w.tab)) {
            w.error = Some(why);
            return;
        }
        let i = self.tabs.iter().position(|t| t.id == w.tab);
        if i.and_then(|i| self.stale(i, &out)).is_some() {
            w.error = Some(
                "Another app or tab changed this file since LimarePDF opened or saved it. \
                 Reload it first, or choose another name."
                    .into(),
            );
            return;
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
        if let Some(why) = &s.default_unsaved {
            text.push_str(&format!("\n\nThe document is signed, but {why}."));
        }
        if s.self_issued {
            text.push_str(
                "\n\nOthers see you as an unknown signer until they trust your certificate: \
                 send it to them from Document > Digital signatures > Digital IDs… (Export…, \
                 Certificate only). LimarePDF shows the same until you trust it.",
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
                    let t = &mut self.tabs[self.active];
                    // Saving again is one click (decision D7).
                    t.dirty = !s.confirmed;
                    t.disk = t.path.as_deref().and_then(|p| disk::version(p).ok());
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
        self.show(Dialog::Message(what.into(), text));
    }

    fn poll_loads(&mut self, ctx: &egui::Context) {
        while let Ok(m) = self.rx.try_recv() {
            match m {
                Loaded::Ok {
                    path,
                    pdf,
                    page,
                    signed,
                    version,
                    into,
                } => {
                    self.opened(&path);
                    self.recent.add(&path);
                    // Reloading: the tab, as it was when asked, is still there, with nothing
                    // typed since on the pages or in the inspector (a title, a label), which
                    // the file would replace.
                    let reload = match into {
                        Some((id, revision)) => {
                            let i = self.tabs.iter().position(|t| t.id == id);
                            let ready = i.filter(|&i| {
                                let t = &self.tabs[i];
                                t.revision == revision && !t.ed.pending() && !t.typing_in_forms()
                            });
                            if ready.is_none() {
                                self.show(Dialog::Message(
                                    "Not reloaded".into(),
                                    "The document was changed, typed in or closed while its \
                                     file was read again, so it wasn't reloaded."
                                        .into(),
                                ));
                                continue;
                            }
                            ready
                        }
                        // A tab got the file meanwhile (its Save as, say): that one is shown.
                        None => match self.tab_of(&path) {
                            Some(i) => {
                                self.active = i;
                                continue;
                            }
                            None => None,
                        },
                    };
                    if self.add_tab(ctx, pdf, Some(path), page, false) {
                        self.tabs[self.active].disk = Some(version);
                        if let Some(i) = reload {
                            // add_tab put it last: it takes the old tab's place there.
                            if let Some(new) = self.tabs.pop() {
                                let old = std::mem::replace(&mut self.tabs[i], new);
                                self.active = i;
                                if page.is_none() {
                                    self.tabs[i].go_to(old.current);
                                }
                                self.closed_copy(&old);
                            }
                        }
                        if let Some((bytes, password)) = signed {
                            self.check_signatures(ctx, self.active, bytes, password);
                        }
                    }
                }
                Loaded::Disk(found) => {
                    for (id, known, now) in found {
                        // Its own save is replacing the file.
                        if self.saving(id) {
                            continue;
                        }
                        let Some(t) = self.tabs.iter_mut().find(|t| t.id == id) else {
                            continue;
                        };
                        // Saved or reloaded since the check started.
                        if t.disk.as_ref() != Some(&known) {
                            continue;
                        }
                        match now {
                            OnDisk::Same(v) => {
                                t.disk = Some(v);
                                t.changed_on_disk = None;
                                t.drop_banner(BannerKind::ChangedOnDisk);
                            }
                            OnDisk::Changed(v) => {
                                let seen = t.changed_on_disk.as_ref();
                                if !seen.is_some_and(|s| s.same_bytes(&v)) {
                                    t.show_banner(crate::banners::changed_on_disk());
                                    t.changed_on_disk = Some(v);
                                }
                            }
                            // Saving puts it back.
                            OnDisk::Gone => {}
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
                Loaded::NeedPassword {
                    path,
                    error,
                    page,
                    into,
                } => {
                    let again = self.retrying.as_ref() == Some(&path);
                    self.opened(&path);
                    let ask = dlg::Password {
                        path,
                        input: String::new(),
                        error,
                    };
                    self.ask_password(Dialog::Password(ask, Unlock::Tab(page, into)), again);
                }
                Loaded::Err(path, e) => {
                    self.opened(&path);
                    self.show(Dialog::Message(CANNOT_OPEN.into(), e));
                }
                Loaded::Printers(search, found) => {
                    let mut dialogs = std::iter::once(&mut self.dialog).chain(&mut self.waiting);
                    let looking = dialogs.find_map(|d| match d {
                        Dialog::Print(_, s) if s.search == dlg::Search::Running(search) => Some(s),
                        _ => None,
                    });
                    timing::mark("printers found", || match &found {
                        Ok(list) => list.len().to_string(),
                        Err(_) => "failed".into(),
                    });
                    if let Some(s) = looking {
                        s.found(found);
                    }
                }
                Loaded::Append(a, stop) => {
                    self.busy = false;
                    self.status.clear();
                    let again = a.files.iter().any(|f| self.retrying.as_ref() == Some(f));
                    if again {
                        self.retrying = None;
                    }
                    match stop {
                        None => self.append_read(ctx, *a),
                        Some(AppendStop::Password(error)) => {
                            let ask = dlg::Password {
                                path: a.files[a.read.len()].clone(),
                                input: String::new(),
                                error,
                            };
                            self.ask_password(Dialog::Password(ask, Unlock::Append(a)), again);
                        }
                        Some(AppendStop::Failed(why)) => self.show(Dialog::Message(
                            match a.to {
                                PagesGo::End => "Append failed".into(),
                                PagesGo::After(_) => "Insert failed".into(),
                            },
                            format!("{why}\n\nNothing was added."),
                        )),
                    }
                }
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
                            self.show(Dialog::Message(title.into(), e));
                            continue;
                        }
                    };
                    // Typing since the job started is a change too: it goes in, and the
                    // result, made without it, is discarded.
                    if changed && (!t.keep_page_typing(ctx) || t.revision != revision) {
                        self.show(Dialog::Message(
                            title.into(),
                            "The document changed while this was running, so the result \
                             was discarded. Run it again."
                                .into(),
                        ));
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
                    self.show(Dialog::Message(title.into(), text));
                }
                Loaded::Saved(s) => self.saved(ctx, *s),
            }
        }
        self.next_dialog();
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
                    self.show(Dialog::Message(
                        "Cannot read image".into(),
                        format!("{}: {e}", f.display()),
                    ));
                    return None;
                }
            }
        }
        Some(out)
    }

    /// Run `work` on a copy of the active document on a background thread, what is being
    /// typed on its pages kept first. The result is applied (undoably) when it arrives, if
    /// the document has not changed meanwhile.
    fn spawn_job(
        &mut self,
        ctx: &egui::Context,
        title: &'static str,
        work: impl FnOnce(&mut Pdf) -> limarepdf_core::Result<(bool, String)> + Send + 'static,
    ) {
        if !self.tabs[self.active].keep_page_typing(ctx) {
            return;
        }
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

    /// Looks for the printers on a background thread, returning the search's number. Its
    /// result fills in the Print dialog looking for it, open or waiting, or is dropped:
    /// one closed meanwhile isn't opened again (BUG-11).
    fn find_printers(&mut self, ctx: &egui::Context) -> u64 {
        self.searches += 1;
        let (search, tx, ctx) = (self.searches, self.tx.clone(), ctx.clone());
        std::thread::spawn(move || {
            let found = crate::guard(print::printers).map_err(|e| format!("{e:#}"));
            let _ = tx.send(Loaded::Printers(search, found));
            ctx.request_repaint();
        });
        search
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
            match p.export_images(&e.pages, e.dpi, e.format, &e.dir, &e.stem) {
                Ok(files) => {
                    let n = files.len();
                    Ok((false, format!("Wrote {n} image(s) to {}.", e.dir.display())))
                }
                Err(err) if not_confirmed(&err) => Ok((
                    false,
                    format!(
                        "Wrote the images to {}, but your system couldn't confirm they are \
                         safely on disk. If the computer lost power just now, export them again.",
                        e.dir.display()
                    ),
                )),
                Err(err) => Err(err),
            }
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
        if let Some(why) = self.in_other_tab(&path, None) {
            self.dialog = Dialog::Message("Open in a tab".into(), why);
            return;
        }
        match limarepdf_core::write_file(&path, text.as_bytes()) {
            Ok(()) => self.status = format!("Wrote the text to {}", path.display()),
            Err(e) if not_confirmed(&e) => {
                self.dialog = Dialog::Message("Saved, not confirmed".into(), NOT_CONFIRMED.into())
            }
            Err(e) => {
                self.dialog =
                    Dialog::Message("Export failed".into(), format!("{}: {e:#}", path.display()))
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
        if let Some(why) = self.in_other_tab(&path, None) {
            self.dialog = Dialog::Message("Open in a tab".into(), why);
            return;
        }
        let t = &self.tabs[self.active];
        let result: limarepdf_core::Result<()> = match layout {
            dlg::Layout::TextFile => limarepdf_core::write_file(&path, text.as_bytes()),
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
            Err(e) => self.show(Dialog::Message(
                "Cannot import images".into(),
                format!("{e:#}"),
            )),
        }
    }

    /// Saves, with what is being typed kept first; if it can't be, nothing is saved.
    /// Saving over a file that was repaired to open it asks first, offering Save as
    /// instead: the original is only replaced when the user says so.
    fn save(&mut self, ctx: &egui::Context, as_new: bool) {
        if let Some(t) = self.tabs.get_mut(self.active) {
            if !t.keep_typing(ctx) {
                return;
            }
        }
        // Before the other questions, which are about what is saved: where it goes.
        if !as_new && self.asked_stale(None) {
            return;
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
        let mut p = Self::save_as(d)?;
        if p.extension().is_none() {
            p.set_extension("pdf");
        }
        Some(p)
    }

    /// The file the user picks in the save chooser `d`, if any.
    #[cfg(not(test))]
    fn save_as(d: rfd::FileDialog) -> Option<PathBuf> {
        d.save_file()
    }

    /// Tests never open the chooser, which would show on the desktop: they get the file
    /// [`tests::SAVE_AS`] holds, or nothing, as when the chooser is cancelled.
    #[cfg(test)]
    fn save_as(_: rfd::FileDialog) -> Option<PathBuf> {
        tests::SAVE_AS.with(std::cell::Cell::take)
    }

    /// Where to save the active tab (see [`Self::save_path`]); `None`, saying why, for a
    /// file open in another tab, or one another app or tab changed since this tab last
    /// read or saved it, until the user says to replace it (BUG-06).
    fn save_target(&mut self, as_new: bool) -> Option<PathBuf> {
        let t = self.tabs.get(self.active)?;
        let path = Self::save_path(t, as_new)?;
        if let Some(why) = self.in_other_tab(&path, Some(t.id)) {
            self.dialog = Dialog::Message("Open in another tab".into(), why);
            return None;
        }
        if self.asked_stale(Some(&path)) {
            return None;
        }
        Some(path)
    }

    /// Why nothing is written to `path` when it is the file of a tab other than `except`:
    /// that tab would be left with an older copy, to save back over it.
    fn in_other_tab(&self, path: &Path, except: Option<u64>) -> Option<String> {
        let t = self.tabs.iter().find(|t| {
            Some(t.id) != except && t.path.as_deref().is_some_and(|p| disk::same_file(p, path))
        })?;
        let which = if except.is_some() {
            "another tab"
        } else {
            "a tab"
        };
        Some(format!(
            "\"{}\" is open in {which}. Close that tab first, or choose another name. Nothing \
             was saved.",
            t.title()
        ))
    }

    /// The file at `path` as it now is, when it is tab `i`'s own file and another app or
    /// tab changed it since the tab last read or saved it, unless the user said to replace
    /// that version. A file found unchanged becomes the version known, read later: one
    /// whose time was too close to the reading to trust (PB-28) is then read once, not at
    /// every check.
    fn stale(&mut self, i: usize, path: &Path) -> Option<disk::Version> {
        let t = self.tabs.get_mut(i)?;
        let (own, known) = (t.path.as_deref()?, t.disk.as_ref()?);
        if !disk::same_file(own, path) {
            return None;
        }
        let now = match disk::compare(path, known) {
            Ok(OnDisk::Changed(now)) => now,
            Ok(OnDisk::Same(now)) => {
                t.disk = Some(now);
                return None;
            }
            _ => return None,
        };
        let t = &self.tabs[i];
        let ok = self.replace_ok.as_ref();
        let ok = ok.is_some_and(|(id, v)| *id == t.id && v.same_bytes(&now));
        (!ok).then_some(now)
    }

    /// Asks before the active tab replaces its file (or the one at `path`) when another
    /// app or tab changed it; true when it asked.
    fn asked_stale(&mut self, path: Option<&Path>) -> bool {
        let Some(t) = self.tabs.get(self.active) else {
            return false;
        };
        let Some(path) = path.map(Path::to_path_buf).or_else(|| t.path.clone()) else {
            return false;
        };
        let Some(now) = self.stale(self.active, &path) else {
            return false;
        };
        let t = &mut self.tabs[self.active];
        // With Reload, which the question has no room for.
        t.show_banner(crate::banners::changed_on_disk());
        t.changed_on_disk = Some(now.clone());
        self.dialog = Dialog::ConfirmStale(Asked::of(t), Box::new(now));
        true
    }

    /// Writes a signed document whose edits were added after what was signed (`bytes`), on
    /// a thread ([`App::start_save`]). The signatures stay, and the saved file becomes what
    /// later saves add to.
    fn save_appended(&mut self, ctx: &egui::Context, as_new: bool, bytes: Vec<u8>) {
        let Some(path) = self.save_target(as_new) else {
            return;
        };
        self.start_save(ctx, path, Write::Appended(bytes), Some(None));
    }

    /// Writes the whole document, on a thread ([`App::start_save`]).
    fn save_now(&mut self, ctx: &egui::Context, as_new: bool) {
        let Some(path) = self.save_target(as_new) else {
            return;
        };
        let tab = &mut self.tabs[self.active];
        tab.pdf.set_encryption(tab.encryption.clone());
        // The Signatures tab describes the file, so the saved one is checked (a saved
        // recovered copy's too, which was never checked).
        let password = tab.encryption.as_ref().map(|e| e.user_password.clone());
        let recheck =
            tab.sigs.is_some() || (tab.recovered.is_some() && tab.pdf.signature_count() > 0);
        let pdf = Box::new(tab.pdf.clone());
        self.start_save(ctx, path, Write::Whole(pdf), recheck.then_some(password));
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
        // Undo or Redo clicked while a text box has the keyboard: its typing, not the
        // document, as with the keys.
        if matches!(a, Act::Undo | Act::Redo) && self.undo_held {
            self.undo_in_box = Some(a);
            ctx.request_repaint();
            return;
        }
        let has = !self.tabs.is_empty();
        // What is being typed goes into the document before a command uses it, or the
        // command doesn't run (BUG-01). A question the command asks is then about the
        // document with the typing in it (BUG-10). Bold and Italic change the line being
        // edited on the page instead, which stays open, as Ctrl+B and Ctrl+I there do; Cut
        // cuts from the text being typed.
        let styles = matches!(a, Act::Bold | Act::Italic);
        let in_line = styles && has && self.tabs[self.active].ed.editing_line();
        let in_typing = a == Act::Cut && has && self.tabs[self.active].ed.typing_id().is_some();
        let on_typing = in_line || in_typing;
        if has && a.uses_document() && !on_typing {
            let t = &mut self.tabs[self.active];
            let kept = match a {
                Act::Undo | Act::Redo => t.keep_typing_but_drafts(ctx),
                _ => t.keep_typing(ctx),
            };
            if !kept {
                return;
            }
        }
        // The page view's commands go back to it first, at the page selected (NEW-14).
        if has && self.tabs[self.active].organising() && of_the_page(a) {
            let t = &mut self.tabs[self.active];
            let p = t.targets()[0];
            t.open_page(ctx, p);
        }
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
            Act::Save | Act::SaveAs if has && self.saving(self.tabs[self.active].id) => {
                self.save_again(a == Act::SaveAs);
            }
            // Reading the file again, or replacing the tab with a signed one, waits for the
            // tab's save (its commands are off meanwhile; a banner's Reload isn't).
            Act::Reload | Act::SignDigital | Act::Certify | Act::DigitalPlaced
                if has && self.saving(self.tabs[self.active].id) =>
            {
                self.show(Dialog::Message(
                    "Still saving".into(),
                    "This document is still being saved. Try again once it is saved.".into(),
                ));
            }
            Act::Save => self.save(ctx, false),
            Act::SaveAs => self.save(ctx, true),
            Act::Reload if has => {
                let t = &self.tabs[self.active];
                if t.unsaved() {
                    self.dialog = Dialog::ConfirmReload(Asked::of(t));
                } else {
                    self.reload(ctx, self.active);
                }
            }
            Act::ExportImages if has => self.reveal(Section::Export),
            Act::ExportText if has => self.export_text(),
            Act::Print | Act::PrintSelection if has && !self.busy => {
                // Open at once, with the printers looked for meanwhile (BUG-11).
                let search = self.find_printers(ctx);
                let t = &self.tabs[self.active];
                let selected = t.ed.selection.pages();
                let file = t.title();
                let selection = (a == Act::PrintSelection).then_some(dlg::Layout::Original);
                timing::mark("print dialog", String::new);
                self.dialog = Dialog::Print(
                    self.asked(),
                    Box::new(dlg::Print {
                        all: selected.is_none(),
                        pages: selected.map(inspector::range_text).unwrap_or_default(),
                        selection,
                        ..dlg::Print::looking(search, file)
                    }),
                );
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
                    self.tabs[self.active].info = Some(
                        "There are no redaction marks: mark areas with Protect > Mark redaction, \
                         or mark selected text or search results"
                            .into(),
                    );
                } else {
                    self.dialog = Dialog::ConfirmApplyRedactions(self.asked(), n);
                }
            }
            Act::FinishSigning if has => {
                let n = self.tabs[self.active].pdf.sign_items();
                if n == 0 {
                    self.tabs[self.active].info = Some(
                        "There is nothing to finish: put down a signature, the date or a tick \
                         with Fill and sign first"
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
            Act::CloseTab if has => self.close_asking(self.active),
            Act::ToggleRecovery => self.toggle_recovery(),
            Act::ShowRecoveryFile => self.show_leftover(),
            Act::Quit => self.quit(ctx),
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
            Act::TogglePlanned => {
                self.settings.show_planned = !self.settings.show_planned;
                self.settings.save();
            }
            Act::ToggleCompact => {
                self.settings.compact_commands = !self.settings.compact_commands;
                self.settings.save();
            }
            Act::Accent(accent) => {
                self.settings.accent = accent;
                self.settings.save();
                self.read_accent(ctx);
            }
            Act::Palette => self.palette = Some(Palette::default()),
            Act::NextRegion => self.regions.step = 1,
            Act::PrevRegion => self.regions.step = -1,
            // The tab strip scrolls the new one into view.
            Act::NextDocument | Act::PrevDocument => {
                let n = self.tabs.len();
                if n > 1 {
                    let by = if a == Act::NextDocument { 1 } else { n - 1 };
                    self.active = (self.active + by) % n;
                }
            }
            Act::TrustedCertificates => self.trusted = Some(TrustedWindow::load()),
            _ if !has => {}
            Act::PageLabels => self.reveal_labels(),
            Act::PagesView => self.show_view(Side::Pages),
            Act::SearchView => self.show_view(Side::Search),
            Act::Bookmarks => self.show_view(Side::Bookmarks),
            Act::Comments => self.show_view(Side::Comments),
            Act::PrevComment | Act::NextComment => {
                let t = &mut self.tabs[self.active];
                if let Some(said) = t.step_comment(a == Act::NextComment) {
                    t.info = Some(said.into());
                }
            }
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
            Act::TogglePageLabels => {
                let s = &mut self.settings;
                s.page_labels = !s.page_labels;
                s.save();
                for t in &mut self.tabs {
                    t.use_labels = s.page_labels;
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
                if t.edit(ctx, |p| p.reset_form()) {
                    t.notice = Some("Reset the form".into());
                }
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
            Act::SelectAllObjects => {
                let t = &mut self.tabs[self.active];
                t.ed.select_all_objects(&t.pdf, t.current);
            }
            Act::Bold | Act::Italic => {
                let t = &mut self.tabs[self.active];
                if let Some(said) = t.ed.switch_style(ctx, &t.pdf, a == Act::Bold) {
                    t.info = Some(said);
                }
                t.apply_actions(ctx);
            }
            Act::DeleteObject => self.tabs[self.active].ed.delete_selected(),
            Act::DeleteSelection => {
                if self.tabs[self.active].ed.has_object() {
                    self.act(ctx, Act::DeleteObject);
                } else if self.tabs[self.active].ed.selection.pages().is_some() {
                    self.act(ctx, Act::Delete);
                } else if self.tabs[self.active].ed.selection.words().is_some() {
                    let t = &mut self.tabs[self.active];
                    if let Some(said) = t.ed.delete_words(&t.pdf) {
                        t.info = Some(said);
                    }
                    t.apply_actions(ctx);
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
            Act::Extend(toward) => {
                let t = &mut self.tabs[self.active];
                t.ed.extend_by(&t.pdf, toward);
                t.reveal = t.ed.words_end();
            }
            // Setting the crop on the pages: Esc closes the Crop section, showing the pages
            // as cropped again.
            Act::Escape if self.tabs[self.active].crop_view() => self.tabs[self.active].end_crop(),
            // Pages dragged stay, pages cut are no longer, then the insertion bar goes, then
            // in Organise the pages selected are let go (Tab::escape_pages).
            Act::Escape => {
                let t = &mut self.tabs[self.active];
                if !t.escape_pages() {
                    t.ed.escape();
                }
            }
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
                // Shown as on, Facing pages turns off again, as View's button does.
                let now = self
                    .tabs
                    .get(self.active)
                    .map_or(self.settings.layout, |t| t.layout);
                let l = match (l, now) {
                    (Layout::Facing, Layout::Facing) => Layout::Continuous,
                    _ => l,
                };
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
            Act::Copy | Act::Cut | Act::Paste => {
                let t = &mut self.tabs[self.active];
                let clip = match a {
                    Act::Copy => Clip::Copy,
                    Act::Cut => Clip::Cut,
                    _ => Clip::Paste,
                };
                match (t.ed.typing_id(), clip) {
                    // The typing takes the keyboard back, and then the system copies or cuts
                    // what is selected in it, or pastes at its caret.
                    (Some(id), _) => {
                        t.ed.refocus_typing();
                        self.clip = Some((clip, id));
                    }
                    // Over words selected while reading: the system pastes, and what it
                    // pastes is typed in their place, as Ctrl+V types it (`App::type_over`).
                    (None, Clip::Paste) if t.ed.selection.words().is_some() => {
                        ctx.send_viewport_cmd(egui::ViewportCommand::RequestPaste);
                    }
                    // Pages cut move to where pages go: the insertion bar, else after the
                    // pages selected.
                    (None, Clip::Paste) => {
                        if let Some(said) = t.has_cut().then(|| t.paste_pages(ctx)).flatten() {
                            t.info = Some(said.into());
                        }
                    }
                    // Pages are cut to paste elsewhere; the system's clipboard gets their
                    // names, so that the paste keys work wherever the clipboard was empty.
                    (None, Clip::Cut) if t.ed.selection.pages().is_some() => {
                        let chosen = t.ed.selection.pages().cloned().unwrap_or_default();
                        let names: Vec<String> = chosen.iter().map(|&p| t.page_name(p)).collect();
                        let n = t.cut_selected_pages();
                        let what = match n {
                            1 => "1 page".to_string(),
                            n => format!("{n} pages"),
                        };
                        ctx.copy_text(format!("Pages {} of {}", names.join(", "), t.name()));
                        self.status = format!(
                            "Cut {what}: paste to put them where the insertion bar is, or after \
                             the pages selected."
                        );
                    }
                    (None, Clip::Copy) => match t.ed.selected_text() {
                        Some(text) => {
                            ctx.copy_text(text);
                            self.status = "Copied the selected text.".into();
                        }
                        None => self.act(ctx, Act::CopyObjectText),
                    },
                    // The words copied, then deleted as Delete deletes them; lines of page
                    // text copied, then deleted.
                    (None, Clip::Cut) => match t.ed.selected_text() {
                        Some(text) => {
                            ctx.copy_text(text);
                            self.status = match t.ed.delete_words(&t.pdf) {
                                None => "Cut the selected text.".into(),
                                // None of it could be taken out: copied only.
                                Some(said) => {
                                    t.info = Some(said);
                                    "Copied the selected text.".into()
                                }
                            };
                            t.apply_actions(ctx);
                        }
                        None => {
                            let lines = t.ed.selected_line_text();
                            if !lines.is_empty() {
                                self.status = match lines.len() {
                                    1 => "Cut the line.".into(),
                                    n => format!("Cut {n} lines."),
                                };
                                ctx.copy_text(lines.join("\n"));
                                t.ed.delete_selected();
                            }
                        }
                    },
                }
            }
            Act::CopyObjectText => {
                let lines = self.tabs[self.active].ed.selected_line_text();
                if !lines.is_empty() {
                    self.status = match lines.len() {
                        1 => "Copied the line's text.".into(),
                        n => format!("Copied the text of {n} lines."),
                    };
                    ctx.copy_text(lines.join("\n"));
                }
            }
            Act::Find => {
                self.show_view(Side::Search);
                self.tabs[self.active].focus_query = true;
            }
            // Organise's cards have a zoom of their own (NEW-14).
            Act::ZoomIn | Act::ZoomOut | Act::Actual | Act::ZoomTo(_)
                if has && self.tabs[self.active].organising() =>
            {
                let z = self.settings.organise_zoom;
                let z = match a {
                    Act::ZoomIn => z * 1.2,
                    Act::ZoomOut => z / 1.2,
                    Act::ZoomTo(percent) => percent as f32 / 100.0,
                    _ => crate::settings::ORGANISE_ZOOM,
                };
                self.tabs[self.active].zoom_grid(z);
                self.keep_grid_zoom();
            }
            Act::ZoomIn => {
                let t = &mut self.tabs[self.active];
                t.set_zoom(t.zoom * 1.2);
            }
            Act::ZoomOut => {
                let t = &mut self.tabs[self.active];
                t.set_zoom(t.zoom / 1.2);
            }
            Act::ZoomTo(percent) => self.tabs[self.active].set_zoom(percent as f32 / 100.0),
            Act::Actual => self.tabs[self.active].set_zoom(1.0),
            // On: the zoom follows the view's width (see Tab::show_pages); off: it stays.
            Act::FitWidth => {
                let t = &mut self.tabs[self.active];
                t.fit = !t.fit;
            }
            Act::Organise if has => {
                let t = &mut self.tabs[self.active];
                let on = !t.organising();
                if t.set_organise(ctx, on) && on {
                    self.band = Band::Pages;
                }
            }
            Act::SelectAllPages if has => self.tabs[self.active].select_all_pages(),
            Act::RotL | Act::RotR => {
                let deg = if matches!(a, Act::RotR) { 90 } else { 270 };
                let t = &mut self.tabs[self.active];
                let idx = t.targets();
                t.arrange(ctx, &idx, |p| p.rotate_pages(&idx, deg));
            }
            Act::Delete => {
                let t = &mut self.tabs[self.active];
                let idx = t.targets();
                let before = t.revision;
                t.arrange(ctx, &[], |p| p.delete_pages(&idx));
                if t.revision != before {
                    let n = idx.len();
                    let what = if n == 1 {
                        format!("page {}", idx[0] + 1)
                    } else {
                        format!("{n} pages")
                    };
                    self.notify(ctx, self.active, format!("Deleted {what}"), true);
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
            // At the page before's size, or the first page's at the start.
            Act::Blank => {
                let t = &self.tabs[self.active];
                let at = t.insert_at();
                let size = t.sizes.get(at.saturating_sub(1)).copied();
                self.insert_blank(ctx, at, size.unwrap_or((612.0, 792.0)));
            }
            Act::Merge => {
                if let Some(files) = Self::pdf_dialog().pick_files() {
                    self.merge_files(ctx, &files);
                }
            }
            Act::InsertPdf => {
                if let Some(files) = Self::pdf_dialog().pick_files() {
                    self.insert_files(ctx, &files);
                }
            }
            Act::MovePages(to) => self.tabs[self.active].move_pages(ctx, to),
            Act::InsertImages => {
                let Some(files) = Self::pick_images() else {
                    return;
                };
                let Some(data) = self.read_images(&files) else {
                    return;
                };
                let t = &mut self.tabs[self.active];
                // At the insertion bar, else after the selected pages or the current page.
                let (at, before) = (t.insert_at(), t.page_count());
                t.arrange(ctx, &[], |p| insert_images(p, at, &data));
                t.placed(at, t.page_count() - before);
            }
            Act::Scans => self.reveal(Section::Scans),
            Act::Ocr => self.reveal(Section::Ocr),
            Act::HeaderFooter => self.reveal(Section::HeaderFooter),
            Act::Watermark => self.reveal(Section::Watermark),
            // Crop again while the crop is being set ends it, as in Word.
            Act::Crop if self.tabs[self.active].crop_view() => self.tabs[self.active].end_crop(),
            Act::Crop => self.reveal(Section::Crop),
            Act::Props => self.reveal(Section::Properties),
            Act::Security => self.reveal(Section::Security),
            // The selected line, on the page; with none, the text tool, to click a line. On
            // (the Text tool, or text being edited on the page), back to the read tool, what
            // was typed kept.
            Act::EditObjectText => {
                let c = self.shown.unwrap_or_else(|| self.state(ctx));
                let t = &mut self.tabs[self.active];
                if band::is_on(Act::EditObjectText, &c) {
                    t.ed.set_tool(Tool::Read);
                } else if t.ed.selection.words().is_some() && t.ed.edit_words(&t.pdf) {
                    self.open_inspector();
                } else if t.ed.one_line_selected() {
                    self.open_inspector();
                    let t = &mut self.tabs[self.active];
                    t.ed.edit_selected_text(&t.pdf);
                } else {
                    t.ed.set_tool(Tool::Text);
                }
            }
            Act::MoveText => self.tabs[self.active].ed.move_text(),
            Act::EditParagraph => self.set_paragraphs(!self.settings.paragraphs),
            Act::JoinHyphen => self.tabs[self.active].ed.settle_hyphen(ctx, true, false),
            Act::KeepHyphen => self.tabs[self.active].ed.settle_hyphen(ctx, false, false),
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

    /// Open the inspector's page labels for the selected pages, or the current page.
    fn reveal_labels(&mut self) {
        self.reveal(Section::Labels);
        let t = &mut self.tabs[self.active];
        if t.ed.selection.pages().is_none() {
            t.ed.selection = Selection::pages_from([t.current].into());
        }
    }

    /// Use the desktop's accent colour, read again now, if the user follows it; else
    /// LimarePDF's teal. Read on a thread where the desktop can be slow to say
    /// ([`accent::asks_slowly`]), the result taken in as the window is drawn.
    fn read_accent(&mut self, ctx: &egui::Context) {
        if self.settings.accent == Accent::LimarePdf {
            self.accent_read = None;
            theme::set_accent(ctx, None);
            return;
        }
        if !accent::asks_slowly() {
            theme::set_accent(ctx, accent::desktop());
            return;
        }
        if self.accent_read.is_some() {
            return;
        }
        let (tx, rx) = channel();
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            // Gone when the app has gone; nothing to tell then.
            let _ = tx.send(accent::desktop());
            ctx.request_repaint();
        });
        self.accent_read = Some(rx);
    }

    /// Copy, Cut or Paste clicked in the band for the text being typed: once that text has
    /// the keyboard back, the system copies or cuts what is selected in it, or pastes at its
    /// caret. Dropped if the typing ends first.
    fn clipboard(&mut self, ctx: &egui::Context) {
        let Some((clip, id)) = self.clip else {
            return;
        };
        let typing = self.tabs.get(self.active).and_then(|t| t.ed.typing_id());
        if typing != Some(id) {
            self.clip = None;
            return;
        }
        if ctx.memory(|m| m.has_focus(id)) && !ctx.input(|i| i.pointer.any_down()) {
            ctx.send_viewport_cmd(match clip {
                Clip::Copy => egui::ViewportCommand::RequestCopy,
                Clip::Cut => egui::ViewportCommand::RequestCut,
                Clip::Paste => egui::ViewportCommand::RequestPaste,
            });
            self.clip = None;
        }
    }

    /// Show one of the navigation panel's views, opening the panel.
    fn show_view(&mut self, side: Side) {
        self.side.set_open(true);
        self.keep_panels();
        self.tabs[self.active].side = side;
    }

    /// Organise's zoom, as the active tab has it now, kept for next time.
    fn keep_grid_zoom(&mut self) {
        let Some(t) = self.tabs.get(self.active) else {
            return;
        };
        if t.grid_zoom != self.settings.organise_zoom {
            self.settings.organise_zoom = t.grid_zoom;
            self.settings.save();
        }
    }

    /// Open the inspector, as the user's choice.
    /// Paragraph editing on or off (Edit > Text flow's Paragraph, the inspector's Editing
    /// row, the menus and the command search; NEW-01), for every tab and remembered between
    /// runs; the text being edited in the tab in front follows ([`Editor::set_paragraph`]).
    fn set_paragraphs(&mut self, on: bool) {
        self.settings.paragraphs = on;
        self.settings.save();
        for t in &mut self.tabs {
            t.ed.paragraphs = on;
        }
        if let Some(t) = self.tabs.get_mut(self.active) {
            t.ed.set_paragraph(&t.pdf, on);
        }
    }

    fn open_inspector(&mut self) {
        self.inspector.set_open(true);
        self.keep_panels();
    }

    /// Moves the keyboard for this frame's F6 or Shift+F6 (ENH-10), and notes the control
    /// that has it as the last used in its region. Typing open on the page waits there for
    /// the keyboard to come back, and a form field's text box, which keeps its value as it
    /// is left, is noted to go back to ([`Parked`]).
    fn move_region(&mut self, ctx: &egui::Context) {
        let focused = ctx.memory(|m| m.focused());
        self.regions.note(ctx, focused);
        let step = std::mem::take(&mut self.regions.step);
        if step == 0 {
            return;
        }
        // Nothing with the keyboard: the keys go to the page.
        let typing = self.tabs.get(self.active).and_then(|t| t.ed.typing_id());
        let from = match focused {
            Some(id) if Some(id) != typing => self.regions.of_control(ctx, id),
            _ => None,
        }
        .unwrap_or(Region::Page);
        let open = !self.tabs.is_empty();
        let Some((to, stop)) = self.regions.next(ctx, from, step, open) else {
            return;
        };
        if let Some(t) = self
            .tabs
            .get_mut(self.active)
            .filter(|_| from == Region::Page)
        {
            t.ed.parked = match (focused, t.ed.form.typing_in()) {
                (Some(id), _) if Some(id) == typing => {
                    // What is selected there, given back with the keyboard.
                    page_text::held(ctx, id);
                    Some(Parked::Typing(id))
                }
                (_, Some((name, widget))) => Some(Parked::Field(name, widget)),
                _ => None,
            };
        }
        self.regions.ring = false;
        match stop {
            Some(id) => ctx.memory_mut(|m| m.request_focus(id)),
            None if to == Region::Page => self.keys_to_page(ctx, focused),
            None => {}
        }
    }

    /// The keyboard back to the page (F6): to the typing or form field left there, or, with
    /// neither, to the page itself, which takes the keys with nothing focused (as after a
    /// click on it); the page being read is outlined to show it, unless a field's ring does.
    fn keys_to_page(&mut self, ctx: &egui::Context, focused: Option<egui::Id>) {
        let Some(t) = self.tabs.get_mut(self.active) else {
            return;
        };
        match t.ed.parked.take() {
            Some(Parked::Typing(id)) => {
                page_text::give_back(ctx, id);
                // Its typing undone as before.
                page_text::hand_back(ctx, id);
            }
            Some(Parked::Field(name, widget)) => t.ed.form.focus_field(&name, widget),
            None => {
                if let Some(id) = focused {
                    ctx.memory_mut(|m| m.surrender_focus(id));
                }
                self.regions.ring = !t.ed.form.ringed();
            }
        }
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

    /// Remember the width of the panel `panel`, `width` points wide this frame, once the
    /// user has let go of its edge (`held`: being moved this frame; see
    /// [`panels::edge_held`]). A panel widened by what it holds keeps the width given.
    fn keep_width(&mut self, panel: &'static str, width: f32, held: bool) {
        let was = self.resizing.contains(&panel);
        if held && !was {
            self.resizing.push(panel);
        } else if !held && was {
            self.resizing.retain(|p| *p != panel);
            self.settings.keep_width(panel, width);
        }
    }

    /// Open section `s` of the inspector and scroll to it: a tool alone, as the task chosen
    /// (PF-40). The panel must show the page or document sections, so an object, comment or
    /// word selection is dropped and a tool that adds content gives way to the read tool;
    /// the document's properties also drop a page selection.
    fn reveal(&mut self, s: Section) {
        self.open_inspector();
        let t = &mut self.tabs[self.active];
        let ed = &mut t.ed;
        ed.selection.clear_object();
        ed.selection.clear_words();
        if ed.selection.annot().is_some() {
            ed.selection = Selection::None;
        }
        if ed.owns_panel() {
            ed.set_tool(Tool::Read);
        }
        if !s.is_task() {
            ed.selection = Selection::None;
        }
        t.forms.choose(s);
    }

    /// Offers the page text selected in the tab in front for X11 and Wayland's PRIMARY
    /// selection (pasted elsewhere with the middle button, as text selected in a text box
    /// is), once it settles: not while a button is held, and once for each new selection
    /// (ENH-34). Other platforms have no PRIMARY selection, and egui ignores it there.
    fn offer_selection(&mut self, ctx: &egui::Context) {
        let Some(t) = self.tabs.get(self.active) else {
            return;
        };
        let same = |(id, s): &(u64, Selection)| *id == t.id && *s == t.ed.selection;
        if self.offered.as_ref().is_some_and(same) || ctx.input(|i| i.pointer.any_down()) {
            return;
        }
        if let Some(text) = t.ed.selected_text().filter(|s| !s.is_empty()) {
            ctx.send_cmd(egui::OutputCommand::TextSelectionSettled(text));
        }
        self.offered = Some((t.id, t.ed.selection.clone()));
    }

    /// The front tab's toast, bottom centre above the status bar, until it times out, is
    /// dismissed, or Undo is pressed. The toasts of tabs behind wait, for a while.
    fn toast_ui(&mut self, ctx: &egui::Context) {
        let now = ctx.input(|i| i.time);
        let front = self.tabs.get(self.active).map(|t| t.id);
        let tabs = &self.tabs;
        self.toasts.retain_mut(|toast| {
            let Some(t) = tabs.iter().find(|t| t.id == toast.tab) else {
                return false;
            };
            // Something came after its change: another change, Undo, Redo, or typing.
            let later = |h| h != t.history() || t.ed.pending() || t.typing_in_forms();
            if toast.undo.is_some_and(later) {
                return false;
            }
            match (Some(t.id) == front, toast.away) {
                (true, None) => now < toast.until,
                (true, Some((since, left))) => {
                    toast.until = now + left.max(TOAST_BACK);
                    toast.away = None;
                    now - since < TOAST_AWAY
                }
                (false, None) => {
                    toast.away = Some((now, toast.until - now));
                    now < toast.until
                }
                (false, Some((since, _))) => now - since < TOAST_AWAY,
            }
        });
        let Some(toast) = self.toasts.iter().find(|t| Some(t.tab) == front) else {
            return;
        };
        ctx.request_repaint_after(std::time::Duration::from_secs_f64(toast.until - now));
        let (mut undo, mut close) = (false, false);
        egui::Area::new(egui::Id::unique("toast"))
            .anchor(egui::Align2::CENTER_BOTTOM, [0.0, -40.0])
            .order(egui::Order::Foreground)
            .show(ctx, |ui| {
                egui::Frame::popup(ui.style())
                    .inner_margin(egui::Margin::symmetric(12, 8))
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            ui.label(&toast.text);
                            ui.add_space(8.0);
                            if toast.undo.is_some() {
                                let b = egui::Button::new(egui::RichText::new("Undo").strong());
                                let tip = tooltip(ctx, actions::get(Act::Undo));
                                if ui.add(b).on_hover_text(tip).clicked() {
                                    (undo, close) = (true, true);
                                }
                            }
                            let x = egui::Button::new(theme::icon(icon::X));
                            let x = ui.add(x.frame_when_inactive(false));
                            x.widget_info(|| named("Dismiss"));
                            if x.on_hover_text("Dismiss").clicked() {
                                close = true;
                            }
                        });
                    });
            });
        if close {
            self.toasts.retain(|t| Some(t.tab) != front);
        }
        // At once, while its change is still the last thing in the tab, as checked above.
        if undo {
            self.run_command(ctx, actions::name(Act::Undo), |app, ctx| {
                app.act(ctx, Act::Undo)
            });
        }
    }

    /// Show a toast about something that just happened in tab `i`, replacing the tab's
    /// earlier one; `undo` for a change Undo takes back.
    fn notify(&mut self, ctx: &egui::Context, i: usize, text: String, undo: bool) {
        let t = &self.tabs[i];
        let toast = Toast {
            tab: t.id,
            text,
            undo: undo.then(|| t.history()),
            until: ctx.input(|i| i.time) + TOAST_SECONDS,
            away: None,
        };
        self.toasts.retain(|x| x.tab != toast.tab);
        self.toasts.push(toast);
    }

    /// Insert a blank page of `size` points as page `at` (0-based) and show it.
    fn insert_blank(&mut self, ctx: &egui::Context, at: usize, size: (f32, f32)) {
        let t = &mut self.tabs[self.active];
        let before = t.page_count();
        t.arrange(ctx, &[], |p| p.insert_blank(at, size.0, size.1));
        t.placed(at, t.page_count() - before);
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

    /// Adds the pages of `files` to the end of the active tab, reading them in the
    /// background; a file that needs a password asks for it (BUG-20).
    fn merge_files(&mut self, ctx: &egui::Context, files: &[PathBuf]) {
        let a = Appending {
            tab: self.tabs[self.active].id,
            to: PagesGo::End,
            files: files.to_vec(),
            read: Vec::new(),
        };
        self.read_appending(ctx, Box::new(a), None);
    }

    /// Inserts the pages of `files` in the active tab where pages go
    /// ([`Tab::insert_at`]), reading them in the background as [`App::merge_files`] does.
    fn insert_files(&mut self, ctx: &egui::Context, files: &[PathBuf]) {
        let t = &self.tabs[self.active];
        let at = t.insert_at();
        let a = Appending {
            tab: t.id,
            to: PagesGo::After(at.checked_sub(1).and_then(|p| t.page_id(p))),
            files: files.to_vec(),
            read: Vec::new(),
        };
        self.read_appending(ctx, Box::new(a), None);
    }

    /// Reads the files of `a` in the background from the first not yet read, that one with
    /// `password` if given, as far as one that needs a password or can't be read.
    fn read_appending(
        &mut self,
        ctx: &egui::Context,
        mut a: Box<Appending>,
        mut password: Option<String>,
    ) {
        let what = match a.to {
            PagesGo::End => "Append PDFs…",
            PagesGo::After(_) => "Insert PDF pages…",
        };
        let (tx, ctx) = (self.tx.clone(), ctx.clone());
        std::thread::spawn(move || {
            let mut stop = None;
            while let Some(f) = a.files.get(a.read.len()).cloned() {
                let given = password.take();
                match crate::guard(|| Pdf::open(&f, given.as_deref())) {
                    Ok(d) => a.read.push(Some(d)),
                    Err(e) => {
                        let s = format!("{e:#}");
                        stop = Some(if needs_password(&s) {
                            AppendStop::Password(given.map(|_| INCORRECT_PASSWORD.into()))
                        } else {
                            AppendStop::Failed(format!("{}: {s}", f.display()))
                        });
                        break;
                    }
                }
            }
            let _ = tx.send(Loaded::Append(a, stop));
            ctx.request_repaint();
        });
        self.busy = true;
        self.status = what.into();
    }

    /// Adds the pages of the files read for `a` to its tab, where they go, in one step. Pages
    /// from a password-protected document make the tab protected: its recovery copies are
    /// withdrawn first, as for a password, and it gets no more. Files with form fields are
    /// asked about first; with an answer LimarePDF can't draw, nothing is added.
    fn append_read(&mut self, ctx: &egui::Context, a: Appending) {
        let Some(i) = self.tabs.iter().position(|t| t.id == a.tab) else {
            self.show(Dialog::Message(
                "Pages not added".into(),
                "The document was closed while the files were read, so nothing was added.".into(),
            ));
            return;
        };
        let (mut docs, mut names) = (Vec::new(), Vec::new());
        for (f, d) in a.files.iter().zip(a.read) {
            let Some(d) = d else { continue };
            if !d.form_fields().is_empty() {
                let name = f.file_name().unwrap_or(f.as_os_str());
                let name = name.to_string_lossy().into_owned();
                // An answer that couldn't be drawn would be lost: nothing is added.
                let lost = d.undrawable_fields();
                if !lost.is_empty() {
                    let (what, them) = match lost.len() {
                        1 => ("the answer of the field", "it"),
                        _ => ("the answers of these fields:", "them"),
                    };
                    self.show(Dialog::Message(
                        "Pages not added".into(),
                        format!(
                            "LimarePDF can't draw {what} {} in “{name}”, so adding its pages \
                             would lose {them}. Nothing was added.",
                            limarepdf_core::quoted_list(&lost)
                        ),
                    ));
                    return;
                }
                names.push(name);
            }
            docs.push(d);
        }
        if docs.is_empty() {
            return;
        }
        if !names.is_empty() {
            let asked = Asked::of(&self.tabs[i]);
            self.show(Dialog::ConfirmAppendForms(Box::new(AppendForms {
                asked,
                to: a.to,
                docs,
                names,
            })));
            return;
        }
        self.add_pages(ctx, i, docs, a.to);
    }

    /// Adds the pages of `docs` to tab `i`'s document where they go, in one step: after the
    /// page named, first, or at the end (also when that page has gone since).
    fn add_pages(&mut self, ctx: &egui::Context, i: usize, docs: Vec<Pdf>, to: PagesGo) {
        let protected = docs.iter().any(Pdf::encrypted);
        if protected {
            let done = "added the pages of the password-protected document";
            if let Err(stop) = self.withdraw_copies(i, done) {
                self.show(stop);
                return;
            }
        }
        let t = &mut self.tabs[i];
        let n = t.page_count();
        let at = match to {
            PagesGo::End => n,
            PagesGo::After(None) => 0,
            PagesGo::After(Some(id)) => t.page_of(id).map_or(n, |p| p + 1),
        };
        if t.arrange(ctx, &[], |p| p.insert_pages(at, docs)) {
            t.copy.protected |= protected;
            if to != PagesGo::End {
                t.placed(at, t.page_count() - n);
            }
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
                self.insert_blank(ctx, after + 1, size);
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
                self.notify(ctx, active, "Changed password security".into(), true);
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
                let files: Vec<PathBuf> = image_names(&e.pages, &e.stem, e.format)
                    .iter()
                    .map(|n| e.dir.join(n))
                    .collect();
                match limarepdf_core::existing(&files).len() {
                    0 => self.start_export(ctx, e),
                    n => {
                        let asked = Asked::of(&self.tabs[active]);
                        self.dialog = Dialog::ConfirmReplace(asked, Replacing::Images(e), n);
                    }
                }
            }
            Apply::Split => {
                let f = &self.tabs[active].forms;
                let by = if f.split_at_marks {
                    SplitBy::Bookmarks(f.split_level.max(1))
                } else {
                    SplitBy::Every(f.split.max(1))
                };
                let Some(dir) = rfd::FileDialog::new()
                    .set_title("Choose output folder")
                    .pick_folder()
                else {
                    return;
                };
                let stem = self.tabs[active].stem("part");
                self.split_into(ctx, SplitFiles { by, dir, stem });
            }
        }
    }

    /// Split the document in front into files, asking first when any of them would replace
    /// a file, so that nothing is replaced unasked and nothing is written on Cancel.
    fn split_into(&mut self, ctx: &egui::Context, s: SplitFiles) {
        let t = &self.tabs[self.active];
        let name = |n, title: Option<&str>| s.dir.join(part_file_name(&s.stem, n, title));
        let files = match t.pdf.split_files(s.by, name) {
            Ok(files) => files,
            Err(e) => {
                self.dialog = Dialog::Message(SPLIT_JOB.into(), format!("{e:#}"));
                return;
            }
        };
        match limarepdf_core::existing(&files).len() {
            0 => self.start_split(ctx, s),
            n => self.dialog = Dialog::ConfirmReplace(Asked::of(t), Replacing::Split(s), n),
        }
    }

    /// Write the split's files on a background thread; the document is not changed.
    fn start_split(&mut self, ctx: &egui::Context, s: SplitFiles) {
        self.spawn_job(ctx, SPLIT_JOB, move |p| {
            let name = |n, title: Option<&str>| s.dir.join(part_file_name(&s.stem, n, title));
            match p.split(s.by, name) {
                Ok(n) => Ok((false, format!("Wrote {n} files to {}.", s.dir.display()))),
                Err(e) if not_confirmed(&e) => Ok((
                    false,
                    format!(
                        "Wrote the files to {}, but your system couldn't confirm they \
                         are safely on disk. If the computer lost power just now, \
                         split the document again.",
                        s.dir.display()
                    ),
                )),
                Err(e) => Err(e),
            }
        });
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

    /// Quit, asking first when there is unsaved work; a save being written ends first.
    fn quit(&mut self, ctx: &egui::Context) {
        if !self.saves.is_empty() {
            self.wait_for_saves(AfterSave::Quit);
        } else if let Some(q) = self.quit_question() {
            self.dialog = q;
        } else {
            self.quit_ok = true;
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
    }

    /// Close tab `i`, asking first when it has unsaved work; once its save ends, when one
    /// is being written.
    fn close_asking(&mut self, i: usize) {
        if self.saving(self.tabs[i].id) {
            self.wait_for_saves(AfterSave::Close(self.tabs[i].id));
        } else if self.tabs[i].unsaved() {
            self.dialog = Dialog::ConfirmClose(Asked::of(&self.tabs[i]), false);
        } else {
            self.close_tab(i);
        }
    }

    /// The question quitting asks when there is unsaved work: about the one document, or,
    /// with several, whether to go through them.
    fn quit_question(&self) -> Option<Dialog> {
        let mut unsaved = self.tabs.iter().filter(|t| t.unsaved());
        let first = unsaved.next()?;
        Some(match unsaved.next() {
            None => Dialog::ConfirmClose(Asked::of(first), true),
            Some(_) => Dialog::ConfirmQuit,
        })
    }

    /// Quitting goes on from an answer to its question: the documents still to ask about.
    /// A question waiting about a save is dropped, since quitting asks about each unsaved
    /// document anyway, and so is a tab's closing it was for.
    fn go_on_quitting(&mut self) -> VecDeque<u64> {
        self.waiting.retain(|d| !d.saving());
        match self.closing.take() {
            Some(Closing::Quit { rest, .. }) => rest,
            _ => VecDeque::new(),
        }
    }

    /// Goes on closing a tab or quitting once no question is open (see [`Closing`]): the
    /// saved tab closes, the next document is asked about, or the app quits. A document
    /// still unsaved stops it.
    fn close_step(&mut self, ctx: &egui::Context) {
        let unsaved = |app: &Self, id| app.tabs.iter().any(|t| t.id == id && t.unsaved());
        match self.closing.take() {
            None => {}
            // Its save is being written: wait for it.
            Some(c) if c.saved().is_some_and(|tab| self.saving(tab)) => self.closing = Some(c),
            Some(Closing::Tab { tab, front }) => {
                if unsaved(self, tab) {
                    return;
                }
                if let Some(i) = self.tabs.iter().position(|t| t.id == tab) {
                    self.close_tab(i);
                }
                if let Some(i) = self.tabs.iter().position(|t| Some(t.id) == front) {
                    self.active = i;
                }
            }
            Some(Closing::Quit { saving, mut rest }) => {
                if saving.is_some_and(|id| unsaved(self, id)) {
                    return;
                }
                while let Some(id) = rest.pop_front() {
                    if let Some(t) = self.tabs.iter().find(|t| t.id == id && t.unsaved()) {
                        self.dialog = Dialog::ConfirmClose(Asked::of(t), true);
                        self.closing = Some(Closing::Quit { saving: None, rest });
                        return;
                    }
                }
                // Another tab's save is still being written.
                if !self.saves.is_empty() {
                    self.closing = Some(Closing::Quit { saving: None, rest });
                    return;
                }
                self.quit_ok = true;
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
        }
    }

    /// Close tab `i`. The tab in front stays there; closing it brings the next one, or the
    /// one before when it was the last (BUG-09).
    fn close_tab(&mut self, i: usize) {
        if i < self.tabs.len() {
            let tab = self.tabs.remove(i);
            if i < self.active {
                self.active -= 1;
            }
            self.active = self.active.min(self.tabs.len().saturating_sub(1));
            self.closed_copy(&tab);
            // Dropped last (stopping its render worker), with the app's state already valid.
            drop(tab);
        }
    }

    fn dialogs(&mut self, ctx: &egui::Context) {
        self.next_dialog();
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
            Dialog::ConfirmClose(asked, quitting) => {
                let quitting = *quitting;
                // A document with no file yet (new, or recovered) is saved as one.
                let tab = self.tabs.iter().find(|t| t.id == asked.tab);
                let save = if tab.is_some_and(|t| t.path.is_none()) {
                    "Save as…"
                } else {
                    "Save"
                };
                let text = if quitting {
                    "This document has changes that aren't saved. Save them before quitting, \
                     or discard them."
                } else {
                    "This document has changes that aren't saved. Save them before closing \
                     it, or discard them."
                };
                match dlg::choose(
                    ctx,
                    Shell::new("Save changes?").file(file),
                    text,
                    Btn::primary(save),
                    Btn::danger("Discard"),
                    true,
                ) {
                    Choice::First => {
                        let front = self.tabs.get(self.active).map(|t| t.id);
                        let rest = quitting.then(|| self.go_on_quitting());
                        // The tab saved comes to the front for its save's questions.
                        if self.still(asked) {
                            self.closing = Some(match rest {
                                Some(rest) => Closing::Quit {
                                    saving: Some(asked.tab),
                                    rest,
                                },
                                None => Closing::Tab {
                                    tab: asked.tab,
                                    front,
                                },
                            });
                            self.save(ctx, false);
                        }
                        done = true;
                    }
                    Choice::Second if quitting => {
                        let rest = self.go_on_quitting();
                        if self.still(asked) {
                            self.closing = Some(Closing::Quit { saving: None, rest });
                        }
                        done = true;
                    }
                    Choice::Second => {
                        // A tab asked about from behind another closes there.
                        let front = self.tabs.get(self.active).map(|t| t.id);
                        if self.still(asked) {
                            self.close_tab(self.active);
                            if let Some(i) = self.tabs.iter().position(|t| Some(t.id) == front) {
                                self.active = i;
                            }
                        }
                        done = true;
                    }
                    Choice::Cancel => {
                        if quitting {
                            self.closing = None;
                        }
                        done = true;
                    }
                    Choice::Open => {}
                }
            }
            Dialog::ConfirmQuit => {
                let n = self.tabs.iter().filter(|t| t.unsaved()).count();
                match dlg::choose(
                    ctx,
                    Shell::new("Quit with unsaved changes?"),
                    &format!(
                        "{} changes that aren't saved. Review them to save or discard each \
                         one's changes, or discard all the changes and quit.",
                        plural(n, "A document has", "{n} documents have")
                    ),
                    Btn::primary("Review…"),
                    Btn::danger("Discard all"),
                    true,
                ) {
                    Choice::First => {
                        self.go_on_quitting();
                        let rest = self.tabs.iter().filter(|t| t.unsaved()).map(|t| t.id);
                        self.closing = Some(Closing::Quit {
                            saving: None,
                            rest: rest.collect(),
                        });
                        done = true;
                    }
                    Choice::Second => {
                        self.quit_ok = true;
                        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                        done = true;
                    }
                    Choice::Cancel => {
                        if matches!(self.closing, Some(Closing::Quit { .. })) {
                            self.closing = None;
                        }
                        done = true;
                    }
                    Choice::Open => {}
                }
            }
            Dialog::Password(s, unlock) => {
                let appending = matches!(unlock, Unlock::Append(_));
                let answer = dlg::password(ctx, s, appending);
                if matches!(answer, Choice::First | Choice::Second) {
                    let path = s.path.clone();
                    let password = (answer == Choice::First).then(|| s.input.clone());
                    match std::mem::replace(unlock, Unlock::Tab(None, None)) {
                        Unlock::Tab(page, None) => {
                            self.spawn_open(ctx, path.clone(), password, page)
                        }
                        Unlock::Tab(page, into) => {
                            self.read_file(ctx, path.clone(), password, page, into)
                        }
                        Unlock::Append(mut a) => {
                            // Skip this file: on to the next.
                            if password.is_none() {
                                a.read.push(None);
                            }
                            self.read_appending(ctx, a, password);
                        }
                    }
                    // Until it is read, the dialogs waiting wait: a wrong password is
                    // asked for again first.
                    if appending || self.opening.contains(&path) {
                        self.retrying = Some(path);
                    }
                }
                done = answer != Choice::Open;
            }
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
            Dialog::ConfirmReplace(asked, r, n) => match dlg::confirm(
                ctx,
                Ask::new(
                    "Replace files?",
                    &format!(
                        "{} in {} already {} the names {} would get. Replacing {} can't be \
                         undone.",
                        plural(*n, "A file", "{n} files"),
                        r.dir().display(),
                        if *n == 1 { "has" } else { "have" },
                        match r {
                            Replacing::Images(_) => "these images",
                            Replacing::Split(_) => "the split's files",
                        },
                        if *n == 1 { "it" } else { "them" },
                    ),
                    Btn::danger("Replace"),
                )
                .file(file),
            ) {
                Outcome::Apply => {
                    if self.still(asked) {
                        match r.clone() {
                            Replacing::Images(e) => self.start_export(ctx, e),
                            Replacing::Split(s) => self.start_split(ctx, s),
                        }
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
                     they look now, and stop being comments, here and in other readers. Their \
                     comment text is dropped. Links and form fields stay. Undo \
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
                            "The item you put down with Fill and sign is",
                            "The {n} items you put down with Fill and sign are"
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
            // Three answers and Cancel don't fit a narrow window: Reload is the banner's.
            Dialog::ConfirmStale(asked, now) => match dlg::choose(
                ctx,
                Shell::new("Replace the changed file?").file(file),
                "Another app or tab changed this file after LimarePDF opened or last saved it. \
                 Replace puts this document in its place, and those changes are lost. Save as \
                 a new file to keep both, or cancel and choose Reload above the pages to see \
                 the file as it now is.",
                Btn::primary("Save as…"),
                Btn::danger("Replace"),
                true,
            ) {
                Choice::First => {
                    if self.still(asked) {
                        self.save(ctx, true);
                    }
                    done = true;
                }
                Choice::Second => {
                    if self.still(asked) {
                        self.replace_ok = Some((asked.tab, (**now).clone()));
                        self.save(ctx, false);
                    }
                    done = true;
                }
                Choice::Cancel => done = true,
                Choice::Open => {}
            },
            Dialog::ConfirmReload(asked) => match dlg::confirm(
                ctx,
                Ask::new(
                    "Reload without your changes?",
                    "This document has changes that aren't saved. Reloading shows the file as \
                     it now is, and loses them.",
                    Btn::danger("Discard and reload"),
                )
                .file(file),
            ) {
                Outcome::Apply => {
                    if self.still(asked) {
                        self.reload(ctx, self.active);
                    }
                    done = true;
                }
                Outcome::Cancel => done = true,
                Outcome::Open => {}
            },
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
                // Try again.
                Outcome::Apply if !s.ready() => {
                    s.search = dlg::Search::Running(self.find_printers(ctx));
                }
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
            Dialog::ConfirmAppendForms(q) => {
                let (has, their) = match q.names.as_slice() {
                    [one] => (format!("“{one}” has form fields."), "its"),
                    _ => (
                        format!(
                            "These files have form fields: {}.",
                            limarepdf_core::quoted_list(&q.names)
                        ),
                        "their",
                    ),
                };
                match dlg::confirm(
                    ctx,
                    Ask::new(
                        "Add pages with form fields?",
                        &format!(
                            "{has} LimarePDF can't combine forms yet, so the fields are drawn \
                             onto {their} pages with their answers, as they look now, and can \
                             no longer be filled in. This document's own fields stay as they \
                             are. Undo removes the added pages."
                        ),
                        Btn::primary("Add pages"),
                    )
                    .file(file),
                ) {
                    Outcome::Apply => {
                        if self.still(&q.asked) {
                            let mut docs = std::mem::take(&mut q.docs);
                            let drawn = docs
                                .iter_mut()
                                .filter(|d| !d.form_fields().is_empty())
                                .try_for_each(|d| d.flatten_form(None).map(|_| ()));
                            match drawn {
                                Ok(()) => self.add_pages(ctx, self.active, docs, q.to),
                                Err(e) => self.show(Dialog::Message(
                                    "Pages not added".into(),
                                    format!("{e:#}\n\nNothing was added."),
                                )),
                            }
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
        // Not while a question about a save waits behind the quit question, which was
        // cancelled: that save goes on first.
        if matches!(self.dialog, Dialog::None) && !self.waiting.front().is_some_and(Dialog::saving)
        {
            self.close_step(ctx);
        }
        // What waited for a save goes on, one at a time, as each may ask something.
        if matches!(self.dialog, Dialog::None) && self.waiting.is_empty() {
            self.after_save(ctx);
        }
        self.next_dialog();
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
            docs: self.tabs.len(),
            busy: self.busy || self.sign_window.as_ref().is_some_and(SignWindow::busy),
            saving: self
                .tabs
                .get(self.active)
                .is_some_and(|t| self.saving(t.id)),
            side: self.side.shown(),
            inspector: self.inspector.shown(),
            docked: self.settings.controls_docked,
            theme: ctx.options(|o| o.theme_preference),
            page_look: self.settings.page_look,
            pictures_too: self.settings.pictures_too,
            recovery: self.settings.recovery,
            page_labels: self.settings.page_labels,
            planned: self.settings.show_planned,
            compact: self.settings.compact_commands,
            accent: self.settings.accent,
            layout: self.settings.layout,
            cover: self.settings.cover,
            highlight: self.settings.highlight,
            line: self.settings.line,
            ..Default::default()
        };
        if let Some(t) = self.tabs.get(self.active) {
            t.fill_ctx(&mut c);
            // In a text box with the keyboard, Undo and Redo are its typing's, as the keys;
            // on a paragraph's frame's handle, the paragraph's.
            let focused = ctx.memory(|m| m.focused());
            let in_box = ctx.text_edit_focused() || t.ed.edge_held(ctx);
            if let Some(id) = focused.filter(|_| in_box) {
                let history = t.ed.typing_history(id);
                if let Some((undo, redo)) = history.or_else(|| page_text::box_history(ctx, id)) {
                    c.can_undo = undo;
                    c.can_redo = redo;
                }
            }
        }
        c
    }

    /// The File menu, built from the registry: File's commands, with Open recent from
    /// `recent` (the recent files that still exist), then a submenu for each other group,
    /// so that every command has a place in it.
    fn menu(ui: &mut egui::Ui, c: &Ctx, recent: &[&PathBuf], acts: &mut Vec<Act>) {
        Self::menu_group(ui, Group::File, c, recent, acts);
        ui.separator();
        for g in Group::MENUS.into_iter().filter(|&g| g != Group::File) {
            ui.menu_button(g.name(), |ui| Self::menu_group(ui, g, c, recent, acts));
        }
    }

    /// A group's commands in the menu, a run with the same submenu as one submenu; File's
    /// with Open recent, and the planned ones where they will be.
    fn menu_group(ui: &mut egui::Ui, g: Group, c: &Ctx, recent: &[&PathBuf], acts: &mut Vec<Act>) {
        let items: Vec<&Action> = ACTIONS.iter().filter(|a| a.group == g && a.menu).collect();
        let mut i = 0;
        while i < items.len() {
            let a = items[i];
            if a.sep && i > 0 {
                ui.separator();
            }
            if a.submenu.is_empty() {
                menu_item(ui, a, c, acts);
                let planned = match a.act {
                    Act::Open => {
                        recent_menu(ui, recent, acts);
                        None
                    }
                    Act::SaveAs => Some(band::FILE_PLANNED[1]),
                    Act::Print => Some(band::FILE_PLANNED[0]),
                    _ => None,
                };
                if let Some(p) = planned.filter(|_| c.planned) {
                    band::planned_menu_item(ui, p);
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
        let modal = egui::Modal::new(egui::Id::unique("palette"))
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
            if let Some(id) = ctx.data(|d| d.get_temp(egui::Id::unique(POPOUT_OPENER))) {
                ctx.memory_mut(|m| m.request_focus(id));
            }
            return;
        }
        // Esc with a list, menu or colour picker open only closes it (a note being typed
        // stays open while its colour is picked); Esc leaving a box for typing on the page
        // only goes back to it.
        let picking = egui::Popup::is_any_open(ctx);
        let back = self.esc_to_typing;
        // Organise's grid and the thumbnails take their keys first (the arrows, Home and
        // End, Page Up and Down, Space and Enter); with them holding the keyboard, the
        // commands' plain keys (Delete, Esc, Alt+Shift and an arrow) work as with nothing.
        let lists = self
            .tabs
            .get(self.active)
            .is_some_and(|t| t.holds_keys(ctx));
        if !picking {
            if let Some(t) = self.tabs.get_mut(self.active) {
                t.list_keys(ctx);
            }
        }
        let c = self.state(ctx);
        // Undo and Redo in a text box take back and redo the typing there, and nothing
        // else, even with nothing left to undo there (form fields, comments and the
        // inspector's boxes too); Undo takes back the whole edit once it is kept. A
        // paragraph's frame's handle with the keyboard is part of its typing, so they are
        // the paragraph's there too. With a button holding the keyboard, they are the
        // document's.
        let typing = ctx.egui_wants_keyboard_input() && !lists;
        let on_edge = (self.tabs.get(self.active)).is_some_and(|t| t.ed.edge_held(ctx));
        let in_text = ctx.text_edit_focused() || on_edge;
        let mut acts = Vec::new();
        // Plain keys a command took with Shift (Shift+Home extending the words selected):
        // with Shift, they aren't the command without it too (First page).
        let mut shifted = Vec::new();
        ctx.input_mut(|i| {
            for (a, k) in actions::shortcuts() {
                // Commands for what is selected on the page, which a text box with the
                // keyboard takes for its own text.
                let page_only = matches!(
                    a.act,
                    Act::Copy
                        | Act::Cut
                        | Act::SelectAllText
                        | Act::SelectAllObjects
                        | Act::Bold
                        | Act::Italic
                );
                let global = (k.modifiers.command || k.modifiers.ctrl) && !page_only;
                let in_box = in_text && matches!(a.act, Act::Undo | Act::Redo);
                let closes = (picking || back) && a.act == Act::Escape;
                // F6 leaves a text box too (in a form, Tab stays among the fields), but
                // not an open list or menu, which Esc closes first.
                let region = matches!(a.act, Act::NextRegion | Act::PrevRegion);
                let skip = if region { picking } else { typing && !global };
                // Ctrl+V arrives as a paste, which App::type_over takes.
                if skip || in_box || closes || a.act == Act::Paste || !(a.enabled)(&c) {
                    continue;
                }
                let hit = if a.act == Act::Copy {
                    // Ctrl+C arrives as a Copy event, not as a key press; Ctrl+X as a Cut.
                    i.events.iter().any(|e| matches!(e, egui::Event::Copy))
                } else if a.act == Act::Cut {
                    i.events.iter().any(|e| matches!(e, egui::Event::Cut))
                } else if global {
                    i.consume_shortcut(&k)
                } else {
                    // The key's own modifiers, not those held at the end of the frame: a
                    // quick Ctrl+A can arrive with Ctrl already let go. Shift tells F6's
                    // two directions apart; Alt with Shift moves pages, so Alt is never
                    // extra.
                    let shadowed = !k.modifiers.shift && shifted.contains(&k.logical_key);
                    i.events.iter().any(|e| {
                        matches!(e, egui::Event::Key { key, pressed: true, modifiers, .. }
                            if *key == k.logical_key && modifiers.matches_logically(k.modifiers)
                                && modifiers.alt == k.modifiers.alt
                                && (!region || modifiers.shift == k.modifiers.shift)
                                && !(shadowed && modifiers.shift))
                    })
                };
                if hit {
                    if k.modifiers.shift && !global {
                        shifted.push(k.logical_key);
                    }
                    acts.push(a.act);
                }
            }
        });
        for a in acts {
            self.run_command(ctx, actions::name(a), |app, ctx| app.act(ctx, a));
        }
    }

    /// Text typed or pasted with no text box having the keyboard goes over the words
    /// selected in the tab in front, as a word processor types over a selection, or into a
    /// line just opened on its page ([`crate::editor::Editor::type_over`]).
    fn type_over(&mut self, ctx: &egui::Context) {
        if self.palette.is_some() || dlg::any_open(ctx) {
            return;
        }
        let Some(t) = self.tabs.get_mut(self.active) else {
            return;
        };
        if ctx.egui_wants_keyboard_input() && !t.holds_keys(ctx) {
            return;
        }
        // An input method's too: with words selected it is on ([`Editor::type_over`]).
        let typed: Vec<egui::Event> = ctx.input(|i| {
            (i.events.iter())
                .filter(|e| {
                    matches!(
                        e,
                        egui::Event::Text(_)
                            | egui::Event::Paste(_)
                            | egui::Event::Ime(
                                egui::ImeEvent::Preedit { .. } | egui::ImeEvent::Commit(_)
                            )
                    )
                })
                .cloned()
                .collect()
        });
        let paste = typed.iter().any(|e| matches!(e, egui::Event::Paste(_)));
        // Pages cut go where pages are selected, or in Organise where the bar is; with
        // nothing selected or typed, after the current page, as the band's Paste puts them.
        let none = matches!(t.ed.selection, Selection::None) && t.ed.typing_id().is_none();
        let pages = t.organising() || t.ed.selection.pages().is_some() || none;
        if paste && pages && t.has_cut() {
            self.act(ctx, Act::Paste);
        } else if !typed.is_empty() && t.ed.type_over(ctx, &t.pdf, typed) {
            ctx.request_repaint();
        } else if paste && t.organising() {
            t.info = Some("To move pages, cut them, then paste them where they go".into());
        } else if paste && t.ed.typing_id().is_none() {
            // Nowhere to paste: where to click.
            t.info = Some(
                "To paste, click in text being typed, or choose Add text and click where the \
                 text goes"
                    .into(),
            );
        }
    }
}

/// A thin upright line between groups of buttons in a row.
fn divider(ui: &mut egui::Ui) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(11.0, 20.0), egui::Sense::hover());
    let stroke = egui::Stroke::new(1.0, theme::border(ui.visuals().dark_mode));
    ui.painter().vline(rect.center().x, rect.y_range(), stroke);
}

/// The most a document tab takes; a longer name is cut short with "…".
const TAB_MOST: f32 = 240.0;
/// The least a document tab shrinks to as more open, before the tabs scroll instead: room
/// for its icon, about ten characters of its name and its close button.
const TAB_LEAST: f32 = 160.0;
/// The least room a tab's name keeps beside the word "Unsaved", about ten characters; a tab
/// with less shows a dot for the word.
const NAME_LEAST: f32 = 70.0;
/// A document tab's height, and the room between two tabs.
const TAB_HEIGHT: f32 = 34.0;
const TAB_GAP: f32 = 6.0;
/// A document tab's margins at its left and right, the room between its parts, and its close
/// button's size.
const TAB_LEFT: f32 = 12.0;
const TAB_RIGHT: f32 = 6.0;
const TAB_SPACE: f32 = 8.0;
const TAB_CLOSE: f32 = 22.0;
/// The size of each of the arrows and the documents button at the tab strip's right.
const STRIP_BUTTON: f32 = 26.0;
/// An arrow held down steps the tabs again after this many seconds, then every
/// [`HOLD_EVERY`], as a scroll bar's arrows do.
const HOLD_FIRST: f64 = 0.4;
const HOLD_EVERY: f64 = 0.12;
/// The most a name takes in the menu of open documents.
const MENU_NAME_MOST: f32 = 360.0;

/// The parts every document tab has, laid out once a frame: its icon, the word "Unsaved" and
/// the dot standing for it on a tab too narrow for the word, and the names' font.
struct TabParts {
    icon: Arc<egui::Galley>,
    unsaved: Arc<egui::Galley>,
    dot: Arc<egui::Galley>,
    font: egui::FontId,
}

impl TabParts {
    fn new(ui: &egui::Ui) -> Self {
        let lay = |text: &str, font| {
            let p = ui.painter();
            p.layout_no_wrap(text.into(), font, egui::Color32::PLACEHOLDER)
        };
        Self {
            icon: lay(icon::FILE_TEXT, egui::FontId::new(16.0, theme::icons())),
            unsaved: lay("Unsaved", egui::FontId::proportional(11.0)),
            dot: lay(icon::CIRCLE, egui::FontId::new(8.0, theme::icons_fill())),
            font: egui::TextStyle::Button.resolve(ui.style()),
        }
    }

    /// What a tab takes besides its name and its unsaved mark.
    fn fixed(&self) -> f32 {
        TAB_LEFT + self.icon.size().x + TAB_SPACE + TAB_SPACE + TAB_CLOSE + TAB_RIGHT
    }

    /// The room the word "Unsaved" takes after a name, and the dot standing for it.
    fn word(&self) -> f32 {
        TAB_SPACE + self.unsaved.size().x
    }

    fn dot(&self) -> f32 {
        TAB_GAP + self.dot.size().x
    }

    /// The width of a tab showing all of `name`, up to [`TAB_MOST`].
    fn own_width(&self, ui: &egui::Ui, name: &str, dirty: bool) -> f32 {
        let mark = if dirty { self.word() } else { 0.0 };
        let name = text_width(ui, name, &self.font);
        (self.fixed() + name + mark).ceil().min(TAB_MOST)
    }
}

/// The width of `text` in `font`, on one line.
fn text_width(ui: &egui::Ui, text: &str, font: &egui::FontId) -> f32 {
    let p = ui.painter();
    let g = p.layout_no_wrap(text.into(), font.clone(), egui::Color32::PLACEHOLDER);
    g.size().x
}

/// `text` as it fits `room` points in `font`: as it is, or its start cut short with "…".
fn shortened(ui: &egui::Ui, text: &str, font: &egui::FontId, room: f32) -> String {
    if text_width(ui, text, font) <= room {
        return text.to_string();
    }
    let cut = |kept: usize| {
        let end = text.char_indices().nth(kept).map_or(text.len(), |(i, _)| i);
        format!("{}…", text[..end].trim_end())
    };
    // The most characters that fit with "…" after them.
    let (mut lo, mut hi) = (0, text.chars().count());
    while lo < hi {
        let mid = (lo + hi).div_ceil(2);
        if text_width(ui, &cut(mid), font) <= room {
            lo = mid;
        } else {
            hi = mid - 1;
        }
    }
    cut(lo)
}

/// The widths of tabs whose own widths are `own` in `room` points: their own while they all
/// fit; else the widest cut to an equal share of what the narrower leave, none below
/// [`TAB_LEAST`] (or its own width, when less), and then they don't all fit.
fn shared_widths(own: &[f32], room: f32) -> Vec<f32> {
    let gaps = TAB_GAP * own.len().saturating_sub(1) as f32;
    if own.iter().sum::<f32>() + gaps <= room {
        return own.to_vec();
    }
    let mut narrow_first = own.to_vec();
    narrow_first.sort_by(f32::total_cmp);
    let (mut left, mut share) = (room - gaps, 0.0);
    for (i, w) in narrow_first.iter().enumerate() {
        share = left / (narrow_first.len() - i) as f32;
        if *w > share {
            break;
        }
        left -= w;
    }
    let share = share.floor().max(TAB_LEAST);
    own.iter().map(|w| w.min(share)).collect()
}

/// A tab's name for screen readers: its document's, saying when it has unsaved changes.
fn tab_title(name: &str, dirty: bool) -> String {
    if dirty {
        format!("{name}, unsaved")
    } else {
        name.to_string()
    }
}

/// How far the document tabs are scrolled, with the tab in front, the number of tabs and the
/// room they were last drawn with: when any of those changes, the tab in front is scrolled
/// into view. `held` is the arrow held down (-1 left, 1 right) and when it steps next.
#[derive(Clone, Copy, Default)]
struct TabScroll {
    offset: f32,
    front: Option<u64>,
    count: usize,
    room: f32,
    held: Option<(i8, f64)>,
}

impl TabScroll {
    /// Whether the arrow `r` toward `dir` steps the tabs this frame, at `now`: once when it
    /// is pressed, clicked or chosen with the keyboard, then again and again while it is held
    /// down. Letting it go after holding it doesn't step again.
    fn steps(&mut self, ctx: &egui::Context, r: &egui::Response, dir: i8, now: f64) -> bool {
        let down = r.is_pointer_button_down_on();
        match self.held.filter(|&(d, _)| d == dir) {
            Some((_, next)) if down => {
                let step = now >= next;
                if step {
                    self.held = Some((dir, now + HOLD_EVERY));
                }
                let wait = self.held.map_or(0.0, |(_, next)| next - now);
                ctx.request_repaint_after(std::time::Duration::from_secs_f64(wait.max(0.0)));
                step
            }
            Some(_) => {
                self.held = None;
                false
            }
            None if down => {
                self.held = Some((dir, now + HOLD_FIRST));
                ctx.request_repaint_after(std::time::Duration::from_secs_f64(HOLD_FIRST));
                true
            }
            None => r.clicked(),
        }
    }
}

/// What was done to a document tab this frame.
struct TabUse {
    /// The tab was clicked, to bring it to the front.
    pick: bool,
    close: bool,
    /// The tab or its close button has just been given the keyboard.
    focused: bool,
}

/// A document tab at `rect` in the tab strip `ui` (clipped to the part of the strip seen):
/// the file's icon and name, cut short with "…" when the tab is narrower than the name,
/// "Unsaved" while it has unsaved changes (a dot for the word when there is no room for it),
/// and a close button; the full name on hover. The tab shown is a card joined to the
/// commands below it. Its ids come from the tab's `id`, so the keyboard and a click stay on
/// it as tabs before it close.
fn tab_button(
    ui: &mut egui::Ui,
    parts: &TabParts,
    id: u64,
    (name, dirty): (&str, bool),
    rect: egui::Rect,
    on: bool,
) -> TabUse {
    let id = egui::Id::unique(("document tab", id));
    let mut ui = ui.new_child(egui::UiBuilder::new().scope_id(id).max_rect(rect));
    let dark = ui.visuals().dark_mode;
    // A tab, saying whether it is the one shown.
    let pick = ui.interact(rect, id.with("pick"), egui::Sense::click());
    let title = tab_title(name, dirty);
    pick.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Tab, true, &title));
    ui.ctx()
        .accesskit_node_builder(pick.id, |n| n.set_selected(on));
    let top = egui::CornerRadius {
        nw: 7,
        ne: 7,
        sw: 0,
        se: 0,
    };
    let p = ui.painter();
    if on {
        let fill = theme::surface(dark);
        p.rect_filled(rect, top, fill);
        let edge = egui::Stroke::new(1.0, theme::border(dark));
        p.rect_stroke(rect, top, edge, egui::StrokeKind::Inside);
        // Open at the bottom, onto the commands.
        let across = rect.left() + 1.0..=rect.right() - 1.0;
        p.hline(across, rect.bottom() - 0.5, egui::Stroke::new(1.0, fill));
    }
    if pick.has_focus() {
        let ring = ui.visuals().selection.stroke;
        p.rect_stroke(rect.shrink(1.0), top, ring, egui::StrokeKind::Inside);
    }
    let mid = rect.center().y;
    let at = |x: f32, g: &egui::Galley| egui::pos2(x, mid - g.size().y / 2.0);
    let mut x = rect.left() + TAB_LEFT;
    let plain = ui.visuals().text_color();
    p.galley(at(x, &parts.icon), parts.icon.clone(), plain);
    x += parts.icon.size().x + TAB_SPACE;
    // The name, and "Unsaved" after it while there is room for the word beside about ten
    // characters of the name (or all of it), else a dot.
    let room = rect.width() - parts.fixed();
    let whole = text_width(&ui, name, &parts.font);
    let word = dirty && room - parts.word() >= whole.min(NAME_LEAST);
    let mark = match (dirty, word) {
        (false, _) => 0.0,
        (true, true) => parts.word(),
        (true, false) => parts.dot(),
    };
    let shown = shortened(&ui, name, &parts.font, room - mark);
    let ink = if on || pick.hovered() {
        ui.style().interact(&pick).text_color()
    } else {
        theme::muted(dark)
    };
    let g = p.layout_no_wrap(shown, parts.font.clone(), ink);
    p.galley(at(x, &g), g.clone(), ink);
    x += g.size().x;
    if dirty {
        let (gap, g) = match word {
            true => (TAB_SPACE, &parts.unsaved),
            false => (TAB_GAP, &parts.dot),
        };
        p.galley(at(x + gap, g), g.clone(), theme::muted(dark));
    }
    let centre = egui::pos2(rect.right() - TAB_RIGHT - TAB_CLOSE / 2.0, mid);
    // Narrow enough for the glyph to fit its place.
    ui.spacing_mut().button_padding.x = 4.0;
    let closer = egui::Button::new(theme::icon(icon::X).size(12.0))
        .frame_when_inactive(false)
        .min_size(egui::Vec2::splat(TAB_CLOSE));
    let closer = ui.put(
        egui::Rect::from_center_size(centre, egui::Vec2::splat(TAB_CLOSE)),
        closer,
    );
    closer.widget_info(|| named(&format!("Close {name}")));
    let focused = pick.gained_focus() || closer.gained_focus();
    let close = closer.on_hover_text("Close tab").clicked();
    let tip = if dirty {
        format!("{name}\nUnsaved changes")
    } else {
        name.to_string()
    };
    TabUse {
        pick: pick.on_hover_text(tip).clicked(),
        close,
        focused,
    }
}

/// The row of document tabs: each at its own width up to [`TAB_MOST`]; when they don't all
/// fit, sharing the room, down to [`TAB_LEAST`]; past that the row scrolls, by the wheel over
/// it (an upright wheel too, as in web browsers) or the arrows at its right, and a button
/// there lists every open document. The tab in front is scrolled into view when it, the
/// number of tabs or the room changes, and a tab given the keyboard is too. Every part is
/// placed by measure, so the row is as wide as the room and one tab high whatever the tabs.
/// Returns the tab chosen and the one whose close button was clicked.
fn tab_strip(ui: &mut egui::Ui, tabs: &[Tab], active: usize) -> (Option<usize>, Option<u64>) {
    let parts = TabParts::new(ui);
    let names: Vec<(String, bool)> = tabs.iter().map(|t| (t.name(), t.unsaved())).collect();
    let own: Vec<f32> = (names.iter())
        .map(|(name, dirty)| parts.own_width(ui, name, *dirty))
        .collect();
    let room = ui.available_width();
    let widths = shared_widths(&own, room);
    // Where each tab starts, from the first one's left.
    let mut starts = Vec::with_capacity(tabs.len());
    let mut end = 0.0f32;
    for w in &widths {
        starts.push(end);
        end += w + TAB_GAP;
    }
    let content = (end - TAB_GAP).max(0.0);
    let more = content > room + 0.5;
    let (row, _) = ui.allocate_exact_size(egui::vec2(room, TAB_HEIGHT), egui::Sense::hover());
    let ends = if more {
        TAB_GAP + 3.0 * STRIP_BUTTON
    } else {
        0.0
    };
    let right = (row.right() - ends).max(row.left());
    let view = egui::Rect::from_min_max(row.min, egui::pos2(right, row.bottom()));
    let most = (content - view.width()).max(0.0);
    // `offset` moved the least that shows tab `i` whole (its start, if it is wider than the
    // part seen).
    let into_view = |offset: f32, i: usize| {
        let shows_end = starts[i] + widths[i] - view.width();
        offset.max(shows_end).min(starts[i])
    };
    let key = ui.scope_id().with("document tabs");
    let mut s: TabScroll = ui.data(|d| d.get_temp(key)).unwrap_or_default();
    let front = tabs.get(active).map(|t| t.id);
    let changed = s.front != front || s.count != tabs.len() || (s.room - room).abs() > 0.5;
    if changed && active < tabs.len() {
        s.offset = into_view(s.offset, active);
    }
    if more && ui.rect_contains_pointer(row) {
        let wheel = ui.input(|i| i.smooth_scroll_delta);
        if wheel != egui::Vec2::ZERO {
            s.offset -= wheel.x + wheel.y;
            ui.input_mut(|i| i.smooth_scroll_delta = egui::Vec2::ZERO);
        }
    }
    s.offset = s.offset.clamp(0.0, most);
    let drawn_at = s.offset;

    // Every tab, those out of view too, so a click egui follows on one isn't lost.
    let mut strip = ui.new_child(egui::UiBuilder::new().max_rect(view));
    strip.shrink_clip_rect(view);
    let (mut pick, mut close, mut shown) = (None, None, None);
    for (i, t) in tabs.iter().enumerate() {
        let min = egui::pos2(view.left() + starts[i] - s.offset, row.top());
        let rect = egui::Rect::from_min_size(min, egui::vec2(widths[i], TAB_HEIGHT));
        let (name, dirty) = &names[i];
        let on = i == active;
        let used = tab_button(&mut strip, &parts, t.id, (name, *dirty), rect, on);
        if on {
            shown = Some(rect);
        }
        if used.pick {
            pick = Some(i);
        }
        if used.close {
            close = Some(t.id);
        }
        if used.focused {
            s.offset = into_view(s.offset, i);
        }
    }

    // The arrows and the documents button, while the tabs don't all fit.
    if more {
        let dark = ui.visuals().dark_mode;
        let place = |k: f32| {
            let x = view.right() + TAB_GAP + (k + 0.5) * STRIP_BUTTON;
            let centre = egui::pos2(x, row.center().y);
            egui::Rect::from_center_size(centre, egui::Vec2::splat(STRIP_BUTTON))
        };
        let ends_id = key.with("ends");
        let mut ends = ui.new_child(egui::UiBuilder::new().scope_id(ends_id).max_rect(row));
        // Narrow enough for each glyph to fit its place.
        ends.spacing_mut().button_padding.x = 4.0;
        let mut arrow = |k: f32, glyph: &str, name: &str, enabled: bool| {
            let b = egui::Button::new(theme::icon(glyph).size(14.0))
                .frame_when_inactive(false)
                .min_size(egui::Vec2::splat(STRIP_BUTTON));
            let r = ends.put(place(k), |ui: &mut egui::Ui| ui.add_enabled(enabled, b));
            r.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, enabled, name));
            r.on_hover_text(name)
        };
        let (can_left, can_right) = (s.offset > 0.5, s.offset < most - 0.5);
        let left = arrow(0.0, icon::CARET_LEFT, "Scroll tabs left", can_left);
        let right = arrow(1.0, icon::CARET_RIGHT, "Scroll tabs right", can_right);
        let now = ui.input(|i| i.time);
        let (to_left, to_right) = (
            s.steps(ui.ctx(), &left, -1, now),
            s.steps(ui.ctx(), &right, 1, now),
        );
        if to_left {
            // The last tab starting left of the part seen, to its left edge.
            let i = starts.iter().rposition(|&x| x < s.offset - 0.5);
            s.offset = i.map_or(0.0, |i| starts[i]);
        }
        if to_right {
            // The first tab ending right of it, to its right edge.
            let seen_to = s.offset + view.width() + 0.5;
            let i = (0..tabs.len()).find(|&i| starts[i] + widths[i] > seen_to);
            s.offset = i.map_or(most, |i| starts[i] + widths[i] - view.width());
        }
        // An arrow scrolled to its end is greyed and can't keep the keyboard (PB-49): it goes
        // to the other arrow.
        if to_right && s.offset >= most - 0.5 && right.has_focus() {
            left.request_focus();
        }
        if to_left && s.offset <= 0.5 && left.has_focus() {
            right.request_focus();
        }
        let list = egui::Button::new(theme::icon(icon::CARET_DOWN).size(14.0))
            .frame_when_inactive(false)
            .min_size(egui::Vec2::splat(STRIP_BUTTON));
        let all = "All open documents";
        // Every open document in tab order, the one in front chosen.
        let menu = |ui: &mut egui::Ui| {
            for (i, (name, dirty)) in names.iter().enumerate() {
                let mut text = egui::Atoms::new(shortened(ui, name, &parts.font, MENU_NAME_MOST));
                if *dirty {
                    let unsaved = egui::RichText::new("Unsaved").size(11.0);
                    text.push_right(unsaved.color(theme::muted(dark)));
                }
                let on = i == active;
                let r = ui.add(egui::RadioButton::new(on, text));
                let title = tab_title(name, *dirty);
                let role = egui::Role::RadioButton;
                r.widget_info(|| egui::WidgetInfo::selected(role, true, on, &title));
                if r.clicked() {
                    pick = Some(i);
                    ui.close();
                }
            }
        };
        ends.put(place(2.0), |ui: &mut egui::Ui| {
            let (r, _) = egui::containers::menu::MenuButton::from_button(list).ui(ui, menu);
            r.widget_info(|| named(all));
            r.on_hover_text(all)
        });
    }
    if s.offset != drawn_at {
        ui.ctx().request_repaint();
    }
    s.front = front;
    s.count = tabs.len();
    s.room = room;
    ui.data_mut(|d| d.insert_temp(key, s));

    // The line under the tabs, open under the one shown where it is seen, which joins the
    // commands below it.
    let y = row.bottom() - 0.5;
    let across = ui.clip_rect().x_range();
    let stroke = egui::Stroke::new(1.0, theme::border(ui.visuals().dark_mode));
    let p = ui.painter();
    let open = shown.map(|r| r.x_range().intersection(view.x_range()));
    match open.filter(|r| r.span() > 0.0) {
        Some(at) => {
            p.hline(across.min..=at.min, y, stroke);
            p.hline(at.max..=across.max, y, stroke);
        }
        None => {
            p.hline(across, y, stroke);
        }
    }
    (pick, close)
}

/// The inspector's heading: what it shows ("Selected text", "Document").
fn inspector_heading(title: &str) -> egui::RichText {
    egui::RichText::new(title)
        .size(15.0)
        .family(theme::medium())
}

/// The inspector's heading row, in the panel (`collapse`) or its pop-out: what it shows, then
/// the Keep tool active pin of a tool that puts things down, a button that closes the task
/// open (PF-40) and, in the panel, the one that collapses it, in reading order for Tab.
/// Closing from the keyboard leaves the keyboard in the inspector: on the collapse button,
/// or the strip's button that opens the pop-out. The pin's choice is app-wide: lent from
/// `s` and saved there when changed. Returns whether the collapse button was clicked.
fn inspector_head(ui: &mut egui::Ui, s: &mut Settings, t: &mut Tab, collapse: bool) -> bool {
    let title = inspector::title(t);
    let task = t.forms.task;
    let pin = t.ed.keeps_pin();
    let mut collapsed = false;
    // The buttons' width when last drawn, for `dialogs::reading_order`; with another number
    // of them, more than they will take, so that the panel is never widened by them showing
    // past their place, and keeps that width.
    let measured = ui.scope_id().with("inspector-head-buttons");
    let buttons = usize::from(pin) + usize::from(task.is_some()) + usize::from(collapse);
    let last = ui
        .data(|d| d.get_temp::<(usize, f32)>(measured))
        .filter(|(n, _)| *n == buttons)
        .map_or(buttons as f32 * 48.0, |(_, width)| width);
    // A tool's long name ("Headers, footers, Bates") goes on two lines, whole, as the
    // menu and the command search have it.
    egui::Sides::new().shrink_left().wrap().show(
        ui,
        |ui| ui.label(inspector_heading(&title)),
        |ui| {
            let width = dlg::reading_order(ui, Some(last), |ui| {
                let mut keys_back = false;
                if pin {
                    t.ed.keep.clone_from(&s.keep_tools);
                    t.ed.keep_button(ui);
                    if t.ed.keep != s.keep_tools {
                        s.keep_tools.clone_from(&t.ed.keep);
                        s.save();
                    }
                }
                if let Some(task) = task {
                    let name = format!("Close {}", task.title());
                    let b = egui::Button::new(theme::icon(icon::X)).frame_when_inactive(false);
                    let r = ui.add(b);
                    r.widget_info(|| named(&name));
                    if r.on_hover_text("Close").clicked() {
                        t.forms.task = None;
                        keys_back = !ui.input(|i| i.pointer.any_click());
                    }
                }
                if collapse {
                    let hide = egui::Button::new(theme::icon(icon::CARET_DOUBLE_RIGHT))
                        .frame_when_inactive(false);
                    // Ids of its own, the same with or without the close button before it,
                    // for the keyboard to go to as that goes.
                    let own = egui::UiBuilder::new().scope_id(egui::Id::unique(COLLAPSE));
                    let r = ui.scope_builder(own, |ui| ui.add(hide)).inner;
                    r.widget_info(|| named("Collapse the inspector"));
                    if keys_back {
                        r.request_focus();
                    }
                    collapsed = r.on_hover_text("Collapse the inspector").clicked();
                } else if keys_back {
                    let opener = egui::Id::unique(POPOUT_OPENER);
                    if let Some(id) = ui.data(|d| d.get_temp::<egui::Id>(opener)) {
                        ui.memory_mut(|m| m.request_focus(id));
                    }
                }
            });
            ui.data_mut(|d| d.insert_temp(measured, (buttons, width)));
        },
    );
    collapsed
}

/// The side panel's views, top to bottom on the strip at the window's left edge: each with
/// the command that shows it, for its icon, and its name.
const SIDE_VIEWS: [(Side, Act, &str); 5] = [
    (Side::Pages, Act::PagesView, "Pages"),
    (Side::Search, Act::SearchView, "Search results"),
    (Side::Bookmarks, Act::Bookmarks, "Bookmarks"),
    (Side::Comments, Act::Comments, "Comments"),
    (Side::Signatures, Act::Signatures, "Digital signatures"),
];

/// Commands that work on the page view: chosen while organising, they go back to it
/// (NEW-14). The Pages view is the thumbnails beside it.
fn of_the_page(a: Act) -> bool {
    matches!(
        a,
        Act::Tool(_)
            | Act::EditObjectText
            | Act::MoveText
            | Act::ReplaceImage
            | Act::NextField
            | Act::PrevField
            | Act::ClearField
            | Act::SignWith(_)
            | Act::CreateSign(_)
            | Act::SignDigital
            | Act::Certify
            | Act::FitWidth
            | Act::Layout(_)
            | Act::ToggleCover
            | Act::PagesView
            | Act::Crop
    )
}

/// The side panel showing view `side`: its id, so that each view keeps the width it is
/// given, and the width it starts at: the pages' as the workspace has it, the views of text
/// wider, so that their lines don't wrap a word or two at a time (ENH-38).
fn side_panel(side: Side) -> (&'static str, f32) {
    match side {
        Side::Pages => ("side", SIDE_WIDTH),
        Side::Search => ("side-search", SIDE_TEXT_WIDTH),
        Side::Bookmarks => ("side-bookmarks", SIDE_TEXT_WIDTH),
        Side::Comments => ("side-comments", SIDE_TEXT_WIDTH),
        Side::Signatures => ("side-signatures", SIDE_TEXT_WIDTH),
    }
}

/// What the side panel calls its view `side`.
fn side_name(side: Side) -> &'static str {
    SIDE_VIEWS
        .iter()
        .find(|(s, ..)| *s == side)
        .map_or("", |(.., name)| name)
}

/// The zoom in the status bar, "96%": a menu of the zoom (Fit width ticked while it
/// follows the width) and the page layouts. It is as wide for any zoom, so the buttons
/// beside it stay where they are as the zoom changes.
fn zoom_menu(ui: &mut egui::Ui, zoom: f32, c: &Ctx, acts: &mut Vec<Act>) {
    let level = format!("{:.0}%", zoom * 100.0);
    let muted = theme::muted(ui.visuals().dark_mode);
    let font = egui::FontId::proportional(12.0);
    let widest = ui.painter().layout_no_wrap("1000%".into(), font, muted);
    let room = widest.size().x + 2.0 * ui.spacing().button_padding.x;
    let b = egui::Button::new(egui::RichText::new(&level).size(12.0).color(muted))
        .frame_when_inactive(false);
    // The menu at the box's right, the end of the bar.
    let size = egui::vec2(room, ui.spacing().interact_size.y);
    let layout = egui::Layout::right_to_left(egui::Align::Center);
    ui.allocate_ui_with_layout(size, layout, |ui| {
        egui::containers::menu::MenuButton::from_button(b)
            .ui(ui, |ui| {
                for act in [Act::ZoomIn, Act::ZoomOut, Act::Actual, Act::FitWidth] {
                    menu_item(ui, actions::get(act), c, acts);
                }
                ui.separator();
                for a in ACTIONS.iter().filter(|a| a.submenu == "Page layout") {
                    if a.sep {
                        ui.separator();
                    }
                    menu_item(ui, a, c, acts);
                }
            })
            .0
            .on_hover_text("Zoom and page layout")
            .widget_info(|| named(&format!("Zoom {level}")));
    });
}

/// The window's width under which the status bar's zoom has no slider, only its buttons.
const SLIDER_ROOM: f32 = 900.0;

/// The status bar's view buttons, as a word processor's: the pages as they read, and
/// Organise's grid (NEW-14), the one showing marked.
fn view_buttons(ui: &mut egui::Ui, organise: bool, acts: &mut Vec<Act>) {
    ui.scope(|ui| {
        ui.spacing_mut().item_spacing.x = 2.0;
        let views = [
            (icon::FILE_TEXT, "Page view", !organise),
            (icon::SQUARES_FOUR, "Organise pages", organise),
        ];
        for (glyph, name, on) in views {
            let b = egui::Button::new(theme::icon(glyph).size(14.0))
                .selected(on)
                .frame_when_inactive(on)
                .min_size(egui::vec2(22.0, 20.0));
            let r = ui.add(b);
            r.widget_info(|| egui::WidgetInfo::selected(egui::Role::Button, true, on, name));
            if r.on_hover_text(name).clicked() && !on {
                acts.push(Act::Organise);
            }
        }
    });
}

/// The status bar's zoom, as a word processor's: − and + beside a slider, then the zoom's
/// menu with its level. Organising, they size the grid's cards. The slider changes the
/// zoom only when moved: one beyond its ends (a key's 6400%) stays as it is.
fn zoom_bar(ui: &mut egui::Ui, zoom: f32, organise: bool, c: &Ctx, acts: &mut Vec<Act>) {
    use crate::settings::{ORGANISE_MAX, ORGANISE_MIN};
    let button = |ui: &mut egui::Ui, glyph: &str, name: &str| {
        let b = egui::Button::new(theme::icon(glyph).size(14.0))
            .frame_when_inactive(false)
            .min_size(egui::vec2(20.0, 20.0));
        let r = ui.add(b);
        r.widget_info(|| named(name));
        r.on_hover_text(name).clicked()
    };
    ui.scope(|ui| {
        ui.spacing_mut().item_spacing.x = 2.0;
        if button(ui, icon::MINUS, "Zoom out") {
            acts.push(Act::ZoomOut);
        }
        if ui.ctx().content_rect().width() >= SLIDER_ROOM {
            let (lo, hi) = match organise {
                true => (ORGANISE_MIN, ORGANISE_MAX),
                false => (0.1, 10.0),
            };
            let mut percent = zoom * 100.0;
            ui.spacing_mut().slider_width = 96.0;
            let slider = egui::Slider::new(&mut percent, lo * 100.0..=hi * 100.0)
                .logarithmic(true)
                .show_value(false)
                .clamping(egui::SliderClamping::Never);
            let r = ui.add(slider);
            let level = f64::from(zoom * 100.0);
            r.widget_info(|| egui::WidgetInfo::slider(true, level, "Zoom"));
            if r.changed() {
                acts.push(Act::ZoomTo(percent.round().max(1.0) as u32));
            }
        }
        if button(ui, icon::PLUS, "Zoom in") {
            acts.push(Act::ZoomIn);
        }
    });
    let mut c = *c;
    c.fit &= !organise;
    zoom_menu(ui, zoom, &c, acts);
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
        ui.data_mut(|d| d.insert_temp(egui::Id::unique(POPOUT_OPENER), r.id));
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
    // Sections' contents a little in from their headers, as the workspace has them.
    ui.spacing_mut().indent = 10.0;
    theme::inspector_style(ui.style_mut());
    t.guarded(|t| {
        if !t.ed.owns_panel() {
            apply = inspector::show(ui, t, c);
            return;
        }
        t.ed.planned = c.planned;
        // Stamp choices and the author's name are app-wide: lent to the tab's inspector
        // and saved when it changes them.
        let author = s.author();
        t.ed.author = author.clone();
        t.ed.stamp_name_date = s.stamp_name_date;
        t.ed.stamp_pictures = s.stamp_pictures.clone();
        t.ed.stamp_wordings = s.stamp_wordings.clone();
        t.ed.sign_kit = s.sign.clone();
        // Lent for the tools' placing; the pin in the heading changes it (`inspector_head`).
        t.ed.keep.clone_from(&s.keep_tools);
        // The selected comment's page by its label, as the Comments view names it.
        t.ed.annot_page = t.ed.selection.annot().map(|(p, _)| t.page_name(p));
        t.ed.inspector(ui);
        if std::mem::take(&mut t.ed.apply_redactions) {
            t.requested.push(Act::ApplyRedactions);
        }
        if std::mem::take(&mut t.ed.finish_signing) {
            t.requested.push(Act::FinishSigning);
        }
        if t.ed.sign_kit != s.sign {
            let kept = t.ed.sign_kit.kept_since(&s.sign);
            s.sign = t.ed.sign_kit.clone();
            s.save_sign(kept);
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
/// The ids of the button that collapses the inspector.
const COLLAPSE: &str = "inspector-collapse";

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
) -> (Option<inspector::Apply>, bool, egui::Rect) {
    let mut apply = None;
    let height = (strip.height() - 16.0).max(120.0);
    let area = egui::Area::new(egui::Id::unique("inspector-popout"))
        .order(egui::Order::Foreground)
        .fixed_pos(egui::pos2(strip.left() - 4.0, strip.top() + 4.0))
        .pivot(egui::Align2::RIGHT_TOP)
        .constrain(true)
        .show(ctx, |ui| {
            regions::name(ui, Region::Inspector);
            egui::Frame::popup(ui.style()).show(ui, |ui| {
                ui.set_width(INSPECTOR_WIDTH);
                inspector_head(ui, s, t, false);
                ui.add_space(4.0);
                let scope = t.ed.scope();
                egui::ScrollArea::vertical()
                    .max_height(height)
                    .show(ui, |ui| {
                        apply = ui.push_id(scope, |ui| inspector_body(s, t, ui, c)).inner;
                    });
            });
        });
    let clicked_outside = ctx.input(|i| {
        let at = i.pointer.interact_pos();
        i.pointer.any_click()
            && at.is_some_and(|p| !area.response.rect.contains(p) && !strip.contains(p))
    });
    // A menu or colour picker opened from the pop-out is outside it, but part of it.
    let child_open = egui::Popup::is_any_open(ctx);
    (apply, !(clicked_outside && !child_open), area.response.rect)
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
    let spoken = actions::spoken_title(a);
    r.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, enabled, &spoken));
    dlg::set_name(&r, &spoken);
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
    // The name after its submenu's, in the weak colour, cut short with … if it would reach
    // the category.
    let left = rect.left() + pad + 26.0;
    let mut job = egui::text::LayoutJob::default();
    let part = |colour| egui::TextFormat::simple(font.clone(), colour);
    if !a.submenu.is_empty() {
        job.append(&format!("{} › ", a.submenu), 0.0, part(weak));
    }
    job.append(a.label, 0.0, part(text));
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
    let id = egui::Id::unique("empty-card");
    // A click anywhere on it opens a file; the keyboard has Open PDF… for that, so the
    // card is no stop for Tab or F6 of its own.
    let r = ui
        .interact(card, id, egui::Sense::CLICK)
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
    fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        // On Wayland, frames go to the screen without waiting for it, which may never take
        // them (BUG-42).
        crate::present::set(frame);
        self.poll_loads(&ctx);
        // Interface text the interface's own fonts can't draw (Chinese, Japanese, Korean,
        // emoji) in installed fonts, found on another thread at start (ENH-33).
        crate::typing_fonts::interface_fallback(&ctx);
        // Back from another app, which may have changed the files open here.
        let focused = ctx.input(|i| i.focused);
        if focused && !self.focused {
            self.check_disk(&ctx, None);
            // The desktop's accent may have changed meanwhile, too.
            self.read_accent(&ctx);
        }
        self.focused = focused;
        if let Some(Ok(accent)) = self.accent_read.as_ref().map(|rx| rx.try_recv()) {
            self.accent_read = None;
            if self.settings.accent == Accent::Desktop {
                theme::set_accent(&ctx, accent);
            }
        }
        self.recovery_tick(&ctx);
        // Page colours follow the setting and, when automatic, the app's theme.
        let colours = self.colours(&ctx);
        for t in &mut self.tabs {
            t.set_colours(colours);
        }
        // Words selected with the read tool, ready for the inspector to restyle.
        if let Some(t) = self.tabs.get_mut(self.active) {
            t.guarded(|t| t.ed.sync_chosen(&t.pdf));
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
        self.open_incoming(&ctx);

        // Guard against closing the window with unsaved work.
        let quit = ctx.input(|i| i.viewport().close_requested()) && !self.quit_ok;
        if quit && !self.saves.is_empty() {
            // A save being written ends first; then quitting goes on as usual.
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            self.wait_for_saves(AfterSave::Quit);
        } else if let Some(q) = quit.then(|| self.quit_question()).flatten() {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            // Asked first: a question open comes back after Cancel.
            let open = std::mem::replace(&mut self.dialog, q);
            if !matches!(open, Dialog::None) && !open.quitting() {
                self.waiting.push_front(open);
            }
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

        // F6's regions are found again as the window is drawn. A click in the tab in front
        // ends what F6 left waiting on its page, as does the typing getting the keyboard
        // back or ending; the ring on the page goes once anything takes it.
        self.regions.begin();
        let pressed = ctx.input(|i| i.pointer.any_pressed());
        let focused = ctx.memory(|m| m.focused());
        if pressed || focused.is_some() {
            self.regions.ring = false;
        }
        // Typing open on the page of a tab sent behind waits there, as for F6, and has the
        // keyboard again once its tab is back in front, as a word processor's document does.
        let front = self.tabs.get(self.active).map(|t| t.id);
        let switched = front != self.front;
        let behind = self.front.filter(|_| switched);
        if let Some(t) = self.tabs.iter_mut().find(|t| Some(t.id) == behind) {
            if let Some(id) = t.ed.typing_id() {
                t.ed.parked = Some(Parked::Typing(id));
            }
        }
        self.front = front;
        for (i, t) in self.tabs.iter_mut().enumerate() {
            let back = matches!(t.ed.parked, Some(Parked::Typing(id))
                if focused == Some(id) || t.ed.typing_id() != Some(id));
            if back || (pressed && i == self.active) {
                t.ed.parked = None;
            }
        }
        let waiting = matches!(self.tabs.get(self.active), Some(t) if t.ed.parked.is_some());
        if switched && waiting {
            self.keys_to_page(&ctx, focused);
        }

        // Undo and Redo in the commands act on the typing in the text box with the keyboard,
        // as Ctrl+Z and Ctrl+Y do there: a click on them leaves the keyboard in it, and the
        // key goes to it on the next frame, before anything else sees that frame's input. A
        // paragraph's frame's handle with the keyboard is part of its typing.
        let on_edge = (self.tabs.get(self.active)).is_some_and(|t| t.ed.edge_held(&ctx));
        let in_box = ctx.text_edit_focused() || on_edge;
        self.undo_held = in_box
            && ctx.input(|i| {
                let at = i.pointer.press_origin().or(i.pointer.latest_pos());
                (i.pointer.any_pressed() || i.pointer.any_released())
                    && at.is_some_and(|p| self.undo_buttons.iter().any(|r| r.contains(p)))
            });
        let surrender = if self.undo_held {
            egui::SurrenderFocusOn::Never
        } else {
            egui::SurrenderFocusOn::Clicks
        };
        ctx.options_mut(|o| o.input_options.surrender_focus_on = surrender);
        if let Some(a) = self.undo_in_box.take().filter(|_| in_box) {
            let modifiers = match a {
                Act::Redo => egui::Modifiers::COMMAND | egui::Modifiers::SHIFT,
                _ => egui::Modifiers::COMMAND,
            };
            ctx.input_mut(|i| {
                i.events.push(egui::Event::Key {
                    key: Key::Z,
                    physical_key: None,
                    pressed: true,
                    repeat: false,
                    modifiers,
                });
            });
        }

        // Before anything sees this frame's keys: a text box that has just got the keyboard
        // starts with nothing to undo.
        let before = self.keyboard;
        // A box the keyboard comes back to, with nothing kept in the document since it left
        // (the tab's revision), keeps its typing's history (ENH-32). A new note or text box
        // has its page's box: one typed and let go never lends its history to the next.
        let doc = self.tabs.get(self.active).map(|t| (t.id, t.revision));
        let now = ctx.memory(|m| m.focused());
        if let (Some(id), Some((tab, revision))) = (before.filter(|_| now != before), doc) {
            self.left_box = Some((id, tab, revision));
        }
        let left = self.left_box;
        let new_box = |id| self.tabs.get(self.active).is_some_and(|t| t.ed.new_box(id));
        let kept = |id| doc.is_some_and(|(t, r)| left == Some((id, t, r))) && !new_box(id);
        self.keyboard = page_text::fresh_history(&ctx, before, kept);
        // Esc in a box of the band or the inspector (the font size, say), with typing open
        // on the page, drops what was typed in the box and goes back to the typing, as in a
        // word processor, rather than ending it.
        let typing = self.tabs.get(self.active).and_then(|t| t.ed.typing_id());
        let in_box = before
            .is_some_and(|id| Some(id) != typing && egui::TextEdit::load_state(&ctx, id).is_some());
        let esc = ctx.input(|i| i.key_pressed(Key::Escape));
        self.esc_to_typing = esc && in_box && typing.is_some();
        if self.esc_to_typing {
            self.tabs[self.active].ed.refocus_typing();
        }
        self.shortcuts(&ctx);
        // Setting the crop: Enter ends it, unless the keyboard is on a control (a margin
        // being typed takes its Enter) or a question is open.
        let free = ctx.memory(|m| m.focused()).is_none() && matches!(self.dialog, Dialog::None);
        let enter = free && ctx.input(|i| i.key_pressed(Key::Enter));
        let t = self.tabs.get_mut(self.active);
        if let Some(t) = t.filter(|t| enter && t.crop_view()) {
            t.end_crop();
        }
        // Paragraph editing as the app has it, in every tab (a tab opened since too).
        for t in &mut self.tabs {
            t.ed.paragraphs = self.settings.paragraphs;
        }
        self.type_over(&ctx);
        self.clipboard(&ctx);

        let mut acts: Vec<Act> = Vec::new();
        let has = !self.tabs.is_empty();
        let c = self.state(&ctx);
        // Recent files that still exist, for the menu and the start screen. Copied, so
        // the frame can change the app while showing them.
        let recent_files: Vec<PathBuf> = self.recent.existing().into_iter().cloned().collect();
        let recent: Vec<&PathBuf> = recent_files.iter().collect();

        // A document command's button in the commands' row: its icon and label, or its icon
        // alone, named; its label, shortcut and hint as the tooltip. Returns where it is.
        let quick =
            |ui: &mut egui::Ui, act: Act, glyph: &str, label: Option<&str>, acts: &mut Vec<Act>| {
                let a = actions::get(act);
                let glyph = theme::icon(glyph).size(16.0);
                let b = match label {
                    Some(label) => egui::Button::new((glyph, label)),
                    None => egui::Button::new(glyph),
                }
                .frame_when_inactive(false)
                .min_size(egui::vec2(28.0, 28.0));
                let enabled = (a.enabled)(&c);
                let r = ui.add_enabled(enabled, b);
                let name = a.label.trim_end_matches('…');
                r.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, enabled, name));
                let tip = tooltip(ui.ctx(), a);
                let at = r.rect;
                if r.on_hover_text(tip.as_str())
                    .on_disabled_hover_text(actions::greyed_tip(ui.ctx(), a, &c))
                    .clicked()
                {
                    acts.push(act);
                }
                at
            };
        // The saved signatures for Fill and sign's menu.
        let sign_menu = self
            .tabs
            .get_mut(self.active)
            .map(|t| t.ed.sign.menu(&ctx, &self.settings.sign));

        // The tab whose close button was clicked, closed at the end of the frame: the rest
        // of it is drawn for the tabs as they were (`has`, `c`).
        let mut close = None;
        if has {
            let tabs = egui::Panel::top("tabs")
                .frame(theme::tabs_frame(ui.style()))
                .show_separator_line(false)
                .show(ui, |ui| {
                    regions::name(ui, Region::Tabs);
                    let (pick, x) = tab_strip(ui, &self.tabs, self.active);
                    if let Some(i) = pick {
                        self.active = i;
                    }
                    close = x;
                });
            self.regions.add(Region::Tabs, tabs.response.rect);
        }

        // The commands (ENH-38), below the tabs: one row of the document's own commands as
        // icons (the File menu with every command, Open, Save, Undo, Redo), the band's names,
        // and Print and the button for compact commands at the right (the user's layout,
        // 2026-10-27, for a shorter window top); under it the chosen band's groups, compact
        // when chosen or while the window is short. (The file's name is on its tab and in
        // the window's title.)
        let short = ctx.content_rect().height() < SHORT_HEIGHT;
        let compact = c.compact || short;
        let room = ctx.content_rect().width();
        let label = |text: &'static str| (room >= 560.0).then_some(text);
        let active = self.active;
        let chosen = &mut self.band;
        let right_last = &mut self.commands_right;
        let undo_buttons = &mut self.undo_buttons;
        let mut tab = self.tabs.get_mut(active);
        let commands = egui::Panel::top("commands")
            .frame(theme::commands_frame(ui.style()))
            .show(ui, |ui| {
                let name = match has {
                    true => format!("{} commands", chosen.name()),
                    false => "Commands".into(),
                };
                regions::name_as(ui, egui::accesskit::Role::Toolbar, &name);
                let row = egui::Margin {
                    left: 10,
                    right: 10,
                    top: 4,
                    bottom: 0,
                };
                let mut toggled = false;
                egui::Frame::new().inner_margin(row).show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 2.0;
                        let file = egui::Button::new(theme::icon(icon::LIST).size(16.0))
                            .frame_when_inactive(false)
                            .min_size(egui::vec2(28.0, 28.0));
                        egui::containers::menu::MenuButton::from_button(file)
                            .ui(ui, |ui| Self::menu(ui, &c, &recent, &mut acts))
                            .0
                            .on_hover_text("File: every command, by kind")
                            .widget_info(|| named("File"));
                        quick(ui, Act::Open, icon::FOLDER_OPEN, None, &mut acts);
                        quick(ui, Act::Save, icon::FLOPPY_DISK, None, &mut acts);
                        divider(ui);
                        *undo_buttons = vec![
                            quick(ui, Act::Undo, icon::ARROW_U_UP_LEFT, None, &mut acts),
                            quick(ui, Act::Redo, icon::ARROW_U_UP_RIGHT, None, &mut acts),
                        ];
                        divider(ui);
                        // The names, leaving room for the controls at the right as they
                        // were last drawn; then those, in reading order.
                        let right = right_last.unwrap_or(110.0);
                        if has {
                            let names = (ui.available_width() - right - 8.0).max(0.0);
                            ui.scope(|ui| {
                                ui.set_max_width(names);
                                band::names(ui, chosen);
                            });
                        }
                        let end = egui::Layout::right_to_left(egui::Align::Center);
                        ui.with_layout(end, |ui| {
                            let width = dlg::reading_order(ui, *right_last, |ui| {
                                quick(ui, Act::Print, icon::PRINTER, label("Print"), &mut acts);
                                if has {
                                    toggled = band::compact_toggle(ui, compact, short);
                                }
                            });
                            *right_last = Some(width);
                        });
                    });
                });
                if toggled {
                    acts.push(Act::ToggleCompact);
                }
                let Some(t) = tab.as_deref_mut() else {
                    return Vec::new();
                };
                egui::Frame::new()
                    .inner_margin(egui::Margin::symmetric(8, 4))
                    .show(ui, |ui| {
                        let mut font = |ui: &mut egui::Ui, planned: &dyn Fn(&mut egui::Ui)| {
                            t.guarded(|t| t.ed.font_band(ui, planned, compact));
                        };
                        let mut parts = band::Parts {
                            c: &c,
                            sign: sign_menu.as_ref(),
                            font: &mut font,
                            compact,
                        };
                        band::show(ui, *chosen, &mut parts, &mut acts)
                    })
                    .inner
            });
        if let Some(t) = tab {
            t.ed.keep_typing = commands.inner;
        }
        self.regions.add(Region::Commands, commands.response.rect);

        // The status bar: what is selected or the tool in use, and messages, at the left;
        // the page, Show planned features and the zoom at the right.
        let status = egui::Panel::bottom("status")
            .frame(theme::bar_frame(ui.style()))
            .show(ui, |ui| {
                regions::name(ui, Region::Status);
                let t = self.tabs.get(self.active);
                let busy = self.busy_sign();
                let muted = theme::muted(ui.visuals().dark_mode);
                let small = |text: &str| egui::RichText::new(text).size(12.0).color(muted);
                let status = |ui: &mut egui::Ui| {
                    let mut said = Vec::new();
                    // A line or paragraph edited on the page: how much the typing can change.
                    if t.is_some_and(|t| t.ed.editing_paragraph()) {
                        said.push("Editing a paragraph".into());
                    } else if t.is_some_and(|t| t.ed.editing_line()) {
                        said.push("Editing one line".into());
                    } else if let Some(n) = t.and_then(Tab::cropping) {
                        // Setting the crop: what is cropped, and the way out.
                        let pages = if n == 1 { "page" } else { "pages" };
                        said.push(format!(
                            "Cropping {n} {pages}: click away or press Esc to finish"
                        ));
                    } else if let Some(t) = t.filter(|t| t.ed.owns_panel()) {
                        said.push(t.ed.heading());
                    }
                    if t.is_some_and(|t| t.encryption.is_some()) {
                        said.push("Will be encrypted on save".into());
                    }
                    if !self.status.is_empty() {
                        said.push(self.status.clone());
                    }
                    // The busy sign while a save is written.
                    if let Some((n, text)) = &busy {
                        ui.add(egui::Spinner::new().size(12.0));
                        said.push(text.clone());
                        timing::once("save shown", || n.to_string());
                    }
                    ui.label(small(&said.join(" · ")));
                };
                let last = &mut self.status_width;
                let grid_zoom = self.settings.organise_zoom;
                let show_planned = &mut self.settings.show_planned;
                let mut planned_changed = false;
                let view = |ui: &mut egui::Ui| {
                    let Some(t) = t else { return };
                    *last = Some(dlg::reading_order(ui, *last, |ui| {
                        ui.spacing_mut().item_spacing.x = 16.0;
                        // Both, whichever the rest of the window shows.
                        let (n, name) = (t.current + 1, t.page_label(t.current));
                        let count = t.page_count();
                        let chosen = t.ed.selection.pages().map_or(0, |s| s.len());
                        ui.label(small(&match (t.organising(), chosen) {
                            // Organise: the pages, and those selected.
                            (true, 0) if count == 1 => "1 page".into(),
                            (true, 0) => format!("{count} pages"),
                            (true, m) => format!("{m} of {count} pages selected"),
                            _ if name == n.to_string() => format!("Page {n} of {count}"),
                            _ => format!("Page {name} ({n} of {count})"),
                        }));
                        let text = small("Show planned features");
                        let r = ui.add(egui::Checkbox::new(show_planned, text));
                        planned_changed = r.changed();
                        r.on_hover_text(actions::get(Act::TogglePlanned).hint);
                        view_buttons(ui, t.organising(), &mut acts);
                        let zoom = if t.organising() { grid_zoom } else { t.zoom };
                        zoom_bar(ui, zoom, t.organising(), &c, &mut acts);
                    }));
                };
                egui::Sides::new()
                    .shrink_left()
                    .truncate()
                    .show(ui, status, view);
                if planned_changed {
                    self.settings.save();
                }
            });
        self.regions.add(Region::Status, status.response.rect);

        // The strip at the window's left edge: an icon for each of the side panel's views.
        // Clicking another view opens it; clicking the one open folds the panel away, and
        // again brings it back (the user chose the strip over tabs in the panel, 2026-10-27:
        // it stays when the panel is closed or folded away by a narrow window).
        if has {
            let t = &self.tabs[self.active];
            // While organising, the grid shows the pages: the Pages view folds away.
            let open = self.side.shown() && !(t.organising() && t.side == Side::Pages);
            let mut pick = None;
            let strip = egui::Panel::left("nav-rail")
                .resizable(false)
                .exact_size(RAIL_WIDTH)
                .show(ui, |ui| {
                    regions::name(ui, Region::Navigation);
                    ui.vertical_centered(|ui| {
                        ui.add_space(6.0);
                        for (side, act, name) in SIDE_VIEWS {
                            let on = open && t.side == side;
                            let glyph = theme::icon(actions::get(act).icon).size(18.0);
                            let b = egui::Button::new(glyph)
                                .selected(on)
                                .frame_when_inactive(on)
                                .min_size(egui::vec2(28.0, 28.0));
                            let r = ui.add(b);
                            r.widget_info(|| {
                                egui::WidgetInfo::selected(egui::Role::Button, true, on, name)
                            });
                            if r.on_hover_text(name).clicked() {
                                pick = Some(side);
                            }
                        }
                    });
                });
            self.regions.add(Region::Navigation, strip.response.rect);
            if let Some(side) = pick {
                if open && t.side == side {
                    self.side.set_open(false);
                    self.keep_panels();
                } else if side == Side::Pages {
                    acts.push(Act::PagesView);
                } else {
                    self.show_view(side);
                }
            }
        }

        // The side panel, beside the strip: pages first, folded away while organising.
        let t = self.tabs.get(self.active);
        let grid = t.is_some_and(|t| t.organising() && t.side == Side::Pages);
        if has && self.side.shown() && !grid {
            let active = self.active;
            let (panel, width) = side_panel(self.tabs[active].side);
            let given = self.settings.panel_widths.get(panel).copied();
            let width = given.unwrap_or(width);
            panels::edge_keys(ui, panel, true);
            let side = egui::Panel::left(panel)
                .resizable(true)
                .default_size(width)
                .min_size(panels::NARROWEST)
                .show(ui, |ui| {
                    let t = &mut self.tabs[active];
                    let heading = side_name(t.side);
                    regions::name_as(ui, egui::accesskit::Role::Navigation, heading);
                    // As wide as it was made, whatever the view holds: egui narrows a panel
                    // to its contents, such as one short line wrapped.
                    ui.set_min_width(ui.available_width());
                    ui.add_space(4.0);
                    let label = match t.side {
                        Side::Pages => format!("{heading} · {}", t.page_count()),
                        _ => heading.to_string(),
                    };
                    let muted = theme::muted(ui.visuals().dark_mode);
                    ui.label(egui::RichText::new(label).size(12.0).color(muted));
                    ui.add_space(4.0);
                    t.guarded(|t| match t.side {
                        Side::Pages => t.show_thumbs(ui, &c),
                        Side::Bookmarks => t.show_outline(ui),
                        Side::Comments => t.show_comments(ui),
                        Side::Signatures => t.show_signatures(ui),
                        Side::Search => {
                            if t.show_results(ui) {
                                acts.push(Act::MarkResultsRedact);
                            }
                        }
                    });
                });
            panels::edge_shown(ui, panel, "Side panel width");
            let held = panels::edge_held(ui, panel);
            self.keep_width(panel, side.response.rect.width(), held);
            self.regions.add(Region::Navigation, side.response.rect);
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
            // A selection or tool the editor's section shows takes the panel from the task
            // open (PF-40), shown or not, as choosing them would.
            if t.ed.owns_panel() {
                t.forms.task = None;
            }
            // As the floating bar: words, a comment or an object, not pages or nothing.
            let selected = matches!(
                t.ed.selection,
                Selection::Words { .. } | Selection::Annot { .. } | Selection::Objects { .. }
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
            let width = s.panel_widths.get("inspector").copied();
            let rail = egui::Panel::right("inspector-rail")
                .frame(theme::inspector_frame(ui.style(), true))
                .resizable(false)
                .exact_size(RAIL_WIDTH);
            let full = egui::Panel::right("inspector")
                .frame(theme::inspector_frame(ui.style(), false))
                .resizable(true)
                .default_size(width.unwrap_or(INSPECTOR_WIDTH))
                .min_size(panels::NARROWEST);
            panels::edge_keys(ui, "inspector", false);
            let shown = egui::Panel::show_switched(ui, &mut open, rail, full, |ui, expanded| {
                regions::name(ui, Region::Inspector);
                if !expanded {
                    (toggle, pop) = inspector_strip(ui, &groups, &c, &mut panel_acts);
                    strip = Some(ui.max_rect());
                    return;
                }
                // The heading stays put above the scrolling content, with the
                // button that hides the panel at its right.
                toggle = inspector_head(ui, s, t, true);
                if !groups.is_empty() {
                    ui.horizontal_wrapped(|ui| {
                        actions::context_buttons(ui, &groups, &c, &mut panel_acts);
                        ui.separator();
                        float_button(ui, &mut panel_acts);
                    });
                    ui.separator();
                }
                ui.add_space(4.0);
                // In the tab's own ids, so that no two documents share a box's state.
                let scope = t.ed.scope();
                egui::ScrollArea::vertical()
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        panel_apply = ui.push_id(scope, |ui| inspector_body(s, t, ui, &c)).inner;
                    });
            });
            panels::edge_shown(ui, "inspector", "Inspector width");
            self.regions.add(Region::Inspector, shown.response.rect);
            let ed = &mut self.tabs[self.active].ed;
            ed.inspector_drawn(ui.ctx(), shown.response.rect);
            // The width given the panel, not its strip's (dragged closed, say).
            let held = panels::edge_held(ui, "inspector");
            match strip {
                None => self.keep_width("inspector", shown.response.rect.width(), held),
                Some(_) => self.resizing.retain(|p| *p != "inspector"),
            }
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
                let (apply, keep, at) = inspector_popout(&ctx, strip, &mut self.settings, t, &c);
                t.ed.inspector_drawn(&ctx, at);
                panel_apply = panel_apply.or(apply);
                self.popout = keep;
                self.regions.add_over(Region::Inspector, at);
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
        let ring = self.regions.ring;
        let grid_zoom = self.settings.organise_zoom;
        let pages = egui::CentralPanel::default().frame(canvas).show(ui, |ui| {
            if let Some(t) = self.tabs.get_mut(self.active) {
                regions::name(ui, Region::Page);
                // App-wide choices, lent to the tab.
                t.ed.author = author;
                t.ed.keep.clone_from(keep);
                t.ed.form.highlight = fields;
                t.page_ring = ring;
                t.grid_zoom = grid_zoom;
                t.guarded(|t| match t.organising() {
                    true => t.show_grid(ui, &c),
                    false => t.show_pages(ui, &c),
                });
                if let Some(why) = &t.broken {
                    broken_state(ui, why, &mut acts);
                }
            } else {
                regions::name_as(ui, egui::accesskit::Role::Main, "Start");
                empty_state(ui, &recent, &mut acts);
            }
        });
        self.regions.add(Region::Page, pages.response.rect);
        // Ctrl with the wheel over the grid changed its zoom.
        self.keep_grid_zoom();
        if let Some(at) = self.tabs.get(self.active).and_then(|t| t.banners_at) {
            self.regions.add(Region::Notices, at);
        }

        // The Create signature window, whether or not the inspector is showing; a
        // signature made is saved with the others.
        if let Some(t) = self.tabs.get_mut(self.active) {
            t.ed.sign_kit.clone_from(&self.settings.sign);
            t.ed.sign_window(ui.ctx());
            if t.ed.sign_kit != self.settings.sign {
                let kept = t.ed.sign_kit.kept_since(&self.settings.sign);
                self.settings.sign = t.ed.sign_kit.clone();
                self.settings.save_sign(kept);
            }
        }
        self.say_unsaved();

        // Surface errors raised by tab operations.
        if let Some(t) = self.tabs.get_mut(self.active) {
            if let Some(e) = t.error.take() {
                self.dialog = Dialog::Message("Error".into(), e);
            }
            if let Some(url) = t.open_link.take() {
                self.dialog = link_dialog(url);
            }
        }

        // The band's Font group's changes, kept once finished, whether or not the inspector
        // is shown.
        if let Some(t) = self.tabs.get_mut(self.active) {
            t.guarded(|t| t.ed.settle_look(&ctx));
            // A form field given the keyboard: the words selected before are let go, so
            // the band and Ctrl+B act on the field's text alone.
            if t.ed.form.focused_field().is_some() {
                t.ed.selection.clear_words();
            }
        }

        // Commands chosen from right-click menus on pages and thumbnails, and undoable
        // changes the editor reports.
        let (mut notice, mut info) = (None, None);
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
            (notice, info) = (t.notice.take(), t.info.take());
        }
        if let Some(text) = notice {
            self.notify(&ctx, self.active, text, true);
        }
        if let Some(text) = info {
            self.notify(&ctx, self.active, text, false);
        }
        self.toast_ui(&ctx);
        self.palette_ui(&ctx, &c, &mut acts);
        // The inspector's Editing switch, for every tab.
        let active = self.tabs.get_mut(self.active);
        if let Some(on) = active.and_then(|t| t.ed.switch.take()) {
            self.set_paragraphs(on);
        }
        self.shown = Some(c);
        for a in acts {
            self.run_command(&ctx, actions::name(a), |app, ctx| app.act(ctx, a));
        }
        self.shown = None;
        // By its id, as a command above may have opened or closed tabs. The tab in front
        // stays there, unless it is the one closed.
        if let Some(i) = close.and_then(|id| self.tabs.iter().position(|t| t.id == id)) {
            self.run_command(&ctx, actions::name(Act::CloseTab), |app, _| {
                app.close_asking(i)
            });
        }
        // The dialog drawn last takes input: messages and questions go last, as they can
        // come from the windows before them.
        self.restore_window(&ctx);
        self.signature_windows(&ctx);
        self.digital_windows(&ctx);
        self.dialogs(&ctx);
        dlg::name_popups(&ctx);
        self.offer_selection(&ctx);
        // Last, once every region and control of the frame is drawn.
        self.move_region(&ctx);

        let title = match self.tabs.get(self.active) {
            Some(t) => format!("{} - LimarePDF", t.title()),
            None => "LimarePDF".into(),
        };
        if title != self.last_title {
            ctx.send_viewport_cmd(egui::ViewportCommand::Title(title.clone()));
            self.last_title = title;
        }
        timing::mark("frame built", || {
            let (frame, pass) = (ctx.cumulative_frame_nr(), ctx.current_pass_index());
            format!("{frame} pass {pass}")
        });
        timing::flush();
    }

    fn raw_input_hook(&mut self, _ctx: &egui::Context, raw: &mut egui::RawInput) {
        if timing::on() {
            for d in raw.events.iter().filter_map(timing::input) {
                timing::mark("input", || d);
            }
        }
    }

    /// A normal exit (not a crash, which never gets here): no recovery copies are left.
    fn on_exit(&mut self) {
        // Quitting waits for saves, but the system may end the app regardless.
        self.finish_saves();
        timing::mark("exit", String::new);
        timing::flush();
        self.end_recovery();
        // The window's size, for the next start. With the window gone, a failure can only
        // be said where the app was started from.
        self.settings.save();
        if let Some((_, text)) = self.settings.take_unsaved() {
            eprintln!("LimarePDF: {text}");
        }
    }
}

/// The window's inner size in points. Not from the viewport's `inner_rect`, which egui
/// leaves out when the window's position isn't known, as on Wayland.
fn window_size(i: &egui::InputState) -> egui::Vec2 {
    i.viewport_rect().size()
}

/// How a save went, for the timing log.
fn saved(written: &limarepdf_core::Result<()>) -> &'static str {
    match written {
        Ok(()) => "saved",
        Err(e) if not_confirmed(e) => "not confirmed",
        Err(_) => "failed",
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
    use crate::page_text;
    use limarepdf_core::{Cipher, Encryption, FontChoice, Restrictions, SignPlace};

    mod backdrop;
    mod forms;
    mod history;
    mod input_methods;
    mod organise;
    mod overflow;
    mod paragraphs;
    mod redact_marks;
    mod review;
    mod sign;
    mod tabs;
    mod typing_box;
    mod typing_menus;

    /// The height of the window the tests draw: room for the commands above the pages.
    const TEST_HEIGHT: f32 = 1000.0;

    thread_local! {
        /// The file the save chooser gives next, once (see [`App::save_as`]).
        pub(super) static SAVE_AS: std::cell::Cell<Option<PathBuf>> =
            const { std::cell::Cell::new(None) };
    }

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
                egui::vec2(width, TEST_HEIGHT),
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
            for name in ["File", "Open", "Save", "Print", "Edit", "View"] {
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
        searched(&mut app, &ctx);
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

        // Text being added on the page, and a line of page text edited there; the page's
        // top in view (the search went to its match).
        app.act(&ctx, Act::FirstPage);
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
            assert_eq!(app.settings.sign.add(role, e), None);
        }
        app.act(&ctx, Act::Tool(Tool::Sign));
        clear_names(&mut app, &ctx, "Fill and sign", &mut bad);
        let tree = settled_at(&mut app, &ctx, 1400.0);
        let initials = node(&tree, "Initials 1");
        assert!(initials.is_some(), "no saved initials shown");
        app.band = Band::Forms;
        click_named(&mut app, &ctx, "Fill and sign choices");
        let tree = settled_at(&mut app, &ctx, 1400.0);
        assert!(egui::Popup::is_any_open(&ctx), "the menu isn't open");
        note_unclear(&tree, "Fill and sign choices", &mut bad);
        app.act(&ctx, Act::Tool(Tool::Read));
        app.band = Band::Edit;

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
        click_named(&mut app, &ctx, "File");
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
            let id = egui::Id::unique("palette");
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

    /// BUG-34 (audit A18): searching "dark" lists the theme's Dark and the page colours'
    /// Dark under their submenus' names, for screen readers too, and Enter runs the first.
    #[test]
    fn command_search_names_each_command_with_its_submenu() {
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        theme::install(&ctx);
        let mut app = app_with(&ctx, 1);
        app.act(&ctx, Act::Palette);
        settled_at(&mut app, &ctx, 1400.0);
        let typed = egui::Event::Text("dark".into());
        window(&mut app, &ctx, 1400.0, vec![typed]);
        let tree = settled_at(&mut app, &ctx, 1400.0);
        let theme = node(&tree, "Theme: Dark").expect("the theme's Dark").0;
        let page = node(&tree, "Page colours: Dark").expect("Dark pages").0;
        assert!(theme.top() < page.top());
        assert!(node(&tree, "Dark").is_none(), "a row named only Dark");
        let enter = key_with(Key::Enter, egui::Modifiers::NONE);
        window(&mut app, &ctx, 1400.0, vec![enter]);
        assert!(app.palette.is_none());
        assert_eq!(app.settings.theme, egui::ThemePreference::Dark);
        use crate::settings::PageLook;
        let dark_pages = PageLook::Fixed(limarepdf_core::page_colours::PageColours::Dark);
        assert_ne!(app.settings.page_look, dark_pages);
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

        // As Edit's Font group lays them out: the font and size, then the styles.
        let reached = tab_walk(&mut app, &ctx, 1400.0, 80);
        let look = [
            "Font",
            "Font size",
            "Font sizes",
            "Bold",
            "Italic",
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
            assert_eq!(app.settings.sign.add(SignRole::Signature, e), None);
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
        // (The strip's view of the document's signatures is no signing.)
        let digital = names
            .iter()
            .flatten()
            .find(|n| n.to_lowercase().contains("digital") && *n != "Digital signatures");
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
        let mut reached = tab_walk(&mut app, &ctx, 1400.0, 36);
        // Not the zoom level, which depends on the window.
        reached.retain(|name| !name.ends_with('%'));
        // Redo is disabled, as nothing was undone, and Paste, Copy and the Font group as
        // nothing is typed or selected; planned features are no Tab stops. The status
        // bar's view buttons and zoom (NEW-14) before the strip; the thumbnails are one
        // stop, their list.
        let bars = [
            "Untitled, unsaved",
            "Close Untitled",
            "File",
            "Open",
            "Save",
            "Undo",
            "Edit",
            "Insert",
            "Review",
            "Pages",
            "Forms and sign",
            "Protect",
            "View",
            "Print",
            "Compact commands",
            "Edit text",
            "Find",
            "Select",
            "Paragraph",
            "Show planned features",
            "Page view",
            "Organise pages",
            "Zoom out",
            "Zoom",
            "Zoom in",
            "Pages",
            "Search results",
            "Bookmarks",
            "Comments",
            "Digital signatures",
            "Page thumbnails",
        ];
        assert_eq!(reached[..bars.len()], bars);
    }

    /// The status bar's zoom stays where it is as it is used: its menu says the zoom alone,
    /// in room for any zoom, so Zoom in and out don't move when the zoom stops following the
    /// width (the user's find, batch 41) or gains a figure. Fit width is ticked in its menu.
    #[test]
    fn the_status_bar_zoom_stays_put_when_zooming() {
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        theme::install(&ctx);
        let ty = Typist::new(&ctx);
        let mut app = app_with(&ctx, 1);
        let tree = ty.pause(&mut app);
        assert!(app.tabs[0].fit, "not fitting the width at first");
        let said = |tree: &egui::accesskit::TreeUpdate| -> Vec<String> {
            (tree.nodes.iter())
                .filter_map(|(_, n)| n.label().map(str::to_string))
                .filter(|l| l.starts_with("Zoom ") && l.ends_with('%'))
                .collect()
        };
        let menu = said(&tree);
        assert!(menu.len() == 1 && !menu[0].contains("Fit"), "{menu:?}");
        let (zoom_in, _) = node(&tree, "Zoom in").expect("Zoom in");
        let p = zoom_in.center();
        // Zoomed in past 100%, then much further: the buttons stay.
        for clicks in [1, 4] {
            for _ in 0..clicks {
                ty.click(&mut app, p);
            }
            let tree = ty.pause(&mut app);
            let (now, _) = node(&tree, "Zoom in").expect("Zoom in");
            assert_eq!(now, zoom_in, "Zoom in moved, at {:?}", said(&tree));
        }
        assert!(!app.tabs[0].fit);
    }

    /// The name of the control with the keyboard in `tree`; `None` with nothing.
    fn focused_name(ctx: &egui::Context, tree: &egui::accesskit::TreeUpdate) -> Option<String> {
        ctx.memory(|m| m.focused())?;
        let (_, n) = tree.nodes.iter().find(|(id, _)| *id == tree.focus)?;
        n.label().map(str::to_string)
    }

    /// Press F6 (Shift+F6 when `back`) in the window `width` points wide; the name of the
    /// control that then has the keyboard, `None` with nothing.
    fn f6_at(app: &mut App, ctx: &egui::Context, width: f32, back: bool) -> Option<String> {
        let shift = if back {
            egui::Modifiers::SHIFT
        } else {
            egui::Modifiers::NONE
        };
        window(app, ctx, width, vec![key_with(Key::F6, shift)]);
        let tree = window(app, ctx, width, vec![]);
        focused_name(ctx, &tree)
    }

    fn f6(app: &mut App, ctx: &egui::Context, back: bool) -> Option<String> {
        f6_at(app, ctx, 1400.0, back)
    }

    /// ENH-10: F6 goes round the window's regions in the order they are on screen, and
    /// Shift+F6 back, each time to the control last used there, else the first; the page
    /// takes the keys with nothing focused, outlined. With nothing focused, the keys are
    /// the page's, so F6 goes on from there.
    #[test]
    fn f6_goes_round_the_regions_of_the_window() {
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        theme::install(&ctx);
        let mut app = app_with(&ctx, 1);
        settled_at(&mut app, &ctx, 1400.0);
        let round = [
            Some("Collapse the inspector"),
            Some("Show planned features"),
            Some("Untitled, unsaved"),
            Some("File"),
            Some("Pages"),
            None,
            Some("Collapse the inspector"),
        ];
        for want in round {
            assert_eq!(f6(&mut app, &ctx, false).as_deref(), want);
        }
        assert!(
            !app.tabs[0].page_ring,
            "outlined with the keyboard elsewhere"
        );
        assert_eq!(f6(&mut app, &ctx, true), None);
        assert!(app.tabs[0].page_ring, "the page isn't outlined");
        // A click takes the outline away, as anything taking the keyboard does.
        let tree = settled_at(&mut app, &ctx, 1400.0);
        let p = node(&tree, "Page 1").expect("the page").0.center();
        window(&mut app, &ctx, 1400.0, click_at(p));
        settled_at(&mut app, &ctx, 1400.0);
        assert!(!app.tabs[0].page_ring, "still outlined after a click");

        // Back the other way, to where the keyboard was last in each region.
        let side = Some("Pages");
        assert_eq!(f6(&mut app, &ctx, true).as_deref(), side);
        assert_eq!(f6(&mut app, &ctx, true).as_deref(), Some("File"));
        let tab = key_with(Key::Tab, egui::Modifiers::NONE);
        window(&mut app, &ctx, 1400.0, vec![tab]);
        let tree = window(&mut app, &ctx, 1400.0, vec![]);
        assert_eq!(focused_name(&ctx, &tree).as_deref(), Some("Open"));
        let doc = Some("Untitled, unsaved");
        assert_eq!(f6(&mut app, &ctx, true).as_deref(), doc);
        assert_eq!(f6(&mut app, &ctx, false).as_deref(), Some("Open"));

        // A notice is a region of its own, between the commands and navigation.
        app.tabs[0].show_banner(crate::banners::changed_on_disk());
        settled_at(&mut app, &ctx, 1400.0);
        assert_eq!(f6(&mut app, &ctx, false).as_deref(), Some("Reload"));
        assert_eq!(f6(&mut app, &ctx, false).as_deref(), side);
    }

    /// ENH-10: in a narrow window the folded panels' strips are regions in their place;
    /// with no document open, F6 goes between the top bar and the start screen.
    #[test]
    fn f6_goes_round_narrow_windows_and_the_start_screen() {
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        theme::install(&ctx);
        let mut app = app_with(&ctx, 1);
        settled_at(&mut app, &ctx, 700.0);
        assert!(!app.side.shown() && !app.inspector.shown());
        // The side panel folded away, its strip stays, to bring it back.
        let round = [
            Some("Expand the inspector"),
            Some("Show planned features"),
            Some("Untitled, unsaved"),
            Some("File"),
            Some("Pages"),
            None,
        ];
        for want in round {
            assert_eq!(f6_at(&mut app, &ctx, 700.0, false).as_deref(), want);
        }

        let mut app = App::with(Settings::from_file(None), Recent::from_file(None));
        settled_at(&mut app, &ctx, 1400.0);
        assert_eq!(f6(&mut app, &ctx, false).as_deref(), Some("File"));
        assert_eq!(f6(&mut app, &ctx, false).as_deref(), Some("Open PDF…"));
        assert_eq!(f6(&mut app, &ctx, false).as_deref(), Some("File"));
        assert_eq!(f6(&mut app, &ctx, true).as_deref(), Some("Open PDF…"));
    }

    /// ENH-10: F6 leaves typing open on the page (new text, a line being edited, a new
    /// note or text box), and coming back gives it the keyboard again, to go on typing.
    #[test]
    fn f6_leaves_typing_open_on_the_page_and_comes_back_to_it() {
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        theme::install(&ctx);
        let none = egui::Modifiers::NONE;
        let ty = Typist::new(&ctx);
        for case in ["new text", "line on the page", "new note", "new text box"] {
            let mut app = App::with(Settings::from_file(None), Recent::from_file(None));
            let note = typing_doc(&mut app, &ctx);
            start_typing(&mut app, &ty, case, note);
            ty.text(&mut app, "Seal");
            let typing = app.tabs[0].ed.typing_id().expect("typing");
            assert_eq!(ctx.memory(|m| m.focused()), Some(typing), "{case}");
            let tree = ty.key(&mut app, Key::F6, none);
            ty.pause(&mut app);
            let away = focused_name(&ctx, &tree);
            // The inspector's first control: its heading's pin, with a tool that puts
            // things down (batch 42), else its collapse button.
            let first = match app.tabs[0].ed.keeps_pin() {
                true => "Keep tool active",
                false => "Collapse the inspector",
            };
            assert_eq!(away.as_deref(), Some(first), "{case}");
            assert_eq!(app.tabs[0].ed.typing_id(), Some(typing), "{case}: ended");
            let tree = ty.key(&mut app, Key::F6, egui::Modifiers::SHIFT);
            assert_eq!(
                ctx.memory(|m| m.focused()),
                Some(typing),
                "{case}: not back"
            );
            assert!(!app.tabs[0].page_ring, "{case}: outlined while typing");
            assert_eq!(focused_text(&tree).as_deref(), Some("Seal"), "{case}");
            ty.text(&mut app, "s");
            let tree = ty.pause(&mut app);
            assert_eq!(focused_text(&tree).as_deref(), Some("Seals"), "{case}");
            ty.key(&mut app, Key::Escape, none);
            ty.pause(&mut app);
            let lines: Vec<String> = app.tabs[0]
                .pdf
                .page_objects(0)
                .unwrap()
                .into_iter()
                .filter_map(|o| o.text)
                .collect();
            let kept = lines.contains(&"Seals".to_string())
                || comments(&app).contains(&"Seals".to_string());
            assert!(kept, "{case}: {lines:?} {:?}", comments(&app));
        }
    }

    /// BUG-41: typing whose change can't be made isn't lost. A line's typing ended (Esc, or
    /// a click away) when its change fails comes back on the page to go on with, the reason
    /// shown and the document as it was; ended again unchanged, it is left out, and that is
    /// said.
    #[test]
    fn typing_whose_change_fails_comes_back_to_go_on_with() {
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        theme::install(&ctx);
        let none = egui::Modifiers::NONE;
        let ty = Typist::new(&ctx);
        for end in ["Esc", "a click away"] {
            let mut app = App::with(Settings::from_file(None), Recent::from_file(None));
            let note = typing_doc(&mut app, &ctx);
            let (min, k) = ty.page(&mut app);
            start_typing(&mut app, &ty, "line on the page", note);
            ty.text(&mut app, "Seal");
            let typing = app.tabs[0].ed.typing_id().expect("typing");
            // The line goes from the document behind the editor's back: its change fails.
            let t = &mut app.tabs[0];
            let objs = t.pdf.page_objects(0).unwrap();
            let line = objs.iter().position(|o| o.text.as_deref() == Some("Otter"));
            t.pdf.delete_objects(0, &[line.expect("the line")]).unwrap();
            let (state, lines) = (t.state(), line_texts(&t.pdf, 0));
            match end {
                "Esc" => drop(ty.key(&mut app, Key::Escape, none)),
                _ => ty.click(&mut app, min + egui::vec2(300.0, 600.0) * k),
            }
            ty.pause(&mut app);
            assert_eq!(message(&app).0, "Error", "{end}");
            // The message closed as it was opened: by Esc, or a click.
            let tree = ty.pause(&mut app);
            match end {
                "Esc" => drop(ty.key(&mut app, Key::Escape, none)),
                _ => ty.click(&mut app, middle(&tree, "OK")),
            }
            let tree = ty.pause(&mut app);
            assert!(
                matches!(app.dialog, Dialog::None),
                "{end}: the message stayed"
            );
            assert_eq!(
                app.tabs[0].ed.typing_id(),
                Some(typing),
                "{end}: typing lost"
            );
            assert_eq!(focused_text(&tree).as_deref(), Some("Seal"), "{end}");
            let t = &app.tabs[0];
            assert!(t.state() == state, "{end}: changed");
            assert_eq!(line_texts(&t.pdf, 0), lines, "{end}");
            // Ended again as it is: left out, and said.
            ty.key(&mut app, Key::Escape, none);
            ty.pause(&mut app);
            assert_eq!(app.tabs[0].ed.typing_id(), None, "{end}: still typing");
            let said = toast(&app).unwrap_or_default();
            assert!(said.contains("left out"), "{end}: {said:?}");
            let t = &app.tabs[0];
            assert!(t.state() == state, "{end}: changed after");
            assert_eq!(line_texts(&t.pdf, 0), lines, "{end}");
        }
    }

    /// ENH-39: typing open on the page (new text, a line being edited, a new note or text
    /// box) waits while another tab is in front, and has the keyboard again when its tab
    /// comes back, to go on typing, as a word processor's document does.
    #[test]
    fn typing_left_in_another_tab_has_the_keyboard_again_on_return() {
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        theme::install(&ctx);
        let none = egui::Modifiers::NONE;
        let ty = Typist::new(&ctx);
        let to_tab = |app: &mut App, name: &str| {
            let tree = ty.pause(app);
            ty.click(app, middle(&tree, name));
        };
        for case in ["new text", "line on the page", "new note", "new text box"] {
            let mut app = App::with(Settings::from_file(None), Recent::from_file(None));
            let note = typing_doc(&mut app, &ctx);
            pages_tab(&mut app, &ctx, 1);
            app.tabs[0].path = Some(PathBuf::from("/nonexistent/seal.pdf"));
            app.tabs[1].path = Some(PathBuf::from("/nonexistent/otter.pdf"));
            app.active = 0;
            start_typing(&mut app, &ty, case, note);
            ty.text(&mut app, "Seal");
            let typing = app.tabs[0].ed.typing_id().expect("typing");
            to_tab(&mut app, "otter.pdf");
            assert_eq!(app.active, 1, "{case}");
            to_tab(&mut app, "seal.pdf, unsaved");
            assert_eq!(app.active, 0, "{case}");
            assert_eq!(app.tabs[0].ed.typing_id(), Some(typing), "{case}: ended");
            assert_eq!(
                ctx.memory(|m| m.focused()),
                Some(typing),
                "{case}: not back"
            );
            // Its typing's history came back with it (ENH-32).
            let tree = ty.key(&mut app, Key::Z, CTRL);
            let undone = focused_text(&tree);
            assert_ne!(undone.as_deref(), Some("Seal"), "{case}: no undo");
            let tree = ty.key(&mut app, Key::Y, CTRL);
            let redone = focused_text(&tree);
            assert_eq!(redone.as_deref(), Some("Seal"), "{case}: no redo");
            ty.text(&mut app, "s");
            let tree = ty.pause(&mut app);
            assert_eq!(focused_text(&tree).as_deref(), Some("Seals"), "{case}");
            ty.key(&mut app, Key::Escape, none);
            ty.pause(&mut app);
            let lines: Vec<String> = app.tabs[0]
                .pdf
                .page_objects(0)
                .unwrap()
                .into_iter()
                .filter_map(|o| o.text)
                .collect();
            let kept = lines.contains(&"Seals".to_string())
                || comments(&app).contains(&"Seals".to_string());
            assert!(kept, "{case}: {lines:?} {:?}", comments(&app));
        }
    }

    /// ENH-10, ENH-38: the band's Font group, reached by F6 and Tab, changes the typing left
    /// open on the page, as with the mouse: Bold for the words selected there.
    #[test]
    fn the_band_changes_typing_left_open_by_f6() {
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        theme::install(&ctx);
        let none = egui::Modifiers::NONE;
        let ty = Typist::new(&ctx);
        for (case, word) in [("new text", "Seal"), ("line on the page", "Otter")] {
            let mut app = App::with(Settings::from_file(None), Recent::from_file(None));
            let note = typing_doc(&mut app, &ctx);
            start_typing(&mut app, &ty, case, note);
            if case == "new text" {
                ty.text(&mut app, word);
                ty.key(&mut app, Key::A, egui::Modifiers::COMMAND);
            }
            // Round to the commands, then along them.
            let mut tree = ty.key(&mut app, Key::F6, none);
            for _ in 0..8 {
                if focused_name(&ctx, &tree).as_deref() == Some("File") {
                    break;
                }
                tree = ty.key(&mut app, Key::F6, none);
            }
            for _ in 0..30 {
                if focused_name(&ctx, &tree).as_deref() == Some("Bold") {
                    break;
                }
                tree = ty.key(&mut app, Key::Tab, none);
            }
            assert_eq!(focused_name(&ctx, &tree).as_deref(), Some("Bold"), "{case}");
            ty.key(&mut app, Key::Space, none);
            ty.pause(&mut app);
            assert!(app.tabs[0].ed.typing_id().is_some(), "{case}: ended");
            ty.key(&mut app, Key::Escape, none);
            ty.pause(&mut app);
            let fonts = line_fonts(&app.tabs[0].pdf, 0);
            let bold = fonts
                .iter()
                .flatten()
                .any(|(t, f)| t.trim() == word && f.ends_with("Bold"));
            assert!(bold, "{case}: {fonts:?}");
        }
    }

    /// ENH-10: in a form, Tab moves between the fields and F6 leaves them, keeping what was
    /// typed; F6 back returns to the field left, to go on typing.
    #[test]
    fn f6_leaves_a_form_and_comes_back_to_the_field() {
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        theme::install(&ctx);
        let none = egui::Modifiers::NONE;
        let mut app = App::with(Settings::from_file(None), Recent::from_file(None));
        let pdf = form_doc(&[("name", 1), ("city", 1)]);
        assert!(app.add_tab(&ctx, pdf, None, None, false));
        let ty = Typist::new(&ctx);
        ty.click_field(&mut app, "name", 0);
        ty.text(&mut app, "Ada");
        ty.key(&mut app, Key::Tab, none);
        ty.text(&mut app, "Os");
        let tree = ty.key(&mut app, Key::F6, none);
        ty.pause(&mut app);
        let away = focused_name(&ctx, &tree);
        assert_eq!(away.as_deref(), Some("Collapse the inspector"));
        assert_eq!(saved_field(&app.tabs[0], "name"), "Ada");
        assert_eq!(saved_field(&app.tabs[0], "city"), "Os");
        let tree = ty.key(&mut app, Key::F6, egui::Modifiers::SHIFT);
        assert_eq!(focused_text(&tree).as_deref(), Some("Os"));
        ty.text(&mut app, "lo");
        ty.key(&mut app, Key::Tab, none);
        ty.pause(&mut app);
        assert_eq!(saved_field(&app.tabs[0], "city"), "Oslo");
    }

    /// ENH-10: F6 does nothing while a dialog, the command search or a menu is open: they
    /// keep the keyboard until closed.
    #[test]
    fn f6_waits_for_dialogs_and_menus() {
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        theme::install(&ctx);
        let mut app = app_with(&ctx, 1);
        let esc = key_with(Key::Escape, egui::Modifiers::NONE);
        app.act(&ctx, Act::Palette);
        settled_at(&mut app, &ctx, 1400.0);
        assert_eq!(f6(&mut app, &ctx, false).as_deref(), Some("Command"));
        assert!(app.palette.is_some());
        window(&mut app, &ctx, 1400.0, vec![esc.clone()]);

        app.dialog = Dialog::Message("Walrus".into(), "The seal ate the fish.".into());
        let tree = settled_at(&mut app, &ctx, 1400.0);
        let before = focused_name(&ctx, &tree);
        assert_eq!(f6(&mut app, &ctx, false), before);
        assert!(matches!(app.dialog, Dialog::Message(..)));
        app.dialog = Dialog::None;
        settled_at(&mut app, &ctx, 1400.0);

        click_named(&mut app, &ctx, "File");
        settled_at(&mut app, &ctx, 1400.0);
        assert!(egui::Popup::is_any_open(&ctx), "the menu didn't open");
        f6(&mut app, &ctx, false);
        assert!(egui::Popup::is_any_open(&ctx), "F6 closed the menu");
    }

    /// ENH-10: each region is named for screen readers, with its role, and the document's
    /// tab says it is the one shown.
    #[test]
    fn regions_are_named_for_screen_readers() {
        use egui::accesskit::Role;
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        theme::install(&ctx);
        let mut app = app_with(&ctx, 1);
        app.tabs[0].show_banner(crate::banners::changed_on_disk());
        let tree = settled_at(&mut app, &ctx, 1400.0);
        let has = |role: Role, label: &str| {
            tree.nodes
                .iter()
                .any(|(_, n)| n.role() == role && n.label() == Some(label))
        };
        for (role, label) in [
            (Role::TabList, "Tabs"),
            (Role::Toolbar, "Edit commands"),
            (Role::Region, "Notices"),
            (Role::Navigation, "Navigation"),
            (Role::Navigation, "Pages"),
            (Role::Main, "Document"),
            (Role::Complementary, "Inspector"),
            (Role::Status, "Status bar"),
        ] {
            assert!(has(role, label), "no {role:?} named {label}");
        }
        let (_, tab) = tree
            .nodes
            .iter()
            .find(|(_, n)| n.label() == Some("Untitled, unsaved"))
            .expect("the tab");
        assert_eq!((tab.role(), tab.is_selected()), (Role::Tab, Some(true)));
        // The inspector's pop-out over the pages, from its strip in a narrow window.
        app.popout = true;
        let tree = settled_at(&mut app, &ctx, 700.0);
        let inspectors = tree
            .nodes
            .iter()
            .filter(|(_, n)| n.role() == Role::Complementary && n.label() == Some("Inspector"))
            .count();
        assert_eq!(inspectors, 2, "the strip and the pop-out");
    }

    /// ENH-11: the edges of the side panel and the inspector, reached by Tab, move from
    /// the keyboard: the arrow keys 10 points, 50 with Shift, Home to the narrowest and
    /// End to half the window, never folding the panel away; the keyboard stays on them.
    #[test]
    fn panel_edges_move_from_the_keyboard() {
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        theme::install(&ctx);
        let mut app = app_with(&ctx, 1);
        settled_at(&mut app, &ctx, 1400.0);
        let (none, shift) = (egui::Modifiers::NONE, egui::Modifiers::SHIFT);
        // Each panel's width from where its edge is: the side panel's after the strip, the
        // inspector's from the window's right.
        for (edge, wider, narrower, width) in [
            (
                "Side panel width",
                Key::ArrowRight,
                Key::ArrowLeft,
                (|x: f32| x - RAIL_WIDTH) as fn(f32) -> f32,
            ),
            ("Inspector width", Key::ArrowLeft, Key::ArrowRight, |x| {
                1400.0 - x
            }),
        ] {
            let mut tree = window(&mut app, &ctx, 1400.0, vec![]);
            for _ in 0..80 {
                if focused_name(&ctx, &tree).as_deref() == Some(edge) {
                    break;
                }
                window(&mut app, &ctx, 1400.0, vec![key_with(Key::Tab, none)]);
                tree = window(&mut app, &ctx, 1400.0, vec![]);
            }
            assert_eq!(focused_name(&ctx, &tree).as_deref(), Some(edge), "Tab");
            let start = width(node(&tree, edge).expect("the edge").0.center().x);
            let mut press = |key: Key, modifiers: egui::Modifiers| {
                window(&mut app, &ctx, 1400.0, vec![key_with(key, modifiers)]);
                let tree = window(&mut app, &ctx, 1400.0, vec![]);
                let (at, focused) = node(&tree, edge).expect("the edge");
                assert!(focused, "{edge} {key:?}: the keyboard left it");
                width(at.center().x)
            };
            for (key, modifiers, want) in [
                (wider, none, start + 10.0),
                (wider, shift, start + 60.0),
                (narrower, none, start + 50.0),
            ] {
                let got = press(key, modifiers);
                let why = format!("{edge} {key:?}: {got}, not {want}");
                assert!((got - want).abs() < 0.5, "{why}");
            }
            // Half the window; the edge's place is a few points off its panel's border, at
            // most its grab width.
            let end = press(Key::End, none);
            assert!((end - 700.0).abs() <= 8.0, "{edge} End: {end}");
            // As narrow as the panel goes (the inspector no narrower than its contents),
            // and no further.
            let home = press(Key::Home, none);
            // (The inspector starts as narrow as the document's sections let it be.)
            let narrowest = home <= start && home >= panels::NARROWEST - 8.0;
            assert!(narrowest, "{edge} Home: {home}");
            let below = press(narrower, shift);
            assert_eq!(below, home, "{edge}: narrower than the narrowest");
        }
        let open = app.side.shown() && app.inspector.shown();
        assert!(open, "a panel folded away");
    }

    /// ENH-11: the outline that moves new text with the mouse is no stop for the keyboard;
    /// as in word processors, Esc keeps the text, selected, and the arrow keys move it.
    #[test]
    fn new_text_moves_from_the_keyboard_once_kept() {
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        theme::install(&ctx);
        let mut app = app_with(&ctx, 1);
        let (min, k) = page_on_screen(&mut app, &ctx);
        app.act(&ctx, Act::Tool(Tool::Text));
        let at = min + egui::vec2(100.0, 200.0) * k;
        window(&mut app, &ctx, 1400.0, click_at(at));
        settled_at(&mut app, &ctx, 1400.0);
        let typed = egui::Event::Text("Seal".into());
        window(&mut app, &ctx, 1400.0, vec![typed]);
        let tree = settled_at(&mut app, &ctx, 1400.0);
        let (_, grip) = tree
            .nodes
            .iter()
            .find(|(_, n)| n.label() == Some("Move the text"))
            .expect("the outline");
        let stop = grip.supports_action(egui::accesskit::Action::Focus);
        assert!(!stop, "the outline is a stop for Tab");
        let none = egui::Modifiers::NONE;
        window(&mut app, &ctx, 1400.0, vec![key_with(Key::Escape, none)]);
        settled_at(&mut app, &ctx, 1400.0);
        let before = boxes(&app, 0);
        assert!(held(&app).is_some(), "the new text isn't selected");
        let right = key_with(Key::ArrowRight, none);
        window(&mut app, &ctx, 1400.0, vec![right]);
        settled_at(&mut app, &ctx, 1400.0);
        let after = boxes(&app, 0);
        let (b, a) = (before.last().unwrap(), after.last().unwrap());
        assert!((a[0] - b[0] - 1.0).abs() < 0.01, "{b:?} to {a:?}");
    }

    /// Esc while new text is being moved by its outline cancels the move, as Esc cancels a
    /// drag under way: the text goes back where it was, still being typed.
    #[test]
    fn esc_while_moving_new_text_puts_it_back() {
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        theme::install(&ctx);
        let mut app = app_with(&ctx, 1);
        let (min, k) = page_on_screen(&mut app, &ctx);
        app.act(&ctx, Act::Tool(Tool::Text));
        let at = min + egui::vec2(100.0, 200.0) * k;
        window(&mut app, &ctx, 1400.0, click_at(at));
        settled_at(&mut app, &ctx, 1400.0);
        let typed = egui::Event::Text("Seal".into());
        window(&mut app, &ctx, 1400.0, vec![typed]);
        let tree = settled_at(&mut app, &ctx, 1400.0);
        let place = |app: &App| app.tabs[0].ed.adding.as_ref().map(|a| a.at);
        let start = place(&app).expect("the new text");
        // On the band around the outline, 3 to 6 points out from the text, at its left.
        let (text, _) = node(&tree, page_text::NAME).expect("the new text");
        let p = egui::pos2(text.min.x - 4.5, text.center().y);
        window(
            &mut app,
            &ctx,
            1400.0,
            vec![egui::Event::PointerMoved(p), press(p, true)],
        );
        for i in 1..=4 {
            let to = p + egui::vec2(10.0, 8.0) * i as f32;
            window(&mut app, &ctx, 1400.0, vec![egui::Event::PointerMoved(to)]);
        }
        assert_ne!(place(&app), Some(start), "not moved");
        let esc = key_with(Key::Escape, egui::Modifiers::NONE);
        window(&mut app, &ctx, 1400.0, vec![esc]);
        let to = p + egui::vec2(40.0, 32.0);
        window(&mut app, &ctx, 1400.0, vec![press(to, false)]);
        settled_at(&mut app, &ctx, 1400.0);
        assert_eq!(place(&app), Some(start), "not put back, or not still typed");
    }

    /// ENH-12: Create signature and Create initials are in the command search; run from
    /// there they open the Create window with Fill and sign on. Not among the top bar's
    /// tools.
    #[test]
    fn create_signature_is_in_the_command_search() {
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        theme::install(&ctx);
        let mut app = app_with(&ctx, 1);
        let enter = key_with(Key::Enter, egui::Modifiers::NONE);
        for (query, role) in [
            ("create signature", limarepdf_core::SignRole::Signature),
            ("create initials", limarepdf_core::SignRole::Initials),
        ] {
            app.act(&ctx, Act::Palette);
            settled_at(&mut app, &ctx, 1400.0);
            let typed = egui::Event::Text(query.into());
            window(&mut app, &ctx, 1400.0, vec![typed]);
            settled_at(&mut app, &ctx, 1400.0);
            window(&mut app, &ctx, 1400.0, vec![enter.clone()]);
            settled_at(&mut app, &ctx, 1400.0);
            assert!(app.palette.is_none(), "{query}: not run");
            let ed = &mut app.tabs[0].ed;
            assert_eq!(ed.tool, Tool::Sign, "{query}");
            let open = ed.sign.create.take().map(|(c, _)| c.role);
            assert_eq!(open, Some(role), "{query}: no Create window");
        }
        // And in Forms and sign, Fill and sign's menu has them under Signature and Initials.
        app.act(&ctx, Act::Tool(Tool::Read));
        click_named(&mut app, &ctx, "Forms and sign");
        click_named(&mut app, &ctx, "Fill and sign choices");
        let tree = settled_at(&mut app, &ctx, 1400.0);
        assert!(node(&tree, "Signature").is_some(), "the menu didn't open");
        assert!(node(&tree, "Initials").is_some(), "no Initials");
    }

    /// ENH-38: each of the band's names, a tab that says it is the one shown, shows its
    /// groups, with no names under them.
    #[test]
    fn the_band_shows_each_names_groups() {
        use egui::accesskit::Role;
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        theme::install(&ctx);
        let mut app = app_with(&ctx, 1);
        // One row under the tabs: the document's commands, the names, then Print and the
        // compact commands button at the right edge.
        let tree = settled_at(&mut app, &ctx, 1400.0);
        let at = |name: &str| node(&tree, name).expect(name).0;
        let (file, edit, print) = (at("File"), at("Edit"), at("Print"));
        let (caret, doc) = (at("Compact commands"), at("Untitled, unsaved"));
        for r in [edit, print, caret] {
            let level = (r.center().y - file.center().y).abs() < 4.0;
            assert!(level, "{r:?} beside {file:?}");
        }
        assert!(doc.bottom() <= file.top(), "the tabs aren't above");
        assert!(file.right() < edit.left() && edit.right() < print.left());
        assert!(print.right() <= caret.left() && caret.right() > 1350.0);
        let tab = |tree: &egui::accesskit::TreeUpdate, name: &str| {
            let (_, n) = (tree.nodes.iter())
                .find(|(_, n)| n.role() == Role::Tab && n.label() == Some(name))
                .expect("the band's name");
            n.clone()
        };
        for band in Band::ALL {
            // By its tab: the side panel is called Pages too.
            let tree = settled_at(&mut app, &ctx, 1400.0);
            let b = tab(&tree, band.name()).bounds().expect("its place");
            let p = egui::pos2((b.x0 + b.x1) as f32 / 2.0, (b.y0 + b.y1) as f32 / 2.0);
            window(&mut app, &ctx, 1400.0, click_at(p));
            assert_eq!(app.band, band);
            let tree = settled_at(&mut app, &ctx, 1400.0);
            let shown = tab(&tree, band.name()).is_selected();
            assert_eq!(shown, Some(true), "{}", band.name());
            let label = format!("{} commands", band.name());
            let named = (tree.nodes.iter()).any(|(_, n)| n.label() == Some(label.as_str()));
            assert!(named, "{label}");
            // The names and one row of groups above the panels, as short as the Font
            // group's two rows allow.
            let (below, _) = node(&tree, "Collapse the inspector").expect("the inspector");
            let tall = below.top() - b.y0 as f32;
            assert!(tall < 125.0, "{label}: {tall}");
            for g in band.groups() {
                let first = g.first_name();
                assert!(node(&tree, &first).is_some(), "{}: no {first}", band.name());
            }
        }
    }

    /// Compact commands, from the button at the right of the band's names: the groups on one
    /// row, their buttons icons named for screen readers and on hover; a short window makes
    /// them compact by itself, its button greyed meanwhile.
    #[test]
    fn compact_commands_put_the_band_on_one_row() {
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        theme::install(&ctx);
        let mut app = app_with(&ctx, 1);
        // From the band's names to the inspector below the commands.
        let depth = |tree: &egui::accesskit::TreeUpdate| {
            let top = node(tree, "Edit").expect("Edit").0.top();
            let (below, _) = node(tree, "Collapse the inspector").expect("the inspector");
            below.top() - top
        };
        let tree = settled_at(&mut app, &ctx, 1400.0);
        let (full, find) = (depth(&tree), node(&tree, "Find").expect("Find").0);
        click_named(&mut app, &ctx, "Compact commands");
        assert!(app.settings.compact_commands);
        let tree = settled_at(&mut app, &ctx, 1400.0);
        let compact = depth(&tree);
        assert!(compact < full - 20.0, "{compact} beside {full}");
        let small = node(&tree, "Find").expect("Find named").0;
        let narrower = small.width() < find.width() - 20.0;
        assert!(narrower, "{small:?} beside {find:?}");
        click_named(&mut app, &ctx, "Compact commands");
        assert!(!app.settings.compact_commands);

        // A short window: compact, the button greyed.
        let short = |app: &mut App| {
            let raw = || egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1400.0, 560.0),
                )),
                ..Default::default()
            };
            let mut f = eframe::Frame::_new_kittest();
            let mut tree = None;
            for _ in 0..5 {
                let mut out = ctx.run_ui(raw(), |ui| eframe::App::ui(app, ui, &mut f));
                out.textures_delta.clear();
                tree = out.platform_output.accesskit_update;
            }
            tree.expect("the window")
        };
        let tree = short(&mut app);
        assert!(depth(&tree) < full - 20.0, "full in a short window");
        let (_, button) = (tree.nodes.iter())
            .find(|(_, n)| n.label() == Some("Compact commands"))
            .expect("the button");
        assert!(button.is_disabled(), "the button isn't greyed");
        assert!(!app.settings.compact_commands, "the choice changed");
    }

    /// ENH-41: planned features show greyed in their groups, named as planned; a click on
    /// one does nothing and they are no Tab stops. Show planned features hides them, kept
    /// in the settings, and the built commands stay where they were.
    #[test]
    fn planned_features_show_greyed_and_do_nothing() {
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        theme::install(&ctx);
        let mut app = app_with(&ctx, 1);
        let tree = settled_at(&mut app, &ctx, 1400.0);
        let name = "Replace — Planned";
        let (at, _) = node(&tree, name).expect("Replace");
        let replace = (tree.nodes.iter()).find(|(_, n)| n.label() == Some(name));
        assert!(replace.is_some_and(|(_, n)| n.is_disabled()), "not greyed");
        let (paste, _) = node(&tree, "Paste").expect("Paste");
        let undo = app.tabs[0].can_undo();
        window(&mut app, &ctx, 1400.0, click_at(at.center()));
        settled_at(&mut app, &ctx, 1400.0);
        let t = &app.tabs[0];
        assert!(matches!(app.dialog, Dialog::None), "a dialog opened");
        assert!(!egui::Popup::is_any_open(&ctx), "a menu opened");
        let now = (t.ed.tool, t.can_undo(), t.side);
        assert_eq!(now, (Tool::Read, undo, Side::Pages));
        let stops = tab_walk(&mut app, &ctx, 1400.0, 40);
        assert!(!stops.iter().any(|n| n.ends_with("Planned")), "{stops:?}");
        let planned = |n: &Option<String>| n.as_deref().is_some_and(|n| n.ends_with("Planned"));

        click_named(&mut app, &ctx, "Show planned features");
        assert!(!app.settings.show_planned);
        for band in Band::ALL {
            app.band = band;
            let tree = settled_at(&mut app, &ctx, 1400.0);
            let shown = (tree.nodes.iter()).find(|(_, n)| planned(&n.label().map(str::to_string)));
            assert!(shown.is_none(), "{}: {shown:?}", band.name());
        }
        app.band = Band::Edit;
        let tree = settled_at(&mut app, &ctx, 1400.0);
        // Those before the planned ones stay where they were.
        let now = node(&tree, "Paste").map(|(r, _)| r);
        assert_eq!(now, Some(paste), "Paste moved");
        app.act(&ctx, Act::TogglePlanned);
        let tree = settled_at(&mut app, &ctx, 1400.0);
        assert!(node(&tree, name).is_some(), "not back");
    }

    /// ENH-38: Paste, Cut and Copy in the band act on the text being typed on the page,
    /// which stays open and gets the keyboard back; then the system pastes at its caret, or
    /// cuts or copies what is selected in it.
    #[test]
    fn paste_cut_and_copy_in_the_band_act_on_the_typing() {
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        theme::install(&ctx);
        let ty = Typist::new(&ctx);
        // A frame of the window, and what it asked of the system.
        let asked = |app: &mut App, events: Vec<egui::Event>| {
            ty.time.set(ty.time.get() + 0.05);
            let raw = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1400.0, TEST_HEIGHT),
                )),
                time: Some(ty.time.get()),
                events,
                ..Default::default()
            };
            let mut f = eframe::Frame::_new_kittest();
            let mut out = ctx.run_ui(raw, |ui| eframe::App::ui(app, ui, &mut f));
            out.textures_delta.clear();
            let root = out.viewport_output.remove(&egui::ViewportId::ROOT);
            root.map(|v| v.commands).unwrap_or_default()
        };
        for case in ["new text", "line on the page"] {
            for (button, want) in [
                ("Paste", egui::ViewportCommand::RequestPaste),
                ("Cut", egui::ViewportCommand::RequestCut),
                ("Copy", egui::ViewportCommand::RequestCopy),
            ] {
                let mut app = App::with(Settings::from_file(None), Recent::from_file(None));
                let note = typing_doc(&mut app, &ctx);
                start_typing(&mut app, &ty, case, note);
                if case == "new text" {
                    ty.text(&mut app, "Seal");
                }
                let typing = app.tabs[0].ed.typing_id().expect("typing");
                let tree = ty.pause(&mut app);
                let (at, _) = node(&tree, button).expect("the button");
                let p = at.center();
                let mut said = asked(&mut app, vec![egui::Event::PointerMoved(p), press(p, true)]);
                said.extend(asked(&mut app, vec![press(p, false)]));
                for _ in 0..20 {
                    said.extend(asked(&mut app, vec![]));
                }
                let n = said.iter().filter(|c| **c == want).count();
                assert_eq!(n, 1, "{case}, {button}: {said:?}");
                assert_eq!(app.tabs[0].ed.typing_id(), Some(typing), "{case}: ended");
                let back = ctx.memory(|m| m.has_focus(typing));
                assert!(back, "{case}, {button}: the keyboard isn't back");
            }
        }
    }

    /// The strip at the left edge shows each of the side panel's views, saying which is
    /// shown; the one shown again folds the panel away, and the strip stays to bring it
    /// back.
    #[test]
    fn the_strip_shows_each_view_and_folds_the_panel() {
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        theme::install(&ctx);
        let mut app = app_with(&ctx, 1);
        // Pressed, as screen readers say it.
        let selected = |tree: &egui::accesskit::TreeUpdate, name: &str| {
            let (_, n) = (tree.nodes.iter())
                .find(|(_, n)| n.label() == Some(name) && n.role() == egui::accesskit::Role::Button)
                .expect("the view's button");
            n.toggled() == Some(egui::accesskit::Toggled::True)
        };
        for (side, _, name) in SIDE_VIEWS.into_iter().skip(1) {
            click_named(&mut app, &ctx, name);
            let tree = settled_at(&mut app, &ctx, 1400.0);
            assert!(app.side.shown() && app.tabs[0].side == side, "{name}");
            assert!(selected(&tree, name), "{name} doesn't say it's shown");
        }
        click_named(&mut app, &ctx, "Digital signatures");
        let tree = settled_at(&mut app, &ctx, 1400.0);
        assert!(!app.side.shown(), "the panel stayed");
        assert!(!selected(&tree, "Digital signatures"));
        click_named(&mut app, &ctx, "Bookmarks");
        assert!(app.side.shown() && app.tabs[0].side == Side::Bookmarks);
    }

    /// ENH-38: Find opens Search results with the keyboard in its field; Edit text with no
    /// line selected chooses the text tool; Facing pages again goes back to continuous.
    #[test]
    fn find_edit_text_and_facing_pages_from_the_band() {
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        theme::install(&ctx);
        let mut app = app_with(&ctx, 1);
        click_named(&mut app, &ctx, "Find");
        let tree = settled_at(&mut app, &ctx, 1400.0);
        assert_eq!(app.tabs[0].side, Side::Search);
        assert_eq!(focused_name(&ctx, &tree).as_deref(), Some("Search for"));

        click_named(&mut app, &ctx, "Edit text");
        assert_eq!(app.tabs[0].ed.tool, Tool::Text);

        app.band = Band::View;
        click_named(&mut app, &ctx, "Facing pages");
        assert_eq!(app.tabs[0].layout, Layout::Facing);
        click_named(&mut app, &ctx, "Facing pages");
        assert_eq!(app.tabs[0].layout, Layout::Continuous);
    }

    /// ENH-38: groups without room fold from the right into a button with the group's
    /// name, which opens them; groups of planned features alone are left out first; a group
    /// of one command folds to that command's icon.
    #[test]
    fn groups_without_room_fold_into_their_names() {
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        theme::install(&ctx);
        let mut app = app_with(&ctx, 1);
        let tree = settled_at(&mut app, &ctx, 1400.0);
        assert!(node(&tree, "Find").is_some());
        assert!(node(&tree, "Align — Planned").is_some());
        let tree = settled_at(&mut app, &ctx, 1000.0);
        assert!(node(&tree, "Find").is_some(), "Editing folded at 1000");
        let align = node(&tree, "Align — Planned");
        assert!(align.is_none(), "Align shown at 1000");
        // Text flow's Paragraph, its one command, folded to its icon.
        assert!(node(&tree, "Text flow").is_none(), "folded to its name");
        assert!(node(&tree, "Paragraph").is_some(), "no Paragraph at 1000");
        let tree = settled_at(&mut app, &ctx, 700.0);
        let (at, _) = node(&tree, "Editing").expect("Editing folded");
        assert!(node(&tree, "Find").is_none(), "Find shown");
        window(&mut app, &ctx, 700.0, click_at(at.center()));
        let tree = settled_at(&mut app, &ctx, 700.0);
        assert!(egui::Popup::is_any_open(&ctx), "Editing didn't open");
        assert!(node(&tree, "Find").is_some(), "no Find in it");
    }

    /// The screen bounds of the node named `name` in `tree`, and whether it has the keyboard.
    /// The place of the control called `name`, and whether it has the keyboard. Of several,
    /// the biggest: a page rather than its thumbnail, whatever their order in the tree.
    fn node(tree: &egui::accesskit::TreeUpdate, name: &str) -> Option<(egui::Rect, bool)> {
        let (id, n) = tree
            .nodes
            .iter()
            .filter(|(_, n)| n.label() == Some(name))
            .max_by(|(_, a), (_, b)| {
                let area =
                    |n: &egui::accesskit::Node| n.bounds().map_or(0.0, |b| b.width() * b.height());
                area(a).total_cmp(&area(b))
            })?;
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
        // Ctrl+B, then typing, Enter for a second line, and a character no font has: all in
        // bold. Nothing is in the document yet.
        let none = egui::Modifiers::NONE;
        let typed = vec![
            key(Key::B, egui::Modifiers::COMMAND),
            egui::Event::Text("Walrus".into()),
            key(Key::Enter, none),
            egui::Event::Text("Seal \u{2FFFE}".into()),
        ];
        window(&mut app, &ctx, 1400.0, typed);
        let t = &app.tabs[0];
        let adding = |app: &App| app.tabs[0].ed.adding.as_ref().map(|a| a.text.clone());
        let text = adding(&app).expect("the text being typed");
        assert_eq!(text.text(), "Walrus\nSeal ");
        assert!(text.styles().iter().all(|s| s.font.bold));
        assert!(t.unsaved() && t.ed.pending());
        let lines = |app: &App| -> Vec<(String, [f32; 4], Option<String>)> {
            let objs = app.tabs[0].pdf.page_objects(0).unwrap();
            objs.into_iter()
                .filter_map(|o| Some((o.text?, o.bbox, o.font_name)))
                .collect()
        };
        assert!(lines(&app).is_empty());
        tree = window(&mut app, &ctx, 1400.0, vec![]);
        let note = page_text::REFUSED;
        assert!(
            tree.nodes.iter().any(|(_, n)| n.value() == Some(note)),
            "no note about the refused character"
        );
        // Bold switched off in the inspector, with the caret after "Seal ", applies to what
        // is typed next there, and the text takes the keyboard back.
        let (b, _) = node(&tree, "Bold").expect("the inspector's Bold");
        window(&mut app, &ctx, 1400.0, click(b.center()));
        let text = adding(&app).expect("the text being typed");
        assert!(!text.shown((12, 12)).bold && text.shown((0, 12)).bold);
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
            vec![egui::Event::Text("fish".into())],
        );
        let text = adding(&app).expect("the text being typed");
        let bold: String = text
            .styles()
            .iter()
            .map(|s| if s.font.bold { 'B' } else { '.' })
            .collect();
        assert_eq!(bold, "BBBBBBBBBBBB....");
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
        // A click elsewhere on the page adds both lines, as typed, the second 1.2 times the
        // size below and in two styles; the read tool comes back.
        window(&mut app, &ctx, 1400.0, click(at(300.0, 300.0)));
        for _ in 0..2 {
            window(&mut app, &ctx, 1400.0, vec![]);
        }
        let got = lines(&app);
        assert_eq!(got.len(), 2, "{got:?}");
        assert_eq!(
            (got[0].0.as_str(), got[1].0.as_str()),
            ("Walrus", "Seal fish")
        );
        for (_, b, font) in &got {
            assert!((b[0] - x).abs() < 1.0, "{b:?}");
            assert!(
                font.as_deref()
                    .is_some_and(|f| f.contains("Helvetica-Bold")),
                "{font:?}"
            );
        }
        let objs = app.tabs[0].pdf.page_objects(0).unwrap();
        let second = objs.iter().find(|o| o.text.as_deref() == Some("Seal fish"));
        let spans: Vec<(usize, usize, String)> = second
            .map(|o| o.spans.iter())
            .into_iter()
            .flatten()
            .map(|s| (s.chars.0, s.chars.1, s.font_name.clone()))
            .collect();
        assert_eq!(spans.len(), 2, "{spans:?}");
        assert!(
            spans[1].0 == 5 && spans[1].2.ends_with("Helvetica"),
            "{spans:?}"
        );
        assert!(got[0].1[1] < 200.0 && got[0].1[3] > 200.0, "{:?}", got[0].1);
        assert!(((got[1].1[1] - got[0].1[1]) - 16.8).abs() < 0.5);
        let t = &app.tabs[0];
        assert!(t.ed.adding.is_none() && !t.ed.pending());
        assert_eq!(t.ed.tool, Tool::Read);
        // The next new text carries on in the style this one ended in.
        assert!(!t.ed.tfont.bold);
        // One step: Undo takes both lines back.
        app.act(&ctx, Act::Undo);
        assert!(lines(&app).is_empty());
    }

    /// The Unicode text design, section 7: an installed family is found by searching the
    /// font list, Bold and Italic are off for faces it lacks, and text typed in it is
    /// written in it, refusing what it lacks with a note naming it.
    #[test]
    fn installed_fonts_are_chosen_from_the_list_and_written_in() {
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        theme::install(&ctx);
        let serif = crate::font_list::test_family("Source Serif 4");
        // The installed fonts read, as at start.
        limarepdf_core::typefaces();
        let mut app = app_with(&ctx, 1);
        app.act(&ctx, Act::Tool(Tool::Text));
        click_named(&mut app, &ctx, "Font");
        settled_at(&mut app, &ctx, 1400.0);
        assert!(egui::Popup::is_any_open(&ctx), "the list isn't open");
        let enter = key_with(Key::Enter, egui::Modifiers::NONE);
        window(
            &mut app,
            &ctx,
            1400.0,
            vec![egui::Event::Text("source serif".into())],
        );
        window(&mut app, &ctx, 1400.0, vec![enter]);
        let tree = settled_at(&mut app, &ctx, 1400.0);
        assert_eq!(app.tabs[0].ed.tfont.family, serif);
        assert!(!egui::Popup::is_any_open(&ctx), "the list stays open");
        let font = tree.nodes.iter().find(|(_, n)| n.label() == Some("Font"));
        assert_eq!(font.and_then(|(_, n)| n.value()), Some("Source Serif 4"));
        // Source Serif 4 here is regular only.
        for name in ["Bold", "Italic"] {
            let n = tree.nodes.iter().find(|(_, n)| n.label() == Some(name));
            assert!(n.is_some_and(|(_, n)| n.is_disabled()), "{name} is on");
        }

        // Typed on the page: what it has is written in it, embedded.
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
        window(&mut app, &ctx, 1400.0, click(at(100.0, 200.0)));
        settled_at(&mut app, &ctx, 1400.0);
        // A word with a letter it lacks (Ŧ) is typed too, and written whole in a font that
        // has it.
        window(
            &mut app,
            &ctx,
            1400.0,
            vec![egui::Event::Text("Zażółć Ŧest".into())],
        );
        window(&mut app, &ctx, 1400.0, click(at(300.0, 300.0)));
        settled_at(&mut app, &ctx, 1400.0);
        let objs = app.tabs[0].pdf.page_objects(0).unwrap();
        let typed = Some("Zażółć Ŧest");
        let line = objs.iter().find(|o| o.text.as_deref() == typed);
        let spans = &line.expect("the line").spans;
        let fonts: Vec<(&str, (usize, usize))> = (spans.iter())
            .map(|s| (s.font_name.as_str(), s.chars))
            .collect();
        assert_eq!(fonts.len(), 2, "{fonts:?}");
        assert!(fonts[0].0.ends_with("+SourceSerif4-Regular"), "{fonts:?}");
        assert!(!fonts[1].0.ends_with("+SourceSerif4-Regular") && fonts[1].1 == (7, 11));
    }

    #[test]
    fn esc_keeps_new_text_and_empty_text_adds_nothing() {
        let ctx = egui::Context::default();
        let mut app = app_with(&ctx, 1);
        app.act(&ctx, Act::Tool(Tool::Text));
        let ed = &mut app.tabs[0].ed;
        // Nothing typed: Esc closes the editor, adding nothing; Esc again leaves the tool.
        let new = |ed: &crate::editor::Editor, at| {
            Some(page_text::PageText::new(ed.scope(), 0, at, ed.new_look()))
        };
        ed.adding = new(ed, (72.0, 72.0));
        ed.escape();
        assert!(ed.adding.is_none() && ed.actions.is_empty());
        assert_eq!(ed.tool, Tool::Text);
        // Spaces and blank lines are nothing too.
        ed.adding = new(ed, (72.0, 72.0));
        if let Some(a) = ed.adding.as_mut() {
            a.text.sync(" \n ", None);
        }
        assert!(!ed.pending());
        ed.escape();
        assert!(ed.actions.is_empty());
        // Typed text: Esc keeps it, in the look it was typed in.
        ed.tsize = 20.0;
        ed.tcolor = [1.0, 0.0, 0.0];
        ed.adding = new(ed, (72.0, 90.0));
        if let Some(a) = ed.adding.as_mut() {
            a.text.sync("Walrus", None);
        }
        ed.escape();
        assert!(matches!(
            ed.actions.as_slice(),
            [EdAction::AddText(0, (72.0, 90.0), t)] if t.text() == "Walrus"
        ));
        app.tabs[0].apply_actions(&ctx);
        let objs = app.tabs[0].pdf.page_objects(0).unwrap();
        assert!(objs.iter().any(|o| o.text.as_deref() == Some("Walrus")
            && o.font_size == Some(20.0)
            && o.fill == Some([1.0, 0.0, 0.0])));
        // Choosing another tool while typing keeps the text, and that tool.
        let ed = &mut app.tabs[0].ed;
        ed.set_tool(Tool::Text);
        ed.adding = new(ed, (72.0, 400.0));
        if let Some(a) = ed.adding.as_mut() {
            a.text.sync("Seal", None);
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
        // The band, the inspector and the status bar are about the line, not the next new
        // text the tool would add.
        for _ in 0..3 {
            tree = window(&mut app, &ctx, 1400.0, vec![]);
        }
        assert_eq!(app.tabs[0].ed.heading(), "Selected text");
        let size = tree
            .nodes
            .iter()
            .find(|(_, n)| n.label() == Some("Font size"));
        assert_eq!(size.and_then(|(_, n)| n.value()), Some("40"));
        let says = inspector_says(&tree);
        assert!(says.iter().any(|s| s == "One line"), "{says:?}");
        assert!(!says.iter().any(|s| s == "Next new text"), "{says:?}");
    }

    /// With the inspector collapsed, a click where it was ends a line being edited, as any
    /// click away does: only a click in the inspector shown leaves it open.
    #[test]
    fn a_click_where_the_inspector_was_ends_a_line_edit() {
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
        // A double-click on it with the read tool.
        let ty = Typist::new(&ctx);
        let (min, k) = ty.page(&mut app);
        let p = min + egui::vec2(160.0, 190.0) * k;
        ty.frame(&mut app, vec![egui::Event::PointerMoved(p), press(p, true)]);
        ty.frame(&mut app, vec![press(p, false)]);
        ty.frame(&mut app, vec![press(p, true)]);
        ty.frame(&mut app, vec![press(p, false)]);
        let tree = ty.pause(&mut app);
        assert!(
            app.tabs[0].ed.typing_on_page(),
            "the line isn't being edited"
        );
        // Below the inspector's facts (its Editing switch's One line), clear of them.
        let (_, fact) = (tree.nodes.iter())
            .find(|(_, n)| n.label() == Some("One line"))
            .expect("the inspector's facts");
        let b = fact.bounds().expect("where they are");
        let was = egui::pos2(b.x0 as f32 + 4.0, b.y1 as f32 + 200.0);
        // The page made smaller, the click is off it, on the grey around it.
        app.act(&ctx, Act::ToggleInspector);
        for _ in 0..3 {
            app.act(&ctx, Act::ZoomOut);
        }
        let tree = ty.pause(&mut app);
        assert!(
            app.tabs[0].ed.typing_on_page(),
            "collapsing it ended the line"
        );
        let (page, _) = node(&tree, "Page 1").expect("the page");
        assert!(!page.contains(was), "on the page");
        ty.click(&mut app, was);
        ty.pause(&mut app);
        assert!(
            !app.tabs[0].ed.typing_on_page(),
            "the line is still being edited"
        );
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
        let id = app.tabs[0].ed.scope().with(("line-edit", 0usize));
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
        // Typing, a character no font has, and Ctrl+B for the word at the caret, here the
        // whole line.
        let typed = vec![
            egui::Event::Text("X\u{2FFFE}".into()),
            key(Key::B, egui::Modifiers::COMMAND),
        ];
        window(&mut app, &ctx, 1400.0, typed);
        tree = window(&mut app, &ctx, 1400.0, vec![]);
        let value = |tree: &egui::accesskit::TreeUpdate| {
            let n = tree.nodes.iter().find(|(_, n)| n.label() == Some(name));
            n.and_then(|(_, n)| n.value().map(str::to_string))
        };
        assert_eq!(value(&tree).as_deref(), Some("WalXrus"));
        let note = page_text::REFUSED;
        assert!(
            tree.nodes.iter().any(|(_, n)| n.value() == Some(note)),
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

    /// The inspector's font list and Bold change a line being edited on the page, which
    /// stays open and takes the keyboard back; the inspector's own text box doesn't
    /// appear above the list (reported 2026-10-23). Both are kept with the typing.
    #[test]
    fn the_inspector_changes_a_line_being_edited_without_ending_it() {
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        theme::install(&ctx);
        let mut app = app_with(&ctx, 1);
        let courier = FontChoice::from_name("Courier");
        assert!(app.tabs[0].edit(&ctx, |p| {
            p.add_text(0, (100.0, 200.0), "Walrus", courier, 40.0, [0.0; 3])
        }));
        // Frames a twentieth of a second apart, as double-clicks and lists need.
        let time = std::cell::Cell::new(10.0);
        let frame = |app: &mut App, events: Vec<egui::Event>| {
            time.set(time.get() + 0.05);
            let raw = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1400.0, TEST_HEIGHT),
                )),
                time: Some(time.get()),
                events,
                ..Default::default()
            };
            let mut f = eframe::Frame::_new_kittest();
            let mut out = ctx.run_ui(raw, |ui| eframe::App::ui(app, ui, &mut f));
            out.textures_delta.clear();
            out.platform_output.accesskit_update.expect("the window")
        };
        let mut tree = frame(&mut app, vec![]);
        for _ in 0..4 {
            tree = frame(&mut app, vec![]);
        }
        let name = "Editing text: Walrus";
        let editing = |tree: &egui::accesskit::TreeUpdate| node(tree, name).map(|(_, f)| f);
        let text_box = |tree: &egui::accesskit::TreeUpdate| {
            tree.nodes
                .iter()
                .any(|(_, n)| n.label() == Some("Text") && n.value().is_some())
        };
        // A click as the system sends it: a move, a press and a release, a frame each. While
        // the line is edited, each frame keeps it open, without the inspector's text box.
        let tap = |app: &mut App, p: egui::Pos2, editing_line: bool| {
            for e in [
                egui::Event::PointerMoved(p),
                press(p, true),
                press(p, false),
            ] {
                let t = frame(app, vec![e]);
                if editing_line {
                    assert!(editing(&t).is_some(), "the line stopped being edited");
                    assert!(!text_box(&t), "the inspector's text box appeared");
                }
            }
        };
        // A double-click on the line with the read tool edits it; type over the word.
        let (page, _) = node(&tree, "Page 1").expect("the page");
        let k = page.width() / 612.0;
        let p = page.min + egui::vec2(160.0, 190.0) * k;
        frame(&mut app, vec![egui::Event::PointerMoved(p)]);
        for _ in 0..2 {
            frame(&mut app, vec![press(p, true)]);
            frame(&mut app, vec![press(p, false)]);
        }
        for _ in 0..3 {
            tree = frame(&mut app, vec![]);
        }
        assert_eq!(
            editing(&tree),
            Some(true),
            "the line editor doesn't have the keyboard"
        );
        frame(&mut app, vec![egui::Event::Text("Seal".into())]);
        tree = frame(&mut app, vec![]);
        // Bold, then Times from the font list.
        let (bold, _) = node(&tree, "Bold").expect("the inspector's Bold");
        tap(&mut app, bold.center(), true);
        for _ in 0..3 {
            tree = frame(&mut app, vec![]);
        }
        assert_eq!(editing(&tree), Some(true), "the keyboard didn't come back");
        let (font, _) = node(&tree, "Font").expect("the inspector's font list");
        tap(&mut app, font.center(), true);
        tree = frame(&mut app, vec![]);
        assert!(egui::Popup::is_any_open(&ctx), "the font list isn't open");
        assert!(editing(&tree).is_some() && !text_box(&tree));
        let (times, _) = node(&tree, "Times").expect("Times in the font list");
        tap(&mut app, times.center(), true);
        for _ in 0..3 {
            tree = frame(&mut app, vec![]);
            assert!(!text_box(&tree), "the inspector's text box appeared");
        }
        assert_eq!(editing(&tree), Some(true), "the keyboard didn't come back");
        // Typing goes on, in the style set at the caret; Esc keeps the text and the
        // styles together.
        frame(&mut app, vec![egui::Event::Text("s".into())]);
        let esc = egui::Event::Key {
            key: Key::Escape,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::NONE,
        };
        frame(&mut app, vec![esc]);
        for _ in 0..2 {
            frame(&mut app, vec![]);
        }
        let objs = app.tabs[0].pdf.page_objects(0).unwrap();
        let line = objs
            .into_iter()
            .find(|o| o.text.is_some())
            .expect("the line");
        assert_eq!(line.text.as_deref(), Some("Seals"));
        let parts: Vec<(usize, usize, &str)> = line
            .spans
            .iter()
            .map(|s| (s.chars.0, s.chars.1, s.font_name.as_str()))
            .collect();
        assert_eq!(parts, [(0, 4, "Courier"), (4, 5, "Times-Bold")]);
    }

    /// Ctrl+B on a word selected in the line editor makes that word bold, not the line; a
    /// line in parts edited again keeps its parts as they were.
    #[test]
    fn a_word_selected_in_a_line_is_styled_alone() {
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        theme::install(&ctx);
        let mut app = app_with(&ctx, 1);
        let sans = FontChoice::default();
        assert!(app.tabs[0].edit(&ctx, |p| {
            p.add_text(0, (100.0, 200.0), "Walrus ate fish", sans, 40.0, [0.0; 3])
        }));
        let time = std::cell::Cell::new(10.0);
        let frame = |app: &mut App, events: Vec<egui::Event>| {
            time.set(time.get() + 0.05);
            let raw = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1400.0, TEST_HEIGHT),
                )),
                time: Some(time.get()),
                events,
                ..Default::default()
            };
            let mut f = eframe::Frame::_new_kittest();
            let mut out = ctx.run_ui(raw, |ui| eframe::App::ui(app, ui, &mut f));
            out.textures_delta.clear();
            out.platform_output.accesskit_update.expect("the window")
        };
        let key = |key: Key, modifiers: egui::Modifiers| egui::Event::Key {
            key,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers,
        };
        let mut tree = frame(&mut app, vec![]);
        for _ in 0..4 {
            tree = frame(&mut app, vec![]);
        }
        let (page, _) = node(&tree, "Page 1").expect("the page");
        let k = page.width() / 612.0;
        // Double-click a word (page points) to edit its line with the word selected, wait
        // for the keyboard, then press `keys`, Esc, and read the line back.
        let edit_word = |app: &mut App, x: f32, events: Vec<egui::Event>| {
            for _ in 0..25 {
                frame(app, vec![]);
            }
            let p = page.min + egui::vec2(x, 190.0) * k;
            frame(app, vec![egui::Event::PointerMoved(p)]);
            for _ in 0..2 {
                frame(app, vec![press(p, true)]);
                frame(app, vec![press(p, false)]);
            }
            for _ in 0..3 {
                frame(app, vec![]);
            }
            for e in events {
                frame(app, vec![e]);
            }
            frame(app, vec![key(Key::Escape, egui::Modifiers::NONE)]);
            for _ in 0..2 {
                frame(app, vec![]);
            }
            let objs = app.tabs[0].pdf.page_objects(0).unwrap();
            let line = objs
                .into_iter()
                .find(|o| o.text.is_some())
                .expect("the line");
            let parts: Vec<(String, String)> = line
                .spans
                .iter()
                .map(|s| {
                    let text: String = line
                        .text
                        .as_deref()
                        .unwrap_or("")
                        .chars()
                        .skip(s.chars.0)
                        .take(s.chars.1 - s.chars.0)
                        .collect();
                    (text, s.font_name.clone())
                })
                .collect();
            parts
        };
        let owned = |v: &[(&str, &str)]| -> Vec<(String, String)> {
            v.iter()
                .map(|(a, b)| (a.to_string(), b.to_string()))
                .collect()
        };
        // "ate" is from about 236 to 291 points.
        let bold = key(Key::B, egui::Modifiers::COMMAND);
        let parts = edit_word(&mut app, 260.0, vec![bold]);
        assert_eq!(
            parts,
            owned(&[
                ("Walrus ", "Helvetica"),
                ("ate", "Helvetica-Bold"),
                (" fish", "Helvetica")
            ])
        );
        // "fish", from about 302 to 364, typed over: the bold word stays as it was.
        let typed = egui::Event::Text("seals".into());
        let parts = edit_word(&mut app, 330.0, vec![typed]);
        assert_eq!(
            parts,
            owned(&[
                ("Walrus ", "Helvetica"),
                ("ate", "Helvetica-Bold"),
                (" seals", "Helvetica")
            ])
        );
    }

    /// Draw the window 1400 by [`TEST_HEIGHT`] with `events`, at `time` moved on a twentieth
    /// of a second (as double-clicks need), and return its AccessKit tree.
    fn timed_frame(
        app: &mut App,
        ctx: &egui::Context,
        time: &std::cell::Cell<f64>,
        events: Vec<egui::Event>,
    ) -> egui::accesskit::TreeUpdate {
        time.set(time.get() + 0.05);
        let raw = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(1400.0, TEST_HEIGHT),
            )),
            time: Some(time.get()),
            events,
            ..Default::default()
        };
        let mut f = eframe::Frame::_new_kittest();
        let mut out = ctx.run_ui(raw, |ui| eframe::App::ui(app, ui, &mut f));
        out.textures_delta.clear();
        out.platform_output.accesskit_update.expect("the window")
    }

    /// A tab with "Walrus ate fish" at (100, 200) in parts: "ate" bold, 30 points and red,
    /// the rest 40 points and black.
    fn app_with_parts(ctx: &egui::Context) -> App {
        use limarepdf_core::{PartFont, TextPart};
        let mut app = app_with(ctx, 1);
        let sans = FontChoice::default();
        let bold = FontChoice { bold: true, ..sans };
        let part = |text: &str, font, size, color| TextPart {
            text: text.into(),
            font: PartFont::Choice(font),
            size,
            color,
            rise: 0.0,
            from: None,
            origins: Vec::new(),
        };
        let parts = [
            part("Walrus ", sans, 40.0, [0.0; 3]),
            part("ate", bold, 30.0, [1.0, 0.0, 0.0]),
            part(" fish", sans, 40.0, [0.0; 3]),
        ];
        assert!(app.tabs[0].edit(ctx, |p| p.add_text_parts(0, (100.0, 200.0), &parts)));
        app
    }

    /// Page 1's line: its text, and each part's characters, font and size.
    fn parts_of_line(app: &App) -> (String, Vec<(usize, usize, String, f32)>) {
        let objs = app.tabs[0].pdf.page_objects(0).unwrap();
        let line = objs
            .into_iter()
            .find(|o| o.text.is_some())
            .expect("the line");
        let spans = line
            .spans
            .iter()
            .map(|s| (s.chars.0, s.chars.1, s.font_name.clone(), s.font_size))
            .collect();
        (line.text.unwrap_or_default(), spans)
    }

    /// The line styles design, section 4: where the selection mixes fonts, sizes and
    /// colours, the inspector shows those blank; Ctrl+Z and Ctrl+Y in the line take back and
    /// redo style changes and typing, never the document; keeping the edit is one step.
    #[test]
    fn a_line_being_edited_shows_mixed_styles_blank_and_undoes_inside() {
        use egui::accesskit::{Toggled, TreeUpdate};
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        theme::install(&ctx);
        let mut app = app_with_parts(&ctx);
        let time = std::cell::Cell::new(10.0);
        let frame = |app: &mut App, events| timed_frame(app, &ctx, &time, events);
        let key = |key: Key, modifiers: egui::Modifiers| egui::Event::Key {
            key,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers,
        };
        let value = |tree: &TreeUpdate, label: &str| -> String {
            let n = tree.nodes.iter().find(|(_, n)| n.label() == Some(label));
            n.and_then(|(_, n)| n.value())
                .unwrap_or_default()
                .to_string()
        };
        let named = |tree: &TreeUpdate, label: &str| {
            tree.nodes.iter().any(|(_, n)| n.label() == Some(label))
        };
        let bold = |tree: &TreeUpdate| {
            let n = tree.nodes.iter().find(|(_, n)| n.label() == Some("Bold"));
            n.and_then(|(_, n)| n.toggled()) == Some(Toggled::True)
        };
        let mut tree = frame(&mut app, vec![]);
        for _ in 0..4 {
            tree = frame(&mut app, vec![]);
        }
        // A double-click on "Walrus" edits the line with the word selected: its values.
        let (page, _) = node(&tree, "Page 1").expect("the page");
        let p = page.min + egui::vec2(150.0, 190.0) * (page.width() / 612.0);
        frame(&mut app, vec![egui::Event::PointerMoved(p)]);
        for _ in 0..2 {
            frame(&mut app, vec![press(p, true)]);
            frame(&mut app, vec![press(p, false)]);
        }
        for _ in 0..3 {
            tree = frame(&mut app, vec![]);
        }
        let name = "Editing text: Walrus ate fish";
        assert_eq!(node(&tree, name).map(|(_, f)| f), Some(true));
        assert_eq!(value(&tree, "Font"), "Helvetica");
        assert_eq!(value(&tree, "Font size"), "40");
        assert!(named(&tree, "Text colour") && !bold(&tree));
        // All of it: what it mixes shows blank, and Bold off.
        let (ctrl, none) = (egui::Modifiers::COMMAND, egui::Modifiers::NONE);
        frame(&mut app, vec![key(Key::A, ctrl)]);
        tree = frame(&mut app, vec![]);
        assert_eq!(value(&tree, "Font"), "");
        assert_eq!(value(&tree, "Font size"), "");
        assert!(named(&tree, "Text colour (mixed)") && !named(&tree, "Text colour"));
        assert!(!bold(&tree));
        // Ctrl+B makes it all bold; Ctrl+Z and Ctrl+Y take that back and forth.
        for (k, on) in [
            (Key::B, true),
            (Key::Z, false),
            (Key::Y, true),
            (Key::Z, false),
        ] {
            frame(&mut app, vec![key(k, ctrl)]);
            assert_eq!(bold(&frame(&mut app, vec![])), on, "after {k:?}");
        }
        // Typing at the end, then Ctrl+Z: the typing goes, and there is nothing more to
        // undo in the line; the document's Undo isn't reached. Ctrl+Shift+Z redoes it.
        let shift = egui::Modifiers::COMMAND | egui::Modifiers::SHIFT;
        let steps = [
            (
                vec![key(Key::End, none), egui::Event::Text(" too".into())],
                "fish too",
            ),
            (vec![key(Key::Z, ctrl)], "fish"),
            (vec![key(Key::Z, ctrl)], "fish"),
            (vec![key(Key::Z, shift)], "fish too"),
        ];
        for (events, end) in steps {
            frame(&mut app, events);
            let now = value(&frame(&mut app, vec![]), name);
            assert_eq!(now, format!("Walrus ate {end}"));
        }
        // Esc keeps it, its parts as they were; one Undo takes it all back.
        frame(&mut app, vec![key(Key::Escape, none)]);
        for _ in 0..2 {
            frame(&mut app, vec![]);
        }
        let (text, spans) = parts_of_line(&app);
        assert_eq!(text, "Walrus ate fish too");
        assert_eq!(spans.len(), 3, "{spans:?}");
        assert_eq!(spans[1], (7, 10, "Helvetica-Bold".into(), 30.0));
        app.act(&ctx, Act::Undo);
        assert_eq!(parts_of_line(&app).0, "Walrus ate fish");
    }

    /// With the arrow, Edit text and a double-click edit the selected line on the page, as
    /// the read tool does: the inspector has no text box of its own. Ctrl+Z and Ctrl+Y
    /// there take back and redo its typing, not the document; Esc keeps the line selected.
    #[test]
    fn edit_text_with_the_arrow_edits_the_line_on_the_page() {
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        theme::install(&ctx);
        let mut app = app_with_parts(&ctx);
        app.act(&ctx, Act::Tool(Tool::Select));
        let t = &mut app.tabs[0];
        let objs = t.ed.objects(&t.pdf, 0).to_vec();
        let line = objs
            .iter()
            .position(|o| o.text.is_some())
            .expect("the line");
        t.ed.select_object(0, line);
        app.act(&ctx, Act::EditObjectText);
        let time = std::cell::Cell::new(10.0);
        let frame = |app: &mut App, events| timed_frame(app, &ctx, &time, events);
        let key = |key, modifiers| egui::Event::Key {
            key,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers,
        };
        let name = "Editing text: Walrus ate fish";
        let typed = |tree: &egui::accesskit::TreeUpdate| {
            let n = tree.nodes.iter().find(|(_, n)| n.label() == Some(name));
            n.and_then(|(_, n)| n.value())
                .unwrap_or_default()
                .to_string()
        };
        let text_box = |tree: &egui::accesskit::TreeUpdate| {
            tree.nodes
                .iter()
                .any(|(_, n)| n.label() == Some("Text") && n.value().is_some())
        };
        let mut tree = frame(&mut app, vec![]);
        for _ in 0..4 {
            tree = frame(&mut app, vec![]);
        }
        assert_eq!(
            node(&tree, name).map(|(_, f)| f),
            Some(true),
            "not on the page"
        );
        assert!(!text_box(&tree), "the inspector has a text box");
        let (ctrl, none) = (egui::Modifiers::COMMAND, egui::Modifiers::NONE);
        let steps = [
            (
                vec![key(Key::End, none), egui::Event::Text(" too".into())],
                "fish too",
            ),
            (vec![key(Key::Z, ctrl)], "fish"),
            (vec![key(Key::Y, ctrl)], "fish too"),
            (vec![key(Key::Z, ctrl)], "fish"),
            (vec![key(Key::Z, ctrl | egui::Modifiers::SHIFT)], "fish too"),
        ];
        for (events, end) in steps {
            frame(&mut app, events);
            let tree = frame(&mut app, vec![]);
            assert_eq!(typed(&tree), format!("Walrus ate {end}"));
            // The line is still in the document, as it was.
            assert_eq!(parts_of_line(&app).0, "Walrus ate fish");
        }
        // Esc keeps it; with the arrow, the line stays selected.
        frame(&mut app, vec![key(Key::Escape, none)]);
        for _ in 0..3 {
            tree = frame(&mut app, vec![]);
        }
        assert!(node(&tree, name).is_none(), "still editing");
        assert_eq!(app.tabs[0].ed.sel(), Some((0, line)));
        assert_eq!(parts_of_line(&app).0, "Walrus ate fish too");
        // A double-click on it edits it on the page again.
        let (page, _) = node(&tree, "Page 1").expect("the page");
        let p = page.min + egui::vec2(150.0, 190.0) * (page.width() / 612.0);
        frame(&mut app, vec![egui::Event::PointerMoved(p)]);
        for _ in 0..2 {
            frame(&mut app, vec![press(p, true)]);
            frame(&mut app, vec![press(p, false)]);
        }
        for _ in 0..3 {
            tree = frame(&mut app, vec![]);
        }
        let name = "Editing text: Walrus ate fish too";
        assert_eq!(
            node(&tree, name).map(|(_, f)| f),
            Some(true),
            "not on the page"
        );
        assert!(!text_box(&tree), "the inspector has a text box");
    }

    /// With the read tool, finishing a line edited on the page (Esc, Tab, the keyboard going
    /// elsewhere) leaves nothing selected: the inspector goes back to the document, as a word
    /// processor leaves no selection behind.
    #[test]
    fn a_line_edited_with_the_read_tool_is_not_left_selected() {
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        theme::install(&ctx);
        let mut app = app_with_parts(&ctx);
        app.act(&ctx, Act::Tool(Tool::Read));
        let time = std::cell::Cell::new(10.0);
        let frame = |app: &mut App, events| timed_frame(app, &ctx, &time, events);
        let none = egui::Modifiers::NONE;
        let mut tree = frame(&mut app, vec![]);
        for _ in 0..4 {
            tree = frame(&mut app, vec![]);
        }
        let (page, _) = node(&tree, "Page 1").expect("the page");
        let p = page.min + egui::vec2(150.0, 190.0) * (page.width() / 612.0);
        let name = "Editing text: Walrus ate fish";
        for finish in [Key::Escape, Key::Tab] {
            frame(&mut app, vec![egui::Event::PointerMoved(p)]);
            for _ in 0..2 {
                frame(&mut app, vec![press(p, true)]);
                frame(&mut app, vec![press(p, false)]);
            }
            for _ in 0..3 {
                tree = frame(&mut app, vec![]);
            }
            assert_eq!(node(&tree, name).map(|(_, f)| f), Some(true), "not editing");
            frame(&mut app, vec![key_with(finish, none)]);
            for _ in 0..3 {
                frame(&mut app, vec![]);
            }
            assert_eq!(app.tabs[0].ed.sel(), None, "{finish:?} left it selected");
            assert_eq!(inspector::title(&app.tabs[0]), "Document", "{finish:?}");
        }
    }

    /// A turned line is edited in a plain box, which styles a word on its own as the line
    /// editor does.
    #[test]
    fn a_word_in_a_turned_line_is_styled_alone() {
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        theme::install(&ctx);
        let mut app = app_with_parts(&ctx);
        assert!(app.tabs[0].edit(&ctx, |p| p.rotate_pages(&[0], 90)));
        let objs = app.tabs[0].pdf.page_objects(0).unwrap();
        assert!(objs.iter().any(|o| o.text.is_some() && !o.upright));
        let time = std::cell::Cell::new(10.0);
        let frame = |app: &mut App, events| timed_frame(app, &ctx, &time, events);
        let mut tree = frame(&mut app, vec![]);
        for _ in 0..4 {
            tree = frame(&mut app, vec![]);
        }
        // Turned a quarter, the page is 792 points wide.
        let (page, _) = node(&tree, "Page 1").expect("the page");
        let k = page.width() / 792.0;
        let words = app.tabs[0].pdf.page_words(0).unwrap();
        let fish = words.iter().find(|w| w.text == "fish").expect("fish").bbox;
        let at = egui::vec2((fish[0] + fish[2]) / 2.0, (fish[1] + fish[3]) / 2.0);
        let p = page.min + at * k;
        frame(&mut app, vec![egui::Event::PointerMoved(p)]);
        for _ in 0..2 {
            frame(&mut app, vec![press(p, true)]);
            frame(&mut app, vec![press(p, false)]);
        }
        for _ in 0..3 {
            tree = frame(&mut app, vec![]);
        }
        let name = "Editing text: Walrus ate fish";
        assert_eq!(node(&tree, name).map(|(_, f)| f), Some(true));
        let key = |key, modifiers| egui::Event::Key {
            key,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers,
        };
        frame(&mut app, vec![key(Key::I, egui::Modifiers::COMMAND)]);
        frame(&mut app, vec![key(Key::Escape, egui::Modifiers::NONE)]);
        for _ in 0..2 {
            frame(&mut app, vec![]);
        }
        let (text, spans) = parts_of_line(&app);
        assert_eq!(text, "Walrus ate fish");
        let fonts: Vec<&str> = spans.iter().map(|s| s.2.as_str()).collect();
        assert_eq!(
            fonts,
            [
                "Helvetica",
                "Helvetica-Bold",
                "Helvetica",
                "Helvetica-Oblique"
            ]
        );
        assert_eq!((spans[3].0, spans[3].1), (11, 15));
    }

    /// BUG-26: in a line turned on the page, a click with the Text tool puts the caret where
    /// it falls along the line, and a double-click selects the word clicked, the line's
    /// second "fish" too, whichever way the page is turned.
    #[test]
    fn a_click_in_a_turned_line_puts_the_caret_where_it_falls() {
        use limarepdf_core::{PartFont, TextPart};
        let key = |key| egui::Event::Key {
            key,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::NONE,
        };
        // Clicks `n` times with `tool` at `at` (the point of a word's box, in page points,
        // from its middle by `along` of its length along the line), types `typed` and keeps
        // it; the line's text then.
        let edited = |turn: i64, tool, word: usize, along: f32, n: usize, typed: &str| {
            let ctx = egui::Context::default();
            ctx.enable_accesskit();
            theme::install(&ctx);
            let mut app = app_with(&ctx, 1);
            let part = TextPart {
                text: "fish and fish".into(),
                font: PartFont::Choice(FontChoice::default()),
                size: 30.0,
                color: [0.0; 3],
                rise: 0.0,
                from: None,
                origins: Vec::new(),
            };
            assert!(app.tabs[0].edit(&ctx, |p| p.add_text_parts(0, (230.0, 480.0), &[part])));
            assert!(app.tabs[0].edit(&ctx, |p| p.rotate_pages(&[0], turn)));
            app.act(&ctx, Act::Tool(tool));
            let time = std::cell::Cell::new(10.0);
            let frame = |app: &mut App, events| timed_frame(app, &ctx, &time, events);
            let mut tree = frame(&mut app, vec![]);
            for _ in 0..4 {
                tree = frame(&mut app, vec![]);
            }
            let (page, _) = node(&tree, "Page 1").expect("the page");
            let width = if turn % 180 == 0 { 612.0 } else { 792.0 };
            let k = page.width() / width;
            // The page turns clockwise, the line with it; its words in its own order.
            let dir = match turn {
                90 => egui::vec2(0.0, 1.0),
                180 => egui::vec2(-1.0, 0.0),
                _ => egui::vec2(0.0, -1.0),
            };
            let mid = |b: [f32; 4]| egui::pos2((b[0] + b[2]) / 2.0, (b[1] + b[3]) / 2.0);
            let mut words = app.tabs[0].pdf.page_words(0).unwrap();
            words.sort_by(|v, w| {
                let along = |b| mid(b).to_vec2().dot(dir);
                along(v.bbox).total_cmp(&along(w.bbox))
            });
            assert_eq!(words.len(), 3, "{words:?}");
            let b = words[word].bbox;
            let length = (b[2] - b[0]) * dir.x.abs() + (b[3] - b[1]) * dir.y.abs();
            let at = mid(b) + dir * along * length;
            let p = page.min + at.to_vec2() * k;
            // In the window `timed_frame` draws.
            let window =
                egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1400.0, TEST_HEIGHT));
            assert!(window.contains(p), "{p:?} is out of view at {turn}°");
            frame(&mut app, vec![egui::Event::PointerMoved(p)]);
            for _ in 0..n {
                frame(&mut app, vec![press(p, true)]);
                frame(&mut app, vec![press(p, false)]);
            }
            for _ in 0..3 {
                frame(&mut app, vec![]);
            }
            frame(&mut app, vec![egui::Event::Text(typed.into())]);
            frame(&mut app, vec![key(Key::Escape)]);
            for _ in 0..2 {
                frame(&mut app, vec![]);
            }
            parts_of_line(&app).0
        };
        let turns = [90, 180, 270];
        let got: Vec<(i64, String, String)> = turns
            .iter()
            .map(|&turn| {
                // Just inside the end of "and": the caret after its "d".
                let caret = edited(turn, Tool::Text, 1, 0.4, 1, "X");
                (turn, caret, edited(turn, Tool::Read, 2, 0.0, 2, "cod"))
            })
            .collect();
        let want = turns.map(|t| (t, "fish andX fish".into(), "fish and cod".into()));
        assert_eq!(got, want);
    }

    /// A tab with "Walrus ate fish" at (100, 200) in 30-point Helvetica: "ate" in `other`,
    /// the rest in `color`.
    fn app_in_colours(ctx: &egui::Context, color: [f32; 3], other: [f32; 3]) -> App {
        use limarepdf_core::{PartFont, TextPart};
        let mut app = app_with(ctx, 1);
        let part = |text: &str, color| TextPart {
            text: text.into(),
            font: PartFont::Choice(FontChoice::default()),
            size: 30.0,
            color,
            rise: 0.0,
            from: None,
            origins: Vec::new(),
        };
        let parts = [
            part("Walrus ", color),
            part("ate", other),
            part(" fish", color),
        ];
        assert!(app.tabs[0].edit(ctx, |p| p.add_text_parts(0, (100.0, 200.0), &parts)));
        app
    }

    /// Page 1's line as it would be saved now: the colour of each run of its characters,
    /// with where the run ends.
    fn line_colours(app: &App) -> Vec<(usize, Option<[f32; 3]>)> {
        let pdf = saved_pdf(&app.tabs[0]);
        let objs = pdf.page_objects(0).unwrap();
        let line = objs.iter().find(|o| o.text.is_some()).expect("the line");
        let mut out: Vec<(usize, Option<[f32; 3]>)> = Vec::new();
        for s in &line.spans {
            match out.last_mut() {
                Some(last) if last.1 == s.fill => last.0 = s.chars.1,
                _ => out.push((s.chars.1, s.fill)),
            }
        }
        out
    }

    /// Clicks in the open text colour picker's colour area at `at`, from its bottom left
    /// corner (0, 0: black) to its top right (1, 1: the hue at its full).
    fn click_in_colour_area(
        app: &mut App,
        ctx: &egui::Context,
        frame: &mut dyn FnMut(&mut App, Vec<egui::Event>) -> egui::accesskit::TreeUpdate,
        at: egui::Vec2,
    ) {
        let tree = frame(app, vec![]);
        // Above the text's colours offered are egui's colour area, 275 points square, and
        // its hue slider.
        let label = tree
            .nodes
            .iter()
            .find(|(_, n)| n.value() == Some("In the text"))
            .and_then(|(_, n)| n.bounds())
            .expect("the picker");
        let s = ctx.global_style().spacing.clone();
        let bottom = label.y0 as f32 - 4.0 - s.item_spacing.y;
        let area_bottom = bottom - s.interact_size.y - s.item_spacing.y;
        let corner = egui::pos2(label.x0 as f32, area_bottom);
        let p = corner + egui::vec2(at.x, -at.y) * 275.0;
        frame(app, vec![egui::Event::PointerMoved(p)]);
        frame(app, vec![press(p, true)]);
        frame(app, vec![press(p, false)]);
    }

    /// A26: in mixed-colour text, a colour picked makes it all that colour, the one the
    /// picker starts from (the first) too, in one Undo step; opening and closing the
    /// picker changes nothing.
    #[test]
    fn a_colour_picked_makes_mixed_text_one_colour() {
        let (black, red) = ([0.0; 3], [1.0, 0.0, 0.0]);
        let (one, two) = ("Colour 1 of 2 in the text", "Colour 2 of 2 in the text");
        // At the colour area's foot, black to the eye; at its top right, red's hue at its
        // full: each the colour the picker starts from.
        let (foot, corner) = (egui::vec2(0.5, 0.001), egui::vec2(1.0, 1.0));
        for (first, other, at_first) in [(black, red, foot), (red, black, corner)] {
            let ctx = egui::Context::default();
            ctx.enable_accesskit();
            theme::install(&ctx);
            let mut app = app_in_colours(&ctx, first, other);
            app.act(&ctx, Act::Tool(Tool::Select));
            // Undo leaves nothing selected.
            let select_line = |app: &mut App| {
                let t = &mut app.tabs[0];
                let objs = t.ed.objects(&t.pdf, 0).to_vec();
                let line = objs
                    .iter()
                    .position(|o| o.text.is_some())
                    .expect("the line");
                t.ed.select_object(0, line);
                app.open_inspector();
            };
            select_line(&mut app);
            let mixed = vec![(7, Some(first)), (10, Some(other)), (15, Some(first))];
            assert_eq!(line_colours(&app), mixed);
            let mut frame = |app: &mut App, events| window(app, &ctx, 1400.0, events);

            // Opened and closed, by its button or Esc, the picker changes nothing, and adds
            // no Undo step; Esc closes only the picker.
            let esc = || vec![key_with(Key::Escape, egui::Modifiers::NONE)];
            for by_esc in [false, true] {
                click_named(&mut app, &ctx, "Text colour (mixed)");
                let tree = settle(&mut app, &ctx);
                assert!(node(&tree, two).is_some(), "no picker");
                if by_esc {
                    frame(&mut app, esc());
                } else {
                    click_named(&mut app, &ctx, "Text colour (mixed)");
                }
                let tree = settle(&mut app, &ctx);
                assert!(node(&tree, one).is_none(), "still open");
                assert!(node(&tree, "Text colour (mixed)").is_some(), "not selected");
                assert_eq!(line_colours(&app), mixed, "{first:?} first");
            }

            // The first colour picked in the colour area: all of it that colour.
            click_named(&mut app, &ctx, "Text colour (mixed)");
            settle(&mut app, &ctx);
            click_in_colour_area(&mut app, &ctx, &mut frame, at_first);
            click_named(&mut app, &ctx, "Text colour");
            settle(&mut app, &ctx);
            assert_eq!(line_colours(&app), [(15, Some(first))], "{first:?} first");
            // One Undo step, back and forth.
            app.act(&ctx, Act::Undo);
            assert_eq!(line_colours(&app), mixed, "{first:?} first");
            app.act(&ctx, Act::Redo);
            assert_eq!(line_colours(&app), [(15, Some(first))], "{first:?} first");
            app.act(&ctx, Act::Undo);

            // A colour the text has, picked from those offered: all of it that colour.
            for (offered, color) in [(one, first), (two, other)] {
                select_line(&mut app);
                click_named(&mut app, &ctx, "Text colour (mixed)");
                settle(&mut app, &ctx);
                click_named(&mut app, &ctx, offered);
                let tree = settle(&mut app, &ctx);
                assert!(node(&tree, one).is_none(), "still open");
                assert_eq!(line_colours(&app), [(15, Some(color))], "{color:?}");
                app.act(&ctx, Act::Undo);
                assert_eq!(line_colours(&app), mixed, "{color:?}");
            }
            // The adding of the line is the only step left.
            app.act(&ctx, Act::Undo);
            let objs = saved_pdf(&app.tabs[0]).page_objects(0).unwrap();
            assert!(objs.iter().all(|o| o.text.is_none()), "{first:?} first");
        }
    }

    /// Edits page 1's line on the page, "ate fish" selected (a double-click on "ate", then
    /// Shift+End), with `frame`; returns the window's tree then.
    fn edit_ate_fish(
        app: &mut App,
        frame: &mut dyn FnMut(&mut App, Vec<egui::Event>) -> egui::accesskit::TreeUpdate,
    ) -> egui::accesskit::TreeUpdate {
        let mut tree = frame(app, vec![]);
        for _ in 0..4 {
            tree = frame(app, vec![]);
        }
        let (page, _) = node(&tree, "Page 1").expect("the page");
        let words = app.tabs[0].pdf.page_words(0).unwrap();
        let ate = words.iter().find(|w| w.text == "ate").expect("ate").bbox;
        let at = egui::vec2((ate[0] + ate[2]) / 2.0, (ate[1] + ate[3]) / 2.0);
        let p = page.min + at * (page.width() / 612.0);
        frame(app, vec![egui::Event::PointerMoved(p)]);
        for _ in 0..2 {
            frame(app, vec![press(p, true)]);
            frame(app, vec![press(p, false)]);
        }
        for _ in 0..3 {
            frame(app, vec![]);
        }
        frame(app, vec![key_with(Key::End, egui::Modifiers::SHIFT)]);
        for _ in 0..2 {
            tree = frame(app, vec![]);
        }
        let name = "Editing text: Walrus ate fish";
        assert_eq!(node(&tree, name).map(|(_, f)| f), Some(true));
        tree
    }

    /// Clicks the control named `name` in `tree` with `frame`, then lets two frames pass.
    fn click_with(
        app: &mut App,
        frame: &mut dyn FnMut(&mut App, Vec<egui::Event>) -> egui::accesskit::TreeUpdate,
        tree: &egui::accesskit::TreeUpdate,
        name: &str,
    ) -> egui::accesskit::TreeUpdate {
        let p = middle(tree, name);
        frame(app, vec![egui::Event::PointerMoved(p)]);
        frame(app, vec![press(p, true)]);
        frame(app, vec![press(p, false)]);
        frame(app, vec![]);
        frame(app, vec![])
    }

    /// BUG-38: the colour picked is written as the picker shows it: half way up the colour
    /// area, with no colour in it, is the grey the page shows as 0.5 (it was written 0.21,
    /// which the page shows darker).
    #[test]
    fn a_colour_picked_is_written_as_the_picker_shows_it() {
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        theme::install(&ctx);
        let mut app = app_in_colours(&ctx, [0.0; 3], [1.0, 0.0, 0.0]);
        app.act(&ctx, Act::Tool(Tool::Select));
        let t = &mut app.tabs[0];
        let objs = t.ed.objects(&t.pdf, 0).to_vec();
        let line = objs
            .iter()
            .position(|o| o.text.is_some())
            .expect("the line");
        t.ed.select_object(0, line);
        app.open_inspector();
        let mut frame = |app: &mut App, events| window(app, &ctx, 1400.0, events);
        click_named(&mut app, &ctx, "Text colour (mixed)");
        settle(&mut app, &ctx);
        click_in_colour_area(&mut app, &ctx, &mut frame, egui::vec2(0.0, 0.5));
        click_named(&mut app, &ctx, "Text colour");
        settle(&mut app, &ctx);
        let colours = line_colours(&app);
        let [(15, Some(grey))] = colours[..] else {
            panic!("not one colour: {colours:?}");
        };
        assert!(grey.iter().all(|v| (v - 0.5).abs() < 0.01), "{grey:?}");
    }

    /// ENH-07 (V03): a selected line's inspector is headed Selected text (batch 39: as the
    /// workspace has it, and as selected words are) and puts its look first, the
    /// font list across the panel, with no text box (the words are edited on the page, as in
    /// a word processor); its position and size wait in a section
    /// that starts closed, stays as it was left from one selection to the next, and still
    /// moves the line.
    #[test]
    fn the_text_inspector_puts_typography_first() {
        use egui::accesskit::TreeUpdate;
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        theme::install(&ctx);
        let mut app = app_in_colours(&ctx, [0.0; 3], [0.0; 3]);
        let select_line = |app: &mut App| {
            app.act(&ctx, Act::Tool(Tool::Select));
            let t = &mut app.tabs[0];
            let objs = t.ed.objects(&t.pdf, 0).to_vec();
            let line = objs
                .iter()
                .position(|o| o.text.is_some())
                .expect("the line");
            t.ed.select_object(0, line);
            app.open_inspector();
            line
        };
        let line = select_line(&mut app);
        let tree = settled_at(&mut app, &ctx, 1400.0);
        assert_eq!(inspector::title(&app.tabs[0]), "Selected text");
        let shown =
            |tree: &TreeUpdate, text: &str| tree.nodes.iter().any(|(_, n)| n.value() == Some(text));
        assert!(!shown(&tree, "Edit"), "the heading is still Edit");
        let geometry = ["X", "Y", "Width", "Height"];
        for name in geometry {
            assert!(node(&tree, name).is_none(), "{name} is shown");
        }
        // No text box: the words are edited on the page, their look in Edit's Font group
        // above. The inspector says what the line is.
        let text_box = tree
            .nodes
            .iter()
            .any(|(_, n)| n.label() == Some("Text") && n.value().is_some());
        assert!(!text_box, "the inspector has a text box");
        let (section, _) = node(&tree, "Position and size").expect("the section");
        let (font, _) = node(&tree, "Font").expect("the font list");
        let above = font.bottom() < section.top();
        assert!(above, "{font:?} not above {section:?}");
        for fact in ["Editing", "Source font"] {
            assert!(shown(&tree, fact), "no {fact}");
        }
        // How it is edited: the switch, One line pressed.
        assert!(node(&tree, "One line").is_some() && node(&tree, "Paragraph").is_some());
        // Tab goes through the look in the band, then position and size, then Delete.
        let reached = tab_walk(&mut app, &ctx, 1400.0, 60);
        let look = [
            "Font",
            "Font size",
            "Font sizes",
            "Bold",
            "Italic",
            "Text colour",
        ];
        let at = reached.iter().position(|n| n == "Font").expect("no Font");
        assert_eq!(reached[at..at + look.len()], look, "{reached:?}");
        let at = reached
            .iter()
            .position(|n| n == "Position and size")
            .expect("no Position and size");
        assert_eq!(reached[at + 1], "Delete", "{reached:?}");

        // Opened, it has the fields, and X moves the line.
        click_named(&mut app, &ctx, "Position and size");
        let tree = settled_at(&mut app, &ctx, 1400.0);
        for name in geometry {
            assert!(node(&tree, name).is_some(), "{name} isn't shown");
        }
        let set_x = |app: &mut App, value: &str| {
            click_named(app, &ctx, "X");
            let (ctrl, none) = (egui::Modifiers::COMMAND, egui::Modifiers::NONE);
            window(app, &ctx, 1400.0, vec![key_with(Key::A, ctrl)]);
            window(app, &ctx, 1400.0, vec![egui::Event::Text(value.into())]);
            window(app, &ctx, 1400.0, vec![key_with(Key::Enter, none)]);
            settle(app, &ctx);
            let t = &mut app.tabs[0];
            t.ed.objects(&t.pdf, 0)[line].bbox[0]
        };
        let x = set_x(&mut app, "150");
        assert!((x - 150.0).abs() < 0.5, "the line is at {x}");
        // Long numbers fit: the panel doesn't grow, and each field is inside it.
        set_x(&mut app, "12345.678");
        let tree = settled_at(&mut app, &ctx, 1400.0);
        let (opened, _) = node(&tree, "Position and size").expect("the section");
        assert_eq!(
            opened.x_range(),
            section.x_range(),
            "the panel's width changed"
        );
        let edge = node(&tree, "Inspector width").expect("the panel's edge").0;
        for name in geometry {
            let (r, _) = node(&tree, name).expect("the field");
            let inside = r.left() >= edge.center().x && r.right() <= 1400.0;
            assert!(inside, "{name} at {r:?} outside the panel");
        }

        // Another tool and back: still open, as it was left.
        app.act(&ctx, Act::Tool(Tool::Rect));
        settle(&mut app, &ctx);
        assert_eq!(inspector::title(&app.tabs[0]), "New rectangle");
        select_line(&mut app);
        let tree = settled_at(&mut app, &ctx, 1400.0);
        assert!(node(&tree, "X").is_some(), "the section closed again");
    }

    /// ENH-07: the inspector's heading names what it shows, for each tool and kind of
    /// object, never a bare Edit.
    #[test]
    fn the_inspector_is_headed_by_what_it_shows() {
        let ctx = egui::Context::default();
        theme::install(&ctx);
        let mut app = app_in_colours(&ctx, [0.0; 3], [0.0; 3]);
        for (tool, heading) in [
            (Tool::Text, "New text"),
            (Tool::TextBox, "New text box"),
            (Tool::Rect, "New rectangle"),
            (Tool::Ellipse, "New ellipse"),
            (Tool::Line, "New line"),
            (Tool::Image, "Insert image"),
            (Tool::Pen, "Pen"),
            (Tool::Note, "New note"),
            (Tool::Redact, "Mark for redaction"),
        ] {
            // Set directly: the Image tool's command asks for a file.
            app.tabs[0].ed.set_tool(tool);
            assert_eq!(inspector::title(&app.tabs[0]), heading, "{tool:?}");
        }
        app.tabs[0].ed.set_tool(Tool::Select);
        assert_eq!(inspector::title(&app.tabs[0]), "Document");
        let t = &mut app.tabs[0];
        let objs = t.ed.objects(&t.pdf, 0).to_vec();
        let line = objs
            .iter()
            .position(|o| o.text.is_some())
            .expect("the line");
        t.ed.select_object(0, line);
        assert_eq!(inspector::title(&app.tabs[0]), "Selected text");
    }

    /// What the inspector says: the names and texts of everything in it (in the region called
    /// Inspector holding the most: the panel, or the pop-out beside its strip).
    fn inspector_says(tree: &egui::accesskit::TreeUpdate) -> Vec<String> {
        use egui::accesskit::{Node, NodeId, Role};
        use std::collections::HashMap;
        let nodes: HashMap<NodeId, &Node> = tree.nodes.iter().map(|(id, n)| (*id, n)).collect();
        let inside = |root: NodeId| {
            let (mut out, mut next) = (Vec::new(), vec![root]);
            while let Some(id) = next.pop() {
                if let Some(n) = nodes.get(&id) {
                    out.push(*n);
                    next.extend(n.children().iter().copied());
                }
            }
            out
        };
        let region = tree
            .nodes
            .iter()
            .filter(|(_, n)| n.role() == Role::Complementary && n.label() == Some("Inspector"))
            .map(|(id, _)| inside(*id))
            .max_by_key(Vec::len)
            .expect("the inspector");
        (region.into_iter())
            .flat_map(|n| [n.label(), n.value()])
            .flatten()
            .map(str::to_string)
            .collect()
    }

    /// Batch 39: the inspector keeps its width from frame to frame, with nothing selected, a
    /// tool open (its heading row with two buttons) and pages selected: its heading row is
    /// never wider than the panel it is measured in, which would widen it every frame. The
    /// document's sections start it at the workspace's width.
    #[test]
    fn the_inspector_keeps_its_width() {
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        theme::install(&ctx);
        let mut app = app_with(&ctx, 1);
        let edge = |app: &mut App| {
            let tree = window(app, &ctx, 1400.0, vec![]);
            let (at, _) = node(&tree, "Inspector width").expect("the edge");
            at.center().x
        };
        let start = edge(&mut app);
        let width = 1400.0 - start;
        assert!((width - INSPECTOR_WIDTH).abs() <= 8.0, "{width} wide");
        let mut document = None;
        for (what, pages, act) in [
            ("nothing selected", false, None),
            ("pages selected", true, None),
            ("a tool open", false, Some(Act::Watermark)),
        ] {
            if pages {
                app.tabs[0].ed.selection = Selection::pages_from([0].into());
            }
            if let Some(act) = act {
                app.act(&ctx, act);
            }
            // Once its controls have widened it as far as they need.
            settled_at(&mut app, &ctx, 1400.0);
            let at = edge(&mut app);
            for _ in 0..8 {
                assert_eq!(edge(&mut app), at, "{what}: the inspector widened");
            }
            // Selected pages' long button names wrap rather than widen it, so the pages
            // beside it keep their zoom.
            let document = *document.get_or_insert(at);
            if pages {
                assert_eq!(at, document, "pages selected: wider than the document's");
            }
        }
    }

    /// PF-40 (batch 39): with nothing selected the inspector shows the document: a few facts
    /// about it, and its properties and opening view folded; no longer every tool, nor the
    /// commands the band and the File menu have.
    #[test]
    fn nothing_selected_shows_the_document_not_every_tool() {
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        theme::install(&ctx);
        let mut app = app_with(&ctx, 1);
        let says = inspector_says(&settled_at(&mut app, &ctx, 1400.0));
        assert_eq!(inspector::title(&app.tabs[0]), "Document");
        for fact in [
            "Pages",
            "2",
            "Paper",
            "Letter",
            "Security",
            "None",
            "Properties",
            "Opening view",
        ] {
            assert!(says.iter().any(|s| s == fact), "no {fact}: {says:?}");
        }
        for gone in [
            "Crop",
            "Watermark",
            "Headers, footers, Bates",
            "Recognise text (OCR)",
            "Optimise scanned pages",
            "Export pages as images",
            "Split into files",
            "Password security",
            "Export text…",
            "Print…",
            "Append PDFs…",
        ] {
            assert!(!says.iter().any(|s| s == gone), "{gone} is shown: {says:?}");
        }
    }

    /// Batch 39: the document's Security fact says what it is now and what saving does: a
    /// protected file opened has its protection removed on saving (as its notice says) until
    /// a password is set again.
    #[test]
    fn the_security_fact_says_what_saving_does() {
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        theme::install(&ctx);
        let path = std::env::temp_dir().join(format!(
            "limarepdf-app-{}-security-fact.pdf",
            std::process::id()
        ));
        pages_file(&path, 612.0, 1, Some("walrus"));
        let bytes = std::fs::read(&path).unwrap();
        let _ = std::fs::remove_file(&path);
        let pdf = Pdf::from_bytes(&bytes, Some("walrus")).unwrap();
        let mut app = App::with(Settings::from_file(None), Recent::from_file(None));
        assert!(app.add_tab(&ctx, pdf, None, None, false));
        let says = inspector_says(&settled_at(&mut app, &ctx, 1400.0));
        let removed = says.iter().any(|s| s == "Removed on save");
        assert!(removed, "{says:?}");
        app.tabs[0].set_encryption(Some(Encryption {
            user_password: "walrus".into(),
            owner_password: "walrus-owner".into(),
            cipher: Cipher::Aes128,
            restrictions: Restrictions::default(),
        }));
        let says = inspector_says(&settled_at(&mut app, &ctx, 1400.0));
        let open = says.iter().any(|s| s == "Open password");
        assert!(open, "{says:?}");
    }

    /// PF-40: each tool opens alone in the inspector from its command, headed by its name,
    /// with a button that closes it, back to the document (or, for page labels, the page
    /// they were opened for). Neither changes the document.
    #[test]
    fn each_tool_opens_alone_and_closes_back() {
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        theme::install(&ctx);
        let mut app = app_with(&ctx, 1);
        settled_at(&mut app, &ctx, 1400.0);
        let revision = app.tabs[0].revision;
        for (act, name) in [
            (Act::Watermark, "Watermark"),
            (Act::HeaderFooter, "Headers, footers, Bates"),
            (Act::Crop, "Crop"),
            (Act::Ocr, "Recognise text (OCR)"),
            (Act::Scans, "Optimise scanned pages"),
            (Act::ExportImages, "Export pages as images"),
            (Act::Split, "Split into files"),
            (Act::Security, "Password security"),
            (Act::PageLabels, "Page labels"),
        ] {
            app.act(&ctx, act);
            let tree = settled_at(&mut app, &ctx, 1400.0);
            assert_eq!(inspector::title(&app.tabs[0]), name);
            let says = inspector_says(&tree);
            let alone = !says
                .iter()
                .any(|s| s == "Opening view" || s == "Page properties");
            assert!(alone, "{name}: shown with the document or pages: {says:?}");
            let close = format!("Close {name}");
            click_named(&mut app, &ctx, &close);
            let tree = settled_at(&mut app, &ctx, 1400.0);
            let back = match act {
                Act::PageLabels => "Selected page",
                _ => "Document",
            };
            assert_eq!(inspector::title(&app.tabs[0]), back, "{name}");
            let kept = node(&tree, &close).is_some();
            assert!(!kept, "{name} still has its close button");
            let changed = app.tabs[0].revision != revision;
            assert!(!changed, "{name} changed the document");
            app.tabs[0].ed.selection = Selection::None;
        }
    }

    /// PF-40: the document's properties, chosen with a tool open, take its place: the
    /// document shows, its Properties open.
    #[test]
    fn properties_take_the_place_of_a_tool() {
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        theme::install(&ctx);
        let mut app = app_with(&ctx, 1);
        app.act(&ctx, Act::Watermark);
        settle(&mut app, &ctx);
        app.act(&ctx, Act::Props);
        let tree = settled_at(&mut app, &ctx, 1400.0);
        assert_eq!(inspector::title(&app.tabs[0]), "Document");
        assert!(node(&tree, "Title").is_some(), "Properties isn't open");
    }

    /// PF-40: a tool open follows the pages selected, its pages filled in from them, and gives
    /// way to words selected; with those let go, the document shows, not the tool again.
    #[test]
    fn a_tool_follows_the_selected_pages_and_gives_way_to_words() {
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        theme::install(&ctx);
        let mut app = app_in_colours(&ctx, [0.0; 3], [0.0; 3]);
        app.act(&ctx, Act::Watermark);
        settle(&mut app, &ctx);
        app.tabs[0].ed.selection = Selection::pages_from([1].into());
        settle(&mut app, &ctx);
        assert_eq!(inspector::title(&app.tabs[0]), "Watermark");
        assert_eq!(app.tabs[0].forms.wm.range, "2");
        app.tabs[0].ed.selection = Selection::Words {
            anchor: (0, 0),
            end: (0, 1),
        };
        settle(&mut app, &ctx);
        assert_eq!(inspector::title(&app.tabs[0]), "Selected text");
        app.tabs[0].ed.selection = Selection::None;
        settle(&mut app, &ctx);
        assert_eq!(inspector::title(&app.tabs[0]), "Document");
    }

    /// Batch 39: selected pages show what they are: their numbers, paper and orientation (as
    /// shown, after rotation), "Mixed" where they differ; their buttons; and page properties
    /// folded, holding Crop and Page labels.
    #[test]
    fn selected_pages_show_their_paper_and_orientation() {
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        theme::install(&ctx);
        let mut app = App::with(Settings::from_file(None), Recent::from_file(None));
        let mut pdf = Pdf::new();
        pdf.insert_blank(0, 612.0, 792.0).unwrap();
        pdf.insert_blank(1, 612.0, 792.0).unwrap();
        pdf.insert_blank(2, 595.0, 842.0).unwrap();
        pdf.rotate_pages(&[1], 90).unwrap();
        assert!(app.add_tab(&ctx, pdf, None, None, false));
        let (one, more) = ("Selected page", "Selected pages");
        for (pages, heading, facts) in [
            (vec![0], one, ["Page 1 of 3", "Letter", "Portrait"]),
            (vec![1], one, ["Page 2 of 3", "Letter", "Landscape"]),
            (vec![0, 1], more, ["Pages 1–2 of 3", "Letter", "Mixed"]),
            (vec![0, 2], more, ["Pages 1, 3 of 3", "Mixed", "Portrait"]),
        ] {
            app.tabs[0].ed.selection = Selection::pages_from(pages.iter().copied().collect());
            let tree = settled_at(&mut app, &ctx, 1400.0);
            assert_eq!(inspector::title(&app.tabs[0]), heading, "{pages:?}");
            let says = inspector_says(&tree);
            for fact in facts {
                let shown = says.iter().any(|s| s == fact);
                assert!(shown, "{pages:?}: no {fact}: {says:?}");
            }
        }
        let says = inspector_says(&settled_at(&mut app, &ctx, 1400.0));
        for shown in [
            "Paper",
            "Orientation",
            "Rotate clockwise",
            "Rotate counter-clockwise",
            "Delete pages",
            "Insert blank page",
            "Page properties",
        ] {
            assert!(says.iter().any(|s| s == shown), "no {shown}: {says:?}");
        }
        for folded in ["Crop", "Page labels", "Watermark"] {
            let shown = says.iter().any(|s| s == folded);
            assert!(!shown, "{folded} is shown: {says:?}");
        }
        click_named(&mut app, &ctx, "Page properties");
        let says = inspector_says(&settled_at(&mut app, &ctx, 1400.0));
        for inside in ["Crop", "Page labels"] {
            assert!(says.iter().any(|s| s == inside), "no {inside}: {says:?}");
        }
    }

    /// Batch 39 (the mockup): a selected line is headed Selected text, as selected words are,
    /// with the planned text tools folded at the end, marked Planned, saying what is planned;
    /// hidden with the planned features.
    #[test]
    fn selected_text_shows_the_planned_text_tools() {
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        theme::install(&ctx);
        let mut app = app_in_colours(&ctx, [0.0; 3], [0.0; 3]);
        let planned = "Future text tools — Planned";
        app.act(&ctx, Act::Tool(Tool::Select));
        let t = &mut app.tabs[0];
        let objs = t.ed.objects(&t.pdf, 0).to_vec();
        let line = objs
            .iter()
            .position(|o| o.text.is_some())
            .expect("the line");
        t.ed.select_object(0, line);
        app.open_inspector();
        let tree = settled_at(&mut app, &ctx, 1400.0);
        assert_eq!(inspector::title(&app.tabs[0]), "Selected text");
        assert!(node(&tree, planned).is_some(), "no planned text tools");
        click_named(&mut app, &ctx, planned);
        let note = "Underline and strike-through are planned. Your existing text tools keep \
                    working.";
        let says = inspector_says(&settled_at(&mut app, &ctx, 1400.0));
        assert!(says.iter().any(|s| s == note), "no note: {says:?}");
        // Words selected with the read tool.
        app.act(&ctx, Act::Tool(Tool::Read));
        app.tabs[0].ed.selection = Selection::Words {
            anchor: (0, 0),
            end: (0, 1),
        };
        let tree = settled_at(&mut app, &ctx, 1400.0);
        assert_eq!(inspector::title(&app.tabs[0]), "Selected text");
        assert!(node(&tree, planned).is_some(), "none for words");
        // Hidden with the planned features.
        app.settings.show_planned = false;
        let tree = settled_at(&mut app, &ctx, 1400.0);
        let shown = node(&tree, planned).is_some();
        assert!(!shown, "shown with planned features hidden");
    }

    /// PF-40: the inspector's pop-out, for a narrow window, shows a tool as the panel does,
    /// with its button that closes it, back to the document.
    #[test]
    fn the_pop_out_shows_a_tool_with_its_close_button() {
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        theme::install(&ctx);
        let mut app = app_with(&ctx, 1);
        app.act(&ctx, Act::Watermark);
        // Folded to its strip, the tool in the pop-out.
        app.inspector.set_open(false);
        app.popout = true;
        let tree = settled_at(&mut app, &ctx, 900.0);
        assert!(!app.inspector.shown());
        let says = inspector_says(&tree);
        let headed = says.iter().any(|s| s == "Watermark");
        assert!(headed, "no heading: {says:?}");
        let p = middle(&tree, "Close Watermark");
        window(&mut app, &ctx, 900.0, vec![egui::Event::PointerMoved(p)]);
        window(&mut app, &ctx, 900.0, vec![press(p, true)]);
        window(&mut app, &ctx, 900.0, vec![press(p, false)]);
        let tree = settled_at(&mut app, &ctx, 900.0);
        assert!(app.popout, "the pop-out closed");
        assert_eq!(inspector::title(&app.tabs[0]), "Document");
        assert!(node(&tree, "Close Watermark").is_none());
    }

    /// PF-40: a tool's close button is reached from the keyboard, F6 to the inspector putting
    /// it first, and Enter closes the tool, the keyboard staying in the inspector.
    #[test]
    fn a_tool_closes_from_the_keyboard() {
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        theme::install(&ctx);
        let mut app = app_with(&ctx, 1);
        app.act(&ctx, Act::Watermark);
        settled_at(&mut app, &ctx, 1400.0);
        let mut reached = Vec::new();
        for _ in 0..10 {
            let name = f6(&mut app, &ctx, false);
            reached.push(name.clone());
            if name.as_deref() == Some("Close Watermark") {
                break;
            }
        }
        let at = reached.last().cloned().flatten();
        assert_eq!(at.as_deref(), Some("Close Watermark"), "{reached:?}");
        let enter = key_with(Key::Enter, egui::Modifiers::NONE);
        window(&mut app, &ctx, 1400.0, vec![enter]);
        let tree = settled_at(&mut app, &ctx, 1400.0);
        assert_eq!(inspector::title(&app.tabs[0]), "Document");
        let focused = focused_name(&ctx, &tree);
        assert_eq!(focused.as_deref(), Some("Collapse the inspector"));
    }

    /// ENH-38 (batch 39): each of the side panel's views keeps its own width: the pages as
    /// narrow as the workspace has them, the views of text wider, and a view widened stays
    /// so when another is shown and it comes back.
    #[test]
    fn each_side_view_keeps_its_own_width() {
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        theme::install(&ctx);
        let mut app = app_with(&ctx, 1);
        let edge = |app: &mut App| {
            let tree = settled_at(app, &ctx, 1400.0);
            let (at, _) = node(&tree, "Side panel width").expect("the edge");
            at.center()
        };
        let width = |at: egui::Pos2| at.x - RAIL_WIDTH;
        app.show_view(Side::Pages);
        let pages = width(edge(&mut app));
        assert!((pages - SIDE_WIDTH).abs() <= 8.0, "pages: {pages}");
        let texts = [
            Side::Search,
            Side::Bookmarks,
            Side::Comments,
            Side::Signatures,
        ];
        for side in texts {
            app.show_view(side);
            let w = width(edge(&mut app));
            assert!(w >= 200.0, "{side:?}: {w}");
        }
        // Comments widened by its edge: kept, and the pages' width not changed.
        app.show_view(Side::Comments);
        let from = edge(&mut app);
        let to = from + egui::vec2(60.0, 0.0);
        let over = egui::Event::PointerMoved(from);
        window(&mut app, &ctx, 1400.0, vec![over]);
        window(&mut app, &ctx, 1400.0, vec![press(from, true)]);
        for i in 1..=4 {
            let at = from + (to - from) * (i as f32 / 4.0);
            window(&mut app, &ctx, 1400.0, vec![egui::Event::PointerMoved(at)]);
        }
        window(&mut app, &ctx, 1400.0, vec![press(to, false)]);
        let widened = width(edge(&mut app));
        let wanted = width(from) + 60.0;
        assert!((widened - wanted).abs() <= 2.0, "{widened}, not {wanted}");
        app.show_view(Side::Pages);
        let now = width(edge(&mut app));
        assert!((now - pages).abs() <= 0.5, "pages: {now}, not {pages}");
        app.show_view(Side::Comments);
        let back = width(edge(&mut app));
        let kept = (back - widened).abs() <= 0.5;
        assert!(kept, "comments: {back}, not {widened}");
    }

    /// ENH-08 (batch 39): the inspector and each side view start at the widths given them in
    /// an earlier run; a panel widened by what it holds doesn't change that, and a width
    /// given now by dragging an edge is remembered once it is let go.
    #[test]
    fn panel_widths_are_remembered_between_runs() {
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        theme::install(&ctx);
        let mut settings = Settings::from_file(None);
        settings.panel_widths.insert("side-comments".into(), 260.0);
        let mut app = App::with(settings, Recent::from_file(None));
        let mut pdf = Pdf::new();
        pdf.insert_blank(0, 612.0, 792.0).unwrap();
        assert!(app.add_tab(&ctx, pdf, None, None, false));
        let edge = |app: &mut App, name: &str| {
            let tree = settled_at(app, &ctx, 1400.0);
            let (at, _) = node(&tree, name).expect("the edge");
            at.center()
        };
        app.show_view(Side::Comments);
        let comments = edge(&mut app, "Side panel width").x - RAIL_WIDTH;
        assert!((comments - 260.0).abs() <= 8.0, "comments: {comments}");
        // Watermark's controls widen the inspector: not a width the user gave it.
        app.act(&ctx, Act::Watermark);
        let from = edge(&mut app, "Inspector width");
        assert!(1400.0 - from.x > INSPECTOR_WIDTH + 8.0, "not widened");
        assert_eq!(app.settings.panel_widths.get("inspector"), None);
        // Dragged 40 points wider, then let go: from the panel's side of its edge, as the
        // pages' scroll area takes a drag on the other side (PB-33).
        let from = from + egui::vec2(3.0, 0.0);
        let to = from - egui::vec2(40.0, 0.0);
        let over = egui::Event::PointerMoved(from);
        window(&mut app, &ctx, 1400.0, vec![over]);
        window(&mut app, &ctx, 1400.0, vec![press(from, true)]);
        for i in 1..=4 {
            let at = from + (to - from) * (i as f32 / 4.0);
            window(&mut app, &ctx, 1400.0, vec![egui::Event::PointerMoved(at)]);
        }
        let held = app.settings.panel_widths.get("inspector").copied();
        assert_eq!(held, None, "kept while the edge is held");
        window(&mut app, &ctx, 1400.0, vec![press(to, false)]);
        let given = 1400.0 - edge(&mut app, "Inspector width").x;
        let kept = app.settings.panel_widths.get("inspector").copied();
        let near = kept.is_some_and(|w| (w - given).abs() <= 8.0);
        assert!(near, "kept {kept:?} for {given}");
        // The next run starts there.
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        theme::install(&ctx);
        let mut settings = Settings::from_file(None);
        settings.panel_widths = app.settings.panel_widths.clone();
        let mut next = App::with(settings, Recent::from_file(None));
        let mut pdf = Pdf::new();
        pdf.insert_blank(0, 612.0, 792.0).unwrap();
        assert!(next.add_tab(&ctx, pdf, None, None, false));
        let tree = settled_at(&mut next, &ctx, 1400.0);
        let (at, _) = node(&tree, "Inspector width").expect("the edge");
        let width = 1400.0 - at.center().x;
        assert!((width - given).abs() <= 1.0, "{width}, not {given}");
    }

    /// A26: with characters selected in a line edited on the page, the first colour picked
    /// makes those characters that colour and leaves the rest as it was.
    #[test]
    fn a_colour_picked_makes_mixed_selected_characters_one_colour() {
        let (black, red) = ([0.0; 3], [1.0, 0.0, 0.0]);
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        theme::install(&ctx);
        let mut app = app_in_colours(&ctx, black, red);
        let time = std::cell::Cell::new(10.0);
        let mut frame = |app: &mut App, events| timed_frame(app, &ctx, &time, events);
        let none = egui::Modifiers::NONE;
        // "ate fish" selected: red, then black.
        let mut tree = edit_ate_fish(&mut app, &mut frame);
        let name = "Editing text: Walrus ate fish";
        // Red, the first, picked in the colour area.
        click_with(&mut app, &mut frame, &tree, "Text colour (mixed)");
        click_in_colour_area(&mut app, &ctx, &mut frame, egui::vec2(1.0, 1.0));
        // Esc closes the picker; the line takes the keyboard back, the same characters
        // selected, and they are red.
        frame(&mut app, vec![key_with(Key::Escape, none)]);
        for _ in 0..3 {
            tree = frame(&mut app, vec![]);
        }
        let open = node(&tree, "Colour 1 of 2 in the text").is_some();
        assert!(!open, "still open");
        assert_eq!(node(&tree, name).map(|(_, f)| f), Some(true));
        let one = node(&tree, "Text colour").is_some();
        assert!(one, "the selection isn't one colour");
        // Esc in the line keeps the edit.
        frame(&mut app, vec![key_with(Key::Escape, none)]);
        for _ in 0..3 {
            frame(&mut app, vec![]);
        }
        assert_eq!(line_colours(&app), [(7, Some(black)), (15, Some(red))]);
        app.act(&ctx, Act::Undo);
        let mixed = [(7, Some(black)), (10, Some(red)), (15, Some(black))];
        assert_eq!(line_colours(&app), mixed);
    }

    /// Esc in the band's size box, with a size typed there, drops what was typed and hands
    /// the keyboard back to the line being edited, as Esc in a word processor's size box
    /// goes back to the text: the line stays open, its sizes as they were.
    #[test]
    fn esc_in_the_size_box_goes_back_to_the_line() {
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        theme::install(&ctx);
        let mut app = app_in_colours(&ctx, [0.0; 3], [1.0, 0.0, 0.0]);
        let time = std::cell::Cell::new(10.0);
        let mut frame = |app: &mut App, events| timed_frame(app, &ctx, &time, events);
        let tree = edit_ate_fish(&mut app, &mut frame);
        click_with(&mut app, &mut frame, &tree, "Font size");
        frame(&mut app, vec![key_with(Key::A, CTRL)]);
        frame(&mut app, vec![egui::Event::Text("60".into())]);
        frame(&mut app, vec![key_with(Key::Escape, egui::Modifiers::NONE)]);
        let mut tree = frame(&mut app, vec![]);
        for _ in 0..4 {
            tree = frame(&mut app, vec![]);
        }
        let name = "Editing text: Walrus ate fish";
        assert_eq!(
            node(&tree, name).map(|(_, f)| f),
            Some(true),
            "not back in the line"
        );
        // Kept as it is, the line has its sizes as before.
        app.tabs[0].keep_typing(&ctx);
        let (text, spans) = parts_of_line(&app);
        assert_eq!(text, "Walrus ate fish");
        assert!(spans.iter().all(|s| s.3 == 30.0), "{spans:?}");
    }

    /// The inspector's controls change what is selected in a line edited on the page,
    /// although the line gives them the keyboard (egui then keeps only its caret): a size
    /// from the list sizes the selected characters, and the line gets them back selected.
    #[test]
    fn a_size_picked_sizes_the_selected_characters_of_a_line() {
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        theme::install(&ctx);
        let mut app = app_in_colours(&ctx, [0.0; 3], [1.0, 0.0, 0.0]);
        let time = std::cell::Cell::new(10.0);
        let mut frame = |app: &mut App, events| timed_frame(app, &ctx, &time, events);
        let tree = edit_ate_fish(&mut app, &mut frame);
        let tree = click_with(&mut app, &mut frame, &tree, "Font sizes");
        click_with(&mut app, &mut frame, &tree, "12 pt");
        let mut tree = frame(&mut app, vec![]);
        for _ in 0..3 {
            tree = frame(&mut app, vec![]);
        }
        let name = "Editing text: Walrus ate fish";
        assert_eq!(node(&tree, name).map(|(_, f)| f), Some(true));
        // Still selected: what is typed replaces it.
        frame(&mut app, vec![egui::Event::Text("eats".into())]);
        frame(&mut app, vec![key_with(Key::Escape, egui::Modifiers::NONE)]);
        for _ in 0..3 {
            frame(&mut app, vec![]);
        }
        let (text, spans) = parts_of_line(&app);
        assert_eq!(text, "Walrus eats");
        let sizes: Vec<_> = spans.iter().map(|s| (s.0, s.1, s.3)).collect();
        assert_eq!(sizes, [(0, 7, 30.0), (7, 11, 12.0)]);
    }

    /// As in a line edited on the page, a size picked for new text being typed sizes what
    /// is selected in it, which stays selected.
    #[test]
    fn a_size_picked_sizes_the_selected_characters_of_new_text() {
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        theme::install(&ctx);
        let mut app = app_with(&ctx, 1);
        app.act(&ctx, Act::Tool(Tool::Text));
        let time = std::cell::Cell::new(10.0);
        let mut frame = |app: &mut App, events| timed_frame(app, &ctx, &time, events);
        let mut tree = frame(&mut app, vec![]);
        for _ in 0..4 {
            tree = frame(&mut app, vec![]);
        }
        let (page, _) = node(&tree, "Page 1").expect("the page");
        let p = page.min + egui::vec2(100.0, 200.0) * (page.width() / 612.0);
        frame(&mut app, vec![egui::Event::PointerMoved(p)]);
        frame(&mut app, vec![press(p, true)]);
        frame(&mut app, vec![press(p, false)]);
        for _ in 0..3 {
            frame(&mut app, vec![]);
        }
        frame(&mut app, vec![egui::Event::Text("Walrus".into())]);
        frame(&mut app, vec![key_with(Key::Home, egui::Modifiers::SHIFT)]);
        let tree = frame(&mut app, vec![]);
        let tree = click_with(&mut app, &mut frame, &tree, "Font sizes");
        click_with(&mut app, &mut frame, &tree, "12 pt");
        for _ in 0..4 {
            frame(&mut app, vec![]);
        }
        // Still selected: what is typed replaces it.
        frame(&mut app, vec![egui::Event::Text("Seal".into())]);
        frame(&mut app, vec![key_with(Key::Escape, egui::Modifiers::NONE)]);
        for _ in 0..3 {
            frame(&mut app, vec![]);
        }
        let (text, spans) = parts_of_line(&app);
        assert_eq!(text, "Seal");
        assert!(spans.iter().all(|s| s.3 == 12.0), "{spans:?}");
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
                    egui::vec2(1400.0, TEST_HEIGHT),
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
        let id = app.tabs[0].ed.scope().with(("line-edit", 0usize));
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
                    egui::vec2(1400.0, TEST_HEIGHT),
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

    /// A drag on page 1 from page point `from` to `to` in four steps, with Esc pressed
    /// after the second when `esc`, then let go.
    fn drag_on_page(
        app: &mut App,
        ctx: &egui::Context,
        from: (f32, f32),
        to: (f32, f32),
        esc: bool,
    ) {
        let (min, k) = page_on_screen(app, ctx);
        let at = |(x, y): (f32, f32)| min + egui::vec2(x, y) * k;
        let (a, b) = (at(from), at(to));
        window(app, ctx, 1400.0, vec![egui::Event::PointerMoved(a)]);
        window(app, ctx, 1400.0, vec![press(a, true)]);
        for i in 1..=4 {
            let p = a + (b - a) * (i as f32 / 4.0);
            window(app, ctx, 1400.0, vec![egui::Event::PointerMoved(p)]);
            if esc && i == 2 {
                let key = key_with(Key::Escape, egui::Modifiers::NONE);
                window(app, ctx, 1400.0, vec![key]);
            }
        }
        window(app, ctx, 1400.0, vec![press(b, false)]);
        settle(app, ctx);
    }

    /// A move whose page goes out of view before the button is let go still ends when it is
    /// (PB-14): nothing is left being dragged, and the line is where the move put it.
    #[test]
    fn a_drag_ends_when_let_go_with_its_page_out_of_view() {
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        theme::install(&ctx);
        let mut app = app_with(&ctx, 1);
        let courier = FontChoice::from_name("Courier");
        assert!(app.tabs[0].edit(&ctx, |p| {
            p.add_text(0, (100.0, 200.0), "Walrus seal", courier, 40.0, [0.0; 3])?;
            p.insert_blank(2, 612.0, 792.0)
        }));
        let place = |app: &App| app.tabs[0].pdf.find_text_boxes(0, "Walrus").unwrap();
        let before = place(&app);
        app.act(&ctx, Act::Tool(Tool::Select));
        let (min, k) = page_on_screen(&mut app, &ctx);
        let at = |x: f32, y: f32| min + egui::vec2(x, y) * k;
        window(&mut app, &ctx, 1400.0, click_at(at(150.0, 190.0)));
        settle(&mut app, &ctx);
        window(
            &mut app,
            &ctx,
            1400.0,
            vec![egui::Event::PointerMoved(at(150.0, 190.0))],
        );
        window(&mut app, &ctx, 1400.0, vec![press(at(150.0, 190.0), true)]);
        for y in [210.0, 230.0, 250.0] {
            window(
                &mut app,
                &ctx,
                1400.0,
                vec![egui::Event::PointerMoved(at(150.0, y))],
            );
        }
        assert!(app.tabs[0].ed.dragging());
        // Page 2 at the top of the view: page 1 is no longer drawn.
        app.tabs[0].go_to(2);
        let mut tree = window(&mut app, &ctx, 1400.0, vec![]);
        for _ in 0..3 {
            tree = window(&mut app, &ctx, 1400.0, vec![]);
        }
        // Its thumbnail in the side panel has the same name.
        let page_1 = node(&tree, "Page 1").filter(|(r, _)| r.width() > 500.0);
        assert!(page_1.is_none_or(|(r, _)| r.max.y < 0.0), "page 1 in view");
        window(&mut app, &ctx, 1400.0, vec![press(at(150.0, 250.0), false)]);
        settle(&mut app, &ctx);
        assert!(
            !app.tabs[0].ed.dragging(),
            "still dragging after the button was let go"
        );
        assert_ne!(place(&app), before, "the move was lost");
    }

    /// Esc during a move, a pen stroke or a word selection made by dragging cancels it,
    /// whatever the pointer does after (PB-14): nothing moves or is drawn, and the tool
    /// stays for another try.
    #[test]
    fn esc_cancels_a_drag_under_way() {
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        theme::install(&ctx);
        let mut app = app_with(&ctx, 1);
        let courier = FontChoice::from_name("Courier");
        assert!(app.tabs[0].edit(&ctx, |p| {
            p.add_text(0, (100.0, 200.0), "Walrus seal", courier, 40.0, [0.0; 3])
        }));
        let place = |app: &App| app.tabs[0].pdf.find_text_boxes(0, "Walrus").unwrap();
        let before = place(&app);

        // A move with the arrow.
        app.act(&ctx, Act::Tool(Tool::Select));
        let (min, k) = page_on_screen(&mut app, &ctx);
        window(
            &mut app,
            &ctx,
            1400.0,
            click_at(min + egui::vec2(150.0, 190.0) * k),
        );
        settle(&mut app, &ctx);
        assert!(app.tabs[0].ed.sel().is_some(), "the line is selected");
        drag_on_page(&mut app, &ctx, (150.0, 190.0), (150.0, 400.0), true);
        assert_eq!(place(&app), before, "moved after Esc");
        assert!(!app.tabs[0].ed.dragging());

        // A pen stroke: nothing drawn; the pen stays, and a stroke after draws.
        app.act(&ctx, Act::Tool(Tool::Pen));
        drag_on_page(&mut app, &ctx, (100.0, 300.0), (300.0, 350.0), true);
        assert!(app.tabs[0].pdf.annotations().is_empty(), "drawn after Esc");
        assert_eq!(app.tabs[0].ed.tool, Tool::Pen);
        drag_on_page(&mut app, &ctx, (100.0, 300.0), (300.0, 350.0), false);
        assert_eq!(app.tabs[0].pdf.annotations().len(), 1);

        // Words selected by dragging with the read tool: none left selected.
        app.act(&ctx, Act::Tool(Tool::Read));
        drag_on_page(&mut app, &ctx, (102.0, 190.0), (340.0, 190.0), true);
        let ed = &app.tabs[0].ed;
        assert!(ed.selection.words().is_none(), "{:?}", ed.selection);
        assert!(!ed.dragging());
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

    /// [`window`] with `held` keys (Shift, say) down from this frame on, as egui reads them
    /// from its own state rather than from each pointer event.
    fn window_held(
        app: &mut App,
        ctx: &egui::Context,
        events: Vec<egui::Event>,
        held: egui::Modifiers,
    ) -> egui::accesskit::TreeUpdate {
        let mut all = vec![egui::Event::ModifiersChanged(held)];
        all.extend(events);
        window(app, ctx, 1400.0, all)
    }

    /// A tab whose first page has one object of each kind, in this order (page points, y
    /// down), all in the test window: a red shape at 300,60 to 380,120; lines "Walrus one",
    /// "Walrus two" and "Walrus three" on baselines 92, 122 and 152 from x 72; a picture at
    /// 300,150 to 400,230; a group (a form XObject) at 300,250 to 350,300; and invisible
    /// (OCR) text, "hidden words", on baseline 330. Its second page has a shape at 100,100
    /// to 200,150.
    fn objects_tab(ctx: &egui::Context) -> App {
        use lopdf::{dictionary, Object, Stream};
        let mut app = app_with(ctx, 1);
        let shapes = |p: &mut Pdf| {
            let page = p.page_ids()[0];
            let font = p.doc.add_object(dictionary! {
                "Type" => "Font", "Subtype" => "Type1", "BaseFont" => "Helvetica",
                "Encoding" => "WinAnsiEncoding",
            });
            let image = dictionary! {
                "Type" => "XObject", "Subtype" => "Image", "Width" => 1, "Height" => 1,
                "ColorSpace" => "DeviceRGB", "BitsPerComponent" => 8,
            };
            let image = p.doc.add_object(Stream::new(image, vec![0, 128, 0]));
            let bbox: Vec<Object> = [0, 0, 50, 50].map(Object::from).to_vec();
            let form = dictionary! { "Type" => "XObject", "Subtype" => "Form", "BBox" => bbox };
            let form = p
                .doc
                .add_object(Stream::new(form, b"0 0 1 rg 0 0 50 50 re f".to_vec()));
            let content = "q 1 0 0 rg 300 672 80 60 re f Q\n\
                           BT /F1 18 Tf 72 700 Td (Walrus one) Tj ET\n\
                           BT /F1 18 Tf 72 670 Td (Walrus two) Tj ET\n\
                           BT /F1 18 Tf 72 640 Td (Walrus three) Tj ET\n\
                           q 100 0 0 80 300 562 cm /Im1 Do Q\n\
                           q 1 0 0 1 300 492 cm /Fm1 Do Q\n\
                           BT 3 Tr /F1 12 Tf 72 462 Td (hidden words) Tj ET\n";
            let s = p.doc.add_object(Stream::new(
                lopdf::Dictionary::new(),
                content.as_bytes().to_vec(),
            ));
            let res = dictionary! {
                "Font" => dictionary! { "F1" => font },
                "XObject" => dictionary! { "Im1" => image, "Fm1" => form },
            };
            let d = p.doc.get_dictionary_mut(page)?;
            d.set("Contents", Object::Reference(s));
            d.set("Resources", res);
            let st = limarepdf_core::PathStyle {
                fill: Some([0.0, 0.0, 1.0]),
                stroke: None,
                line_width: 1.0,
            };
            p.add_shape(
                1,
                limarepdf_core::Shape::Rect,
                [100.0, 100.0, 200.0, 150.0],
                &st,
            )
        };
        assert!(app.tabs[0].edit(ctx, shapes));
        let kinds: Vec<_> = (app.tabs[0].pdf.page_objects(0).unwrap().iter())
            .map(|o| (o.kind, o.hidden))
            .collect();
        use limarepdf_core::ObjectKind::*;
        let text = (Text, false);
        assert_eq!(
            kinds,
            [
                (Path, false),
                text,
                text,
                text,
                (Image, false),
                (Form, false),
                (Text, true)
            ]
        );
        app
    }

    /// The objects selected in the tab in front: page and indices, in the order selected.
    fn held(app: &App) -> Option<(usize, Vec<usize>)> {
        app.tabs[app.active].ed.selection.object_ids()
    }

    /// Select objects `ids` of page 1 in the tab in front, once the window has read them.
    fn select(app: &mut App, ctx: &egui::Context, ids: &[usize]) {
        settle(app, ctx);
        app.tabs[app.active]
            .ed
            .select_objects(0, ids.iter().copied());
        assert_eq!(held(app), Some((0, ids.to_vec())));
    }

    /// Where each object of page `page` is now.
    fn boxes(app: &App, page: usize) -> Vec<[f32; 4]> {
        let objs = app.tabs[app.active].pdf.page_objects(page).unwrap();
        objs.iter().map(|o| o.bbox).collect()
    }

    /// Click at page point `at` (x, y) on the page in view, Shift held or not.
    fn click_page(app: &mut App, ctx: &egui::Context, at: (f32, f32), shift: bool) {
        click_held(app, ctx, at, shift, 0);
    }

    /// [`click_page`], the button held a second without moving: longer than egui takes a
    /// press for a click, so it is a drag that goes nowhere.
    fn slow_click_page(app: &mut App, ctx: &egui::Context, at: (f32, f32), shift: bool) {
        click_held(app, ctx, at, shift, 60);
    }

    /// Click at page point `at`, the button held down for `frames` frames of 1/60 s.
    fn click_held(app: &mut App, ctx: &egui::Context, at: (f32, f32), shift: bool, frames: usize) {
        let (min, k) = page_on_screen(app, ctx);
        let p = min + egui::vec2(at.0, at.1) * k;
        let m = match shift {
            true => egui::Modifiers::SHIFT,
            false => egui::Modifiers::NONE,
        };
        window_held(app, ctx, vec![egui::Event::PointerMoved(p)], m);
        window_held(app, ctx, vec![press(p, true)], m);
        for _ in 0..frames {
            window_held(app, ctx, vec![], m);
        }
        window_held(app, ctx, vec![press(p, false)], m);
        window_held(app, ctx, vec![], egui::Modifiers::NONE);
        settle(app, ctx);
    }

    /// Drag from page point `from` to `to` on the page in view, Shift held or not.
    fn drag_page(
        app: &mut App,
        ctx: &egui::Context,
        from: (f32, f32),
        to: (f32, f32),
        shift: bool,
    ) {
        let (min, k) = page_on_screen(app, ctx);
        let at = |(x, y): (f32, f32)| min + egui::vec2(x, y) * k;
        let (a, b) = (at(from), at(to));
        let m = match shift {
            true => egui::Modifiers::SHIFT,
            false => egui::Modifiers::NONE,
        };
        window_held(app, ctx, vec![egui::Event::PointerMoved(a)], m);
        window_held(app, ctx, vec![press(a, true)], m);
        for i in 1..=4 {
            let p = a + (b - a) * (i as f32 / 4.0);
            window_held(app, ctx, vec![egui::Event::PointerMoved(p)], m);
        }
        window_held(app, ctx, vec![press(b, false)], m);
        window_held(app, ctx, vec![], egui::Modifiers::NONE);
        settle(app, ctx);
    }

    /// ENH-06: Shift+click with the arrow adds an object to those selected, or takes it out;
    /// a plain click selects one alone; Shift+click on nothing changes nothing; an object on
    /// another page starts afresh there.
    #[test]
    fn shift_click_adds_and_takes_out_objects() {
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        theme::install(&ctx);
        let mut app = objects_tab(&ctx);
        app.act(&ctx, Act::Tool(Tool::Select));
        click_page(&mut app, &ctx, (340.0, 90.0), false);
        assert_eq!(held(&app), Some((0, vec![0])), "the shape");
        click_page(&mut app, &ctx, (350.0, 190.0), true);
        click_page(&mut app, &ctx, (325.0, 275.0), true);
        assert_eq!(
            held(&app),
            Some((0, vec![0, 4, 5])),
            "shape, picture and group"
        );
        assert_eq!(app.tabs[0].ed.heading(), "3 objects");
        click_page(&mut app, &ctx, (350.0, 190.0), true);
        assert_eq!(held(&app), Some((0, vec![0, 5])), "the picture taken out");
        click_page(&mut app, &ctx, (500.0, 40.0), true);
        assert_eq!(held(&app), Some((0, vec![0, 5])), "Shift+click on nothing");
        click_page(&mut app, &ctx, (90.0, 117.0), false);
        assert_eq!(
            held(&app),
            Some((0, vec![2])),
            "a plain click: that line alone"
        );
        // Taking the last one out leaves nothing selected.
        click_page(&mut app, &ctx, (90.0, 117.0), true);
        assert_eq!(held(&app), None);
        // Another page's object: it alone, on its page.
        click_page(&mut app, &ctx, (340.0, 90.0), false);
        app.tabs[0].go_to(1);
        settle(&mut app, &ctx);
        app.tabs[0].ed.toggle_object(1, 0);
        assert_eq!(held(&app), Some((1, vec![0])));
    }

    /// ENH-06: a rectangle dragged with the arrow selects what is wholly inside it, not
    /// what it only touches; with Shift it adds to what is selected; over nothing it clears
    /// the selection; invisible text joins only while it is shown; Esc lets go of one being
    /// drawn, changing nothing.
    #[test]
    fn a_rectangle_selects_what_is_wholly_inside_it() {
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        theme::install(&ctx);
        let mut app = objects_tab(&ctx);
        app.act(&ctx, Act::Tool(Tool::Select));
        // Round the first two lines, and through the third (on baseline 152).
        drag_page(&mut app, &ctx, (60.0, 60.0), (250.0, 140.0), false);
        assert_eq!(held(&app), Some((0, vec![1, 2])));
        // With Shift, round the picture: added. Half over the group: not.
        drag_page(&mut app, &ctx, (290.0, 140.0), (410.0, 240.0), true);
        drag_page(&mut app, &ctx, (290.0, 240.0), (325.0, 310.0), true);
        assert_eq!(held(&app), Some((0, vec![1, 2, 4])));
        // Round the invisible text: nothing there to select, so nothing is.
        drag_page(&mut app, &ctx, (60.0, 310.0), (250.0, 345.0), false);
        assert_eq!(held(&app), None);
        app.tabs[0].ed.show_hidden = true;
        drag_page(&mut app, &ctx, (60.0, 310.0), (250.0, 345.0), false);
        assert_eq!(held(&app), Some((0, vec![6])), "shown, it is selected");
        app.tabs[0].ed.show_hidden = false;
        settle(&mut app, &ctx);
        assert_eq!(held(&app), None, "hidden again, it is let go");
        // Esc while drawing one: nothing selected, nothing being dragged.
        select(&mut app, &ctx, &[0]);
        drag_on_page(&mut app, &ctx, (60.0, 60.0), (250.0, 140.0), true);
        assert_eq!(held(&app), Some((0, vec![0])), "Esc changed the selection");
        assert!(!app.tabs[0].ed.dragging());
    }

    /// ENH-06: Ctrl+A with the arrow selects every object on the page, invisible text only
    /// while it is shown; with the read tool it still selects the page's text.
    #[test]
    fn ctrl_a_with_the_arrow_selects_the_page_objects() {
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        theme::install(&ctx);
        let mut app = objects_tab(&ctx);
        app.act(&ctx, Act::Tool(Tool::Select));
        settle(&mut app, &ctx);
        let ctrl_a = key_with(Key::A, egui::Modifiers::COMMAND);
        window(&mut app, &ctx, 1400.0, vec![ctrl_a.clone()]);
        assert_eq!(held(&app), Some((0, (0..6).collect())));
        app.tabs[0].ed.show_hidden = true;
        window(&mut app, &ctx, 1400.0, vec![ctrl_a.clone()]);
        assert_eq!(held(&app), Some((0, (0..7).collect())));
        app.act(&ctx, Act::Tool(Tool::Read));
        window(&mut app, &ctx, 1400.0, vec![ctrl_a]);
        assert!(app.tabs[0].ed.selection.words().is_some());
    }

    /// ENH-06: objects selected together move together, each by the same amount, whether
    /// dragged by any one of them or nudged with the arrow keys, and one Undo puts them all
    /// back; the others stay where they were.
    #[test]
    fn several_objects_move_together_with_one_undo() {
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        theme::install(&ctx);
        let mut app = objects_tab(&ctx);
        app.act(&ctx, Act::Tool(Tool::Select));
        select(&mut app, &ctx, &[0, 2, 4, 5]);
        let (before, steps) = (boxes(&app, 0), app.tabs[0].history().1);
        // Dragged by the picture.
        drag_page(&mut app, &ctx, (350.0, 190.0), (390.0, 220.0), false);
        let after = boxes(&app, 0);
        assert_eq!(app.tabs[0].history().1, steps + 1, "one undo step");
        for (i, (a, b)) in after.iter().zip(&before).enumerate() {
            let moved = [0, 2, 4, 5].contains(&i);
            let (dx, dy) = if moved { (40.0, 30.0) } else { (0.0, 0.0) };
            let d = [
                a[0] - b[0] - dx,
                a[1] - b[1] - dy,
                a[2] - b[2] - dx,
                a[3] - b[3] - dy,
            ];
            assert!(
                d.iter().all(|v| v.abs() < 0.6),
                "object {i}: {b:?} to {a:?}"
            );
        }
        assert_eq!(held(&app), Some((0, vec![0, 2, 4, 5])), "still selected");
        // Nudged: each a point to the right.
        let key = key_with(Key::ArrowRight, egui::Modifiers::NONE);
        window(&mut app, &ctx, 1400.0, vec![key]);
        settle(&mut app, &ctx);
        let nudged = boxes(&app, 0);
        for i in [0, 2, 4, 5] {
            assert!(
                (nudged[i][0] - after[i][0] - 1.0).abs() < 0.05,
                "object {i}"
            );
        }
        assert!((nudged[1][0] - after[1][0]).abs() < 0.05, "line one stayed");
        // Undo twice: all where they started.
        app.act(&ctx, Act::Undo);
        app.act(&ctx, Act::Undo);
        let back = boxes(&app, 0);
        for (a, b) in back.iter().zip(&before) {
            assert!(
                a.iter().zip(b).all(|(p, q)| (p - q).abs() < 0.05),
                "{b:?} to {a:?}"
            );
        }
    }

    /// ENH-06: the inspector's X and Y for several objects are those of the box round them
    /// all, and move them together; there is no width or height for several.
    #[test]
    fn several_objects_move_by_their_position_in_the_inspector() {
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        theme::install(&ctx);
        let mut app = objects_tab(&ctx);
        app.act(&ctx, Act::Tool(Tool::Select));
        app.open_inspector();
        select(&mut app, &ctx, &[1, 4]);
        let before = boxes(&app, 0);
        let tree = settled_at(&mut app, &ctx, 1400.0);
        assert_eq!(inspector::title(&app.tabs[0]), "2 objects");
        click_named(&mut app, &ctx, "Position");
        let tree_open = settled_at(&mut app, &ctx, 1400.0);
        assert!(node(&tree, "X").is_none() && node(&tree_open, "X").is_some());
        assert!(node(&tree_open, "Width").is_none(), "a width for several");
        click_named(&mut app, &ctx, "X");
        let (ctrl, none) = (egui::Modifiers::COMMAND, egui::Modifiers::NONE);
        window(&mut app, &ctx, 1400.0, vec![key_with(Key::A, ctrl)]);
        window(
            &mut app,
            &ctx,
            1400.0,
            vec![egui::Event::Text("100".into())],
        );
        window(&mut app, &ctx, 1400.0, vec![key_with(Key::Enter, none)]);
        settle(&mut app, &ctx);
        // The box round them started at the line's left, 72: both 28 to the right.
        let after = boxes(&app, 0);
        for (i, (a, b)) in after.iter().zip(&before).enumerate() {
            let dx = if [1, 4].contains(&i) { 28.0 } else { 0.0 };
            assert!((a[0] - b[0] - dx).abs() < 0.6, "object {i}: {b:?} to {a:?}");
            assert!((a[1] - b[1]).abs() < 0.05, "object {i} moved up or down");
        }
    }

    /// ENH-06: Delete deletes every object selected, as one step a notice counts and Undo
    /// takes back; the others stay.
    #[test]
    fn several_objects_delete_with_one_undo() {
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        theme::install(&ctx);
        let mut app = objects_tab(&ctx);
        app.act(&ctx, Act::Tool(Tool::Select));
        let before = app.tabs[0].pdf.page_objects(0).unwrap();
        select(&mut app, &ctx, &[1, 3, 4]);
        let key = key_with(Key::Delete, egui::Modifiers::NONE);
        window(&mut app, &ctx, 1400.0, vec![key]);
        settle(&mut app, &ctx);
        let left: Vec<_> = (app.tabs[0].pdf.page_objects(0).unwrap().into_iter())
            .map(|o| o.text.unwrap_or_else(|| format!("{:?}", o.kind)))
            .collect();
        assert_eq!(left, ["Path", "Walrus two", "Form", "hidden words"]);
        assert_eq!(toast(&app).as_deref(), Some("Deleted 3 objects"));
        assert_eq!(held(&app), None);
        app.act(&ctx, Act::Undo);
        let back = app.tabs[0].pdf.page_objects(0).unwrap();
        let text = |v: &[limarepdf_core::PageObject]| -> Vec<_> {
            v.iter().map(|o| (o.kind, o.text.clone(), o.bbox)).collect()
        };
        assert_eq!(text(&back), text(&before));
    }

    /// ENH-06: Bold on several objects selected changes the lines of text among them (each
    /// whole, as one undo step) and nothing else; rewritten, the lines go to the end of the
    /// page's content, and the same objects stay selected, so Delete then takes those.
    #[test]
    fn a_change_that_reorders_the_page_keeps_the_same_objects() {
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        theme::install(&ctx);
        let mut app = objects_tab(&ctx);
        app.act(&ctx, Act::Tool(Tool::Select));
        select(&mut app, &ctx, &[1, 3, 0]);
        let steps = app.tabs[0].history().1;
        app.act(&ctx, Act::Bold);
        settle(&mut app, &ctx);
        assert_eq!(app.tabs[0].history().1, steps + 1, "one undo step");
        let objs = app.tabs[0].pdf.page_objects(0).unwrap();
        let font = |t: &str| {
            let o = objs.iter().find(|o| o.text.as_deref() == Some(t)).unwrap();
            o.font_name.clone().unwrap_or_default()
        };
        assert!(
            font("Walrus one").contains("Bold"),
            "{}",
            font("Walrus one")
        );
        assert!(font("Walrus three").contains("Bold"));
        assert!(!font("Walrus two").contains("Bold"), "not selected");
        let (page, ids) = held(&app).expect("still selected");
        let now: Vec<_> = ids.iter().map(|&i| objs[i].text.clone()).collect();
        assert_eq!(page, 0);
        assert_eq!(
            now,
            [Some("Walrus one".into()), Some("Walrus three".into()), None]
        );
        assert_eq!(objs[ids[2]].kind, limarepdf_core::ObjectKind::Path);
        assert_ne!(ids, [1, 3, 0], "the lines were rewritten at the end");
        app.act(&ctx, Act::DeleteSelection);
        settle(&mut app, &ctx);
        let left: Vec<_> = (app.tabs[0].pdf.page_objects(0).unwrap().into_iter())
            .map(|o| o.text.unwrap_or_else(|| format!("{:?}", o.kind)))
            .collect();
        assert_eq!(left, ["Walrus two", "Image", "Form", "hidden words"]);
    }

    /// ENH-06: a change to several objects that one of them can't take changes none of
    /// them, says so, and leaves them selected.
    #[test]
    fn a_change_to_several_refused_changes_nothing() {
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        theme::install(&ctx);
        let mut app = objects_tab(&ctx);
        app.act(&ctx, Act::Tool(Tool::Select));
        select(&mut app, &ctx, &[0, 4]);
        let (before, revision) = (boxes(&app, 0), app.tabs[0].history());
        let tiny = [300.0, 150.0, 300.2, 150.2];
        let t = &mut app.tabs[0];
        t.ed.actions.push(EdAction::SetBox(
            0,
            vec![(0, [310.0, 60.0, 390.0, 120.0]), (4, tiny)],
        ));
        t.apply_actions(&ctx);
        assert_eq!(app.tabs[0].history(), revision, "changed");
        assert_eq!(boxes(&app, 0), before);
        let said = app.tabs[0].error.clone().unwrap_or_default();
        assert!(
            said.starts_with("One of the selected objects couldn't be moved, so none of them was"),
            "{said}"
        );
        assert_eq!(held(&app), Some((0, vec![0, 4])));
    }

    /// ENH-06: new text of several lines is selected as one when it is put down, and moves
    /// together with the read tool.
    #[test]
    fn new_text_of_several_lines_is_selected_as_one() {
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        theme::install(&ctx);
        let mut app = app_with(&ctx, 1);
        app.act(&ctx, Act::Tool(Tool::Text));
        let ed = &mut app.tabs[0].ed;
        ed.tsize = 30.0;
        let mut a = page_text::PageText::new(ed.scope(), 0, (100.0, 200.0), ed.new_look());
        a.text.sync("Walrus\nseal\nfish", None);
        ed.adding = Some(a);
        ed.escape();
        app.tabs[0].apply_actions(&ctx);
        settle(&mut app, &ctx);
        assert_eq!(app.tabs[0].ed.tool, Tool::Read);
        assert_eq!(held(&app), Some((0, vec![0, 1, 2])));
        let before = boxes(&app, 0);
        drag_page(&mut app, &ctx, (110.0, 190.0), (160.0, 240.0), false);
        let after = boxes(&app, 0);
        for (a, b) in after.iter().zip(&before) {
            assert!((a[0] - b[0] - 50.0).abs() < 0.6 && (a[1] - b[1] - 50.0).abs() < 0.6);
        }
    }

    /// ENH-06: a drag that starts on a background (an object covering most of the page)
    /// that isn't selected draws a rectangle instead of moving it; a click still selects
    /// it, and then it moves.
    #[test]
    fn a_background_not_selected_is_not_dragged_away() {
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        theme::install(&ctx);
        let mut app = app_with(&ctx, 1);
        let st = limarepdf_core::PathStyle {
            fill: Some([0.9, 0.9, 0.9]),
            stroke: None,
            line_width: 1.0,
        };
        let rect = |p: &mut Pdf, b| p.add_shape(0, limarepdf_core::Shape::Rect, b, &st);
        assert!(app.tabs[0].edit(&ctx, |p| {
            rect(p, [0.0, 0.0, 612.0, 792.0])?;
            rect(p, [100.0, 100.0, 150.0, 130.0])
        }));
        app.act(&ctx, Act::Tool(Tool::Select));
        let before = boxes(&app, 0);
        drag_page(&mut app, &ctx, (80.0, 80.0), (200.0, 200.0), false);
        assert_eq!(boxes(&app, 0), before, "the background moved");
        assert_eq!(
            held(&app),
            Some((0, vec![1])),
            "the shape inside was selected"
        );
        click_page(&mut app, &ctx, (400.0, 400.0), false);
        assert_eq!(
            held(&app),
            Some((0, vec![0])),
            "a click selects the background"
        );
        drag_page(&mut app, &ctx, (400.0, 400.0), (420.0, 400.0), false);
        assert!(
            (boxes(&app, 0)[0][0] - 20.0).abs() < 0.6,
            "selected, it moves"
        );
    }

    /// ENH-06: a right-click on one of the objects selected keeps them all, for the menu
    /// for them; on another object it selects that one.
    #[test]
    fn a_right_click_on_one_of_several_keeps_them() {
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        theme::install(&ctx);
        let mut app = objects_tab(&ctx);
        app.act(&ctx, Act::Tool(Tool::Select));
        select(&mut app, &ctx, &[0, 4]);
        let (min, k) = page_on_screen(&mut app, &ctx);
        let right = |at: (f32, f32)| {
            let p = min + egui::vec2(at.0, at.1) * k;
            let button = |pressed| egui::Event::PointerButton {
                pos: p,
                button: egui::PointerButton::Secondary,
                pressed,
                modifiers: Default::default(),
            };
            vec![egui::Event::PointerMoved(p), button(true), button(false)]
        };
        window(&mut app, &ctx, 1400.0, right((350.0, 190.0)));
        settle(&mut app, &ctx);
        assert_eq!(held(&app), Some((0, vec![0, 4])));
        let esc = key_with(Key::Escape, egui::Modifiers::NONE);
        window(&mut app, &ctx, 1400.0, vec![esc]);
        settle(&mut app, &ctx);
        window(&mut app, &ctx, 1400.0, right((325.0, 275.0)));
        settle(&mut app, &ctx);
        assert_eq!(held(&app), Some((0, vec![5])));
    }

    /// ENH-06: the read tool, the one a document opens with, selects several pictures and
    /// shapes as the arrow does: Shift+click adds one or takes it out, and Shift+click away
    /// from them changes nothing. The ones selected move together and a right-click on one
    /// keeps them. With words selected, Shift+click still extends the words.
    #[test]
    fn the_read_tool_selects_several_pictures_and_shapes() {
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        theme::install(&ctx);
        let mut app = objects_tab(&ctx);
        assert_eq!(app.tabs[0].ed.tool, Tool::Read);
        click_page(&mut app, &ctx, (340.0, 90.0), false);
        assert_eq!(held(&app), Some((0, vec![0])), "the shape");
        click_page(&mut app, &ctx, (350.0, 190.0), true);
        assert_eq!(held(&app), Some((0, vec![0, 4])), "the picture added");
        assert_eq!(app.tabs[0].ed.heading(), "2 objects");
        click_page(&mut app, &ctx, (500.0, 40.0), true);
        assert_eq!(held(&app), Some((0, vec![0, 4])), "Shift+click on nothing");
        click_page(&mut app, &ctx, (100.0, 87.0), true);
        assert_eq!(held(&app), Some((0, vec![0, 4])), "Shift+click on text");
        // Dragged by the shape, both move.
        let before = boxes(&app, 0);
        drag_page(&mut app, &ctx, (340.0, 90.0), (360.0, 100.0), false);
        let after = boxes(&app, 0);
        for i in [0, 4] {
            let d = (after[i][0] - before[i][0], after[i][1] - before[i][1]);
            assert!(
                (d.0 - 20.0).abs() < 0.6 && (d.1 - 10.0).abs() < 0.6,
                "object {i} moved by {d:?}"
            );
        }
        assert_eq!(held(&app), Some((0, vec![0, 4])), "still selected");
        // A right-click on one of them keeps them, for the menu for them.
        let (min, k) = page_on_screen(&mut app, &ctx);
        let p = min + egui::vec2(370.0, 200.0) * k;
        let button = |pressed| egui::Event::PointerButton {
            pos: p,
            button: egui::PointerButton::Secondary,
            pressed,
            modifiers: Default::default(),
        };
        let right = vec![egui::Event::PointerMoved(p), button(true), button(false)];
        window(&mut app, &ctx, 1400.0, right);
        settle(&mut app, &ctx);
        assert_eq!(held(&app), Some((0, vec![0, 4])), "after a right-click");
        let esc = key_with(Key::Escape, egui::Modifiers::NONE);
        window(&mut app, &ctx, 1400.0, vec![esc]);
        settle(&mut app, &ctx);
        assert_eq!(held(&app), Some((0, vec![0, 4])), "after Esc");
        // Away from the right-click, which a click there would make a double-click.
        click_page(&mut app, &ctx, (400.0, 225.0), true);
        assert_eq!(held(&app), Some((0, vec![0])), "the picture taken out");
        // With words selected, Shift+click on a picture extends them, as before.
        drag_page(&mut app, &ctx, (75.0, 87.0), (120.0, 87.0), false);
        assert!(app.tabs[0].ed.selection.words().is_some(), "words selected");
        click_page(&mut app, &ctx, (340.0, 180.0), true);
        assert!(app.tabs[0].ed.selection.words().is_some(), "words extended");
        assert_eq!(held(&app), None);
    }

    /// ENH-06: a click held down longer than egui takes for a click is still one, with the
    /// read tool and with the arrow: Shift adds the object under it or takes it out, and
    /// without Shift it selects it alone, a background included, or clears the selection on
    /// nothing.
    #[test]
    fn a_slow_click_is_still_a_click() {
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        theme::install(&ctx);
        let mut app = objects_tab(&ctx);
        slow_click_page(&mut app, &ctx, (340.0, 90.0), false);
        assert_eq!(held(&app), Some((0, vec![0])), "read tool: the shape");
        slow_click_page(&mut app, &ctx, (350.0, 190.0), true);
        assert_eq!(
            held(&app),
            Some((0, vec![0, 4])),
            "read tool: the picture added"
        );
        slow_click_page(&mut app, &ctx, (350.0, 190.0), true);
        assert_eq!(
            held(&app),
            Some((0, vec![0])),
            "read tool: the picture taken out"
        );
        app.act(&ctx, Act::Tool(Tool::Select));
        click_page(&mut app, &ctx, (340.0, 90.0), false);
        slow_click_page(&mut app, &ctx, (350.0, 190.0), true);
        assert_eq!(held(&app), Some((0, vec![0, 4])), "the picture added");
        slow_click_page(&mut app, &ctx, (500.0, 40.0), true);
        assert_eq!(held(&app), Some((0, vec![0, 4])), "Shift on nothing");
        slow_click_page(&mut app, &ctx, (350.0, 190.0), true);
        assert_eq!(held(&app), Some((0, vec![0])), "the picture taken out");
        slow_click_page(&mut app, &ctx, (325.0, 275.0), false);
        assert_eq!(held(&app), Some((0, vec![5])), "the group alone");
        slow_click_page(&mut app, &ctx, (500.0, 40.0), false);
        assert_eq!(held(&app), None, "on nothing");
        // A background, covering the page, which a drag doesn't take.
        let st = limarepdf_core::PathStyle {
            fill: Some([0.9, 0.9, 0.9]),
            stroke: None,
            line_width: 1.0,
        };
        let page = [0.0, 0.0, 612.0, 792.0];
        let rect = |p: &mut Pdf| p.add_shape(0, limarepdf_core::Shape::Rect, page, &st);
        assert!(app.tabs[0].edit(&ctx, rect));
        settle(&mut app, &ctx);
        let n = boxes(&app, 0).len();
        slow_click_page(&mut app, &ctx, (500.0, 400.0), false);
        assert_eq!(held(&app), Some((0, vec![n - 1])), "the background");
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
        let typing = |app: &App| {
            let id = app.tabs[0].ed.writing_id();
            id.is_some_and(|id| ctx.memory(|m| m.has_focus(id)))
        };
        // Put a note down at (x, y) with the Note tool and type `text` into it.
        let note = |app: &mut App, x: f32, y: f32, text: &str| {
            app.act(&ctx, Act::Tool(Tool::Note));
            window(app, &ctx, 1400.0, click_at(at(x, y)));
            let tree = window(app, &ctx, 1400.0, vec![]);
            assert!(typing(app), "the note doesn't have the keyboard");
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
        let typing = |app: &App| {
            let id = app.tabs[0].ed.writing_id();
            id.is_some_and(|id| ctx.memory(|m| m.has_focus(id)))
        };
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
        // The inspector's colour button (not the band's text colour, greyed).
        let well = |tree: &egui::accesskit::TreeUpdate| {
            let role = egui::accesskit::Role::ColorWell;
            let mine = |n: &egui::accesskit::Node| n.role() == role && !n.is_disabled();
            let (_, n) = tree.nodes.iter().find(|(_, n)| mine(n)).unwrap();
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
        assert!(typing(&app), "the note didn't get the keyboard back");
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
            let id = app.tabs[0].ed.writing_id().expect("the box");
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

    /// BUG-24: a text box is set in Helvetica, or its embedded face, so what neither has
    /// isn't typed, with a note at the caret: on the page, in the inspector's Text box and
    /// in Fill and sign's Text. It was saved as `?` before.
    #[test]
    fn text_boxes_refuse_at_the_caret_what_they_cant_write() {
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        theme::install(&ctx);
        let mut app = app_with(&ctx, 1);
        let (min, k) = page_on_screen(&mut app, &ctx);
        let at = |x: f32, y: f32| min + egui::vec2(x, y) * k;
        let none = egui::Modifiers::NONE;
        let sequence = "LimarePDF can't add this emoji yet: flags, skin tones, keycaps and joined \
                        emoji aren't supported";
        let noted = |tree: &egui::accesskit::TreeUpdate, note: &str| {
            tree.nodes.iter().any(|(_, n)| n.value() == Some(note))
        };
        let typed = |app: &mut App, text: &str| {
            window(app, &ctx, 1400.0, vec![egui::Event::Text(text.into())]);
            window(app, &ctx, 1400.0, vec![])
        };
        // On the page.
        app.act(&ctx, Act::Tool(Tool::TextBox));
        window(&mut app, &ctx, 1400.0, click_at(at(100.0, 100.0)));
        window(&mut app, &ctx, 1400.0, vec![]);
        // A character no font has (a noncharacter).
        let tree = typed(&mut app, "Walrus\u{2FFFE}");
        assert!(noted(&tree, page_text::REFUSED), "no note at the caret");
        let tree = typed(&mut app, "👍🏽");
        assert!(noted(&tree, sequence), "no note for the sequence");
        window(&mut app, &ctx, 1400.0, vec![key_with(Key::Escape, none)]);
        for _ in 0..2 {
            window(&mut app, &ctx, 1400.0, vec![]);
        }
        assert_eq!(comments(&app), ["Walrus"]);

        // In the inspector, with the text box selected.
        app.act(&ctx, Act::Tool(Tool::Select));
        let t = &mut app.tabs[0];
        let id = t.pdf.annotations()[0].id;
        t.ed.annots(&t.pdf, 0);
        t.ed.selection = Selection::Annot { page: 0, id };
        click_named(&mut app, &ctx, "Text");
        let tree = typed(&mut app, "\u{2FFFE}");
        assert!(noted(&tree, page_text::REFUSED), "no note in the inspector");
        typed(&mut app, " ate fish");
        app.tabs[0].ed.keep(crate::editor::Typed::Comment);
        app.tabs[0].apply_actions(&ctx);
        assert_eq!(comments(&app), ["Walrus ate fish"]);

        // Fill and sign's text.
        app.act(&ctx, Act::Tool(Tool::Sign));
        click_named(&mut app, &ctx, "Text to place");
        let tree = typed(&mut app, "Seal\u{2FFFE}");
        assert!(noted(&tree, page_text::REFUSED), "no note in Fill and sign");
        assert_eq!(app.tabs[0].ed.sign.text, "Seal");
        // Alike when pasted with the middle button (X11 and Wayland's PRIMARY selection,
        // since egui 0.37), past the end of the text.
        let p = middle(&tree, "Text to place");
        let paste = egui::Event::MiddleClickPaste {
            pos: p,
            text: "\u{2FFFE} ate".into(),
        };
        window(
            &mut app,
            &ctx,
            1400.0,
            vec![egui::Event::PointerMoved(p), paste],
        );
        window(&mut app, &ctx, 1400.0, vec![]);
        assert_eq!(app.tabs[0].ed.sign.text, "Seal ate");
    }

    /// Tab in the inspector's multi-line Text box moves on to the next control, as in every
    /// other field, and keeps what was typed; it types no tab (PB-20).
    #[test]
    fn tab_leaves_the_inspector_s_text_box() {
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        theme::install(&ctx);
        let mut app = app_with(&ctx, 1);
        let (min, k) = page_on_screen(&mut app, &ctx);
        let none = egui::Modifiers::NONE;
        app.act(&ctx, Act::Tool(Tool::TextBox));
        window(
            &mut app,
            &ctx,
            1400.0,
            click_at(min + egui::vec2(100.0, 100.0) * k),
        );
        window(&mut app, &ctx, 1400.0, vec![]);
        window(
            &mut app,
            &ctx,
            1400.0,
            vec![egui::Event::Text("Walrus".into())],
        );
        window(&mut app, &ctx, 1400.0, vec![key_with(Key::Escape, none)]);
        settle(&mut app, &ctx);
        app.act(&ctx, Act::Tool(Tool::Select));
        let t = &mut app.tabs[0];
        let id = t.pdf.annotations()[0].id;
        t.ed.annots(&t.pdf, 0);
        t.ed.selection = Selection::Annot { page: 0, id };
        click_named(&mut app, &ctx, "Text");
        // A click moves egui's clock on: the typist's starts after it.
        let ty = Typist::new(&ctx);
        ty.frame(&mut app, vec![egui::Event::Text(" ate fish".into())]);
        ty.key(&mut app, Key::Tab, none);
        let tree = ty.pause(&mut app);
        assert_eq!(focused_text(&tree), None, "still typing in a box");
        assert_eq!(comments(&app), ["Walrus ate fish"]);
    }

    /// BUG-39: a text box another app wrote in Chinese can't be drawn again in Helvetica, so
    /// a new colour or size is refused with the reason, and the document stays as it was.
    #[test]
    fn another_apps_text_box_refuses_a_new_look_with_the_reason() {
        use lopdf::{dictionary, Object, StringFormat};
        let ctx = egui::Context::default();
        let mut pdf = Pdf::new();
        pdf.insert_blank(0, 612.0, 792.0).unwrap();
        let page = pdf.page_ids()[0];
        // Hebrew: no font writes it yet (right to left).
        let utf16: Vec<u8> = [0xFE, 0xFF]
            .into_iter()
            .chain("שלום".encode_utf16().flat_map(u16::to_be_bytes))
            .collect();
        let rect: Vec<Object> = [100, 600, 300, 640].iter().map(|&v| v.into()).collect();
        let id = pdf.doc.add_object(dictionary! {
            "Type" => "Annot", "Subtype" => "FreeText", "Rect" => rect, "P" => page,
            "Contents" => Object::String(utf16, StringFormat::Hexadecimal),
            "DA" => Object::string_literal("/Helv 12 Tf 0 g"),
        });
        pdf.doc
            .get_dictionary_mut(page)
            .unwrap()
            .set("Annots", vec![Object::from(id)]);
        let mut app = App::with(Settings::from_file(None), Recent::from_file(None));
        assert!(app.add_tab(&ctx, pdf, None, None, false));
        let a = app.tabs[0].pdf.annotations()[0].clone();
        let state = app.tabs[0].state();
        let why = "LimarePDF can't draw this text box again: it has \"שלום\", in a script \
                   LimarePDF can't write yet (right-to-left writing, and scripts whose letters \
                   join, aren't supported)";
        let red = crate::editor::AnnotEdit {
            color: Some([1.0, 0.0, 0.0]),
            opacity: a.opacity,
            contents: a.contents.clone(),
            text: a.text_style,
            shape: None,
        };
        let b = a.bbox;
        let wider = [b[0], b[1], b[2] + 50.0, b[3]];
        use crate::editor::Action as Change;
        for act in [
            Change::SetAnnot(0, a.id, red),
            Change::MoveAnnot(0, a.id, wider),
        ] {
            app.tabs[0].ed.actions.push(act);
            app.tabs[0].apply_actions(&ctx);
            assert_eq!(app.tabs[0].error.take().as_deref(), Some(why));
            assert!(app.tabs[0].state() == state && !app.tabs[0].dirty);
        }
    }

    /// The systems whose way of writing keys the tooltips are checked in: Linux and
    /// Windows write "Ctrl+B", macOS "⌘B" (the macOS builds failed when tests looked for
    /// "Ctrl+" alone).
    const KEY_STYLES: [egui::os::OperatingSystem; 2] = [
        egui::os::OperatingSystem::Nix,
        egui::os::OperatingSystem::Mac,
    ];

    /// Ctrl+B or Ctrl+I that can't apply, the family having no such face, says so at the
    /// caret instead of doing nothing; the greyed B and I say so on hover, after their names
    /// and keys, as the system writes them.
    #[test]
    fn bold_a_family_lacks_says_so() {
        for os in KEY_STYLES {
            bold_a_family_lacks_says_so_on(os);
        }
    }

    fn bold_a_family_lacks_says_so_on(os: egui::os::OperatingSystem) {
        let ctx = egui::Context::default();
        ctx.set_os(os);
        ctx.enable_accesskit();
        theme::install(&ctx);
        let serif = crate::font_list::test_family("Source Serif 4");
        let mut app = app_with(&ctx, 1);
        let ty = Typist::new(&ctx);
        let (min, k) = ty.page(&mut app);
        app.act(&ctx, Act::Tool(Tool::Text));
        app.tabs[0].ed.tfont.family = serif;
        ty.click(&mut app, min + egui::vec2(100.0, 200.0) * k);
        ty.text(&mut app, "Walrus");
        let noted = |tree: &egui::accesskit::TreeUpdate, note: &str| {
            tree.nodes
                .iter()
                .any(|(_, n)| n.value().is_some_and(|v| v.contains(note)))
        };
        let tree = ty.key(&mut app, Key::B, CTRL);
        assert!(noted(&tree, "Source Serif 4 has no bold."), "Ctrl+B");
        let tree = ty.key(&mut app, Key::I, CTRL);
        assert!(noted(&tree, "Source Serif 4 has no italic."), "Ctrl+I");
        ty.pause(&mut app);
        for (act, name, face) in [
            (Act::Bold, "Bold", "bold"),
            (Act::Italic, "Italic", "italic"),
        ] {
            let tree = ty.pause(&mut app);
            let p = middle(&tree, name);
            ty.frame(&mut app, vec![egui::Event::PointerMoved(p)]);
            let tree = ty.pause(&mut app);
            let key = ctx.format_shortcut(&actions::get(act).keys[0]);
            let tip = format!("{name} ({key})\nSource Serif 4 has no {face}.");
            assert!(noted(&tree, &tip), "{name} on hover, {os:?}");
        }
        assert!(
            app.tabs[0].ed.adding.is_some(),
            "the hover ended the typing"
        );
    }

    /// The commands say what is in use: the Edit band's Select menu ticks the tool, and Edit
    /// text is pressed while the Text tool is on. A form field with the keyboard heads the
    /// inspector, and the words selected before it are let go, so the band and Ctrl+B act
    /// on one thing.
    #[test]
    fn the_commands_and_inspector_say_what_is_in_use() {
        use egui::accesskit::{Role, Toggled};
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        theme::install(&ctx);
        let mut app = App::with(Settings::from_file(None), Recent::from_file(None));
        typing_doc(&mut app, &ctx);
        let ty = Typist::new(&ctx);
        let toggled = |tree: &egui::accesskit::TreeUpdate, name: &str, role: Role| {
            let (_, n) = (tree.nodes.iter())
                .find(|(_, n)| n.label() == Some(name) && n.role() == role)
                .unwrap_or_else(|| panic!("{name}"));
            n.toggled() == Some(Toggled::True)
        };
        for tool in [Tool::Read, Tool::Select] {
            app.act(&ctx, Act::Tool(tool));
            let tree = ty.pause(&mut app);
            ty.click(&mut app, middle(&tree, "Select"));
            let tree = ty.pause(&mut app);
            let read = toggled(&tree, "Read and select text", Role::RadioButton);
            let select = toggled(&tree, "Select, move and resize objects", Role::RadioButton);
            assert_eq!((read, select), (tool == Tool::Read, tool == Tool::Select));
            ty.key(&mut app, Key::Escape, egui::Modifiers::NONE);
        }
        app.act(&ctx, Act::Tool(Tool::Text));
        let tree = ty.pause(&mut app);
        assert!(
            toggled(&tree, "Edit text", Role::Button),
            "Edit text isn't pressed"
        );
        // Words selected, then a form field clicked.
        app.act(&ctx, Act::Tool(Tool::Read));
        app.tabs[0].ed.selection = Selection::word((0, 0));
        ty.pause(&mut app);
        ty.click_field(&mut app, "name", 0);
        assert!(
            app.tabs[0].ed.selection.words().is_none(),
            "the words are still selected"
        );
        assert_eq!(inspector::title(&app.tabs[0]), "Form field");
    }

    /// Ctrl+V with nothing being typed and no words to type over says where to click,
    /// instead of doing nothing; the document is unchanged.
    #[test]
    fn paste_with_nowhere_to_paste_says_where_to_click() {
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        theme::install(&ctx);
        let mut app = App::with(Settings::from_file(None), Recent::from_file(None));
        lines_tab(&mut app, &ctx, 400.0, &paragraph(72.0, 100.0, 3, 1, false));
        settled_at(&mut app, &ctx, 1400.0);
        let revision = app.tabs[0].revision;
        let paste = vec![egui::Event::Paste("seal".into())];
        window(&mut app, &ctx, 1400.0, paste);
        let said = toast(&app).expect("nothing said");
        assert!(said.contains("Add text"), "{said}");
        assert_eq!(app.tabs[0].revision, revision);
    }

    /// Greyed commands in the commands row and the band say why on hover, after their
    /// names and keys, as the system writes them.
    #[test]
    fn greyed_buttons_say_why_on_hover() {
        for os in KEY_STYLES {
            greyed_buttons_say_why_on_hover_on(os);
        }
    }

    fn greyed_buttons_say_why_on_hover_on(os: egui::os::OperatingSystem) {
        let ctx = egui::Context::default();
        ctx.set_os(os);
        ctx.enable_accesskit();
        theme::install(&ctx);
        let mut app = App::with(Settings::from_file(None), Recent::from_file(None));
        lines_tab(&mut app, &ctx, 400.0, &paragraph(72.0, 100.0, 3, 1, false));
        let ty = Typist::new(&ctx);
        let says = |tree: &egui::accesskit::TreeUpdate, s: &str| {
            tree.nodes
                .iter()
                .any(|(_, n)| n.value().is_some_and(|v| v.contains(s)))
        };
        let greyed = [
            (Act::Undo, "Undo", "Nothing to undo."),
            (Act::Cut, "Cut", "Select some text first."),
        ];
        for (act, name, why) in greyed {
            let tree = ty.pause(&mut app);
            let (r, _) = node(&tree, name).expect(name);
            ty.frame(&mut app, vec![egui::Event::PointerMoved(r.center())]);
            let tree = ty.pause(&mut app);
            let key = ctx.format_shortcut(&actions::get(act).keys[0]);
            let tip = format!("{name} ({key})");
            assert!(says(&tree, &tip), "{name}'s tooltip, {os:?}");
            assert!(says(&tree, why), "{name}: no reason, {os:?}");
        }
    }

    /// BUG-24: new text in an installed font with outline emoji takes single emoji; one the
    /// font draws only in colour, and emoji sequences, aren't typed, each with its note.
    #[test]
    fn single_emoji_are_typed_in_an_installed_font_that_has_them() {
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        theme::install(&ctx);
        let emoji = crate::font_list::test_family("Limare Emoji Test");
        let mut app = app_with(&ctx, 1);
        let (min, k) = page_on_screen(&mut app, &ctx);
        let at = |x: f32, y: f32| min + egui::vec2(x, y) * k;
        app.act(&ctx, Act::Tool(Tool::Text));
        app.tabs[0].ed.tfont.family = emoji;
        window(&mut app, &ctx, 1400.0, click_at(at(100.0, 200.0)));
        settled_at(&mut app, &ctx, 1400.0);
        let noted = |tree: &egui::accesskit::TreeUpdate, note: &str| {
            tree.nodes.iter().any(|(_, n)| n.value() == Some(note))
        };
        let typed = |app: &mut App, text: &str| {
            window(app, &ctx, 1400.0, vec![egui::Event::Text(text.into())]);
            window(app, &ctx, 1400.0, vec![])
        };
        typed(&mut app, "Walrus 😀");
        let tree = typed(&mut app, "👍🏽");
        let sequence = "LimarePDF can't add this emoji yet: flags, skin tones, keycaps and joined \
                        emoji aren't supported";
        assert!(noted(&tree, sequence), "no note for the sequence");
        window(&mut app, &ctx, 1400.0, click_at(at(300.0, 400.0)));
        settled_at(&mut app, &ctx, 1400.0);
        let objs = app.tabs[0].pdf.page_objects(0).unwrap();
        let lines: Vec<String> = objs.into_iter().filter_map(|o| o.text).collect();
        assert_eq!(lines, ["Walrus 😀"]);
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

    /// The version of a small file, for questions that show one.
    fn some_version() -> disk::Version {
        let p = std::env::temp_dir().join(format!("limarepdf-app-{}-v", std::process::id()));
        std::fs::write(&p, "v").unwrap();
        let v = disk::version(&p).unwrap();
        let _ = std::fs::remove_file(&p);
        v
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
                "Save changes?".into(),
                vec!["Save", "Discard", "Cancel"],
                Dialog::ConfirmClose(app.asked(), false),
            ),
            2 => (
                "Quit with unsaved changes?".into(),
                vec!["Review…", "Discard all", "Cancel"],
                Dialog::ConfirmQuit,
            ),
            3 => (
                "Replace files?".into(),
                two("Replace"),
                Dialog::ConfirmReplace(
                    app.asked(),
                    Replacing::Images(ImageExport {
                        pages: vec![0, 1],
                        dpi: 150.0,
                        format: ImageFormat::Png,
                        dir: PathBuf::from(format!("/home/sam/{long}")),
                        stem: long.into(),
                    }),
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
                Dialog::Password(
                    dlg::Password {
                        path: PathBuf::from(format!("/home/sam/{long}.pdf")),
                        input: String::new(),
                        error: Some(format!("That isn't the password. {long}")),
                    },
                    Unlock::Tab(None, None),
                ),
            ),
            22 => (
                "Replace files?".into(),
                two("Replace"),
                Dialog::ConfirmReplace(
                    app.asked(),
                    Replacing::Split(SplitFiles {
                        by: SplitBy::Bookmarks(1),
                        dir: PathBuf::from(format!("/home/sam/{long}")),
                        stem: long.into(),
                    }),
                    12,
                ),
            ),
            23 => (
                "Replace the changed file?".into(),
                vec!["Save as…", "Replace", "Cancel"],
                Dialog::ConfirmStale(app.asked(), Box::new(some_version())),
            ),
            24 => (
                "Reload without your changes?".into(),
                two("Discard and reload"),
                Dialog::ConfirmReload(app.asked()),
            ),
            25 => (
                "Password required".into(),
                vec!["Add pages", "Skip this file", "Cancel"],
                Dialog::Password(
                    dlg::Password {
                        path: PathBuf::from(format!("/home/sam/{long}.pdf")),
                        input: String::new(),
                        error: Some(format!("That isn't the password. {long}")),
                    },
                    Unlock::Append(Box::new(Appending {
                        tab: app.tabs[0].id,
                        to: PagesGo::End,
                        files: vec![PathBuf::from(format!("/home/sam/{long}.pdf"))],
                        read: Vec::new(),
                    })),
                ),
            ),
            // Print looking for the printers (Print shown, not yet enabled), unable to list
            // them, and finding none (BUG-11).
            26 => (
                "Print".into(),
                vec!["Cancel"],
                Dialog::Print(
                    app.asked(),
                    Box::new(dlg::Print::looking(1, "test.pdf".into())),
                ),
            ),
            27 => {
                let mut print = dlg::Print::looking(1, "test.pdf".into());
                print.found(Err(format!("lpstat: {long}")));
                let print = Dialog::Print(app.asked(), Box::new(print));
                ("Print".into(), two("Try again"), print)
            }
            28 => (
                "Print".into(),
                two("Try again"),
                Dialog::Print(
                    app.asked(),
                    Box::new(dlg::Print::new(Vec::new(), "test.pdf".into())),
                ),
            ),
            // Quitting, about one document (ENH-01).
            29 => (
                "Save changes?".into(),
                vec!["Save", "Discard", "Cancel"],
                Dialog::ConfirmClose(app.asked(), true),
            ),
            // Appending files with form fields (BUG-19).
            30 => (
                "Add pages with form fields?".into(),
                two("Add pages"),
                Dialog::ConfirmAppendForms(Box::new(AppendForms {
                    asked: app.asked(),
                    to: PagesGo::End,
                    docs: Vec::new(),
                    names: vec![format!("{long}.pdf"), "form.pdf".into()],
                })),
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
        let (at, _) = node(&tree, "File").expect("the File button");
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

    /// ENH-34: text selected on the page is offered for X11 and Wayland's PRIMARY selection
    /// (pasted elsewhere with the middle button) once it settles: not during the drag, and
    /// once, not every frame after.
    #[test]
    fn selected_page_text_is_offered_for_middle_click_paste() {
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        theme::install(&ctx);
        let mut app = app_with_parts(&ctx);
        let tree = settled_at(&mut app, &ctx, 1400.0);
        let (page, _) = node(&tree, "Page 1").expect("the page");
        let words = app.tabs[0].pdf.page_words(0).unwrap();
        let at = |w: &limarepdf_core::PageWord| {
            let b = w.bbox;
            let mid = egui::vec2((b[0] + b[2]) / 2.0, (b[1] + b[3]) / 2.0);
            page.min + mid * (page.width() / 612.0)
        };
        let (first, last) = (at(&words[0]), at(words.last().unwrap()));
        let offered = |app: &mut App, events: Vec<egui::Event>| -> Vec<String> {
            let raw = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1400.0, TEST_HEIGHT),
                )),
                events,
                ..Default::default()
            };
            let mut frame = eframe::Frame::_new_kittest();
            let mut out = ctx.run_ui(raw, |ui| eframe::App::ui(app, ui, &mut frame));
            out.textures_delta.clear();
            let commands = out.platform_output.commands.into_iter();
            commands
                .filter_map(|c| match c {
                    egui::OutputCommand::TextSelectionSettled(s) => Some(s),
                    _ => None,
                })
                .collect()
        };
        let mut sent = offered(&mut app, vec![egui::Event::PointerMoved(first)]);
        sent.extend(offered(&mut app, vec![press(first, true)]));
        for step in 1..=4 {
            let p = first + (last - first) * step as f32 / 4.0;
            sent.extend(offered(&mut app, vec![egui::Event::PointerMoved(p)]));
        }
        assert!(
            app.tabs[0].ed.selection.words().is_some(),
            "nothing selected"
        );
        assert!(sent.is_empty(), "offered during the drag: {sent:?}");
        sent.extend(offered(&mut app, vec![press(last, false)]));
        for _ in 0..3 {
            sent.extend(offered(&mut app, vec![]));
        }
        let text = app.tabs[0].ed.selected_text().expect("the selected text");
        assert!(text.contains("Walrus") && text.contains("ate"), "{text:?}");
        assert_eq!(sent, [text]);
    }

    /// The down arrow goes through an open menu's items in order, past those that are off
    /// (Show cover page alone, outside facing pages), and round to the first, staying in
    /// the menu (egui 0.37 reworked arrow keys in menus). The keyboard moves a frame after
    /// the key.
    #[test]
    fn the_down_arrow_goes_through_a_menu() {
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        theme::install(&ctx);
        let mut app = app_with(&ctx, 1);
        click_named(&mut app, &ctx, "View");
        click_named(&mut app, &ctx, "Theme");
        settled_at(&mut app, &ctx, 1400.0);
        assert!(egui::Popup::is_any_open(&ctx), "the menu didn't open");
        let mut reached = Vec::new();
        for _ in 0..4 {
            let down = key_with(Key::ArrowDown, egui::Modifiers::NONE);
            window(&mut app, &ctx, 1400.0, vec![down]);
            let tree = window(&mut app, &ctx, 1400.0, vec![]);
            let focus = tree.nodes.iter().find(|(id, _)| *id == tree.focus);
            reached.push(focus.and_then(|(_, n)| n.label()).unwrap_or("").to_string());
        }
        assert_eq!(reached, ["System", "Light", "Dark", "System"]);
        assert!(egui::Popup::is_any_open(&ctx), "the menu closed");
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
                assert_eq!(i, 31, "a question was left out");
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
                let one_field = matches!(app.dialog, Dialog::Password(..) | Dialog::CommentName(_));
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
                    reached.push(focus.and_then(|(_, n)| n.label()).map(str::to_string));
                }
                for b in &buttons {
                    let b = Some(b.to_string());
                    assert!(reached.contains(&b), "{at}: Tab missed {b:?}");
                }
                // Once on the controls, Tab stays on them, round and round (PB-15).
                let on = reached.iter().skip_while(|r| r.is_none());
                assert!(on.clone().all(Option::is_some), "{at}: {reached:?}");
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
                    egui::vec2(1000.0, TEST_HEIGHT),
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
        // A printed text field filled in another app with text LimarePDF can't draw
        // (Hebrew, written right to left), and no drawing of its own.
        assert!(app.tabs[0].edit(&ctx, |p| {
            let mut v = vec![0xFE, 0xFF];
            v.extend("שלום".encode_utf16().flat_map(u16::to_be_bytes));
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
                    egui::vec2(1000.0, TEST_HEIGHT),
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
        click(&mut app, &ctx, "Discard");
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
        assert!(matches!(app.dialog, Dialog::ConfirmClose(_, true)));
        // Discard drops it.
        app.dialog = Dialog::None;
        app.act(&ctx, Act::CloseTab);
        click(&mut app, &ctx, "Discard");
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
        saves_done(&mut app, &ctx);
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
                egui::vec2(1400.0, TEST_HEIGHT),
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

    /// The window drawn until Liberation Sans, which Helvetica is drawn in, is in, for a
    /// minute at most: typing fonts are read on another thread, Inter standing in until
    /// then, which on a busy computer is longer than settling takes (macOS CI after 3a).
    fn liberation_in(app: &mut App, ctx: &egui::Context) {
        let want = egui::FontFamily::Name("Liberation Sans".into());
        let helvetica = crate::typing_fonts::Wanted::Standard(FontChoice::default());
        for _ in 0..6000 {
            if crate::typing_fonts::family(ctx, helvetica) == want {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
            window(app, ctx, 1400.0, vec![]);
        }
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
        saves_done(&mut app, &ctx);
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

    /// PB-07: in a signed document, text after an undo goes on growing the font this session
    /// embedded; the undo's copy of the document doesn't make it look like a signed one.
    #[test]
    fn undo_in_a_signed_document_keeps_growing_the_sessions_font() {
        let ctx = egui::Context::default();
        let mut app = app_with(&ctx, 1);
        let t = &mut app.tabs[0];
        // Fonts are grown only when they were made since opening once the document holds a
        // signature dictionary.
        use lopdf::dictionary;
        t.pdf.doc.add_object(dictionary! {
            "Type" => "Sig", "ByteRange" => vec![0.into(), 0.into(), 0.into(), 0.into()],
        });
        let style = limarepdf_core::TextStyle::default();
        let add = |t: &mut Tab, text: &'static str| {
            assert!(t.edit(&ctx, |p| {
                p.add_text_box(0, [20.0, 120.0, 280.0, 160.0], text, &style, "Sam")
                    .map(|_| ())
            }));
        };
        add(t, "Съешь");
        add(t, "ещё");
        t.undo(&ctx);
        add(t, "этих");
        let type0 = |o: &lopdf::Object| {
            o.as_dict()
                .is_ok_and(|d| d.get(b"Subtype").and_then(|s| s.as_name()).ok() == Some(b"Type0"))
        };
        assert_eq!(t.pdf.doc.objects.values().filter(|o| type0(o)).count(), 1);
    }

    /// A watermark is shown on the pages while its section is open, and nothing is written
    /// until Add to pages; then changes update it in place. Headers and footers likewise.
    #[test]
    fn stamps_are_shown_on_the_pages_until_added() {
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        theme::install(&ctx);
        let mut app = app_with(&ctx, 1);
        app.tabs[0].forms.reveal = Some(inspector::Section::Watermark);
        let tree = settle(&mut app, &ctx);
        liberation_in(&mut app, &ctx);
        // On the page in view, written nowhere: drawn over it while a setting is being
        // changed, its text field having the keyboard (once settled, the page is drawn
        // with it instead, ENH-23).
        let field = app.tabs[0].ed.scope().with(("stamp-text", "Text"));
        ctx.memory_mut(|m| m.request_focus(field));
        window(&mut app, &ctx, 1400.0, vec![]);
        app.tabs[0].forms.wm.opacity = 0.3;
        let texts = drawn_texts(&mut app, &ctx, vec![]);
        assert_eq!(texts, ["DRAFT"]);
        let lines = |app: &App| -> Vec<String> {
            let objs = app.tabs[0].pdf.page_objects(0).unwrap();
            objs.into_iter().filter_map(|o| o.text).collect()
        };
        assert!(lines(&app).is_empty());
        // A character no font has isn't typed, and the note says so.
        ctx.memory_mut(|m| m.request_focus(field));
        window(&mut app, &ctx, 1400.0, vec![]);
        let none = egui::Event::Text("\u{2FFFE}".into());
        window(&mut app, &ctx, 1400.0, vec![none]);
        let typed = window(&mut app, &ctx, 1400.0, vec![]);
        assert!(
            typed
                .nodes
                .iter()
                .any(|(_, n)| n.value() == Some(page_text::REFUSED)),
            "no note about the refused character"
        );
        assert_eq!(app.tabs[0].forms.wm.text, "DRAFT");
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
        let scope = app.tabs[0].ed.scope();
        let footer = scope.with(("stamp-text", "Footer, centre"));
        ctx.memory_mut(|m| m.request_focus(footer));
        window(&mut app, &ctx, 1400.0, vec![]);
        app.tabs[0].forms.hf.size = 11.0;
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

    /// Cropping ends as Word's picture crop does (the user's find, batch 42): the band's
    /// Crop shows pressed while the crop is set and a click on it again ends it, showing the
    /// pages cropped; so do a click on a page away from the crop box's edges, a click on the
    /// grey beside the pages, and Enter. Meanwhile the status bar says what is cropped and
    /// how to end it.
    #[test]
    fn cropping_ends_as_a_word_processor_crop_does() {
        use egui::accesskit::{Role, Toggled};
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        theme::install(&ctx);
        let mut app = app_with(&ctx, 1);
        app.band = Band::Pages;
        // The band's Crop button (the inspector's heading says Crop too): where, and
        // whether pressed.
        let crop = |tree: &egui::accesskit::TreeUpdate| {
            let n = tree.nodes.iter().find(|(_, n)| {
                n.role() == Role::Button && n.label() == Some("Crop") && n.bounds().is_some()
            });
            let n = &n.expect("the band's Crop").1;
            let b = n.bounds().unwrap();
            let at = egui::pos2((b.x0 + b.x1) as f32 / 2.0, (b.y0 + b.y1) as f32 / 2.0);
            (at, n.toggled() == Some(Toggled::True))
        };
        let says = |tree: &egui::accesskit::TreeUpdate| {
            let mut values = tree.nodes.iter().filter_map(|(_, n)| n.value());
            values.any(|v| v.starts_with("Cropping 2 pages"))
        };
        let tree = settle(&mut app, &ctx);
        let (at, on) = crop(&tree);
        assert!(!on, "pressed before cropping");
        assert!(!says(&tree), "said before cropping");
        // The band's button: on, then off again.
        window(&mut app, &ctx, 1400.0, click_at(at));
        let tree = settle(&mut app, &ctx);
        assert!(app.tabs[0].crop_view(), "not cropping");
        assert!(crop(&tree).1, "not pressed while cropping");
        assert!(says(&tree), "the status bar");
        window(&mut app, &ctx, 1400.0, click_at(at));
        let tree = settle(&mut app, &ctx);
        assert!(!app.tabs[0].crop_view(), "Crop again didn't end it");
        assert!(!crop(&tree).1);
        // A click on the page, inside the crop box, away from its edges.
        let page = |app: &mut App| {
            app.act(&ctx, Act::Crop);
            let tree = settle(app, &ctx);
            assert!(app.tabs[0].crop_view());
            let (r, _) = node(&tree, "Crop box on page 1").expect("the crop box");
            r.shrink(8.0)
        };
        let r = page(&mut app);
        window(&mut app, &ctx, 1400.0, click_at(r.center()));
        settle(&mut app, &ctx);
        assert!(!app.tabs[0].crop_view(), "a click on the page");
        // A click on the grey beside the pages, zoomed out to leave room.
        for _ in 0..4 {
            app.act(&ctx, Act::ZoomOut);
        }
        let r = page(&mut app);
        let beside = egui::pos2(r.min.x - 40.0, r.center().y);
        window(&mut app, &ctx, 1400.0, click_at(beside));
        settle(&mut app, &ctx);
        assert!(!app.tabs[0].crop_view(), "a click beside the pages");
        // Enter.
        page(&mut app);
        let enter = key_with(Key::Enter, egui::Modifiers::NONE);
        window(&mut app, &ctx, 1400.0, vec![enter]);
        settle(&mut app, &ctx);
        assert!(!app.tabs[0].crop_view(), "Enter didn't end it");
        // A drag of the crop box's edge still crops, and leaves it open.
        let r = page(&mut app);
        let top = egui::pos2(r.center().x, r.min.y);
        window(&mut app, &ctx, 1400.0, vec![egui::Event::PointerMoved(top)]);
        window(&mut app, &ctx, 1400.0, vec![press(top, true)]);
        for i in 1..=4 {
            let p = top + egui::vec2(0.0, 10.0 * i as f32);
            window(&mut app, &ctx, 1400.0, vec![egui::Event::PointerMoved(p)]);
        }
        let end = top + egui::vec2(0.0, 40.0);
        window(&mut app, &ctx, 1400.0, vec![press(end, false)]);
        settle(&mut app, &ctx);
        assert!(app.tabs[0].crop_view(), "a drag ended it");
        let top = app.tabs[0].pdf.crop_margins(0).unwrap()[0];
        assert!(top > 0.0, "not cropped");
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
            default_unsaved: None,
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

    /// BUG-14: an ID used that couldn't be kept as the default is said with the signing,
    /// which is done all the same.
    #[test]
    fn a_default_not_kept_is_said_with_the_signing() {
        let ctx = egui::Context::default();
        let mut app = app_with(&ctx, 1);
        let (id, revision) = (app.tabs[0].id, app.tabs[0].revision);
        let s = digital::Signed {
            default_unsaved: Some("the ID couldn't be kept as the default (a test)".into()),
            ..signed_copy()
        };
        app.signed(&ctx, s, id, revision);
        let (title, text) = message(&app);
        assert_eq!(title, "Signed");
        let said = "The document is signed, but the ID couldn't be kept as the default (a test).";
        assert!(text.contains(said), "{text}");
        assert!(!app.tabs[0].dirty);
    }

    /// BUG-33: signed with a self-issued ID, the message says where to export its
    /// certificate, as the menus have it.
    #[test]
    fn a_self_issued_signing_says_where_to_export_the_certificate() {
        let ctx = egui::Context::default();
        let mut app = app_with(&ctx, 1);
        let (id, revision) = (app.tabs[0].id, app.tabs[0].revision);
        let s = digital::Signed {
            self_issued: true,
            ..signed_copy()
        };
        app.signed(&ctx, s, id, revision);
        let ids = actions::get(Act::DigitalIds);
        let way = format!("{} > {} > {}", ids.group.name(), ids.submenu, ids.label);
        let text = message(&app).1;
        assert!(text.contains(&way), "{text}");
    }

    /// BUG-14: with a settings folder that can't be written, what is changed works for the
    /// session, and that it isn't kept is said once; a signature saved for next time that
    /// can't be is said each time, and stays usable.
    #[test]
    fn settings_that_can_t_be_saved_are_said() {
        use crate::signing::{Entry, Saved};
        use limarepdf_core::{SignRole, SignStyle};
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        theme::install(&ctx);
        let dir = test_dir("unwritable-settings");
        // A file where the settings folder would be.
        std::fs::write(dir.join("blocker"), "a file, not a folder").unwrap();
        let folder = dir.join("blocker");
        let settings = Settings::from_file(Some(folder.join("settings.txt")));
        let mut app = App::with(settings, Recent::from_file(Some(folder.join("recent.txt"))));
        let mut pdf = Pdf::new();
        pdf.insert_blank(0, 612.0, 792.0).unwrap();
        assert!(app.add_tab(&ctx, pdf, None, None, false));
        app.act(&ctx, Act::Theme(egui::ThemePreference::Dark));
        settle(&mut app, &ctx);
        let (title, text) = message(&app);
        assert_eq!(title, "Settings not saved");
        let start = "LimarePDF couldn't save its settings (making ";
        assert!(text.starts_with(start), "{text}");
        assert!(text.ends_with("This isn't said again until LimarePDF starts again."));
        assert_eq!(app.settings.theme, egui::ThemePreference::Dark);
        app.dialog = Dialog::None;
        // Once a session.
        app.act(&ctx, Act::Theme(egui::ThemePreference::Light));
        settle(&mut app, &ctx);
        assert!(matches!(app.dialog, Dialog::None), "said again");
        // A signature saved for next time, as the Create window adds it: said each time,
        // and listed.
        for (i, text) in ["Sam Lee", "S. Lee"].into_iter().enumerate() {
            let saved = Saved::Typed {
                text: text.into(),
                style: SignStyle::ALL[0],
            };
            let e = Entry { saved, keep: true };
            let mut kit = app.settings.sign.clone();
            assert_eq!(kit.add(SignRole::Signature, e), None);
            let kept = kit.kept_since(&app.settings.sign);
            assert!(kept);
            app.settings.sign = kit;
            app.settings.save_sign(kept);
            settle(&mut app, &ctx);
            let (title, said) = message(&app);
            assert_eq!(title, "Not saved for next time", "{i}");
            let start = "What you saved for next time can be used until LimarePDF closes, but \
                         LimarePDF couldn't save it (making ";
            assert!(said.starts_with(start), "{said}");
            assert_eq!(app.settings.sign.signatures.len(), i + 1);
            app.dialog = Dialog::None;
        }
        let _ = std::fs::remove_dir_all(&dir);
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

    /// A one-page document with empty text fields: for each of `fields`, its name and how
    /// many widgets show it, each widget a row of its own from the top of the page.
    fn form_doc(fields: &[(&str, usize)]) -> Pdf {
        use lopdf::{dictionary, Object};
        let mut p = Pdf::new();
        p.insert_blank(0, 612.0, 792.0).unwrap();
        let page = p.page_ids()[0];
        let (mut annots, mut tops) = (Vec::new(), Vec::new());
        let mut row = 0;
        for &(name, n) in fields {
            let parent = p.doc.new_object_id();
            let mut kids = Vec::new();
            for _ in 0..n {
                let top = 740 - 60 * row;
                row += 1;
                let rect: Vec<Object> = [72, top - 24, 400, top].map(Object::from).to_vec();
                kids.push(Object::Reference(p.doc.add_object(dictionary! {
                    "Type" => "Annot", "Subtype" => "Widget", "F" => 4, "Rect" => rect,
                    "P" => page, "Parent" => parent,
                })));
            }
            annots.extend(kids.iter().cloned());
            let field = dictionary! {
                "FT" => "Tx", "T" => Object::string_literal(name), "Kids" => kids,
            };
            p.doc.objects.insert(parent, Object::Dictionary(field));
            tops.push(Object::Reference(parent));
        }
        p.doc
            .get_dictionary_mut(page)
            .unwrap()
            .set("Annots", annots);
        let form = dictionary! { "Fields" => tops };
        p.doc.catalog_mut().unwrap().set("AcroForm", form);
        p
    }

    /// The text of field `name` in the tab's document as it would be saved now, read back
    /// from its bytes.
    fn saved_field(t: &Tab, name: &str) -> String {
        let bytes = t.pdf.clone().snapshot().unwrap();
        let pdf = Pdf::from_bytes(&bytes, None).unwrap();
        let f = pdf.form_fields().into_iter().find(|f| f.name == name);
        match f.map(|f| f.value) {
            Some(limarepdf_core::FieldValue::Text(t)) => t,
            v => panic!("{name}: {v:?}"),
        }
    }

    /// The text in the control with the keyboard.
    fn focused_text(tree: &egui::accesskit::TreeUpdate) -> Option<String> {
        let (_, n) = tree.nodes.iter().find(|(id, _)| *id == tree.focus)?;
        n.value().map(str::to_string)
    }

    /// Drives the window as someone typing: each frame moves the clock on, so typing
    /// settles into a step of a text box's own undo after a pause, as it does for people.
    struct Typist<'a> {
        ctx: &'a egui::Context,
        time: std::cell::Cell<f64>,
    }

    impl<'a> Typist<'a> {
        fn new(ctx: &'a egui::Context) -> Self {
            Self {
                ctx,
                time: std::cell::Cell::new(10.0),
            }
        }

        fn frame(&self, app: &mut App, events: Vec<egui::Event>) -> egui::accesskit::TreeUpdate {
            timed_frame(app, self.ctx, &self.time, events)
        }

        /// A second and a half with no input.
        fn pause(&self, app: &mut App) -> egui::accesskit::TreeUpdate {
            for _ in 0..30 {
                self.frame(app, vec![]);
            }
            self.frame(app, vec![])
        }

        /// Press `key` with `modifiers`; the window as it then is.
        fn key(
            &self,
            app: &mut App,
            key: Key,
            modifiers: egui::Modifiers,
        ) -> egui::accesskit::TreeUpdate {
            self.frame(app, vec![key_with(key, modifiers)]);
            self.frame(app, vec![])
        }

        /// Type `text`, then pause.
        fn text(&self, app: &mut App, text: &str) {
            self.frame(app, vec![egui::Event::Text(text.into())]);
            self.pause(app);
        }

        fn click(&self, app: &mut App, p: egui::Pos2) {
            self.frame(app, click_at(p));
            self.pause(app);
        }

        /// Where the active tab's first page is on screen, and the size of its points.
        fn page(&self, app: &mut App) -> (egui::Pos2, f32) {
            let tree = self.pause(app);
            let (page, _) = node(&tree, "Page 1").expect("the page");
            (page.min, page.width() / 612.0)
        }

        /// Click the middle of widget `widget` of field `name` on the active tab's first
        /// page.
        fn click_field(&self, app: &mut App, name: &str, widget: usize) {
            let (min, k) = self.page(app);
            let t = &app.tabs[app.active];
            let f = t.pdf.form_fields().into_iter().find(|f| f.name == name);
            let b = f.expect("the field").widgets[widget].bbox;
            let at = egui::vec2((b[0] + b[2]) / 2.0, (b[1] + b[3]) / 2.0);
            self.click(app, min + at * k);
        }
    }

    /// A 40 x 20 JPEG, red on the left and blue on the right, whose EXIF orientation 6
    /// says to show it turned a quarter clockwise: 20 wide and 40 high, red at the top.
    fn turned_photo() -> Vec<u8> {
        use limarepdf_core::render::{encode, Bitmap, ImageFormat};
        let (red, blue) = ([255, 0, 0, 255], [0, 0, 255, 255]);
        let rgba = (0..40 * 20)
            .flat_map(|i| if i % 40 < 20 { red } else { blue })
            .collect();
        let bitmap = Bitmap {
            width: 40,
            height: 20,
            rgba,
        };
        let jpeg = encode(&bitmap, ImageFormat::Jpeg(95), 72.0).unwrap();
        // An APP1 segment right after the start, its Exif block holding only the orientation.
        let exif = b"Exif\0\0II*\0\x08\0\0\0\x01\0\x12\x01\x03\0\x01\0\0\0\x06\0\0\0\0\0\0\0";
        let mut out = jpeg[..2].to_vec();
        out.extend([0xFF, 0xE1]);
        out.extend((exif.len() as u16 + 2).to_be_bytes());
        out.extend(exif);
        out.extend(&jpeg[2..]);
        out
    }

    /// BUG-18: a turned photo placed with a click gets a box of its upright shape, and
    /// shows upright in it.
    #[test]
    fn a_turned_photo_placed_by_a_click_is_upright_in_a_box_of_its_shape() {
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        theme::install(&ctx);
        let mut app = App::with(Settings::from_file(None), Recent::from_file(None));
        let mut pdf = Pdf::new();
        pdf.insert_blank(0, 612.0, 792.0).unwrap();
        assert!(app.add_tab(&ctx, pdf, None, None, false));
        let ty = Typist::new(&ctx);
        // Not through the command, which asks for a file.
        app.tabs[0].arm_image(turned_photo());
        let (min, k) = ty.page(&mut app);
        ty.click(&mut app, min + egui::vec2(100.0, 100.0) * k);
        let t = &mut app.tabs[0];
        let objs = t.pdf.page_objects(0).unwrap();
        let image = limarepdf_core::ObjectKind::Image;
        let picture = objs.iter().find(|o| o.kind == image);
        let [x0, y0, x1, y1] = picture.expect("the picture placed").bbox;
        let (w, h) = (x1 - x0, y1 - y0);
        let shape = (w - 200.0).abs() < 0.5 && (h - 400.0).abs() < 0.5;
        assert!(shape, "{w} x {h}");
        let hp = limarepdf_core::render::load(t.pdf.render_snapshot().unwrap(), None).unwrap();
        let page = limarepdf_core::render::Renderer::new(&hp)
            .render(0, 1.0)
            .unwrap();
        let px = |x: f32, y: f32| {
            let i = (y as usize * page.width + x as usize) * 4;
            [page.rgba[i], page.rgba[i + 1], page.rgba[i + 2]]
        };
        let mid = x0 + w / 2.0;
        let (top, bottom) = (px(mid, y0 + h / 4.0), px(mid, y1 - h / 4.0));
        assert!(top[0] > 200 && top[2] < 60, "red at the top: {top:?}");
        let blue = bottom[2] > 200 && bottom[0] < 60;
        assert!(blue, "blue at the bottom: {bottom:?}");
    }

    const CTRL: egui::Modifiers = egui::Modifiers::COMMAND;
    const CTRL_SHIFT: egui::Modifiers = egui::Modifiers::COMMAND.plus(egui::Modifiers::SHIFT);

    /// Two documents with a field of the same name keep their typing apart: Ctrl+Z in the
    /// second one's field, with nothing typed there and no document history, leaves it as it
    /// was, and each saved value is its own (BUG-03, audit A10).
    #[test]
    fn same_named_fields_in_two_documents_keep_their_typing_apart() {
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        theme::install(&ctx);
        let mut app = App::with(Settings::from_file(None), Recent::from_file(None));
        for _ in 0..2 {
            assert!(app.add_tab(&ctx, form_doc(&[("name", 1)]), None, None, false));
        }
        let ty = Typist::new(&ctx);
        app.active = 0;
        ty.click_field(&mut app, "name", 0);
        ty.text(&mut app, "Alpha");
        ty.key(&mut app, Key::Escape, egui::Modifiers::NONE);
        ty.pause(&mut app);
        assert_eq!(saved_field(&app.tabs[0], "name"), "Alpha");
        // The other document, as a click on its tab brings it to the front.
        app.active = 1;
        ty.click_field(&mut app, "name", 0);
        for (key, modifiers) in [(Key::Z, CTRL), (Key::Z, CTRL_SHIFT), (Key::Y, CTRL)] {
            let tree = ty.key(&mut app, key, modifiers);
            let typed = focused_text(&tree);
            assert_eq!(typed.as_deref(), Some(""), "{key:?} {modifiers:?}");
        }
        ty.key(&mut app, Key::Escape, egui::Modifiers::NONE);
        ty.pause(&mut app);
        assert!(!app.tabs[1].dirty);
        assert_eq!(saved_field(&app.tabs[1], "name"), "");
        assert_eq!(saved_field(&app.tabs[0], "name"), "Alpha");
        // The first document closed and opened again as saved: nothing to undo in its field.
        let bytes = app.tabs[0].pdf.clone().snapshot().unwrap();
        app.close_tab(0);
        let pdf = Pdf::from_bytes(&bytes, None).unwrap();
        assert!(app.add_tab(&ctx, pdf, None, None, false));
        ty.click_field(&mut app, "name", 0);
        let tree = ty.key(&mut app, Key::Z, CTRL);
        assert_eq!(focused_text(&tree).as_deref(), Some("Alpha"));
        ty.key(&mut app, Key::Escape, egui::Modifiers::NONE);
        ty.pause(&mut app);
        assert_eq!(saved_field(&app.tabs[1], "name"), "Alpha");
    }

    /// A field shown twice keeps one value, and the box over its second widget starts with
    /// nothing to undo: Ctrl+Z there doesn't take back what was typed in the first.
    #[test]
    fn a_field_shown_twice_starts_each_box_with_nothing_to_undo() {
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        theme::install(&ctx);
        let mut app = App::with(Settings::from_file(None), Recent::from_file(None));
        assert!(app.add_tab(&ctx, form_doc(&[("name", 2)]), None, None, false));
        let ty = Typist::new(&ctx);
        ty.click_field(&mut app, "name", 0);
        ty.text(&mut app, "Alpha");
        ty.key(&mut app, Key::Escape, egui::Modifiers::NONE);
        ty.pause(&mut app);
        assert_eq!(saved_field(&app.tabs[0], "name"), "Alpha");
        ty.click_field(&mut app, "name", 1);
        let tree = ty.key(&mut app, Key::Z, CTRL);
        assert_eq!(focused_text(&tree).as_deref(), Some("Alpha"));
        ty.key(&mut app, Key::Escape, egui::Modifiers::NONE);
        ty.pause(&mut app);
        assert_eq!(saved_field(&app.tabs[0], "name"), "Alpha");
    }

    /// A second note on a page starts with nothing to undo: Ctrl+Z in it doesn't bring
    /// back another note's text, whether it is new or an existing one opened for typing
    /// (BUG-03, audit A10).
    #[test]
    fn a_second_note_on_a_page_does_not_undo_into_the_first() {
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        theme::install(&ctx);
        let mut app = app_with(&ctx, 1);
        let ty = Typist::new(&ctx);
        let (min, k) = ty.page(&mut app);
        let at = |x: f32, y: f32| min + egui::vec2(x, y) * k;
        let typed = |app: &App| app.tabs[0].ed.writing.as_ref().map(|w| w.text.clone());
        let esc = |app: &mut App| {
            ty.key(app, Key::Escape, egui::Modifiers::NONE);
            ty.pause(app);
        };
        for (x, text) in [(100.0, "Walrus"), (300.0, "")] {
            app.act(&ctx, Act::Tool(Tool::Note));
            ty.click(&mut app, at(x, 100.0));
            ty.text(&mut app, text);
            ty.key(&mut app, Key::Z, CTRL);
            ty.key(&mut app, Key::Z, CTRL);
            assert_eq!(typed(&app).as_deref(), Some(""));
            ty.key(&mut app, Key::Y, CTRL);
            assert_eq!(typed(&app).as_deref(), Some(text));
            esc(&mut app);
        }
        assert_eq!(comments(&app), ["Walrus"]);
        // Existing notes opened for typing: one changed, then the other.
        app.act(&ctx, Act::Tool(Tool::Note));
        ty.click(&mut app, at(300.0, 100.0));
        ty.text(&mut app, "Seal");
        esc(&mut app);
        assert_eq!(comments(&app), ["Walrus", "Seal"]);
        let open = |app: &mut App, text: &str| {
            let t = &mut app.tabs[0];
            let annots = t.pdf.annotations();
            let a = annots
                .iter()
                .find(|a| a.contents == text)
                .expect("the note");
            t.ed.annots(&t.pdf, 0);
            t.ed.selection = Selection::Annot { page: 0, id: a.id };
            t.ed.write_selected();
            ty.pause(app);
        };
        open(&mut app, "Walrus");
        ty.key(&mut app, Key::End, egui::Modifiers::NONE);
        ty.text(&mut app, " typed");
        esc(&mut app);
        assert_eq!(comments(&app), ["Walrus typed", "Seal"]);
        open(&mut app, "Seal");
        ty.key(&mut app, Key::Z, CTRL);
        assert_eq!(typed(&app).as_deref(), Some("Seal"));
        esc(&mut app);
        assert_eq!(comments(&app), ["Walrus typed", "Seal"]);
    }

    /// A note open on the page for typing stays open while its inspector changes it: the
    /// opacity applies at once, the keyboard comes back to the note, and the inspector has
    /// no comment box of its own for it meanwhile, so one comment has one editor (ENH-13;
    /// the user, batch 40's Q1).
    #[test]
    fn an_open_note_stays_open_while_its_inspector_changes_it() {
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        theme::install(&ctx);
        let mut app = App::with(Settings::from_file(None), Recent::from_file(None));
        let note = typing_doc(&mut app, &ctx);
        let ty = Typist::new(&ctx);
        let t = &mut app.tabs[0];
        t.ed.annots(&t.pdf, 0);
        t.ed.selection = Selection::Annot { page: 0, id: note };
        t.ed.write_selected();
        ty.pause(&mut app);
        ty.key(&mut app, Key::End, egui::Modifiers::NONE);
        ty.text(&mut app, " ate fish");
        let typed = |app: &App| app.tabs[0].ed.writing.as_ref().map(|w| w.text.clone());
        let tree = ty.pause(&mut app);
        let says = inspector_says(&tree);
        assert!(says.iter().any(|s| s == "Opacity"), "{says:?}");
        assert!(
            !says.iter().any(|s| s == "Comment"),
            "a second editor: {says:?}"
        );
        // The slider's middle, pressed and let go as people do, a frame apart.
        let p = middle(&tree, "Opacity");
        ty.frame(&mut app, vec![egui::Event::PointerMoved(p), press(p, true)]);
        ty.frame(&mut app, vec![press(p, false)]);
        ty.pause(&mut app);
        assert_eq!(
            typed(&app).as_deref(),
            Some("Walrus ate fish"),
            "the note closed"
        );
        let opacity = |app: &App| app.tabs[0].pdf.annotations()[0].opacity;
        assert!(opacity(&app) < 0.9, "the opacity wasn't applied");
        let focused = ctx.memory(|m| m.focused());
        assert_eq!(focused, app.tabs[0].ed.writing_id(), "the keyboard");
        ty.text(&mut app, "!");
        ty.key(&mut app, Key::Escape, egui::Modifiers::NONE);
        ty.pause(&mut app);
        assert_eq!(comments(&app), ["Walrus ate fish!"]);
        assert!(opacity(&app) < 0.9);
    }

    /// A new text box being typed, not on the page yet, stays open while the inspector
    /// changes the Text box tool's look, as word processors keep a box open while its pane
    /// is used: the keyboard comes back to it, the box shows the size chosen, and it is kept
    /// with the look chosen (the user, with batch 40's stage 2 report).
    #[test]
    fn a_new_text_box_stays_open_while_the_inspector_changes_its_look() {
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        theme::install(&ctx);
        let mut app = app_with(&ctx, 1);
        let ty = Typist::new(&ctx);
        let (min, k) = ty.page(&mut app);
        app.act(&ctx, Act::Tool(Tool::TextBox));
        ty.click(&mut app, min + egui::vec2(100.0, 100.0) * k);
        ty.text(&mut app, "Walrus");
        let typed = |app: &App| app.tabs[0].ed.writing.as_ref().map(|w| w.text.clone());
        let height = |tree: &egui::accesskit::TreeUpdate| {
            let (_, n) = tree.nodes.iter().find(|(id, _)| *id == tree.focus).unwrap();
            n.bounds().expect("the box's place").height()
        };
        let tree = ty.pause(&mut app);
        let before = height(&tree);
        ty.click(&mut app, middle(&tree, "Background"));
        assert_eq!(typed(&app).as_deref(), Some("Walrus"), "the box closed");
        let focused = ctx.memory(|m| m.focused());
        assert_eq!(focused, app.tabs[0].ed.writing_id(), "the keyboard");
        // A bigger size, as picked in the inspector, shows on the box being typed.
        app.tabs[0].ed.box_style.size = 48.0;
        let tree = ty.pause(&mut app);
        let after = height(&tree);
        assert!(after > before * 1.4, "{after} then {before}");
        ty.text(&mut app, " ate");
        ty.key(&mut app, Key::Escape, egui::Modifiers::NONE);
        ty.pause(&mut app);
        assert_eq!(comments(&app), ["Walrus ate"]);
        let style = app.tabs[0].pdf.annotations()[0]
            .text_style
            .expect("its look");
        assert!(style.fill.is_some(), "no background");
        assert_eq!(style.size, 48.0);
    }

    /// A box's typing history survives clicking away and back while nothing is kept in the
    /// document meanwhile: typing undone in a field, clicked away from and back into, is
    /// redone by Ctrl+Shift+Z. With a change to the document in between, the history is
    /// gone, so nothing the document has since can be overwritten from it (ENH-32,
    /// BUG-03/04's guard).
    #[test]
    fn a_box_keeps_its_typing_history_when_clicked_away_and_back() {
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        theme::install(&ctx);
        let ty = Typist::new(&ctx);
        for changed in [false, true] {
            let mut app = App::with(Settings::from_file(None), Recent::from_file(None));
            let pdf = form_doc(&[("Name", 1)]);
            assert!(app.add_tab(&ctx, pdf, None, None, false));
            let (min, k) = ty.page(&mut app);
            ty.click_field(&mut app, "Name", 0);
            ty.text(&mut app, "Ann");
            ty.pause(&mut app);
            let tree = ty.key(&mut app, Key::Z, CTRL);
            assert_eq!(focused_text(&tree).as_deref(), Some(""), "{changed}");
            // Away, on the page: nothing to keep.
            ty.click(&mut app, min + egui::vec2(300.0, 600.0) * k);
            assert_eq!(saved_field(&app.tabs[0], "Name"), "", "{changed}");
            if changed {
                let font = FontChoice::default();
                let add = |p: &mut Pdf| p.add_text(0, (100.0, 700.0), "Seal", font, 12.0, [0.0; 3]);
                assert!(app.tabs[0].edit(&ctx, add));
            }
            ty.click_field(&mut app, "Name", 0);
            let tree = ty.key(&mut app, Key::Z, CTRL_SHIFT);
            let want = if changed { "" } else { "Ann" };
            assert_eq!(
                focused_text(&tree).as_deref(),
                Some(want),
                "changed: {changed}"
            );
        }
    }

    /// A new note typed in, emptied and let go adds nothing; the next new note on the page,
    /// with the same box, starts with nothing to undo: Ctrl+Z there never brings back what
    /// was typed in the first (ENH-32's guard).
    #[test]
    fn a_new_note_never_takes_the_last_ones_typing_history() {
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        theme::install(&ctx);
        let mut app = app_with(&ctx, 1);
        let ty = Typist::new(&ctx);
        let (min, k) = ty.page(&mut app);
        app.act(&ctx, Act::Tool(Tool::Note));
        ty.click(&mut app, min + egui::vec2(100.0, 100.0) * k);
        ty.text(&mut app, "Walrus");
        ty.pause(&mut app);
        for _ in 0.."Walrus".len() {
            ty.key(&mut app, Key::Backspace, egui::Modifiers::NONE);
        }
        ty.pause(&mut app);
        // Away, on the page: nothing to keep. Then a new note elsewhere on it.
        ty.click(&mut app, min + egui::vec2(300.0, 300.0) * k);
        assert!(comments(&app).is_empty());
        ty.click(&mut app, min + egui::vec2(100.0, 400.0) * k);
        let tree = ty.key(&mut app, Key::Z, CTRL);
        assert_eq!(focused_text(&tree).as_deref(), Some(""));
    }

    /// Undo keys in a form field act on its typing only: with Name kept and Notes being
    /// typed, they take back and redo Notes' typing, never Name, and with nothing left to
    /// undo there they do nothing. Out of the fields, Undo and Redo are the document's
    /// again (BUG-04, audit A03).
    #[test]
    fn undo_in_a_form_field_stays_in_the_field() {
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        theme::install(&ctx);
        let mut app = App::with(Settings::from_file(None), Recent::from_file(None));
        let pdf = form_doc(&[("Name", 1), ("Notes", 1)]);
        assert!(app.add_tab(&ctx, pdf, None, None, false));
        let ty = Typist::new(&ctx);
        ty.click_field(&mut app, "Name", 0);
        ty.text(&mut app, "Ann");
        // Tab keeps Name and moves on to Notes.
        ty.key(&mut app, Key::Tab, egui::Modifiers::NONE);
        ty.pause(&mut app);
        assert_eq!(saved_field(&app.tabs[0], "Name"), "Ann");
        ty.text(&mut app, "Hello");
        let steps = [
            (Key::Z, CTRL, ""),
            (Key::Z, CTRL, ""),
            (Key::Z, CTRL_SHIFT, "Hello"),
            (Key::Z, CTRL, ""),
            (Key::Y, CTRL, "Hello"),
        ];
        for (key, modifiers, want) in steps {
            let tree = ty.key(&mut app, key, modifiers);
            let typed = focused_text(&tree);
            assert_eq!(typed.as_deref(), Some(want), "{key:?} {modifiers:?}");
            assert_eq!(saved_field(&app.tabs[0], "Name"), "Ann");
        }
        ty.key(&mut app, Key::Escape, egui::Modifiers::NONE);
        ty.pause(&mut app);
        let saved = |app: &App| {
            let t = &app.tabs[0];
            (saved_field(t, "Name"), saved_field(t, "Notes"))
        };
        assert_eq!(saved(&app), ("Ann".into(), "Hello".into()));
        let steps = [
            (Key::Z, CTRL, ("Ann", "")),
            (Key::Z, CTRL, ("", "")),
            (Key::Y, CTRL, ("Ann", "")),
            (Key::Z, CTRL_SHIFT, ("Ann", "Hello")),
        ];
        for (key, modifiers, (name, notes)) in steps {
            ty.key(&mut app, key, modifiers);
            let want = (name.to_string(), notes.to_string());
            assert_eq!(saved(&app), want, "{key:?} {modifiers:?}");
        }
    }

    /// A tab of a page with the empty field "name", the note "Walrus" at (100, 400), the
    /// line "Otter" at (450, 300) and the title "Seal", set as an undoable change: a
    /// document with history to undo and one of each kind of thing typed in. Returns the
    /// note's id.
    fn typing_doc(app: &mut App, ctx: &egui::Context) -> (u32, u16) {
        let mut pdf = form_doc(&[("name", 1)]);
        let note = pdf.add_note(0, (100.0, 400.0), [1.0, 1.0, 0.0], "Walrus", "Tester");
        let font = FontChoice::default();
        pdf.add_text(0, (450.0, 300.0), "Otter", font, 20.0, [0.0; 3])
            .unwrap();
        assert!(app.add_tab(ctx, pdf, None, None, false));
        let info = limarepdf_core::Info {
            title: Some("Seal".into()),
            ..Default::default()
        };
        assert!(app.tabs[0].edit(ctx, |p| p.set_info(&info)));
        note.unwrap()
    }

    /// Give the keyboard to the box `case` names in [`typing_doc`]'s tab, with the read
    /// tool and nothing selected first.
    fn start_typing(app: &mut App, ty: &Typist, case: &str, note: (u32, u16)) {
        let ctx = ty.ctx;
        app.act(ctx, Act::Tool(Tool::Read));
        app.tabs[0].ed.selection = Selection::None;
        let (min, k) = ty.page(app);
        let at = |x: f32, y: f32| min + egui::vec2(x, y) * k;
        match case {
            "form field" => ty.click_field(app, "name", 0),
            "new note" | "new text box" | "new text" => {
                // Within the window, clear of the field, the note and the line.
                let (tool, y) = match case {
                    "new note" => (Tool::Note, 150.0),
                    "new text box" => (Tool::TextBox, 220.0),
                    _ => (Tool::Text, 330.0),
                };
                app.act(ctx, Act::Tool(tool));
                ty.click(app, at(300.0, y));
            }
            "line on the page" => {
                // A double-click on it with the read tool.
                let p = at(470.0, 295.0);
                ty.frame(app, vec![egui::Event::PointerMoved(p), press(p, true)]);
                ty.frame(app, vec![press(p, false)]);
                ty.frame(app, vec![press(p, true)]);
                ty.frame(app, vec![press(p, false)]);
                ty.pause(app);
            }
            "comment in the inspector" => {
                let t = &mut app.tabs[0];
                t.ed.annots(&t.pdf, 0);
                t.ed.selection = Selection::Annot { page: 0, id: note };
                let tree = ty.pause(app);
                ty.click(app, middle(&tree, "Comment"));
            }
            "document title" => {
                app.tabs[0].forms.reveal = Some(inspector::Section::Properties);
                let tree = ty.pause(app);
                let (_, n) = tree
                    .nodes
                    .iter()
                    .find(|(_, n)| n.value() == Some("Seal"))
                    .expect("the title");
                let b = n.bounds().unwrap();
                let p = egui::pos2(((b.x0 + b.x1) / 2.0) as f32, ((b.y0 + b.y1) / 2.0) as f32);
                ty.click(app, p);
            }
            "watermark text" => {
                app.tabs[0].forms.reveal = Some(inspector::Section::Watermark);
                ty.pause(app);
                let id = app.tabs[0].ed.scope().with(("stamp-text", "Text"));
                ctx.memory_mut(|m| m.request_focus(id));
                ty.pause(app);
            }
            _ => panic!("no case {case}"),
        }
    }

    /// The undo keys in each kind of text box, in a document with history to undo: Ctrl+Z,
    /// Ctrl+Shift+Z and Ctrl+Y take back and redo the box's own typing, Ctrl+Z with nothing
    /// left there does nothing, and the document is never touched (BUG-04).
    #[test]
    fn undo_keys_in_every_text_box_act_on_its_typing_only() {
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        theme::install(&ctx);
        let mut app = App::with(Settings::from_file(None), Recent::from_file(None));
        let note = typing_doc(&mut app, &ctx);
        let ty = Typist::new(&ctx);
        let cases = [
            "form field",
            "new note",
            "new text box",
            "new text",
            "line on the page",
            "comment in the inspector",
            "document title",
            "watermark text",
        ];
        for case in cases {
            start_typing(&mut app, &ty, case, note);
            let tree = ty.key(&mut app, Key::End, egui::Modifiers::NONE);
            let start = focused_text(&tree).unwrap_or_else(|| panic!("{case}: no keyboard"));
            let state = app.tabs[0].state();
            ty.text(&mut app, "typed");
            let typed = format!("{start}typed");
            let steps = [
                (Key::Z, CTRL, &start),
                (Key::Z, CTRL_SHIFT, &typed),
                (Key::Z, CTRL, &start),
                (Key::Y, CTRL, &typed),
                (Key::Z, CTRL, &start),
                (Key::Z, CTRL, &start),
            ];
            for (key, modifiers, want) in steps {
                let tree = ty.key(&mut app, key, modifiers);
                let now = focused_text(&tree);
                assert_eq!(now.as_ref(), Some(want), "{case}: {key:?} {modifiers:?}");
                assert!(app.tabs[0].state() == state, "{case}: the document changed");
            }
            ty.key(&mut app, Key::Escape, egui::Modifiers::NONE);
            ty.pause(&mut app);
            assert!(app.tabs[0].state() == state, "{case}: something was kept");
        }
    }

    /// Undo and Redo in the commands, clicked in each kind of text box, do what Ctrl+Z and
    /// Ctrl+Y do there: take back and redo the box's own typing, the keyboard staying in
    /// it, greyed when there is nothing there to take back or redo though the document has
    /// history, and the document never touched (ENH-39; the user, batch 40's Q4).
    #[test]
    fn the_undo_buttons_act_on_the_typing_in_every_text_box() {
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        theme::install(&ctx);
        let mut app = App::with(Settings::from_file(None), Recent::from_file(None));
        let note = typing_doc(&mut app, &ctx);
        assert!(app.tabs[0].can_undo(), "the document has nothing to undo");
        let ty = Typist::new(&ctx);
        let on = |tree: &egui::accesskit::TreeUpdate, name: &str| {
            let button = |n: &egui::accesskit::Node| {
                n.label() == Some(name) && n.role() == egui::accesskit::Role::Button
            };
            let (_, n) = tree.nodes.iter().find(|(_, n)| button(n)).expect(name);
            !n.is_disabled()
        };
        let cases = [
            "form field",
            "new note",
            "new text box",
            "new text",
            "line on the page",
            "comment in the inspector",
            "document title",
            "watermark text",
        ];
        for case in cases {
            start_typing(&mut app, &ty, case, note);
            let tree = ty.key(&mut app, Key::End, egui::Modifiers::NONE);
            let start = focused_text(&tree).unwrap_or_else(|| panic!("{case}: no keyboard"));
            let state = app.tabs[0].state();
            ty.text(&mut app, "typed");
            let typed = format!("{start}typed");
            let tree = ty.pause(&mut app);
            assert_eq!(
                (on(&tree, "Undo"), on(&tree, "Redo")),
                (true, false),
                "{case}"
            );
            let steps = [
                ("Undo", &start, (false, true)),
                ("Redo", &typed, (true, false)),
                ("Undo", &start, (false, true)),
            ];
            for (button, want, buttons) in steps {
                let tree = ty.pause(&mut app);
                ty.click(&mut app, middle(&tree, button));
                let tree = ty.pause(&mut app);
                assert_eq!(focused_text(&tree).as_ref(), Some(want), "{case}: {button}");
                let now = (on(&tree, "Undo"), on(&tree, "Redo"));
                assert_eq!(now, buttons, "{case}: after {button}");
                assert!(app.tabs[0].state() == state, "{case}: the document changed");
            }
            ty.key(&mut app, Key::Escape, egui::Modifiers::NONE);
            ty.pause(&mut app);
            assert!(app.tabs[0].state() == state, "{case}: something was kept");
        }
    }

    /// The tab's document as it would be saved now, read back from its bytes.
    fn saved_pdf(t: &Tab) -> Pdf {
        Pdf::from_bytes(&t.pdf.clone().snapshot().unwrap(), None).unwrap()
    }

    /// The first page's rotation in the tab's document as it would be saved now.
    fn saved_rotation(t: &Tab) -> i64 {
        let pdf = saved_pdf(t);
        let page = pdf.doc.get_dictionary(pdf.page_ids()[0]).unwrap();
        page.get(b"Rotate").and_then(|o| o.as_i64()).unwrap_or(0)
    }

    /// Run the command `name` from the command palette, typed there.
    fn from_palette(app: &mut App, ty: &Typist, name: &str) {
        ty.key(app, Key::K, CTRL);
        ty.text(app, name);
        ty.key(app, Key::Enter, egui::Modifiers::NONE);
        ty.pause(app);
    }

    /// Add text, type, Rotate clockwise from the palette: the text goes into the document
    /// first, a step of its own, so the page turns with it on; Undo turns the page back
    /// and keeps the text, and Undo again takes the text back (BUG-01, audit A20).
    #[test]
    fn new_text_is_kept_before_the_page_is_rotated() {
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        theme::install(&ctx);
        let mut app = app_with(&ctx, 1);
        let ty = Typist::new(&ctx);
        let (min, k) = ty.page(&mut app);
        app.act(&ctx, Act::Tool(Tool::Text));
        ty.click(&mut app, min + egui::vec2(100.0, 200.0) * k);
        ty.text(&mut app, "PendingAuditText");
        from_palette(&mut app, &ty, "Rotate clockwise");
        let typed = |app: &App| {
            saved_pdf(&app.tabs[0])
                .page_text(0)
                .contains("PendingAuditText")
        };
        let t = &app.tabs[0];
        assert_eq!(saved_rotation(t), 90);
        assert!(typed(&app), "the text was lost");
        assert!(t.ed.adding.is_none() && !t.ed.pending());
        app.act(&ctx, Act::Undo);
        assert_eq!(saved_rotation(&app.tabs[0]), 0);
        assert!(typed(&app), "Undo took the text back with the rotation");
        app.act(&ctx, Act::Undo);
        assert!(!typed(&app));
        // The change before the text is still there.
        assert_eq!(app.tabs[0].page_count(), 2);
        app.act(&ctx, Act::Redo);
        app.act(&ctx, Act::Redo);
        assert_eq!(saved_rotation(&app.tabs[0]), 90);
        assert!(typed(&app));
    }

    /// Each kind of typing goes into the document before a page command run from the
    /// palette, and the command is a step of its own after it: Undo turns the page back and
    /// keeps the typing (BUG-01).
    #[test]
    fn every_kind_of_typing_is_kept_before_a_page_command() {
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        theme::install(&ctx);
        let mut app = App::with(Settings::from_file(None), Recent::from_file(None));
        let note = typing_doc(&mut app, &ctx);
        let ty = Typist::new(&ctx);
        let cases = [
            "form field",
            "new note",
            "new text box",
            "new text",
            "line on the page",
            "comment in the inspector",
            "document title",
        ];
        for case in cases {
            start_typing(&mut app, &ty, case, note);
            ty.key(&mut app, Key::End, egui::Modifiers::NONE);
            ty.text(&mut app, "typed");
            from_palette(&mut app, &ty, "Rotate clockwise");
            let kept = |app: &App| {
                let pdf = saved_pdf(&app.tabs[0]);
                let comment = |text: &str| pdf.annotations().iter().any(|a| a.contents == text);
                let line = |text: &str| {
                    let objs = pdf.page_objects(0).unwrap();
                    objs.iter().any(|o| o.text.as_deref() == Some(text))
                };
                match case {
                    "form field" => saved_field(&app.tabs[0], "name") == "typed",
                    "new note" | "new text box" => comment("typed"),
                    "new text" => line("typed"),
                    "line on the page" => line("Ottertyped"),
                    "comment in the inspector" => comment("Walrustyped"),
                    _ => pdf.info().title.as_deref() == Some("Sealtyped"),
                }
            };
            let t = &app.tabs[0];
            assert_eq!(saved_rotation(t), 90, "{case}: not rotated");
            assert!(kept(&app), "{case}: the typing was lost");
            assert!(
                !t.ed.pending() && !t.typing_in_forms(),
                "{case}: still pending"
            );
            app.act(&ctx, Act::Undo);
            assert_eq!(saved_rotation(&app.tabs[0]), 0, "{case}: Undo");
            assert!(
                kept(&app),
                "{case}: Undo took the typing back with the rotation"
            );
        }
    }

    /// Text being added goes into the document before a change made on the tab itself
    /// (pages reordered by dragging their thumbnails, a bookmark), and before the Crop
    /// section shows the pages whole: each is drawn again without the text being typed.
    #[test]
    fn typing_is_kept_before_the_tab_changes_or_redraws_its_pages() {
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        theme::install(&ctx);
        let mut app = app_with(&ctx, 1);
        let ty = Typist::new(&ctx);
        let type_at = |app: &mut App, y: f32, text: &str| {
            let (min, k) = ty.page(app);
            app.act(&ctx, Act::Tool(Tool::Text));
            ty.click(app, min + egui::vec2(100.0, y) * k);
            ty.text(app, text);
        };
        let lines = |app: &App, page: usize| saved_pdf(&app.tabs[0]).page_text(page);
        type_at(&mut app, 200.0, "Walrus");
        assert!(app.tabs[0].edit(&ctx, |p| p.reorder(&[1, 0])));
        assert!(lines(&app, 1).contains("Walrus"), "the text was lost");
        // Steps of their own: Undo puts the pages back with the text on.
        app.act(&ctx, Act::Undo);
        assert!(lines(&app, 0).contains("Walrus"));
        type_at(&mut app, 300.0, "Seal");
        app.act(&ctx, Act::Crop);
        ty.pause(&mut app);
        assert!(app.tabs[0].crop_view());
        assert!(lines(&app, 0).contains("Seal"), "the text was lost");
    }

    /// Undo clicked in the commands while a new note is typed takes back its typing, as
    /// Ctrl+Z does there, not the change before it (BUG-01, seen in batch 1); the note stays
    /// open, and Redo brings the typing back. Once the note is kept, Undo takes it back
    /// whole, and Redo brings it back (ENH-39; the user, batch 40's Q4).
    #[test]
    fn undo_on_the_toolbar_takes_back_a_note_being_typed() {
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        theme::install(&ctx);
        let mut app = app_with(&ctx, 1);
        let ty = Typist::new(&ctx);
        let (min, k) = ty.page(&mut app);
        app.act(&ctx, Act::Tool(Tool::Note));
        ty.click(&mut app, min + egui::vec2(100.0, 100.0) * k);
        ty.text(&mut app, "Walrus");
        let typed = |app: &App| app.tabs[0].ed.writing.as_ref().map(|w| w.text.clone());
        let tree = ty.pause(&mut app);
        ty.click(&mut app, middle(&tree, "Undo"));
        ty.pause(&mut app);
        let t = &app.tabs[0];
        assert_eq!(
            t.page_count(),
            2,
            "Undo took back the change before the note"
        );
        assert!(comments(&app).is_empty());
        assert_eq!(typed(&app).as_deref(), Some(""), "the note's typing");
        let tree = ty.pause(&mut app);
        ty.click(&mut app, middle(&tree, "Redo"));
        ty.pause(&mut app);
        assert_eq!(typed(&app).as_deref(), Some("Walrus"));
        // Kept, it is one change to take back.
        ty.key(&mut app, Key::Escape, egui::Modifiers::NONE);
        let tree = ty.pause(&mut app);
        assert_eq!(comments(&app), ["Walrus"]);
        ty.click(&mut app, middle(&tree, "Undo"));
        let tree = ty.pause(&mut app);
        assert!(comments(&app).is_empty() && app.tabs[0].ed.writing.is_none());
        assert_eq!(app.tabs[0].page_count(), 2);
        ty.click(&mut app, middle(&tree, "Redo"));
        ty.pause(&mut app);
        assert_eq!(comments(&app), ["Walrus"]);
    }

    /// Remove blank pages run while text is being added: the text goes into the document
    /// before the job takes its copy, so the result has it. A result arriving while text is
    /// typed is for a document that has changed since: the typing is kept and the result
    /// is discarded, saying so (BUG-01).
    #[test]
    fn background_results_keep_what_is_being_typed() {
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        theme::install(&ctx);
        let mut app = App::with(Settings::from_file(None), Recent::from_file(None));
        let mut pdf = Pdf::new();
        for i in 0..3 {
            pdf.insert_blank(i, 612.0, 792.0).unwrap();
        }
        let font = FontChoice::default();
        pdf.add_text(0, (100.0, 100.0), "Walrus", font, 20.0, [0.0; 3])
            .unwrap();
        assert!(app.add_tab(&ctx, pdf, None, None, false));
        let ty = Typist::new(&ctx);
        let type_at = |app: &mut App, y: f32, text: &str| {
            let (min, k) = ty.page(app);
            app.act(&ctx, Act::Tool(Tool::Text));
            ty.click(app, min + egui::vec2(100.0, y) * k);
            ty.text(app, text);
        };
        let page_text = |app: &App| saved_pdf(&app.tabs[0]).page_text(0);
        type_at(&mut app, 300.0, "Seal");
        from_palette(&mut app, &ty, "Remove blank pages");
        for _ in 0..500 {
            if !app.busy {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
            ty.frame(&mut app, vec![]);
        }
        assert!(!app.busy, "the job didn't finish");
        assert_eq!(app.tabs[0].page_count(), 1);
        assert!(page_text(&app).contains("Seal"), "the text was lost");
        app.dialog = Dialog::None;
        // A result for the document as it is now, arriving while text is typed.
        let t = &app.tabs[0];
        let (tab, revision) = (t.id, t.revision);
        let mut other = saved_pdf(t);
        other.insert_blank(1, 612.0, 792.0).unwrap();
        type_at(&mut app, 400.0, "Otter");
        let result = Ok((other, true, "Done.".to_string()));
        let title = "Remove blank pages";
        let job = Loaded::Job {
            tab,
            revision,
            title,
            result,
        };
        app.tx.send(job).unwrap();
        ty.pause(&mut app);
        assert_eq!(app.tabs[0].page_count(), 1, "the result was applied");
        assert!(page_text(&app).contains("Otter"), "the text was lost");
        assert!(
            matches!(&app.dialog, Dialog::Message(_, m) if m.contains("changed while")),
            "nothing said"
        );
    }

    /// Names the folder of stand-ins for CUPS, for the tests run by [`rerun_with_cups`].
    #[cfg(not(windows))]
    const STAND_IN_CUPS: &str = "LIMAREPDF_TEST_STAND_IN_CUPS";

    /// A new folder `name` holding the stand-in commands `scripts` (name, shell script).
    #[cfg(not(windows))]
    fn stand_in_cups(name: &str, scripts: &[(&str, &str)]) -> PathBuf {
        use std::os::unix::fs::PermissionsExt;
        let dir = std::env::temp_dir().join(format!("limarepdf-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        for (name, script) in scripts {
            let path = dir.join(name);
            std::fs::write(&path, script).unwrap();
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        dir
    }

    /// The timing log (REL-07) by the whole route: a file opened and a key pressed with
    /// `LIMAREPDF_TIMING` set are logged from the read to the first page drawn, and from the
    /// key to the frame built after it. Run in a copy of the tests, as the log is the
    /// process's.
    #[test]
    fn the_timing_log_follows_a_file_from_its_read_to_its_page_drawn() {
        let dir = test_dir("timing");
        let log = dir.join("timing.tsv");
        pages_file(&dir.join("doc.pdf"), 300.0, 2, None);
        let out = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "app::tests::timed_with_the_log_on"])
            .args(["--ignored", "--test-threads=1", "--nocapture"])
            .env(timing::VAR, &log)
            .output()
            .unwrap();
        let said = String::from_utf8_lossy(&out.stdout).to_string()
            + &String::from_utf8_lossy(&out.stderr);
        assert!(out.status.success() && said.contains("1 passed"), "{said}");
        let text = std::fs::read_to_string(&log).unwrap();
        let lines: Vec<Vec<&str>> = text.lines().map(|l| l.split('\t').collect()).collect();
        let at = |event: &str, detail: &str| {
            let found = lines
                .iter()
                .position(|l| l[2] == event && l[3].starts_with(detail));
            found.unwrap_or_else(|| panic!("no {event} {detail} in\n{text}"))
        };
        assert_eq!(at("start", ""), 0);
        assert!(at("open start", "") < at("open end", ""));
        assert!(at("open end", "") < at("render start", "page 1"));
        assert!(at("render end", "page 1") < at("page drawn", "tab 1 page 1 at"));
        let key = at("input", "key Escape down");
        assert!(lines[key..].iter().any(|l| l[2] == "frame built"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The part of [`the_timing_log_follows_a_file_from_its_read_to_its_page_drawn`] run with
    /// the log on; without it it does nothing.
    #[test]
    #[ignore = "run by the_timing_log_follows_a_file_from_its_read_to_its_page_drawn"]
    fn timed_with_the_log_on() {
        let Some(log) = std::env::var_os(timing::VAR) else {
            return;
        };
        timing::start();
        assert!(timing::on());
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        theme::install(&ctx);
        let mut app = App::with(Settings::from_file(None), Recent::from_file(None));
        let doc = Path::new(&log).with_file_name("doc.pdf");
        open_and_wait(&mut app, &ctx, &doc, 1);
        // A frame as eframe runs it: the input through the hook, then the window.
        let frame = |app: &mut App, events: Vec<egui::Event>| {
            let mut raw = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1400.0, TEST_HEIGHT),
                )),
                events,
                ..Default::default()
            };
            eframe::App::raw_input_hook(app, &ctx, &mut raw);
            let mut f = eframe::Frame::_new_kittest();
            let mut out = ctx.run_ui(raw, |ui| eframe::App::ui(app, ui, &mut f));
            out.textures_delta.clear();
        };
        let drawn = || std::fs::read_to_string(&log).is_ok_and(|t| t.contains("\tpage drawn\t"));
        for _ in 0..500 {
            frame(&mut app, vec![]);
            if drawn() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        assert!(drawn(), "the first page was never drawn");
        frame(&mut app, vec![key_with(Key::Escape, Default::default())]);
    }

    /// Runs the ignored test `test` in a copy of the tests with the stand-ins in `dir` first
    /// on its PATH, so no job reaches a real printer and the rest of the tests keep their
    /// PATH.
    #[cfg(not(windows))]
    fn rerun_with_cups(dir: &Path, test: &str) {
        let path = std::env::var("PATH").unwrap_or_default();
        let out = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", test])
            .args(["--ignored", "--test-threads=1", "--nocapture"])
            .env("PATH", format!("{}:{path}", dir.display()))
            .env(STAND_IN_CUPS, dir)
            .output()
            .unwrap();
        let said = String::from_utf8_lossy(&out.stdout).to_string()
            + &String::from_utf8_lossy(&out.stderr);
        assert!(out.status.success() && said.contains("1 passed"), "{said}");
    }

    /// Print right after typing in a field prints what was typed, by the whole route: Ctrl+P
    /// with the field still being typed in, the printers listed, Print pressed. A value the
    /// field can't take stops Print first, with the typing kept. Run with stand-ins for
    /// `lpstat` and `lp` (BUG-10, audit A08).
    #[cfg(not(windows))]
    #[test]
    fn print_right_after_typing_prints_the_typed_value() {
        let lpstat = "#!/bin/sh\ncase \"$1\" in\n  -e) echo limarepdf-test ;;\n  \
                      -d) echo 'system default destination: limarepdf-test' ;;\nesac\n";
        let lp = "#!/bin/sh\ncat > \"$(dirname \"$0\")/job.pdf\"\n\
                  echo 'request id is limarepdf-test-1 (1 file(s))'\n";
        let dir = stand_in_cups(
            "print",
            &[("lpstat", lpstat), ("lp", lp), ("lpoptions", "#!/bin/sh\n")],
        );
        let job = dir.join("job.pdf");
        rerun_with_cups(&dir, "app::tests::print_with_stand_in_cups");
        let printed = Pdf::open(&job, None).expect("the job sent to lp");
        let text = printed.page_text(0);
        assert!(text.contains("Walrus"), "the job holds {text:?}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The part of [`print_right_after_typing_prints_the_typed_value`] run with the
    /// stand-ins first on PATH; without them it does nothing.
    #[cfg(not(windows))]
    #[test]
    #[ignore = "run by print_right_after_typing_prints_the_typed_value"]
    fn print_with_stand_in_cups() {
        if std::env::var_os(STAND_IN_CUPS).is_none() {
            return;
        }
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        theme::install(&ctx);
        let mut app = App::with(Settings::from_file(None), Recent::from_file(None));
        assert!(app.add_tab(&ctx, form_doc(&[("name", 1)]), None, None, false));
        let ty = Typist::new(&ctx);
        // Right-to-left text, which a field can't hold: Print stops, with the typing kept
        // in the field and the reason shown.
        ty.click_field(&mut app, "name", 0);
        ty.text(&mut app, "مرحبا");
        ty.key(&mut app, Key::P, CTRL);
        assert!(!matches!(app.dialog, Dialog::Print(..)), "Print opened");
        assert!(matches!(app.dialog, Dialog::Message(..)), "no reason given");
        let t = &app.tabs[0];
        assert!(t.ed.pending() && !t.dirty, "the typing wasn't kept");
        assert_eq!(saved_field(t, "name"), "");
        app.dialog = Dialog::None;
        let tree = ty.pause(&mut app);
        assert_eq!(focused_text(&tree).as_deref(), Some("مرحبا"));
        // Typed over, and Ctrl+P with the field still being typed in.
        ty.key(&mut app, Key::A, CTRL);
        ty.text(&mut app, "Walrus");
        ty.key(&mut app, Key::P, CTRL);
        assert!(matches!(app.dialog, Dialog::Print(..)), "Print didn't open");
        assert_eq!(saved_field(&app.tabs[0], "name"), "Walrus");
        wait_for(&mut app, &ctx, print_ready);
        click(&mut app, &ctx, "Print");
        // Its frames moved the clock on.
        ty.time.set(ty.time.get() + 10.0);
        for _ in 0..500 {
            if !app.busy {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
            ty.frame(&mut app, vec![]);
        }
        let sent = "Sent 1 page(s) to limarepdf-test.";
        assert!(
            matches!(&app.dialog, Dialog::Message(_, m) if m.starts_with(sent)),
            "{}",
            match &app.dialog {
                Dialog::Message(_, m) => m.as_str(),
                _ => "no message",
            }
        );
    }

    /// Whether Print is open with the printers found, ready to print.
    #[cfg(not(windows))]
    fn print_ready(app: &App) -> bool {
        matches!(&app.dialog, Dialog::Print(_, s) if s.ready())
    }

    /// A stand-in `lpstat`. Listing the printers (`-e`), it waits the seconds in the file
    /// `delay` beside it, if there is one, then fails if there is a file `fail`, or lists
    /// the printers in `printers`. Naming the default (`-d`, asked last) it names the first,
    /// and leaves a file `answered`.
    #[cfg(not(windows))]
    const SLOW_LPSTAT: &str = "#!/bin/sh\nd=$(dirname \"$0\")\ncase \"$1\" in\n  -e)\n    \
        [ -f \"$d/delay\" ] && sleep \"$(cat \"$d/delay\")\"\n    \
        [ -f \"$d/fail\" ] && { echo 'lpstat: scheduler is not running' >&2; exit 1; }\n    \
        cat \"$d/printers\" ;;\n  -d)\n    \
        echo \"system default destination: $(head -n 1 \"$d/printers\")\"\n    \
        touch \"$d/answered\" ;;\nesac\n";

    /// Print opens at once while the printing service is slow to list the printers, and the
    /// printers fill it in when found; a dialog closed meanwhile isn't opened again, and a
    /// failure or no printers can be tried again (BUG-11, audit A04). Run with a stand-in
    /// `lpstat`.
    #[cfg(not(windows))]
    #[test]
    fn print_opens_while_printers_are_looked_for() {
        let dir = stand_in_cups("discovery", &[("lpstat", SLOW_LPSTAT)]);
        std::fs::write(dir.join("printers"), "limarepdf-test\n").unwrap();
        rerun_with_cups(&dir, "app::tests::print_discovery_with_stand_in_cups");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The part of [`print_opens_while_printers_are_looked_for`] run with the stand-in
    /// first on PATH; without it it does nothing.
    #[cfg(not(windows))]
    #[test]
    #[ignore = "run by print_opens_while_printers_are_looked_for"]
    fn print_discovery_with_stand_in_cups() {
        use std::time::{Duration, Instant};
        let Some(dir) = std::env::var_os(STAND_IN_CUPS).map(PathBuf::from) else {
            return;
        };
        let set = |name: &str, text: Option<&str>| match text {
            Some(t) => std::fs::write(dir.join(name), t).unwrap(),
            None => {
                let _ = std::fs::remove_file(dir.join(name));
            }
        };
        // Waits for the search running to finish, and takes in its result.
        let answered = |app: &mut App, ctx: &egui::Context| {
            let marker = dir.join("answered");
            let start = Instant::now();
            while !marker.exists() {
                let late = start.elapsed() > Duration::from_secs(15);
                assert!(!late, "lpstat never answered");
                std::thread::sleep(Duration::from_millis(10));
            }
            std::fs::remove_file(&marker).unwrap();
            for _ in 0..30 {
                std::thread::sleep(Duration::from_millis(10));
                app.poll_loads(ctx);
            }
        };
        let search = |app: &App| match &app.dialog {
            Dialog::Print(_, s) => s.search.clone(),
            _ => panic!("Print isn't open"),
        };
        let done = |a: &App| match &a.dialog {
            Dialog::Print(_, s) => !matches!(s.search, dlg::Search::Running(_)),
            _ => false,
        };
        let found = |app: &App| -> Vec<String> {
            match &app.dialog {
                Dialog::Print(_, s) => s.printers.iter().map(|p| p.name.clone()).collect(),
                _ => panic!("Print isn't open"),
            }
        };
        let ctx = egui::Context::default();
        theme::install(&ctx);
        let mut app = app_with(&ctx, 1);

        // A service taking 4 s to list the printers: the dialog opens at once, looking.
        set("delay", Some("4"));
        let start = Instant::now();
        app.act(&ctx, Act::Print);
        let waited = start.elapsed();
        let quick = waited < Duration::from_secs(1);
        assert!(quick, "the window waited {waited:?} for printers");
        assert!(matches!(search(&app), dlg::Search::Running(_)));
        // Cancelled while looking: the printers found later open nothing.
        click(&mut app, &ctx, "Cancel");
        assert!(matches!(app.dialog, Dialog::None));
        answered(&mut app, &ctx);
        let reopened = !matches!(app.dialog, Dialog::None) || !app.waiting.is_empty();
        assert!(!reopened, "the printers found reopened Print");

        // Opened again: the printers fill the dialog in, the default chosen.
        set("delay", Some("1"));
        app.act(&ctx, Act::Print);
        wait_for(&mut app, &ctx, done);
        match &app.dialog {
            Dialog::Print(_, s) => {
                assert!(s.ready());
                let p = &s.printers[s.printer];
                assert!(p.name == "limarepdf-test" && p.default, "{p:?}");
            }
            _ => unreachable!(),
        }
        set("answered", None);
        click(&mut app, &ctx, "Cancel");

        // A search cancelled, and Print opened again: the first one's result, coming last,
        // leaves the new dialog as it is.
        set("delay", Some("3"));
        app.act(&ctx, Act::Print);
        click(&mut app, &ctx, "Cancel");
        // Started, the first search waits its 3 s.
        std::thread::sleep(Duration::from_millis(300));
        set("delay", None);
        app.act(&ctx, Act::Print);
        wait_for(&mut app, &ctx, done);
        assert_eq!(found(&app), ["limarepdf-test"]);
        set("answered", None);
        set("printers", Some("stale\n"));
        answered(&mut app, &ctx);
        let stale = found(&app) != ["limarepdf-test"];
        assert!(!stale, "an earlier search filled the dialog");
        set("printers", Some("limarepdf-test\n"));
        click(&mut app, &ctx, "Cancel");

        // A service that fails: why, and Try again lists the printers.
        set("fail", Some(""));
        app.act(&ctx, Act::Print);
        wait_for(&mut app, &ctx, done);
        match search(&app) {
            dlg::Search::Failed(why) => assert!(why.contains("scheduler is not running"), "{why}"),
            s => panic!("{s:?}"),
        }
        set("fail", None);
        click(&mut app, &ctx, "Try again");
        assert!(matches!(search(&app), dlg::Search::Running(_)));
        wait_for(&mut app, &ctx, done);
        assert_eq!(found(&app), ["limarepdf-test"]);
        set("answered", None);
        click(&mut app, &ctx, "Cancel");

        // No printers set up: said, and Try again finds one added meanwhile.
        set("printers", Some(""));
        app.act(&ctx, Act::Print);
        wait_for(&mut app, &ctx, done);
        assert!(found(&app).is_empty() && search(&app) == dlg::Search::Done);
        set("printers", Some("limarepdf-test\n"));
        click(&mut app, &ctx, "Try again");
        wait_for(&mut app, &ctx, done);
        assert_eq!(found(&app), ["limarepdf-test"]);
        set("answered", None);
        click(&mut app, &ctx, "Cancel");

        // The window closed while looking: Quit is asked first, the printers fill in the
        // dialog waiting, and it is back after Cancel.
        set("delay", Some("1"));
        app.act(&ctx, Act::Print);
        close_window(&mut app, &ctx);
        assert!(matches!(app.dialog, Dialog::ConfirmClose(_, true)));
        let waits_ready =
            |a: &App| matches!(a.waiting.front(), Some(Dialog::Print(_, s)) if s.ready());
        wait_for(&mut app, &ctx, waits_ready);
        click(&mut app, &ctx, "Cancel");
        assert!(print_ready(&app), "Print isn't back");
    }

    /// Add a tab of `n` blank pages, with nothing unsaved, and bring it to the front.
    fn pages_tab(app: &mut App, ctx: &egui::Context, n: usize) {
        let mut pdf = Pdf::new();
        for i in 0..n {
            pdf.insert_blank(i, 612.0, 792.0).unwrap();
        }
        assert!(app.add_tab(ctx, pdf, None, None, false));
    }

    /// Where the undo toast's Undo button is, when one is shown: the button called Undo in
    /// the window's lower half (the toolbar's is at the top).
    fn toast_undo(tree: &egui::accesskit::TreeUpdate) -> Option<egui::Pos2> {
        tree.nodes
            .iter()
            .filter(|(_, n)| n.role() == egui::accesskit::Role::Button && n.label() == Some("Undo"))
            .filter_map(|(_, n)| n.bounds())
            .find(|b| b.y0 > 400.0)
            .map(|b| egui::pos2(((b.x0 + b.x1) / 2.0) as f32, ((b.y0 + b.y1) / 2.0) as f32))
    }

    /// Whether a label in the window starts with `text`.
    fn shows(tree: &egui::accesskit::TreeUpdate, text: &str) -> bool {
        tree.nodes
            .iter()
            .any(|(_, n)| n.value().is_some_and(|v| v.starts_with(text)))
    }

    /// A notice's Undo takes back its own change only: after a page is deleted in one
    /// document, the other offers no Undo that would take back its own edit; back in the
    /// first soon after, the notice and its Undo are there again. Both files saved and read
    /// back hold what was done (BUG-02, audit A19).
    #[test]
    fn a_notice_undoes_its_own_change_in_its_own_tab() {
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        theme::install(&ctx);
        let mut app = App::with(Settings::from_file(None), Recent::from_file(None));
        pages_tab(&mut app, &ctx, 12);
        pages_tab(&mut app, &ctx, 12);
        let ty = Typist::new(&ctx);
        // In the second document the first page turned; in the first, page 1 deleted.
        ty.frame(&mut app, vec![]);
        app.act(&ctx, Act::RotR);
        app.active = 0;
        app.act(&ctx, Act::Delete);
        let tree = ty.pause(&mut app);
        assert!(shows(&tree, "Deleted page 1"), "no notice");
        assert!(toast_undo(&tree).is_some(), "no Undo in the notice");
        assert!(node(&tree, "Dismiss").is_some(), "Dismiss has no name");
        // In the second document, no Undo there takes back its turn.
        app.active = 1;
        let tree = ty.pause(&mut app);
        if let Some(undo) = toast_undo(&tree) {
            ty.click(&mut app, undo);
        }
        assert_eq!(saved_rotation(&app.tabs[1]), 90, "the turn was undone");
        assert_eq!(app.tabs[0].page_count(), 11);
        // Back in the first: the notice's Undo brings the page back.
        app.active = 0;
        let tree = ty.pause(&mut app);
        assert!(shows(&tree, "Deleted page 1"), "the notice is gone");
        ty.click(&mut app, toast_undo(&tree).expect("no Undo in the notice"));
        assert_eq!(app.tabs[0].page_count(), 12);
        let dir = std::env::temp_dir();
        let paths = ["a", "b"].map(|n| {
            dir.join(format!(
                "limarepdf-app-{}-notice-{n}.pdf",
                std::process::id()
            ))
        });
        for (i, path) in paths.iter().enumerate() {
            app.active = i;
            app.tabs[i].path = Some(path.clone());
            app.act(&ctx, Act::Save);
        }
        saves_done(&mut app, &ctx);
        let back = paths.map(|p| {
            let pdf = Pdf::open(&p, None).unwrap();
            let _ = std::fs::remove_file(&p);
            pdf
        });
        assert_eq!(back[0].page_ids().len(), 12);
        assert_eq!(back[1].page_ids().len(), 12);
        let page = back[1].doc.get_dictionary(back[1].page_ids()[0]).unwrap();
        let turn = page.get(b"Rotate").and_then(|o| o.as_i64()).unwrap_or(0);
        assert_eq!(turn, 90, "the saved turn");
    }

    /// The notice and its Undo go once its change isn't the last thing in its tab: after
    /// another edit, Undo, Undo and Redo, starting to type, or the tab closing; also after
    /// too long in another tab (BUG-02, audit A19).
    #[test]
    fn a_notice_goes_once_its_change_is_not_the_last() {
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        theme::install(&ctx);
        type Then = fn(&mut App, &egui::Context, &Typist);
        let cases: [(&str, Then); 6] = [
            ("another edit", |app, ctx, _| app.act(ctx, Act::RotR)),
            ("Undo", |app, ctx, _| app.act(ctx, Act::Undo)),
            ("Undo and Redo", |app, ctx, _| {
                app.act(ctx, Act::Undo);
                app.act(ctx, Act::Redo);
            }),
            ("typing", |app, ctx, ty| {
                app.act(ctx, Act::Tool(Tool::Text));
                let tree = ty.frame(app, vec![]);
                let (page, _) = node(&tree, "Page 1").expect("the page");
                ty.click(app, page.center());
                ty.text(app, "Walrus");
                assert!(app.tabs[1].ed.pending(), "nothing typed");
            }),
            ("closing the tab", |app, _, _| app.close_tab(1)),
            ("a long wait in another tab", |app, _, ty| {
                app.active = 0;
                ty.frame(app, vec![]);
                ty.time.set(ty.time.get() + 40.0);
                ty.frame(app, vec![]);
                app.active = 1;
            }),
        ];
        let ty = Typist::new(&ctx);
        for (case, then) in cases {
            let mut app = App::with(Settings::from_file(None), Recent::from_file(None));
            pages_tab(&mut app, &ctx, 3);
            pages_tab(&mut app, &ctx, 3);
            ty.frame(&mut app, vec![]);
            app.act(&ctx, Act::Delete);
            let tree = ty.frame(&mut app, vec![]);
            assert!(toast_undo(&tree).is_some(), "{case}: no notice");
            then(&mut app, &ctx, &ty);
            let tree = ty.frame(&mut app, vec![]);
            let tree = match toast_undo(&tree) {
                Some(_) => tree,
                None => ty.frame(&mut app, vec![]),
            };
            assert!(!shows(&tree, "Deleted page"), "{case}: the notice stayed");
            assert!(toast_undo(&tree).is_none(), "{case}: Undo still offered");
        }
    }

    /// A notice waits while its tab is behind another: back within the wait, it shows for
    /// what it had left, at least a few seconds, then goes (BUG-02).
    #[test]
    fn a_notice_waits_while_its_tab_is_behind() {
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        theme::install(&ctx);
        let mut app = App::with(Settings::from_file(None), Recent::from_file(None));
        pages_tab(&mut app, &ctx, 3);
        pages_tab(&mut app, &ctx, 3);
        let ty = Typist::new(&ctx);
        let shown = |app: &mut App| toast_undo(&ty.frame(app, vec![])).is_some();
        ty.frame(&mut app, vec![]);
        app.act(&ctx, Act::Delete);
        ty.frame(&mut app, vec![]);
        app.active = 0;
        ty.frame(&mut app, vec![]);
        ty.time.set(ty.time.get() + 20.0);
        assert!(!shown(&mut app), "shown in the other tab");
        app.active = 1;
        assert!(shown(&mut app), "gone after 20 seconds");
        ty.time.set(ty.time.get() + 2.0);
        assert!(shown(&mut app), "gone at once");
        ty.time.set(ty.time.get() + 5.0);
        assert!(!shown(&mut app), "never goes");
    }

    /// A notice about nothing done offers no Undo, which would take back the change before
    /// it (audit A19).
    #[test]
    fn a_notice_about_nothing_done_offers_no_undo() {
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        theme::install(&ctx);
        let ty = Typist::new(&ctx);
        for act in [Act::ApplyRedactions, Act::FinishSigning] {
            // A tab with a change to undo.
            let mut app = app_with(&ctx, 1);
            app.act(&ctx, act);
            let tree = ty.frame(&mut app, vec![]);
            assert!(shows(&tree, "There "), "{act:?}: no notice");
            assert!(toast_undo(&tree).is_none(), "{act:?}: Undo offered");
        }
    }

    /// Three tabs of one page, a.pdf, b.pdf and c.pdf, with nothing unsaved; their ids.
    fn named_tabs(app: &mut App, ctx: &egui::Context) -> Vec<u64> {
        for n in ["a", "b", "c"] {
            pages_tab(app, ctx, 1);
            let path = PathBuf::from(format!("/nonexistent/{n}.pdf"));
            app.tabs.last_mut().unwrap().path = Some(path);
        }
        app.tabs.iter().map(|t| t.id).collect()
    }

    /// Click the close button on the tab called `title` ("a.pdf *" when it has unsaved
    /// changes, as [`Tab::title`] says), as a user would.
    fn close_by_button(app: &mut App, ctx: &egui::Context, title: &str) {
        let tree = settle(app, ctx);
        let name = title.trim_end_matches(" *");
        let (r, _) = node(&tree, &format!("Close {name}")).expect("the close button");
        window(app, ctx, 1400.0, click_at(r.center()));
        settle(app, ctx);
    }

    /// Closing a tab behind the one in front leaves that one in front; closing the one in
    /// front brings the next, or the one before when it was the last (BUG-09, audit A02).
    #[test]
    fn closing_a_tab_keeps_the_one_in_front() {
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        theme::install(&ctx);
        let names = ["a.pdf", "b.pdf", "c.pdf"];
        // The tab in front, the one closed, and the one in front then.
        let cases = [
            (0, 2, 0),
            (2, 0, 2),
            (1, 2, 1),
            (1, 0, 1),
            (1, 1, 2),
            (2, 2, 1),
        ];
        for (front, closed, then) in cases {
            let mut app = App::with(Settings::from_file(None), Recent::from_file(None));
            let ids = named_tabs(&mut app, &ctx);
            app.active = front;
            close_by_button(&mut app, &ctx, names[closed]);
            let case = format!("{} in front, {} closed", names[front], names[closed]);
            let gone = app.tabs.iter().all(|t| t.id != ids[closed]);
            assert!(gone, "{case}: not closed");
            assert_eq!(app.tabs.len(), 2, "{case}");
            assert_eq!(app.tabs[app.active].id, ids[then], "{case}");
        }
    }

    /// Closing the only tab by its close button leaves the start screen, without the rest
    /// of the frame reaching for the tab just closed; and three tabs closed by their buttons
    /// one after another, in every order, leave none (the delta audit's DA01).
    #[test]
    fn closing_every_tab_by_its_button_leaves_the_start_screen() {
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        theme::install(&ctx);
        let mut app = App::with(Settings::from_file(None), Recent::from_file(None));
        pages_tab(&mut app, &ctx, 1);
        app.tabs[0].path = Some(PathBuf::from("/nonexistent/a.pdf"));
        close_by_button(&mut app, &ctx, "a.pdf");
        assert!(app.tabs.is_empty());
        let tree = settle(&mut app, &ctx);
        assert!(node(&tree, "Open PDF…").is_some(), "no start screen");
        let names = ["a.pdf", "b.pdf", "c.pdf"];
        let orders = [
            [0, 1, 2],
            [0, 2, 1],
            [1, 0, 2],
            [1, 2, 0],
            [2, 0, 1],
            [2, 1, 0],
        ];
        for (front, order) in (0..3).flat_map(|f| orders.iter().map(move |o| (f, o))) {
            let mut app = App::with(Settings::from_file(None), Recent::from_file(None));
            named_tabs(&mut app, &ctx);
            app.active = front;
            for (closed, &i) in order.iter().enumerate() {
                close_by_button(&mut app, &ctx, names[i]);
                assert_eq!(app.tabs.len(), 2 - closed, "{front} in front, {order:?}");
                assert!(app.active < app.tabs.len().max(1));
            }
        }
    }

    /// The only tab, unsaved, closed by its button: Cancel keeps it, Discard closes it,
    /// and Save saves it and then closes it, leaving the start screen (DA01). The save is
    /// written on its thread (ENH-35), here held as a slow disk would: the tab stays until
    /// it is written, however long that takes (macOS and Windows CI were slower than the
    /// test's frames, after 0.10.0).
    #[test]
    fn closing_the_only_unsaved_tab_by_its_button() {
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        theme::install(&ctx);
        let dir = test_dir("close-only");
        let file = dir.join("a.pdf");
        for answer in ["Cancel", "Discard", "Save"] {
            let mut app = App::with(Settings::from_file(None), Recent::from_file(None));
            unsaved_tab(&mut app, &ctx, Some(file.clone()));
            close_by_button(&mut app, &ctx, "a.pdf *");
            let asked = matches!(app.dialog, Dialog::ConfirmClose(..));
            assert!(asked, "{answer}: didn't ask");
            let (go, gate) = channel();
            app.save_gate = Some(gate);
            click(&mut app, &ctx, answer);
            settle(&mut app, &ctx);
            if answer == "Save" {
                assert_eq!(app.tabs.len(), 1, "closed before it was saved");
                go.send(()).unwrap();
                saves_done(&mut app, &ctx);
                settle(&mut app, &ctx);
            }
            assert_eq!(app.tabs.is_empty(), answer != "Cancel", "{answer}");
            assert_eq!(file.exists(), answer == "Save", "{answer}");
            if answer == "Cancel" {
                assert!(app.tabs[0].dirty, "Cancel lost the changes");
            }
        }
        assert_eq!(pages_in(&file), 2, "not saved");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// Closing an unsaved tab behind the one in front asks about it from there, and the
    /// one in front stays: Cancel changes nothing, Discard closes it, also when another
    /// file finished opening meanwhile (BUG-09, audit A02).
    #[test]
    fn closing_an_unsaved_tab_behind_asks_and_keeps_the_one_in_front() {
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        theme::install(&ctx);
        let mut app = App::with(Settings::from_file(None), Recent::from_file(None));
        let ids = named_tabs(&mut app, &ctx);
        app.tabs[2].dirty = true;
        app.active = 0;
        close_by_button(&mut app, &ctx, "c.pdf *");
        assert!(matches!(app.dialog, Dialog::ConfirmClose(..)), "didn't ask");
        assert_eq!(app.active, 0, "the tab asked about came to the front");
        click(&mut app, &ctx, "Cancel");
        assert!(matches!(app.dialog, Dialog::None));
        assert_eq!(app.tabs.len(), 3, "Cancel closed a tab");
        assert_eq!(app.active, 0, "Cancel changed the tab in front");
        close_by_button(&mut app, &ctx, "c.pdf *");
        click(&mut app, &ctx, "Discard");
        let left: Vec<u64> = app.tabs.iter().map(|t| t.id).collect();
        assert_eq!(left, ids[..2]);
        assert_eq!(app.active, 0, "Discard changed the tab in front");
        // A file finishes opening while the question is open: it comes to the front, and
        // stays there.
        app.tabs[1].dirty = true;
        close_by_button(&mut app, &ctx, "b.pdf *");
        assert!(matches!(app.dialog, Dialog::ConfirmClose(..)), "didn't ask");
        pages_tab(&mut app, &ctx, 1);
        let opened = app.tabs[2].id;
        click(&mut app, &ctx, "Discard");
        let left: Vec<u64> = app.tabs.iter().map(|t| t.id).collect();
        assert_eq!(left, [ids[0], opened]);
        assert_eq!(app.active, 1, "the opened tab left the front");
    }

    /// A value a field can't take, typed and then left by a click elsewhere, Tab, Enter or
    /// Esc, stays in the field with the keyboard once the reason is read, and nothing is
    /// kept; Tab doesn't move on. A value it can take is kept as before (BUG-37).
    #[test]
    fn a_refused_field_value_stays_in_the_field_when_left() {
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        theme::install(&ctx);
        type Leave = fn(&Typist, &mut App);
        let cases: [(&str, Leave); 4] = [
            ("a click away", |ty, app| {
                let (min, k) = ty.page(app);
                ty.click(app, min + egui::vec2(500.0, 300.0) * k);
            }),
            ("Tab", |ty, app| {
                ty.key(app, Key::Tab, egui::Modifiers::NONE);
                ty.pause(app);
            }),
            ("Enter", |ty, app| {
                ty.key(app, Key::Enter, egui::Modifiers::NONE);
                ty.pause(app);
            }),
            ("Esc", |ty, app| {
                ty.key(app, Key::Escape, egui::Modifiers::NONE);
                ty.pause(app);
            }),
        ];
        let ty = Typist::new(&ctx);
        for (case, leave) in cases {
            let mut app = App::with(Settings::from_file(None), Recent::from_file(None));
            let pdf = form_doc(&[("a", 1), ("b", 1)]);
            assert!(app.add_tab(&ctx, pdf, None, None, false));
            ty.click_field(&mut app, "a", 0);
            ty.text(&mut app, "مرحبا");
            leave(&ty, &mut app);
            let why = matches!(app.dialog, Dialog::Message(..));
            assert!(why, "{case}: no reason given");
            let t = &app.tabs[0];
            assert!(!t.dirty && saved_field(t, "a").is_empty(), "{case}: kept");
            app.dialog = Dialog::None;
            let tree = ty.pause(&mut app);
            assert_eq!(
                focused_text(&tree).as_deref(),
                Some("مرحبا"),
                "{case}: the typing was lost"
            );
            assert!(app.tabs[0].ed.pending(), "{case}: not still being typed");
            // Typed over, and left the same way: kept.
            ty.key(&mut app, Key::A, CTRL);
            ty.text(&mut app, "Walrus");
            leave(&ty, &mut app);
            assert!(matches!(app.dialog, Dialog::None), "{case}: refused");
            assert_eq!(saved_field(&app.tabs[0], "a"), "Walrus", "{case}");
            assert!(!app.tabs[0].ed.pending(), "{case}: still being typed");
        }
    }

    /// A new note or text box with nothing typed in it closes at toolbar Undo and at a page
    /// command, as the page under it may move or go (BUG-37).
    #[test]
    fn an_empty_new_box_closes_at_undo_and_page_commands() {
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        theme::install(&ctx);
        let ty = Typist::new(&ctx);
        for tool in [Tool::Note, Tool::Text] {
            for act in [Act::Undo, Act::RotR, Act::Delete] {
                let mut app = app_with(&ctx, 1);
                app.act(&ctx, Act::Tool(tool));
                let (min, k) = ty.page(&mut app);
                ty.click(&mut app, min + egui::vec2(300.0, 300.0) * k);
                let ed = &app.tabs[0].ed;
                assert!(ed.typing_on_page(), "{tool:?}: no box opened");
                assert!(!ed.pending(), "{tool:?}: something typed");
                app.act(&ctx, act);
                ty.pause(&mut app);
                let open = app.tabs[0].ed.typing_on_page();
                assert!(!open, "{tool:?}, {act:?}: the box stayed open");
            }
        }
    }

    /// Split asks once before replacing files: Cancel writes nothing and leaves the file
    /// there as it was, Replace writes every part, and with nothing in the way it splits
    /// without asking. A folder that can't be written to is reported (BUG-05, audit A07).
    #[test]
    fn split_asks_before_replacing_files() {
        let ctx = egui::Context::default();
        let mut app = App::with(Settings::from_file(None), Recent::from_file(None));
        pages_tab(&mut app, &ctx, 3);
        let dir = std::env::temp_dir().join(format!("limarepdf-app-{}-split", std::process::id()));
        let fresh = || {
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).unwrap();
        };
        let split = |app: &mut App| {
            app.dialog = Dialog::None;
            let s = SplitFiles {
                by: SplitBy::Every(1),
                dir: dir.clone(),
                stem: "doc".into(),
            };
            app.split_into(&ctx, s);
        };
        let finish = |app: &mut App| {
            for _ in 0..500 {
                app.poll_loads(&ctx);
                if !app.busy {
                    break;
                }
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
            assert!(!app.busy, "the split didn't finish");
            match &app.dialog {
                Dialog::Message(_, m) => m.clone(),
                _ => panic!("no message"),
            }
        };
        let files = |dir: &Path| {
            let mut v: Vec<String> = std::fs::read_dir(dir)
                .unwrap()
                .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
                .collect();
            v.sort();
            v
        };
        let parts = ["doc-001.pdf", "doc-002.pdf", "doc-003.pdf"];
        fresh();
        split(&mut app);
        assert!(app.busy, "it asked with nothing in the way");
        assert!(finish(&mut app).starts_with("Wrote 3 files"));
        assert_eq!(files(&dir), parts);

        fresh();
        std::fs::write(dir.join("doc-002.pdf"), "keep").unwrap();
        split(&mut app);
        assert!(!app.busy, "it started before asking");
        assert!(matches!(
            app.dialog,
            Dialog::ConfirmReplace(_, Replacing::Split(_), 1)
        ));
        click(&mut app, &ctx, "Cancel");
        assert!(matches!(app.dialog, Dialog::None));
        assert!(!app.busy, "it split on Cancel");
        assert_eq!(files(&dir), ["doc-002.pdf"]);
        assert_eq!(std::fs::read(dir.join("doc-002.pdf")).unwrap(), b"keep");

        split(&mut app);
        click(&mut app, &ctx, "Replace");
        assert!(finish(&mut app).starts_with("Wrote 3 files"));
        assert_eq!(files(&dir), parts);
        for p in parts {
            assert_eq!(Pdf::open(dir.join(p), None).unwrap().page_count(), 1, "{p}");
        }

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fresh();
            std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o555)).unwrap();
            // Root can write anywhere; there's nothing to check then.
            if std::fs::write(dir.join("probe"), "").is_err() {
                split(&mut app);
                let message = finish(&mut app);
                assert!(!message.starts_with("Wrote"), "{message}");
                assert!(matches!(&app.dialog, Dialog::Message(t, _) if t == SPLIT_JOB));
            }
            std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// A fresh folder of its own for test `name`.
    fn test_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("limarepdf-app-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// A form with the fields Name and Notes, saved at `path`.
    fn form_file(path: &Path, name: &str) {
        let mut pdf = form_doc(&[("Name", 1), ("Notes", 1)]);
        pdf.set_field("Name", limarepdf_core::FieldValue::Text(name.into()))
            .unwrap();
        pdf.save(path).unwrap();
    }

    /// The text of field `name` in the file at `path`.
    fn field_in(path: &Path, name: &str) -> String {
        let pdf = Pdf::open(path, None).unwrap();
        let f = pdf.form_fields().into_iter().find(|f| f.name == name);
        match f.map(|f| f.value) {
            Some(limarepdf_core::FieldValue::Text(t)) => t,
            v => panic!("{name}: {v:?}"),
        }
    }

    /// Opens `path` as Open does, and takes in the results until there are `tabs` tabs, or
    /// for a few tenths of a second when that many are open already.
    fn open_and_wait(app: &mut App, ctx: &egui::Context, path: &Path, tabs: usize) {
        app.spawn_open(ctx, path.to_path_buf(), None, None);
        let mut quiet = 0;
        for _ in 0..500 {
            app.poll_loads(ctx);
            if app.tabs.len() >= tabs {
                quiet += 1;
                if quiet > 30 {
                    break;
                }
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        assert!(app.tabs.len() >= tabs, "the file didn't open");
    }

    /// Sets field `name` of the active tab's document to `value`, as a change.
    fn fill(app: &mut App, ctx: &egui::Context, name: &str, value: &str) {
        let t = &mut app.tabs[app.active];
        let v = limarepdf_core::FieldValue::Text(value.into());
        assert!(t.edit(ctx, |p| p.set_field(name, v)));
    }

    /// The audit's A13 (BUG-06): the same file opened twice, its Name changed and saved in
    /// the first tab, then its Notes in the second. The saved file keeps both changes,
    /// where the second save used to put the old Name back without a word.
    #[test]
    fn opening_a_file_twice_never_loses_a_saved_change() {
        let ctx = egui::Context::default();
        let mut app = App::with(Settings::from_file(None), Recent::from_file(None));
        let dir = test_dir("twice");
        let path = dir.join("form.pdf");
        form_file(&path, "AlphaUnique");
        open_and_wait(&mut app, &ctx, &path, 1);
        let first = app.tabs[app.active].id;
        // A second tab came here before.
        open_and_wait(&mut app, &ctx, &path, 1);
        app.active = 0;
        fill(&mut app, &ctx, "Name", "First tab saved");
        app.act(&ctx, Act::Save);
        saves_done(&mut app, &ctx);
        assert_eq!(field_in(&path, "Name"), "First tab saved");
        app.active = app.tabs.len() - 1;
        fill(&mut app, &ctx, "Notes", "Second tab saved");
        app.act(&ctx, Act::Save);
        saves_done(&mut app, &ctx);
        assert_eq!(field_in(&path, "Name"), "First tab saved");
        assert_eq!(app.tabs.len(), 1, "a second tab of the file");
        assert_eq!(app.tabs[app.active].id, first);
        assert_eq!(field_in(&path, "Notes"), "Second tab saved");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// A file another app changed after it was opened isn't replaced by Save without
    /// asking first (BUG-06).
    #[test]
    fn saving_never_silently_replaces_a_file_changed_since_it_was_read() {
        let ctx = egui::Context::default();
        let mut app = App::with(Settings::from_file(None), Recent::from_file(None));
        let dir = test_dir("changed");
        let path = dir.join("form.pdf");
        form_file(&path, "AlphaUnique");
        open_and_wait(&mut app, &ctx, &path, 1);
        // Another app saves its own change.
        form_file(&path, "Other app");
        fill(&mut app, &ctx, "Notes", "Ours");
        app.act(&ctx, Act::Save);
        saves_done(&mut app, &ctx);
        assert_eq!(field_in(&path, "Name"), "Other app", "replaced");
        assert!(app.tabs[0].dirty, "the change counts as saved");
        assert_eq!(saved_field(&app.tabs[0], "Notes"), "Ours", "lost");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// Takes in background results until `done`, for a few seconds at most.
    fn wait_for(app: &mut App, ctx: &egui::Context, done: impl Fn(&App) -> bool) {
        for _ in 0..500 {
            app.poll_loads(ctx);
            if done(app) {
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        panic!("nothing came");
    }

    /// Takes in the results of the saves being written (ENH-35), once they are all written,
    /// and runs the dialogs once: what waited for them (closing, quitting) goes on there.
    pub(super) fn saves_done(app: &mut App, ctx: &egui::Context) {
        wait_for(app, ctx, |a| a.saves.is_empty());
        let mut out = ctx.run_ui(egui::RawInput::default(), |ui| app.dialogs(ui.ctx()));
        out.textures_delta.clear();
    }

    // ------------------------------------------------ batch 22: search as you type

    /// A tab whose pages hold `lines`, one line of text a page (none for "").
    fn text_tab(app: &mut App, ctx: &egui::Context, lines: &[&str]) {
        let mut pdf = Pdf::new();
        for (i, line) in lines.iter().enumerate() {
            pdf.insert_blank(i, 612.0, 792.0).unwrap();
            if !line.is_empty() {
                let font = FontChoice::default();
                pdf.add_text(i, (72.0, 100.0), line, font, 20.0, [0.0; 3])
                    .unwrap();
            }
        }
        assert!(app.add_tab(ctx, pdf, None, None, false));
    }

    /// Draw frames with `frame` until the active tab's search has all its matches.
    fn searched_with(app: &mut App, mut frame: impl FnMut(&mut App)) {
        for _ in 0..500 {
            frame(app);
            let t = &app.tabs[app.active];
            if t.search.complete(&t.query) {
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        panic!("the search didn't finish");
    }

    fn searched(app: &mut App, ctx: &egui::Context) {
        searched_with(app, |a| {
            window(a, ctx, 1400.0, vec![]);
        });
    }

    /// The window's search results: each page heading's and each row's name, in order.
    fn result_rows(tree: &egui::accesskit::TreeUpdate) -> Vec<String> {
        use egui::accesskit::Role;
        let mut rows: Vec<(f64, String)> = tree
            .nodes
            .iter()
            .filter_map(|(_, n)| {
                // A label's text is its value.
                let name = match n.role() {
                    // Not the status bar's "Page 1 of 3".
                    Role::Label => n
                        .value()
                        .filter(|v| v.starts_with("Page ") && !v.contains(" of "))?,
                    Role::Button => n
                        .label()
                        .filter(|l| l.starts_with("Page ") && l.contains(": "))?,
                    _ => return None,
                };
                Some((n.bounds().map_or(0.0, |b| b.y0), name.to_string()))
            })
            .collect();
        rows.sort_by(|a, b| a.0.total_cmp(&b.0));
        rows.into_iter().map(|r| r.1).collect()
    }

    /// ENH-02: typing in the search field shows Search results at once, but searches only
    /// once the typing pauses; then the matches come, read by page (ENH-03).
    #[test]
    fn search_waits_for_a_pause_in_the_typing() {
        let ctx = egui::Context::default();
        theme::install(&ctx);
        ctx.enable_accesskit();
        let mut app = App::with(Settings::from_file(None), Recent::from_file(None));
        text_tab(&mut app, &ctx, &["Walrus here", "", "the walrus again"]);
        let ty = Typist::new(&ctx);
        ty.pause(&mut app);
        app.act(&ctx, Act::Find);
        ty.frame(&mut app, vec![]);
        ty.frame(&mut app, vec![egui::Event::Text("walrus".into())]);
        let t = &app.tabs[0];
        assert_eq!(t.query, "walrus");
        assert!(app.side.shown() && t.side == Side::Search);
        assert!(t.search.asked.is_none() && t.hits.is_empty());
        // A tenth of a second later, still nothing.
        ty.frame(&mut app, vec![]);
        ty.frame(&mut app, vec![]);
        assert!(app.tabs[0].search.asked.is_none());
        searched_with(&mut app, |a| {
            ty.frame(a, vec![]);
        });
        let pages: Vec<usize> = app.tabs[0].hits.iter().map(|h| h.page).collect();
        assert_eq!(pages, [0, 2]);
        let tree = ty.pause(&mut app);
        assert!(shows(&tree, "2 matches"));
        assert_eq!(
            result_rows(&tree),
            [
                "Page 1",
                "Page 1: Walrus here",
                "Page 3",
                "Page 3: the walrus again"
            ]
        );
    }

    /// ENH-02 (the user's choice): typing goes to the first match at or after the page being
    /// read, not to the document's first.
    #[test]
    fn typing_goes_to_the_first_match_from_the_page_being_read() {
        let ctx = egui::Context::default();
        theme::install(&ctx);
        ctx.enable_accesskit();
        let mut app = App::with(Settings::from_file(None), Recent::from_file(None));
        text_tab(&mut app, &ctx, &["walrus", "", "", "walrus", ""]);
        let ty = Typist::new(&ctx);
        ty.pause(&mut app);
        app.tabs[0].go_to(2);
        ty.pause(&mut app);
        assert_eq!(app.tabs[0].current, 2);
        app.act(&ctx, Act::Find);
        ty.frame(&mut app, vec![]);
        ty.frame(&mut app, vec![egui::Event::Text("walrus".into())]);
        searched_with(&mut app, |a| {
            ty.frame(a, vec![]);
        });
        ty.pause(&mut app);
        let t = &app.tabs[0];
        assert_eq!((t.hit_idx, t.hits[t.hit_idx].page), (1, 3));
        assert_eq!(t.current, 3);
    }

    /// ENH-02: Enter searches at once when the typing hasn't been searched yet, then goes to
    /// the next match, Shift+Enter to the one before, round the ends; the field keeps the
    /// keyboard. ◀ and ▶ step too.
    #[test]
    fn enter_and_the_arrows_step_through_the_matches() {
        let ctx = egui::Context::default();
        theme::install(&ctx);
        ctx.enable_accesskit();
        let mut app = App::with(Settings::from_file(None), Recent::from_file(None));
        text_tab(&mut app, &ctx, &["walrus", "walrus", "walrus"]);
        let ty = Typist::new(&ctx);
        ty.pause(&mut app);
        app.act(&ctx, Act::Find);
        ty.frame(&mut app, vec![]);
        ty.frame(&mut app, vec![egui::Event::Text("walrus".into())]);
        let none = egui::Modifiers::NONE;
        ty.frame(&mut app, vec![key_with(Key::Enter, none)]);
        assert!(
            app.tabs[0].search.asked.is_some(),
            "Enter didn't search at once"
        );
        searched_with(&mut app, |a| {
            ty.frame(a, vec![]);
        });
        assert_eq!(app.tabs[0].hit_idx, 0);
        let mut steps = Vec::new();
        for shift in [false, false, false, true] {
            let m = if shift { egui::Modifiers::SHIFT } else { none };
            ty.key(&mut app, Key::Enter, m);
            steps.push(app.tabs[0].hit_idx);
        }
        assert_eq!(steps, [1, 2, 0, 2]);
        let tree = ty.pause(&mut app);
        ty.click(&mut app, middle(&tree, "Previous match"));
        assert_eq!(app.tabs[0].hit_idx, 1);
        let tree = ty.pause(&mut app);
        ty.click(&mut app, middle(&tree, "Next match"));
        assert_eq!(app.tabs[0].hit_idx, 2);
        // A row clicked is the current match.
        let tree = ty.pause(&mut app);
        ty.click(&mut app, middle(&tree, "Page 1: walrus"));
        assert_eq!(app.tabs[0].hit_idx, 0);
    }

    /// ENH-02: the matches shown are always the query's: changing the query takes the old
    /// ones away at once, and emptying the field leaves none, with no search.
    #[test]
    fn matches_of_an_older_query_are_never_shown() {
        let ctx = egui::Context::default();
        theme::install(&ctx);
        ctx.enable_accesskit();
        let mut app = App::with(Settings::from_file(None), Recent::from_file(None));
        text_tab(&mut app, &ctx, &["walrus", "walrus"]);
        let ty = Typist::new(&ctx);
        ty.pause(&mut app);
        app.act(&ctx, Act::Find);
        ty.frame(&mut app, vec![]);
        ty.frame(&mut app, vec![egui::Event::Text("walrus".into())]);
        searched_with(&mut app, |a| {
            ty.frame(a, vec![]);
        });
        assert_eq!(app.tabs[0].hits.len(), 2);
        let tree = ty.frame(&mut app, vec![egui::Event::Text("x".into())]);
        assert!(app.tabs[0].hits.is_empty());
        assert!(result_rows(&tree).is_empty());
        searched_with(&mut app, |a| {
            ty.frame(a, vec![]);
        });
        let tree = ty.pause(&mut app);
        assert!(app.tabs[0].hits.is_empty() && shows(&tree, "No matches"));
        app.tabs[0].query.clear();
        ty.frame(&mut app, vec![]);
        let t = &app.tabs[0];
        assert!(t.hits.is_empty() && t.search.asked.is_none() && t.search.typed.is_none());
    }

    /// ENH-02: an edit searches again at once, the old matches gone meanwhile, and the
    /// current match stays the one it was.
    #[test]
    fn an_edit_searches_again_and_keeps_the_current_match() {
        let ctx = egui::Context::default();
        theme::install(&ctx);
        ctx.enable_accesskit();
        let mut app = App::with(Settings::from_file(None), Recent::from_file(None));
        text_tab(&mut app, &ctx, &["walrus", "", "walrus"]);
        let t = &mut app.tabs[0];
        t.query = "walrus".into();
        t.run_search();
        searched(&mut app, &ctx);
        app.tabs[0].choose_hit(1);
        let t = &mut app.tabs[0];
        let font = FontChoice::default();
        let add = |p: &mut Pdf| p.add_text(1, (72.0, 100.0), "walrus", font, 20.0, [0.0; 3]);
        assert!(t.edit(&ctx, add));
        assert!(t.hits.is_empty());
        searched(&mut app, &ctx);
        let t = &app.tabs[0];
        let pages: Vec<usize> = t.hits.iter().map(|h| h.page).collect();
        assert_eq!(pages, [0, 1, 2]);
        assert_eq!(t.hits[t.hit_idx].page, 2);
    }

    /// Mark all for redaction marks every match, also while the search is still running.
    #[test]
    fn mark_all_for_redaction_marks_every_match_even_before_the_search_ends() {
        let ctx = egui::Context::default();
        let mut app = App::with(Settings::from_file(None), Recent::from_file(None));
        text_tab(&mut app, &ctx, &["walrus", "", "walrus", "walrus"]);
        let t = &mut app.tabs[0];
        t.query = "walrus".into();
        t.run_search();
        t.mark_results_for_redaction(&ctx, "Tester");
        let pages: Vec<usize> = t.pdf.redaction_marks().iter().map(|m| m.0).collect();
        assert_eq!(pages, [0, 2, 3]);
    }

    /// ENH-36: View > Use the document's page labels chooses what pages are called in search
    /// results, the thumbnails and a new bookmark; the status bar shows both either way.
    #[test]
    fn pages_are_called_by_their_labels_or_their_numbers() {
        use limarepdf_core::{LabelRange, LabelStyle};
        let ctx = egui::Context::default();
        theme::install(&ctx);
        ctx.enable_accesskit();
        let mut app = App::with(Settings::from_file(None), Recent::from_file(None));
        text_tab(&mut app, &ctx, &["walrus", "", "walrus"]);
        let roman = LabelRange {
            page: 0,
            style: Some(LabelStyle::LowerRoman),
            prefix: String::new(),
            first: 1,
        };
        assert!(app.tabs[0].edit(&ctx, |p| p.set_label_ranges(&[roman])));
        app.tabs[0].go_to(2);
        settle(&mut app, &ctx);
        let t = &mut app.tabs[0];
        t.query = "walrus".into();
        t.run_search();
        searched(&mut app, &ctx);
        let mut seen = Vec::new();
        for labels in [true, false, true] {
            if app.settings.page_labels != labels {
                app.act(&ctx, Act::TogglePageLabels);
            }
            assert_eq!(app.tabs[0].use_labels, labels);
            app.act(&ctx, Act::SearchView);
            let tree = settle(&mut app, &ctx);
            let rows = result_rows(&tree);
            let status = shows(&tree, "Page iii (3 of 3)");
            app.act(&ctx, Act::PagesView);
            let tree = settle(&mut app, &ctx);
            // The page shown is "Page 3" either way; its thumbnail is called as chosen.
            let named = |name: &str| {
                tree.nodes
                    .iter()
                    .filter(|(_, n)| n.label() == Some(name))
                    .count()
            };
            seen.push((rows, status, [named("Page iii"), named("Page 3")]));
        }
        let rows = |third: &str| {
            vec![
                format!("Page {}", if third == "iii" { "i" } else { "1" }),
                format!("Page {}: walrus", if third == "iii" { "i" } else { "1" }),
                format!("Page {third}"),
                format!("Page {third}: walrus"),
            ]
        };
        assert_eq!(seen[0], (rows("iii"), true, [1, 1]));
        assert_eq!(seen[1], (rows("3"), true, [0, 2]));
        assert_eq!(seen[2], seen[0]);
        // A new bookmark is called the same way.
        app.act(&ctx, Act::TogglePageLabels);
        app.act(&ctx, Act::Bookmarks);
        click_named(&mut app, &ctx, "Add bookmark");
        let titles: Vec<String> = app.tabs[0]
            .pdf
            .outline()
            .into_iter()
            .map(|i| i.title)
            .collect();
        assert_eq!(titles, ["Page 3"]);
    }

    /// One file opened by several paths at once, while the first is still being read, and
    /// again once open: one tab, which comes to the front (BUG-06).
    #[test]
    fn a_file_is_opened_once_by_any_path_to_it() {
        let ctx = egui::Context::default();
        let mut app = App::with(Settings::from_file(None), Recent::from_file(None));
        let dir = test_dir("aliases");
        let path = dir.join("form.pdf");
        form_file(&path, "AlphaUnique");
        let other = dir.join("other.pdf");
        form_file(&other, "Other");
        std::fs::create_dir(dir.join("sub")).unwrap();
        // Links are added on Unix only.
        #[cfg_attr(not(unix), allow(unused_mut))]
        let mut paths = vec![path.clone(), dir.join("sub/../form.pdf")];
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(&path, dir.join("link.pdf")).unwrap();
            std::fs::hard_link(&path, dir.join("hard.pdf")).unwrap();
            paths.extend([dir.join("link.pdf"), dir.join("hard.pdf")]);
        }
        for p in &paths {
            app.spawn_open(&ctx, p.clone(), None, None);
        }
        app.spawn_open(&ctx, other.clone(), None, None);
        wait_for(&mut app, &ctx, |a| a.opening.is_empty());
        assert_eq!(app.tabs.len(), 2);
        // The two came in either order.
        let at = |app: &App, p: &PathBuf| app.tabs.iter().position(|t| t.path.as_ref() == Some(p));
        let (i, j) = (at(&app, &path).unwrap(), at(&app, &other).unwrap());
        for p in &paths {
            app.active = j;
            app.spawn_open(&ctx, p.clone(), None, None);
            assert!(app.opening.is_empty(), "{p:?} read again");
            assert_eq!(app.active, i, "{p:?}");
        }
        app.spawn_open(&ctx, other.clone(), None, None);
        assert_eq!((app.tabs.len(), app.active), (2, j));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// Saving over a file another app changed asks: Cancel keeps both the file and the
    /// unsaved changes, and shows the notice with Reload; Replace saves; after that, Save
    /// doesn't ask again. A file only touched isn't a change (BUG-06).
    #[test]
    fn save_asks_before_replacing_a_changed_file() {
        let ctx = egui::Context::default();
        let mut app = App::with(Settings::from_file(None), Recent::from_file(None));
        let dir = test_dir("replace");
        let path = dir.join("form.pdf");
        form_file(&path, "AlphaUnique");
        open_and_wait(&mut app, &ctx, &path, 1);
        // The same bytes written again, as a sync tool might.
        let bytes = std::fs::read(&path).unwrap();
        std::fs::write(dir.join("copy"), &bytes).unwrap();
        std::fs::rename(dir.join("copy"), &path).unwrap();
        fill(&mut app, &ctx, "Notes", "Ours");
        app.act(&ctx, Act::Save);
        saves_done(&mut app, &ctx);
        assert!(matches!(app.dialog, Dialog::None), "the same bytes");
        assert_eq!(field_in(&path, "Notes"), "Ours");

        form_file(&path, "Other app");
        let theirs = std::fs::read(&path).unwrap();
        fill(&mut app, &ctx, "Notes", "Ours again");
        app.act(&ctx, Act::Save);
        assert!(app.saves.is_empty(), "it saved without asking");
        assert!(matches!(app.dialog, Dialog::ConfirmStale(..)));
        let banner = |app: &App| {
            let b = app.tabs[0].banners.iter();
            b.filter(|b| b.kind == BannerKind::ChangedOnDisk).count()
        };
        assert_eq!(banner(&app), 1, "no Reload offered");
        click(&mut app, &ctx, "Cancel");
        assert!(matches!(app.dialog, Dialog::None));
        assert_eq!(std::fs::read(&path).unwrap(), theirs);
        assert!(app.tabs[0].dirty);
        assert_eq!(saved_field(&app.tabs[0], "Notes"), "Ours again");

        app.act(&ctx, Act::Save);
        click(&mut app, &ctx, "Replace");
        saves_done(&mut app, &ctx);
        assert_eq!(field_in(&path, "Name"), "AlphaUnique");
        assert_eq!(field_in(&path, "Notes"), "Ours again");
        assert!(!app.tabs[0].dirty);
        assert_eq!(banner(&app), 0);
        fill(&mut app, &ctx, "Notes", "Third");
        app.act(&ctx, Act::Save);
        saves_done(&mut app, &ctx);
        assert!(matches!(app.dialog, Dialog::None), "its own save");
        assert_eq!(field_in(&path, "Notes"), "Third");

        // A file deleted meanwhile is saved again without asking.
        std::fs::remove_file(&path).unwrap();
        fill(&mut app, &ctx, "Notes", "Fourth");
        app.act(&ctx, Act::Save);
        saves_done(&mut app, &ctx);
        assert!(matches!(app.dialog, Dialog::None));
        assert_eq!(field_in(&path, "Notes"), "Fourth");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// Back in the window after another app changed a file open here, its tab says so,
    /// once; Reload asks first when there are unsaved changes, then shows the file as it
    /// now is, in the same place among the tabs (BUG-06).
    #[test]
    fn a_file_changed_elsewhere_is_noticed_and_reloaded() {
        let ctx = egui::Context::default();
        theme::install(&ctx);
        ctx.enable_accesskit();
        let mut app = App::with(Settings::from_file(None), Recent::from_file(None));
        let dir = test_dir("reload");
        let path = dir.join("form.pdf");
        form_file(&path, "AlphaUnique");
        let other = dir.join("other.pdf");
        form_file(&other, "Other");
        open_and_wait(&mut app, &ctx, &path, 1);
        open_and_wait(&mut app, &ctx, &other, 2);
        app.active = 0;
        let changed = |app: &App| {
            let mut b = app.tabs[0].banners.iter();
            b.any(|b| b.kind == BannerKind::ChangedOnDisk)
        };
        // Back from another app with nothing changed.
        app.focused = false;
        window(&mut app, &ctx, 1000.0, vec![]);
        assert!(app.focused);
        std::thread::sleep(std::time::Duration::from_millis(200));
        app.poll_loads(&ctx);
        assert!(!changed(&app));

        form_file(&path, "Other app");
        app.focused = false;
        window(&mut app, &ctx, 1000.0, vec![]);
        wait_for(&mut app, &ctx, changed);
        let mut others = app.tabs[1].banners.iter();
        assert!(!others.any(|b| b.kind == BannerKind::ChangedOnDisk));
        // Dismissed, it doesn't come back for the same change.
        app.tabs[0].drop_banner(BannerKind::ChangedOnDisk);
        app.check_disk(&ctx, None);
        std::thread::sleep(std::time::Duration::from_millis(200));
        app.poll_loads(&ctx);
        assert!(!changed(&app));

        fill(&mut app, &ctx, "Notes", "Ours");
        app.act(&ctx, Act::Reload);
        assert!(matches!(app.dialog, Dialog::ConfirmReload(_)));
        click(&mut app, &ctx, "Cancel");
        assert_eq!(saved_field(&app.tabs[0], "Notes"), "Ours");
        app.act(&ctx, Act::Reload);
        let old = app.tabs[0].id;
        click(&mut app, &ctx, "Discard and reload");
        wait_for(&mut app, &ctx, |a| a.tabs[0].id != old);
        assert_eq!((app.tabs.len(), app.active), (2, 0));
        let t = &app.tabs[0];
        assert_eq!(saved_field(t, "Name"), "Other app");
        assert_eq!(saved_field(t, "Notes"), "");
        assert!(!t.dirty && !changed(&app));
        // Its Save replaces the file without asking: it is what was read.
        fill(&mut app, &ctx, "Notes", "After reload");
        app.act(&ctx, Act::Save);
        saves_done(&mut app, &ctx);
        assert!(matches!(app.dialog, Dialog::None));
        assert_eq!(field_in(&path, "Notes"), "After reload");

        // Unchanged, it reloads without asking; changed while being read, it doesn't.
        let old = app.tabs[0].id;
        app.act(&ctx, Act::Reload);
        assert!(matches!(app.dialog, Dialog::None));
        wait_for(&mut app, &ctx, |a| a.tabs[0].id != old);
        app.act(&ctx, Act::Reload);
        fill(&mut app, &ctx, "Notes", "Typed meanwhile");
        wait_for(&mut app, &ctx, |a| a.opening.is_empty());
        assert!(matches!(&app.dialog, Dialog::Message(t, _) if t == "Not reloaded"));
        assert_eq!(saved_field(&app.tabs[0], "Notes"), "Typed meanwhile");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// Nothing is saved over the file of another tab, by any path to it; a tab's own file
    /// by another path to it is still checked for changes made elsewhere (BUG-06).
    #[test]
    fn nothing_is_saved_over_another_tabs_file() {
        let ctx = egui::Context::default();
        let mut app = App::with(Settings::from_file(None), Recent::from_file(None));
        let dir = test_dir("other-tab");
        let (a, b) = (dir.join("a.pdf"), dir.join("b.pdf"));
        form_file(&a, "A");
        form_file(&b, "B");
        open_and_wait(&mut app, &ctx, &a, 1);
        open_and_wait(&mut app, &ctx, &b, 2);
        let (ta, tb) = (app.tabs[0].id, app.tabs[1].id);
        let alias = dir.join("sub/../a.pdf");
        std::fs::create_dir(dir.join("sub")).unwrap();
        let why = app.in_other_tab(&alias, Some(tb)).expect("refused");
        assert!(why.contains("\"a.pdf\" is open in another tab"), "{why}");
        assert!(app.in_other_tab(&alias, Some(ta)).is_none(), "its own file");
        assert!(app.in_other_tab(&dir.join("c.pdf"), Some(tb)).is_none());
        assert!(app.in_other_tab(&b, None).is_some());
        assert!(app.stale(0, &alias).is_none());
        form_file(&a, "Changed");
        assert!(app.stale(0, &alias).is_some(), "by another path");
        assert!(app.stale(1, &alias).is_none(), "not tab b's file");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// Reload reads the file again in the background. Typing begun while it reads (the
    /// document title, a page label's prefix, new text on the page) or a change made
    /// meanwhile keeps the tab as it is, saying so, and the typing goes in as before. With
    /// nothing done meanwhile the file replaces the tab, also after the user agreed to
    /// lose the changes made before asking (the delta audit's DA02). The read's result is
    /// delivered by the test, after the typing, as Reload asked for it.
    #[test]
    fn reload_keeps_what_is_typed_while_the_file_is_read() {
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        theme::install(&ctx);
        let dir = test_dir("reload-typing");
        let file = dir.join("a.pdf");
        pages_file(&file, 100.0, 3, None);
        let mut app = App::with(Settings::from_file(None), Recent::from_file(None));
        let note = typing_doc(&mut app, &ctx);
        app.tabs[0].path = Some(file.clone());
        let ty = Typist::new(&ctx);
        // What Reload records when it starts (`App::reload`).
        let asked = |app: &App| (app.tabs[0].id, app.tabs[0].revision);
        let deliver = |app: &mut App, into| {
            let pdf = Pdf::from_bytes(&std::fs::read(&file).unwrap(), None).unwrap();
            let (path, version) = (file.clone(), disk::version(&file).unwrap());
            let (page, signed, into) = (None, None, Some(into));
            app.tx
                .send(Loaded::Ok {
                    path,
                    pdf,
                    page,
                    signed,
                    version,
                    into,
                })
                .unwrap();
            app.poll_loads(&ctx);
        };
        for case in ["document title", "label prefix", "new text", "a change"] {
            let before = asked(&app);
            let tab = app.tabs[0].id;
            match case {
                "label prefix" => {
                    app.reveal_labels();
                    let tree = ty.pause(&mut app);
                    ty.click(&mut app, middle(&tree, "Prefix"));
                    ty.text(&mut app, "A-");
                }
                "a change" => {
                    assert!(app.tabs[0].edit(&ctx, |p| p.insert_blank(1, 300.0, 500.0)));
                }
                _ => {
                    start_typing(&mut app, &ty, case, note);
                    ty.key(&mut app, Key::End, egui::Modifiers::NONE);
                    ty.text(&mut app, " Walrus");
                }
            }
            let waiting = case == "a change" || app.tabs[0].revision == before.1;
            assert!(waiting, "{case}: kept already");
            deliver(&mut app, before);
            assert_eq!(app.tabs[0].id, tab, "{case}: reloaded over it");
            assert_eq!(message(&app).0, "Not reloaded", "{case}");
            app.dialog = Dialog::None;
            let kept = app.tabs[0].keep_typing(&ctx);
            assert!(kept, "{case}: the typing wasn't kept");
            let pdf = &app.tabs[0].pdf;
            match case {
                "document title" => assert_eq!(pdf.info().title.as_deref(), Some("Seal Walrus")),
                "label prefix" => assert_eq!(pdf.label_ranges()[0].prefix, "A-"),
                "new text" => assert!(saved_pdf(&app.tabs[0]).page_text(0).contains("Walrus")),
                _ => assert_eq!(pdf.page_count(), 2),
            }
        }
        // Nothing done while it reads: the file replaces the tab and its changes.
        assert!(app.tabs[0].unsaved());
        let before = asked(&app);
        deliver(&mut app, before);
        assert_ne!(app.tabs[0].id, before.0, "not reloaded");
        assert_eq!(app.tabs[0].page_count(), 3);
        assert!(!app.tabs[0].unsaved());
        // Through the question, reading the file for real: Discard and reload.
        assert!(app.tabs[0].edit(&ctx, |p| p.insert_blank(1, 300.0, 500.0)));
        app.act(&ctx, Act::Reload);
        assert!(matches!(app.dialog, Dialog::ConfirmReload(_)));
        click(&mut app, &ctx, "Discard and reload");
        wait_for(&mut app, &ctx, |a| !a.tabs[0].unsaved());
        assert_eq!(app.tabs[0].page_count(), 3);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// A document of `pages` blank pages `width` points wide, saved at `path`, locked with
    /// `password` when given.
    fn pages_file(path: &Path, width: f32, pages: usize, password: Option<&str>) {
        let mut pdf = Pdf::new();
        for i in 0..pages {
            pdf.insert_blank(i, width, 500.0).unwrap();
        }
        pdf.set_encryption(password.map(|p| Encryption {
            user_password: p.into(),
            owner_password: format!("{p}-owner"),
            cipher: Cipher::Aes128,
            restrictions: Restrictions::default(),
        }));
        pdf.save(path).unwrap();
    }

    /// The file whose password is asked for, if one is.
    fn asked_password(app: &App) -> Option<PathBuf> {
        match &app.dialog {
            Dialog::Password(s, ..) => Some(s.path.clone()),
            _ => None,
        }
    }

    /// Puts `password` in the password asked for, and clicks `button`.
    fn answer(app: &mut App, ctx: &egui::Context, password: &str, button: &str) {
        match &mut app.dialog {
            Dialog::Password(s, ..) => s.input = password.into(),
            _ => panic!("no password asked for"),
        }
        click(app, ctx, button);
    }

    /// The widths of the pages of tab `i`.
    fn widths(app: &App, i: usize) -> Vec<f32> {
        let p = &app.tabs[i].pdf;
        (0..p.page_count())
            .map(|n| p.page_size(n).unwrap().0)
            .collect()
    }

    /// The audit's A01 (BUG-07): two protected files opened together each ask for their
    /// password in turn, a wrong password asks again before anything else, and both open.
    #[test]
    fn every_protected_file_opened_together_asks_for_its_password() {
        let ctx = egui::Context::default();
        let mut app = App::with(Settings::from_file(None), Recent::from_file(None));
        let dir = test_dir("two-locked");
        let (a, b) = (dir.join("a.pdf"), dir.join("b.pdf"));
        pages_file(&a, 100.0, 1, Some("walrus-a"));
        pages_file(&b, 200.0, 1, Some("walrus-b"));
        let pass = |p: &Path| if p == a { "walrus-a" } else { "walrus-b" };
        app.spawn_open(&ctx, a.clone(), None, None);
        app.spawn_open(&ctx, b.clone(), None, None);
        wait_for(&mut app, &ctx, |a| a.opening.is_empty());
        let first = asked_password(&app).expect("no password asked for");
        answer(&mut app, &ctx, "wrong", "Open");
        wait_for(&mut app, &ctx, |a| a.opening.is_empty());
        assert_eq!(
            asked_password(&app).as_ref(),
            Some(&first),
            "not asked again"
        );
        assert!(matches!(&app.dialog, Dialog::Password(s, ..) if s.error.is_some()));
        answer(&mut app, &ctx, pass(&first), "Open");
        wait_for(&mut app, &ctx, |a| a.tabs.len() == 1);
        let second = asked_password(&app).expect("the second file's request was lost");
        assert_ne!(second, first);
        answer(&mut app, &ctx, pass(&second), "Open");
        wait_for(&mut app, &ctx, |a| a.tabs.len() == 2);
        let mut opened = [widths(&app, 0)[0], widths(&app, 1)[0]];
        opened.sort_by(f32::total_cmp);
        assert_eq!(opened, [100.0, 200.0]);
        assert!(matches!(app.dialog, Dialog::None));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// BUG-28: files sent to open (Finder's Open With and double-click on macOS) open as the
    /// command line's do, sent before the window exists or while it runs: several at once,
    /// names with spaces and accents, one open already (its tab comes to the front), and a
    /// protected one, whose password is asked for after the message about one not found.
    #[test]
    fn files_sent_to_open_open_as_the_command_lines_do() {
        let ctx = egui::Context::default();
        let mut app = App::with(Settings::from_file(None), Recent::from_file(None));
        let dir = test_dir("sent");
        let (a, b) = (dir.join("a letter.pdf"), dir.join("Ünïcødé €.pdf"));
        let locked = dir.join("locked one.pdf");
        pages_file(&a, 100.0, 1, None);
        pages_file(&b, 200.0, 1, None);
        pages_file(&locked, 300.0, 1, Some("walrus"));
        // Sent before the window exists: they wait for its first frame.
        app.incoming.hand_in(vec![a.clone(), b.clone()], 0);
        app.incoming.wake(&ctx);
        app.open_incoming(&ctx);
        wait_for(&mut app, &ctx, |a| a.tabs.len() == 2);
        let mut opened = [widths(&app, 0)[0], widths(&app, 1)[0]];
        opened.sort_by(f32::total_cmp);
        assert_eq!(opened, [100.0, 200.0]);

        // Sent while it runs.
        let first = (0..2).find(|&i| widths(&app, i)[0] == 100.0).unwrap();
        app.active = 1 - first;
        app.incoming.hand_in(vec![a.clone(), locked.clone()], 1);
        app.open_incoming(&ctx);
        assert_eq!(
            app.active, first,
            "the open file's tab didn't come to the front"
        );
        assert_eq!(
            message(&app),
            (
                CANNOT_OPEN,
                "A file sent to LimarePDF to open couldn't be found."
            )
        );
        app.dialog = Dialog::None;
        app.next_dialog();
        wait_for(&mut app, &ctx, |a| asked_password(a).is_some());
        assert_eq!(asked_password(&app), Some(locked.clone()));
        answer(&mut app, &ctx, "walrus", "Open");
        wait_for(&mut app, &ctx, |a| a.tabs.len() == 3);
        assert!(matches!(app.dialog, Dialog::None));
        assert_eq!(app.incoming.take(), (Vec::new(), 0));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// BUG-07: what comes in while a password is being typed waits for it, the typing kept;
    /// Cancel leaves only its own file unopened; files that can't be opened are told about
    /// in one message.
    #[test]
    fn results_wait_for_the_password_being_typed() {
        let ctx = egui::Context::default();
        let mut app = App::with(Settings::from_file(None), Recent::from_file(None));
        let dir = test_dir("mixed");
        let (locked, plain) = (dir.join("locked.pdf"), dir.join("plain.pdf"));
        let (bad1, bad2) = (dir.join("bad1.pdf"), dir.join("bad2.pdf"));
        pages_file(&locked, 100.0, 1, Some("walrus"));
        pages_file(&plain, 200.0, 1, None);
        std::fs::write(&bad1, "not a PDF").unwrap();
        std::fs::write(&bad2, "not a PDF either").unwrap();
        app.spawn_open(&ctx, locked.clone(), None, None);
        wait_for(&mut app, &ctx, |a| asked_password(a).is_some());
        match &mut app.dialog {
            Dialog::Password(s, ..) => s.input = "wal".into(),
            _ => unreachable!(),
        }
        for p in [&bad1, &plain, &bad2] {
            app.spawn_open(&ctx, p.clone(), None, None);
        }
        wait_for(&mut app, &ctx, |a| {
            a.opening.is_empty() && a.tabs.len() == 1
        });
        match &app.dialog {
            Dialog::Password(s, ..) => assert_eq!((&s.path, s.input.as_str()), (&locked, "wal")),
            _ => panic!("the password being typed was replaced"),
        }
        click(&mut app, &ctx, "Cancel");
        let (title, text) = message(&app);
        assert_eq!(title, "Cannot open files");
        assert!(
            text.contains("bad1.pdf") && text.contains("bad2.pdf"),
            "{text}"
        );
        click(&mut app, &ctx, "OK");
        assert!(matches!(app.dialog, Dialog::None));
        assert_eq!(app.tabs.len(), 1);
        assert_eq!(app.tabs[0].path.as_ref(), Some(&plain));
        // Cancelled, it asks again when opened again.
        app.spawn_open(&ctx, locked.clone(), None, None);
        wait_for(&mut app, &ctx, |a| asked_password(a).is_some());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// BUG-07: a password asked for to reload a tab and one to open a file each wait their
    /// turn and go where they were asked for, the page asked for kept.
    #[test]
    fn reload_and_open_passwords_each_go_to_their_own_place() {
        let ctx = egui::Context::default();
        let mut app = App::with(Settings::from_file(None), Recent::from_file(None));
        let dir = test_dir("reload-locked");
        let (a, b) = (dir.join("a.pdf"), dir.join("b.pdf"));
        pages_file(&a, 100.0, 1, None);
        pages_file(&b, 200.0, 3, Some("walrus-b"));
        let pass = |p: &Path| if p == a { "walrus-a" } else { "walrus-b" };
        open_and_wait(&mut app, &ctx, &a, 1);
        let old = app.tabs[0].id;
        // Another app puts a password on it.
        pages_file(&a, 150.0, 1, Some("walrus-a"));
        app.act(&ctx, Act::Reload);
        app.spawn_open(&ctx, b.clone(), None, Some(3));
        wait_for(&mut app, &ctx, |a| a.opening.is_empty());
        for _ in 0..2 {
            let p = asked_password(&app).expect("a request was lost");
            answer(&mut app, &ctx, pass(&p), "Open");
            wait_for(&mut app, &ctx, |a| a.opening.is_empty());
        }
        assert!(matches!(app.dialog, Dialog::None));
        assert_eq!(app.tabs.len(), 2);
        assert_ne!(app.tabs[0].id, old, "not reloaded");
        assert_eq!(widths(&app, 0), [150.0]);
        assert_eq!(widths(&app, 1), [200.0; 3]);
        assert_eq!(app.tabs[1].current, 2, "the page asked for");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// The audit's A09 (BUG-08): Enter, pressed and released as a keyboard does, answers the
    /// password field (a wrong password asks again) and the name field; Esc cancels; Tab
    /// goes from the field to the buttons.
    #[test]
    fn enter_in_the_password_and_name_fields_answers_them() {
        let ctx = egui::Context::default();
        theme::install(&ctx);
        ctx.enable_accesskit();
        let mut app = App::with(Settings::from_file(None), Recent::from_file(None));
        let dir = test_dir("enter");
        let (locked, other) = (dir.join("locked.pdf"), dir.join("other.pdf"));
        pages_file(&locked, 100.0, 1, Some("walrus"));
        pages_file(&other, 200.0, 1, Some("walrus"));
        let ty = Typist::new(&ctx);
        let release = |key| egui::Event::Key {
            key,
            physical_key: None,
            pressed: false,
            repeat: false,
            modifiers: egui::Modifiers::NONE,
        };
        let press = |app: &mut App, key| {
            ty.frame(app, vec![key_with(key, egui::Modifiers::NONE)]);
            ty.frame(app, vec![release(key)])
        };
        let until = |app: &mut App, done: &dyn Fn(&App) -> bool| {
            for _ in 0..300 {
                if done(app) {
                    ty.pause(app);
                    return;
                }
                ty.frame(app, vec![]);
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
            panic!("nothing came");
        };
        app.spawn_open(&ctx, locked.clone(), None, None);
        until(&mut app, &|a| asked_password(a).is_some());
        ty.text(&mut app, "wrong");
        press(&mut app, Key::Enter);
        until(
            &mut app,
            &|a| matches!(&a.dialog, Dialog::Password(s, ..) if s.error.is_some()),
        );
        ty.text(&mut app, "walrus");
        press(&mut app, Key::Enter);
        until(&mut app, &|a| a.tabs.len() == 1);
        assert!(matches!(app.dialog, Dialog::None));

        app.spawn_open(&ctx, other.clone(), None, None);
        until(&mut app, &|a| asked_password(a).is_some());
        let mut reached = Vec::new();
        for _ in 0..2 {
            let tree = press(&mut app, Key::Tab);
            let focus = tree.nodes.iter().find(|(id, _)| *id == tree.focus);
            reached.extend(focus.and_then(|(_, n)| n.label()).map(str::to_string));
        }
        let mut want = ["Open", "Cancel"];
        if !dlg::ACTION_FIRST {
            want.reverse();
        }
        assert_eq!(reached, want);
        ty.text(&mut app, "walrus");
        press(&mut app, Key::Escape);
        ty.pause(&mut app);
        assert!(matches!(app.dialog, Dialog::None), "Esc");
        assert!(
            app.opening.is_empty() && app.tabs.len() == 1,
            "Esc opened it"
        );

        app.act(&ctx, Act::CommentName);
        ty.pause(&mut app);
        ty.key(&mut app, Key::A, egui::Modifiers::COMMAND);
        ty.text(&mut app, "Pat Example");
        press(&mut app, Key::Enter);
        ty.pause(&mut app);
        assert!(
            matches!(app.dialog, Dialog::None),
            "Enter left the name open"
        );
        assert_eq!(app.settings.author.as_deref(), Some("Pat Example"));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// Tab goes round a dialog's controls, from the last straight back to the first, and
    /// Shift+Tab the other way (PB-15): egui alone stops once on the window after the last
    /// control and then skips the first.
    #[test]
    fn tab_goes_round_a_dialog_s_controls() {
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        theme::install(&ctx);
        let mut app = app_with(&ctx, 1);
        let ty = Typist::new(&ctx);
        app.act(&ctx, Act::CommentName);
        ty.pause(&mut app);
        let mut walk = |modifiers| {
            let mut reached = Vec::new();
            for _ in 0..4 {
                let tree = ty.key(&mut app, Key::Tab, modifiers);
                let focus = tree.nodes.iter().find(|(id, _)| *id == tree.focus);
                let name = focus.and_then(|(_, n)| n.label()).unwrap_or("(none)");
                reached.push(name.to_string());
            }
            reached
        };
        let (mut ok, mut cancel) = ("Save", "Cancel");
        if !dlg::ACTION_FIRST {
            (ok, cancel) = (cancel, ok);
        }
        // From the name, which has the keyboard when the dialog opens.
        assert_eq!(walk(egui::Modifiers::NONE), [ok, cancel, "Name", ok]);
        assert_eq!(walk(egui::Modifiers::SHIFT), ["Name", cancel, ok, "Name"]);
    }

    /// The window, drawn once as the system asks it to close.
    fn close_window(app: &mut App, ctx: &egui::Context) {
        let mut raw = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(1000.0, TEST_HEIGHT),
            )),
            ..Default::default()
        };
        let viewport = raw.viewports.entry(egui::ViewportId::ROOT).or_default();
        viewport.events.push(egui::ViewportEvent::Close);
        let mut frame = eframe::Frame::_new_kittest();
        let mut out = ctx.run_ui(raw, |ui| eframe::App::ui(app, ui, &mut frame));
        out.textures_delta.clear();
    }

    /// BUG-07: closing the window with unsaved changes asks first, and a password being
    /// typed comes back, typing kept, after Cancel.
    #[test]
    fn closing_the_window_keeps_the_question_it_interrupts() {
        let ctx = egui::Context::default();
        theme::install(&ctx);
        let mut app = app_with(&ctx, 1);
        let dir = test_dir("close-asking");
        let locked = dir.join("locked.pdf");
        pages_file(&locked, 100.0, 1, Some("walrus"));
        app.spawn_open(&ctx, locked.clone(), None, None);
        wait_for(&mut app, &ctx, |a| asked_password(a).is_some());
        match &mut app.dialog {
            Dialog::Password(s, ..) => s.input = "wal".into(),
            _ => unreachable!(),
        }
        close_window(&mut app, &ctx);
        assert!(matches!(app.dialog, Dialog::ConfirmClose(_, true)));
        click(&mut app, &ctx, "Cancel");
        assert!(!app.quit_ok);
        match &app.dialog {
            Dialog::Password(s, ..) => assert_eq!(s.input, "wal"),
            _ => panic!("the password being typed was lost"),
        }
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// Add a tab of one blank page, its file at `path` (not written yet), with a page
    /// added and not saved, and bring it to the front.
    fn unsaved_tab(app: &mut App, ctx: &egui::Context, path: Option<PathBuf>) -> u64 {
        pages_tab(app, ctx, 1);
        let t = app.tabs.last_mut().unwrap();
        t.path = path;
        assert!(t.edit(ctx, |p| p.insert_blank(1, 300.0, 500.0)));
        t.id
    }

    /// The number of pages in the PDF at `path`.
    fn pages_in(path: &Path) -> usize {
        Pdf::from_bytes(&std::fs::read(path).unwrap(), None)
            .unwrap()
            .page_count()
    }

    fn ids(app: &App) -> Vec<u64> {
        app.tabs.iter().map(|t| t.id).collect()
    }

    /// ENH-01: Save when closing a tab saves it, then closes it, and the tab in front
    /// stays; a document with no file yet is saved as one, and cancelling the chooser
    /// keeps it open with its changes.
    #[test]
    fn closing_with_save_saves_the_document_then_closes_it() {
        let ctx = egui::Context::default();
        let mut app = App::with(Settings::from_file(None), Recent::from_file(None));
        let dir = test_dir("close-save");
        let (a, c) = (dir.join("a.pdf"), dir.join("c.pdf"));
        let front = unsaved_tab(&mut app, &ctx, Some(a.clone()));
        let new = unsaved_tab(&mut app, &ctx, None);
        let behind = unsaved_tab(&mut app, &ctx, Some(c.clone()));
        app.active = 0;
        // Cancel changes nothing.
        app.close_asking(2);
        click(&mut app, &ctx, "Cancel");
        assert_eq!(ids(&app), [front, new, behind]);
        assert!(!c.exists() && app.closing.is_none());
        // Save, about a tab behind the one in front.
        app.close_asking(2);
        click(&mut app, &ctx, "Save");
        saves_done(&mut app, &ctx);
        assert_eq!(pages_in(&c), 2, "not saved");
        assert_eq!(ids(&app), [front, new], "the saved tab wasn't closed");
        assert_eq!(app.tabs[app.active].id, front, "the tab in front changed");
        // A new document: Save as…, with the chooser cancelled, then a file chosen.
        app.close_asking(1);
        click(&mut app, &ctx, "Save as…");
        assert_eq!(ids(&app), [front, new], "closed without saving");
        assert!(app.tabs[1].dirty && app.closing.is_none());
        SAVE_AS.with(|s| s.set(Some(dir.join("new"))));
        app.close_asking(1);
        click(&mut app, &ctx, "Save as…");
        saves_done(&mut app, &ctx);
        assert_eq!(pages_in(&dir.join("new.pdf")), 2, "not saved as a file");
        assert_eq!(ids(&app), [front]);
        assert!(app.tabs[0].dirty && !a.exists(), "the tab left changed");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// ENH-01: a save that fails, or is cancelled at one of its questions, keeps the
    /// document open with its changes, and nothing closes later on its own.
    #[test]
    fn closing_keeps_a_document_whose_save_fails_or_is_cancelled() {
        let ctx = egui::Context::default();
        let mut app = App::with(Settings::from_file(None), Recent::from_file(None));
        let dir = test_dir("close-kept");
        let kept = |app: &App, id| app.tabs.iter().any(|t| t.id == id && t.dirty);
        // Written into a folder that isn't there: Save failed.
        let failing = unsaved_tab(&mut app, &ctx, Some(dir.join("gone").join("a.pdf")));
        app.close_asking(0);
        click(&mut app, &ctx, "Save");
        saves_done(&mut app, &ctx);
        assert!(matches!(&app.dialog, Dialog::Message(t, _) if t == "Save failed"));
        click(&mut app, &ctx, "OK");
        assert!(
            kept(&app, failing),
            "a document that failed to save was closed"
        );
        assert!(app.closing.is_none());
        // Repaired: Cancel at Replace the damaged original?
        let repaired = unsaved_tab(&mut app, &ctx, Some(dir.join("repaired.pdf")));
        app.tabs[1].repaired = true;
        app.close_asking(1);
        click(&mut app, &ctx, "Save");
        assert!(matches!(app.dialog, Dialog::ConfirmSaveRepaired(_)));
        click(&mut app, &ctx, "Cancel");
        assert!(kept(&app, repaired) && app.closing.is_none());
        // Changed by another app: Cancel at Replace the changed file?, and Save as… there
        // with the chooser cancelled.
        let changed = dir.join("changed.pdf");
        pages_file(&changed, 100.0, 1, None);
        open_and_wait(&mut app, &ctx, &changed, 3);
        let stale = app.tabs[2].id;
        assert!(app.tabs[2].edit(&ctx, |p| p.insert_blank(1, 300.0, 500.0)));
        pages_file(&changed, 200.0, 3, None);
        for answer in ["Cancel", "Save as…"] {
            app.close_asking(2);
            click(&mut app, &ctx, "Save");
            assert!(matches!(app.dialog, Dialog::ConfirmStale(..)));
            click(&mut app, &ctx, answer);
            assert!(kept(&app, stale), "closed after {answer}");
            assert!(app.closing.is_none() && pages_in(&changed) == 3);
        }
        // Nothing closes later either.
        for _ in 0..3 {
            let out = ctx.run_ui(egui::RawInput::default(), |ui| app.dialogs(ui.ctx()));
            drop(out);
        }
        assert_eq!(ids(&app), [failing, repaired, stale]);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// ENH-01: quitting with several documents unsaved asks about each in turn; Cancel part
    /// way quits nothing and closes nothing, the documents saved stay saved and the rest
    /// keep their changes; once every one is saved or discarded, the app quits. Discard all
    /// quits at once.
    #[test]
    fn quitting_asks_about_each_unsaved_document_in_turn() {
        let ctx = egui::Context::default();
        let mut app = App::with(Settings::from_file(None), Recent::from_file(None));
        let dir = test_dir("quit-each");
        let files: Vec<PathBuf> = ["a", "b", "c"]
            .iter()
            .map(|n| dir.join(format!("{n}.pdf")))
            .collect();
        pages_tab(&mut app, &ctx, 1);
        for f in &files {
            unsaved_tab(&mut app, &ctx, Some(f.clone()));
        }
        let all = ids(&app);
        let asked = |app: &App| match &app.dialog {
            Dialog::ConfirmClose(a, true) => app.tabs.iter().position(|t| t.id == a.tab),
            _ => None,
        };
        app.act(&ctx, Act::Quit);
        assert!(matches!(app.dialog, Dialog::ConfirmQuit));
        click(&mut app, &ctx, "Review…");
        assert_eq!(asked(&app), Some(1));
        click(&mut app, &ctx, "Save");
        saves_done(&mut app, &ctx);
        assert_eq!(pages_in(&files[0]), 2, "not saved");
        assert_eq!(asked(&app), Some(2), "didn't go on to the next");
        click(&mut app, &ctx, "Cancel");
        assert!(!app.quit_ok && app.closing.is_none());
        assert_eq!(ids(&app), all, "a document was closed");
        let dirty: Vec<bool> = app.tabs.iter().map(|t| t.dirty).collect();
        assert_eq!(dirty, [false, false, true, true]);
        assert!(!files[1].exists() && !files[2].exists());
        // Again: the first is saved now; Discard the second, Save the third, and quit.
        app.act(&ctx, Act::Quit);
        click(&mut app, &ctx, "Review…");
        assert_eq!(asked(&app), Some(2));
        click(&mut app, &ctx, "Discard");
        assert_eq!(asked(&app), Some(3));
        assert!(!app.quit_ok, "quit with a document not yet asked about");
        click(&mut app, &ctx, "Save");
        saves_done(&mut app, &ctx);
        assert_eq!(pages_in(&files[2]), 2);
        assert!(app.quit_ok, "didn't quit");
        assert!(!files[1].exists(), "the discarded document was saved");
        // Discard all.
        let mut app = App::with(Settings::from_file(None), Recent::from_file(None));
        unsaved_tab(&mut app, &ctx, None);
        unsaved_tab(&mut app, &ctx, None);
        app.act(&ctx, Act::Quit);
        click(&mut app, &ctx, "Discard all");
        assert!(app.quit_ok);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// ENH-01: a save failing while quitting stops quitting, with every document open.
    #[test]
    fn quitting_stops_when_a_save_fails() {
        let ctx = egui::Context::default();
        theme::install(&ctx);
        let mut app = App::with(Settings::from_file(None), Recent::from_file(None));
        let dir = test_dir("quit-fails");
        let (a, gone) = (dir.join("a.pdf"), dir.join("gone").join("b.pdf"));
        unsaved_tab(&mut app, &ctx, Some(a.clone()));
        unsaved_tab(&mut app, &ctx, Some(gone));
        let all = ids(&app);
        close_window(&mut app, &ctx);
        click(&mut app, &ctx, "Review…");
        click(&mut app, &ctx, "Save");
        saves_done(&mut app, &ctx);
        click(&mut app, &ctx, "Save");
        saves_done(&mut app, &ctx);
        assert!(matches!(&app.dialog, Dialog::Message(t, _) if t == "Save failed"));
        click(&mut app, &ctx, "OK");
        assert!(!app.quit_ok, "quit with a document that failed to save");
        assert!(app.closing.is_none() && matches!(app.dialog, Dialog::None));
        assert_eq!(ids(&app), all);
        assert!(!app.tabs[0].dirty && app.tabs[1].dirty);
        assert_eq!(pages_in(&a), 2);
        // With one document unsaved, quitting asks about it alone, and Discard quits.
        close_window(&mut app, &ctx);
        assert!(matches!(app.dialog, Dialog::ConfirmClose(_, true)));
        click(&mut app, &ctx, "Discard");
        assert!(app.quit_ok);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// Shapes with outlines thinner than the inspector's smallest width (0.25 pt), selected
    /// with the arrow and not touched, are no change: closing doesn't ask, the document stays
    /// as it was, and quitting with other changes quits on Discard, with no "The document
    /// changed" (the user's find, 2026-10-28: the width field raised them to 0.25 pt).
    #[test]
    fn thin_shapes_selected_are_no_change() {
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        theme::install(&ctx);
        let mut app = thin_tab(&ctx);
        let looks = |app: &App| -> Vec<_> {
            let objs = app.tabs[0].pdf.page_objects(0).unwrap();
            objs.iter().map(|o| (o.bbox, o.line_width)).collect()
        };
        let (was, before) = (looks(&app), app.tabs[0].history());
        app.act(&ctx, Act::Tool(Tool::Select));
        for ids in [&[0][..], &[1], &[0, 1]] {
            select(&mut app, &ctx, ids);
            settled_at(&mut app, &ctx, 1400.0);
            let t = &app.tabs[0];
            assert!(!t.unsaved(), "{ids:?}: selecting them is a change");
            assert_eq!(t.history(), before, "{ids:?}: the document changed");
            assert_eq!(looks(&app), was, "{ids:?}");
        }
        // Quitting with another change unsaved: Discard quits.
        app.tabs[0].dirty = true;
        close_window(&mut app, &ctx);
        assert!(matches!(app.dialog, Dialog::ConfirmClose(_, true)));
        click_named(&mut app, &ctx, "Discard");
        if let Dialog::Message(_, text) = &app.dialog {
            panic!("Discard said: {text}");
        }
        assert!(app.quit_ok);
    }

    /// A tab, as if just opened, whose first page has a hairline (0 pt) outlined shape at
    /// 300,60 to 380,120 (page points, y down), a red one filled with a 0.1 pt line width
    /// set at 300,160 to 380,220, and a 0.5 pt rule along y 292 from x 72 to 272.
    fn thin_tab(ctx: &egui::Context) -> App {
        let mut app = app_with(ctx, 1);
        let thin = |p: &mut Pdf| {
            let page = p.page_ids()[0];
            let content = "q 0 w 0 0 1 RG 300 672 80 60 re S Q\n\
                           q 0.1 w 1 0 0 rg 300 572 80 60 re f Q\n\
                           q 0.5 w 0 0 0 RG 72 500 m 272 500 l S Q\n";
            let content = lopdf::Stream::new(lopdf::Dictionary::new(), content.into());
            let s = p.doc.add_object(content);
            let d = p.doc.get_dictionary_mut(page)?;
            d.set("Contents", lopdf::Object::Reference(s));
            Ok(())
        };
        assert!(app.tabs[0].edit(ctx, thin));
        app.tabs[0].dirty = false;
        app
    }

    /// A rule thinner than the inspector's smallest height (1 pt), moved by its X in the
    /// inspector, moves across only: its height isn't made 1 pt.
    #[test]
    fn a_thin_rule_moved_in_the_inspector_moves_across_only() {
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        theme::install(&ctx);
        let mut app = thin_tab(&ctx);
        app.act(&ctx, Act::Tool(Tool::Select));
        app.open_inspector();
        select(&mut app, &ctx, &[2]);
        let was = boxes(&app, 0)[2];
        assert!(was[3] - was[1] < 1.0, "{was:?}");
        click_named(&mut app, &ctx, "Position and size");
        click_named(&mut app, &ctx, "X");
        let (ctrl, none) = (egui::Modifiers::COMMAND, egui::Modifiers::NONE);
        window(&mut app, &ctx, 1400.0, vec![key_with(Key::A, ctrl)]);
        let typed = vec![egui::Event::Text("100".into())];
        window(&mut app, &ctx, 1400.0, typed);
        window(&mut app, &ctx, 1400.0, vec![key_with(Key::Enter, none)]);
        settle(&mut app, &ctx);
        let now = boxes(&app, 0)[2];
        assert!((now[0] - 100.0).abs() < 0.6, "not moved: {now:?}");
        // Made 1 pt tall, a rule moves down by half the difference.
        let (top, bottom) = ((now[1] - was[1]).abs(), (now[3] - was[3]).abs());
        assert!(top < 0.05 && bottom < 0.05, "{was:?} to {now:?}");
    }

    /// ENH-01: closing the window while a tab's save asks a question asks Quit first;
    /// after Cancel the save's question is back, and once answered the tab is saved and
    /// closed. Going on to quit instead asks about that document again, once.
    #[test]
    fn closing_the_window_during_a_save_keeps_the_save() {
        let ctx = egui::Context::default();
        theme::install(&ctx);
        let mut app = App::with(Settings::from_file(None), Recent::from_file(None));
        let dir = test_dir("close-during-save");
        let path = dir.join("repaired.pdf");
        let front = unsaved_tab(&mut app, &ctx, None);
        unsaved_tab(&mut app, &ctx, Some(path.clone()));
        app.tabs[1].repaired = true;
        app.active = 0;
        app.close_asking(1);
        click(&mut app, &ctx, "Save");
        assert!(matches!(app.dialog, Dialog::ConfirmSaveRepaired(_)));
        close_window(&mut app, &ctx);
        assert!(matches!(app.dialog, Dialog::ConfirmQuit));
        click(&mut app, &ctx, "Cancel");
        assert!(
            matches!(app.dialog, Dialog::ConfirmSaveRepaired(_)),
            "the save's question is gone"
        );
        click(&mut app, &ctx, "Replace original");
        saves_done(&mut app, &ctx);
        assert_eq!(pages_in(&path), 2);
        assert_eq!(ids(&app), [front], "the saved tab wasn't closed");
        // Quitting from the middle of a save: Review asks about each document once.
        let other = unsaved_tab(&mut app, &ctx, Some(path.clone()));
        app.tabs[1].repaired = true;
        app.close_asking(1);
        click(&mut app, &ctx, "Save");
        close_window(&mut app, &ctx);
        click(&mut app, &ctx, "Review…");
        assert!(app.waiting.is_empty(), "the save's question still waits");
        let mut asked = Vec::new();
        while let Dialog::ConfirmClose(a, true) = &app.dialog {
            asked.push(a.tab);
            click(&mut app, &ctx, "Discard");
        }
        assert_eq!(asked, [front, other]);
        assert!(app.quit_ok);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// ENH-35: the file is written on a thread of its own, the next save's held back by
    /// the test until it says go. The window meanwhile keeps drawing frames, with the busy
    /// sign, and the tab is unsaved until the write is done; then it says Saved, as before.
    #[test]
    fn saving_writes_on_its_own_thread_while_the_window_keeps_drawing() {
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        theme::install(&ctx);
        let mut app = App::with(Settings::from_file(None), Recent::from_file(None));
        let dir = test_dir("save-thread");
        let path = dir.join("a.pdf");
        unsaved_tab(&mut app, &ctx, Some(path.clone()));
        let (go, gate) = channel();
        app.save_gate = Some(gate);
        app.act(&ctx, Act::Save);
        let tree = settle(&mut app, &ctx);
        assert!(shows(&tree, "Saving a.pdf…"), "no busy sign");
        assert!(!path.exists(), "written before the window drew");
        assert!(app.tabs[0].dirty, "saved before it was written");
        go.send(()).unwrap();
        saves_done(&mut app, &ctx);
        let tree = settle(&mut app, &ctx);
        assert!(!shows(&tree, "Saving a.pdf…"), "the busy sign stayed");
        assert_eq!(app.status, format!("Saved {}", path.display()));
        assert_eq!(pages_in(&path), 2);
        assert!(!app.tabs[0].dirty);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// ENH-35: the document can be changed while it is written. The file gets it as it was
    /// when saved, and the tab, changed since, stays unsaved. A question asked about the
    /// tab meanwhile still acts on it once the save is done.
    #[test]
    fn a_document_changed_while_it_is_saved_stays_unsaved() {
        let ctx = egui::Context::default();
        let mut app = App::with(Settings::from_file(None), Recent::from_file(None));
        let dir = test_dir("save-changed");
        let path = dir.join("form.pdf");
        form_file(&path, "AlphaUnique");
        open_and_wait(&mut app, &ctx, &path, 1);
        fill(&mut app, &ctx, "Notes", "Saved");
        let (go, gate) = channel();
        app.save_gate = Some(gate);
        app.act(&ctx, Act::Save);
        fill(&mut app, &ctx, "Notes", "After");
        app.act(&ctx, Act::FlattenForm);
        assert!(matches!(app.dialog, Dialog::ConfirmFlatten(_)));
        go.send(()).unwrap();
        saves_done(&mut app, &ctx);
        assert_eq!(field_in(&path, "Notes"), "Saved");
        assert!(
            app.tabs[0].dirty,
            "a change made during the save counts as saved"
        );
        assert_eq!(saved_field(&app.tabs[0], "Notes"), "After");
        click(&mut app, &ctx, "Flatten form");
        assert!(
            matches!(app.dialog, Dialog::None),
            "the question no longer applied"
        );
        assert!(app.tabs[0].pdf.form_fields().is_empty(), "not flattened");
        // Saved again, with nothing changed during the save, it is saved.
        app.act(&ctx, Act::Save);
        saves_done(&mut app, &ctx);
        assert!(!app.tabs[0].dirty);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// ENH-35: Save chosen again while the tab's save is written saves again once it ends,
    /// with what changed meanwhile; Reload and signing wait, saying so.
    #[test]
    fn saving_again_reloading_and_signing_wait_for_the_save() {
        let ctx = egui::Context::default();
        let mut app = App::with(Settings::from_file(None), Recent::from_file(None));
        let dir = test_dir("save-again");
        let path = dir.join("a.pdf");
        unsaved_tab(&mut app, &ctx, Some(path.clone()));
        let (go, gate) = channel();
        app.save_gate = Some(gate);
        app.act(&ctx, Act::Save);
        let c = app.state(&ctx);
        for a in [Act::Reload, Act::SignDigital, Act::Certify] {
            assert!(!(actions::get(a).enabled)(&c), "{a:?} on while saving");
        }
        app.act(&ctx, Act::Reload);
        assert_eq!(message(&app).0, "Still saving");
        app.dialog = Dialog::None;
        assert!(app.tabs[0].edit(&ctx, |p| p.insert_blank(2, 300.0, 500.0)));
        app.act(&ctx, Act::Save);
        assert_eq!(app.saves.len(), 1, "two saves of a tab at once");
        go.send(()).unwrap();
        wait_for(&mut app, &ctx, |a| a.saves.is_empty());
        assert_eq!(pages_in(&path), 2);
        assert!(app.tabs[0].dirty);
        // The second save starts once the first has ended.
        saves_done(&mut app, &ctx);
        assert!(app.after_saves.is_empty());
        saves_done(&mut app, &ctx);
        assert_eq!(pages_in(&path), 3);
        assert!(!app.tabs[0].dirty);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// ENH-35: closing the tab, quitting and closing the window wait for a save being
    /// written, then go on as usual (here with nothing left unsaved, without asking). At
    /// exit, a save still being written is finished first.
    #[test]
    fn closing_and_quitting_wait_for_a_save() {
        let ctx = egui::Context::default();
        theme::install(&ctx);
        let dir = test_dir("save-close");
        let path = dir.join("a.pdf");
        let start = |path: &Path| {
            let mut app = App::with(Settings::from_file(None), Recent::from_file(None));
            unsaved_tab(&mut app, &ctx, Some(path.to_path_buf()));
            let (go, gate) = channel();
            app.save_gate = Some(gate);
            app.act(&ctx, Act::Save);
            (app, go)
        };
        let (mut app, go) = start(&path);
        app.close_asking(0);
        assert_eq!(app.tabs.len(), 1, "closed before it was saved");
        assert!(
            matches!(app.dialog, Dialog::None),
            "asked about a tab being saved"
        );
        go.send(()).unwrap();
        saves_done(&mut app, &ctx);
        assert!(app.tabs.is_empty(), "not closed once saved");
        assert_eq!(pages_in(&path), 2);

        for close in [false, true] {
            let (mut app, go) = start(&path);
            if close {
                close_window(&mut app, &ctx);
            } else {
                app.act(&ctx, Act::Quit);
            }
            assert!(
                !app.quit_ok && matches!(app.dialog, Dialog::None),
                "{close}"
            );
            go.send(()).unwrap();
            saves_done(&mut app, &ctx);
            assert!(app.quit_ok, "didn't quit once saved ({close})");
        }

        std::fs::remove_file(&path).unwrap();
        let (mut app, go) = start(&path);
        let later = std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_millis(100));
            go.send(()).unwrap();
        });
        eframe::App::on_exit(&mut app);
        assert_eq!(pages_in(&path), 2, "the exit didn't wait for the save");
        later.join().unwrap();
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// ENH-35: a check of the files, done when the window comes back to the front, that
    /// ends while a tab's own save replaces its file doesn't say another app changed it.
    #[test]
    fn a_file_check_during_its_save_is_left_out() {
        let ctx = egui::Context::default();
        let mut app = App::with(Settings::from_file(None), Recent::from_file(None));
        let dir = test_dir("save-check");
        let path = dir.join("form.pdf");
        form_file(&path, "AlphaUnique");
        open_and_wait(&mut app, &ctx, &path, 1);
        fill(&mut app, &ctx, "Notes", "Ours");
        let (go, gate) = channel();
        app.save_gate = Some(gate);
        app.act(&ctx, Act::Save);
        let known = app.tabs[0].disk.clone().unwrap();
        form_file(&dir.join("other.pdf"), "Other");
        let other = disk::version(&dir.join("other.pdf")).unwrap();
        let found = vec![(app.tabs[0].id, known, OnDisk::Changed(other))];
        app.tx.send(Loaded::Disk(found)).unwrap();
        app.poll_loads(&ctx);
        let mut b = app.tabs[0].banners.iter();
        assert!(
            !b.any(|b| b.kind == BannerKind::ChangedOnDisk),
            "said changed"
        );
        go.send(()).unwrap();
        saves_done(&mut app, &ctx);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// A file whose time changed, or is too close to its reading to trust (PB-28), is read
    /// in full at each check; found unchanged, it becomes the version known, so that Save
    /// doesn't read the whole file again at every check before writing (ENH-35's
    /// measurement).
    #[test]
    fn a_file_found_unchanged_becomes_the_version_known() {
        let ctx = egui::Context::default();
        let mut app = App::with(Settings::from_file(None), Recent::from_file(None));
        let dir = test_dir("save-known");
        let path = dir.join("form.pdf");
        form_file(&path, "AlphaUnique");
        open_and_wait(&mut app, &ctx, &path, 1);
        let before = app.tabs[0].disk.clone();
        // Touched, as a sync tool might: the same bytes with a new time.
        let f = std::fs::File::options().write(true).open(&path).unwrap();
        f.set_modified(std::time::SystemTime::now()).unwrap();
        drop(f);
        assert!(app.stale(0, &path).is_none());
        assert_ne!(app.tabs[0].disk, before, "the file read again isn't known");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// The audit's A17 (BUG-20): Append PDFs asks for a protected file's password (again
    /// when wrong) and adds every file's pages in order in one step; Skip this file leaves
    /// that one out; Cancel, or a file that can't be read, adds nothing.
    #[test]
    fn append_asks_for_a_protected_files_password() {
        let ctx = egui::Context::default();
        let mut app = app_with(&ctx, 1);
        let dir = test_dir("append");
        let (one, locked) = (dir.join("one.pdf"), dir.join("locked.pdf"));
        let (three, broken) = (dir.join("three.pdf"), dir.join("broken.pdf"));
        pages_file(&one, 100.0, 1, None);
        pages_file(&locked, 200.0, 2, Some("walrus"));
        pages_file(&three, 300.0, 1, None);
        std::fs::write(&broken, "not a PDF").unwrap();
        let files = [one.clone(), locked.clone(), three.clone()];
        let before = widths(&app, 0);
        assert_eq!(before, [612.0, 612.0]);

        app.merge_files(&ctx, &files);
        wait_for(&mut app, &ctx, |a| !a.busy);
        assert_eq!(asked_password(&app).as_ref(), Some(&locked), "not asked");
        answer(&mut app, &ctx, "wrong", "Add pages");
        wait_for(&mut app, &ctx, |a| !a.busy);
        assert!(matches!(&app.dialog, Dialog::Password(s, ..) if s.error.is_some()));
        assert_eq!(widths(&app, 0), before);
        answer(&mut app, &ctx, "walrus", "Add pages");
        wait_for(&mut app, &ctx, |a| !a.busy);
        assert!(matches!(app.dialog, Dialog::None));
        assert_eq!(widths(&app, 0), [612.0, 612.0, 100.0, 200.0, 200.0, 300.0]);
        assert!(app.tabs[0].copy.protected, "pages of a protected file");
        app.act(&ctx, Act::Undo);
        assert_eq!(widths(&app, 0), before, "not one step");

        app.merge_files(&ctx, &files);
        wait_for(&mut app, &ctx, |a| !a.busy);
        click(&mut app, &ctx, "Skip this file");
        wait_for(&mut app, &ctx, |a| !a.busy);
        assert_eq!(widths(&app, 0), [612.0, 612.0, 100.0, 300.0]);
        app.act(&ctx, Act::Undo);

        let state = app.tabs[0].state();
        app.merge_files(&ctx, &files);
        wait_for(&mut app, &ctx, |a| !a.busy);
        click(&mut app, &ctx, "Cancel");
        assert!(matches!(app.dialog, Dialog::None));
        assert!(app.tabs[0].state() == state, "Cancel changed it");

        app.merge_files(&ctx, &[one.clone(), broken.clone()]);
        wait_for(&mut app, &ctx, |a| !a.busy);
        let (title, text) = message(&app);
        assert_eq!(title, "Append failed");
        assert!(
            text.contains("broken.pdf") && text.ends_with("Nothing was added."),
            "{text}"
        );
        assert!(app.tabs[0].state() == state);
        click(&mut app, &ctx, "OK");

        // Its tab closed while the files are read.
        app.merge_files(&ctx, std::slice::from_ref(&one));
        app.close_tab(0);
        wait_for(&mut app, &ctx, |a| !a.busy);
        assert_eq!(message(&app).0, "Pages not added");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// A one-page form whose field Name holds `answer` with no drawing of its own, as apps
    /// that leave the drawing to the reader save it, at `path`.
    fn undrawn_form_file(path: &Path, answer: lopdf::Object) {
        let mut pdf = form_doc(&[("Name", 1)]);
        let catalog = pdf.doc.catalog().unwrap();
        let form = catalog.get(b"AcroForm").unwrap().as_dict().unwrap();
        let field = form.get(b"Fields").unwrap().as_array().unwrap()[0].clone();
        let field = field.as_reference().unwrap();
        pdf.doc.get_dictionary_mut(field).unwrap().set("V", answer);
        pdf.save(path).unwrap();
    }

    /// The audit's A16 (BUG-19): Append PDFs asks before adding a file with form fields,
    /// whose answers are then drawn onto its pages; the document's own fields stay
    /// fillable, and Undo removes the pages. Cancel adds nothing; nor does an answer
    /// LimarePDF can't draw, which is named.
    #[test]
    fn append_draws_the_form_fields_of_added_pages_once_asked() {
        let ctx = egui::Context::default();
        let mut app = App::with(Settings::from_file(None), Recent::from_file(None));
        let dir = test_dir("append-forms");
        let (own, filled) = (dir.join("own.pdf"), dir.join("filled.pdf"));
        let (plain, foreign) = (dir.join("plain.pdf"), dir.join("foreign.pdf"));
        form_file(&own, "Grace");
        undrawn_form_file(&filled, lopdf::Object::string_literal("Ada Lovelace"));
        pages_file(&plain, 100.0, 1, None);
        // Text in characters LimarePDF can't write in a field yet (Hebrew, right to left).
        let mut utf16 = vec![0xFE, 0xFF];
        utf16.extend("שלום".encode_utf16().flat_map(u16::to_be_bytes));
        undrawn_form_file(
            &foreign,
            lopdf::Object::String(utf16, lopdf::StringFormat::Hexadecimal),
        );
        open_and_wait(&mut app, &ctx, &own, 1);
        let state = app.tabs[0].state();

        app.merge_files(&ctx, &[plain.clone(), filled.clone()]);
        wait_for(&mut app, &ctx, |a| !a.busy);
        let Dialog::ConfirmAppendForms(q) = &app.dialog else {
            panic!("not asked");
        };
        assert_eq!(q.names, ["filled.pdf"]);
        click(&mut app, &ctx, "Cancel");
        assert!(matches!(app.dialog, Dialog::None));
        assert!(app.tabs[0].state() == state, "Cancel changed it");

        app.merge_files(&ctx, &[plain.clone(), filled.clone()]);
        wait_for(&mut app, &ctx, |a| !a.busy);
        click(&mut app, &ctx, "Add pages");
        assert!(matches!(app.dialog, Dialog::None));
        let mut pdf = app.tabs[0].pdf.clone();
        assert_eq!(pdf.page_count(), 3);
        assert!(
            pdf.page_text(2).contains("Ada Lovelace"),
            "answer not drawn"
        );
        let names: Vec<String> = pdf.form_fields().into_iter().map(|f| f.name).collect();
        assert_eq!(names, ["Name", "Notes"]);
        pdf.set_field("Name", limarepdf_core::FieldValue::Text("Hopper".into()))
            .unwrap();
        assert!(pdf.page_text(2).contains("Ada Lovelace"));
        app.act(&ctx, Act::Undo);
        assert_eq!(app.tabs[0].pdf.page_count(), 1, "not one step");

        let state = app.tabs[0].state();
        app.merge_files(&ctx, &[filled.clone(), foreign.clone()]);
        wait_for(&mut app, &ctx, |a| !a.busy);
        let (title, text) = message(&app);
        assert_eq!(title, "Pages not added");
        assert!(
            text.contains("\"Name\" in “foreign.pdf”") && text.ends_with("Nothing was added."),
            "{text}"
        );
        assert!(app.tabs[0].state() == state);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    // ------------------------------------------- batch 24: formatting selected words

    /// The font of the lines here: a subset of Helvetica Oblique, which new text can't be
    /// written in, so text written again comes out in another font.
    const SUBSET: &str = "ABCDEF+Helvetica-Oblique";

    /// A tab whose lines, all 20 points in [`SUBSET`], are on page 1 "one two three" and
    /// "four five six" ("five" red), and on page 2 "seven eight nine" and the invisible
    /// (recognised) "hidden words".
    fn subset_tab(app: &mut App, ctx: &egui::Context) {
        use lopdf::{dictionary, Dictionary, Object, Stream};
        let mut pdf = Pdf::new();
        let contents = [
            "BT /F3 20 Tf 72 700 Td (one two three) Tj ET\n\
             BT /F3 20 Tf 72 650 Td (four ) Tj 1 0 0 rg (five) Tj 0 0 0 rg ( six) Tj ET",
            "BT /F3 20 Tf 72 700 Td (seven eight nine) Tj ET\n\
             BT 3 Tr /F3 20 Tf 72 600 Td (hidden words) Tj ET",
        ];
        for (i, c) in contents.iter().enumerate() {
            pdf.insert_blank(i, 612.0, 792.0).unwrap();
            let font = pdf.doc.add_object(dictionary! {
                "Type" => "Font", "Subtype" => "Type1", "BaseFont" => SUBSET,
                "Encoding" => "WinAnsiEncoding",
            });
            let s = (pdf.doc).add_object(Stream::new(Dictionary::new(), c.as_bytes().to_vec()));
            let page = pdf.page_ids()[i];
            let d = pdf.doc.get_dictionary_mut(page).unwrap();
            d.set("Contents", Object::Reference(s));
            let fonts = dictionary! { "F3" => font };
            d.set("Resources", dictionary! { "Font" => fonts });
        }
        assert!(app.add_tab(ctx, pdf, None, None, false));
    }

    /// Page `page`'s lines in `pdf`, by their text: each part's text and font, parts next
    /// to each other in one font (in two font objects, or two colours) together.
    fn line_fonts(pdf: &Pdf, page: usize) -> Vec<Vec<(String, String)>> {
        let mut lines: Vec<Vec<(String, String)>> = (pdf.page_objects(page).unwrap())
            .into_iter()
            .filter_map(|o| {
                let text: Vec<char> = o.text?.chars().collect();
                let mut parts: Vec<(String, String)> = Vec::new();
                for s in &o.spans {
                    let words: String = text[s.chars.0..s.chars.1].iter().collect();
                    match parts.last_mut() {
                        Some((t, f)) if *f == s.font_name => t.push_str(&words),
                        _ => parts.push((words, s.font_name.clone())),
                    }
                }
                Some(parts)
            })
            .collect();
        lines.sort();
        lines
    }

    /// The text of the toast for the tab in front, if one shows.
    fn toast(app: &App) -> Option<String> {
        let front = app.tabs.get(app.active).map(|t| t.id);
        let t = app.toasts.iter().find(|t| Some(t.tab) == front)?;
        Some(t.text.clone())
    }

    /// Lines of parts, each part's text and font, as [`line_fonts`] gives them.
    fn fonts_are(lines: &[&[(&str, &str)]]) -> Vec<Vec<(String, String)>> {
        let line = |parts: &&[(&str, &str)]| {
            (parts.iter())
                .map(|(t, f)| (t.to_string(), f.to_string()))
                .collect()
        };
        lines.iter().map(line).collect()
    }

    /// A window with the subset tab and the words from `anchor` to `end` selected with the
    /// read tool.
    fn words_selected(
        ctx: &egui::Context,
        anchor: crate::selection::WordAt,
        end: crate::selection::WordAt,
    ) -> App {
        ctx.enable_accesskit();
        theme::install(ctx);
        let mut app = App::with(Settings::from_file(None), Recent::from_file(None));
        subset_tab(&mut app, ctx);
        settle(&mut app, ctx);
        app.tabs[0].ed.selection = Selection::Words { anchor, end };
        settle(&mut app, ctx);
        app
    }

    /// D1's 1.0 baseline (ENH-05): words selected with the read tool across lines and pages,
    /// then Ctrl+B. Those words are bold and still selected; the words around them keep
    /// their own font, the subset, as do the other lines; the same after saving and
    /// reopening; one Undo takes it all back.
    #[test]
    fn ctrl_b_makes_words_selected_across_lines_and_pages_bold() {
        let ctx = egui::Context::default();
        let mut app = words_selected(&ctx, (0, 2), (1, 1));
        let (undo, before) = (app.tabs[0].history().1, line_fonts(&app.tabs[0].pdf, 0));
        window(&mut app, &ctx, 1400.0, vec![key_with(Key::B, CTRL)]);
        settle(&mut app, &ctx);
        let bold = "Helvetica-BoldOblique";
        let page_1 = fonts_are(&[
            &[("four five six", bold)],
            &[("one two ", SUBSET), ("three", bold)],
        ]);
        let page_2 = fonts_are(&[
            &[("hidden words", SUBSET)],
            &[("seven eight", bold), (" nine", SUBSET)],
        ]);
        let t = &app.tabs[0];
        assert_eq!(line_fonts(&t.pdf, 0), page_1);
        assert_eq!(line_fonts(&t.pdf, 1), page_2);
        let selected = Selection::Words {
            anchor: (0, 2),
            end: (1, 1),
        };
        assert_eq!(t.ed.selection, selected, "not kept");
        assert_eq!(toast(&app), None);
        let t = &app.tabs[0];
        let saved = saved_pdf(t);
        assert_eq!(line_fonts(&saved, 0), page_1);
        assert_eq!(line_fonts(&saved, 1), page_2);
        assert_eq!(t.history().1, undo + 1, "not one step");
        // Undo and Redo leave the same words selected, as a word processor does.
        app.act(&ctx, Act::Undo);
        assert_eq!(line_fonts(&app.tabs[0].pdf, 0), before);
        assert_eq!(app.tabs[0].history().1, undo);
        settle(&mut app, &ctx);
        assert_eq!(app.tabs[0].ed.selection, selected, "not kept by Undo");
        app.act(&ctx, Act::Redo);
        assert_eq!(line_fonts(&app.tabs[0].pdf, 0), page_1);
        settle(&mut app, &ctx);
        assert_eq!(app.tabs[0].ed.selection, selected, "not kept by Redo");
    }

    /// Bold and Italic follow Word's rule across the whole selection: some of it bold
    /// makes all of it bold; all of it bold takes the bold off. Italic over words all in
    /// an oblique font takes the slant off.
    #[test]
    fn bold_and_italic_follow_words_rule_across_lines() {
        let ctx = egui::Context::default();
        let mut app = words_selected(&ctx, (0, 2), (0, 2));
        window(&mut app, &ctx, 1400.0, vec![key_with(Key::B, CTRL)]);
        settle(&mut app, &ctx);
        // "two three four": "three" bold already, so all of it bold.
        app.tabs[0].ed.selection = Selection::Words {
            anchor: (0, 1),
            end: (0, 3),
        };
        settle(&mut app, &ctx);
        window(&mut app, &ctx, 1400.0, vec![key_with(Key::B, CTRL)]);
        settle(&mut app, &ctx);
        let bold = "Helvetica-BoldOblique";
        let lines = line_fonts(&app.tabs[0].pdf, 0);
        let want = fonts_are(&[
            &[("four", bold), (" five six", SUBSET)],
            &[("one ", SUBSET), ("two three", bold)],
        ]);
        assert_eq!(lines, want);
        // All of it bold: the bold comes off.
        window(&mut app, &ctx, 1400.0, vec![key_with(Key::B, CTRL)]);
        settle(&mut app, &ctx);
        let lines = line_fonts(&app.tabs[0].pdf, 0);
        let fonts: Vec<&str> = (lines.iter().flatten())
            .filter(|(t, _)| ["two three", "four"].contains(&t.as_str()))
            .map(|(_, f)| f.as_str())
            .collect();
        let oblique = "Helvetica-Oblique";
        assert_eq!(fonts, [oblique, oblique], "{lines:?}");
        // Italic over oblique words: upright.
        window(&mut app, &ctx, 1400.0, vec![key_with(Key::I, CTRL)]);
        settle(&mut app, &ctx);
        let lines = line_fonts(&app.tabs[0].pdf, 0);
        assert!(
            (lines.iter().flatten()).any(|(t, f)| t == "two three" && f == "Helvetica"),
            "{lines:?}"
        );
    }

    /// The inspector shows the selected words' look, headed "Selected text", blank where
    /// they mix it; a colour picked makes them all that colour, keeping their font, as one
    /// undo step, and they stay selected.
    #[test]
    fn the_inspector_restyles_selected_words_and_a_pick_unifies_their_colour() {
        let ctx = egui::Context::default();
        let mut app = words_selected(&ctx, (0, 3), (0, 5));
        app.open_inspector();
        settle(&mut app, &ctx);
        assert_eq!(inspector::title(&app.tabs[0]), "Selected text");
        let undo = app.tabs[0].history().1;
        let mut frame = |app: &mut App, events| window(app, &ctx, 1400.0, events);
        click_named(&mut app, &ctx, "Text colour (mixed)");
        settle(&mut app, &ctx);
        click_in_colour_area(&mut app, &ctx, &mut frame, egui::vec2(0.0, 0.0));
        click_named(&mut app, &ctx, "Text colour");
        settle(&mut app, &ctx);
        let t = &app.tabs[0];
        let line = (t.pdf.page_objects(0).unwrap().into_iter())
            .find(|o| o.text.as_deref() == Some("four five six"))
            .expect("the line");
        let looks: Vec<(&str, Option<[f32; 3]>)> = (line.spans.iter())
            .map(|s| (s.font_name.as_str(), s.fill))
            .collect();
        let black = |&(f, c): &(&str, Option<[f32; 3]>)| f == SUBSET && c == Some([0.0; 3]);
        assert!(looks.iter().all(black), "{looks:?}");
        assert_eq!(t.history().1, undo + 1, "not one step");
        assert!(t.ed.selection.words().is_some(), "not kept");
        assert_eq!(inspector::title(t), "Selected text");
    }

    /// Words that can't be restyled are left as they were and said: invisible recognised
    /// text beside visible words, and a selection of only invisible text, which changes
    /// nothing.
    #[test]
    fn invisible_words_in_the_selection_are_left_as_they_were_and_said() {
        let ctx = egui::Context::default();
        let mut app = words_selected(&ctx, (1, 2), (1, 4));
        window(&mut app, &ctx, 1400.0, vec![key_with(Key::B, CTRL)]);
        settle(&mut app, &ctx);
        let page_2 = fonts_are(&[
            &[("hidden words", SUBSET)],
            &[("seven eight ", SUBSET), ("nine", "Helvetica-BoldOblique")],
        ]);
        assert_eq!(line_fonts(&app.tabs[0].pdf, 1), page_2);
        let said = "Not changed: 1 line of invisible recognised text";
        assert_eq!(toast(&app).as_deref(), Some(said));
        // Only invisible words: nothing changes, and that is said.
        app.toasts.clear();
        app.tabs[0].ed.selection = Selection::Words {
            anchor: (1, 3),
            end: (1, 4),
        };
        settle(&mut app, &ctx);
        let history = app.tabs[0].history();
        window(&mut app, &ctx, 1400.0, vec![key_with(Key::B, CTRL)]);
        settle(&mut app, &ctx);
        assert_eq!(app.tabs[0].history(), history);
        assert_eq!(toast(&app).as_deref(), Some(said));
        assert_ne!(inspector::title(&app.tabs[0]), "Selected text");
    }

    /// Ctrl+B and Ctrl+I also change a line selected whole with the arrow, as its
    /// inspector's buttons do.
    #[test]
    fn ctrl_b_makes_a_line_selected_with_the_arrow_bold() {
        let ctx = egui::Context::default();
        let mut app = words_selected(&ctx, (0, 0), (0, 0));
        let t = &mut app.tabs[0];
        t.ed.set_tool(Tool::Select);
        let id = (t.pdf.page_objects(0).unwrap().into_iter())
            .find(|o| o.text.as_deref() == Some("one two three"))
            .expect("the line")
            .id;
        t.ed.select_object(0, id);
        settle(&mut app, &ctx);
        window(&mut app, &ctx, 1400.0, vec![key_with(Key::B, CTRL)]);
        settle(&mut app, &ctx);
        let lines = line_fonts(&app.tabs[0].pdf, 0);
        let bold = "Helvetica-BoldOblique";
        assert_eq!(lines[1], fonts_are(&[&[("one two three", bold)]])[0]);
    }

    /// Selected words are shaded one band a line, the spaces between them too, as word
    /// processors shade a selection, not a box a word.
    #[test]
    fn selected_words_are_shaded_one_band_a_line() {
        let ctx = egui::Context::default();
        let mut app = words_selected(&ctx, (0, 0), (0, 4));
        let tree = settle(&mut app, &ctx);
        let (page, _) = node(&tree, "Page 1").expect("the page");
        let raw = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(1400.0, TEST_HEIGHT),
            )),
            ..Default::default()
        };
        let mut f = eframe::Frame::_new_kittest();
        let mut out = ctx.run_ui(raw, |ui| eframe::App::ui(&mut app, ui, &mut f));
        out.textures_delta.clear();
        let fill = theme::text_shading(ctx.global_style().visuals.dark_mode);
        fn walk(s: &egui::Shape, fill: egui::Color32, out: &mut Vec<egui::Rect>) {
            match s {
                egui::Shape::Rect(r) if r.fill == fill => out.push(r.rect),
                egui::Shape::Vec(v) => v.iter().for_each(|s| walk(s, fill, out)),
                _ => {}
            }
        }
        let mut bands = Vec::new();
        for c in &out.shapes {
            walk(&c.shape, fill, &mut bands);
        }
        bands.retain(|r| page.contains_rect(*r));
        // "one two three" and "four five": each band from its first word to its last.
        let words = app.tabs[0].pdf.page_words(0).unwrap();
        let k = page.width() / 612.0;
        let x = |v: f32| page.min.x + v * k;
        assert_eq!(bands.len(), 2, "{bands:?}");
        for (band, (first, last)) in bands.iter().zip([(0, 2), (3, 4)]) {
            let (left, right) = (x(words[first].bbox[0]), x(words[last].bbox[2]));
            assert!((band.left() - left).abs() < 1.0, "{band:?} from {left}");
            assert!((band.right() - right).abs() < 1.0, "{band:?} to {right}");
        }
    }

    /// The floating toolbar over selected words has Bold and Italic.
    #[test]
    fn the_floating_toolbar_has_bold_and_italic_for_selected_words() {
        let ctx = egui::Context::default();
        let mut app = words_selected(&ctx, (0, 0), (0, 1));
        let tree = settle(&mut app, &ctx);
        assert!(node(&tree, "Italic").is_some());
        click_named(&mut app, &ctx, "Bold");
        settle(&mut app, &ctx);
        let lines = line_fonts(&app.tabs[0].pdf, 0);
        let bold = "Helvetica-BoldOblique";
        let want = fonts_are(&[&[("one two", bold), (" three", SUBSET)]]);
        assert_eq!(lines[1], want[0]);
    }

    /// The line editor keeps the words left as they were in their own font too: Ctrl+B on a
    /// word double-clicked makes that word bold, the rest of the line staying in the subset.
    #[test]
    fn the_line_editor_keeps_untouched_words_in_their_own_font() {
        let ctx = egui::Context::default();
        let mut app = words_selected(&ctx, (0, 0), (0, 0));
        app.tabs[0].ed.selection = Selection::None;
        let time = std::cell::Cell::new(10.0);
        let mut tree = timed_frame(&mut app, &ctx, &time, vec![]);
        for _ in 0..25 {
            tree = timed_frame(&mut app, &ctx, &time, vec![]);
        }
        let (page, _) = node(&tree, "Page 1").expect("the page");
        let k = page.width() / 612.0;
        // "two", from about 111 to 142 points across, its baseline 92 down.
        let p = page.min + egui::vec2(125.0, 86.0) * k;
        timed_frame(&mut app, &ctx, &time, vec![egui::Event::PointerMoved(p)]);
        for _ in 0..2 {
            timed_frame(&mut app, &ctx, &time, vec![press(p, true)]);
            timed_frame(&mut app, &ctx, &time, vec![press(p, false)]);
        }
        for _ in 0..3 {
            timed_frame(&mut app, &ctx, &time, vec![]);
        }
        timed_frame(&mut app, &ctx, &time, vec![key_with(Key::B, CTRL)]);
        let esc = key_with(Key::Escape, egui::Modifiers::NONE);
        timed_frame(&mut app, &ctx, &time, vec![esc]);
        for _ in 0..2 {
            timed_frame(&mut app, &ctx, &time, vec![]);
        }
        let lines = line_fonts(&app.tabs[0].pdf, 0);
        let bold = "Helvetica-BoldOblique";
        let want = fonts_are(&[&[("one ", SUBSET), ("two", bold), (" three", SUBSET)]]);
        assert_eq!(lines[1], want[0]);
        assert_eq!(toast(&app), None);
    }

    /// `n` lines of 12-point text 15 points apart from `top` down, starting `x` points across
    /// and `wide` (about 450 points long) or short (about 70); line `odd` says "otter" where
    /// the others say "walrus", or, short, "seal", so that no other line is longer.
    fn paragraph(x: f32, top: f32, n: usize, odd: usize, wide: bool) -> Vec<(String, egui::Pos2)> {
        (0..n)
            .map(|i| {
                let who = match (i == odd, wide) {
                    (true, _) => "otter",
                    (false, true) => "walrus",
                    (false, false) => "seal",
                };
                let text = if wide {
                    format!("The {who} ate fish by the sea and the seal ate fish by the shore {i}")
                } else {
                    format!("The {who} ate")
                };
                (text, egui::pos2(x, top + 15.0 * i as f32))
            })
            .collect()
    }

    /// A tab of one page 612 points wide and `h` high, with `lines` on it (Helvetica, 12
    /// points, each from its place), and the read tool.
    fn lines_tab(app: &mut App, ctx: &egui::Context, h: f32, lines: &[(String, egui::Pos2)]) {
        let mut pdf = Pdf::new();
        pdf.insert_blank(0, 612.0, h).unwrap();
        for (text, at) in lines {
            let font = FontChoice::default();
            pdf.add_text(0, (at.x, at.y), text, font, 12.0, [0.0; 3])
                .unwrap();
        }
        assert!(app.add_tab(ctx, pdf, None, None, false));
        app.act(ctx, Act::Tool(Tool::Read));
    }

    /// The floating bar's buttons, by name and place: those in the row of its Dock button,
    /// which only the bar has, up to it.
    fn bar_buttons(tree: &egui::accesskit::TreeUpdate) -> Vec<(String, egui::Rect)> {
        let Some((dock, _)) = node(tree, "Dock controls on the right") else {
            return Vec::new();
        };
        let button = |(_, n): &&(egui::accesskit::NodeId, egui::accesskit::Node)| {
            n.role() == egui::accesskit::Role::Button
        };
        (tree.nodes.iter().filter(button))
            .filter_map(|(_, n)| {
                let b = n.bounds()?;
                let r = egui::Rect::from_min_max(
                    egui::pos2(b.x0 as f32, b.y0 as f32),
                    egui::pos2(b.x1 as f32, b.y1 as f32),
                );
                let row = (r.center().y - dock.center().y).abs() < 2.0
                    && r.max.x <= dock.max.x
                    && dock.min.x - r.min.x < 320.0;
                row.then(|| (n.label().unwrap_or_default().to_string(), r))
            })
            .collect()
    }

    /// Opens the line saying "otter" in the active tab for editing on the page, a
    /// double-click on that word with the read tool, in windows `width` wide; returns the
    /// window's tree then and the size of a point on screen. `case` names the test's case.
    fn edit_otter(
        app: &mut App,
        ctx: &egui::Context,
        width: f32,
        case: &str,
    ) -> (egui::accesskit::TreeUpdate, f32) {
        let tree = settled_at(app, ctx, width);
        let (page, _) = node(&tree, "Page 1").expect("the page");
        let k = page.width() / 612.0;
        let words = app.tabs[app.active].pdf.page_words(0).unwrap();
        let otter = words.iter().find(|w| w.text == "otter");
        let otter = otter.expect("otter").bbox;
        let at = egui::vec2((otter[0] + otter[2]) / 2.0, (otter[1] + otter[3]) / 2.0);
        let p = page.min + at * k;
        window(app, ctx, width, vec![egui::Event::PointerMoved(p)]);
        for _ in 0..2 {
            window(app, ctx, width, vec![press(p, true)]);
            window(app, ctx, width, vec![press(p, false)]);
        }
        let tree = settled_at(app, ctx, width);
        let mut labels = tree.nodes.iter().filter_map(|(_, n)| n.label());
        let editing = labels.any(|l| l.starts_with("Editing text: The otter"));
        assert!(editing, "{case}: the line isn't being edited");
        (tree, k)
    }

    /// The tab and the window's title say unsaved as soon as a line being edited is typed
    /// in, as word processors mark a document changed at the first key, not only once the
    /// line is kept; a line opened and left as it was is no change.
    #[test]
    fn typing_in_a_line_marks_the_tab_unsaved_at_once() {
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        theme::install(&ctx);
        let mut app = App::with(Settings::from_file(None), Recent::from_file(None));
        lines_tab(&mut app, &ctx, 400.0, &paragraph(72.0, 100.0, 3, 1, false));
        let unsaved = |tree: &egui::accesskit::TreeUpdate| {
            let tab = |n: &egui::accesskit::Node| n.role() == egui::accesskit::Role::Tab;
            let mut tabs = tree.nodes.iter().filter(|(_, n)| tab(n));
            tabs.any(|(_, n)| n.label().is_some_and(|l| l.ends_with(", unsaved")))
        };
        let (tree, _) = edit_otter(&mut app, &ctx, 1400.0, "opened");
        assert!(!unsaved(&tree), "opened, not typed in");
        assert!(!app.last_title.contains('*'), "{}", app.last_title);
        window(&mut app, &ctx, 1400.0, vec![egui::Event::Text("s".into())]);
        let tree = settled_at(&mut app, &ctx, 1400.0);
        assert!(unsaved(&tree), "the tab");
        assert_eq!(app.last_title, "Untitled * - LimarePDF");
        assert!(!app.tabs[0].dirty, "kept already");
    }

    /// While a line is edited the status bar says so, and the inspector names the PDF's
    /// font and says, as the line is typed, whether it keeps that font: kept for plain
    /// ASCII in a font that can take it; where a character can't be, the words typed are
    /// in the nearest standard font and the rest keep theirs (ENH-39's scope and font).
    #[test]
    fn a_line_edit_says_its_scope_and_whether_its_font_is_kept() {
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        theme::install(&ctx);
        let mut app = App::with(Settings::from_file(None), Recent::from_file(None));
        lines_tab(&mut app, &ctx, 400.0, &paragraph(72.0, 100.0, 3, 1, false));
        let status = |tree: &egui::accesskit::TreeUpdate| {
            let mut values = tree.nodes.iter().filter_map(|(_, n)| n.value());
            values.any(|v| v.starts_with("Editing one line"))
        };
        let tree = settled_at(&mut app, &ctx, 1400.0);
        assert!(!status(&tree), "said before a line was edited");
        let (tree, _) = edit_otter(&mut app, &ctx, 1400.0, "opened");
        assert!(status(&tree), "the status bar");
        let says = inspector_says(&tree);
        let has = |says: &[String], s: &str| says.iter().any(|x| x == s);
        assert!(
            has(&says, "PDF font") && has(&says, "Helvetica"),
            "{says:?}"
        );
        assert!(has(&says, "Preserved"), "{says:?}");
        window(&mut app, &ctx, 1400.0, vec![egui::Event::Text("s".into())]);
        let says = inspector_says(&settled_at(&mut app, &ctx, 1400.0));
        assert!(has(&says, "Preserved"), "plain ASCII: {says:?}");
        window(&mut app, &ctx, 1400.0, vec![egui::Event::Text("é".into())]);
        let says = inspector_says(&settled_at(&mut app, &ctx, 1400.0));
        assert!(has(&says, "Partly preserved"), "{says:?}");
        // All of it typed over.
        window(&mut app, &ctx, 1400.0, vec![key_with(Key::A, CTRL)]);
        window(&mut app, &ctx, 1400.0, vec![egui::Event::Text("é".into())]);
        let says = inspector_says(&settled_at(&mut app, &ctx, 1400.0));
        assert!(has(&says, "Nearest standard"), "{says:?}");
        assert!(!has(&says, "Partly preserved"), "{says:?}");
    }

    /// Text typed on past the end of its line into the line beside it, or past the page's
    /// edge, says so on a tag by its outline, in the caution colour; nothing is shrunk or
    /// moved. New text past the edge says so too.
    #[test]
    fn typing_on_past_a_line_says_what_it_runs_over() {
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        theme::install(&ctx);
        let mut app = App::with(Settings::from_file(None), Recent::from_file(None));
        let lines = [
            ("The otter ate", (72.0, 100.0)),
            ("Seal", (200.0, 100.0)),
            ("The walrus ate", (515.0, 160.0)),
        ];
        let lines = lines.map(|(s, (x, y))| (s.to_string(), egui::pos2(x, y)));
        lines_tab(&mut app, &ctx, 400.0, &lines);
        let says = |tree: &egui::accesskit::TreeUpdate, s: &str| {
            tree.nodes.iter().any(|(_, n)| n.value() == Some(s))
        };
        let (over, past) = ("Over other text", "Past the page edge");
        let page = |app: &mut App| {
            let tree = settled_at(app, &ctx, 1400.0);
            let (page, _) = node(&tree, "Page 1").expect("the page");
            (page.min, page.width() / 612.0)
        };
        // Double-click `word` to edit its line.
        let open = |app: &mut App, word: &str| {
            let (min, k) = page(app);
            let words = app.tabs[0].pdf.page_words(0).unwrap();
            let b = words.iter().find(|w| w.text == word).expect(word).bbox;
            let p = min + egui::vec2((b[0] + b[2]) / 2.0, (b[1] + b[3]) / 2.0) * k;
            window(app, &ctx, 1400.0, vec![egui::Event::PointerMoved(p)]);
            for _ in 0..2 {
                window(app, &ctx, 1400.0, vec![press(p, true)]);
                window(app, &ctx, 1400.0, vec![press(p, false)]);
            }
            settled_at(app, &ctx, 1400.0)
        };
        let typed = |app: &mut App, text: &str| {
            window(app, &ctx, 1400.0, vec![egui::Event::Text(text.into())]);
            settled_at(app, &ctx, 1400.0)
        };
        let tree = open(&mut app, "otter");
        assert!(app.tabs[0].ed.editing_line());
        assert!(!says(&tree, over) && !says(&tree, past), "as it was");
        let esc = |app: &mut App| {
            let esc = key_with(Key::Escape, egui::Modifiers::NONE);
            window(app, &ctx, 1400.0, vec![esc]);
        };
        let tree = typed(&mut app, "otter and the seal ate fish by the sea");
        assert!(says(&tree, over) && !says(&tree, past), "into Seal");
        esc(&mut app);
        let tree = open(&mut app, "walrus");
        assert!(!says(&tree, over) && !says(&tree, past), "as it was");
        let tree = typed(&mut app, "walrus and the seal");
        assert!(says(&tree, past) && !says(&tree, over), "past the edge");
        esc(&mut app);
        let tree = settled_at(&mut app, &ctx, 1400.0);
        assert!(!says(&tree, past), "the tag stayed");
        // New text, put down near the edge.
        app.act(&ctx, Act::Tool(Tool::Text));
        let (min, k) = page(&mut app);
        let edge = min + egui::vec2(560.0, 300.0) * k;
        window(&mut app, &ctx, 1400.0, click_at(edge));
        settled_at(&mut app, &ctx, 1400.0);
        let tree = typed(&mut app, "Walrus and seal");
        assert!(says(&tree, past), "new text past the edge");
    }

    /// Inside a line being edited, Ctrl+X, Ctrl+V, Ctrl+A and Shift+click act on its text,
    /// as in a word processor, never on the page's: Cut takes the selected word out and
    /// copies it, Paste puts text at the caret, Shift+click selects from the caret to the
    /// click, Ctrl+A selects the line's text alone; kept, only that line has changed.
    #[test]
    fn keys_and_shift_click_inside_a_line_act_on_its_text() {
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        theme::install(&ctx);
        let mut app = App::with(Settings::from_file(None), Recent::from_file(None));
        lines_tab(&mut app, &ctx, 400.0, &paragraph(72.0, 100.0, 3, 1, false));
        let before = line_texts(&app.tabs[0].pdf, 0);
        edit_otter(&mut app, &ctx, 1400.0, "cut");
        let typed = |app: &mut App| focused_text(&settled_at(app, &ctx, 1400.0));
        let type_in = |app: &mut App, event: egui::Event| window(app, &ctx, 1400.0, vec![event]);
        assert_eq!(copied_by(&mut app, &ctx, vec![egui::Event::Cut]), ["otter"]);
        assert_eq!(typed(&mut app).as_deref(), Some("The  ate"));
        type_in(&mut app, egui::Event::Paste("walrus".into()));
        assert_eq!(typed(&mut app).as_deref(), Some("The walrus ate"));
        // From the line's start to a click by its end, Shift held.
        let none = egui::Modifiers::NONE;
        type_in(&mut app, key_with(Key::Home, none));
        let area = app.tabs[0].ed.line_area(0).expect("the line's place");
        let end = area.right_center() - egui::vec2(8.0, 0.0);
        let mut click = vec![egui::Event::PointerMoved(end)];
        click.extend([press(end, true), press(end, false)]);
        window_held(&mut app, &ctx, click, egui::Modifiers::SHIFT);
        window_held(&mut app, &ctx, vec![], none);
        type_in(&mut app, egui::Event::Text("Seal".into()));
        assert_eq!(typed(&mut app).as_deref(), Some("Seal"));
        // Ctrl+A: all of the line, not the page's text.
        type_in(&mut app, key_with(Key::A, CTRL));
        type_in(&mut app, egui::Event::Text("Walrus".into()));
        assert_eq!(typed(&mut app).as_deref(), Some("Walrus"));
        let words = app.tabs[0].ed.selection.words();
        assert!(words.is_none(), "the page's words selected");
        window(&mut app, &ctx, 1400.0, vec![key_with(Key::Escape, none)]);
        settled_at(&mut app, &ctx, 1400.0);
        let mut want = before.clone();
        want[1] = "Walrus".into();
        assert_eq!(line_texts(&app.tabs[0].pdf, 0), want);
    }

    /// What is typed in a line is never lost clicking away and back: a click on the page
    /// away from it, or on a page in the side panel, keeps it, and a double-click edits it
    /// again (another tab and back: `typing_left_in_another_tab_has_the_keyboard_again_on_return`).
    #[test]
    fn a_line_typed_in_is_kept_clicking_away_and_back() {
        use egui::accesskit::Role;
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        theme::install(&ctx);
        let mut app = App::with(Settings::from_file(None), Recent::from_file(None));
        lines_tab(&mut app, &ctx, 400.0, &paragraph(72.0, 100.0, 3, 1, false));
        app.show_view(Side::Pages);
        let ty = Typist::new(&ctx);
        let second = |app: &App| line_texts(&app.tabs[0].pdf, 0)[1].clone();
        // Double-click `word` to edit its line, as a person does.
        let edit = |app: &mut App, word: &str| {
            let (min, k) = ty.page(app);
            let words = app.tabs[0].pdf.page_words(0).unwrap();
            let b = words.iter().find(|w| w.text == word).expect(word).bbox;
            let p = min + egui::vec2((b[0] + b[2]) / 2.0, (b[1] + b[3]) / 2.0) * k;
            ty.frame(app, click_at(p));
            ty.frame(app, click_at(p));
            ty.pause(app);
            assert!(app.tabs[0].ed.editing_line(), "{word} isn't edited");
        };
        edit(&mut app, "otter");
        ty.text(&mut app, "walrus");
        // Away on the page, below the text.
        let (min, k) = ty.page(&mut app);
        ty.click(&mut app, min + egui::vec2(300.0, 300.0) * k);
        assert!(!app.tabs[0].ed.editing_line(), "still edited");
        assert_eq!(second(&app), "The walrus ate");
        // Back, by a double-click on the word typed.
        edit(&mut app, "walrus");
        ty.text(&mut app, "seal");
        let tree = ty.pause(&mut app);
        assert_eq!(focused_text(&tree).as_deref(), Some("The seal ate"));
        // Away to the page in the side panel: kept, as a click on the page keeps it.
        let thumb = (tree.nodes.iter())
            .find(|(_, n)| n.role() == Role::Button && n.label() == Some("Page 1"))
            .and_then(|(_, n)| n.bounds())
            .expect("the thumbnail");
        let (x, y) = ((thumb.x0 + thumb.x1) / 2.0, (thumb.y0 + thumb.y1) / 2.0);
        ty.click(&mut app, egui::pos2(x as f32, y as f32));
        assert!(!app.tabs[0].ed.editing_line(), "still edited");
        assert_eq!(second(&app), "The seal ate");
        edit(&mut app, "seal");
    }

    /// A line being edited is kept by Save, and the file read back has the words typed in
    /// that line and everything else on the page as it was: the other lines' words, fonts
    /// and places.
    #[test]
    fn a_line_edit_saved_and_reopened_changes_only_that_line() {
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        theme::install(&ctx);
        let dir = test_dir("line-saved");
        let path = dir.join("lines.pdf");
        let mut pdf = Pdf::new();
        pdf.insert_blank(0, 612.0, 400.0).unwrap();
        for (text, at) in paragraph(72.0, 100.0, 3, 1, false) {
            let font = FontChoice::default();
            pdf.add_text(0, (at.x, at.y), &text, font, 12.0, [0.0; 3])
                .unwrap();
        }
        pdf.save(&path).unwrap();
        let mut app = App::with(Settings::from_file(None), Recent::from_file(None));
        open_and_wait(&mut app, &ctx, &path, 1);
        app.act(&ctx, Act::Tool(Tool::Read));
        // Each line but the edited one: its words, font and place.
        let others = |pdf: &Pdf| {
            let objs = pdf.page_objects(0).unwrap().into_iter();
            let lines = objs.filter(|o| o.text.as_deref().is_some_and(|t| !t.contains("otter")));
            let lines = lines.filter(|o| o.text.as_deref().is_some_and(|t| !t.contains("walrus")));
            let line = |o: limarepdf_core::PageObject| {
                let at = o.bbox.map(|v| (v * 100.0).round() as i64);
                (o.text, o.font_name, at)
            };
            lines.map(line).collect::<Vec<_>>()
        };
        let before = others(&app.tabs[0].pdf);
        assert_eq!(before.len(), 2);
        edit_otter(&mut app, &ctx, 1400.0, "saved");
        let walrus = vec![egui::Event::Text("walrus".into())];
        window(&mut app, &ctx, 1400.0, walrus);
        settled_at(&mut app, &ctx, 1400.0);
        app.act(&ctx, Act::Save);
        saves_done(&mut app, &ctx);
        let back = Pdf::from_bytes(&std::fs::read(&path).unwrap(), None).unwrap();
        let _ = std::fs::remove_dir_all(&dir);
        let want = ["The seal ate", "The walrus ate", "The seal ate"];
        assert_eq!(line_texts(&back, 0), want);
        assert_eq!(others(&back), before);
        assert!(!app.tabs[0].unsaved(), "unsaved after saving");
    }

    /// A triple-click on a line, as in a word processor, takes the whole line, by the tool
    /// in use: the read tool selects its words, the arrow the line, and the Text tool opens
    /// it with all of it selected, so typing replaces it.
    #[test]
    fn a_triple_click_takes_the_line_with_each_tool() {
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        theme::install(&ctx);
        for tool in [Tool::Read, Tool::Select, Tool::Text] {
            let mut app = App::with(Settings::from_file(None), Recent::from_file(None));
            lines_tab(&mut app, &ctx, 400.0, &paragraph(72.0, 100.0, 3, 1, false));
            app.act(&ctx, Act::Tool(tool));
            let words = app.tabs[0].pdf.page_words(0).unwrap();
            let otter = words.iter().position(|w| w.text == "otter").expect("otter");
            // The words of otter's line.
            let line: Vec<usize> = (words.iter().enumerate())
                .filter(|(_, w)| (w.bbox[1] - words[otter].bbox[1]).abs() < 1.0)
                .map(|(i, _)| i)
                .collect();
            assert_eq!(line.len(), 3);
            let b = words[line[1]].bbox;
            let at = egui::vec2((b[0] + b[2]) / 2.0, (b[1] + b[3]) / 2.0);
            clicks_on_page(&mut app, &ctx, at, 3);
            let ed = &app.tabs[0].ed;
            match tool {
                Tool::Read => {
                    let all = ((0, line[0]), (0, line[2]));
                    assert_eq!(ed.selection.words(), Some(all), "{tool:?}");
                }
                Tool::Select => {
                    assert_eq!(ed.selected_line_text(), ["The otter ate"], "{tool:?}");
                    assert!(!ed.editing_line(), "{tool:?}");
                }
                _ => {
                    assert!(ed.editing_line(), "{tool:?}");
                    let seal = vec![egui::Event::Text("Seal".into())];
                    window(&mut app, &ctx, 1400.0, seal);
                    let tree = settled_at(&mut app, &ctx, 1400.0);
                    assert_eq!(focused_text(&tree).as_deref(), Some("Seal"), "{tool:?}");
                }
            }
        }
    }

    /// ENH-20: the floating bar of a line being edited keeps clear of it, next to it: above,
    /// below or beside it where that covers no other text, else just above or below it,
    /// over as little text as can be (the user's finding: by a heading with text all round
    /// it, it went to the top of the view); never over the line (and its caret); at the
    /// page's top, bottom and edges, at three zooms and in a narrow window.
    #[test]
    fn the_floating_bar_keeps_clear_of_the_line_being_edited() {
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        theme::install(&ctx);
        let cases = [
            "paragraph",
            "paragraph at half size",
            "paragraph at twice the size",
            "narrow window",
            "heading among columns",
            "top of the page",
            "bottom of the page",
            "left edge",
            "right edge",
        ];
        // A heading in the middle of three columns, a line across the page just above it
        // and two lines of its column below it.
        let mut columns = vec![(
            "For questions about the walrus and the seal see the shore".to_string(),
            egui::pos2(72.0, 285.0),
        )];
        for (x, y, who) in [(72.0, 300.0, "walrus"), (250.0, 300.0, "otter")] {
            columns.push((format!("The {who} ate"), egui::pos2(x, y)));
        }
        for (x, y) in [(330.0, 300.0), (250.0, 315.0), (250.0, 330.0)] {
            columns.push(("The walrus ate".to_string(), egui::pos2(x, y)));
        }
        for case in cases {
            // The page's height and lines: a paragraph across it, short lines at an edge, or
            // the columns.
            let (h, lines) = match case {
                "heading among columns" => (792.0, columns.clone()),
                "top of the page" => (792.0, paragraph(72.0, 14.0, 3, 0, true)),
                "bottom of the page" => (120.0, paragraph(72.0, 75.0, 3, 2, true)),
                "left edge" => (792.0, paragraph(3.0, 300.0, 3, 1, false)),
                "right edge" => (792.0, paragraph(538.0, 300.0, 3, 1, false)),
                _ => (792.0, paragraph(72.0, 300.0, 9, 4, true)),
            };
            let mut app = App::with(Settings::from_file(None), Recent::from_file(None));
            lines_tab(&mut app, &ctx, h, &lines);
            match case {
                "paragraph at half size" => app.tabs[0].set_zoom(0.5),
                "paragraph at twice the size" => app.tabs[0].set_zoom(2.0),
                _ => {}
            }
            let narrow = case == "narrow window";
            let width = if narrow { 700.0 } else { 1400.0 };
            // The paragraph's middle in view, or the page from its top.
            let top = match case {
                "paragraph" | "paragraph at twice the size" => 250.0,
                "left edge" | "right edge" | "heading among columns" => 250.0,
                _ => 0.0,
            };
            // Narrow, the panels fold away over some frames.
            for _ in 0..40 {
                window(&mut app, &ctx, width, vec![]);
            }
            app.tabs[0].go_to_at(0, top);
            let (tree, k) = edit_otter(&mut app, &ctx, width, case);
            let buttons = bar_buttons(&tree);
            let bar = (buttons.iter().map(|(_, r)| *r))
                .reduce(|a, b| a.union(b))
                .unwrap_or_else(|| panic!("{case}: no bar"));
            let (page, _) = node(&tree, "Page 1").expect("the page");
            let mut objs = app.tabs[0].pdf.page_objects(0).unwrap();
            objs.retain(|o| o.text.is_some());
            objs.sort_by(|a, b| a.bbox[1].total_cmp(&b.bbox[1]));
            let screen = |b: [f32; 4]| {
                egui::Rect::from_min_max(
                    page.min + egui::vec2(b[0], b[1]) * k,
                    page.min + egui::vec2(b[2], b[3]) * k,
                )
            };
            let i = (objs.iter())
                .position(|o| o.text.as_deref().is_some_and(|t| t.contains("otter")))
                .expect("the otter line");
            let name = format!("Editing text: {}", objs[i].text.as_deref().unwrap_or(""));
            let (editor, _) = node(&tree, &name).expect("the line editor");
            let line = screen(objs[i].bbox).union(editor);
            assert!(!bar.intersects(line), "{case}: over the line, {bar:?}");
            // Next to it: the bar's frame round its buttons and the line's outline add some
            // points to the gap.
            let apart = (bar.min.y - line.max.y)
                .max(line.min.y - bar.max.y)
                .max(bar.min.x - line.max.x)
                .max(line.min.x - bar.max.x);
            assert!(apart < 24.0, "{case}: {apart} from the line, {bar:?}");
            // With room by the line, over no other line.
            let room = ["bottom of the page", "left edge", "right edge"];
            if room.contains(&case) {
                for (j, o) in objs.iter().enumerate().filter(|(j, _)| *j != i) {
                    let r = screen(o.bbox);
                    assert!(!bar.intersects(r), "{case}: over line {j}, {bar:?}");
                }
            }
            let shown = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(width, 1000.0));
            assert!(shown.contains_rect(bar), "{case}: out of view, {bar:?}");
        }
    }

    /// ENH-20: while a line is edited on the page, its floating bar has Bold and Italic for
    /// what is selected in it, as Ctrl+B and Ctrl+I there, and a click on them leaves the
    /// line open with the keyboard back in it.
    #[test]
    fn the_floating_bar_styles_the_line_being_edited_and_keeps_it_open() {
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        theme::install(&ctx);
        let mut app = App::with(Settings::from_file(None), Recent::from_file(None));
        lines_tab(&mut app, &ctx, 792.0, &paragraph(72.0, 300.0, 3, 1, true));
        let (tree, _) = edit_otter(&mut app, &ctx, 1400.0, "otter");
        let typing = app.tabs[0].ed.typing_id().expect("typing");
        let mut tree = tree;
        for name in ["Bold", "Italic"] {
            // Where the bar is now: the line, bolder, may move it.
            let buttons = bar_buttons(&tree);
            let names: Vec<&str> = buttons.iter().map(|(n, _)| n.as_str()).collect();
            let r = (buttons.iter())
                .find(|(n, _)| n == name)
                .unwrap_or_else(|| panic!("no {name} in the bar: {names:?}"))
                .1;
            window(&mut app, &ctx, 1400.0, click_at(r.center()));
            tree = settled_at(&mut app, &ctx, 1400.0);
            assert_eq!(app.tabs[0].ed.typing_id(), Some(typing), "{name}: ended");
            assert_eq!(
                ctx.memory(|m| m.focused()),
                Some(typing),
                "{name}: the keyboard isn't back"
            );
        }
        let esc = key_with(Key::Escape, egui::Modifiers::NONE);
        window(&mut app, &ctx, 1400.0, vec![esc]);
        settled_at(&mut app, &ctx, 1400.0);
        let lines = line_fonts(&app.tabs[0].pdf, 0);
        let otter = (lines.iter().flatten())
            .find(|(t, _)| t.contains("otter"))
            .expect("the otter line");
        assert_eq!(otter.0.trim(), "otter", "{lines:?}");
        assert_eq!(otter.1, "Helvetica-BoldOblique", "{lines:?}");
    }

    /// A click on the inspector's heading, or on its Collapse button, leaves a line being
    /// edited on the page open with the keyboard back in it, as a panel closing in a word
    /// processor leaves the typing open: the heading is part of the inspector's place. So
    /// are the collapsed inspector's strip and its pop-out, heading and all.
    #[test]
    fn the_inspectors_heading_leaves_a_line_edit_open() {
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        theme::install(&ctx);
        let mut app = App::with(Settings::from_file(None), Recent::from_file(None));
        lines_tab(&mut app, &ctx, 792.0, &paragraph(72.0, 300.0, 3, 1, true));
        let (tree, _) = edit_otter(&mut app, &ctx, 1400.0, "otter");
        let typing = app.tabs[0].ed.typing_id().expect("typing");
        let (collapse, _) = node(&tree, "Collapse the inspector").expect("the collapse button");
        // The heading's title, left of the button on its row.
        let title = egui::pos2(collapse.min.x - 80.0, collapse.center().y);
        let heading = inspector::title(&app.tabs[0]);
        let options = "Options for the selection or the document";
        let mut tree = tree;
        // The heading, its button, the strip's Options (the pop-out) and its heading.
        for what in ["heading", "Collapse", "Options", "pop-out"] {
            let at = match what {
                "heading" => title,
                "Collapse" => collapse.center(),
                "Options" => node(&tree, options).expect("Options").0.center(),
                _ => {
                    // A label's text is its node's value; the status bar's may say it too,
                    // at the window's left.
                    let b = (tree.nodes.iter())
                        .filter(|(_, n)| n.value() == Some(heading.as_str()))
                        .filter_map(|(_, n)| n.bounds())
                        .max_by(|a, b| a.x0.total_cmp(&b.x0))
                        .expect("the pop-out's heading");
                    egui::pos2((b.x0 + b.x1) as f32 / 2.0, (b.y0 + b.y1) as f32 / 2.0)
                }
            };
            window(&mut app, &ctx, 1400.0, click_at(at));
            // The inspector folds away over more frames than one settling.
            settled_at(&mut app, &ctx, 1400.0);
            tree = settled_at(&mut app, &ctx, 1400.0);
            assert_eq!(app.tabs[0].ed.typing_id(), Some(typing), "{what}: ended");
            assert_eq!(
                ctx.memory(|m| m.focused()),
                Some(typing),
                "{what}: the keyboard isn't back"
            );
        }
        assert!(!app.inspector.shown(), "not collapsed");
        assert!(app.popout, "the pop-out closed");
        // A click on the page beside the pop-out, at its height, is a click away: it ends
        // the line edit, keeping what was typed.
        let (strip, _) = node(&tree, options).unwrap();
        let away = egui::pos2(300.0, strip.center().y + 60.0);
        window(&mut app, &ctx, 1400.0, click_at(away));
        settled_at(&mut app, &ctx, 1400.0);
        let typing = app.tabs[0].ed.typing_id();
        assert_eq!(typing, None, "a click away kept it open");
    }

    /// `n` clicks in a row at `at` (page points) on page 1 of the active tab, in a window
    /// 1400 points wide: two are a double-click, three a triple-click.
    fn clicks_on_page(app: &mut App, ctx: &egui::Context, at: egui::Vec2, n: usize) {
        let tree = settled_at(app, ctx, 1400.0);
        let (page, _) = node(&tree, "Page 1").expect("the page");
        let p = page.min + at * (page.width() / 612.0);
        window(app, ctx, 1400.0, vec![egui::Event::PointerMoved(p)]);
        for _ in 0..n {
            window(app, ctx, 1400.0, vec![press(p, true)]);
            window(app, ctx, 1400.0, vec![press(p, false)]);
        }
        settled_at(app, ctx, 1400.0);
    }

    /// A double-click or triple-click with the read tool off the text, below it or in the
    /// margin beside it, selects nothing, as in a word processor: not the nearest word or
    /// line, however far away.
    #[test]
    fn clicks_off_the_text_select_no_far_word_or_line() {
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        theme::install(&ctx);
        for (case, at, n) in [
            ("double-click below", egui::vec2(300.0, 300.0), 2),
            ("triple-click below", egui::vec2(300.0, 300.0), 3),
            ("double-click in the margin", egui::vec2(20.0, 115.0), 2),
            ("triple-click in the margin", egui::vec2(20.0, 115.0), 3),
        ] {
            let mut app = App::with(Settings::from_file(None), Recent::from_file(None));
            lines_tab(&mut app, &ctx, 792.0, &paragraph(72.0, 100.0, 3, 1, true));
            clicks_on_page(&mut app, &ctx, at, n);
            let ed = &app.tabs[0].ed;
            assert_eq!(ed.selection, Selection::None, "{case}");
            assert_eq!(ed.typing_id(), None, "{case}");
        }
    }

    /// Ctrl+A with the Text tool, nothing being typed, selects the page's text as with the
    /// read tool (the Text tool selects words with a drag too); Ctrl+C copies it.
    #[test]
    fn ctrl_a_with_the_text_tool_selects_the_pages_text() {
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        theme::install(&ctx);
        let mut app = App::with(Settings::from_file(None), Recent::from_file(None));
        lines_tab(&mut app, &ctx, 792.0, &paragraph(72.0, 100.0, 3, 1, true));
        app.act(&ctx, Act::Tool(Tool::Text));
        settled_at(&mut app, &ctx, 1400.0);
        window(&mut app, &ctx, 1400.0, vec![key_with(Key::A, CTRL)]);
        settled_at(&mut app, &ctx, 1400.0);
        let n = app.tabs[0].pdf.page_words(0).unwrap().len();
        let all = Selection::Words {
            anchor: (0, 0),
            end: (0, n - 1),
        };
        assert_eq!(app.tabs[0].ed.selection, all);
        assert_eq!(app.tabs[0].ed.tool, Tool::Text);
        let text = app.tabs[0].ed.selected_text().expect("the text");
        let all = text.starts_with("The walrus ate") && text.contains("otter");
        assert!(all, "{text}");
    }

    /// An app with a tab of two pages, each with the three lines of `paragraph(72.0, 100.0,
    /// 3, 1, true)`, 16 words a line (words 16 to 31 of each page are its line saying
    /// "otter"), and the read tool, drawn and settled.
    fn two_pages_tab(ctx: &egui::Context) -> App {
        ctx.enable_accesskit();
        theme::install(ctx);
        let mut app = App::with(Settings::from_file(None), Recent::from_file(None));
        let mut pdf = Pdf::new();
        for page in 0..2 {
            pdf.insert_blank(page, 612.0, 792.0).unwrap();
            for (text, at) in paragraph(72.0, 100.0, 3, 1, true) {
                let font = FontChoice::default();
                pdf.add_text(page, (at.x, at.y), &text, font, 12.0, [0.0; 3])
                    .unwrap();
            }
        }
        assert!(app.add_tab(ctx, pdf, None, None, false));
        app.act(ctx, Act::Tool(Tool::Read));
        settled_at(&mut app, ctx, 1400.0);
        app
    }

    /// Each line of page text on `page` of `pdf`, top to bottom, as it reads.
    fn line_texts(pdf: &Pdf, page: usize) -> Vec<String> {
        let mut objs = pdf.page_objects(page).unwrap();
        objs.retain(|o| o.text.is_some());
        objs.sort_by(|a, b| a.bbox[1].total_cmp(&b.bbox[1]));
        objs.into_iter().filter_map(|o| o.text).collect()
    }

    /// The lines of [`two_pages_tab`]'s pages as they read with the words `cut` (the first
    /// and last, by line and word on its page: page 1's lines 0 to 2, page 2's 3 to 5)
    /// deleted as a word processor deletes a selection, one space beside them going too.
    fn two_pages_without(cut: ((usize, usize), (usize, usize))) -> [Vec<String>; 2] {
        let ((l0, w0), (l1, w1)) = cut;
        let mut lines: Vec<Vec<String>> = (0..6)
            .map(|i| {
                let text = &paragraph(72.0, 100.0, 3, 1, true)[i % 3].0;
                text.split(' ').map(String::from).collect()
            })
            .collect();
        for (l, words) in lines.iter_mut().enumerate() {
            let from = if l == l0 { w0 } else { 0 };
            let to = if l == l1 { w1 + 1 } else { words.len() };
            if l >= l0 && l <= l1 {
                words.drain(from..to);
            }
        }
        let text = |l: &[Vec<String>]| -> Vec<String> {
            (l.iter().filter(|w| !w.is_empty()))
                .map(|w| w.join(" "))
                .collect()
        };
        [text(&lines[..3]), text(&lines[3..])]
    }

    /// Delete or Backspace with words selected with the read tool takes them out of their
    /// lines, as a word processor deletes a selection: across lines and pages in one undo
    /// step, with one space beside them where two would be left together or one at a line's
    /// start or end (as Word's smart cut has it); a line emptied goes. Nothing is selected
    /// after; the same once saved; one Undo puts it all back.
    #[test]
    fn delete_takes_words_selected_with_the_read_tool_out() {
        let ctx = egui::Context::default();
        let (del, back) = (Key::Delete, Key::Backspace);
        // The words: their first and last by page and index, and by line and index there.
        // Across lines of a paragraph LimarePDF is sure of, they are taken out in it, which
        // rewraps (`tests/paragraphs.rs`); across pages, from each line, which stay apart.
        for (case, key, anchor, end, cut) in [
            ("a word", del, (0, 1), (0, 1), ((0, 1), (0, 1))),
            ("a whole line", back, (0, 16), (0, 31), ((1, 0), (1, 15))),
            ("across pages", del, (0, 46), (1, 1), ((2, 14), (3, 1))),
        ] {
            let mut app = two_pages_tab(&ctx);
            let before = [0, 1].map(|p| line_texts(&app.tabs[0].pdf, p));
            app.tabs[0].ed.selection = Selection::Words { anchor, end };
            settled_at(&mut app, &ctx, 1400.0);
            let undo = app.tabs[0].history().1;
            let key = key_with(key, egui::Modifiers::NONE);
            window(&mut app, &ctx, 1400.0, vec![key]);
            settled_at(&mut app, &ctx, 1400.0);
            let want = two_pages_without(cut);
            let t = &app.tabs[0];
            let now = [0, 1].map(|p| line_texts(&t.pdf, p));
            assert_eq!(now, want, "{case}");
            assert_eq!(t.ed.selection, Selection::None, "{case}");
            assert_eq!(t.history().1, undo + 1, "{case}: not one step");
            let saved = saved_pdf(t);
            assert_eq!([0, 1].map(|p| line_texts(&saved, p)), want, "{case}: saved");
            window(&mut app, &ctx, 1400.0, vec![key_with(Key::Z, CTRL)]);
            settled_at(&mut app, &ctx, 1400.0);
            let back = [0, 1].map(|p| line_texts(&app.tabs[0].pdf, p));
            assert_eq!(back, before, "{case}: Undo");
        }
    }

    /// A frame of the window 1400 points wide with `events`, and the text it copied.
    fn copied_by(app: &mut App, ctx: &egui::Context, events: Vec<egui::Event>) -> Vec<String> {
        let raw = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(1400.0, TEST_HEIGHT),
            )),
            events,
            ..Default::default()
        };
        let mut f = eframe::Frame::_new_kittest();
        let mut out = ctx.run_ui(raw, |ui| eframe::App::ui(app, ui, &mut f));
        out.textures_delta.clear();
        (out.platform_output.commands.into_iter())
            .filter_map(|c| match c {
                egui::OutputCommand::CopyText(text) => Some(text),
                _ => None,
            })
            .collect()
    }

    /// Ctrl+X (the system's Cut) with words selected with the read tool copies them and
    /// takes them out, as Delete does; with lines of page text selected with the arrow, it
    /// copies their text and deletes them. One undo step each.
    #[test]
    fn cut_copies_and_takes_out_words_or_lines() {
        let ctx = egui::Context::default();
        for case in ["words", "lines"] {
            let mut app = two_pages_tab(&ctx);
            let (want, copied) = match case {
                "words" => {
                    app.tabs[0].ed.selection = Selection::Words {
                        anchor: (0, 1),
                        end: (0, 2),
                    };
                    (two_pages_without(((0, 1), (0, 2))), "walrus ate")
                }
                _ => {
                    app.act(&ctx, Act::Tool(Tool::Select));
                    let objs = app.tabs[0].pdf.page_objects(0).unwrap();
                    let otter = |o: &limarepdf_core::PageObject| {
                        o.text.as_deref().is_some_and(|t| t.contains("otter"))
                    };
                    let id = objs.iter().position(otter).expect("the otter line");
                    select(&mut app, &ctx, &[id]);
                    let line = "The otter ate fish by the sea and the seal ate fish by the shore 1";
                    (two_pages_without(((1, 0), (1, 15))), line)
                }
            };
            settled_at(&mut app, &ctx, 1400.0);
            let undo = app.tabs[0].history().1;
            let said = copied_by(&mut app, &ctx, vec![egui::Event::Cut]);
            settled_at(&mut app, &ctx, 1400.0);
            assert_eq!(said, [copied], "{case}");
            let t = &app.tabs[0];
            assert_eq!([0, 1].map(|p| line_texts(&t.pdf, p)), want, "{case}");
            assert_eq!(t.history().1, undo + 1, "{case}: not one step");
        }
    }

    /// Typing with words selected with the read tool in one line types over them, as in a
    /// word processor: their line opens for editing on the page, what is typed in their
    /// place, the keys after it going on in the line; Ctrl+V pastes over them the same way.
    /// Esc keeps it, one Undo takes it back. Across paragraphs (here pages) nothing is
    /// typed; a note at the selection's end says why. (Across lines of one paragraph it is
    /// typed there: `tests/paragraphs.rs`.)
    #[test]
    fn typing_over_words_selected_with_the_read_tool_replaces_them() {
        let ctx = egui::Context::default();
        let otter = "The otter ate fish by the sea and the seal ate fish by the shore 1";
        let seal = "The seal fish by the sea and the seal ate fish by the shore 1";
        for case in ["typed", "pasted", "across paragraphs"] {
            let mut app = two_pages_tab(&ctx);
            let (anchor, end) = match case {
                "across paragraphs" => ((0, 46), (1, 1)),
                // "otter ate".
                _ => ((0, 17), (0, 18)),
            };
            app.tabs[0].ed.selection = Selection::Words { anchor, end };
            settled_at(&mut app, &ctx, 1400.0);
            let undo = app.tabs[0].history().1;
            match case {
                "pasted" => {
                    let paste = egui::Event::Paste("seal".into());
                    window(&mut app, &ctx, 1400.0, vec![paste]);
                }
                _ => {
                    for c in ["s", "e", "a", "l"] {
                        window(&mut app, &ctx, 1400.0, vec![egui::Event::Text(c.into())]);
                    }
                }
            }
            let tree = settled_at(&mut app, &ctx, 1400.0);
            // The note at the selection's end is on the second page, out of view here
            // (`tests/paragraphs.rs` reads it on one page).
            if case == "across paragraphs" {
                let t = &app.tabs[0];
                assert_eq!(t.ed.typing_id(), None, "{case}: typing");
                assert_eq!(t.ed.selection, Selection::Words { anchor, end }, "{case}");
                assert_eq!(line_texts(&t.pdf, 0)[1], otter, "{case}");
                continue;
            }
            // Named by the line as it is in the document.
            let name = format!("Editing text: {otter}");
            let (_, focused) = node(&tree, &name).unwrap_or_else(|| panic!("{case}: not open"));
            assert!(focused, "{case}: the line hasn't the keyboard");
            // The caret after what was typed.
            let id = app.tabs[0].ed.typing_id().expect("typing");
            assert_eq!(page_text::selection(&ctx, id), Some((8, 8)), "{case}");
            let esc = key_with(Key::Escape, egui::Modifiers::NONE);
            window(&mut app, &ctx, 1400.0, vec![esc]);
            settled_at(&mut app, &ctx, 1400.0);
            let t = &app.tabs[0];
            assert_eq!(t.ed.typing_id(), None, "{case}: still typing");
            assert_eq!(line_texts(&t.pdf, 0)[1], seal, "{case}");
            assert_eq!(t.history().1, undo + 1, "{case}: not one step");
            window(&mut app, &ctx, 1400.0, vec![key_with(Key::Z, CTRL)]);
            settled_at(&mut app, &ctx, 1400.0);
            assert_eq!(line_texts(&app.tabs[0].pdf, 0)[1], otter, "{case}: Undo");
        }
    }

    /// Edit text with words selected with the read tool edits their line on the page with
    /// them selected there; across lines of a paragraph LimarePDF is sure of, the
    /// paragraph, with them all selected in it, whichever way they were selected.
    #[test]
    fn edit_text_edits_the_line_of_the_words_selected() {
        let ctx = egui::Context::default();
        let para = "Editing a paragraph: The walrus ate fish by the sea and";
        for (case, anchor, end, name, chars) in [
            // "otter ate".
            ("one line", (0, 17), (0, 18), None, (4, 13)),
            // "shore 0 The otter", in the paragraph's text.
            ("across lines", (0, 14), (0, 17), Some(para), (60, 77)),
            // From "otter" back to "shore".
            ("backwards", (0, 17), (0, 14), Some(para), (60, 77)),
        ] {
            let mut app = two_pages_tab(&ctx);
            app.tabs[0].ed.selection = Selection::Words { anchor, end };
            settled_at(&mut app, &ctx, 1400.0);
            app.act(&ctx, Act::EditObjectText);
            let tree = settled_at(&mut app, &ctx, 1400.0);
            let text = &paragraph(72.0, 100.0, 3, 1, true)[1].0;
            let name = name.map_or(format!("Editing text: {text}"), str::to_string);
            let (_, focused) = node(&tree, &name).unwrap_or_else(|| panic!("{case}: not editing"));
            assert!(focused, "{case}: the line hasn't the keyboard");
            let id = app.tabs[0].ed.typing_id().expect("typing");
            let held = page_text::selection(&ctx, id).map(|(a, b)| (a.min(b), a.max(b)));
            assert_eq!(held, Some(chars), "{case}");
        }
    }

    /// Shift with an arrow, Home or End moves the moving end of words selected with the read
    /// tool, as a word processor extends a selection: a word left or right in reading
    /// order, across pages; to the word nearest across on the line below or above; to its
    /// line's start or end. Shift+Home and Shift+End no longer go to the first or last page.
    #[test]
    fn shift_keys_extend_words_selected_with_the_read_tool() {
        let ctx = egui::Context::default();
        let mut app = two_pages_tab(&ctx);
        // "otter".
        app.tabs[0].ed.selection = Selection::word((0, 17));
        settled_at(&mut app, &ctx, 1400.0);
        let shift = egui::Modifiers::SHIFT;
        for (key, end) in [
            (Key::ArrowRight, (0, 18)),
            (Key::ArrowLeft, (0, 17)),
            (Key::ArrowLeft, (0, 16)),
            (Key::ArrowDown, (0, 32)),
            (Key::ArrowUp, (0, 16)),
            (Key::ArrowUp, (0, 0)),
            (Key::End, (0, 15)),
            (Key::Home, (0, 0)),
            (Key::End, (0, 15)),
            (Key::ArrowDown, (0, 31)),
            (Key::ArrowDown, (0, 47)),
            (Key::ArrowRight, (1, 0)),
            (Key::ArrowLeft, (0, 47)),
            (Key::ArrowDown, (1, 15)),
            (Key::ArrowUp, (0, 47)),
        ] {
            let page = app.tabs[0].current;
            window(&mut app, &ctx, 1400.0, vec![key_with(key, shift)]);
            let tree = settled_at(&mut app, &ctx, 1400.0);
            let want = Selection::Words {
                anchor: (0, 17),
                end,
            };
            assert_eq!(app.tabs[0].ed.selection, want, "Shift+{key:?}");
            if matches!(key, Key::Home | Key::End) {
                let now = app.tabs[0].current;
                assert_eq!(now, page, "Shift+{key:?} went to another page");
            }
            // The view follows the end, as a word processor's follows its caret: the page is
            // shown, not only its thumbnail.
            let name = format!("Page {}", end.0 + 1);
            let shown = node(&tree, &name).filter(|(r, _)| r.width() > 400.0);
            let (shown, _) = shown.unwrap_or_else(|| panic!("Shift+{key:?}: {name} not shown"));
            let k = shown.width() / 612.0;
            let b = app.tabs[0].pdf.page_words(end.0).unwrap()[end.1].bbox;
            let (top, bottom) = (shown.min.y + b[1] * k, shown.min.y + b[3] * k);
            let view = 150.0..TEST_HEIGHT - 40.0;
            let seen = view.contains(&top) && view.contains(&bottom);
            assert!(seen, "Shift+{key:?}: the end is out of view at {top}");
        }
    }
}

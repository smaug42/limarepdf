use crate::actions::{self, Act, Ctx};
use crate::banners::{self, Banner};
use crate::editor::{annot_tip, Action, Editor, StampPick, Tool};
use crate::inspector::{Forms, Sec};
use crate::markup::{self, Mark};
use crate::selection::Selection;
use crate::settings::Layout;
use crate::signatures::{self, Check, PanelAct, SigState};
use crate::signing::SignPick;
use crate::theme;
use crate::worker::{Colours, Key, Worker, TILE};
use eframe::egui::{self, Color32, Pos2, Rect, Sense, TextureHandle, TextureOptions, Vec2};
use egui_phosphor::regular as icon;
use limarepdf_core::render::{Missing, MAX_REGION_SCALE};
use limarepdf_core::{
    Annot, BookmarkPlace, Encryption, LinkTarget, OutlineItem, Pdf, SearchHit, Shape, ShapeLook,
    StampPlace,
};
use std::collections::{BTreeSet, HashMap, HashSet};
use std::path::PathBuf;

const GAP: f32 = 16.0;
/// Room left beside a fitted page, beyond the gaps, so it never meets the scroll bar.
const FIT_MARGIN: f32 = 12.0;
const THUMB_W: f32 = 120.0;
const MAX_UNDO: usize = 20;
/// egui points at zoom 1.0 per PDF point (96 dpi logical).
const BASE: f32 = 96.0 / 72.0;
/// Highest zoom: 6400%, as Acrobat.
const MAX_ZOOM: f32 = 64.0;
/// The longest side of a whole-page bitmap, in pixels. A page shown bigger than this is
/// drawn whole at this size, with tiles at full size over the part in view.
const WHOLE_MAX: f32 = 4096.0;

#[derive(PartialEq, Eq, Clone, Copy)]
pub enum Side {
    Pages,
    Search,
    Bookmarks,
    Comments,
    Signatures,
}

/// One undo or redo step: the state to go back to.
struct Step {
    /// The document's bytes; `None` when only the password settings changed.
    pdf: Option<Vec<u8>>,
    encryption: Option<Encryption>,
    scrubbed: bool,
}

/// Inspector edits that applying again replaces, while nothing else has changed since.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Stamp {
    Watermark,
    HeaderFooter,
}

pub struct Tab {
    pub id: u64,
    pub pdf: Pdf,
    pub path: Option<PathBuf>,
    pub dirty: bool,
    /// Bumped on every change to `pdf`, so background work can tell whether its input
    /// is still current.
    pub revision: u64,
    pub encryption: Option<Encryption>,
    /// The file holds digital signatures, so saving adds the edits after what was signed;
    /// cleared once a save had to write the file out in full, breaking them.
    pub signed: bool,
    /// Redactions were applied or hidden information removed since the file was opened
    /// or last saved. Saving then writes the file out in full, so that nothing removed is
    /// left in an earlier version of it. Follows undo and redo.
    pub scrubbed: bool,
    worker: Worker,
    /// The colours pages are drawn in on screen.
    colours: Colours,
    pub sizes: Vec<(f32, f32)>,
    pages: HashMap<usize, (u32, TextureHandle)>,
    /// At high zoom, the parts of the pages in view drawn at full size, over `pages`. The
    /// flag is cleared when a tile goes out of date; it is still shown until the new one
    /// arrives.
    tiles: HashMap<Key, (bool, TextureHandle)>,
    /// Per page, the tile scale whose tiles last covered the view. They stay under the
    /// tiles of a new zoom until those cover it.
    covered: HashMap<usize, u32>,
    thumbs: HashMap<usize, TextureHandle>,
    pending: HashSet<Key>,
    /// Pages that couldn't be drawn, and why; not asked for again until the document
    /// changes.
    failed: HashMap<usize, String>,
    /// Tiles that couldn't be drawn; not asked for again until the document changes.
    bad_tiles: HashSet<Key>,
    /// What the renderer couldn't draw so far, for the banner.
    missing: BTreeSet<Missing>,
    pub zoom: f32,
    /// The zoom follows the width of the view (on until the user zooms).
    pub fit: bool,
    /// The place at the top of the view: page, and points down it.
    view: (usize, f32),
    /// After a zoom change: the place to put back at the top of the view.
    rezoom: Option<(usize, f32)>,
    pub current: usize,
    /// Each page's label, when the document has page labels.
    pub labels: Option<Vec<String>>,
    anchor: usize,
    goto: Option<usize>,
    undo: Vec<Step>,
    redo: Vec<Step>,
    /// The last edit made by [`Tab::edit_stamp`]: its kind, and the revision and undo depth just after it.
    last_stamp: Option<(Stamp, u64, usize)>,
    pub side: Side,
    pub query: String,
    pub hits: Vec<SearchHit>,
    pub hit_idx: usize,
    /// The search matches on each page with hits: for each, a box per line it is on
    /// (displayed-page points).
    matches: HashMap<usize, Vec<Vec<[f32; 4]>>>,
    /// Scroll to this height on the `goto` page (points from its top), with `goto`.
    goto_y: Option<f32>,
    pub error: Option<String>,
    /// Showing the document failed with an internal error (see [`Tab::guarded`]); the tab
    /// shows this message instead of the pages.
    pub broken: Option<String>,
    pub ed: Editor,
    /// Pages whose texture is out of date but still shown until the new render arrives.
    stale: HashSet<usize>,
    /// Commands chosen from a right-click menu, run by the app after the frame.
    pub requested: Vec<Act>,
    /// State of the inspector's page and document forms.
    pub forms: Forms,
    /// Something undoable just happened, for the undo toast.
    pub notice: Option<String>,
    /// A link to a web address (or anything outside the document) was clicked; the app
    /// asks before opening it.
    pub open_link: Option<String>,
    /// A thumbnail drag in progress: the thumbnail it started on.
    thumb_drag: Option<usize>,
    /// Notices about the document shown above the pages, most important first.
    pub banners: Vec<Banner>,
    /// The revision the redaction marks were last counted at (into `ed.redact_marks`).
    marks_rev: u64,
    /// The mark count when the redaction banner was dismissed: it stays hidden until
    /// there are more marks.
    marks_hidden: usize,
    /// The same for the fill-and-sign banner and its items (counted into
    /// `ed.sign_items`).
    signs_hidden: usize,
    /// The bookmarks, read when first shown and again after the document changes:
    /// the revision they were read at, and the items.
    outline: Option<(u64, Vec<OutlineItem>)>,
    /// For each bookmark, whether its children are shown.
    outline_open: Vec<bool>,
    /// The bookmark last clicked; a new one goes after it.
    outline_sel: Option<usize>,
    /// A bookmark being renamed, and its title so far; `outline_focus` until the box
    /// has been given the keyboard.
    outline_rename: Option<(usize, String)>,
    outline_focus: bool,
    /// The annotations, read when the Comments tab is first shown and again after the
    /// document changes: the revision they were read at, and the list.
    comments: Option<(u64, Vec<Annot>)>,
    /// Single-page layout: whether the view was at the page's top and bottom at the end
    /// of the last frame, so the next wheel step past it turns the page.
    edge: (bool, bool),
    /// When the wheel last turned the page (egui time), so one long trackpad swipe turns
    /// one page.
    flipped: f64,
    /// This document's page layout and cover page choice: the file's own preference
    /// when it has one, else the app's default.
    pub layout: Layout,
    pub cover: bool,
    /// The digital signature check of a file with signature fields; `None` for other
    /// documents.
    pub sigs: Option<SigState>,
    /// What to call a document that has no file, such as a signed version.
    pub label: Option<String>,
}

impl Tab {
    pub fn new(
        ctx: &egui::Context,
        id: u64,
        mut pdf: Pdf,
        path: Option<PathBuf>,
        colours: Colours,
    ) -> Result<Self, String> {
        let size = path.as_ref().and_then(|p| std::fs::metadata(p).ok());
        let banners = banners::detect(&pdf, size.map(|m| m.len()));
        let signed = pdf.signature_count() > 0;
        // Form fields without drawings would show nothing; this isn't a change to save. A
        // signed document's fields stay as they were signed (saving adds only the edits
        // made to it); its snapshots draw them instead.
        if !signed {
            pdf.refresh_appearances();
        }
        let bytes = pdf.app_snapshot().map_err(|e| e.to_string())?;
        let sizes = sizes_of(&pdf);
        let labels = pdf.page_labels();
        Ok(Self {
            labels,
            banners,
            marks_rev: u64::MAX,
            marks_hidden: 0,
            signs_hidden: 0,
            id,
            pdf,
            path,
            dirty: false,
            revision: 0,
            encryption: None,
            signed,
            scrubbed: false,
            worker: Worker::spawn(ctx.clone(), bytes, colours),
            colours,
            sizes,
            pages: HashMap::new(),
            tiles: HashMap::new(),
            covered: HashMap::new(),
            thumbs: HashMap::new(),
            pending: HashSet::new(),
            failed: HashMap::new(),
            bad_tiles: HashSet::new(),
            missing: BTreeSet::new(),
            zoom: 1.0,
            fit: true,
            view: (0, 0.0),
            rezoom: None,
            current: 0,
            anchor: 0,
            goto: None,
            undo: Vec::new(),
            redo: Vec::new(),
            last_stamp: None,
            side: Side::Pages,
            query: String::new(),
            hits: Vec::new(),
            hit_idx: 0,
            matches: HashMap::new(),
            goto_y: None,
            error: None,
            broken: None,
            ed: Editor::default(),
            requested: Vec::new(),
            forms: Forms::default(),
            notice: None,
            open_link: None,
            thumb_drag: None,
            stale: HashSet::new(),
            outline: None,
            outline_open: Vec::new(),
            outline_sel: None,
            outline_rename: None,
            outline_focus: false,
            comments: None,
            edge: (false, false),
            flipped: 0.0,
            layout: Layout::default(),
            cover: true,
            sigs: None,
            label: None,
        })
    }

    /// The file name without extension, for naming files made from the document.
    pub fn stem(&self, fallback: &str) -> String {
        self.path
            .as_ref()
            .and_then(|p| p.file_stem())
            .map(|s| s.to_string_lossy().into_owned())
            .or_else(|| self.label.clone())
            .unwrap_or_else(|| fallback.into())
    }

    pub fn title(&self) -> String {
        let name = self
            .path
            .as_ref()
            .and_then(|p| p.file_name())
            .map(|n| limarepdf_core::display_line(&n.to_string_lossy(), 120))
            .or_else(|| self.label.clone())
            .unwrap_or_else(|| "Untitled".into());
        if self.dirty {
            format!("{name} *")
        } else {
            name
        }
    }

    pub fn page_count(&self) -> usize {
        self.sizes.len()
    }

    /// What to call page `p` (0-based): its label, or its number.
    pub fn page_name(&self, p: usize) -> String {
        self.labels
            .as_ref()
            .and_then(|l| l.get(p).cloned())
            .unwrap_or_else(|| (p + 1).to_string())
    }

    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }
    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }

    /// Pages an operation applies to: the selection, else the current page.
    pub fn targets(&self) -> Vec<usize> {
        match self.ed.selection.pages() {
            Some(s) => s.iter().copied().collect(),
            None => vec![self.current.min(self.page_count().saturating_sub(1))],
        }
    }

    pub fn go_to(&mut self, page: usize) {
        let p = page.min(self.page_count().saturating_sub(1));
        self.current = p;
        self.goto = Some(p);
    }

    /// The page `delta` rows on from the current one: in facing pages, the first page
    /// of the next or previous pair.
    pub fn step_page(&self, delta: isize, layout: Layout, cover: bool) -> usize {
        let rows = rows_of(self.page_count(), layout, cover);
        let r = rows
            .iter()
            .position(|row| row.iter().any(|&(p, _)| p == self.current))
            .unwrap_or(0);
        let r = r
            .saturating_add_signed(delta)
            .min(rows.len().saturating_sub(1));
        rows.get(r).map_or(0, |row| row[0].0)
    }

    // ------------------------------------------------------------ editing

    /// Draw part of the tab, unless it is broken. A panic while drawing (reading a damaged
    /// file) marks the tab broken instead of closing the app with every other tab.
    pub fn guarded(&mut self, f: impl FnOnce(&mut Self)) {
        if self.broken.is_some() {
            return;
        }
        if let Err(e) = crate::guard(|| {
            f(self);
            Ok(())
        }) {
            self.broken = Some(e.to_string());
        }
    }

    /// Run an edit with undo support. On failure the document is left unchanged, and
    /// false is returned.
    pub fn edit(
        &mut self,
        ctx: &egui::Context,
        f: impl FnOnce(&mut Pdf) -> limarepdf_core::Result<()>,
    ) -> bool {
        self.edit_inner(ctx, None, f)
    }

    /// Like [`Tab::edit`], but if the last change was the same kind of stamp, it is replaced rather than
    /// added to, so adjusting a setting and applying again doesn't pile up copies. One undo step either way.
    pub fn edit_stamp(
        &mut self,
        ctx: &egui::Context,
        kind: Stamp,
        f: impl FnOnce(&mut Pdf) -> limarepdf_core::Result<()>,
    ) -> bool {
        let mut replaced = None;
        if self.replaces_stamp(kind) {
            if let Some(Step {
                pdf: Some(bytes), ..
            }) = self.undo.last()
            {
                if let Ok(before) = Pdf::from_bytes(bytes, None) {
                    let step = self.undo.pop();
                    replaced = Some((std::mem::replace(&mut self.pdf, before), step));
                }
            }
        }
        let ok = self.edit(ctx, f);
        if ok {
            self.last_stamp = Some((kind, self.revision, self.undo.len()));
        } else if let Some((pdf, step)) = replaced {
            // Put the earlier stamp back.
            self.pdf = pdf;
            self.undo.extend(step);
        }
        ok
    }

    /// Keep the last stamp: the next one is added alongside it.
    pub fn forget_stamp(&mut self) {
        self.last_stamp = None;
    }

    /// Whether applying a stamp of this kind now would replace the previous one.
    pub fn replaces_stamp(&self, kind: Stamp) -> bool {
        self.last_stamp == Some((kind, self.revision, self.undo.len()))
    }

    /// Like [`Tab::edit`] for changes confined to the given pages: the old picture stays visible until the new
    /// one is rendered, and the selection is kept.
    fn edit_pages(
        &mut self,
        ctx: &egui::Context,
        pages: &[usize],
        f: impl FnOnce(&mut Pdf) -> limarepdf_core::Result<()>,
    ) -> bool {
        self.edit_inner(ctx, Some(pages), f)
    }

    fn edit_inner(
        &mut self,
        ctx: &egui::Context,
        soft: Option<&[usize]>,
        f: impl FnOnce(&mut Pdf) -> limarepdf_core::Result<()>,
    ) -> bool {
        let before = match crate::guard(|| self.pdf.snapshot()) {
            Ok(b) => b,
            Err(e) => {
                self.error = Some(format!("{e:#}"));
                return false;
            }
        };
        // A panic leaves the document half changed, so it is put back like any other failure.
        if let Err(e) = crate::guard(|| f(&mut self.pdf)) {
            if let Ok(p) = Pdf::from_bytes(&before, None) {
                self.pdf = p;
            }
            self.error = Some(format!("{e:#}"));
            return false;
        }
        self.push_undo(Some(before));
        self.revision += 1;
        match soft {
            Some(pages) => self.soft_refresh(ctx, pages),
            None => self.refresh(ctx),
        }
        true
    }

    /// The colour of blank paper in the current colours, shown until a page is drawn.
    fn paper(&self) -> Color32 {
        let [r, g, b] = self.colours.mode.paper();
        Color32::from_rgb(r, g, b)
    }

    /// Draw pages in `colours` from now on. Pages on screen keep their old drawing until
    /// the new one arrives.
    pub fn set_colours(&mut self, colours: Colours) {
        if colours == self.colours {
            return;
        }
        self.colours = colours;
        self.worker.set_colours(colours);
        self.pending.clear();
        self.stale.extend(self.pages.keys().copied());
        for t in self.tiles.values_mut() {
            t.0 = false;
        }
        self.thumbs.clear();
    }

    fn soft_refresh(&mut self, ctx: &egui::Context, pages: &[usize]) {
        match self.pdf.app_snapshot() {
            Ok(bytes) => {
                self.worker = Worker::spawn(ctx.clone(), bytes, self.colours);
                self.pending.clear();
                self.failed.clear();
                self.bad_tiles.clear();
                for &p in pages {
                    self.stale.insert(p);
                    self.thumbs.remove(&p);
                }
                for (k, t) in self.tiles.iter_mut() {
                    if pages.contains(&k.0) {
                        t.0 = false;
                    }
                }
                self.hits.clear();
                self.matches.clear();
                self.ed.invalidate();
            }
            Err(e) => self.error = Some(e.to_string()),
        }
    }

    /// Carry out the changes queued by the editing interface.
    pub fn apply_actions(&mut self, ctx: &egui::Context) {
        let acts = std::mem::take(&mut self.ed.actions);
        for a in acts {
            match a {
                Action::SetBox(p, id, b) => {
                    self.edit_pages(ctx, &[p], |pdf| pdf.set_object_bbox(p, id, b));
                }
                Action::Delete(p, id) => {
                    if self.edit_pages(ctx, &[p], |pdf| pdf.delete_object(p, id)) {
                        self.ed.selection.clear_object();
                        self.notice = Some("Deleted the object".into());
                    }
                }
                Action::AddShape(p, shape, rect) => {
                    let st = self.ed.style_for_new();
                    if self.edit_pages(ctx, &[p], |pdf| pdf.add_shape(p, shape, rect, &st)) {
                        self.after_insert(p);
                    }
                }
                Action::AddText(p, pos, text) => {
                    let (font, size, color) = (self.ed.tfont, self.ed.tsize, self.ed.tcolor);
                    if self.edit_pages(ctx, &[p], |pdf| {
                        pdf.add_text(p, pos, &text, font, size, color)
                    }) {
                        self.after_insert(p);
                    }
                }
                Action::AddImage(p, rect) => {
                    let Some((bytes, _)) = self.ed.pending_image.clone() else {
                        continue;
                    };
                    if self.edit_pages(ctx, &[p], |pdf| pdf.add_image(p, rect, &bytes)) {
                        self.after_insert(p);
                    }
                }
                Action::ReplaceImage(p, id, bytes) => {
                    self.edit_pages(ctx, &[p], |pdf| pdf.replace_image(p, id, &bytes));
                }
                Action::EditText(p, id, te) => {
                    let want = te.text.clone();
                    if self.edit_pages(ctx, &[p], |pdf| pdf.edit_text(p, id, &te)) {
                        let same = self
                            .ed
                            .objects(&self.pdf, p)
                            .get(id)
                            .and_then(|o| o.text.clone())
                            .as_deref()
                            == Some(want.as_str());
                        if same {
                            self.ed.select_object(p, id);
                        } else {
                            self.ed.select_last(&self.pdf, p);
                        }
                    }
                }
                Action::PathStyle(p, id, st) => {
                    self.edit_pages(ctx, &[p], |pdf| pdf.set_path_style(p, id, &st));
                }
                Action::SetField(pages, name, v) => {
                    self.edit_pages(ctx, &pages, |pdf| pdf.set_field(&name, v));
                }
                Action::SetAnnot(p, id, e) => {
                    self.edit_pages(ctx, &[p], |pdf| {
                        pdf.set_annotation(id, e.color, e.opacity, &e.contents)?;
                        if let Some(s) = &e.shape {
                            pdf.set_shape_look(id, s)?;
                        }
                        match &e.text {
                            Some(s) => pdf.set_text_style(id, s),
                            None => Ok(()),
                        }
                    });
                }
                Action::AnnotText(p, id, text) => {
                    let Some(a) = self.pdf.annotations().into_iter().find(|a| a.id == id) else {
                        continue;
                    };
                    self.edit_pages(ctx, &[p], |pdf| {
                        pdf.set_annotation(id, a.color, a.opacity, &text)
                    });
                }
                Action::MoveAnnot(p, id, to) => {
                    self.edit_pages(ctx, &[p], |pdf| pdf.move_annotation(p, id, to));
                }
                Action::LineEnds(p, id, ends) => {
                    self.edit_pages(ctx, &[p], |pdf| pdf.set_line_ends(p, id, ends));
                }
                Action::RotateInk(p, id, degrees) => {
                    self.edit_pages(ctx, &[p], |pdf| pdf.rotate_ink(p, id, degrees));
                }
                Action::AddNote(p, at, text) => {
                    let (color, author) = (self.ed.note_color, self.ed.author.clone());
                    let mut new = None;
                    let add = |pdf: &mut Pdf| {
                        new = Some(pdf.add_note(p, at, color, &text, &author)?);
                        Ok(())
                    };
                    if self.edit_pages(ctx, &[p], add) {
                        self.after_annot(p, new);
                    }
                }
                Action::AddRedactMark(p, rect) => {
                    // The tool stays, so several areas can be marked in a row.
                    let author = self.ed.author.clone();
                    self.edit_pages(ctx, &[p], |pdf| {
                        pdf.add_redaction_mark(p, &[rect], &author).map(drop)
                    });
                }
                Action::AddStamp(p, place) => {
                    let pick = self.ed.stamp.clone();
                    let picture = match &pick {
                        StampPick::Picture(path) => match std::fs::read(path) {
                            Ok(b) => b,
                            Err(e) => {
                                self.error = Some(format!("Can't read {}: {e}", path.display()));
                                continue;
                            }
                        },
                        StampPick::Text(_) | StampPick::Custom(..) => Vec::new(),
                    };
                    let stamp = match &pick {
                        StampPick::Text(kind) => limarepdf_core::Stamp::Text {
                            kind: *kind,
                            name_and_date: self.ed.stamp_name_date,
                        },
                        StampPick::Custom(text, color) => limarepdf_core::Stamp::Custom {
                            text,
                            color: *color,
                            name_and_date: self.ed.stamp_name_date,
                        },
                        StampPick::Picture(_) => limarepdf_core::Stamp::Picture(&picture),
                    };
                    let author = self.ed.author.clone();
                    let mut new = None;
                    let add = |pdf: &mut Pdf| {
                        new = Some(pdf.add_stamp(p, &stamp, place, &author)?);
                        Ok(())
                    };
                    if self.edit_pages(ctx, &[p], add) {
                        self.after_annot(p, new);
                    }
                }
                Action::AddSign(p, place) => {
                    let ed = &self.ed;
                    let (kit, author) = (&ed.sign_kit, ed.author.clone());
                    let color = kit.color;
                    let centre = match place {
                        StampPlace::Centre(x, y) => (x, y),
                        StampPlace::Fill(r) => ((r[0] + r[2]) / 2.0, (r[1] + r[3]) / 2.0),
                    };
                    let text = match ed.sign.pick {
                        SignPick::Date => Some(kit.date.today()),
                        SignPick::Name => Some(author.clone()),
                        SignPick::Text => Some(ed.sign.text.clone()),
                        SignPick::Saved(..) | SignPick::Mark(_) => None,
                    };
                    let saved = match ed.sign.pick {
                        SignPick::Saved(role, i) => {
                            kit.list(role).get(i).map(|e| (role, e.saved.clone()))
                        }
                        _ => None,
                    };
                    let mark = match ed.sign.pick {
                        SignPick::Mark(m) => Some(m),
                        _ => None,
                    };
                    let bytes = match saved.as_ref().map(|(_, s)| s.bytes()) {
                        Some(Err(e)) => {
                            self.error = Some(format!("Can't read the signature's picture: {e}"));
                            continue;
                        }
                        Some(Ok(b)) => b,
                        None => Vec::new(),
                    };
                    let mut new = None;
                    let add = |pdf: &mut Pdf| {
                        new = Some(match (&text, &saved, mark) {
                            (Some(t), _, _) => pdf.add_sign_text(p, centre, t, color, &author)?,
                            (_, Some((role, s)), _) => {
                                pdf.add_stamp(p, &s.stamp(&bytes, *role, color), place, &author)?
                            }
                            (_, _, Some(mark)) => pdf.add_stamp(
                                p,
                                &limarepdf_core::Stamp::Mark { mark, color },
                                place,
                                &author,
                            )?,
                            _ => anyhow::bail!("pick something to put down first"),
                        });
                        Ok(())
                    };
                    if self.edit_pages(ctx, &[p], add) {
                        self.after_annot(p, new);
                    }
                }
                Action::SignHere(p, rect) => {
                    // A signature field takes a signature (or the initials picked), made
                    // first when none is saved.
                    self.ed.set_tool(Tool::Sign);
                    if !matches!(self.ed.sign.pick, SignPick::Saved(..)) {
                        self.ed.sign.pick = SignPick::Saved(limarepdf_core::SignRole::Signature, 0);
                    }
                    self.ed.place_sign(p, StampPlace::Fill(rect));
                }
                Action::SignatureDetails(name) => {
                    let found = self.sig_report().and_then(|c| {
                        c.report
                            .signatures
                            .iter()
                            .position(|s| s.fields.contains(&name))
                    });
                    // Still being checked, or not found: the Signatures tab says why.
                    self.requested.push(match found {
                        Some(i) => Act::SignatureDetails(i),
                        None => Act::Signatures,
                    });
                }
                Action::AddTextBox(p, rect, text) => {
                    let (style, author) = (self.ed.box_style, self.ed.author.clone());
                    let mut new = None;
                    let add = |pdf: &mut Pdf| {
                        new = Some(pdf.add_text_box(p, rect, &text, &style, &author)?);
                        Ok(())
                    };
                    if self.edit_pages(ctx, &[p], add) {
                        self.after_annot(p, new);
                    }
                }
                Action::AddShapeAnnot(p, shape, rect) => {
                    let ed = &self.ed;
                    let color = ed.stroke_on.then_some(ed.stroke);
                    let look = ShapeLook {
                        fill: (shape != Shape::Line && ed.fill_on).then_some(ed.fill),
                        width: ed.width,
                        arrow: shape == Shape::Line && ed.arrow,
                    };
                    let author = ed.author.clone();
                    let mut new = None;
                    let add = |pdf: &mut Pdf| {
                        new =
                            Some(pdf.add_shape_annotation(p, shape, rect, color, &look, &author)?);
                        Ok(())
                    };
                    if self.edit_pages(ctx, &[p], add) {
                        self.after_annot(p, new);
                    }
                }
                Action::AddInk(p, stroke) => {
                    let ed = &self.ed;
                    let (color, width, author) = (ed.pen_color, ed.pen_width, ed.author.clone());
                    let mut new = None;
                    let add = |pdf: &mut Pdf| {
                        new = Some(pdf.add_ink(p, &[stroke], color, width, &author)?);
                        Ok(())
                    };
                    // The pen stays in hand, ready for the next stroke.
                    if self.edit_pages(ctx, &[p], add) {
                        self.ed.ink = new.map(|id| (p, id));
                    }
                }
                Action::AddInkStroke(p, id, stroke) => {
                    if self.edit_pages(ctx, &[p], |pdf| pdf.add_ink_stroke(p, id, &stroke)) {
                        self.ed.ink = Some((p, id));
                    }
                }
                Action::Reply(p, id, text) => {
                    let author = self.ed.author.clone();
                    if self.edit_pages(ctx, &[p], |pdf| {
                        pdf.add_reply(id, &text, &author).map(|_| ())
                    }) {
                        self.ed.selection = Selection::Annot { page: p, id };
                    }
                }
                Action::DeleteAnnot(p, id) => {
                    let gone = |pdf: &mut Pdf| pdf.remove_annotations(|a| a.id == id).map(|_| ());
                    if self.edit_pages(ctx, &[p], gone) {
                        self.ed.selection.clear_annot();
                        self.notice = Some("Deleted the comment".into());
                    }
                }
            }
        }
    }

    /// Mark the selected words with `mark` in `color`, one annotation per page, as one
    /// undo step. The words are let go so the mark shows.
    pub fn add_markup(&mut self, ctx: &egui::Context, mark: Mark, color: [f32; 3], author: &str) {
        let lines = self.ed.selected_lines();
        if lines.is_empty() {
            return;
        }
        let pages: Vec<usize> = lines.iter().map(|(p, _)| *p).collect();
        let kind = mark.kind();
        let add = |pdf: &mut Pdf| {
            for (p, boxes) in &lines {
                pdf.add_markup(*p, &kind, boxes, color, author)?;
            }
            Ok(())
        };
        if self.edit_pages(ctx, &pages, add) {
            self.ed.selection.clear_words();
        }
    }

    /// Mark the selected words for redaction: one mark per page, a box per line.
    pub fn mark_selection_for_redaction(&mut self, ctx: &egui::Context, author: &str) {
        let lines = self.ed.selected_lines();
        if lines.is_empty() {
            return;
        }
        let pages: Vec<usize> = lines.iter().map(|(p, _)| *p).collect();
        let add = |pdf: &mut Pdf| {
            for (p, boxes) in &lines {
                pdf.add_redaction_mark(*p, boxes, author)?;
            }
            Ok(())
        };
        if self.edit_pages(ctx, &pages, add) {
            self.ed.selection.clear_words();
            self.notice = Some("Marked for redaction; apply the marks to remove the text".into());
        }
    }

    /// Mark every match of the last search for redaction: one mark per page.
    pub fn mark_results_for_redaction(&mut self, ctx: &egui::Context, author: &str) {
        let marks: Vec<(usize, Vec<[f32; 4]>)> = self
            .matches
            .iter()
            .map(|(p, m)| (*p, m.iter().flatten().copied().collect()))
            .filter(|(_, b): &(usize, Vec<[f32; 4]>)| !b.is_empty())
            .collect();
        if marks.is_empty() {
            self.notice = Some("Search first: there are no results to mark".into());
            return;
        }
        let pages: Vec<usize> = marks.iter().map(|(p, _)| *p).collect();
        let n: usize = marks.iter().map(|(_, b)| b.len()).sum();
        let add = |pdf: &mut Pdf| {
            for (p, boxes) in &marks {
                pdf.add_redaction_mark(*p, boxes, author)?;
            }
            Ok(())
        };
        if self.edit_pages(ctx, &pages, add) {
            self.notice = Some(format!(
                "Marked {n} areas on {} pages for redaction; apply the marks to remove them",
                pages.len()
            ));
        }
    }

    /// A new note or text box is selected, with the arrow to move it, as new objects are.
    fn after_annot(&mut self, page: usize, id: Option<(u32, u16)>) {
        self.ed.set_tool(Tool::Select);
        if let Some(id) = id {
            self.ed.selection = Selection::Annot { page, id };
        }
    }

    fn after_insert(&mut self, page: usize) {
        self.ed.select_last(&self.pdf, page);
        self.ed.set_tool_keep_sel(Tool::Select);
    }

    /// Ask for an image file and arm the image tool with it.
    pub fn choose_image(&mut self) {
        let Some(bytes) = crate::editor::pick_image() else {
            return;
        };
        match limarepdf_core::image_dimensions(&bytes) {
            Ok((w, h)) => {
                self.ed.pending_image = Some((bytes, w as f32 / h.max(1) as f32));
                self.ed.set_tool(Tool::Image);
            }
            Err(e) => self.error = Some(e.to_string()),
        }
    }

    /// Record the state before a change: the document's bytes (`None` if it did not
    /// change), the current password settings and whether anything was removed for good.
    fn push_undo(&mut self, pdf: Option<Vec<u8>>) {
        let encryption = self.encryption.clone();
        self.undo.push(Step {
            pdf,
            encryption,
            scrubbed: self.scrubbed,
        });
        if self.undo.len() > MAX_UNDO {
            self.undo.remove(0);
        }
        self.redo.clear();
        self.dirty = true;
    }

    /// Change the password settings, with undo. They take effect when the document is
    /// saved.
    pub fn set_encryption(&mut self, encryption: Option<Encryption>) {
        self.push_undo(None);
        self.encryption = encryption;
    }

    pub fn undo(&mut self, ctx: &egui::Context) {
        if let Some(step) = self.undo.pop() {
            if let Some(back) = self.restore(ctx, step) {
                self.redo.push(back);
            }
        }
    }

    pub fn redo(&mut self, ctx: &egui::Context) {
        if let Some(step) = self.redo.pop() {
            if let Some(back) = self.restore(ctx, step) {
                self.undo.push(back);
            }
        }
    }

    /// Go back to a step's state. Returns the step that reverses it.
    fn restore(&mut self, ctx: &egui::Context, step: Step) -> Option<Step> {
        let pdf = match step.pdf {
            Some(bytes) => {
                let cur = self.pdf.snapshot().ok()?;
                self.pdf = Pdf::from_bytes(&bytes, None).ok()?;
                Some(cur)
            }
            None => None,
        };
        let encryption = std::mem::replace(&mut self.encryption, step.encryption);
        let scrubbed = std::mem::replace(&mut self.scrubbed, step.scrubbed);
        self.forms.sec = Sec::of(self.encryption.as_ref());
        self.dirty = true;
        if pdf.is_some() {
            self.revision += 1;
            self.refresh(ctx);
        }
        Some(Step {
            pdf,
            encryption,
            scrubbed,
        })
    }

    /// Re-render from the current document state.
    pub fn refresh(&mut self, ctx: &egui::Context) {
        match self.pdf.app_snapshot() {
            Ok(bytes) => {
                self.worker = Worker::spawn(ctx.clone(), bytes, self.colours);
                self.sizes = sizes_of(&self.pdf);
                self.labels = self.pdf.page_labels();
                self.pages.clear();
                self.tiles.clear();
                self.covered.clear();
                self.thumbs.clear();
                self.pending.clear();
                self.failed.clear();
                self.bad_tiles.clear();
                self.stale.clear();
                self.ed.invalidate();
                self.ed.selection.clear_object();
                self.ed.new_text = None;
                self.hits.clear();
                self.matches.clear();
                let n = self.page_count();
                self.ed.selection.retain_pages(n);
                self.current = self.current.min(n.saturating_sub(1));
                self.goto = Some(self.current);
            }
            Err(e) => self.error = Some(e.to_string()),
        }
    }

    /// The Bookmarks side tab: the outline indented by level, with a caret to show or
    /// hide an entry's children. Clicking an entry goes to its page; double-click or
    /// right-click to rename, add under it or delete it.
    pub fn show_outline(&mut self, ui: &mut egui::Ui) {
        let rev = self.revision;
        if self.outline.as_ref().is_none_or(|(r, _)| *r != rev) {
            let items = self.pdf.outline();
            self.outline_open = items.iter().map(|i| i.open).collect();
            if self.outline_sel.is_some_and(|s| s >= items.len()) {
                self.outline_sel = None;
            }
            // Keep the selected bookmark (a new one, say) in view: open the ones above it.
            if let Some(s) = self.outline_sel {
                let mut depth = items[s].depth;
                for k in (0..s).rev() {
                    if items[k].depth < depth {
                        depth = items[k].depth;
                        self.outline_open[k] = true;
                    }
                }
            }
            self.outline = Some((rev, items));
        }
        let Some((_, items)) = &self.outline else {
            return;
        };
        let add = ui
            .button((theme::icon(icon::PLUS), "Add bookmark"))
            .on_hover_text(match self.outline_sel {
                Some(_) => "Bookmark the current page, after the selected bookmark",
                None => "Bookmark the current page",
            })
            .clicked();
        let mut place = add.then_some(match self.outline_sel {
            Some(s) => BookmarkPlace::After(s),
            None => BookmarkPlace::End,
        });
        if items.is_empty() {
            ui.weak("This document has no bookmarks.");
        }
        let mut jump = None;
        let mut toggle = None;
        let mut rename = None;
        let mut renamed = None;
        let mut delete = None;
        let mut editing = self.outline_rename.take();
        let mut focus = std::mem::take(&mut self.outline_focus);
        let mut sel = self.outline_sel;
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                // Items deeper than this are inside a closed entry.
                let mut closed_at: Option<usize> = None;
                for (i, item) in items.iter().enumerate() {
                    match closed_at {
                        Some(d) if item.depth > d => continue,
                        _ => closed_at = None,
                    }
                    let open = self.outline_open[i];
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 2.0;
                        ui.add_space(item.depth as f32 * 12.0);
                        let caret = if open {
                            icon::CARET_DOWN
                        } else {
                            icon::CARET_RIGHT
                        };
                        let caret = egui::Button::new(theme::icon(caret).size(10.0))
                            .frame_when_inactive(false);
                        if ui.add_visible(item.has_children, caret).clicked() {
                            toggle = Some(i);
                        }
                        if let Some((at, title)) = &mut editing {
                            if *at == i {
                                let r = ui.add(
                                    egui::TextEdit::singleline(title).desired_width(f32::INFINITY),
                                );
                                if std::mem::take(&mut focus) {
                                    r.request_focus();
                                }
                                if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                                    rename = Some(None);
                                } else if r.lost_focus() {
                                    let title = title.trim().to_string();
                                    renamed = Some((i, title, item.title.clone()));
                                }
                                return;
                            }
                        }
                        let shown = limarepdf_core::display_line(&item.title, 300);
                        let mut text = egui::RichText::new(&shown);
                        if item.page.is_none() {
                            text = text.weak();
                        }
                        let selected = sel == Some(i);
                        let row = egui::Button::selectable(selected, text).truncate();
                        let tip = match item.page {
                            Some(p) => format!("{shown}\nPage {}", self.page_name(p)),
                            None => format!("{shown}\nGoes outside this document"),
                        };
                        let r = ui.add(row).on_hover_text(tip);
                        if r.clicked() {
                            sel = Some(i);
                            jump = item.page;
                        }
                        if r.double_clicked() {
                            rename = Some(Some((i, item.title.clone())));
                        }
                        r.context_menu(|ui| {
                            let pencil = theme::icon(icon::PENCIL_SIMPLE);
                            if ui.button((pencil, "Rename")).clicked() {
                                rename = Some(Some((i, item.title.clone())));
                            }
                            let plus = theme::icon(icon::PLUS);
                            if ui.button((plus, "Add bookmark under this")).clicked() {
                                place = Some(BookmarkPlace::Under(i));
                            }
                            let label = match item.has_children {
                                true => "Delete, with its children",
                                false => "Delete",
                            };
                            if ui.button((theme::icon(icon::TRASH), label)).clicked() {
                                delete = Some(i);
                            }
                        });
                    });
                    if item.has_children && !open {
                        closed_at = Some(item.depth);
                    }
                }
            });
        self.outline_rename = editing;
        self.outline_focus = focus;
        self.outline_sel = sel;
        if let Some(i) = toggle {
            self.outline_open[i] = !self.outline_open[i];
        }
        if let Some(p) = jump {
            self.go_to(p);
        }
        let ctx = ui.ctx().clone();
        if let Some(r) = rename {
            self.outline_rename = r;
            self.outline_focus = true;
        }
        if let Some((i, title, old)) = renamed {
            self.outline_rename = None;
            if !title.is_empty() && title != old {
                self.edit(&ctx, |p| p.rename_bookmark(i, &title));
            }
        }
        if let Some(i) = delete {
            if self.edit(&ctx, |p| p.remove_bookmark(i)) {
                self.outline_sel = None;
                self.notice = Some("Deleted the bookmark".into());
            }
        }
        if let Some(place) = place {
            let page = self.current;
            let title = format!("Page {}", self.page_name(page));
            let mut added = None;
            let ok = self.edit(&ctx, |p| {
                added = Some(p.add_bookmark(&title, page, place)?);
                Ok(())
            });
            if let Some(i) = added.filter(|_| ok) {
                self.outline_sel = Some(i);
                self.outline_rename = Some((i, title));
                self.outline_focus = true;
            }
        }
    }

    /// The Comments side tab: the annotations by page, replies under what they answer.
    /// Clicking one goes to it and outlines it; right-click to delete it.
    pub fn show_comments(&mut self, ui: &mut egui::Ui) {
        let rev = self.revision;
        if self.comments.as_ref().is_none_or(|(r, _)| *r != rev) {
            let list = self.pdf.annotations();
            let gone = |(_, id)| !list.iter().any(|a| a.id == id);
            if self.ed.selection.annot().is_some_and(gone) {
                self.ed.selection.clear_annot();
            }
            self.comments = Some((rev, list));
        }
        let chosen_id = self.ed.selection.annot().map(|(_, id)| id);
        let Some((_, list)) = &self.comments else {
            return;
        };
        if list.is_empty() {
            ui.weak("This document has no comments or markup.");
            return;
        }
        let mut chosen = None;
        let mut delete = None;
        let mut reply = false;
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                let mut page = usize::MAX;
                for (i, depth) in thread_order(list) {
                    let a = &list[i];
                    if a.page != page {
                        page = a.page;
                        ui.add_space(4.0);
                        ui.label(
                            egui::RichText::new(format!("Page {}", page + 1))
                                .small()
                                .strong(),
                        );
                    }
                    let first = a
                        .contents
                        .lines()
                        .map(|l| limarepdf_core::display_line(l, 200))
                        .find(|l| !l.is_empty());
                    let who = limarepdf_core::display_line(&a.author, 80);
                    let text = match (who.as_str(), first) {
                        ("", None) => a.label(),
                        ("", Some(t)) => format!("{}: {t}", a.label()),
                        (w, None) => format!("{} · {w}", a.label()),
                        (w, Some(t)) => format!("{w}: {t}"),
                    };
                    let mut text = egui::RichText::new(text);
                    if a.hidden() {
                        text = text.weak();
                    }
                    let tip = annot_tip(a);
                    ui.horizontal(|ui| {
                        ui.add_space(depth as f32 * 12.0);
                        let row =
                            egui::Button::selectable(chosen_id == Some(a.id), text).truncate();
                        let r = ui.add(row).on_hover_text(tip);
                        if r.clicked() {
                            chosen = Some(i);
                        }
                        r.context_menu(|ui| {
                            let answer = theme::icon(icon::ARROW_BEND_UP_LEFT);
                            if ui.button((answer, "Reply…")).clicked() {
                                chosen = Some(i);
                                reply = true;
                            }
                            let label = if list.iter().any(|b| b.reply_to == Some(a.id)) {
                                "Delete, with its replies"
                            } else {
                                "Delete"
                            };
                            let del = ui
                                .add_enabled(!a.locked(), egui::Button::new(label))
                                .on_disabled_hover_text("It is locked in this document.");
                            if del.clicked() {
                                delete = Some(a.id);
                            }
                        });
                    });
                }
            });
        if let Some(i) = chosen {
            let a = &list[i];
            let (id, page, top) = (a.id, a.page, a.bbox[1]);
            self.ed.selection = Selection::Annot { page, id };
            self.ed.reply_focus = reply;
            self.go_to(page);
            self.goto_y = Some(top);
        }
        if let Some(id) = delete {
            let ctx = ui.ctx().clone();
            if self.edit(&ctx, |p| p.remove_annotations(|a| a.id == id).map(|_| ())) {
                self.notice = Some("Deleted the comment".into());
            }
        }
    }

    /// The Signatures side tab.
    pub fn show_signatures(&mut self, ui: &mut egui::Ui) {
        match signatures::panel(ui, self.sigs.as_ref(), self.dirty) {
            Some(PanelAct::GoTo(page, top)) => self.go_to_at(page, top),
            Some(PanelAct::Details(i)) => self.requested.push(Act::SignatureDetails(i)),
            Some(PanelAct::SignedVersion(i)) => self.requested.push(Act::SignedVersion(i)),
            Some(PanelAct::Trusted) => self.requested.push(Act::TrustedCertificates),
            None => {}
        }
    }

    /// Shows page `page` with the place `top` points down it at the top of the view.
    pub fn go_to_at(&mut self, page: usize, top: f32) {
        self.go_to(page);
        self.goto_y = Some(top);
    }

    /// Takes the result of a signature check (dropped when a newer check has started)
    /// and shows its banner first.
    pub fn sig_checked(&mut self, generation: u64, check: Check) {
        let Some(s) = self.sigs.as_mut().filter(|s| s.generation == generation) else {
            return;
        };
        s.check = check;
        self.sig_banner();
    }

    /// Shows the signatures banner for the current check, first; takes it away when
    /// there is nothing to say.
    pub fn sig_banner(&mut self) {
        self.banners.retain(|b| b.kind != banners::Kind::Signatures);
        if let Some(b) = self
            .sigs
            .as_ref()
            .and_then(|s| signatures::banner(&s.check))
        {
            self.banners.insert(0, b);
        }
    }

    /// The report of the last finished signature check, if any.
    pub fn sig_report(&self) -> Option<&signatures::Checked> {
        match &self.sigs.as_ref()?.check {
            Check::Done(c) => Some(c),
            _ => None,
        }
    }

    pub fn run_search(&mut self) {
        self.hit_idx = 0;
        self.matches.clear();
        match self.pdf.search(&self.query) {
            Ok(h) => self.hits = h,
            Err(e) => self.error = Some(e.to_string()),
        }
        let pages: BTreeSet<usize> = self.hits.iter().map(|h| h.page).collect();
        for p in pages {
            if let Ok(b) = self.pdf.find_text_matches(p, &self.query) {
                self.matches.insert(p, b);
            }
        }
        if !self.hits.is_empty() {
            self.show_hit();
        }
    }

    pub fn step_hit(&mut self, delta: isize) {
        if self.hits.is_empty() {
            return;
        }
        let n = self.hits.len() as isize;
        self.hit_idx = ((self.hit_idx as isize + delta).rem_euclid(n)) as usize;
        self.show_hit();
    }

    /// The current hit's page, and which of that page's matches it is. Hits and matches
    /// come from the same page text, so their counts agree; the index is clamped all the
    /// same.
    fn current_match(&self) -> Option<(usize, usize)> {
        let h = self.hits.get(self.hit_idx)?;
        let nth = self.hits[..self.hit_idx]
            .iter()
            .filter(|o| o.page == h.page)
            .count();
        let n = self.matches.get(&h.page).map_or(0, |v| v.len());
        Some((h.page, nth.min(n.saturating_sub(1))))
    }

    /// Scroll to the current hit's match box, or to its page when it has none.
    fn show_hit(&mut self) {
        let Some((page, k)) = self.current_match() else {
            return;
        };
        self.go_to(page);
        let first = self.matches.get(&page).and_then(|v| v.get(k)?.first());
        self.goto_y = first.map(|b| b[1]);
    }

    // ------------------------------------------------------------ drawing

    fn collect(&mut self, ctx: &egui::Context) {
        while let Ok(r) = self.worker.rx.try_recv() {
            self.pending.remove(&r.key);
            self.note_missing(&r.missing);
            if r.colours != self.colours {
                continue;
            }
            if r.key.2.is_some() {
                match r.bitmap {
                    Ok(b) => {
                        let img =
                            egui::ColorImage::from_rgba_premultiplied([b.width, b.height], &b.rgba);
                        let (p, q, t) = (r.key.0, r.key.1, r.key.2.unwrap_or_default());
                        let name = format!("t{}-{p}-{q}-{}-{}", self.id, t.0, t.1);
                        let tex = ctx.load_texture(name, img, TextureOptions::LINEAR);
                        self.tiles.insert(r.key, (true, tex));
                    }
                    Err(_) => {
                        self.bad_tiles.insert(r.key);
                    }
                }
                continue;
            }
            let b = match r.bitmap {
                Ok(b) => b,
                Err(e) => {
                    if r.key.1 != 0 {
                        self.pages.remove(&r.key.0);
                        self.stale.remove(&r.key.0);
                    }
                    self.failed.insert(r.key.0, e);
                    continue;
                }
            };
            let img = egui::ColorImage::from_rgba_premultiplied([b.width, b.height], &b.rgba);
            let tex = ctx.load_texture(
                format!("p{}-{}-{}", self.id, r.key.0, r.key.1),
                img,
                TextureOptions::LINEAR,
            );
            if r.key.1 == 0 {
                self.thumbs.insert(r.key.0, tex);
            } else {
                self.stale.remove(&r.key.0);
                self.pages.insert(r.key.0, (r.key.1, tex));
            }
        }
    }

    /// Add what the renderer couldn't draw to the banner, showing it again (even if it
    /// was dismissed) when something new turns up.
    fn note_missing(&mut self, what: &BTreeSet<Missing>) {
        if what.is_subset(&self.missing) {
            return;
        }
        self.missing.extend(what);
        self.banners.retain(|b| b.kind != banners::Kind::Missing);
        let at = self
            .banners
            .partition_point(|b| b.kind < banners::Kind::Missing);
        self.banners.insert(at, banners::missing(&self.missing));
    }

    /// How page `i` is drawn at `scale` (bitmap pixels per point): the whole page's render
    /// key and scale, kept within [`WHOLE_MAX`] and the largest texture the GPU takes, and,
    /// when that limit cuts the scale, the quantised scale of the tiles drawn over it.
    fn page_key(&self, ctx: &egui::Context, i: usize, scale: f32) -> (Key, f32, Option<u32>) {
        let (w, h) = self.sizes.get(i).copied().unwrap_or((1.0, 1.0));
        let side = (ctx.input(|s| s.max_texture_side) as f32).min(WHOLE_MAX);
        // Quantised in twentieths, rounded down at the limit so it stays under it.
        let cap = ((side / w.max(h).max(1.0)) * 20.0).floor().max(1.0);
        let want = (scale.min(MAX_REGION_SCALE) * 20.0).round().max(1.0);
        let q = want.min(cap) as u32;
        let tiles = (want > cap).then_some(want as u32);
        ((i, q, None), q as f32 / 20.0, tiles)
    }

    /// Draw the tiles of page `i`, shown at `r` with tiles at quantised scale `q`, that
    /// fall in the view of `painter`. Adds the tiles to ask for to `want`, most wanted
    /// last, and the tiles to keep to `keep`: those in view and a ring of one around them.
    fn draw_tiles(
        &mut self,
        painter: &egui::Painter,
        i: usize,
        r: Rect,
        q: u32,
        want: &mut Vec<Key>,
        keep: &mut HashSet<Key>,
    ) {
        let (w, h) = self.sizes.get(i).copied().unwrap_or((1.0, 1.0));
        let vis = painter.clip_rect().intersect(r);
        if !vis.is_positive() {
            return;
        }
        // Screen points per tile pixel at quantised scale `q`.
        let per = |q: u32| r.width() / (w * q as f32 / 20.0).max(1.0);
        let k = per(q);
        let s = q as f32 / 20.0;
        let cols = ((w * s) as u32).div_ceil(TILE).max(1);
        let rows = ((h * s) as u32).div_ceil(TILE).max(1);
        let span = |a: f32, b: f32, n: u32| {
            let t = TILE as f32 * k;
            let lo = ((a / t).floor().max(0.0) as u32).min(n - 1);
            let hi = (((b / t).ceil() as u32).max(1) - 1).min(n - 1);
            (lo, hi)
        };
        let (c0, c1) = span(vis.min.x - r.min.x, vis.max.x - r.min.x, cols);
        let (r0, r1) = span(vis.min.y - r.min.y, vis.max.y - r.min.y, rows);
        // A tile's rectangle on screen, from its own size (the last ones are cut to the page).
        let place = |key: &Key, tex: &TextureHandle| {
            let (c, row) = key.2.unwrap_or_default();
            let k = per(key.1);
            let min = r.min + Vec2::new((c * TILE) as f32, (row * TILE) as f32) * k;
            let [tw, th] = tex.size();
            Rect::from_min_size(min, Vec2::new(tw as f32, th as f32) * k)
        };
        let uv = Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0));
        // Tiles of an earlier zoom, until the ones at this zoom cover the view.
        if let Some(&old) = self.covered.get(&i).filter(|&&o| o != q) {
            for (key, (_, tex)) in &self.tiles {
                if key.0 == i && key.1 == old {
                    let at = place(key, tex);
                    if at.intersects(vis) {
                        painter.image(tex.id(), at, uv, Color32::WHITE);
                        keep.insert(*key);
                    }
                }
            }
        }
        // The ring is asked for before the tiles in view, so those are drawn first (the
        // worker takes the newest jobs first).
        let ring = |a: u32, b: u32, n: u32| a.saturating_sub(1)..=(b + 1).min(n - 1);
        let mut inner = Vec::new();
        for row in ring(r0, r1, rows) {
            for c in ring(c0, c1, cols) {
                let key = (i, q, Some((c, row)));
                keep.insert(key);
                let seen = (c0..=c1).contains(&c) && (r0..=r1).contains(&row);
                let fresh = self.tiles.get(&key).map(|t| t.0);
                if fresh != Some(true) && !self.bad_tiles.contains(&key) {
                    if seen {
                        inner.push(key);
                    } else {
                        want.push(key);
                    }
                }
            }
        }
        let mut all_in = true;
        for row in r0..=r1 {
            for c in c0..=c1 {
                let key = (i, q, Some((c, row)));
                match self.tiles.get(&key) {
                    Some((_, tex)) => {
                        painter.image(tex.id(), place(&key, tex), uv, Color32::WHITE);
                    }
                    None => all_in &= self.bad_tiles.contains(&key),
                }
            }
        }
        want.extend(inner);
        if all_in {
            self.covered.insert(i, q);
        }
    }

    /// Zoom chosen by the user: turns fitting to the width off.
    pub fn set_zoom(&mut self, z: f32) {
        self.fit = false;
        self.zoom_to(z);
    }

    /// Change the zoom, keeping the same place in the document at the top of the view.
    fn zoom_to(&mut self, z: f32) {
        let z = z.clamp(0.1, MAX_ZOOM);
        if (z - self.zoom).abs() < 1e-4 {
            return;
        }
        self.zoom = z;
        self.rezoom = Some(self.view);
    }

    /// The command state for this tab's part: document, undo, selection and tool.
    pub fn fill_ctx(&self, c: &mut Ctx) {
        c.doc = true;
        c.layout = self.layout;
        c.cover = self.cover;
        c.can_undo = self.can_undo();
        c.can_redo = self.can_redo();
        c.words = self.ed.selected_text().is_some();
        c.object = self.ed.has_object();
        c.obj_kind = match self.ed.selection {
            Selection::Object { kind, .. } => Some(kind),
            _ => None,
        };
        c.pages = self.ed.selection.pages().is_some();
        c.tool = Some(self.ed.tool);
        c.annot_tool = Some(self.ed.annot_tool);
        c.fit = self.fit;
        c.hidden_text = self.ed.show_hidden;
        c.form = !self.ed.form.fields().is_empty();
        c.field = self.ed.form.focused_field().is_some();
        c.fields_shown = self.ed.form.highlight;
        let annot = self.ed.selected_annot();
        c.annot = annot.is_some();
        c.annot_locked = annot.is_some_and(|a| a.locked());
        c.annot_moves = self.ed.tool == Tool::Select && annot.is_some_and(crate::editor::movable);
    }

    /// A right-click menu for what is selected, its commands queued in `requested`. The
    /// state is `base` updated for this tab, since the click may just have changed the
    /// selection.
    fn context_menu(&mut self, resp: &egui::Response, base: &Ctx) {
        let mut c = *base;
        self.fill_ctx(&mut c);
        let sel = self.ed.selection.clone();
        resp.context_menu(|ui| actions::context_menu(ui, &sel, &c, &mut self.requested));
    }

    /// A small bar just above the selected object or words (below, when there is no
    /// room above), with icon buttons for the right-click menu's commands. `page` is the
    /// page's screen rectangle, `k` the screen size of a point, `b` the selection's box
    /// in page points. It scrolls with the page and stays inside the view.
    fn floating_toolbar(&mut self, ui: &egui::Ui, base: &Ctx, page: Rect, k: f32, b: [f32; 4]) {
        let at = |x: f32, y: f32| Pos2::new(page.min.x + x * k, page.min.y + y * k);
        let sel = Rect::from_min_max(at(b[0], b[1]), at(b[2], b[3]));
        let view = ui.clip_rect();
        if !view.intersects(sel) {
            return;
        }
        let mut c = *base;
        self.fill_ctx(&mut c);
        let groups = actions::toolbar_items(&self.ed.selection, &c);
        if groups.is_empty() {
            return;
        }
        // Bar height plus a gap.
        let room_above = sel.top() - view.top() > 44.0;
        let x = sel.center().x;
        let (pos, pivot) = if room_above {
            (Pos2::new(x, sel.top() - 8.0), egui::Align2::CENTER_BOTTOM)
        } else {
            (Pos2::new(x, sel.bottom() + 8.0), egui::Align2::CENTER_TOP)
        };
        egui::Area::new(egui::Id::new(("floating-toolbar", self.id)))
            .fixed_pos(pos)
            .pivot(pivot)
            .constrain_to(view)
            .show(ui.ctx(), |ui| {
                egui::Frame::popup(ui.style())
                    .inner_margin(egui::Margin::same(4))
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            for (n, group) in groups.iter().enumerate() {
                                if n > 0 {
                                    ui.separator();
                                }
                                for a in group {
                                    let b = egui::Button::new(theme::icon(a.icon).size(16.0))
                                        .frame_when_inactive(false)
                                        .min_size(egui::vec2(26.0, 26.0));
                                    let tip = actions::tooltip(ui.ctx(), a);
                                    if ui.add(b).on_hover_text(tip).clicked() {
                                        self.requested.push(a.act);
                                    }
                                    if a.act == Act::Mark(Mark::Highlight) {
                                        let more = egui::Button::new(
                                            theme::icon(icon::CARET_DOWN).size(12.0),
                                        )
                                        .frame_when_inactive(false)
                                        .min_size(egui::vec2(16.0, 26.0));
                                        let acts = &mut self.requested;
                                        egui::containers::menu::MenuButton::from_button(more)
                                            .ui(ui, |ui| markup::menu(ui, &c, acts))
                                            .0
                                            .on_hover_text("Underline, strike out, colours");
                                    }
                                }
                            }
                        });
                    });
            });
    }

    /// Up to two banners above the pages: icon, text, the banner's button if it has one,
    /// and a close button. A button's command is queued in `requested`.
    fn banners_ui(&mut self, ui: &mut egui::Ui) {
        self.refresh_marks_banner();
        let mut close = None;
        let dark = ui.visuals().dark_mode;
        for (i, b) in self.banners.iter().take(2).enumerate() {
            let top = if i == 0 { 12 } else { 6 };
            let tone = theme::tone(dark, b.tone);
            let fill = tone.map_or(ui.visuals().selection.bg_fill, |c| {
                c.gamma_multiply(if dark { 0.22 } else { 0.14 })
            });
            egui::Frame::new()
                .fill(fill)
                .corner_radius(theme::RADIUS)
                .inner_margin(egui::Margin::symmetric(12, 6))
                .outer_margin(egui::Margin {
                    left: 16,
                    right: 16,
                    top,
                    bottom: 0,
                })
                .show(ui, |ui| {
                    let text = |ui: &mut egui::Ui| {
                        let glyph = theme::icon(b.icon()).size(16.0);
                        ui.label(match tone {
                            Some(c) => glyph.color(c),
                            None => glyph,
                        });
                        ui.add(egui::Label::new(&b.text).wrap());
                    };
                    let buttons = |ui: &mut egui::Ui| {
                        let x = egui::Button::new(theme::icon(icon::X)).frame_when_inactive(false);
                        if ui.add(x).on_hover_text("Dismiss").clicked() {
                            close = Some(i);
                        }
                        if let Some((label, act)) = b.action() {
                            if ui.button(label).clicked() {
                                self.requested.push(act);
                            }
                        }
                    };
                    egui::Sides::new().shrink_left().show(ui, text, buttons);
                });
        }
        if let Some(i) = close {
            match self.banners[i].kind {
                banners::Kind::Redactions => self.marks_hidden = self.ed.redact_marks,
                banners::Kind::Signing => self.signs_hidden = self.ed.sign_items,
                _ => {}
            }
            self.banners.remove(i);
        }
    }

    /// After the document changes, count the redaction marks and the fill-and-sign items
    /// again and show, update or take away their banners, which go first while there are
    /// any.
    fn refresh_marks_banner(&mut self) {
        if self.marks_rev == self.revision {
            return;
        }
        self.marks_rev = self.revision;
        let n = self.pdf.redaction_marks().len();
        self.ed.redact_marks = n;
        // Fewer marks lower the bar, so the next one added shows the banner again.
        self.marks_hidden = self.marks_hidden.min(n);
        self.banners.retain(|b| b.kind != banners::Kind::Redactions);
        if n > self.marks_hidden {
            // After the signatures banner, which stays first.
            let at = self
                .banners
                .partition_point(|b| b.kind < banners::Kind::Redactions);
            self.banners.insert(at, banners::redactions(n));
        }
        // Fill-and-sign items still movable, likewise; after the redaction banner.
        let n = self.pdf.sign_items();
        self.ed.sign_items = n;
        self.signs_hidden = self.signs_hidden.min(n);
        self.banners.retain(|b| b.kind != banners::Kind::Signing);
        if n > self.signs_hidden {
            let at = self
                .banners
                .partition_point(|b| b.kind < banners::Kind::Signing);
            self.banners.insert(at, banners::signing(n));
        }
    }

    pub fn show_pages(&mut self, ui: &mut egui::Ui, base: &Ctx) {
        let ctx = ui.ctx().clone();
        self.collect(&ctx);
        let ppp = ctx.pixels_per_point();
        let dark = ui.visuals().dark_mode;
        let paper = self.paper();

        self.banners_ui(ui);
        let zd = ui.input(|i| i.zoom_delta());
        if zd != 1.0 {
            self.set_zoom(self.zoom * zd);
        }
        // Fitting: the widest page (two side by side when facing) fills the width between
        // the side panels, whatever the window size or which panels are open.
        let cols = if base.layout == Layout::Facing { 2 } else { 1 };
        if self.fit {
            let z = fit_zoom(&self.sizes, ui.available_width(), cols);
            self.zoom_to(z);
        }

        // The rows of pages shown: in single-page layout, just the page being gone to or
        // the current one.
        let n = self.sizes.len();
        let single = base.layout == Layout::Single;
        // Single-page layout: a wheel step past the page's top or bottom turns the page.
        // Positive: scrolling up. Ctrl or Cmd with the wheel zooms instead.
        let (wheel, now) = ui.input(|i| {
            let steps = i.events.iter().filter_map(|e| match e {
                egui::Event::MouseWheel {
                    delta, modifiers, ..
                } if !modifiers.command && !modifiers.ctrl => Some(delta.y),
                _ => None,
            });
            (steps.sum::<f32>(), i.time)
        });
        if single && wheel != 0.0 && self.goto.is_none() && ui.ui_contains_pointer() {
            let (top, bottom) = self.edge;
            let p = self.current;
            if now - self.flipped > 0.4 {
                if wheel < 0.0 && bottom && p + 1 < n {
                    self.go_to(p + 1);
                    self.flipped = now;
                } else if wheel > 0.0 && top && p > 0 {
                    // Back to the previous page's bottom, as if scrolling up into it.
                    self.go_to(p - 1);
                    self.goto_y = Some(self.sizes[p - 1].1);
                    self.flipped = now;
                }
            }
        }
        let shown = self.goto.unwrap_or(self.current).min(n.saturating_sub(1));
        let rows = match (single, n) {
            (_, 0) => Vec::new(),
            (true, _) => vec![vec![(shown, Place::Middle)]],
            (false, _) => rows_of(n, base.layout, base.cover),
        };
        let scale = BASE * self.zoom;
        let page_w: Vec<f32> = self.sizes.iter().map(|s| s.0 * scale).collect();
        let page_h: Vec<f32> = self.sizes.iter().map(|s| s.1 * scale).collect();
        let mut row_of = vec![usize::MAX; n];
        let (mut row_top, mut row_h) = (Vec::with_capacity(rows.len()), Vec::new());
        let mut widest = 0.0f32;
        let mut y = GAP;
        for (r, row) in rows.iter().enumerate() {
            let mut h = 0.0f32;
            for &(p, _) in row {
                row_of[p] = r;
                h = h.max(page_h[p]);
                widest = widest.max(page_w[p]);
            }
            row_top.push(y);
            row_h.push(h);
            y += h + GAP;
        }
        let total_h = y;
        let content_w = cols as f32 * (widest + GAP) + GAP;
        // A page's top, below its row's top when it is shorter than the row.
        let page_top = |p: usize| match row_of.get(p) {
            Some(&r) if r != usize::MAX => Some(row_top[r] + (row_h[r] - page_h[p]) / 2.0),
            _ => None,
        };

        // egui fades scroll edges towards the background, which smears grey over pages.
        ui.style_mut().spacing.scroll.fade.strength = 0.0;
        let mut area = egui::ScrollArea::both()
            .id_salt(("pages", self.id))
            .auto_shrink([false, false]);
        let rezoom = self.rezoom.take();
        if let Some(p) = self.goto.take() {
            // Within the page, keep a little context above the target.
            let top = match self.goto_y.take() {
                Some(y) => page_top(p).unwrap_or(0.0) - GAP + (y * scale - 80.0).max(0.0),
                None => row_of
                    .get(p)
                    .and_then(|&r| row_top.get(r))
                    .map_or(0.0, |t| t - GAP),
            };
            area = area.vertical_scroll_offset(top.max(0.0));
        } else if let Some((p, y)) = rezoom {
            // After a zoom change, put the same place back at the top of the view.
            let top = row_of.get(p).and_then(|&r| row_top.get(r)).copied();
            area = area.vertical_scroll_offset((top.unwrap_or(0.0) + y * scale).max(0.0));
        }
        let current = self.current_match();
        let mut wanted: Vec<(Key, f32)> = Vec::new();
        let (mut want_tiles, mut keep_tiles) = (Vec::new(), HashSet::new());
        let mut tiled = false;
        let mut visible = (usize::MAX, 0usize);
        let mut probe = shown;
        let out = area.show_viewport(ui, |ui, vp| {
            let avail_w = ui.available_width().max(content_w);
            let (rect, _) = ui.allocate_exact_size(Vec2::new(avail_w, total_h), Sense::hover());
            let painter = ui.painter().clone();
            let spine = rect.min.x + avail_w / 2.0;
            let first = row_top.partition_point(|&t| t < vp.min.y).saturating_sub(1);
            let on_view = (first..rows.len()).take_while(|&r| row_top[r] <= vp.max.y);
            for i in on_view.flat_map(|r| rows[r].iter().copied()) {
                let (i, place) = i;
                visible.0 = visible.0.min(i);
                visible.1 = visible.1.max(i);
                let x = match place {
                    Place::Middle => spine - page_w[i] / 2.0,
                    Place::Left => spine - GAP / 2.0 - page_w[i],
                    Place::Right => spine + GAP / 2.0,
                };
                let r = Rect::from_min_size(
                    Pos2::new(x, rect.min.y + page_top(i).unwrap_or(0.0)),
                    Vec2::new(page_w[i], page_h[i]),
                );
                painter.add(theme::page_shadow(dark).as_shape(r, 0));
                painter.rect_filled(r, 0.0, paper);
                let (key, key_scale, tiles) = self.page_key(&ctx, i, self.zoom * BASE * ppp);
                let failed = self.failed.contains_key(&i);
                match self.pages.get(&i) {
                    Some((_, tex)) => {
                        painter.image(
                            tex.id(),
                            r,
                            Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)),
                            Color32::WHITE,
                        );
                    }
                    None if failed => {
                        failed_page(&painter, r, &self.failed[&i]);
                    }
                    None => {
                        painter.text(
                            r.center(),
                            egui::Align2::CENTER_CENTER,
                            "Rendering…",
                            egui::FontId::proportional(14.0),
                            Color32::GRAY,
                        );
                    }
                }
                if let Some(q) = tiles.filter(|_| !failed) {
                    tiled = true;
                    self.draw_tiles(&painter, i, r, q, &mut want_tiles, &mut keep_tiles);
                }
                let matches = self.matches.get(&i).map_or(&[][..], |v| v.as_slice());
                let cur = current.filter(|c| c.0 == i).map(|c| c.1);
                self.ed
                    .overlay(ui, &self.pdf, i, r, self.zoom * BASE, matches, cur);
                let resp = self
                    .ed
                    .page_ui(ui, &self.pdf, self.id, i, r, self.zoom * BASE);
                self.context_menu(&resp, base);
                // The right-click menu replaces the toolbar while it is open.
                let menu_open = resp.context_menu_opened();
                if let Some((p, b)) = self.ed.selection_box() {
                    if p == i && !self.ed.dragging() && !menu_open {
                        self.floating_toolbar(ui, base, r, self.zoom * BASE, b);
                    }
                }
                if !self.failed.contains_key(&i)
                    && (self.pages.get(&i).map(|(k, _)| *k) != Some(key.1)
                        || self.stale.contains(&i))
                {
                    wanted.push((key, key_scale));
                }
            }
            if self.ed.selecting_words(ui.ctx()) {
                edge_scroll(ui);
            }
            let mid = vp.min.y + vp.height() / 3.0;
            let r = row_top.partition_point(|&t| t <= mid).saturating_sub(1);
            if let Some(row) = rows.get(r) {
                probe = row[0].0;
            }
        });
        // Remember the place at the top of the view (the row's first page, and points down
        // the row), so a zoom change can keep it there.
        let top = out.state.offset.y;
        let r = row_top.partition_point(|&t| t <= top).saturating_sub(1);
        if let (Some(row), Some(&t)) = (rows.get(r), row_top.get(r)) {
            self.view = (row[0].0, (top - t) / scale);
        }
        let room = out.content_size.y - out.inner_rect.height();
        self.edge = (top <= 0.5, top >= room - 0.5);
        let ed = &mut self.ed;
        ed.form.keys(&ctx, &mut ed.actions);
        self.apply_actions(&ctx);
        // The keyboard moved to a field on a page out of view.
        if let Some(p) = self.ed.form.scroll_page() {
            if p < visible.0 || p > visible.1 {
                self.go_to(p);
            }
        }
        match self.ed.follow.take() {
            Some(LinkTarget::Page(p)) => self.go_to(p),
            Some(LinkTarget::Uri(u)) => self.open_link = Some(u),
            None => {}
        }
        if self.goto.is_none() {
            self.current = probe;
        }
        for (key, scale) in wanted {
            if self.pending.insert(key) {
                self.worker.request(key, scale);
            }
        }
        // Prefetch one page either side.
        if visible.0 != usize::MAX {
            let n = self.page_count();
            for i in [
                visible.0.saturating_sub(1),
                (visible.1 + 1).min(n.saturating_sub(1)),
            ] {
                let (key, scale, _) = self.page_key(&ctx, i, self.zoom * BASE * ppp);
                if i < n
                    && !self.failed.contains_key(&i)
                    && (self.pages.get(&i).map(|(k, _)| *k) != Some(key.1)
                        || self.stale.contains(&i))
                    && self.pending.insert(key)
                {
                    self.worker.request(key, scale);
                }
            }
            for key in want_tiles {
                if self.pending.insert(key) {
                    self.worker.request(key, key.1 as f32 / 20.0);
                }
            }
            // Evict textures far from the viewport; fewer whole pages when tiles are
            // shown, as each is then the largest kind.
            let reach = if tiled { 2 } else { 6 };
            let (lo, hi) = (visible.0.saturating_sub(reach), visible.1 + reach);
            self.pages.retain(|&p, _| p >= lo && p <= hi);
            self.tiles.retain(|k, _| keep_tiles.contains(k));
            self.covered
                .retain(|&p, _| p >= visible.0 && p <= visible.1);
            let keep = |k: &Key| match k.2 {
                Some(_) => keep_tiles.contains(k),
                None => k.1 == 0 || (k.0 + 3 >= lo && k.0 <= hi + 3),
            };
            self.worker.retain(keep);
            self.pending.retain(keep);
        }
    }

    pub fn show_thumbs(&mut self, ui: &mut egui::Ui, base: &Ctx) {
        let ctx = ui.ctx().clone();
        let ppp = ctx.pixels_per_point();
        let paper = self.paper();
        let mut click: Option<(usize, egui::Modifiers)> = None;
        // Thumbnail rectangles, for finding where a drag drops.
        let mut rects: Vec<Rect> = Vec::with_capacity(self.sizes.len());
        let (mut dropped, mut target) = (false, None);
        egui::ScrollArea::vertical()
            .id_salt(("thumbs", self.id))
            .auto_shrink([false, false])
            .show(ui, |ui| {
                for i in 0..self.sizes.len() {
                    let (w, h) = self.sizes[i];
                    let th = THUMB_W * h / w.max(1.0);
                    let (rect, resp) = ui.allocate_exact_size(
                        Vec2::new(ui.available_width(), th + 22.0),
                        Sense::click_and_drag(),
                    );
                    rects.push(rect);
                    // Dragging a thumbnail moves it, or the selected pages if it is one
                    // of them.
                    if resp.drag_started() {
                        let picked = self.ed.selection.pages().is_some_and(|s| s.contains(&i));
                        if !picked {
                            self.ed.selection = Selection::pages_from([i].into());
                            self.anchor = i;
                        }
                        self.thumb_drag = Some(i);
                    }
                    if resp.drag_stopped() && self.thumb_drag == Some(i) {
                        dropped = true;
                    }
                    if !ui.is_rect_visible(rect) {
                        continue;
                    }
                    let img = Rect::from_center_size(
                        Pos2::new(rect.center().x, rect.min.y + 4.0 + th / 2.0),
                        Vec2::new(THUMB_W, th),
                    );
                    let sel = self.ed.selection.pages().is_some_and(|s| s.contains(&i));
                    if sel || i == self.current {
                        let c = if sel {
                            ui.visuals().selection.bg_fill
                        } else {
                            ui.visuals().faint_bg_color
                        };
                        ui.painter().rect_filled(rect, 4.0, c);
                    }
                    ui.painter().rect_filled(img, 0.0, paper);
                    match self.thumbs.get(&i) {
                        Some(t) => {
                            ui.painter().image(
                                t.id(),
                                img,
                                Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)),
                                Color32::WHITE,
                            );
                        }
                        None if self.failed.contains_key(&i) => {
                            ui.painter().text(
                                img.center(),
                                egui::Align2::CENTER_CENTER,
                                icon::IMAGE_BROKEN,
                                egui::FontId::new(24.0, theme::icons()),
                                Color32::GRAY,
                            );
                        }
                        None => {
                            let key = (i, 0u32, None);
                            if self.pending.insert(key) {
                                self.worker.request(key, THUMB_W * ppp / w.max(1.0));
                            }
                        }
                    }
                    ui.painter().text(
                        Pos2::new(rect.center().x, rect.max.y - 9.0),
                        egui::Align2::CENTER_CENTER,
                        self.page_name(i),
                        egui::FontId::proportional(12.0),
                        ui.visuals().text_color(),
                    );
                    if resp.clicked() {
                        click = Some((i, ui.input(|s| s.modifiers)));
                    }
                    // Right-click acts on this page, or on the selected pages if it is
                    // one of them.
                    let picked = self.ed.selection.pages().is_some_and(|s| s.contains(&i));
                    if resp.secondary_clicked() && !picked {
                        self.ed.selection = Selection::pages_from([i].into());
                        self.anchor = i;
                    }
                    self.context_menu(&resp, base);
                }
                if self.thumb_drag.is_some() {
                    let pointer = ui.input(|i| i.pointer.interact_pos());
                    if let Some(p) = pointer {
                        target = Some(drop_index(&rects, p.y));
                        drag_feedback(ui, &rects, target, p, self.moving_count());
                    }
                }
            });
        if dropped {
            self.thumb_drag = None;
            if let Some(to) = target {
                self.move_selected_pages(&ctx, to);
            }
        } else if self.thumb_drag.is_some() && !ctx.input(|i| i.pointer.any_down()) {
            // Released outside any thumbnail's response: cancel.
            self.thumb_drag = None;
        }
        if let Some((i, m)) = click {
            // Shift adds a range from the last click, Ctrl/Cmd toggles one page; either
            // way a selected object or words give way to the pages.
            let mut set = self.ed.selection.pages().cloned().unwrap_or_default();
            if m.shift {
                let (a, b) = (self.anchor.min(i), self.anchor.max(i));
                set = (a..=b).collect();
            } else if m.command {
                if !set.remove(&i) {
                    set.insert(i);
                }
                self.anchor = i;
            } else {
                set = BTreeSet::from([i]);
                self.anchor = i;
            }
            self.ed.selection = Selection::pages_from(set);
            self.go_to(i);
        }
    }
}

impl Tab {
    /// How many pages a thumbnail drag moves.
    fn moving_count(&self) -> usize {
        self.ed.selection.pages().map_or(1, |s| s.len())
    }

    /// Move the selected pages, in order, to just before page `to` (0-based; the page
    /// count means the end). Undoable; the pages stay selected in their new place.
    fn move_selected_pages(&mut self, ctx: &egui::Context, to: usize) {
        let Some(moving) = self.ed.selection.pages().cloned() else {
            return;
        };
        let n = self.page_count();
        let order = moved_order(n, &moving, to);
        if order.iter().copied().eq(0..n) {
            return;
        }
        let at = (0..to).filter(|p| !moving.contains(p)).count();
        let before = self.revision;
        self.edit(ctx, |p| p.reorder(&order));
        if self.revision != before {
            let placed: BTreeSet<usize> = (at..at + moving.len()).collect();
            self.ed.selection = Selection::pages_from(placed);
            self.anchor = at;
            self.go_to(at);
        }
    }
}

/// The zoom at which the widest of the pages (`sizes`, in points) fits in `room` screen
/// points beside the gaps, in whole percent so resizing the window does not re-render
/// the pages at every step.
/// The zoom at which `cols` of the widest page, with gaps around them, fill `room`.
fn fit_zoom(sizes: &[(f32, f32)], room: f32, cols: usize) -> f32 {
    let widest = sizes.iter().map(|s| s.0).fold(1.0, f32::max);
    let room = room - (cols + 1) as f32 * GAP - FIT_MARGIN;
    ((room / (cols as f32 * widest * BASE)) * 100.0).floor() / 100.0
}

/// Which side of the middle of the view a page is drawn on.
#[derive(Clone, Copy, PartialEq, Debug)]
enum Place {
    Middle,
    Left,
    Right,
}

/// The pages of an `n`-page document in rows, for continuous or facing pages (single
/// page shows one of these rows at a time). Facing with `cover` puts the first page
/// alone on the right, as in a book.
fn rows_of(n: usize, layout: Layout, cover: bool) -> Vec<Vec<(usize, Place)>> {
    if layout != Layout::Facing {
        return (0..n).map(|p| vec![(p, Place::Middle)]).collect();
    }
    let start = usize::from(cover).min(n);
    let mut rows: Vec<_> = (0..start).map(|p| vec![(p, Place::Right)]).collect();
    for p in (start..n).step_by(2) {
        let mut row = vec![(p, Place::Left)];
        if p + 1 < n {
            row.push((p + 1, Place::Right));
        }
        rows.push(row);
    }
    rows
}

/// What a page that couldn't be drawn shows instead: an icon, a line saying so, and the
/// reason, centred in the page's rectangle `r`.
fn failed_page(painter: &egui::Painter, r: Rect, why: &str) {
    let grey = Color32::GRAY;
    let wrap = (r.width() - 32.0).max(80.0);
    let lines = [
        (
            icon::IMAGE_BROKEN.to_string(),
            egui::FontId::new(32.0, theme::icons()),
        ),
        (
            "This page couldn't be drawn".to_string(),
            egui::FontId::proportional(15.0),
        ),
        (why.to_string(), egui::FontId::proportional(12.0)),
    ];
    let galleys: Vec<_> = lines
        .into_iter()
        .map(|(t, f)| painter.layout(t, f, grey, wrap))
        .collect();
    let gap = 6.0;
    let total: f32 = galleys.iter().map(|g| g.size().y + gap).sum::<f32>() - gap;
    let mut y = r.center().y - total / 2.0;
    for g in galleys {
        let x = r.center().x - g.size().x / 2.0;
        let h = g.size().y;
        painter.galley(Pos2::new(x, y), g, grey);
        y += h + gap;
    }
}

fn sizes_of(pdf: &Pdf) -> Vec<(f32, f32)> {
    (0..pdf.page_count())
        .map(|i| pdf.page_size(i).unwrap_or((612.0, 792.0)))
        .collect()
}

/// The page order after moving `moving` to just before `to`: for each new position,
/// the old page index.
fn moved_order(n: usize, moving: &BTreeSet<usize>, to: usize) -> Vec<usize> {
    let rest: Vec<usize> = (0..n).filter(|p| !moving.contains(p)).collect();
    let at = rest.iter().filter(|&&p| p < to).count();
    let mut order = rest[..at].to_vec();
    order.extend(moving.iter().copied());
    order.extend_from_slice(&rest[at..]);
    order
}

/// Where a thumbnail dropped at height `y` goes: before the first thumbnail whose middle
/// is below `y`, else at the end.
fn drop_index(rects: &[Rect], y: f32) -> usize {
    rects
        .iter()
        .position(|r| y < r.center().y)
        .unwrap_or(rects.len())
}

/// How fast (points per second) to scroll when the pointer is `d` points inside the edge
/// margin of the view (positive) or past the edge (larger). Zero outside the margin.
fn edge_speed(d: f32) -> f32 {
    if d > 0.0 {
        (d * 12.0).min(3000.0)
    } else {
        0.0
    }
}

/// While a text selection is dragged: scroll when the pointer is near or past an edge of
/// the pages view, faster the further out it is, and keep going while it is held still.
fn edge_scroll(ui: &egui::Ui) {
    const EDGE: f32 = 40.0;
    let Some(p) = ui.input(|i| i.pointer.latest_pos()) else {
        return;
    };
    let view = ui.clip_rect();
    let dx = edge_speed(view.left() + EDGE - p.x) - edge_speed(p.x - (view.right() - EDGE));
    let dy = edge_speed(view.top() + EDGE - p.y) - edge_speed(p.y - (view.bottom() - EDGE));
    if dx != 0.0 || dy != 0.0 {
        let dt = ui.input(|i| i.stable_dt).min(0.1);
        ui.scroll_with_delta(Vec2::new(dx, dy) * dt);
        ui.ctx().request_repaint();
    }
}

/// The order the Comments tab lists annotations in, with each one's depth: page by page in
/// the file's order, with replies under what they answer. A reply whose comment is missing
/// or on another page is listed as a comment of its own.
fn thread_order(list: &[Annot]) -> Vec<(usize, usize)> {
    const MAX_DEPTH: usize = 6;
    let n = list.len();
    let index: HashMap<(u32, u16), usize> =
        list.iter().enumerate().map(|(i, a)| (a.id, i)).collect();
    let parent: Vec<Option<usize>> = list
        .iter()
        .enumerate()
        .map(|(i, a)| {
            let p = *index.get(&a.reply_to?)?;
            (p != i && list[p].page == a.page).then_some(p)
        })
        .collect();
    let mut kids = vec![Vec::new(); n];
    for (i, p) in parent.iter().enumerate() {
        if let Some(p) = *p {
            kids[p].push(i);
        }
    }
    let mut out = Vec::with_capacity(n);
    let mut seen = vec![false; n];
    for root in (0..n).filter(|&i| parent[i].is_none()) {
        let mut stack = vec![(root, 0)];
        while let Some((i, d)) = stack.pop() {
            if std::mem::replace(&mut seen[i], true) {
                continue;
            }
            out.push((i, d.min(MAX_DEPTH)));
            stack.extend(kids[i].iter().rev().map(|&k| (k, d + 1)));
        }
    }
    // Replies that answer each other in a circle have no first comment.
    out.extend((0..n).filter(|&i| !seen[i]).map(|i| (i, 0)));
    out
}

/// While thumbnails are dragged: a line where they will land, a label at the pointer,
/// and scrolling when the pointer nears the top or bottom of the list.
fn drag_feedback(ui: &egui::Ui, rects: &[Rect], target: Option<usize>, p: Pos2, n: usize) {
    let accent = ui.visuals().selection.stroke.color;
    if let Some(t) = target {
        let y = match rects.get(t) {
            Some(r) => r.top() - 1.0,
            None => rects.last().map_or(0.0, |r| r.bottom() + 1.0),
        };
        let span = rects.first().map(|r| r.x_range());
        let x = span.unwrap_or(ui.max_rect().x_range());
        ui.painter().hline(x, y, egui::Stroke::new(3.0, accent));
    }
    ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
    let view = ui.clip_rect();
    if p.y < view.top() + 32.0 {
        ui.scroll_with_delta(Vec2::new(0.0, 8.0));
    } else if p.y > view.bottom() - 32.0 {
        ui.scroll_with_delta(Vec2::new(0.0, -8.0));
    }
    let text = if n == 1 {
        "Move 1 page".to_string()
    } else {
        format!("Move {n} pages")
    };
    let layer = egui::LayerId::new(egui::Order::Tooltip, egui::Id::new("thumb-drag"));
    let painter = ui.ctx().layer_painter(layer);
    let font = egui::FontId::proportional(12.0);
    let galley = painter.layout_no_wrap(text, font, ui.visuals().strong_text_color());
    let at = p + Vec2::new(14.0, 10.0);
    let bg = Rect::from_min_size(at, galley.size()).expand(5.0);
    painter.rect_filled(bg, 6.0, ui.visuals().window_fill);
    let edge = egui::Stroke::new(1.0, accent);
    painter.rect_stroke(bg, 6.0, edge, egui::StrokeKind::Inside);
    painter.galley(at, galley, Color32::PLACEHOLDER);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn replies_follow_what_they_answer() {
        let a = |id: u32, page: usize, reply_to: Option<u32>| Annot {
            id: (id, 0),
            page,
            kind: limarepdf_core::AnnotKind::Note,
            bbox: [0.0; 4],
            quads: Vec::new(),
            color: None,
            opacity: 1.0,
            contents: String::new(),
            author: String::new(),
            subject: String::new(),
            name: String::new(),
            modified: String::new(),
            flags: 0,
            reply_to: reply_to.map(|r| (r, 0)),
            has_appearance: true,
            text_style: None,
            shape: None,
            paths: Vec::new(),
        };
        // 3 answers 1, 4 answers 3, 5 answers a comment on another page, 6 and 7 answer
        // each other.
        let list = [
            a(1, 0, None),
            a(2, 0, None),
            a(3, 0, Some(1)),
            a(4, 0, Some(3)),
            a(5, 1, Some(2)),
            a(6, 1, Some(7)),
            a(7, 1, Some(6)),
        ];
        assert_eq!(
            thread_order(&list),
            [(0, 0), (2, 1), (3, 2), (1, 0), (4, 0), (5, 0), (6, 0)]
        );
    }

    #[test]
    fn moving_pages_keeps_their_order() {
        let set = |v: &[usize]| v.iter().copied().collect::<BTreeSet<usize>>();
        // Pages 2 and 4 (0-based 1, 3) to the start.
        assert_eq!(moved_order(5, &set(&[1, 3]), 0), [1, 3, 0, 2, 4]);
        // Page 1 to the end.
        assert_eq!(moved_order(3, &set(&[0]), 3), [1, 2, 0]);
        // Page 3 before page 2: a swap.
        assert_eq!(moved_order(3, &set(&[2]), 1), [0, 2, 1]);
        // Dropped onto itself: no change.
        assert_eq!(moved_order(3, &set(&[1]), 1), [0, 1, 2]);
        assert_eq!(moved_order(3, &set(&[1]), 2), [0, 1, 2]);
    }

    #[test]
    fn fitting_uses_the_widest_page_and_whole_percent() {
        let pages = [(612.0, 792.0), (792.0, 612.0), (300.0, 300.0)];
        let room = 900.0;
        let z = fit_zoom(&pages, room, 1);
        // The landscape page decides, and it fits beside the gaps and margin.
        let (width, space) = (792.0 * BASE * z, room - 2.0 * GAP - FIT_MARGIN);
        assert!(width <= space, "{z}");
        // Within one percent step of filling it.
        assert!(width > space - 792.0 * BASE * 0.01, "{z}");
        assert_eq!((z * 100.0).round() / 100.0, z);
        // Facing: two of the widest page and the gap between them fit.
        let z = fit_zoom(&pages, room, 2);
        let (width, space) = (2.0 * 792.0 * BASE * z, room - 3.0 * GAP - FIT_MARGIN);
        assert!(
            width <= space && width > space - 2.0 * 792.0 * BASE * 0.01,
            "{z}"
        );
    }

    #[test]
    fn facing_rows_pair_pages_after_the_cover() {
        use Place::{Left, Middle, Right};
        assert_eq!(
            rows_of(4, Layout::Facing, true),
            [
                vec![(0, Right)],
                vec![(1, Left), (2, Right)],
                vec![(3, Left)]
            ]
        );
        assert_eq!(
            rows_of(3, Layout::Facing, false),
            [vec![(0, Left), (1, Right)], vec![(2, Left)]]
        );
        assert_eq!(
            rows_of(2, Layout::Single, true),
            [[(0, Middle)], [(1, Middle)]]
        );
        assert!(rows_of(0, Layout::Facing, true).is_empty());
    }

    #[test]
    fn drops_before_the_thumbnail_under_the_upper_half() {
        let r = |y: f32| Rect::from_min_size(Pos2::new(0.0, y), Vec2::new(10.0, 100.0));
        let rects = [r(0.0), r(100.0), r(200.0)];
        assert_eq!(drop_index(&rects, 10.0), 0);
        assert_eq!(drop_index(&rects, 60.0), 1);
        assert_eq!(drop_index(&rects, 290.0), 3);
    }

    #[test]
    fn edge_scrolling_grows_past_the_edge() {
        assert_eq!(edge_speed(-5.0), 0.0);
        assert_eq!(edge_speed(0.0), 0.0);
        assert!(edge_speed(10.0) > 0.0);
        assert!(edge_speed(100.0) > edge_speed(10.0));
        assert_eq!(edge_speed(1e6), 3000.0);
    }
}

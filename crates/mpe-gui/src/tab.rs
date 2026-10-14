use crate::actions::{self, Act, Ctx};
use crate::banners::{self, Banner};
use crate::editor::{Action, Editor, Tool};
use crate::inspector::{Forms, Sec};
use crate::selection::Selection;
use crate::theme;
use crate::worker::{Key, Worker};
use eframe::egui::{self, Color32, Pos2, Rect, Sense, TextureHandle, TextureOptions, Vec2};
use egui_phosphor::regular as icon;
use mpe_core::{Annot, Encryption, LinkTarget, OutlineItem, Pdf, SearchHit};
use std::collections::{BTreeSet, HashMap, HashSet};
use std::path::PathBuf;

const GAP: f32 = 16.0;
/// Room left beside a fitted page, beyond the gaps, so it never meets the scroll bar.
const FIT_MARGIN: f32 = 12.0;
const THUMB_W: f32 = 120.0;
const MAX_UNDO: usize = 20;
/// egui points at zoom 1.0 per PDF point (96 dpi logical).
const BASE: f32 = 96.0 / 72.0;

#[derive(PartialEq, Eq, Clone, Copy)]
pub enum Side {
    Pages,
    Search,
    Bookmarks,
    Comments,
}

/// One undo or redo step: the state to go back to.
struct Step {
    /// The document's bytes; `None` when only the password settings changed.
    pdf: Option<Vec<u8>>,
    encryption: Option<Encryption>,
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
    worker: Worker,
    pub sizes: Vec<(f32, f32)>,
    pages: HashMap<usize, (u32, TextureHandle)>,
    thumbs: HashMap<usize, TextureHandle>,
    pending: HashSet<Key>,
    pub zoom: f32,
    /// The zoom follows the width of the view (on until the user zooms).
    pub fit: bool,
    /// The place at the top of the view: page, and points down it.
    view: (usize, f32),
    /// After a zoom change: the place to put back at the top of the view.
    rezoom: Option<(usize, f32)>,
    pub current: usize,
    anchor: usize,
    goto: Option<usize>,
    undo: Vec<Step>,
    redo: Vec<Step>,
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
    /// The bookmarks, read when first shown and again after the document changes:
    /// the revision they were read at, and the items.
    outline: Option<(u64, Vec<OutlineItem>)>,
    /// For each bookmark, whether its children are shown.
    outline_open: Vec<bool>,
    /// The annotations, read when the Comments tab is first shown and again after the
    /// document changes: the revision they were read at, and the list.
    comments: Option<(u64, Vec<Annot>)>,
    /// The annotation chosen in the Comments tab, outlined on its page.
    comment_sel: Option<(u32, u16)>,
}

impl Tab {
    pub fn new(
        ctx: &egui::Context,
        id: u64,
        mut pdf: Pdf,
        path: Option<PathBuf>,
    ) -> Result<Self, String> {
        let size = path.as_ref().and_then(|p| std::fs::metadata(p).ok());
        let banners = banners::detect(&pdf, size.map(|m| m.len()));
        // Form fields without drawings would show nothing; this isn't a change to save.
        pdf.refresh_appearances();
        let bytes = pdf.render_snapshot().map_err(|e| e.to_string())?;
        let sizes = sizes_of(&pdf);
        Ok(Self {
            banners,
            id,
            pdf,
            path,
            dirty: false,
            revision: 0,
            encryption: None,
            worker: Worker::spawn(ctx.clone(), bytes),
            sizes,
            pages: HashMap::new(),
            thumbs: HashMap::new(),
            pending: HashSet::new(),
            zoom: 1.0,
            fit: true,
            view: (0, 0.0),
            rezoom: None,
            current: 0,
            anchor: 0,
            goto: None,
            undo: Vec::new(),
            redo: Vec::new(),
            side: Side::Pages,
            query: String::new(),
            hits: Vec::new(),
            hit_idx: 0,
            matches: HashMap::new(),
            goto_y: None,
            error: None,
            ed: Editor::default(),
            requested: Vec::new(),
            forms: Forms::default(),
            notice: None,
            open_link: None,
            thumb_drag: None,
            stale: HashSet::new(),
            outline: None,
            outline_open: Vec::new(),
            comments: None,
            comment_sel: None,
        })
    }

    /// The file name without extension, for naming files made from the document.
    pub fn stem(&self, fallback: &str) -> String {
        self.path
            .as_ref()
            .and_then(|p| p.file_stem())
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| fallback.into())
    }

    pub fn title(&self) -> String {
        let name = self
            .path
            .as_ref()
            .and_then(|p| p.file_name())
            .map(|n| n.to_string_lossy().into_owned())
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

    // ------------------------------------------------------------ editing

    /// Run an edit with undo support. On failure the document is left unchanged, and
    /// false is returned.
    pub fn edit(
        &mut self,
        ctx: &egui::Context,
        f: impl FnOnce(&mut Pdf) -> mpe_core::Result<()>,
    ) -> bool {
        self.edit_inner(ctx, None, f)
    }

    /// Like [`Tab::edit`] for changes confined to the given pages: the old picture stays visible until the new
    /// one is rendered, and the selection is kept.
    fn edit_pages(
        &mut self,
        ctx: &egui::Context,
        pages: &[usize],
        f: impl FnOnce(&mut Pdf) -> mpe_core::Result<()>,
    ) -> bool {
        self.edit_inner(ctx, Some(pages), f)
    }

    fn edit_inner(
        &mut self,
        ctx: &egui::Context,
        soft: Option<&[usize]>,
        f: impl FnOnce(&mut Pdf) -> mpe_core::Result<()>,
    ) -> bool {
        let before = match self.pdf.snapshot() {
            Ok(b) => b,
            Err(e) => {
                self.error = Some(e.to_string());
                return false;
            }
        };
        if let Err(e) = f(&mut self.pdf) {
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

    fn soft_refresh(&mut self, ctx: &egui::Context, pages: &[usize]) {
        match self.pdf.render_snapshot() {
            Ok(bytes) => {
                self.worker = Worker::spawn(ctx.clone(), bytes);
                self.pending.clear();
                for &p in pages {
                    self.stale.insert(p);
                    self.thumbs.remove(&p);
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
            }
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
        match mpe_core::image_dimensions(&bytes) {
            Ok((w, h)) => {
                self.ed.pending_image = Some((bytes, w as f32 / h.max(1) as f32));
                self.ed.set_tool(Tool::Image);
            }
            Err(e) => self.error = Some(e.to_string()),
        }
    }

    /// Record the state before a change: the document's bytes (`None` if it did not
    /// change) and the current password settings.
    fn push_undo(&mut self, pdf: Option<Vec<u8>>) {
        let encryption = self.encryption.clone();
        self.undo.push(Step { pdf, encryption });
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
        self.forms.sec = Sec::of(self.encryption.as_ref());
        self.dirty = true;
        if pdf.is_some() {
            self.revision += 1;
            self.refresh(ctx);
        }
        Some(Step { pdf, encryption })
    }

    /// Re-render from the current document state.
    pub fn refresh(&mut self, ctx: &egui::Context) {
        match self.pdf.render_snapshot() {
            Ok(bytes) => {
                self.worker = Worker::spawn(ctx.clone(), bytes);
                self.sizes = sizes_of(&self.pdf);
                self.pages.clear();
                self.thumbs.clear();
                self.pending.clear();
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
    /// hide an entry's children. Clicking an entry goes to its page.
    pub fn show_outline(&mut self, ui: &mut egui::Ui) {
        let rev = self.revision;
        if self.outline.as_ref().is_none_or(|(r, _)| *r != rev) {
            let items = self.pdf.outline();
            self.outline_open = items.iter().map(|i| i.open).collect();
            self.outline = Some((rev, items));
        }
        let Some((_, items)) = &self.outline else {
            return;
        };
        if items.is_empty() {
            ui.weak("This document has no bookmarks.");
            return;
        }
        let mut jump = None;
        let mut toggle = None;
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
                        let row = egui::Button::selectable(false, &item.title).truncate();
                        let tip = match item.page {
                            Some(p) => format!("{}\nPage {}", item.title, p + 1),
                            None => format!("{}\nGoes outside this document", item.title),
                        };
                        let r = ui.add_enabled(item.page.is_some(), row);
                        if r.on_hover_text(tip).clicked() {
                            jump = item.page;
                        }
                    });
                    if item.has_children && !open {
                        closed_at = Some(item.depth);
                    }
                }
            });
        if let Some(i) = toggle {
            self.outline_open[i] = !self.outline_open[i];
        }
        if let Some(p) = jump {
            self.go_to(p);
        }
    }

    /// The Comments side tab: the annotations by page, replies under what they answer.
    /// Clicking one goes to it and outlines it; right-click to delete it.
    pub fn show_comments(&mut self, ui: &mut egui::Ui) {
        let rev = self.revision;
        if self.comments.as_ref().is_none_or(|(r, _)| *r != rev) {
            let list = self.pdf.annotations();
            if !list.iter().any(|a| Some(a.id) == self.comment_sel) {
                self.comment_sel = None;
            }
            self.comments = Some((rev, list));
        }
        let Some((_, list)) = &self.comments else {
            return;
        };
        if list.is_empty() {
            ui.weak("This document has no comments or markup.");
            return;
        }
        let mut chosen = None;
        let mut delete = None;
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
                    let first = a.contents.lines().find(|l| !l.trim().is_empty());
                    let who = if a.author.is_empty() { "" } else { &a.author };
                    let text = match (who, first) {
                        ("", None) => a.kind.label(),
                        ("", Some(t)) => format!("{}: {t}", a.kind.label()),
                        (w, None) => format!("{} · {w}", a.kind.label()),
                        (w, Some(t)) => format!("{w}: {t}"),
                    };
                    let mut text = egui::RichText::new(text);
                    if a.hidden() {
                        text = text.weak();
                    }
                    let mut tip = a.kind.label();
                    for extra in [&a.author, &a.modified] {
                        if !extra.is_empty() {
                            tip += &format!(" · {extra}");
                        }
                    }
                    if a.hidden() {
                        tip += " · hidden";
                    }
                    if !a.contents.is_empty() {
                        tip += &format!("\n\n{}", a.contents);
                    }
                    ui.horizontal(|ui| {
                        ui.add_space(depth as f32 * 12.0);
                        let row = egui::Button::selectable(self.comment_sel == Some(a.id), text)
                            .truncate();
                        let r = ui.add(row).on_hover_text(tip);
                        if r.clicked() {
                            chosen = Some(i);
                        }
                        r.context_menu(|ui| {
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
            self.comment_sel = Some(id);
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

    /// The outline of the annotation chosen in the Comments tab, if it is on `page`.
    fn chosen_comment(&self, page: usize) -> Option<[f32; 4]> {
        if self.side != Side::Comments {
            return None;
        }
        let id = self.comment_sel?;
        let (_, list) = self.comments.as_ref()?;
        list.iter()
            .find(|a| a.id == id && a.page == page)
            .map(|a| a.bbox)
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
            let Ok(b) = r.bitmap else { continue };
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

    /// Zoom chosen by the user: turns fitting to the width off.
    pub fn set_zoom(&mut self, z: f32) {
        self.fit = false;
        self.zoom_to(z);
    }

    /// Change the zoom, keeping the same place in the document at the top of the view.
    fn zoom_to(&mut self, z: f32) {
        let z = z.clamp(0.1, 8.0);
        if (z - self.zoom).abs() < 1e-4 {
            return;
        }
        self.zoom = z;
        self.rezoom = Some(self.view);
    }

    /// The command state for this tab's part: document, undo, selection and tool.
    pub fn fill_ctx(&self, c: &mut Ctx) {
        c.doc = true;
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
        c.fit = self.fit;
        c.hidden_text = self.ed.show_hidden;
        c.form = !self.ed.form.fields().is_empty();
        c.field = self.ed.form.focused_field().is_some();
        c.fields_shown = self.ed.form.highlight;
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
                                }
                            }
                        });
                    });
            });
    }

    /// Up to two banners above the pages: icon, text, the banner's button if it has one,
    /// and a close button. A button's command is queued in `requested`.
    fn banners_ui(&mut self, ui: &mut egui::Ui) {
        let mut close = None;
        let fill = ui.visuals().selection.bg_fill;
        for (i, b) in self.banners.iter().take(2).enumerate() {
            let top = if i == 0 { 12 } else { 6 };
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
                        ui.label(theme::icon(b.icon()).size(16.0));
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
            self.banners.remove(i);
        }
    }

    pub fn show_pages(&mut self, ui: &mut egui::Ui, base: &Ctx) {
        let ctx = ui.ctx().clone();
        self.collect(&ctx);
        let ppp = ctx.pixels_per_point();
        let dark = ui.visuals().dark_mode;

        self.banners_ui(ui);
        let zd = ui.input(|i| i.zoom_delta());
        if zd != 1.0 {
            self.set_zoom(self.zoom * zd);
        }
        // Fitting: the widest page fills the width between the side panels, whatever the
        // window size or which panels are open.
        if self.fit {
            let z = fit_zoom(&self.sizes, ui.available_width());
            self.zoom_to(z);
        }

        let page_w: Vec<f32> = self.sizes.iter().map(|s| s.0 * BASE * self.zoom).collect();
        let page_h: Vec<f32> = self.sizes.iter().map(|s| s.1 * BASE * self.zoom).collect();
        let mut tops = Vec::with_capacity(page_h.len());
        let mut y = GAP;
        for h in &page_h {
            tops.push(y);
            y += h + GAP;
        }
        let total_h = y;
        let content_w = page_w.iter().cloned().fold(0.0, f32::max) + 2.0 * GAP;

        // egui fades scroll edges towards the background, which smears grey over pages.
        ui.style_mut().spacing.scroll.fade.strength = 0.0;
        let mut area = egui::ScrollArea::both()
            .id_salt(("pages", self.id))
            .auto_shrink([false, false]);
        let rezoom = self.rezoom.take();
        if let Some(p) = self.goto.take() {
            // Within the page, keep a little context above the target.
            let within = self
                .goto_y
                .take()
                .map_or(0.0, |y| (y * BASE * self.zoom - 80.0).max(0.0));
            let top = tops.get(p).copied().unwrap_or(0.0) - GAP + within;
            area = area.vertical_scroll_offset(top.max(0.0));
        } else if let Some((p, y)) = rezoom {
            // After a zoom change, put the same place back at the top of the view.
            let top = tops.get(p).copied().unwrap_or(0.0) + y * BASE * self.zoom;
            area = area.vertical_scroll_offset(top.max(0.0));
        }
        let current = self.current_match();
        let mut wanted: Vec<(Key, f32)> = Vec::new();
        let mut visible = (usize::MAX, 0usize);
        let mut probe = self.current;
        let out = area.show_viewport(ui, |ui, vp| {
            let avail_w = ui.available_width().max(content_w);
            let (rect, _) = ui.allocate_exact_size(Vec2::new(avail_w, total_h), Sense::hover());
            let painter = ui.painter().clone();
            let first = tops.partition_point(|&t| t < vp.min.y).saturating_sub(1);
            for i in first..tops.len() {
                if tops[i] > vp.max.y {
                    break;
                }
                visible.0 = visible.0.min(i);
                visible.1 = i;
                let x = rect.min.x + (avail_w - page_w[i]) / 2.0;
                let r = Rect::from_min_size(
                    Pos2::new(x, rect.min.y + tops[i]),
                    Vec2::new(page_w[i], page_h[i]),
                );
                painter.add(theme::page_shadow(dark).as_shape(r, 0));
                painter.rect_filled(r, 0.0, Color32::WHITE);
                let scale = self.zoom * BASE * ppp;
                let key = (i, ((scale * 20.0).round() as u32).max(1));
                match self.pages.get(&i) {
                    Some((_, tex)) => {
                        painter.image(
                            tex.id(),
                            r,
                            Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)),
                            Color32::WHITE,
                        );
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
                let matches = self.matches.get(&i).map_or(&[][..], |v| v.as_slice());
                let cur = current.filter(|c| c.0 == i).map(|c| c.1);
                self.ed
                    .overlay(ui, &self.pdf, i, r, self.zoom * BASE, matches, cur);
                if let Some(b) = self.chosen_comment(i) {
                    let k = self.zoom * BASE;
                    let sr = Rect::from_min_max(
                        Pos2::new(r.min.x + b[0] * k, r.min.y + b[1] * k),
                        Pos2::new(r.min.x + b[2] * k, r.min.y + b[3] * k),
                    );
                    let stroke = egui::Stroke::new(2.0, ui.visuals().selection.stroke.color);
                    painter.rect_stroke(sr.expand(2.0), 2.0, stroke, egui::StrokeKind::Outside);
                }
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
                if self.pages.get(&i).map(|(k, _)| *k) != Some(key.1) || self.stale.contains(&i) {
                    wanted.push((key, key.1 as f32 / 20.0));
                }
            }
            if self.ed.selecting_words(ui.ctx()) {
                edge_scroll(ui);
            }
            let mid = vp.min.y + vp.height() / 3.0;
            probe = tops
                .partition_point(|&t| t <= mid)
                .saturating_sub(1)
                .min(tops.len().saturating_sub(1));
        });
        // Remember the place at the top of the view (page, and points down it), so a zoom
        // change can keep it there.
        let top = out.state.offset.y;
        let p = tops.partition_point(|&t| t <= top).saturating_sub(1);
        if let Some(&t) = tops.get(p) {
            self.view = (p, (top - t) / (BASE * self.zoom));
        }
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
                let scale = self.zoom * BASE * ppp;
                let key = (i, ((scale * 20.0).round() as u32).max(1));
                if i < n
                    && (self.pages.get(&i).map(|(k, _)| *k) != Some(key.1)
                        || self.stale.contains(&i))
                    && self.pending.insert(key)
                {
                    self.worker.request(key, key.1 as f32 / 20.0);
                }
            }
            // Evict textures far from the viewport.
            let (lo, hi) = (visible.0.saturating_sub(6), visible.1 + 6);
            self.pages.retain(|&p, _| p >= lo && p <= hi);
            let keep = |k: &Key| k.1 == 0 || (k.0 + 3 >= lo && k.0 <= hi + 3);
            self.worker.retain(keep);
            self.pending.retain(keep);
        }
    }

    pub fn show_thumbs(&mut self, ui: &mut egui::Ui, base: &Ctx) {
        let ctx = ui.ctx().clone();
        let ppp = ctx.pixels_per_point();
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
                    ui.painter().rect_filled(img, 0.0, Color32::WHITE);
                    match self.thumbs.get(&i) {
                        Some(t) => {
                            ui.painter().image(
                                t.id(),
                                img,
                                Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)),
                                Color32::WHITE,
                            );
                        }
                        None => {
                            let key = (i, 0u32);
                            if self.pending.insert(key) {
                                self.worker.request(key, THUMB_W * ppp / w.max(1.0));
                            }
                        }
                    }
                    ui.painter().text(
                        Pos2::new(rect.center().x, rect.max.y - 9.0),
                        egui::Align2::CENTER_CENTER,
                        (i + 1).to_string(),
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
fn fit_zoom(sizes: &[(f32, f32)], room: f32) -> f32 {
    let widest = sizes.iter().map(|s| s.0).fold(1.0, f32::max);
    let room = room - 2.0 * GAP - FIT_MARGIN;
    ((room / (widest * BASE)) * 100.0).floor() / 100.0
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
            kind: mpe_core::AnnotKind::Note,
            bbox: [0.0; 4],
            color: None,
            opacity: 1.0,
            contents: String::new(),
            author: String::new(),
            subject: String::new(),
            modified: String::new(),
            flags: 0,
            reply_to: reply_to.map(|r| (r, 0)),
            has_appearance: true,
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
        let z = fit_zoom(&pages, room);
        // The landscape page decides, and it fits beside the gaps and margin.
        let (width, space) = (792.0 * BASE * z, room - 2.0 * GAP - FIT_MARGIN);
        assert!(width <= space, "{z}");
        // Within one percent step of filling it.
        assert!(width > space - 792.0 * BASE * 0.01, "{z}");
        assert_eq!((z * 100.0).round() / 100.0, z);
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

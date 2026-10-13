use crate::editor::{Action, Editor, Tool};
use crate::worker::{Key, Worker};
use eframe::egui::{self, Color32, Pos2, Rect, Sense, TextureHandle, TextureOptions, Vec2};
use mpe_core::{Encryption, Pdf, SearchHit};
use std::collections::{BTreeSet, HashMap, HashSet};
use std::path::PathBuf;

const GAP: f32 = 14.0;
const THUMB_W: f32 = 120.0;
const MAX_UNDO: usize = 20;
/// egui points at zoom 1.0 per PDF point (96 dpi logical).
const BASE: f32 = 96.0 / 72.0;

#[derive(PartialEq, Eq, Clone, Copy)]
pub enum Side {
    Pages,
    Search,
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
    pub current: usize,
    pub selected: BTreeSet<usize>,
    anchor: usize,
    goto: Option<usize>,
    undo: Vec<Vec<u8>>,
    redo: Vec<Vec<u8>>,
    pub side: Side,
    pub query: String,
    pub hits: Vec<SearchHit>,
    pub hit_idx: usize,
    /// Boxes of the search matches on each page with hits (displayed-page points).
    matches: HashMap<usize, Vec<[f32; 4]>>,
    /// Scroll to this height on the `goto` page (points from its top), with `goto`.
    goto_y: Option<f32>,
    pub error: Option<String>,
    pub ed: Editor,
    /// Pages whose texture is out of date but still shown until the new render arrives.
    stale: HashSet<usize>,
}

impl Tab {
    pub fn new(
        ctx: &egui::Context,
        id: u64,
        mut pdf: Pdf,
        path: Option<PathBuf>,
    ) -> Result<Self, String> {
        let bytes = pdf.render_snapshot().map_err(|e| e.to_string())?;
        let sizes = sizes_of(&pdf);
        Ok(Self {
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
            current: 0,
            selected: BTreeSet::new(),
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
            stale: HashSet::new(),
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
        if self.selected.is_empty() {
            vec![self.current.min(self.page_count().saturating_sub(1))]
        } else {
            self.selected.iter().copied().collect()
        }
    }

    pub fn go_to(&mut self, page: usize) {
        let p = page.min(self.page_count().saturating_sub(1));
        self.current = p;
        self.goto = Some(p);
    }

    // ------------------------------------------------------------ editing

    /// Run an edit with undo support. On failure the document is left unchanged.
    pub fn edit(&mut self, ctx: &egui::Context, f: impl FnOnce(&mut Pdf) -> mpe_core::Result<()>) {
        self.edit_inner(ctx, None, f);
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
        self.undo.push(before);
        if self.undo.len() > MAX_UNDO {
            self.undo.remove(0);
        }
        self.redo.clear();
        self.dirty = true;
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
                        self.ed.sel = None;
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
                            self.ed.sel = Some((p, id));
                        } else {
                            self.ed.select_last(&self.pdf, p);
                        }
                    }
                }
                Action::PathStyle(p, id, st) => {
                    self.edit_pages(ctx, &[p], |pdf| pdf.set_path_style(p, id, &st));
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

    pub fn undo(&mut self, ctx: &egui::Context) {
        if let (Some(prev), Ok(cur)) = (self.undo.pop(), self.pdf.snapshot()) {
            if let Ok(p) = Pdf::from_bytes(&prev, None) {
                self.pdf = p;
                self.redo.push(cur);
                self.dirty = true;
                self.revision += 1;
                self.refresh(ctx);
            }
        }
    }

    pub fn redo(&mut self, ctx: &egui::Context) {
        if let (Some(next), Ok(cur)) = (self.redo.pop(), self.pdf.snapshot()) {
            if let Ok(p) = Pdf::from_bytes(&next, None) {
                self.pdf = p;
                self.undo.push(cur);
                self.dirty = true;
                self.revision += 1;
                self.refresh(ctx);
            }
        }
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
                self.ed.sel = None;
                self.ed.new_text = None;
                self.hits.clear();
                self.matches.clear();
                let n = self.page_count();
                self.selected.retain(|&p| p < n);
                self.current = self.current.min(n.saturating_sub(1));
                self.goto = Some(self.current);
            }
            Err(e) => self.error = Some(e.to_string()),
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
            if let Ok(b) = self.pdf.find_text_boxes(p, &self.query) {
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

    /// The current hit's page, and which of that page's match boxes it is (search finds
    /// hits in the extracted text and boxes in the page objects, so the counts can differ).
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
        self.goto_y = self.matches.get(&page).and_then(|v| v.get(k)).map(|b| b[1]);
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

    /// Change zoom and keep the current page at the top of the view.
    pub fn set_zoom(&mut self, z: f32) {
        self.zoom = z.clamp(0.1, 8.0);
        self.goto = Some(self.current);
    }

    pub fn fit_width(&mut self, avail: f32) {
        if let Some(&(w, _)) = self.sizes.get(self.current) {
            self.set_zoom((avail - 40.0) / (w * BASE));
        }
    }

    pub fn show_pages(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        self.collect(&ctx);
        let ppp = ctx.pixels_per_point();

        if !ctx.egui_wants_keyboard_input() {
            self.ed.keys(&ctx, &self.pdf, self.current);
        }
        let zd = ui.input(|i| i.zoom_delta());
        if zd != 1.0 {
            self.set_zoom(self.zoom * zd);
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

        let mut area = egui::ScrollArea::both()
            .id_salt(("pages", self.id))
            .auto_shrink([false, false]);
        if let Some(p) = self.goto.take() {
            // Within the page, keep a little context above the target.
            let within = self
                .goto_y
                .take()
                .map_or(0.0, |y| (y * BASE * self.zoom - 80.0).max(0.0));
            let top = tops.get(p).copied().unwrap_or(0.0) - GAP + within;
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
                painter.rect_filled(
                    r.translate(Vec2::new(2.0, 2.0)),
                    0.0,
                    Color32::from_black_alpha(50),
                );
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
                self.ed
                    .page_ui(ui, &self.pdf, self.id, i, r, self.zoom * BASE);
                if self.pages.get(&i).map(|(k, _)| *k) != Some(key.1) || self.stale.contains(&i) {
                    wanted.push((key, key.1 as f32 / 20.0));
                }
            }
            let mid = vp.min.y + vp.height() / 3.0;
            probe = tops
                .partition_point(|&t| t <= mid)
                .saturating_sub(1)
                .min(tops.len().saturating_sub(1));
        });
        let _ = out;
        self.apply_actions(&ctx);
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

    pub fn show_thumbs(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        let ppp = ctx.pixels_per_point();
        let mut click: Option<(usize, egui::Modifiers)> = None;
        egui::ScrollArea::vertical()
            .id_salt(("thumbs", self.id))
            .auto_shrink([false, false])
            .show(ui, |ui| {
                for i in 0..self.sizes.len() {
                    let (w, h) = self.sizes[i];
                    let th = THUMB_W * h / w.max(1.0);
                    let (rect, resp) = ui.allocate_exact_size(
                        Vec2::new(ui.available_width(), th + 22.0),
                        Sense::click(),
                    );
                    if !ui.is_rect_visible(rect) {
                        continue;
                    }
                    let img = Rect::from_center_size(
                        Pos2::new(rect.center().x, rect.min.y + 4.0 + th / 2.0),
                        Vec2::new(THUMB_W, th),
                    );
                    let sel = self.selected.contains(&i);
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
                }
            });
        if let Some((i, m)) = click {
            if m.shift {
                let (a, b) = (self.anchor.min(i), self.anchor.max(i));
                self.selected = (a..=b).collect();
            } else if m.command {
                if !self.selected.remove(&i) {
                    self.selected.insert(i);
                }
                self.anchor = i;
            } else {
                self.selected = BTreeSet::from([i]);
                self.anchor = i;
            }
            self.go_to(i);
        }
    }
}

fn sizes_of(pdf: &Pdf) -> Vec<(f32, f32)> {
    (0..pdf.page_count())
        .map(|i| pdf.page_size(i).unwrap_or((612.0, 792.0)))
        .collect()
}

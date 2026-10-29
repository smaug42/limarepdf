//! What the inspector's page settings show on the pages while their section is open: the
//! crop box, on pages drawn whole, with handles to drag it; and the watermark and headers
//! and footers being set, until they are added. Once their settings settle, the pages and
//! thumbnails are drawn with the stamp put on (`proofs.rs`); while the settings are being
//! changed, and until those drawings come, the stamp is drawn over the pages. Those drawn
//! previews are laid out by core's own functions (`Watermark::text_start`, `picture_size`,
//! `HeaderFooter::lines`), so they sit where the stamp will be written, in Liberation Sans,
//! which has Helvetica's widths; a stamp set behind the content is drawn over it.

use super::{Stamp, StampForm, Tab};
use crate::inspector::Section;
use crate::page_text;
use crate::typing_fonts::{self, Wanted};
use eframe::egui::{self, Color32, CursorIcon, Pos2, Rect, Sense, Stroke, StrokeKind, Vec2};
use limarepdf_core::{parse_page_range, picture_size, today_iso, writable, FontChoice};
use std::path::PathBuf;

/// How near an edge of the crop box (screen points) the pointer takes it.
const GRAB: f32 = 8.0;
/// The least a crop leaves of a page, in points either way.
const MIN_CROP: f32 = 18.0;
/// The longest side of the watermark picture's preview, in pixels.
const PREVIEW_MAX: u32 = 1024;

/// The crop box being dragged on a page: the page, which sides move (top, right, bottom,
/// left, as the margins), the margins when the drag started and where it started.
pub(super) struct CropDrag {
    page: usize,
    sides: [bool; 4],
    from: [f32; 4],
    origin: Pos2,
}

/// The watermark picture's file, and its texture and size in pixels if it could be read.
pub(super) type Picture = (PathBuf, Option<(egui::TextureHandle, (f32, f32))>);

/// What the pages show this frame for the inspector: the pages the crop box can be dragged
/// on, and those the watermark and the headers and footers are previewed on (each with
/// its place among the pages stamped, for the Bates number), and whether they are drawn
/// with the stamps put on where those drawings have come.
#[derive(Default)]
pub(super) struct Previews {
    crop: Vec<usize>,
    wm: Vec<usize>,
    hf: Vec<usize>,
    proof: bool,
}

/// `s` as Helvetica (or its embedded face) writes it: characters neither can write become
/// `?`.
fn written(s: &str) -> String {
    let helvetica = FontChoice::default();
    s.chars()
        .map(|c| if writable(helvetica, c) { c } else { '?' })
        .collect()
}

/// The distance from the top of `font`'s first row to its baseline, as `page_text` measures
/// it.
fn ascent(painter: &egui::Painter, font: &egui::FontId) -> f32 {
    let probe = painter.layout_no_wrap("M".into(), font.clone(), Color32::WHITE);
    probe
        .rows
        .first()
        .and_then(|row| Some(row.pos.y + row.glyphs.first()?.pos.y))
        .unwrap_or(font.size * 0.9)
}

impl Tab {
    /// Show the pages whole while the Crop section is open, and work out what else to show
    /// on them. Called before the pages are laid out.
    pub(super) fn previews(&mut self, ctx: &egui::Context) -> Previews {
        let pass = ctx.cumulative_pass_nr();
        // A form field typed in, and the panel no longer drawn (a tool chosen): kept.
        if self.forms.typing() && !self.forms.drawn(pass) {
            self.keep_forms(ctx);
        }
        let cropping = self.forms.open(Section::Crop, pass);
        self.set_crop_view(ctx, cropping);
        if cropping {
            self.sync_proofs(ctx, Vec::new());
            let n = self.page_count();
            return Previews {
                crop: parse_page_range(&self.forms.crop.pages, n).unwrap_or_default(),
                ..Previews::default()
            };
        }
        let stamps = self.stamping(pass);
        let pages = |kind: Stamp| {
            let of = stamps.iter().filter(|(f, _)| f.kind() == kind);
            of.flat_map(|(_, pages)| pages.iter().copied()).collect()
        };
        let (wm, hf) = (pages(Stamp::Watermark), pages(Stamp::HeaderFooter));
        Previews {
            crop: Vec::new(),
            wm,
            hf,
            proof: self.sync_proofs(ctx, stamps),
        }
    }

    /// The stamps the pages show until they are added, each with the pages it is for: the
    /// watermark and the headers and footers, while their section is open (in pass `pass`
    /// or the one before) and they aren't added or linked to one added, which the pages
    /// show as it is.
    pub(super) fn stamping(&self, pass: u64) -> Vec<(StampForm, Vec<usize>)> {
        let f = &self.forms;
        let shows = |s: Section, kind: Stamp, added: bool| {
            f.open(s, pass) && self.linked(kind).is_none() && !added
        };
        let mut stamps = Vec::new();
        if shows(Section::Watermark, Stamp::Watermark, f.wm_added) && f.wm.ready() {
            stamps.push((StampForm::Wm(f.wm.clone()), &f.wm.range));
        }
        if shows(Section::HeaderFooter, Stamp::HeaderFooter, f.hf_added) {
            stamps.push((StampForm::Hf(f.hf.clone()), &f.hf.range));
        }
        let n = self.page_count();
        let pages = |range: &str| parse_page_range(range, n).unwrap_or_default();
        let stamps = stamps.into_iter().map(|(form, range)| (form, pages(range)));
        stamps.filter(|(_, pages)| !pages.is_empty()).collect()
    }

    /// Draw page `page`'s previews, its screen rectangle `r` and `k` screen points per PDF
    /// point: the page drawn with the stamps put on, once that has come, else the stamps
    /// drawn over it.
    pub(super) fn draw_previews(
        &mut self,
        ui: &egui::Ui,
        p: &Previews,
        page: usize,
        r: Rect,
        k: f32,
    ) {
        if p.proof && self.proof_page(ui.painter(), page, r) {
            return;
        }
        if p.wm.contains(&page) {
            self.preview_watermark(ui, page, r, k);
        }
        if let Some(seq) = p.hf.iter().position(|&i| i == page) {
            self.preview_header_footer(ui, page, seq, r, k);
        }
    }

    fn preview_watermark(&mut self, ui: &egui::Ui, page: usize, r: Rect, k: f32) {
        let painter = ui.painter();
        let s = &self.forms.wm;
        let (sin, cos) = s.rotation.to_radians().sin_cos();
        // A point in points from the page's middle (y up), turned with the watermark.
        let centre = r.center();
        let at =
            |x: f32, y: f32| centre + Vec2::new(k * (cos * x - sin * y), -k * (sin * x + cos * y));
        if s.picture {
            let Some(path) = s.image.clone() else {
                return;
            };
            if self.wm_picture.as_ref().map(|(p, _)| p) != Some(&path) {
                let texture = std::fs::read(&path)
                    .ok()
                    .and_then(|b| limarepdf_core::picture_preview(&b, PREVIEW_MAX).ok())
                    .map(|(w, h, px)| {
                        let size = [w as usize, h as usize];
                        let img = egui::ColorImage::from_rgba_unmultiplied(size, &px);
                        let tex = ui.ctx().load_texture(
                            "watermark-preview",
                            img,
                            egui::TextureOptions::LINEAR,
                        );
                        (tex, (w as f32, h as f32))
                    });
                self.wm_picture = Some((path, texture));
            }
            let Some((_, Some((tex, (iw, ih))))) = &self.wm_picture else {
                return;
            };
            let s = &self.forms.wm;
            let (vw, vh) = self.sizes.get(page).copied().unwrap_or((612.0, 792.0));
            let (pw, ph) = picture_size(s.scale / 100.0, *iw, *ih, vw, vh);
            let (x, y) = (pw / 2.0, ph / 2.0);
            let tint = Color32::from_white_alpha((s.opacity.clamp(0.0, 1.0) * 255.0) as u8);
            let mut mesh = egui::Mesh::with_texture(tex.id());
            for (corner, uv) in [
                (at(-x, y), Pos2::new(0.0, 0.0)),
                (at(x, y), Pos2::new(1.0, 0.0)),
                (at(x, -y), Pos2::new(1.0, 1.0)),
                (at(-x, -y), Pos2::new(0.0, 1.0)),
            ] {
                mesh.vertices.push(egui::epaint::Vertex {
                    pos: corner,
                    uv,
                    color: tint,
                });
            }
            mesh.add_triangle(0, 1, 2);
            mesh.add_triangle(0, 2, 3);
            painter.add(mesh);
            return;
        }
        let colour = page_text::shown(self.colours.mode, s.color).gamma_multiply(s.opacity);
        let family = typing_fonts::family(ui.ctx(), Wanted::Standard(FontChoice::default()));
        let font = egui::FontId::new(s.size * k, family);
        let rise = ascent(painter, &font);
        let galley = painter.layout_no_wrap(written(&s.text), font, colour);
        let (tx, ty) = s.watermark().text_start();
        // The galley turns about its top left corner, the ascent above the baseline's start.
        let pos = at(tx, ty) + Vec2::new(-sin, -cos) * rise;
        let text =
            egui::epaint::TextShape::new(pos, galley, colour).with_angle(-s.rotation.to_radians());
        painter.add(text);
    }

    fn preview_header_footer(&self, ui: &egui::Ui, page: usize, seq: usize, r: Rect, k: f32) {
        let painter = ui.painter();
        let s = &self.forms.hf;
        let size = self.sizes.get(page).copied().unwrap_or((612.0, 792.0));
        let colour = page_text::shown(self.colours.mode, [0.0; 3]);
        let family = typing_fonts::family(ui.ctx(), Wanted::Standard(FontChoice::default()));
        let font = egui::FontId::new(s.size * k, family);
        let rise = ascent(painter, &font);
        let lines = s
            .header_footer()
            .lines(page, seq, self.page_count(), size, &today_iso());
        for l in lines {
            let galley = painter.layout_no_wrap(written(&l.text), font.clone(), colour);
            let base = Pos2::new(r.min.x + l.x * k, r.max.y - l.y * k);
            painter.galley(base - Vec2::new(0.0, rise), galley, colour);
        }
    }

    /// Page `page` while the crop is being set: the area outside its crop box dimmed, the
    /// box outlined, and on the pages it is being set for (`p`), handles to drag its edges
    /// and corners. Letting go crops those pages to it, one undo step.
    pub(super) fn crop_page_ui(
        &mut self,
        ui: &mut egui::Ui,
        p: &Previews,
        page: usize,
        r: Rect,
        k: f32,
    ) {
        let ctx = ui.ctx().clone();
        let settable = p.crop.contains(&page);
        let live = self.crop_drag.is_some() || self.forms.crop.dragged;
        let own = self
            .crop_view
            .as_ref()
            .and_then(|m| m.get(page).copied())
            .unwrap_or_default();
        let m = if settable && live {
            self.forms.crop.margins
        } else {
            own
        };
        let shown = |m: [f32; 4]| {
            Rect::from_min_max(
                Pos2::new(r.min.x + m[3] * k, r.min.y + m[0] * k),
                Pos2::new(r.max.x - m[1] * k, r.max.y - m[2] * k),
            )
        };
        let b = shown(m);
        let painter = ui.painter().clone();
        let dim = Color32::from_black_alpha(110);
        for part in [
            Rect::from_min_max(r.min, Pos2::new(r.max.x, b.min.y)),
            Rect::from_min_max(Pos2::new(r.min.x, b.max.y), r.max),
            Rect::from_min_max(Pos2::new(r.min.x, b.min.y), Pos2::new(b.min.x, b.max.y)),
            Rect::from_min_max(Pos2::new(b.max.x, b.min.y), Pos2::new(r.max.x, b.max.y)),
        ] {
            if part.is_positive() {
                painter.rect_filled(part, 0.0, dim);
            }
        }
        let accent = ui.visuals().selection.stroke.color;
        painter.rect_stroke(b, 0.0, Stroke::new(1.5, accent), StrokeKind::Middle);
        if !settable {
            return;
        }
        for c in [
            b.left_top(),
            b.center_top(),
            b.right_top(),
            b.right_center(),
            b.right_bottom(),
            b.center_bottom(),
            b.left_bottom(),
            b.left_center(),
        ] {
            let h = Rect::from_center_size(c, Vec2::splat(7.0));
            painter.rect_filled(h, 1.0, Color32::WHITE);
            painter.rect_stroke(h, 1.0, Stroke::new(1.0, accent), StrokeKind::Middle);
        }
        // Which sides a press at `at` takes: those whose edge is near it, along the box.
        let sides = |at: Pos2| {
            let along_x = at.x > b.min.x - GRAB && at.x < b.max.x + GRAB;
            let along_y = at.y > b.min.y - GRAB && at.y < b.max.y + GRAB;
            [
                along_x && (at.y - b.min.y).abs() < GRAB,
                along_y && (at.x - b.max.x).abs() < GRAB,
                along_x && (at.y - b.max.y).abs() < GRAB,
                along_y && (at.x - b.min.x).abs() < GRAB,
            ]
        };
        let id = egui::Id::unique(("crop-box", self.id, page));
        // The page and the room around it to take its edges; a click there, not a drag,
        // ends the crop, as a click away from a picture ends Word's.
        let resp = ui.interact(r.expand(GRAB), id, Sense::click_and_drag());
        if resp.clicked() {
            self.end_crop();
        }
        resp.widget_info(|| {
            let name = format!("Crop box on page {}", page + 1);
            egui::WidgetInfo::labeled(egui::Role::Unknown, true, name)
        });
        let grabbed = self
            .crop_drag
            .as_ref()
            .filter(|d| d.page == page)
            .map(|d| d.sides)
            .or_else(|| resp.hover_pos().map(sides));
        if let Some(s) = grabbed {
            let icon = match s {
                [true, false, false, true] | [false, true, true, false] => CursorIcon::ResizeNwSe,
                [true, true, false, false] | [false, false, true, true] => CursorIcon::ResizeNeSw,
                [true, _, _, _] | [_, _, true, _] => CursorIcon::ResizeVertical,
                [_, true, _, _] | [_, _, _, true] => CursorIcon::ResizeHorizontal,
                _ => CursorIcon::Default,
            };
            if icon != CursorIcon::Default {
                ctx.set_cursor_icon(icon);
            }
        }
        if resp.drag_started() {
            let origin = ctx
                .input(|i| i.pointer.press_origin())
                .unwrap_or(r.center());
            let s = sides(origin);
            if s.contains(&true) {
                self.crop_drag = Some(CropDrag {
                    page,
                    sides: s,
                    from: own,
                    origin,
                });
            }
        }
        let Some(d) = self.crop_drag.as_ref().filter(|d| d.page == page) else {
            return;
        };
        if let Some(at) = resp.interact_pointer_pos() {
            let delta = (at - d.origin) / k;
            let (w, h) = (r.width() / k, r.height() / k);
            let mut m = d.from;
            let [top, right, bottom, left] = d.sides;
            // Whole points, each side stopping short of the one across from it.
            let fit = |v: f32, room: f32| v.round().min(room - MIN_CROP).max(0.0);
            if top {
                m[0] = fit(d.from[0] + delta.y, h - m[2]);
            }
            if bottom {
                m[2] = fit(d.from[2] - delta.y, h - m[0]);
            }
            if left {
                m[3] = fit(d.from[3] + delta.x, w - m[1]);
            }
            if right {
                m[1] = fit(d.from[1] - delta.x, w - m[3]);
            }
            self.forms.crop.margins = m;
        }
        if resp.drag_stopped() {
            self.crop_drag = None;
            let m = self.forms.crop.margins;
            let pages = p.crop.clone();
            self.crop(&ctx, &pages, m, None);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::actions::Ctx;
    use crate::inspector::{self, Wm};
    use crate::theme;
    use crate::worker::Colours;
    use egui::TextureId;
    use limarepdf_core::render::{self, Renderer};
    use limarepdf_core::Pdf;
    use std::collections::HashMap;
    use std::sync::Arc;
    use std::time::{Duration, Instant};

    /// The window the tests draw, in points.
    const SCREEN: Vec2 = Vec2::new(1200.0, 900.0);
    /// A blank part of the inspector, below its controls.
    const BLANK: Pos2 = Pos2::new(1040.0, 880.0);

    /// What a frame drew: the pictures in the order painted, with where, and the text,
    /// with where it starts.
    #[derive(Default)]
    struct Drawn {
        images: Vec<(TextureId, Rect)>,
        texts: Vec<(String, Pos2)>,
    }

    impl Drawn {
        fn add(&mut self, s: &egui::Shape) {
            match s {
                egui::Shape::Vec(v) => v.iter().for_each(|s| self.add(s)),
                egui::Shape::Mesh(m) if m.texture_id != TextureId::default() => {
                    self.images.push((m.texture_id, m.calc_bounds()));
                }
                egui::Shape::Text(t) => self.texts.push((t.galley.text().to_string(), t.pos)),
                _ => {}
            }
        }

        /// The picture on top of the `nth` page from the top (`wide`), or thumbnail.
        fn on_top(&self, wide: bool, nth: usize) -> Option<TextureId> {
            let mut rects: Vec<Rect> = self.images.iter().map(|(_, r)| *r).collect();
            rects.retain(|r| (r.width() > 300.0) == wide);
            rects.sort_by(|a, b| a.min.y.total_cmp(&b.min.y));
            rects.dedup_by(|a, b| a.min.distance(b.min) < 0.5);
            let r = rects.get(nth)?;
            let same = |x: &Rect| x.min.distance(r.min) < 0.5 && x.max.distance(r.max) < 0.5;
            self.images.iter().rev().find(|(_, x)| same(x)).map(|i| i.0)
        }

        /// Whether the text `s` was drawn on a page: a stamp's drawn preview.
        fn says(&self, s: &str) -> bool {
            let on_page = |at: Pos2| {
                let pages = self.images.iter().filter(|(_, r)| r.width() > 300.0);
                pages.map(|i| i.1).any(|r| r.contains(at))
            };
            self.texts.iter().any(|(t, at)| t == s && on_page(*at))
        }
    }

    /// A tab drawn as the app lays it out: the thumbnails, the inspector with the
    /// Watermark tool open, and the pages.
    struct Window {
        ctx: egui::Context,
        t: Tab,
        /// Every picture loaded so far, by texture.
        loaded: HashMap<TextureId, Arc<egui::ColorImage>>,
    }

    impl Window {
        /// A tab of `pages` Letter pages, each with a line of large black capitals across
        /// its middle, which a watermark there crosses.
        fn new(pages: usize) -> Self {
            let ctx = egui::Context::default();
            theme::install(&ctx);
            let mut pdf = Pdf::new();
            let bold = FontChoice {
                bold: true,
                ..FontChoice::default()
            };
            for i in 0..pages {
                pdf.insert_blank(i, 612.0, 792.0).unwrap();
                pdf.add_text(i, (40.0, 440.0), "HHHHHH", bold, 120.0, [0.0; 3])
                    .unwrap();
            }
            let t = Tab::new(&ctx, 1, pdf, None, Colours::default()).unwrap();
            let mut w = Window {
                ctx,
                t,
                loaded: HashMap::new(),
            };
            w.t.forms.choose(Section::Watermark);
            w.frame(vec![]);
            w
        }

        fn frame(&mut self, events: Vec<egui::Event>) -> Drawn {
            let raw = egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, SCREEN)),
                events,
                ..Default::default()
            };
            let mut c = Ctx::default();
            self.t.fill_ctx(&mut c);
            let t = &mut self.t;
            let mut out = self.ctx.run_ui(raw, |ui| {
                egui::Panel::left("side")
                    .exact_size(170.0)
                    .show(ui, |ui| t.show_thumbs(ui, &c));
                egui::Panel::right("inspector")
                    .exact_size(320.0)
                    .show(ui, |ui| inspector::show(ui, t, &c));
                egui::CentralPanel::default().show(ui, |ui| t.show_pages(ui, &c));
            });
            let set = std::mem::take(&mut out.textures_delta.set);
            out.textures_delta.clear();
            for (id, set) in set {
                for d in set.into_iter().filter(|d| d.pos.is_none()) {
                    let egui::ImageData::Color(image) = d.image;
                    self.loaded.insert(id, image);
                }
            }
            let mut d = Drawn::default();
            for s in &out.shapes {
                d.add(&s.shape);
            }
            d
        }

        /// Draw frames until `done` gives what it waits for, for 20 seconds at most.
        fn until<T>(&mut self, what: &str, mut done: impl FnMut(&Self, &Drawn) -> Option<T>) -> T {
            let start = Instant::now();
            while start.elapsed() < Duration::from_secs(20) {
                let d = self.frame(vec![]);
                if let Some(v) = done(self, &d) {
                    return v;
                }
                std::thread::sleep(Duration::from_millis(10));
            }
            panic!("{what} never came");
        }

        /// Page `page`'s own picture, drawn from the document as it is, and its quantised
        /// scale.
        fn page(&self, page: usize) -> Option<(TextureId, u32)> {
            let (q, tex) = self.t.pages.get(&page)?;
            (!self.t.stale.contains(&page)).then_some((tex.id(), *q))
        }

        /// The picture shown on page `page` once it isn't the page's own: the preview.
        fn preview(&mut self, page: usize) -> TextureId {
            self.until("the preview", |w, d| {
                let top = d.on_top(true, page)?;
                let own = w.t.pages.get(&page).map(|p| p.1.id());
                (Some(top) != own && w.loaded.contains_key(&top)).then_some(top)
            })
        }

        fn pixels(&self, id: TextureId) -> &[Color32] {
            &self.loaded[&id].pixels
        }
    }

    fn press(at: Pos2, down: bool) -> egui::Event {
        egui::Event::PointerButton {
            pos: at,
            button: egui::PointerButton::Primary,
            pressed: down,
            modifiers: Default::default(),
        }
    }

    /// Page `page` of `pdf` as the app draws it at quantised scale `q`, after the
    /// watermark `wm`, if any, is put on `pages` of a copy.
    fn drawn_with(pdf: &Pdf, wm: Option<(&Wm, &[usize])>, page: usize, q: u32) -> Vec<Color32> {
        let mut copy = pdf.clone();
        if let Some((wm, pages)) = wm {
            copy.add_watermark(pages, &wm.watermark()).unwrap();
        }
        let parsed = render::load(copy.app_snapshot().unwrap(), None).unwrap();
        let r = Renderer::new(&parsed);
        let b = r.render(page, q as f32 / 20.0).unwrap();
        let (px, _) = b.rgba.as_chunks::<4>();
        let colour = |&[r, g, b, a]: &[u8; 4]| Color32::from_rgba_premultiplied(r, g, b, a);
        px.iter().map(colour).collect()
    }

    /// Whether `a` and `b` are the same picture, but for a few pixels a little different.
    fn alike(a: &[Color32], b: &[Color32]) -> bool {
        let differ = |(x, y): (&Color32, &Color32)| {
            let (x, y) = (x.to_array(), y.to_array());
            (0..4).any(|i| x[i].abs_diff(y[i]) > 8)
        };
        a.len() == b.len() && a.iter().zip(b).filter(|p| differ(*p)).count() * 1000 <= a.len()
    }

    fn dark(c: Color32) -> bool {
        c.r() < 60 && c.g() < 60 && c.b() < 60
    }

    fn red(c: Color32) -> bool {
        c.r() > 150 && c.g() < 60 && c.b() < 60
    }

    fn white(c: Color32) -> bool {
        c.r() > 200 && c.g() > 200 && c.b() > 200
    }

    /// A big, solid red watermark across the capitals, set behind them or above.
    fn crossing(foreground: bool) -> Wm {
        Wm {
            text: "WWWW".into(),
            size: 150.0,
            rotation: 0.0,
            opacity: 1.0,
            color: [0.9, 0.0, 0.0],
            foreground,
            ..Wm::default()
        }
    }

    /// ENH-23: a watermark behind the content is previewed behind it, in a drawing of the
    /// page with the watermark put on, not drawn over the page: where the capitals cross
    /// it they stay black, and the rest of it shows.
    #[test]
    fn a_watermark_behind_the_content_is_previewed_behind_its_text() {
        let mut w = Window::new(1);
        let (_, q) = w.until("the page", |w, _| w.page(0));
        let wm = crossing(false);
        w.t.forms.wm = wm.clone();
        let plain = drawn_with(&w.t.pdf, None, 0, q);
        let mut blank = Pdf::new();
        blank.insert_blank(0, 612.0, 792.0).unwrap();
        let alone = drawn_with(&blank, Some((&wm, &[0])), 0, q);
        let shown = w.preview(0);
        let d = w.frame(vec![]);
        assert_eq!(d.on_top(true, 0), Some(shown));
        assert!(!d.says("WWWW"), "drawn over the preview");
        let p = w.pixels(shown);
        assert_eq!(p.len(), plain.len());
        let crossed: Vec<usize> = (0..p.len())
            .filter(|&i| dark(plain[i]) && red(alone[i]))
            .collect();
        assert!(crossed.len() > 500, "{}", crossed.len());
        let n = crossed.len();
        let black = crossed.iter().filter(|&&i| dark(p[i])).count();
        assert!(black * 100 >= n * 99, "{black} of {n}");
        let bare: Vec<usize> = (0..p.len())
            .filter(|&i| white(plain[i]) && red(alone[i]))
            .collect();
        let shows = bare.iter().filter(|&&i| red(p[i])).count();
        assert!(shows * 100 >= bare.len() * 95, "{shows} of {}", bare.len());
    }

    /// ENH-23: the preview is the page as adding the watermark makes it; once added, the
    /// page is drawn from the document again and the preview is gone.
    #[test]
    fn the_preview_is_the_page_as_adding_the_stamp_makes_it() {
        let mut w = Window::new(1);
        w.until("the page", |w, _| w.page(0));
        w.t.forms.wm = Wm {
            rotation: 30.0,
            opacity: 0.5,
            ..crossing(false)
        };
        let shown = w.preview(0);
        let preview = w.pixels(shown).to_vec();
        let ctx = w.ctx.clone();
        w.t.stamp_from_form(&ctx, Stamp::Watermark, false);
        assert!(w.t.linked(Stamp::Watermark).is_some());
        let added = w.until("the page with the watermark", |w, d| {
            let (own, _) = w.page(0)?;
            (d.on_top(true, 0) == Some(own)).then_some(own)
        });
        assert!(alike(&preview, w.pixels(added)));
    }

    /// ENH-23: previewing changes nothing: the document, its undo and redo, its revision
    /// and whether it has unsaved changes; and closing the tool drops the preview.
    #[test]
    fn previewing_leaves_the_document_and_its_history_alone() {
        let mut w = Window::new(2);
        let ctx = w.ctx.clone();
        assert!(w.t.edit(&ctx, |p| p.insert_blank(2, 612.0, 792.0)));
        w.t.undo(&ctx);
        let state = w.t.state();
        let (history, unsaved) = (w.t.history(), w.t.unsaved());
        // Of a copy: writing the document out adds to it (lopdf's cross-reference stream).
        let written = |t: &Tab| t.pdf.clone().snapshot().unwrap();
        let bytes = written(&w.t);
        w.until("the page", |w, _| w.page(0));
        w.t.forms.wm = crossing(true);
        w.preview(0);
        let same = |w: &mut Window| {
            assert!(w.t.state() == state, "the tab's state changed");
            assert_eq!(w.t.history(), history);
            assert_eq!(w.t.unsaved(), unsaved);
            assert!(written(&w.t) == bytes, "the document changed");
        };
        same(&mut w);
        w.t.forms.close = Some(Section::Watermark);
        w.until("the page without the preview", |w, d| {
            let (own, _) = w.page(0)?;
            (d.on_top(true, 0) == Some(own) && !d.says("WWWW")).then_some(())
        });
        same(&mut w);
    }

    /// ENH-23: a drawing made for settings since changed is never shown. While a setting is
    /// being changed (the pointer held), the page shows the drawn preview; once let go, the
    /// drawing for the new settings, never one for settings passed through.
    #[test]
    fn a_render_for_settings_since_changed_is_never_shown() {
        let mut w = Window::new(1);
        let (_, q) = w.until("the page", |w, _| w.page(0));
        let wm = crossing(true);
        w.t.forms.wm = wm.clone();
        let first = w.preview(0);
        let pdf = w.t.pdf.clone();
        let with = |text: &str| {
            let wm = Wm {
                text: text.into(),
                ..wm.clone()
            };
            drawn_with(&pdf, Some((&wm, &[0])), 0, q)
        };
        let (m, n) = (with("MMMM"), with("NNNN"));
        assert!(alike(w.pixels(first), &with("WWWW")));
        // Held: the drawn preview of the new text, on the page's own picture.
        w.frame(vec![egui::Event::PointerMoved(BLANK), press(BLANK, true)]);
        w.t.forms.wm.text = "MMMM".into();
        for _ in 0..20 {
            let d = w.frame(vec![]);
            let own = w.t.pages.get(&0).map(|p| p.1.id());
            assert_eq!(d.on_top(true, 0), own);
            assert!(d.says("MMMM"));
            std::thread::sleep(Duration::from_millis(20));
        }
        // Let go, then changed again at once: only the last settings' drawing shows.
        w.frame(vec![press(BLANK, false)]);
        w.t.forms.wm.text = "NNNN".into();
        w.until("the preview of the last settings", |w, d| {
            let top = d.on_top(true, 0)?;
            if Some(top) == w.t.pages.get(&0).map(|p| p.1.id()) {
                assert!(d.says("NNNN"));
                return None;
            }
            let p = w.pixels(top);
            assert!(top != first && !alike(p, &m), "a stale preview was shown");
            assert!(alike(p, &n));
            Some(())
        });
    }

    /// ENH-23: at high zoom, where a page is drawn in tiles over its whole drawing, the
    /// preview is too: its tiles, with the watermark, over the page's own.
    #[test]
    fn at_high_zoom_the_preview_is_drawn_in_tiles() {
        let mut w = Window::new(1);
        w.t.forms.wm = crossing(true);
        w.t.set_zoom(4.0);
        w.t.go_to_at(0, 380.0);
        w.until("a tile of the preview", |w, d| {
            let own: Vec<TextureId> = w.t.tiles.values().map(|t| t.1.id()).collect();
            // Tile-sized, over one of the page's own tiles, in the same place.
            let over = |i: usize, r: &Rect| {
                let under = &d.images[..i];
                let same = |x: &Rect| x.min.distance(r.min) < 0.5 && x.max.distance(r.max) < 0.5;
                under.iter().any(|(id, x)| own.contains(id) && same(x))
            };
            let stamped = |id: &TextureId| w.loaded[id].pixels.iter().any(|&c| red(c));
            let mut tiles = d.images.iter().enumerate().filter(|(i, (id, r))| {
                !own.contains(id) && w.loaded.contains_key(id) && over(*i, r)
            });
            tiles.any(|(_, (id, _))| stamped(id)).then_some(())
        });
    }

    /// ENH-23: the thumbnails of the pages being stamped show the stamp, and only theirs.
    #[test]
    fn the_thumbnails_of_stamped_pages_show_the_stamp() {
        let mut w = Window::new(2);
        w.t.forms.wm = Wm {
            range: "2".into(),
            ..crossing(true)
        };
        let reds = |w: &Window, id: TextureId| w.pixels(id).iter().filter(|&&c| red(c)).count();
        w.until("the second thumbnail's preview", |w, d| {
            let second = d.on_top(false, 1).filter(|id| w.loaded.contains_key(id))?;
            (reds(w, second) > 20).then_some(())
        });
        let d = w.frame(vec![]);
        let first = d.on_top(false, 0).unwrap();
        assert_eq!(Some(first), w.t.thumbs.get(&0).map(|t| t.id()));
        assert_eq!(reds(&w, first), 0);
    }
}

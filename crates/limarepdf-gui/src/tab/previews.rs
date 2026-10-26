//! What the inspector's page settings show on the pages while their section is open: the
//! crop box, on pages drawn whole, with handles to drag it; and the watermark and headers
//! and footers being set, drawn over the pages until they are added. The previews are
//! laid out by core's own functions (`Watermark::text_start`, `picture_size`,
//! `HeaderFooter::lines`), so they sit where the stamp will be written, in Liberation Sans,
//! which has Helvetica's widths. A stamp set behind the content is previewed over it.

use super::{Stamp, Tab};
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
/// its place among the pages stamped, for the Bates number).
#[derive(Default)]
pub(super) struct Previews {
    crop: Vec<usize>,
    wm: Vec<usize>,
    hf: Vec<usize>,
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
        let n = self.page_count();
        let pages = |range: &str| parse_page_range(range, n).unwrap_or_default();
        if cropping {
            return Previews {
                crop: pages(&self.forms.crop.pages),
                ..Previews::default()
            };
        }
        let f = &self.forms;
        let shows = |s: Section, kind: Stamp, added: bool| {
            f.open(s, pass) && self.linked(kind).is_none() && !added
        };
        let wm = shows(Section::Watermark, Stamp::Watermark, f.wm_added) && f.wm.ready();
        let hf = shows(Section::HeaderFooter, Stamp::HeaderFooter, f.hf_added);
        Previews {
            crop: Vec::new(),
            wm: if wm { pages(&f.wm.range) } else { Vec::new() },
            hf: if hf { pages(&f.hf.range) } else { Vec::new() },
        }
    }

    /// Draw page `page`'s previews, its screen rectangle `r` and `k` screen points per PDF
    /// point.
    pub(super) fn draw_previews(
        &mut self,
        ui: &egui::Ui,
        p: &Previews,
        page: usize,
        r: Rect,
        k: f32,
    ) {
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
        // The page and the room around it to take its edges.
        let resp = ui.interact(r.expand(GRAB), id, Sense::drag());
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

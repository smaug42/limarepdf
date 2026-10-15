//! Text markup made from the read tool's selection: its kinds, the colours offered, and
//! the small menu with the less used kinds and the colour choice.

use crate::actions::{self, Act, Ctx};
use eframe::egui::{self, Color32, Sense, Stroke, StrokeKind};
use mpe_core::AnnotKind;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Mark {
    Highlight,
    Underline,
    StrikeOut,
    Squiggly,
}

impl Mark {
    pub fn kind(self) -> AnnotKind {
        match self {
            Mark::Highlight => AnnotKind::Highlight,
            Mark::Underline => AnnotKind::Underline,
            Mark::StrikeOut => AnnotKind::StrikeOut,
            Mark::Squiggly => AnnotKind::Squiggly,
        }
    }
}

/// The highlight colours offered, light so text stays readable.
pub const HIGHLIGHTS: [(&str, [f32; 3]); 5] = [
    ("Yellow", [1.0, 0.92, 0.23]),
    ("Green", [0.56, 0.9, 0.42]),
    ("Blue", [0.5, 0.78, 1.0]),
    ("Pink", [1.0, 0.62, 0.82]),
    ("Orange", [1.0, 0.72, 0.32]),
];

/// The colours offered for underlines, strike-outs and squiggly lines.
pub const LINES: [(&str, [f32; 3]); 4] = [
    ("Red", [0.86, 0.1, 0.1]),
    ("Blue", [0.1, 0.36, 0.86]),
    ("Green", [0.1, 0.6, 0.22]),
    ("Black", [0.0, 0.0, 0.0]),
];

/// The colour a mark is made in, from the colours last chosen.
pub fn color(m: Mark, c: &Ctx) -> [f32; 3] {
    match m {
        Mark::Highlight => HIGHLIGHTS[c.highlight.min(HIGHLIGHTS.len() - 1)].1,
        _ => LINES[c.line.min(LINES.len() - 1)].1,
    }
}

/// The small menu next to Highlight: the other kinds, and the colours for highlights and
/// for lines. Choosing a colour keeps the menu open.
pub fn menu(ui: &mut egui::Ui, c: &Ctx, acts: &mut Vec<Act>) {
    for m in [Mark::Underline, Mark::StrikeOut, Mark::Squiggly] {
        actions::menu_item(ui, actions::get(Act::Mark(m)), c, acts);
    }
    ui.separator();
    swatches(
        ui,
        "Highlight colour",
        &HIGHLIGHTS,
        c.highlight,
        false,
        acts,
    );
    swatches(ui, "Line colour", &LINES, c.line, true, acts);
}

fn swatches(
    ui: &mut egui::Ui,
    label: &str,
    colours: &[(&str, [f32; 3])],
    chosen: usize,
    line: bool,
    acts: &mut Vec<Act>,
) {
    ui.label(label);
    ui.horizontal(|ui| {
        for (index, (name, rgb)) in colours.iter().enumerate() {
            let (r, resp) = ui.allocate_exact_size(egui::vec2(20.0, 20.0), Sense::click());
            let [cr, cg, cb] = rgb.map(|v| (v * 255.0).round() as u8);
            let p = ui.painter();
            p.rect_filled(r.shrink(2.0), 3.0, Color32::from_rgb(cr, cg, cb));
            let stroke = if index == chosen {
                ui.visuals().selection.stroke
            } else {
                Stroke::new(1.0, ui.visuals().weak_text_color())
            };
            let width = if index == chosen { 2.0 } else { 1.0 };
            p.rect_stroke(
                r.shrink(2.0),
                3.0,
                Stroke::new(width, stroke.color),
                StrokeKind::Outside,
            );
            if resp.on_hover_text(*name).clicked() {
                acts.push(Act::MarkColor { line, index });
            }
        }
    });
}

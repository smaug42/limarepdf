//! Whether a side panel is open: what the user chose, kept apart from a narrow window
//! folding the panel away for now; and its edge, moved from the keyboard.

use eframe::egui::{self, accesskit};

/// The narrowest a side panel can be made, by its edge (egui's default).
pub const NARROWEST: f32 = 96.0;
/// How far the arrow keys move a panel's edge, and with Shift, in points.
const EDGE_STEP: f32 = 10.0;
const EDGE_BIG_STEP: f32 = 50.0;

/// The id egui gives the panel `panel` (by its id) shown in `ui`: `ui`'s scope with its id.
fn panel_id(ui: &egui::Ui, panel: &str) -> egui::Id {
    ui.scope_id().with(panel)
}

/// The id of the edge egui gives the panel `panel` shown in `ui`, to resize it: egui keeps
/// it under the panel's id with "__resize"; the tests of the edges fail if that changes.
fn edge_id(ui: &egui::Ui, panel: &str) -> egui::Id {
    panel_id(ui, panel).with("__resize")
}

/// Moves the edge of the panel `panel` from the keyboard while it has the keyboard (ENH-11):
/// Left and Right move it 10 points, 50 with Shift; Home makes the panel its narrowest and
/// End half the window wide. `left`: the panel is at the window's left, so Right widens
/// it. It never folds the panel away, as dragging the edge can. Call it before the panel
/// is shown, with the `ui` it is shown in: the width goes through egui's own resizing, as
/// a screen reader's change of the edge does.
pub fn edge_keys(ui: &egui::Ui, panel: &str, left: bool) {
    let ctx = ui.ctx();
    let id = edge_id(ui, panel);
    if !ctx.memory(|m| m.has_focus(id)) {
        return;
    }
    // The arrow keys move the edge, not the keyboard to the control beside it.
    let arrows = egui::EventFilter {
        horizontal_arrows: true,
        ..Default::default()
    };
    ctx.memory_mut(|m| m.set_focus_lock_filter(id, arrows));
    let Some(now) = egui::PanelState::load(ctx, panel_id(ui, panel)) else {
        return;
    };
    let room = ui.available_rect_before_wrap().width();
    let widest = (ctx.content_rect().width() / 2.0).max(NARROWEST);
    let mut width = now.size().x;
    let moved = ctx.input(|i| {
        let mut moved = false;
        for e in &i.events {
            let egui::Event::Key {
                key,
                pressed: true,
                modifiers,
                ..
            } = e
            else {
                continue;
            };
            let step = if modifiers.shift {
                EDGE_BIG_STEP
            } else {
                EDGE_STEP
            };
            let wider = if left { step } else { -step };
            width = match key {
                egui::Key::ArrowRight => width + wider,
                egui::Key::ArrowLeft => width - wider,
                egui::Key::Home => NARROWEST,
                egui::Key::End => widest,
                _ => continue,
            }
            .clamp(NARROWEST, room.max(NARROWEST));
            moved = true;
        }
        moved
    });
    if moved && room > 0.0 {
        let set = accesskit::ActionRequest {
            action: accesskit::Action::SetValue,
            target_tree: accesskit::TreeId::ROOT,
            target_node: id.accesskit_id(),
            data: Some(accesskit::ActionData::NumericValue(f64::from(width / room))),
        };
        ctx.input_mut(|i| i.events.push(egui::Event::AccessKitActionRequest(set)));
    }
}

/// After the panel `panel` is shown in `ui`: names its edge `name` for screen readers, and
/// draws a line along it in the accent colour while it has the keyboard. egui names it no
/// more than "Resize panel". Only once egui has added it this frame, so no stray node is
/// made.
pub fn edge_shown(ui: &egui::Ui, panel: &str, name: &str) {
    let ctx = ui.ctx();
    let id = edge_id(ui, panel);
    let Some(rect) = ctx.viewport(|v| v.this_pass.widgets.get(id).map(|w| w.rect)) else {
        return;
    };
    ctx.accesskit_node_builder(id, |n| {
        n.set_role(accesskit::Role::Splitter);
        n.set_label(name);
    });
    if ctx.memory(|m| m.has_focus(id)) {
        let stroke = egui::Stroke::new(2.0, ui.visuals().selection.stroke.color);
        // On the layer egui draws the edge's own highlight on, above the panels.
        let layer = egui::LayerId::new(egui::Order::Middle, id);
        ctx.layer_painter(layer)
            .vline(rect.center().x, rect.y_range(), stroke);
    }
}

/// A side panel's state. The user's choice is remembered between runs; a window too
/// narrow for the panel folds it away without changing that choice, and it comes back
/// when there is room again. Opening it while the window is narrow keeps it open.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Panel {
    /// The user wants it open.
    want: bool,
    /// The window is too narrow for it.
    narrow: bool,
    /// Opened by the user while the window was narrow: shown despite that.
    kept: bool,
}

impl Panel {
    pub fn new(want: bool) -> Self {
        Self {
            want,
            narrow: false,
            kept: false,
        }
    }

    /// Whether it is shown now.
    pub fn shown(self) -> bool {
        self.want && (!self.narrow || self.kept)
    }

    /// What the user chose, to remember.
    pub fn wanted(self) -> bool {
        self.want
    }

    /// The window's width changed whether there is room. Becoming narrow folds the panel
    /// away, even one opened while narrow before.
    pub fn set_narrow(&mut self, narrow: bool) {
        if narrow && !self.narrow {
            self.kept = false;
        }
        self.narrow = narrow;
    }

    /// The user opened or closed it.
    pub fn set_open(&mut self, open: bool) {
        self.want = open;
        self.kept = open && self.narrow;
    }

    /// The user opened it if hidden, or closed it if shown.
    pub fn toggle(&mut self) {
        self.set_open(!self.shown());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn closing_by_hand_survives_resizing() {
        let mut p = Panel::new(true);
        p.set_open(false);
        p.set_narrow(true);
        p.set_narrow(false);
        assert!(!p.shown());
        assert!(!p.wanted());
    }

    #[test]
    fn a_narrow_window_folds_it_away_until_there_is_room() {
        let mut p = Panel::new(true);
        p.set_narrow(true);
        assert!(!p.shown());
        // Folding away isn't the user's choice: it is still remembered as open.
        assert!(p.wanted());
        p.set_narrow(false);
        assert!(p.shown());
    }

    #[test]
    fn opening_it_in_a_narrow_window_keeps_it_open() {
        let mut p = Panel::new(true);
        p.set_narrow(true);
        p.toggle();
        assert!(p.shown());
        // The next frame's width check doesn't undo it.
        p.set_narrow(true);
        assert!(p.shown());
        p.set_narrow(false);
        assert!(p.shown());
        // Narrowing again later folds it away as usual.
        p.set_narrow(true);
        assert!(!p.shown());
    }

    #[test]
    fn closed_at_start_stays_closed_when_wide() {
        let mut p = Panel::new(false);
        p.set_narrow(false);
        assert!(!p.shown());
        p.toggle();
        assert!(p.shown() && p.wanted());
    }
}

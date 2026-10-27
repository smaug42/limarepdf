//! The window's regions, which F6 and Shift+F6 move the keyboard between (ENH-10): the
//! tabs, the commands, the document's notices, navigation, the page, the inspector and the
//! status bar, in the order they are on screen. Each is found by where it was drawn this
//! frame, and a control belongs to the region its middle is in.

use eframe::egui::{self, accesskit::Role, Id, Rect};

/// A part of the window the keyboard can be moved to.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Region {
    Tabs,
    /// The document's commands, the band's names and its groups of commands (ENH-38).
    Commands,
    /// The document's notices above the pages, while there are any.
    Notices,
    /// The side panel.
    Navigation,
    /// The pages, or the start screen with no document open.
    Page,
    /// The inspector, its strip when collapsed, or its pop-out.
    Inspector,
    Status,
}

impl Region {
    /// F6's order; it goes round from the last to the first.
    pub const ORDER: [Region; 7] = [
        Region::Tabs,
        Region::Commands,
        Region::Notices,
        Region::Navigation,
        Region::Page,
        Region::Inspector,
        Region::Status,
    ];

    /// The order a control's region is looked for in: those drawn over another first (the
    /// notices and the inspector's pop-out over the pages).
    const LOOKUP: [Region; 7] = [
        Region::Notices,
        Region::Inspector,
        Region::Navigation,
        Region::Tabs,
        Region::Commands,
        Region::Status,
        Region::Page,
    ];

    fn index(self) -> usize {
        Self::ORDER.iter().position(|&r| r == self).unwrap_or(0)
    }

    /// Its role and name for screen readers.
    pub fn named(self) -> (Role, &'static str) {
        match self {
            Region::Tabs => (Role::TabList, "Tabs"),
            Region::Commands => (Role::Toolbar, "Commands"),
            Region::Notices => (Role::Region, "Notices"),
            Region::Navigation => (Role::Navigation, "Navigation"),
            Region::Page => (Role::Main, "Document"),
            Region::Inspector => (Role::Complementary, "Inspector"),
            Region::Status => (Role::Status, "Status bar"),
        }
    }
}

/// Names the panel `ui` is in (or `ui` itself, outside a panel) for screen readers, as
/// `region`.
pub fn name(ui: &egui::Ui, region: Region) {
    let (role, label) = region.named();
    name_as(ui, role, label);
}

/// Names the panel `ui` is in (or `ui` itself, outside a panel) for screen readers: its
/// `role` and `label`. egui gives each panel a node of its own, with no name.
pub fn name_as(ui: &egui::Ui, role: Role, label: &str) {
    let stack = ui.stack();
    let id = stack
        .iter()
        .find(|s| s.is_panel_ui())
        .map_or(stack.unique_id, |s| s.unique_id);
    ui.ctx().accesskit_node_builder(id, |n| {
        n.set_role(role);
        n.set_label(label);
    });
}

/// A control that can take the keyboard this frame.
struct Control {
    id: Id,
    rect: Rect,
    /// Tab's order: its layer's (the panels' first), then its place in the layer.
    order: (u8, usize),
}

/// The controls that can take the keyboard this frame: enabled, shown and not behind a
/// dialog.
fn controls(ctx: &egui::Context) -> Vec<Control> {
    let all: Vec<(egui::LayerId, Control)> = ctx.viewport(|v| {
        v.this_pass
            .widgets
            .layers()
            .flat_map(|(layer, widgets)| {
                widgets.iter().enumerate().filter_map(move |(i, w)| {
                    let usable = w.enabled
                        && w.visible
                        && w.sense.is_focusable()
                        && w.interact_rect.is_positive();
                    usable.then_some((
                        *layer,
                        Control {
                            id: w.id,
                            rect: w.rect,
                            order: (layer.order as u8, i),
                        },
                    ))
                })
            })
            .collect()
    });
    all.into_iter()
        .filter(|(layer, _)| ctx.memory(|m| m.allows_interaction(*layer)))
        .map(|(_, c)| c)
        .collect()
}

/// Where each region is this frame, the control last used in each, and F6 pressed.
#[derive(Default)]
pub struct Regions {
    /// Each region's places on screen, the one to look in first first.
    places: Vec<(Region, Rect)>,
    /// The control that last had the keyboard in each region, in [`Region::ORDER`]'s order.
    last: [Option<Id>; Region::ORDER.len()],
    /// F6 (1) or Shift+F6 (-1) was pressed this frame; taken at the end of it.
    pub step: i32,
    /// The page being read shows that the keyboard is there: F6 took it to the page, and
    /// nothing has taken it since.
    pub ring: bool,
}

impl Regions {
    /// A new frame: the regions are found again as the window is drawn.
    pub fn begin(&mut self) {
        self.places.clear();
    }

    /// `region` was drawn at `rect` (one of its places, if it has several).
    pub fn add(&mut self, region: Region, rect: Rect) {
        if rect.is_positive() {
            self.places.push((region, rect));
        }
    }

    /// `region` was drawn at `rect`, over its other places: looked in first.
    pub fn add_over(&mut self, region: Region, rect: Rect) {
        if rect.is_positive() {
            self.places.insert(0, (region, rect));
        }
    }

    /// The region a control drawn at `rect` is in.
    fn of(&self, rect: Rect) -> Option<Region> {
        let c = rect.center();
        Region::LOOKUP.into_iter().find(|&r| {
            self.places
                .iter()
                .any(|(q, place)| *q == r && place.contains(c))
        })
    }

    /// The region of the control `id`, as it was drawn this frame or the last.
    pub fn of_control(&self, ctx: &egui::Context, id: Id) -> Option<Region> {
        let rect = ctx.viewport(|v| {
            let w = v.this_pass.widgets.get(id);
            w.or_else(|| v.prev_pass.widgets.get(id)).map(|w| w.rect)
        })?;
        self.of(rect)
    }

    /// Notes the control with the keyboard, `focused`, as the last used in its region.
    pub fn note(&mut self, ctx: &egui::Context, focused: Option<Id>) {
        if let Some((id, r)) = focused.and_then(|id| Some((id, self.of_control(ctx, id)?))) {
            self.last[r.index()] = Some(id);
        }
    }

    /// Where F6 (`step` 1) or Shift+F6 (-1) takes the keyboard from the region `from`: the
    /// next region round with a control to take it, and that control: the last used there
    /// if it is still shown, else the first in Tab's order. `None` for the page while
    /// `page_open` (a document is open), which then has the keys with no control taking
    /// them.
    pub fn next(
        &self,
        ctx: &egui::Context,
        from: Region,
        step: i32,
        page_open: bool,
    ) -> Option<(Region, Option<Id>)> {
        let controls = controls(ctx);
        let stop = |r: Region| -> Option<Option<Id>> {
            if r == Region::Page && page_open {
                return Some(None);
            }
            let inside = |c: &&Control| self.of(c.rect) == Some(r);
            let last = self.last[r.index()];
            if let Some(c) = controls.iter().filter(inside).find(|c| Some(c.id) == last) {
                return Some(Some(c.id));
            }
            self.places
                .iter()
                .filter(|(q, _)| *q == r)
                .find_map(|(_, place)| {
                    controls
                        .iter()
                        .filter(inside)
                        .filter(|c| place.contains(c.rect.center()))
                        .min_by_key(|c| c.order)
                })
                .map(|c| Some(c.id))
        };
        round(from, step).find_map(|r| Some((r, stop(r)?)))
    }
}

/// The regions after `from` in F6's order (before it, for a negative `step`), round to
/// the one before `from`.
fn round(from: Region, step: i32) -> impl Iterator<Item = Region> {
    let n = Region::ORDER.len() as i32;
    let at = from.index() as i32;
    let dir = step.signum();
    (1..n).map(move |i| Region::ORDER[(at + dir * i).rem_euclid(n) as usize])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn f6_goes_round_in_the_order_on_screen() {
        let after: Vec<_> = round(Region::Page, 1).collect();
        assert_eq!(
            after,
            [
                Region::Inspector,
                Region::Status,
                Region::Tabs,
                Region::Commands,
                Region::Notices,
                Region::Navigation,
            ]
        );
        let before: Vec<_> = round(Region::Tabs, -1).collect();
        assert_eq!(before[..2], [Region::Status, Region::Inspector]);
        assert_eq!(before.last(), Some(&Region::Commands));
        let after: Vec<_> = round(Region::Tabs, 1).collect();
        assert_eq!(after[..2], [Region::Commands, Region::Notices]);
    }

    #[test]
    fn a_control_is_in_the_region_drawn_over_the_others() {
        let mut r = Regions::default();
        let page = Rect::from_min_max(egui::pos2(200.0, 60.0), egui::pos2(1100.0, 760.0));
        let notices = Rect::from_min_max(egui::pos2(200.0, 60.0), egui::pos2(1100.0, 120.0));
        let side = Rect::from_min_max(egui::pos2(36.0, 60.0), egui::pos2(200.0, 760.0));
        r.add(Region::Navigation, side);
        r.add(Region::Page, page);
        r.add(Region::Notices, notices);
        let at = |x: f32, y: f32| Rect::from_center_size(egui::pos2(x, y), egui::vec2(20.0, 20.0));
        assert_eq!(r.of(at(600.0, 90.0)), Some(Region::Notices));
        assert_eq!(r.of(at(600.0, 400.0)), Some(Region::Page));
        // The side panel's edge, on the border between it and the pages.
        assert_eq!(r.of(at(200.0, 400.0)), Some(Region::Navigation));
        assert_eq!(r.of(at(10.0, 10.0)), None);
    }
}

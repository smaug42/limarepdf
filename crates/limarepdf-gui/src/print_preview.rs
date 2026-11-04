//! The Print dialog's preview (ENH-14): each sheet as it will print, drawn off the window's
//! thread from a copy of what is printed by core's own plan of the sheets
//! ([`print::sheets`]) and sheet writer ([`Pdf::print_planned`]), so the preview is what
//! reaches the printer. The sheet shown and its neighbours are drawn, the one shown first,
//! and kept until what is printed, the page setup or the size they are shown at changes. A
//! sheet held sideways to be read (a page turned onto it, or pages side by side, NEW-05) is
//! shown so.

use crate::dialogs::{set_name, Layout};
use crate::icon;
use crate::theme;
use eframe::egui::{self, Align, Color32, Pos2, Rect, RichText, TextureHandle, Vec2};
use limarepdf_core::print::{self, PageSetup, SheetPlan};
use limarepdf_core::render::{self, Bitmap, Renderer};
use limarepdf_core::{format_page_range, Pdf};
use std::collections::hash_map::Entry;
use std::collections::{HashMap, HashSet};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};

/// Sheets are drawn in steps of a twentieth of a pixel a point: one drawn for a size within
/// a step of the size shown serves it.
const STEPS: f32 = 20.0;

/// How many sheets either side of the one shown are kept drawn.
const KEEP: usize = 2;

/// The space around the sheet, in points.
const PAD: f32 = 20.0;

/// All of a texture.
const WHOLE: Rect = Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0));

/// The selected text made into a document of its own in a layout.
pub type Make = Box<dyn FnMut(Layout) -> limarepdf_core::Result<Pdf> + Send>;

/// What is printed: a copy of the document, or the selected text, made into a document of
/// its own in each layout when first wanted.
pub enum Source {
    Pages(Box<Pdf>),
    Selection(Make),
}

/// The page sizes, as shown, of each document the sheets are made from
/// ([`What::layout`]), or why it couldn't be made.
type Sizes = HashMap<Option<Layout>, Result<Vec<(f32, f32)>, String>>;

/// The pages printed: some of the document's (from 0), or all of those of the selected
/// text's document in a layout.
#[derive(Clone, Debug, PartialEq)]
pub enum What {
    Pages(Vec<usize>),
    Selection(Layout),
}

impl What {
    /// The layout of the document the sheets are made from; `None` for the document's own.
    fn layout(&self) -> Option<Layout> {
        match self {
            What::Pages(_) => None,
            What::Selection(l) => Some(*l),
        }
    }
}

/// What the preview shows: the pages printed, laid out as `setup` says.
#[derive(Clone, Debug, PartialEq)]
pub struct Ask {
    pub what: What,
    pub setup: PageSetup,
}

/// A sheet to draw: the `sheet`th, which prints pages `pages` of the document made in
/// `layout` where `plan` puts them (its pages by their place in `pages`), at `step` /
/// [`STEPS`] pixels a point.
struct Job {
    generation: u64,
    sheet: usize,
    layout: Option<Layout>,
    pages: Vec<usize>,
    plan: SheetPlan,
    step: u32,
}

/// The sheets waiting to be drawn, the newest last: drawn first.
#[derive(Default)]
struct Queue {
    jobs: Vec<Job>,
    /// What is shown now: jobs for anything earlier are dropped.
    generation: u64,
    /// The selected text's document to make before any sheet is asked for, so its sheets
    /// are counted.
    make: Option<Layout>,
    closed: bool,
}

/// What the thread does next.
enum Work {
    Make(Layout),
    Draw(Job),
}

type Shared = Arc<(Mutex<Queue>, Condvar)>;

/// Lock the queue; one left poisoned is still usable, and dropping the preview must never
/// panic.
fn lock(m: &Mutex<Queue>) -> MutexGuard<'_, Queue> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

/// What the thread sends back.
enum Came {
    /// The page sizes, as shown, of the document made in a layout (`None`: the document's
    /// own), or why it couldn't be made.
    Sizes(Option<Layout>, Result<Vec<(f32, f32)>, String>),
    /// For generation `.0`, sheet `.1` drawn at step `.2`, or why it couldn't be.
    Sheet(u64, usize, u32, Result<Bitmap, String>),
}

/// The thread drawing the sheets of `source`, the newest asked for first, until the preview
/// is dropped.
fn run(source: Source, shared: Shared, tx: Sender<Came>, ctx: egui::Context) {
    let mut made: HashMap<Option<Layout>, Result<Pdf, String>> = HashMap::new();
    let mut make = match source {
        Source::Pages(pdf) => {
            if tx.send(Came::Sizes(None, Ok(pdf.page_sizes()))).is_err() {
                return;
            }
            ctx.request_repaint();
            made.insert(None, Ok(*pdf));
            None
        }
        Source::Selection(make) => Some(make),
    };
    loop {
        let work = {
            let (m, cv) = &*shared;
            let mut q = lock(m);
            loop {
                if q.closed {
                    return;
                }
                if let Some(l) = q.make.take() {
                    break Work::Make(l);
                }
                match q.jobs.pop() {
                    Some(j) if j.generation == q.generation => break Work::Draw(j),
                    Some(_) => {}
                    None => q = cv.wait(q).unwrap_or_else(PoisonError::into_inner),
                }
            }
        };
        let layout = match &work {
            Work::Make(l) => Some(*l),
            Work::Draw(j) => j.layout,
        };
        if let Entry::Vacant(e) = made.entry(layout) {
            let doc = match (layout, make.as_mut()) {
                (Some(l), Some(make)) => crate::guard(|| make(l)).map_err(|e| format!("{e:#}")),
                _ => Err("there is nothing to print".into()),
            };
            let sizes = doc.as_ref().map(Pdf::page_sizes).map_err(Clone::clone);
            if tx.send(Came::Sizes(layout, sizes)).is_err() {
                return;
            }
            ctx.request_repaint();
            e.insert(doc);
        }
        let Work::Draw(job) = work else {
            continue;
        };
        let picture = match &made[&job.layout] {
            Ok(doc) => crate::guard(|| draw(doc, &job.pages, &job.plan, job.step))
                .map_err(|e| format!("{e:#}")),
            Err(why) => Err(why.clone()),
        };
        let sheet = Came::Sheet(job.generation, job.sheet, job.step, picture);
        if tx.send(sheet).is_err() {
            return;
        }
        ctx.request_repaint();
    }
}

/// The sheet `plan` makes of pages `pages` of `doc`, drawn at `step` / [`STEPS`] pixels a
/// point, and held as it is read: turned a quarter turn clockwise when held sideways.
fn draw(doc: &Pdf, pages: &[usize], plan: &SheetPlan, step: u32) -> limarepdf_core::Result<Bitmap> {
    let mut sheet = doc.print_planned(pages, std::slice::from_ref(plan))?;
    let hp = render::load(sheet.to_bytes()?, None)?;
    let b = Renderer::new(&hp).render(0, step as f32 / STEPS)?;
    Ok(if plan.sideways { clockwise(&b) } else { b })
}

/// `b` turned a quarter turn clockwise.
fn clockwise(b: &Bitmap) -> Bitmap {
    let (w, h) = (b.width, b.height);
    let mut rgba = vec![0; w * h * 4];
    for y in 0..h {
        for x in 0..w {
            // (x, y) goes to (h - 1 - y, x) in a picture h wide.
            let from = (y * w + x) * 4;
            let to = (x * h + (h - 1 - y)) * 4;
            rgba[to..to + 4].copy_from_slice(&b.rgba[from..from + 4]);
        }
    }
    Bitmap {
        width: h,
        height: w,
        rgba,
    }
}

/// The preview: the thread drawing the sheets, and the sheets it drew.
#[derive(Default)]
pub struct Preview {
    thread: Option<(Shared, Receiver<Came>)>,
    sizes: Sizes,
    ask: Option<Ask>,
    /// The sheets of what is shown, once its pages' sizes are known.
    plan: Option<Vec<SheetPlan>>,
    generation: u64,
    /// The sheets drawn, by their number and step, or why they couldn't be.
    drawn: HashMap<(usize, u32), Result<TextureHandle, String>>,
    pending: HashSet<(usize, u32)>,
    /// What is said under the sheets ([`Preview::notes`]).
    notes: Vec<String>,
    /// The sheet shown, from 0.
    pub at: usize,
}

impl Drop for Preview {
    fn drop(&mut self) {
        if let Some((shared, _)) = &self.thread {
            let (m, cv) = &**shared;
            lock(m).closed = true;
            cv.notify_all();
        }
    }
}

impl Preview {
    /// A preview of `source`, its sheets drawn on a thread of its own, which stops once
    /// the preview is dropped.
    pub fn spawn(ctx: &egui::Context, source: Source) -> Self {
        let shared: Shared = Arc::new((Mutex::new(Queue::default()), Condvar::new()));
        let (tx, rx) = channel();
        let (s, c) = (shared.clone(), ctx.clone());
        std::thread::spawn(move || run(source, s, tx, c));
        Preview {
            thread: Some((shared, rx)),
            sizes: HashMap::new(),
            ask: None,
            plan: None,
            generation: 0,
            drawn: HashMap::new(),
            pending: HashSet::new(),
            notes: Vec::new(),
            at: 0,
        }
    }

    /// Show `ask` from now on; `None` until the paper is known. A change drops the sheets
    /// drawn and those waiting to be.
    pub fn set(&mut self, ask: Option<Ask>) {
        if self.ask == ask {
            return;
        }
        self.ask = ask;
        self.generation += 1;
        self.drawn.clear();
        self.pending.clear();
        let make = match self.ask.as_ref().map(|a| &a.what) {
            Some(What::Selection(l)) if !self.sizes.contains_key(&Some(*l)) => Some(*l),
            _ => None,
        };
        if let Some((shared, _)) = &self.thread {
            let (m, cv) = &**shared;
            let mut q = lock(m);
            q.jobs.clear();
            q.generation = self.generation;
            q.make = make;
            cv.notify_one();
        }
        self.replan();
    }

    pub fn ask(&self) -> Option<&Ask> {
        self.ask.as_ref()
    }

    /// How many sheets print, once known: for the document's pages, before their sizes are.
    pub fn sheets(&self) -> Option<usize> {
        if let Some(plan) = &self.plan {
            return Some(plan.len());
        }
        let ask = self.ask.as_ref()?;
        match (&ask.what, ask.setup.booklet) {
            (What::Pages(p), Some(b)) => Some(b.sides_printed(p.len())),
            (What::Pages(p), None) => Some(p.len().div_ceil(ask.setup.per_sheet.count())),
            (What::Selection(_), _) => None,
        }
    }

    /// The sheets of what is shown, once its pages' sizes are known.
    #[cfg(test)]
    pub fn plan(&self) -> Option<&[SheetPlan]> {
        self.plan.as_deref()
    }

    /// Plan the sheets of what is shown, once its pages' sizes are known, and say what
    /// may not print.
    fn replan(&mut self) {
        self.plan = self.ask.as_ref().and_then(|ask| {
            let sizes = self.sizes.get(&ask.what.layout())?.as_ref().ok()?;
            let printed = match &ask.what {
                What::Pages(p) => p.iter().map(|&i| sizes.get(i).copied()).collect(),
                What::Selection(_) => Some(sizes.clone()),
            };
            Some(print::sheets(&printed?, &ask.setup))
        });
        self.notes = self.notes_now();
    }

    /// The page of the document the sheets are made from printed `i`th.
    fn printed(&self, i: usize) -> Option<usize> {
        match &self.ask.as_ref()?.what {
            What::Pages(p) => p.get(i).copied(),
            What::Selection(_) => Some(i),
        }
    }

    /// What is said under the sheets: that the printer doesn't say how near the edges it
    /// prints, and which pages reach past the printable area.
    #[cfg(test)]
    pub fn notes(&self) -> &[String] {
        &self.notes
    }

    fn notes_now(&self) -> Vec<String> {
        let Some(ask) = &self.ask else {
            return Vec::new();
        };
        let mut out = Vec::new();
        if ask.setup.paper.margins.is_none() {
            out.push(
                "The printer doesn't say how near the edges it can print, so anything near \
                 them may be cut off."
                    .to_string(),
            );
        }
        let Some(plan) = &self.plan else {
            return out;
        };
        let placed = plan.iter().flat_map(|s| &s.pages);
        let cut = placed.filter(|(_, p)| p.cut).map(|&(i, _)| i);
        let (mut cut, kind): (Vec<usize>, _) = match &ask.what {
            What::Pages(_) => (cut.filter_map(|i| self.printed(i)).collect(), "page"),
            What::Selection(_) => (cut.collect(), "sheet"),
        };
        cut.sort_unstable();
        cut.dedup();
        if !cut.is_empty() {
            let (s, reach, its) = match cut.len() {
                1 => ("", "reaches", "its"),
                _ => ("s", "reach", "their"),
            };
            out.push(format!(
                "At this scale {kind}{s} {} {reach} past the printable area, so {its} edges \
                 won't print.",
                format_page_range(&cut)
            ));
        }
        out
    }

    /// Take in what the thread sent.
    fn collect(&mut self, ctx: &egui::Context) {
        let Some((_, rx)) = &self.thread else {
            return;
        };
        let came: Vec<Came> = rx.try_iter().collect();
        for c in came {
            match c {
                Came::Sizes(layout, sizes) => {
                    self.sizes.insert(layout, sizes);
                    self.replan();
                }
                Came::Sheet(generation, sheet, step, picture) if generation == self.generation => {
                    self.pending.remove(&(sheet, step));
                    let tex = picture.map(|b| {
                        let img =
                            egui::ColorImage::from_rgba_premultiplied([b.width, b.height], &b.rgba);
                        let name = format!("print-preview-{sheet}-{step}");
                        ctx.load_texture(name, img, egui::TextureOptions::LINEAR)
                    });
                    self.drawn.insert((sheet, step), tex);
                }
                Came::Sheet(..) => {}
            }
        }
    }

    /// Ask for sheet `sheet` drawn at `step`, unless it is drawn or being drawn.
    fn want(&mut self, sheet: usize, step: u32) {
        let key = (sheet, step);
        if self.drawn.contains_key(&key) || self.pending.contains(&key) {
            return;
        }
        let plan = self.plan.as_ref().and_then(|p| p.get(sheet));
        let (Some(ask), Some(plan), Some((shared, _))) = (&self.ask, plan, &self.thread) else {
            return;
        };
        // Its own pages alone, the plan's pages counted among them.
        let pages = plan.pages.iter().map(|&(i, _)| self.printed(i)).collect();
        let Some(pages) = pages else {
            return;
        };
        let mut plan = plan.clone();
        for (j, p) in plan.pages.iter_mut().enumerate() {
            p.0 = j;
        }
        let job = Job {
            generation: self.generation,
            sheet,
            layout: ask.what.layout(),
            pages,
            plan,
            step,
        };
        let (m, cv) = &**shared;
        lock(m).jobs.push(job);
        cv.notify_one();
        self.pending.insert(key);
    }

    /// Drop the sheets far from the one shown, drawn or waiting, and those drawn at another
    /// step once the step shown has come.
    fn trim(&mut self, step: u32) {
        let at = self.at;
        let near = |sheet: usize| sheet.abs_diff(at) <= KEEP;
        let come: HashSet<usize> = (self.drawn.keys())
            .filter(|k| k.1 == step)
            .map(|k| k.0)
            .collect();
        (self.drawn).retain(|&(sheet, s), _| near(sheet) && (s == step || !come.contains(&sheet)));
        self.pending.retain(|&(sheet, s)| near(sheet) && s == step);
        if let Some((shared, _)) = &self.thread {
            lock(&shared.0)
                .jobs
                .retain(|j| near(j.sheet) && j.step == step);
        }
    }

    /// Draw the preview `size` points big: the sheet shown on the pages' background, the
    /// sheet's number with the arrows to page through them under it, and the notes. With
    /// nothing to show yet, a spinner while `waiting`.
    pub fn show(&mut self, ui: &mut egui::Ui, size: Vec2, waiting: bool) {
        self.collect(ui.ctx());
        if let Some(n) = self.sheets() {
            self.at = self.at.min(n.saturating_sub(1));
        }
        let (rect, _) = ui.allocate_exact_size(size, egui::Sense::hover());
        let dark = ui.visuals().dark_mode;
        ui.painter()
            .rect_filled(rect, theme::RADIUS, theme::canvas(dark));
        let inner = rect.shrink(PAD);
        let layout = egui::Layout::bottom_up(Align::Center);
        let mut col = ui.new_child(egui::UiBuilder::new().max_rect(inner).layout(layout));
        // From the bottom: the notes, last first, then the sheet's number.
        for note in self.notes.iter().rev() {
            col.add(egui::Label::new(RichText::new(note).weak()).wrap());
        }
        if self.ask.is_some() {
            col.add_space(4.0);
            self.counter(&mut col);
            col.add_space(8.0);
        }
        let room = col.available_rect_before_wrap();
        self.sheet(&mut col, room, waiting);
    }

    /// The sheet's number and the arrows either side of it, centred; for a booklet, the
    /// side's number, which side of which sheet it is above them.
    fn counter(&mut self, ui: &mut egui::Ui) {
        let n = self.sheets();
        let booklet = self.ask.as_ref().and_then(|a| a.setup.booklet);
        let (before, after) = match booklet {
            Some(_) => ("Previous side", "Next side"),
            None => ("Previous sheet", "Next sheet"),
        };
        let id = ui.scope_id().with("sheet number");
        let last: Option<f32> = ui.data(|d| d.get_temp(id));
        let size = Vec2::new(last.unwrap_or(0.0), theme::DIALOG_CONTROL);
        let layout = egui::Layout::left_to_right(Align::Center);
        let row = ui.allocate_ui_with_layout(size, layout, |ui| {
            let arrow = |ui: &mut egui::Ui, on: bool, glyph: &str, name: &str| {
                let b = egui::Button::new(theme::icon(glyph)).min_size(Vec2::splat(28.0));
                let r = ui.add_enabled(on, b).on_hover_text(name);
                set_name(&r, name);
                r
            };
            let prev = arrow(ui, self.at > 0, icon::CARET_LEFT, before);
            let shown = match n {
                Some(n) => format!("{} of {n}", self.at + 1),
                None => format!("{}", self.at + 1),
            };
            ui.label(shown);
            let more = n.is_some_and(|n| self.at + 1 < n);
            let next = arrow(ui, more, icon::CARET_RIGHT, after);
            if prev.clicked() {
                self.at -= 1;
                // A greyed button can't keep the keyboard: it goes on to the other.
                if self.at == 0 {
                    next.request_focus();
                }
            }
            if next.clicked() {
                self.at += 1;
                if n.is_some_and(|n| self.at + 1 >= n) {
                    prev.request_focus();
                }
            }
        });
        let width = row.response.rect.width();
        if last != Some(width) {
            ui.data_mut(|d| d.insert_temp(id, width));
            ui.ctx().request_discard("sheet number measured");
        }
        // Added after the row, above it, as the column is laid out from the bottom.
        if let Some(b) = booklet {
            let (sheet, front) = b.side(self.at);
            let side = if front { "Front" } else { "Back" };
            ui.label(format!("{side} of sheet {}", sheet + 1));
        }
    }

    /// The sheet shown, as large as fits in `room`, held as it is read; its drawing once it
    /// comes, the sheet blank with a spinner until then.
    fn sheet(&mut self, ui: &mut egui::Ui, room: Rect, waiting: bool) {
        let spinner = |ui: &mut egui::Ui, at: Pos2| {
            let r = Rect::from_center_size(at, Vec2::splat(24.0));
            ui.put(r, egui::Spinner::new());
        };
        let found = self.plan.as_ref().and_then(|p| p.get(self.at));
        let Some(plan) = found else {
            if waiting || self.ask.is_some() {
                spinner(ui, room.center());
            }
            return;
        };
        let (pw, ph) = plan.size;
        let held = match plan.sideways {
            true => Vec2::new(ph, pw),
            false => Vec2::new(pw, ph),
        };
        let k = (room.width() / held.x).min(room.height() / held.y).max(0.0);
        let r = Rect::from_center_size(room.center(), held * k);
        let dark = ui.visuals().dark_mode;
        ui.painter().add(theme::page_shadow(dark).as_shape(r, 0));
        ui.painter().rect_filled(r, 0.0, Color32::WHITE);
        let step = ((k * ui.ctx().pixels_per_point() * STEPS).ceil() as u32).max(1);
        // The neighbours asked for first, so the sheet shown is drawn first.
        let n = self.sheets().unwrap_or(1);
        if self.at + 1 < n {
            self.want(self.at + 1, step);
        }
        if self.at > 0 {
            self.want(self.at - 1, step);
        }
        self.want(self.at, step);
        // The sheet at another step stands in until it comes at this one.
        let drawn = (self.drawn.get(&(self.at, step))).or_else(|| {
            let other = self.drawn.iter().find(|(k, d)| k.0 == self.at && d.is_ok());
            other.map(|(_, d)| d)
        });
        match drawn {
            Some(Ok(tex)) => {
                ui.painter().image(tex.id(), r, WHOLE, Color32::WHITE);
            }
            Some(Err(why)) => {
                let text = format!("This sheet couldn't be drawn: {why}");
                let ink = Color32::from_rgb(110, 110, 110);
                let label = egui::Label::new(RichText::new(text).color(ink)).wrap();
                ui.put(r.shrink(8.0), label);
            }
            None => spinner(ui, r.center()),
        }
        self.trim(step);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use limarepdf_core::print::{Booklet, BookletSides, Orientation, Paper, PerSheet, Scale};
    use std::time::{Duration, Instant};

    fn letter(margins: Option<[f32; 4]>) -> Paper {
        Paper {
            id: "na_letter_8.5x11in".into(),
            label: "Letter (8.5 × 11 in)".into(),
            size: (612.0, 792.0),
            margins,
        }
    }

    fn setup(orientation: Orientation, scale: Scale) -> PageSetup {
        PageSetup {
            paper: letter(Some([12.0; 4])),
            orientation,
            scale,
            per_sheet: PerSheet::default(),
            booklet: None,
        }
    }

    /// Three pages: Letter, a wide one, and A3, each with a black square in its top-left
    /// corner.
    fn doc() -> Box<Pdf> {
        let mut pdf = Pdf::new();
        for (i, (w, h)) in [(612.0, 792.0), (792.0, 400.0), (842.0, 1191.0)]
            .into_iter()
            .enumerate()
        {
            pdf.insert_blank(i, w, h).unwrap();
            let square = format!("0 g 0 {} 100 100 re f", h - 100.0).into_bytes();
            let content = lopdf::Stream::new(lopdf::Dictionary::new(), square);
            let content = pdf.doc.add_object(content);
            let page = pdf.page_ids()[i];
            let d = pdf.doc.get_dictionary_mut(page).unwrap();
            d.set("Contents", content);
        }
        Box::new(pdf)
    }

    /// Draws the preview `size` big, frame after frame, until `done` says so.
    fn shown_until(
        ctx: &egui::Context,
        p: &mut Preview,
        size: Vec2,
        done: impl Fn(&Preview) -> bool,
    ) {
        let start = Instant::now();
        loop {
            let raw = egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(800.0, 800.0))),
                ..Default::default()
            };
            let mut out = ctx.run_ui(raw, |ui| p.show(ui, size, false));
            out.textures_delta.clear();
            if done(p) {
                return;
            }
            assert!(start.elapsed() < Duration::from_secs(30), "never drawn");
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    /// The drawing of sheet `sheet`, at any step.
    fn drawing(p: &Preview, sheet: usize) -> Option<[usize; 2]> {
        let (_, d) = p.drawn.iter().find(|(k, _)| k.0 == sheet)?;
        d.as_ref().ok().map(|t| t.size())
    }

    /// The sheets are drawn from the copy by core's sheet writer, each the paper's shape as
    /// it is held: a wide page turned onto the sheet shows on a sheet held sideways, and
    /// the neighbours of the sheet shown are drawn too.
    #[test]
    fn sheets_are_drawn_as_held_with_their_neighbours() {
        let ctx = egui::Context::default();
        let mut p = Preview::spawn(&ctx, Source::Pages(doc()));
        let ask = Ask {
            what: What::Pages(vec![0, 1, 2]),
            setup: setup(Orientation::Automatic, Scale::ShrinkOversized),
        };
        p.set(Some(ask));
        let size = Vec2::new(330.0, 420.0);
        shown_until(&ctx, &mut p, size, |p| {
            drawing(p, 0).is_some() && drawing(p, 1).is_some()
        });
        assert_eq!(p.sheets(), Some(3));
        let shape = |[w, h]: [usize; 2]| w as f32 / h as f32;
        // Letter, upright: 612 by 792.
        let first = drawing(&p, 0).unwrap();
        assert!((shape(first) - 612.0 / 792.0).abs() < 0.01, "{first:?}");
        // The wide page is turned onto the sheet; the sheet is shown held sideways.
        let second = drawing(&p, 1).unwrap();
        assert!((shape(second) - 792.0 / 612.0).abs() < 0.01, "{second:?}");
        // Not yet the third: two away from the first.
        assert!(p.drawn.keys().all(|k| k.0 < 2));
        // Paged on, the third is drawn too.
        p.at = 1;
        shown_until(&ctx, &mut p, size, |p| drawing(p, 2).is_some());
        assert!(p.notes().is_empty(), "{:?}", p.notes());
    }

    /// A sheet held sideways reads the right way up: the corner square at the page's top
    /// left is at the top left of what is shown.
    #[test]
    fn a_turned_sheet_reads_the_right_way_up() {
        let pdf = doc();
        let s = setup(Orientation::Automatic, Scale::ActualSize);
        let plan = &print::sheets(&[pdf.page_size(1).unwrap()], &s)[0];
        let b = draw(&pdf, &[1], plan, 20).unwrap();
        assert_eq!((b.width, b.height), (792, 612));
        let dark = |x: usize, y: usize| b.rgba[(y * b.width + x) * 4] < 60;
        // The page, 792 by 400, is centred on the sheet held sideways, 792 by 612: as wide
        // as it, from its left edge.
        let top = ((612.0 - 400.0) / 2.0) as usize;
        assert!(dark(30, top + 30), "the square isn't at the top left");
        assert!(!dark(b.width - 30, b.height - top - 30));
    }

    /// NEW-05: with several pages a sheet the sheets are counted and held as core plans
    /// them: two upright pages side by side on a sheet held sideways, the last sheet with
    /// the page left over; four a sheet, all three on one.
    #[test]
    fn several_pages_a_sheet_are_shown_as_planned() {
        let ctx = egui::Context::default();
        let mut p = Preview::spawn(&ctx, Source::Pages(doc()));
        let ask = |pages| {
            let mut s = setup(Orientation::Automatic, Scale::ShrinkOversized);
            s.per_sheet.pages = pages;
            Ask {
                what: What::Pages(vec![0, 2, 1]),
                setup: s,
            }
        };
        p.set(Some(ask(2)));
        let size = Vec2::new(330.0, 420.0);
        shown_until(&ctx, &mut p, size, |p| {
            drawing(p, 0).is_some() && drawing(p, 1).is_some()
        });
        assert_eq!(p.sheets(), Some(2));
        let plan = p.plan().unwrap();
        // Letter and A3, upright: side by side; the wide page alone, as it comes.
        assert!(plan[0].sideways && !plan[1].sideways, "{plan:?}");
        assert_eq!(plan[0].pages.len(), 2);
        let [w, h] = drawing(&p, 0).unwrap();
        assert!(w > h, "held sideways: {w} x {h}");
        let [w, h] = drawing(&p, 1).unwrap();
        assert!(w < h, "held upright: {w} x {h}");
        p.set(Some(ask(4)));
        shown_until(&ctx, &mut p, size, |p| drawing(p, 0).is_some());
        assert_eq!(p.sheets(), Some(1));
        assert_eq!(p.plan().unwrap()[0].pages.len(), 3);
        assert!(p.notes().is_empty(), "{:?}", p.notes());
    }

    /// NEW-05: a booklet's sides are counted before the pages' sizes are known, each held
    /// sideways, and a blank side is drawn as a blank sheet: one page makes one sheet, its
    /// back blank; the fronts alone, one side.
    #[test]
    fn a_booklets_sides_are_shown_as_planned() {
        let ctx = egui::Context::default();
        let mut p = Preview::spawn(&ctx, Source::Pages(doc()));
        let ask = |sides| {
            let mut s = setup(Orientation::Automatic, Scale::ShrinkOversized);
            s.booklet = Some(Booklet {
                sides,
                ..Default::default()
            });
            Ask {
                what: What::Pages(vec![0]),
                setup: s,
            }
        };
        p.set(Some(ask(BookletSides::Both)));
        assert_eq!(p.sheets(), Some(2), "counted before the sizes come");
        let size = Vec2::new(420.0, 330.0);
        shown_until(&ctx, &mut p, size, |p| {
            drawing(p, 0).is_some() && drawing(p, 1).is_some()
        });
        let plan = p.plan().unwrap();
        assert!(plan.iter().all(|s| s.sideways), "{plan:?}");
        assert!(plan[1].pages.is_empty(), "{plan:?}");
        let [w, h] = drawing(&p, 1).expect("the blank back drawn");
        assert!(w > h, "held sideways: {w} x {h}");
        p.set(Some(ask(BookletSides::Fronts)));
        assert_eq!(p.sheets(), Some(1));
    }

    /// A change of page setup drops the sheets drawn for the old one: what is shown is
    /// always the setup as it is.
    #[test]
    fn a_change_of_setup_draws_the_sheets_again() {
        let ctx = egui::Context::default();
        let mut p = Preview::spawn(&ctx, Source::Pages(doc()));
        let ask = |o| Ask {
            what: What::Pages(vec![1]),
            setup: setup(o, Scale::Fit),
        };
        p.set(Some(ask(Orientation::Automatic)));
        let size = Vec2::new(330.0, 420.0);
        shown_until(&ctx, &mut p, size, |p| drawing(p, 0).is_some());
        let [w, h] = drawing(&p, 0).unwrap();
        assert!(w > h);
        p.set(Some(ask(Orientation::Portrait)));
        assert!(drawing(&p, 0).is_none(), "the old drawing is kept");
        shown_until(&ctx, &mut p, size, |p| drawing(p, 0).is_some());
        let [w, h] = drawing(&p, 0).unwrap();
        assert!(w < h, "held upright: {w} x {h}");
    }

    /// The notes: the printer's margins not known, and the pages reaching past the
    /// printable area named.
    #[test]
    fn the_notes_say_what_may_not_print() {
        let ctx = egui::Context::default();
        let mut p = Preview::spawn(&ctx, Source::Pages(doc()));
        let mut s = setup(Orientation::Portrait, Scale::ActualSize);
        s.paper.margins = None;
        p.set(Some(Ask {
            what: What::Pages(vec![0, 1, 2]),
            setup: s,
        }));
        let size = Vec2::new(330.0, 420.0);
        shown_until(&ctx, &mut p, size, |p| p.notes().len() == 2);
        assert!(p.notes()[0].contains("doesn't say how near the edges"));
        // Letter itself is cut by the quarter inch kept clear; so are the wide page and A3.
        assert_eq!(
            p.notes()[1],
            "At this scale pages 1-3 reach past the printable area, so their edges won't print."
        );
    }

    /// The selected text's document is made on the preview's thread, in the layout asked
    /// for, and its sheets counted.
    #[test]
    fn the_selection_is_made_in_its_layout() {
        let ctx = egui::Context::default();
        let made = Arc::new(Mutex::new(Vec::new()));
        let seen = made.clone();
        let make: Make = Box::new(move |layout| {
            seen.lock().unwrap().push(layout);
            Pdf::from_text("Walrus", (612.0, 792.0))
        });
        let mut p = Preview::spawn(&ctx, Source::Selection(make));
        p.set(Some(Ask {
            what: What::Selection(Layout::Plain),
            setup: setup(Orientation::Automatic, Scale::ShrinkOversized),
        }));
        shown_until(&ctx, &mut p, Vec2::new(330.0, 420.0), |p| {
            drawing(p, 0).is_some()
        });
        assert_eq!(p.sheets(), Some(1));
        assert_eq!(*made.lock().unwrap(), [Layout::Plain]);
    }

    #[test]
    fn a_quarter_turn_clockwise_moves_the_top_left_to_the_top_right() {
        // 2 x 1: red, blue.
        let b = Bitmap {
            width: 2,
            height: 1,
            rgba: vec![255, 0, 0, 255, 0, 0, 255, 255],
        };
        let t = clockwise(&b);
        assert_eq!((t.width, t.height), (1, 2));
        assert_eq!(t.rgba, [255, 0, 0, 255, 0, 0, 255, 255]);
        // 1 x 2, red over blue: blue, red.
        let t = clockwise(&t);
        assert_eq!((t.width, t.height), (2, 1));
        assert_eq!(t.rgba, [0, 0, 255, 255, 255, 0, 0, 255]);
    }
}

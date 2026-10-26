//! Fill and sign: the saved signatures and initials, the Create signature window and the
//! Sign tool's pane. What is placed is a picture of a signature, not a digital signature.
//!
//! Saved signatures are kept in `signatures.txt` in LimarePDF's settings folder, and pictures
//! as copies in its `signatures` folder, so they keep working when the original moves.
//! Like the other settings they stay on this computer, unencrypted, and failing to read
//! or write them is not an error.

use crate::actions::Act;
use crate::dialogs::{self, Btn, Shell};
use crate::page_text;
use crate::theme;
use eframe::egui::{self, Color32, Pos2, Rect, Sense, Stroke, TextureHandle, TextureOptions, Vec2};
use limarepdf_core::{
    typed_signature, DateStyle, InkColor, Mark, SignArt, SignRole, SignStyle, Stamp, StampPlace,
};
use std::collections::HashMap;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::path::{Path, PathBuf};

/// How many signatures, and how many sets of initials, are remembered.
pub const SAVED: usize = 5;
/// The drawing pad's size, and the pen's width on it, in points. Strokes are kept in these
/// points whatever size the pad is drawn at ([`pad_size`]).
const PAD: Vec2 = Vec2::new(480.0, 160.0);
const PEN: f32 = 2.5;
/// The most a preview in the pane takes up, in points.
const PREVIEW: Vec2 = Vec2::new(190.0, 48.0);
/// The most pixels across or down a chosen picture is shown with in the Create window,
/// and the most points it takes up there.
const VIEW: u32 = 1200;
const VIEW_SIZE: Vec2 = Vec2::new(480.0, 240.0);
/// How far an arrow key moves or sizes the part of a picture used, as a share of it.
const NUDGE: f32 = 0.02;

/// A saved signature or set of initials.
#[derive(Clone, PartialEq, Debug)]
pub enum Saved {
    /// Pen strokes in the pad's points (y down), and the pen's width.
    Drawn {
        strokes: Vec<Vec<(f32, f32)>>,
        width: f32,
    },
    Typed {
        text: String,
        style: SignStyle,
    },
    /// A picture: its file (a copy in the settings folder when remembered), whether the
    /// paper around the ink is made transparent, and the part of it used, if not all (see
    /// [`SignArt::Picture`]).
    Picture {
        path: PathBuf,
        clear_paper: bool,
        crop: Option<[f32; 4]>,
    },
}

impl Saved {
    /// The picture's bytes, for a picture; empty otherwise.
    pub fn bytes(&self) -> std::io::Result<Vec<u8>> {
        match self {
            Saved::Picture { path, .. } => std::fs::read(path),
            _ => Ok(Vec::new()),
        }
    }

    /// The stamp that puts it on a page; `bytes` from [`Saved::bytes`].
    pub fn stamp<'a>(&'a self, bytes: &'a [u8], role: SignRole, color: InkColor) -> Stamp<'a> {
        Stamp::Signature {
            art: self.art(bytes),
            role,
            color,
            date: false,
        }
    }

    /// What it looks like, for a stamp or a digital signature; `bytes` from
    /// [`Saved::bytes`].
    pub fn art<'a>(&'a self, bytes: &'a [u8]) -> SignArt<'a> {
        match self {
            Saved::Drawn { strokes, width } => SignArt::Drawn {
                strokes,
                width: *width,
            },
            Saved::Typed { text, style } => SignArt::Typed {
                text,
                style: *style,
            },
            Saved::Picture {
                clear_paper, crop, ..
            } => SignArt::Picture {
                bytes,
                clear_paper: *clear_paper,
                crop: *crop,
            },
        }
    }

    /// As a line of the file: `draw WIDTH x,y x,y;x,y …`, `type STYLE TEXT`,
    /// `picture clear|keep PATH`, or for part of a picture `cropped LEFT,TOP,RIGHT,BOTTOM
    /// clear|keep PATH` (which older versions skip).
    fn line(&self) -> String {
        match self {
            Saved::Drawn { strokes, width } => {
                let strokes: Vec<String> = strokes
                    .iter()
                    .map(|s| {
                        s.iter()
                            .map(|(x, y)| format!("{:.1},{:.1}", x, y))
                            .collect::<Vec<_>>()
                            .join(" ")
                    })
                    .collect();
                format!("draw {width} {}", strokes.join(";"))
            }
            Saved::Typed { text, style } => format!("type {} {text}", style.number()),
            Saved::Picture {
                path,
                clear_paper,
                crop,
            } => {
                let how = if *clear_paper { "clear" } else { "keep" };
                match crop {
                    Some(c) => {
                        let c = c.map(|v| format!("{v:.4}")).join(",");
                        format!("cropped {c} {how} {}", path.display())
                    }
                    None => format!("picture {how} {}", path.display()),
                }
            }
        }
    }

    fn parse(s: &str) -> Option<Self> {
        let (how, rest) = s.trim().split_once(' ')?;
        match how {
            "draw" => {
                let (width, rest) = rest.split_once(' ')?;
                let width: f32 = width.parse().ok().filter(|w: &f32| *w > 0.0)?;
                let strokes: Vec<Vec<(f32, f32)>> = rest
                    .split(';')
                    .map(|s| {
                        s.split_whitespace()
                            .filter_map(|p| {
                                let (x, y) = p.split_once(',')?;
                                Some((x.parse().ok()?, y.parse().ok()?))
                            })
                            .collect()
                    })
                    .filter(|s: &Vec<_>| !s.is_empty())
                    .collect();
                (!strokes.is_empty()).then_some(Saved::Drawn { strokes, width })
            }
            "type" => {
                let (n, text) = rest.split_once(' ')?;
                let style = SignStyle::from_number(n.parse().ok()?)?;
                let text = typed_signature(text, style).ok()?;
                Some(Saved::Typed { text, style })
            }
            "picture" => {
                let (how, path) = rest.split_once(' ')?;
                Some(Saved::Picture {
                    path: PathBuf::from(path.trim()),
                    clear_paper: how != "keep",
                    crop: None,
                })
            }
            "cropped" => {
                let (c, rest) = rest.split_once(' ')?;
                let c: Vec<f32> = c
                    .split(',')
                    .map(|v| v.parse().ok())
                    .collect::<Option<_>>()?;
                let crop = match c[..] {
                    [l, t, r, b]
                        if 0.0 <= l && l < r && r <= 1.0 && 0.0 <= t && t < b && b <= 1.0 =>
                    {
                        [l, t, r, b]
                    }
                    _ => return None,
                };
                let (how, path) = rest.split_once(' ')?;
                Some(Saved::Picture {
                    path: PathBuf::from(path.trim()),
                    clear_paper: how != "keep",
                    crop: Some(crop),
                })
            }
            _ => None,
        }
    }
}

/// A signature or set of initials in the list.
#[derive(Clone, PartialEq, Debug)]
pub struct Entry {
    pub saved: Saved,
    /// Written to the file; otherwise kept for this session only.
    pub keep: bool,
}

/// The saved signatures and initials, and the ink and date choices.
#[derive(Clone, PartialEq, Debug)]
pub struct SignKit {
    pub signatures: Vec<Entry>,
    pub initials: Vec<Entry>,
    pub color: InkColor,
    pub date: DateStyle,
    file: Option<PathBuf>,
}

impl SignKit {
    /// The signatures kept in `file` (read now, written by [`SignKit::save`]).
    pub fn from_file(file: Option<PathBuf>) -> Self {
        let mut k = SignKit {
            signatures: Vec::new(),
            initials: Vec::new(),
            color: InkColor::Black,
            date: DateStyle::Iso,
            file,
        };
        let text = k
            .file
            .as_ref()
            .and_then(|f| std::fs::read_to_string(f).ok())
            .unwrap_or_default();
        for line in text.lines() {
            let Some((key, value)) = line.split_once('=') else {
                continue;
            };
            let entry = || Saved::parse(value).map(|saved| Entry { saved, keep: true });
            match key.trim() {
                "ink" => k.color = InkColor::from_name(value).unwrap_or_default(),
                "date" => k.date = DateStyle::from_name(value).unwrap_or_default(),
                "signature" if k.signatures.len() < SAVED => k.signatures.extend(entry()),
                "initials" if k.initials.len() < SAVED => k.initials.extend(entry()),
                _ => {}
            }
        }
        k
    }

    /// Writes the kit to its file ([`crate::settings::write_own`]): why it couldn't be, or
    /// why the system couldn't confirm it is safely on disk.
    pub fn save(&self) -> anyhow::Result<Option<String>> {
        let Some(file) = &self.file else {
            return Ok(None);
        };
        let mut text = format!("ink = {}\ndate = {}\n", self.color.name(), self.date.name());
        for (key, list) in [
            ("signature", &self.signatures),
            ("initials", &self.initials),
        ] {
            for e in list.iter().filter(|e| e.keep) {
                text += &format!("{key} = {}\n", e.saved.line());
            }
        }
        crate::settings::write_own(file, text.as_bytes())
    }

    pub fn list(&self, role: SignRole) -> &[Entry] {
        match role {
            SignRole::Signature => &self.signatures,
            SignRole::Initials => &self.initials,
        }
    }

    fn list_mut(&mut self, role: SignRole) -> &mut Vec<Entry> {
        match role {
            SignRole::Signature => &mut self.signatures,
            SignRole::Initials => &mut self.initials,
        }
    }

    /// Whether a signature or initials was saved for next time since `before`.
    pub fn kept_since(&self, before: &SignKit) -> bool {
        let kept = |k: &SignKit| {
            let all = k.signatures.iter().chain(&k.initials);
            all.filter(|e| e.keep).cloned().collect::<Vec<_>>()
        };
        let was = kept(before);
        kept(self).iter().any(|e| !was.contains(e))
    }

    /// The folder remembered pictures are copied to.
    fn picture_dir(&self) -> Option<PathBuf> {
        Some(self.file.as_ref()?.parent()?.join("signatures"))
    }

    /// Put a signature or set of initials first in its list, copying a remembered picture
    /// into the settings folder. The oldest beyond [`SAVED`] is dropped. A picture that
    /// can't be copied is kept for this session only: returns what to say then.
    pub fn add(&mut self, role: SignRole, mut e: Entry) -> Option<String> {
        let mut unkept = None;
        if let (true, Saved::Picture { path, .. }, Some(dir)) =
            (e.keep, &mut e.saved, self.picture_dir())
        {
            let ext = path
                .extension()
                .map(|x| x.to_string_lossy().to_lowercase())
                .unwrap_or_else(|| "png".into());
            let stamp = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |d| d.as_millis());
            let name = match role {
                SignRole::Signature => "signature",
                SignRole::Initials => "initials",
            };
            let copy = dir.join(format!("{name}-{stamp}.{ext}"));
            match std::fs::create_dir_all(&dir).and_then(|_| std::fs::copy(&*path, &copy)) {
                Ok(_) => *path = copy,
                Err(err) => {
                    e.keep = false;
                    unkept = Some(format!(
                        "The {name} can be used until LimarePDF closes, but LimarePDF couldn't \
                         keep a copy of its picture for next time ({err})."
                    ));
                }
            }
        }
        let list = self.list_mut(role);
        list.retain(|x| x.saved != e.saved);
        list.insert(0, e);
        let dropped: Vec<Entry> = list.drain(SAVED.min(list.len())..).collect();
        for d in dropped {
            self.forget(&d);
        }
        unkept
    }

    /// Take a signature or set of initials out of its list, deleting its picture's copy.
    pub fn remove(&mut self, role: SignRole, i: usize) {
        let list = self.list_mut(role);
        if i < list.len() {
            let e = list.remove(i);
            self.forget(&e);
        }
    }

    /// Delete a picture copy LimarePDF made for `e`.
    fn forget(&self, e: &Entry) {
        if let (Saved::Picture { path, .. }, Some(dir)) = (&e.saved, self.picture_dir()) {
            if path.starts_with(&dir) {
                let _ = std::fs::remove_file(path);
            }
        }
    }
}

/// What the Sign tool puts down next.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SignPick {
    /// A saved signature or set of initials, by its place in the list.
    Saved(SignRole, usize),
    Date,
    Name,
    Text,
    Mark(Mark),
}

/// Pictures of signatures for lists, made by drawing them with LimarePDF's renderer, so they
/// look exactly as they will on the page.
#[derive(Default)]
pub struct Previews {
    /// Each picture, or why it can't be drawn.
    made: HashMap<u64, Result<TextureHandle, String>>,
    /// Chosen pictures as they are seen, or why they can't be shown, by file.
    views: HashMap<PathBuf, Result<TextureHandle, String>>,
}

impl Previews {
    /// The picture of `saved`, or `None` when it can't be drawn (a picture file gone).
    fn get(
        &mut self,
        ctx: &egui::Context,
        saved: &Saved,
        role: SignRole,
        color: InkColor,
    ) -> Option<TextureHandle> {
        self.made_or_why(ctx, saved, role, color).ok()
    }

    /// The picture of `saved`, or why it can't be drawn.
    fn made_or_why(
        &mut self,
        ctx: &egui::Context,
        saved: &Saved,
        role: SignRole,
        color: InkColor,
    ) -> Result<TextureHandle, String> {
        let mut h = DefaultHasher::new();
        format!("{saved:?}{role:?}{color:?}").hash(&mut h);
        let key = h.finish();
        if self.made.len() > 64 {
            self.made.clear();
        }
        self.made
            .entry(key)
            .or_insert_with(|| {
                let why = |e: &dyn std::fmt::Display| capital(&e.to_string());
                let bytes = saved.bytes().map_err(|e| why(&e))?;
                let bmp = saved
                    .stamp(&bytes, role, color)
                    .preview("", 2.0)
                    .map_err(|e| why(&e))?;
                let img =
                    egui::ColorImage::from_rgba_unmultiplied([bmp.width, bmp.height], &bmp.rgba);
                Ok(ctx.load_texture(format!("sign-{key}"), img, TextureOptions::LINEAR))
            })
            .clone()
    }

    /// The picture at `path` as it is seen, to choose the part that holds the signature, or
    /// why it can't be shown.
    fn view(&mut self, ctx: &egui::Context, path: &Path) -> Result<TextureHandle, String> {
        if self.views.len() > 8 {
            self.views.clear();
        }
        let n = self.views.len();
        self.views
            .entry(path.to_path_buf())
            .or_insert_with(|| {
                let bytes = std::fs::read(path).map_err(|e| capital(&e.to_string()))?;
                let (w, h, rgba) = limarepdf_core::picture_view(&bytes, VIEW)
                    .map_err(|e| capital(&e.to_string()))?;
                let img = egui::ColorImage::from_rgba_unmultiplied([w as usize, h as usize], &rgba);
                Ok(ctx.load_texture(format!("sign-view-{n}"), img, TextureOptions::LINEAR))
            })
            .clone()
    }

    /// A preview's size in points: half its pixels, shrunk to fit `max`.
    fn size(tex: &TextureHandle, max: Vec2) -> Vec2 {
        let s = tex.size_vec2() / 2.0;
        s * (max.x / s.x).min(max.y / s.y).min(1.0)
    }
}

/// How a new signature is made.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Way {
    Draw,
    Type,
    Picture,
}

/// The Create signature window.
pub struct Create {
    pub role: SignRole,
    way: Way,
    strokes: Vec<Vec<(f32, f32)>>,
    stroke: Option<Vec<(f32, f32)>>,
    /// The Type tab's text as the window opened.
    prefill: String,
    text: String,
    style: SignStyle,
    picture: Option<PathBuf>,
    clear_paper: bool,
    /// The part of the picture to use, if not all (see [`SignArt::Picture`]), and while a
    /// drag chooses it, where it started and where it is now, in the same shares of the
    /// picture.
    crop: Option<[f32; 4]>,
    drag: Option<(Pos2, Pos2)>,
    keep: bool,
    error: Option<String>,
}

pub enum Outcome {
    Open,
    Cancel,
    Done(Entry),
}

/// A name's initials: the first letter of each word, in capitals.
fn initials_of(name: &str) -> String {
    name.split_whitespace()
        .filter_map(|w| w.chars().next())
        .flat_map(char::to_uppercase)
        .collect()
}

impl Create {
    /// A new window for `role`, the Type tab filled in from `name`.
    pub fn new(role: SignRole, name: &str) -> Self {
        let text = match role {
            SignRole::Signature => name.trim().to_string(),
            SignRole::Initials => initials_of(name),
        };
        Create {
            role,
            way: Way::Draw,
            strokes: Vec::new(),
            stroke: None,
            prefill: text.clone(),
            text,
            style: SignStyle::Caveat,
            picture: None,
            clear_paper: true,
            crop: None,
            drag: None,
            keep: true,
            error: None,
        }
    }

    pub fn show(
        &mut self,
        ctx: &egui::Context,
        previews: &mut Previews,
        color: &mut InkColor,
    ) -> Outcome {
        let (title, done) = match self.role {
            SignRole::Signature => ("Create signature", "Use signature"),
            SignRole::Initials => ("Create initials", "Use initials"),
        };
        let mut out = Outcome::Open;
        let shell = Shell::new(title)
            .width(PAD.x + 2.0 * dialogs::SIDE)
            .esc(!self.staged());
        let (_, esc) = shell.show(ctx, |ui| {
            dialogs::content(ui, |ui| {
                ui.horizontal(|ui| {
                    let ways = [
                        (Way::Draw, "Draw"),
                        (Way::Type, "Type"),
                        (Way::Picture, "Image"),
                    ];
                    let gap = ui.spacing().item_spacing.x;
                    let w = (ui.available_width() - 2.0 * gap) / 3.0;
                    for (way, label) in ways {
                        let b = egui::Button::new(label).selected(self.way == way);
                        if ui.add_sized([w, theme::DIALOG_CONTROL], b).clicked() {
                            self.way = way;
                            self.error = None;
                        }
                    }
                });
                ui.add_space(8.0);
                match self.way {
                    Way::Draw => self.pad(ui, *color),
                    Way::Type => self.typed(ui, previews, *color),
                    Way::Picture => self.picture(ui, previews, *color),
                }
                if let Some(e) = &self.error {
                    ui.colored_label(ui.visuals().error_fg_color, e);
                }
                ui.add_space(dialogs::FIELD_GAP);
                ink_field(ui, color);
                ui.add_space(dialogs::FIELD_GAP);
                ui.checkbox(&mut self.keep, "Save for next time")
                    .on_hover_text(
                        "Keep it in LimarePDF's settings folder on this computer, so it is there \
                         the next time LimarePDF starts. Unticked, it stays in the list only \
                         until LimarePDF closes.",
                    );
            });
            match dialogs::footer(ui, None, &[Btn::primary(done), Btn::cancel()]) {
                Some(0) => match self.entry() {
                    Ok(e) => out = Outcome::Done(e),
                    Err(e) => self.error = Some(e),
                },
                Some(_) => out = Outcome::Cancel,
                None => {}
            }
        });
        if esc {
            return Outcome::Cancel;
        }
        out
    }

    /// Whether the window holds work that closing it would lose: something drawn, typed
    /// (beyond the name it was filled in with) or chosen.
    fn staged(&self) -> bool {
        !self.strokes.is_empty()
            || self.stroke.is_some()
            || self.picture.is_some()
            || self.text != self.prefill
    }

    /// The drawing pad: white, with a line to sign on.
    fn pad(&mut self, ui: &mut egui::Ui, color: InkColor) {
        let (rect, resp) = ui.allocate_exact_size(pad_size(ui), Sense::click_and_drag());
        crate::dialogs::set_name(&resp, "Drawing area: sign with the mouse or a pen");
        // Points on screen per point of the full-size pad.
        let k = rect.width() / PAD.x;
        let painter = ui.painter_at(rect);
        painter.rect_filled(rect, 4.0, Color32::WHITE);
        // Outlined as the dialog's fields are, to show on a white dialog.
        let edge = ui.visuals().widgets.inactive.bg_stroke;
        painter.rect_stroke(rect, 4.0, edge, egui::StrokeKind::Inside);
        let base = rect.top() + rect.height() * 0.72;
        painter.line_segment(
            [
                Pos2::new(rect.left() + 24.0 * k, base),
                Pos2::new(rect.right() - 24.0 * k, base),
            ],
            Stroke::new(1.0, Color32::from_gray(200)),
        );
        let local = |p: Pos2| ((p.x - rect.left()) / k, (p.y - rect.top()) / k);
        if resp.drag_started() {
            if let Some(p) = ui.input(|i| i.pointer.press_origin()) {
                self.stroke = Some(vec![local(p)]);
            }
        }
        if resp.dragged() {
            if let (Some(s), Some(p)) = (self.stroke.as_mut(), resp.interact_pointer_pos()) {
                let p = local(p);
                let far = s.last().is_none_or(|q| (q.0 - p.0).hypot(q.1 - p.1) >= 1.0);
                if far {
                    s.push(p);
                }
            }
        }
        if resp.drag_stopped() {
            self.strokes.extend(self.stroke.take());
        } else if resp.clicked() {
            if let Some(p) = resp.interact_pointer_pos() {
                self.strokes.push(vec![local(p)]);
            }
        }
        let [r, g, b] = color.rgb().map(|v| (v * 255.0) as u8);
        let ink = Color32::from_rgb(r, g, b);
        for s in self.strokes.iter().chain(&self.stroke) {
            let pts: Vec<Pos2> = s
                .iter()
                .map(|&(x, y)| Pos2::new(rect.left() + x * k, rect.top() + y * k))
                .collect();
            if pts.len() == 1 {
                painter.circle_filled(pts[0], PEN * k / 2.0, ink);
            } else {
                painter.add(egui::Shape::line(pts, Stroke::new(PEN * k, ink)));
            }
        }
        if resp.hovered() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::Crosshair);
        }
        ui.horizontal(|ui| {
            if ui
                .add_enabled(!self.strokes.is_empty(), egui::Button::new("Clear"))
                .clicked()
            {
                self.strokes.clear();
            }
            ui.label("Draw with the mouse, touchpad or a pen.");
        });
    }

    /// Typing a name, and picking one of the handwriting styles, shown as they will look.
    fn typed(&mut self, ui: &mut egui::Ui, previews: &mut Previews, color: InkColor) {
        let name = match self.role {
            SignRole::Signature => "Your name",
            SignRole::Initials => "Your initials",
        };
        dialogs::field(ui, name, |ui| {
            let edit = egui::TextEdit::singleline(&mut self.text).desired_width(f32::INFINITY);
            ui.add(theme::field(edit))
        });
        ui.add_space(4.0);
        let room = ui.available_width() - 2.0 * ui.spacing().button_padding.x;
        for style in SignStyle::ALL {
            let saved = Saved::Typed {
                text: self.text.clone(),
                style,
            };
            let ok = typed_signature(&self.text, style).is_ok();
            let tex = ok
                .then(|| previews.get(ui.ctx(), &saved, self.role, color))
                .flatten();
            let r = match tex {
                Some(tex) => {
                    let size = Previews::size(&tex, Vec2::new(room, 60.0));
                    let img = egui::Image::new((tex.id(), size));
                    let r = ui.add(egui::Button::image(img).selected(self.style == style));
                    dialogs::set_name(&r, style.label());
                    r
                }
                None => ui.selectable_label(self.style == style, style.label()),
            };
            if r.on_hover_text(style.label()).clicked() {
                self.style = style;
            }
        }
        if let Err(e) = typed_signature(&self.text, self.style) {
            if !self.text.trim().is_empty() {
                ui.colored_label(ui.visuals().error_fg_color, capital(&e.to_string()));
            }
        }
    }

    /// Choosing an image of a signature on paper.
    fn picture(&mut self, ui: &mut egui::Ui, previews: &mut Previews, color: InkColor) {
        ui.horizontal(|ui| {
            if ui.button("Choose an image…").clicked() {
                if let Some(p) = rfd::FileDialog::new()
                    .add_filter("Images", &["png", "jpg", "jpeg", "tif", "tiff"])
                    .pick_file()
                {
                    self.picture = Some(p);
                    self.crop = None;
                    self.drag = None;
                    self.error = None;
                }
            }
            if let Some(p) = &self.picture {
                ui.label(file_name(p));
            }
        });
        ui.checkbox(&mut self.clear_paper, "Make the paper transparent")
            .on_hover_text(
                "Keep only the ink: the paper around it becomes see-through and the empty \
                 margins are cut off. Untick for a picture that should stay as it is.",
            );
        let Some(p) = self.picture.clone() else {
            return;
        };
        match previews.view(ui.ctx(), &p) {
            Ok(tex) => self.cropper(ui, &tex),
            Err(why) => {
                ui.colored_label(ui.visuals().error_fg_color, why);
            }
        }
        ui.add_space(dialogs::FIELD_GAP);
        ui.label("Preview");
        ui.add_space(2.0);
        let saved = Saved::Picture {
            path: p,
            clear_paper: self.clear_paper,
            crop: self.crop,
        };
        match previews.made_or_why(ui.ctx(), &saved, self.role, color) {
            Ok(tex) => {
                let size = Previews::size(&tex, Vec2::new(ui.available_width(), 80.0));
                ui.image((tex.id(), size));
            }
            Err(why) => {
                ui.colored_label(ui.visuals().error_fg_color, why);
                if self.clear_paper {
                    ui.label(
                        "Drag around the signature on the picture, or untick Make the paper \
                         transparent.",
                    );
                }
            }
        }
    }

    /// The chosen picture, the part used marked; dragging over it chooses that part.
    fn cropper(&mut self, ui: &mut egui::Ui, tex: &TextureHandle) {
        let s = tex.size_vec2();
        let max = Vec2::new(ui.available_width().min(VIEW_SIZE.x), VIEW_SIZE.y);
        let size = s * (max.x / s.x).min(max.y / s.y);
        let (rect, resp) = ui.allocate_exact_size(size, Sense::click_and_drag());
        if resp.clicked() || resp.drag_started() {
            resp.request_focus();
        }
        dialogs::set_name(
            &resp,
            "Picture: drag around the signature, or use the arrow keys, to use only that part",
        );
        let painter = ui.painter_at(rect.expand(2.0));
        let whole = Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0));
        painter.image(tex.id(), rect, whole, Color32::WHITE);
        let edge = ui.visuals().widgets.inactive.bg_stroke;
        painter.rect_stroke(rect, 0.0, edge, egui::StrokeKind::Outside);
        let share = |p: Pos2| {
            ((p - rect.min) / rect.size())
                .to_pos2()
                .clamp(Pos2::ZERO, whole.max)
        };
        if resp.drag_started() {
            if let Some(p) = ui.input(|i| i.pointer.press_origin()) {
                self.drag = Some((share(p), share(p)));
            }
        }
        if let (Some(d), Some(p)) = (&mut self.drag, resp.interact_pointer_pos()) {
            d.1 = share(p);
        }
        let boxed = |(a, b): (Pos2, Pos2)| [a.x.min(b.x), a.y.min(b.y), a.x.max(b.x), a.y.max(b.y)];
        if resp.drag_stopped() {
            if let Some(c) = self.drag.take().map(boxed) {
                // A box too small to hold anything is taken as a slip of the mouse.
                if (c[2] - c[0]) * rect.width() >= 4.0 && (c[3] - c[1]) * rect.height() >= 4.0 {
                    self.crop = Some(c);
                    self.error = None;
                }
            }
        }
        // With the keyboard on the picture, the arrow keys move the part used (the middle
        // half to begin with), and Shift with them sizes it by its right and bottom edges.
        if resp.has_focus() {
            let arrows = egui::EventFilter {
                horizontal_arrows: true,
                vertical_arrows: true,
                ..Default::default()
            };
            ui.memory_mut(|m| m.set_focus_lock_filter(resp.id, arrows));
            let (mut dx, mut dy, mut size) = (0.0, 0.0, false);
            ui.input(|i| {
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
                    let (x, y) = match key {
                        egui::Key::ArrowLeft => (-1.0, 0.0),
                        egui::Key::ArrowRight => (1.0, 0.0),
                        egui::Key::ArrowUp => (0.0, -1.0),
                        egui::Key::ArrowDown => (0.0, 1.0),
                        _ => continue,
                    };
                    (dx, dy) = (dx + x, dy + y);
                    size |= modifiers.shift;
                }
            });
            if dx != 0.0 || dy != 0.0 {
                let [l, t, r, b] = self.crop.unwrap_or([0.25, 0.25, 0.75, 0.75]);
                let c = if size {
                    let r = (r + dx * NUDGE).clamp(l + NUDGE, 1.0);
                    [l, t, r, (b + dy * NUDGE).clamp(t + NUDGE, 1.0)]
                } else {
                    let (w, h) = (r - l, b - t);
                    let l = (l + dx * NUDGE).clamp(0.0, 1.0 - w);
                    let t = (t + dy * NUDGE).clamp(0.0, 1.0 - h);
                    [l, t, l + w, t + h]
                };
                self.crop = Some(c);
                self.error = None;
            }
        }
        if let Some(c) = self.drag.map(boxed).or(self.crop) {
            let at = |u: f32, v: f32| rect.min + rect.size() * Vec2::new(u, v);
            let part = Rect::from_min_max(at(c[0], c[1]), at(c[2], c[3]));
            let shade = Color32::from_black_alpha(120);
            for r in [
                Rect::from_x_y_ranges(rect.x_range(), rect.top()..=part.top()),
                Rect::from_x_y_ranges(rect.x_range(), part.bottom()..=rect.bottom()),
                Rect::from_x_y_ranges(rect.left()..=part.left(), part.y_range()),
                Rect::from_x_y_ranges(part.right()..=rect.right(), part.y_range()),
            ] {
                painter.rect_filled(r, 0.0, shade);
            }
            let line = Stroke::new(2.0, ui.visuals().selection.stroke.color);
            painter.rect_stroke(part, 0.0, line, egui::StrokeKind::Middle);
        }
        if resp.hovered() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::Crosshair);
        }
        ui.horizontal_wrapped(|ui| {
            if ui
                .add_enabled(
                    self.crop.is_some(),
                    egui::Button::new("Use the whole picture"),
                )
                .clicked()
            {
                self.crop = None;
            }
            ui.label("Drag over the picture, or use the arrow keys, to use part of it.");
        });
    }

    /// The signature made, or why it can't be used yet.
    fn entry(&self) -> Result<Entry, String> {
        let saved = match self.way {
            Way::Draw if self.strokes.is_empty() => return Err("Draw your signature first".into()),
            Way::Draw => Saved::Drawn {
                strokes: self.strokes.clone(),
                width: PEN,
            },
            Way::Type => Saved::Typed {
                text: typed_signature(&self.text, self.style)
                    .map_err(|e| capital(&e.to_string()))?,
                style: self.style,
            },
            Way::Picture => {
                let path = self.picture.clone().ok_or("Choose an image first")?;
                let saved = Saved::Picture {
                    path,
                    clear_paper: self.clear_paper,
                    crop: self.crop,
                };
                let bytes = saved.bytes().map_err(|e| e.to_string())?;
                saved
                    .stamp(&bytes, self.role, InkColor::Black)
                    .size("")
                    .map_err(|e| capital(&e.to_string()))?;
                saved
            }
        };
        Ok(Entry {
            saved,
            keep: self.keep,
        })
    }
}

/// The drawing pad's size on screen, and the most an image preview takes up: [`PAD`]'s
/// shape, up to [`PAD`], as wide as there is room for and no taller than the part of the
/// window's content that shows at once, so it can be scrolled wholly into view.
fn pad_size(ui: &egui::Ui) -> Vec2 {
    let k = (ui.available_width() / PAD.x).min(ui.clip_rect().height() / PAD.y);
    PAD * k.min(1.0)
}

/// The ink colours to pick from, in the Fill and sign pane and the Create window.
/// The ink colours, with "Ink" beside them, as in the Fill and sign panel.
fn ink_ui(ui: &mut egui::Ui, color: &mut InkColor) {
    ui.horizontal(|ui| {
        ui.label("Ink");
        ink_choices(ui, color);
    });
}

/// The ink colours, with "Ink" above them, as dialogs label their fields.
fn ink_field(ui: &mut egui::Ui, color: &mut InkColor) {
    ui.label("Ink");
    ui.add_space(2.0);
    ui.horizontal(|ui| ink_choices(ui, color));
}

fn ink_choices(ui: &mut egui::Ui, color: &mut InkColor) {
    // A dot in the ink's colour, then its name in the usual text colour, which stays
    // readable when picked.
    let font = egui::TextStyle::Button.resolve(ui.style());
    let fg = ui.visuals().text_color();
    for c in InkColor::ALL {
        let [r, g, b] = c.rgb().map(|v| (v * 255.0) as u8);
        let mut job = egui::text::LayoutJob::default();
        let fmt = |color| egui::TextFormat::simple(font.clone(), color);
        job.append("● ", 0.0, fmt(Color32::from_rgb(r, g, b)));
        job.append(&capital(c.name()), 0.0, fmt(fg));
        let r = ui.selectable_value(color, c, job);
        crate::dialogs::set_name(&r, &format!("{} ink", capital(c.name())));
    }
}

fn capital(s: &str) -> String {
    let mut c = s.chars();
    c.next()
        .map(|f| f.to_uppercase().chain(c).collect())
        .unwrap_or_default()
}

fn file_name(p: &Path) -> String {
    p.file_name()
        .map_or_else(|| p.display().to_string(), |n| n.to_string_lossy().into())
}

/// The Sign tool's pane, and the state it keeps.
pub struct SignPane {
    pub pick: SignPick,
    /// Words of your own to put down.
    pub text: String,
    /// The note about a character those words can't have.
    refused: page_text::Refused,
    pub previews: Previews,
    /// The Create signature window, when open, and where to put the result.
    pub create: Option<(Create, Option<(usize, StampPlace)>)>,
}

impl Default for SignPane {
    fn default() -> Self {
        SignPane {
            pick: SignPick::Saved(SignRole::Signature, 0),
            text: String::new(),
            refused: page_text::Refused::default(),
            previews: Previews::default(),
            create: None,
        }
    }
}

/// The saved signatures and initials as the Fill and sign button's menu shows them: each
/// one's picture, or `None` when it can't be drawn (a picture file gone).
pub struct SignMenu {
    signatures: Vec<Option<TextureHandle>>,
    initials: Vec<Option<TextureHandle>>,
}

/// The Fill and sign button's menu: Signature and Initials, each with the saved ones and
/// Create, then the date, your name, text and the marks. A choice turns the Fill and sign
/// tool on with it.
pub fn menu_ui(ui: &mut egui::Ui, menu: &SignMenu, acts: &mut Vec<Act>) {
    for (role, label, create) in [
        (SignRole::Signature, "Signature", "Create signature…"),
        (SignRole::Initials, "Initials", "Create initials…"),
    ] {
        let list = match role {
            SignRole::Signature => &menu.signatures,
            SignRole::Initials => &menu.initials,
        };
        ui.menu_button(label, |ui| {
            for (i, tex) in list.iter().enumerate() {
                let r = match tex {
                    Some(tex) => {
                        let img = egui::Image::new((tex.id(), Previews::size(tex, PREVIEW)));
                        ui.add(egui::Button::image(img))
                    }
                    None => ui.button("Picture not found"),
                };
                let name = format!("{label} {}", i + 1);
                r.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, &name));
                if r.clicked() {
                    acts.push(Act::SignWith(SignPick::Saved(role, i)));
                    ui.close();
                }
            }
            if !list.is_empty() {
                ui.separator();
            }
            if ui.button(create).clicked() {
                acts.push(Act::CreateSign(role));
                ui.close();
            }
        });
    }
    ui.separator();
    for (pick, label) in [
        (SignPick::Date, "Date"),
        (SignPick::Name, "Your name"),
        (SignPick::Text, "Text"),
    ] {
        if ui.button(label).clicked() {
            acts.push(Act::SignWith(pick));
            ui.close();
        }
    }
    ui.separator();
    for (m, glyph) in [(Mark::Tick, "✔"), (Mark::Cross, "✖"), (Mark::Dot, "●")] {
        let r = ui.button(format!("{glyph}  {}", capital(m.label())));
        dialogs::set_name(&r, &capital(m.label()));
        if r.clicked() {
            acts.push(Act::SignWith(SignPick::Mark(m)));
            ui.close();
        }
    }
}

/// What the pane asks of the app.
#[derive(Default)]
pub struct PaneOut {
    /// The Finish signing command.
    pub finish: bool,
}

impl SignPane {
    /// Whether the picked item exists: a saved signature that is in the list, or anything
    /// else.
    pub fn ready(&self, kit: &SignKit) -> bool {
        match self.pick {
            SignPick::Saved(role, i) => i < kit.list(role).len(),
            SignPick::Text => !self.text.trim().is_empty(),
            _ => true,
        }
    }

    /// The saved signatures and initials in `kit`, for the Fill and sign button's menu.
    pub fn menu(&mut self, ctx: &egui::Context, kit: &SignKit) -> SignMenu {
        let mut list = |role| {
            kit.list(role)
                .iter()
                .map(|e| self.previews.get(ctx, &e.saved, role, kit.color))
                .collect()
        };
        SignMenu {
            signatures: list(SignRole::Signature),
            initials: list(SignRole::Initials),
        }
    }

    /// Open the Create window for `role`, placing the result at `place` when given.
    pub fn start_create(
        &mut self,
        role: SignRole,
        author: &str,
        place: Option<(usize, StampPlace)>,
    ) {
        self.create = Some((Create::new(role, author), place));
    }

    /// The pane, its text box's id made from the tab's `scope`.
    pub fn ui(
        &mut self,
        ui: &mut egui::Ui,
        scope: egui::Id,
        kit: &mut SignKit,
        author: &mut String,
        items: usize,
    ) -> PaneOut {
        let mut out = PaneOut::default();
        ui.strong("Fill and sign");
        for role in [SignRole::Signature, SignRole::Initials] {
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                ui.label(match role {
                    SignRole::Signature => "Signatures",
                    SignRole::Initials => "Initials",
                });
                if ui
                    .small_button("New…")
                    .on_hover_text("Draw, type or choose a picture")
                    .clicked()
                {
                    self.start_create(role, author, None);
                }
            });
            if kit.list(role).is_empty() {
                let what = match role {
                    SignRole::Signature => "Create signature…",
                    SignRole::Initials => "Create initials…",
                };
                if ui.button(what).clicked() {
                    self.start_create(role, author, None);
                }
            }
            let mut remove = None;
            for (i, e) in kit.list(role).iter().enumerate() {
                let picked = self.pick == SignPick::Saved(role, i);
                // Not saved for next time: tagged beside it, so it's clear it will go.
                let r = ui
                    .horizontal(|ui| {
                        let r = match self.previews.get(ui.ctx(), &e.saved, role, kit.color) {
                            Some(tex) => {
                                let size = Previews::size(&tex, PREVIEW);
                                let img = egui::Image::new((tex.id(), size));
                                let r = ui.add(egui::Button::image(img).selected(picked));
                                // Named as in the toolbar's list.
                                let name = match role {
                                    SignRole::Signature => "Signature",
                                    SignRole::Initials => "Initials",
                                };
                                dialogs::set_name(&r, &format!("{name} {}", i + 1));
                                r
                            }
                            None => ui.selectable_label(picked, "Picture not found"),
                        };
                        if !e.keep {
                            ui.label(egui::RichText::new("until LimarePDF closes").small().weak());
                        }
                        r
                    })
                    .inner;
                if r.clicked() {
                    self.pick = SignPick::Saved(role, i);
                }
                r.context_menu(|ui| {
                    if ui.button("Remove from the list").clicked() {
                        remove = Some(i);
                        ui.close();
                    }
                });
            }
            if let Some(i) = remove {
                kit.remove(role, i);
                if let SignPick::Saved(r, j) = self.pick {
                    if r == role && j >= i && j > 0 {
                        self.pick = SignPick::Saved(r, j - 1);
                    }
                }
            }
        }
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            ui.selectable_value(&mut self.pick, SignPick::Date, "Date");
            let r = egui::ComboBox::from_id_salt("sign-date")
                .selected_text(kit.date.today())
                .show_ui(ui, |ui| {
                    for d in DateStyle::ALL {
                        ui.selectable_value(&mut kit.date, d, d.today());
                    }
                });
            dialogs::set_name(&r.response, "Date format");
        });
        ui.horizontal(|ui| {
            ui.selectable_value(&mut self.pick, SignPick::Name, "Name");
            let r = ui
                .add(egui::TextEdit::singleline(author).hint_text("Your name"))
                .on_hover_text("Also signs your comments (Edit > Your name for comments)");
            dialogs::set_name(&r, "Your name");
        });
        ui.horizontal(|ui| {
            ui.selectable_value(&mut self.pick, SignPick::Text, "Text");
            // Put down as a text box, in Helvetica or its embedded face: what neither has
            // isn't typed, as on the page.
            let id = scope.with("sign-text");
            if ui.ctx().memory(|m| m.has_focus(id)) {
                let helvetica = limarepdf_core::FontChoice::default();
                if let Some(c) = page_text::leave_out_untypable(ui, true, helvetica) {
                    let note = page_text::fixed_note(&c).to_string();
                    self.refused.mark(ui.ctx(), note);
                }
            }
            let out = egui::TextEdit::singleline(&mut self.text)
                .id(id)
                .hint_text("Type, then click")
                .show(ui);
            self.refused.show(ui.ctx(), &out, id);
            let r = out.response.response;
            dialogs::set_name(&r, "Text to place");
            if r.changed() {
                self.pick = SignPick::Text;
            }
        });
        ui.horizontal(|ui| {
            ui.label("Marks");
            for (m, glyph) in [(Mark::Tick, "✔"), (Mark::Cross, "✖"), (Mark::Dot, "●")] {
                let r = ui.selectable_value(&mut self.pick, SignPick::Mark(m), glyph);
                dialogs::set_name(&r, &capital(m.label()));
                r.on_hover_text(m.label());
            }
        });
        ink_ui(ui, &mut kit.color);
        ui.add_space(6.0);
        ui.weak("A picture, not a digital signature.");
        ui.add_space(6.0);
        ui.label(match items {
            0 => "Nothing placed yet.".to_string(),
            1 => "1 item can still be moved.".to_string(),
            n => format!("{n} items can still be moved."),
        });
        if ui
            .add_enabled(items > 0, egui::Button::new("Finish signing…"))
            .on_hover_text("Draw the placed items into the pages for good")
            .clicked()
        {
            out.finish = true;
        }
        out
    }

    /// Show the Create window while it is open; a signature made goes first in its list
    /// and is picked. Returns where to put it, when the window was opened by a click.
    pub fn create_ui(
        &mut self,
        ctx: &egui::Context,
        kit: &mut SignKit,
        error: &mut Option<String>,
    ) -> Option<(usize, StampPlace)> {
        let (c, place) = self.create.as_mut()?;
        match c.show(ctx, &mut self.previews, &mut kit.color) {
            Outcome::Open => None,
            Outcome::Cancel => {
                self.create = None;
                None
            }
            Outcome::Done(e) => {
                let (role, place) = (c.role, *place);
                self.create = None;
                // Usable now even when it couldn't be saved for next time, which is said.
                *error = kit.add(role, e);
                self.pick = SignPick::Saved(role, 0);
                place
            }
        }
    }
}

/// Whether `p` (page points) is in one of the boxes.
pub fn box_at(boxes: &[[f32; 4]], p: Pos2) -> Option<[f32; 4]> {
    boxes
        .iter()
        .find(|b| Rect::from_min_max(Pos2::new(b[0], b[1]), Pos2::new(b[2], b[3])).contains(p))
        .copied()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("limarepdf-sign-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir.join("signatures.txt")
    }

    /// Esc closes the Create window only while closing it loses nothing: the name it was
    /// filled in with doesn't count, a stroke, other text or an image does.
    #[test]
    fn create_holds_work_once_drawn_typed_or_chosen() {
        let fresh = Create::new(SignRole::Signature, "Sam Lee");
        assert!(!fresh.staged());
        let mut c = Create::new(SignRole::Signature, "Sam Lee");
        c.strokes.push(vec![(1.0, 1.0)]);
        assert!(c.staged());
        let mut c = Create::new(SignRole::Initials, "Sam Lee");
        c.text.push('X');
        assert!(c.staged());
        let mut c = Create::new(SignRole::Signature, "");
        c.picture = Some(PathBuf::from("sig.png"));
        assert!(c.staged());
    }

    /// Picking another tool closes the Create window.
    #[test]
    fn another_tool_closes_create() {
        let mut ed = crate::editor::Editor::default();
        ed.set_tool(crate::editor::Tool::Sign);
        ed.sign.start_create(SignRole::Signature, "Sam Lee", None);
        assert!(ed.sign.create.is_some());
        ed.set_tool(crate::editor::Tool::Read);
        assert!(ed.sign.create.is_none());
    }

    /// Saved through the safe writer (R04): the file is replaced in one step, not written
    /// into, so another name for the old file (a hard link) keeps the old contents.
    #[test]
    fn saved_in_one_step() {
        let f = file("one-step");
        std::fs::create_dir_all(f.parent().unwrap()).unwrap();
        std::fs::write(&f, "ink = blue\n").unwrap();
        let other = f.with_file_name("other.txt");
        std::fs::hard_link(&f, &other).unwrap();
        assert_eq!(SignKit::from_file(Some(f.clone())).save().unwrap(), None);
        assert_eq!(std::fs::read_to_string(&other).unwrap(), "ink = blue\n");
        assert!(std::fs::read_to_string(&f)
            .unwrap()
            .starts_with("ink = blue\ndate = "));
        std::fs::remove_dir_all(f.parent().unwrap()).unwrap();
    }

    #[test]
    fn signatures_are_saved_and_read_back() {
        let f = file("saved");
        let mut k = SignKit::from_file(Some(f.clone()));
        assert!(k.signatures.is_empty() && k.color == InkColor::Black);
        k.color = InkColor::Blue;
        k.date = DateStyle::MonthDayYear;
        let drawn = Saved::Drawn {
            strokes: vec![vec![(1.0, 2.0), (3.5, 4.0)], vec![(9.0, 9.0)]],
            width: 2.5,
        };
        let typed = Saved::Typed {
            text: "Sam Lee".into(),
            style: SignStyle::Zeyada,
        };
        let drawn_entry = Entry {
            saved: drawn.clone(),
            keep: true,
        };
        assert_eq!(k.add(SignRole::Signature, drawn_entry), None);
        let typed_entry = Entry {
            saved: typed.clone(),
            keep: true,
        };
        assert_eq!(k.add(SignRole::Initials, typed_entry), None);
        // Not remembered: listed now, gone next time.
        let other = Saved::Typed {
            text: "Sam".into(),
            style: SignStyle::Caveat,
        };
        let other = Entry {
            saved: other,
            keep: false,
        };
        assert_eq!(k.add(SignRole::Signature, other), None);
        assert_eq!(k.signatures.len(), 2);
        assert_eq!(k.save().unwrap(), None);
        let k = SignKit::from_file(Some(f));
        assert_eq!((k.color, k.date), (InkColor::Blue, DateStyle::MonthDayYear));
        assert_eq!(
            k.signatures,
            [Entry {
                saved: drawn,
                keep: true
            }]
        );
        assert_eq!(
            k.initials,
            [Entry {
                saved: typed,
                keep: true
            }]
        );
    }

    #[test]
    fn remembered_pictures_are_copied_and_deleted_with_their_entry() {
        let f = file("pictures");
        let dir = f.parent().unwrap().to_path_buf();
        std::fs::create_dir_all(&dir).unwrap();
        let original = dir.join("mine.png");
        std::fs::write(&original, b"not really a png").unwrap();
        let mut k = SignKit::from_file(Some(f));
        let saved = Saved::Picture {
            path: original.clone(),
            clear_paper: true,
            crop: Some([0.0, 0.25, 1.0, 0.75]),
        };
        let e = Entry { saved, keep: true };
        assert_eq!(k.add(SignRole::Signature, e), None);
        let Saved::Picture {
            path: copy, crop, ..
        } = k.signatures[0].saved.clone()
        else {
            panic!("not a picture");
        };
        assert!(copy.starts_with(dir.join("signatures")) && copy.exists());
        // The whole picture is copied; the part used stays a part of it.
        assert_eq!(std::fs::read(&copy).unwrap(), b"not really a png");
        assert_eq!(crop, Some([0.0, 0.25, 1.0, 0.75]));
        k.remove(SignRole::Signature, 0);
        assert!(!copy.exists() && original.exists());
    }

    /// BUG-14: a picture whose copy can't be kept is still listed, to use until the app
    /// closes, and that is said; it isn't saved for next time.
    #[test]
    fn a_picture_that_can_t_be_copied_is_kept_for_the_session() {
        let f = file("picture-unkept");
        let dir = f.parent().unwrap().to_path_buf();
        std::fs::create_dir_all(&dir).unwrap();
        let original = dir.join("mine.png");
        std::fs::write(&original, b"not really a png").unwrap();
        // A file where the pictures' folder would be.
        std::fs::write(dir.join("signatures"), "a file, not a folder").unwrap();
        let mut k = SignKit::from_file(Some(f.clone()));
        let saved = Saved::Picture {
            path: original.clone(),
            clear_paper: true,
            crop: None,
        };
        let said = k.add(SignRole::Signature, Entry { saved, keep: true });
        let said = said.expect("not said");
        let start = "The signature can be used until LimarePDF closes, but LimarePDF couldn't \
                     keep a copy of its picture for next time (";
        assert!(said.starts_with(start), "{said}");
        let e = &k.signatures[0];
        assert!(!e.keep);
        assert!(matches!(&e.saved, Saved::Picture { path, .. } if *path == original));
        assert_eq!(k.save().unwrap(), None);
        assert!(SignKit::from_file(Some(f)).signatures.is_empty());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn only_five_are_kept_and_the_newest_comes_first() {
        let mut k = SignKit::from_file(None);
        for i in 0..7 {
            let saved = Saved::Typed {
                text: format!("Sam {i}"),
                style: SignStyle::Cedarville,
            };
            let e = Entry { saved, keep: true };
            assert_eq!(k.add(SignRole::Signature, e), None);
        }
        assert_eq!(k.signatures.len(), SAVED);
        assert!(matches!(&k.signatures[0].saved, Saved::Typed { text, .. } if text == "Sam 6"));
    }

    const DRAWING: &str = "Drawing area: sign with the mouse or a pen";
    const PICTURE: &str =
        "Picture: drag around the signature, or use the arrow keys, to use only that part";

    /// The Create window on a screen `size` points big, at `scale` pixels per point.
    struct Win {
        ctx: egui::Context,
        c: Create,
        previews: Previews,
        size: Vec2,
        scale: f32,
    }

    impl Win {
        fn new(c: Create, size: Vec2, scale: f32) -> Self {
            let ctx = egui::Context::default();
            ctx.enable_accesskit();
            crate::theme::install(&ctx);
            Win {
                ctx,
                c,
                previews: Previews::default(),
                size,
                scale,
            }
        }

        /// Draws the window with `events`, and returns its AccessKit tree.
        fn frame(&mut self, events: Vec<egui::Event>) -> egui::accesskit::TreeUpdate {
            let mut raw = egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, self.size)),
                events,
                ..Default::default()
            };
            raw.viewports
                .entry(egui::ViewportId::ROOT)
                .or_default()
                .native_pixels_per_point = Some(self.scale);
            let Win {
                ctx, c, previews, ..
            } = self;
            let mut out = ctx.run_ui(raw, |_| {
                c.show(ctx, previews, &mut InkColor::Black);
            });
            out.textures_delta.clear();
            out.platform_output
                .accesskit_update
                .expect("the window's widgets")
        }

        /// Draws the window until it has settled.
        fn settled(&mut self) -> egui::accesskit::TreeUpdate {
            for _ in 0..4 {
                self.frame(vec![]);
            }
            self.frame(vec![])
        }

        /// Moves the pointer to `pos`, pressing or releasing the primary button there.
        fn pointer(&mut self, pos: Pos2, pressed: Option<bool>) {
            let mut events = vec![egui::Event::PointerMoved(pos)];
            events.extend(pressed.map(|pressed| egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed,
                modifiers: Default::default(),
            }));
            self.frame(events);
        }
    }

    /// Where the control named `label` is, in points.
    fn bounds(tree: &egui::accesskit::TreeUpdate, label: &str) -> Rect {
        let b = tree
            .nodes
            .iter()
            .find(|(_, n)| n.label() == Some(label))
            .and_then(|(_, n)| n.bounds())
            .unwrap_or_else(|| panic!("no {label}"));
        Rect::from_min_max(
            Pos2::new(b.x0 as f32, b.y0 as f32),
            Pos2::new(b.x1 as f32, b.y1 as f32),
        )
    }

    /// G02: drawn, typed or chosen, the Create window and its work area stay inside a narrow,
    /// short or wide window at every scale, with Use signature on screen; and what it holds
    /// is kept as the window is resized.
    #[test]
    fn create_fits_narrow_short_and_wide_windows() {
        let picture = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs/images/icon.png");
        let sizes = [
            Vec2::new(400.0, 800.0),
            Vec2::new(700.0, 300.0),
            Vec2::new(1000.0, 450.0),
        ];
        for scale in [1.0, 1.5, 2.0] {
            for way in [Way::Draw, Way::Type, Way::Picture] {
                let mut c = Create::new(SignRole::Signature, "Sam Lee");
                c.way = way;
                c.strokes = vec![vec![(30.0, 120.0), (450.0, 40.0)], vec![(240.0, 80.0)]];
                c.text = "Sam Q. Lee".into();
                c.picture = Some(picture.clone());
                let mut w = Win::new(c, sizes[0], scale);
                for size in sizes {
                    w.size = size;
                    let tree = w.settled();
                    let screen = Rect::from_min_size(Pos2::ZERO, size).expand(0.5);
                    let id = egui::Id::new(("dialog", "Create signature"));
                    let window = w.ctx.memory(|m| m.area_rect(id)).expect("the window");
                    let at = format!("{way:?} at {size:?}, scale {scale}");
                    assert!(screen.contains_rect(window), "{at}: window {window:?}");
                    let done = bounds(&tree, "Use signature");
                    assert!(screen.contains_rect(done), "{at}: Use signature {done:?}");
                    if way == Way::Draw {
                        // As wide as the window allows, and no taller than the content
                        // that shows at once (dialogs::content): whole, above the
                        // buttons, at the pad's shape.
                        let pad = bounds(&tree, DRAWING);
                        let wide = pad.left() >= window.left() - 0.5
                            && pad.right() <= window.right() + 0.5;
                        let tall = pad.top() >= window.top() && pad.bottom() <= done.top();
                        assert!(wide && tall, "{at}: pad {pad:?}");
                        let shape = pad.width() / pad.height();
                        assert!((shape - 3.0).abs() < 0.05, "{at}: pad {pad:?}");
                    }
                }
                let c = &w.c;
                assert_eq!(c.strokes.len(), 2);
                assert_eq!(c.strokes[0], [(30.0, 120.0), (450.0, 40.0)]);
                assert_eq!(c.text, "Sam Q. Lee");
                assert_eq!(c.picture.as_ref(), Some(&picture));
            }
        }
    }

    /// G06: drawn, typed or chosen, for a signature or initials, every control in the Create
    /// window says what it is, and the handwriting styles say which one is picked.
    #[test]
    fn create_controls_are_named() {
        use egui::accesskit::Toggled;
        let picture = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs/images/icon.png");
        for role in [SignRole::Signature, SignRole::Initials] {
            for way in [Way::Draw, Way::Type, Way::Picture] {
                let mut c = Create::new(role, "Sam Lee");
                c.way = way;
                c.picture = Some(picture.clone());
                let tree = Win::new(c, Vec2::new(1000.0, 800.0), 1.0).settled();
                let unclear = crate::dialogs::unclear_names(&tree);
                assert_eq!(unclear, [""; 0], "{role:?}, {way:?}");
            }
        }
        let mut c = Create::new(SignRole::Signature, "Sam Lee");
        c.way = Way::Type;
        c.style = SignStyle::ALL[1];
        let tree = Win::new(c, Vec2::new(1000.0, 800.0), 1.0).settled();
        let toggled = |name: &str| {
            let n = tree.nodes.iter().find(|(_, n)| n.label() == Some(name));
            n.and_then(|(_, n)| n.toggled())
        };
        for (i, style) in SignStyle::ALL.into_iter().enumerate() {
            let on = [Toggled::False, Toggled::True][usize::from(i == 1)];
            assert_eq!(toggled(style.label()), Some(on), "{style:?}");
        }
    }

    /// G02: strokes are kept in the full-size pad's points, so a stroke drawn wide stays as it
    /// was when the window narrows, and one drawn on the narrower pad lands where it would on
    /// the full-size one; the saved signature holds them unchanged.
    #[test]
    fn strokes_keep_their_place_as_the_pad_resizes() {
        let c = Create::new(SignRole::Signature, "Sam Lee");
        // Tall enough not to scroll: a scroll bar takes some of the pad's width.
        let mut w = Win::new(c, Vec2::new(1000.0, 600.0), 1.0);
        let pad = bounds(&w.settled(), DRAWING);
        assert_eq!(pad.size(), PAD);
        let at = |t: f32| pad.lerp_inside(Vec2::new(t, 0.5));
        w.pointer(at(0.25), Some(true));
        for t in [0.4, 0.6, 0.75] {
            w.pointer(at(t), None);
        }
        w.pointer(at(0.75), Some(false));
        let drawn = w.c.strokes.clone();
        assert_eq!(drawn.len(), 1, "{drawn:?}");
        let near = |(x, y): (f32, f32), (u, v): (f32, f32)| (x - u).hypot(y - v) < 1.0;
        assert!(near(drawn[0][0], (120.0, 80.0)), "{drawn:?}");
        assert!(near(*drawn[0].last().unwrap(), (360.0, 80.0)), "{drawn:?}");

        w.size = Vec2::new(400.0, 800.0);
        let pad = bounds(&w.settled(), DRAWING);
        assert!(pad.width() < PAD.x && pad.right() <= w.size.x, "{pad:?}");
        assert_eq!(w.c.strokes, drawn);
        let p = pad.lerp_inside(Vec2::new(0.75, 0.25));
        w.pointer(p, Some(true));
        w.pointer(p, Some(false));
        let dot = w.c.strokes.last().cloned().unwrap();
        assert_eq!(w.c.strokes.len(), 2, "{:?}", w.c.strokes);
        assert!(near(dot[0], (360.0, 40.0)), "{dot:?}");

        w.size = Vec2::new(1000.0, 600.0);
        w.settled();
        let Ok(Entry {
            saved: Saved::Drawn { strokes, width },
            ..
        }) = w.c.entry()
        else {
            panic!("not a drawn signature");
        };
        assert_eq!((strokes, width), (vec![drawn[0].clone(), dot], PEN));
    }

    /// BUG-21: dragging over the chosen picture, either way, chooses the part used, which the
    /// signature made keeps; a slip of the mouse doesn't, and Use the whole picture goes back
    /// to all of it.
    #[test]
    fn dragging_over_the_picture_chooses_the_part_used() {
        let picture = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs/images/icon.png");
        let mut c = Create::new(SignRole::Signature, "Sam Lee");
        c.way = Way::Picture;
        c.picture = Some(picture.clone());
        c.clear_paper = false;
        let mut w = Win::new(c, Vec2::new(1000.0, 800.0), 1.0);
        let view = bounds(&w.settled(), PICTURE);
        let at = |u: f32, v: f32| view.lerp_inside(Vec2::new(u, v));
        w.pointer(at(0.6, 0.7), Some(true));
        for t in [0.5, 0.4, 0.3] {
            w.pointer(at(t, t + 0.05), None);
        }
        w.pointer(at(0.25, 0.2), Some(false));
        let crop = w.c.crop.expect("no part chosen");
        let near = |a: [f32; 4], b: [f32; 4]| a.iter().zip(b).all(|(x, y)| (x - y).abs() < 0.01);
        assert!(near(crop, [0.25, 0.2, 0.6, 0.7]), "{crop:?}");
        // A box of a pixel or two is a slip.
        let p = at(0.1, 0.1);
        w.pointer(p, Some(true));
        w.pointer(p + Vec2::splat(6.0), None);
        w.pointer(p + Vec2::new(2.0, 1.0), Some(false));
        assert_eq!(w.c.crop, Some(crop));
        w.settled();
        let Ok(Entry {
            saved: Saved::Picture { crop: kept, .. },
            ..
        }) = w.c.entry()
        else {
            panic!("not a picture");
        };
        assert_eq!(kept, Some(crop));
        let whole = bounds(&w.settled(), "Use the whole picture").center();
        w.pointer(whole, Some(true));
        w.pointer(whole, Some(false));
        assert_eq!(w.c.crop, None);
    }

    /// The part used is chosen from the keyboard too (PB-21): with the picture focused, the
    /// arrow keys move the part (the middle half to begin with) and Shift with them sizes it,
    /// always inside the picture.
    #[test]
    fn the_arrow_keys_choose_the_part_used() {
        use egui::Key;
        let picture = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs/images/icon.png");
        let mut c = Create::new(SignRole::Signature, "Sam Lee");
        c.way = Way::Picture;
        c.picture = Some(picture);
        c.clear_paper = false;
        let mut w = Win::new(c, Vec2::new(1000.0, 800.0), 1.0);
        let view = bounds(&w.settled(), PICTURE);
        // A click gives the picture the keyboard and chooses nothing. (egui keeps the arrows
        // for it from the frame after it took the keyboard.)
        w.pointer(view.center(), Some(true));
        w.pointer(view.center(), Some(false));
        w.settled();
        assert_eq!(w.c.crop, None);
        let key = |key, modifiers| egui::Event::Key {
            key,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers,
        };
        let none = egui::Modifiers::NONE;
        w.frame(vec![key(Key::ArrowRight, none)]);
        let near = |a: [f32; 4], b: [f32; 4]| a.iter().zip(b).all(|(x, y)| (x - y).abs() < 1e-4);
        let crop = w.c.crop.expect("no part chosen");
        assert!(near(crop, [0.27, 0.25, 0.77, 0.75]), "{crop:?}");
        w.frame(vec![key(Key::ArrowDown, egui::Modifiers::SHIFT)]);
        let crop = w.c.crop.unwrap();
        assert!(near(crop, [0.27, 0.25, 0.77, 0.77]), "{crop:?}");
        // Held against the picture's edges.
        for _ in 0..30 {
            w.frame(vec![key(Key::ArrowLeft, none), key(Key::ArrowUp, none)]);
        }
        let crop = w.c.crop.unwrap();
        assert!(near(crop, [0.0, 0.0, 0.5, 0.52]), "{crop:?}");
        for _ in 0..30 {
            w.frame(vec![key(Key::ArrowLeft, egui::Modifiers::SHIFT)]);
        }
        let crop = w.c.crop.unwrap();
        assert!(near(crop, [0.0, 0.0, NUDGE, 0.52]), "{crop:?}");
    }

    #[test]
    fn a_part_of_a_picture_is_remembered() {
        for crop in [None, Some([0.125, 0.5, 0.75, 1.0])] {
            for clear_paper in [true, false] {
                let s = Saved::Picture {
                    path: PathBuf::from("/pictures/my signature.png"),
                    clear_paper,
                    crop,
                };
                assert_eq!(Saved::parse(&s.line()), Some(s.clone()), "{}", s.line());
            }
        }
        // A part outside the picture, turned inside out or not of four shares isn't read.
        for bad in [
            "cropped 0.5,0,0.2,1 clear /x.png",
            "cropped 0,0,1.5,1 keep /x.png",
            "cropped 0,0,1 clear /x.png",
            "cropped a,0,1,1 clear /x.png",
        ] {
            assert_eq!(Saved::parse(bad), None, "{bad}");
        }
    }

    #[test]
    fn initials_come_from_the_name() {
        assert_eq!(initials_of("  sam  van lee "), "SVL");
        assert_eq!(initials_of(""), "");
    }
}

//! Fill and sign: the saved signatures and initials, the Create signature window and the
//! Sign tool's pane. What is placed is a picture of a signature, not a digital signature.
//!
//! Saved signatures are kept in `signatures.txt` in MPE's settings folder, and pictures
//! as copies in its `signatures` folder, so they keep working when the original moves.
//! Like the other settings they stay on this computer, unencrypted, and failing to read
//! or write them is not an error.

use eframe::egui::{self, Color32, Pos2, Rect, Sense, Stroke, TextureHandle, TextureOptions, Vec2};
use mpe_core::{
    typed_signature, DateStyle, InkColor, Mark, SignArt, SignRole, SignStyle, Stamp, StampPlace,
};
use std::collections::HashMap;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::path::{Path, PathBuf};

/// How many signatures, and how many sets of initials, are remembered.
pub const SAVED: usize = 5;
/// The drawing pad's size, and the pen's width on it, in points.
const PAD: Vec2 = Vec2::new(480.0, 160.0);
const PEN: f32 = 2.5;
/// The most a preview in the pane takes up, in points.
const PREVIEW: Vec2 = Vec2::new(190.0, 48.0);

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
    /// A picture: its file (a copy in the settings folder when remembered), and whether
    /// the paper around the ink is made transparent.
    Picture {
        path: PathBuf,
        clear_paper: bool,
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
        let art = match self {
            Saved::Drawn { strokes, width } => SignArt::Drawn {
                strokes,
                width: *width,
            },
            Saved::Typed { text, style } => SignArt::Typed {
                text,
                style: *style,
            },
            Saved::Picture { clear_paper, .. } => SignArt::Picture {
                bytes,
                clear_paper: *clear_paper,
            },
        };
        Stamp::Signature {
            art,
            role,
            color,
            date: false,
        }
    }

    /// As a line of the file: `draw WIDTH x,y x,y;x,y …`, `type STYLE TEXT` or
    /// `picture clear|keep PATH`.
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
            Saved::Picture { path, clear_paper } => {
                let how = if *clear_paper { "clear" } else { "keep" };
                format!("picture {how} {}", path.display())
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

    pub fn save(&self) {
        let Some(file) = &self.file else { return };
        if let Some(dir) = file.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let mut text = format!("ink = {}\ndate = {}\n", self.color.name(), self.date.name());
        for (key, list) in [
            ("signature", &self.signatures),
            ("initials", &self.initials),
        ] {
            for e in list.iter().filter(|e| e.keep) {
                text += &format!("{key} = {}\n", e.saved.line());
            }
        }
        let _ = std::fs::write(file, text);
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

    /// The folder remembered pictures are copied to.
    fn picture_dir(&self) -> Option<PathBuf> {
        Some(self.file.as_ref()?.parent()?.join("signatures"))
    }

    /// Put a signature or set of initials first in its list, copying a remembered picture
    /// into the settings folder. The oldest beyond [`SAVED`] is dropped.
    pub fn add(&mut self, role: SignRole, mut e: Entry) -> Result<(), String> {
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
            std::fs::create_dir_all(&dir)
                .and_then(|_| std::fs::copy(&*path, &copy))
                .map_err(|err| format!("Can't keep a copy of the picture: {err}"))?;
            *path = copy;
        }
        let list = self.list_mut(role);
        list.retain(|x| x.saved != e.saved);
        list.insert(0, e);
        let dropped: Vec<Entry> = list.drain(SAVED.min(list.len())..).collect();
        for d in dropped {
            self.forget(&d);
        }
        Ok(())
    }

    /// Take a signature or set of initials out of its list, deleting its picture's copy.
    pub fn remove(&mut self, role: SignRole, i: usize) {
        let list = self.list_mut(role);
        if i < list.len() {
            let e = list.remove(i);
            self.forget(&e);
        }
    }

    /// Delete a picture copy MPE made for `e`.
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

/// Pictures of signatures for lists, made by drawing them with MPE's renderer, so they
/// look exactly as they will on the page.
#[derive(Default)]
pub struct Previews {
    made: HashMap<u64, Option<TextureHandle>>,
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
        let mut h = DefaultHasher::new();
        format!("{saved:?}{role:?}{color:?}").hash(&mut h);
        let key = h.finish();
        if self.made.len() > 64 {
            self.made.clear();
        }
        self.made
            .entry(key)
            .or_insert_with(|| {
                let bytes = saved.bytes().ok()?;
                let bmp = saved.stamp(&bytes, role, color).preview("", 2.0).ok()?;
                let img =
                    egui::ColorImage::from_rgba_unmultiplied([bmp.width, bmp.height], &bmp.rgba);
                Some(ctx.load_texture(format!("sign-{key}"), img, TextureOptions::LINEAR))
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
    text: String,
    style: SignStyle,
    picture: Option<PathBuf>,
    clear_paper: bool,
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
            text,
            style: SignStyle::Caveat,
            picture: None,
            clear_paper: true,
            keep: true,
            error: None,
        }
    }

    pub fn show(
        &mut self,
        ctx: &egui::Context,
        previews: &mut Previews,
        color: InkColor,
    ) -> Outcome {
        let title = match self.role {
            SignRole::Signature => "Create signature",
            SignRole::Initials => "Create initials",
        };
        let mut open = true;
        let mut out = Outcome::Open;
        egui::Window::new(title)
            .collapsible(false)
            .resizable(false)
            .min_width(PAD.x)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .open(&mut open)
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    for (w, label) in [
                        (Way::Draw, "Draw"),
                        (Way::Type, "Type"),
                        (Way::Picture, "Picture"),
                    ] {
                        if ui.selectable_label(self.way == w, label).clicked() {
                            self.way = w;
                            self.error = None;
                        }
                    }
                });
                ui.add_space(6.0);
                match self.way {
                    Way::Draw => self.pad(ui, color),
                    Way::Type => self.typed(ui, previews, color),
                    Way::Picture => self.picture(ui, previews, color),
                }
                ui.add_space(6.0);
                ui.checkbox(&mut self.keep, "Save for next time")
                    .on_hover_text(
                        "Keep it in MPE's settings folder on this computer, so it is there the \
                     next time MPE starts. Unticked, it stays in the list only until MPE closes.",
                    );
                if let Some(e) = &self.error {
                    ui.colored_label(ui.visuals().error_fg_color, e);
                }
                ui.separator();
                ui.horizontal(|ui| {
                    if ui.button("Done").clicked() {
                        match self.entry() {
                            Ok(e) => out = Outcome::Done(e),
                            Err(e) => self.error = Some(e),
                        }
                    }
                    if ui.button("Cancel").clicked() {
                        out = Outcome::Cancel;
                    }
                });
            });
        if !open {
            return Outcome::Cancel;
        }
        out
    }

    /// The drawing pad: white, with a line to sign on.
    fn pad(&mut self, ui: &mut egui::Ui, color: InkColor) {
        let (rect, resp) = ui.allocate_exact_size(PAD, Sense::click_and_drag());
        let painter = ui.painter_at(rect);
        painter.rect_filled(rect, 4.0, Color32::WHITE);
        let base = rect.top() + rect.height() * 0.72;
        painter.line_segment(
            [
                Pos2::new(rect.left() + 24.0, base),
                Pos2::new(rect.right() - 24.0, base),
            ],
            Stroke::new(1.0, Color32::from_gray(200)),
        );
        let local = |p: Pos2| (p.x - rect.left(), p.y - rect.top());
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
                .map(|&(x, y)| Pos2::new(rect.left() + x, rect.top() + y))
                .collect();
            if pts.len() == 1 {
                painter.circle_filled(pts[0], PEN / 2.0, ink);
            } else {
                painter.add(egui::Shape::line(pts, Stroke::new(PEN, ink)));
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
        ui.add(
            egui::TextEdit::singleline(&mut self.text)
                .hint_text(match self.role {
                    SignRole::Signature => "Your name",
                    SignRole::Initials => "Your initials",
                })
                .desired_width(PAD.x),
        );
        ui.add_space(4.0);
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
                    let size = Previews::size(&tex, Vec2::new(PAD.x, 60.0));
                    let img = egui::Image::new((tex.id(), size));
                    ui.add(egui::Button::image(img).selected(self.style == style))
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

    /// Choosing a picture of a signature on paper.
    fn picture(&mut self, ui: &mut egui::Ui, previews: &mut Previews, color: InkColor) {
        ui.horizontal(|ui| {
            if ui.button("Choose a picture…").clicked() {
                if let Some(p) = rfd::FileDialog::new()
                    .add_filter("Pictures", &["png", "jpg", "jpeg", "tif", "tiff"])
                    .pick_file()
                {
                    self.picture = Some(p);
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
        if let Some(p) = &self.picture {
            let saved = Saved::Picture {
                path: p.clone(),
                clear_paper: self.clear_paper,
            };
            match previews.get(ui.ctx(), &saved, self.role, color) {
                Some(tex) => {
                    let size = Previews::size(&tex, Vec2::new(PAD.x, PAD.y));
                    ui.image((tex.id(), size));
                }
                None => {
                    let why = saved
                        .bytes()
                        .map_err(|e| e.to_string())
                        .and_then(|b| {
                            saved
                                .stamp(&b, self.role, color)
                                .size("")
                                .map(drop)
                                .map_err(|e| e.to_string())
                        })
                        .err()
                        .unwrap_or_else(|| "Can't show this picture".into());
                    ui.colored_label(ui.visuals().error_fg_color, capital(&why));
                }
            }
        }
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
                let path = self.picture.clone().ok_or("Choose a picture first")?;
                let saved = Saved::Picture {
                    path,
                    clear_paper: self.clear_paper,
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
    pub previews: Previews,
    /// The Create signature window, when open, and where to put the result.
    pub create: Option<(Create, Option<(usize, StampPlace)>)>,
}

impl Default for SignPane {
    fn default() -> Self {
        SignPane {
            pick: SignPick::Saved(SignRole::Signature, 0),
            text: String::new(),
            previews: Previews::default(),
            create: None,
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

    /// Open the Create window for `role`, placing the result at `place` when given.
    pub fn start_create(
        &mut self,
        role: SignRole,
        author: &str,
        place: Option<(usize, StampPlace)>,
    ) {
        self.create = Some((Create::new(role, author), place));
    }

    pub fn ui(
        &mut self,
        ui: &mut egui::Ui,
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
                                ui.add(egui::Button::image(img).selected(picked))
                            }
                            None => ui.selectable_label(picked, "Picture not found"),
                        };
                        if !e.keep {
                            ui.label(egui::RichText::new("until MPE closes").small().weak());
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
            egui::ComboBox::from_id_salt("sign-date")
                .selected_text(kit.date.today())
                .show_ui(ui, |ui| {
                    for d in DateStyle::ALL {
                        ui.selectable_value(&mut kit.date, d, d.today());
                    }
                });
        });
        ui.horizontal(|ui| {
            ui.selectable_value(&mut self.pick, SignPick::Name, "Name");
            ui.add(egui::TextEdit::singleline(author).hint_text("Your name"))
                .on_hover_text("Also signs your comments (Edit > Your name for comments)");
        });
        ui.horizontal(|ui| {
            ui.selectable_value(&mut self.pick, SignPick::Text, "Text");
            let r =
                ui.add(egui::TextEdit::singleline(&mut self.text).hint_text("Type, then click"));
            if r.changed() {
                self.pick = SignPick::Text;
            }
        });
        ui.horizontal(|ui| {
            ui.label("Marks");
            for (m, glyph) in [(Mark::Tick, "✔"), (Mark::Cross, "✖"), (Mark::Dot, "●")] {
                ui.selectable_value(&mut self.pick, SignPick::Mark(m), glyph)
                    .on_hover_text(m.label());
            }
        });
        ui.horizontal(|ui| {
            ui.label("Ink");
            // A dot in the ink's colour, then its name in the usual text colour, which
            // stays readable when picked.
            let font = egui::TextStyle::Button.resolve(ui.style());
            let fg = ui.visuals().text_color();
            for c in InkColor::ALL {
                let [r, g, b] = c.rgb().map(|v| (v * 255.0) as u8);
                let mut job = egui::text::LayoutJob::default();
                let fmt = |color| egui::TextFormat::simple(font.clone(), color);
                job.append("● ", 0.0, fmt(Color32::from_rgb(r, g, b)));
                job.append(&capital(c.name()), 0.0, fmt(fg));
                ui.selectable_value(&mut kit.color, c, job);
            }
        });
        ui.add_space(6.0);
        ui.label(
            "Click on the page to put the picked item down, or drag a box to size it. Click \
             an empty signature field to sign in it. Ctrl+click an item to select it, then \
             move or resize it.",
        );
        ui.label(
            egui::RichText::new(
                "This is a picture of your signature, not a digital signature: it doesn't \
                 prove who signed.",
            )
            .weak(),
        );
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
        match c.show(ctx, &mut self.previews, kit.color) {
            Outcome::Open => None,
            Outcome::Cancel => {
                self.create = None;
                None
            }
            Outcome::Done(e) => {
                let (role, place) = (c.role, *place);
                self.create = None;
                match kit.add(role, e) {
                    Ok(()) => {
                        self.pick = SignPick::Saved(role, 0);
                        place
                    }
                    Err(e) => {
                        *error = Some(e);
                        None
                    }
                }
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
        let dir = std::env::temp_dir().join(format!("mpe-sign-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir.join("signatures.txt")
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
        k.add(
            SignRole::Signature,
            Entry {
                saved: drawn.clone(),
                keep: true,
            },
        )
        .unwrap();
        k.add(
            SignRole::Initials,
            Entry {
                saved: typed.clone(),
                keep: true,
            },
        )
        .unwrap();
        // Not remembered: listed now, gone next time.
        let other = Saved::Typed {
            text: "Sam".into(),
            style: SignStyle::Caveat,
        };
        k.add(
            SignRole::Signature,
            Entry {
                saved: other,
                keep: false,
            },
        )
        .unwrap();
        assert_eq!(k.signatures.len(), 2);
        k.save();
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
        };
        k.add(SignRole::Signature, Entry { saved, keep: true })
            .unwrap();
        let Saved::Picture { path: copy, .. } = k.signatures[0].saved.clone() else {
            panic!("not a picture");
        };
        assert!(copy.starts_with(dir.join("signatures")) && copy.exists());
        k.remove(SignRole::Signature, 0);
        assert!(!copy.exists() && original.exists());
    }

    #[test]
    fn only_five_are_kept_and_the_newest_comes_first() {
        let mut k = SignKit::from_file(None);
        for i in 0..7 {
            let saved = Saved::Typed {
                text: format!("Sam {i}"),
                style: SignStyle::Cedarville,
            };
            k.add(SignRole::Signature, Entry { saved, keep: true })
                .unwrap();
        }
        assert_eq!(k.signatures.len(), SAVED);
        assert!(matches!(&k.signatures[0].saved, Saved::Typed { text, .. } if text == "Sam 6"));
    }

    #[test]
    fn initials_come_from_the_name() {
        assert_eq!(initials_of("  sam  van lee "), "SVL");
        assert_eq!(initials_of(""), "");
    }
}

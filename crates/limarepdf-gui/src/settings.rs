//! Settings kept between runs, in LimarePDF's settings folder:
//!
//! - Linux and other Unix: `$XDG_CONFIG_HOME/limarepdf/`, else `~/.config/limarepdf/`
//! - macOS: `~/Library/Application Support/limarepdf/`
//! - Windows: `%APPDATA%\limarepdf\`
//!
//! `settings.txt` holds `key = value` lines; unknown keys and bad values are ignored. The
//! files stay on this computer. Failing to read them is not an error; failing to write them
//! is said ([`Unsaved`]), and what was changed works until the app closes.

use crate::editor::Tool;
use crate::signing::SignKit;
use anyhow::Context as _;
use eframe::egui::ThemePreference;
use limarepdf_core::page_colours::PageColours;
pub use limarepdf_core::settings_dir;
use limarepdf_core::{OpenLayout, StampColor};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// Writes one of LimarePDF's own files (settings, saved signatures, recent files), making
/// its folder first. Returns why it couldn't be written, or, written, why the system
/// couldn't confirm it is safely on disk.
pub fn write_own(file: &Path, text: &[u8]) -> anyhow::Result<Option<String>> {
    if let Some(dir) = file.parent() {
        std::fs::create_dir_all(dir).with_context(|| format!("making {}", dir.display()))?;
    }
    crate::app::unconfirmed(limarepdf_core::write_file(file, text))
}

/// Something of LimarePDF's own that couldn't be saved, for the app to say: a title and a
/// message.
pub type Unsaved = (String, String);

/// What [`write_own`] writing `what` ("its settings") left to say, if anything.
pub fn unsaved(what: &str, written: anyhow::Result<Option<String>>) -> Option<Unsaved> {
    match written {
        Ok(None) => None,
        Ok(Some(why)) => Some((
            "Settings not confirmed".into(),
            format!(
                "LimarePDF saved {what}, but your system couldn't confirm the file is safely on \
                 disk ({why}). If the computer loses power, the latest changes may be lost."
            ),
        )),
        Err(e) => Some((
            "Settings not saved".into(),
            format!(
                "LimarePDF couldn't save {what} ({e:#}). What you change works until LimarePDF \
                 closes, but won't be kept for next time."
            ),
        )),
    }
}

/// How pages are laid out in the view.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Layout {
    /// One column, scrolling through the document.
    #[default]
    Continuous,
    /// Two pages side by side, scrolling through the document.
    Facing,
    /// One page at a time.
    Single,
}

impl Layout {
    fn name(self) -> &'static str {
        match self {
            Layout::Continuous => "continuous",
            Layout::Facing => "facing",
            Layout::Single => "single",
        }
    }

    /// The layout and cover page choice a document's preference asks for; `cover` is kept
    /// when the preference doesn't say.
    pub fn from_open(l: OpenLayout, cover: bool) -> (Self, bool) {
        match l {
            OpenLayout::Continuous => (Layout::Continuous, cover),
            OpenLayout::Single => (Layout::Single, cover),
            OpenLayout::Facing => (Layout::Facing, false),
            OpenLayout::FacingCover => (Layout::Facing, true),
        }
    }

    /// This layout as a document's preference.
    pub fn to_open(self, cover: bool) -> OpenLayout {
        match self {
            Layout::Continuous => OpenLayout::Continuous,
            Layout::Single => OpenLayout::Single,
            Layout::Facing if cover => OpenLayout::FacingCover,
            Layout::Facing => OpenLayout::Facing,
        }
    }
}

/// The accent colour: the desktop's where it says one, else LimarePDF's teal; or always
/// LimarePDF's.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Accent {
    #[default]
    Desktop,
    LimarePdf,
}

/// How pages are coloured on screen: a fixed mode, or Dark while the app's theme is dark.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum PageLook {
    Automatic,
    Fixed(PageColours),
}

impl Default for PageLook {
    fn default() -> Self {
        PageLook::Fixed(PageColours::Normal)
    }
}

impl PageLook {
    fn name(self) -> &'static str {
        match self {
            PageLook::Automatic => "automatic",
            PageLook::Fixed(PageColours::Normal) => "normal",
            PageLook::Fixed(PageColours::Dark) => "dark",
            PageLook::Fixed(PageColours::HighContrast) => "high_contrast",
            PageLook::Fixed(PageColours::Sepia) => "sepia",
        }
    }

    fn from_name(s: &str) -> Self {
        match s {
            "automatic" => PageLook::Automatic,
            "dark" => PageLook::Fixed(PageColours::Dark),
            "high_contrast" => PageLook::Fixed(PageColours::HighContrast),
            "sepia" => PageLook::Fixed(PageColours::Sepia),
            _ => PageLook::default(),
        }
    }

    /// The mode in force while the app's theme is dark or light.
    pub fn mode(self, dark_theme: bool) -> PageColours {
        match self {
            PageLook::Automatic if dark_theme => PageColours::Dark,
            PageLook::Automatic => PageColours::Normal,
            PageLook::Fixed(m) => m,
        }
    }
}

/// How many stamp pictures, and how many stamp wordings of your own, are remembered.
pub const STAMP_PICTURES: usize = 5;
pub const STAMP_WORDINGS: usize = 5;

/// The window's size in points when none is remembered, the smallest it can be, and the
/// largest a remembered size opens at (so a size kept on a big monitor stays usable on a
/// smaller one).
pub const WINDOW_DEFAULT: [f32; 2] = [1200.0, 850.0];
pub const WINDOW_MIN: [f32; 2] = [640.0, 420.0];
pub const WINDOW_MAX: [f32; 2] = [3840.0, 2160.0];
/// The zoom Organise's cards start at, and the smallest and largest (NEW-14).
pub const ORGANISE_ZOOM: f32 = 0.2;
pub const ORGANISE_MIN: f32 = 0.08;
pub const ORGANISE_MAX: f32 = 0.6;
/// The panel widths kept, in points: from the narrowest a panel goes
/// ([`crate::panels::NARROWEST`]) to half the widest window.
const PANEL_WIDTHS: std::ops::RangeInclusive<f32> = crate::panels::NARROWEST..=WINDOW_MAX[0] / 2.0;

/// A window size within [`WINDOW_MIN`] and [`WINDOW_MAX`], in whole points; `None` when it
/// isn't a real size (not a number, or under 100 points either way, as when minimized).
fn window_size([w, h]: [f32; 2]) -> Option<[f32; 2]> {
    let ok = |v: f32| v.is_finite() && v >= 100.0;
    (ok(w) && ok(h)).then(|| {
        [
            w.round().clamp(WINDOW_MIN[0], WINDOW_MAX[0]),
            h.round().clamp(WINDOW_MIN[1], WINDOW_MAX[1]),
        ]
    })
}

pub struct Settings {
    pub theme: ThemePreference,
    /// Page colours on screen, and whether pictures are recoloured too.
    pub page_look: PageLook,
    pub pictures_too: bool,
    pub layout: Layout,
    /// In the facing layout, the first page is shown alone on the right, as in a book.
    pub cover: bool,
    /// The colours text markup was last made in: indices into
    /// [`crate::markup::HIGHLIGHTS`] and [`crate::markup::LINES`].
    pub highlight: usize,
    pub line: usize,
    /// The name written into comments and markup; `None` until it is set, meaning the
    /// system user name. Empty writes no name.
    pub author: Option<String>,
    /// Text stamps get a line with the author's name and the date.
    pub stamp_name_date: bool,
    /// Pictures used as stamps, the latest first, at most [`STAMP_PICTURES`].
    pub stamp_pictures: Vec<PathBuf>,
    /// Stamp wordings of your own and their colours, the latest first, at most
    /// [`STAMP_WORDINGS`].
    pub stamp_wordings: Vec<(String, StampColor)>,
    /// Saved signatures and initials, kept in their own file beside this one.
    pub sign: SignKit,
    /// Keep recovery copies of documents with unsaved changes (on unless turned off).
    pub recovery: bool,
    /// Whether the user wants the navigation panel and the inspector open (both on
    /// unless closed). A narrow window may fold them away for now without changing this.
    pub navigation_panel: bool,
    pub inspector_panel: bool,
    /// The widths the user gave the inspector and the side panel's views by moving their
    /// edges, in points, by the app's id for the panel ("inspector", "side-comments"); the
    /// others start at the workspace's widths (ENH-08).
    pub panel_widths: BTreeMap<String, f32>,
    /// The selection's controls are in the inspector instead of a bar floating by the
    /// selection.
    pub controls_docked: bool,
    /// The placing tools pinned to stay active after putting something down.
    pub keep_tools: Vec<Tool>,
    /// Form fields are tinted, and required ones outlined (on unless turned off).
    pub fields_shown: bool,
    /// Pages are called by the document's page labels, where it has them, rather than by
    /// their numbers (on unless turned off).
    pub page_labels: bool,
    /// Features planned but not built show, greyed, where they will be (ENH-41; on unless
    /// turned off).
    pub show_planned: bool,
    /// The band's commands on one row, as icons (their names on hover); off unless chosen.
    pub compact_commands: bool,
    /// The zoom Organise shows the pages at, as cards (NEW-14).
    pub organise_zoom: f32,
    pub accent: Accent,
    /// The window's size in points when it isn't maximized, and whether it was maximized,
    /// when the app last closed.
    pub window: [f32; 2],
    pub window_maximized: bool,
    /// Where they are saved; `None` when there is no settings folder.
    file: Option<PathBuf>,
    /// What couldn't be saved last, until the app says it ([`Settings::take_unsaved`]).
    unsaved: Option<Unsaved>,
    /// A signature or initials saved for next time that couldn't be, until the app says
    /// it ([`Settings::take_sign_unsaved`]).
    sign_unsaved: Option<String>,
}

/// The name of the user logged in, or empty.
fn system_user() -> String {
    ["USER", "USERNAME", "LOGNAME"]
        .iter()
        .find_map(|k| {
            let v = std::env::var(k).ok()?;
            let v = v.trim();
            (!v.is_empty()).then(|| v.to_string())
        })
        .unwrap_or_default()
}

fn theme_name(t: ThemePreference) -> &'static str {
    match t {
        ThemePreference::System => "system",
        ThemePreference::Light => "light",
        ThemePreference::Dark => "dark",
    }
}

impl Settings {
    /// The settings saved by earlier runs.
    pub fn load() -> Self {
        Self::from_file(settings_dir().map(|d| d.join("settings.txt")))
    }

    /// The settings kept in `file` (read now, written by [`Settings::save`]).
    pub fn from_file(file: Option<PathBuf>) -> Self {
        let mut s = Self {
            theme: ThemePreference::System,
            page_look: PageLook::default(),
            pictures_too: false,
            layout: Layout::Continuous,
            cover: true,
            highlight: 0,
            line: 0,
            author: None,
            stamp_name_date: true,
            stamp_pictures: Vec::new(),
            stamp_wordings: Vec::new(),
            sign: SignKit::from_file(
                file.as_ref()
                    .and_then(|f| f.parent())
                    .map(|d| d.join("signatures.txt")),
            ),
            recovery: true,
            navigation_panel: true,
            inspector_panel: true,
            panel_widths: BTreeMap::new(),
            controls_docked: false,
            keep_tools: Vec::new(),
            fields_shown: true,
            page_labels: true,
            show_planned: true,
            compact_commands: false,
            organise_zoom: ORGANISE_ZOOM,
            accent: Accent::Desktop,
            window: WINDOW_DEFAULT,
            window_maximized: false,
            file,
            unsaved: None,
            sign_unsaved: None,
        };
        let text = s
            .file
            .as_ref()
            .and_then(|f| std::fs::read_to_string(f).ok())
            .unwrap_or_default();
        for line in text.lines() {
            let Some((key, value)) = line.split_once('=') else {
                continue;
            };
            let value = value.trim();
            match key.trim() {
                "theme" => {
                    s.theme = match value {
                        "light" => ThemePreference::Light,
                        "dark" => ThemePreference::Dark,
                        _ => ThemePreference::System,
                    };
                }
                "page_colours" => s.page_look = PageLook::from_name(value),
                "recolour_pictures" => s.pictures_too = value == "true",
                "page_layout" => {
                    s.layout = match value {
                        "facing" => Layout::Facing,
                        "single" => Layout::Single,
                        _ => Layout::Continuous,
                    };
                }
                "cover_page_alone" => s.cover = value != "false",
                "highlight_colour" => {
                    s.highlight = value
                        .parse()
                        .ok()
                        .filter(|&i| i < crate::markup::HIGHLIGHTS.len())
                        .unwrap_or(0);
                }
                "line_colour" => {
                    s.line = value
                        .parse()
                        .ok()
                        .filter(|&i| i < crate::markup::LINES.len())
                        .unwrap_or(0);
                }
                "comment_name" => s.author = Some(value.to_string()),
                "stamp_name_and_date" => s.stamp_name_date = value != "false",
                "recovery_copies" => s.recovery = value != "false",
                "navigation_panel" => s.navigation_panel = value != "closed",
                "inspector_panel" => s.inspector_panel = value != "closed",
                "panel_width" => {
                    let width = value.split_once(' ').and_then(|(panel, w)| {
                        let w = w.trim().parse::<f32>().ok()?;
                        (PANEL_WIDTHS.contains(&w)).then(|| (panel.to_string(), w.round()))
                    });
                    s.panel_widths.extend(width);
                }
                "selection_controls" => s.controls_docked = value == "docked",
                "keep_tool_active" => {
                    s.keep_tools.clear();
                    for t in value.split(',').filter_map(|t| Tool::from_key(t.trim())) {
                        if !s.keep_tools.contains(&t) {
                            s.keep_tools.push(t);
                        }
                    }
                }
                "form_fields" => s.fields_shown = value != "plain",
                "page_names" => s.page_labels = value != "numbers",
                "planned_features" => s.show_planned = value != "hidden",
                "commands" => s.compact_commands = value == "compact",
                "organise_size" => {
                    if let Ok(percent) = value.parse::<f32>() {
                        let zoom = percent / 100.0;
                        s.organise_zoom = zoom.clamp(ORGANISE_MIN, ORGANISE_MAX);
                    }
                }
                "accent_colour" => {
                    s.accent = match value {
                        "limarepdf" => Accent::LimarePdf,
                        _ => Accent::Desktop,
                    }
                }
                "window" => {
                    let size = value.split_once('x').and_then(|(w, h)| {
                        window_size([w.trim().parse().ok()?, h.trim().parse().ok()?])
                    });
                    if let Some(size) = size {
                        s.window = size;
                    }
                }
                "window_maximized" => s.window_maximized = value == "true",
                "stamp_picture" if !value.is_empty() && s.stamp_pictures.len() < STAMP_PICTURES => {
                    s.stamp_pictures.push(PathBuf::from(value));
                }
                "stamp_wording" if s.stamp_wordings.len() < STAMP_WORDINGS => {
                    let (color, text) = value.split_once(' ').unwrap_or((value, ""));
                    if let (Some(color), Ok(text)) = (
                        StampColor::from_name(color),
                        limarepdf_core::stamp_wording(text),
                    ) {
                        s.stamp_wordings.push((text, color));
                    }
                }
                _ => {}
            }
        }
        s
    }

    /// The name to write into new comments and markup; may be empty.
    pub fn author(&self) -> String {
        self.author.clone().unwrap_or_else(system_user)
    }

    /// Keeps the window's current size (in points, if known) and whether it is maximized,
    /// for [`Settings::save`]. While maximized, the size it had before is kept.
    pub fn keep_window(&mut self, size: Option<[f32; 2]>, maximized: bool) {
        self.window_maximized = maximized;
        if let Some(size) = size.filter(|_| !maximized).and_then(window_size) {
            self.window = size;
        }
    }

    /// Keeps the width the user gave panel `panel` (in points, whole), and saves it when
    /// it changed.
    pub fn keep_width(&mut self, panel: &str, width: f32) {
        let width = width.round();
        if !PANEL_WIDTHS.contains(&width) || self.panel_widths.get(panel) == Some(&width) {
            return;
        }
        self.panel_widths.insert(panel.to_string(), width);
        self.save();
    }

    /// What couldn't be saved last, said once taken.
    pub fn take_unsaved(&mut self) -> Option<Unsaved> {
        self.unsaved.take()
    }

    /// A signature or initials saved for next time that couldn't be, said once taken.
    pub fn take_sign_unsaved(&mut self) -> Option<String> {
        self.sign_unsaved.take()
    }

    /// Saves the signatures and initials ([`SignKit::save`]). `kept`: one was just saved
    /// for next time, so its failing is said each time it happens.
    pub fn save_sign(&mut self, kept: bool) {
        match self.sign.save() {
            Err(e) if kept => {
                self.sign_unsaved = Some(format!(
                    "What you saved for next time can be used until LimarePDF closes, but \
                     LimarePDF couldn't save it ({e:#})."
                ));
            }
            written => {
                if let Some(u) = unsaved("your saved signatures", written) {
                    self.unsaved = Some(u);
                }
            }
        }
    }

    pub fn save(&mut self) {
        let Some(file) = &self.file else { return };
        let mut text = format!(
            "theme = {}\npage_colours = {}\nrecolour_pictures = {}\npage_layout = {}\n\
             cover_page_alone = {}\nhighlight_colour = {}\nline_colour = {}\n",
            theme_name(self.theme),
            self.page_look.name(),
            self.pictures_too,
            self.layout.name(),
            self.cover,
            self.highlight,
            self.line
        );
        if let Some(name) = &self.author {
            // One line: a name pasted with line breaks keeps its first line.
            let name = name.lines().next().unwrap_or("").trim();
            text += &format!("comment_name = {name}\n");
        }
        text += &format!("stamp_name_and_date = {}\n", self.stamp_name_date);
        text += &format!("recovery_copies = {}\n", self.recovery);
        let open = |o: bool| if o { "open" } else { "closed" };
        text += &format!(
            "navigation_panel = {}\ninspector_panel = {}\nselection_controls = {}\n",
            open(self.navigation_panel),
            open(self.inspector_panel),
            if self.controls_docked {
                "docked"
            } else {
                "floating"
            }
        );
        let keep: Vec<&str> = self.keep_tools.iter().map(|t| t.key()).collect();
        text += &format!(
            "keep_tool_active = {}\nform_fields = {}\npage_names = {}\n",
            keep.join(", "),
            if self.fields_shown {
                "highlighted"
            } else {
                "plain"
            },
            if self.page_labels {
                "labels"
            } else {
                "numbers"
            }
        );
        let planned = if self.show_planned { "shown" } else { "hidden" };
        text += &format!("planned_features = {planned}\n");
        let commands = if self.compact_commands {
            "compact"
        } else {
            "full"
        };
        let accent = match self.accent {
            Accent::Desktop => "desktop",
            Accent::LimarePdf => "limarepdf",
        };
        text += &format!("commands = {commands}\naccent_colour = {accent}\n");
        text += &format!("organise_size = {:.0}\n", self.organise_zoom * 100.0);
        text += &format!(
            "window = {} x {}\nwindow_maximized = {}\n",
            self.window[0], self.window[1], self.window_maximized
        );
        for (panel, width) in &self.panel_widths {
            text += &format!("panel_width = {panel} {width}\n");
        }
        for p in &self.stamp_pictures {
            text += &format!("stamp_picture = {}\n", p.display());
        }
        for (wording, color) in &self.stamp_wordings {
            text += &format!("stamp_wording = {} {wording}\n", color.name());
        }
        if let Some(u) = unsaved("its settings", write_own(file, text.as_bytes())) {
            self.unsaved = Some(u);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("limarepdf-settings-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir.join("settings.txt")
    }

    /// Saved through the safe writer (R04): the file is replaced in one step, not written
    /// into, so another name for the old file (a hard link) keeps the old contents.
    #[test]
    fn saved_in_one_step() {
        let f = file("one-step");
        std::fs::create_dir_all(f.parent().unwrap()).unwrap();
        std::fs::write(&f, "theme = dark\n").unwrap();
        let other = f.with_file_name("other.txt");
        std::fs::hard_link(&f, &other).unwrap();
        Settings::from_file(Some(f.clone())).save();
        assert_eq!(std::fs::read_to_string(&other).unwrap(), "theme = dark\n");
        assert!(std::fs::read_to_string(&f)
            .unwrap()
            .starts_with("theme = dark\npage_colours = "));
        std::fs::remove_dir_all(f.parent().unwrap()).unwrap();
    }

    #[test]
    fn theme_is_saved_and_read_back() {
        use ThemePreference::{Dark, Light, System};
        let f = file("theme");
        assert_eq!(Settings::from_file(Some(f.clone())).theme, System);
        for t in [Dark, Light, System] {
            let mut s = Settings::from_file(Some(f.clone()));
            s.theme = t;
            s.save();
            assert_eq!(Settings::from_file(Some(f.clone())).theme, t);
        }
    }

    #[test]
    fn page_colours_are_saved_and_read_back() {
        let f = file("page-colours");
        let s = Settings::from_file(Some(f.clone()));
        assert_eq!(s.page_look, PageLook::Fixed(PageColours::Normal));
        assert!(!s.pictures_too);
        for look in [
            PageLook::Automatic,
            PageLook::Fixed(PageColours::Dark),
            PageLook::Fixed(PageColours::HighContrast),
            PageLook::Fixed(PageColours::Sepia),
            PageLook::Fixed(PageColours::Normal),
        ] {
            let mut s = Settings::from_file(Some(f.clone()));
            s.page_look = look;
            s.pictures_too = !s.pictures_too;
            let too = s.pictures_too;
            s.save();
            let back = Settings::from_file(Some(f.clone()));
            assert_eq!((back.page_look, back.pictures_too), (look, too));
        }
        assert_eq!(PageLook::Automatic.mode(true), PageColours::Dark);
        assert_eq!(PageLook::Automatic.mode(false), PageColours::Normal);
    }

    #[test]
    fn markup_colours_and_name_are_saved_and_read_back() {
        let f = file("markup");
        let s = Settings::from_file(Some(f.clone()));
        assert_eq!((s.highlight, s.line, s.author.as_deref()), (0, 0, None));
        let mut s = Settings::from_file(Some(f.clone()));
        s.highlight = 2;
        s.line = 1;
        s.author = Some(String::new());
        s.save();
        let s = Settings::from_file(Some(f.clone()));
        assert_eq!((s.highlight, s.line), (2, 1));
        assert_eq!(s.author(), "", "an empty name stays empty");
        std::fs::write(&f, "highlight_colour = 99\ncomment_name = Sam Lee\n").unwrap();
        let s = Settings::from_file(Some(f));
        assert_eq!(s.highlight, 0, "out of range falls back to the first");
        assert_eq!(s.author(), "Sam Lee");
    }

    #[test]
    fn page_layout_is_saved_and_read_back() {
        let f = file("layout");
        let s = Settings::from_file(Some(f.clone()));
        assert_eq!((s.layout, s.cover), (Layout::Continuous, true));
        for (layout, cover) in [(Layout::Facing, false), (Layout::Single, true)] {
            let mut s = Settings::from_file(Some(f.clone()));
            s.layout = layout;
            s.cover = cover;
            s.save();
            let s = Settings::from_file(Some(f.clone()));
            assert_eq!((s.layout, s.cover), (layout, cover));
        }
    }

    #[test]
    fn stamp_choices_are_saved_and_read_back() {
        let f = file("stamps");
        let s = Settings::from_file(Some(f.clone()));
        assert!(s.stamp_name_date && s.stamp_pictures.is_empty());
        let mut s = Settings::from_file(Some(f.clone()));
        s.stamp_name_date = false;
        s.stamp_pictures = vec!["/a/b c.png".into(), "/d.jpg".into()];
        s.stamp_wordings = vec![
            ("PAID IN FULL".into(), StampColor::Green),
            ("COPY".into(), StampColor::Blue),
        ];
        s.save();
        let s = Settings::from_file(Some(f));
        assert!(!s.stamp_name_date);
        assert_eq!(
            s.stamp_pictures,
            [PathBuf::from("/a/b c.png"), "/d.jpg".into()]
        );
        assert_eq!(
            s.stamp_wordings,
            [
                ("PAID IN FULL".into(), StampColor::Green),
                ("COPY".into(), StampColor::Blue)
            ]
        );
    }

    #[test]
    fn panels_and_docking_are_remembered() {
        let f = file("panels");
        let s = Settings::from_file(Some(f.clone()));
        assert!(s.navigation_panel && s.inspector_panel && !s.controls_docked);
        let mut s = s;
        (s.navigation_panel, s.controls_docked) = (false, true);
        s.save();
        let s = Settings::from_file(Some(f));
        assert!(!s.navigation_panel && s.inspector_panel && s.controls_docked);
    }

    /// ENH-36: page labels are used unless turned off, and the choice is kept.
    #[test]
    fn page_numbers_or_labels_are_remembered() {
        let f = file("page names");
        let mut s = Settings::from_file(Some(f.clone()));
        assert!(s.page_labels);
        s.page_labels = false;
        s.save();
        assert!(!Settings::from_file(Some(f.clone())).page_labels);
        std::fs::write(&f, "page_names = walrus\n").unwrap();
        assert!(Settings::from_file(Some(f)).page_labels);
    }

    /// ENH-41: planned features show unless turned off, and the choice is kept.
    #[test]
    fn planned_features_shown_or_hidden_are_remembered() {
        let f = file("planned");
        let mut s = Settings::from_file(Some(f.clone()));
        assert!(s.show_planned);
        s.show_planned = false;
        s.save();
        assert!(!Settings::from_file(Some(f.clone())).show_planned);
        std::fs::write(&f, "planned_features = walrus\n").unwrap();
        assert!(Settings::from_file(Some(f)).show_planned);
    }

    /// The band's commands full unless made compact, and the desktop's accent unless
    /// LimarePDF's is chosen; both kept.
    #[test]
    fn compact_commands_and_the_accent_are_remembered() {
        let f = file("commands");
        let mut s = Settings::from_file(Some(f.clone()));
        assert!(!s.compact_commands);
        assert_eq!(s.accent, Accent::Desktop);
        (s.compact_commands, s.accent) = (true, Accent::LimarePdf);
        s.save();
        let s = Settings::from_file(Some(f.clone()));
        assert!(s.compact_commands);
        assert_eq!(s.accent, Accent::LimarePdf);
        std::fs::write(&f, "commands = walrus\naccent_colour = walrus\n").unwrap();
        let s = Settings::from_file(Some(f));
        assert!(!s.compact_commands);
        assert_eq!(s.accent, Accent::Desktop);
    }

    /// NEW-14: the size of Organise's cards is kept, within its range; nonsense gives the
    /// default.
    #[test]
    fn the_organise_size_is_remembered() {
        let f = file("organise");
        let mut s = Settings::from_file(Some(f.clone()));
        assert_eq!(s.organise_zoom, ORGANISE_ZOOM);
        s.organise_zoom = 0.31;
        s.save();
        let s = Settings::from_file(Some(f.clone()));
        assert_eq!(s.organise_zoom, 0.31);
        std::fs::write(&f, "organise_size = 900\n").unwrap();
        let s = Settings::from_file(Some(f.clone()));
        assert_eq!(s.organise_zoom, ORGANISE_MAX);
        std::fs::write(&f, "organise_size = walrus\n").unwrap();
        let s = Settings::from_file(Some(f));
        assert_eq!(s.organise_zoom, ORGANISE_ZOOM);
    }

    #[test]
    fn pinned_tools_and_field_highlights_are_remembered() {
        let f = file("pins");
        let s = Settings::from_file(Some(f.clone()));
        assert!(s.keep_tools.is_empty() && s.fields_shown);
        let mut s = s;
        s.keep_tools = vec![Tool::Stamp, Tool::Redact];
        s.fields_shown = false;
        s.save();
        let s = Settings::from_file(Some(f.clone()));
        assert_eq!(s.keep_tools, [Tool::Stamp, Tool::Redact]);
        assert!(!s.fields_shown);
        // Unknown names, tools that can't be pinned (the Note tool no longer can) and
        // repeats are left out.
        let line = "keep_tool_active = stamp, walrus, , stamp, pen, note, line\n";
        std::fs::write(&f, line).unwrap();
        let s = Settings::from_file(Some(f.clone()));
        assert_eq!(s.keep_tools, [Tool::Stamp, Tool::Line]);
        assert!(s.fields_shown);
        // Every placing tool has a name of its own.
        for t in Tool::PLACING {
            assert_eq!(Tool::from_key(t.key()), Some(t));
        }
    }

    #[test]
    fn recovery_copies_are_on_unless_turned_off() {
        let f = file("recovery");
        assert!(Settings::from_file(Some(f.clone())).recovery);
        let mut s = Settings::from_file(Some(f.clone()));
        s.recovery = false;
        s.save();
        assert!(!Settings::from_file(Some(f.clone())).recovery);
        std::fs::write(&f, "recovery_copies = maybe\n").unwrap();
        assert!(Settings::from_file(Some(f)).recovery);
    }

    #[test]
    fn window_size_is_saved_and_read_back() {
        let f = file("window");
        let s = Settings::from_file(Some(f.clone()));
        assert_eq!((s.window, s.window_maximized), (WINDOW_DEFAULT, false));
        let mut s = Settings::from_file(Some(f.clone()));
        s.keep_window(Some([1400.4, 900.0]), false);
        s.save();
        let s = Settings::from_file(Some(f.clone()));
        assert_eq!((s.window, s.window_maximized), ([1400.0, 900.0], false));
        // While maximized, the size from before is kept, so un-maximizing gives it back.
        let mut s = Settings::from_file(Some(f.clone()));
        s.keep_window(Some([2560.0, 1400.0]), true);
        s.save();
        let s = Settings::from_file(Some(f.clone()));
        assert_eq!((s.window, s.window_maximized), ([1400.0, 900.0], true));
        // A window too small to be real (minimized, say) leaves the size as it was.
        let mut s = Settings::from_file(Some(f.clone()));
        s.keep_window(Some([0.0, 0.0]), false);
        s.keep_window(None, false);
        assert_eq!((s.window, s.window_maximized), ([1400.0, 900.0], false));
    }

    /// ENH-08 (batch 39): the widths given to the inspector and the side panel's views are
    /// saved and read back, each by its panel; widths that aren't real are left out.
    #[test]
    fn panel_widths_are_saved_and_read_back() {
        let f = file("panel-widths");
        let s = Settings::from_file(Some(f.clone()));
        assert!(s.panel_widths.is_empty());
        let mut s = Settings::from_file(Some(f.clone()));
        s.keep_width("inspector", 312.4);
        s.keep_width("side-comments", 260.0);
        let s = Settings::from_file(Some(f.clone()));
        let widths: Vec<(&str, f32)> = (s.panel_widths.iter())
            .map(|(k, v)| (k.as_str(), *v))
            .collect();
        assert_eq!(widths, [("inspector", 312.0), ("side-comments", 260.0)]);
        let lines = "panel_width = side 12\npanel_width = side-search wide\n\
                     panel_width = inspector NaN\npanel_width = side-bookmarks 180\n";
        std::fs::write(&f, lines).unwrap();
        let s = Settings::from_file(Some(f));
        let widths: Vec<&str> = s.panel_widths.keys().map(String::as_str).collect();
        assert_eq!(widths, ["side-bookmarks"]);
    }

    #[test]
    fn bad_window_sizes_are_ignored_or_clamped() {
        let f = file("window-bad");
        std::fs::create_dir_all(f.parent().unwrap()).unwrap();
        for bad in [
            "window = big",
            "window = 1400",
            "window = 1400 x",
            "window = NaN x 900",
            "window = inf x 900",
            "window = 1400 x -900",
            "window = 10 x 10",
        ] {
            std::fs::write(&f, format!("{bad}\nwindow_maximized = maybe\n")).unwrap();
            let s = Settings::from_file(Some(f.clone()));
            assert_eq!(
                (s.window, s.window_maximized),
                (WINDOW_DEFAULT, false),
                "{bad}"
            );
        }
        std::fs::write(&f, "window = 300 x 100000\nwindow_maximized = true\n").unwrap();
        let s = Settings::from_file(Some(f.clone()));
        assert_eq!(s.window, [WINDOW_MIN[0], WINDOW_MAX[1]]);
        assert!(s.window_maximized);
        let mut s = Settings::from_file(Some(f.clone()));
        s.keep_window(Some([9000.0, 200.0]), false);
        assert_eq!(s.window, [WINDOW_MAX[0], WINDOW_MIN[1]]);
    }

    #[test]
    fn unknown_lines_are_ignored() {
        let f = file("junk");
        std::fs::create_dir_all(f.parent().unwrap()).unwrap();
        std::fs::write(&f, "colour = blue\nnot a setting\ntheme = dark\n").unwrap();
        assert_eq!(Settings::from_file(Some(f)).theme, ThemePreference::Dark);
    }
}

//! Settings kept between runs, in MPE's settings folder:
//!
//! - Linux and other Unix: `$XDG_CONFIG_HOME/mpe/`, else `~/.config/mpe/`
//! - macOS: `~/Library/Application Support/mpe/`
//! - Windows: `%APPDATA%\mpe\`
//!
//! `settings.txt` holds `key = value` lines; unknown keys and bad values are ignored. The
//! files stay on this computer, and failing to read or write them is not an error.

use eframe::egui::ThemePreference;
use std::path::PathBuf;

/// The settings folder for MPE, from the environment.
pub fn settings_dir() -> Option<PathBuf> {
    let env = |k: &str| {
        let v = std::env::var_os(k)?;
        (!v.is_empty()).then(|| PathBuf::from(v))
    };
    let base = if cfg!(windows) {
        env("APPDATA")?
    } else if cfg!(target_os = "macos") {
        env("HOME")?.join("Library/Application Support")
    } else {
        env("XDG_CONFIG_HOME").or_else(|| Some(env("HOME")?.join(".config")))?
    };
    Some(base.join("mpe"))
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
}

pub struct Settings {
    pub theme: ThemePreference,
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
    /// Where they are saved; `None` when there is no settings folder.
    file: Option<PathBuf>,
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
            layout: Layout::Continuous,
            cover: true,
            highlight: 0,
            line: 0,
            author: None,
            file,
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
                _ => {}
            }
        }
        s
    }

    /// The name to write into new comments and markup; may be empty.
    pub fn author(&self) -> String {
        self.author.clone().unwrap_or_else(system_user)
    }

    pub fn save(&self) {
        let Some(file) = &self.file else { return };
        if let Some(dir) = file.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let mut text = format!(
            "theme = {}\npage_layout = {}\ncover_page_alone = {}\nhighlight_colour = {}\n\
             line_colour = {}\n",
            theme_name(self.theme),
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
        let _ = std::fs::write(file, text);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("mpe-settings-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir.join("settings.txt")
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
    fn unknown_lines_are_ignored() {
        let f = file("junk");
        std::fs::create_dir_all(f.parent().unwrap()).unwrap();
        std::fs::write(&f, "colour = blue\nnot a setting\ntheme = dark\n").unwrap();
        assert_eq!(Settings::from_file(Some(f)).theme, ThemePreference::Dark);
    }
}

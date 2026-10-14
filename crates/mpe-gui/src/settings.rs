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

pub struct Settings {
    pub theme: ThemePreference,
    /// Where they are saved; `None` when there is no settings folder.
    file: Option<PathBuf>,
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
            if key.trim() == "theme" {
                s.theme = match value.trim() {
                    "light" => ThemePreference::Light,
                    "dark" => ThemePreference::Dark,
                    _ => ThemePreference::System,
                };
            }
        }
        s
    }

    pub fn save(&self) {
        let Some(file) = &self.file else { return };
        if let Some(dir) = file.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let _ = std::fs::write(file, format!("theme = {}\n", theme_name(self.theme)));
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
    fn unknown_lines_are_ignored() {
        let f = file("junk");
        std::fs::create_dir_all(f.parent().unwrap()).unwrap();
        std::fs::write(&f, "colour = blue\nnot a setting\ntheme = dark\n").unwrap();
        assert_eq!(Settings::from_file(Some(f)).theme, ThemePreference::Dark);
    }
}

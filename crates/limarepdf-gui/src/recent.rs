//! Recently opened files, kept between runs in `recent.txt` (one path per line) in LimarePDF's
//! settings folder (see [`crate::settings`]).
//!
//! The list stays on this computer. Failing to read it is not an error: the list is a
//! convenience. Failing to write it is said, as for the settings.

use crate::settings::{settings_dir, unsaved, write_own, Unsaved};
use std::path::{Path, PathBuf};

/// How many files the list keeps.
const MAX: usize = 10;

pub struct Recent {
    paths: Vec<PathBuf>,
    /// Where the list is saved; `None` when there is no settings folder.
    file: Option<PathBuf>,
    /// What couldn't be saved last, until the app says it ([`Recent::take_unsaved`]).
    unsaved: Option<Unsaved>,
}

impl Recent {
    /// The list saved by earlier runs.
    pub fn load() -> Self {
        Self::from_file(settings_dir().map(|d| d.join("recent.txt")))
    }

    /// The list kept in `file` (read now, written on every change).
    pub fn from_file(file: Option<PathBuf>) -> Self {
        let text = file
            .as_ref()
            .and_then(|f| std::fs::read_to_string(f).ok())
            .unwrap_or_default();
        let paths = text
            .lines()
            .map(str::trim)
            .filter(|l| !l.is_empty())
            .map(PathBuf::from)
            .take(MAX)
            .collect();
        Self {
            paths,
            file,
            unsaved: None,
        }
    }

    /// Most recent first, whether or not the files still exist.
    #[cfg(test)]
    fn paths(&self) -> &[PathBuf] {
        &self.paths
    }

    /// The files that still exist, most recent first.
    pub fn existing(&self) -> Vec<&PathBuf> {
        self.paths.iter().filter(|p| p.is_file()).collect()
    }

    /// Put `path` first. Paths that are not valid Unicode are not kept, since the list
    /// is a text file.
    pub fn add(&mut self, path: &Path) {
        let path = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
        if path.to_str().is_none() {
            return;
        }
        self.paths.retain(|p| p != &path);
        self.paths.insert(0, path);
        self.paths.truncate(MAX);
        self.save();
    }

    pub fn clear(&mut self) {
        self.paths.clear();
        self.save();
    }

    /// What couldn't be saved last, said once taken.
    pub fn take_unsaved(&mut self) -> Option<Unsaved> {
        self.unsaved.take()
    }

    fn save(&mut self) {
        let Some(file) = &self.file else { return };
        let text: String = self
            .paths
            .iter()
            .filter_map(|p| p.to_str())
            .map(|p| format!("{p}\n"))
            .collect();
        let written = write_own(file, text.as_bytes());
        if let Some(u) = unsaved("its list of recent files", written) {
            self.unsaved = Some(u);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A list saved in a fresh file under the system's temporary folder.
    fn list(name: &str) -> (Recent, PathBuf) {
        let dir =
            std::env::temp_dir().join(format!("limarepdf-recent-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let file = dir.join("recent.txt");
        (Recent::from_file(Some(file.clone())), file)
    }

    /// Saved through the safe writer (R04): the file is replaced in one step, not written
    /// into, so another name for the old file (a hard link) keeps the old contents.
    #[test]
    fn saved_in_one_step() {
        let (_, file) = list("one-step");
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(&file, "/a.pdf\n").unwrap();
        let other = file.with_file_name("other.txt");
        std::fs::hard_link(&file, &other).unwrap();
        let mut r = Recent::from_file(Some(file.clone()));
        r.add(Path::new("/b.pdf"));
        assert_eq!(std::fs::read_to_string(&other).unwrap(), "/a.pdf\n");
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "/b.pdf\n/a.pdf\n");
        std::fs::remove_dir_all(file.parent().unwrap()).unwrap();
    }

    /// BUG-14: a list that can't be written is kept for this session, and why is held for
    /// the app to say.
    #[test]
    fn a_list_that_can_t_be_written_is_said() {
        let (_, file) = list("unwritable");
        let dir = file.parent().unwrap();
        std::fs::create_dir_all(dir).unwrap();
        // A file where the settings folder would be.
        std::fs::write(dir.join("blocker"), "a file, not a folder").unwrap();
        let mut r = Recent::from_file(Some(dir.join("blocker").join("recent.txt")));
        r.add(Path::new("/a.pdf"));
        assert_eq!(r.paths(), [PathBuf::from("/a.pdf")]);
        let (title, text) = r.take_unsaved().expect("not said");
        assert_eq!(title, "Settings not saved");
        assert!(
            text.starts_with("LimarePDF couldn't save its list of recent files (making "),
            "{text}"
        );
        assert!(r.take_unsaved().is_none(), "said twice");
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn newest_first_without_duplicates() {
        let (mut r, file) = list("order");
        for p in ["/a.pdf", "/b.pdf", "/a.pdf"] {
            r.add(Path::new(p));
        }
        let want = [PathBuf::from("/a.pdf"), PathBuf::from("/b.pdf")];
        assert_eq!(r.paths(), want);
        // Saved, and read back by the next run.
        assert_eq!(Recent::from_file(Some(file)).paths(), want);
    }

    #[test]
    fn keeps_at_most_ten() {
        let (mut r, _) = list("max");
        for i in 0..15 {
            r.add(Path::new(&format!("/f{i}.pdf")));
        }
        assert_eq!(r.paths().len(), MAX);
        assert_eq!(r.paths()[0], PathBuf::from("/f14.pdf"));
    }

    #[test]
    fn clear_empties_the_saved_list() {
        let (mut r, file) = list("clear");
        r.add(Path::new("/a.pdf"));
        r.clear();
        assert!(Recent::from_file(Some(file)).paths().is_empty());
    }

    #[test]
    fn only_existing_files_are_offered() {
        let (mut r, _) = list("exists");
        let real = std::env::current_exe().unwrap();
        r.add(Path::new("/no/such/file.pdf"));
        r.add(&real);
        let shown = r.existing();
        assert_eq!(shown.len(), 1);
        assert!(shown[0].ends_with(real.file_name().unwrap()));
    }
}

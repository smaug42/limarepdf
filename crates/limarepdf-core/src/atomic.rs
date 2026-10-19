//! Replacing a file in one step: the new contents go to a temporary file beside it, which is
//! then renamed over it, so a failure part way leaves any existing file as it was.
//!
//! The temporary file is always created new (`create_new`, which fails when anything is at
//! the name, a symbolic link included, and never follows one), under a fresh name each time.
//! So a stale or planted file at a name is never opened, truncated, given new permissions or
//! removed; another name is tried instead. On Unix the temporary file is for this user only
//! (mode 600) from the moment it exists whenever its contents may be private: when it
//! replaces a file, and always for [`Access::Private`].

use crate::Result;
use anyhow::{anyhow, bail, Context};
use std::ffi::{OsStr, OsString};
use std::fs::File;
use std::io::{BufWriter, ErrorKind};
use std::path::{Path, PathBuf};

/// How many names are tried, each found taken, before giving up.
const TRIES: usize = 16;

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Access {
    /// A document: a symbolic link is followed, and an existing file keeps its permissions. A
    /// new file gets the default ones.
    Keep,
    /// For this user only (mode 600 on Unix). A symbolic link at `path` is replaced, not
    /// followed.
    Private,
}

/// Writes a file through `write` to a temporary file beside `path`, then renames it over
/// `path`. On failure only the temporary file this call created is removed.
pub(crate) fn replace(
    path: &Path,
    access: Access,
    write: impl FnOnce(&mut BufWriter<File>) -> Result<()>,
) -> Result<()> {
    let path = match access {
        Access::Keep => std::fs::canonicalize(path).unwrap_or_else(|_| path.into()),
        Access::Private => path.into(),
    };
    let name = path.file_name().context("no file name to save to")?;
    let old = match access {
        Access::Keep => std::fs::metadata(&path).ok().map(|m| m.permissions()),
        Access::Private => None,
    };
    let (tmp, file) = create(&path, name, access == Access::Private || old.is_some())?;
    let written = (|| -> Result<()> {
        #[cfg(unix)]
        if access == Access::Private {
            // The umask may have taken away this user's own read access.
            use std::os::unix::fs::PermissionsExt;
            file.set_permissions(std::fs::Permissions::from_mode(0o600))?;
        }
        #[cfg(test)]
        tests::observe(&tmp);
        let mut w = BufWriter::with_capacity(1 << 20, file);
        write(&mut w)?;
        let f = w.into_inner().map_err(|e| e.into_error())?;
        f.sync_all()?;
        #[cfg(test)]
        tests::observe(&tmp);
        if let Some(p) = old {
            f.set_permissions(p)?;
        }
        drop(f);
        std::fs::rename(&tmp, &path).with_context(|| format!("replacing {}", path.display()))?;
        Ok(())
    })();
    if written.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    written
}

/// Creates a new, empty temporary file beside `path` for this user only, under a fresh name,
/// for a caller that writes, syncs and renames it itself.
pub(crate) fn create_private(path: &Path) -> Result<(PathBuf, File)> {
    let name = path.file_name().context("no file name to write")?;
    let (tmp, file) = create(path, name, true)?;
    #[cfg(unix)]
    {
        // The umask may have taken away this user's own read access.
        use std::os::unix::fs::PermissionsExt;
        if let Err(e) = file.set_permissions(std::fs::Permissions::from_mode(0o600)) {
            drop(file);
            let _ = std::fs::remove_file(&tmp);
            return Err(e.into());
        }
    }
    Ok((tmp, file))
}

/// Creates a new temporary file beside `path`, private to this user on Unix when `private`.
fn create(path: &Path, name: &OsStr, private: bool) -> Result<(PathBuf, File)> {
    #[cfg(not(unix))]
    let _ = private;
    for _ in 0..TRIES {
        let mut n = OsString::from(".");
        n.push(name);
        n.push(format!(".limarepdf-{}.tmp", suffix()));
        let tmp = path.with_file_name(n);
        let mut o = std::fs::OpenOptions::new();
        o.write(true).create_new(true);
        #[cfg(unix)]
        if private {
            use std::os::unix::fs::OpenOptionsExt;
            o.mode(0o600);
        }
        match o.open(&tmp) {
            Ok(f) => return Ok((tmp, f)),
            Err(e) if e.kind() == ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(anyhow!(e).context(format!("creating {}", tmp.display()))),
        }
    }
    bail!(
        "couldn't make a temporary file beside {}: all {TRIES} names tried were taken",
        path.display()
    )
}

/// A fresh part for a temporary file's name: the process, a count and a random number (the
/// time when the system's random source fails). Only exclusive creation makes the temporary
/// file safe; the name just makes collisions rare.
fn suffix() -> String {
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNT: AtomicU64 = AtomicU64::new(0);
    #[cfg(test)]
    if let Some(s) = tests::next_suffix() {
        return s;
    }
    let count = COUNT.fetch_add(1, Ordering::Relaxed);
    let random = crate::pkcs12::random::<8>()
        .map(u64::from_ne_bytes)
        .unwrap_or_else(|_| {
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |d| d.as_nanos() as u64)
        });
    format!("{}-{count}-{random:016x}", std::process::id())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::collections::VecDeque;
    use std::io::Write;

    thread_local! {
        static SUFFIXES: RefCell<VecDeque<String>> = const { RefCell::new(VecDeque::new()) };
        #[allow(clippy::type_complexity)]
        static OBSERVER: RefCell<Option<Box<dyn FnMut(&Path)>>> = const { RefCell::new(None) };
    }

    /// The next name part a test chose, if any are left.
    pub(super) fn next_suffix() -> Option<String> {
        SUFFIXES.with(|s| s.borrow_mut().pop_front())
    }

    /// Called with the temporary file before its first byte and after its last one.
    pub(super) fn observe(tmp: &Path) {
        OBSERVER.with(|o| {
            if let Some(f) = o.borrow_mut().as_mut() {
                f(tmp)
            }
        })
    }

    /// The names this thread's next temporary files try first, in order.
    fn choose(suffixes: &[&str]) {
        SUFFIXES.with(|s| *s.borrow_mut() = suffixes.iter().map(|s| s.to_string()).collect());
    }

    /// `f` is called with the temporary file at the two points [`observe`] marks, on this
    /// thread, until [`unwatch`].
    #[cfg_attr(not(unix), allow(dead_code))]
    fn watch(f: impl FnMut(&Path) + 'static) {
        OBSERVER.with(|o| *o.borrow_mut() = Some(Box::new(f)));
    }

    fn unwatch() {
        OBSERVER.with(|o| *o.borrow_mut() = None);
    }

    /// A fresh, empty folder for one test.
    fn folder(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("limarepdf-atomic-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// The temporary name a chosen suffix gives for `doc`.
    fn tmp_name(dir: &Path, doc: &str, suffix: &str) -> PathBuf {
        dir.join(format!(".{doc}.limarepdf-{suffix}.tmp"))
    }

    /// Every file in `dir` with its contents, sorted, symbolic links as their target.
    fn snapshot(dir: &Path) -> Vec<(PathBuf, String)> {
        let mut v: Vec<_> = std::fs::read_dir(dir)
            .unwrap()
            .map(|e| {
                let p = e.unwrap().path();
                let what = match std::fs::read_link(&p) {
                    Ok(t) => format!("link to {}", t.display()),
                    Err(_) => String::from_utf8_lossy(&std::fs::read(&p).unwrap()).into_owned(),
                };
                (p, what)
            })
            .collect();
        v.sort();
        v
    }

    #[cfg(unix)]
    fn mode(p: &Path) -> u32 {
        use std::os::unix::fs::PermissionsExt;
        std::fs::symlink_metadata(p).unwrap().permissions().mode() & 0o777
    }

    #[cfg(unix)]
    fn set_mode(p: &Path, m: u32) {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(p, std::fs::Permissions::from_mode(m)).unwrap();
    }

    /// Asserts every file in `dir` other than `skip` is for this user only, and that there
    /// is at least one.
    #[cfg(unix)]
    fn all_private(dir: &Path, skip: &[&Path]) {
        let mut seen = 0;
        for e in std::fs::read_dir(dir).unwrap() {
            let p = e.unwrap().path();
            if skip.contains(&p.as_path()) {
                continue;
            }
            assert_eq!(mode(&p) & 0o077, 0, "{} is readable by others", p.display());
            seen += 1;
        }
        assert!(seen > 0, "no temporary file was found");
    }

    /// A document's contents are never readable by others through the temporary file, from
    /// before its first byte to after its last.
    #[cfg(unix)]
    #[test]
    fn replacing_a_private_file_stays_private() {
        let dir = folder("private");
        let doc = dir.join("doc.pdf");
        std::fs::write(&doc, "old").unwrap();
        set_mode(&doc, 0o600);
        let (d, o) = (dir.clone(), doc.clone());
        replace(&doc, Access::Keep, |w| {
            all_private(&d, &[&o]);
            for _ in 0..1000 {
                w.write_all(&[b'x'; 1024])?;
            }
            w.flush()?;
            all_private(&d, &[&o]);
            Ok(())
        })
        .unwrap();
        assert_eq!(mode(&doc), 0o600);
        assert_eq!(std::fs::read(&doc).unwrap().len(), 1024 * 1000);
        assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 1);
        std::fs::remove_dir_all(dir).unwrap();
    }

    /// The same through `write_file`, the path signed saves take, and `Pdf::save`.
    #[cfg(unix)]
    #[test]
    fn signed_and_plain_saves_stay_private() {
        let dir = folder("write-file");
        let doc = dir.join("doc.pdf");
        std::fs::write(&doc, "old").unwrap();
        set_mode(&doc, 0o600);
        let calls = std::rc::Rc::new(std::cell::Cell::new(0));
        let (d, o, c) = (dir.clone(), doc.clone(), calls.clone());
        watch(move |tmp| {
            assert!(tmp.exists());
            all_private(&d, &[&o]);
            c.set(c.get() + 1);
        });
        crate::write_file(&doc, b"signed").unwrap();
        assert_eq!(calls.get(), 2);
        let mut p = crate::Pdf::new();
        p.insert_blank(0, 300.0, 200.0).unwrap();
        p.save(&doc).unwrap();
        assert_eq!(calls.get(), 4);
        unwatch();
        assert_eq!(mode(&doc), 0o600);
        assert!(std::fs::read(&doc).unwrap().starts_with(b"%PDF"));
        assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 1);
        std::fs::remove_dir_all(dir).unwrap();
    }

    /// A saved file keeps the original's mode; a new file gets the default one.
    #[cfg(unix)]
    #[test]
    fn modes_of_saved_files() {
        let dir = folder("modes");
        let doc = dir.join("doc.pdf");
        std::fs::write(&doc, "old").unwrap();
        set_mode(&doc, 0o644);
        crate::write_file(&doc, b"new").unwrap();
        assert_eq!(mode(&doc), 0o644);
        set_mode(&doc, 0o640);
        crate::write_file(&doc, b"newer").unwrap();
        assert_eq!(mode(&doc), 0o640);
        let plain = dir.join("plain");
        File::create(&plain).unwrap();
        let fresh = dir.join("fresh.pdf");
        crate::write_file(&fresh, b"new").unwrap();
        assert_eq!(mode(&fresh), mode(&plain));
        std::fs::remove_dir_all(dir).unwrap();
    }

    /// A stale file at the name older versions used (or one another user planted) is left
    /// as it is, and the save still succeeds.
    #[test]
    fn a_stale_temporary_file_is_left_alone() {
        let dir = folder("stale");
        let doc = dir.join("doc.pdf");
        std::fs::write(&doc, "old").unwrap();
        let pid = std::process::id().to_string();
        let stale = tmp_name(&dir, "doc.pdf", &pid);
        std::fs::write(&stale, "stale").unwrap();
        #[cfg(unix)]
        {
            set_mode(&doc, 0o600);
            set_mode(&stale, 0o644);
            let (d, o, s) = (dir.clone(), doc.clone(), stale.clone());
            watch(move |tmp| {
                assert_ne!(tmp, s);
                all_private(&d, &[&o, &s]);
            });
        }
        choose(&[&pid]);
        crate::write_file(&doc, b"new").unwrap();
        unwatch();
        assert_eq!(std::fs::read(&doc).unwrap(), b"new");
        assert_eq!(std::fs::read(&stale).unwrap(), b"stale");
        #[cfg(unix)]
        assert_eq!(mode(&stale), 0o644);
        assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 2);
        std::fs::remove_dir_all(dir).unwrap();
    }

    /// A symbolic link at a chosen name is neither followed nor removed.
    #[cfg(unix)]
    #[test]
    fn a_link_at_the_name_is_not_followed() {
        let dir = folder("link");
        let doc = dir.join("doc.pdf");
        std::fs::write(&doc, "old").unwrap();
        let target = dir.join("target");
        std::fs::write(&target, "keep").unwrap();
        set_mode(&target, 0o644);
        let link = tmp_name(&dir, "doc.pdf", "a");
        std::os::unix::fs::symlink(&target, &link).unwrap();
        let dangling = tmp_name(&dir, "doc.pdf", "b");
        std::os::unix::fs::symlink(dir.join("nothing"), &dangling).unwrap();
        choose(&["a", "b"]);
        crate::write_file(&doc, b"new").unwrap();
        assert_eq!(std::fs::read(&doc).unwrap(), b"new");
        assert_eq!(std::fs::read(&target).unwrap(), b"keep");
        assert_eq!(mode(&target), 0o644);
        assert_eq!(std::fs::read_link(&link).unwrap(), target);
        assert_eq!(std::fs::read_link(&dangling).unwrap(), dir.join("nothing"));
        assert!(!dir.join("nothing").exists());
        assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 4);
        std::fs::remove_dir_all(dir).unwrap();
    }

    /// Taken names are skipped; when every name tried is taken, the save fails and nothing
    /// changes.
    #[test]
    fn collisions() {
        let dir = folder("collide");
        let doc = dir.join("doc.pdf");
        std::fs::write(&doc, "old").unwrap();
        let names: Vec<String> = (0..TRIES).map(|i| format!("taken{i}")).collect();
        for n in &names {
            std::fs::write(tmp_name(&dir, "doc.pdf", n), n).unwrap();
        }
        let refs: Vec<&str> = names.iter().map(String::as_str).collect();

        choose(&refs[..3]);
        crate::write_file(&doc, b"new").unwrap();
        assert_eq!(std::fs::read(&doc).unwrap(), b"new");
        assert_eq!(std::fs::read_dir(&dir).unwrap().count(), TRIES + 1);

        let before = snapshot(&dir);
        choose(&refs);
        let e = crate::write_file(&doc, b"newer").unwrap_err();
        assert!(e.to_string().contains("names tried were taken"), "{e}");
        assert_eq!(snapshot(&dir), before);
        std::fs::remove_dir_all(dir).unwrap();
    }

    /// `write_private` gets the same exclusive creation, and is private throughout.
    #[test]
    fn private_files() {
        let dir = folder("write-private");
        let file = dir.join("trust.txt");
        std::fs::write(&file, "old").unwrap();
        let stale = tmp_name(&dir, "trust.txt", "stale");
        std::fs::write(&stale, "stale").unwrap();
        let old_name = dir.join("trust.txt.tmp");
        std::fs::write(&old_name, "older").unwrap();
        #[cfg(unix)]
        {
            set_mode(&stale, 0o644);
            set_mode(&old_name, 0o644);
            let (d, f, s, n) = (dir.clone(), file.clone(), stale.clone(), old_name.clone());
            watch(move |_| all_private(&d, &[&f, &s, &n]));
        }
        choose(&["stale"]);
        crate::write_private(&file, b"new").unwrap();
        unwatch();
        assert_eq!(std::fs::read(&file).unwrap(), b"new");
        assert_eq!(std::fs::read(&stale).unwrap(), b"stale");
        assert_eq!(std::fs::read(&old_name).unwrap(), b"older");
        #[cfg(unix)]
        {
            assert_eq!(mode(&file), 0o600);
            assert_eq!(mode(&stale), 0o644);
            let target = dir.join("target");
            std::fs::write(&target, "keep").unwrap();
            std::os::unix::fs::symlink(&target, tmp_name(&dir, "trust.txt", "link")).unwrap();
            choose(&["link"]);
            crate::write_private(&file, b"newer").unwrap();
            assert_eq!(std::fs::read(&target).unwrap(), b"keep");
            assert_eq!(std::fs::read(&file).unwrap(), b"newer");
            std::fs::remove_file(target).unwrap();
            std::fs::remove_file(tmp_name(&dir, "trust.txt", "link")).unwrap();
        }

        let names: Vec<String> = (0..TRIES).map(|i| format!("taken{i}")).collect();
        for n in &names {
            std::fs::write(tmp_name(&dir, "trust.txt", n), n).unwrap();
        }
        let before = snapshot(&dir);
        choose(&names.iter().map(String::as_str).collect::<Vec<_>>());
        assert!(crate::write_private(&file, b"newest").is_err());
        assert_eq!(snapshot(&dir), before);
        std::fs::remove_dir_all(dir).unwrap();
    }

    /// Recovery copies' temporary files (`disk::create_private_temp`): a stale file and a
    /// symbolic link at a name are skipped, not opened, followed or removed, and the file is
    /// private from the start.
    #[test]
    fn private_temporary_files() {
        let dir = folder("private-temp");
        let copy = dir.join("t1-2-3.pdf");
        let stale = tmp_name(&dir, "t1-2-3.pdf", "stale");
        std::fs::write(&stale, "stale").unwrap();
        let mut names = vec!["stale"];
        #[cfg(unix)]
        {
            let target = dir.join("target");
            std::fs::write(&target, "keep").unwrap();
            std::os::unix::fs::symlink(&target, tmp_name(&dir, "t1-2-3.pdf", "link")).unwrap();
            names.push("link");
        }
        names.push("fresh");
        choose(&names);
        let before = snapshot(&dir);
        let (tmp, f) = crate::disk::create_private_temp(&copy).unwrap();
        drop(f);
        assert_eq!(tmp, tmp_name(&dir, "t1-2-3.pdf", "fresh"));
        assert_eq!(std::fs::read(&tmp).unwrap(), b"");
        #[cfg(unix)]
        assert_eq!(mode(&tmp), 0o600);
        std::fs::remove_file(&tmp).unwrap();
        assert_eq!(snapshot(&dir), before);
        std::fs::remove_dir_all(dir).unwrap();
    }

    /// A write that fails half way leaves the original's bytes and permissions, and no
    /// temporary file (R2).
    #[test]
    fn a_failed_write_leaves_the_original() {
        let dir = folder("half");
        let doc = dir.join("doc.pdf");
        std::fs::write(&doc, "original").unwrap();
        #[cfg(unix)]
        set_mode(&doc, 0o640);
        let e = replace(&doc, Access::Keep, |w| {
            w.write_all(&[b'x'; 3 << 20])?;
            bail!("the disk is full")
        })
        .unwrap_err();
        assert_eq!(e.to_string(), "the disk is full");
        assert_eq!(std::fs::read(&doc).unwrap(), b"original");
        #[cfg(unix)]
        assert_eq!(mode(&doc), 0o640);
        assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 1);
        std::fs::remove_dir_all(dir).unwrap();
    }
}

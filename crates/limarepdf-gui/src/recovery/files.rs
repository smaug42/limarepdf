//! The file operations recovery uses, behind an interface so that tests can make a chosen one
//! fail, or run them on a model file system that simulates power cuts.

use std::io;
use std::path::{Path, PathBuf};

/// A lock taken with [`Fs::lock`]; dropping it releases the lock.
pub type Held = Box<dyn std::any::Any + Send + Sync>;

pub trait Fs: Send + Sync {
    /// Makes `dir` and any missing parents, private to this user on Unix.
    fn create_dir_all(&self, dir: &Path) -> io::Result<()>;
    /// Makes `dir`, which must not exist yet, private to this user on Unix.
    fn create_dir_new(&self, dir: &Path) -> io::Result<()>;
    /// Checks that `dir` is a folder (not a symbolic link) owned by this user, and makes it
    /// private to this user if it is wider (Unix; elsewhere the profile's permissions apply).
    fn check_folder(&self, dir: &Path) -> io::Result<()>;
    /// Writes `bytes` to a new temporary file beside `path`, created exclusively under a fresh
    /// name and private to this user, and syncs it to disk. Returns its path. On failure,
    /// nothing this call created is left.
    fn write_temp(&self, path: &Path, bytes: &[u8]) -> io::Result<PathBuf>;
    fn rename(&self, from: &Path, to: &Path) -> io::Result<()>;
    fn remove_file(&self, path: &Path) -> io::Result<()>;
    fn remove_dir_all(&self, dir: &Path) -> io::Result<()>;
    /// Whether anything is at `path`, a symbolic link included.
    fn exists(&self, path: &Path) -> bool;
    /// Makes the renames and removals in `dir` durable. Does nothing on Windows, which has no
    /// call for it (the design's limit 2).
    fn sync_dir(&self, dir: &Path) -> io::Result<()>;
    fn read(&self, path: &Path) -> io::Result<Vec<u8>>;
    /// The names in `dir`. Names that aren't Unicode are left out: recovery never makes them.
    fn list(&self, dir: &Path) -> io::Result<Vec<String>>;
    /// Takes the lock on the file at `path`, made first if `create`; `None` when another
    /// program (or another open of it) holds it.
    fn lock(&self, path: &Path, create: bool) -> io::Result<Option<Held>>;
    /// The bytes this user may still write on the disk holding `dir`.
    fn free_space(&self, dir: &Path) -> io::Result<u64>;
}

/// The real file system.
pub struct RealFs;

/// What to do with a folder found at the recovery location (Unix).
#[derive(Debug, PartialEq)]
pub(super) enum Verdict {
    Fine,
    /// Owned by this user, but others may get in, or this user can't: set it to 0700.
    Tighten,
    /// Owned by another user: recovery is off for the run.
    NotOurs,
}

/// Judges a folder by its owner and mode, for `me`.
pub(super) fn judge_folder(owner: u32, mode: u32, me: u32) -> Verdict {
    if owner != me {
        Verdict::NotOurs
    } else if mode & 0o777 != 0o700 {
        Verdict::Tighten
    } else {
        Verdict::Fine
    }
}

fn dir_builder(recursive: bool) -> std::fs::DirBuilder {
    let mut b = std::fs::DirBuilder::new();
    b.recursive(recursive);
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        b.mode(0o700);
    }
    b
}

impl Fs for RealFs {
    fn create_dir_all(&self, dir: &Path) -> io::Result<()> {
        dir_builder(true).create(dir)
    }

    fn create_dir_new(&self, dir: &Path) -> io::Result<()> {
        dir_builder(false).create(dir)
    }

    fn check_folder(&self, dir: &Path) -> io::Result<()> {
        let m = std::fs::symlink_metadata(dir)?;
        if !m.is_dir() {
            return Err(io::Error::other(format!(
                "{} is not a folder",
                dir.display()
            )));
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::{MetadataExt, PermissionsExt};
            match judge_folder(m.uid(), m.mode(), limarepdf_core::disk::user_id()) {
                Verdict::Fine => {}
                Verdict::Tighten => {
                    std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700))?
                }
                Verdict::NotOurs => {
                    return Err(io::Error::new(
                        io::ErrorKind::PermissionDenied,
                        format!("{} belongs to another user", dir.display()),
                    ))
                }
            }
        }
        Ok(())
    }

    fn write_temp(&self, path: &Path, bytes: &[u8]) -> io::Result<PathBuf> {
        use std::io::Write;
        let (tmp, mut f) = limarepdf_core::disk::create_private_temp(path).map_err(|e| {
            e.downcast::<io::Error>()
                .unwrap_or_else(|e| io::Error::other(format!("{e:#}")))
        })?;
        let written = f.write_all(bytes).and_then(|()| f.sync_all());
        drop(f);
        match written {
            Ok(()) => Ok(tmp),
            Err(e) => {
                let _ = std::fs::remove_file(&tmp);
                Err(e)
            }
        }
    }

    fn rename(&self, from: &Path, to: &Path) -> io::Result<()> {
        std::fs::rename(from, to)
    }

    fn remove_file(&self, path: &Path) -> io::Result<()> {
        std::fs::remove_file(path)
    }

    fn remove_dir_all(&self, dir: &Path) -> io::Result<()> {
        std::fs::remove_dir_all(dir)
    }

    fn exists(&self, path: &Path) -> bool {
        std::fs::symlink_metadata(path).is_ok()
    }

    fn sync_dir(&self, dir: &Path) -> io::Result<()> {
        #[cfg(unix)]
        std::fs::File::open(dir)?.sync_all()?;
        #[cfg(not(unix))]
        let _ = dir;
        Ok(())
    }

    fn read(&self, path: &Path) -> io::Result<Vec<u8>> {
        std::fs::read(path)
    }

    fn list(&self, dir: &Path) -> io::Result<Vec<String>> {
        let mut names = Vec::new();
        for e in std::fs::read_dir(dir)? {
            if let Ok(n) = e?.file_name().into_string() {
                names.push(n);
            }
        }
        names.sort();
        Ok(names)
    }

    fn lock(&self, path: &Path, create: bool) -> io::Result<Option<Held>> {
        let mut o = std::fs::OpenOptions::new();
        o.read(true).write(true);
        if create {
            o.create_new(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;
                o.mode(0o600);
            }
        }
        let f = o.open(path)?;
        match f.try_lock() {
            Ok(()) => Ok(Some(Box::new(f))),
            Err(std::fs::TryLockError::WouldBlock) => Ok(None),
            Err(std::fs::TryLockError::Error(e)) => Err(e),
        }
    }

    fn free_space(&self, dir: &Path) -> io::Result<u64> {
        limarepdf_core::disk::free_space(dir)
    }
}

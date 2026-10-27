//! Disk helpers for the app's own private files (crash recovery copies): temporary files
//! created exclusively, a disk's free space, this user's id and a SHA-256 checksum. And for
//! documents: whether two paths name one file, and the [`Version`] of a file last read or
//! written, to tell before saving over it whether something else changed it since.

use crate::Result;
use std::fs::{File, Metadata};
use std::io::{ErrorKind, Read};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

/// Creates a new, empty file beside `path` for this user only (mode 600 on Unix), under a
/// fresh name nothing was at, with the rules of [`write_private`](crate::write_private): a
/// stale file or symbolic link at a name is never opened, followed or removed. The caller
/// writes, syncs and renames it, and removes it on failure.
pub fn create_private_temp(path: &Path) -> Result<(PathBuf, File)> {
    crate::atomic::create_private(path)
}

/// The bytes this user may still write on the disk holding `dir`.
pub fn free_space(dir: &Path) -> std::io::Result<u64> {
    #[cfg(unix)]
    {
        let s = rustix::fs::statvfs(dir)?;
        Ok(s.f_bavail.saturating_mul(s.f_frsize))
    }
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt;
        use windows_sys::Win32::Storage::FileSystem::GetDiskFreeSpaceExW;
        let wide: Vec<u16> = dir.as_os_str().encode_wide().chain(Some(0)).collect();
        let mut free = 0u64;
        // SAFETY: `wide` is a NUL-terminated UTF-16 string that outlives the call, and the
        // two totals not wanted may be null.
        let ok = unsafe {
            GetDiskFreeSpaceExW(
                wide.as_ptr(),
                &mut free,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
            )
        };
        if ok == 0 {
            return Err(std::io::Error::last_os_error());
        }
        Ok(free)
    }
    #[cfg(not(any(unix, windows)))]
    {
        let _ = dir;
        Err(std::io::ErrorKind::Unsupported.into())
    }
}

/// This process's effective user id, to compare with a file's owner.
#[cfg(unix)]
pub fn user_id() -> u32 {
    rustix::process::geteuid().as_raw()
}

/// The SHA-256 checksum of `bytes`.
pub fn sha256(bytes: &[u8]) -> [u8; 32] {
    let d = crate::cms_check::DigestAlg::Sha256.hash(&[bytes]);
    d.try_into().expect("SHA-256 gives 32 bytes")
}

/// Whether `a` and `b` name the same file: the same path, or the same once links and `..`
/// are followed, or on Unix the same device and inode, as a hard link is.
pub fn same_file(a: &Path, b: &Path) -> bool {
    if a == b
        || a.canonicalize()
            .is_ok_and(|a| b.canonicalize().is_ok_and(|b| a == b))
    {
        return true;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if let (Ok(a), Ok(b)) = (std::fs::metadata(a), std::fs::metadata(b)) {
            return a.dev() == b.dev() && a.ino() == b.ino();
        }
    }
    false
}

/// How much older than its reading a file's time must be to be trusted: a change made in
/// the same tick of the file's clock keeps the time, and FAT keeps times to 2 seconds.
const SETTLED: Duration = Duration::from_secs(2);

/// A file as it was when last read or written: its size, time and (Unix) inode, which tell
/// quickly that it is unchanged, and the checksum of its bytes, which tells whether a file
/// whose size or time changed holds anything different.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Version {
    len: u64,
    modified: Option<SystemTime>,
    /// Saving renames a new file over the old one, so a saved file has a new inode.
    #[cfg(unix)]
    node: (u64, u64),
    sum: [u8; 32],
    /// When it was read: a time not [`SETTLED`] before this could still be the time of a
    /// change made just after the reading (PB-28), as git's "racy" files.
    read: SystemTime,
}

impl Version {
    fn of(m: &Metadata, bytes: &[u8], read: SystemTime) -> Self {
        #[cfg(unix)]
        use std::os::unix::fs::MetadataExt;
        Self {
            len: m.len(),
            modified: m.modified().ok(),
            #[cfg(unix)]
            node: (m.dev(), m.ino()),
            sum: sha256(bytes),
            read,
        }
    }

    /// Whether `m` shows the file as this version knew it, so it needn't be read. A time
    /// too close to the reading isn't trusted: the file is read and its checksum compared.
    fn stamp_is(&self, m: &Metadata) -> bool {
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            if self.node != (m.dev(), m.ino()) {
                return false;
            }
        }
        let settled = self
            .modified
            .and_then(|t| t.checked_add(SETTLED))
            .is_some_and(|t| t <= self.read);
        settled && self.len == m.len() && self.modified == m.modified().ok()
    }

    /// Whether the two hold the same bytes.
    pub fn same_bytes(&self, other: &Version) -> bool {
        self.sum == other.sum
    }
}

/// Reads the file at `path`, with its version: both through one handle, so they agree even
/// when another file is renamed over it meanwhile.
pub fn read(path: &Path) -> std::io::Result<(Vec<u8>, Version)> {
    let mut f = File::open(path)?;
    // Before the time is read, so a change at the moment of reading counts as too recent.
    let read = SystemTime::now();
    let m = f.metadata()?;
    let mut bytes = Vec::with_capacity(usize::try_from(m.len()).unwrap_or(0));
    f.read_to_end(&mut bytes)?;
    let v = Version::of(&m, &bytes, read);
    Ok((bytes, v))
}

/// The version of the file at `path` now.
pub fn version(path: &Path) -> std::io::Result<Version> {
    read(path).map(|(_, v)| v)
}

/// The file at a path, next to a version of it known earlier.
#[derive(Debug)]
pub enum OnDisk {
    /// It holds the same bytes, as it now is (its time may be new).
    Same(Version),
    /// Something changed it: as it now is.
    Changed(Version),
    /// Nothing is there any more.
    Gone,
}

/// Compares the file at `path` with `known`. The file is read only when its size, time or
/// inode differ, or its time was too close to `known`'s reading to be trusted, to tell a
/// change from a file touched or copied unchanged.
pub fn compare(path: &Path, known: &Version) -> std::io::Result<OnDisk> {
    match std::fs::metadata(path) {
        Ok(m) if known.stamp_is(&m) => return Ok(OnDisk::Same(known.clone())),
        Ok(_) => {}
        Err(e) if e.kind() == ErrorKind::NotFound => return Ok(OnDisk::Gone),
        Err(e) => return Err(e),
    }
    let now = match version(path) {
        Ok(v) => v,
        Err(e) if e.kind() == ErrorKind::NotFound => return Ok(OnDisk::Gone),
        Err(e) => return Err(e),
    };
    Ok(if now.same_bytes(known) {
        OnDisk::Same(now)
    } else {
        OnDisk::Changed(now)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_dir(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("limarepdf-disk-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn same_file_through_any_path_to_it() {
        let dir = test_dir("same");
        let (a, b) = (dir.join("a.pdf"), dir.join("b.pdf"));
        std::fs::write(&a, "a").unwrap();
        std::fs::write(&b, "a").unwrap();
        assert!(same_file(&a, &a));
        std::fs::create_dir(dir.join("sub")).unwrap();
        assert!(same_file(&a, &dir.join("sub/../a.pdf")));
        assert!(!same_file(&a, &b), "the same bytes in another file");
        assert!(!same_file(&a, &dir.join("none.pdf")));
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(&a, dir.join("link.pdf")).unwrap();
            assert!(same_file(&dir.join("link.pdf"), &a));
            std::fs::hard_link(&a, dir.join("hard.pdf")).unwrap();
            assert!(same_file(&dir.join("hard.pdf"), &a));
        }
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn a_version_tells_changed_bytes_from_a_touched_file() {
        let dir = test_dir("version");
        let p = dir.join("doc.pdf");
        std::fs::write(&p, "first").unwrap();
        let (bytes, v) = read(&p).unwrap();
        assert_eq!(bytes, b"first");
        assert!(matches!(compare(&p, &v).unwrap(), OnDisk::Same(s) if s.same_bytes(&v)));
        // The same bytes written again, as a sync tool or `touch` might: not a change.
        let again = dir.join("again.pdf");
        std::fs::write(&again, "first").unwrap();
        std::fs::rename(&again, &p).unwrap();
        match compare(&p, &v).unwrap() {
            OnDisk::Same(now) => assert!(now.same_bytes(&v)),
            o => panic!("{o:?}"),
        }
        // Changed in place, at the same size.
        std::fs::write(&p, "fir5t").unwrap();
        match compare(&p, &v).unwrap() {
            OnDisk::Changed(now) => {
                assert!(!now.same_bytes(&v));
                assert!(matches!(compare(&p, &now).unwrap(), OnDisk::Same(_)));
            }
            o => panic!("{o:?}"),
        }
        std::fs::remove_file(&p).unwrap();
        assert!(matches!(compare(&p, &v).unwrap(), OnDisk::Gone));
        std::fs::remove_dir_all(dir).unwrap();
    }

    /// PB-28: a change at the same size within one tick of the file's clock, so with the
    /// same time, is noticed: on Windows, with no inode to tell it, it read as unchanged.
    /// The time is set here, so the race happens on every platform.
    #[test]
    fn a_change_in_the_same_tick_as_the_read_is_noticed() {
        let dir = test_dir("tick");
        let p = dir.join("doc.pdf");
        std::fs::write(&p, "first").unwrap();
        let (_, v) = read(&p).unwrap();
        let f = std::fs::OpenOptions::new().write(true).open(&p).unwrap();
        std::io::Write::write_all(&mut &f, b"fir5t").unwrap();
        f.set_modified(v.modified.unwrap()).unwrap();
        drop(f);
        match compare(&p, &v).unwrap() {
            OnDisk::Changed(now) => assert!(!now.same_bytes(&v)),
            o => panic!("{o:?}"),
        }
        std::fs::remove_dir_all(dir).unwrap();
    }

    /// A file last changed well before it was read is trusted by its size and time,
    /// without reading it again.
    #[test]
    fn an_older_file_is_known_by_its_size_and_time() {
        let dir = test_dir("older");
        let p = dir.join("doc.pdf");
        std::fs::write(&p, "first").unwrap();
        let hour_ago = SystemTime::now() - std::time::Duration::from_secs(3600);
        let f = std::fs::OpenOptions::new().write(true).open(&p).unwrap();
        f.set_modified(hour_ago).unwrap();
        drop(f);
        let (_, v) = read(&p).unwrap();
        assert!(v.stamp_is(&std::fs::metadata(&p).unwrap()));
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn sha256_known_answer() {
        let d = sha256(b"abc");
        assert_eq!(d[..4], [0xba, 0x78, 0x16, 0xbf]);
        assert_eq!(d[28..], [0xf2, 0x00, 0x15, 0xad]);
    }

    #[test]
    fn free_space_of_the_temporary_folder() {
        assert!(free_space(&std::env::temp_dir()).unwrap() > 0);
        assert!(free_space(Path::new("/no/such/folder/limarepdf")).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn user_id_owns_new_files() {
        use std::os::unix::fs::MetadataExt;
        let dir = std::env::temp_dir().join(format!("limarepdf-disk-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let (tmp, f) = create_private_temp(&dir.join("copy.pdf")).unwrap();
        drop(f);
        let m = std::fs::metadata(&tmp).unwrap();
        assert_eq!(m.uid(), user_id());
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(m.permissions().mode() & 0o777, 0o600);
        assert!(tmp
            .file_name()
            .unwrap()
            .to_str()
            .unwrap()
            .starts_with(".copy.pdf."));
        std::fs::remove_dir_all(dir).unwrap();
    }
}

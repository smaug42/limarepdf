//! Disk helpers for the app's own private files (crash recovery copies): temporary files
//! created exclusively, a disk's free space, this user's id and a SHA-256 checksum.

use crate::Result;
use std::fs::File;
use std::path::{Path, PathBuf};

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

#[cfg(test)]
mod tests {
    use super::*;

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

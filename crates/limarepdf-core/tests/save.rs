//! Saving replaces files safely.

use limarepdf_core::Pdf;
use std::path::PathBuf;

/// A fresh, empty folder for one test.
fn folder(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("limarepdf-save-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn pages(n: usize) -> Pdf {
    let mut p = Pdf::new();
    for i in 0..n {
        p.insert_blank(i, 300.0, 200.0).unwrap();
    }
    p
}

#[test]
fn replaces_a_file_and_leaves_nothing_behind() {
    let dir = folder("replace");
    let path = dir.join("doc.pdf");
    pages(1).save(&path).unwrap();
    pages(3).save(&path).unwrap();
    let back = Pdf::from_bytes(&std::fs::read(&path).unwrap(), None).unwrap();
    assert_eq!(back.page_count(), 3);
    let names: Vec<_> = std::fs::read_dir(&dir).unwrap().collect();
    assert_eq!(names.len(), 1, "a temporary file was left behind");
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn a_failed_save_creates_nothing() {
    let dir = folder("fail");
    let path = dir.join("missing").join("doc.pdf");
    assert!(pages(1).save(&path).is_err());
    assert!(!path.exists());
    std::fs::remove_dir_all(dir).unwrap();
}

#[cfg(unix)]
#[test]
fn keeps_permissions_and_follows_links() {
    use std::os::unix::fs::PermissionsExt;
    let dir = folder("unix");
    let path = dir.join("doc.pdf");
    pages(1).save(&path).unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
    let link = dir.join("link.pdf");
    std::os::unix::fs::symlink(&path, &link).unwrap();
    pages(2).save(&link).unwrap();
    assert!(std::fs::symlink_metadata(&link)
        .unwrap()
        .file_type()
        .is_symlink());
    let mode = std::fs::metadata(&path).unwrap().permissions().mode();
    assert_eq!(mode & 0o777, 0o600);
    let back = Pdf::from_bytes(&std::fs::read(&path).unwrap(), None).unwrap();
    assert_eq!(back.page_count(), 2);
    std::fs::remove_dir_all(dir).unwrap();
}

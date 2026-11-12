//! Saving replaces files safely.

#[allow(dead_code)]
#[path = "common/tool.rs"]
mod tool;

use limarepdf_core::Pdf;
use tool::Scratch;

fn pages(n: usize) -> Pdf {
    let mut p = Pdf::new();
    for i in 0..n {
        p.insert_blank(i, 300.0, 200.0).unwrap();
    }
    p
}

#[test]
fn replaces_a_file_and_leaves_nothing_behind() {
    let dir = Scratch::new("save-replace");
    let path = dir.join("doc.pdf");
    pages(1).save(&path).unwrap();
    pages(3).save(&path).unwrap();
    let back = Pdf::from_bytes(&std::fs::read(&path).unwrap(), None).unwrap();
    assert_eq!(back.page_count(), 3);
    let names: Vec<_> = std::fs::read_dir(dir.path()).unwrap().collect();
    assert_eq!(names.len(), 1, "a temporary file was left behind");
}

#[test]
fn a_failed_save_creates_nothing() {
    let dir = Scratch::new("save-fail");
    let path = dir.join("missing").join("doc.pdf");
    assert!(pages(1).save(&path).is_err());
    assert!(!path.exists());
}

#[cfg(unix)]
#[test]
fn keeps_permissions_and_follows_links() {
    use std::os::unix::fs::PermissionsExt;
    let dir = Scratch::new("save-unix");
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
}

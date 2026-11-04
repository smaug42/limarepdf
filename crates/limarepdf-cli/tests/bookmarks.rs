//! `limarepdf-cli bookmarks`: bookmarks added and moved, read back with `list`.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

/// A PDF of three blank pages.
fn three_pages(dir: &Path) -> PathBuf {
    let objects = [
        "<< /Type /Catalog /Pages 2 0 R >>",
        "<< /Type /Pages /Kids [3 0 R 4 0 R 5 0 R] /Count 3 >>",
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 200 200] >>",
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 200 200] >>",
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 200 200] >>",
    ];
    let mut pdf = b"%PDF-1.7\n".to_vec();
    let mut offsets = Vec::new();
    for (i, o) in objects.iter().enumerate() {
        offsets.push(pdf.len());
        pdf.extend_from_slice(format!("{} 0 obj\n{o}\nendobj\n", i + 1).as_bytes());
    }
    let xref = pdf.len();
    pdf.extend_from_slice(b"xref\n0 6\n0000000000 65535 f \n");
    for o in offsets {
        pdf.extend_from_slice(format!("{o:010} 00000 n \n").as_bytes());
    }
    pdf.extend_from_slice(
        format!("trailer\n<< /Size 6 /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n").as_bytes(),
    );
    let path = dir.join("three.pdf");
    std::fs::write(&path, pdf).unwrap();
    path
}

fn cli(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_limarepdf-cli"))
        .args(args)
        .output()
        .unwrap()
}

/// The file's bookmarks, each title after two spaces for each level it is in.
fn listed(file: &Path) -> Vec<String> {
    let out = cli(&["bookmarks", "list", file.to_str().unwrap()]);
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(out.status.success(), "{err}");
    let text = String::from_utf8_lossy(&out.stdout).into_owned();
    // Past the number and the page column ("p. " and five places, then a space).
    let title = |l: &str| l[l.find("p. ").unwrap() + 9..].to_string();
    text.lines().map(title).collect()
}

/// Runs `bookmarks ACTION FILE ARGS... -o FILE2` and goes on from FILE2.
struct Steps {
    file: PathBuf,
    n: usize,
}

impl Steps {
    fn run(&mut self, action: &str, args: &[&str]) -> Output {
        self.n += 1;
        let next = self.file.with_file_name(format!("step-{}.pdf", self.n));
        let (input, output) = (self.file.to_str().unwrap(), next.to_str().unwrap());
        let mut all = vec!["bookmarks", action, input];
        all.extend(args);
        all.extend(["-o", output]);
        let out = cli(&all);
        if out.status.success() {
            self.file = next;
        }
        out
    }

    fn ok(&mut self, action: &str, args: &[&str]) {
        let out = self.run(action, args);
        let err = String::from_utf8_lossy(&out.stderr);
        assert!(out.status.success(), "{action} {args:?}: {err}");
    }
}

#[test]
fn moves_bookmarks_before_after_under_and_to_the_end() {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join("bookmarks-move");
    std::fs::create_dir_all(&dir).unwrap();
    let mut s = Steps {
        file: three_pages(&dir),
        n: 0,
    };
    for (title, page) in [("A", "1"), ("B", "2"), ("C", "3")] {
        s.ok("add", &["--title", title, "--page", page]);
    }
    s.ok("add", &["--title", "A1", "--page", "2", "--under", "1"]);
    s.ok("add", &["--title", "Zero", "--page", "1", "--before", "1"]);
    assert_eq!(listed(&s.file), ["Zero", "A", "  A1", "B", "C"]);

    s.ok("move", &["5", "--before", "2"]);
    assert_eq!(listed(&s.file), ["Zero", "C", "A", "  A1", "B"]);
    s.ok("move", &["4", "--after", "5"]);
    assert_eq!(listed(&s.file), ["Zero", "C", "A", "B", "A1"]);
    s.ok("move", &["2", "--under", "4"]);
    assert_eq!(listed(&s.file), ["Zero", "A", "B", "  C", "A1"]);
    // With no place given, last at the top level.
    s.ok("move", &["3"]);
    assert_eq!(listed(&s.file), ["Zero", "A", "A1", "B", "  C"]);

    // Into itself, or past the end: refused, and nothing written.
    let at = s.file.clone();
    let out = s.run("move", &["4", "--under", "5"]);
    assert!(!out.status.success());
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(err.contains("inside itself"), "{err}");
    assert!(!s.run("move", &["6"]).status.success());
    assert!(!s.run("move", &["0"]).status.success());
    let both = ["1", "--before", "2", "--after", "3"];
    assert!(!s.run("move", &both).status.success());
    assert_eq!(s.file, at);
}

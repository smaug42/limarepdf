//! The public test files of `tools/render-samples.txt`, fetched into tests/data/render by
//! `tools/fetch-render-samples.sh` (never committed).

use std::path::{Path, PathBuf};

#[derive(Clone)]
pub struct Sample {
    /// `pdfjs/issue1234`, the file's path under tests/data/render without `.pdf`.
    pub name: String,
    /// The pages to draw (1-based): the ones hayro tests, at most three.
    pub first: usize,
    pub last: usize,
    pub password: Option<String>,
}

impl Sample {
    pub fn path(&self) -> PathBuf {
        root().join(format!("{}.pdf", self.name))
    }
}

/// Where the fetched files are.
pub fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/data/render")
}

/// The listed files whose name contains `only`, fetched or not.
pub fn samples(only: &str) -> Vec<Sample> {
    let list = Path::new(env!("CARGO_MANIFEST_DIR")).join("tools/render-samples.txt");
    let text = std::fs::read_to_string(&list).expect("tools/render-samples.txt");
    text.lines()
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .map(|l| {
            let f: Vec<&str> = l.split_whitespace().collect();
            let dir = f[0].rsplit('-').next().unwrap();
            let (a, b) = f[3].split_once('-').expect("pages as first-last");
            Sample {
                name: format!("{dir}/{}", f[1]),
                first: a.parse().unwrap(),
                last: b.parse().unwrap(),
                password: f.get(4).map(|s| s.to_string()),
            }
        })
        .filter(|s| s.name.contains(only))
        .collect()
}

//! `limarepdf-cli search` (batch 22, ENH-03): one line a match, `page N: ` and the words
//! around it, then the count in words; `--boxes` the match's boxes with their pages. A phrase
//! running on to the next page is found (PF-38).

use std::path::{Path, PathBuf};
use std::process::Command;

/// A two-page PDF: "the walrus" on the first page, "ate fish" and "a walrus" on the second.
fn make_pdf(dir: &Path) -> PathBuf {
    let page = |text: &str| format!("BT /F1 12 Tf 10 150 Td ({text}) Tj ET");
    let (one, two) = (
        page("the walrus"),
        "BT /F1 12 Tf 10 150 Td (ate fish) Tj 0 -20 Td (a walrus) Tj ET".to_string(),
    );
    let objects = [
        "<< /Type /Catalog /Pages 2 0 R >>".to_string(),
        "<< /Type /Pages /Kids [3 0 R 4 0 R] /Count 2 >>".into(),
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 200 200] /Contents 5 0 R \
         /Resources << /Font << /F1 7 0 R >> >> >>"
            .into(),
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 200 200] /Contents 6 0 R \
         /Resources << /Font << /F1 7 0 R >> >> >>"
            .into(),
        format!("<< /Length {} >>\nstream\n{one}\nendstream", one.len()),
        format!("<< /Length {} >>\nstream\n{two}\nendstream", two.len()),
        "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".into(),
    ];
    let mut pdf = b"%PDF-1.7\n".to_vec();
    let mut offsets = Vec::new();
    for (i, o) in objects.iter().enumerate() {
        offsets.push(pdf.len());
        pdf.extend_from_slice(format!("{} 0 obj\n{o}\nendobj\n", i + 1).as_bytes());
    }
    let xref = pdf.len();
    pdf.extend_from_slice(
        format!("xref\n0 {}\n0000000000 65535 f \n", objects.len() + 1).as_bytes(),
    );
    for o in offsets {
        pdf.extend_from_slice(format!("{o:010} 00000 n \n").as_bytes());
    }
    pdf.extend_from_slice(
        format!(
            "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n",
            objects.len() + 1
        )
        .as_bytes(),
    );
    let path = dir.join("walrus.pdf");
    std::fs::write(&path, pdf).unwrap();
    path
}

fn search(file: &Path, args: &[&str]) -> String {
    let out = Command::new(env!("CARGO_BIN_EXE_limarepdf-cli"))
        .arg("search")
        .arg(file)
        .args(args)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).into_owned()
}

#[test]
fn search_prints_each_match_with_its_page_and_the_count_in_words() {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join("search");
    std::fs::create_dir_all(&dir).unwrap();
    let file = make_pdf(&dir);
    let out = search(&file, &["walrus"]);
    let lines: Vec<&str> = out.lines().collect();
    assert_eq!(
        lines,
        [
            "page 1: the walrus",
            "page 2: ate fish a walrus",
            "2 matches"
        ],
        "{out}"
    );
    // A phrase across the page break, with its boxes on both pages.
    let out = search(&file, &["walrus ate", "--boxes"]);
    let lines: Vec<&str> = out.lines().collect();
    assert_eq!(
        &lines[..2],
        ["page 1: the walrus ate fish a walrus", "1 match"],
        "{out}"
    );
    let pages: Vec<&str> = lines[2..]
        .iter()
        .map(|l| l.split(" [").next().unwrap())
        .collect();
    assert_eq!(pages, ["page 1", "page 2"], "{out}");
    assert_eq!(search(&file, &["otter"]).trim(), "No matches");
}

/// NEW-04a: `--match-case` and `--whole-words`, as Search results' Match case and Whole
/// words only.
#[test]
fn search_matches_case_and_whole_words_when_asked() {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join("search-options");
    std::fs::create_dir_all(&dir).unwrap();
    let file = make_pdf(&dir);
    let last = |out: String| out.lines().last().unwrap_or("").to_string();
    assert_eq!(last(search(&file, &["WALRUS"])), "2 matches");
    let case = |q: &str| last(search(&file, &[q, "--match-case"]));
    assert_eq!(case("WALRUS"), "No matches");
    assert_eq!(case("walrus"), "2 matches");
    assert_eq!(last(search(&file, &["walr"])), "2 matches");
    let whole = last(search(&file, &["walr", "--whole-words"]));
    assert_eq!(whole, "No matches");
    let out = search(&file, &["fish", "--whole-words", "--match-case"]);
    let lines: Vec<&str> = out.lines().collect();
    assert_eq!(lines, ["page 2: ate fish a walrus", "1 match"]);
}

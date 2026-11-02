//! The paper sizes LimarePDF reads for this computer's printers, checked against what CUPS's
//! own `ipptool` reports. Opt-in (`-- --ignored cross_check`): it needs a printer set up and
//! `ipptool`, and asks the printing service, never the printer itself.
#![cfg(not(windows))]

use limarepdf_core::print;
use std::process::Command;

/// The values `ipptool -tv` prints for `name`, as "        media-supported (1setOf keyword)
/// = a,b,c".
fn ipptool_values(out: &str, name: &str) -> Vec<String> {
    out.lines()
        .find_map(|l| {
            let (k, v) = l.trim().split_once(" = ")?;
            (k.split_whitespace().next() == Some(name)).then(|| v.to_string())
        })
        .map(|v| v.split(',').map(str::to_string).collect())
        .unwrap_or_default()
}

#[test]
#[ignore]
fn cross_check_paper_sizes_with_ipptool() {
    let printers = print::printers().expect("listing the printers");
    if printers.is_empty() {
        eprintln!("no printers are set up: nothing to check");
        return;
    }
    for p in printers {
        let list = print::papers(&p.name).expect("the paper sizes");
        eprintln!("{}:", p.name);
        for (i, paper) in list.papers.iter().enumerate() {
            let mark = if list.default == Some(i) {
                " (default)"
            } else {
                ""
            };
            eprintln!(
                "  {} = {}{mark}, margins {:?}",
                paper.id, paper.label, paper.margins
            );
        }
        let queue = p.name.split('/').next().unwrap();
        let uri = format!("ipp://localhost/printers/{queue}");
        let out = Command::new("ipptool")
            .args(["-tv", &uri, "get-printer-attributes.test"])
            .output();
        let Ok(out) = out else {
            eprintln!("ipptool isn't installed: the list above isn't checked");
            continue;
        };
        let out = String::from_utf8_lossy(&out.stdout);
        let supported = ipptool_values(&out, "media-supported");
        let default = ipptool_values(&out, "media-default");
        let named: Vec<&String> = supported
            .iter()
            .filter(|m| !m.starts_with("custom_"))
            .collect();
        assert!(!named.is_empty(), "ipptool listed no sizes for {}", p.name);
        // Every named size LimarePDF reads is one CUPS lists, and the default is CUPS's.
        for paper in &list.papers {
            assert!(supported.contains(&paper.id), "{} isn't CUPS's", paper.id);
            assert!(paper.size.0 > 0.0 && paper.size.1 > 0.0);
        }
        let ids: Vec<&String> = list.papers.iter().map(|p| &p.id).collect();
        let missing: Vec<&&String> = named.iter().filter(|m| !ids.contains(m)).collect();
        assert!(
            missing.len() * 4 <= named.len(),
            "most sizes missing: {missing:?}"
        );
        let ours = list.default.map(|i| list.papers[i].id.clone());
        assert_eq!(ours.as_ref(), default.first(), "the default");
    }
}

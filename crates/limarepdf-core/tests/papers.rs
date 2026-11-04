//! The paper sizes LimarePDF reads for this computer's printers, checked against what CUPS's
//! own `ipptool` reports. Opt-in (`-- --ignored cross_check`): it needs a printer set up and
//! `ipptool`, and asks the printing service, never the printer itself.
#![cfg(not(windows))]

#[allow(dead_code)]
#[path = "common/tool.rs"]
mod tool;

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
        let out = tool::output(Command::new("ipptool").args([
            "-tv",
            &uri,
            "get-printer-attributes.test",
        ]));
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

/// The printers listed at once over IPP (ENH-47) are among those `lpstat` lists after
/// waiting for the network's, with the same default; how long each took is printed.
#[test]
#[ignore]
fn cross_check_printers_listed_at_once_with_lpstat() {
    let start = std::time::Instant::now();
    let mut steps = Vec::new();
    print::printers_in_steps(|found, last| steps.push((found, last, start.elapsed())));
    for (found, last, at) in &steps {
        let names: Vec<String> = match found {
            Ok(list) => list.iter().map(|p| p.name.clone()).collect(),
            Err(e) => vec![format!("{e:#}")],
        };
        eprintln!("{at:?} (last {last}): {names:?}");
    }
    let (full, true, _) = steps.last().expect("the full list") else {
        panic!("the last step isn't the last");
    };
    let full = full.as_ref().expect("lpstat's list");
    let [(Ok(now), false, _), _] = &steps[..] else {
        eprintln!("CUPS has no printer queues, or IPP couldn't be asked: nothing listed at once");
        return;
    };
    for p in now {
        let there = full.iter().find(|f| f.name == p.name);
        let there = there.unwrap_or_else(|| panic!("{} isn't in lpstat's list", p.name));
        assert_eq!(there.default, p.default, "{}'s default", p.name);
    }
}

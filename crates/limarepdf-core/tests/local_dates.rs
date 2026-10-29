//! The dates people see (headers and footers, Fill and sign's Today, stamps' name-and-date
//! lines, the date under a signature) are this computer's calendar date, not UTC's, and a
//! comment's date is shown in this computer's time zone. Each time zone is tried in a child
//! test process of its own, with `TZ` set.

#![cfg(unix)]

use limarepdf_core::{
    today_iso, DateStyle, HeaderFooter, InkColor, Pdf, SignArt, SignRole, SignStyle, Stamp,
    StampKind, StampPlace,
};
use std::process::Command;

const CHILD: &str = "LIMAREPDF_LOCAL_DATES_CHILD";

/// Every visible date LimarePDF writes, made now.
fn dates_made_now() -> Vec<(&'static str, String)> {
    let mut p = Pdf::new();
    p.insert_blank(0, 300.0, 200.0).unwrap();
    let hf = HeaderFooter {
        header: ["{date}".into(), "".into(), "".into()],
        footer: Default::default(),
        ..Default::default()
    };
    p.add_header_footer(&[0], &hf).unwrap();
    let header = p
        .page_objects(0)
        .unwrap()
        .into_iter()
        .find_map(|o| o.text)
        .unwrap();
    let stamp = Stamp::Text {
        kind: StampKind::Approved,
        name_and_date: true,
    };
    let signature = Stamp::Signature {
        art: SignArt::Typed {
            text: "Sam Smith",
            style: SignStyle::Caveat,
        },
        role: SignRole::Signature,
        color: InkColor::Blue,
        date: true,
    };
    let drawn = |stamp: &Stamp| {
        let mut p = Pdf::new();
        p.insert_blank(0, 300.0, 200.0).unwrap();
        p.add_stamp(0, stamp, StampPlace::Centre(150.0, 100.0), "Sam")
            .unwrap();
        // The appearance streams' text, where the date line is written.
        p.doc
            .objects
            .values()
            .filter_map(|o| o.as_stream().ok())
            .map(|s| s.decompressed_content().unwrap_or(s.content.clone()))
            .map(|b| String::from_utf8_lossy(&b).into_owned())
            .collect::<String>()
    };
    let (stamp, signature) = (drawn(&stamp), drawn(&signature));
    // The first YYYY-MM-DD in `text`.
    let date_in = |text: &str| {
        let is_date = |s: &[u8]| {
            let digit = |k: usize| s[k].is_ascii_digit();
            s[4] == b'-' && s[7] == b'-' && [0, 1, 2, 3, 5, 6, 8, 9].into_iter().all(digit)
        };
        let b = text.as_bytes();
        (0..b.len().saturating_sub(9))
            .find(|&i| is_date(&b[i..i + 10]))
            .map_or("none".into(), |i| text[i..i + 10].to_string())
    };
    vec![
        ("today_iso", today_iso()),
        ("Fill and sign's Today", DateStyle::Iso.today()),
        ("header", header),
        ("stamp", date_in(&stamp)),
        ("signature", date_in(&signature)),
    ]
}

/// Run by [`visible_dates_are_local`] in a time zone of its own.
#[test]
#[ignore = "run by visible_dates_are_local with TZ set"]
fn print_dates_made_now() {
    if std::env::var_os(CHILD).is_none() {
        return;
    }
    for (what, date) in dates_made_now() {
        println!("date|{what}|{date}");
    }
}

#[test]
fn visible_dates_are_local() {
    // 14 hours ahead of UTC and 11 behind: 25 hours apart, so at any moment at least one
    // of them is on a different date from UTC. POSIX TZ strings need no time zone files.
    for (tz, hours) in [("<+14>-14", 14), ("<-11>11", -11)] {
        let day = || {
            (chrono::Utc::now() + chrono::Duration::hours(hours))
                .format("%Y-%m-%d")
                .to_string()
        };
        let before = day();
        let out = Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "print_dates_made_now", "--ignored"])
            .args(["--nocapture", "--test-threads", "1"])
            .env("TZ", tz)
            .env(CHILD, "1")
            .output()
            .unwrap();
        let after = day();
        let text = String::from_utf8_lossy(&out.stdout);
        let errors = String::from_utf8_lossy(&out.stderr);
        assert!(out.status.success(), "{text}{errors}");
        let dates: Vec<(&str, &str)> = text
            .lines()
            // The first follows the test's name on its line.
            .filter_map(|l| l.split_once("date|").map(|(_, d)| d))
            .filter_map(|l| l.split_once('|'))
            .collect();
        assert_eq!(dates.len(), 5, "{text}");
        for (what, date) in dates {
            // Midnight there may come while the child runs.
            assert!(
                date == before || date == after,
                "{what} in TZ {tz}: {date}, want {before}"
            );
        }
    }
}

/// When comments were last changed, as written in the file: in UTC, as LimarePDF writes
/// them; an hour ahead of UTC; with no time zone; and a date alone.
const WRITTEN: [&str; 4] = [
    "D:20261029033300Z",
    "D:20240305142233+01'00'",
    "D:20240305142233",
    "D:20240305",
];

/// Run by [`comment_dates_are_shown_in_local_time`] in a time zone of its own.
#[test]
#[ignore = "run by comment_dates_are_shown_in_local_time with TZ set"]
fn print_comment_dates() {
    if std::env::var_os(CHILD).is_none() {
        return;
    }
    let mut p = Pdf::new();
    p.insert_blank(0, 300.0, 200.0).unwrap();
    for (i, written) in WRITTEN.into_iter().enumerate() {
        let at = (40.0 + 50.0 * i as f32, 100.0);
        let id = p.add_note(0, at, [1.0, 0.8, 0.0], "A note", "Sam").unwrap();
        let note = p.doc.get_object_mut(id).unwrap().as_dict_mut().unwrap();
        note.set("M", lopdf::Object::string_literal(written));
    }
    for a in p.annotations() {
        println!("date|{}", a.modified);
    }
}

#[test]
fn comment_dates_are_shown_in_local_time() {
    // A note made at 03:33 UTC on 29 October is shown at that moment's time there; a date
    // without a time zone, as written.
    for (tz, want) in [
        (
            "<+14>-14",
            [
                "2026-10-29 17:33",
                "2024-03-06 03:22",
                "2024-03-05 14:22",
                "2024-03-05",
            ],
        ),
        (
            "<-11>11",
            [
                "2026-10-28 16:33",
                "2024-03-05 02:22",
                "2024-03-05 14:22",
                "2024-03-05",
            ],
        ),
    ] {
        let out = Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "print_comment_dates", "--ignored"])
            .args(["--nocapture", "--test-threads", "1"])
            .env("TZ", tz)
            .env(CHILD, "1")
            .output()
            .unwrap();
        let text = String::from_utf8_lossy(&out.stdout);
        let errors = String::from_utf8_lossy(&out.stderr);
        assert!(out.status.success(), "{text}{errors}");
        let shown: Vec<&str> = text
            .lines()
            .filter_map(|l| l.split_once("date|").map(|(_, d)| d))
            .collect();
        assert_eq!(shown, want, "in TZ {tz}, for {WRITTEN:?}");
    }
}

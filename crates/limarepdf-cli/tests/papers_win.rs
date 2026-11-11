//! A Windows printer's paper sizes, read by a copy of the program (C-01), with the stand-in
//! printer of `papers_seam`: a driver writing past its buffers, a list that changes while it
//! is read, and a copy that hangs or whose answer is too long, malformed or cut off each give
//! an error saying what happened, and the program asking goes on.
#![cfg(windows)]

use limarepdf_core::papers_seam::{in_process, through, PRINTER, VAR};
use limarepdf_core::print::{self, PaperList, PAPERS_ARG};
use std::path::Path;
use std::process::Command;
use std::time::{Duration, Instant};

const CLI: &str = env!("CARGO_BIN_EXE_limarepdf-cli");

/// `printers --paper` with the stand-in printer doing `how`: its output and its warnings. It
/// exits 0 whatever reading the paper sizes did.
fn printers(how: &str) -> (String, String) {
    let out = limarepdf_core::child_seam::output(
        Command::new(CLI)
            .args(["printers", "--paper"])
            .env(VAR, how)
            .env_remove("RUST_BACKTRACE"),
    )
    .unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
    assert_eq!(
        out.status.code(),
        Some(0),
        "stdout: {stdout}\nstderr: {stderr}"
    );
    (stdout, stderr)
}

/// The CLI, to be started as a copy reading paper sizes, with the stand-in doing `how`.
fn copy(how: &str) -> Command {
    let mut cmd = Command::new(CLI);
    cmd.env(VAR, how);
    cmd
}

/// A driver that writes more papers than it said it had (C-01) ends only the copy reading
/// them, at the inaccessible page after its buffers: the CLI says so and goes on. Read in the
/// CLI's own process, as before the copy, the same write ended the CLI.
#[test]
fn a_driver_writing_past_its_buffers_ends_only_the_copy() {
    let (out, err) = printers("grows");
    assert_eq!(out, "LimarePDF stand-in (default)\n");
    assert_eq!(
        err,
        "warning: couldn't read the paper sizes of LimarePDF stand-in: reading the printer's \
         paper sizes stopped unexpectedly (exception 0xC0000005)\n"
    );
}

/// A list that changed while it was read isn't shown: the copy says so.
#[test]
fn a_list_that_changes_while_it_is_read_is_not_shown() {
    let (out, err) = printers("changes");
    assert_eq!(out, "LimarePDF stand-in (default)\n");
    assert_eq!(
        err,
        "warning: couldn't read the paper sizes of LimarePDF stand-in: the printer's list of \
         paper sizes changed while it was read\n"
    );
}

/// The papers a copy answers with are listed as the CLI listed the driver's before.
#[test]
fn the_papers_a_copy_answers_with_are_listed() {
    let (out, err) = printers("lists");
    assert_eq!(err, "");
    assert_eq!(
        out,
        "LimarePDF stand-in (default)\n  Letter (8.5 × 11 in)\n  A4 (210 × 297 mm), default\n"
    );
}

/// An answer too long, malformed or cut off isn't used, and the error says why.
#[test]
fn a_bad_answer_is_not_used() {
    let unusable = "reading the printer's paper sizes gave an answer LimarePDF can't use: ";
    for (how, why) in [
        ("floods", "it is longer than 1 MiB"),
        (
            "garbles",
            "paper 2: its size isn't two positive whole numbers",
        ),
        ("cuts", "it is cut off"),
    ] {
        let e = through(copy(how), PRINTER, Duration::from_secs(60)).unwrap_err();
        assert_eq!(format!("{e:#}"), format!("{unusable}{why}"), "{how}");
    }
}

/// A copy that never answers is stopped at the limit, and holds nothing after: the file it
/// held open, unshared, can then be removed.
#[test]
fn a_copy_that_never_answers_is_stopped() {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join("papers-win-hangs");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let held = dir.join("held");
    let how = format!("hangs:{}", held.display());
    let limit = Duration::from_secs(5);
    let asking = std::thread::spawn(move || through(copy(&how), PRINTER, limit));
    let start = Instant::now();
    while std::fs::read_dir(&dir).unwrap().next().is_none() {
        assert!(start.elapsed() < limit, "the copy didn't open the file");
        std::thread::sleep(Duration::from_millis(20));
    }
    assert!(
        std::fs::remove_file(&held).is_err(),
        "the copy holds the file while it waits"
    );
    let e = asking.join().unwrap().unwrap_err();
    assert_eq!(
        format!("{e:#}"),
        "the printing service didn't answer within 5 seconds when asked for the printer's \
         paper sizes"
    );
    let start = Instant::now();
    while let Err(e) = std::fs::remove_file(&held) {
        assert!(
            start.elapsed() < Duration::from_secs(5),
            "the copy still holds the file: {e}"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// A program whose `main` doesn't answer the argument (this test's own) gets an error, asked
/// for paper sizes or started as a copy, rather than reading them in its own process.
#[test]
fn a_program_that_does_not_answer_gets_an_error() {
    let e = print::papers(PRINTER).unwrap_err();
    assert_eq!(
        format!("{e:#}"),
        "this program can't read a Windows printer's paper sizes: its `main` doesn't call \
         `print::answer_if_asked`"
    );
    let me = std::env::current_exe().unwrap();
    let e = through(Command::new(me), PRINTER, Duration::from_secs(60)).unwrap_err();
    let e = format!("{e:#}");
    assert!(e.contains(PAPERS_ARG.trim_start_matches('-')), "{e}");
}

/// Each printer the system has lists the same papers through a copy as read in this process,
/// as the CLI and the app read them before the copy, apart from margins only one of them
/// measured in the time it has (it stops measuring after 3 seconds). The times are logged as
/// a record, not a limit. With no printers it logs that it didn't run.
#[test]
fn a_printers_papers_are_the_same_through_a_copy() {
    let printers = print::printers().unwrap();
    if printers.is_empty() {
        eprintln!("not run: the system has no printers");
    }
    for p in printers {
        let start = Instant::now();
        let here = in_process(&p.name);
        let here_took = start.elapsed();
        let start = Instant::now();
        let copied = through(Command::new(CLI), &p.name, Duration::from_secs(60));
        let copy_took = start.elapsed();
        eprintln!(
            "{}: read in this process in {here_took:?}, through a copy in {copy_took:?}",
            p.name
        );
        match (here, copied) {
            (Ok(a), Ok(b)) => same(&p.name, &a, &b),
            (Err(a), Err(b)) => eprintln!("{}: not read: {a:#}; through a copy: {b:#}", p.name),
            (a, b) => panic!("{}: in this process {a:?}, through a copy {b:?}", p.name),
        }
    }
}

fn same(printer: &str, a: &PaperList, b: &PaperList) {
    assert_eq!(a.default, b.default, "{printer}");
    assert_eq!(a.papers.len(), b.papers.len(), "{printer}");
    let measured = |l: &PaperList| l.papers.iter().filter(|p| p.margins.is_some()).count();
    eprintln!(
        "{printer}: {} papers, margins measured for {} here and {} through a copy",
        a.papers.len(),
        measured(a),
        measured(b)
    );
    for (p, q) in a.papers.iter().zip(&b.papers) {
        assert_eq!(
            (&p.id, &p.label, p.size),
            (&q.id, &q.label, q.size),
            "{printer}"
        );
        if let (Some(m), Some(n)) = (p.margins, q.margins) {
            assert_eq!(m, n, "{printer}: {}", p.label);
        }
    }
}

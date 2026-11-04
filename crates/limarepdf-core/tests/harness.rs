//! The test helpers themselves, on small made-up cases: an outside program that hangs, one
//! that writes a lot, and the folders tests work in.

mod common;

use common::apart::{self, Ran};
use common::tool::{self, Scratch};
use std::process::Command;
use std::time::{Duration, Instant};

/// A program that writes more than a pipe holds, to both outputs, isn't stopped by it.
#[cfg(unix)]
#[test]
fn a_program_writing_a_lot_finishes() {
    let out = tool::output(Command::new("sh").args([
        "-c",
        "head -c 3000000 /dev/zero; head -c 2000000 /dev/zero >&2; exit 3",
    ]))
    .unwrap();
    assert_eq!(out.status.code(), Some(3));
    assert_eq!(out.stdout.len(), 3_000_000);
    assert_eq!(out.stderr.len(), 2_000_000);
}

/// Input larger than a pipe holds reaches the program while its output is read.
#[cfg(unix)]
#[test]
fn input_reaches_the_program() {
    let input = vec![b'x'; 1_000_000];
    let out = tool::output_with_input(&mut Command::new("cat"), &input).unwrap();
    assert!(out.status.success());
    assert_eq!(out.stdout, input);
}

/// A program that hangs fails its test at the deadline, and what it started is stopped too:
/// the call doesn't wait for the background `sleep`, which holds the pipes open.
#[cfg(unix)]
#[test]
fn a_hanging_program_fails_at_the_deadline() {
    use std::panic::{catch_unwind, AssertUnwindSafe};
    let start = Instant::now();
    let failed = catch_unwind(AssertUnwindSafe(|| {
        tool::output_within(
            Command::new("sh").args(["-c", "sleep 60 & sleep 60"]),
            Duration::from_millis(300),
        )
    }));
    let why = failed.expect_err("the call should fail");
    let why = why.downcast_ref::<String>().cloned().unwrap_or_default();
    assert!(why.contains("sh didn't finish within"), "{why}");
    assert!(
        start.elapsed() < Duration::from_secs(10),
        "{:?}",
        start.elapsed()
    );
}

/// A program that isn't installed is an error, not a failed test.
#[test]
fn a_missing_program_is_an_error() {
    assert!(tool::output(&mut Command::new("limarepdf-no-such-program")).is_err());
    assert!(!tool::installed("limarepdf-no-such-program", "-v"));
}

/// The child process the next tests start: what `HARNESS_CHILD` asks for.
#[test]
#[ignore = "started by the tests below"]
fn harness_child() {
    match std::env::var("HARNESS_CHILD").as_deref() {
        Ok("report") => {
            println!("harness|one|a");
            println!("harness|done");
        }
        Ok("hang") => std::thread::sleep(Duration::from_secs(60)),
        Ok("die") => std::process::exit(7),
        _ => {}
    }
}

/// A child's report lines reach the parent.
#[test]
fn a_child_reports() {
    let env = [("HARNESS_CHILD", "report".to_string())];
    let Ran::Ended(text, status) = apart::run("harness_child", &env, Duration::from_secs(60))
    else {
        panic!("the child didn't end by itself");
    };
    assert!(status.success());
    assert_eq!(apart::reports(&text, "harness|"), ["one|a", "done"]);
}

/// A child that hangs is stopped at its time, and the parent goes on.
#[test]
fn a_hanging_child_is_stopped() {
    let start = Instant::now();
    let env = [("HARNESS_CHILD", "hang".to_string())];
    let ran = apart::run("harness_child", &env, Duration::from_millis(500));
    assert!(matches!(ran, Ran::TimedOut));
    assert!(
        start.elapsed() < Duration::from_secs(10),
        "{:?}",
        start.elapsed()
    );
}

/// A child that ends without reporting says how it ended.
#[test]
fn a_child_ending_early_says_how() {
    let env = [("HARNESS_CHILD", "die".to_string())];
    let Ran::Ended(text, status) = apart::run("harness_child", &env, Duration::from_secs(60))
    else {
        panic!("the child didn't end by itself");
    };
    assert!(apart::reports(&text, "harness|").is_empty());
    assert_eq!(
        apart::why_stopped(status),
        "the check stopped with status 7"
    );
}

/// Each folder is new and its own, and goes when the test is done with it.
#[test]
fn scratch_folders_are_separate_and_removed() {
    let (a, b) = (Scratch::new("same"), Scratch::new("same"));
    assert_ne!(a.path(), b.path());
    assert!(a.path().starts_with(env!("CARGO_TARGET_TMPDIR")));
    std::fs::write(a.join("f"), b"x").unwrap();
    let kept = a.path().to_path_buf();
    drop(a);
    assert!(!kept.exists());
    assert!(b.path().is_dir());
}

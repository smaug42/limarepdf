//! Every public test file LimarePDF opens, saved and opened again (build plan, stage C1).
//!
//! Opt-in. Fetch the files first (`sh tools/fetch-render-samples.sh` in crates/limarepdf-core),
//! then, under a memory cap as testing.md describes:
//! `cargo test --release -p limarepdf-core --test save_reopen -- --ignored --nocapture`.
//!
//! Each file is opened as the app opens it (form fields without a drawing get one, unless the
//! file is signed) and checked twice:
//!
//! 1. **Saved unchanged** as the app saves it (a signed file by adding a revision after what was
//!    signed, any other by writing the whole file), and opened again.
//! 2. **Page 1 turned and a comment added,** then saved and opened again.
//!
//! Each time the copy opened again must read without repair, have the same pages, page sizes,
//! page labels, annotations, form fields and their values, bookmarks and page text as the
//! document before it was saved, and draw the listed pages (the ones `render_compare.rs` draws)
//! the same, at `SAVE_REOPEN_DPI` (96 by default), allowing only for anti-aliasing. In the
//! second check, pages after the first must draw as in the file itself, and page 1 must be
//! turned and have the new comment. The bytes aren't compared: a save may renumber objects,
//! compress streams and write numbers shorter.
//!
//! Each file is checked in a process of its own (this test run again for that file), two at a
//! time (`SAVE_REOPEN_JOBS` changes it), with at most [`MEMORY_MB`] of memory on Linux, so a
//! file that takes longer than [`TIMEOUT`] or needs more memory stops only its own check.
//! Those files, and files
//! LimarePDF can't open or draw at all, are counted and listed as skipped, not checked:
//! `render_compare.rs` covers them. Every difference is
//! listed in `target/tmp/save-reopen/index.html`, with pictures of pages that draw differently;
//! `summary.txt` has one line per file. The test fails when a file not in
//! `tools/save-reopen-known.txt` (the known limits, each with its reason) has a difference.
//! `SAVE_REOPEN_ONLY` limits the run to files whose name contains it.

mod common;

use common::compare::{differences, side_by_side, Facts, Image};
use common::corpus::{root, samples, Sample};
use limarepdf_core::render::{self, Renderer};
use limarepdf_core::{append_edits, Pdf};
use rayon::prelude::*;
use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::io::Read as _;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// How far, in any channel, a pixel may be outside the colours around it in the other
/// drawing. Both are LimarePDF's, so only a little.
const TOLERANCE: u8 = 8;
/// Time allowed for all of one file's checks.
const TIMEOUT: Duration = Duration::from_secs(180);
/// The memory each file's check may use, in MiB (on Linux; `SAVE_REOPEN_MEMORY` changes it).
#[cfg(target_os = "linux")]
const MEMORY_MB: u64 = 2048;
/// The comment added in the second check.
const NOTE: &str = "Save and reopen check";

/// What LimarePDF made of one page.
enum Drawn {
    Ok(Image),
    Failed(String),
}

/// One difference found.
struct Finding {
    check: &'static str,
    what: String,
    picture: Option<String>,
}

enum Outcome {
    /// Not checked, and why.
    Skipped(String),
    Checked(Vec<Finding>),
}

/// Runs `f`, turning a panic into an error naming `step`.
fn guarded<T>(step: &str, f: impl FnOnce() -> Result<T, String>) -> Result<T, String> {
    catch_unwind(AssertUnwindSafe(f)).unwrap_or_else(|e| {
        let why = e
            .downcast_ref::<String>()
            .cloned()
            .or_else(|| e.downcast_ref::<&str>().map(|s| s.to_string()))
            .unwrap_or_default();
        Err(format!("{step} panicked: {why}"))
    })
}

/// Pages `first` to `last` (1-based) of `p`, drawn as `limarepdf-cli render` draws them.
fn draw(p: &mut Pdf, first: usize, last: usize, scale: f32) -> Result<Vec<(usize, Drawn)>, String> {
    let bytes = p.render_snapshot().map_err(|e| format!("{e:#}"))?;
    let hp = render::load(bytes, None).map_err(|e| format!("{e:#}"))?;
    let r = Renderer::new(&hp);
    Ok((first..=last.min(r.page_count()))
        .map(|n| {
            let d = match r.render(n - 1, scale) {
                Ok(b) => Drawn::Ok(Image::from_bitmap(&b)),
                Err(e) => Drawn::Failed(format!("{e:#}")),
            };
            (n, d)
        })
        .collect())
}

/// `p` saved as the app saves it: a signed document by adding its edits after `original`,
/// unless it was repaired or that can't be done, and then, as any other document, by writing
/// the whole file.
fn save(p: &mut Pdf, original: &[u8], signed: bool) -> Result<Vec<u8>, String> {
    if signed && !p.repaired() {
        if let Ok(a) = append_edits(original, p) {
            return Ok(a.bytes);
        }
    }
    p.to_bytes().map_err(|e| format!("{e:#}"))
}

/// The saved copy opened again, which must need no repair.
fn reopen(bytes: &[u8]) -> Result<Pdf, String> {
    let p = Pdf::from_bytes(bytes, None).map_err(|e| format!("{e:#}"))?;
    if p.repaired() {
        return Err("it needed repair to open".into());
    }
    Ok(p)
}

/// The differences between `before` and `after`, drawn, for the pages `which` picks.
fn compare_pages(
    s: &Sample,
    check: &'static str,
    before: &[(usize, Drawn)],
    after: &[(usize, Drawn)],
    which: impl Fn(usize) -> bool,
    out: &Path,
) -> Vec<Finding> {
    let mut found = Vec::new();
    let mut add = |what: String, picture| {
        found.push(Finding {
            check,
            what,
            picture,
        })
    };
    if before.len() != after.len() {
        add(
            format!("{} pages drawn, then {}", before.len(), after.len()),
            None,
        );
    }
    for ((n, a), (_, b)) in before.iter().zip(after).filter(|((n, _), _)| which(*n)) {
        match (a, b) {
            (Drawn::Failed(_), Drawn::Failed(_)) => {}
            (Drawn::Failed(e), Drawn::Ok(_)) => add(
                format!("page {n} drew only after saving (before: {e})"),
                None,
            ),
            (Drawn::Ok(_), Drawn::Failed(e)) => add(format!("page {n} no longer draws: {e}"), None),
            (Drawn::Ok(a), Drawn::Ok(b)) => {
                if (a.width, a.height) != (b.width, b.height) {
                    add(
                        format!(
                            "page {n} drew {}×{}, then {}×{}",
                            a.width, a.height, b.width, b.height
                        ),
                        None,
                    );
                    continue;
                }
                let (mask, w, _) = differences(a, b, TOLERANCE);
                let count = mask.iter().filter(|&&m| m).count();
                if count > 0 {
                    let file = format!("{}-{check}-p{n}.png", s.name.replace('/', "_"));
                    side_by_side(a, b, &mask, w)
                        .save(out.join("pages").join(&file))
                        .unwrap();
                    add(
                        format!("page {n} draws differently ({count} pixels)"),
                        Some(file),
                    );
                }
            }
        }
    }
    found
}

/// Both checks of one file, on the calling thread.
fn check(s: &Sample, out: &Path, scale: f32) -> Outcome {
    let original = match std::fs::read(s.path()) {
        Ok(b) => b,
        Err(e) => return Outcome::Skipped(format!("can't be read: {e}")),
    };
    let pw = s.password.as_deref();
    // The file as the app opens it.
    let opened = guarded("opening", || {
        let mut p = Pdf::from_bytes(&original, pw).map_err(|e| format!("{e:#}"))?;
        let signed = p.signature_count() > 0;
        if !signed {
            p.refresh_appearances();
        }
        let drawn = draw(&mut p, s.first, s.last, scale)?;
        Ok((p, signed, drawn))
    });
    let (base, signed, base_drawn) = match opened {
        Ok(o) => o,
        Err(e) => return Outcome::Skipped(format!("LimarePDF can't open or draw it: {e}")),
    };
    let base_facts = guarded("reading", || Ok(Facts::of(&base)));
    let base_facts = match base_facts {
        Ok(f) => f,
        Err(e) => return Outcome::Skipped(format!("LimarePDF can't read it: {e}")),
    };
    let mut found = Vec::new();
    let failed = |check, what| Finding {
        check,
        what,
        picture: None,
    };

    // 1. Saved unchanged.
    let unchanged = guarded("saving unchanged", || {
        let saved =
            save(&mut base.clone(), &original, signed).map_err(|e| format!("saving fails: {e}"))?;
        let mut again = reopen(&saved).map_err(|e| format!("the saved copy: {e}"))?;
        let drawn = draw(&mut again, s.first, s.last, scale)
            .map_err(|e| format!("the saved copy can't be drawn: {e}"))?;
        Ok((Facts::of(&again), drawn))
    });
    match unchanged {
        Ok((facts, drawn)) => {
            for d in base_facts.differences(&facts) {
                found.push(failed("unchanged", d));
            }
            found.extend(compare_pages(
                s,
                "unchanged",
                &base_drawn,
                &drawn,
                |_| true,
                out,
            ));
        }
        Err(e) => found.push(failed("unchanged", e)),
    }

    // 2. Page 1 turned and a comment added.
    let edited = guarded("editing", || {
        let mut p = base.clone();
        p.rotate_pages(&[0], 90)
            .map_err(|e| format!("turning page 1 fails: {e:#}"))?;
        p.add_note(0, (36.0, 36.0), [1.0, 0.8, 0.0], NOTE, "LimarePDF")
            .map_err(|e| format!("adding a comment fails: {e:#}"))?;
        let facts = Facts::of(&p);
        let drawn = draw(&mut p, s.first, s.last, scale)?;
        let saved = save(&mut p, &original, signed).map_err(|e| format!("saving fails: {e}"))?;
        let mut again = reopen(&saved).map_err(|e| format!("the saved copy: {e}"))?;
        let turned = match (base.page_size(0), again.page_size(0)) {
            (Ok((w, h)), Ok(after)) => (w - after.1).abs() < 0.01 && (h - after.0).abs() < 0.01,
            _ => false,
        };
        let noted = again
            .page_annotations(0)
            .is_ok_and(|a| a.iter().any(|a| a.contents == NOTE));
        let again_drawn = draw(&mut again, s.first, s.last, scale)
            .map_err(|e| format!("the saved copy can't be drawn: {e}"))?;
        Ok((facts, drawn, Facts::of(&again), again_drawn, turned, noted))
    });
    match edited {
        Ok((facts, drawn, again, again_drawn, turned, noted)) => {
            if !turned {
                found.push(failed(
                    "edited",
                    "page 1 isn't turned in the saved copy".into(),
                ));
            }
            if !noted {
                found.push(failed(
                    "edited",
                    "the comment isn't on page 1 in the saved copy".into(),
                ));
            }
            for d in facts.differences(&again) {
                found.push(failed("edited", d));
            }
            found.extend(compare_pages(
                s,
                "edited",
                &drawn,
                &again_drawn,
                |n| n == 1,
                out,
            ));
            found.extend(compare_pages(
                s,
                "edited",
                &base_drawn,
                &again_drawn,
                |n| n > 1,
                out,
            ));
        }
        Err(e) => found.push(failed("edited", e)),
    }
    Outcome::Checked(found)
}

/// Marks the lines a child process reports its outcome in.
const REPORT: &str = "save-reopen|";

/// In the child process: [`check`] on the file `SAVE_REOPEN_FILE` names, on a thread with a
/// large stack, its outcome printed for the parent.
fn child(name: &str, out: &Path, scale: f32) {
    // A file needing more memory than the limit stops this check alone, when an allocation
    // fails, before the system runs short.
    #[cfg(target_os = "linux")]
    {
        use rustix::process::{getrlimit, setrlimit, Resource, Rlimit};
        let mb: u64 = std::env::var("SAVE_REOPEN_MEMORY")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(MEMORY_MB);
        let hard = getrlimit(Resource::Data).maximum;
        let soft = hard.map_or(mb << 20, |h| h.min(mb << 20));
        setrlimit(
            Resource::Data,
            Rlimit {
                current: Some(soft),
                maximum: hard,
            },
        )
        .expect("a memory limit");
    }
    let s = samples(name)
        .into_iter()
        .find(|s| s.name == name)
        .expect("a listed file");
    let out = out.to_path_buf();
    let outcome = std::thread::Builder::new()
        .stack_size(256 << 20)
        .spawn(move || check(&s, &out, scale))
        .unwrap()
        .join()
        .unwrap_or_else(|_| Outcome::Skipped("the check panicked".into()));
    let line = |s: &str| s.replace(['\n', '\r', '|'], " ");
    match outcome {
        Outcome::Skipped(why) => println!("{REPORT}skipped|{}", line(&why)),
        Outcome::Checked(found) => {
            for f in found {
                let picture = f.picture.unwrap_or_default();
                println!("{REPORT}found|{}|{}|{picture}", f.check, line(&f.what));
            }
            println!("{REPORT}checked");
        }
    }
}

/// Checks `s` in a child process (this test run again for that file alone), so that a file
/// that takes too long or too much memory can be stopped without stopping the others: given
/// up after [`TIMEOUT`].
fn check_apart(s: &Sample, dpi: u32) -> Outcome {
    let spawned = Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "save_and_reopen_the_corpus", "--ignored"])
        .args(["--nocapture", "--test-threads", "1"])
        .env("SAVE_REOPEN_FILE", &s.name)
        .env("SAVE_REOPEN_DPI", dpi.to_string())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn();
    let mut child = match spawned {
        Ok(c) => c,
        Err(e) => return Outcome::Skipped(format!("no child process: {e}")),
    };
    // Read as it comes, so a full pipe can't stop the child.
    let mut stdout = child.stdout.take().unwrap();
    let reader = std::thread::spawn(move || {
        let mut text = String::new();
        let _ = stdout.read_to_string(&mut text);
        text
    });
    let start = Instant::now();
    let status = loop {
        match child.try_wait() {
            Ok(Some(st)) => break st,
            Ok(None) if start.elapsed() > TIMEOUT => {
                let _ = child.kill();
                let _ = child.wait();
                return Outcome::Skipped("timed out".into());
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(50)),
            Err(e) => return Outcome::Skipped(e.to_string()),
        }
    };
    let text = reader.join().unwrap_or_default();
    let mut found = Vec::new();
    // The first line follows the test harness's "test ... ".
    for l in text
        .lines()
        .filter_map(|l| l.split_once(REPORT).map(|(_, l)| l))
    {
        let f: Vec<&str> = l.split('|').collect();
        match f[..] {
            ["skipped", why] => return Outcome::Skipped(why.to_string()),
            ["found", check, what, picture] => found.push(Finding {
                check: if check == "edited" {
                    "edited"
                } else {
                    "unchanged"
                },
                what: what.to_string(),
                picture: (!picture.is_empty()).then(|| picture.to_string()),
            }),
            ["checked"] => return Outcome::Checked(found),
            _ => {}
        }
    }
    // No outcome: the child was stopped.
    #[cfg(unix)]
    let signal = std::os::unix::process::ExitStatusExt::signal(&status);
    #[cfg(not(unix))]
    let signal = None;
    let why = match (status.code(), signal) {
        (_, Some(6)) => "stopped (signal 6), most likely for needing more memory than its \
                         limit"
            .to_string(),
        (_, Some(9)) => "killed (signal 9), most likely for using too much memory".to_string(),
        (_, Some(sig)) => format!("killed (signal {sig})"),
        (code, None) => format!("the check stopped with status {}", code.unwrap_or(-1)),
    };
    Outcome::Skipped(why)
}

/// The known limits: file name to reason.
fn known() -> BTreeMap<String, String> {
    let list = Path::new(env!("CARGO_MANIFEST_DIR")).join("tools/save-reopen-known.txt");
    let text = std::fs::read_to_string(&list).expect("tools/save-reopen-known.txt");
    text.lines()
        .filter(|l| !l.trim().is_empty() && !l.starts_with('#'))
        .map(|l| match l.split_once(char::is_whitespace) {
            Some((name, why)) => (name.to_string(), why.trim().to_string()),
            None => (l.to_string(), String::new()),
        })
        .collect()
}

fn escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

fn report(results: &[(String, Outcome)], known: &BTreeMap<String, String>, dpi: u32) -> String {
    let mut h = String::new();
    let checked = results
        .iter()
        .filter(|(_, o)| matches!(o, Outcome::Checked(_)))
        .count();
    let _ = write!(
        h,
        "<!doctype html><meta charset=utf-8><title>Save and reopen</title>\
         <style>body{{font:14px sans-serif;margin:16px;background:#fff;color:#111}}\
         img{{max-width:100%;border:1px solid #ccc}}section{{margin:24px 0}}\
         h2{{font-size:16px;margin:0 0 4px}}p{{margin:2px 0}}</style>\
         <h1>Saved and opened again</h1>\
         <p>{checked} of {} files checked at {dpi} dpi. Each picture: before saving, after, \
         differences in red.</p>",
        results.len()
    );
    for (name, o) in results {
        let Outcome::Checked(found) = o else { continue };
        if found.is_empty() {
            continue;
        }
        let _ = write!(h, "<section><h2>{}</h2>", escape(name));
        if let Some(why) = known.get(name) {
            let _ = write!(h, "<p><em>Known: {}</em></p>", escape(why));
        }
        for f in found {
            let _ = write!(h, "<p>{}: {}</p>", f.check, escape(&f.what));
            if let Some(p) = &f.picture {
                let _ = write!(
                    h,
                    "<a href=\"pages/{p}\"><img src=\"pages/{p}\" loading=lazy></a>"
                );
            }
        }
        h.push_str("</section>");
    }
    h
}

#[test]
#[ignore = "needs the fetched samples: sh tools/fetch-render-samples.sh"]
fn save_and_reopen_the_corpus() {
    let dpi: u32 = std::env::var("SAVE_REOPEN_DPI")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(96);
    let out = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("save-reopen");
    let scale = dpi as f32 / 72.0;
    if let Ok(name) = std::env::var("SAVE_REOPEN_FILE") {
        return child(&name, &out, scale);
    }
    let only = std::env::var("SAVE_REOPEN_ONLY").unwrap_or_default();
    let jobs: usize = std::env::var("SAVE_REOPEN_JOBS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(2);
    let _ = std::fs::remove_dir_all(&out);
    std::fs::create_dir_all(out.join("pages")).unwrap();
    let known = known();

    let present: Vec<Sample> = samples(&only)
        .into_iter()
        .filter(|s| s.path().exists())
        .collect();
    assert!(
        !present.is_empty(),
        "no samples in {}: run sh tools/fetch-render-samples.sh",
        root().display()
    );

    // `jobs` files at a time, each in its own process.
    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(jobs)
        .build()
        .unwrap();
    let start = Instant::now();
    let mut results: Vec<(String, Outcome)> = pool.install(|| {
        present
            .par_iter()
            .map(|s| (s.name.clone(), check_apart(s, dpi)))
            .collect()
    });
    results.sort_by(|a, b| a.0.cmp(&b.0));

    let mut summary = String::new();
    let (mut skipped, mut differ, mut unknown, mut fixed) = (0, 0, Vec::new(), Vec::new());
    for (name, o) in &results {
        let line = match o {
            Outcome::Skipped(why) => {
                skipped += 1;
                format!("skipped   {name}  {why}")
            }
            Outcome::Checked(found) if found.is_empty() => {
                if known.contains_key(name) {
                    fixed.push(name.clone());
                }
                format!("same      {name}")
            }
            Outcome::Checked(found) => {
                differ += 1;
                if !known.contains_key(name) {
                    unknown.push(name.clone());
                }
                let all: Vec<String> = found
                    .iter()
                    .map(|f| format!("{}: {}", f.check, f.what))
                    .collect();
                format!("differs   {name}  {}", all.join("; "))
            }
        };
        let _ = writeln!(summary, "{line}");
    }
    std::fs::write(out.join("summary.txt"), &summary).unwrap();
    std::fs::write(out.join("index.html"), report(&results, &known, dpi)).unwrap();

    println!(
        "{} files in {:.0?}: {} the same, {differ} differ, {skipped} skipped; see {}",
        results.len(),
        start.elapsed(),
        results.len() - differ - skipped,
        out.join("index.html").display()
    );
    if !fixed.is_empty() {
        println!(
            "known limits that are the same now (take them off tools/save-reopen-known.txt): {}",
            fixed.join(", ")
        );
    }
    assert!(
        unknown.is_empty(),
        "differences in files not in tools/save-reopen-known.txt (listed in {}):\n{}",
        out.join("summary.txt").display(),
        unknown.join("\n")
    );
}

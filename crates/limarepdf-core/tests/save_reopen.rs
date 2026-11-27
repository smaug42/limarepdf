//! Every public test file LimarePDF opens, saved and opened again (build plan, stage C1).
//!
//! Opt-in. Fetch the files first (`sh tools/fetch-render-samples.sh` in crates/limarepdf-core),
//! then, under a memory cap as docs/developer/testing.md describes:
//! `cargo test --release -p limarepdf-core --test all -- --ignored --nocapture
//! save_reopen::`.
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
//! Each file is checked in a process of its own (this test run again for that file, through
//! `common/apart.rs`), two at a time (`SAVE_REOPEN_JOBS` changes it), with at most
//! [`MEMORY_MB`] of memory on Linux, so a file that takes longer than [`TIMEOUT`] or needs
//! more memory stops only its own check, and is stopped. Those files, and files
//! LimarePDF can't open or draw at all, are counted and listed as skipped, not checked:
//! `render_compare.rs` covers them. A page drawn neither before nor after saving can't be
//! compared: it is listed and counted, and its file is `undrawn` in summary.txt, not
//! different. Every difference is listed in `target/tmp/save-reopen/index.html`, with
//! pictures of pages that draw differently, with every page not drawn and, for a signed file,
//! each save that wrote the whole file instead of adding a revision; `summary.txt` has one
//! line per file. The counts of files checked, the same, different, skipped and not
//! downloaded, of pages compared and not drawn, and of signed files written whole are printed.
//! The test fails on a difference that isn't in `tools/save-reopen-known.txt` (the known
//! limits: each names a file, the text of its difference and the reason, so another
//! difference in the same file still fails), and when it compares no page or, from
//! [`MIN_FILES`] files, more than a tenth of them are skipped or have no page compared.
//! `SAVE_REOPEN_ONLY` limits the run to files whose name contains it.

use crate::common;

use common::apart;
use common::compare::{differences, side_by_side, Facts, Image};
use common::corpus::{root, samples, Sample};
use limarepdf_core::render::{self, Renderer};
use limarepdf_core::{append_edits, Pdf};
use rayon::prelude::*;
use std::fmt::Write as _;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

/// How far, in any channel, a pixel may be outside the colours around it in the other
/// drawing. Both are LimarePDF's, so only a little.
const TOLERANCE: u8 = 8;
/// Time allowed for all of one file's checks.
const TIMEOUT: Duration = Duration::from_secs(180);
/// The memory each file's check may use, in MiB (on Linux; `SAVE_REOPEN_MEMORY` changes it).
const MEMORY_MB: u64 = 2048;
/// From this many files a run fails when it skips more than a tenth of them.
const MIN_FILES: usize = 20;
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

/// A page drawn neither before nor after saving, so not compared.
struct Undrawn {
    check: &'static str,
    page: usize,
    why: String,
}

/// How a signed document's save added a revision; any other way says why.
const APPENDED: &str = "a revision added";

/// What checking a file found.
#[derive(Default)]
struct Checked {
    found: Vec<Finding>,
    undrawn: Vec<Undrawn>,
    /// Pages drawn before and after saving and compared, in both checks.
    compared: usize,
    /// A signed document's saves: each check's, and how it saved.
    signed: Vec<(&'static str, String)>,
}

enum Outcome {
    /// Not checked, and why.
    Skipped(String),
    Checked(Checked),
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
/// the whole file. For a signed document, also how it was saved.
fn save(p: &mut Pdf, original: &[u8], signed: bool) -> Result<(Vec<u8>, Option<String>), String> {
    let how = if !signed {
        None
    } else if p.repaired() {
        Some("written whole: it was repaired".to_string())
    } else {
        match append_edits(original, p) {
            Ok(a) => return Ok((a.bytes, Some(APPENDED.into()))),
            Err(e) => Some(format!("written whole: adding a revision fails: {e:#}")),
        }
    };
    let bytes = p.to_bytes().map_err(|e| format!("{e:#}"))?;
    Ok((bytes, how))
}

/// The saved copy opened again, which must need no repair.
fn reopen(bytes: &[u8]) -> Result<Pdf, String> {
    let p = Pdf::from_bytes(bytes, None).map_err(|e| format!("{e:#}"))?;
    if p.repaired() {
        return Err("it needed repair to open".into());
    }
    Ok(p)
}

/// The differences between `before` and `after`, drawn, for the pages `which` picks, added to
/// `c` with the pages compared and those drawn neither time.
fn compare_pages(
    s: &Sample,
    check: &'static str,
    before: &[(usize, Drawn)],
    after: &[(usize, Drawn)],
    which: impl Fn(usize) -> bool,
    out: &Path,
    c: &mut Checked,
) {
    let mut add = |what: String, picture| {
        c.found.push(Finding {
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
            (Drawn::Failed(a), Drawn::Failed(b)) => c.undrawn.push(Undrawn {
                check,
                page: *n,
                why: if a == b {
                    a.clone()
                } else {
                    format!("before: {a}; after: {b}")
                },
            }),
            (Drawn::Failed(e), Drawn::Ok(_)) => add(
                format!("page {n} drew only after saving (before: {e})"),
                None,
            ),
            (Drawn::Ok(_), Drawn::Failed(e)) => add(format!("page {n} no longer draws: {e}"), None),
            (Drawn::Ok(a), Drawn::Ok(b)) => {
                c.compared += 1;
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
}

/// Both checks of the file at `path`, `s`, on the calling thread.
fn check(s: &Sample, path: &Path, out: &Path, scale: f32) -> Outcome {
    let original = match std::fs::read(path) {
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
    let mut c = Checked::default();
    let failed = |check, what| Finding {
        check,
        what,
        picture: None,
    };

    // 1. Saved unchanged.
    let unchanged = guarded("saving unchanged", || {
        let (saved, how) =
            save(&mut base.clone(), &original, signed).map_err(|e| format!("saving fails: {e}"))?;
        let mut again = reopen(&saved).map_err(|e| format!("the saved copy: {e}"))?;
        let drawn = draw(&mut again, s.first, s.last, scale)
            .map_err(|e| format!("the saved copy can't be drawn: {e}"))?;
        Ok((Facts::of(&again), drawn, how))
    });
    match unchanged {
        Ok((facts, drawn, how)) => {
            c.signed.extend(how.map(|h| ("unchanged", h)));
            for d in base_facts.differences(&facts) {
                c.found.push(failed("unchanged", d));
            }
            compare_pages(s, "unchanged", &base_drawn, &drawn, |_| true, out, &mut c);
        }
        Err(e) => c.found.push(failed("unchanged", e)),
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
        let (saved, how) =
            save(&mut p, &original, signed).map_err(|e| format!("saving fails: {e}"))?;
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
        Ok((
            facts,
            drawn,
            Facts::of(&again),
            again_drawn,
            turned,
            noted,
            how,
        ))
    });
    match edited {
        Ok((facts, drawn, again, again_drawn, turned, noted, how)) => {
            c.signed.extend(how.map(|h| ("edited", h)));
            if !turned {
                c.found.push(failed(
                    "edited",
                    "page 1 isn't turned in the saved copy".into(),
                ));
            }
            if !noted {
                c.found.push(failed(
                    "edited",
                    "the comment isn't on page 1 in the saved copy".into(),
                ));
            }
            for d in facts.differences(&again) {
                c.found.push(failed("edited", d));
            }
            compare_pages(s, "edited", &drawn, &again_drawn, |n| n == 1, out, &mut c);
            compare_pages(
                s,
                "edited",
                &base_drawn,
                &again_drawn,
                |n| n > 1,
                out,
                &mut c,
            );
        }
        Err(e) => c.found.push(failed("edited", e)),
    }
    Outcome::Checked(c)
}

/// Marks the lines a child process reports its outcome in.
const REPORT: &str = "save-reopen|";

/// In the child process: [`check`] on the file `SAVE_REOPEN_FILE` names, on a thread with a
/// large stack, its outcome printed for the parent.
fn child(name: &str, out: &Path, scale: f32) {
    apart::limit_memory("SAVE_REOPEN_MEMORY", MEMORY_MB);
    let s = samples(name)
        .into_iter()
        .find(|s| s.name == name)
        .expect("a listed file");
    let out = out.to_path_buf();
    let outcome = apart::on_big_stack(move || check(&s, &s.path(), &out, scale))
        .unwrap_or_else(|| Outcome::Skipped("the check panicked".into()));
    for l in lines(&outcome) {
        println!("{REPORT}{l}");
    }
}

/// The lines a child process reports `o` in, as [`outcome`] reads them.
fn lines(o: &Outcome) -> Vec<String> {
    let field = apart::field;
    let c = match o {
        Outcome::Skipped(why) => return vec![format!("skipped|{}", field(why))],
        Outcome::Checked(c) => c,
    };
    let mut l = Vec::new();
    for f in &c.found {
        let picture = f.picture.as_deref().unwrap_or_default();
        l.push(format!("found|{}|{}|{picture}", f.check, field(&f.what)));
    }
    for u in &c.undrawn {
        l.push(format!("undrawn|{}|{}|{}", u.check, u.page, field(&u.why)));
    }
    for (check, how) in &c.signed {
        l.push(format!("signed|{check}|{}", field(how)));
    }
    l.push(format!("checked|{}", c.compared));
    l
}

/// The outcome a child process reported in `lines`, unless it was stopped first.
fn outcome<'a>(lines: impl IntoIterator<Item = &'a str>) -> Option<Outcome> {
    // The check a line names, as `Finding` and `Undrawn` hold it.
    let named = |check| {
        if check == "edited" {
            "edited"
        } else {
            "unchanged"
        }
    };
    let mut c = Checked::default();
    for l in lines {
        let f: Vec<&str> = l.split('|').collect();
        match f[..] {
            ["skipped", why] => return Some(Outcome::Skipped(why.to_string())),
            ["found", check, what, picture] => c.found.push(Finding {
                check: named(check),
                what: what.to_string(),
                picture: (!picture.is_empty()).then(|| picture.to_string()),
            }),
            ["undrawn", check, page, why] => c.undrawn.push(Undrawn {
                check: named(check),
                page: page.parse().unwrap_or_default(),
                why: why.to_string(),
            }),
            ["signed", check, how] => c.signed.push((named(check), how.to_string())),
            ["checked", compared] => {
                c.compared = compared.parse().unwrap_or_default();
                return Some(Outcome::Checked(c));
            }
            _ => {}
        }
    }
    None
}

/// Checks `s` in a child process (this test run again for that file alone), so that a file
/// that takes too long or too much memory can be stopped without stopping the others: given
/// up after [`TIMEOUT`].
fn check_apart(s: &Sample, dpi: u32) -> Outcome {
    let env = [
        ("SAVE_REOPEN_FILE", s.name.clone()),
        ("SAVE_REOPEN_DPI", dpi.to_string()),
    ];
    let test = "save_and_reopen_the_corpus";
    let (text, status) = match apart::run(module_path!(), test, &env, TIMEOUT) {
        apart::Ran::Ended(text, status) => (text, status),
        apart::Ran::TimedOut => {
            return Outcome::Skipped(format!("timed out after {} s", TIMEOUT.as_secs()))
        }
        apart::Ran::NotStarted(e) => return Outcome::Skipped(format!("no child process: {e}")),
    };
    // No outcome: the child was stopped.
    outcome(apart::reports(&text, REPORT))
        .unwrap_or_else(|| Outcome::Skipped(apart::why_stopped(status)))
}

/// A known limit: a file, the text its difference contains, and the reason.
struct Known {
    name: String,
    text: String,
    why: String,
}

impl Known {
    /// Whether this limit is `f`, found in the file `name`.
    fn covers(&self, name: &str, f: &Finding) -> bool {
        self.name == name && said(f).contains(&self.text)
    }
}

/// A finding as summary.txt and the known list say it: "edited: page 1 draws differently".
fn said(f: &Finding) -> String {
    format!("{}: {}", f.check, f.what)
}

/// The known limits, from tools/save-reopen-known.txt.
fn known() -> Vec<Known> {
    let list = Path::new(env!("CARGO_MANIFEST_DIR")).join("tools/save-reopen-known.txt");
    let text = std::fs::read_to_string(&list).expect("tools/save-reopen-known.txt");
    text.lines()
        .filter(|l| !l.trim().is_empty() && !l.starts_with('#'))
        .map(|l| {
            let f: Vec<&str> = l.split(" | ").map(str::trim).collect();
            match f[..] {
                [name, text, why] if !text.is_empty() => Known {
                    name: name.to_string(),
                    text: text.to_string(),
                    why: why.to_string(),
                },
                _ => panic!(
                    "tools/save-reopen-known.txt: \"{l}\" isn't name | the difference | the reason"
                ),
            }
        })
        .collect()
}

/// A file's line in summary.txt: `same`, `undrawn` (the same, but for pages drawn neither
/// before nor after saving), `differs` or `skipped`, its name, and what was found.
fn summary_line(name: &str, o: &Outcome) -> String {
    let c = match o {
        Outcome::Skipped(why) => return format!("skipped   {name}  {why}"),
        Outcome::Checked(c) => c,
    };
    let status = if !c.found.is_empty() {
        "differs"
    } else if !c.undrawn.is_empty() {
        "undrawn"
    } else {
        "same"
    };
    let mut line = format!("{status:<10}{name}");
    if !c.found.is_empty() {
        let all: Vec<String> = c.found.iter().map(said).collect();
        let _ = write!(line, "  {}", all.join("; "));
    }
    if !c.undrawn.is_empty() {
        let all: Vec<String> = c
            .undrawn
            .iter()
            .map(|u| format!("{}: page {} ({})", u.check, u.page, u.why))
            .collect();
        let _ = write!(
            line,
            "  not drawn before or after saving: {}",
            all.join("; ")
        );
    }
    if !c.signed.is_empty() {
        let all: Vec<String> = c
            .signed
            .iter()
            .map(|(check, how)| format!("{check}: {how}"))
            .collect();
        let _ = write!(line, "  signed: {}", all.join("; "));
    }
    line
}

/// What a run found, counted.
#[derive(Default)]
struct Counts {
    checked: usize,
    same: usize,
    /// Files the same, but for pages drawn neither before nor after saving.
    undrawn: usize,
    differ: usize,
    skipped: usize,
    /// Pages drawn before and after saving and compared, in both checks.
    compared: usize,
    /// Pages drawn neither before nor after saving, each counted once in its file.
    pages_undrawn: usize,
    /// Files checked in which no page was compared.
    none_compared: usize,
    signed: usize,
    /// Signed files a save wrote whole instead of adding a revision.
    written_whole: usize,
}

impl Counts {
    fn of(results: &[(String, Outcome)]) -> Counts {
        let mut n = Counts::default();
        for (_, o) in results {
            let Outcome::Checked(c) = o else {
                n.skipped += 1;
                continue;
            };
            n.checked += 1;
            if !c.found.is_empty() {
                n.differ += 1;
            } else if !c.undrawn.is_empty() {
                n.undrawn += 1;
            } else {
                n.same += 1;
            }
            n.compared += c.compared;
            let mut pages: Vec<usize> = c.undrawn.iter().map(|u| u.page).collect();
            pages.sort_unstable();
            pages.dedup();
            n.pages_undrawn += pages.len();
            n.none_compared += usize::from(c.compared == 0);
            if !c.signed.is_empty() {
                n.signed += 1;
                n.written_whole += usize::from(c.signed.iter().any(|(_, how)| how != APPENDED));
            }
        }
        n
    }
}

fn escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

fn report(
    results: &[(String, Outcome)],
    n: &Counts,
    known: &[Known],
    dpi: u32,
    missing: usize,
) -> String {
    let mut h = String::new();
    let _ = write!(
        h,
        "<!doctype html><meta charset=utf-8><title>Save and reopen</title>\
         <style>body{{font:14px sans-serif;margin:16px;background:#fff;color:#111}}\
         img{{max-width:100%;border:1px solid #ccc}}section{{margin:24px 0}}\
         h2{{font-size:16px;margin:0 0 4px}}p{{margin:2px 0}}</style>\
         <h1>Saved and opened again</h1>\
         <p>{} of {} files checked at {dpi} dpi, {} skipped (in summary.txt); {} pages \
         compared, {} drawn neither before nor after saving and not compared; {} signed \
         files, {} of them written whole; {missing} listed files weren't downloaded. Each \
         picture: before saving, after, differences in red.</p>",
        n.checked,
        results.len(),
        n.skipped,
        n.compared,
        n.pages_undrawn,
        n.signed,
        n.written_whole,
    );
    for (name, o) in results {
        let Outcome::Checked(c) = o else { continue };
        let whole: Vec<_> = c.signed.iter().filter(|(_, how)| how != APPENDED).collect();
        if c.found.is_empty() && c.undrawn.is_empty() && whole.is_empty() {
            continue;
        }
        let _ = write!(h, "<section><h2>{}</h2>", escape(name));
        for f in &c.found {
            let _ = write!(h, "<p>{}: {}</p>", f.check, escape(&f.what));
            if let Some(k) = known.iter().find(|k| k.covers(name, f)) {
                let _ = write!(h, "<p><em>Known: {}</em></p>", escape(&k.why));
            }
            if let Some(p) = &f.picture {
                let _ = write!(
                    h,
                    "<a href=\"pages/{p}\"><img src=\"pages/{p}\" loading=lazy></a>"
                );
            }
        }
        for u in &c.undrawn {
            let _ = write!(
                h,
                "<p>{}: page {} drawn neither before nor after saving, not compared: {}</p>",
                u.check,
                u.page,
                escape(&u.why)
            );
        }
        for (check, how) in whole {
            let _ = write!(h, "<p>{check}: signed, {}</p>", escape(how));
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
    let jobs = apart::jobs("SAVE_REOPEN_JOBS");
    let _ = std::fs::remove_dir_all(&out);
    std::fs::create_dir_all(out.join("pages")).unwrap();
    let known = known();

    let (present, absent): (Vec<Sample>, Vec<Sample>) =
        samples(&only).into_iter().partition(|s| s.path().exists());
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

    let n = Counts::of(&results);
    let mut summary = String::new();
    let (mut unknown, mut seen) = (Vec::new(), vec![false; known.len()]);
    for (name, o) in &results {
        if let Outcome::Checked(c) = o {
            // Each difference is known only if a line of the list names it.
            for f in &c.found {
                match known.iter().position(|k| k.covers(name, f)) {
                    Some(i) => seen[i] = true,
                    None => unknown.push(format!("{name}  {}", said(f))),
                }
            }
        }
        let _ = writeln!(summary, "{}", summary_line(name, o));
    }
    std::fs::write(out.join("summary.txt"), &summary).unwrap();
    std::fs::write(
        out.join("index.html"),
        report(&results, &n, &known, dpi, absent.len()),
    )
    .unwrap();

    println!(
        "{} files in {:.0?}: {} the same, {} the same but for pages drawn neither before nor \
         after saving, {} differ, {} skipped; {} pages compared, {} drawn neither before nor \
         after saving; {} signed, {} of them written whole; {} listed files weren't downloaded; \
         see {}",
        results.len(),
        start.elapsed(),
        n.same,
        n.undrawn,
        n.differ,
        n.skipped,
        n.compared,
        n.pages_undrawn,
        n.signed,
        n.written_whole,
        absent.len(),
        out.join("index.html").display()
    );
    // Known limits not seen in a file that was checked: fixed, or now different.
    let gone: Vec<String> = known
        .iter()
        .zip(&seen)
        .filter(|(k, &seen)| {
            !seen
                && results
                    .iter()
                    .any(|(n, o)| *n == k.name && matches!(o, Outcome::Checked(_)))
        })
        .map(|(k, _)| format!("{} | {}", k.name, k.text))
        .collect();
    if !gone.is_empty() {
        println!(
            "known limits not seen now (take them off tools/save-reopen-known.txt): {}",
            gone.join(", ")
        );
    }
    assert!(
        unknown.is_empty(),
        "differences not in tools/save-reopen-known.txt (listed in {}):\n{}",
        out.join("summary.txt").display(),
        unknown.join("\n")
    );
    // A run that skips most of its files, or compares no page in them, says little: about 1
    // in 200 is skipped (files LimarePDF can't open; summary.txt says why).
    let unseen = n.skipped + n.none_compared;
    assert!(
        n.compared > 0 && (results.len() < MIN_FILES || unseen * 10 <= results.len()),
        "{} of {} files skipped and {} with no page compared, too many for the run to count \
         (see {})",
        n.skipped,
        results.len(),
        n.none_compared,
        out.join("summary.txt").display()
    );
}

#[test]
fn pages_not_drawn_are_counted() {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("save-reopen-undrawn");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("pages")).unwrap();
    // Page 2 is 10 million points a side, too large to draw at any scale.
    let mut p = Pdf::new();
    p.insert_blank(0, 300.0, 200.0).unwrap();
    p.insert_blank(1, 300.0, 200.0).unwrap();
    let page = p.page_ids()[1];
    p.doc
        .get_dictionary_mut(page)
        .unwrap()
        .set("MediaBox", vec![0.into(), 0.into(), 1e7.into(), 1e7.into()]);
    let path = dir.join("huge.pdf");
    std::fs::write(&path, p.to_bytes().unwrap()).unwrap();
    let scale = 96.0 / 72.0;
    let sample = |first, last| Sample {
        name: "huge".into(),
        first,
        last,
        password: None,
    };

    let o = check(&sample(1, 2), &path, &dir, scale);
    let Outcome::Checked(c) = &o else {
        panic!("skipped: {}", summary_line("huge", &o))
    };
    let found: Vec<String> = c.found.iter().map(said).collect();
    assert!(found.is_empty(), "{found:?}");
    let undrawn: Vec<_> = c.undrawn.iter().map(|u| (u.check, u.page)).collect();
    assert_eq!(undrawn, [("unchanged", 2), ("edited", 2)]);
    for u in &c.undrawn {
        assert!(u.why.contains("too large"), "{}", u.why);
    }
    // Page 1, saved unchanged and turned.
    assert_eq!(c.compared, 2);
    let line = summary_line("huge", &o);
    assert!(line.starts_with("undrawn   huge  "), "{line}");
    // The parent reads the same outcome back from the child's lines.
    let back = outcome(lines(&o).iter().map(String::as_str)).expect("an outcome");
    assert_eq!(summary_line("huge", &back), line);
    let n = Counts::of(&[("huge".into(), o)]);
    assert_eq!(
        (
            n.same,
            n.undrawn,
            n.pages_undrawn,
            n.compared,
            n.none_compared
        ),
        (0, 1, 1, 2, 0)
    );

    // Only page 2 listed: no page compared, which counts toward the skip limit.
    let o = check(&sample(2, 2), &path, &dir, scale);
    let n = Counts::of(&[("huge".into(), o)]);
    assert_eq!((n.undrawn, n.compared, n.none_compared), (1, 0, 1));

    // A signed file saved by adding a revision, in both checks.
    let signed =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/data/signatures/node-signpdf/signed.pdf");
    let o = check(&sample(1, 1), &signed, &dir, scale);
    let line = summary_line("signed", &o);
    assert!(line.starts_with("same      signed  signed: "), "{line}");
    let Outcome::Checked(c) = &o else {
        unreachable!()
    };
    let appended = APPENDED.to_string();
    assert_eq!(
        c.signed,
        [("unchanged", appended.clone()), ("edited", appended)]
    );
    let n = Counts::of(&[("signed".into(), o)]);
    assert_eq!((n.same, n.signed, n.written_whole), (1, 1, 0));
}

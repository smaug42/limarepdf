//! The paragraph check (NEW-01; the paragraph design's 4.5): paragraph detection judged on
//! the two answer sets in `tests/data/paragraphs` (its README). Opt-in, as it takes a while
//! and needs the render corpus for its first set:
//!
//! `cargo test -p limarepdf-core --release --test paragraph_check -- --ignored paragraph_check --nocapture`
//!
//! The threshold (batch 44's Q3): on each set's test half, at most 1% of confident blocks join
//! lines of two answer parts (over-merge), at most 3% stop short of their part (under-merge),
//! and at least 70% of multi-line body parts are found exactly as one confident block. The
//! tagged corpus is judged without its tags, which are its answers; the made-up set as the
//! detector reads documents (the user's answer to the plan's Q2, 2026-10-30). Both passing
//! turns paragraph editing on as the default; the output says which.
//!
//! `PARAGRAPH_CHECK_HALF=tune` shows the tuning halves alone, for tuning the detector's
//! constants without looking at the halves that judge them; `PARAGRAPH_CHECK_FILES=1` adds
//! each file's figures, `PARAGRAPH_CHECK_ERRORS=1` each error, and
//! `PARAGRAPH_CHECK_ONLY=name,name` judges those files alone.

use limarepdf_core::{Block, Detect, ObjectKind, PageObject, Pdf};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::time::Instant;

const DATA: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/data/");
const BODY: [&str; 4] = ["paragraph", "list-item", "quote", "footnote"];
const OVER_MAX: f64 = 1.0;
const UNDER_MAX: f64 = 3.0;
const COVERAGE_MIN: f64 = 70.0;

/// A line's answer.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
enum Ans {
    None,
    Left,
    Part(String),
}

struct ALine {
    page: usize,
    bbox: [f32; 4],
    base: f32,
    size: f32,
    ans: Ans,
}

struct Part {
    kind: String,
    page: usize,
    rows: usize,
}

struct File {
    set: &'static str,
    name: String,
    half: String,
    pages: usize,
    parts: HashMap<String, Part>,
    lines: Vec<ALine>,
    /// Set 1's tag elements of several paragraphs: element, page, rows.
    left_out: Vec<(String, usize, usize)>,
}

impl File {
    fn path(&self) -> PathBuf {
        match self.set {
            "madeup" => PathBuf::from(format!("{DATA}paragraphs/madeup/{}.pdf", self.name)),
            _ => {
                let (source, name) = self.name.split_once('-').expect("source-name");
                PathBuf::from(format!("{DATA}render/{source}/{name}.pdf"))
            }
        }
    }
}

/// Set 1's answer parts that hold several visual paragraphs, by their lines alone (no
/// detector): rows with a blank line's gap between them (more than half again the part's
/// closest rows' gap), or an indented row after one that ends well short. Their lines are
/// left out, as the user chose (the plan's Q1, 2026-10-30), and listed.
fn leave_out_several(f: &mut File) {
    let mut by_part: HashMap<String, Vec<usize>> = HashMap::new();
    for (i, l) in f.lines.iter().enumerate() {
        if let Ans::Part(p) = &l.ans {
            by_part.entry(p.clone()).or_default().push(i);
        }
    }
    let mut out: Vec<(String, usize, usize, &str)> = Vec::new();
    for (p, idx) in by_part {
        let mut ls: Vec<&ALine> = idx.iter().map(|&i| &f.lines[i]).collect();
        ls.sort_by(|a, b| a.base.total_cmp(&b.base));
        // Rows: lines on one baseline, as the answers count them.
        let mut rows: Vec<(f32, f32, f32, f32)> = Vec::new(); // base, x0, x1, size
        for l in ls {
            match rows.last_mut() {
                Some(r) if l.base - r.0 <= 0.6 * r.3.max(l.size).max(1.0) => {
                    r.1 = r.1.min(l.bbox[0]);
                    r.2 = r.2.max(l.bbox[2]);
                    r.3 = r.3.max(l.size);
                }
                _ => rows.push((l.base, l.bbox[0], l.bbox[2], l.size)),
            }
        }
        if rows.len() < 3 {
            continue;
        }
        let size = rows.iter().map(|r| r.3).fold(0.0, f32::max);
        let gaps: Vec<f32> = rows.windows(2).map(|w| w[1].0 - w[0].0).collect();
        let closest = gaps
            .iter()
            .copied()
            .filter(|g| *g >= 0.8 * size)
            .fold(f32::MAX, f32::min);
        let gap = closest < f32::MAX
            && gaps
                .iter()
                .any(|g| *g > 1.5 * closest && g - closest >= 0.5 * size);
        let left = rows[1..].iter().map(|r| r.1).fold(f32::MAX, f32::min);
        let right = rows.iter().map(|r| r.2).fold(f32::MIN, f32::max);
        let indent = (1..rows.len())
            .any(|k| rows[k].1 - left >= size && rows[k - 1].2 <= right - 3.0 * size);
        if gap || indent {
            let why = if gap {
                "a blank line's gap"
            } else {
                "an indent after a short line"
            };
            let page = f.parts[&p].page;
            out.push((p, page, rows.len(), why));
        }
    }
    out.sort();
    for (p, page, rows, why) in out {
        for l in f.lines.iter_mut().filter(|l| l.ans == Ans::Part(p.clone())) {
            l.ans = Ans::Left;
        }
        f.left_out
            .push((format!("part {p} (paragraphs apart by {why})"), page, rows));
    }
}

fn read(set: &'static str) -> Vec<File> {
    let text = std::fs::read_to_string(format!("{DATA}paragraphs/{set}.tsv")).unwrap();
    let mut files: Vec<File> = Vec::new();
    let f32_at = |v: &[&str], i: usize| v[i].parse::<f32>().unwrap();
    for line in text.lines().filter(|l| !l.starts_with('#')) {
        let v: Vec<&str> = line.split('\t').collect();
        match v[0] {
            "F" => files.push(File {
                set,
                name: v[1].into(),
                half: v[2].into(),
                pages: v[3].parse().unwrap(),
                parts: HashMap::new(),
                lines: Vec::new(),
                left_out: Vec::new(),
            }),
            "P" => {
                let part = Part {
                    kind: v[2].into(),
                    page: v[3].parse().unwrap(),
                    rows: v[4].parse().unwrap(),
                };
                files.last_mut().unwrap().parts.insert(v[1].into(), part);
            }
            "L" => {
                let ans = match v[8] {
                    "-" => Ans::None,
                    "X" => Ans::Left,
                    p => Ans::Part(p.into()),
                };
                files.last_mut().unwrap().lines.push(ALine {
                    page: v[1].parse().unwrap(),
                    bbox: [f32_at(&v, 2), f32_at(&v, 3), f32_at(&v, 4), f32_at(&v, 5)],
                    base: f32_at(&v, 6),
                    size: f32_at(&v, 7),
                    ans,
                });
            }
            "X" => {
                let element = format!("element {} (several paragraphs or a list)", v[1]);
                let x = (element, v[2].parse().unwrap(), v[3].parse().unwrap());
                files.last_mut().unwrap().left_out.push(x);
            }
            other => panic!("unknown record {other}"),
        }
    }
    if set == "corpus" {
        files.iter_mut().for_each(leave_out_several);
    }
    files
}

/// Counts for one file, or a sum of them.
#[derive(Default, Clone)]
struct Counts {
    confident: usize,
    ignored: usize,
    unjudged: usize,
    judged: usize,
    over: usize,
    under: usize,
    targets: usize,
    found: usize,
    misses: BTreeMap<String, usize>,
    /// Examples: over-merges, under-merges, misses.
    over_ex: Vec<String>,
    under_ex: Vec<String>,
    miss_ex: Vec<String>,
}

impl Counts {
    fn add(&mut self, o: &Counts) {
        self.confident += o.confident;
        self.ignored += o.ignored;
        self.unjudged += o.unjudged;
        self.judged += o.judged;
        self.over += o.over;
        self.under += o.under;
        self.targets += o.targets;
        self.found += o.found;
        for (k, n) in &o.misses {
            *self.misses.entry(k.clone()).or_default() += n;
        }
        self.over_ex.extend(o.over_ex.iter().cloned());
        self.under_ex.extend(o.under_ex.iter().cloned());
        self.miss_ex.extend(o.miss_ex.iter().cloned());
    }

    fn figures(&self) -> (f64, f64, f64) {
        let pct = |a: usize, b: usize| {
            if b == 0 {
                0.0
            } else {
                100.0 * a as f64 / b as f64
            }
        };
        (
            pct(self.over, self.judged),
            pct(self.under, self.judged),
            pct(self.found, self.targets),
        )
    }

    fn passes(&self) -> bool {
        let (o, u, c) = self.figures();
        o <= OVER_MAX && u <= UNDER_MAX && c >= COVERAGE_MIN
    }

    fn line(&self) -> String {
        let (o, u, c) = self.figures();
        let mark = |ok: bool| if ok { "pass" } else { "FAIL" };
        format!(
            "over {}/{} = {o:.2}% ({}); under {}/{} = {u:.2}% ({}); coverage {}/{} = {c:.1}% ({}); confident {}, with left-out lines {}, unjudged {}",
            self.over,
            self.judged,
            mark(o <= OVER_MAX),
            self.under,
            self.judged,
            mark(u <= UNDER_MAX),
            self.found,
            self.targets,
            mark(c >= COVERAGE_MIN),
            self.confident,
            self.ignored,
            self.unjudged,
        )
    }
}

/// Whether today's line `o` is answer line `a`: on one baseline, overlapping along it by
/// half the shorter.
fn same_line(o: &PageObject, a: &ALine) -> bool {
    let (Some(base), Some(size)) = (o.baseline, o.font_size) else {
        return false;
    };
    let overlap = o.bbox[2].min(a.bbox[2]) - o.bbox[0].max(a.bbox[0]);
    let shorter = (o.bbox[2] - o.bbox[0]).min(a.bbox[2] - a.bbox[0]).max(0.1);
    (base.1 - a.base).abs() < 0.3 * size.max(a.size) && overlap >= 0.5 * shorter
}

fn first_words(text: &str) -> String {
    text.split_whitespace()
        .take(4)
        .collect::<Vec<_>>()
        .join(" ")
}

/// One file judged with `how`: its counts, and each page's detection time in seconds.
fn judge(f: &File, how: Detect) -> (Counts, Vec<f64>) {
    let mut c = Counts::default();
    let mut times = Vec::new();
    let Ok(pdf) = Pdf::open(f.path(), None) else {
        c.misses.insert("file not opened".into(), 1);
        return (c, times);
    };
    // Each answer line's owners: today's lines that are it, by (page, object id).
    let mut owners: Vec<Vec<(usize, usize)>> = vec![Vec::new(); f.lines.len()];
    // Each of today's lines' answer.
    let mut answer: HashMap<(usize, usize), Ans> = HashMap::new();
    let mut texts: HashMap<(usize, usize), String> = HashMap::new();
    let mut pages: Vec<(usize, Vec<Block>)> = Vec::new();
    for page in 0..f.pages.min(pdf.page_count()) {
        let objs = pdf.page_objects(page).unwrap_or_default();
        let here: Vec<usize> = (0..f.lines.len())
            .filter(|&i| f.lines[i].page == page)
            .collect();
        for o in objs
            .iter()
            .filter(|o| o.kind == ObjectKind::Text && !o.hidden)
        {
            let mut got: HashSet<&Ans> = HashSet::new();
            for &i in &here {
                if same_line(o, &f.lines[i]) {
                    owners[i].push((page, o.id));
                    got.insert(&f.lines[i].ans);
                }
            }
            got.remove(&Ans::None);
            let ans = match got.len() {
                0 => Ans::None,
                1 => got.into_iter().next().unwrap().clone(),
                _ => Ans::Left,
            };
            answer.insert((page, o.id), ans);
            texts.insert((page, o.id), o.text.clone().unwrap_or_default());
        }
        let t = Instant::now();
        let blocks = pdf.page_blocks_by(page, how).unwrap_or_default();
        times.push(t.elapsed().as_secs_f64());
        pages.push((page, blocks));
    }
    // Each part's lines that today's lines are.
    let mut want: HashMap<&str, HashSet<usize>> = HashMap::new();
    for (i, l) in f.lines.iter().enumerate() {
        if let (Ans::Part(p), false) = (&l.ans, owners[i].is_empty()) {
            want.entry(p.as_str()).or_default().insert(i);
        }
    }
    let mut exact: HashSet<&str> = HashSet::new();
    // Which block each of today's lines is in, and whether that block is confident and why.
    let mut owner: HashMap<(usize, usize), (usize, usize)> = HashMap::new();
    for (page, blocks) in &pages {
        for (b, block) in blocks.iter().enumerate() {
            let ids: HashSet<(usize, usize)> = block.pieces().map(|id| (*page, id)).collect();
            for id in &ids {
                owner.insert(*id, (*page, b));
            }
            if !block.confident() {
                continue;
            }
            c.confident += 1;
            let answers: Vec<&Ans> = ids.iter().filter_map(|id| answer.get(id)).collect();
            if answers.contains(&&Ans::Left) {
                c.ignored += 1;
                continue;
            }
            let parts: HashSet<&str> = (answers.iter())
                .filter_map(|a| match a {
                    Ans::Part(p) => Some(p.as_str()),
                    _ => None,
                })
                .collect();
            let first = block.lines[0]
                .pieces
                .first()
                .and_then(|&id| texts.get(&(*page, id)));
            let first = first_words(first.map_or("", String::as_str));
            match parts.len() {
                0 => c.unjudged += 1,
                1 => {
                    c.judged += 1;
                    let p = *parts.iter().next().unwrap();
                    let mine: HashSet<usize> = (want.get(p).into_iter().flatten())
                        .copied()
                        .filter(|&i| owners[i].iter().any(|id| ids.contains(id)))
                        .collect();
                    let all = &want[p];
                    if !all.is_subset(&mine) {
                        c.under += 1;
                        c.under_ex.push(format!(
                            "{} p{} part {p} ({}): block of {} lines from “{first}”, missing {} of the part's {} lines",
                            f.name,
                            page + 1,
                            f.parts[p].kind,
                            block.lines.len(),
                            all.len() - mine.len(),
                            all.len()
                        ));
                    } else if &mine == all {
                        exact.insert(p);
                    }
                }
                _ => {
                    c.judged += 1;
                    c.over += 1;
                    let mut ps: Vec<&str> = parts.into_iter().collect();
                    ps.sort_unstable();
                    c.over_ex.push(format!(
                        "{} p{}: block of {} lines from “{first}” holds parts {}",
                        f.name,
                        page + 1,
                        block.lines.len(),
                        ps.join(", ")
                    ));
                }
            }
        }
    }
    let blocks_of: HashMap<usize, &Vec<Block>> = pages.iter().map(|(p, b)| (*p, b)).collect();
    let mut parts: Vec<(&String, &Part)> = f.parts.iter().collect();
    parts.sort_by(|a, b| a.0.cmp(b.0));
    for (p, info) in parts {
        let Some(lines) = want.get(p.as_str()) else {
            continue;
        };
        if !BODY.contains(&info.kind.as_str()) || info.rows < 2 {
            continue;
        }
        c.targets += 1;
        if exact.contains(p.as_str()) {
            c.found += 1;
            continue;
        }
        // Why not: the block holding most of the part's lines.
        let mut hold: HashMap<(usize, usize), usize> = HashMap::new();
        for &i in lines {
            for id in &owners[i] {
                if let Some(b) = owner.get(id) {
                    *hold.entry(*b).or_default() += 1;
                }
            }
        }
        let why = match hold.into_iter().max_by_key(|&(b, n)| (n, b)) {
            None => "no lines in a block".to_string(),
            Some(((page, b), _)) => match blocks_of[&page][b].doubt {
                Some(d) => format!("uncertain: {d:?}"),
                None => "confident but other lines".to_string(),
            },
        };
        let first = lines
            .iter()
            .min()
            .and_then(|&i| owners[i].first())
            .and_then(|id| texts.get(id));
        c.miss_ex.push(format!(
            "{} p{} part {p} ({}, {} rows) “{}”: {why}",
            f.name,
            info.page + 1,
            info.kind,
            info.rows,
            first_words(first.map_or("", String::as_str))
        ));
        *c.misses.entry(why).or_default() += 1;
    }
    (c, times)
}

/// Pages' detection times: seconds, file name, page.
type Times = Vec<(f64, String, usize)>;

/// Every file of `files` judged with `how`, several at once: by name, with page times.
fn judge_all(files: &[File], how: Detect) -> (HashMap<String, Counts>, Times) {
    let jobs = std::thread::available_parallelism()
        .map_or(2, |n| n.get())
        .min(4);
    let next = std::sync::atomic::AtomicUsize::new(0);
    let results = std::sync::Mutex::new((HashMap::new(), Vec::new()));
    std::thread::scope(|s| {
        for _ in 0..jobs {
            s.spawn(|| loop {
                let k = next.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                let Some(f) = files.get(k) else { break };
                let (c, times) = judge(f, how);
                let mut r = results.lock().unwrap();
                r.0.insert(f.name.clone(), c);
                r.1.extend(
                    times
                        .into_iter()
                        .enumerate()
                        .map(|(p, t)| (t, f.name.clone(), p)),
                );
            });
        }
    });
    results.into_inner().unwrap()
}

fn total<'a>(res: &HashMap<String, Counts>, files: impl Iterator<Item = &'a File>) -> Counts {
    let mut c = Counts::default();
    for f in files {
        if let Some(r) = res.get(&f.name) {
            c.add(r);
        }
    }
    c
}

#[test]
#[ignore = "opt-in: the paragraph check over both answer sets (see the top of this file)"]
fn paragraph_check() {
    let only_tune = std::env::var("PARAGRAPH_CHECK_HALF").is_ok_and(|h| h == "tune");
    let halves: &[&str] = if only_tune {
        &["tune"]
    } else {
        &["tune", "test"]
    };
    // `PARAGRAPH_CHECK_ONLY=name,name` judges those files alone, and lists every error.
    let only: Option<Vec<String>> = std::env::var("PARAGRAPH_CHECK_ONLY")
        .ok()
        .map(|v| v.split(',').map(str::to_string).collect());
    let pick = |files: Vec<File>| -> Vec<File> {
        match &only {
            Some(names) => files
                .into_iter()
                .filter(|f| names.contains(&f.name))
                .collect(),
            None => files,
        }
    };
    let mut sets = vec![("madeup", pick(read("madeup")))];
    let corpus = pick(read("corpus"));
    let have = corpus
        .iter()
        .filter(|f| Path::new(&f.path()).exists())
        .count();
    if have == corpus.len() {
        sets.insert(0, ("corpus", corpus));
    } else {
        println!(
            "The render corpus isn't here ({have} of {} files; tools/fetch-render-samples.sh): \
             the made-up set alone is judged, and no verdict on the default is given.",
            corpus.len()
        );
    }
    let variants = [
        (
            "geometry",
            Detect {
                tags: false,
                fonts: true,
            },
        ),
        (
            "tags",
            Detect {
                tags: true,
                fonts: true,
            },
        ),
        (
            "geometry, any font",
            Detect {
                tags: false,
                fonts: false,
            },
        ),
    ];
    let mut verdict: Vec<(String, bool)> = Vec::new();
    let mut all_times: Times = Vec::new();
    for (set, files) in &sets {
        for (vname, how) in variants {
            let (res, times) = judge_all(files, how);
            if vname == "tags" {
                all_times.extend(times);
            }
            for half in halves {
                let c = total(&res, files.iter().filter(|f| f.half == *half));
                println!("{set:7} {half:5} {vname:19} {}", c.line());
                // The verdict: set 1 without its tags, set 2 as documents are read.
                let judged = (*set == "corpus" && vname == "geometry")
                    || (*set == "madeup" && vname == "tags");
                if *half == "test" && judged {
                    verdict.push((format!("{set} ({vname})"), c.passes()));
                }
                if vname == "geometry" || (*set == "madeup" && vname == "tags") {
                    let misses: Vec<String> = (c.misses.iter())
                        .map(|(k, n)| format!("{k}: {n}"))
                        .collect();
                    println!("        misses: {}", misses.join("; "));
                }
                let errors = vname == "geometry" || (*set == "madeup" && vname == "tags");
                if errors && std::env::var("PARAGRAPH_CHECK_FILES").is_ok() {
                    for f in files.iter().filter(|f| f.half == *half) {
                        println!("        {:34} {}", f.name, res[&f.name].line());
                    }
                }
                if errors && std::env::var("PARAGRAPH_CHECK_ERRORS").is_ok() {
                    for e in &c.over_ex {
                        println!("        over: {e}");
                    }
                    for e in &c.under_ex {
                        println!("        under: {e}");
                    }
                    let shown = if only.is_some() { usize::MAX } else { 60 };
                    for e in c.miss_ex.iter().take(shown) {
                        println!("        miss: {e}");
                    }
                }
            }
        }
        if *set == "corpus" {
            for half in halves {
                let left: Vec<&File> = files.iter().filter(|f| f.half == *half).collect();
                let n: usize = left.iter().map(|f| f.left_out.len()).sum();
                println!("        left out of the {half} half: {n} tag elements and parts");
                for f in left {
                    for (e, page, rows) in &f.left_out {
                        println!(
                            "        left out: {} p{} {e}, {rows} rows",
                            f.name,
                            page + 1
                        );
                    }
                }
            }
        }
    }
    all_times.sort_by(|a, b| a.0.total_cmp(&b.0));
    if let Some(median) = all_times.get(all_times.len() / 2) {
        let worst: Vec<String> = (all_times.iter().rev().take(5))
            .map(|(t, n, p)| format!("{:.1} ms ({n} p{})", t * 1000.0, p + 1))
            .collect();
        println!(
            "detection per page ({} pages): median {:.2} ms; worst {}",
            all_times.len(),
            median.0 * 1000.0,
            worst.join(", ")
        );
    }
    if !only_tune && verdict.len() == 2 {
        let on = verdict.iter().all(|(_, ok)| *ok);
        for (what, ok) in &verdict {
            println!("verdict: {what} {}", if *ok { "passes" } else { "fails" });
        }
        println!(
            "paragraph editing as the default: {}",
            if on {
                "on"
            } else {
                "off (the switch's Paragraph ships alone)"
            }
        );
    }
}

//! LimarePDF's rendering compared with poppler's, page by page, over public test files.
//!
//! Opt-in, and needs `pdftoppm` (poppler-utils). From crates/limarepdf-core, run
//! `sh tools/render-compare.sh`, which fetches the files listed in
//! `tools/render-samples.txt` (the pdf.js and PDFBox test files hayro is tested on) and
//! then runs this test:
//! `cargo test --release -p limarepdf-core --test render_compare -- --ignored --nocapture`.
//!
//! Each listed page is drawn as the app and `limarepdf-cli render` draw it, and by
//! `pdftoppm`, at `RENDER_COMPARE_DPI` (96 by default). A pixel counts as different only
//! when no pixel within one of it in the other image is close in colour, so anti-aliasing
//! and edges a pixel apart don't count. Pages with more than `FLAG` of their pixels
//! different, a different size, or a failure on one side only are listed in
//! `target/tmp/render-compare/index.html`, worst first, with LimarePDF, poppler and the
//! differences side by side. `summary.txt` beside it lists every page's share, to compare
//! runs. `RENDER_COMPARE_ONLY` limits the run to files whose name contains it.
//!
//! The test fails only when it can't run at all; the differences are for a person to judge.

use limarepdf_core::render::{self, Bitmap, Missing, Renderer};
use limarepdf_core::Pdf;
use rayon::prelude::*;
use std::collections::BTreeSet;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

/// How far, in any channel, a pixel may be outside the colours around it in the other image.
const TOLERANCE: u8 = 64;
/// Share of a page's pixels that may differ before the page is listed.
const FLAG: f64 = 0.005;
/// Size difference in pixels allowed for rounding.
const SIZE_SLACK: usize = 2;
/// Time allowed for one file on each side.
const TIMEOUT: Duration = Duration::from_secs(60);

struct Sample {
    /// `pdfjs/issue1234`, the file's path under tests/data/render without `.pdf`.
    name: String,
    first: usize,
    last: usize,
    password: Option<String>,
}

fn samples(only: &str) -> Vec<Sample> {
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

struct Image {
    width: usize,
    height: usize,
    rgb: Vec<u8>,
}

impl Image {
    fn from_bitmap(b: &Bitmap) -> Self {
        let rgb = b
            .rgba
            .as_chunks::<4>()
            .0
            .iter()
            .flat_map(|&[r, g, b, _]| [r, g, b])
            .collect();
        Image {
            width: b.width,
            height: b.height,
            rgb,
        }
    }

    fn px(&self, x: usize, y: usize) -> [u8; 3] {
        let i = (y * self.width + x) * 3;
        [self.rgb[i], self.rgb[i + 1], self.rgb[i + 2]]
    }
}

/// What one side made of one page.
enum Drawn {
    Ok(Image, BTreeSet<Missing>),
    Failed(String),
}

/// The listed pages of `s` drawn by LimarePDF, or why none could be. Runs on its own
/// thread with a large stack, given up after [`TIMEOUT`].
fn ours(path: &Path, s: &Sample, scale: f32) -> Result<Vec<(usize, Drawn)>, String> {
    let bytes = std::fs::read(path).map_err(|e| e.to_string())?;
    let pw = s.password.clone();
    let (first, last) = (s.first, s.last);
    let (tx, rx) = mpsc::channel();
    std::thread::Builder::new()
        .stack_size(256 << 20)
        .spawn(move || {
            let pw = pw.as_deref();
            // As `limarepdf-cli render` does: the app's view of the file, or the file as
            // it is when the document reader can't open it.
            let loaded = match Pdf::from_bytes(&bytes, pw).and_then(|mut p| p.render_snapshot()) {
                Ok(b) => render::load(b, None),
                Err(_) => render::load(bytes, pw),
            };
            let hp = match loaded {
                Ok(hp) => hp,
                Err(e) => {
                    let _ = tx.send(Err(e.to_string()));
                    return;
                }
            };
            let r = Renderer::new(&hp);
            let pages = (first..=last.min(r.page_count()))
                .map(|n| {
                    let d = match r.render(n - 1, scale) {
                        Ok(b) => Drawn::Ok(Image::from_bitmap(&b), r.take_missing()),
                        Err(e) => Drawn::Failed(e.to_string()),
                    };
                    (n, d)
                })
                .collect();
            let _ = tx.send(Ok(pages));
        })
        .map_err(|e| e.to_string())?;
    match rx.recv_timeout(TIMEOUT) {
        Ok(r) => r,
        Err(mpsc::RecvTimeoutError::Timeout) => Err("timed out".into()),
        Err(mpsc::RecvTimeoutError::Disconnected) => Err("panicked".into()),
    }
}

/// Page `n` of `path` drawn by `pdftoppm`.
fn poppler(path: &Path, s: &Sample, n: usize, dpi: u32, tmp: &Path) -> Drawn {
    let stem = tmp.join(format!("p{n}"));
    let (dpi, n_arg) = (dpi.to_string(), n.to_string());
    let mut cmd = Command::new("pdftoppm");
    cmd.args(["-r", &dpi, "-f", &n_arg, "-l", &n_arg])
        .args(["-singlefile", "-cropbox", "-png"]);
    if let Some(pw) = &s.password {
        cmd.args(["-upw", pw]);
    }
    let mut child = match cmd
        .arg(path)
        .arg(&stem)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
    {
        Ok(c) => c,
        Err(e) => return Drawn::Failed(e.to_string()),
    };
    let start = Instant::now();
    let status = loop {
        match child.try_wait() {
            Ok(Some(st)) => break st,
            Ok(None) if start.elapsed() > TIMEOUT => {
                let _ = child.kill();
                let _ = child.wait();
                return Drawn::Failed("timed out".into());
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(20)),
            Err(e) => return Drawn::Failed(e.to_string()),
        }
    };
    let png = stem.with_extension("png");
    let img = image::open(&png);
    let _ = std::fs::remove_file(&png);
    match img {
        Ok(i) => {
            let i = i.to_rgb8();
            Drawn::Ok(
                Image {
                    width: i.width() as usize,
                    height: i.height() as usize,
                    rgb: i.into_raw(),
                },
                BTreeSet::new(),
            )
        }
        Err(_) => Drawn::Failed(format!("exit {}", status.code().unwrap_or(-1))),
    }
}

/// Whether `a`'s pixel at (x, y) is, in some channel, further than [`TOLERANCE`] outside
/// the range of `b`'s pixels within one of it. Any blend of neighbouring colours, as
/// anti-aliasing at an edge a fraction of a pixel away gives, is inside the range.
fn unmatched(a: &Image, b: &Image, x: usize, y: usize, w: usize, h: usize) -> bool {
    let p = a.px(x, y);
    let (mut lo, mut hi) = ([255u8; 3], [0u8; 3]);
    for qy in y.saturating_sub(1)..=(y + 1).min(h - 1) {
        for qx in x.saturating_sub(1)..=(x + 1).min(w - 1) {
            let q = b.px(qx, qy);
            for k in 0..3 {
                lo[k] = lo[k].min(q[k]);
                hi[k] = hi[k].max(q[k]);
            }
        }
    }
    (0..3).any(|k| p[k].saturating_add(TOLERANCE) < lo[k] || p[k] > hi[k].saturating_add(TOLERANCE))
}

/// The pixels that differ beyond anti-aliasing, over the area both images cover.
fn differences(a: &Image, b: &Image) -> (Vec<bool>, usize, usize) {
    let (w, h) = (a.width.min(b.width), a.height.min(b.height));
    let mut mask = vec![false; w * h];
    mask.par_chunks_mut(w.max(1))
        .enumerate()
        .for_each(|(y, row)| {
            for (x, m) in row.iter_mut().enumerate() {
                *m = unmatched(a, b, x, y, w, h) || unmatched(b, a, x, y, w, h);
            }
        });
    (mask, w, h)
}

/// LimarePDF, poppler and the differences (red on a faded poppler page), side by side.
fn side_by_side(a: &Image, b: &Image, mask: &[bool], w: usize) -> image::RgbImage {
    const GAP: usize = 8;
    let h = a.height.max(b.height);
    let total = a.width + b.width + w + 2 * GAP;
    let mut out = image::RgbImage::from_pixel(total as u32, h as u32, image::Rgb([128; 3]));
    let mut put = |img: &Image, left: usize| {
        for y in 0..img.height {
            for x in 0..img.width {
                out.put_pixel((left + x) as u32, y as u32, image::Rgb(img.px(x, y)));
            }
        }
    };
    put(a, 0);
    put(b, a.width + GAP);
    let left = a.width + b.width + 2 * GAP;
    for (i, &m) in mask.iter().enumerate() {
        let (x, y) = (i % w, i / w);
        let c = if m {
            [230, 0, 0]
        } else {
            b.px(x, y).map(|v| 255 - (255 - v) / 4)
        };
        out.put_pixel((left + x) as u32, y as u32, image::Rgb(c));
    }
    out
}

/// One page's result.
struct Row {
    name: String,
    page: usize,
    /// Share of pixels that differ, or `None` when a side failed.
    share: Option<f64>,
    note: String,
    picture: Option<String>,
}

impl Row {
    fn flagged(&self) -> bool {
        self.share.is_none_or(|s| s > FLAG) || !self.note.is_empty()
    }
}

fn compare(s: &Sample, root: &Path, out: &Path, dpi: u32) -> Vec<Row> {
    let path = root.join(format!("{}.pdf", s.name));
    let tmp = out.join("tmp").join(s.name.replace('/', "_"));
    std::fs::create_dir_all(&tmp).unwrap();
    let row = |page, share, note: String| Row {
        name: s.name.clone(),
        page,
        share,
        note,
        picture: None,
    };
    let mine = match ours(&path, s, dpi as f32 / 72.0) {
        Ok(p) => p,
        Err(e) => {
            // Is it LimarePDF's failure alone?
            let theirs = match poppler(&path, s, s.first, dpi, &tmp) {
                Drawn::Ok(..) => "poppler draws it".to_string(),
                Drawn::Failed(why) => format!("poppler fails too ({why})"),
            };
            return vec![row(s.first, None, format!("LimarePDF: {e}; {theirs}"))];
        }
    };
    let mut rows = Vec::new();
    for (n, d) in mine {
        let theirs = poppler(&path, s, n, dpi, &tmp);
        let r = match (d, theirs) {
            (Drawn::Failed(a), Drawn::Failed(b)) => {
                row(n, None, format!("both fail (LimarePDF: {a}; poppler: {b})"))
            }
            (Drawn::Failed(a), Drawn::Ok(..)) => row(n, None, format!("LimarePDF fails: {a}")),
            (Drawn::Ok(..), Drawn::Failed(b)) => row(n, None, format!("poppler fails: {b}")),
            (Drawn::Ok(a, missing), Drawn::Ok(b, _)) => {
                let (mask, w, h) = differences(&a, &b);
                let share = mask.iter().filter(|&&m| m).count() as f64 / (w * h).max(1) as f64;
                let mut notes = Vec::new();
                if a.width.abs_diff(b.width) > SIZE_SLACK
                    || a.height.abs_diff(b.height) > SIZE_SLACK
                {
                    notes.push(format!(
                        "size {}×{} against poppler's {}×{}",
                        a.width, a.height, b.width, b.height
                    ));
                }
                // Fonts drawn with a stand-in aren't listed for that alone.
                let lost: Vec<String> = missing
                    .iter()
                    .filter(|m| m.lost())
                    .map(|m| m.describe())
                    .collect();
                if !lost.is_empty() {
                    notes.push(format!("LimarePDF couldn't draw {}", lost.join(", ")));
                }
                let mut r = row(n, Some(share), notes.join("; "));
                if r.flagged() {
                    let file = format!("{}-p{n}.png", s.name.replace('/', "_"));
                    side_by_side(&a, &b, &mask, w)
                        .save(out.join("pages").join(&file))
                        .unwrap();
                    r.picture = Some(file);
                }
                r
            }
        };
        rows.push(r);
    }
    let _ = std::fs::remove_dir_all(&tmp);
    rows
}

fn escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

fn report(rows: &[Row], dpi: u32, missing: usize) -> String {
    let flagged: Vec<&Row> = rows.iter().filter(|r| r.flagged()).collect();
    let mut h = String::new();
    let _ = write!(
        h,
        "<!doctype html><meta charset=utf-8><title>Render comparison</title>\
         <style>body{{font:14px sans-serif;margin:16px;background:#fff;color:#111}}\
         img{{max-width:100%;border:1px solid #ccc}}section{{margin:24px 0}}\
         h2{{font-size:16px;margin:0 0 4px}}p{{margin:2px 0}}</style>\
         <h1>LimarePDF against poppler</h1>\
         <p>{} pages from {} files at {dpi} dpi; {} listed (more than {}% of pixels \
         different, a size difference, something LimarePDF couldn't draw, or a failure). \
         {missing} listed files weren't downloaded.</p>\
         <p>Each picture: LimarePDF, poppler, differences in red.</p>",
        rows.len(),
        rows.iter().map(|r| &r.name).collect::<BTreeSet<_>>().len(),
        flagged.len(),
        FLAG * 100.0,
    );
    for r in flagged {
        let share = r
            .share
            .map_or("—".to_string(), |s| format!("{:.2}%", s * 100.0));
        let _ = write!(
            h,
            "<section><h2>{} page {}: {share}</h2>",
            escape(&r.name),
            r.page
        );
        if !r.note.is_empty() {
            let _ = write!(h, "<p>{}</p>", escape(&r.note));
        }
        if let Some(p) = &r.picture {
            let _ = write!(
                h,
                "<a href=\"pages/{p}\"><img src=\"pages/{p}\" loading=lazy></a>"
            );
        }
        h.push_str("</section>");
    }
    h
}

#[test]
#[ignore = "needs pdftoppm and the fetched samples: sh tools/render-compare.sh"]
fn render_compare() {
    assert!(
        Command::new("pdftoppm").arg("-v").output().is_ok(),
        "needs pdftoppm (poppler-utils)"
    );
    let dpi: u32 = std::env::var("RENDER_COMPARE_DPI")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(96);
    let only = std::env::var("RENDER_COMPARE_ONLY").unwrap_or_default();
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/data/render");
    let out = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("render-compare");
    let _ = std::fs::remove_dir_all(&out);
    std::fs::create_dir_all(out.join("pages")).unwrap();

    let all = samples(&only);
    let (present, absent): (Vec<_>, Vec<_>) = all
        .into_iter()
        .partition(|s| root.join(format!("{}.pdf", s.name)).exists());
    assert!(
        !present.is_empty(),
        "no samples in {}: run sh tools/fetch-render-samples.sh",
        root.display()
    );

    // The files are spread over a pool of their own: LimarePDF's rendering uses rayon's
    // global pool, which would starve if its workers were the ones waiting for it.
    let pool = rayon::ThreadPoolBuilder::new().build().unwrap();
    let start = Instant::now();
    let mut rows: Vec<Row> = pool.install(|| {
        present
            .par_iter()
            .flat_map_iter(|s| compare(s, &root, &out, dpi))
            .collect()
    });
    let _ = std::fs::remove_dir_all(out.join("tmp"));
    rows.sort_by(|a, b| {
        let key = |r: &Row| r.share.unwrap_or(f64::INFINITY);
        key(b)
            .total_cmp(&key(a))
            .then_with(|| a.name.cmp(&b.name))
            .then(a.page.cmp(&b.page))
    });

    let mut summary = String::new();
    for r in &rows {
        let share = r
            .share
            .map_or("failed".to_string(), |s| format!("{:.3}%", s * 100.0));
        let _ = writeln!(summary, "{share:>8}  {} p{}  {}", r.name, r.page, r.note);
    }
    std::fs::write(out.join("summary.txt"), &summary).unwrap();
    std::fs::write(out.join("index.html"), report(&rows, dpi, absent.len())).unwrap();

    let flagged = rows.iter().filter(|r| r.flagged()).count();
    println!(
        "{} pages from {} files in {:.0?}; {flagged} listed in {}",
        rows.len(),
        present.len(),
        start.elapsed(),
        out.join("index.html").display()
    );
}

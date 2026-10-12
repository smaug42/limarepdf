use anyhow::{bail, Context, Result};

/// println! that ignores a closed pipe (e.g. `mpe text file.pdf | head`).
macro_rules! out {
    ($($t:tt)*) => {{
        use std::io::Write;
        let _ = writeln!(std::io::stdout(), $($t)*);
    }};
}
use clap::{Args, Parser, Subcommand, ValueEnum};
use mpe_core::{
    parse_page_range, render, Bates, Cipher, Encryption, HeaderFooter, Info, Pdf, Restrictions, Watermark,
};
use rayon::prelude::*;
use std::path::{Path, PathBuf};

#[derive(Parser)]
#[command(name = "mpe", version, about = "Fast PDF toolkit (pure Rust)")]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Args, Clone)]
struct Io {
    /// Input PDF
    input: PathBuf,
    /// Output PDF
    #[arg(short, long)]
    output: PathBuf,
    /// Password for an encrypted input
    #[arg(long)]
    password: Option<String>,
}

#[derive(Subcommand)]
enum Cmd {
    /// Show page count, page sizes and document information
    Info { input: PathBuf, #[arg(long)] password: Option<String> },
    /// Set document information fields
    SetInfo {
        #[command(flatten)]
        io: Io,
        #[arg(long)] title: Option<String>,
        #[arg(long)] author: Option<String>,
        #[arg(long)] subject: Option<String>,
        #[arg(long)] keywords: Option<String>,
    },
    /// Merge PDFs in the order given
    Merge {
        #[arg(required = true, num_args = 2..)] inputs: Vec<PathBuf>,
        #[arg(short, long)] output: PathBuf,
    },
    /// Split into parts of N pages: out-dir/<stem>-001.pdf, ...
    Split {
        input: PathBuf,
        #[arg(long, default_value_t = 1)] every: usize,
        #[arg(long)] out_dir: PathBuf,
        #[arg(long)] password: Option<String>,
    },
    /// Keep only the listed pages, e.g. --pages 1-3,7
    Extract { #[command(flatten)] io: Io, #[arg(long)] pages: String },
    /// Delete the listed pages
    Delete { #[command(flatten)] io: Io, #[arg(long)] pages: String },
    /// Rotate pages clockwise by 90, 180 or 270
    Rotate { #[command(flatten)] io: Io, #[arg(long, default_value = "all")] pages: String, #[arg(long)] degrees: i64 },
    /// Insert a blank page after page N (0 = at the start)
    InsertBlank {
        #[command(flatten)] io: Io,
        #[arg(long)] after: usize,
        #[arg(long, default_value_t = 612.0)] width: f32,
        #[arg(long, default_value_t = 792.0)] height: f32,
    },
    /// Set a page's crop box in points: left,bottom,right,top
    Crop { #[command(flatten)] io: Io, #[arg(long)] page: usize, #[arg(long)] rect: String },
    /// Reorder all pages: a permutation such as 3,1,2
    Reorder { #[command(flatten)] io: Io, #[arg(long)] order: String },
    /// Add a text watermark
    Watermark {
        #[command(flatten)] io: Io,
        #[arg(long)] text: String,
        #[arg(long, default_value = "all")] pages: String,
        #[arg(long, default_value_t = 96.0)] size: f32,
        #[arg(long, default_value_t = 0.25)] opacity: f32,
        #[arg(long, default_value_t = 45.0)] rotation: f32,
        /// Put the watermark behind the page content
        #[arg(long)] background: bool,
    },
    /// Add header/footer text; tokens: {page} {pages} {date} {bates}
    HeaderFooter {
        #[command(flatten)] io: Io,
        #[arg(long, default_value = "all")] pages: String,
        #[arg(long)] header_left: Option<String>,
        #[arg(long)] header_center: Option<String>,
        #[arg(long)] header_right: Option<String>,
        #[arg(long)] footer_left: Option<String>,
        #[arg(long)] footer_center: Option<String>,
        #[arg(long)] footer_right: Option<String>,
        #[arg(long, default_value_t = 10.0)] size: f32,
    },
    /// Bates-number the pages
    Bates {
        #[command(flatten)] io: Io,
        #[arg(long, default_value = "all")] pages: String,
        #[arg(long, default_value = "")] prefix: String,
        #[arg(long, default_value = "")] suffix: String,
        #[arg(long, default_value_t = 6)] digits: usize,
        #[arg(long, default_value_t = 1)] start: u64,
    },
    /// Password-protect a PDF
    Encrypt {
        #[command(flatten)] io: Io,
        #[arg(long, default_value = "")] user_password: String,
        #[arg(long)] owner_password: String,
        #[arg(long, value_enum, default_value = "aes256")] cipher: CipherArg,
        #[arg(long)] no_print: bool,
        #[arg(long)] no_copy: bool,
        #[arg(long)] no_modify: bool,
    },
    /// Remove password protection (needs the password)
    Decrypt { #[command(flatten)] io: Io },
    /// Search page text
    Search { input: PathBuf, query: String, #[arg(long)] password: Option<String> },
    /// Print extracted text
    Text { input: PathBuf, #[arg(long, default_value = "all")] pages: String, #[arg(long)] password: Option<String> },
    /// Render pages to PNG files: out-dir/<stem>-<page>.png
    Render {
        input: PathBuf,
        #[arg(long, default_value = "all")] pages: String,
        #[arg(long, default_value_t = 150.0)] dpi: f32,
        #[arg(long)] out_dir: PathBuf,
        #[arg(long)] password: Option<String>,
    },
}

#[derive(Copy, Clone, ValueEnum)]
enum CipherArg {
    Rc4_40,
    Rc4_128,
    Aes128,
    Aes256,
}

fn open(p: &Path, pw: &Option<String>) -> Result<Pdf> {
    Pdf::open(p, pw.as_deref())
}

fn run_edit(io: &Io, f: impl FnOnce(&mut Pdf) -> Result<()>) -> Result<()> {
    let mut pdf = open(&io.input, &io.password)?;
    f(&mut pdf)?;
    pdf.save(&io.output)?;
    out!("wrote {}", io.output.display());
    Ok(())
}

fn some(s: &Option<String>) -> String {
    s.clone().unwrap_or_default()
}

fn main() -> Result<()> {
    match Cli::parse().cmd {
        Cmd::Info { input, password } => {
            let p = open(&input, &password)?;
            let i = p.info();
            out!("pages:    {}", p.page_count());
            for (k, v) in [("title", i.title), ("author", i.author), ("subject", i.subject),
                           ("keywords", i.keywords), ("creator", i.creator), ("producer", i.producer)] {
                if let Some(v) = v { out!("{k}: {v}"); }
            }
            for n in 0..p.page_count().min(5) {
                let (w, h) = p.page_size(n)?;
                out!("page {}: {:.1} x {:.1} pt", n + 1, w, h);
            }
            if p.page_count() > 5 { out!("..."); }
        }
        Cmd::SetInfo { io, title, author, subject, keywords } => run_edit(&io, |p| {
            p.set_info(&Info { title, author, subject, keywords, ..Default::default() })
        })?,
        Cmd::Merge { inputs, output } => {
            let docs = inputs.par_iter().map(|p| Pdf::open(p, None)).collect::<Result<Vec<_>>>()?;
            let mut m = Pdf::merge(docs)?;
            m.save(&output)?;
            out!("wrote {} ({} pages)", output.display(), m.page_count());
        }
        Cmd::Split { input, every, out_dir, password } => {
            let p = open(&input, &password)?;
            std::fs::create_dir_all(&out_dir)?;
            let stem = input.file_stem().and_then(|s| s.to_str()).unwrap_or("part").to_string();
            let parts = p.split_every(every)?;
            parts.into_par_iter().enumerate().try_for_each(|(i, mut part)| -> Result<()> {
                part.save(out_dir.join(format!("{stem}-{:03}.pdf", i + 1)))
            })?;
            out!("split into {} files in {}", p.page_count().div_ceil(every), out_dir.display());
        }
        Cmd::Extract { io, pages } => run_edit(&io, |p| {
            let idx = parse_page_range(&pages, p.page_count())?;
            *p = p.extract(&idx)?;
            Ok(())
        })?,
        Cmd::Delete { io, pages } => run_edit(&io, |p| {
            let idx = parse_page_range(&pages, p.page_count())?;
            p.delete_pages(&idx)
        })?,
        Cmd::Rotate { io, pages, degrees } => run_edit(&io, |p| {
            let idx = parse_page_range(&pages, p.page_count())?;
            p.rotate_pages(&idx, degrees)
        })?,
        Cmd::InsertBlank { io, after, width, height } => run_edit(&io, |p| p.insert_blank(after, width, height))?,
        Cmd::Crop { io, page, rect } => run_edit(&io, |p| {
            let v: Vec<f32> = rect.split(',').map(|x| x.trim().parse()).collect::<Result<_, _>>()
                .context("--rect needs four numbers: left,bottom,right,top")?;
            if v.len() != 4 { bail!("--rect needs four numbers: left,bottom,right,top"); }
            if page == 0 { bail!("pages are numbered from 1"); }
            p.crop_page(page - 1, [v[0], v[1], v[2], v[3]])
        })?,
        Cmd::Reorder { io, order } => run_edit(&io, |p| {
            let o: Vec<usize> = order.split(',').map(|x| x.trim().parse::<usize>().map(|n| n.wrapping_sub(1)))
                .collect::<Result<_, _>>()?;
            p.reorder(&o)
        })?,
        Cmd::Watermark { io, text, pages, size, opacity, rotation, background } => run_edit(&io, |p| {
            let idx = parse_page_range(&pages, p.page_count())?;
            p.add_watermark(&idx, &Watermark { text, font_size: size, opacity, rotation, foreground: !background, ..Default::default() })
        })?,
        Cmd::HeaderFooter { io, pages, header_left, header_center, header_right, footer_left, footer_center, footer_right, size } => {
            run_edit(&io, |p| {
                let idx = parse_page_range(&pages, p.page_count())?;
                p.add_header_footer(&idx, &HeaderFooter {
                    header: [some(&header_left), some(&header_center), some(&header_right)],
                    footer: [some(&footer_left), some(&footer_center), some(&footer_right)],
                    font_size: size,
                    ..Default::default()
                })
            })?
        }
        Cmd::Bates { io, pages, prefix, suffix, digits, start } => run_edit(&io, |p| {
            let idx = parse_page_range(&pages, p.page_count())?;
            p.add_header_footer(&idx, &HeaderFooter {
                header: Default::default(),
                footer: Default::default(),
                bates: Some(Bates { prefix, suffix, digits, start }),
                ..Default::default()
            })
        })?,
        Cmd::Encrypt { io, user_password, owner_password, cipher, no_print, no_copy, no_modify } => run_edit(&io, |p| {
            p.set_encryption(Some(Encryption {
                user_password,
                owner_password,
                cipher: match cipher {
                    CipherArg::Rc4_40 => Cipher::Rc4_40,
                    CipherArg::Rc4_128 => Cipher::Rc4_128,
                    CipherArg::Aes128 => Cipher::Aes128,
                    CipherArg::Aes256 => Cipher::Aes256,
                },
                restrictions: Restrictions { print: !no_print, copy: !no_copy, modify: !no_modify, ..Default::default() },
            }));
            Ok(())
        })?,
        Cmd::Decrypt { io } => run_edit(&io, |_| Ok(()))?,
        Cmd::Search { input, query, password } => {
            let p = open(&input, &password)?;
            let hits = p.search(&query)?;
            for h in &hits { out!("p{}: …{}…", h.page + 1, h.snippet); }
            out!("{} match(es)", hits.len());
        }
        Cmd::Text { input, pages, password } => {
            let p = open(&input, &password)?;
            for i in parse_page_range(&pages, p.page_count())? {
                out!("--- page {} ---\n{}", i + 1, p.page_text(i));
            }
        }
        Cmd::Render { input, pages, dpi, out_dir, password } => {
            let bytes = std::fs::read(&input)?;
            let hp = render::load(bytes, password.as_deref())?;
            let n = render::Renderer::new(&hp).page_count();
            let idx = parse_page_range(&pages, n)?;
            std::fs::create_dir_all(&out_dir)?;
            let stem = input.file_stem().and_then(|s| s.to_str()).unwrap_or("page").to_string();
            let t = std::time::Instant::now();
            let bitmaps = render::render_pages_parallel(&hp, &idx, dpi / 72.0);
            for (i, b) in idx.iter().zip(bitmaps) {
                let b = b?;
                let path = out_dir.join(format!("{stem}-{:03}.png", i + 1));
                image::save_buffer(&path, &b.rgba, b.width as u32, b.height as u32, image::ColorType::Rgba8)?;
            }
            out!("rendered {} page(s) in {:.2?}", idx.len(), t.elapsed());
        }
    }
    Ok(())
}

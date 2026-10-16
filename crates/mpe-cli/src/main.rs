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
    format_page_range, parse_page_range, part_file_name, print, render, Annot, AnnotKind, Bates,
    BookmarkPlace, Cipher, Encryption, Field, FieldKind, FieldValue, HeaderFooter, ImageImport,
    ImageWatermark, Info, LabelRange, LabelStyle, Ocr, OcrResult, Pdf, Restrictions, ScanMode,
    ScanOptions, ScanResult, Stamp, StampKind, StampPlace, Watermark,
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
    Info {
        input: PathBuf,
        #[arg(long)]
        password: Option<String>,
    },
    /// Set document information fields
    SetInfo {
        #[command(flatten)]
        io: Io,
        #[arg(long)]
        title: Option<String>,
        #[arg(long)]
        author: Option<String>,
        #[arg(long)]
        subject: Option<String>,
        #[arg(long)]
        keywords: Option<String>,
    },
    /// Merge PDFs in the order given
    Merge {
        #[arg(required = true, num_args = 2..)]
        inputs: Vec<PathBuf>,
        #[arg(short, long)]
        output: PathBuf,
    },
    /// Split into parts of N pages (out-dir/<stem>-001.pdf, ...) or at bookmarks (out-dir/<stem>-001 Title.pdf, ...)
    Split {
        input: PathBuf,
        #[arg(long, default_value_t = 1)]
        every: usize,
        /// Start a part at each bookmark down to this level (1 = top level only)
        #[arg(long, num_args = 0..=1, default_missing_value = "1", conflicts_with = "every")]
        bookmarks: Option<usize>,
        #[arg(long)]
        out_dir: PathBuf,
        #[arg(long)]
        password: Option<String>,
    },
    /// Keep only the listed pages, e.g. --pages 1-3,7
    Extract {
        #[command(flatten)]
        io: Io,
        #[arg(long)]
        pages: String,
    },
    /// Delete the listed pages
    Delete {
        #[command(flatten)]
        io: Io,
        #[arg(long)]
        pages: String,
    },
    /// Remove pages that look blank: one even colour, give or take specks or a lone page number
    RemoveBlank {
        #[command(flatten)]
        io: Io,
        /// Percent of a page that may differ from its background (0 = nothing at all)
        #[arg(long, default_value_t = mpe_core::BLANK_TOLERANCE)]
        tolerance: f32,
    },
    /// Rotate pages clockwise by 90, 180 or 270
    Rotate {
        #[command(flatten)]
        io: Io,
        #[arg(long, default_value = "all")]
        pages: String,
        #[arg(long)]
        degrees: i64,
    },
    /// Insert a blank page after page N (0 = at the start)
    InsertBlank {
        #[command(flatten)]
        io: Io,
        #[arg(long)]
        after: usize,
        #[arg(long, default_value_t = 612.0)]
        width: f32,
        #[arg(long, default_value_t = 792.0)]
        height: f32,
    },
    /// Make a PDF with one page per image (PNG, JPEG, TIFF; every page of a TIFF)
    Images {
        #[arg(required = true)]
        inputs: Vec<PathBuf>,
        #[arg(short, long)]
        output: PathBuf,
        #[command(flatten)]
        size: ImageSize,
    },
    /// Shrink and clean up scanned pages (pages that are one image): black and white
    /// becomes CCITT G4, greyscale and colour become JPEG
    OptimizeScans {
        #[command(flatten)]
        io: Io,
        #[arg(long, default_value = "all")]
        pages: String,
        #[arg(long, value_enum, default_value = "auto")]
        mode: ScanModeArg,
        /// Greyscale and colour images above this resolution are downsampled to it
        #[arg(long, default_value_t = 200.0)]
        max_dpi: f32,
        /// JPEG quality for greyscale and colour images, 1 to 100
        #[arg(long, default_value_t = 75)]
        quality: u8,
        /// Do not straighten tilted pages
        #[arg(long)]
        no_deskew: bool,
        /// Do not remove specks from black-and-white pages
        #[arg(long)]
        no_despeckle: bool,
    },
    /// Recognise text on pages that have none (scans) and add it as an invisible,
    /// searchable layer. Latin script only
    Ocr {
        #[command(flatten)]
        io: Io,
        #[arg(long, default_value = "all")]
        pages: String,
    },
    /// Insert images as pages after page N (0 = at the start)
    InsertImages {
        #[command(flatten)]
        io: Io,
        #[arg(long)]
        after: usize,
        #[arg(long, required = true, num_args = 1..)]
        images: Vec<PathBuf>,
        #[command(flatten)]
        size: ImageSize,
    },
    /// Set a page's crop box in points: left,bottom,right,top
    Crop {
        #[command(flatten)]
        io: Io,
        #[arg(long)]
        page: usize,
        #[arg(long)]
        rect: String,
    },
    /// Reorder all pages: a permutation such as 3,1,2
    Reorder {
        #[command(flatten)]
        io: Io,
        #[arg(long)]
        order: String,
    },
    /// Add a text or picture watermark
    Watermark {
        #[command(flatten)]
        io: Io,
        #[arg(long, required_unless_present = "image", conflicts_with = "image")]
        text: Option<String>,
        /// Use a PNG, JPEG or TIFF picture instead of text
        #[arg(long)]
        image: Option<PathBuf>,
        #[arg(long, default_value = "all")]
        pages: String,
        /// Font size of text
        #[arg(long, default_value_t = 96.0, conflicts_with = "image")]
        size: f32,
        /// Picture size, as a percentage of the page it fits inside
        #[arg(long, default_value_t = 50.0, conflicts_with = "text")]
        scale: f32,
        #[arg(long, default_value_t = 0.25)]
        opacity: f32,
        #[arg(long, default_value_t = 45.0)]
        rotation: f32,
        /// Put the watermark behind the page content
        #[arg(long)]
        background: bool,
    },
    /// Put a stamp on pages: Approved, Draft and so on, or a picture
    Stamp {
        #[command(flatten)]
        io: Io,
        /// approved, not-approved, draft, final, confidential, for-comment or void
        #[arg(long, required_unless_present = "image", conflicts_with = "image")]
        kind: Option<String>,
        /// Use a PNG, JPEG or TIFF picture instead
        #[arg(long)]
        image: Option<PathBuf>,
        #[arg(long, default_value = "1")]
        pages: String,
        /// Centre of the stamp as X,Y in points from the page's top left as shown;
        /// default: the top right corner
        #[arg(long)]
        at: Option<String>,
        /// Size, as a percentage of the stamp's own size
        #[arg(long, default_value_t = 100.0)]
        scale: f32,
        /// Add a line with the author's name and today's date
        #[arg(long, conflicts_with = "image")]
        date: bool,
        /// The stamp's author, also written on the date line
        #[arg(long, default_value = "")]
        author: String,
    },
    /// Add header/footer text; tokens: {page} {pages} {date} {bates}
    HeaderFooter {
        #[command(flatten)]
        io: Io,
        #[arg(long, default_value = "all")]
        pages: String,
        #[arg(long)]
        header_left: Option<String>,
        #[arg(long)]
        header_center: Option<String>,
        #[arg(long)]
        header_right: Option<String>,
        #[arg(long)]
        footer_left: Option<String>,
        #[arg(long)]
        footer_center: Option<String>,
        #[arg(long)]
        footer_right: Option<String>,
        #[arg(long, default_value_t = 10.0)]
        size: f32,
    },
    /// Bates-number the pages
    Bates {
        #[command(flatten)]
        io: Io,
        #[arg(long, default_value = "all")]
        pages: String,
        #[arg(long, default_value = "")]
        prefix: String,
        #[arg(long, default_value = "")]
        suffix: String,
        #[arg(long, default_value_t = 6)]
        digits: usize,
        #[arg(long, default_value_t = 1)]
        start: u64,
    },
    /// Password-protect a PDF
    Encrypt {
        #[command(flatten)]
        io: Io,
        #[arg(long, default_value = "")]
        user_password: String,
        #[arg(long)]
        owner_password: String,
        #[arg(long, value_enum, default_value = "aes256")]
        cipher: CipherArg,
        #[arg(long)]
        no_print: bool,
        #[arg(long)]
        no_copy: bool,
        #[arg(long)]
        no_modify: bool,
    },
    /// Remove password protection (needs the password)
    Decrypt {
        #[command(flatten)]
        io: Io,
    },
    /// Search page text
    Search {
        input: PathBuf,
        query: String,
        #[arg(long)]
        password: Option<String>,
        /// Also print where each match is on its page: left, top, right, bottom in points
        /// from the top-left of the displayed page
        #[arg(long)]
        boxes: bool,
    },
    /// List editable page objects (text lines, images, paths) with their boxes
    Objects {
        input: PathBuf,
        #[arg(long, default_value = "1")]
        pages: String,
        #[arg(long)]
        password: Option<String>,
    },
    /// Print extracted text
    Text {
        input: PathBuf,
        #[arg(long, default_value = "all")]
        pages: String,
        #[arg(long)]
        password: Option<String>,
    },
    /// Work with form fields
    Form {
        #[command(subcommand)]
        cmd: FormCmd,
    },
    /// Work with annotations: comments, notes, highlights and drawings
    Annot {
        #[command(subcommand)]
        cmd: AnnotCmd,
    },
    /// Work with page labels: the page numbers a document shows, such as i, ii, 1, 2
    Labels {
        #[command(subcommand)]
        cmd: LabelsCmd,
    },
    /// Work with bookmarks (the document outline)
    Bookmarks {
        #[command(subcommand)]
        cmd: BookmarksCmd,
    },
    /// List the printers; the default is marked
    Printers,
    /// Print (CUPS on Linux and macOS, the print spooler on Windows)
    Print {
        input: PathBuf,
        /// Printer name from `mpe printers`; default: the default printer
        #[arg(long)]
        printer: Option<String>,
        #[arg(long, default_value = "all")]
        pages: String,
        #[arg(long, default_value_t = 1)]
        copies: u32,
        #[arg(long, value_enum, default_value_t = SidesArg::One)]
        sides: SidesArg,
        /// Send pages rendered by MPE (300 dpi images) instead of the PDF itself. Windows
        /// always prints pages rendered by MPE
        #[arg(long)]
        as_image: bool,
        #[arg(long)]
        password: Option<String>,
    },
    /// Render pages to image files: out-dir/<stem>-<page>.png (or .jpg), replacing files
    /// of the same name
    Render {
        input: PathBuf,
        #[arg(long, default_value = "all")]
        pages: String,
        /// Resolution, at most 1152
        #[arg(long, default_value_t = 150.0)]
        dpi: f32,
        #[arg(long, value_enum, default_value_t = FormatArg::Png)]
        format: FormatArg,
        /// JPEG quality, 1 to 100
        #[arg(long, default_value_t = 90)]
        quality: u8,
        #[arg(long)]
        out_dir: PathBuf,
        #[arg(long)]
        password: Option<String>,
    },
}

#[derive(Copy, Clone, ValueEnum)]
enum FormatArg {
    Png,
    Jpeg,
}

#[derive(Copy, Clone, ValueEnum)]
enum LabelStyleArg {
    /// 1, 2, 3
    Decimal,
    /// i, ii, iii
    LowerRoman,
    /// I, II, III
    UpperRoman,
    /// a, b, c
    LowerLetters,
    /// A, B, C
    UpperLetters,
    /// The prefix alone
    None,
}

#[derive(Subcommand)]
enum LabelsCmd {
    /// List the label ranges: the pages each covers and their first and last labels
    List {
        input: PathBuf,
        #[arg(long)]
        password: Option<String>,
    },
    /// Number the pages from page N on, up to the next range that starts after it. Pages before the first
    /// range keep plain numbers
    Set {
        #[command(flatten)]
        io: Io,
        /// The first page to number (1-based)
        #[arg(long, default_value_t = 1)]
        from: usize,
        #[arg(long, value_enum, default_value_t = LabelStyleArg::Decimal)]
        style: LabelStyleArg,
        /// Text before each number, such as "A-"
        #[arg(long, default_value = "")]
        prefix: String,
        /// The number of the first page
        #[arg(long, default_value_t = 1)]
        start: u32,
    },
    /// Remove all page labels
    Remove {
        #[command(flatten)]
        io: Io,
    },
}

#[derive(Subcommand)]
enum BookmarksCmd {
    /// List the bookmarks, numbered, indented by level, with the page each goes to
    List {
        input: PathBuf,
        #[arg(long)]
        password: Option<String>,
    },
    /// Add a bookmark to a page. Without --after or --under it goes last at the top level
    Add {
        #[command(flatten)]
        io: Io,
        #[arg(long)]
        title: String,
        /// The page it goes to (1-based)
        #[arg(long)]
        page: usize,
        /// Put it just after bookmark N (from `list`), at its level
        #[arg(long, value_name = "N", conflicts_with = "under")]
        after: Option<usize>,
        /// Put it last among bookmark N's children
        #[arg(long, value_name = "N")]
        under: Option<usize>,
    },
    /// Give bookmark N (from `list`) a new title
    Rename {
        #[command(flatten)]
        io: Io,
        number: usize,
        #[arg(long)]
        title: String,
    },
    /// Remove bookmark N (from `list`) and its children
    Remove {
        #[command(flatten)]
        io: Io,
        number: usize,
    },
}

#[derive(Copy, Clone, ValueEnum)]
enum SidesArg {
    One,
    /// Two-sided, flipped on the long edge
    LongEdge,
    /// Two-sided, flipped on the short edge
    ShortEdge,
}

#[derive(Copy, Clone, ValueEnum)]
enum CipherArg {
    Rc4_40,
    Rc4_128,
    Aes128,
    Aes256,
}

#[derive(Copy, Clone, ValueEnum)]
enum ScanModeArg {
    /// Per page: black and white for text, else greyscale or colour
    Auto,
    Bw,
    Gray,
    Color,
}

fn kb(bytes: usize) -> String {
    format!("{:.0} kB", bytes as f64 / 1000.0)
}

/// Page size for imported images.
#[derive(Args, Clone)]
struct ImageSize {
    /// Resolution in dots per inch, overriding the one stored in the files
    #[arg(long)]
    dpi: Option<f32>,
    /// Resolution assumed for files that state none
    #[arg(long, default_value_t = 150.0)]
    fallback_dpi: f32,
}

impl ImageSize {
    fn opts(&self) -> ImageImport {
        ImageImport {
            dpi: self.dpi,
            fallback_dpi: self.fallback_dpi,
        }
    }
}

fn read_all(paths: &[PathBuf]) -> Result<Vec<Vec<u8>>> {
    paths
        .iter()
        .map(|p| std::fs::read(p).with_context(|| format!("reading {}", p.display())))
        .collect()
}

#[derive(Subcommand)]
enum FormCmd {
    /// List the fields: name, kind, value, pages and flags
    List {
        input: PathBuf,
        /// Print the values as a JSON object instead, keyed by field name
        #[arg(long)]
        json: bool,
        #[arg(long)]
        password: Option<String>,
    },
    /// Fill in fields and write the result. A values file is applied first, then each
    /// --set; any unknown name or unsuitable value stops it before anything is written
    Fill {
        #[command(flatten)]
        io: Io,
        /// Set one field: NAME=VALUE. Checkboxes take on or off; a multi-select list takes
        /// its choices separated by |
        #[arg(long = "set", value_name = "NAME=VALUE")]
        set: Vec<String>,
        /// A JSON object of values, as `mpe form list --json` prints them
        #[arg(long, value_name = "FILE")]
        values: Option<PathBuf>,
        /// Then flatten the form, so the values can't be changed
        #[arg(long)]
        flatten: bool,
    },
    /// Put every field back to its default value
    Reset {
        #[command(flatten)]
        io: Io,
    },
    /// Draw the fields into the pages as they look now and remove them, so the values
    /// can no longer be changed
    Flatten {
        #[command(flatten)]
        io: Io,
        /// Only these pages, e.g. 1-3,7; fields elsewhere stay fillable
        #[arg(long)]
        pages: Option<String>,
    },
}

#[derive(Subcommand)]
enum AnnotCmd {
    /// List the annotations: page, kind, author, date and comment. Links and form fields
    /// are not included
    List {
        input: PathBuf,
        /// Only these pages, e.g. 1-3,7
        #[arg(long)]
        pages: Option<String>,
        /// Print a JSON array instead
        #[arg(long)]
        json: bool,
        #[arg(long)]
        password: Option<String>,
    },
    /// Remove annotations, with their replies. With no options, removes them all
    Remove {
        #[command(flatten)]
        io: Io,
        /// Only on these pages, e.g. 1-3,7
        #[arg(long)]
        pages: Option<String>,
        /// Only these kinds, separated by commas: highlight, underline, strikeout,
        /// squiggly, note, textbox, square, circle, line, polygon, polyline, ink, stamp,
        /// or another subtype as the file names it
        #[arg(long = "type", value_name = "KINDS")]
        kinds: Option<String>,
        /// Only those by this author (exact match)
        #[arg(long)]
        author: Option<String>,
    },
    /// Draw the annotations into the pages as they look now and remove them, so they can
    /// no longer be changed. Links and form fields are kept
    Flatten {
        #[command(flatten)]
        io: Io,
        /// Only these pages, e.g. 1-3,7
        #[arg(long)]
        pages: Option<String>,
    },
}

fn annot_list(input: &Path, pages: &Option<String>, json: bool, pw: &Option<String>) -> Result<()> {
    let pdf = open(input, pw)?;
    let pages = match pages {
        Some(r) => Some(parse_page_range(r, pdf.page_count())?),
        None => None,
    };
    let list: Vec<Annot> = pdf
        .annotations()
        .into_iter()
        .filter(|a| pages.as_ref().is_none_or(|p| p.contains(&a.page)))
        .collect();
    let index = |id: (u32, u16)| list.iter().position(|a| a.id == id);
    // f32 noise such as 0.8999999761 left out.
    let round = |v: &[f32]| -> Vec<f64> {
        v.iter()
            .map(|&x| (f64::from(x) * 1000.0).round() / 1000.0)
            .collect()
    };
    if json {
        let rows: Vec<serde_json::Value> = list
            .iter()
            .enumerate()
            .map(|(i, a)| {
                serde_json::json!({
                    "index": i + 1,
                    "page": a.page + 1,
                    "type": a.kind.name(),
                    "author": a.author,
                    "modified": a.modified,
                    "contents": a.contents,
                    "subject": a.subject,
                    "color": a.color.map(|c| round(&c)),
                    "opacity": round(&[a.opacity])[0],
                    "rect": round(&a.bbox),
                    "reply_to": a.reply_to.and_then(index).map(|i| i + 1),
                    "hidden": a.hidden(),
                    "locked": a.locked(),
                })
            })
            .collect();
        out!("{}", serde_json::to_string_pretty(&rows)?);
        return Ok(());
    }
    if list.is_empty() {
        out!("no annotations");
    }
    for (i, a) in list.iter().enumerate() {
        let mut notes = Vec::new();
        if let Some(r) = a.reply_to.and_then(index) {
            notes.push(format!("reply to #{}", r + 1));
        }
        if a.hidden() {
            notes.push("hidden".into());
        }
        if a.locked() {
            notes.push("locked".into());
        }
        let dash = |s: &str| {
            if s.is_empty() {
                "-".to_string()
            } else {
                s.to_string()
            }
        };
        out!(
            "#{:<3} page {:<4} {:<10} {:<16} {:<16} {:?}{}",
            i + 1,
            a.page + 1,
            a.kind.name(),
            dash(&a.author),
            dash(&a.modified),
            a.contents,
            if notes.is_empty() {
                String::new()
            } else {
                format!("  ({})", notes.join(", "))
            }
        );
    }
    Ok(())
}

/// A checkbox value given as text: on/off and the like, or the box's export value.
fn check_from_text(f: &Field, s: &str) -> Result<bool> {
    match s.to_ascii_lowercase().as_str() {
        "on" | "yes" | "true" | "1" => Ok(true),
        "off" | "no" | "false" | "0" | "" => Ok(false),
        _ if f.options.iter().any(|o| o.0 == s) => Ok(true),
        _ => bail!("\"{}\" is a checkbox: give on or off, not \"{s}\"", f.name),
    }
}

/// A field's value from `--set NAME=VALUE` text.
fn value_from_text(f: &Field, s: &str) -> Result<FieldValue> {
    Ok(match &f.kind {
        FieldKind::Text { .. } => FieldValue::Text(s.to_string()),
        FieldKind::Check => FieldValue::Check(check_from_text(f, s)?),
        FieldKind::Choice { multi: true, .. } if !s.is_empty() => {
            FieldValue::Choice(s.split('|').map(str::to_string).collect())
        }
        FieldKind::Radio | FieldKind::Choice { .. } if s.is_empty() => FieldValue::Choice(vec![]),
        FieldKind::Radio | FieldKind::Choice { .. } => FieldValue::Choice(vec![s.to_string()]),
        FieldKind::Button | FieldKind::Signature => FieldValue::None,
    })
}

/// A field's value from a JSON value, as `form list --json` prints them.
fn value_from_json(f: &Field, v: &serde_json::Value) -> Result<FieldValue> {
    use serde_json::Value;
    let text = |v: &Value| -> Result<String> {
        match v {
            Value::String(s) => Ok(s.clone()),
            Value::Number(n) => Ok(n.to_string()),
            Value::Null => Ok(String::new()),
            _ => bail!("\"{}\" takes text, not {v}", f.name),
        }
    };
    Ok(match (&f.kind, v) {
        (FieldKind::Check, Value::Bool(b)) => FieldValue::Check(*b),
        (FieldKind::Check, v) => FieldValue::Check(check_from_text(f, &text(v)?)?),
        (FieldKind::Radio | FieldKind::Choice { .. }, Value::Array(a)) => {
            FieldValue::Choice(a.iter().map(text).collect::<Result<_>>()?)
        }
        (FieldKind::Radio | FieldKind::Choice { .. }, v) => {
            let s = text(v)?;
            FieldValue::Choice(if s.is_empty() { vec![] } else { vec![s] })
        }
        (_, v) => FieldValue::Text(text(v)?),
    })
}

/// Why an XFA-only form can't be listed or filled.
const XFA_ONLY: &str =
    "this form was made for Adobe Reader (an XFA form): its fields are built by \
                        Adobe software, so MPE can't list or fill them";

fn form_fill(io: &Io, set: &[String], values: &Option<PathBuf>, flatten: bool) -> Result<()> {
    let mut pdf = open(&io.input, &io.password)?;
    if pdf.xfa_only() {
        bail!("{XFA_ONLY}");
    }
    let fields = pdf.form_fields();
    let field = |name: &str| -> Result<&Field> {
        fields
            .iter()
            .find(|f| f.name == name)
            .with_context(|| format!("no form field named \"{name}\""))
    };
    let mut todo: Vec<(String, FieldValue)> = Vec::new();
    if let Some(path) = values {
        let text = std::fs::read_to_string(path)
            .with_context(|| format!("cannot read {}", path.display()))?;
        let json: serde_json::Value = serde_json::from_str(&text)
            .with_context(|| format!("{} is not valid JSON", path.display()))?;
        let serde_json::Value::Object(map) = json else {
            bail!(
                "{} must hold a JSON object of field names and values",
                path.display()
            );
        };
        for (name, v) in &map {
            todo.push((name.clone(), value_from_json(field(name)?, v)?));
        }
    }
    for s in set {
        let Some((name, v)) = s.split_once('=') else {
            bail!("--set takes NAME=VALUE, not \"{s}\"");
        };
        todo.push((name.to_string(), value_from_text(field(name)?, v)?));
    }
    if todo.is_empty() {
        bail!("nothing to fill in: give --set NAME=VALUE or --values FILE");
    }
    for (name, v) in &todo {
        pdf.set_field(name, v.clone())?;
    }
    if flatten {
        pdf.flatten_form(None)?;
    }
    pdf.save(&io.output)?;
    out!(
        "filled {} field(s){}; wrote {}",
        todo.len(),
        if flatten {
            " and flattened the form"
        } else {
            ""
        },
        io.output.display()
    );
    Ok(())
}

/// A field's kind for `form list`.
fn kind_name(k: &FieldKind) -> String {
    match k {
        FieldKind::Text {
            multiline,
            password,
            comb,
            max_len,
        } => {
            let mut s = String::from("text");
            for (on, what) in [
                (*multiline, "multiline"),
                (*password, "password"),
                (*comb, "comb"),
            ] {
                if on {
                    s += &format!(" {what}");
                }
            }
            if let Some(n) = max_len {
                s += &format!(" max {n}");
            }
            s
        }
        FieldKind::Check => "checkbox".into(),
        FieldKind::Radio => "radio".into(),
        FieldKind::Choice {
            combo,
            editable,
            multi,
        } => match (combo, editable, multi) {
            (true, true, _) => "combo editable".into(),
            (true, false, _) => "combo".into(),
            (false, _, true) => "list multi".into(),
            (false, _, false) => "list".into(),
        },
        FieldKind::Button => "button".into(),
        FieldKind::Signature => "signature".into(),
    }
}

/// A field's value as JSON: a string for text and single choices, true or false for
/// checkboxes, an array for multi-select lists, null when nothing is chosen. `None` for
/// fields without a value (push buttons and signatures).
fn json_value(f: &Field) -> Option<serde_json::Value> {
    use serde_json::Value;
    Some(match &f.value {
        FieldValue::Text(t) => Value::String(t.clone()),
        FieldValue::Check(on) => Value::Bool(*on),
        FieldValue::Choice(c) if matches!(f.kind, FieldKind::Choice { multi: true, .. }) => {
            Value::Array(c.iter().cloned().map(Value::String).collect())
        }
        FieldValue::Choice(c) => c.first().cloned().map_or(Value::Null, Value::String),
        FieldValue::None => return None,
    })
}

fn form_list(input: &Path, json: bool, password: &Option<String>) -> Result<()> {
    let pdf = open(input, password)?;
    if pdf.xfa_only() {
        // An empty object for scripts; the reason on stderr.
        if json {
            out!("{{}}");
        }
        eprintln!("{XFA_ONLY}");
        return Ok(());
    }
    let fields = pdf.form_fields();
    if json {
        // Written by hand to keep the form's order; serde_json's map would sort the names.
        let rows: Vec<String> = fields
            .iter()
            .filter_map(|f| {
                let v = json_value(f)?;
                Some(format!(
                    "  {}: {}",
                    serde_json::Value::from(f.name.clone()),
                    v
                ))
            })
            .collect();
        if rows.is_empty() {
            out!("{{}}");
        } else {
            out!("{{\n{}\n}}", rows.join(",\n"));
        }
        return Ok(());
    }
    if fields.is_empty() {
        out!("no form fields");
    }
    for f in &fields {
        let value = match &f.value {
            FieldValue::Text(t) => format!("{t:?}"),
            FieldValue::Check(on) => if *on { "on" } else { "off" }.into(),
            FieldValue::Choice(c) => format!("{c:?}"),
            FieldValue::None => "-".into(),
        };
        let mut pages: Vec<usize> = f.widgets.iter().map(|w| w.page + 1).collect();
        pages.sort_unstable();
        pages.dedup();
        let pages = match pages.as_slice() {
            [] => "no page".into(),
            [p] => format!("page {p}"),
            _ => format!(
                "pages {}",
                pages
                    .iter()
                    .map(|p| p.to_string())
                    .collect::<Vec<_>>()
                    .join(",")
            ),
        };
        let mut notes = vec![pages];
        if f.required {
            notes.push("required".into());
        }
        if f.read_only {
            notes.push("read-only".into());
        }
        if !f.options.is_empty() {
            let opts: Vec<&str> = f.options.iter().map(|o| o.0.as_str()).collect();
            notes.push(format!("options {opts:?}"));
        }
        out!(
            "{} [{}] = {} ({})",
            f.name,
            kind_name(&f.kind),
            value,
            notes.join(", ")
        );
    }
    Ok(())
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

/// A bookmark number from `mpe bookmarks list` as a position in the outline.
fn bookmark_index(n: usize) -> Result<usize> {
    n.checked_sub(1).context("bookmarks are numbered from 1")
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
            for (k, v) in [
                ("title", i.title),
                ("author", i.author),
                ("subject", i.subject),
                ("keywords", i.keywords),
                ("creator", i.creator),
                ("producer", i.producer),
            ] {
                if let Some(v) = v {
                    out!("{k}: {v}");
                }
            }
            for n in 0..p.page_count().min(5) {
                let (w, h) = p.page_size(n)?;
                out!("page {}: {:.1} x {:.1} pt", n + 1, w, h);
            }
            if p.page_count() > 5 {
                out!("...");
            }
        }
        Cmd::SetInfo {
            io,
            title,
            author,
            subject,
            keywords,
        } => run_edit(&io, |p| {
            p.set_info(&Info {
                title,
                author,
                subject,
                keywords,
                ..Default::default()
            })
        })?,
        Cmd::Merge { inputs, output } => {
            let docs = inputs
                .par_iter()
                .map(|p| Pdf::open(p, None))
                .collect::<Result<Vec<_>>>()?;
            let mut m = Pdf::merge(docs)?;
            m.save(&output)?;
            out!("wrote {} ({} pages)", output.display(), m.page_count());
        }
        Cmd::Split {
            input,
            every,
            bookmarks,
            out_dir,
            password,
        } => {
            let p = open(&input, &password)?;
            let stem = input
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("part")
                .to_string();
            let parts = match bookmarks {
                Some(level) => p.split_at_bookmarks(level)?,
                None => p
                    .split_every(every)?
                    .into_iter()
                    .map(|d| (None, d))
                    .collect(),
            };
            std::fs::create_dir_all(&out_dir)?;
            let n = parts.len();
            parts.into_par_iter().enumerate().try_for_each(
                |(i, (title, mut part))| -> Result<()> {
                    part.save(out_dir.join(part_file_name(&stem, i + 1, title.as_deref())))
                },
            )?;
            out!("split into {n} files in {}", out_dir.display());
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
        Cmd::RemoveBlank { io, tolerance } => run_edit(&io, |p| {
            let gone = p.remove_blank_pages(tolerance)?;
            match gone.len() {
                0 => out!("no blank pages"),
                n => out!("removed {n} blank page(s): {}", format_page_range(&gone)),
            }
            Ok(())
        })?,
        Cmd::Rotate { io, pages, degrees } => run_edit(&io, |p| {
            let idx = parse_page_range(&pages, p.page_count())?;
            p.rotate_pages(&idx, degrees)
        })?,
        Cmd::InsertBlank {
            io,
            after,
            width,
            height,
        } => run_edit(&io, |p| p.insert_blank(after, width, height))?,
        Cmd::Images {
            inputs,
            output,
            size,
        } => {
            let mut p = Pdf::from_images(&read_all(&inputs)?, &size.opts())?;
            p.save(&output)?;
            out!("wrote {} ({} pages)", output.display(), p.page_count());
        }
        Cmd::OptimizeScans {
            io,
            pages,
            mode,
            max_dpi,
            quality,
            no_deskew,
            no_despeckle,
        } => run_edit(&io, |p| {
            let idx = parse_page_range(&pages, p.page_count())?;
            let opts = ScanOptions {
                mode: match mode {
                    ScanModeArg::Auto => ScanMode::Auto,
                    ScanModeArg::Bw => ScanMode::BlackWhite,
                    ScanModeArg::Gray => ScanMode::Gray,
                    ScanModeArg::Color => ScanMode::Color,
                },
                max_dpi,
                jpeg_quality: quality,
                deskew: !no_deskew,
                despeckle: !no_despeckle,
            };
            let (mut before_all, mut after_all) = (0, 0);
            for (i, r) in p.optimize_scans(&idx, &opts)? {
                match r {
                    ScanResult::Optimized {
                        mode,
                        before,
                        after,
                        deskewed,
                    } => {
                        let kind = match mode {
                            ScanMode::BlackWhite => "black and white",
                            ScanMode::Gray => "greyscale",
                            _ => "colour",
                        };
                        let tilt = deskewed
                            .map(|a| format!(", straightened {a:.1}°"))
                            .unwrap_or_default();
                        out!(
                            "page {}: {kind}, {} -> {}{tilt}",
                            i + 1,
                            kb(before),
                            kb(after)
                        );
                        before_all += before;
                        after_all += after;
                    }
                    ScanResult::Skipped(why) => out!("page {}: skipped: {why}", i + 1),
                }
            }
            out!("images: {} -> {}", kb(before_all), kb(after_all));
            Ok(())
        })?,
        Cmd::Ocr { io, pages } => run_edit(&io, |p| {
            let idx = parse_page_range(&pages, p.page_count())?;
            let ocr = Ocr::bundled()?;
            for (i, r) in p.ocr_pages(&idx, &ocr)? {
                match r {
                    OcrResult::Added { lines, words } => {
                        out!("page {}: {lines} lines, {words} words", i + 1)
                    }
                    OcrResult::Skipped(why) => out!("page {}: skipped: {why}", i + 1),
                }
            }
            Ok(())
        })?,
        Cmd::InsertImages {
            io,
            after,
            images,
            size,
        } => {
            let files = read_all(&images)?;
            run_edit(&io, |p| {
                p.insert_image_pages(after, &files, &size.opts()).map(drop)
            })?
        }
        Cmd::Crop { io, page, rect } => run_edit(&io, |p| {
            let v: Vec<f32> = rect
                .split(',')
                .map(|x| x.trim().parse())
                .collect::<Result<_, _>>()
                .context("--rect needs four numbers: left,bottom,right,top")?;
            if v.len() != 4 {
                bail!("--rect needs four numbers: left,bottom,right,top");
            }
            if page == 0 {
                bail!("pages are numbered from 1");
            }
            p.crop_page(page - 1, [v[0], v[1], v[2], v[3]])
        })?,
        Cmd::Reorder { io, order } => run_edit(&io, |p| {
            let o: Vec<usize> = order
                .split(',')
                .map(|x| x.trim().parse::<usize>().map(|n| n.wrapping_sub(1)))
                .collect::<Result<_, _>>()?;
            p.reorder(&o)
        })?,
        Cmd::Watermark {
            io,
            text,
            image,
            pages,
            size,
            scale,
            opacity,
            rotation,
            background,
        } => {
            let image = match image {
                Some(path) => Some(
                    std::fs::read(&path).with_context(|| format!("reading {}", path.display()))?,
                ),
                None => None,
            };
            run_edit(&io, |p| {
                let idx = parse_page_range(&pages, p.page_count())?;
                match image {
                    Some(image) => p.add_image_watermark(
                        &idx,
                        &ImageWatermark {
                            image,
                            scale: scale / 100.0,
                            opacity,
                            rotation,
                            foreground: !background,
                        },
                    ),
                    None => p.add_watermark(
                        &idx,
                        &Watermark {
                            text: text.unwrap_or_default(),
                            font_size: size,
                            opacity,
                            rotation,
                            foreground: !background,
                            ..Default::default()
                        },
                    ),
                }
            })?
        }
        Cmd::Stamp {
            io,
            kind,
            image,
            pages,
            at,
            scale,
            date,
            author,
        } => {
            if !(scale > 0.0 && scale.is_finite()) {
                bail!("--scale must be above 0");
            }
            let image = match image {
                Some(path) => Some(
                    std::fs::read(&path).with_context(|| format!("reading {}", path.display()))?,
                ),
                None => None,
            };
            let stamp = match &image {
                Some(bytes) => Stamp::Picture(bytes),
                None => {
                    let name = kind.unwrap_or_default();
                    Stamp::Text {
                        kind: StampKind::from_name(&name)
                            .with_context(|| format!("no stamp called {name:?}"))?,
                        name_and_date: date,
                    }
                }
            };
            let at = match at {
                Some(s) => {
                    let (x, y) = s.split_once(',').context("--at takes X,Y")?;
                    Some((x.trim().parse::<f32>()?, y.trim().parse::<f32>()?))
                }
                None => None,
            };
            let (w, h) = stamp.size(&author)?;
            let (w, h) = (w * scale / 100.0, h * scale / 100.0);
            run_edit(&io, |p| {
                for i in parse_page_range(&pages, p.page_count())? {
                    let (x, y) = match at {
                        Some(xy) => xy,
                        None => (p.page_size(i)?.0 - 36.0 - w / 2.0, 36.0 + h / 2.0),
                    };
                    let r = [x - w / 2.0, y - h / 2.0, x + w / 2.0, y + h / 2.0];
                    p.add_stamp(i, &stamp, StampPlace::Fill(r), &author)?;
                }
                Ok(())
            })?
        }
        Cmd::HeaderFooter {
            io,
            pages,
            header_left,
            header_center,
            header_right,
            footer_left,
            footer_center,
            footer_right,
            size,
        } => run_edit(&io, |p| {
            let idx = parse_page_range(&pages, p.page_count())?;
            p.add_header_footer(
                &idx,
                &HeaderFooter {
                    header: [
                        some(&header_left),
                        some(&header_center),
                        some(&header_right),
                    ],
                    footer: [
                        some(&footer_left),
                        some(&footer_center),
                        some(&footer_right),
                    ],
                    font_size: size,
                    ..Default::default()
                },
            )
        })?,
        Cmd::Bates {
            io,
            pages,
            prefix,
            suffix,
            digits,
            start,
        } => run_edit(&io, |p| {
            let idx = parse_page_range(&pages, p.page_count())?;
            p.add_header_footer(
                &idx,
                &HeaderFooter {
                    header: Default::default(),
                    footer: Default::default(),
                    bates: Some(Bates {
                        prefix,
                        suffix,
                        digits,
                        start,
                    }),
                    ..Default::default()
                },
            )
        })?,
        Cmd::Encrypt {
            io,
            user_password,
            owner_password,
            cipher,
            no_print,
            no_copy,
            no_modify,
        } => run_edit(&io, |p| {
            p.set_encryption(Some(Encryption {
                user_password,
                owner_password,
                cipher: match cipher {
                    CipherArg::Rc4_40 => Cipher::Rc4_40,
                    CipherArg::Rc4_128 => Cipher::Rc4_128,
                    CipherArg::Aes128 => Cipher::Aes128,
                    CipherArg::Aes256 => Cipher::Aes256,
                },
                restrictions: Restrictions {
                    print: !no_print,
                    copy: !no_copy,
                    modify: !no_modify,
                    ..Default::default()
                },
            }));
            Ok(())
        })?,
        Cmd::Decrypt { io } => run_edit(&io, |_| Ok(()))?,
        Cmd::Search {
            input,
            query,
            password,
            boxes,
        } => {
            let p = open(&input, &password)?;
            let hits = p.search(&query)?;
            for h in &hits {
                out!("p{}: …{}…", h.page + 1, h.snippet);
            }
            out!("{} match(es)", hits.len());
            if boxes {
                let mut pages: Vec<usize> = hits.iter().map(|h| h.page).collect();
                pages.dedup();
                for i in pages {
                    for b in p.find_text_boxes(i, &query)? {
                        out!(
                            "p{} [{:.1} {:.1} {:.1} {:.1}]",
                            i + 1,
                            b[0],
                            b[1],
                            b[2],
                            b[3]
                        );
                    }
                }
            }
        }
        Cmd::Objects {
            input,
            pages,
            password,
        } => {
            let p = open(&input, &password)?;
            for i in parse_page_range(&pages, p.page_count())? {
                out!("--- page {} ---", i + 1);
                for o in p.page_objects(i)? {
                    out!(
                        "#{:<3} {:?}{} [{:.1} {:.1} {:.1} {:.1}] {}",
                        o.id,
                        o.kind,
                        if o.hidden { " (hidden)" } else { "" },
                        o.bbox[0],
                        o.bbox[1],
                        o.bbox[2],
                        o.bbox[3],
                        o.text
                            .as_deref()
                            .map(|t| format!(
                                "{:?} ({} {:.1}pt)",
                                t,
                                o.font_name.clone().unwrap_or_default(),
                                o.font_size.unwrap_or(0.0)
                            ))
                            .unwrap_or_default()
                    );
                }
            }
        }
        Cmd::Form { cmd } => match cmd {
            FormCmd::List {
                input,
                json,
                password,
            } => form_list(&input, json, &password)?,
            FormCmd::Fill {
                io,
                set,
                values,
                flatten,
            } => form_fill(&io, &set, &values, flatten)?,
            FormCmd::Reset { io } => run_edit(&io, |p| p.reset_form())?,
            FormCmd::Flatten { io, pages } => {
                let mut p = open(&io.input, &io.password)?;
                if p.xfa_only() {
                    bail!("{XFA_ONLY}");
                }
                if p.form_fields().is_empty() {
                    bail!("{} has no form fields", io.input.display());
                }
                let idx = match pages {
                    Some(r) => Some(parse_page_range(&r, p.page_count())?),
                    None => None,
                };
                let n = p.flatten_form(idx.as_deref())?;
                p.save(&io.output)?;
                out!(
                    "flattened {n} field widget(s); wrote {}",
                    io.output.display()
                );
            }
        },
        Cmd::Labels { cmd } => match cmd {
            LabelsCmd::List { input, password } => {
                let p = open(&input, &password)?;
                let (ranges, labels) = (p.label_ranges(), p.page_labels());
                let Some(labels) = labels else {
                    out!("no page labels");
                    return Ok(());
                };
                // Pages before the first range are numbered plainly.
                let mut starts: Vec<usize> = ranges.iter().map(|r| r.page).collect();
                if starts.first() != Some(&0) {
                    starts.insert(0, 0);
                }
                for (k, &s) in starts.iter().enumerate() {
                    let e = starts.get(k + 1).map_or(labels.len(), |&n| n) - 1;
                    let pages = format_page_range(&(s..=e).collect::<Vec<_>>());
                    match s == e {
                        true => out!("{pages:<12} {}", labels[s]),
                        false => out!("{pages:<12} {} to {}", labels[s], labels[e]),
                    }
                }
            }
            LabelsCmd::Set {
                io,
                from,
                style,
                prefix,
                start,
            } => run_edit(&io, |p| {
                if from == 0 || from > p.page_count() {
                    bail!(
                        "page {from} does not exist (document has {})",
                        p.page_count()
                    );
                }
                let style = match style {
                    LabelStyleArg::Decimal => Some(LabelStyle::Decimal),
                    LabelStyleArg::LowerRoman => Some(LabelStyle::LowerRoman),
                    LabelStyleArg::UpperRoman => Some(LabelStyle::UpperRoman),
                    LabelStyleArg::LowerLetters => Some(LabelStyle::LowerLetters),
                    LabelStyleArg::UpperLetters => Some(LabelStyle::UpperLetters),
                    LabelStyleArg::None => None,
                };
                p.set_label_range(LabelRange {
                    page: from - 1,
                    style,
                    prefix,
                    first: start.max(1),
                })
            })?,
            LabelsCmd::Remove { io } => run_edit(&io, |p| p.set_label_ranges(&[]))?,
        },
        Cmd::Bookmarks { cmd } => match cmd {
            BookmarksCmd::List { input, password } => {
                let p = open(&input, &password)?;
                let items = p.outline();
                if items.is_empty() {
                    out!("no bookmarks");
                }
                let width = items.len().to_string().len();
                for (n, item) in items.iter().enumerate() {
                    let page = item.page.map_or("-".into(), |p| (p + 1).to_string());
                    let indent = "  ".repeat(item.depth);
                    out!("{:>width$}  p. {page:<5} {indent}{}", n + 1, item.title);
                }
            }
            BookmarksCmd::Add {
                io,
                title,
                page,
                after,
                under,
            } => run_edit(&io, |p| {
                let place = match (after, under) {
                    (Some(n), _) => BookmarkPlace::After(bookmark_index(n)?),
                    (_, Some(n)) => BookmarkPlace::Under(bookmark_index(n)?),
                    _ => BookmarkPlace::End,
                };
                if page == 0 {
                    bail!("pages are numbered from 1");
                }
                p.add_bookmark(&title, page - 1, place).map(|_| ())
            })?,
            BookmarksCmd::Rename { io, number, title } => {
                run_edit(&io, |p| p.rename_bookmark(bookmark_index(number)?, &title))?
            }
            BookmarksCmd::Remove { io, number } => {
                run_edit(&io, |p| p.remove_bookmark(bookmark_index(number)?))?
            }
        },
        Cmd::Annot { cmd } => match cmd {
            AnnotCmd::List {
                input,
                pages,
                json,
                password,
            } => annot_list(&input, &pages, json, &password)?,
            AnnotCmd::Remove {
                io,
                pages,
                kinds,
                author,
            } => {
                let mut p = open(&io.input, &io.password)?;
                let pages = match pages {
                    Some(r) => Some(parse_page_range(&r, p.page_count())?),
                    None => None,
                };
                let kinds: Option<Vec<AnnotKind>> = kinds.map(|k| {
                    k.split(',')
                        .map(str::trim)
                        .filter(|s| !s.is_empty())
                        .map(AnnotKind::parse)
                        .collect()
                });
                let n = p.remove_annotations(|a| {
                    pages.as_ref().is_none_or(|p| p.contains(&a.page))
                        && kinds.as_ref().is_none_or(|k| k.contains(&a.kind))
                        && author.as_ref().is_none_or(|w| *w == a.author)
                })?;
                p.save(&io.output)?;
                out!("removed {n} annotation(s); wrote {}", io.output.display());
            }
            AnnotCmd::Flatten { io, pages } => {
                let mut p = open(&io.input, &io.password)?;
                let idx = match pages {
                    Some(r) => Some(parse_page_range(&r, p.page_count())?),
                    None => None,
                };
                let n = p.flatten_annotations(idx.as_deref())?;
                p.save(&io.output)?;
                out!("flattened {n} annotation(s); wrote {}", io.output.display());
            }
        },
        Cmd::Printers => {
            let list = print::printers()?;
            if list.is_empty() {
                out!("no printers are set up");
            }
            for p in list {
                out!("{}{}", p.name, if p.default { " (default)" } else { "" });
            }
        }
        Cmd::Print {
            input,
            printer,
            pages,
            copies,
            sides,
            as_image,
            password,
        } => {
            let mut p = open(&input, &password)?;
            let idx = parse_page_range(&pages, p.page_count())?;
            let opts = print::PrintOptions {
                printer,
                copies,
                sides: match sides {
                    SidesArg::One => print::Sides::One,
                    SidesArg::LongEdge => print::Sides::LongEdge,
                    SidesArg::ShortEdge => print::Sides::ShortEdge,
                },
                as_image,
                title: input
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_default(),
            };
            out!("{}", p.print(&idx, &opts)?);
        }
        Cmd::Text {
            input,
            pages,
            password,
        } => {
            let p = open(&input, &password)?;
            for i in parse_page_range(&pages, p.page_count())? {
                out!("--- page {} ---\n{}", i + 1, p.page_text(i));
            }
        }
        Cmd::Render {
            input,
            pages,
            dpi,
            format,
            quality,
            out_dir,
            password,
        } => {
            // As the app shows it (NoView annotations hidden, missing annotation drawings
            // added), or the file as it is when the document reader can't open it.
            let hp = match open(&input, &password).and_then(|mut p| p.render_snapshot()) {
                Ok(bytes) => render::load(bytes, None)?,
                Err(_) => render::load(std::fs::read(&input)?, password.as_deref())?,
            };
            let n = render::Renderer::new(&hp).page_count();
            let idx = parse_page_range(&pages, n)?;
            let stem = input
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("page")
                .to_string();
            let format = match format {
                FormatArg::Png => render::ImageFormat::Png,
                FormatArg::Jpeg => render::ImageFormat::Jpeg(quality),
            };
            let t = std::time::Instant::now();
            render::write_images(&hp, &idx, dpi, format, &out_dir, &stem)?;
            out!("rendered {} page(s) in {:.2?}", idx.len(), t.elapsed());
        }
    }
    Ok(())
}

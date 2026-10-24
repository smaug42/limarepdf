use anyhow::{anyhow, bail, Context, Result};

/// println! that returns from the function when the output can't be written: see [`unwritten`].
macro_rules! out {
    ($($t:tt)*) => {{
        use std::io::Write;
        writeln!(std::io::stdout(), $($t)*).map_err(unwritten)?;
    }};
}
use clap::{Args, Parser, Subcommand, ValueEnum};
use limarepdf_core::{display_line, DigitalId, IdStore, NewId, SignPlace, SignRequest};
use limarepdf_core::{
    format_page_range, parse_page_range, part_file_name, print, render, Annot, AnnotKind, Bates,
    BookmarkPlace, Cipher, Encryption, Field, FieldKind, FieldValue, HeaderFooter, ImageImport,
    ImageWatermark, Info, LabelRange, LabelStyle, Ocr, OcrResult, OpenLayout, OpenPanel, Pdf,
    Restrictions, ScanMode, ScanOptions, ScanResult, SplitBy, Stamp, StampColor, StampKind,
    StampPlace, ViewPrefs, Watermark,
};
use limarepdf_core::{sign_label, InkColor, SignArt, SignRole, SignStyle};
use limarepdf_core::{
    CertInfo, ChangeKind, FieldLock, LockAction, Permissions, Refusal, SigKind, SigStatus,
    SignatureReport, TimeSource, Trust, TrustList, TrustRole,
};
use rayon::prelude::*;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use zeroize::Zeroizing;

#[derive(Parser)]
#[command(
    name = "limarepdf-cli",
    version,
    about = "Fast PDF toolkit (pure Rust)"
)]
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
    /// For a digitally signed input: save even when its signatures don't allow the
    /// changes (they stay valid for what was signed, and report the changes)
    #[arg(long)]
    accept_disallowed: bool,
    /// For a digitally signed input whose changes LimarePDF can't add after what was signed:
    /// write the whole file anyway, which breaks the signatures
    #[arg(long)]
    break_signatures: bool,
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
        /// Replace files already at the parts' names; without it nothing is written then
        #[arg(long)]
        overwrite: bool,
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
        #[arg(long, default_value_t = limarepdf_core::BLANK_TOLERANCE)]
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
    /// Remove text, pictures and drawings under areas or wherever some text occurs, for
    /// good (not just covered), and paint the areas black
    Redact {
        #[command(flatten)]
        io: Io,
        /// An area as PAGE:X,Y,W,H, in points from the page's top left as shown; repeat
        /// for more
        #[arg(long = "area", required_unless_present_any = ["texts", "apply"])]
        areas: Vec<String>,
        /// Text to remove wherever it occurs (case is ignored); repeat for more
        #[arg(long = "text")]
        texts: Vec<String>,
        /// Only mark the areas for redaction, to be checked (in LimarePDF or another app) and
        /// applied later; nothing is removed
        #[arg(long, conflicts_with = "apply")]
        mark_only: bool,
        /// Also apply the redaction marks already in the file
        #[arg(long)]
        apply: bool,
        /// Print what was removed as JSON
        #[arg(long)]
        json: bool,
    },
    /// Remove hidden information before sharing: metadata, scripts, attachments, hidden
    /// layers and private data (and, if asked, comments, form values, bookmarks and what
    /// lies outside the page area)
    Sanitise {
        input: PathBuf,
        /// Output PDF (not needed with --list)
        #[arg(short, long, required_unless_present = "list")]
        output: Option<PathBuf>,
        #[arg(long)]
        password: Option<String>,
        /// Kinds to keep, comma-separated: metadata, scripts, attachments, hidden-layers,
        /// private-data
        #[arg(long, default_value = "")]
        keep: String,
        /// Kinds to remove as well, comma-separated: comments, form-values, bookmarks,
        /// outside-crop
        #[arg(long, default_value = "")]
        also: String,
        /// Only list what the document has of each kind; nothing is written
        #[arg(long)]
        list: bool,
        /// For a digitally signed input: go ahead, although writing the cleaned file breaks
        /// its signatures (what is removed would otherwise stay in the signed version)
        #[arg(long)]
        break_signatures: bool,
    },
    /// Put a stamp on pages: Approved, Draft and so on, your own wording, a picture, or a
    /// typed signature
    Stamp {
        #[command(flatten)]
        io: Io,
        /// approved, not-approved, draft, final, confidential, for-comment or void
        #[arg(long, required_unless_present_any = ["image", "text", "signature"], conflicts_with_all = ["image", "text", "signature"])]
        kind: Option<String>,
        /// Your own wording instead, shown in capitals
        #[arg(long, conflicts_with_all = ["image", "signature"])]
        text: Option<String>,
        /// Sign with this name, in handwriting (not a digital signature)
        #[arg(long, conflicts_with = "image")]
        signature: Option<String>,
        /// The handwriting: 1 Caveat, 2 Cedarville Cursive, 3 Homemade Apple, 4 Zeyada
        #[arg(long, requires = "signature", default_value_t = 1)]
        style: u8,
        /// The colour of your own wording: green, red or blue (default red); of a
        /// signature: black or blue (default black)
        #[arg(long)]
        color: Option<String>,
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
        /// Add a line with the author's name and today's date; under a signature, the
        /// date only
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
    /// Check the digital signatures in a PDF. Exits with status 1 unless every signature
    /// checks as valid
    Verify {
        input: PathBuf,
        /// Print the results as JSON
        #[arg(long)]
        json: bool,
        #[arg(long)]
        password: Option<String>,
        /// Use this trust list instead of LimarePDF's own (see `limarepdf-cli trust`)
        #[arg(long, value_name = "FILE")]
        trust_file: Option<PathBuf>,
        /// Also exit with status 1 unless the file has signatures and every signer is
        /// trusted
        #[arg(long)]
        require_trusted: bool,
        /// Save the version signature N (numbered as listed) signed, instead of the
        /// results
        #[arg(long, value_name = "N", requires = "output")]
        signed_version: Option<usize>,
        /// Where to save the signed version
        #[arg(short, long, requires = "signed_version")]
        output: Option<PathBuf>,
    },
    /// Sign a PDF with your digital ID (see `limarepdf-cli id`), in an empty signature field or in a
    /// new box. The signature is added after the file's existing bytes, so earlier
    /// signatures stay valid, and the signed copy is checked before it is written
    Sign {
        input: PathBuf,
        /// Where to save the signed copy
        #[arg(short, long)]
        output: PathBuf,
        /// The digital ID: its number or name in `limarepdf-cli id list`; default: the default ID
        #[arg(long)]
        id: Option<String>,
        /// Sign this empty signature field (its full name, as `limarepdf-cli verify` lists it)
        #[arg(long, conflicts_with_all = ["page", "rect"], required_unless_present = "rect")]
        field: Option<String>,
        /// The page for a new signature box
        #[arg(long, requires = "rect")]
        page: Option<usize>,
        /// A new signature box as X,Y,W,H in points from the page's top left as shown
        #[arg(long, requires = "page")]
        rect: Option<String>,
        /// Why you sign, shown with the signature
        #[arg(long)]
        reason: Option<String>,
        /// Where you sign, shown with the signature
        #[arg(long)]
        location: Option<String>,
        /// Certify the document as its author instead (only before anyone signs it), saying
        /// what others may still do: no-changes, forms (fill in and sign) or comments (also
        /// comment)
        #[arg(long, value_enum)]
        certify: Option<CertifyLevel>,
        /// Show this PNG, JPEG or TIFF picture of your handwritten signature beside the
        /// text; default: your name in handwriting
        #[arg(long)]
        image: Option<PathBuf>,
        /// The handwriting your name is shown in: 1 Caveat, 2 Cedarville Cursive, 3 Homemade
        /// Apple, 4 Zeyada
        #[arg(long, conflicts_with = "image", default_value_t = 1)]
        style: u8,
        /// Read the ID's password from the first line of this file instead of asking
        #[arg(long, value_name = "FILE")]
        password_file: Option<PathBuf>,
        #[command(flatten)]
        folder: IdFolder,
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
    /// Choose whose digital signatures LimarePDF trusts: signers, and certificate authorities
    /// that vouch for signers. Affects LimarePDF only
    Trust {
        #[command(subcommand)]
        cmd: TrustCmd,
    },
    /// Your digital IDs, for signing documents: make one, import one (.p12, .pfx), export,
    /// list or remove them
    Id {
        #[command(subcommand)]
        cmd: IdCmd,
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
    /// Work with viewer preferences: the layout, side panel and page a document opens with
    ViewPrefs {
        #[command(subcommand)]
        cmd: ViewPrefsCmd,
    },
    /// List the printers; the default is marked
    Printers,
    /// Print (CUPS on Linux and macOS, the print spooler on Windows)
    Print {
        input: PathBuf,
        /// Printer name from `limarepdf-cli printers`; default: the default printer
        #[arg(long)]
        printer: Option<String>,
        #[arg(long, default_value = "all")]
        pages: String,
        #[arg(long, default_value_t = 1)]
        copies: u32,
        #[arg(long, value_enum, default_value_t = SidesArg::One)]
        sides: SidesArg,
        /// Send pages rendered by LimarePDF (300 dpi images) instead of the PDF itself. Windows
        /// always prints pages rendered by LimarePDF
        #[arg(long)]
        as_image: bool,
        #[arg(long)]
        password: Option<String>,
    },
    /// Render pages to image files: out-dir/<stem>-<page>.png (or .jpg)
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
        /// Replace files already at the images' names; without it nothing is written then
        #[arg(long)]
        overwrite: bool,
        #[arg(long)]
        password: Option<String>,
    },
}

#[derive(Copy, Clone, ValueEnum)]
enum CertifyLevel {
    /// No changes at all
    NoChanges,
    /// Filling in forms and signing
    Forms,
    /// Filling in forms, signing and comments
    Comments,
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

#[derive(Copy, Clone, ValueEnum)]
enum OpenLayoutArg {
    /// One column of pages, scrolled
    Continuous,
    /// Two pages side by side, odd pages on the left
    Facing,
    /// Two pages side by side, the first page alone on the right
    FacingCover,
    /// One page at a time
    Single,
    /// Remove the preference
    None,
}

#[derive(Copy, Clone, ValueEnum)]
enum OpenPanelArg {
    /// Page thumbnails
    Pages,
    /// The bookmarks
    Bookmarks,
    /// Remove the preference
    None,
}

#[derive(Subcommand)]
enum ViewPrefsCmd {
    /// List the document's viewer preferences
    List {
        input: PathBuf,
        #[arg(long)]
        password: Option<String>,
    },
    /// Set viewer preferences; those not given are kept
    Set {
        #[command(flatten)]
        io: Io,
        #[arg(long, value_enum)]
        layout: Option<OpenLayoutArg>,
        /// The side panel shown on opening
        #[arg(long, value_enum)]
        panel: Option<OpenPanelArg>,
        /// The page the document opens on (1-based), or "none"
        #[arg(long, value_parser = parse_open_page)]
        page: Option<OpenPageArg>,
    },
    /// Remove all viewer preferences LimarePDF knows (an opening action that isn't a page is kept)
    Clear {
        #[command(flatten)]
        io: Io,
    },
}

#[derive(Copy, Clone)]
enum OpenPageArg {
    Page(usize),
    None,
}

fn parse_open_page(s: &str) -> std::result::Result<OpenPageArg, String> {
    match s {
        "none" => Ok(OpenPageArg::None),
        _ => match s.parse::<usize>() {
            Ok(n) if n >= 1 => Ok(OpenPageArg::Page(n)),
            _ => Err("expected a page number from 1, or \"none\"".into()),
        },
    }
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
enum TrustCmd {
    /// List the certificates LimarePDF trusts
    List {
        /// Print the list as JSON
        #[arg(long)]
        json: bool,
        /// Use this trust list instead of LimarePDF's own
        #[arg(long, value_name = "FILE")]
        trust_file: Option<PathBuf>,
    },
    /// Trust a certificate: from a certificate file (.cer, .crt, .pem, .p7c), or the
    /// signer of a signature in a PDF. Compare its SHA-256 fingerprint with its owner first
    Add {
        input: PathBuf,
        /// Trust it as a certificate authority: every signer it issued a certificate to
        #[arg(long)]
        authority: bool,
        /// From a PDF: the signature whose signer to trust, numbered as `limarepdf-cli verify`
        /// lists them (needed when there are several)
        #[arg(long, value_name = "N")]
        signature: Option<usize>,
        /// From a file holding several certificates: which one, numbered from 1
        #[arg(long, value_name = "N")]
        pick: Option<usize>,
        /// A name to show for it; default: the name in the certificate
        #[arg(long)]
        label: Option<String>,
        /// Opens an encrypted PDF
        #[arg(long)]
        password: Option<String>,
        /// Use this trust list instead of LimarePDF's own
        #[arg(long, value_name = "FILE")]
        trust_file: Option<PathBuf>,
    },
    /// Stop trusting a certificate: its number in `limarepdf-cli trust list`, or its SHA-256
    /// fingerprint
    Remove {
        which: String,
        /// Use this trust list instead of LimarePDF's own
        #[arg(long, value_name = "FILE")]
        trust_file: Option<PathBuf>,
    },
}

#[derive(Subcommand)]
enum IdCmd {
    /// List your digital IDs
    List {
        /// Print the list as JSON
        #[arg(long)]
        json: bool,
        #[command(flatten)]
        folder: IdFolder,
    },
    /// Make a self-signed digital ID (RSA 3072-bit, valid five years), protected by a
    /// password you choose. Others trust your signatures once you send them its certificate
    Create {
        /// Your name, as your signatures will show it
        #[arg(long)]
        name: String,
        #[arg(long)]
        organisation: Option<String>,
        #[arg(long)]
        email: Option<String>,
        /// Read the new password from the first line of this file instead of asking
        #[arg(long, value_name = "FILE")]
        password_file: Option<PathBuf>,
        #[command(flatten)]
        folder: IdFolder,
    },
    /// Import a digital ID from a .p12 or .pfx file (from Acrobat, Windows, macOS or a
    /// certificate authority). It keeps its password
    Import {
        input: PathBuf,
        /// A name for it in LimarePDF; default: the name in its certificate
        #[arg(long)]
        name: Option<String>,
        /// Read the file's password from the first line of this file instead of asking
        #[arg(long, value_name = "FILE")]
        password_file: Option<PathBuf>,
        #[command(flatten)]
        folder: IdFolder,
    },
    /// Export a digital ID. To OUT.p12 or OUT.pfx: the ID with its key, to use in other
    /// apps or keep as a backup. To OUT.cer: its certificate only, for others to trust
    Export {
        /// The ID: its number or name in `limarepdf-cli id list`
        which: String,
        #[arg(short, long)]
        output: PathBuf,
        /// Read the ID's password from the first line of this file instead of asking
        #[arg(long, value_name = "FILE")]
        password_file: Option<PathBuf>,
        /// Protect the exported .p12 with a new password (asked twice) instead of the ID's
        #[arg(long)]
        new_password: bool,
        /// Read the exported .p12's new password from the first line of this file
        #[arg(long, value_name = "FILE")]
        new_password_file: Option<PathBuf>,
        #[command(flatten)]
        folder: IdFolder,
    },
    /// Remove a digital ID. It can't sign again unless you have an exported copy
    Remove {
        /// The ID: its number or name in `limarepdf-cli id list`
        which: String,
        /// Remove it without asking first
        #[arg(long)]
        yes: bool,
        #[command(flatten)]
        folder: IdFolder,
    },
}

#[derive(Args)]
struct IdFolder {
    /// Use this folder of digital IDs instead of LimarePDF's own
    #[arg(long, value_name = "FOLDER")]
    id_folder: Option<PathBuf>,
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
        /// A JSON object of values, as `limarepdf-cli form list --json` prints them
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
                shown(s)
            }
        };
        out!(
            "#{:<3} page {:<4} {:<10} {:<16} {:<16} {:?}{}",
            i + 1,
            a.page + 1,
            match sign_label(&a.name) {
                Some(l) if a.kind == AnnotKind::Stamp => l.to_lowercase(),
                _ => a.kind.name().to_string(),
            },
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
                        Adobe software, so LimarePDF can't list or fill them";

fn form_fill(io: &Io, set: &[String], values: &Option<PathBuf>, flatten: bool) -> Result<()> {
    let (mut pdf, original) = open_io(io)?;
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
    save_edited(io, &mut pdf, &original, false)?;
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

/// Text from the file as one line of a listing: escape codes, bidi controls and line breaks
/// can't reach the terminal ([`display_line`]), and nothing is cut. JSON output stays exact.
fn shown(s: &str) -> String {
    display_line(s, usize::MAX)
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
            shown(&f.name),
            kind_name(&f.kind),
            value,
            notes.join(", ")
        );
    }
    Ok(())
}

fn open(p: &Path, pw: &Option<String>) -> Result<Pdf> {
    Pdf::open(p, pw.as_deref()).map(|pdf| note_repaired(p, pdf))
}

/// Says on stderr when `pdf`, read from `p`, was damaged and repaired to read it, or when
/// other PDF apps may show it differently.
fn note_repaired(p: &Path, pdf: Pdf) -> Pdf {
    let name = || display_line(&p.display().to_string(), 300);
    if pdf.repaired() {
        eprintln!(
            "note: {} is damaged; LimarePDF repaired it to read it, and some content may be \
             missing",
            name()
        );
    } else if pdf.differs().is_some() {
        eprintln!(
            "note: {} isn't put together quite as the PDF format asks, so other PDF apps may \
             show it differently from LimarePDF",
            name()
        );
    }
    pdf
}

fn run_edit(io: &Io, f: impl FnOnce(&mut Pdf) -> Result<()>) -> Result<()> {
    let (mut pdf, original) = open_io(io)?;
    f(&mut pdf)?;
    save_edited(io, &mut pdf, &original, false)?;
    out!("wrote {}", io.output.display());
    Ok(())
}

/// Opens `io.input`, with its bytes for [`save_edited`].
fn open_io(io: &Io) -> Result<(Pdf, Vec<u8>)> {
    not_an_input(&[&io.input], &io.output)?;
    let bytes =
        std::fs::read(&io.input).with_context(|| format!("reading {}", io.input.display()))?;
    let pdf = Pdf::from_bytes(&bytes, io.password.as_deref())?;
    Ok((note_repaired(&io.input, pdf), bytes))
}

/// Writes `pdf`, edited from `original` (the input's bytes), to `io.output`. A digitally
/// signed input keeps its signatures: the edits are added after what was signed (see
/// docs/signatures.md). That is refused when the signatures don't allow the edits, unless
/// `--accept-disallowed`, and when the edits can't be added that way, unless
/// `--break-signatures` (the whole file is written, and the signatures break). `removes`
/// marks edits that remove information for good, which the signed version would still
/// hold: those always need the whole file written.
fn save_edited(io: &Io, pdf: &mut Pdf, original: &[u8], removes: bool) -> Result<()> {
    if Pdf::from_bytes(original, io.password.as_deref())?.signature_count() == 0 {
        return saved(pdf.save(&io.output));
    }
    let input = display_line(&io.input.display().to_string(), 300);
    let appended = if removes {
        Err(anyhow!(
            "these changes remove information for good, and the version that was signed would \
             still hold it"
        ))
    } else {
        limarepdf_core::append_edits(original, pdf)
    };
    match appended {
        Ok(a) => {
            if let Some(d) = &a.disallowed {
                let mut list = String::new();
                for (who, what) in &d.signers {
                    let what: Vec<String> = what.iter().map(|w| display_line(w, 160)).collect();
                    list.push_str(&format!(
                        "\n  - {}: {}",
                        display_line(who, 80),
                        what.join("; ")
                    ));
                }
                if !io.accept_disallowed {
                    bail!(
                        "{input} is digitally signed, and its signatures don't allow these \
                         changes; anyone who checks them would be told:{list}\nNothing was \
                         written. Add --accept-disallowed to save anyway (the signatures stay \
                         valid for what was signed, and report the changes)."
                    );
                }
                eprintln!("warning: the signatures will report changes they don't allow:{list}");
            }
            saved(limarepdf_core::write_file(&io.output, &a.bytes))?;
            if a.bytes.len() > original.len() {
                eprintln!(
                    "note: the changes were added after what was signed, so the signatures stay"
                );
            }
        }
        Err(e) => {
            if !io.break_signatures {
                bail!(
                    "{input} is digitally signed, and LimarePDF can't add these changes after what was \
                     signed: {}. Nothing was written. Add --break-signatures to write the whole \
                     file anyway, which breaks the signatures.",
                    display_line(&format!("{e:#}"), 300)
                );
            }
            saved(pdf.save(&io.output))?;
            eprintln!(
                "warning: the whole file was written, so the signatures in {} are broken",
                display_line(&io.output.display().to_string(), 300)
            );
        }
    }
    Ok(())
}

/// Areas grouped by page, in page order.
fn by_page(marks: &[(usize, [f32; 4])]) -> std::collections::BTreeMap<usize, Vec<[f32; 4]>> {
    let mut out: std::collections::BTreeMap<usize, Vec<[f32; 4]>> = Default::default();
    for &(p, b) in marks {
        out.entry(p).or_default().push(b);
    }
    out
}

/// `limarepdf-cli redact`: the file is written only when the check afterwards finds nothing left
/// under the marks.
fn redact(
    io: &Io,
    areas: &[String],
    texts: &[String],
    mark_only: bool,
    apply: bool,
    json: bool,
) -> Result<()> {
    let (mut pdf, original) = open_io(io)?;
    let mut marks = Vec::new();
    for a in areas {
        let bad = || format!("--area {a:?} should be PAGE:X,Y,W,H");
        let (page, rect) = a.split_once(':').with_context(bad)?;
        let page: usize = page.trim().parse().with_context(bad)?;
        let v: Vec<f32> = rect
            .split(',')
            .map(|x| x.trim().parse())
            .collect::<Result<_, _>>()
            .with_context(bad)?;
        if page == 0 || v.len() != 4 || v[2] <= 0.0 || v[3] <= 0.0 {
            bail!("{}", bad());
        }
        marks.push((page - 1, [v[0], v[1], v[0] + v[2], v[1] + v[3]]));
    }
    for t in texts {
        let found = pdf.find_redaction_marks(t)?;
        if found.is_empty() {
            eprintln!("not found: {t:?}");
        }
        marks.extend(found);
    }
    if mark_only {
        if marks.is_empty() {
            bail!("nothing to mark; the file was not written");
        }
        for (p, boxes) in by_page(&marks) {
            pdf.add_redaction_mark(p, &boxes, "")?;
            out!("page {}: marked {} areas", p + 1, boxes.len());
        }
        save_edited(io, &mut pdf, &original, false)?;
        out!("nothing was removed yet; apply the marks with --apply");
        out!("wrote {}", io.output.display());
        return Ok(());
    }
    let rep = if apply {
        // The new areas become marks too, and applying takes every mark away.
        for (p, boxes) in by_page(&marks) {
            pdf.add_redaction_mark(p, &boxes, "")?;
        }
        pdf.apply_redactions()?
    } else if marks.is_empty() {
        bail!("nothing to redact; the file was not written");
    } else {
        pdf.redact(&marks)?
    };
    if !rep.left.is_empty() {
        for l in &rep.left {
            eprintln!("{l}");
        }
        bail!("the check after redacting failed, so the file was not written");
    }
    // What was redacted must not stay in an earlier version of the file.
    save_edited(io, &mut pdf, &original, true)?;
    if json {
        let pages: Vec<serde_json::Value> = rep
            .pages
            .iter()
            .map(|p| {
                serde_json::json!({
                    "page": p.page + 1,
                    "characters": p.chars,
                    "text": p.text,
                    "pictures_removed": p.images,
                    "pictures_blacked_out": p.images_cut,
                    "drawings": p.paths,
                    "clipped": p.clipped,
                    "annotations": p.annots,
                })
            })
            .collect();
        let elsewhere: Vec<serde_json::Value> = rep
            .elsewhere
            .iter()
            .map(|(t, places)| serde_json::json!({ "text": t, "found": places }))
            .collect();
        out!(
            "{}",
            serde_json::to_string_pretty(
                &serde_json::json!({ "pages": pages, "found_elsewhere": elsewhere })
            )?
        );
        return Ok(());
    }
    for p in &rep.pages {
        let mut parts = Vec::new();
        for (n, what) in [
            (p.chars, "characters"),
            (p.images, "unreadable pictures (whole)"),
            (p.paths, "parts of drawings"),
            (p.annots, "comments, links or fields"),
        ] {
            if n > 0 {
                parts.push(format!("{n} {what}"));
            }
        }
        if p.images_cut > 0 {
            parts.push(format!("{} pictures blacked out", p.images_cut));
        }
        if p.clipped > 0 {
            parts.push(format!("{} drawings cut around the area", p.clipped));
        }
        let what = if parts.is_empty() {
            "nothing under the areas".to_string()
        } else {
            parts.join(", ")
        };
        out!("page {}: removed {what}", p.page + 1);
    }
    out!("checked: nothing readable is left under the areas");
    for (t, places) in &rep.elsewhere {
        out!(
            "warning: removed text {:?} is still in the file: {}",
            limarepdf_core::display_line(t, 60),
            places.join(", ")
        );
    }
    out!("wrote {}", io.output.display());
    Ok(())
}

/// A bookmark number from `limarepdf-cli bookmarks list` as a position in the outline.
fn bookmark_index(n: usize) -> Result<usize> {
    n.checked_sub(1).context("bookmarks are numbered from 1")
}

fn some(s: &Option<String>) -> String {
    s.clone().unwrap_or_default()
}

/// How `limarepdf-cli verify` writes times: 2026-10-16 14:23:05 -04:00.
const VERIFY_TIME: &str = "%Y-%m-%d %H:%M:%S %:z";

/// `limarepdf-cli verify` as text. Everything taken from the file is cleaned for display.
fn print_verify(input: &Path, rep: &SignatureReport) -> Result<()> {
    let clean = |s: &str| limarepdf_core::display_line(s, 80);
    let n = rep.signatures.len();
    match n {
        0 => out!("{}: no digital signatures", input.display()),
        1 => out!("{}: 1 digital signature", input.display()),
        _ => out!("{}: {n} digital signatures", input.display()),
    }
    macro_rules! row {
        ($label:expr, $value:expr $(,)?) => {
            out!("   {:<12} {}", $label, $value)
        };
    }
    for (i, s) in rep.signatures.iter().enumerate() {
        out!("");
        let fields: Vec<String> = s
            .fields
            .iter()
            .map(|f| format!("\"{}\"", clean(f)))
            .collect();
        let kind = match s.kind {
            SigKind::Approval => "Signature",
            SigKind::Certification(_) => "Certification",
            SigKind::DocumentTimestamp => "Document timestamp",
        };
        let page = s
            .page
            .map(|p| format!(" on page {}", p + 1))
            .unwrap_or_default();
        out!("{}. {kind} in field {}{page}", i + 1, fields.join(", "));
        let who = s.signer.as_ref().map(|c| clean(c.display_name()));
        let who = who.as_deref().unwrap_or("the signer");
        let stamp = s.kind == SigKind::DocumentTimestamp;
        let unconfirmed = if s.trust.as_ref().is_some_and(Trust::is_trusted) {
            ""
        } else {
            "; LimarePDF can't confirm who that is"
        };
        row!(
            "Status:",
            match &s.status {
                SigStatus::Valid if !s.changes_allowed() && stamp => {
                    "CHANGED: changed after the timestamp in ways that aren't allowed".into()
                }
                SigStatus::Valid if !s.changes_allowed() => {
                    format!(
                        "CHANGED: changed after {who} signed it in ways the signer didn't allow"
                    )
                }
                SigStatus::Valid if stamp => {
                    "valid: the document existed like this at the time shown".into()
                }
                SigStatus::Valid if !s.changes.is_empty() => {
                    format!("valid: only allowed changes since {who} signed it{unconfirmed}")
                }
                SigStatus::Valid => {
                    format!("valid: not changed since {who} signed it{unconfirmed}")
                }
                SigStatus::Broken(why) => format!("BROKEN: {}", display_line(why, 300)),
                SigStatus::CantCheck(why) => format!("can't check: {}", display_line(why, 300)),
            },
        );
        if let Some(t) = &s.trust {
            let mut line = trust_text(t, &clean);
            if matches!(t, Trust::Unknown(_)) && !stamp {
                let file = input.display();
                let n = i + 1;
                let self_made = s.signer.as_ref().is_some_and(|c| c.self_issued);
                line.push_str(&if self_made {
                    format!(
                        "; to trust this signer, compare the SHA-256 fingerprint below with \
                         them, then run: limarepdf-cli trust add \"{file}\" --signature {n}"
                    )
                } else {
                    format!(
                        "; to trust the authority that issued it, run: limarepdf-cli trust add \"{file}\" \
                         --signature {n} --authority (and compare its fingerprint with the \
                         authority's), or this signer alone: limarepdf-cli trust add \"{file}\" \
                         --signature {n}"
                    )
                });
            }
            row!("Trust:", line);
        }
        if !s.changes.is_empty() {
            let signed = s
                .signed_len
                .map(|l| format!(" ({l} of {} bytes signed)", rep.file_len))
                .unwrap_or_default();
            row!("Changes:", format!("after signing{signed}:"));
            for c in &s.changes {
                let line = match &c.refused {
                    None => format!("- allowed: {}", clean(&c.to_string())),
                    Some(r) => format!("- NOT ALLOWED: {} ({r})", clean(&c.to_string())),
                };
                row!("", line);
            }
        }
        if let SigKind::Certification(p) = s.kind {
            row!("Certified:", p.to_string());
        }
        if let Some(c) = &s.signer {
            let mut who = clean(c.display_name());
            if let Some(e) = c
                .email
                .as_deref()
                .filter(|e| Some(*e) != c.common_name.as_deref())
            {
                who.push_str(&format!(" <{}>", clean(e)));
            }
            if let Some(o) = c
                .organisation
                .as_deref()
                .filter(|o| Some(*o) != c.common_name.as_deref())
            {
                who.push_str(&format!(", {}", clean(o)));
            }
            let label = if s.kind == SigKind::DocumentTimestamp {
                "Authority:"
            } else {
                "Signed by:"
            };
            row!(label, who);
            let issuer = if c.self_issued {
                "itself (a self-made digital ID)".into()
            } else {
                clean(c.issuer_common_name.as_deref().unwrap_or(&c.issuer))
            };
            row!("Issued by:", issuer);
        }
        if let Some(t) = &s.time {
            let from = match t.source {
                TimeSource::Timestamp => "from a timestamp authority",
                TimeSource::SignerClock | TimeSource::Dictionary => "from the signer's computer",
            };
            let label = if s.kind == SigKind::DocumentTimestamp {
                "Time:"
            } else {
                "Signed at:"
            };
            row!(label, format!("{} ({from})", t.time.format(VERIFY_TIME)));
        }
        if let Some(ts) = s
            .timestamp
            .as_ref()
            .filter(|_| s.kind != SigKind::DocumentTimestamp)
        {
            let trusted = match &ts.trust {
                Some(t) if t.is_trusted() => " (trusted)".to_string(),
                Some(Trust::Failed(why)) => format!(" (NOT TRUSTED: {})", clean(why)),
                _ => " (LimarePDF can't confirm this authority)".to_string(),
            };
            row!(
                "Timestamp:",
                format!(
                    "{} by {}{trusted}",
                    ts.time.format(VERIFY_TIME),
                    clean(ts.authority.display_name())
                ),
            );
        }
        for (label, v) in [
            ("Name:", &s.name),
            ("Reason:", &s.reason),
            ("Location:", &s.location),
            ("Contact:", &s.contact),
        ] {
            if let Some(v) = v.as_deref().filter(|v| !v.trim().is_empty()) {
                row!(label, clean(v));
            }
        }
        let mut alg: Vec<String> = Vec::new();
        if let Some(c) = &s.signer {
            alg.push(c.key.clone());
        }
        alg.extend(s.algorithm.map(String::from));
        alg.extend(s.digest.map(String::from));
        if !s.sub_filter.is_empty() {
            alg.push(clean(&s.sub_filter));
        }
        if !alg.is_empty() {
            row!("Algorithm:", alg.join(", "));
        }
        if let Some(l) = &s.locks {
            row!("Locked:", lock_text(l, &clean));
        }
        if let Some(c) = &s.signer {
            row!(
                "Certificate:",
                format!(
                    "serial {}, valid {} to {}",
                    c.serial,
                    c.not_before.format("%Y-%m-%d"),
                    c.not_after.format("%Y-%m-%d")
                ),
            );
            row!("SHA-256:", c.sha256.clone());
        }
        for w in &s.warnings {
            row!("Warning:", display_line(&w.to_string(), 300));
        }
    }
    for f in &rep.empty_fields {
        out!("");
        let page = f
            .page
            .map(|p| format!(" on page {}", p + 1))
            .unwrap_or_default();
        out!("Empty signature field \"{}\"{page}", clean(&f.name));
        if let Some(l) = &f.lock {
            out!("   {:<12} {}", "Locks:", lock_text(l, &clean));
        }
    }
    Ok(())
}

/// Whether a signer is trusted, and why, in one line.
fn trust_text(t: &Trust, clean: &dyn Fn(&str) -> String) -> String {
    match t {
        Trust::Signer { label } => format!("trusted: you trust \"{}\" in LimarePDF", clean(label)),
        Trust::Authority { label, chain } => {
            let path: Vec<String> = chain.iter().map(|c| clean(c.display_name())).collect();
            format!(
                "trusted: issued under \"{}\", a certificate authority you trust in LimarePDF ({})",
                clean(label),
                path.join(" < ")
            )
        }
        Trust::Unknown(why) => format!("not confirmed: {}", clean(why)),
        Trust::Failed(why) => format!("NOT TRUSTED: {}", clean(why)),
    }
}

fn trust_json(t: &Trust, cert: &dyn Fn(&CertInfo) -> serde_json::Value) -> serde_json::Value {
    use serde_json::json;
    match t {
        Trust::Signer { label } => json!({"status": "trusted", "as": "signer", "label": label}),
        Trust::Authority { label, chain } => json!({
            "status": "trusted",
            "as": "authority",
            "label": label,
            "chain": chain.iter().map(cert).collect::<Vec<_>>(),
        }),
        Trust::Unknown(why) => json!({"status": "unknown", "reason": why}),
        Trust::Failed(why) => json!({"status": "failed", "reason": why}),
    }
}

const TRUST_NOTE: &str = "Trust settings affect LimarePDF only.";

fn run_trust(cmd: TrustCmd) -> Result<()> {
    let clean = |s: &str| limarepdf_core::display_line(s, 80);
    match cmd {
        TrustCmd::List { json, trust_file } => {
            let path = trust_path(&trust_file)?;
            let list = TrustList::load(&path)?;
            if json {
                let entries: Vec<serde_json::Value> = list
                    .entries
                    .iter()
                    .map(|e| {
                        serde_json::json!({
                            "as": match e.role {
                                TrustRole::Signer => "signer",
                                TrustRole::Authority => "authority",
                            },
                            "label": e.label,
                            "added": e.added.map(|d| d.to_string()),
                            "subject": e.cert.subject,
                            "issuer": e.cert.issuer,
                            "self_issued": e.cert.self_issued,
                            "not_before": e.cert.not_before.to_rfc3339(),
                            "not_after": e.cert.not_after.to_rfc3339(),
                            "sha256": e.cert.sha256,
                        })
                    })
                    .collect();
                let v = serde_json::json!({
                    "file": path.display().to_string(),
                    "trusted": entries,
                    "problems": list.problems,
                });
                out!("{}", serde_json::to_string_pretty(&v)?);
                return Ok(());
            }
            if list.entries.is_empty() {
                out!(
                    "LimarePDF trusts no certificates yet ({}). Add one with `limarepdf-cli trust add`. {TRUST_NOTE}",
                    path.display()
                );
            } else {
                out!("Trusted certificates in {}. {TRUST_NOTE}", path.display());
            }
            for (i, e) in list.entries.iter().enumerate() {
                out!("");
                let added = e.added.map(|d| format!(", added {d}")).unwrap_or_default();
                out!(
                    "{}. \"{}\": trusted as {}{added}",
                    i + 1,
                    clean(&e.label),
                    e.role
                );
                out!("   {:<12} {}", "Certificate:", clean(&e.cert.subject));
                let issuer = if e.cert.self_issued {
                    "itself (a self-made digital ID)".into()
                } else {
                    clean(&e.cert.issuer)
                };
                out!("   {:<12} {issuer}", "Issued by:");
                out!(
                    "   {:<12} {} to {}",
                    "Valid:",
                    e.cert.not_before.format("%Y-%m-%d"),
                    e.cert.not_after.format("%Y-%m-%d")
                );
                out!("   {:<12} {}", "SHA-256:", e.cert.sha256);
            }
            for p in &list.problems {
                out!("");
                out!("Problem: {}", limarepdf_core::display_line(p, 200));
            }
        }
        TrustCmd::Add {
            input,
            authority,
            signature,
            pick,
            label,
            password,
            trust_file,
        } => {
            let path = trust_path(&trust_file)?;
            let mut list = TrustList::load(&path)?;
            let bytes =
                std::fs::read(&input).with_context(|| format!("reading {}", input.display()))?;
            let role = if authority {
                TrustRole::Authority
            } else {
                TrustRole::Signer
            };
            let is_pdf = bytes.windows(5).take(1024).any(|w| w == b"%PDF-");
            // One certificate out of several, chosen with --pick.
            let choose = |mut certs: Vec<CertInfo>, what: &str| -> Result<CertInfo> {
                match (certs.len(), pick) {
                    (0, _) => bail!("there is no {what}"),
                    (1, None | Some(1)) => Ok(certs.remove(0)),
                    (n, Some(k)) if (1..=n).contains(&k) => Ok(certs.remove(k - 1)),
                    (n, Some(k)) => bail!("there is no {what} {k}; there are {n}"),
                    (n, None) => {
                        let names: Vec<String> = certs
                            .iter()
                            .enumerate()
                            .map(|(i, c)| format!("  {}. {}", i + 1, clean(&c.subject)))
                            .collect();
                        bail!(
                            "there are {n} to choose from; choose one with --pick N:\n{}",
                            names.join("\n")
                        )
                    }
                }
            };
            let cert: CertInfo = if is_pdf {
                let rep = limarepdf_core::verify_signatures(&bytes, password.as_deref(), &list)?;
                let n = match (signature, rep.signatures.len()) {
                    (_, 0) => bail!("{} has no digital signatures", input.display()),
                    (Some(n), _) => n,
                    (None, 1) => 1,
                    (None, many) => bail!(
                        "{} has {many} signatures; choose one with --signature N, numbered as \
                         `limarepdf-cli verify` lists them",
                        input.display()
                    ),
                };
                let s = n
                    .checked_sub(1)
                    .and_then(|i| rep.signatures.get(i))
                    .with_context(|| {
                        format!(
                            "there is no signature {n}; the file has {}",
                            rep.signatures.len()
                        )
                    })?;
                match &s.status {
                    SigStatus::Valid => {}
                    SigStatus::Broken(why) | SigStatus::CantCheck(why) => bail!(
                        "signature {n} isn't valid ({}); LimarePDF only trusts the signer of a valid \
                         signature",
                        display_line(why, 300)
                    ),
                }
                if authority {
                    // An authority from the certificates the signature carries, as in
                    // Acrobat's certificate viewer.
                    let cas = s
                        .certificates
                        .iter()
                        .filter(|c| limarepdf_core::is_authority(c))
                        .cloned()
                        .collect();
                    choose(cas, "certificate authority in that signature")?
                } else {
                    if pick.is_some() {
                        bail!("--pick chooses an authority (with --authority) or a certificate in a file");
                    }
                    s.signer
                        .clone()
                        .context("the signature has no signer certificate")?
                }
            } else {
                if signature.is_some() {
                    bail!(
                        "--signature is for PDF files; for a certificate file, choose with --pick"
                    );
                }
                let certs = limarepdf_core::read_certificates(&bytes)
                    .with_context(|| format!("reading {}", input.display()))?;
                choose(certs, "certificate in the file")?
            };
            let added = list.add(&cert.der, role, label.as_deref())?;
            saved(list.save(&path))?;
            let e = list
                .entries
                .iter()
                .find(|e| e.cert.sha256 == cert.sha256)
                .context("the certificate wasn't added")?;
            let was = match added {
                limarepdf_core::Added::RoleChanged(old) => format!(" (instead of as {old})"),
                limarepdf_core::Added::New => String::new(),
            };
            out!("Now trusting \"{}\" as {role}{was}.", clean(&e.label));
            out!("   {:<12} {}", "Certificate:", clean(&cert.subject));
            out!("   {:<12} {}", "SHA-256:", cert.sha256);
            if role == TrustRole::Authority {
                out!(
                    "Signers whose certificates it issued, directly or through authorities it \
                     allowed to, are now trusted."
                );
            }
            out!(
                "Make sure this fingerprint is right by checking it with the certificate's owner \
                 in a way you trust (in person, by phone): anyone can make a certificate with any \
                 name."
            );
            if !cert.valid_now() {
                out!(
                    "Note: the certificate isn't valid today (valid {} to {}); only signatures \
                     made while it was valid are trusted.",
                    cert.not_before.format("%Y-%m-%d"),
                    cert.not_after.format("%Y-%m-%d")
                );
            }
            out!("{TRUST_NOTE} Saved in {}.", path.display());
        }
        TrustCmd::Remove { which, trust_file } => {
            let path = trust_path(&trust_file)?;
            let mut list = TrustList::load(&path)?;
            let index = match which.trim().parse::<usize>() {
                Ok(n) if (1..=list.entries.len()).contains(&n) => n - 1,
                Ok(n) => bail!(
                    "there is no entry {n}; the list has {} (see `limarepdf-cli trust list`)",
                    list.entries.len()
                ),
                Err(_) => list.find(&which).with_context(|| {
                    format!("no trusted certificate has the fingerprint {which}")
                })?,
            };
            let e = list.remove(index).context("no such entry")?;
            saved(list.save(&path))?;
            out!(
                "No longer trusting \"{}\" (SHA-256 {}). {TRUST_NOTE}",
                clean(&e.label),
                e.cert.sha256
            );
        }
    }
    Ok(())
}

/// The trust list to use: `explicit`, else LimarePDF's own in the settings folder.
fn id_store(folder: &IdFolder) -> Result<IdStore> {
    folder
        .id_folder
        .clone()
        .or_else(IdStore::default_dir)
        .map(IdStore::new)
        .context(
            "LimarePDF can't find its settings folder (HOME or APPDATA isn't set); use --id-folder",
        )
}

/// A password: the first line of `file`, or asked for on the terminal without showing it.
fn read_password(file: &Option<PathBuf>, prompt: &str) -> Result<Zeroizing<String>> {
    if let Some(f) = file {
        let size = std::fs::metadata(f)
            .with_context(|| format!("reading {}", f.display()))?
            .len();
        if size > 4096 {
            bail!("{} is too large to be a password file", f.display());
        }
        let text = Zeroizing::new(
            std::fs::read_to_string(f).with_context(|| format!("reading {}", f.display()))?,
        );
        let line = text.split('\n').next().unwrap_or_default();
        return Ok(Zeroizing::new(
            line.strip_suffix('\r').unwrap_or(line).to_string(),
        ));
    }
    rpassword::prompt_password(prompt)
        .map(Zeroizing::new)
        .map_err(|e| anyhow::anyhow!("can't ask for the password ({e}); use --password-file"))
}

/// A new password: from `file`, or asked for twice.
fn choose_password(file: &Option<PathBuf>, prompt: &str) -> Result<Zeroizing<String>> {
    let p = read_password(file, prompt)?;
    if let Some(why) = limarepdf_core::password_problem(&p) {
        bail!("{why}");
    }
    if file.is_none() && *read_password(&None, "Type it again: ")? != *p {
        bail!("the two passwords aren't the same; nothing was done");
    }
    Ok(p)
}

/// The facts about an ID's certificate, indented under its heading.
fn id_details(id: &DigitalId, clean: &dyn Fn(&str) -> String) -> Result<()> {
    let c = &id.cert;
    out!("   {:<12} {}", "Certificate:", clean(&c.subject));
    let issuer = if c.self_issued {
        "itself (a self-made digital ID)".into()
    } else {
        clean(&c.issuer)
    };
    out!("   {:<12} {issuer}", "Issued by:");
    if !id.chain.is_empty() {
        let n = id.chain.len();
        out!(
            "   {:<12} {n} more certificate{} (its authorities)",
            "With:",
            if n == 1 { "" } else { "s" }
        );
    }
    out!("   {:<12} {}", "Key:", c.key);
    let note = if c.valid_now() {
        ""
    } else if c.expired() {
        " (expired: LimarePDF won't sign with it)"
    } else {
        " (not valid yet: LimarePDF won't sign with it)"
    };
    out!(
        "   {:<12} {} to {}{note}",
        "Valid:",
        c.not_before.format("%Y-%m-%d"),
        c.not_after.format("%Y-%m-%d")
    );
    out!("   {:<12} {}", "SHA-256:", c.sha256);
    Ok(())
}

fn run_id(cmd: IdCmd) -> Result<()> {
    let clean = |s: &str| limarepdf_core::display_line(s, 80);
    match cmd {
        IdCmd::List { json, folder } => {
            let store = id_store(&folder)?;
            let list = store.list();
            if json {
                let cert = |c: &CertInfo| {
                    serde_json::json!({
                        "subject": c.subject,
                        "issuer": c.issuer,
                        "self_issued": c.self_issued,
                        "key": c.key,
                        "not_before": c.not_before.to_rfc3339(),
                        "not_after": c.not_after.to_rfc3339(),
                        "sha256": c.sha256,
                    })
                };
                let ids: Vec<serde_json::Value> = list
                    .ids
                    .iter()
                    .map(|i| {
                        let mut v = cert(&i.cert);
                        v["name"] = i.name.clone().into();
                        v["label"] = i.cert.display_name().into();
                        v["default"] = i.default.into();
                        v["valid_now"] = i.valid_now().into();
                        v["file"] = i.path.display().to_string().into();
                        v["chain"] = i.chain.iter().map(cert).collect();
                        v
                    })
                    .collect();
                let v = serde_json::json!({
                    "folder": store.dir().display().to_string(),
                    "ids": ids,
                    "problems": list.problems,
                });
                out!("{}", serde_json::to_string_pretty(&v)?);
                return Ok(());
            }
            if list.ids.is_empty() {
                out!(
                    "You have no digital IDs yet ({}). Make one with `limarepdf-cli id create --name \
                     \"Your Name\"`, or import one with `limarepdf-cli id import FILE.p12`.",
                    store.dir().display()
                );
            } else {
                out!("Your digital IDs, in {}:", store.dir().display());
            }
            for (i, id) in list.ids.iter().enumerate() {
                out!("");
                let default = if id.default { ", the default" } else { "" };
                out!(
                    "{}. \"{}\" ({}){default}",
                    i + 1,
                    clean(id.cert.display_name()),
                    id.name
                );
                id_details(id, &clean)?;
            }
            for p in &list.problems {
                out!("");
                out!("Problem: {}", limarepdf_core::display_line(p, 200));
            }
        }
        IdCmd::Create {
            name,
            organisation,
            email,
            password_file,
            folder,
        } => {
            let store = id_store(&folder)?;
            let new = NewId {
                name,
                organisation,
                email,
            };
            new.check()?;
            let password = choose_password(
                &password_file,
                &format!(
                    "Password for the new ID (at least {} characters): ",
                    limarepdf_core::MIN_PASSWORD
                ),
            )?;
            let id = saved(store.create(&new, &password))?;
            out!(
                "Made the digital ID \"{}\" ({}), in {}. It is the default for signing.",
                clean(id.cert.display_name()),
                id.name,
                id.path.display()
            );
            id_details(&id, &clean)?;
            out!("");
            out!(
                "Keep the password safe: LimarePDF can't recover it, and without it the ID can't \
                 sign. Keep a copy of the ID too (`limarepdf-cli id export {} -o {}.p12`): if this \
                 computer is lost, so is the ID.",
                id.name,
                id.name
            );
            out!(
                "For others to trust your signatures, send them your certificate (`limarepdf-cli id \
                 export {} -o {}.cer`) and tell them its SHA-256 fingerprint another way, such \
                 as by phone, so they can check it is yours.",
                id.name,
                id.name
            );
        }
        IdCmd::Import {
            input,
            name,
            password_file,
            folder,
        } => {
            let store = id_store(&folder)?;
            let bytes =
                std::fs::read(&input).with_context(|| format!("reading {}", input.display()))?;
            let file_name = input
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            let password = read_password(&password_file, &format!("Password of {file_name}: "))?;
            let id = saved(store.import(&bytes, &password, name.as_deref()))?;
            out!(
                "Imported the digital ID \"{}\" ({}), in {}. It is the default for signing.",
                clean(id.cert.display_name()),
                id.name,
                id.path.display()
            );
            id_details(&id, &clean)?;
            out!("");
            out!(
                "It keeps its password, now with LimarePDF's own encryption. {} still holds the ID: \
                 keep it safe, or delete it if LimarePDF's copy is enough.",
                input.display()
            );
        }
        IdCmd::Export {
            which,
            output,
            password_file,
            new_password,
            new_password_file,
            folder,
        } => {
            let store = id_store(&folder)?;
            let id = store.find(&which)?;
            let label = clean(id.cert.display_name());
            let ext = output
                .extension()
                .map(|e| e.to_string_lossy().to_lowercase())
                .unwrap_or_default();
            match ext.as_str() {
                "p12" | "pfx" => {
                    let password =
                        read_password(&password_file, &format!("Password of \"{label}\": "))?;
                    let unlocked = store.unlock(&id, &password)?;
                    let fresh = new_password || new_password_file.is_some();
                    let file_password = if fresh {
                        choose_password(
                            &new_password_file,
                            &format!(
                                "Password for the exported file (at least {} characters): ",
                                limarepdf_core::MIN_PASSWORD
                            ),
                        )?
                    } else {
                        if limarepdf_core::password_problem(&password).is_some() {
                            bail!(
                                "the ID's password is shorter than {} characters; choose a new \
                                 one for the exported file with --new-password",
                                limarepdf_core::MIN_PASSWORD
                            );
                        }
                        password
                    };
                    let bytes = unlocked.to_p12(&file_password)?;
                    saved(limarepdf_core::write_private(&output, &bytes))?;
                    out!(
                        "Exported the digital ID \"{label}\" with its key to {}, protected by {}.",
                        output.display(),
                        if fresh {
                            "the new password"
                        } else {
                            "the ID's password"
                        }
                    );
                    out!("Keep it safe: anyone with this file and its password can sign as you.");
                }
                "cer" | "crt" | "der" => {
                    saved(
                        limarepdf_core::write_file(&output, &id.cert.der)
                            .with_context(|| format!("writing {}", output.display())),
                    )?;
                    out!(
                        "Exported the certificate of \"{label}\" to {}. It holds no key, so it \
                         is safe to share.",
                        output.display()
                    );
                    out!("   {:<12} {}", "SHA-256:", id.cert.sha256);
                    out!(
                        "Recipients trust it in LimarePDF with `limarepdf-cli trust add {}`, or in their PDF \
                         app's trusted certificates, after checking the fingerprint with you \
                         another way, such as by phone.",
                        output.display()
                    );
                }
                _ => bail!(
                    "name the exported file .p12 or .pfx for the ID with its key (for other \
                     apps, or a backup), or .cer for its certificate only (for others to trust)"
                ),
            }
        }
        IdCmd::Remove { which, yes, folder } => {
            let store = id_store(&folder)?;
            let id = store.find(&which)?;
            let label = clean(id.cert.display_name());
            if !yes {
                use std::io::{BufRead, IsTerminal, Write};
                if !std::io::stdin().is_terminal() {
                    bail!(
                        "removing a digital ID can't be undone; add --yes to remove \"{label}\" \
                         ({}) without being asked",
                        id.name
                    );
                }
                eprint!(
                    "Remove the digital ID \"{label}\" ({})? It can't sign again unless you have \
                     an exported copy. Type yes to remove it: ",
                    id.name
                );
                let _ = std::io::stderr().flush();
                let mut answer = String::new();
                std::io::stdin().lock().read_line(&mut answer)?;
                if !answer.trim().eq_ignore_ascii_case("yes") {
                    out!("Nothing was removed.");
                    return Ok(());
                }
            }
            saved(store.remove(&id))?;
            out!("Removed the digital ID \"{label}\" ({}).", id.name);
        }
    }
    Ok(())
}

fn trust_path(explicit: &Option<PathBuf>) -> Result<PathBuf> {
    explicit.clone().or_else(TrustList::default_path).context(
        "LimarePDF can't find its settings folder (HOME or APPDATA isn't set); use --trust-file",
    )
}

/// The trust list for checking: an explicit file must load; LimarePDF's own may be missing or
/// unusable, which leaves every signer unconfirmed, with a warning.
/// Whether `a` and `b` name the same file: the same path once links are followed, or on Unix
/// the same device and inode, as a hard link is.
fn same_file(a: &Path, b: &Path) -> bool {
    if a.canonicalize()
        .is_ok_and(|a| b.canonicalize().is_ok_and(|b| a == b))
    {
        return true;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if let (Ok(a), Ok(b)) = (std::fs::metadata(a), std::fs::metadata(b)) {
            return a.dev() == b.dev() && a.ino() == b.ino();
        }
    }
    false
}

/// Refuses an `output` that is one of the `inputs`, by any path or link to it, so the input is
/// never changed.
fn not_an_input(inputs: &[impl AsRef<Path>], output: &Path) -> Result<()> {
    if let Some(input) = inputs.iter().find(|i| same_file(i.as_ref(), output)) {
        bail!(
            "{} is the input {}; the result must be saved to another file, as the input is never \
             changed. Nothing was written",
            display_line(&output.display().to_string(), 300),
            display_line(&input.as_ref().display().to_string(), 300)
        );
    }
    Ok(())
}

/// A command writing several files never replaces one silently (BUG-05): with anything at
/// one of `files` already, it is refused before anything is written, unless `overwrite`.
fn not_taken(files: &[PathBuf], overwrite: bool) -> Result<()> {
    const SHOWN: usize = 5;
    let taken = limarepdf_core::existing(files);
    if overwrite || taken.is_empty() {
        return Ok(());
    }
    let mut names: Vec<String> = taken
        .iter()
        .take(SHOWN)
        .map(|p| display_line(&p.display().to_string(), 300))
        .collect();
    if taken.len() > SHOWN {
        names.push(format!("and {} more", taken.len() - SHOWN));
    }
    let (what, it) = match taken.len() {
        1 => ("a file is".to_string(), "it"),
        n => (format!("{n} files are"), "them"),
    };
    bail!(
        "{what} already there: {}. Nothing was written; choose another --out-dir, or add \
         --overwrite to replace {it}",
        names.join(", ")
    );
}

fn trust_for_checking(explicit: &Option<PathBuf>) -> Result<TrustList> {
    let list = match (explicit, TrustList::default_path()) {
        (Some(p), _) if !p.exists() => bail!("there is no trust list at {}", p.display()),
        (Some(p), _) => TrustList::load(p)?,
        (None, Some(p)) => TrustList::load(&p).unwrap_or_else(|e| {
            eprintln!("Warning: {e:#}; no signer is trusted");
            TrustList::default()
        }),
        (None, None) => TrustList::default(),
    };
    for p in &list.problems {
        eprintln!(
            "Warning: trust list: {}",
            limarepdf_core::display_line(p, 200)
        );
    }
    Ok(list)
}

fn lock_text(l: &FieldLock, clean: &dyn Fn(&str) -> String) -> String {
    let names = || {
        l.fields
            .iter()
            .map(|f| format!("\"{}\"", clean(f)))
            .collect::<Vec<_>>()
            .join(", ")
    };
    let fields = match l.action {
        LockAction::All => "every form field".into(),
        LockAction::Include => format!("the fields {}", names()),
        LockAction::Exclude => format!("every form field except {}", names()),
    };
    match l.permissions {
        Some(p) => format!("{fields}; afterwards {p}"),
        None => fields,
    }
}

fn permissions_slug(p: Permissions) -> &'static str {
    match p {
        Permissions::NoChanges => "no_changes",
        Permissions::FormFilling => "form_filling",
        Permissions::FormFillingAndComments => "form_filling_and_comments",
    }
}

/// A stable name for each kind of change, for `limarepdf-cli verify --json`.
fn change_slug(k: &ChangeKind) -> &'static str {
    match k {
        ChangeKind::FieldFilled(_) => "field_filled",
        ChangeKind::FieldMadeReadOnly(_) => "field_made_read_only",
        ChangeKind::Signed(_) => "signed",
        ChangeKind::DocumentTimestamp(_) => "document_timestamp",
        ChangeKind::SignatureFieldAdded(_) => "signature_field_added",
        ChangeKind::CommentAdded => "comment_added",
        ChangeKind::CommentChanged => "comment_changed",
        ChangeKind::CommentRemoved => "comment_removed",
        ChangeKind::FormSettings => "form_settings",
        ChangeKind::CommentAppearances => "comment_appearances",
        ChangeKind::ValidationData => "validation_data",
        ChangeKind::UsageRightsRemoved => "usage_rights_removed",
        ChangeKind::Metadata => "metadata",
        ChangeKind::PageContent => "page_content",
        ChangeKind::PagesChanged => "pages_changed",
        ChangeKind::AnnotationChanged => "annotation_changed",
        ChangeKind::FieldAdded(_) => "field_added",
        ChangeKind::FieldRemoved(_) => "field_removed",
        ChangeKind::FieldChanged(_) => "field_changed",
        ChangeKind::SignatureChanged(_) => "signature_changed",
        ChangeKind::Actions => "actions",
        ChangeKind::DocumentChanged(_) => "document_changed",
        ChangeKind::Unrecognised(_) => "unrecognised",
        ChangeKind::Malformed(_) => "malformed",
    }
}

/// `limarepdf-cli verify --json`.
fn verify_json(rep: &SignatureReport) -> serde_json::Value {
    use serde_json::json;
    let cert = |c: &CertInfo| {
        json!({
            "subject": c.subject,
            "common_name": c.common_name,
            "organisation": c.organisation,
            "email": c.email,
            "issuer": c.issuer,
            "self_issued": c.self_issued,
            "serial": c.serial,
            "not_before": c.not_before.to_rfc3339(),
            "not_after": c.not_after.to_rfc3339(),
            "sha256": c.sha256,
            "key": c.key,
        })
    };
    let lock = |l: &FieldLock| {
        json!({
            "action": match l.action {
                LockAction::All => "all",
                LockAction::Include => "include",
                LockAction::Exclude => "exclude",
            },
            "fields": l.fields,
            "permissions": l.permissions.map(permissions_slug),
        })
    };
    let rect = |b: &[f32; 4]| -> Vec<f64> {
        b.iter()
            .map(|&x| (f64::from(x) * 1000.0).round() / 1000.0)
            .collect()
    };
    let sigs: Vec<serde_json::Value> = rep
        .signatures
        .iter()
        .map(|s| {
            let (status, problem) = match &s.status {
                SigStatus::Valid => ("valid", None),
                SigStatus::Broken(why) => ("broken", Some(why)),
                SigStatus::CantCheck(why) => ("cant_check", Some(why)),
            };
            let (kind, permissions) = match s.kind {
                SigKind::Approval => ("approval", None),
                SigKind::Certification(p) => ("certification", Some(permissions_slug(p))),
                SigKind::DocumentTimestamp => ("document_timestamp", None),
            };
            json!({
                "fields": s.fields,
                "page": s.page.map(|p| p + 1),
                "rect": s.bbox.as_ref().map(rect),
                "kind": kind,
                "permissions": permissions,
                "sub_filter": s.sub_filter,
                "status": status,
                "problem": problem,
                "byte_range": s.byte_range,
                "covers_whole_file": s.covers_whole_file,
                "signed_length": s.signed_len,
                "changes_allowed": s.changes_allowed(),
                "changes": s.changes.iter().map(|c| json!({
                    "change": c.to_string(),
                    "kind": change_slug(&c.kind),
                    "page": c.page.map(|p| p + 1),
                    "allowed": c.allowed(),
                    "refused": c.refused.map(|r| match r {
                        Refusal::Never => "never",
                        Refusal::Certification(_) => "certification",
                        Refusal::Locked => "locked",
                    }),
                })).collect::<Vec<_>>(),
                "signer": s.signer.as_ref().map(cert),
                "trust": s.trust.as_ref().map(|t| trust_json(t, &cert)),
                "certificates": s.certificates.iter().map(cert).collect::<Vec<_>>(),
                "name": s.name,
                "reason": s.reason,
                "location": s.location,
                "contact": s.contact,
                "time": s.time.map(|t| t.time.to_rfc3339()),
                "time_source": s.time.map(|t| match t.source {
                    TimeSource::Timestamp => "timestamp",
                    TimeSource::SignerClock => "signer_clock",
                    TimeSource::Dictionary => "signature_dictionary",
                }),
                "timestamp": s.timestamp.as_ref().map(|t| json!({
                    "time": t.time.to_rfc3339(),
                    "authority": cert(&t.authority),
                    "trust": t.trust.as_ref().map(|t| trust_json(t, &cert)),
                })),
                "digest": s.digest,
                "algorithm": s.algorithm,
                "locks": s.locks.as_ref().map(lock),
                "warnings": s.warnings.iter().map(|w| w.to_string()).collect::<Vec<_>>(),
            })
        })
        .collect();
    let empty: Vec<serde_json::Value> = rep
        .empty_fields
        .iter()
        .map(|f| {
            json!({
                "field": f.name,
                "page": f.page.map(|p| p + 1),
                "rect": f.bbox.as_ref().map(rect),
                "lock": f.lock.as_ref().map(lock),
            })
        })
        .collect();
    json!({
        "all_valid": rep.all_valid(),
        "all_ok": rep.all_ok(),
        "file_size": rep.file_len,
        "signatures": sigs,
        "empty_fields": empty,
    })
}

/// Run a command that a malformed file could make panic, turning a panic into a plain error, as the app's
/// `guard` does.
fn guard<T>(f: impl FnOnce() -> Result<T>) -> Result<T> {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(f)).unwrap_or_else(|p| {
        let why = p
            .downcast_ref::<&str>()
            .map(|s| s.to_string())
            .or_else(|| p.downcast_ref::<String>().cloned())
            .unwrap_or_else(|| "unknown cause".into());
        Err(anyhow!(
            "LimarePDF hit an internal error ({why}). The file may be damaged. \
             Run with RUST_BACKTRACE=1 to see where it happened."
        ))
    })
}

/// Set once the command has saved a file, so that a report it can't write afterwards says so.
static SAVED: AtomicBool = AtomicBool::new(false);

/// `r`, noting that a file was saved when it is `Ok`.
fn saved<T>(r: Result<T>) -> Result<T> {
    if r.is_ok() {
        SAVED.store(true, Ordering::Relaxed);
    }
    r
}

/// The reader of the output went away, as with `limarepdf-cli text file.pdf | head`: the
/// command stops quietly, with the status it would have had otherwise (decision D5).
#[derive(Debug)]
struct Closed;

impl std::fmt::Display for Closed {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        f.write_str("the output was closed")
    }
}

impl std::error::Error for Closed {}

/// The output couldn't be written for another reason, such as a full disk.
#[derive(Debug)]
struct Unwritten(std::io::Error);

impl std::fmt::Display for Unwritten {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        write!(f, "couldn't write the output: {}", self.0)
    }
}

impl std::error::Error for Unwritten {}

/// The error for output that couldn't be written (audit R15).
fn unwritten(e: std::io::Error) -> anyhow::Error {
    if e.kind() == std::io::ErrorKind::BrokenPipe {
        Closed.into()
    } else {
        Unwritten(e).into()
    }
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    // Only `guard`'s error is printed for a panic; with RUST_BACKTRACE set, Rust's own message and the
    // backtrace are printed too, for bug reports.
    if std::env::var_os("RUST_BACKTRACE").is_none_or(|v| v == "0") {
        std::panic::set_hook(Box::new(|_| {}));
    }
    let done = guard(|| run(cli.cmd)).and_then(|()| {
        use std::io::Write;
        std::io::stdout().flush().map_err(unwritten)
    });
    let Err(e) = done else {
        return Ok(());
    };
    if e.is::<Closed>() {
        return Ok(());
    }
    // The file is in place, so never "nothing was written" (decision D7).
    if let Some(n) = e.downcast_ref::<limarepdf_core::NotConfirmed>() {
        return Err(anyhow!(
            "{n}. If the computer lost power just now, run the command again."
        ));
    }
    // Said plainly, without what the command was doing when it wrote.
    match e.downcast_ref::<Unwritten>() {
        Some(Unwritten(why)) if SAVED.load(Ordering::Relaxed) => Err(anyhow!(
            "the file was saved, but its report couldn't be written: {why}"
        )),
        Some(u) => Err(anyhow!("{u}")),
        None => Err(e),
    }
}

fn run(cmd: Cmd) -> Result<()> {
    match cmd {
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
                    out!("{k}: {}", shown(&v));
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
            not_an_input(&inputs, &output)?;
            let docs = inputs
                .par_iter()
                .map(|p| open(p, &None))
                .collect::<Result<Vec<_>>>()?;
            let mut m = Pdf::merge(docs)?;
            saved(m.save(&output))?;
            out!("wrote {} ({} pages)", output.display(), m.page_count());
        }
        Cmd::Split {
            input,
            every,
            bookmarks,
            out_dir,
            overwrite,
            password,
        } => {
            let p = open(&input, &password)?;
            let stem = input
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("part")
                .to_string();
            let by = match bookmarks {
                Some(level) => SplitBy::Bookmarks(level),
                None => SplitBy::Every(every),
            };
            let name = |n, title: Option<&str>| out_dir.join(part_file_name(&stem, n, title));
            // Asked first, so that nothing is written when the document can't be split.
            not_taken(&p.split_files(by, name)?, overwrite)?;
            std::fs::create_dir_all(&out_dir)?;
            let n = saved(p.split(by, name))?;
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
            not_an_input(&inputs, &output)?;
            let mut p = Pdf::from_images(&read_all(&inputs)?, &size.opts())?;
            saved(p.save(&output))?;
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
                        original_kept,
                    } => {
                        let kind = match mode {
                            ScanMode::BlackWhite => "black and white",
                            ScanMode::Gray => "greyscale",
                            _ => "colour",
                        };
                        let tilt = deskewed
                            .map(|a| format!(", straightened {a:.1}°"))
                            .unwrap_or_default();
                        let kept = if original_kept {
                            " (the original is also used elsewhere in the document, so the \
                             file keeps it too)"
                        } else {
                            ""
                        };
                        out!(
                            "page {}: {kind}, {} -> {}{tilt}{kept}",
                            i + 1,
                            kb(before),
                            kb(after)
                        );
                        before_all += before;
                        after_all += after;
                    }
                    ScanResult::SameImage(first) => out!(
                        "page {}: the same image as page {}, optimised with it",
                        i + 1,
                        first + 1
                    ),
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
        Cmd::Redact {
            io,
            areas,
            texts,
            mark_only,
            apply,
            json,
        } => redact(&io, &areas, &texts, mark_only, apply, json)?,
        Cmd::Sanitise {
            input,
            output,
            password,
            keep,
            also,
            list,
            break_signatures,
        } => {
            if let Some(output) = output.as_ref().filter(|_| !list) {
                not_an_input(&[&input], output)?;
            }
            let mut pdf = open(&input, &password)?;
            let signed = pdf.signature_count() > 0;
            if !list && !break_signatures && signed {
                bail!(
                    "{} is digitally signed, and removing hidden information means writing the \
                     whole file, which breaks the signatures (the signed version would \
                     otherwise still hold what is removed). Nothing was written. Add \
                     --break-signatures to go ahead.",
                    display_line(&input.display().to_string(), 300)
                );
            }
            if list {
                let found = pdf.find_hidden()?;
                for h in limarepdf_core::Hidden::ALL {
                    let n = found.count(h);
                    let mark = if h.default_on() { "" } else { "  (--also)" };
                    out!("{:<14} {n:>5}  {}{mark}", h.name(), h.label());
                }
                return Ok(());
            }
            let kinds = |s: &str| -> Result<Vec<limarepdf_core::Hidden>> {
                s.split(',')
                    .filter(|x| !x.trim().is_empty())
                    .map(|x| {
                        limarepdf_core::Hidden::parse(x).with_context(|| format!("no kind {x:?}"))
                    })
                    .collect()
            };
            let (keep, also) = (kinds(&keep)?, kinds(&also)?);
            let what: Vec<limarepdf_core::Hidden> = limarepdf_core::Hidden::ALL
                .into_iter()
                .filter(|h| (h.default_on() && !keep.contains(h)) || also.contains(h))
                .collect();
            let rep = pdf.remove_hidden(&what).map_err(|e| {
                if e.is::<limarepdf_core::LayersDamaged>() {
                    anyhow!(
                        "Part of this document is damaged, so LimarePDF can't check all of it \
                         for hidden layers. Nothing was changed. To remove the other kinds of \
                         information, run it again with --keep hidden-layers."
                    )
                } else {
                    e
                }
            })?;
            if rep.removed.is_empty() {
                out!("nothing to remove");
            }
            for (h, n) in &rep.removed {
                out!("removed {n:>5}  {}", h.label());
            }
            if let Some(note) = rep.note() {
                out!("{note}");
            }
            let output = output.context("--output is needed")?;
            saved(pdf.save(&output))?;
            if signed {
                eprintln!(
                    "warning: the whole file was written, so the signatures in {} are broken",
                    display_line(&output.display().to_string(), 300)
                );
            }
            out!("wrote {}", output.display());
        }
        Cmd::Stamp {
            io,
            kind,
            text,
            signature,
            style,
            color,
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
            if color.is_some() && text.is_none() && signature.is_none() {
                bail!("--color is for --text stamps and signatures only");
            }
            let style = SignStyle::from_number(style).context("--style is 1, 2, 3 or 4")?;
            let image = match image {
                Some(path) => Some(
                    std::fs::read(&path).with_context(|| format!("reading {}", path.display()))?,
                ),
                None => None,
            };
            let stamp = match (&image, &text) {
                (Some(bytes), _) => Stamp::Picture(bytes),
                (None, None) if signature.is_some() => Stamp::Signature {
                    art: SignArt::Typed {
                        text: signature.as_deref().unwrap_or_default(),
                        style,
                    },
                    role: SignRole::Signature,
                    color: match color {
                        Some(c) => InkColor::from_name(&c)
                            .with_context(|| format!("a signature is black or blue, not {c:?}"))?,
                        None => InkColor::Black,
                    },
                    date,
                },
                (None, Some(text)) => Stamp::Custom {
                    text,
                    color: match color {
                        Some(c) => StampColor::from_name(&c)
                            .with_context(|| format!("no colour called {c:?}"))?,
                        None => StampColor::Red,
                    },
                    name_and_date: date,
                },
                (None, None) => {
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
        Cmd::Verify {
            input,
            json,
            password,
            trust_file,
            require_trusted,
            signed_version,
            output,
        } => {
            let bytes =
                std::fs::read(&input).with_context(|| format!("reading {}", input.display()))?;
            let trust = trust_for_checking(&trust_file)?;
            let rep = limarepdf_core::verify_signatures(&bytes, password.as_deref(), &trust)?;
            if let (Some(n), Some(output)) = (signed_version, output) {
                let s = n
                    .checked_sub(1)
                    .and_then(|i| rep.signatures.get(i))
                    .with_context(|| {
                        format!(
                            "there is no signature {n}; the file has {}",
                            rep.signatures.len()
                        )
                    })?;
                if let SigStatus::Broken(why) = &s.status {
                    bail!(
                        "signature {n} is broken ({}), so it has no genuine signed version",
                        display_line(why, 300)
                    );
                }
                let len = s.signed_len.with_context(|| {
                    format!("LimarePDF can't tell which version signature {n} signed")
                })?;
                not_an_input(&[&input], &output)?;
                // A new file renamed into place, so a name that is a link to the input (or to
                // anything else) never has its bytes cut short (audit R16).
                saved(limarepdf_core::write_file(&output, &bytes[..len]))
                    .with_context(|| format!("writing {}", output.display()))?;
                out!(
                    "Saved the version signature {n} signed ({len} of {} bytes) to {}",
                    bytes.len(),
                    output.display()
                );
                if let SigStatus::CantCheck(why) = &s.status {
                    out!(
                        "Note: LimarePDF couldn't check signature {n} ({}), so this may not be what was signed.",
                        display_line(why, 300)
                    );
                }
                return Ok(());
            }
            let shown = (|| -> Result<()> {
                if json {
                    out!("{}", serde_json::to_string_pretty(&verify_json(&rep))?);
                } else {
                    print_verify(&input, &rep)?;
                }
                Ok(())
            })();
            // A reader that went away doesn't change the verdict's status (decision D5).
            match shown {
                Err(e) if !e.is::<Closed>() => return Err(e),
                _ if !rep.all_ok() || (require_trusted && !rep.all_trusted()) => {
                    std::process::exit(1)
                }
                shown => shown?,
            }
        }
        Cmd::Trust { cmd } => run_trust(cmd)?,
        Cmd::Id { cmd } => run_id(cmd)?,
        Cmd::Sign {
            input,
            output,
            id,
            field,
            page,
            rect,
            reason,
            location,
            certify,
            image,
            style,
            password_file,
            folder,
        } => {
            let place = match (field, page, rect) {
                (Some(name), _, _) => SignPlace::Field(name),
                (None, Some(page), Some(rect)) => {
                    let bad = || format!("--rect {rect:?} should be X,Y,W,H");
                    let v: Vec<f32> = rect
                        .split(',')
                        .map(|x| x.trim().parse())
                        .collect::<Result<_, _>>()
                        .with_context(bad)?;
                    if page == 0 || v.len() != 4 {
                        bail!("{}", bad());
                    }
                    SignPlace::NewBox {
                        page: page - 1,
                        rect: [v[0], v[1], v[0] + v[2], v[1] + v[3]],
                    }
                }
                _ => bail!("say where to sign: --field NAME, or --page N with --rect X,Y,W,H"),
            };
            let style = SignStyle::from_number(style).context("--style is 1, 2, 3 or 4")?;
            not_an_input(&[&input], &output)?;
            let picture = match &image {
                Some(path) => Some(
                    std::fs::read(path).with_context(|| format!("reading {}", path.display()))?,
                ),
                None => None,
            };
            let bytes =
                std::fs::read(&input).with_context(|| format!("reading {}", input.display()))?;
            let store = id_store(&folder)?;
            let chosen = match &id {
                Some(which) => store.find(which)?,
                None => store.default_id().context(
                    "you have no digital ID yet: make one with `limarepdf-cli id create` or import one \
                     with `limarepdf-cli id import`",
                )?,
            };
            let label = limarepdf_core::display_line(chosen.cert.display_name(), 80);
            if chosen.cert.expired() {
                bail!(
                    "the digital ID \"{label}\" expired on {}; LimarePDF doesn't sign with expired IDs",
                    chosen.cert.not_after.format("%Y-%m-%d")
                );
            }
            let password = read_password(&password_file, &format!("Password of \"{label}\": "))?;
            let unlocked = store.unlock(&chosen, &password)?;
            drop(password);
            let name = unlocked.cert.display_name().to_string();
            let art = match &picture {
                Some(b) => Some(SignArt::Picture {
                    bytes: b,
                    clear_paper: true,
                }),
                None if style != SignStyle::Caveat => Some(SignArt::Typed { text: &name, style }),
                None => None,
            };
            let signed = limarepdf_core::sign_document(
                &bytes,
                None,
                &SignRequest {
                    id: &unlocked,
                    place,
                    reason,
                    location,
                    certify: certify.map(|c| match c {
                        CertifyLevel::NoChanges => Permissions::NoChanges,
                        CertifyLevel::Forms => Permissions::FormFilling,
                        CertifyLevel::Comments => Permissions::FormFillingAndComments,
                    }),
                    art,
                    accept_disallowed: false,
                },
            )?;
            saved(limarepdf_core::write_file(&output, &signed.bytes))?;
            store.set_default(&chosen)?;
            let what = if certify.is_some() {
                "Certified"
            } else {
                "Signed"
            };
            out!(
                "{what} as \"{label}\" in the field \"{}\"; wrote {}.",
                limarepdf_core::display_line(&signed.field, 80),
                output.display()
            );
            if signed.rewritten {
                out!(
                    "The file's structure was damaged, or other PDF apps may show it \
                     differently, so it was written out in full, as LimarePDF shows it, before \
                     signing."
                );
            }
            if signed.finished_items > 0 {
                out!(
                    "{} fill-and-sign item(s) were drawn into the pages first.",
                    signed.finished_items
                );
            }
            if chosen.cert.self_issued {
                out!(
                    "Others see you as an unknown signer until they trust your certificate: send \
                     it with `limarepdf-cli id export {} -o {}.cer`.",
                    chosen.name,
                    chosen.name
                );
            }
        }
        Cmd::Search {
            input,
            query,
            password,
            boxes,
        } => {
            let p = open(&input, &password)?;
            let hits = p.search(&query)?;
            for h in &hits {
                out!("p{}: …{}…", h.page + 1, shown(&h.snippet));
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
                                "{:?} ({} {:.1}pt{})",
                                t,
                                shown(o.font_name.as_deref().unwrap_or_default()),
                                o.font_size.unwrap_or(0.0),
                                // A line in more than one style: its first part's.
                                if o.spans.iter().any(|s| !s.looks_like(&o.spans[0])) {
                                    ", mixed"
                                } else {
                                    ""
                                }
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
                let (mut p, original) = open_io(&io)?;
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
                save_edited(&io, &mut p, &original, false)?;
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
                        true => out!("{pages:<12} {}", shown(&labels[s])),
                        false => {
                            out!("{pages:<12} {} to {}", shown(&labels[s]), shown(&labels[e]))
                        }
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
        Cmd::ViewPrefs { cmd } => match cmd {
            ViewPrefsCmd::List { input, password } => {
                let p = open(&input, &password)?;
                let prefs = p.view_prefs();
                let labels = p.page_labels();
                let page = prefs.page.map(|n| match &labels {
                    Some(l) if l[n] != (n + 1).to_string() => {
                        format!("{} (label {})", n + 1, shown(&l[n]))
                    }
                    _ => (n + 1).to_string(),
                });
                out!("layout: {}", prefs.layout.map_or("not set", |l| l.name()));
                out!("panel:  {}", prefs.panel.map_or("not set", |s| s.name()));
                out!("page:   {}", page.as_deref().unwrap_or("not set"));
            }
            ViewPrefsCmd::Set {
                io,
                layout,
                panel,
                page,
            } => run_edit(&io, |p| {
                let mut prefs = p.view_prefs();
                if let Some(l) = layout {
                    prefs.layout = match l {
                        OpenLayoutArg::Continuous => Some(OpenLayout::Continuous),
                        OpenLayoutArg::Facing => Some(OpenLayout::Facing),
                        OpenLayoutArg::FacingCover => Some(OpenLayout::FacingCover),
                        OpenLayoutArg::Single => Some(OpenLayout::Single),
                        OpenLayoutArg::None => None,
                    };
                }
                if let Some(s) = panel {
                    prefs.panel = match s {
                        OpenPanelArg::Pages => Some(OpenPanel::Pages),
                        OpenPanelArg::Bookmarks => Some(OpenPanel::Bookmarks),
                        OpenPanelArg::None => None,
                    };
                }
                if let Some(n) = page {
                    prefs.page = match n {
                        OpenPageArg::Page(n) => Some(n - 1),
                        OpenPageArg::None => None,
                    };
                }
                p.set_view_prefs(&prefs)
            })?,
            ViewPrefsCmd::Clear { io } => {
                run_edit(&io, |p| p.set_view_prefs(&ViewPrefs::default()))?
            }
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
                    out!(
                        "{:>width$}  p. {page:<5} {indent}{}",
                        n + 1,
                        shown(&item.title)
                    );
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
                let (mut p, original) = open_io(&io)?;
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
                save_edited(&io, &mut p, &original, false)?;
                out!("removed {n} annotation(s); wrote {}", io.output.display());
            }
            AnnotCmd::Flatten { io, pages } => {
                let (mut p, original) = open_io(&io)?;
                let idx = match pages {
                    Some(r) => Some(parse_page_range(&r, p.page_count())?),
                    None => None,
                };
                let n = p.flatten_annotations(idx.as_deref())?;
                save_edited(&io, &mut p, &original, false)?;
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
            // The app asks; here the fields are named instead.
            let blank = p.unprintable_fields(&idx);
            if !blank.is_empty() {
                let (these, their) = match blank.len() {
                    1 => ("this field", "its value"),
                    _ => ("these fields", "their values"),
                };
                bail!(
                    "{these} wouldn't show on paper, because LimarePDF can't draw {their}: {}. \
                     Nothing was sent to the printer.",
                    limarepdf_core::quoted_list(&blank)
                );
            }
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
            overwrite,
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
            let files: Vec<PathBuf> = render::image_names(&idx, &stem, format)
                .iter()
                .map(|n| out_dir.join(n))
                .collect();
            not_taken(&files, overwrite)?;
            let t = std::time::Instant::now();
            saved(render::write_images(
                &hp, &idx, dpi, format, &out_dir, &stem,
            ))?;
            out!("rendered {} page(s) in {:.2?}", idx.len(), t.elapsed());
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    #[test]
    fn guard_turns_panics_into_errors() {
        assert_eq!(super::guard(|| Ok(3)).unwrap(), 3);
        let e = super::guard::<()>(|| panic!("bad xref")).unwrap_err();
        assert!(e.to_string().contains("bad xref"));
        assert!(e.to_string().contains("internal error"));
        let n = 7;
        let e = super::guard::<()>(|| panic!("index {n}")).unwrap_err();
        assert!(e.to_string().contains("index 7"));
        // Errors pass through unchanged.
        let e = super::guard::<()>(|| Err(anyhow::anyhow!("no such file"))).unwrap_err();
        assert_eq!(e.to_string(), "no such file");
    }
}

//! Printing.
//!
//! Linux and macOS print through CUPS (`lp`, `lpstat` and `lpoptions`). The selected pages
//! go to CUPS as a PDF on the printer's default paper, which CUPS renders with its own
//! engine. With
//! [`PrintOptions::as_image`], LimarePDF renders the pages itself and sends a PDF of page
//! images instead, so the output matches the screen.
//!
//! Windows prints through the print spooler and GDI: LimarePDF always renders the pages
//! itself and draws them on the printer (see `print_win.rs`).

use crate::render::{self, ImageFormat};
use crate::{ImageImport, Pdf, Result};
use anyhow::{anyhow, bail, Context};
use std::ffi::OsStr;
use std::io::Write;
use std::process::{Command, Stdio};

/// Resolution of pages printed as images.
pub const IMAGE_DPI: f32 = 300.0;

/// A printer (a CUPS queue, or a Windows printer).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Printer {
    pub name: String,
    /// The user's or the system's default printer.
    pub default: bool,
}

/// Two-sided printing.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Sides {
    #[default]
    One,
    /// Flip on the long edge (portrait books).
    LongEdge,
    /// Flip on the short edge (landscape, calendars).
    ShortEdge,
}

impl Sides {
    /// The IPP `sides` keyword.
    pub fn keyword(self) -> &'static str {
        match self {
            Sides::One => "one-sided",
            Sides::LongEdge => "two-sided-long-edge",
            Sides::ShortEdge => "two-sided-short-edge",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PrintOptions {
    /// Queue name; `None` prints to the default destination.
    pub printer: Option<String>,
    pub copies: u32,
    pub sides: Sides,
    /// Render the pages in LimarePDF and send images of them. Windows always does.
    pub as_image: bool,
    /// Job name shown in the print queue.
    pub title: String,
}

impl Default for PrintOptions {
    fn default() -> Self {
        Self {
            printer: None,
            copies: 1,
            sides: Sides::One,
            as_image: false,
            title: String::new(),
        }
    }
}

/// Run a CUPS command with untranslated output, explaining a missing command.
#[cfg(not(windows))]
fn cups(program: &str, args: &[&str]) -> Result<String> {
    let out = Command::new(program)
        .args(args)
        .env("LC_ALL", "C")
        .output()
        .map_err(|e| not_found(program, e))?;
    if !out.status.success() {
        return Err(failed(program, &out));
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

/// The error a CUPS command reported. Its messages start with its own name.
fn failed(program: &str, out: &std::process::Output) -> anyhow::Error {
    let err = String::from_utf8_lossy(&out.stderr);
    let err = err.trim();
    if err.is_empty() {
        anyhow!("{program} failed ({})", out.status)
    } else if err.starts_with(program) {
        anyhow!("{err}")
    } else {
        anyhow!("{program}: {err}")
    }
}

fn not_found(program: &str, e: std::io::Error) -> anyhow::Error {
    if e.kind() == std::io::ErrorKind::NotFound {
        anyhow!("printing needs CUPS, and its `{program}` command was not found")
    } else {
        anyhow!("cannot run {program}: {e}")
    }
}

/// The printers the system knows, default first.
pub fn printers() -> Result<Vec<Printer>> {
    #[cfg(windows)]
    return crate::print_win::printers();
    #[cfg(not(windows))]
    cups_printers()
}

#[cfg(not(windows))]
fn cups_printers() -> Result<Vec<Printer>> {
    let names = cups("lpstat", &["-e"])?;
    // "system default destination: NAME", or "no system default destination".
    let default = cups("lpstat", &["-d"]).unwrap_or_default();
    let default = default
        .lines()
        .find_map(|l| l.split_once("destination: "))
        .map(|(_, n)| n.trim().to_string());
    let mut v: Vec<Printer> = names
        .lines()
        .map(str::trim)
        .filter(|n| !n.is_empty())
        .map(|n| Printer {
            name: n.to_string(),
            default: default.as_deref() == Some(n),
        })
        .collect();
    v.sort_by_key(|p| !p.default);
    Ok(v)
}

/// The printer's default paper size, as `lpoptions -l` marks it (`Letter`, `A4`), or
/// `None` when it can't be found. Some printer setups print every page blurred when a
/// job doesn't name its paper, even though the printer has a default (other apps name it).
#[cfg(not(windows))]
pub fn default_paper(printer: Option<&str>) -> Option<String> {
    let out = match printer {
        Some(p) => cups("lpoptions", &["-p", p, "-l"]),
        None => cups("lpoptions", &["-l"]),
    };
    paper_in_lpoptions(&out.ok()?)
}

/// The marked choice of the `PageSize` line in `lpoptions -l` output, such as
/// "PageSize/Media Size: A5 *Letter A4". A custom size is left to the printer.
pub fn paper_in_lpoptions(text: &str) -> Option<String> {
    let line = text.lines().find(|l| l.starts_with("PageSize/"))?;
    let (_, choices) = line.split_once(':')?;
    let paper = choices
        .split_whitespace()
        .find_map(|c| c.strip_prefix('*'))?;
    (!paper.starts_with("Custom")).then(|| paper.to_string())
}

/// Arguments for `lp`, with `paper` (from [`default_paper`]) named when known; the
/// document is read from standard input.
pub fn lp_args(o: &PrintOptions, paper: Option<&str>) -> Vec<String> {
    let mut a = Vec::new();
    if let Some(p) = &o.printer {
        a.extend(["-d".into(), p.clone()]);
    }
    a.extend(["-n".into(), o.copies.max(1).to_string()]);
    if let Some(paper) = paper {
        a.extend(["-o".into(), format!("media={paper}")]);
    }
    if o.copies > 1 {
        a.extend(["-o".into(), "collate=true".into()]);
    }
    a.extend(["-o".into(), format!("sides={}", o.sides.keyword())]);
    let title = crate::display_line(&o.title, 100);
    if !title.is_empty() {
        a.extend(["-t".into(), title]);
    }
    a
}

/// Run `program` (normally `lp`) with `args`, writing `data` to its standard input.
/// Returns what it printed, such as "request id is Office-42 (1 file(s))".
pub fn send(program: impl AsRef<OsStr>, args: &[String], data: &[u8]) -> Result<String> {
    let program = program.as_ref();
    let name = program.to_string_lossy().into_owned();
    let mut child = Command::new(program)
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| not_found(&name, e))?;
    let mut stdin = child.stdin.take().context("no stdin")?;
    // Write from another thread so a full output pipe cannot block the copy.
    let out = std::thread::scope(|s| {
        let writer = s.spawn(move || stdin.write_all(data));
        let out = child.wait_with_output();
        (writer.join(), out)
    });
    let (written, out) = out;
    let out = out.with_context(|| format!("running {name}"))?;
    if !out.status.success() {
        let base = std::path::Path::new(program).file_name().unwrap_or(program);
        return Err(failed(&base.to_string_lossy(), &out));
    }
    match written {
        Ok(Ok(())) => {}
        Ok(Err(e)) => bail!("{name} did not take the whole document: {e}"),
        Err(_) => bail!("sending the document to {name} failed"),
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

impl Pdf {
    /// The document CUPS receives for `pages` (0-based, in order): those pages as an
    /// unencrypted PDF, or with `as_image` a PDF of page images at [`IMAGE_DPI`], rendered
    /// as for printing (annotations without the Print flag are left out).
    pub fn print_data(&mut self, pages: &[usize], as_image: bool) -> Result<Vec<u8>> {
        if pages.is_empty() {
            bail!("no pages selected");
        }
        if as_image {
            let hp = render::load(self.print_snapshot()?, None)?;
            let images = render::encode_pages(&hp, pages, IMAGE_DPI, ImageFormat::Jpeg(92))?;
            let opts = ImageImport {
                dpi: Some(IMAGE_DPI),
                ..Default::default()
            };
            return Pdf::from_images(&images, &opts)?.to_bytes();
        }
        let mut out = self.extract(pages)?;
        out.encryption = None;
        // Form fields go in as drawings: some CUPS setups drop fields without drawing them,
        // so filled values would print empty.
        out.flatten_form_for_print()?;
        // Each page's content leaves the graphics state as it found it, so annotations that
        // CUPS draws into the page after it aren't turned by a transformation the page set
        // and never undid (some CUPS versions add them without restoring the state).
        for page in out.page_ids() {
            out.wrap_contents(page, None, None)?;
        }
        out.to_bytes()
    }

    /// Print `pages` (0-based): with CUPS, returning `lp`'s reply, or on Windows through
    /// the print spooler, returning the job number.
    pub fn print(&mut self, pages: &[usize], opts: &PrintOptions) -> Result<String> {
        if pages.is_empty() {
            bail!("no pages selected");
        }
        #[cfg(windows)]
        return crate::print_win::print(self, pages, opts);
        #[cfg(not(windows))]
        {
            let data = self.print_data(pages, opts.as_image)?;
            let paper = default_paper(opts.printer.as_deref());
            send("lp", &lp_args(opts, paper.as_deref()), &data)
        }
    }
}

/// Where a page goes on a sheet when LimarePDF draws it on the printer itself (Windows).
#[derive(Clone, Copy, Debug, PartialEq)]
#[cfg_attr(not(windows), allow(dead_code))]
pub(crate) struct Placement {
    /// Turn the page a quarter turn anticlockwise, so its top is at the sheet's left.
    pub rotate: bool,
    /// Box in device pixels, relative to the printable area.
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
    /// Scale to render the page at: device pixels per point, at most 600 dpi.
    pub scale: f32,
}

/// Device measurements of a printer's sheet, from GDI's GetDeviceCaps.
#[derive(Clone, Copy, Debug)]
#[cfg_attr(not(windows), allow(dead_code))]
pub(crate) struct Sheet {
    /// Printable area, in device pixels.
    pub area: (i32, i32),
    /// Whole sheet, in device pixels.
    pub paper: (i32, i32),
    /// Top-left of the printable area on the sheet.
    pub offset: (i32, i32),
    /// Device pixels per inch.
    pub dpi: (i32, i32),
}

/// Place a page of `size` points on `sheet`: turned to match the sheet's orientation,
/// at actual size when it fits in the printable area and shrunk to fit when not, centred
/// on the sheet as far as the printable area allows.
#[cfg_attr(not(windows), allow(dead_code))]
pub(crate) fn place(size: (f32, f32), sheet: &Sheet) -> Placement {
    let (aw, ah) = (sheet.area.0.max(1) as f32, sheet.area.1.max(1) as f32);
    let rotate = (size.0 > size.1 && aw < ah) || (size.0 < size.1 && aw > ah);
    let (pw, ph) = if rotate { (size.1, size.0) } else { size };
    let (pw, ph) = (pw.max(1.0), ph.max(1.0));
    let (dx, dy) = (sheet.dpi.0.max(1) as f32, sheet.dpi.1.max(1) as f32);
    // Device pixels at actual size.
    let (fw, fh) = (pw / 72.0 * dx, ph / 72.0 * dy);
    let fit = (aw / fw).min(ah / fh).min(1.0);
    let (w, h) = ((fw * fit).round(), (fh * fit).round());
    let centre = |paper: i32, offset: i32, area: f32, len: f32| {
        let x = (paper as f32 - len) / 2.0 - offset as f32;
        x.clamp(0.0, (area - len).max(0.0)).round() as i32
    };
    Placement {
        rotate,
        x: centre(sheet.paper.0, sheet.offset.0, aw, w),
        y: centre(sheet.paper.1, sheet.offset.1, ah, h),
        w: w as i32,
        h: h as i32,
        scale: (w / pw).min(600.0 / 72.0),
    }
}

/// `rgba` (width `w`) as top-down BGRA for GDI, turned a quarter turn anticlockwise when
/// `rotate`. Returns the pixels and their width and height.
#[cfg_attr(not(windows), allow(dead_code))]
pub(crate) fn to_bgra(rgba: &[u8], w: usize, h: usize, rotate: bool) -> (Vec<u8>, usize, usize) {
    let (px, _) = rgba.as_chunks::<4>();
    let bgra = |p: &[u8; 4]| [p[2], p[1], p[0], 255];
    if !rotate {
        return (px.iter().flat_map(bgra).collect(), w, h);
    }
    // Anticlockwise: source (x, y) goes to (y, w - 1 - x) in an h-wide image.
    let mut out = vec![0u8; w * h * 4];
    for y in 0..h {
        for x in 0..w {
            let d = ((w - 1 - x) * h + y) * 4;
            out[d..d + 4].copy_from_slice(&bgra(&px[y * w + x]));
        }
    }
    (out, h, w)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// US Letter at 600 dpi with quarter-inch margins.
    const LETTER: Sheet = Sheet {
        area: (4800, 6300),
        paper: (5100, 6600),
        offset: (150, 150),
        dpi: (600, 600),
    };

    #[test]
    fn small_page_prints_at_actual_size_centred() {
        // 4 x 3 inch page: 2400 x 1800 px, landscape, so turned to 1800 x 2400.
        let p = place((288.0, 216.0), &LETTER);
        assert!(p.rotate);
        assert_eq!((p.w, p.h), (1800, 2400));
        // Centred on the sheet: (5100 - 1800) / 2 - 150 = 1500.
        assert_eq!((p.x, p.y), (1500, 1950));
        assert!((p.scale - 600.0 / 72.0).abs() < 1e-3);
    }

    #[test]
    fn full_page_shrinks_to_the_printable_area() {
        let p = place((612.0, 792.0), &LETTER);
        assert!(!p.rotate);
        // 5100 x 6600 px at actual size; the width limits: 4800 / 5100 of it.
        assert_eq!((p.w, p.h), (4800, 6212));
        // Across: the whole printable width. Down: (6600 - 6212) / 2 - 150 = 44.
        assert_eq!((p.x, p.y), (0, 44));
    }

    #[test]
    fn render_scale_is_capped_at_600_dpi() {
        let sheet = Sheet {
            dpi: (1200, 1200),
            area: (9600, 12600),
            paper: (10200, 13200),
            offset: (300, 300),
        };
        let p = place((288.0, 216.0), &sheet);
        assert!((p.scale - 600.0 / 72.0).abs() < 1e-3, "{p:?}");
    }

    #[test]
    fn quarter_turn_puts_the_top_on_the_left() {
        // 2 x 1 image: red, green.
        let rgba = [255, 0, 0, 255, 0, 255, 0, 255];
        let (out, w, h) = to_bgra(&rgba, 2, 1, true);
        assert_eq!((w, h), (1, 2));
        // Green (the right end) goes to the top, red to the bottom; bytes are BGRA.
        assert_eq!(out, [0, 255, 0, 255, 0, 0, 255, 255]);
        let (out, w, h) = to_bgra(&rgba, 2, 1, false);
        assert_eq!((w, h), (2, 1));
        assert_eq!(out, [0, 0, 255, 255, 0, 255, 0, 255]);
    }
}

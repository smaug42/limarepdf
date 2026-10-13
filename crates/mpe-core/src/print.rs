//! Printing through CUPS (`lp` and `lpstat`), on Linux and macOS.
//!
//! The selected pages go to CUPS as a PDF, which CUPS renders with its own engine. With
//! [`PrintOptions::as_image`], MPE renders the pages itself and sends a PDF of page
//! images instead, so the output matches the screen.

use crate::render::{self, ImageFormat};
use crate::{ImageImport, Pdf, Result};
use anyhow::{anyhow, bail, Context};
use std::ffi::OsStr;
use std::io::Write;
use std::process::{Command, Stdio};

/// Resolution of pages printed as images.
pub const IMAGE_DPI: f32 = 300.0;

/// A print queue known to CUPS.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Printer {
    pub name: String,
    /// The user's or the system's default destination.
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
    /// Render the pages in MPE and send images of them.
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

fn unsupported() -> Result<()> {
    if cfg!(windows) {
        bail!("printing is not available on Windows yet");
    }
    Ok(())
}

/// Run a CUPS command with untranslated output, explaining a missing command.
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

/// The printers CUPS knows, default first.
pub fn printers() -> Result<Vec<Printer>> {
    unsupported()?;
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

/// Arguments for `lp`; the document is read from standard input.
pub fn lp_args(o: &PrintOptions) -> Vec<String> {
    let mut a = Vec::new();
    if let Some(p) = &o.printer {
        a.extend(["-d".into(), p.clone()]);
    }
    a.extend(["-n".into(), o.copies.max(1).to_string()]);
    if o.copies > 1 {
        a.extend(["-o".into(), "collate=true".into()]);
    }
    a.extend(["-o".into(), format!("sides={}", o.sides.keyword())]);
    if !o.title.is_empty() {
        a.extend(["-t".into(), o.title.clone()]);
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
        out.to_bytes()
    }

    /// Print `pages` (0-based) with CUPS. Returns `lp`'s reply.
    pub fn print(&mut self, pages: &[usize], opts: &PrintOptions) -> Result<String> {
        unsupported()?;
        let data = self.print_data(pages, opts.as_image)?;
        send("lp", &lp_args(opts), &data)
    }
}

//! Printing.
//!
//! LimarePDF lays out every printout itself: each page goes on a sheet of the chosen paper
//! as the page setup says (`sheets.rs`), so what is printed is the same on every platform.
//!
//! Linux and macOS print through CUPS (`lp`, `lpstat`, `lpoptions`, and IPP for the paper
//! sizes). The sheets go to CUPS as a PDF, named with their paper and with CUPS's own
//! scaling turned off, and CUPS renders them with its own engine. With
//! [`PrintOptions::as_image`], LimarePDF renders the sheets itself and sends a PDF of their
//! images instead, so the output matches the screen.
//!
//! Windows prints through the print spooler and GDI: LimarePDF always renders the sheets
//! itself and draws them on the printer (see `print_win.rs`).

use crate::render::{self, ImageFormat};
use crate::{ImageImport, Pdf, Result};
use anyhow::{anyhow, bail, Context};
use std::ffi::OsStr;
use std::io::{Read, Write};
use std::process::{Command, Output, Stdio};
use std::time::{Duration, Instant};

pub use crate::paper::{Paper, PaperList};
pub use crate::sheets::{
    place, sheets, Orientation, PageOrder, PageSetup, PerSheet, Placed, Scale, SheetPlan,
    ASSUMED_MARGIN, MAX_PERCENT, PAGES_PER_SHEET,
};

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

#[derive(Clone, Debug, PartialEq)]
pub struct PrintOptions {
    /// Queue name; `None` prints to the default destination.
    pub printer: Option<String>,
    pub copies: u32,
    pub sides: Sides,
    /// Render the sheets in LimarePDF and send images of them. Windows always does.
    pub as_image: bool,
    /// Job name shown in the print queue.
    pub title: String,
    /// The paper to print on, one of [`papers`]; `None` for the printer's default
    /// ([`printer_paper`]).
    pub paper: Option<Paper>,
    pub orientation: Orientation,
    pub scale: Scale,
    pub per_sheet: PerSheet,
}

impl Default for PrintOptions {
    fn default() -> Self {
        Self {
            printer: None,
            copies: 1,
            sides: Sides::One,
            as_image: false,
            title: String::new(),
            paper: None,
            orientation: Orientation::Automatic,
            scale: Scale::ShrinkOversized,
            per_sheet: PerSheet::default(),
        }
    }
}

/// How long a CUPS command asking about the printers (`lpstat`, `lpoptions`) may take. A
/// working printing service answers at once.
pub const ASK_WITHIN: Duration = Duration::from_secs(10);

/// How long `lp` may take to take the document, and then to confirm the job.
pub const SEND_WITHIN: Duration = Duration::from_secs(120);

/// A command run by [`run_within`] that finished in time.
struct Ran {
    out: Output,
    /// Whether it took all of its input.
    written: std::io::Result<()>,
}

/// Run `cmd`, writing `input` to its standard input, and wait for it, as long as `limit`
/// from its start and again from its taking all of the input. One that takes longer is
/// stopped, with any commands it started itself, and gives `None`.
fn run_within(cmd: &mut Command, input: &[u8], limit: Duration) -> std::io::Result<Option<Ran>> {
    #[cfg(unix)]
    std::os::unix::process::CommandExt::process_group(cmd, 0);
    let mut child = cmd
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    let (mut stdin, stdout, stderr) =
        (child.stdin.take(), child.stdout.take(), child.stderr.take());
    std::thread::scope(|s| {
        // Written and read from threads of their own, so that a full pipe can't block.
        let writer = s.spawn(move || match &mut stdin {
            Some(w) => w.write_all(input),
            None => Err(std::io::Error::other("no stdin")),
        });
        let (stdout, stderr) = (read_all(s, stdout), read_all(s, stderr));
        let mut deadline = Instant::now() + limit;
        let mut writing = true;
        let status = loop {
            if writing && writer.is_finished() {
                writing = false;
                deadline = Instant::now() + limit;
            }
            match child.try_wait() {
                Ok(Some(status)) => break status,
                Ok(None) if Instant::now() < deadline => {}
                Ok(None) => {
                    stop(&mut child);
                    return Ok(None);
                }
                Err(e) => {
                    stop(&mut child);
                    return Err(e);
                }
            }
            std::thread::sleep(Duration::from_millis(10));
        };
        let written = writer
            .join()
            .unwrap_or_else(|_| Err(std::io::Error::other("panicked")));
        let out = Output {
            status,
            stdout: stdout.join().unwrap_or_default(),
            stderr: stderr.join().unwrap_or_default(),
        };
        Ok(Some(Ran { out, written }))
    })
}

/// Read all of `pipe` on a thread of scope `s`.
fn read_all<'scope>(
    s: &'scope std::thread::Scope<'scope, '_>,
    pipe: Option<impl Read + Send + 'scope>,
) -> std::thread::ScopedJoinHandle<'scope, Vec<u8>> {
    s.spawn(move || {
        let mut v = Vec::new();
        if let Some(mut p) = pipe {
            let _ = p.read_to_end(&mut v);
        }
        v
    })
}

/// Stop `child` and every command it started in its process group, and wait for it.
fn stop(child: &mut std::process::Child) {
    #[cfg(unix)]
    let _ = rustix::process::kill_process_group(
        rustix::process::Pid::from_child(child),
        rustix::process::Signal::KILL,
    );
    let _ = child.kill();
    let _ = child.wait();
}

/// Why a command was stopped by [`run_within`] after `limit`.
#[cfg(not(windows))]
fn too_slow(program: &str, limit: Duration) -> anyhow::Error {
    anyhow!(
        "the printing service didn't answer within {} seconds ({program})",
        limit.as_secs()
    )
}

/// Runs `job` on a thread of its own and waits for it for `limit`. One that takes longer is
/// left to finish on its own (a call into the print spooler can't be stopped) and gives an
/// error saying the printing service didn't answer when asked for `what`.
pub(crate) fn within<T: Send + 'static>(
    what: &str,
    limit: Duration,
    job: impl FnOnce() -> Result<T> + Send + 'static,
) -> Result<T> {
    use std::sync::mpsc::{channel, RecvTimeoutError};
    let (tx, rx) = channel();
    std::thread::spawn(move || {
        let _ = tx.send(job());
    });
    rx.recv_timeout(limit).unwrap_or_else(|e| match e {
        RecvTimeoutError::Timeout => bail!(
            "the printing service didn't answer within {} seconds when asked for {what}",
            limit.as_secs()
        ),
        RecvTimeoutError::Disconnected => bail!("asking the printing service for {what} failed"),
    })
}

/// Run a CUPS command with untranslated output, explaining a missing command, stopping it
/// after [`ASK_WITHIN`].
#[cfg(not(windows))]
pub(crate) fn cups(program: &str, args: &[&str]) -> Result<String> {
    let ran = run_within(
        Command::new(program).args(args).env("LC_ALL", "C"),
        &[],
        ASK_WITHIN,
    )
    .map_err(|e| not_found(program, e))?;
    let out = ran.ok_or_else(|| too_slow(program, ASK_WITHIN))?.out;
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

/// The printers the system knows, default first, if the printing service lists them within
/// [`ASK_WITHIN`] (for each `lpstat` it is asked with).
pub fn printers() -> Result<Vec<Printer>> {
    // The spooler's call can't be stopped: it is left to finish on its own thread.
    #[cfg(windows)]
    {
        within("the printers", ASK_WITHIN, crate::print_win::printers)
    }
    #[cfg(not(windows))]
    cups_printers()
}

/// The printers the system knows, default first, given to `found` as they become known, its
/// second argument true for the last. On Linux and macOS those CUPS has queues for come
/// first, asked over IPP in milliseconds (not when there are none), then the whole list as
/// [`printers`] gives it, which waits about a second for printers the network announces. On
/// Windows the print spooler's list, once.
pub fn printers_in_steps(mut found: impl FnMut(Result<Vec<Printer>>, bool)) {
    #[cfg(not(windows))]
    if let Ok(now) = cups_printers_now() {
        if !now.is_empty() {
            found(Ok(now), false);
        }
    }
    found(printers(), true);
}

/// The printers CUPS has queues for, default first, asked over IPP.
#[cfg(not(windows))]
fn cups_printers_now() -> Result<Vec<Printer>> {
    let server = crate::ipp::server();
    // The thread's limit a little over the requests' own, so that their message is said.
    let limit = ASK_WITHIN + Duration::from_secs(2);
    let (names, default) = within("the printers", limit, move || {
        crate::ipp::cups_printers(&server, ASK_WITHIN)
    })?;
    let mut v: Vec<Printer> = names
        .into_iter()
        .map(|name| Printer {
            default: default.as_deref() == Some(name.as_str()),
            name,
        })
        .collect();
    v.sort_by_key(|p| !p.default);
    Ok(v)
}

/// The paper sizes `printer` takes and its default, as the printing service reports them,
/// each with the margins the printer can't print in when it says, within [`ASK_WITHIN`] for
/// each question asked. On Linux and macOS CUPS is asked over IPP, and failing that its
/// `lpoptions` names the sizes without margins; on Windows the printer's driver is asked.
pub fn papers(printer: &str) -> Result<PaperList> {
    #[cfg(windows)]
    {
        let name = printer.to_string();
        within("the printer's paper sizes", ASK_WITHIN, move || {
            crate::print_win::papers(&name)
        })
    }
    #[cfg(not(windows))]
    crate::paper::cups_papers(printer)
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

/// The default printer's name.
fn default_printer() -> Result<String> {
    printers()?
        .into_iter()
        .find(|p| p.default)
        .map(|p| p.name)
        .ok_or_else(|| anyhow!("no default printer is set"))
}

/// The paper `printer` (the default printer for `None`) prints on when none is chosen: its
/// default size, else the first it lists ([`papers`]).
pub fn printer_paper(printer: Option<&str>) -> Result<Paper> {
    let name = match printer {
        Some(p) => p.to_string(),
        None => default_printer()?,
    };
    let list = papers(&name)?;
    let i = list.default.unwrap_or(0);
    let paper = list.papers.into_iter().nth(i);
    paper.ok_or_else(|| anyhow!("the printing service doesn't list the printer's paper sizes"))
}

/// The paper called `name` that `printer` (the default printer for `None`) takes, matched
/// in any case with its id, its name for people ("Letter (8.5 × 11 in)") or that name
/// without its size ("Letter"). When the printing service can't list the printer's sizes,
/// CUPS is sent a standard size by its own name ("A4", "Letter"), margins unknown.
pub fn paper_named(printer: Option<&str>, name: &str) -> Result<Paper> {
    let listed = match printer {
        Some(p) => papers(p),
        None => default_printer().and_then(|p| papers(&p)),
    };
    match listed {
        Ok(list) => crate::paper::named(&list, name),
        #[cfg(not(windows))]
        Err(e) => crate::paper::standard(name).ok_or(e),
        #[cfg(windows)]
        Err(e) => Err(e),
    }
}

/// Arguments for `lp` sending sheets laid out on the paper with id `paper`: the paper
/// named, and CUPS's own scaling turned off, as CUPS would otherwise shrink a sheet the
/// paper's size into the printable area again. The document is read from standard input.
pub fn lp_args(o: &PrintOptions, paper: &str) -> Vec<String> {
    let mut a = Vec::new();
    if let Some(p) = &o.printer {
        a.extend(["-d".into(), p.clone()]);
    }
    a.extend(["-n".into(), o.copies.max(1).to_string()]);
    a.extend(["-o".into(), format!("media={paper}")]);
    a.extend(["-o".into(), "print-scaling=none".into()]);
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

/// Runs `job` on a thread of its own, which calls the beat it is given each time it gets
/// on, and waits for it while it beats at least every `within`. A job that stops beating is
/// left to finish on its own: a Windows print job stuck in the print spooler can't be
/// stopped, and waiting for it would hold up the app's other work until it closes (PB-17).
#[cfg_attr(not(windows), allow(dead_code))]
pub(crate) fn while_beating<T: Send + 'static>(
    within: Duration,
    job: impl FnOnce(&dyn Fn()) -> Result<T> + Send + 'static,
) -> Result<T> {
    use std::sync::mpsc::{channel, RecvTimeoutError};
    let (tx, rx) = channel::<Option<Result<T>>>();
    let beats = tx.clone();
    std::thread::spawn(move || {
        let beat = move || {
            let _ = beats.send(None);
        };
        let out = job(&beat);
        let _ = tx.send(Some(out));
    });
    loop {
        match rx.recv_timeout(within) {
            Ok(Some(out)) => return out,
            Ok(None) => {}
            Err(RecvTimeoutError::Timeout) => bail!(
                "the printer didn't take the document within {} seconds, so LimarePDF stopped \
                 waiting. Check the printer's queue before printing again: the job may still \
                 print once the printer is ready",
                within.as_secs()
            ),
            Err(RecvTimeoutError::Disconnected) => {
                bail!("the print job stopped without saying how it went")
            }
        }
    }
}

/// Run `program` (normally `lp`) with `args`, writing `data` to its standard input.
/// Returns what it printed, such as "request id is Office-42 (1 file(s))". It is stopped
/// if it takes longer than [`SEND_WITHIN`] to take the document, or then to answer.
pub fn send(program: impl AsRef<OsStr>, args: &[String], data: &[u8]) -> Result<String> {
    send_within(program, args, data, SEND_WITHIN)
}

fn send_within(
    program: impl AsRef<OsStr>,
    args: &[String],
    data: &[u8],
    limit: Duration,
) -> Result<String> {
    let program = program.as_ref();
    let name = program.to_string_lossy().into_owned();
    let ran = run_within(Command::new(program).args(args), data, limit)
        .map_err(|e| not_found(&name, e))?;
    let Some(Ran { out, written }) = ran else {
        bail!(
            "the printing service didn't confirm the job within {} seconds, so LimarePDF \
             stopped waiting. Check the printer's queue before printing again: the job may \
             have been sent",
            limit.as_secs()
        );
    };
    if !out.status.success() {
        let base = std::path::Path::new(program).file_name().unwrap_or(program);
        return Err(failed(&base.to_string_lossy(), &out));
    }
    if let Err(e) = written {
        bail!("{name} did not take the whole document: {e}");
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

impl Pdf {
    /// The document CUPS receives for `pages` (0-based, in order) laid out as `setup` says:
    /// the sheets ([`Pdf::print_sheets`]) as an unencrypted PDF, or with `as_image` a PDF
    /// of their images at [`IMAGE_DPI`].
    pub fn print_data(
        &self,
        pages: &[usize],
        setup: &PageSetup,
        as_image: bool,
    ) -> Result<Vec<u8>> {
        let mut sheets = self.print_sheets(pages, setup)?;
        if !as_image {
            return sheets.to_bytes();
        }
        let all: Vec<usize> = (0..sheets.page_count()).collect();
        let hp = render::load(sheets.to_bytes()?, None)?;
        let images = render::encode_pages(&hp, &all, IMAGE_DPI, ImageFormat::Jpeg(92))?;
        let opts = ImageImport {
            dpi: Some(IMAGE_DPI),
            ..Default::default()
        };
        Pdf::from_images(&images, &opts)?.to_bytes()
    }

    /// Print `pages` (0-based) laid out as `opts` says, on the printer's default paper when
    /// it names none: with CUPS, returning `lp`'s reply, or on Windows through the print
    /// spooler, returning the job number. Gives up waiting, as `lp` is given up on, when the
    /// spooler takes no step for [`SEND_WITHIN`] ([`while_beating`]).
    pub fn print(&mut self, pages: &[usize], opts: &PrintOptions) -> Result<String> {
        if pages.is_empty() {
            bail!("no pages selected");
        }
        let paper = match &opts.paper {
            Some(p) => p.clone(),
            None => printer_paper(opts.printer.as_deref()).context(
                "LimarePDF couldn't find out which paper the printer takes, so nothing was \
                 sent to it",
            )?,
        };
        let setup = PageSetup {
            paper,
            orientation: opts.orientation,
            scale: opts.scale,
            per_sheet: opts.per_sheet,
        };
        #[cfg(windows)]
        {
            let mut sheets = self.print_sheets(pages, &setup)?;
            let (count, data) = (sheets.page_count(), sheets.to_bytes()?);
            let opts = opts.clone();
            while_beating(SEND_WITHIN, move |beat| {
                crate::print_win::print(data, count, &setup.paper, &opts, beat)
            })
        }
        #[cfg(not(windows))]
        {
            let data = self.print_data(pages, &setup, opts.as_image)?;
            send("lp", &lp_args(opts, &setup.paper.id), &data)
        }
    }
}

/// Device measurements of a printer's sheet, from GDI's GetDeviceCaps.
#[derive(Clone, Copy, Debug)]
#[cfg_attr(not(windows), allow(dead_code))]
pub(crate) struct Sheet {
    /// Printable area, in device pixels.
    pub area: (i32, i32),
    /// Top-left of the printable area on the sheet.
    pub offset: (i32, i32),
    /// Device pixels per inch.
    pub dpi: (i32, i32),
}

/// The scale to draw a sheet `size` points large at for printer sheet `dev`: device pixels
/// per point, at most 600 dpi and at most [`crate::render::MAX_PIXELS`] in all (the printer
/// scales a smaller picture up).
#[cfg_attr(not(windows), allow(dead_code))]
pub(crate) fn sheet_scale(size: (f32, f32), dev: &Sheet) -> f32 {
    let dpi = dev.dpi.0.max(dev.dpi.1).max(1) as f32;
    let (w, h) = (size.0.max(1.0), size.1.max(1.0));
    // Just under the pixel limit, so rounding in f32 can't take it over.
    (dpi / 72.0)
        .min(600.0 / 72.0)
        .min((crate::render::MAX_PIXELS as f32 / (w * h)).sqrt() * 0.999)
}

/// Part of a picture of a sheet, and where it goes on the printer's sheet.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(not(windows), allow(dead_code))]
pub(crate) struct Part {
    /// Where it is in the picture: left, top, width and height in pixels.
    pub from: [usize; 4],
    /// Where it goes: left, top, width and height in device pixels from the printable
    /// area's top-left corner.
    pub to: [i32; 4],
}

/// The part of a picture of a sheet `size` points large, `px` pixels wide and high, that
/// falls on the printable area of printer sheet `dev`, the sheet's top-left corner on the
/// paper's. `None` when none of it does.
#[cfg_attr(not(windows), allow(dead_code))]
pub(crate) fn printable_part(size: (f32, f32), px: (usize, usize), dev: &Sheet) -> Option<Part> {
    // Device pixels a pixel of the picture.
    let (kx, ky) = (
        size.0 / 72.0 * dev.dpi.0 as f32 / px.0.max(1) as f32,
        size.1 / 72.0 * dev.dpi.1 as f32 / px.1.max(1) as f32,
    );
    if kx <= 0.0 || ky <= 0.0 {
        return None;
    }
    let span = |offset: i32, len: i32, k: f32, most: usize| {
        let a = (offset as f32 / k).floor().clamp(0.0, most as f32) as usize;
        let b = ((offset + len) as f32 / k).ceil().clamp(0.0, most as f32) as usize;
        (b > a).then_some((a, b))
    };
    let (x0, x1) = span(dev.offset.0, dev.area.0, kx, px.0)?;
    let (y0, y1) = span(dev.offset.1, dev.area.1, ky, px.1)?;
    let to = |a: usize, b: usize, k: f32, offset: i32| {
        let start = (a as f32 * k).round() as i32 - offset;
        (start, (b as f32 * k).round() as i32 - offset - start)
    };
    let ((tx, tw), (ty, th)) = (to(x0, x1, kx, dev.offset.0), to(y0, y1, ky, dev.offset.1));
    Some(Part {
        from: [x0, y0, x1 - x0, y1 - y0],
        to: [tx, ty, tw, th],
    })
}

/// The part `from` (left, top, width, height) of the picture `rgba`, `w` pixels wide, as
/// top-down BGRA for GDI.
#[cfg_attr(not(windows), allow(dead_code))]
pub(crate) fn to_bgra(rgba: &[u8], w: usize, from: [usize; 4]) -> Vec<u8> {
    let (px, _) = rgba.as_chunks::<4>();
    let [x, y, pw, ph] = from;
    let mut out = Vec::with_capacity(pw * ph * 4);
    for row in y..y + ph {
        for p in &px[row * w + x..row * w + x + pw] {
            out.extend_from_slice(&[p[2], p[1], p[0], 255]);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// US Letter at 600 dpi with quarter-inch margins.
    const LETTER: Sheet = Sheet {
        area: (4800, 6300),
        offset: (150, 150),
        dpi: (600, 600),
    };

    /// A job that keeps beating is waited for however long it takes; one that stops is let
    /// go after the wait, the app free again while it goes on (PB-17). The beats come 50
    /// times as often as the wait, and the stuck job sleeps three times as long, so a slow,
    /// busy CI runner doesn't miss a beat (a 200 ms wait with beats every 100 ms failed on
    /// macOS Intel).
    #[test]
    fn a_job_is_waited_for_while_it_gets_on() {
        let within = Duration::from_secs(1);
        let start = Instant::now();
        let steady = while_beating(within, |beat| {
            for _ in 0..100 {
                std::thread::sleep(Duration::from_millis(20));
                beat();
            }
            Ok("done")
        });
        assert_eq!(steady.unwrap(), "done");
        // Longer than the wait: it was waited for because it kept beating.
        assert!(start.elapsed() > within, "{:?}", start.elapsed());
        let (tx, rx) = std::sync::mpsc::channel();
        let start = Instant::now();
        let stuck = while_beating(within, move |beat| {
            beat();
            std::thread::sleep(Duration::from_secs(3));
            let _ = tx.send(());
            Ok(())
        });
        let waited = start.elapsed();
        let err = format!("{:#}", stuck.unwrap_err());
        assert!(err.contains("stopped waiting"), "{err}");
        assert!(waited < Duration::from_millis(2500), "{waited:?}");
        // The job itself goes on.
        assert!(rx.recv_timeout(Duration::from_secs(10)).is_ok());
        // One that ends without saying (it panicked) is said too.
        let gone = while_beating::<()>(within, |_| panic!("lost"));
        assert!(format!("{:#}", gone.unwrap_err()).contains("without saying"));
    }

    #[test]
    fn a_sheet_drawn_at_the_printers_resolution_gives_its_printable_area() {
        // A Letter sheet drawn at 600 dpi: a picture pixel a device pixel.
        assert!((sheet_scale((612.0, 792.0), &LETTER) - 600.0 / 72.0).abs() < 1e-4);
        let part = printable_part((612.0, 792.0), (5100, 6600), &LETTER).unwrap();
        assert_eq!(part.from, [150, 150, 4800, 6300]);
        assert_eq!(part.to, [0, 0, 4800, 6300]);
    }

    #[test]
    fn a_sheet_drawn_at_half_the_resolution_is_stretched_over_the_printable_area() {
        let part = printable_part((612.0, 792.0), (2550, 3300), &LETTER).unwrap();
        assert_eq!(part.from, [75, 75, 2400, 3150]);
        assert_eq!(part.to, [0, 0, 4800, 6300]);
        // An odd offset: the picture's pixels straddling the edges are kept whole, a device
        // pixel over each side (the printer leaves out what is off its area).
        let odd = Sheet {
            offset: (151, 150),
            ..LETTER
        };
        let part = printable_part((612.0, 792.0), (2550, 3300), &odd).unwrap();
        assert_eq!(part.from[0], 75);
        assert_eq!(part.from[2], 2476 - 75);
        assert_eq!((part.to[0], part.to[2]), (-1, 4802));
    }

    #[test]
    fn a_sheet_off_the_printable_area_draws_nothing() {
        // A sheet an inch square, where the printable area starts two inches in.
        let far = Sheet {
            offset: (1200, 1200),
            ..LETTER
        };
        assert_eq!(printable_part((72.0, 72.0), (600, 600), &far), None);
    }

    #[test]
    fn the_drawing_scale_is_capped_at_600_dpi() {
        let sheet = Sheet {
            dpi: (1200, 1200),
            area: (9600, 12600),
            offset: (300, 300),
        };
        let s = sheet_scale((612.0, 792.0), &sheet);
        assert!((s - 600.0 / 72.0).abs() < 1e-3, "{s}");
    }

    #[test]
    fn the_drawing_scale_keeps_a_poster_within_the_pixel_limit() {
        // A0 on an A0 plotter at 600 dpi: 19,866 x 28,087 px, more than the limit.
        let sheet = Sheet {
            dpi: (600, 600),
            area: (19866, 28087),
            offset: (0, 0),
        };
        let (pw, ph) = (2384.0, 3370.0);
        let s = sheet_scale((pw, ph), &sheet);
        assert!(s < 600.0 / 72.0, "{s}");
        let pixels = (pw * s) as u64 * (ph * s) as u64;
        assert!(pixels <= crate::render::MAX_PIXELS, "{pixels}");
        assert!(pixels > crate::render::MAX_PIXELS * 99 / 100, "{pixels}");
    }

    #[test]
    fn bgra_takes_the_part_asked_for() {
        // 3 x 2 picture: red, green, blue over white, black, grey.
        let rgba = [
            255, 0, 0, 255, 0, 255, 0, 255, 0, 0, 255, 255, //
            255, 255, 255, 255, 0, 0, 0, 255, 9, 9, 9, 255,
        ];
        // The right two columns of both rows; bytes are BGRA.
        let out = to_bgra(&rgba, 3, [1, 0, 2, 2]);
        assert_eq!(
            out,
            [0, 255, 0, 255, 255, 0, 0, 255, 0, 0, 0, 255, 9, 9, 9, 255]
        );
    }

    #[cfg(unix)]
    fn sh(script: &str) -> Command {
        let mut c = Command::new("sh");
        c.args(["-c", script]);
        c
    }

    /// A command that never answers is stopped at the limit, with the commands it started:
    /// their open output would otherwise keep it waiting (BUG-11).
    #[cfg(unix)]
    #[test]
    fn a_command_that_never_answers_is_stopped() {
        let start = Instant::now();
        // `; true` keeps the shell from becoming `sleep`: sleep is its own command.
        let ran = run_within(&mut sh("sleep 30; true"), &[], Duration::from_millis(300)).unwrap();
        assert!(ran.is_none());
        let took = start.elapsed();
        assert!(took < Duration::from_secs(5), "{took:?}");
    }

    /// `lp` taking neither the document nor answering is stopped, and the user told the
    /// job may have been sent.
    #[cfg(unix)]
    #[test]
    fn lp_that_never_takes_the_document_is_stopped() {
        // More than a pipe holds, so writing it waits on the command.
        let data = vec![b'x'; 1 << 20];
        let start = Instant::now();
        let args = ["-c".into(), "sleep 30; true".into()];
        let e = send_within("sh", &args, &data, Duration::from_millis(300)).unwrap_err();
        let took = start.elapsed();
        assert!(took < Duration::from_secs(5), "{took:?}");
        let e = e.to_string();
        let told = e.contains("didn't confirm the job") && e.contains("queue");
        assert!(told, "{e}");
    }

    /// The limit applies to taking the document, and again to answering after it, so a
    /// large job taken slowly isn't cut short.
    #[cfg(unix)]
    #[test]
    fn the_limit_starts_again_once_the_document_is_taken() {
        let data = vec![b'x'; 1 << 20];
        let script = "sleep 0.6; cat > /dev/null; sleep 0.6; echo queued";
        let ran = run_within(&mut sh(script), &data, Duration::from_secs(1)).unwrap();
        let ran = ran.expect("stopped while within the limits");
        assert!(ran.written.is_ok() && ran.out.status.success());
        assert_eq!(String::from_utf8_lossy(&ran.out.stdout), "queued\n");
    }
}

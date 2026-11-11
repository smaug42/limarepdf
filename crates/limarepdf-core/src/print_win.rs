//! Printing on Windows, through the print spooler and GDI.
//!
//! LimarePDF renders each sheet ([`crate::Pdf::print_sheets`]) itself and draws the part of
//! it on the printable area on the printer as a bitmap, the sheet's paper and portrait set
//! in the printer's settings.
//!
//! A printer's paper sizes are read from its driver by a copy of the program, started with
//! [`PAPERS_ARG`] ([`answer_if_asked`]) and stopped after [`ASK_WITHIN`] (C-01). Windows takes
//! no capacity for the lists it writes, so a list that grows between the count and the
//! reading is written past the buffers; the copy's buffers end at an inaccessible page, so a
//! driver that does that, or crashes, ends only the copy, and one that hangs is stopped with
//! it. The program asking checks the copy's answer before using it ([`paper_answer`]).

use crate::paper::{Paper, PaperList};
use crate::paper_answer::{self, fits, Answer, Listed, MAX_ANSWER, MAX_NAME, MAX_PAPERS};
use crate::print::{
    printable_part, sheet_scale, to_bgra, Part, PrintOptions, Printer, Sheet, Sides, ASK_WITHIN,
    PAPERS_ARG,
};
use crate::render::{self, Renderer};
use crate::Result;
use anyhow::{anyhow, bail, Context};
use std::ffi::{c_void, OsStr};
use std::process::Command;
use std::ptr::{null, null_mut};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};
use windows_sys::Win32::Foundation::{SetHandleInformation, HANDLE_FLAG_INHERIT};
use windows_sys::Win32::Graphics::Gdi::{
    CreateDCW, CreateICW, DeleteDC, GetDeviceCaps, SetBrushOrgEx, SetStretchBltMode, StretchDIBits,
    BITMAPINFO, BITMAPINFOHEADER, BI_RGB, DEVMODEW, DIB_RGB_COLORS, DMCOLLATE_TRUE,
    DMDUP_HORIZONTAL, DMDUP_SIMPLEX, DMDUP_VERTICAL, DMORIENT_PORTRAIT, DM_COLLATE, DM_COPIES,
    DM_DUPLEX, DM_IN_BUFFER, DM_ORIENTATION, DM_OUT_BUFFER, DM_PAPERSIZE, HALFTONE, HDC, HORZRES,
    LOGPIXELSX, LOGPIXELSY, PHYSICALHEIGHT, PHYSICALOFFSETX, PHYSICALOFFSETY, PHYSICALWIDTH,
    SRCCOPY, VERTRES,
};
use windows_sys::Win32::Graphics::Printing::{
    ClosePrinter, DocumentPropertiesW, EnumPrintersW, GetDefaultPrinterW, OpenPrinterW,
    PRINTER_ENUM_CONNECTIONS, PRINTER_ENUM_LOCAL, PRINTER_HANDLE, PRINTER_INFO_4W,
};
use windows_sys::Win32::Storage::Xps::{
    AbortDoc, DeviceCapabilitiesW, EndDoc, EndPage, StartDocW, StartPage, DC_PAPERNAMES, DC_PAPERS,
    DC_PAPERSIZE, DOCINFOW, PRINTER_DEVICE_CAPABILITIES,
};
use windows_sys::Win32::System::Diagnostics::Debug::{
    SetErrorMode, SEM_FAILCRITICALERRORS, SEM_NOGPFAULTERRORBOX,
};
use windows_sys::Win32::System::Memory::{
    VirtualAlloc, VirtualFree, VirtualProtect, MEM_COMMIT, MEM_RELEASE, MEM_RESERVE, PAGE_NOACCESS,
    PAGE_READWRITE,
};
use windows_sys::Win32::System::SystemInformation::{GetSystemInfo, SYSTEM_INFO};
use windows_sys::Win32::System::Threading::CREATE_NO_WINDOW;

/// NUL-terminated UTF-16.
fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(Some(0)).collect()
}

/// A NUL-terminated UTF-16 string from Windows.
///
/// # Safety
/// `p` is null or points to a NUL-terminated UTF-16 string.
unsafe fn from_wide(p: *const u16) -> String {
    if p.is_null() {
        return String::new();
    }
    let mut n = 0;
    // SAFETY: the string is NUL-terminated, so every read up to the NUL is in bounds.
    while unsafe { *p.add(n) } != 0 {
        n += 1;
    }
    // SAFETY: the n units before the NUL were just read.
    String::from_utf16_lossy(unsafe { std::slice::from_raw_parts(p, n) })
}

fn os_error(what: &str) -> anyhow::Error {
    anyhow!("{what} failed: {}", std::io::Error::last_os_error())
}

/// A zeroed buffer of at least `bytes` bytes, aligned for the structures Windows writes
/// into it.
fn buffer(bytes: usize) -> Vec<u64> {
    vec![0u64; bytes.div_ceil(8)]
}

pub(crate) fn printers() -> Result<Vec<Printer>> {
    #[cfg(any(test, feature = "test-seams"))]
    if let Some(v) = papers_seam::printers() {
        return Ok(v);
    }
    let flags = PRINTER_ENUM_LOCAL | PRINTER_ENUM_CONNECTIONS;
    let (mut needed, mut count) = (0u32, 0u32);
    // SAFETY: a size query: no buffer, and both counts point to locals.
    unsafe { EnumPrintersW(flags, null(), 4, null_mut(), 0, &mut needed, &mut count) };
    if needed == 0 {
        return Ok(Vec::new());
    }
    let mut buf = buffer(needed as usize);
    // SAFETY: `buf` holds `needed` bytes, aligned for PRINTER_INFO_4W.
    let ok = unsafe {
        EnumPrintersW(
            flags,
            null(),
            4,
            buf.as_mut_ptr().cast(),
            needed,
            &mut needed,
            &mut count,
        )
    };
    if ok == 0 {
        return Err(os_error("listing printers"));
    }
    let first = buf.as_ptr().cast::<PRINTER_INFO_4W>();
    // SAFETY: Windows wrote `count` PRINTER_INFO_4W records at the start of `buf`; their
    // strings point into `buf`, which outlives this slice.
    let infos = unsafe { std::slice::from_raw_parts(first, count as usize) };
    let default = default_printer();
    let mut v: Vec<Printer> = infos
        .iter()
        .map(|i| {
            // SAFETY: pPrinterName is a NUL-terminated string in `buf`.
            let name = unsafe { from_wide(i.pPrinterName) };
            Printer {
                default: default.as_deref() == Some(name.as_str()),
                name,
            }
        })
        .collect();
    v.sort_by_key(|p| !p.default);
    Ok(v)
}

fn default_printer() -> Option<String> {
    let mut n = 0u32;
    // SAFETY: a size query with no buffer.
    unsafe { GetDefaultPrinterW(null_mut(), &mut n) };
    if n == 0 {
        return None;
    }
    let mut buf = vec![0u16; n as usize];
    // SAFETY: `buf` holds `n` UTF-16 units.
    if unsafe { GetDefaultPrinterW(buf.as_mut_ptr(), &mut n) } == 0 {
        return None;
    }
    // SAFETY: Windows wrote a NUL-terminated name into `buf`.
    Some(unsafe { from_wide(buf.as_ptr()) })
}

/// The printer's default settings (a DEVMODEW plus driver data) with the copies, two-sided
/// options and paper applied, held portrait (the sheets are laid out upright), and checked
/// by the driver.
fn settings(name: &[u16], o: &PrintOptions, paper: &Paper) -> Result<Vec<u64>> {
    settings_with(name, |dm| {
        dm.Anonymous1.Anonymous1.dmCopies = o.copies.clamp(1, 999) as i16;
        dm.Anonymous1.Anonymous1.dmOrientation = DMORIENT_PORTRAIT as i16;
        dm.dmFields |= DM_COPIES | DM_ORIENTATION;
        if let Ok(id) = paper.id.parse::<u16>() {
            dm.Anonymous1.Anonymous1.dmPaperSize = id as i16;
            dm.dmFields |= DM_PAPERSIZE;
        }
        dm.dmCollate = DMCOLLATE_TRUE;
        dm.dmDuplex = match o.sides_used() {
            Sides::One => DMDUP_SIMPLEX,
            Sides::LongEdge => DMDUP_VERTICAL,
            Sides::ShortEdge => DMDUP_HORIZONTAL,
        };
        dm.dmFields |= DM_COLLATE | DM_DUPLEX;
    })
}

/// The printer's default settings, changed by `change` and checked by the driver.
fn settings_with(name: &[u16], change: impl FnOnce(&mut DEVMODEW)) -> Result<Vec<u64>> {
    let mut h = PRINTER_HANDLE::default();
    // SAFETY: `name` is NUL-terminated; `h` receives the handle.
    if unsafe { OpenPrinterW(name.as_ptr(), &mut h, null()) } == 0 {
        return Err(os_error("opening the printer"));
    }
    let pname = name.as_ptr();
    let r = (|| {
        // SAFETY: with fmode 0 this returns the size of the settings buffer.
        let size = unsafe { DocumentPropertiesW(null_mut(), h, pname, null_mut(), null(), 0) };
        if size <= 0 {
            return Err(os_error("reading the printer settings"));
        }
        let mut buf = buffer(size as usize);
        let dm = buf.as_mut_ptr().cast::<DEVMODEW>();
        // SAFETY: `buf` holds `size` bytes, aligned for DEVMODEW.
        let n = unsafe { DocumentPropertiesW(null_mut(), h, pname, dm, null(), DM_OUT_BUFFER) };
        if n < 0 {
            return Err(os_error("reading the printer settings"));
        }
        // SAFETY: `dm` points to the DEVMODEW Windows just wrote, which nothing else uses.
        change(unsafe { &mut *dm });
        // Let the driver merge the changes, dropping what the printer cannot do.
        // SAFETY: input and output are the same valid buffer, as the API allows.
        let both = DM_IN_BUFFER | DM_OUT_BUFFER;
        let n = unsafe { DocumentPropertiesW(null_mut(), h, pname, dm, dm, both) };
        if n < 0 {
            return Err(os_error("applying the print settings"));
        }
        Ok(buf)
    })();
    // SAFETY: `h` was opened above.
    unsafe { ClosePrinter(h) };
    r
}

/// How long measuring the printable area of each of a printer's papers may take in all; the
/// papers left are given no margins.
const MEASURE_WITHIN: Duration = Duration::from_secs(3);

/// Whether this program answers [`PAPERS_ARG`]: its `main` called [`answer_if_asked`].
static ANSWERS: AtomicBool = AtomicBool::new(false);

/// [`crate::print::answer_if_asked`] on Windows: in a copy started by [`papers`], writes the
/// printer's paper sizes and exits; otherwise notes that [`papers`] may start copies of this
/// program.
pub(crate) fn answer_if_asked() {
    let mut args = std::env::args_os().skip(1);
    if args.next().as_deref() != Some(OsStr::new(PAPERS_ARG)) {
        ANSWERS.store(true, Ordering::Relaxed);
        return;
    }
    let code = match (args.next().map(|a| a.into_string()), args.next()) {
        (Some(Ok(name)), None) => answer(&name),
        _ => {
            eprintln!("{PAPERS_ARG} takes one printer's name");
            2
        }
    };
    std::process::exit(code);
}

/// The paper sizes printer `name` takes and its default, each with the printable area of a
/// sheet of it held portrait, read by a copy of this program ([`ask`]).
pub(crate) fn papers(name: &str) -> Result<PaperList> {
    if !ANSWERS.load(Ordering::Relaxed) {
        bail!(
            "this program can't read a Windows printer's paper sizes: its `main` doesn't call \
             `print::answer_if_asked`"
        );
    }
    let program = std::env::current_exe().context("finding this program's file")?;
    ask(Command::new(program), name, ASK_WITHIN)
}

/// Starts `program` with [`PAPERS_ARG`] and printer `name`, with no window, and reads the
/// paper sizes it answers with, stopping it after `limit`.
fn ask(mut program: Command, name: &str, limit: Duration) -> Result<PaperList> {
    use std::os::windows::process::CommandExt;
    program
        .args([OsStr::new(PAPERS_ARG), OsStr::new(name)])
        .creation_flags(CREATE_NO_WINDOW);
    let ran = crate::child::run_kept(&mut program, limit, MAX_ANSWER + 1)
        .context("starting a copy of LimarePDF to read the printer's paper sizes")?;
    if ran.timed_out {
        bail!(
            "the printing service didn't answer within {} seconds when asked for the \
             printer's paper sizes",
            limit.as_secs()
        );
    }
    if !ran.status.success() {
        return Err(stopped(&ran));
    }
    let answer = paper_answer::read(&ran.stdout)
        .context("reading the printer's paper sizes gave an answer LimarePDF can't use")?;
    Ok(paper_answer::paper_list(answer))
}

/// Why a copy that exited with a failure gave no paper sizes: the exception that ended it, or
/// what it said.
fn stopped(ran: &crate::child::Ran) -> anyhow::Error {
    let code = ran.status.code().unwrap_or_default() as u32;
    // An exception's code (an NTSTATUS error), as a crash in the driver ends the copy with.
    if code >= 0xC000_0000 {
        return anyhow!(
            "reading the printer's paper sizes stopped unexpectedly (exception 0x{code:08X})"
        );
    }
    let said = String::from_utf8_lossy(&ran.stderr);
    let said: String = (said.trim().chars().take(300))
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect();
    match said.is_empty() {
        true => anyhow!("reading the printer's paper sizes failed (exit code {code})"),
        false => anyhow!("{said}"),
    }
}

/// The copy's side: writes printer `name`'s paper sizes to standard output
/// ([`paper_answer`]). 0 when it did, else 1, the error written to standard error.
fn answer(name: &str) -> i32 {
    use std::io::Write;
    use std::os::windows::io::AsRawHandle;
    // SAFETY: changes only how this process's own failures are reported: a crash in the
    // driver ends the copy with no dialog waiting for someone to close it.
    unsafe { SetErrorMode(SEM_FAILCRITICALERRORS | SEM_NOGPFAULTERRORBOX) };
    // Nothing the driver starts inherits the outputs, so nothing but the copy holds them open.
    for h in [
        std::io::stdout().as_raw_handle(),
        std::io::stderr().as_raw_handle(),
    ] {
        // SAFETY: the handle is this process's standard output or error, open until it exits.
        unsafe { SetHandleInformation(h, HANDLE_FLAG_INHERIT, 0) };
    }
    #[cfg(any(test, feature = "test-seams"))]
    if let Some(code) = papers_seam::copy() {
        return code;
    }
    let written = read_papers(name).and_then(|a| {
        let s = paper_answer::write(&a);
        if s.len() > MAX_ANSWER {
            bail!("the printer's list of paper sizes is too long to pass on");
        }
        let mut out = std::io::stdout().lock();
        out.write_all(s.as_bytes())?;
        out.flush()?;
        Ok(())
    });
    match written {
        Ok(()) => 0,
        Err(e) => {
            eprintln!("{e:#}");
            1
        }
    }
}

/// The copy's side: the paper sizes printer `name` takes, as its driver lists them, and its
/// default, each with the printable area of a sheet of it held portrait ([`margins`]).
fn read_papers(name: &str) -> Result<Answer> {
    let wname = wide(name);
    let failed = || os_error("reading the printer's paper sizes");
    // SAFETY: a size query, with no buffer.
    let count = || unsafe { device_caps(&wname, DC_PAPERS, null_mut()) };
    let asked = count();
    if asked <= 0 {
        return Err(failed());
    }
    let asked = asked as usize;
    if asked > MAX_PAPERS {
        bail!("the printer lists {asked} paper sizes, more than the {MAX_PAPERS} LimarePDF reads");
    }
    // A number, a name of up to 64 UTF-16 units and a size in tenths of a millimetre each.
    // Windows takes no capacity for them: each buffer ends at an inaccessible page, so a
    // driver writing past one ends the copy there and then.
    let mut ids = Guarded::<u16>::new(asked)?;
    let mut names = Guarded::<u16>::new(asked * MAX_NAME)?;
    let mut sizes = Guarded::<i32>::new(asked * 2)?;
    // SAFETY: each buffer holds what the driver writes for `asked` papers.
    let got = unsafe {
        [
            device_caps(&wname, DC_PAPERS, ids.as_mut_ptr()),
            device_caps(&wname, DC_PAPERNAMES, names.as_mut_ptr()),
            device_caps(&wname, DC_PAPERSIZE, sizes.as_mut_ptr().cast()),
        ]
    };
    // A list that changed between the questions is read as far as all three go.
    let n = got
        .iter()
        .map(|&g| g.max(0) as usize)
        .fold(asked, usize::min);
    if n == 0 {
        return Err(failed());
    }
    // One that changed is not shown.
    match count() {
        c if c == asked as i32 => {}
        c if c <= 0 => return Err(failed()),
        _ => bail!("the printer's list of paper sizes changed while it was read"),
    }
    let settings = settings_with(&wname, |_| {})?;
    // SAFETY: `settings` starts with the DEVMODEW the driver wrote.
    let written = unsafe { &*settings.as_ptr().cast::<DEVMODEW>() };
    // SAFETY: a printer's settings hold the printer's half of the union.
    let default = unsafe { written.Anonymous1.Anonymous1.dmPaperSize } as u16;
    let start = Instant::now();
    let mut papers = Vec::new();
    for i in 0..n {
        let (w, h) = (sizes[2 * i], sizes[2 * i + 1]);
        if w <= 0 || h <= 0 {
            continue;
        }
        let size = (paper_answer::points(w), paper_answer::points(h));
        let in_time = start.elapsed() < MEASURE_WITHIN;
        let measured = in_time.then(|| margins(&wname, &settings, ids[i]));
        papers.push(Listed {
            id: ids[i],
            tenths: (w, h),
            // Margins that leave none of the paper aren't given.
            margins: measured.flatten().filter(|&m| fits(size, m)),
            name: paper_answer::name(&names[i * MAX_NAME..(i + 1) * MAX_NAME]),
        });
    }
    let default = papers.iter().any(|p| p.id == default).then_some(default);
    Ok(Answer { papers, default })
}

/// `DeviceCapabilitiesW` for printer `name` with its default settings; the tests' stand-in
/// printer answers it itself ([`papers_seam`]).
///
/// # Safety
/// `out` is null, or holds what the driver writes for `what`.
unsafe fn device_caps(name: &[u16], what: PRINTER_DEVICE_CAPABILITIES, out: *mut u16) -> i32 {
    #[cfg(any(test, feature = "test-seams"))]
    // SAFETY: as the caller promises.
    if let Some(n) = unsafe { papers_seam::device_caps(what, out) } {
        return n;
    }
    // SAFETY: `name` is NUL-terminated; `out` is as the caller promises.
    unsafe { DeviceCapabilitiesW(name.as_ptr(), null(), what, out, null()) }
}

/// Values any bytes are one of, which a [`Guarded`] buffer holds.
trait Plain: Copy {}
impl Plain for u16 {}
impl Plain for i32 {}

/// A buffer of values, zeroed, that ends where an inaccessible page starts: a write past its
/// end faults at once rather than changing other memory.
struct Guarded<T: Plain> {
    /// The pages set aside, the inaccessible one last.
    base: *mut c_void,
    start: *mut T,
    len: usize,
}

impl<T: Plain> Guarded<T> {
    /// A buffer of `len` values.
    fn new(len: usize) -> Result<Guarded<T>> {
        let page = page_size();
        let bytes = len * std::mem::size_of::<T>();
        let data = bytes.div_ceil(page) * page;
        // SAFETY: sets aside new pages, zeroed, readable and writable.
        let base = unsafe {
            VirtualAlloc(
                null(),
                data + page,
                MEM_RESERVE | MEM_COMMIT,
                PAGE_READWRITE,
            )
        };
        if base.is_null() {
            return Err(os_error("setting memory aside"));
        }
        // SAFETY: `data - bytes` is within the pages; a whole page is a whole number of `T`s,
        // so the values are aligned.
        let start = unsafe { base.cast::<u8>().add(data - bytes) }.cast::<T>();
        let guarded = Guarded { base, start, len };
        let mut was = 0;
        // SAFETY: the last page is one of those just set aside.
        let last = unsafe { base.cast::<u8>().add(data) }.cast::<c_void>();
        // SAFETY: changes only the last page's access, which nothing uses.
        if unsafe { VirtualProtect(last, page, PAGE_NOACCESS, &mut was) } == 0 {
            return Err(os_error("setting memory aside"));
        }
        Ok(guarded)
    }
}

impl<T: Plain> std::ops::Deref for Guarded<T> {
    type Target = [T];
    fn deref(&self) -> &[T] {
        // SAFETY: `start` is followed by `len` values in pages this owns, zeroed when set
        // aside, and any bytes are a value.
        unsafe { std::slice::from_raw_parts(self.start, self.len) }
    }
}

impl<T: Plain> std::ops::DerefMut for Guarded<T> {
    fn deref_mut(&mut self) -> &mut [T] {
        // SAFETY: as for `deref`, and `&mut self` makes this the only use of them.
        unsafe { std::slice::from_raw_parts_mut(self.start, self.len) }
    }
}

impl<T: Plain> Drop for Guarded<T> {
    fn drop(&mut self) {
        // SAFETY: the pages were set aside by VirtualAlloc, and are given back once.
        unsafe { VirtualFree(self.base, 0, MEM_RELEASE) };
    }
}

/// The size of a page of memory.
fn page_size() -> usize {
    let mut info = SYSTEM_INFO::default();
    // SAFETY: fills in `info`.
    unsafe { GetSystemInfo(&mut info) };
    info.dwPageSize as usize
}

/// How near each edge a sheet of paper number `id` held portrait can't be printed on (top,
/// right, bottom and left, in points), measured on an information context for it.
fn margins(name: &[u16], settings: &[u64], id: u16) -> Option<[f32; 4]> {
    let mut dm = settings.to_vec();
    let p = dm.as_mut_ptr().cast::<DEVMODEW>();
    // SAFETY: `dm` is a copy of the printer's settings, which start with a DEVMODEW.
    let fields = unsafe { &mut *p };
    fields.Anonymous1.Anonymous1.dmPaperSize = id as i16;
    fields.Anonymous1.Anonymous1.dmOrientation = DMORIENT_PORTRAIT as i16;
    fields.dmFields |= DM_PAPERSIZE | DM_ORIENTATION;
    let driver = wide("WINSPOOL");
    // SAFETY: the strings are NUL-terminated and `dm` holds the printer's settings.
    let ic = unsafe { CreateICW(driver.as_ptr(), name.as_ptr(), null(), p.cast_const()) };
    if ic.is_null() {
        return None;
    }
    let ic = Dc(ic);
    // SAFETY: GetDeviceCaps only reads from a valid information context.
    let caps = |c: u32| unsafe { GetDeviceCaps(ic.0, c as i32) } as f32;
    let (dx, dy) = (caps(LOGPIXELSX), caps(LOGPIXELSY));
    if dx <= 0.0 || dy <= 0.0 {
        return None;
    }
    let (left, top) = (caps(PHYSICALOFFSETX), caps(PHYSICALOFFSETY));
    let right = caps(PHYSICALWIDTH) - left - caps(HORZRES);
    let bottom = caps(PHYSICALHEIGHT) - top - caps(VERTRES);
    let pt = |px: f32, dpi: f32| (px / dpi * 72.0).max(0.0);
    Some([pt(top, dy), pt(right, dx), pt(bottom, dy), pt(left, dx)])
}

/// A printer device or information context, deleted when dropped.
struct Dc(HDC);

impl Drop for Dc {
    fn drop(&mut self) {
        // SAFETY: the context was created by CreateDCW or CreateICW and is deleted once.
        unsafe { DeleteDC(self.0) };
    }
}

/// Prints the `count` sheets of `sheets` ([`crate::Pdf::print_sheets`], written out), laid
/// out on `paper`, calling `beat` after each step the spooler takes, so the wait for it can
/// tell a job getting on from a stuck one ([`crate::print::while_beating`]).
pub(crate) fn print(
    sheets: Vec<u8>,
    count: usize,
    paper: &Paper,
    o: &PrintOptions,
    beat: &dyn Fn(),
) -> Result<String> {
    let name = match &o.printer {
        Some(n) => n.clone(),
        None => default_printer().ok_or_else(|| anyhow!("no default printer is set"))?,
    };
    let wname = wide(&name);
    let dm = settings(&wname, o, paper)?;
    beat();
    let hp = render::load(sheets, None)?;
    let r = Renderer::new(&hp);
    for i in 0..count {
        r.page_size(i)?;
    }
    let driver = wide("WINSPOOL");
    // SAFETY: the strings are NUL-terminated and `dm` holds the driver's settings.
    let dc = unsafe { CreateDCW(driver.as_ptr(), wname.as_ptr(), null(), dm.as_ptr().cast()) };
    if dc.is_null() {
        return Err(os_error("opening the printer"));
    }
    beat();
    let dc = Dc(dc);
    let title = wide(if o.title.is_empty() {
        "LimarePDF"
    } else {
        &o.title
    });
    let info = DOCINFOW {
        cbSize: std::mem::size_of::<DOCINFOW>() as i32,
        lpszDocName: title.as_ptr(),
        ..Default::default()
    };
    // SAFETY: `info` and the title it points to live until EndDoc.
    let job = unsafe { StartDocW(dc.0, &info) };
    if job <= 0 {
        return Err(os_error("starting the print job"));
    }
    beat();
    for i in 0..count {
        if let Err(e) = page(&dc, &r, i) {
            // SAFETY: a document was started on this DC.
            unsafe { AbortDoc(dc.0) };
            return Err(e.context(format!("sheet {}", i + 1)));
        }
        beat();
    }
    // SAFETY: a document was started on this DC.
    if unsafe { EndDoc(dc.0) } <= 0 {
        return Err(os_error("finishing the print job"));
    }
    Ok(format!("Print job {job}."))
}

/// Render sheet `i` and draw its part on the printable area on a new page of the printer.
fn page(dc: &Dc, r: &Renderer, i: usize) -> Result<()> {
    // SAFETY: GetDeviceCaps only reads from a valid DC.
    let caps = |c: u32| unsafe { GetDeviceCaps(dc.0, c as i32) };
    let sheet = Sheet {
        area: (caps(HORZRES), caps(VERTRES)),
        offset: (caps(PHYSICALOFFSETX), caps(PHYSICALOFFSETY)),
        dpi: (caps(LOGPIXELSX), caps(LOGPIXELSY)),
    };
    let size = r.page_size(i)?;
    let b = r.render(i, sheet_scale(size, &sheet))?;
    let part = printable_part(size, (b.width, b.height), &sheet);
    // SAFETY: a document is open on this DC.
    if unsafe { StartPage(dc.0) } <= 0 {
        return Err(os_error("starting a page"));
    }
    if let Some(part) = part {
        draw(dc, &b, part)?;
    }
    // SAFETY: a page was started on this DC.
    if unsafe { EndPage(dc.0) } <= 0 {
        return Err(os_error("finishing a page"));
    }
    Ok(())
}

/// Draw `part` of the sheet's picture `b` where it goes on the printable area.
fn draw(dc: &Dc, b: &render::Bitmap, part: Part) -> Result<()> {
    let [_, _, w, h] = part.from;
    let bgra = to_bgra(&b.rgba, b.width, part.from);
    let [x, y, dw, dh] = part.to;
    let bmi = BITMAPINFO {
        bmiHeader: BITMAPINFOHEADER {
            biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: w as i32,
            // Negative: rows run top to bottom.
            biHeight: -(h as i32),
            biPlanes: 1,
            biBitCount: 32,
            biCompression: BI_RGB,
            ..Default::default()
        },
        ..Default::default()
    };
    // SAFETY: `bgra` holds w x h 32-bit pixels as `bmi` describes, and outlives the call.
    let drawn = unsafe {
        SetStretchBltMode(dc.0, HALFTONE);
        SetBrushOrgEx(dc.0, 0, 0, null_mut());
        StretchDIBits(
            dc.0,
            x,
            y,
            dw,
            dh,
            0,
            0,
            w as i32,
            h as i32,
            bgra.as_ptr().cast(),
            &bmi,
            DIB_RGB_COLORS,
            SRCCOPY,
        )
    };
    if drawn == 0 {
        return Err(os_error("drawing the page"));
    }
    Ok(())
}

/// A test seam (C-01): with [`VAR`](papers_seam::VAR) set, the printers listed are a stand-in
/// alone, whose driver, or the copy reading its paper sizes, does as the variable says. Built
/// only for tests: core's own, and the app's and the CLI's through the `test-seams` feature.
#[cfg(any(test, feature = "test-seams"))]
pub mod papers_seam {
    use super::{Answer, Listed, PaperList, Printer, Result, MAX_ANSWER, MAX_NAME};
    use std::process::Command;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::Duration;
    use windows_sys::Win32::Storage::Xps::{
        DC_PAPERNAMES, DC_PAPERS, DC_PAPERSIZE, PRINTER_DEVICE_CAPABILITIES,
    };

    /// What the stand-in does, for a program a test runs:
    /// - `grows`: its driver says it has 2 papers, then writes 5 of each, as a list that grew
    ///   between the questions would be written;
    /// - `changes`: its driver says 2 papers, then 3 when asked again;
    /// - `hangs:<file>`: the copy holds `<file>` open, unshared, and never answers;
    /// - `floods`: the copy answers with more than an answer may be;
    /// - `garbles`: the copy's answer has a paper with no width;
    /// - `cuts`: the copy's answer stops before its end;
    /// - `lists`: the copy answers with Letter and A4, A4 the default.
    pub const VAR: &str = "LIMAREPDF_TEST_PAPERS";

    /// The stand-in printer's name.
    pub const PRINTER: &str = "LimarePDF stand-in";

    /// The paper sizes `printer` takes, read by `program` (with the environment the test
    /// gives it), started as the app and the CLI start a copy of themselves and stopped after
    /// `limit`.
    pub fn through(program: Command, printer: &str, limit: Duration) -> Result<PaperList> {
        super::ask(program, printer, limit)
    }

    /// The paper sizes `printer` takes, read in this process as a copy reads them.
    pub fn in_process(printer: &str) -> Result<PaperList> {
        let read = super::read_papers(printer)?;
        Ok(super::paper_answer::paper_list(read))
    }

    /// The stand-in printer alone, when [`VAR`] is set.
    pub(super) fn printers() -> Option<Vec<Printer>> {
        std::env::var_os(VAR)?;
        Some(vec![Printer {
            name: PRINTER.to_string(),
            default: true,
        }])
    }

    /// The stand-in driver's answer, when [`VAR`] names something it does.
    ///
    /// # Safety
    /// As [`super::device_caps`].
    pub(super) unsafe fn device_caps(
        what: PRINTER_DEVICE_CAPABILITIES,
        out: *mut u16,
    ) -> Option<i32> {
        static COUNTED: AtomicUsize = AtomicUsize::new(0);
        let (said, writes) = match std::env::var(VAR).ok()?.as_str() {
            "grows" => (2, 5),
            "changes" if out.is_null() && COUNTED.fetch_add(1, Ordering::Relaxed) > 0 => (3, 2),
            "changes" => (2, 2),
            _ => return None,
        };
        if out.is_null() {
            return Some(said);
        }
        let sizes = out.cast::<i32>();
        for i in 0..writes {
            let name: Vec<u16> = format!("Stand-in {i}").encode_utf16().collect();
            // As a driver writes, through the pointer it was given, and volatile so each write
            // is made as here.
            // SAFETY: those within `said` are within the buffer the caller holds; the rest
            // are the overrun the stand-in is for, and fault on the inaccessible page at the
            // buffer's end before reaching any other memory.
            unsafe {
                match what {
                    DC_PAPERS => out.wrapping_add(i).write_volatile(i as u16 + 1),
                    DC_PAPERNAMES => {
                        let slot = out.wrapping_add(i * MAX_NAME);
                        for (k, c) in name.iter().chain(&[0u16]).enumerate() {
                            slot.wrapping_add(k).write_volatile(*c);
                        }
                    }
                    DC_PAPERSIZE => {
                        sizes.wrapping_add(2 * i).write_volatile(2100);
                        sizes.wrapping_add(2 * i + 1).write_volatile(2970);
                    }
                    _ => return Some(-1),
                }
            }
        }
        Some(writes as i32)
    }

    /// The stand-in copy's exit code, when [`VAR`] names something it does: it has answered.
    pub(super) fn copy() -> Option<i32> {
        use std::io::Write;
        let how = std::env::var(VAR).ok()?;
        if let Some(file) = how.strip_prefix("hangs:") {
            use std::os::windows::fs::OpenOptionsExt;
            let mut open = std::fs::OpenOptions::new();
            let _held = open.write(true).create(true).share_mode(0).open(file);
            std::thread::sleep(Duration::from_secs(600));
            return Some(1);
        }
        let paper = |id, tenths, margins, name: &str| Listed {
            id,
            tenths,
            margins,
            name: name.to_string(),
        };
        let two = super::paper_answer::write(&Answer {
            papers: vec![
                paper(1, (2159, 2794), Some([18.0, 12.0, 18.0, 12.0]), "Letter"),
                paper(9, (2100, 2970), None, "A4"),
            ],
            default: Some(9),
        });
        let answer = match how.as_str() {
            "floods" => two + &" ".repeat(MAX_ANSWER),
            "garbles" => two.replace("\t2100\t", "\t0\t"),
            "cuts" => two.replace("end\t2\n", ""),
            "lists" => two,
            _ => return None,
        };
        let mut out = std::io::stdout().lock();
        let _ = out.write_all(answer.as_bytes()).and_then(|()| out.flush());
        Some(0)
    }
}

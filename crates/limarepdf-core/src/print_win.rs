//! Printing on Windows, through the print spooler and GDI.
//!
//! LimarePDF renders each sheet ([`crate::Pdf::print_sheets`]) itself and draws the part of
//! it on the printable area on the printer as a bitmap, the sheet's paper and portrait set
//! in the printer's settings.

use crate::paper::{dims, listed, Paper, PaperList};
use crate::print::{
    printable_part, sheet_scale, to_bgra, Part, PrintOptions, Printer, Sheet, Sides,
};
use crate::render::{self, Renderer};
use crate::Result;
use anyhow::anyhow;
use std::ptr::{null, null_mut};
use std::time::{Duration, Instant};
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
    DC_PAPERSIZE, DOCINFOW,
};

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
        dm.dmDuplex = match o.sides {
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

/// The paper sizes printer `name` takes, as its driver lists them, and its default, each
/// with the printable area of a sheet of it held portrait ([`margins`]).
pub(crate) fn papers(name: &str) -> Result<PaperList> {
    let wname = wide(name);
    let failed = || os_error("reading the printer's paper sizes");
    // SAFETY: a size query, with no buffer.
    let n = unsafe { DeviceCapabilitiesW(wname.as_ptr(), null(), DC_PAPERS, null_mut(), null()) };
    if n <= 0 {
        return Err(failed());
    }
    let n = n as usize;
    // A number, a name of up to 64 UTF-16 units and a size in tenths of a millimetre each.
    let mut ids = vec![0u16; n];
    let mut names = vec![0u16; n * 64];
    let mut sizes = vec![0i32; n * 2];
    // SAFETY: each buffer holds what the driver writes for `n` papers.
    let got = unsafe {
        let ask =
            |what, out: *mut u16| DeviceCapabilitiesW(wname.as_ptr(), null(), what, out, null());
        [
            ask(DC_PAPERS, ids.as_mut_ptr()),
            ask(DC_PAPERNAMES, names.as_mut_ptr()),
            ask(DC_PAPERSIZE, sizes.as_mut_ptr().cast()),
        ]
    };
    // A list that changed between the questions is read as far as all three go.
    let n = got.iter().map(|&g| g.max(0) as usize).fold(n, usize::min);
    if n == 0 {
        return Err(failed());
    }
    let settings = settings_with(&wname, |_| {})?;
    // SAFETY: `settings` starts with the DEVMODEW the driver wrote.
    let written = unsafe { &*settings.as_ptr().cast::<DEVMODEW>() };
    // SAFETY: a printer's settings hold the printer's half of the union.
    let default = unsafe { written.Anonymous1.Anonymous1.dmPaperSize };
    let start = Instant::now();
    let mut found = Vec::new();
    for i in 0..n {
        let (w, h) = (sizes[2 * i], sizes[2 * i + 1]);
        if w <= 0 || h <= 0 {
            continue;
        }
        let slot = &names[i * 64..(i + 1) * 64];
        let end = slot.iter().position(|&c| c == 0).unwrap_or(64);
        let label = String::from_utf16_lossy(&slot[..end]).trim().to_string();
        // Tenths of a millimetre.
        let size = (w as f32 / 254.0 * 72.0, h as f32 / 254.0 * 72.0);
        let label = match label.is_empty() {
            true => dims(size.0, size.1),
            false => format!("{label} ({})", dims(size.0, size.1)),
        };
        let in_time = start.elapsed() < MEASURE_WITHIN;
        let measured = in_time.then(|| margins(&wname, &settings, ids[i]));
        found.push(Paper {
            id: ids[i].to_string(),
            label,
            size,
            margins: measured.flatten(),
        });
    }
    Ok(listed(found, Some(&(default as u16).to_string())))
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

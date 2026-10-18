//! Printing on Windows, through the print spooler and GDI.
//!
//! LimarePDF renders each page itself and draws it on the printer as a bitmap, placed by
//! [`crate::print::place`]: at actual size when it fits the printable area, shrunk to fit
//! when not, turned to match the paper's orientation.

use crate::print::{place, to_bgra, PrintOptions, Printer, Sheet, Sides};
use crate::render::{self, Renderer};
use crate::{Pdf, Result};
use anyhow::anyhow;
use std::ptr::{null, null_mut};
use windows_sys::Win32::Graphics::Gdi::{
    CreateDCW, DeleteDC, GetDeviceCaps, SetBrushOrgEx, SetStretchBltMode, StretchDIBits,
    BITMAPINFO, BITMAPINFOHEADER, BI_RGB, DEVMODEW, DIB_RGB_COLORS, DMCOLLATE_TRUE,
    DMDUP_HORIZONTAL, DMDUP_SIMPLEX, DMDUP_VERTICAL, DM_COLLATE, DM_COPIES, DM_DUPLEX,
    DM_IN_BUFFER, DM_OUT_BUFFER, HALFTONE, HDC, HORZRES, LOGPIXELSX, LOGPIXELSY, PHYSICALHEIGHT,
    PHYSICALOFFSETX, PHYSICALOFFSETY, PHYSICALWIDTH, SRCCOPY, VERTRES,
};
use windows_sys::Win32::Graphics::Printing::{
    ClosePrinter, DocumentPropertiesW, EnumPrintersW, GetDefaultPrinterW, OpenPrinterW,
    PRINTER_ENUM_CONNECTIONS, PRINTER_ENUM_LOCAL, PRINTER_HANDLE, PRINTER_INFO_4W,
};
use windows_sys::Win32::Storage::Xps::{AbortDoc, EndDoc, EndPage, StartDocW, StartPage, DOCINFOW};

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

/// The printer's default settings (a DEVMODEW plus driver data) with the copies and
/// two-sided options applied and checked by the driver.
fn settings(name: &[u16], o: &PrintOptions) -> Result<Vec<u64>> {
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
        // SAFETY: `dm` points to the DEVMODEW Windows just wrote.
        unsafe {
            (*dm).Anonymous1.Anonymous1.dmCopies = o.copies.clamp(1, 999) as i16;
            (*dm).dmCollate = DMCOLLATE_TRUE;
            (*dm).dmDuplex = match o.sides {
                Sides::One => DMDUP_SIMPLEX,
                Sides::LongEdge => DMDUP_VERTICAL,
                Sides::ShortEdge => DMDUP_HORIZONTAL,
            };
            (*dm).dmFields |= DM_COPIES | DM_COLLATE | DM_DUPLEX;
        }
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

/// A printer device context, deleted when dropped.
struct Dc(HDC);

impl Drop for Dc {
    fn drop(&mut self) {
        // SAFETY: the DC was created by CreateDCW and is deleted once.
        unsafe { DeleteDC(self.0) };
    }
}

pub(crate) fn print(pdf: &mut Pdf, pages: &[usize], o: &PrintOptions) -> Result<String> {
    let name = match &o.printer {
        Some(n) => n.clone(),
        None => default_printer().ok_or_else(|| anyhow!("no default printer is set"))?,
    };
    let wname = wide(&name);
    let dm = settings(&wname, o)?;
    let hp = render::load(pdf.print_snapshot()?, None)?;
    let r = Renderer::new(&hp);
    for &i in pages {
        r.page_size(i)?;
    }
    let driver = wide("WINSPOOL");
    // SAFETY: the strings are NUL-terminated and `dm` holds the driver's settings.
    let dc = unsafe { CreateDCW(driver.as_ptr(), wname.as_ptr(), null(), dm.as_ptr().cast()) };
    if dc.is_null() {
        return Err(os_error("opening the printer"));
    }
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
    for &i in pages {
        if let Err(e) = page(&dc, &r, i) {
            // SAFETY: a document was started on this DC.
            unsafe { AbortDoc(dc.0) };
            return Err(e.context(format!("page {}", i + 1)));
        }
    }
    // SAFETY: a document was started on this DC.
    if unsafe { EndDoc(dc.0) } <= 0 {
        return Err(os_error("finishing the print job"));
    }
    Ok(format!("Print job {job}."))
}

/// Render page `i` and draw it on a new sheet.
fn page(dc: &Dc, r: &Renderer, i: usize) -> Result<()> {
    // SAFETY: GetDeviceCaps only reads from a valid DC.
    let caps = |c: u32| unsafe { GetDeviceCaps(dc.0, c as i32) };
    let sheet = Sheet {
        area: (caps(HORZRES), caps(VERTRES)),
        paper: (caps(PHYSICALWIDTH), caps(PHYSICALHEIGHT)),
        offset: (caps(PHYSICALOFFSETX), caps(PHYSICALOFFSETY)),
        dpi: (caps(LOGPIXELSX), caps(LOGPIXELSY)),
    };
    let at = place(r.page_size(i)?, &sheet);
    let b = r.render(i, at.scale)?;
    let (bgra, w, h) = to_bgra(&b.rgba, b.width, b.height, at.rotate);
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
    // SAFETY: a document is open on this DC.
    if unsafe { StartPage(dc.0) } <= 0 {
        return Err(os_error("starting a page"));
    }
    // SAFETY: `bgra` holds w x h 32-bit pixels as `bmi` describes, and outlives the call.
    let drawn = unsafe {
        SetStretchBltMode(dc.0, HALFTONE);
        SetBrushOrgEx(dc.0, 0, 0, null_mut());
        StretchDIBits(
            dc.0,
            at.x,
            at.y,
            at.w,
            at.h,
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
    // SAFETY: a page was started on this DC.
    if unsafe { EndPage(dc.0) } <= 0 {
        return Err(os_error("finishing a page"));
    }
    Ok(())
}

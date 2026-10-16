//! Page overlays: watermarks, headers/footers, page numbers and Bates numbering.
//! All text uses the PDF standard Helvetica font (WinAnsi, Latin-1 range).

use crate::imgs::orient_matrix;
use crate::{std14, Pdf, Result};
use anyhow::bail;
use lopdf::dictionary;
use lopdf::{Dictionary, Object, ObjectId, Stream};

/// Width in points of `s` in Helvetica at `size`.
pub fn text_width(s: &str, size: f32) -> f32 {
    s.chars()
        .map(|c| std14::width("Helvetica", c).unwrap_or(556.0))
        .sum::<f32>()
        * size
        / 1000.0
}

fn escape(s: &str) -> String {
    let mut o = String::new();
    for c in s.chars() {
        let u = c as u32;
        match c {
            '(' | ')' | '\\' => {
                o.push('\\');
                o.push(c);
            }
            _ if (32..=126).contains(&u) => o.push(c),
            _ if (0xA0..=0xFF).contains(&u) => o.push_str(&format!("\\{:03o}", u)),
            _ => o.push('?'),
        }
    }
    o
}

#[derive(Clone, Debug)]
pub struct Watermark {
    pub text: String,
    pub font_size: f32,
    /// 0.0 (invisible) to 1.0 (opaque)
    pub opacity: f32,
    /// Counter-clockwise rotation in degrees.
    pub rotation: f32,
    pub color: (f32, f32, f32),
    /// Draw above the page content (true) or behind it (false).
    pub foreground: bool,
}

impl Default for Watermark {
    fn default() -> Self {
        Self {
            text: "DRAFT".into(),
            font_size: 96.0,
            opacity: 0.25,
            rotation: 45.0,
            color: (0.6, 0.0, 0.0),
            foreground: true,
        }
    }
}

/// A picture placed in the middle of each page, such as a logo.
#[derive(Clone, Debug)]
pub struct ImageWatermark {
    /// PNG, JPEG or TIFF file contents (the first image of a TIFF is used).
    pub image: Vec<u8>,
    /// Size as a fraction of the page: the picture fits inside a box this fraction of
    /// the page's width and height, keeping its proportions.
    pub scale: f32,
    /// 0.0 (invisible) to 1.0 (opaque)
    pub opacity: f32,
    /// Counter-clockwise rotation in degrees.
    pub rotation: f32,
    /// Draw above the page content (true) or behind it (false).
    pub foreground: bool,
}

impl ImageWatermark {
    /// Half the page, a quarter opaque, upright, above the content.
    pub fn new(image: Vec<u8>) -> Self {
        Self {
            image,
            scale: 0.5,
            opacity: 0.25,
            rotation: 0.0,
            foreground: true,
        }
    }
}

fn matrix(m: [f32; 6]) -> String {
    format!(
        "{:.5} {:.5} {:.5} {:.5} {:.3} {:.3} cm",
        m[0], m[1], m[2], m[3], m[4], m[5]
    )
}

#[derive(Clone, Debug)]
pub struct Bates {
    pub prefix: String,
    pub suffix: String,
    pub digits: usize,
    pub start: u64,
}

impl Default for Bates {
    fn default() -> Self {
        Self {
            prefix: String::new(),
            suffix: String::new(),
            digits: 6,
            start: 1,
        }
    }
}

/// Header and footer text in left/centre/right slots.
/// Tokens: `{page}`, `{pages}`, `{date}`, `{bates}`.
#[derive(Clone, Debug)]
pub struct HeaderFooter {
    pub header: [String; 3],
    pub footer: [String; 3],
    pub font_size: f32,
    pub margin: f32,
    pub color: (f32, f32, f32),
    pub bates: Option<Bates>,
}

impl Default for HeaderFooter {
    fn default() -> Self {
        Self {
            header: Default::default(),
            footer: ["".into(), "Page {page} of {pages}".into(), "".into()],
            font_size: 10.0,
            margin: 36.0,
            color: (0.0, 0.0, 0.0),
            bates: None,
        }
    }
}

pub(crate) const FONT_NAME: &str = "MPEF1";

impl Pdf {
    pub(crate) fn make_font(&mut self) -> ObjectId {
        self.doc.add_object(dictionary! {
            "Type" => "Font", "Subtype" => "Type1", "BaseFont" => "Helvetica", "Encoding" => "WinAnsiEncoding",
        })
    }

    fn make_gs(&mut self, alpha: f32) -> ObjectId {
        let a = alpha.clamp(0.0, 1.0);
        self.doc.add_object(dictionary! {
            "Type" => "ExtGState", "ca" => a, "CA" => a,
        })
    }

    /// Matrix mapping a bottom-left, y-up "visual page" space to page user space,
    /// plus the visual width and height.
    pub(crate) fn visual_matrix(&self, page: ObjectId) -> ([f32; 6], f32, f32) {
        let [x0, y0, x1, y1] = self.page_box(page);
        let (w, h) = (x1 - x0, y1 - y0);
        match self.page_rotation(page) {
            90 => ([0.0, 1.0, -1.0, 0.0, x0 + w, y0], h, w),
            180 => ([-1.0, 0.0, 0.0, -1.0, x0 + w, y0 + h], w, h),
            270 => ([0.0, -1.0, 1.0, 0.0, x0, y0 + h], h, w),
            _ => ([1.0, 0.0, 0.0, 1.0, x0, y0], w, h),
        }
    }

    fn page_content_refs(&self, page: ObjectId) -> Vec<Object> {
        let Ok(d) = self.doc.get_dictionary(page) else {
            return vec![];
        };
        match d.get(b"Contents") {
            Ok(Object::Reference(id)) => match self.doc.get_object(*id) {
                Ok(Object::Array(a)) => a.clone(),
                _ => vec![Object::Reference(*id)],
            },
            Ok(Object::Array(a)) => a.clone(),
            _ => vec![],
        }
    }

    pub(crate) fn resolved_dict(&self, o: Option<&Object>) -> Dictionary {
        o.and_then(|o| self.doc.dereference(o).ok())
            .and_then(|(_, o)| o.as_dict().ok())
            .cloned()
            .unwrap_or_default()
    }

    /// Wrap the page's existing content in q/Q, add `below` and `above` streams,
    /// and register the font in the page resources as [`FONT_NAME`].
    pub(crate) fn add_layers(
        &mut self,
        page: ObjectId,
        below: Option<Vec<u8>>,
        above: Option<Vec<u8>>,
        font: ObjectId,
    ) -> Result<()> {
        self.wrap_contents(page, below, above)?;
        let mut res = self.resolved_dict(self.inherited(page, b"Resources").as_ref());
        let mut fonts = self.resolved_dict(res.get(b"Font").ok());
        fonts.set(FONT_NAME, Object::Reference(font));
        res.set("Font", fonts);
        self.doc.get_dictionary_mut(page)?.set("Resources", res);
        Ok(())
    }

    /// Wrap the page's existing content in q/Q, so that it leaves the graphics state as
    /// it found it, and add `below` and `above` streams.
    pub(crate) fn wrap_contents(
        &mut self,
        page: ObjectId,
        below: Option<Vec<u8>>,
        above: Option<Vec<u8>>,
    ) -> Result<()> {
        let existing = self.page_content_refs(page);
        let mut contents: Vec<Object> = Vec::with_capacity(existing.len() + 4);
        if let Some(b) = below {
            contents.push(
                self.doc
                    .add_object(Stream::new(Dictionary::new(), b))
                    .into(),
            );
        }
        contents.push(
            self.doc
                .add_object(Stream::new(Dictionary::new(), b"q\n".to_vec()))
                .into(),
        );
        contents.extend(existing);
        contents.push(
            self.doc
                .add_object(Stream::new(Dictionary::new(), b"\nQ\n".to_vec()))
                .into(),
        );
        if let Some(a) = above {
            contents.push(
                self.doc
                    .add_object(Stream::new(Dictionary::new(), a))
                    .into(),
            );
        }
        self.doc.get_dictionary_mut(page)?.set("Contents", contents);
        Ok(())
    }

    /// Apply a text watermark to the given pages (0-based).
    pub fn add_watermark(&mut self, idx: &[usize], wm: &Watermark) -> Result<()> {
        if wm.text.trim().is_empty() {
            bail!("watermark text is empty");
        }
        let ids = self.page_ids();
        if let Some(&bad) = idx.iter().find(|&&i| i >= ids.len()) {
            bail!("page {} does not exist", bad + 1);
        }
        let font = self.make_font();
        let gs = self.make_gs(wm.opacity);
        for &i in idx {
            let page = ids[i];
            let (m, vw, vh) = self.visual_matrix(page);
            let (s, c) = wm.rotation.to_radians().sin_cos();
            let tw = text_width(&wm.text, wm.font_size);
            let font_nm = self.add_resource(page, "Font", "MPEWf", font.into())?;
            let gs_nm = self.add_resource(page, "ExtGState", "MPEGS", gs.into())?;
            let stream = format!(
                "q\n{}\n/{gs_nm} gs\n{:.3} {:.3} {:.3} rg\n{}\n\
                 BT /{font_nm} {:.2} Tf {:.3} {:.3} Td ({}) Tj ET\nQ\n",
                matrix(m),
                wm.color.0,
                wm.color.1,
                wm.color.2,
                matrix([c, s, -s, c, vw / 2.0, vh / 2.0]),
                wm.font_size,
                -tw / 2.0,
                -wm.font_size * 0.35,
                escape(&wm.text),
            );
            self.add_overlay(page, stream, wm.foreground)?;
        }
        Ok(())
    }

    /// Apply a picture watermark, centred, to the given pages (0-based). The image is
    /// stored once and shared by every page.
    pub fn add_image_watermark(&mut self, idx: &[usize], wm: &ImageWatermark) -> Result<()> {
        if !(wm.scale > 0.0 && wm.scale <= 1.0) {
            bail!("watermark size must be above 0% and at most 100% of the page");
        }
        let ids = self.page_ids();
        if let Some(&bad) = idx.iter().find(|&&i| i >= ids.len()) {
            bail!("page {} does not exist", bad + 1);
        }
        let (img, iw, ih, o) = self.add_upright_image(&wm.image)?;
        let gs = self.make_gs(wm.opacity);
        for &i in idx {
            let page = ids[i];
            let (m, vw, vh) = self.visual_matrix(page);
            let k = (wm.scale * vw / iw).min(wm.scale * vh / ih);
            let (pw, ph) = (iw * k, ih * k);
            let (s, c) = wm.rotation.to_radians().sin_cos();
            let img_nm = self.add_resource(page, "XObject", "MPEWi", img.into())?;
            let gs_nm = self.add_resource(page, "ExtGState", "MPEGS", gs.into())?;
            let stream = format!(
                "q\n{}\n/{gs_nm} gs\n{}\n{}\n{}\n/{img_nm} Do\nQ\n",
                matrix(m),
                matrix([c, s, -s, c, vw / 2.0, vh / 2.0]),
                matrix([1.0, 0.0, 0.0, 1.0, -pw / 2.0, -ph / 2.0]),
                matrix(orient_matrix(o, pw, ph)),
            );
            self.add_overlay(page, stream, wm.foreground)?;
        }
        Ok(())
    }

    /// Add a self-contained stream above or below the page's (wrapped) content.
    fn add_overlay(&mut self, page: ObjectId, stream: String, foreground: bool) -> Result<()> {
        let s = Some(stream.into_bytes());
        if foreground {
            self.wrap_contents(page, None, s)
        } else {
            self.wrap_contents(page, s, None)
        }
    }

    /// Apply headers, footers, page numbers and Bates numbers to the given pages (0-based).
    pub fn add_header_footer(&mut self, idx: &[usize], hf: &HeaderFooter) -> Result<()> {
        let ids = self.page_ids();
        let total = ids.len();
        if let Some(&bad) = idx.iter().find(|&&i| i >= total) {
            bail!("page {} does not exist", bad + 1);
        }
        let font = self.make_font();
        let date = today_iso();
        for (seq, &i) in idx.iter().enumerate() {
            let page = ids[i];
            let (m, vw, vh) = self.visual_matrix(page);
            let bates = hf.bates.as_ref().map(|b| {
                format!(
                    "{}{:0w$}{}",
                    b.prefix,
                    b.start + seq as u64,
                    b.suffix,
                    w = b.digits
                )
            });
            let expand = |t: &str| {
                t.replace("{page}", &(i + 1).to_string())
                    .replace("{pages}", &total.to_string())
                    .replace("{date}", &date)
                    .replace("{bates}", bates.as_deref().unwrap_or(""))
            };
            let mut s = format!(
                "q\n{:.5} {:.5} {:.5} {:.5} {:.3} {:.3} cm\n{:.3} {:.3} {:.3} rg\nBT /{FONT_NAME} {:.2} Tf\n",
                m[0], m[1], m[2], m[3], m[4], m[5], hf.color.0, hf.color.1, hf.color.2, hf.font_size
            );
            let mut put = |text: &str, slot: usize, y: f32| {
                if text.is_empty() {
                    return;
                }
                let t = expand(text);
                let tw = text_width(&t, hf.font_size);
                let x = match slot {
                    0 => hf.margin,
                    1 => (vw - tw) / 2.0,
                    _ => vw - hf.margin - tw,
                };
                s.push_str(&format!(
                    "1 0 0 1 {:.3} {:.3} Tm ({}) Tj\n",
                    x,
                    y,
                    escape(&t)
                ));
            };
            for slot in 0..3 {
                put(&hf.header[slot], slot, vh - hf.margin);
                put(&hf.footer[slot], slot, hf.margin - hf.font_size);
            }
            s.push_str("ET\nQ\n");
            // Bates numbers with no explicit slot go bottom-right if the user did not use {bates}.
            if hf.bates.is_some()
                && !hf
                    .header
                    .iter()
                    .chain(hf.footer.iter())
                    .any(|t| t.contains("{bates}"))
            {
                let t = bates.clone().unwrap_or_default();
                let tw = text_width(&t, hf.font_size);
                s.push_str(&format!(
                    "q\n{:.5} {:.5} {:.5} {:.5} {:.3} {:.3} cm\n{:.3} {:.3} {:.3} rg\nBT /{FONT_NAME} {:.2} Tf 1 0 0 1 {:.3} {:.3} Tm ({}) Tj ET\nQ\n",
                    m[0], m[1], m[2], m[3], m[4], m[5], hf.color.0, hf.color.1, hf.color.2, hf.font_size,
                    vw - hf.margin - tw, hf.margin / 2.0, escape(&t)
                ));
            }
            self.add_layers(page, None, Some(s.into_bytes()), font)?;
        }
        Ok(())
    }
}

/// Today's UTC date as YYYY-MM-DD (no external time crate).
fn today_iso() -> String {
    let [y, m, d, ..] = utc_now();
    format!("{y:04}-{m:02}-{d:02}")
}

/// The time now in UTC: year, month, day, hour, minute, second.
pub(crate) fn utc_now() -> [i64; 6] {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    let t = secs.rem_euclid(86_400);
    let z = secs.div_euclid(86_400) + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    [y, m, d, t / 3600, t % 3600 / 60, t % 60]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn widths_and_escape() {
        assert!((text_width("Hello", 10.0) - 22.78).abs() < 0.01);
        assert_eq!(escape("a(b)\\"), "a\\(b\\)\\\\");
        assert_eq!(escape("é€"), "\\351?");
    }

    #[test]
    fn overlays_survive_roundtrip() {
        let mut p = Pdf::new();
        for i in 0..3 {
            p.insert_blank(i, 612.0, 792.0).unwrap();
        }
        p.rotate_pages(&[1], 90).unwrap();
        let all = [0, 1, 2];
        p.add_watermark(&all, &Watermark::default()).unwrap();
        let hf = HeaderFooter {
            bates: Some(Bates {
                prefix: "ABC".into(),
                digits: 4,
                ..Default::default()
            }),
            ..Default::default()
        };
        p.add_header_footer(&all, &hf).unwrap();
        let bytes = p.to_bytes().unwrap();
        let re = Pdf::from_bytes(&bytes, None).unwrap();
        assert_eq!(re.page_count(), 3);
        let content =
            String::from_utf8_lossy(&re.doc.get_page_content(re.page_ids()[2])).to_string();
        assert!(content.contains("Page 3 of 3"));
        assert!(content.contains("ABC0003"));
        assert!(content.contains("DRAFT"));
    }
}

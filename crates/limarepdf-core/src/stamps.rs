//! Stamps: rubber-stamp marks (Approved, Draft and so on, optionally with a name and
//! date) and pictures, as `/Stamp` annotations. The artwork is drawn once at its own
//! size and viewers fit it to the annotation's box, so moving and resizing a stamp only
//! change `/Rect`. Opacity is set in an outer drawing around the artwork, so changing it
//! leaves the artwork alone.

use crate::form_ap::nums;
use crate::imgs::{decode_upright_size, orient_matrix};
use crate::matrix as mx;
use crate::overlay::{escape, utc_now};
use crate::sign::{mark_art, sign_label, signature_art, InkColor, Mark, SignArt, SignRole};
use crate::{std14, Pdf, Result};
use anyhow::{anyhow, bail};
use lopdf::{dictionary, Dictionary, Object, ObjectId, Stream};

/// Annotation flag (`/F`): printed.
const PRINT: i64 = 1 << 2;
/// Resource names in a stamp's drawings.
const ART: &str = "LimareStamp";
const GS: &str = "LimareGs";
const FONT: &str = "LimareB";
const IMAGE: &str = "LimareIm";
/// A text stamp's word size, its name-and-date line's size, the space inside its frame,
/// and the frame's width and corner radius, in points.
const WORD: f32 = 22.0;
const SMALL: f32 = 9.0;
const PAD: f32 = 8.0;
const FRAME: f32 = 2.5;
const CORNER: f32 = 6.0;
/// Helvetica's capital height, as a share of the text size.
const CAP: f32 = 0.72;
/// The longer side of a picture stamp at its own size, in points.
const PICTURE: f32 = 160.0;
/// Control-point distance for a quarter circle drawn as a Bézier curve.
pub(crate) const KAPPA: f32 = 0.5523;

/// The ready-made stamps.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum StampKind {
    Approved,
    NotApproved,
    Draft,
    Final,
    Confidential,
    ForComment,
    Void,
}

impl StampKind {
    pub const ALL: [StampKind; 7] = [
        StampKind::Approved,
        StampKind::NotApproved,
        StampKind::Draft,
        StampKind::Final,
        StampKind::Confidential,
        StampKind::ForComment,
        StampKind::Void,
    ];

    /// Its wording, as menus show it; the stamp shows it in capitals.
    pub fn label(self) -> &'static str {
        match self {
            StampKind::Approved => "Approved",
            StampKind::NotApproved => "Not approved",
            StampKind::Draft => "Draft",
            StampKind::Final => "Final",
            StampKind::Confidential => "Confidential",
            StampKind::ForComment => "For comment",
            StampKind::Void => "Void",
        }
    }

    /// The stamp's `/Name`: the standard icon names, except Void, which has none.
    fn name(self) -> &'static str {
        match self {
            StampKind::Approved => "Approved",
            StampKind::NotApproved => "NotApproved",
            StampKind::Draft => "Draft",
            StampKind::Final => "Final",
            StampKind::Confidential => "Confidential",
            StampKind::ForComment => "ForComment",
            StampKind::Void => "Void",
        }
    }

    /// Green for yes, red for no or restricted, blue for work in progress.
    pub fn color(self) -> StampColor {
        match self {
            StampKind::Approved | StampKind::Final => StampColor::Green,
            StampKind::NotApproved | StampKind::Confidential | StampKind::Void => StampColor::Red,
            StampKind::Draft | StampKind::ForComment => StampColor::Blue,
        }
    }

    /// The stamp called `s`: its label or `/Name`, in any case, with spaces, hyphens
    /// or underscores between words or none.
    pub fn from_name(s: &str) -> Option<Self> {
        let key = |t: &str| {
            t.chars()
                .filter(|c| c.is_alphanumeric())
                .collect::<String>()
                .to_lowercase()
        };
        let want = key(s);
        Self::ALL.into_iter().find(|k| key(k.name()) == want)
    }
}

/// A text stamp's colour.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum StampColor {
    Green,
    Red,
    Blue,
}

impl StampColor {
    pub const ALL: [StampColor; 3] = [StampColor::Green, StampColor::Red, StampColor::Blue];

    pub fn rgb(self) -> [f32; 3] {
        match self {
            StampColor::Green => [0.1, 0.5, 0.15],
            StampColor::Red => [0.78, 0.1, 0.1],
            StampColor::Blue => [0.1, 0.3, 0.75],
        }
    }

    /// Its name in lower case, as settings and the command line spell it.
    pub fn name(self) -> &'static str {
        match self {
            StampColor::Green => "green",
            StampColor::Red => "red",
            StampColor::Blue => "blue",
        }
    }

    /// The colour called `s`, in any case.
    pub fn from_name(s: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|c| c.name().eq_ignore_ascii_case(s.trim()))
    }
}

/// Your own wording as a stamp shows it: trimmed and in capitals. Fails when it is
/// empty or has characters the stamp's font can't show (it covers Western European
/// languages only).
pub fn stamp_wording(text: &str) -> Result<String> {
    let word = text.trim().to_uppercase();
    if word.is_empty() {
        bail!("the stamp needs some wording");
    }
    let mut bad = String::new();
    for c in word.chars() {
        let u = c as u32;
        if !((0x20..=0x7E).contains(&u) || (0xA0..=0xFF).contains(&u)) && !bad.contains(c) {
            bad.push(c);
        }
    }
    if !bad.is_empty() {
        bail!("a stamp can't show {bad:?} yet: its font covers Western European languages only");
    }
    Ok(word)
}

/// What a stamp shows.
pub enum Stamp<'a> {
    /// A ready-made stamp, with a second line giving the author's name (when there is
    /// one) and today's date.
    Text {
        kind: StampKind,
        name_and_date: bool,
    },
    /// A stamp with your own wording (see [`stamp_wording`]), with the same optional
    /// name-and-date line.
    Custom {
        text: &'a str,
        color: StampColor,
        name_and_date: bool,
    },
    /// A PNG, JPEG or TIFF picture.
    Picture(&'a [u8]),
    /// A signature or initials, with a line under it giving today's date when `date`.
    Signature {
        art: SignArt<'a>,
        role: SignRole,
        color: InkColor,
        date: bool,
    },
    /// A tick, cross or dot.
    Mark { mark: Mark, color: InkColor },
}

/// Where a stamp goes, in displayed-page points (y down).
#[derive(Clone, Copy, Debug)]
pub enum StampPlace {
    /// Centred here, at its own size.
    Centre(f32, f32),
    /// As large as fits in this box, centred, keeping its shape.
    Fill([f32; 4]),
}

/// A text stamp's wording and look.
struct TextStamp {
    /// Its word, in capitals.
    word: String,
    /// The name-and-date line.
    line: Option<String>,
    color: StampColor,
    /// The standard icon name; none for your own wording.
    name: Option<&'static str>,
}

/// The name-and-date line.
fn name_and_date_line(author: &str) -> String {
    let [y, m, d, ..] = utc_now();
    let date = format!("{y:04}-{m:02}-{d:02}");
    match author.trim() {
        "" => date,
        name => format!("{name}, {date}"),
    }
}

fn bold_width(s: &str, size: f32) -> f32 {
    s.chars()
        .map(|c| std14::width("Helvetica-Bold", c).unwrap_or(611.0))
        .sum::<f32>()
        * size
        / 1000.0
}

/// A text stamp's size, in points.
fn text_size(word: &str, line: Option<&str>) -> (f32, f32) {
    let wide = bold_width(word, WORD).max(line.map_or(0.0, |l| bold_width(l, SMALL)));
    let tall = WORD * CAP + line.map_or(0.0, |_| SMALL * CAP + 5.0);
    (wide + 2.0 * (PAD + FRAME), tall + 2.0 * (PAD + FRAME))
}

/// A picture's size as a stamp: its upright shape, the longer side [`PICTURE`] points.
fn picture_size(w: f32, h: f32) -> (f32, f32) {
    let k = PICTURE / w.max(h).max(1.0);
    (w * k, h * k)
}

impl Stamp<'_> {
    /// A text stamp's wording and look; `None` for a picture.
    fn text(&self, author: &str) -> Result<Option<TextStamp>> {
        let (word, color, name, name_and_date) = match self {
            Stamp::Text {
                kind,
                name_and_date,
            } => (
                kind.label().to_uppercase(),
                kind.color(),
                Some(kind.name()),
                *name_and_date,
            ),
            Stamp::Custom {
                text,
                color,
                name_and_date,
            } => (stamp_wording(text)?, *color, None, *name_and_date),
            Stamp::Picture(_) | Stamp::Signature { .. } | Stamp::Mark { .. } => return Ok(None),
        };
        Ok(Some(TextStamp {
            word,
            line: name_and_date.then(|| name_and_date_line(author)),
            color,
            name,
        }))
    }

    /// Its size on the page when placed with [`StampPlace::Centre`], in points.
    pub fn size(&self, author: &str) -> Result<(f32, f32)> {
        Ok(match (self, self.text(author)?) {
            (_, Some(t)) => text_size(&t.word, t.line.as_deref()),
            (Stamp::Picture(bytes), None) => {
                let (w, h) = decode_upright_size(bytes)?;
                picture_size(w, h)
            }
            (Stamp::Mark { mark, color }, None) => {
                let a = mark_art(*mark, *color);
                (a.w, a.h)
            }
            (
                Stamp::Signature {
                    art,
                    role,
                    color,
                    date,
                },
                None,
            ) => {
                let a = signature_art(art, *role, *color, *date)?;
                (a.w, a.h)
            }
            _ => unreachable!("stamps with wording have it"),
        })
    }

    /// A picture of the stamp at `scale` pixels per point, on white, for showing in lists.
    pub fn preview(&self, author: &str, scale: f32) -> Result<crate::render::Bitmap> {
        let (w, h) = self.size(author)?;
        let mut p = Pdf::new();
        p.insert_blank(0, w, h)?;
        p.add_stamp(0, self, StampPlace::Fill([0.0, 0.0, w, h]), author)?;
        let doc = crate::render::load(p.render_snapshot()?, None)?;
        crate::render::Renderer::new(&doc).render(0, scale)
    }

    /// A fill-and-sign stamp's `/Name`, `/Subj` and colour.
    fn sign_entries(&self) -> Option<(&'static str, &'static str, InkColor)> {
        match self {
            Stamp::Signature { role, color, .. } => {
                Some((role.name(), sign_label(role.name())?, *color))
            }
            Stamp::Mark { mark, color } => Some((mark.name(), mark.label(), *color)),
            _ => None,
        }
    }
}

/// A rectangle with rounded corners, as a path.
fn rounded(r: [f32; 4], c: f32) -> String {
    let [x0, y0, x1, y1] = r;
    let k = c * (1.0 - KAPPA);
    format!(
        "{a} {y0} m {b} {y0} l {bk} {y0} {x1} {k0} {x1} {c0} c {x1} {c1} l {x1} {k1} {bk} {y1} \
         {b} {y1} c {a} {y1} l {ak} {y1} {x0} {k1} {x0} {c1} c {x0} {c0} l {x0} {k0} {ak} {y0} \
         {a} {y0} c h",
        a = x0 + c,
        b = x1 - c,
        ak = x0 + k,
        bk = x1 - k,
        c0 = y0 + c,
        c1 = y1 - c,
        k0 = y0 + k,
        k1 = y1 - k,
    )
}

impl Pdf {
    /// Put a stamp on page `page_index` (zero-based). `author` is written as the
    /// annotation's author, and on a text stamp's name-and-date line. Returns its id.
    pub fn add_stamp(
        &mut self,
        page_index: usize,
        stamp: &Stamp,
        place: StampPlace,
        author: &str,
    ) -> Result<(u32, u16)> {
        let page = self.page_id_at(page_index)?;
        let text = stamp.text(author)?;
        let (art, (w, h)) = match (&text, stamp) {
            (Some(t), _) => {
                let size = text_size(&t.word, t.line.as_deref());
                (self.text_art(t, size), size)
            }
            (None, Stamp::Picture(bytes)) => self.picture_art(bytes)?,
            (
                None,
                Stamp::Signature {
                    art,
                    role,
                    color,
                    date,
                },
            ) => {
                let a = signature_art(art, *role, *color, *date)?;
                let size = (a.w, a.h);
                (self.add_art(a), size)
            }
            (None, Stamp::Mark { mark, color }) => {
                let a = mark_art(*mark, *color);
                let size = (a.w, a.h);
                (self.add_art(a), size)
            }
            (None, _) => unreachable!("stamps with wording have it"),
        };
        let shown = match place {
            StampPlace::Centre(x, y) => [x - w / 2.0, y - h / 2.0, x + w / 2.0, y + h / 2.0],
            StampPlace::Fill(r) => {
                let (x0, x1) = (r[0].min(r[2]), r[0].max(r[2]));
                let (y0, y1) = (r[1].min(r[3]), r[1].max(r[3]));
                let k = ((x1 - x0) / w).min((y1 - y0) / h);
                let (cx, cy, hw, hh) = ((x0 + x1) / 2.0, (y0 + y1) / 2.0, w * k / 2.0, h * k / 2.0);
                [cx - hw, cy - hh, cx + hw, cy + hh]
            }
        };
        if shown[2] - shown[0] < 2.0 || shown[3] - shown[1] < 2.0 {
            bail!("the stamp is too small");
        }
        let (to_user, _, _) = self.vis_down(page);
        // Turned with the page, so the stamp reads upright as the page is shown.
        let matrix = match self.page_rotation(page) {
            90 => Some([0.0, 1.0, -1.0, 0.0, 0.0, 0.0]),
            180 => Some([-1.0, 0.0, 0.0, -1.0, 0.0, 0.0]),
            270 => Some([0.0, -1.0, 1.0, 0.0, 0.0, 0.0]),
            _ => None,
        };
        let ap = self.stamp_wrapper(art, [0.0, 0.0, w, h], matrix, 1.0);
        let mut d = dictionary! {
            "Subtype" => "Stamp",
            "Rect" => nums(&mx::transform_rect(to_user, shown)),
            "F" => PRINT,
            "AP" => dictionary! { "N" => ap },
        };
        if let Some(t) = &text {
            if let Some(name) = t.name {
                d.set("Name", name);
            }
            d.set("C", nums(&t.color.rgb()));
        }
        if let Some((name, subject, color)) = stamp.sign_entries() {
            d.set("Name", name);
            d.set("Subj", Object::string_literal(subject));
            d.set("C", nums(&color.rgb()));
        }
        self.insert_annot(page, d, author)
    }

    /// A text stamp's artwork: its word in bold capitals in a rounded frame, with the
    /// name-and-date line under it.
    fn text_art(&mut self, t: &TextStamp, (w, h): (f32, f32)) -> ObjectId {
        let (word, line) = (t.word.as_str(), t.line.as_deref());
        let [r, g, b] = t.color.rgb();
        let half = FRAME / 2.0;
        let mut s = format!(
            "q {r} {g} {b} RG {r} {g} {b} rg {FRAME} w {} S\n",
            rounded([half, half, w - half, h - half], CORNER)
        );
        let base = FRAME + PAD;
        let mut y = base;
        if let Some(l) = line {
            let x = (w - bold_width(l, SMALL)) / 2.0;
            s += &format!("BT /{FONT} {SMALL} Tf {x} {y} Td ({}) Tj ET\n", escape(l));
            y += SMALL * CAP + 5.0;
        }
        let x = (w - bold_width(word, WORD)) / 2.0;
        s += &format!(
            "BT /{FONT} {WORD} Tf {x} {y} Td ({}) Tj ET\nQ\n",
            escape(word)
        );
        let font = self.doc.add_object(dictionary! {
            "Type" => "Font", "Subtype" => "Type1", "BaseFont" => "Helvetica-Bold",
            "Encoding" => "WinAnsiEncoding",
        });
        let form = dictionary! {
            "Type" => "XObject",
            "Subtype" => "Form",
            "BBox" => nums(&[0.0, 0.0, w, h]),
            "Resources" => dictionary! { "Font" => dictionary! { FONT => font } },
        };
        self.doc.add_object(Stream::new(form, s.into_bytes()))
    }

    /// A picture stamp's artwork and its size.
    fn picture_art(&mut self, bytes: &[u8]) -> Result<(ObjectId, (f32, f32))> {
        let (img, iw, ih, o) = self.add_upright_image(bytes)?;
        let (w, h) = picture_size(iw, ih);
        let m = orient_matrix(o, w, h).map(|v| v.to_string()).join(" ");
        let form = dictionary! {
            "Type" => "XObject",
            "Subtype" => "Form",
            "BBox" => nums(&[0.0, 0.0, w, h]),
            "Resources" => dictionary! { "XObject" => dictionary! { IMAGE => img } },
        };
        let s = format!("q {m} cm /{IMAGE} Do Q\n");
        Ok((
            self.doc.add_object(Stream::new(form, s.into_bytes())),
            (w, h),
        ))
    }

    /// The drawing a stamp shows: its artwork `art` at `opacity`.
    fn stamp_wrapper(
        &mut self,
        art: ObjectId,
        bbox: [f32; 4],
        matrix: Option<[f32; 6]>,
        opacity: f32,
    ) -> ObjectId {
        let mut res = dictionary! { "XObject" => dictionary! { ART => art } };
        let mut s = String::from("q ");
        if opacity < 1.0 {
            let gs = dictionary! { "Type" => "ExtGState", "CA" => opacity, "ca" => opacity };
            res.set("ExtGState", dictionary! { GS => gs });
            s += &format!("/{GS} gs ");
        }
        s += &format!("/{ART} Do Q\n");
        let mut form = dictionary! {
            "Type" => "XObject",
            "Subtype" => "Form",
            "BBox" => nums(&bbox),
            "Resources" => res,
        };
        if let Some(m) = matrix {
            form.set("Matrix", nums(&m));
        }
        self.doc.add_object(Stream::new(form, s.into_bytes()))
    }

    /// Draw the stamp `id` again at its `/CA` opacity. Stamps LimarePDF didn't make are left
    /// alone.
    pub(crate) fn restamp(&mut self, id: ObjectId) -> Result<()> {
        let d = self.doc.get_dictionary(id)?;
        let opacity = d
            .get(b"CA")
            .ok()
            .and_then(|o| self.num(o))
            .map_or(1.0, |a| a.clamp(0.0, 1.0));
        let Some(n) = d
            .get(b"AP")
            .ok()
            .and_then(|o| self.dict_of(o))
            .and_then(|ap| ap.get(b"N").ok())
            .and_then(|o| o.as_reference().ok())
        else {
            return Ok(());
        };
        let form = &self.doc.get_object(n)?.as_stream()?.dict;
        let art = form
            .get(b"Resources")
            .ok()
            .and_then(|o| self.dict_of(o))
            .and_then(|r| r.get(b"XObject").ok())
            .and_then(|o| self.dict_of(o))
            .and_then(|x| x.get(ART.as_bytes()).ok())
            .and_then(|o| o.as_reference().ok());
        let Some(art) = art else { return Ok(()) };
        let bbox = form
            .get(b"BBox")
            .ok()
            .and_then(|o| self.rect(o))
            .ok_or_else(|| anyhow!("the stamp's drawing has no box"))?;
        let matrix = self
            .numbers(form.get(b"Matrix").ok())
            .and_then(|m| <[f32; 6]>::try_from(m).ok());
        let ap = self.stamp_wrapper(art, bbox, matrix, opacity);
        let mut apd = Dictionary::new();
        apd.set("N", Object::Reference(ap));
        self.doc.get_dictionary_mut(id)?.set("AP", apd);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_match_in_any_spelling() {
        assert_eq!(
            StampKind::from_name("not-approved"),
            Some(StampKind::NotApproved)
        );
        assert_eq!(
            StampKind::from_name("For comment"),
            Some(StampKind::ForComment)
        );
        assert_eq!(StampKind::from_name("VOID"), Some(StampKind::Void));
        assert_eq!(StampKind::from_name("rejected"), None);
    }

    #[test]
    fn own_wording_is_trimmed_in_capitals_and_western_european_only() {
        assert_eq!(stamp_wording("  paid in full ").unwrap(), "PAID IN FULL");
        assert_eq!(stamp_wording("reçu").unwrap(), "REÇU");
        assert!(stamp_wording("  ").is_err());
        let e = stamp_wording("Ωmega Ω 你").unwrap_err().to_string();
        assert!(e.contains("Ω你"), "{e}");
    }

    #[test]
    fn colours_match_in_any_case() {
        assert_eq!(StampColor::from_name("RED"), Some(StampColor::Red));
        assert_eq!(StampColor::from_name("pink"), None);
    }

    #[test]
    fn name_and_date_line_widens_and_heightens() {
        let (w1, h1) = text_size("OK", None);
        let (w2, h2) = text_size("OK", Some("A long name, 2026-01-01"));
        assert!(w2 > w1 && h2 > h1);
    }

    #[test]
    fn pictures_keep_their_shape() {
        let (w, h) = picture_size(400.0, 200.0);
        assert_eq!((w, h), (PICTURE, PICTURE / 2.0));
    }
}

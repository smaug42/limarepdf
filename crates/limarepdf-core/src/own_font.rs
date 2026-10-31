//! New text written in a PDF font's own embedded program, so that an edited line keeps its
//! font (ENH-31), and the text of an Identity CID font without a ToUnicode map read from that
//! program (PB-26).
//!
//! A character can be written in a font when a code of the font reads as exactly that
//! character (LimarePDF's own reading, [`FontCtx`]), its glyph is in the program as hayro
//! finds glyphs to draw them, isn't `.notdef` and draws something (a space needn't), the
//! PDF's width for the code is within 2% of the program's own, a simple font's encoding names
//! the same character as its ToUnicode map gives, and the program's own idea of the glyph (its
//! Unicode cmap read backwards, else the glyph's name) doesn't make it another. Of several
//! such codes: the one the line already uses, else the page; else, when they all draw one
//! glyph, the lowest; else none ([`CharRefusal::Ambiguous`]).
//!
//! The glyph lookups are hayro 0.8's (its `font/true_type.rs`, `type1.rs` and `cid.rs`), so
//! a code chosen here draws in LimarePDF what it draws in the file's other readers. hayro
//! draws a simple font whose program is OpenType in `/FontFile3` in a standard face instead;
//! such a program is looked up by its glyph names, as the other readers do. A line's fonts
//! are read when they are asked for ([`Pdf::line_fonts`]), but each character is checked only
//! when it is first asked about, so the thousands of glyphs of a Chinese, Japanese or Korean
//! font aren't all drawn.

use crate::base_encodings::{mac_code, Base};
use crate::edit::{drawn, show_items, Drawn};
use crate::fonts::FontCtx;
use crate::objects::{Parsed, Span, TextData};
use crate::own_encoding::{glyph_text, listed, MAX_PROGRAM};
use crate::preview_font::{opentype, Glyph, Metrics, Outline, Seg};
use crate::stand_in::StandIn;
use crate::{Pdf, Result};
use anyhow::anyhow;
use lopdf::content::Operation;
use lopdf::{Dictionary, Document, Object, ObjectId, Stream};
use skrifa::instance::{LocationRef, Size};
use skrifa::outline::{DrawSettings, OutlinePen};
use skrifa::raw::ps::cff::CffFontRef;
use skrifa::raw::ps::string::Sid;
use skrifa::raw::ps::type1::Type1Font;
use skrifa::raw::tables::cmap::PlatformId;
use skrifa::raw::tables::post::DEFAULT_GLYPH_NAMES;
use skrifa::raw::types::Tag;
use skrifa::raw::{FontRef, TableProvider};
use skrifa::{GlyphId, MetadataProvider};
use std::collections::hash_map::DefaultHasher;
use std::collections::{HashMap, HashSet};
use std::hash::{Hash, Hasher};
use std::sync::{Arc, LazyLock, Mutex, OnceLock};

/// Why a PDF font's own program can't take new text.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FontRefusal {
    /// A Type 3 font, whose glyphs are drawings of their own.
    Type3,
    /// The font's program isn't in the file.
    NotEmbedded,
    /// A Type 0 font whose codes aren't the CIDs themselves: a CMap other than Identity-H.
    Cmap,
    /// Vertical writing, which an edited line isn't written in.
    Vertical,
    /// The program can't be read, or isn't one the renderer draws with.
    Unreadable,
    /// One of LimarePDF's own fonts, which grows as text is added to it (`embed.rs`).
    Own,
}

/// Why a character can't be written in a font's own program.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CharRefusal {
    /// The font as a whole can't take new text.
    Font(FontRefusal),
    /// No code of the font reads as the character.
    NoCode,
    /// The program has no glyph that draws for any code that reads as it.
    NoGlyph,
    /// The PDF gives no width for the code, or one more than 2% off the program's.
    Width,
    /// The code's ToUnicode text isn't the character its encoding names.
    Encoding,
    /// The program says the code's glyph is another character.
    Contradicted,
    /// Several codes read as it, drawing different glyphs, and the line and page use none.
    Ambiguous,
}

/// The font of one part of a line ([`Pdf::line_fonts`]), as it takes new text: which code
/// writes a character in it, if one can.
#[derive(Clone)]
pub struct LineFont {
    font: std::result::Result<Arc<OwnFont>, FontRefusal>,
    /// The font is an unembedded simple one whose plain ASCII is written as it is
    /// (`FontCtx::reusable`): the reader draws it in a font of its own. An embedded font's
    /// program decides, so no code is written whose glyph it lacks.
    ascii: bool,
    /// The codes the line shows in this font, and those the page shows in it.
    line: Arc<HashSet<u32>>,
    page: Arc<HashSet<u32>>,
    /// How each of the line's characters is drawn, for writing it again as it was
    /// (`edit.rs`), the same for all the line's fonts; `None` when that isn't known.
    drawn: Option<Arc<[Drawn]>>,
    /// A Type 3 font that stands its glyphs up where the text matrix turns text space over
    /// (`FontCtx::flipped`, PB-56): an edited line written anew turns text space over for
    /// what it keeps in the font (`edit.rs`).
    flipped: bool,
    /// A simple font's widths by code (in 1/1000 em), as the page reads them
    /// (`FontCtx::width`): what text written in it is measured by (`rewrap.rs`).
    widths: Option<Arc<[f32]>>,
    /// What the pages draw the font in when the PDF doesn't hold it (`stand_in.rs`).
    stand_in: Option<StandIn>,
}

impl std::fmt::Debug for LineFont {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LineFont")
            .field("refused", &self.refused())
            .field("ascii", &self.ascii)
            .finish()
    }
}

impl LineFont {
    /// Why the font's own program can't take new text, if it can't. An unembedded font
    /// says [`FontRefusal::NotEmbedded`] even when its plain ASCII can be written.
    pub fn refused(&self) -> Option<FontRefusal> {
        self.font.as_ref().err().copied()
    }

    /// Whether new text can be written in the font at all: its own program can take it, or
    /// it is an unembedded font whose plain ASCII is written as it is.
    pub fn takes_new_text(&self) -> bool {
        self.font.is_ok() || self.ascii
    }

    /// The code (its bytes) that writes `c` in this font, or why it can't be written. An
    /// unembedded font whose plain ASCII is reused writes ASCII as itself.
    pub fn code(&self, c: char) -> std::result::Result<Vec<u8>, CharRefusal> {
        let font = match &self.font {
            Ok(font) => font,
            Err(_) if self.ascii && (' '..='~').contains(&c) => return Ok(vec![c as u8]),
            Err(why) => return Err(CharRefusal::Font(*why)),
        };
        let code = self.pick(font, c)?;
        Ok(match font.two {
            true => vec![(code >> 8) as u8, code as u8],
            false => vec![code as u8],
        })
    }

    /// The code that writes `c` in `font`, this one's program: of those that pass every
    /// check, the one the line uses, else the page, else the lowest when they draw one glyph.
    fn pick(&self, font: &OwnFont, c: char) -> std::result::Result<u32, CharRefusal> {
        let found = font.check(c)?;
        let used = |codes: &HashSet<u32>| found.iter().find(|(k, _)| codes.contains(k));
        match used(&self.line).or_else(|| used(&self.page)) {
            Some(&(k, _)) => Ok(k),
            None if found.iter().all(|(_, g)| *g == found[0].1) => {
                Ok(found.iter().map(|(k, _)| *k).min().expect("found"))
            }
            None => Err(CharRefusal::Ambiguous),
        }
    }

    /// A small OpenType font drawing what can be written in this font as its own program
    /// draws it, for showing text typed in it (the app's typing fonts): the characters the
    /// line shows with a code of their own, as it shows them, then the others the font has a
    /// code for ([`LineFont::code`]), at most 4,096, each with its glyph's outline
    /// and the PDF's width as its advance, so that the caret goes where the text will be
    /// written. `None` when the program can't take new text. Drawing every glyph, it is for
    /// another thread than the window's.
    pub fn preview(&self) -> Option<Vec<u8>> {
        let font = self.font.as_ref().ok()?;
        // Each character with its codes and their widths.
        type Codes<'a> = &'a [(u32, Option<f32>)];
        let mut chars: Vec<(char, Codes)> =
            (font.by_char.iter()).map(|(c, k)| (*c, &k[..])).collect();
        chars.sort_unstable_by_key(|(c, _)| *c);
        let mut chosen: Vec<(char, u32, Option<f32>)> = Vec::new();
        let mut taken = HashSet::new();
        for &(c, codes) in &chars {
            if let Some(&(k, w)) = codes.iter().find(|(k, _)| self.line.contains(k)) {
                chosen.push((c, k, w));
                taken.insert(c);
            }
        }
        for &(c, codes) in &chars {
            if chosen.len() >= PREVIEW_MOST {
                break;
            }
            if let (false, Ok(k)) = (taken.contains(&c), self.pick(font, c)) {
                let w = (codes.iter().find(|(code, _)| *code == k)).and_then(|(_, w)| *w);
                chosen.push((c, k, w));
            }
        }
        let glyphs: Vec<Glyph> = (chosen.into_iter())
            .map(|(c, k, w)| {
                let mut outline = Outline::default();
                let advance = font.program.trace(font.glyph(k), &mut outline);
                let outline = rounded(outline);
                let advance = w.or(advance).unwrap_or(0.0);
                Glyph {
                    c,
                    advance,
                    outline,
                }
            })
            .collect();
        Some(opentype("Limare Preview", &glyphs, font.metrics))
    }

    /// What tells this font's preview ([`LineFont::preview`]) from another's: the same for
    /// one font on the lines that show the same codes in it. `None` when it has none.
    pub fn preview_key(&self) -> Option<u64> {
        let font = self.font.as_ref().ok()?;
        let mut line: Vec<u32> = self.line.iter().copied().collect();
        line.sort_unstable();
        let mut h = DefaultHasher::new();
        (font.key, line).hash(&mut h);
        Some(h.finish())
    }

    /// How wide `c` is when it is written in this font ([`LineFont::code`]), in 1/1000 em as
    /// the page reads it; `None` when it can't be written.
    pub(crate) fn advance(&self, c: char) -> Option<f32> {
        let bytes = self.code(c).ok()?;
        match &bytes[..] {
            [b] => self.widths.as_ref()?.get(*b as usize).copied(),
            [hi, lo] => {
                let code = u32::from(*hi) << 8 | u32::from(*lo);
                let font = self.font.as_ref().ok()?;
                let codes = font.by_char.get(&c)?;
                let found = codes.iter().find(|(k, _)| *k == code);
                found.and_then(|(_, w)| *w)
            }
            _ => None,
        }
    }

    /// What LimarePDF's pages draw the font in when the PDF names it but doesn't hold its
    /// program (ENH-25): text typed in it shows in that face while typing. `None` for a
    /// font the PDF holds, a Type 3 font, and Symbol or ZapfDingbats.
    pub fn stand_in(&self) -> Option<&StandIn> {
        self.stand_in.as_ref()
    }

    /// How each of the line's characters is drawn, when that is known.
    pub(crate) fn drawn(&self) -> Option<&[Drawn]> {
        self.drawn.as_deref()
    }

    /// The font turns its glyphs over to stand them up where the text matrix turns text
    /// space over (`FontCtx::flipped`), so what an edited line keeps in it is written in text
    /// space turned over.
    pub(crate) fn flipped(&self) -> bool {
        self.flipped
    }
}

impl Pdf {
    /// The fonts of text object `id` on page `page_index` (ids as in
    /// [`Pdf::page_objects`]), one for each part of the line ([`crate::PageObject::spans`]),
    /// in order: which characters each can write in its own program.
    pub fn line_fonts(&self, page_index: usize, id: usize) -> Result<Vec<LineFont>> {
        let (page, parsed) = self.load_edit(page_index)?;
        let obj = Self::pick(&parsed, id)?;
        let t = (obj.text.as_ref()).ok_or_else(|| anyhow!("object {id} is not text"))?;
        Ok(self.fonts_of(page, &parsed, t))
    }

    /// [`Pdf::line_fonts`] for every line of page `page_index`, by object id as in
    /// [`Pdf::page_objects`] (`None` for an object that isn't text), with the page read once
    /// and each of its fonts once: for reading the fonts of a page's lines as they are
    /// selected, one after another.
    pub fn page_line_fonts(&self, page_index: usize) -> Result<Vec<Option<Vec<LineFont>>>> {
        let (page, parsed) = self.load_edit(page_index)?;
        let fonts = self.font_resources(page);
        let mut read = PageFonts::new(&self.doc, &fonts);
        let lines = parsed.objs.iter().map(|o| o.text.as_ref());
        Ok(lines.map(|t| t.map(|t| read.of(&parsed, t))).collect())
    }

    /// [`Pdf::line_fonts`] for line `t` of page `page`, parsed as `parsed`.
    pub(crate) fn fonts_of(&self, page: ObjectId, parsed: &Parsed, t: &TextData) -> Vec<LineFont> {
        let fonts = self.font_resources(page);
        PageFonts::new(&self.doc, &fonts).of(parsed, t)
    }

    /// The fonts in page `page`'s resources, by name.
    pub(crate) fn font_resources(&self, page: ObjectId) -> Dictionary {
        let res = self.resolved_dict(self.inherited(page, b"Resources").as_ref());
        self.resolved_dict(res.get(b"Font").ok())
    }
}

/// A page's fonts as [`Pdf::line_fonts`] reads them, each read once: kept while several of
/// its lines are written.
pub(crate) struct PageFonts<'a> {
    doc: &'a Document,
    /// The page's fonts by name ([`Pdf::font_resources`]).
    fonts: &'a Dictionary,
    /// Each font read so far, as the page is read and as it takes new text; `None` for one
    /// the page doesn't have.
    read: HashMap<Vec<u8>, Option<(FontCtx<'a>, LineFont)>>,
}

impl<'a> PageFonts<'a> {
    pub(crate) fn new(doc: &'a Document, fonts: &'a Dictionary) -> Self {
        Self {
            doc,
            fonts,
            read: HashMap::new(),
        }
    }

    /// The fonts of line `t` of the page, parsed as `parsed`: one for each of its parts, in
    /// order. The codes a line uses in a font are those of all its parts in it.
    pub(crate) fn of(&mut self, parsed: &Parsed, t: &TextData) -> Vec<LineFont> {
        let (doc, fonts) = (self.doc, self.fonts);
        let mut out = Vec::new();
        for s in &t.spans {
            let read = self.read.entry(s.font.clone()).or_insert_with(|| {
                let (_, dict) = doc.dereference(fonts.get(&s.font).ok()?).ok()?;
                let dict = dict.as_dict().ok()?;
                let ctx = FontCtx::new(doc, dict);
                let mut page = HashSet::new();
                for o in parsed.objs.iter().filter_map(|o| o.text.as_ref()) {
                    for other in o.spans.iter().filter(|o| o.font == s.font) {
                        codes(&ctx, &parsed.ops, o, other, &mut page);
                    }
                }
                let font = OwnFont::read(doc, dict, &ctx).map(Arc::new);
                let unembedded = matches!(font, Err(FontRefusal::NotEmbedded));
                let widths = (!ctx.type0).then(|| (0..=255).map(|k| ctx.width(k, 1)).collect());
                let font = LineFont {
                    font,
                    ascii: unembedded && ctx.reusable(),
                    line: Arc::default(),
                    page: Arc::new(page),
                    drawn: None,
                    flipped: ctx.flipped,
                    widths,
                    stand_in: crate::stand_in::stand_in(doc, dict),
                };
                Some((ctx, font))
            });
            let Some((ctx, font)) = read else {
                out.push(LineFont {
                    font: Err(FontRefusal::Unreadable),
                    ascii: false,
                    line: Arc::default(),
                    page: Arc::default(),
                    drawn: None,
                    flipped: false,
                    widths: None,
                    stand_in: None,
                });
                continue;
            };
            let mut line = HashSet::new();
            for other in t.spans.iter().filter(|o| o.font == s.font) {
                codes(ctx, &parsed.ops, t, other, &mut line);
            }
            out.push(LineFont {
                line: Arc::new(line),
                ..font.clone()
            });
        }
        let ctxs: Vec<Option<&FontCtx>> = (t.spans.iter())
            .map(|s| Some(&self.read.get(&s.font)?.as_ref()?.0))
            .collect();
        let drawn: Option<Arc<[Drawn]>> = drawn(&parsed.ops, t, &ctxs).map(Into::into);
        for f in &mut out {
            f.drawn = drawn.clone();
        }
        out
    }
}

/// The codes part `s` of line `t` shows in its font, read as `ctx` reads them, put in
/// `into`; `ops` are the page's operators.
fn codes(ctx: &FontCtx, ops: &[Operation], t: &TextData, s: &Span, into: &mut HashSet<u32>) {
    for run in &t.runs[s.runs.0..s.runs.1] {
        for it in show_items(&ops[run.op], run.show) {
            if let Object::String(bytes, _) = it {
                into.extend(ctx.codes(&bytes).into_iter().map(|(k, ..)| k));
            }
        }
    }
}

/// Most characters a preview font holds ([`LineFont::preview`]): enough for a language's
/// letters and more, not all of a Chinese, Japanese or Korean font's.
const PREVIEW_MOST: usize = 4096;

/// Counts the segments of an outline.
#[derive(Default)]
struct Segments(usize);

impl OutlinePen for Segments {
    fn move_to(&mut self, _: f32, _: f32) {}
    fn line_to(&mut self, _: f32, _: f32) {
        self.0 += 1;
    }
    fn quad_to(&mut self, _: f32, _: f32, _: f32, _: f32) {
        self.0 += 1;
    }
    fn curve_to(&mut self, _: f32, _: f32, _: f32, _: f32, _: f32, _: f32) {
        self.0 += 1;
    }
    fn close(&mut self) {}
}

/// An embedded font program, as it is read to draw.
enum Program {
    /// TrueType or OpenType, with TrueType or CFF outlines.
    Sfnt(Arc<[u8]>),
    /// A bare CFF program.
    Cff(Arc<[u8]>),
    /// A Type 1 program.
    Type1(Arc<Type1Font>),
}

impl Program {
    /// A TrueType or OpenType program, unless hayro would pass over it: one whose `post`
    /// table has a version it doesn't know (pdf.js issue 9462).
    fn sfnt(data: Vec<u8>) -> Option<Self> {
        let font = FontRef::from_index(&data, 0).ok()?;
        let known = |v: (u16, u16)| matches!(v, (1, 0) | (2, 0) | (2, 5) | (3, 0));
        if font
            .post()
            .is_ok_and(|p| !known(p.version().to_major_minor()))
        {
            return None;
        }
        Some(Self::Sfnt(data.into()))
    }

    /// A bare CFF (or CFF2) program, when its fonts can be read.
    fn cff(data: Vec<u8>) -> Option<Self> {
        let font = CffFontRef::new(&data, 0, None).ok()?;
        for i in 0..font.num_subfonts() {
            font.subfont(i, &[]).ok()?;
        }
        Some(Self::Cff(data.into()))
    }

    fn type1(data: &[u8]) -> Option<Self> {
        Type1Font::new(data).ok().map(|f| Self::Type1(Arc::new(f)))
    }

    /// A CID font's program, read by what it turns out to be (as hayro reads it), not by
    /// the descriptor's key.
    fn any(data: Vec<u8>) -> Option<Self> {
        if FontRef::from_index(&data, 0).is_ok() {
            Self::sfnt(data)
        } else if CffFontRef::new(&data, 0, None).is_ok() {
            Self::cff(data)
        } else {
            Self::type1(&data)
        }
    }

    fn font(&self) -> Option<FontRef<'_>> {
        match self {
            Self::Sfnt(data) => FontRef::from_index(data, 0).ok(),
            _ => None,
        }
    }

    /// The CFF outlines: a bare program, or an OpenType font's `CFF ` table.
    fn cff_font(&self) -> Option<CffFontRef<'_>> {
        let data: &[u8] = match self {
            Self::Cff(data) => data,
            Self::Sfnt(_) => self.font()?.table_data(Tag::new(b"CFF "))?.as_bytes(),
            Self::Type1(_) => return None,
        };
        CffFontRef::new(data, 0, None).ok()
    }

    fn num_glyphs(&self) -> u32 {
        match self {
            Self::Sfnt(_) => {
                (self.font().and_then(|f| f.maxp().ok())).map_or(0, |m| u32::from(m.num_glyphs()))
            }
            Self::Cff(_) => self.cff_font().map_or(0, |c| c.num_glyphs()),
            Self::Type1(t) => t.num_glyphs(),
        }
    }

    /// Whether glyph `gid` draws something, and its advance width in 1/1000 em.
    fn draw(&self, gid: GlyphId) -> (bool, Option<f32>) {
        let mut pen = Segments::default();
        let advance = self.trace(gid, &mut pen);
        (pen.0 > 0, advance)
    }

    /// Glyph `gid` drawn with `pen` at 1000 units to the em; its advance width.
    fn trace(&self, gid: GlyphId, pen: &mut impl OutlinePen) -> Option<f32> {
        match self {
            Self::Sfnt(_) => {
                let font = self.font()?;
                if let Some(o) = font.outline_glyphs().get(gid) {
                    let at = DrawSettings::unhinted(Size::new(1000.0), LocationRef::default());
                    let _ = o.draw(at, &mut *pen);
                }
                let metrics = font.glyph_metrics(Size::new(1000.0), LocationRef::default());
                metrics.advance_width(gid)
            }
            Self::Cff(_) => self.cff_font().and_then(|c| {
                let sub = c.subfont(c.subfont_index(gid)?, &[]).ok()?;
                c.draw(&sub, gid, &[], Some(1000.0), pen).ok().flatten()
            }),
            Self::Type1(t) => t.draw(gid, Some(1000.0), pen).ok().flatten(),
        }
    }

    /// A CFF program's glyph names (not a CID-keyed one's), by glyph, in its charset's order.
    fn cff_names(&self) -> Vec<(GlyphId, String)> {
        let Some(cff) = self.cff_font().filter(|c| !c.is_cid()) else {
            return Vec::new();
        };
        let Some(charset) = cff.charset() else {
            return Vec::new();
        };
        (charset.iter())
            .filter_map(|(gid, sid)| {
                let name = std::str::from_utf8(cff.string(sid)?).ok()?;
                Some((gid, name.to_string()))
            })
            .collect()
    }

    /// A TrueType or OpenType program's glyph names in its `post` table, by glyph, as hayro
    /// reads them (versions 1 and 2 only).
    fn post_names(&self) -> Vec<(GlyphId, String)> {
        let Some(post) = self.font().and_then(|f| f.post().ok()) else {
            return Vec::new();
        };
        match post.version().to_major_minor() {
            (1, 0) => (DEFAULT_GLYPH_NAMES.iter().enumerate())
                .map(|(gid, n)| (GlyphId::new(gid as u32), n.to_string()))
                .collect(),
            (2, 0) => {
                let (Some(index), Some(strings)) = (post.glyph_name_index(), post.string_data())
                else {
                    return Vec::new();
                };
                let own: Vec<Option<&str>> = (strings.iter())
                    .map(|s| s.ok().map(|s| s.as_str()))
                    .collect();
                let standard = DEFAULT_GLYPH_NAMES.len();
                (index.iter().enumerate())
                    .filter_map(|(gid, i)| {
                        let i = usize::from(i.get());
                        let name = match DEFAULT_GLYPH_NAMES.get(i) {
                            Some(n) => *n,
                            None => (*own.get(i - standard)?)?,
                        };
                        Some((GlyphId::new(gid as u32), name.to_string()))
                    })
                    .collect()
            }
            _ => Vec::new(),
        }
    }

    /// The name of glyph `gid` in the program, where it names its glyphs.
    fn name(&self, gid: GlyphId, post: &[(GlyphId, String)]) -> Option<String> {
        match self {
            Self::Type1(t) => t.glyph_name(gid).map(str::to_string),
            _ => match self.cff_font().filter(|c| !c.is_cid()) {
                Some(cff) => {
                    let sid = cff.charset()?.string_id(gid).ok()?;
                    Some(std::str::from_utf8(cff.string(sid)?).ok()?.to_string())
                }
                None => (post.iter().find(|(g, _)| *g == gid)).map(|(_, n)| n.clone()),
            },
        }
    }

    /// Each glyph's characters by the program's Unicode cmaps: (3,1), (3,10) and (0,_).
    fn cmap_chars(&self) -> HashMap<GlyphId, Vec<char>> {
        let mut out: HashMap<GlyphId, Vec<char>> = HashMap::new();
        let Some(cmap) = self.font().and_then(|f| f.cmap().ok()) else {
            return out;
        };
        for r in cmap.encoding_records() {
            let unicode = match r.platform_id() {
                PlatformId::Windows => matches!(r.encoding_id(), 1 | 10),
                PlatformId::Unicode => true,
                _ => false,
            };
            let Some(sub) = unicode
                .then(|| r.subtable(cmap.offset_data()).ok())
                .flatten()
            else {
                continue;
            };
            for (cp, gid) in sub.iter() {
                let chars = out.entry(gid).or_default();
                if let Some(c) = char::from_u32(cp).filter(|c| !chars.contains(c)) {
                    chars.push(c);
                }
            }
        }
        out
    }
}

/// How the font descriptor's `/Flags` say a TrueType font's glyphs are found, as hayro reads
/// them: by the encoding's glyph names when the nonsymbolic flag is set and no flag the PDF
/// specification doesn't define is (hayro's flags are then unread), else by code.
fn nonsymbolic(flags: Option<i64>) -> bool {
    const DEFINED: i64 = (0b111 << 16) | 0b110_1111;
    flags.is_some_and(|f| (0..=u32::MAX as i64).contains(&f) && f & !DEFINED == 0 && f & 32 != 0)
}

/// A CID font's `/CIDToGIDMap`: from CIDs to the program's glyphs.
#[derive(Clone)]
enum CidMap {
    Identity,
    Mapped {
        to: HashMap<u16, GlyphId>,
        back: HashMap<GlyphId, u16>,
    },
}

impl CidMap {
    /// The map of CIDFont `desc`, as hayro reads it: a stream of two-byte glyph numbers, else
    /// Identity (named so, or not given, or anything else).
    fn of(doc: &Document, desc: &Dictionary) -> Self {
        let stream = (desc.get(b"CIDToGIDMap").ok())
            .and_then(|o| doc.dereference(o).ok())
            .and_then(|(_, o)| o.as_stream().ok())
            .and_then(|s| s.get_plain_content_with_limit(1 << 20).ok());
        let Some(data) = stream else {
            return Self::Identity;
        };
        let (mut to, mut back) = (HashMap::new(), HashMap::new());
        for (cid, g) in data.as_chunks::<2>().0.iter().enumerate() {
            let gid = GlyphId::new(u32::from(u16::from_be_bytes([g[0], g[1]])));
            to.insert(cid as u16, gid);
            back.insert(gid, cid as u16);
        }
        Self::Mapped { to, back }
    }

    fn to(&self, cid: u16) -> GlyphId {
        match self {
            Self::Identity => GlyphId::new(u32::from(cid)),
            Self::Mapped { to, .. } => to.get(&cid).copied().unwrap_or(GlyphId::NOTDEF),
        }
    }

    fn back(&self, gid: GlyphId) -> u16 {
        match self {
            Self::Identity => gid.to_u32() as u16,
            Self::Mapped { back, .. } => back.get(&gid).copied().unwrap_or(gid.to_u32() as u16),
        }
    }
}

/// The glyph CID `cid` draws in a CID font's `program`, as hayro finds it: through the
/// `/CIDToGIDMap`, except in a CID-keyed CFF program, whose charset gives a CID's glyph
/// (or, with a map given, the map read backwards), and a Type 1 program (the CID itself).
fn cid_glyph(program: &Program, map: &CidMap, cid: u32) -> GlyphId {
    let cid = cid as u16;
    match program {
        Program::Sfnt(_) => map.to(cid),
        Program::Cff(_) => match program.cff_font().filter(|c| c.is_cid()) {
            Some(c) if matches!(map, CidMap::Identity) => (c.charset())
                .and_then(|s| s.glyph_id(Sid::new(cid)).ok())
                .unwrap_or(GlyphId::NOTDEF),
            Some(_) => GlyphId::new(u32::from(map.back(GlyphId::new(u32::from(cid))))),
            None => map.to(cid),
        },
        Program::Type1(_) => GlyphId::new(u32::from(cid)),
    }
}

/// The program in font descriptor `fd` under the first of `keys` it has, with that key;
/// `None` when it has none.
fn program_stream<'a>(
    doc: &'a Document,
    fd: &'a Dictionary,
    keys: &[&'static [u8]],
) -> Option<(&'static [u8], &'a Stream)> {
    keys.iter().find_map(|&k| {
        let (_, o) = doc.dereference(fd.get(k).ok()?).ok()?;
        Some((k, o.as_stream().ok()?))
    })
}

fn sub_dict<'a>(doc: &'a Document, d: &'a Dictionary, key: &[u8]) -> Option<&'a Dictionary> {
    let (_, o) = doc.dereference(d.get(key).ok()?).ok()?;
    match o {
        Object::Dictionary(d) => Some(d),
        Object::Stream(s) => Some(&s.dict),
        _ => None,
    }
}

fn name<'a>(doc: &'a Document, d: &'a Dictionary, key: &[u8]) -> Option<&'a [u8]> {
    let (_, o) = doc.dereference(d.get(key).ok()?).ok()?;
    o.as_name().ok()
}

fn number(doc: &Document, d: &Dictionary, key: &[u8]) -> Option<f32> {
    doc.dereference(d.get(key).ok()?).ok()?.1.as_float().ok()
}

/// A hash of a font's program `data` and its dictionaries `dicts` (as they print, the
/// objects they refer to by number), which tells it from another font.
fn key_of(data: &[u8], dicts: &[&Dictionary]) -> u64 {
    let mut h = DefaultHasher::new();
    data.hash(&mut h);
    for d in dicts {
        format!("{d:?}").hash(&mut h);
    }
    h.finish()
}

/// Font descriptor `fd`'s ascent and descent, in 1/1000 em, where it gives them; else
/// those of most Latin fonts.
fn metrics_of(doc: &Document, fd: &Dictionary) -> Metrics {
    let ascent = number(doc, fd, b"Ascent").filter(|a| a.is_finite() && *a > 0.0);
    let descent = number(doc, fd, b"Descent").filter(|d| d.is_finite() && *d < 0.0);
    Metrics {
        ascent: ascent.unwrap_or(800.0),
        descent: descent.unwrap_or(-200.0),
    }
}

/// `o` with each coordinate rounded to a whole unit (1/1000 em), which the renderer draws
/// alike and writes in fewer bytes.
fn rounded(o: Outline) -> Outline {
    let r = |x: f32| x.round();
    Outline(
        (o.0.into_iter())
            .map(|s| match s {
                Seg::Move(x, y) => Seg::Move(r(x), r(y)),
                Seg::Line(x, y) => Seg::Line(r(x), r(y)),
                Seg::Curve(a, b, c, d, e, f) => Seg::Curve(r(a), r(b), r(c), r(d), r(e), r(f)),
                Seg::Close => Seg::Close,
            })
            .collect(),
    )
}

/// The character a glyph name is drawn for in a TrueType font's Unicode cmap, as hayro reads
/// it: by the Adobe Glyph List, else a `uniXXXX` or `uXXXX` name.
fn name_char(name: &str) -> Option<char> {
    if let Some(c) = listed(name).and_then(|t| t.chars().next()) {
        return Some(c);
    }
    let hex = |s: &str| u32::from_str_radix(s, 16).ok().and_then(char::from_u32);
    match name.strip_prefix("uni") {
        Some(rest) => hex(rest),
        None => name.strip_prefix('u').and_then(hex),
    }
}

/// A font's own program, read for writing new text in it.
struct OwnFont {
    program: Program,
    /// Codes are two bytes (Identity-H), not one.
    two: bool,
    /// What tells the font from another: its program's bytes and its dictionaries, hashed.
    key: u64,
    /// The font descriptor's ascent and descent.
    metrics: Metrics,
    /// A simple font's glyph for each code, as the renderer finds it.
    glyphs: Vec<GlyphId>,
    /// A CID font's glyphs by CID.
    cids: CidMap,
    /// Each character with the codes that read as it alone, and the PDF's width for each
    /// (in 1/1000 em; `None` where it gives none).
    by_char: HashMap<char, Vec<(u32, Option<f32>)>>,
    /// Codes whose ToUnicode text isn't the character their encoding names.
    disagree: HashSet<u32>,
    /// The program's own glyph names in `post`, read when first needed.
    post: OnceLock<Vec<(GlyphId, String)>>,
    /// The program's characters for each glyph by its cmap, read when first needed.
    chars: OnceLock<HashMap<GlyphId, Vec<char>>>,
    /// Each character checked so far: its codes that pass, with their glyphs, or why none.
    checked: Mutex<HashMap<char, Checked>>,
}

/// The codes of a character that pass every check, with their glyphs, or why none does.
type Checked = std::result::Result<Vec<(u32, GlyphId)>, CharRefusal>;

impl OwnFont {
    /// Font `dict`'s own program, read for new text, or why it can't take any; `ctx` is the
    /// font as the page is read (`FontCtx::new(doc, dict)`).
    fn read(
        doc: &Document,
        dict: &Dictionary,
        ctx: &FontCtx,
    ) -> std::result::Result<Self, FontRefusal> {
        if dict.has(crate::embed::KEY) {
            return Err(FontRefusal::Own);
        }
        match name(doc, dict, b"Subtype").unwrap_or_default() {
            b"Type3" => Err(FontRefusal::Type3),
            b"Type0" => Self::cid(doc, dict, ctx),
            subtype => Self::simple(doc, dict, subtype, ctx),
        }
    }

    fn new(program: Program, two: bool) -> Self {
        Self {
            program,
            two,
            key: 0,
            metrics: Metrics {
                ascent: 800.0,
                descent: -200.0,
            },
            glyphs: Vec::new(),
            cids: CidMap::Identity,
            by_char: HashMap::new(),
            disagree: HashSet::new(),
            post: OnceLock::new(),
            chars: OnceLock::new(),
            checked: Mutex::new(HashMap::new()),
        }
    }

    /// A simple font (`subtype` Type1, MMType1, TrueType or OpenType).
    fn simple(
        doc: &Document,
        dict: &Dictionary,
        subtype: &[u8],
        ctx: &FontCtx,
    ) -> std::result::Result<Self, FontRefusal> {
        use FontRefusal::*;
        let fd = sub_dict(doc, dict, b"FontDescriptor").ok_or(NotEmbedded)?;
        let truetype = matches!(subtype, b"TrueType" | b"OpenType");
        let keys: [&[u8]; 3] = [b"FontFile2", b"FontFile3", b"FontFile"];
        let (key, stream) = program_stream(doc, fd, &keys).ok_or(NotEmbedded)?;
        let data = (stream.get_plain_content_with_limit(MAX_PROGRAM)).map_err(|_| Unreadable)?;
        let (key_hash, metrics) = (key_of(&data, &[dict]), metrics_of(doc, fd));
        // The renderer reads a TrueType font's program from `/FontFile2` and a Type 1 font's
        // from `/FontFile3` (CFF) or `/FontFile`; OpenType in `/FontFile3`, which it draws
        // in a standard face, is read as the other readers read it.
        let program = match (truetype, key) {
            (true, b"FontFile2") => Program::sfnt(data),
            (_, b"FontFile3") => match CffFontRef::new(&data, 0, None) {
                Ok(_) => Program::cff(data),
                Err(_) => Program::sfnt(data),
            },
            (false, b"FontFile") => Program::type1(&data),
            _ => None,
        };
        let mut font = Self::new(program.ok_or(Unreadable)?, false);
        (font.key, font.metrics) = (key_hash, metrics);

        let (base, differences) =
            match doc.dereference(dict.get(b"Encoding").unwrap_or(&Object::Null)) {
                Ok((_, Object::Dictionary(e))) => {
                    let base = name(doc, e, b"BaseEncoding").map_or(Base::BuiltIn, Base::named);
                    (base, differences(doc, e))
                }
                Ok((_, Object::Name(n))) => (Base::named(n), HashMap::new()),
                _ => (Base::BuiltIn, HashMap::new()),
            };
        let flags = (fd.get(b"Flags").ok())
            .and_then(|o| doc.dereference(o).ok())
            .and_then(|(_, o)| o.as_i64().ok());
        let by_name = match (&font.program, truetype) {
            (Program::Sfnt(_), true) => false,
            (Program::Sfnt(_), false) => font.program.cff_font().is_some(),
            _ => true,
        };
        font.glyphs = match by_name {
            true => font.named_glyphs(base, &differences),
            false => font.truetype_glyphs(base, &differences, nonsymbolic(flags)),
        };

        let widths = simple_widths(doc, dict);
        for code in 0..=255u32 {
            let text = ctx.decode_code(code, &[code as u8]);
            let mut chars = text.chars();
            if let (Some(c), None) = (chars.next(), chars.next()) {
                if !c.is_control() && c != '\u{FFFD}' {
                    let w = widths[code as usize];
                    font.by_char.entry(c).or_default().push((code, w));
                }
            }
        }
        // A symbolic TrueType font is drawn by code, whatever its encoding names.
        let outlines_cff = font.program.cff_font().is_some();
        let by_code = !by_name && !outlines_cff && !nonsymbolic(flags);
        if !by_code {
            let mut plain = dict.clone();
            plain.remove(b"ToUnicode");
            let enc = FontCtx::new(doc, &plain);
            for code in 0..=255u32 {
                let known = match differences.get(&(code as u8)) {
                    Some(n) => glyph_text(n).is_some(),
                    None => base != Base::BuiltIn,
                };
                let raw = [code as u8];
                if known
                    && ctx.mapped(code).is_some()
                    && enc.decode_code(code, &raw) != ctx.decode_code(code, &raw)
                {
                    font.disagree.insert(code);
                }
            }
        }
        Ok(font)
    }

    /// A Type 0 font, whose codes must be its CIDs (Identity-H).
    fn cid(
        doc: &Document,
        dict: &Dictionary,
        ctx: &FontCtx,
    ) -> std::result::Result<Self, FontRefusal> {
        use FontRefusal::*;
        if ctx.vertical {
            return Err(Vertical);
        }
        if name(doc, dict, b"Encoding") != Some(b"Identity-H") {
            return Err(Cmap);
        }
        let desc = (dict.get(b"DescendantFonts").ok())
            .and_then(|o| doc.dereference(o).ok())
            .and_then(|(_, o)| o.as_array().ok())
            .and_then(|a| a.first())
            .and_then(|o| doc.dereference(o).ok())
            .and_then(|(_, o)| o.as_dict().ok())
            .ok_or(Unreadable)?;
        let fd = sub_dict(doc, desc, b"FontDescriptor").ok_or(NotEmbedded)?;
        let keys: [&[u8]; 3] = [b"FontFile2", b"FontFile3", b"FontFile"];
        let (_, stream) = program_stream(doc, fd, &keys).ok_or(NotEmbedded)?;
        let data = (stream.get_plain_content_with_limit(MAX_PROGRAM)).map_err(|_| Unreadable)?;
        let (key, metrics) = (key_of(&data, &[dict, desc]), metrics_of(doc, fd));
        let mut font = Self::new(Program::any(data).ok_or(Unreadable)?, true);
        (font.key, font.metrics) = (key, metrics);
        font.cids = CidMap::of(doc, desc);
        for code in 0..=0xFFFFu32 {
            let raw = [(code >> 8) as u8, code as u8];
            let Some(text) = ctx.known_text(code, &raw) else {
                continue;
            };
            let mut chars = text.chars();
            if let (Some(c), None) = (chars.next(), chars.next()) {
                if !c.is_control() && c != '\u{FFFD}' {
                    let w = ctx.width(code, 2);
                    font.by_char.entry(c).or_default().push((code, Some(w)));
                }
            }
        }
        Ok(font)
    }

    /// Each code's glyph in a font whose glyphs are found by name (Type 1 and CFF
    /// programs), as hayro finds them: a `/Differences` name, else the program's own
    /// encoding, else the base encoding's name. A CFF program's names are also tried as
    /// `space` and `hyphen` for `nbspace` and `sfthyphen`.
    fn named_glyphs(&self, base: Base, differences: &HashMap<u8, String>) -> Vec<GlyphId> {
        let mut names: HashMap<String, GlyphId> = HashMap::new();
        let cff = matches!(self.program, Program::Cff(_) | Program::Sfnt(_));
        let builtin: Box<dyn Fn(u8) -> Option<GlyphId> + '_> = match &self.program {
            Program::Type1(t) => {
                names.extend(t.glyph_names().map(|(g, n)| (n.to_string(), g)));
                Box::new(move |c| t.encoding().and_then(|e| e.map(c)))
            }
            _ => {
                names.extend(self.program.cff_names().into_iter().map(|(g, n)| (n, g)));
                let font = self.program.cff_font();
                Box::new(move |c| font.as_ref()?.encoding()?.map(c))
            }
        };
        let glyph = |n: &str| {
            let normal = match n {
                "nbspace" if cff => "space",
                "sfthyphen" if cff => "hyphen",
                n => n,
            };
            names.get(n).or_else(|| names.get(normal)).copied()
        };
        (0..=255u8)
            .map(|code| {
                match differences.get(&code) {
                    Some(n) => glyph(n),
                    None if base == Base::BuiltIn => builtin(code),
                    None => base.name(code).and_then(glyph),
                }
                .unwrap_or(GlyphId::NOTDEF)
            })
            .collect()
    }

    /// Each code's glyph in a TrueType font, as hayro finds them. A program with CFF
    /// outlines: by the code's glyph name (`/Differences`, else the base encoding, else the
    /// standard one) in its charset. A nonsymbolic one: that name's character through a
    /// (3,1) cmap, else its Mac OS Roman code through a (1,0) cmap, else the name in `post`
    /// (or a `gNN` name, the glyph's number). A symbolic one: the code through a (3,0) or
    /// (3,1) cmap (also at 0xF000, 0xF100 and 0xF200), or a (1,0) or (0,0) one.
    fn truetype_glyphs(
        &self,
        base: Base,
        differences: &HashMap<u8, String>,
        nonsymbolic: bool,
    ) -> Vec<GlyphId> {
        let code_name = |code: u8| {
            (differences.get(&code).map(String::as_str))
                .or_else(|| base.name(code))
                .or_else(|| Base::Standard.name(code))
        };
        let nonzero = |g: &GlyphId| *g != GlyphId::NOTDEF;
        if self.program.cff_font().is_some() {
            let names = self.program.cff_names();
            return (0..=255u8)
                .map(|code| {
                    (code_name(code))
                        .and_then(|n| names.iter().find(|(_, m)| m == n))
                        .map_or(GlyphId::NOTDEF, |(g, _)| *g)
                })
                .collect();
        }
        let Some(font) = self.program.font() else {
            return vec![GlyphId::NOTDEF; 256];
        };
        let cmap = font.cmap().ok();
        let records: Vec<_> = (cmap.iter())
            .flat_map(|c| c.encoding_records().iter().map(move |r| (c, r)))
            .filter_map(|(c, r)| {
                let sub = r.subtable(c.offset_data()).ok()?;
                Some((r.platform_id(), r.encoding_id(), sub))
            })
            .collect();
        let post: HashMap<&str, GlyphId> = (self.post().iter())
            .map(|(g, n)| (n.as_str(), *g))
            .collect();
        (0..=255u8)
            .map(|code| {
                let mut glyph = None;
                if nonsymbolic {
                    let Some(name) = code_name(code) else {
                        return GlyphId::NOTDEF;
                    };
                    for (p, e, sub) in &records {
                        if *p == PlatformId::Windows && *e == 1 {
                            glyph = glyph
                                .or_else(|| name_char(name).and_then(|c| sub.map_codepoint(c)))
                                .filter(nonzero);
                        }
                    }
                    for (p, e, sub) in &records {
                        if *p == PlatformId::Macintosh && *e == 0 {
                            glyph = glyph
                                .or_else(|| mac_code(name).and_then(|c| sub.map_codepoint(c)))
                                .filter(nonzero);
                        }
                    }
                    if glyph.is_none() {
                        glyph = post.get(name).copied().or_else(|| {
                            let n = name.strip_prefix('g')?.parse::<u32>().ok()?;
                            Some(GlyphId::new(n))
                        });
                    }
                } else {
                    for (p, e, sub) in &records {
                        if *p == PlatformId::Windows && matches!(*e, 0 | 1) {
                            for at in [0, 0xF000, 0xF100, 0xF200] {
                                glyph = glyph
                                    .or_else(|| sub.map_codepoint(u32::from(code) + at))
                                    .filter(nonzero);
                            }
                        } else if matches!(p, PlatformId::Macintosh | PlatformId::Unicode)
                            && *e == 0
                        {
                            glyph = glyph.or_else(|| sub.map_codepoint(code)).filter(nonzero);
                        }
                    }
                }
                glyph.unwrap_or(GlyphId::NOTDEF)
            })
            .collect()
    }

    fn post(&self) -> &[(GlyphId, String)] {
        self.post.get_or_init(|| self.program.post_names())
    }

    /// The glyph code `code` draws.
    fn glyph(&self, code: u32) -> GlyphId {
        match self.two {
            true => cid_glyph(&self.program, &self.cids, code),
            false => (self.glyphs.get(code as usize).copied()).unwrap_or(GlyphId::NOTDEF),
        }
    }

    /// What the program says glyph `gid` is: the characters its Unicode cmap maps to it,
    /// else its name's text; empty when it doesn't say.
    fn says(&self, gid: GlyphId) -> Vec<String> {
        let chars = self.chars.get_or_init(|| self.program.cmap_chars());
        match chars.get(&gid) {
            Some(cs) => cs.iter().map(char::to_string).collect(),
            None => (self.program.name(gid, self.post()))
                .and_then(|n| glyph_text(&n))
                .into_iter()
                .collect(),
        }
    }

    /// The codes that pass every check for `c`, with their glyphs, or why none does: of
    /// several failing, the reason the last one failed for.
    fn check(&self, c: char) -> Checked {
        let mut checked = self.checked.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(r) = checked.get(&c) {
            return r.clone();
        }
        let r = self.verify(c);
        checked.insert(c, r.clone());
        r
    }

    fn verify(&self, c: char) -> Checked {
        use CharRefusal::*;
        let Some(codes) = self.by_char.get(&c) else {
            return Err(NoCode);
        };
        let blank = c.is_whitespace();
        let count = self.program.num_glyphs();
        let (mut why, mut found) = (NoCode, Vec::new());
        for &(code, width) in codes {
            if self.disagree.contains(&code) {
                why = Encoding;
                continue;
            }
            let gid = self.glyph(code);
            if gid == GlyphId::NOTDEF || gid.to_u32() >= count {
                why = NoGlyph;
                continue;
            }
            let (draws, advance) = self.program.draw(gid);
            if !draws && !blank {
                why = NoGlyph;
                continue;
            }
            let Some(w) = width else {
                why = Width;
                continue;
            };
            let off = match advance {
                Some(a) => (w - a).abs() > (a.abs() * 0.02).max(2.0),
                None => w <= 0.0 && !blank,
            };
            if off {
                why = Width;
                continue;
            }
            let says = self.says(gid);
            if !says.is_empty() && !says.iter().any(|t| t.chars().eq([c])) {
                why = Contradicted;
                continue;
            }
            found.push((code, gid));
        }
        if found.is_empty() {
            Err(why)
        } else {
            Ok(found)
        }
    }
}

/// An encoding dictionary's `/Differences`, as hayro reads them: each name at its code, a
/// number setting the next code.
fn differences(doc: &Document, e: &Dictionary) -> HashMap<u8, String> {
    let mut out = HashMap::new();
    let list = (e.get(b"Differences").ok())
        .and_then(|o| doc.dereference(o).ok())
        .and_then(|(_, o)| o.as_array().ok());
    let mut code = 0i64;
    for o in list.into_iter().flatten() {
        match doc.dereference(o).map(|(_, o)| o) {
            Ok(Object::Integer(n)) => code = *n,
            Ok(Object::Real(n)) => code = *n as i64,
            Ok(Object::Name(n)) => {
                if let Ok(c) = u8::try_from(code) {
                    out.insert(c, String::from_utf8_lossy(n).into_owned());
                }
                code += 1;
            }
            _ => {}
        }
    }
    out
}

/// A simple font's width for each code, as hayro reads them: given from `/FirstChar` to
/// `/LastChar` by `/Widths` (all three needed), `None` elsewhere.
fn simple_widths(doc: &Document, dict: &Dictionary) -> Vec<Option<f32>> {
    let mut out = vec![None; 256];
    let (Some(first), Some(last)) = (
        number(doc, dict, b"FirstChar"),
        number(doc, dict, b"LastChar"),
    ) else {
        return out;
    };
    let widths = (dict.get(b"Widths").ok())
        .and_then(|o| doc.dereference(o).ok())
        .and_then(|(_, o)| o.as_array().ok());
    let (Some(widths), true) = (widths, first >= 0.0 && last >= first) else {
        return out;
    };
    let count = (last - first) as usize + 1;
    for (i, w) in widths.iter().take(count).enumerate() {
        let code = first as usize + i;
        if let (Some(slot), Ok((_, w))) = (out.get_mut(code), doc.dereference(w)) {
            *slot = w.as_float().ok();
        }
    }
    out
}

/// The text of an Identity CID font's glyphs from its program (PB-26): each glyph's
/// character by the program's Unicode cmap (the lowest outside the private use areas when
/// it has several), else by its glyph name. `None` when the program says nothing: a
/// CID-keyed CFF program names no glyphs.
pub(crate) struct CidText {
    program: Program,
    cids: CidMap,
    text: HashMap<GlyphId, String>,
}

impl CidText {
    /// The text of CID `cid`, if its glyph has any.
    pub(crate) fn get(&self, cid: u32) -> Option<&str> {
        let gid = cid_glyph(&self.program, &self.cids, cid);
        self.text.get(&gid).map(String::as_str)
    }
}

/// Most programs whose glyphs' text is kept between pages.
const KEPT: usize = 64;

/// [`CidText`] of CIDFont `desc`, kept between calls for the same program and map.
pub(crate) fn cid_text(doc: &Document, desc: &Dictionary) -> Option<Arc<CidText>> {
    static KEEP: LazyLock<Mutex<HashMap<u64, Option<Arc<CidText>>>>> =
        LazyLock::new(Default::default);
    let fd = sub_dict(doc, desc, b"FontDescriptor")?;
    let keys: [&[u8]; 3] = [b"FontFile2", b"FontFile3", b"FontFile"];
    let (_, stream) = program_stream(doc, fd, &keys)?;
    let mut h = DefaultHasher::new();
    stream.content.hash(&mut h);
    let map = (desc.get(b"CIDToGIDMap").ok()).and_then(|o| doc.dereference(o).ok());
    match map {
        Some((_, Object::Stream(s))) => s.content.hash(&mut h),
        Some((_, o)) => format!("{o:?}").hash(&mut h),
        None => {}
    }
    let key = h.finish();
    if let Some(t) = KEEP.lock().unwrap_or_else(|e| e.into_inner()).get(&key) {
        return t.clone();
    }
    let text = (stream.get_plain_content_with_limit(MAX_PROGRAM).ok())
        .and_then(Program::any)
        .and_then(|program| {
            let pua = |c: char| {
                matches!(c, '\u{E000}'..='\u{F8FF}' | '\u{F0000}'..='\u{FFFFD}' | '\u{100000}'..='\u{10FFFD}')
            };
            let mut text: HashMap<GlyphId, String> = HashMap::new();
            for (gid, chars) in program.cmap_chars() {
                let best = (chars.iter().filter(|c| !pua(**c)).min())
                    .or_else(|| chars.iter().min());
                text.extend(best.map(|c| (gid, c.to_string())));
            }
            let post = program.post_names();
            for g in 0..program.num_glyphs() {
                let gid = GlyphId::new(g);
                if !text.contains_key(&gid) {
                    let t = program.name(gid, &post).and_then(|n| glyph_text(&n));
                    text.extend(t.map(|t| (gid, t)));
                }
            }
            let cids = CidMap::of(doc, desc);
            (!text.is_empty()).then(|| Arc::new(CidText { program, cids, text }))
        });
    let mut keep = KEEP.lock().unwrap_or_else(|e| e.into_inner());
    if keep.len() >= KEPT {
        keep.clear();
    }
    keep.insert(key, text.clone());
    text
}

#[cfg(test)]
mod tests {
    use super::*;
    use hayro::hayro_interpret::font::{Glyph, GlyphRun};
    use hayro::hayro_interpret::{
        interpret_page, BlendMode, ClipPath, Context, Device, DrawMode, DrawProps, Image,
        ImageDrawProps, InterpreterCache, InterpreterSettings, SoftMask,
    };
    use hayro::hayro_syntax::Pdf as HPdf;
    use hayro::kurbo::{Affine, BezPath, Rect};
    use lopdf::dictionary;

    const FONTS: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/data/fonts/");

    /// A device that notes the glyph of each outline glyph drawn, in order.
    struct Drawn(Vec<u32>);

    impl<'a> Device<'a> for Drawn {
        fn draw_path(&mut self, _: &BezPath, _: DrawProps<'a>, _: &DrawMode) {}
        fn push_clip_path(&mut self, _: &ClipPath) {}
        fn push_transparency_group(&mut self, _: f32, _: Option<SoftMask<'a>>, _: BlendMode) {}
        fn draw_glyph_run(&mut self, run: &GlyphRun<'_, 'a>, _: DrawProps<'a>, _: &DrawMode) {
            for g in run.glyphs() {
                if let Glyph::Outline(o) = &**g {
                    self.0.push(o.glyph_id().to_u32());
                }
            }
        }
        fn draw_image(&mut self, _: Image<'a, '_>, _: ImageDrawProps<'a>) {}
        fn pop_clip(&mut self) {}
        fn pop_transparency_group(&mut self) {}
    }

    /// The glyph of each of `codes` (two bytes each when `two`) in font `font` as hayro draws
    /// it, on a page showing them, and as [`OwnFont`] finds it.
    fn both(mut p: Pdf, font: Dictionary, codes: &[u32], two: bool) -> (Vec<u32>, Vec<u32>) {
        let font = p.doc.add_object(font);
        p.insert_blank(0, 300.0, 200.0).unwrap();
        let hex: String = (codes.iter())
            .map(|c| match two {
                true => format!("{c:04X}"),
                false => format!("{c:02X}"),
            })
            .collect();
        let content = format!("BT /F1 10 Tf 10 100 Td <{hex}> Tj ET").into_bytes();
        let s = p.doc.add_object(Stream::new(Dictionary::new(), content));
        let page = p.page_ids()[0];
        let d = p.doc.get_dictionary_mut(page).unwrap();
        d.set("Contents", s);
        d.set(
            "Resources",
            dictionary! { "Font" => dictionary! { "F1" => font } },
        );
        let dict = p.doc.get_dictionary(font).unwrap();
        let own = OwnFont::read(&p.doc, dict, &FontCtx::new(&p.doc, dict))
            .ok()
            .unwrap();
        let ours = codes.iter().map(|&c| own.glyph(c).to_u32()).collect();
        let hayro = HPdf::new(p.to_bytes().unwrap()).unwrap();
        let page = hayro.pages().first().unwrap();
        let cache = InterpreterCache::new();
        let full = Rect::new(0.0, 0.0, 300.0, 200.0);
        let settings = InterpreterSettings::default();
        let mut ctx = Context::new(Affine::IDENTITY, full, &cache, page.xref(), settings);
        let mut drawn = Drawn(Vec::new());
        interpret_page(page, &mut ctx, &mut drawn);
        (drawn.0, ours)
    }

    /// A simple font of `subtype` with `program` under `key` (its stream's entries `extra`),
    /// descriptor flags `flags` and `/Encoding` `encoding`, in `p`.
    fn simple(
        p: &mut Pdf,
        subtype: &str,
        (key, extra, program): (&str, Dictionary, Vec<u8>),
        flags: i64,
        encoding: Option<Object>,
    ) -> Dictionary {
        let program = p.doc.add_object(Stream::new(extra, program));
        let fd = p.doc.add_object(dictionary! {
            "Type" => "FontDescriptor", "FontName" => "ABCDEF+Test", "Flags" => flags,
            key => program,
        });
        let mut font = dictionary! {
            "Type" => "Font", "Subtype" => subtype, "BaseFont" => "ABCDEF+Test",
            "FontDescriptor" => fd,
        };
        if let Some(e) = encoding {
            font.set("Encoding", e);
        }
        font
    }

    /// A Type 0 font with Identity-H over a CIDFont of `subtype` with `program` under `key`,
    /// and `/CIDToGIDMap` `map` if any, in `p`.
    fn type0(
        p: &mut Pdf,
        subtype: &str,
        (key, program): (&str, Vec<u8>),
        map: Option<Object>,
    ) -> Dictionary {
        let program = p.doc.add_object(Stream::new(Dictionary::new(), program));
        let fd = p.doc.add_object(dictionary! {
            "Type" => "FontDescriptor", "FontName" => "ABCDEF+Test", "Flags" => 4, key => program,
        });
        let mut cid = dictionary! {
            "Type" => "Font", "Subtype" => subtype, "BaseFont" => "ABCDEF+Test",
            "FontDescriptor" => fd, "DW" => 1000,
            "CIDSystemInfo" => dictionary! {
                "Registry" => Object::string_literal("Adobe"),
                "Ordering" => Object::string_literal("Identity"), "Supplement" => 0,
            },
        };
        if let Some(m) = map {
            cid.set("CIDToGIDMap", m);
        }
        let cid = p.doc.add_object(cid);
        dictionary! {
            "Type" => "Font", "Subtype" => "Type0", "BaseFont" => "ABCDEF+Test",
            "Encoding" => "Identity-H", "DescendantFonts" => vec![cid.into()],
        }
    }

    fn file(name: &str) -> Vec<u8> {
        std::fs::read(format!("{FONTS}{name}")).unwrap()
    }

    fn sans() -> Vec<u8> {
        crate::bundled_face("Liberation Sans").unwrap().to_vec()
    }

    /// Every code of a simple font, each drawn alone where hayro draws the glyph found.
    fn every_simple_code(
        subtype: &str,
        program: (&str, Dictionary, Vec<u8>),
        flags: i64,
        e: Option<Object>,
    ) {
        let mut p = Pdf::new();
        let font = simple(&mut p, subtype, program, flags, e);
        let codes: Vec<u32> = (1..=255).collect();
        let (hayro, ours) = both(p, font, &codes, false);
        assert_eq!(hayro.len(), codes.len(), "hayro drew {hayro:?}");
        for ((code, h), o) in codes.iter().zip(&hayro).zip(&ours) {
            assert_eq!(h, o, "code {code:#04x}: hayro {h}, LimarePDF {o}");
        }
    }

    /// An encoding over WinAnsi with differences named in each way hayro reads names.
    fn win_ansi_with_differences() -> Option<Object> {
        let names = "Aring g90 uni0416 a.sc nbspace sfthyphen nosuchglyph".split(' ');
        let mut list = vec![Object::Integer(0x80)];
        list.extend(names.map(|n| Object::Name(n.as_bytes().to_vec())));
        Some(Object::Dictionary(dictionary! {
            "Type" => "Encoding", "BaseEncoding" => "WinAnsiEncoding", "Differences" => list,
        }))
    }

    #[test]
    fn simple_fonts_find_each_code_s_glyph_as_hayro_draws_it() {
        let ttf = |data| ("FontFile2", Dictionary::new(), data);
        let win = || Some(Object::Name(b"WinAnsiEncoding".to_vec()));
        let mac = || Some(Object::Name(b"MacRomanEncoding".to_vec()));
        // TrueType, nonsymbolic: by the encoding's names, through (3,1) then (1,0) cmaps.
        every_simple_code("TrueType", ttf(sans()), 32, win());
        every_simple_code("TrueType", ttf(sans()), 32, mac());
        every_simple_code("TrueType", ttf(sans()), 32, win_ansi_with_differences());
        every_simple_code("TrueType", ttf(sans()), 32, None);
        // Symbolic, or with a flag the specification doesn't define: by code.
        every_simple_code("TrueType", ttf(sans()), 4, win());
        every_simple_code("TrueType", ttf(sans()), 32 | 16, win());
        every_simple_code("TrueType", ttf(file("LimareSymbolic.ttf")), 4, None);
        // CFF outlines in /FontFile2: by name in its charset.
        every_simple_code("TrueType", ttf(file("SourceSerif4-Regular.otf")), 32, win());
        // Bare CFF and Type 1: their own encoding, or the PDF's names.
        let type1c = || dictionary! { "Subtype" => "Type1C" };
        let cff = || ("FontFile3", type1c(), file("LimareOwnEncoding.cff"));
        every_simple_code("Type1", cff(), 4, None);
        every_simple_code("Type1", cff(), 4, win_ansi_with_differences());
        let pfa = file("LimareOwnEncoding.pfa");
        let lengths = dictionary! { "Length1" => pfa.len() as i64, "Length2" => 0, "Length3" => 0 };
        every_simple_code("Type1", ("FontFile", lengths.clone(), pfa.clone()), 4, None);
        let named = win_ansi_with_differences();
        every_simple_code("Type1", ("FontFile", lengths, pfa), 4, named);
    }

    #[test]
    fn cid_fonts_find_each_cid_s_glyph_as_hayro_draws_it() {
        let codes: Vec<u32> = (1..=700).collect();
        let check = |subtype: &str, program: (&str, Vec<u8>), map: Option<Object>| {
            let mut p = Pdf::new();
            let font = type0(&mut p, subtype, program, map);
            let (hayro, ours) = both(p, font, &codes, true);
            assert_eq!(hayro.len(), codes.len());
            for ((code, h), o) in codes.iter().zip(&hayro).zip(&ours) {
                assert_eq!(h, o, "CID {code}: hayro draws {h}, LimarePDF finds {o}");
            }
        };
        let identity = || Some(Object::Name(b"Identity".to_vec()));
        check("CIDFontType2", ("FontFile2", sans()), identity());
        check("CIDFontType2", ("FontFile2", sans()), None);
        // A map from CIDs to glyphs, here backwards from 2047.
        let mut p = Pdf::new();
        let gids: Vec<u8> = (0..2048u16).rev().flat_map(|g| g.to_be_bytes()).collect();
        let map = p.doc.add_object(Stream::new(Dictionary::new(), gids));
        let map = Some(map.into());
        let font = type0(&mut p, "CIDFontType2", ("FontFile2", sans()), map);
        let (hayro, ours) = both(p, font, &codes, true);
        assert_eq!(hayro, ours);
        let otf = file("SourceSerif4-Regular.otf");
        check("CIDFontType0", ("FontFile3", otf), None);
    }

    #[test]
    fn flags_say_nonsymbolic_as_hayro_reads_them() {
        assert!(nonsymbolic(Some(32)));
        assert!(nonsymbolic(Some(32 | 2 | 1 << 18)));
        assert!(!nonsymbolic(Some(4)));
        assert!(!nonsymbolic(None));
        // A flag the specification doesn't define leaves hayro's flags unread: symbolic.
        assert!(!nonsymbolic(Some(32 | 16)));
        assert!(!nonsymbolic(Some(-1)));
    }

    #[test]
    fn glyph_names_give_a_truetype_cmap_s_character_as_hayro_reads_them() {
        assert_eq!(name_char("A"), Some('A'));
        assert_eq!(name_char("uni00E9"), Some('é'));
        assert_eq!(name_char("u1F600"), Some('😀'));
        // Unlike LimarePDF's reading of names, no suffix is left out.
        assert_eq!(name_char("a.sc"), None);
        assert_eq!(name_char("unixyz"), None);
    }

    /// The box around everything a pen draws.
    #[derive(Default)]
    struct Bounds(Option<[f32; 4]>);

    impl Bounds {
        fn add(&mut self, x: f32, y: f32) {
            let b = self.0.get_or_insert([x, y, x, y]);
            *b = [b[0].min(x), b[1].min(y), b[2].max(x), b[3].max(y)];
        }
    }

    impl OutlinePen for Bounds {
        fn move_to(&mut self, x: f32, y: f32) {
            self.add(x, y);
        }
        fn line_to(&mut self, x: f32, y: f32) {
            self.add(x, y);
        }
        fn quad_to(&mut self, a: f32, b: f32, x: f32, y: f32) {
            self.add(a, b);
            self.add(x, y);
        }
        fn curve_to(&mut self, a: f32, b: f32, c: f32, d: f32, x: f32, y: f32) {
            self.add(a, b);
            self.add(c, d);
            self.add(x, y);
        }
        fn close(&mut self) {}
    }

    /// A line's preview font holds the characters the font can write, each drawn as the
    /// program draws its glyph, at the PDF's width (here 1% more than the program's, which
    /// the checks allow), and nothing the font has no code for.
    #[test]
    fn a_preview_font_draws_the_program_s_glyphs_at_the_pdf_s_widths() {
        let mut p = Pdf::new();
        p.insert_blank(0, 300.0, 200.0).unwrap();
        let program = ("FontFile2", Dictionary::new(), sans());
        let encoding = Some("WinAnsiEncoding".into());
        let mut font = simple(&mut p, "TrueType", program, 32, encoding);
        let pdf_width = |c: char| crate::text_width(&c.to_string(), 1000.0) * 1.01;
        let widths: Vec<Object> = (32u8..=126).map(|b| pdf_width(b as char).into()).collect();
        font.set("FirstChar", 32);
        font.set("LastChar", 126);
        font.set("Widths", widths);
        let font = p.doc.add_object(font);
        let content = b"BT /F1 10 Tf 10 100 Td (Walrus) Tj ET".to_vec();
        let s = p.doc.add_object(Stream::new(Dictionary::new(), content));
        let page = p.page_ids()[0];
        let d = p.doc.get_dictionary_mut(page).unwrap();
        d.set("Contents", s);
        d.set(
            "Resources",
            dictionary! { "Font" => dictionary! { "F1" => font } },
        );
        let fonts = p.line_fonts(0, 0).unwrap();
        let bytes = fonts[0].preview().unwrap();
        let preview = FontRef::new(&bytes).unwrap();
        let sans = sans();
        let program = FontRef::new(&sans).unwrap();
        let unscaled = preview.glyph_metrics(Size::unscaled(), LocationRef::default());
        for c in ['W', 'a', 'z', '!', '7'] {
            let gid = preview.charmap().map(c).unwrap_or_else(|| panic!("{c}"));
            let want = Some(pdf_width(c).round());
            assert_eq!(unscaled.advance_width(gid), want, "{c}");
            let drawn = |f: &FontRef, gid, size| {
                let mut b = Bounds::default();
                let at = DrawSettings::unhinted(size, LocationRef::default());
                let outline = f.outline_glyphs().get(gid).unwrap();
                outline.draw(at, &mut b).unwrap();
                b.0.unwrap()
            };
            let at = program.charmap().map(c).unwrap();
            let own = drawn(&program, at, Size::new(1000.0));
            let shown = drawn(&preview, gid, Size::unscaled());
            for (a, b) in own.iter().zip(shown) {
                assert!((a - b).abs() <= 1.5, "{c}: {own:?} vs {shown:?}");
            }
        }
        assert_eq!(preview.charmap().map('Ω'), None);
        // One font on lines showing the same codes has one preview.
        let again = p.line_fonts(0, 0).unwrap();
        assert_eq!(fonts[0].preview_key(), again[0].preview_key());
    }
}

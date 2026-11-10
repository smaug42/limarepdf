//! Text from what is typed to what is written (the text design, NEW-02): the font each word
//! is set in, falling back where a font lacks a character; shaping; widths; line breaks;
//! and the shows that write it. Every writer sets text through it, so what is measured is
//! what is written.
//!
//! A [`Setter`] measures and breaks text without changing the document. [`Pdf::pen`] then
//! embeds what the lines a writer will show need, once, and gives a [`Pen`] that writes
//! exactly those lines, so no glyph is written that wasn't embedded.
//!
//! Each word, as Unicode's word rule finds it (UAX #29, so each Chinese or Japanese
//! ideograph is one), is set in the first font that has all of it: the font asked for; a
//! font for its script, installed (for Chinese, Japanese and Korean by the document's
//! language, else the text's, else the system's); the asked font's standard stand-in; any
//! installed family that has it. Else it is refused. A space between two words that fall
//! back to one font goes with them. New text in an embedded face is shaped with the font's
//! own features (kerning, ligatures, marks); text in a standard font itself is shown in
//! WinAnsi codes, kerned as its bundled face kerns it.

use crate::embed::{self, emoji_sequence, Face, Placed, Shaped};
use crate::fonts::winansi_byte;
use crate::installed_fonts::{maps_all, Language};
use crate::{std14, Family, FontChoice, Pdf, Result};
use anyhow::bail;
use lopdf::content::Operation;
use lopdf::{dictionary, Object, ObjectId, StringFormat};
use std::borrow::Borrow;
use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::hash::Hash;
use std::ops::Range;
use std::rc::Rc;
use std::sync::{LazyLock, Mutex, OnceLock};
use unicode_normalization::UnicodeNormalization;
use unicode_script::{Script, UnicodeScript};
use unicode_segmentation::UnicodeSegmentation;

/// The font a stretch of text is set in.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum SetIn {
    /// A standard font itself, nothing embedded: each character its WinAnsi code, kerned
    /// as its bundled face kerns it.
    Standard(FontChoice),
    /// The embedded face of a standard font (its bundled one) or of an installed family.
    Face(FontChoice),
}

impl SetIn {
    /// `f` as it is written when it can write all it sets: a standard font as itself, an
    /// installed family's face embedded.
    pub fn of(f: FontChoice) -> SetIn {
        match f.is_standard() {
            true => SetIn::Standard(f),
            false => SetIn::Face(f),
        }
    }

    /// The font chosen, standard or installed.
    pub fn choice(self) -> FontChoice {
        match self {
            SetIn::Standard(f) | SetIn::Face(f) => f,
        }
    }
}

/// Text as it is set ([`Setter::lay`]): stretches, each in one font and one script.
#[derive(Clone, Debug, Default)]
pub(crate) struct Laid {
    pub runs: Vec<Run>,
}

/// A stretch of text set in one font and one script.
#[derive(Clone, Debug)]
pub(crate) struct Run {
    pub font: SetIn,
    /// What it writes, which its glyphs' characters index: its characters put together as
    /// the face writes them ([`Face::spelled`]).
    pub text: Vec<char>,
    /// For each of `text`, the characters of the text set ([`Setter::lay`]'s) it came from,
    /// from and to.
    pub from: Vec<(usize, usize)>,
    /// Its glyphs; in a standard font itself, each glyph is a character's WinAnsi code.
    pub glyphs: Vec<Shaped>,
}

impl Laid {
    /// How wide it is at `size`, in points.
    pub(crate) fn width(&self, size: f32) -> f32 {
        let units: f32 = (self.runs.iter())
            .flat_map(|r| &r.glyphs)
            .map(|g| g.advance)
            .sum();
        units * size / 1000.0
    }

    /// Its glyph clusters in order, what a line may not break inside: each one's characters
    /// in the text set (from and to) and its advance, in thousandths of the size. A cluster
    /// is a glyph that draws characters and the glyphs after it that draw none (a mark
    /// placed apart, say).
    pub(crate) fn clusters(&self) -> Vec<((usize, usize), f32)> {
        let mut out: Vec<((usize, usize), f32)> = Vec::new();
        for run in &self.runs {
            for g in &run.glyphs {
                let (a, b) = g.chars;
                match out.last_mut() {
                    Some(last) if b <= a => last.1 += g.advance,
                    _ if b <= a => {}
                    _ => out.push(((run.from[a].0, run.from[b - 1].1), g.advance)),
                }
            }
        }
        out
    }

    /// How many codes it is written in: what character spacing is added for, once each.
    pub(crate) fn codes(&self) -> usize {
        self.runs.iter().map(|r| r.glyphs.len()).sum()
    }
}

/// Sets text asked for in one font ([`Setter::new`]): which font each word goes to, how it
/// is shaped, how wide it is and where its lines break. Nothing in the document changes.
pub(crate) struct Setter {
    asked: FontChoice,
    /// Whether the asked font is a standard one written as itself, every character of the
    /// text fitting WinAnsi (the Unicode text design, question 2); otherwise its embedded
    /// face.
    standard: bool,
    /// The document's language, for shaping.
    language: Option<harfrust::Language>,
    /// Which of Chinese, Japanese and Korean the text is, when something says so.
    cjk: Option<Language>,
    /// What it set, by the text: each line tried while breaking among them.
    laid: RefCell<Kept<String, Rc<Laid>, LAID_MOST, LAID_CHARS>>,
}

/// The font each word went to, by the font asked, whether that is written as itself, the
/// language for Chinese, Japanese and Korean, and the word; kept while the installed
/// families stay as they are (their count).
type Units = (usize, Kept<UnitKey, Option<SetIn>, UNITS_MOST, UNITS_BYTES>);

/// What a word's font is kept by ([`Units`]).
type UnitKey = (FontChoice, bool, Option<Language>, String);

static UNITS: LazyLock<Mutex<Units>> = LazyLock::new(Mutex::default);

/// The most words [`UNITS`] keeps, and the most bytes of their text.
const UNITS_MOST: usize = 1 << 16;
const UNITS_BYTES: usize = 4 << 20;

/// The most texts a [`Setter`] keeps set, and the most characters among them.
const LAID_MOST: usize = 1 << 12;
const LAID_CHARS: usize = 1 << 20;

/// What is kept to use again, at most `MOST` entries and `SIZE` among them (M-08): emptied
/// before one more would pass either, as the streams found fine are (`unfillable.rs`'s
/// `Checked`). An entry bigger than `SIZE` alone isn't kept.
struct Kept<K, V, const MOST: usize, const SIZE: usize> {
    map: HashMap<K, V>,
    /// What the entries kept hold, as [`Kept::keep`] was told.
    size: usize,
}

impl<K, V, const MOST: usize, const SIZE: usize> Default for Kept<K, V, MOST, SIZE> {
    fn default() -> Self {
        Self {
            map: HashMap::new(),
            size: 0,
        }
    }
}

impl<K: Eq + Hash, V, const MOST: usize, const SIZE: usize> Kept<K, V, MOST, SIZE> {
    fn get<Q: Eq + Hash + ?Sized>(&self, key: &Q) -> Option<&V>
    where
        K: Borrow<Q>,
    {
        self.map.get(key)
    }

    /// Keeps `value` by `key`, `size` being what it holds.
    fn keep(&mut self, key: K, value: V, size: usize) {
        if size > SIZE {
            return;
        }
        if self.map.len() >= MOST || self.size + size > SIZE {
            self.map.clear();
            self.size = 0;
        }
        if self.map.insert(key, value).is_none() {
            self.size += size;
        }
    }
}

impl Setter {
    /// A setter for text in `f`, `text` being all it will set (whether a standard font is
    /// written as itself depends on all of it), in a document whose language is `lang`
    /// (its `/Lang`), when known.
    pub(crate) fn new(f: FontChoice, text: &str, lang: Option<&str>) -> Self {
        let standard = f.is_standard()
            && (text.chars())
                .filter(|c| !c.is_control())
                .all(|c| winansi_byte(c).is_some());
        let cjk = (lang.and_then(Language::of_tag))
            .or_else(|| text_language(text))
            .or_else(system_language);
        Self {
            asked: f,
            standard,
            language: lang.and_then(|l| l.parse().ok()),
            cjk,
            laid: RefCell::default(),
        }
    }

    /// The font asked for, as it is written.
    pub(crate) fn own(&self) -> SetIn {
        match self.standard {
            true => SetIn::Standard(self.asked),
            false => SetIn::Face(self.asked),
        }
    }

    /// Why the font asked for can't be written at all (an installed font that can't be
    /// read, say), when it can't.
    pub(crate) fn unusable(&self) -> Option<String> {
        match self.own() {
            SetIn::Face(f) => Face::of(f).err().map(|e| e.to_string()),
            SetIn::Standard(_) => None,
        }
    }

    /// `text` set: each word in its font, shaped. Characters no font can write
    /// ([`Setter::missing`]) are left out.
    pub(crate) fn lay(&self, text: &str) -> Rc<Laid> {
        if let Some(l) = self.laid.borrow().get(text) {
            return Rc::clone(l);
        }
        let laid = Rc::new(self.lay_anew(text));
        let size = text.chars().count();
        (self.laid.borrow_mut()).keep(text.to_string(), Rc::clone(&laid), size);
        laid
    }

    fn lay_anew(&self, text: &str) -> Laid {
        let chars: Vec<char> = text.chars().collect();
        let pieces = self.pieces(text);
        let mut runs = Vec::new();
        let mut i = 0;
        while i < pieces.len() {
            let Some(font) = pieces[i].1 else {
                i += 1;
                continue;
            };
            let from = pieces[i].0 .0;
            let mut to = pieces[i].0 .1;
            while i + 1 < pieces.len() && pieces[i + 1].1 == Some(font) {
                i += 1;
                to = pieces[i].0 .1;
            }
            i += 1;
            for (a, b, script) in items(&chars[from..to]) {
                let mut run = self.run(font, &chars[from + a..from + b], script);
                for r in &mut run.from {
                    *r = (r.0 + from + a, r.1 + from + a);
                }
                runs.push(run);
            }
        }
        Laid { runs }
    }

    /// `text`'s words and the spaces between, each with the font it goes to (`None` for
    /// what no font can write): its characters, from and to.
    fn pieces(&self, text: &str) -> Vec<((usize, usize), Option<SetIn>)> {
        let mut pieces: Vec<((usize, usize), Option<SetIn>, bool)> = Vec::new();
        let mut at = 0;
        for unit in text.split_word_bounds() {
            let n = unit.chars().count();
            let space = unit.chars().all(char::is_whitespace);
            let font = (!space).then(|| self.unit_font(unit)).flatten();
            pieces.push(((at, at + n), font, space));
            at += n;
        }
        let words: Vec<Option<SetIn>> = pieces.iter().map(|p| p.1).collect();
        let own = self.own();
        let chars: Vec<char> = text.chars().collect();
        for i in 0..pieces.len() {
            if !pieces[i].2 {
                continue;
            }
            let near = |mut range: Box<dyn Iterator<Item = usize>>| {
                range.find(|&k| !pieces[k].2).and_then(|k| words[k])
            };
            let before = near(Box::new((0..i).rev()));
            let after = near(Box::new(i + 1..pieces.len()));
            let (a, b) = pieces[i].0;
            let space: String = chars[a..b].iter().collect();
            pieces[i].1 = match (before, after) {
                (Some(x), Some(y)) if x == y => Some(x),
                _ if covers(own, &space) => Some(own),
                _ => [before, after]
                    .into_iter()
                    .flatten()
                    .find(|&f| covers(f, &space)),
            };
        }
        pieces.into_iter().map(|(r, f, _)| (r, f)).collect()
    }

    /// The font a word goes to: the first in the order of fonts tried that has all of it.
    fn unit_font(&self, unit: &str) -> Option<SetIn> {
        let key = (self.asked, self.standard, self.cjk, unit.to_string());
        let count = crate::typefaces::typeface_count();
        {
            let mut units = UNITS.lock().unwrap_or_else(|e| e.into_inner());
            if units.0 != count {
                *units = (count, Kept::default());
            }
            if let Some(f) = units.1.get(&key) {
                return *f;
            }
        }
        let found = self.find_font(unit);
        let mut units = UNITS.lock().unwrap_or_else(|e| e.into_inner());
        units.1.keep(key, found, unit.len());
        found
    }

    fn find_font(&self, unit: &str) -> Option<SetIn> {
        let own = self.own();
        if covers(own, unit) {
            return Some(own);
        }
        let installed = |f: FontChoice| could(f, unit) && covers(SetIn::Face(f), unit);
        if let Some(f) = self.script_fonts(unit).into_iter().find(|&f| installed(f)) {
            return Some(SetIn::Face(f));
        }
        let stand_in = SetIn::Face(self.asked.standard());
        if !self.asked.is_standard() && covers(stand_in, unit) {
            return Some(stand_in);
        }
        (crate::typefaces::every_typeface().into_iter())
            .map(|(n, t)| self.styled(n, &t))
            .find(|&f| installed(f))
            .map(SetIn::Face)
    }

    /// The installed family `n`'s face nearest the asked font's style.
    fn styled(&self, n: u16, t: &crate::Typeface) -> FontChoice {
        let (bold, italic) = t.nearest(self.asked.bold, self.asked.italic);
        FontChoice {
            family: Family::Installed(n),
            bold,
            italic,
        }
    }

    /// The installed fonts usual for the script of `unit`, best first, in the asked font's
    /// kind (sans, serif or monospaced) and its nearest style.
    fn script_fonts(&self, unit: &str) -> Vec<FontChoice> {
        let kind = self.asked.standard().family;
        let serif = kind == Family::Times;
        let mut names: Vec<String> = Vec::new();
        match unit_script(unit) {
            Kind::Cjk => {
                let first = self.cjk.into_iter().chain(text_language(unit));
                for language in first.chain(Language::ALL) {
                    for s in [serif, !serif] {
                        names.extend(language.families(s).iter().map(|n| n.to_string()));
                    }
                }
            }
            Kind::Emoji => names.extend(EMOJI.iter().map(|n| n.to_string())),
            Kind::Script(Script::Latin | Script::Greek | Script::Cyrillic) => {
                let list = match kind {
                    Family::Times => ["Noto Serif", "DejaVu Serif"],
                    Family::Courier => ["Noto Sans Mono", "DejaVu Sans Mono"],
                    _ => ["Noto Sans", "DejaVu Sans"],
                };
                names.extend(list.iter().map(|n| n.to_string()));
            }
            Kind::Script(s) => {
                let name = s.full_name().replace('_', " ");
                let style = if serif { "Serif" } else { "Sans" };
                names.push(format!("Noto {style} {name}"));
                names.push(format!("Noto Sans {name}"));
                names.extend(["Noto Sans", "DejaVu Sans"].map(String::from));
            }
            Kind::Symbol => {
                names.extend(SYMBOLS.iter().map(|n| n.to_string()));
            }
        }
        let mut out: Vec<FontChoice> = Vec::new();
        for name in names {
            let Some(n) = crate::typefaces::family_named(&name) else {
                continue;
            };
            let Some(t) = crate::typeface(n) else {
                continue;
            };
            let f = self.styled(n, &t);
            if !out.contains(&f) {
                out.push(f);
            }
        }
        out
    }

    /// The characters of `text` no font can write, each once, an emoji sequence whole
    /// ([`emoji_sequence`]); control characters among them. All of them when the font
    /// asked for can't be written at all ([`Setter::unusable`]).
    pub(crate) fn missing(&self, text: &str) -> String {
        let mut out = String::new();
        let mut add = |s: &str| {
            if !out.contains(s) {
                out.push_str(s);
            }
        };
        if self.unusable().is_some() {
            for c in text.chars() {
                add(c.encode_utf8(&mut [0; 4]));
            }
            return out;
        }
        let chars: Vec<char> = text.chars().collect();
        for ((a, b), font) in self.pieces(text) {
            if font.is_some() {
                continue;
            }
            let unit: String = chars[a..b].iter().collect();
            for cluster in unit.graphemes(true) {
                if emoji_sequence(cluster) {
                    add(cluster);
                    continue;
                }
                for c in cluster.chars() {
                    let one = c.to_string();
                    let found = match c.is_whitespace() {
                        true => covers(self.own(), &one),
                        false => self.unit_font(&one).is_some(),
                    };
                    if !found {
                        add(&one);
                    }
                }
            }
        }
        out
    }

    /// The words of `text` set in another font than the one asked for, and that font, in
    /// order: what a note names ("Not in Calibri: 漢字. Those characters use Noto Sans CJK
    /// JP.").
    pub(crate) fn fallbacks(&self, text: &str) -> Vec<(String, FontChoice)> {
        let chars: Vec<char> = text.chars().collect();
        let mut out: Vec<(String, FontChoice)> = Vec::new();
        let own = self.own();
        // Whether the last piece went to the same font as the one before.
        let mut joins = false;
        for ((a, b), font) in self.pieces(text) {
            let Some(font) = font.filter(|&f| f != own) else {
                joins = false;
                continue;
            };
            let word: String = chars[a..b].iter().collect();
            match out.last_mut() {
                Some((w, f)) if joins && *f == font.choice() => w.push_str(&word),
                _ => out.push((word, font.choice())),
            }
            joins = true;
        }
        out
    }

    /// How wide `text` is at `size`, in points, as it is written.
    pub(crate) fn width(&self, text: &str, size: f32) -> f32 {
        self.lay(text).width(size)
    }

    /// `text` broken into lines no wider than `width` points at `size` (the text design,
    /// 4.6): at its line breaks, then where Unicode's line-breaking rule (UAX #14) allows
    /// (after spaces and hyphens, between ideographs, never before closing punctuation
    /// or small kana), each line taking what fits; a word wider than a line is broken
    /// between its glyphs' clusters ([`Setter::ends`]). Spaces at a line's end aren't counted,
    /// and are left out. A soft hyphen shows as a hyphen at a line's end, and nowhere else.
    pub(crate) fn lines(&self, text: &str, width: f32, size: f32) -> Vec<String> {
        break_lines(text, width, |line| self.width(line, size), |w| self.ends(w))
    }

    /// Where a line may end inside `word` when it is broken between its characters (S1-14's
    /// legal boundaries): after each of its grapheme clusters that no cluster of its glyphs,
    /// the word set whole, runs across. In bytes, in order, the last its end.
    pub(crate) fn ends(&self, word: &str) -> Vec<usize> {
        let n = word.chars().count();
        let mut inside = vec![false; n + 1];
        for ((a, b), _) in self.lay(word).clusters() {
            inside[a + 1..b].fill(true);
        }
        let mut out = Vec::new();
        let mut k = 0;
        for (i, g) in word.grapheme_indices(true) {
            k += g.chars().count();
            if !inside[k] {
                out.push(i + g.len());
            }
        }
        out
    }

    /// A stretch of the text in one font and one script, set, its characters mapped to
    /// `chars`.
    fn run(&self, font: SetIn, chars: &[char], script: Script) -> Run {
        let script = harfrust_script(script);
        let language = self.language.as_ref();
        match font {
            SetIn::Face(f) => {
                let Ok(face) = Face::of(f) else {
                    return Run {
                        font,
                        text: Vec::new(),
                        from: Vec::new(),
                        glyphs: Vec::new(),
                    };
                };
                let spelled = face.spelled(&chars.iter().collect::<String>());
                let (text, from) = (spelled.chars, spelled.from);
                let glyphs = face.shape(&text, script, language, false);
                Run {
                    font,
                    text,
                    from,
                    glyphs,
                }
            }
            SetIn::Standard(f) => {
                let face = Face::liberation(f);
                let shaped = face.shape(chars, script, language, true);
                let afm = |c: char| {
                    std14::width(f.base_font(), c)
                        .unwrap_or_else(|| face.advance(face_glyph(face, c)))
                };
                // One glyph for each character, in order, as kerning alone gives.
                let one_each = shaped.len() == chars.len()
                    && (shaped.iter().enumerate()).all(|(i, g)| g.chars == (i, i + 1));
                let glyphs = (chars.iter().enumerate())
                    .map(|(i, &c)| {
                        let kern = match one_each {
                            true => shaped[i].advance - face.advance(shaped[i].glyph),
                            false => 0.0,
                        };
                        Shaped {
                            glyph: winansi_byte(c).unwrap_or(b'?').into(),
                            chars: (i, i + 1),
                            advance: afm(c) + kern,
                            x: if one_each { shaped[i].x } else { 0.0 },
                            y: if one_each { shaped[i].y } else { 0.0 },
                        }
                    })
                    .collect();
                Run {
                    font,
                    text: chars.to_vec(),
                    from: (0..chars.len()).map(|i| (i, i + 1)).collect(),
                    glyphs,
                }
            }
        }
    }
}

/// The words of `text` LimarePDF writes in another font than `f`, as `f` lacks a character
/// in them, each with the font it goes to, in order: what a note names ("Not in Calibri:
/// 漢字. Those characters use Noto Sans CJK JP.").
pub fn fallbacks(f: FontChoice, text: &str) -> Vec<(String, FontChoice)> {
    Setter::new(f, text, None).fallbacks(text)
}

/// The installed faces LimarePDF writes the characters of `samples` in where Helvetica lacks
/// them, each face once, and one left out when a face before it has the character: what the
/// interface falls back to for text its own fonts can't draw (ENH-33). The first call reads
/// the installed fonts' list, so it is asked off the window's thread.
pub fn fallback_faces(samples: &str) -> Vec<crate::FaceFile> {
    let mut out: Vec<crate::FaceFile> = Vec::new();
    for c in samples.chars() {
        if out.iter().any(|f| maps_all(&f.path, f.index, &[c])) {
            continue;
        }
        let one = c.to_string();
        let Some(SetIn::Face(f)) = Setter::new(FontChoice::default(), &one, None).unit_font(&one)
        else {
            continue;
        };
        // A standard font's bundled face is the interface's to have, not an installed one.
        let face = f.typeface().and_then(|t| t.face(f.bold, f.italic).cloned());
        if let Some(face) = face.filter(|face| !out.contains(face)) {
            out.push(face);
        }
    }
    out
}

/// Finds, once, the fonts each standard font's text falls back to for the system's language,
/// when it is Chinese, Japanese or Korean (by its locale), and for emoji: reading the
/// installed fonts' list and those fonts' files, so the first such character typed doesn't
/// wait for them. A character of another script is found when first typed (its font is
/// small); one no script's font has, by looking through every installed font. Slow: asked
/// off the window's thread, at start.
pub fn warm_fallbacks() {
    let samples: &[&str] = match system_language() {
        Some(Language::Japanese) => &["漢", "あ", "😀"],
        Some(Language::Korean) => &["한", "漢", "😀"],
        Some(_) => &["漢", "😀"],
        None => &["😀"],
    };
    for family in [Family::Helvetica, Family::Times, Family::Courier] {
        let f = FontChoice {
            family,
            ..FontChoice::default()
        };
        for s in samples {
            Setter::new(f, s, None).unit_font(s);
        }
    }
}

/// `text` in the stretches LimarePDF writes it in when it is set in `f`, in a document whose
/// language is `lang` (its `/Lang`, [`Pdf::language`]): each stretch's characters (from,
/// to) and its font, `f` or one it falls back to ([`fallbacks`]), as itself or in its
/// embedded face, `None` for what no font can write; in order, neighbours in one font
/// joined. The typing box draws each stretch in its font, and decorates it as the font is
/// written ([`SetIn::lines`]).
pub fn set_in(
    f: FontChoice,
    text: &str,
    lang: Option<&str>,
) -> Vec<((usize, usize), Option<SetIn>)> {
    let mut out: Vec<((usize, usize), Option<SetIn>)> = Vec::new();
    for ((a, b), font) in Setter::new(f, text, lang).pieces(text) {
        match out.last_mut() {
            Some((r, last)) if *last == font => r.1 = b,
            _ => out.push(((a, b), font)),
        }
    }
    out
}

/// The face's glyph for `c` as its character map gives it, 0 when it has none.
fn face_glyph(face: &Face, c: char) -> u16 {
    face.glyph_of(c).unwrap_or(0)
}

/// Fonts for single emoji drawn with outlines, best first.
const EMOJI: [&str; 6] = [
    "Noto Emoji",
    "Symbola",
    "Segoe UI Symbol",
    "Segoe UI Emoji",
    "Apple Symbols",
    "DejaVu Sans",
];

/// Fonts for symbols, best first.
const SYMBOLS: [&str; 5] = [
    "Noto Sans Symbols",
    "Noto Sans Symbols 2",
    "Noto Sans Math",
    "DejaVu Sans",
    "Symbola",
];

/// What a word is written in, for the fonts tried for it.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Kind {
    /// Chinese, Japanese or Korean: ideographs, kana, Hangul, and their punctuation.
    Cjk,
    /// A single emoji.
    Emoji,
    /// Symbols and punctuation of no script.
    Symbol,
    Script(Script),
}

/// What `unit`, a word, is written in: its first character with a script of its own,
/// else what its characters are.
fn unit_script(unit: &str) -> Kind {
    let cjk = |s: Script| {
        matches!(
            s,
            Script::Han | Script::Hiragana | Script::Katakana | Script::Hangul | Script::Bopomofo
        )
    };
    if let Some(s) = unit.chars().map(|c| c.script()).find(|s| !common(*s)) {
        return if cjk(s) { Kind::Cjk } else { Kind::Script(s) };
    }
    if unit.chars().any(cjk_punctuation) {
        Kind::Cjk
    } else if unit.chars().any(emoji) {
        Kind::Emoji
    } else {
        Kind::Symbol
    }
}

/// Whether `s` belongs to the text around it: shared punctuation and digits, and marks.
fn common(s: Script) -> bool {
    matches!(s, Script::Common | Script::Inherited | Script::Unknown)
}

/// Whether `c` is punctuation or a symbol of Chinese, Japanese and Korean text: CJK
/// symbols and punctuation, enclosed letters, compatibility forms, full-width forms.
fn cjk_punctuation(c: char) -> bool {
    matches!(
        c,
        '\u{3000}'..='\u{303F}' | '\u{3200}'..='\u{33FF}' | '\u{FE30}'..='\u{FE4F}' | '\u{FF00}'..='\u{FFEF}'
    )
}

/// Whether `c` is an emoji's picture.
fn emoji(c: char) -> bool {
    matches!(c, '\u{2600}'..='\u{27BF}' | '\u{2B00}'..='\u{2BFF}' | '\u{1F000}'..='\u{1FAFF}')
}

/// `chars` cut into stretches of one script, from and to, Common and Inherited characters
/// (spaces, punctuation, digits, marks) joining the stretch they are in, or the first after
/// them at the start.
fn items(chars: &[char]) -> Vec<(usize, usize, Script)> {
    let mut out: Vec<(usize, usize, Script)> = Vec::new();
    for (i, c) in chars.iter().enumerate() {
        let s = c.script();
        match out.last_mut() {
            Some(last) if common(s) || last.2 == s => last.1 = i + 1,
            Some(last) if common(last.2) => {
                last.1 = i + 1;
                last.2 = s;
            }
            _ => out.push((i, i + 1, s)),
        }
    }
    out
}

/// The script harfrust shapes `chars` in: their first with one of its own.
#[cfg(test)]
pub(crate) fn script_of(chars: &[char]) -> harfrust::Script {
    let s = (chars.iter().map(|c| c.script()))
        .find(|s| !common(*s))
        .unwrap_or(Script::Common);
    harfrust_script(s)
}

/// `s` as harfrust names it.
fn harfrust_script(s: Script) -> harfrust::Script {
    let b = s.short_name().as_bytes();
    let tag = match b {
        [a, b, c, d] => skrifa::Tag::new(&[*a, *b, *c, *d]),
        _ => skrifa::Tag::new(b"Zyyy"),
    };
    harfrust::Script::from_iso15924_tag(tag).unwrap_or(harfrust::script::COMMON)
}

/// Whether `font` has every character of `text`, as it writes them (a standard font
/// itself: WinAnsi; a face: [`Face::spelled`]). Control characters never.
fn covers(font: SetIn, text: &str) -> bool {
    if text.chars().any(char::is_control) {
        return false;
    }
    match font {
        SetIn::Standard(_) => text.chars().all(|c| winansi_byte(c).is_some()),
        SetIn::Face(f) => Face::of(f).is_ok_and(|face| face.spelled(text).missing.is_empty()),
    }
}

/// Whether installed font `f`'s character map has `text`'s characters, put together or
/// apart: asked before [`covers`] reads the whole font, so looking through installed
/// fonts reads none of them.
fn could(f: FontChoice, text: &str) -> bool {
    let Some(file) = f.typeface().and_then(|t| t.face(f.bold, f.italic).cloned()) else {
        return false;
    };
    let together: Vec<char> = text.nfc().collect();
    let apart: Vec<char> = text.nfd().collect();
    maps_all(&file.path, file.index, &together) || maps_all(&file.path, file.index, &apart)
}

/// Which of Chinese, Japanese and Korean `text` is, when its characters say: kana are
/// Japanese, Hangul Korean.
fn text_language(text: &str) -> Option<Language> {
    text.chars().find_map(|c| match c.script() {
        Script::Hiragana | Script::Katakana => Some(Language::Japanese),
        Script::Hangul => Some(Language::Korean),
        _ => None,
    })
}

/// Which of Chinese, Japanese and Korean the system's language is, when it is one (by
/// the locale's environment variables).
fn system_language() -> Option<Language> {
    static FOUND: OnceLock<Option<Language>> = OnceLock::new();
    *FOUND.get_or_init(|| {
        ["LC_ALL", "LC_CTYPE", "LANG", "LANGUAGE"]
            .into_iter()
            .filter_map(|v| std::env::var(v).ok())
            .find(|v| !v.is_empty())
            .and_then(|v| Language::of_tag(&v))
    })
}

/// `text` broken into lines no wider than `width` as `measure` measures a line as it shows
/// ([`Setter::lines`]), a word wider than a line where `ends` lets it end ([`Setter::ends`]).
pub(crate) fn break_lines(
    text: &str,
    width: f32,
    measure: impl Fn(&str) -> f32,
    ends: impl Fn(&str) -> Vec<usize>,
) -> Vec<String> {
    (break_ranges(text, width, measure, ends).into_iter())
        .map(|line| shown(&text[line]))
        .collect()
}

/// The ends of `word`'s grapheme clusters, in bytes: where a line may end inside a word
/// written without shaping ([`Setter::ends`]).
pub(crate) fn grapheme_ends(word: &str) -> Vec<usize> {
    (word.grapheme_indices(true))
        .map(|(i, g)| i + g.len())
        .collect()
}

/// Where [`break_lines`] breaks `text`: each line's bytes in it, the spaces it ends with
/// among them, the line breaks between paragraphs not.
pub(crate) fn break_ranges(
    text: &str,
    width: f32,
    measure: impl Fn(&str) -> f32,
    ends: impl Fn(&str) -> Vec<usize>,
) -> Vec<Range<usize>> {
    let fits = |line: &Range<usize>| measure(&shown(&text[line.clone()])) <= width + 1e-3;
    let mut out = Vec::new();
    for para in paragraphs(text) {
        let mut line = para.start..para.start;
        for (at, _) in unicode_linebreak::linebreaks(&text[para.clone()]) {
            let end = para.start + at;
            if end == line.end {
                continue;
            }
            let joined = line.start..end;
            if line.is_empty() || fits(&joined) {
                line = joined;
            } else {
                out.push(line.clone());
                line = line.end..end;
            }
            // A word wider than a line of its own is broken where it fills one, where `ends`
            // lets it: each line takes the clusters that fit ([`fitting`]), or one.
            if !fits(&line) {
                let all: Vec<usize> = (ends(&text[line.clone()]).into_iter())
                    .map(|e| line.start + e)
                    .collect();
                // The whole line doesn't fit, so its end isn't tried at first.
                let mut tried = all.len().saturating_sub(1);
                let mut from = 0;
                while from < tried {
                    let n = fitting(fits, line.start, &all[from..tried]).max(1);
                    if from + n == all.len() {
                        break;
                    }
                    out.push(line.start..all[from + n - 1]);
                    line.start = all[from + n - 1];
                    from += n;
                    tried = all.len();
                }
            }
        }
        out.push(line);
    }
    out
}

/// How many of `ends` fit, a line from `start` ending at each in turn: those that fit come
/// first, as a line gets wider with each cluster it gains. It tries the 1st, 2nd, 4th, 8th…
/// until one doesn't fit, then halves the range between the last that does and the first
/// that doesn't, so a word broken over many lines is measured a few times a line, not once
/// for each of its characters (M-08).
fn fitting(fits: impl Fn(&Range<usize>) -> bool, start: usize, ends: &[usize]) -> usize {
    let (mut lo, mut hi) = (0, ends.len() + 1);
    while lo < ends.len() {
        let n = (2 * lo).clamp(1, ends.len());
        if !fits(&(start..ends[n - 1])) {
            hi = n;
            break;
        }
        lo = n;
    }
    while lo + 1 < hi {
        let n = (lo + hi) / 2;
        match fits(&(start..ends[n - 1])) {
            true => lo = n,
            false => hi = n,
        }
    }
    lo
}

/// `text`'s paragraphs, as it is split at its line breaks (`\r\n`, `\n`, `\r`, and Unicode's
/// line and paragraph separators, vertical tab, form feed and next line): their bytes.
fn paragraphs(text: &str) -> Vec<Range<usize>> {
    let mut out = Vec::new();
    let mut from = 0;
    let mut chars = text.char_indices().peekable();
    let ends = [
        '\n', '\r', '\u{0B}', '\u{0C}', '\u{85}', '\u{2028}', '\u{2029}',
    ];
    while let Some((i, c)) = chars.next() {
        if ends.contains(&c) {
            out.push(from..i);
            from = i + c.len_utf8();
            if c == '\r' && chars.peek().is_some_and(|&(_, n)| n == '\n') {
                chars.next();
                from += 1;
            }
        }
    }
    out.push(from..text.len());
    out
}

/// A line as it shows: spaces at its end left out, a soft hyphen at its end shown as a
/// hyphen and others left out.
pub(crate) fn shown(line: &str) -> String {
    let line = line.trim_end_matches(char::is_whitespace);
    match line.strip_suffix('\u{AD}') {
        Some(rest) => format!("{}-", rest.replace('\u{AD}', "")),
        None => line.replace('\u{AD}', ""),
    }
}

/// Writes lines of text a [`Setter`] set ([`Pdf::pen`]): the fonts they use, in the order
/// first used, and each line's shows.
#[derive(Clone, Debug)]
pub(crate) struct Pen {
    fonts: Vec<ObjectId>,
    lines: Vec<Vec<Show>>,
    /// The font objects of the standard fonts written as themselves, to use again.
    standard: Vec<(FontChoice, ObjectId)>,
}

/// A show of part of a line in one font and at one height: the font (its place in
/// [`Pen::fonts`]), how far it is raised, in thousandths of an em (a mark the font places
/// higher or lower), and what its `TJ` shows: codes, and the moves between them that place
/// each glyph where shaping put it.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Show {
    pub font: usize,
    pub rise: f32,
    pub items: Vec<Object>,
}

impl Pen {
    /// The font objects the lines are written in, to name in the resources.
    pub(crate) fn fonts(&self) -> &[ObjectId] {
        &self.fonts
    }

    /// The font objects of the standard fonts it writes as themselves, by font, to give
    /// another pen ([`Pdf::pen_reusing`]).
    pub(crate) fn standard_fonts(&self) -> &[(FontChoice, ObjectId)] {
        &self.standard
    }

    /// Names for its fonts in resources of their own (a form's): `first`, then `first`
    /// with 2, 3 and on.
    pub(crate) fn names(&self, first: &str) -> Vec<String> {
        (0..self.fonts.len().max(1))
            .map(|k| match k {
                0 => first.to_string(),
                k => format!("{first}{}", k + 1),
            })
            .collect()
    }

    /// Its fonts by `names` ([`Pen::names`]), for a `/Font` resource dictionary.
    pub(crate) fn font_dict(&self, names: &[String]) -> lopdf::Dictionary {
        let mut d = lopdf::Dictionary::new();
        for (name, &font) in names.iter().zip(&self.fonts) {
            d.set(name.as_bytes().to_vec(), font);
        }
        d
    }

    /// The shows of line `line` (the `line`th of those the pen was made for).
    pub(crate) fn shows(&self, line: usize) -> &[Show] {
        self.lines.get(line).map_or(&[], Vec::as_slice)
    }

    /// The operators that write line `line` at `size`, raised `rise` points, the fonts
    /// being named `names` in the resources (in [`Pen::fonts`]' order), with the first of
    /// them, at `size` and `rise`, already set: each show's `TJ`, with `Tf` and `Ts` where
    /// they change, put back after.
    pub(crate) fn ops(
        &self,
        line: usize,
        size: f32,
        rise: f32,
        names: &[String],
    ) -> Vec<Operation> {
        let mut ops = Vec::new();
        let (mut font, mut up) = (0, 0.0);
        let tf = |k: usize| {
            let name = Object::Name(names.get(k).cloned().unwrap_or_default().into_bytes());
            Operation::new("Tf", vec![name, number(size)])
        };
        for s in self.shows(line) {
            if s.font != font {
                ops.push(tf(s.font));
                font = s.font;
            }
            if s.rise != up {
                let ts = number(rise + s.rise * size / 1000.0);
                ops.push(Operation::new("Ts", vec![ts]));
                up = s.rise;
            }
            ops.push(Operation::new("TJ", vec![Object::Array(s.items.clone())]));
        }
        if font != 0 {
            ops.push(tf(0));
        }
        if up != 0.0 {
            ops.push(Operation::new("Ts", vec![number(rise)]));
        }
        ops
    }

    /// [`Pen::ops`] as content stream text, not raised: every code in `<…>`, so the text is
    /// ASCII.
    pub(crate) fn content(&self, line: usize, size: f32, names: &[String]) -> String {
        let mut ops = self.ops(line, size, 0.0, names);
        for o in &mut ops {
            for items in o.operands.iter_mut().filter_map(|a| a.as_array_mut().ok()) {
                for item in items {
                    if let Object::String(_, format) = item {
                        *format = StringFormat::Hexadecimal;
                    }
                }
            }
        }
        let bytes = (lopdf::content::Content { operations: ops })
            .encode()
            .unwrap_or_default();
        String::from_utf8_lossy(&bytes).into_owned()
    }
}

/// `text` in the standard font `f` itself, as content stream text: its `TJ`, kerned as
/// [`Pdf::pen`] writes it, the font set by the caller. `None` when a character doesn't fit
/// WinAnsi, so the font's embedded face is needed.
pub(crate) fn standard_tj(f: FontChoice, text: &str) -> Option<String> {
    let setter = Setter::new(f, text, None);
    if !setter.standard || !setter.missing(text).is_empty() {
        return None;
    }
    // The font is the caller's: any object stands for it.
    let here = (0, 0);
    let laid = setter.lay(text);
    let shows = shows(&laid, &HashMap::new(), &[(f, here)], &mut Vec::new());
    let pen = Pen {
        fonts: vec![here],
        lines: vec![shows],
        standard: Vec::new(),
    };
    Some(pen.content(0, 1.0, &[String::new()]))
}

/// `x` as a number in a content stream, to a hundredth.
fn number(x: f32) -> Object {
    Object::Real((x * 100.0).round() / 100.0)
}

/// An embedded face a pen writes in: the font chosen, its face, and each of its glyphs the
/// lines need with the text it is written for.
type Needs = (FontChoice, &'static Face, Vec<(u16, String)>);

/// How far written text may be from where shaping put it before a move puts it back, in
/// thousandths of an em.
const DRIFT: f64 = 0.005;

impl Pdf {
    /// The document's language (`/Lang`), when it gives one: what text is shaped for, and
    /// which of Chinese, Japanese and Korean fonts its ideographs are set in.
    pub fn language(&self) -> Option<String> {
        let root = self.doc.trailer.get(b"Root").ok()?;
        let catalog = self.doc.dereference(root).ok()?.1.as_dict().ok()?;
        let lang = self.doc.dereference(catalog.get(b"Lang").ok()?).ok()?.1;
        let s = crate::doc::decode_text(lang.as_str().ok()?);
        (!s.trim().is_empty()).then(|| s.trim().to_string())
    }

    /// `fonts` (a pen's) added to page `page`'s resources, under fresh names from `prefix`:
    /// the names, in order.
    pub(crate) fn font_names(
        &mut self,
        page: ObjectId,
        prefix: &str,
        fonts: &[ObjectId],
    ) -> Result<Vec<String>> {
        (fonts.iter())
            .map(|&f| self.add_resource(page, "Font", prefix, f.into()))
            .collect()
    }

    /// A setter for text in `f` in this document ([`Setter::new`]), `text` being all it
    /// will set.
    pub(crate) fn setter(&self, f: FontChoice, text: &str) -> Setter {
        Setter::new(f, text, self.language().as_deref())
    }

    /// A pen writing `lines` in `f` ([`Pdf::pen_with`]).
    pub(crate) fn pen(&mut self, f: FontChoice, lines: &[&str]) -> Result<Pen> {
        let setter = self.setter(f, &lines.concat());
        self.pen_with(&setter, lines)
    }

    /// A pen writing `lines` as `setter` sets them, every glyph they need embedded (each
    /// face's font object grown once), and the standard fonts written as themselves added
    /// as font objects. Refused, with nothing changed, when a character can't be written
    /// in any font, or the font asked for can't be read.
    pub(crate) fn pen_with(&mut self, setter: &Setter, lines: &[&str]) -> Result<Pen> {
        self.pen_reusing(setter, lines, &[])
    }

    /// [`Pdf::pen_with`], a standard font written as itself in the font object `reuse`
    /// gives for it, when it gives one (a form's own, in its default resources). Control
    /// characters (a line break, a stray NUL) aren't written, and aren't refused.
    pub(crate) fn pen_reusing(
        &mut self,
        setter: &Setter,
        lines: &[&str],
        reuse: &[(FontChoice, ObjectId)],
    ) -> Result<Pen> {
        let all: String = (lines.concat().chars())
            .filter(|c| !c.is_control())
            .collect();
        if let Some(why) = setter.unusable() {
            bail!("{why}");
        }
        let missing = setter.missing(&all);
        if !missing.is_empty() {
            bail!("{}", embed::refusal(setter.asked, &missing));
        }
        let laid: Vec<Rc<Laid>> = lines.iter().map(|l| setter.lay(l)).collect();
        let laid: Vec<&Laid> = laid.iter().map(|l| l.as_ref()).collect();
        let mut pen = self.pen_laid(&laid, reuse)?;
        // Lines with nothing to show still name a font: the standard one asked for.
        if let (true, SetIn::Standard(f)) = (pen.fonts.is_empty(), setter.own()) {
            let id = match pen.standard.iter().find(|(c, _)| *c == f) {
                Some(&(_, id)) => id,
                None => {
                    let font = dictionary! {
                        "Type" => "Font", "Subtype" => "Type1", "BaseFont" => f.base_font(),
                        "Encoding" => "WinAnsiEncoding",
                    };
                    let id = self.doc.add_object(font);
                    pen.standard.push((f, id));
                    id
                }
            };
            pen.fonts.push(id);
        }
        Ok(pen)
    }

    /// A pen writing lines already set (`laid`), as they were set and measured: every
    /// glyph they need embedded (each face's font object grown once), and the standard
    /// fonts written as themselves added as font objects, or taken from `reuse`.
    pub(crate) fn pen_laid(
        &mut self,
        laid: &[&Laid],
        reuse: &[(FontChoice, ObjectId)],
    ) -> Result<Pen> {
        // What each face must hold, each glyph and text once.
        let mut faces: Vec<Needs> = Vec::new();
        let mut wanted: HashSet<(FontChoice, u16, String)> = HashSet::new();
        for run in laid.iter().flat_map(|l| &l.runs) {
            let SetIn::Face(f) = run.font else {
                continue;
            };
            let face = Face::of(f)?;
            let at = match faces.iter().position(|(c, ..)| *c == f) {
                Some(at) => at,
                None => {
                    faces.push((f, face, Vec::new()));
                    faces.len() - 1
                }
            };
            for g in &run.glyphs {
                let text: String = run.text[g.chars.0..g.chars.1].iter().collect();
                if wanted.insert((f, g.glyph, text.clone())) {
                    faces[at].2.push((g.glyph, text));
                }
            }
        }
        let mut placed: HashMap<(FontChoice, u16, String), Placed> = HashMap::new();
        for (f, face, glyphs) in &faces {
            for (p, (g, t)) in self.embed_glyphs(face, glyphs)?.into_iter().zip(glyphs) {
                placed.insert((*f, *g, t.clone()), p);
            }
        }
        let mut standard: Vec<(FontChoice, ObjectId)> = reuse.to_vec();
        for run in laid.iter().flat_map(|l| &l.runs) {
            if let SetIn::Standard(f) = run.font {
                if !standard.iter().any(|(c, _)| *c == f) {
                    let font = dictionary! {
                        "Type" => "Font", "Subtype" => "Type1", "BaseFont" => f.base_font(),
                        "Encoding" => "WinAnsiEncoding",
                    };
                    standard.push((f, self.doc.add_object(font)));
                }
            }
        }
        let mut fonts: Vec<ObjectId> = Vec::new();
        let mut out = Vec::with_capacity(laid.len());
        for l in laid {
            out.push(shows(l, &placed, &standard, &mut fonts));
        }
        Ok(Pen {
            fonts,
            lines: out,
            standard,
        })
    }
}

/// The shows that write `laid` with the codes its glyphs were `placed` at (in a standard
/// font itself, its object in `standard`), each glyph where shaping put it: a move before
/// a glyph wherever the widths the fonts give would leave it more than [`DRIFT`] off.
/// `fonts` gathers the font objects used.
fn shows(
    laid: &Laid,
    placed: &HashMap<(FontChoice, u16, String), Placed>,
    standard: &[(FontChoice, ObjectId)],
    fonts: &mut Vec<ObjectId>,
) -> Vec<Show> {
    let mut out: Vec<Show> = Vec::new();
    // Where the next glyph goes as written, and where shaping put it.
    let (mut written, mut shaped) = (0.0f64, 0.0f64);
    for run in &laid.runs {
        for g in &run.glyphs {
            let (font, code, format, width) = match run.font {
                SetIn::Standard(f) => {
                    let id = standard.iter().find(|(c, _)| *c == f).map(|(_, id)| *id);
                    let c = run.text[g.chars.0];
                    let width = std14::width(f.base_font(), c).unwrap_or(g.advance);
                    let byte = vec![g.glyph as u8];
                    (id, byte, StringFormat::Literal, width)
                }
                SetIn::Face(f) => {
                    let text: String = run.text[g.chars.0..g.chars.1].iter().collect();
                    let p = placed.get(&(f, g.glyph, text));
                    let code = p.map(|p| p.code.to_be_bytes().to_vec()).unwrap_or_default();
                    let width = p.map_or(0.0, |p| p.width);
                    (p.map(|p| p.font), code, StringFormat::Hexadecimal, width)
                }
            };
            let Some(font) = font else {
                continue;
            };
            let k = match fonts.iter().position(|&f| f == font) {
                Some(k) => k,
                None => {
                    fonts.push(font);
                    fonts.len() - 1
                }
            };
            if out.last().is_none_or(|s| s.font != k || s.rise != g.y) {
                out.push(Show {
                    font: k,
                    rise: g.y,
                    items: Vec::new(),
                });
            }
            let items = &mut out.last_mut().expect("pushed").items;
            let off = written - (shaped + g.x as f64);
            if off.abs() > DRIFT {
                let n = (off * 100.0).round() / 100.0;
                items.push(Object::Real(n as f32));
                written -= n;
            }
            match items.last_mut() {
                Some(Object::String(bytes, f)) if *f == format => bytes.extend(code),
                _ => items.push(Object::String(code, format)),
            }
            written += width as f64;
            shaped += g.advance as f64;
        }
    }
    // The line ends where shaping ends it.
    let off = written - shaped;
    if let Some(s) = out.last_mut().filter(|_| off.abs() > DRIFT) {
        let n = (off * 100.0).round() / 100.0;
        s.items.push(Object::Real(n as f32));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    const SANS: FontChoice = FontChoice {
        family: Family::Helvetica,
        bold: false,
        italic: false,
    };

    #[test]
    fn lines_break_where_unicode_allows_and_spaces_hang() {
        let s = Setter::new(SANS, "", None);
        let w = s.width("aaaa bbbb", 12.0) + 0.01;
        assert_eq!(s.lines("aaaa bbbb cccc", w, 12.0), ["aaaa bbbb", "cccc"]);
        // Spaces at a line's end aren't counted.
        assert_eq!(s.lines("aaaa bbbb    cccc", w, 12.0), ["aaaa bbbb", "cccc"]);
        // After a hyphen.
        let w = s.width("known", 12.0) + 0.01;
        assert_eq!(s.lines("well-known", w, 12.0), ["well-", "known"]);
        // Typed line breaks stay, an empty line too.
        assert_eq!(s.lines("a\r\n\nb", 100.0, 12.0), ["a", "", "b"]);
        // A word wider than a line is broken between characters.
        let w = s.width("abc", 12.0) + 0.01;
        assert_eq!(s.lines("abcdefg", w, 12.0), ["abc", "def", "g"]);
        // A soft hyphen shows only at a line's end.
        let w = s.width("hyphen-", 12.0) + 0.01;
        assert_eq!(s.lines("hyphen\u{AD}ation", w, 12.0), ["hyphen-", "ation"]);
        assert_eq!(s.lines("hyphen\u{AD}ation", 500.0, 12.0), ["hyphenation"]);
    }

    /// The typing box draws each line as it is typed, so each is found in the text: the
    /// spaces it ends with in it, the line breaks between lines not.
    #[test]
    fn each_line_is_found_in_the_text_as_typed() {
        let s = Setter::new(SANS, "", None);
        let ranges =
            |text: &str, w: f32| break_ranges(text, w, |l| s.width(l, 12.0), |w| s.ends(w));
        let w = s.width("aaaa bbbb", 12.0) + 0.01;
        let text = "aaaa bbbb    cccc\r\n\ndd\u{AD}ee";
        let got = ranges(text, w);
        let lines: Vec<&str> = got.iter().map(|r| &text[r.clone()]).collect();
        assert_eq!(lines, ["aaaa bbbb    ", "cccc", "", "dd\u{AD}ee"]);
        // As written, they are the lines `lines` gives.
        let shown: Vec<String> = got.into_iter().map(|r| shown(&text[r])).collect();
        assert_eq!(shown, s.lines(text, w, 12.0));
        // A word broken between characters, and a final line break.
        let w = s.width("abc", 12.0) + 0.01;
        let text = "abcdefg\n";
        let got: Vec<&str> = ranges(text, w).into_iter().map(|r| &text[r]).collect();
        assert_eq!(got, ["abc", "def", "g", ""]);
    }

    /// How many words [`UNITS`] keeps, and the bytes of their text.
    fn units_kept() -> (usize, usize) {
        let units = UNITS.lock().unwrap_or_else(|e| e.into_inner());
        (
            units.1.map.len(),
            units.1.map.keys().map(|k| k.3.len()).sum(),
        )
    }

    /// How many texts `s` keeps set, and their characters.
    fn laid_kept(s: &Setter) -> (usize, usize) {
        let laid = s.laid.borrow();
        (
            laid.map.len(),
            laid.map.keys().map(|t| t.chars().count()).sum(),
        )
    }

    /// What is kept is emptied before one more entry would pass either limit, and an entry
    /// bigger than the size limit alone isn't kept (M-08).
    #[test]
    fn what_is_kept_starts_again_at_its_limits() {
        let mut kept: Kept<&str, u8, 3, 10> = Kept::default();
        let size = |k: &Kept<&str, u8, 3, 10>| (k.map.len(), k.size);
        kept.keep("a", 1, 2);
        kept.keep("b", 2, 2);
        kept.keep("b", 3, 2);
        assert_eq!((size(&kept), kept.get("b")), ((2, 4), Some(&3)));
        kept.keep("c", 4, 2);
        kept.keep("d", 5, 2);
        assert_eq!((size(&kept), kept.get("a")), ((1, 2), None));
        kept.keep("e", 6, 9);
        assert_eq!((size(&kept), kept.get("d")), ((1, 9), None));
        kept.keep("f", 7, 11);
        assert_eq!((size(&kept), kept.get("e")), ((1, 9), Some(&6)));
    }

    /// A word wider than its box is broken in a few measures a line, not one for each of its
    /// characters (M-08): the measures and the characters they measure grow with its length
    /// times the logarithm of it, not with its square.
    #[test]
    fn a_long_word_is_measured_a_few_times_a_line() {
        let mut over = Vec::new();
        for n in [200, 400, 800] {
            let text = "a".repeat(n);
            let (calls, chars) = (Cell::new(0), Cell::new(0));
            let measure = |l: &str| {
                calls.set(calls.get() + 1);
                chars.set(chars.get() + l.len());
                l.len() as f32
            };
            let lines = break_ranges(&text, 10.0, measure, grapheme_ends);
            assert_eq!(lines.len(), n / 10);
            assert!(lines.iter().all(|l| l.len() == 10), "{lines:?}");
            let most = 2.0 * n as f64 * (n as f64).log2();
            let (calls, chars) = (calls.get(), chars.get());
            let row = format!(
                "{n} letters: {calls} measures of {chars} characters, at most {most:.0} each"
            );
            eprintln!("{row}");
            if calls as f64 > most || chars as f64 > most {
                over.push(row);
            }
        }
        assert!(over.is_empty(), "{over:#?}");
    }

    /// The fonts found for words are kept to use again, but no more than the limits allow,
    /// however many words are set (M-08).
    #[test]
    fn the_fonts_found_for_words_are_kept_within_limits() {
        let words: Vec<String> = (0..70_000).map(|i| format!("w{i}")).collect();
        let text = words.join(" ");
        Setter::new(SANS, &text, None).lay(&text);
        let (n, bytes) = units_kept();
        assert!(
            n <= UNITS_MOST && bytes <= UNITS_BYTES,
            "{n} words, {bytes} bytes"
        );
    }

    /// A setter keeps what it set to set again, within limits (M-08).
    #[test]
    fn a_setter_keeps_what_it_set_within_limits() {
        let s = Setter::new(SANS, "", None);
        for i in 0..5_000 {
            s.lay(&format!("text {i}"));
        }
        let (n, chars) = laid_kept(&s);
        assert!(
            n <= LAID_MOST && chars <= LAID_CHARS,
            "{n} texts, {chars} characters"
        );
    }

    /// A word wider than its box breaks between the clusters of its glyphs, never inside one
    /// (S1-14's legal boundaries): "ffiffi" in Source Serif 4 is two "ffi" ligatures, each
    /// alone on its line in a box narrower than one.
    #[test]
    fn a_long_word_breaks_only_between_its_glyph_clusters() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        let serif = dir.join("tests/data/fonts/SourceSerif4-Regular.otf");
        let n = crate::add_font_files(&[serif.as_path()])[0];
        let face = FontChoice {
            family: Family::Installed(n),
            bold: false,
            italic: false,
        };
        let s = Setter::new(face, "ffiffi", None);
        let clusters = s.lay("ffiffi").clusters();
        let spans: Vec<(usize, usize)> = clusters.iter().map(|c| c.0).collect();
        assert_eq!(spans, [(0, 3), (3, 6)]);
        let w = s.width("ffi", 12.0) * 0.8;
        assert_eq!(s.lines("ffiffi", w, 12.0), ["ffi", "ffi"]);
    }

    /// [`break_ranges`] as it was before S1-15 (`260a7b7`), a long word shrunk a grapheme at
    /// a time: the lines the search must still find.
    fn one_at_a_time(text: &str, width: f32, measure: impl Fn(&str) -> f32) -> Vec<Range<usize>> {
        let fits = |line: &Range<usize>| measure(&shown(&text[line.clone()])) <= width + 1e-3;
        let mut out = Vec::new();
        for para in paragraphs(text) {
            let mut line = para.start..para.start;
            for (at, _) in unicode_linebreak::linebreaks(&text[para.clone()]) {
                let end = para.start + at;
                if end == line.end {
                    continue;
                }
                let joined = line.start..end;
                if line.is_empty() || fits(&joined) {
                    line = joined;
                } else {
                    out.push(line.clone());
                    line = line.end..end;
                }
                while !fits(&line) {
                    let ends: Vec<usize> = (text[line.clone()].grapheme_indices(true))
                        .map(|(i, c)| line.start + i + c.len())
                        .collect();
                    let mut n = ends.len().saturating_sub(1);
                    while n > 1 && !fits(&(line.start..ends[n - 1])) {
                        n -= 1;
                    }
                    if n == 0 {
                        break;
                    }
                    out.push(line.start..ends[n - 1]);
                    line = ends[n - 1]..line.end;
                }
            }
            out.push(line);
        }
        out
    }

    /// Text broken as it was before S1-15 is broken the same: words, long ones among them,
    /// accents, Greek and Cyrillic, hyphens, soft hyphens, spaces and line breaks, in boxes
    /// from narrower than nothing to wide; measured as Helvetica's bundled face sets them,
    /// and (quicker, so more of them) at so many points a character.
    #[test]
    fn long_words_break_where_they_did() {
        let (xy, m) = ("xy".repeat(7), "m".repeat(22));
        let pieces = [
            "a", "bo", "W", "é", "e\u{301}", "Ωλ", "жё", "-", "\u{AD}", " ", "  ", "\n", &xy, &m,
        ];
        let mut seed: u64 = 15;
        let mut next = |n: usize| {
            seed = (seed.wrapping_mul(6364136223846793005)).wrapping_add(1442695040888963407);
            (seed >> 33) as usize % n
        };
        let quick = |l: &str| l.chars().map(|c| (c as u32 % 7 + 1) as f32).sum::<f32>();
        for round in 0..600 {
            let text: String = (0..next(30)).map(|_| pieces[next(pieces.len())]).collect();
            let s = Setter::new(SANS, &text, None);
            let set = |l: &str| s.width(l, 12.0);
            for w in [-1.0, 0.0, 3.0, 7.5, 12.0, 20.0, 35.0, 60.0, 100.0, 200.0] {
                assert_eq!(
                    break_ranges(&text, w, quick, grapheme_ends),
                    one_at_a_time(&text, w, quick),
                    "{text:?} in {w}, quickly"
                );
                if round % 30 == 0 {
                    assert_eq!(
                        break_ranges(&text, w, set, |word| s.ends(word)),
                        one_at_a_time(&text, w, set),
                        "{text:?} in {w} points"
                    );
                }
            }
        }
    }

    /// Breaking a word of 1,000 letters in a box ten letters wide within 20 ms (M-08), as a
    /// long value in a narrow form field is broken when a document opens. Opt-in, in a
    /// release build: `cargo test --release -p limarepdf-core --lib -- --ignored speed_of`.
    #[test]
    #[ignore]
    fn speed_of_a_long_word() {
        let text: String = "abcdefghij".chars().cycle().take(1000).collect();
        let w = Setter::new(SANS, "", None).width("abcdefghij", 12.0) + 0.01;
        let mut times = Vec::new();
        for _ in 0..5 {
            let s = Setter::new(SANS, &text, None);
            let t = std::time::Instant::now();
            let lines = s.lines(&text, w, 12.0);
            times.push(t.elapsed());
            assert!(lines.len() >= 100, "{}", lines.len());
        }
        times.sort();
        let (median, most) = (times[times.len() / 2], times[times.len() - 1]);
        eprintln!("1,000 letters: median {median:?}, slowest {most:?}");
        assert!(median < std::time::Duration::from_millis(20), "{median:?}");
    }

    #[test]
    fn closing_punctuation_and_small_kana_stay_off_a_line_s_start() {
        // Between any two ideographs or kana a line may break, but not before "。" or a
        // small kana: the rule keeps them with the character before.
        let breaks = |text: &str| -> Vec<usize> {
            unicode_linebreak::linebreaks(text)
                .map(|(i, _)| i)
                .collect()
        };
        let text = "これはテストです。";
        assert!(breaks(text).contains(&text.find('す').unwrap()));
        assert!(!breaks(text).contains(&text.find('。').unwrap()));
        let small = "ちょっと";
        assert!(!breaks(small).contains(&small.find('ょ').unwrap()));
    }

    #[test]
    fn standard_fonts_keep_one_code_a_character_and_kern() {
        let s = Setter::new(SANS, "AVATAR", None);
        assert_eq!(s.own(), SetIn::Standard(SANS));
        let laid = s.lay("AVATAR");
        assert_eq!(laid.runs.len(), 1);
        let codes: Vec<u16> = laid.runs[0].glyphs.iter().map(|g| g.glyph).collect();
        assert_eq!(codes, "AVATAR".bytes().map(u16::from).collect::<Vec<_>>());
        // Kerned: narrower than the widths alone.
        let plain: f32 = "AVATAR"
            .chars()
            .map(|c| std14::width("Helvetica", c).unwrap())
            .sum();
        let kerned = laid.width(1000.0);
        assert!(kerned < plain - 1.0, "{kerned} {plain}");
        // "fi" stays two codes.
        let fi = s.lay("fi");
        assert_eq!(fi.runs[0].glyphs.len(), 2);
    }

    /// Each cluster a face draws maps to the characters typed it draws (S1-14): "ffi",
    /// drawn as one ligature in Source Serif 4, to all three, and its width is the
    /// ligature's.
    #[test]
    fn a_ligature_is_one_cluster_of_the_characters_it_draws() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        let serif = dir.join("tests/data/fonts/SourceSerif4-Regular.otf");
        let n = crate::add_font_files(&[serif.as_path()])[0];
        let face = FontChoice {
            family: Family::Installed(n),
            bold: false,
            italic: false,
        };
        let s = Setter::new(face, "office", None);
        let laid = s.lay("office");
        assert_eq!(laid.codes(), 4);
        let clusters = laid.clusters();
        let spans: Vec<(usize, usize)> = clusters.iter().map(|c| c.0).collect();
        assert_eq!(spans, [(0, 1), (1, 4), (4, 5), (5, 6)]);
        let total: f32 = clusters.iter().map(|c| c.1).sum();
        assert!((total - laid.width(1000.0)).abs() < 1e-3, "{total}");
    }

    #[test]
    fn control_characters_are_left_out_not_refused() {
        // A stray NUL in text read from a file: not written, and the rest is.
        let mut p = Pdf::new();
        p.insert_blank(0, 200.0, 200.0).unwrap();
        let pen = p.pen(SANS, &["a\u{0}b"]).unwrap();
        let shown: Vec<u8> = (pen.shows(0).iter())
            .flat_map(|s| &s.items)
            .filter_map(|i| i.as_str().ok())
            .flatten()
            .copied()
            .collect();
        assert_eq!(shown, b"ab");
    }

    #[test]
    fn items_split_by_script_with_shared_characters_joining() {
        let chars: Vec<char> = "abc, 漢字。 def".chars().collect();
        let got: Vec<(usize, usize, Script)> = items(&chars);
        assert_eq!(
            got,
            [
                (0, 5, Script::Latin),
                (5, 9, Script::Han),
                (9, 12, Script::Latin)
            ]
        );
    }

    /// Shaping and breaking a paragraph of 2,000 characters within 2 ms (the text design,
    /// section 9), as each keystroke would: the fonts for its words known, its shaping not.
    /// Opt-in, in a release build: `cargo test --release -p limarepdf-core --lib --
    /// --ignored speed_of`.
    #[test]
    #[ignore]
    fn speed_of_a_paragraph() {
        let words = "The quick brown fox jumps over the lazy dog, AVATAR, Łódź Ωmega жёлтый. ";
        let text: String = words.chars().cycle().take(2000).collect();
        let mut times = Vec::new();
        for _ in 0..30 {
            let s = Setter::new(SANS, &text, None);
            let t = std::time::Instant::now();
            let lines = s.lines(&text, 300.0, 12.0);
            times.push(t.elapsed());
            assert!(lines.len() > 20, "{}", lines.len());
        }
        let first = times[0];
        times.sort();
        let (median, most) = (times[times.len() / 2], times[times.len() - 1]);
        eprintln!("2,000 characters: first {first:?}, median {median:?}, slowest {most:?}");
        assert!(median < std::time::Duration::from_millis(2), "{median:?}");
        // Text its font has needs no look at the installed fonts, even the first time.
        assert!(first < std::time::Duration::from_millis(50), "{first:?}");
    }

    #[test]
    fn languages_from_tags() {
        let traditional = Some(Language::TraditionalChinese);
        assert_eq!(Language::of_tag("ja-JP"), Some(Language::Japanese));
        assert_eq!(Language::of_tag("zh-Hant"), traditional);
        assert_eq!(Language::of_tag("zh_TW.UTF-8"), traditional);
        assert_eq!(Language::of_tag("zh-CN"), Some(Language::SimplifiedChinese));
        assert_eq!(Language::of_tag("ko"), Some(Language::Korean));
        assert_eq!(Language::of_tag("en-CA"), None);
        assert_eq!(text_language("漢字とかな"), Some(Language::Japanese));
        assert_eq!(text_language("漢字"), None);
    }
}

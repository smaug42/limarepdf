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
use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;
use std::sync::{LazyLock, Mutex, OnceLock};
use unicode_normalization::UnicodeNormalization;
use unicode_script::{Script, UnicodeScript};
use unicode_segmentation::UnicodeSegmentation;

/// The font a stretch of text is set in.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum SetIn {
    /// A standard font itself, nothing embedded: each character its WinAnsi code, kerned
    /// as its bundled face kerns it.
    Standard(FontChoice),
    /// The embedded face of a standard font (its bundled one) or of an installed family.
    Face(FontChoice),
}

impl SetIn {
    /// The font chosen, standard or installed.
    pub(crate) fn choice(self) -> FontChoice {
        match self {
            SetIn::Standard(f) | SetIn::Face(f) => f,
        }
    }
}

/// Text as it is set ([`Setter::lay`]): stretches, each in one font and one script.
#[derive(Debug, Default)]
pub(crate) struct Laid {
    pub runs: Vec<Run>,
}

/// A stretch of text set in one font and one script.
#[derive(Debug)]
pub(crate) struct Run {
    pub font: SetIn,
    /// What it writes, which its glyphs' characters index: its characters put together as
    /// the face writes them ([`Face::spelled`]).
    pub text: Vec<char>,
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
    laid: RefCell<HashMap<String, Rc<Laid>>>,
}

/// The font each word went to, by the font asked, whether that is written as itself, the
/// language for Chinese, Japanese and Korean, and the word; kept while the installed
/// families stay as they are (their count).
type Units = (usize, HashMap<UnitKey, Option<SetIn>>);

/// What a word's font is kept by ([`Units`]).
type UnitKey = (FontChoice, bool, Option<Language>, String);

static UNITS: LazyLock<Mutex<Units>> = LazyLock::new(Mutex::default);

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
        (self.laid.borrow_mut()).insert(text.to_string(), Rc::clone(&laid));
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
                runs.push(self.run(font, &chars[from + a..from + b], script));
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
                *units = (count, HashMap::new());
            }
            if let Some(f) = units.1.get(&key) {
                return *f;
            }
        }
        let found = self.find_font(unit);
        let mut units = UNITS.lock().unwrap_or_else(|e| e.into_inner());
        units.1.insert(key, found);
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
    /// between characters. Spaces at a line's end aren't counted, and are left out. A soft
    /// hyphen shows as a hyphen at a line's end, and nowhere else.
    pub(crate) fn lines(&self, text: &str, width: f32, size: f32) -> Vec<String> {
        break_lines(text, width, |line| self.width(line, size))
    }

    /// A stretch of the text in one font and one script, set.
    fn run(&self, font: SetIn, chars: &[char], script: Script) -> Run {
        let script = harfrust_script(script);
        let language = self.language.as_ref();
        match font {
            SetIn::Face(f) => {
                let Ok(face) = Face::of(f) else {
                    return Run {
                        font,
                        text: Vec::new(),
                        glyphs: Vec::new(),
                    };
                };
                let text = face.spelled(&chars.iter().collect::<String>()).0;
                let glyphs = face.shape(&text, script, language, false);
                Run { font, text, glyphs }
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

/// `text` in the stretches LimarePDF writes it in when it is set in `f`, in a document whose
/// language is `lang` (its `/Lang`, [`Pdf::language`]): each stretch's characters (from,
/// to) and its font, `f` or one it falls back to ([`fallbacks`]), `None` for what no font
/// can write; in order, neighbours in one font joined. The typing box draws each stretch in
/// its font, as it will be written.
pub fn set_in(
    f: FontChoice,
    text: &str,
    lang: Option<&str>,
) -> Vec<((usize, usize), Option<FontChoice>)> {
    let mut out: Vec<((usize, usize), Option<FontChoice>)> = Vec::new();
    for ((a, b), font) in Setter::new(f, text, lang).pieces(text) {
        let font = font.map(SetIn::choice);
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
        SetIn::Face(f) => Face::of(f).is_ok_and(|face| face.spelled(text).1.is_empty()),
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
/// ([`Setter::lines`]).
pub(crate) fn break_lines(text: &str, width: f32, measure: impl Fn(&str) -> f32) -> Vec<String> {
    let fits = |line: &str| measure(&shown(line)) <= width + 1e-3;
    let mut out = Vec::new();
    for para in paragraphs(text) {
        let mut line = String::new();
        let mut from = 0;
        for (at, _) in unicode_linebreak::linebreaks(para) {
            let segment = &para[from..at];
            from = at;
            if segment.is_empty() {
                continue;
            }
            let joined = format!("{line}{segment}");
            if line.is_empty() || fits(&joined) {
                line = joined;
            } else {
                out.push(shown(&line));
                line = segment.to_string();
            }
            // A word wider than a line of its own is broken where it fills one.
            while !fits(&line) {
                let clusters: Vec<&str> = line.graphemes(true).collect();
                let mut n = clusters.len().saturating_sub(1);
                while n > 1 && !fits(&clusters[..n].concat()) {
                    n -= 1;
                }
                if n == 0 {
                    break;
                }
                out.push(shown(&clusters[..n].concat()));
                line = clusters[n..].concat();
            }
        }
        out.push(shown(&line));
    }
    out
}

/// `text` split at its line breaks (`\r\n`, `\n`, `\r`, and Unicode's line and paragraph
/// separators, vertical tab, form feed and next line).
fn paragraphs(text: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut from = 0;
    let mut chars = text.char_indices().peekable();
    let ends = [
        '\n', '\r', '\u{0B}', '\u{0C}', '\u{85}', '\u{2028}', '\u{2029}',
    ];
    while let Some((i, c)) = chars.next() {
        if ends.contains(&c) {
            out.push(&text[from..i]);
            from = i + c.len_utf8();
            if c == '\r' && chars.peek().is_some_and(|&(_, n)| n == '\n') {
                chars.next();
                from += 1;
            }
        }
    }
    out.push(&text[from..]);
    out
}

/// A line as it shows: spaces at its end left out, a soft hyphen at its end shown as a
/// hyphen and others left out.
fn shown(line: &str) -> String {
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
        for l in &laid {
            out.push(shows(l, &placed, &standard, &mut fonts));
        }
        // Lines with nothing to show still name a font: the standard one asked for.
        if let (true, SetIn::Standard(f)) = (fonts.is_empty(), setter.own()) {
            let id = match standard.iter().find(|(c, _)| *c == f) {
                Some(&(_, id)) => id,
                None => {
                    let font = dictionary! {
                        "Type" => "Font", "Subtype" => "Type1", "BaseFont" => f.base_font(),
                        "Encoding" => "WinAnsiEncoding",
                    };
                    let id = self.doc.add_object(font);
                    standard.push((f, id));
                    id
                }
            };
            fonts.push(id);
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

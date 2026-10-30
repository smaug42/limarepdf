//! Installed fonts, for fonts a PDF names but doesn't embed (pure Rust, `fontdb`).
//!
//! hayro 0.8 asks for such a font only when it is a CID font (Chinese, Japanese or
//! Korean, as a rule, but also Latin fonts that some programs write as CID fonts);
//! other fonts that aren't embedded get the closest standard font without a question.
//!
//! A font is looked up by its name, then by its family with the weight and style asked
//! for, then among free fonts with the same widths (Liberation Sans for Arial). Those
//! show the document as it was made. Failing that, a stand-in is used: for the four
//! Adobe character collections, the usual fonts for the language and then any installed
//! font with its characters; otherwise a common sans, serif or monospaced font.

use fontdb::{Database, Style, ID};
use hayro::hayro_interpret::font::{FallbackFontQuery, FontData};
use hayro::hayro_interpret::hayro_cmap::{self, CMapName, CidFamily};
use skrifa::prelude::{FontRef, MetadataProvider};
use std::collections::HashMap;
use std::path::Path;
use std::sync::{Arc, LazyLock, Mutex, OnceLock, Weak};

/// Fonts to look in for fonts that aren't embedded.
pub struct InstalledFonts {
    db: Database,
    /// Font files in use, shared by every renderer while one holds them.
    data: Mutex<HashMap<ID, Weak<Vec<u8>>>>,
    /// The stand-in found for each language and sans or serif, `None` when nothing has
    /// the language's characters.
    by_language: Mutex<HashMap<(Language, bool), Option<ID>>>,
}

/// An installed font for a font that isn't embedded.
pub struct Found {
    pub data: FontData,
    /// The font's index in its file.
    pub index: u32,
    /// The installed font's family when it only looks like the font asked for, as
    /// DejaVu Sans does for Tahoma; `None` when it is that font or has its widths.
    pub stand_in: Option<String>,
}

/// A language written with a CID font's character collection.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Language {
    Japanese,
    SimplifiedChinese,
    TraditionalChinese,
    Korean,
}

impl Language {
    /// The language of a font that isn't embedded, from its character collection.
    pub fn of(q: &FallbackFontQuery) -> Option<Language> {
        Some(match q.character_collection.as_ref()?.family {
            CidFamily::AdobeJapan1 => Language::Japanese,
            CidFamily::AdobeGB1 => Language::SimplifiedChinese,
            CidFamily::AdobeCNS1 => Language::TraditionalChinese,
            CidFamily::AdobeKorea1 => Language::Korean,
            _ => return None,
        })
    }

    /// Its name, for a message: "Japanese".
    pub fn name(self) -> &'static str {
        match self {
            Language::Japanese => "Japanese",
            Language::SimplifiedChinese => "Simplified Chinese",
            Language::TraditionalChinese => "Traditional Chinese",
            Language::Korean => "Korean",
        }
    }

    /// A free font to suggest installing for it.
    pub fn suggestion(self) -> &'static str {
        self.families(false)[0]
    }

    /// The language a language tag names, when it is one of these ("ja", "zh-Hant-TW",
    /// "ko_KR.UTF-8"): Chinese is Traditional for Taiwan, Hong Kong and Macao and where the
    /// tag says so, else Simplified.
    pub(crate) fn of_tag(tag: &str) -> Option<Language> {
        let tag = tag.to_ascii_lowercase().replace('_', "-");
        let mut parts = tag.split(['-', '.', '@']);
        Some(match parts.next()? {
            "ja" => Language::Japanese,
            "ko" => Language::Korean,
            "zh" => match parts.any(|p| matches!(p, "hant" | "tw" | "hk" | "mo")) {
                true => Language::TraditionalChinese,
                false => Language::SimplifiedChinese,
            },
            _ => return None,
        })
    }

    /// The four, in the order fonts for Chinese, Japanese and Korean are tried when nothing
    /// says which the text is.
    pub(crate) const ALL: [Language; 4] = [
        Language::SimplifiedChinese,
        Language::TraditionalChinese,
        Language::Japanese,
        Language::Korean,
    ];

    /// A character any font for the language has, and fonts for the others usually don't.
    fn sample(self) -> char {
        match self {
            Language::Japanese => 'あ',
            Language::SimplifiedChinese => '国',
            Language::TraditionalChinese => '國',
            Language::Korean => '한',
        }
    }

    /// The usual fonts for the language on Linux, Windows and macOS, best first.
    pub(crate) fn families(self, serif: bool) -> &'static [&'static str] {
        match (self, serif) {
            (Language::Japanese, false) => &[
                "Noto Sans CJK JP",
                "Source Han Sans JP",
                "Source Han Sans",
                "Yu Gothic",
                "Hiragino Sans",
                "Hiragino Kaku Gothic ProN",
                "MS Gothic",
                "IPAexGothic",
                "IPAGothic",
                "TakaoGothic",
                "VL Gothic",
            ],
            (Language::Japanese, true) => &[
                "Noto Serif CJK JP",
                "Source Han Serif JP",
                "Source Han Serif",
                "Yu Mincho",
                "Hiragino Mincho ProN",
                "MS Mincho",
                "IPAexMincho",
                "IPAMincho",
                "TakaoMincho",
            ],
            (Language::SimplifiedChinese, false) => &[
                "Noto Sans CJK SC",
                "Source Han Sans SC",
                "Source Han Sans CN",
                "Microsoft YaHei",
                "PingFang SC",
                "SimHei",
                "WenQuanYi Micro Hei",
                "WenQuanYi Zen Hei",
            ],
            (Language::SimplifiedChinese, true) => &[
                "Noto Serif CJK SC",
                "Source Han Serif SC",
                "Source Han Serif CN",
                "SimSun",
                "Songti SC",
                "AR PL UMing CN",
            ],
            (Language::TraditionalChinese, false) => &[
                "Noto Sans CJK TC",
                "Source Han Sans TC",
                "Source Han Sans TW",
                "Microsoft JhengHei",
                "PingFang TC",
            ],
            (Language::TraditionalChinese, true) => &[
                "Noto Serif CJK TC",
                "Source Han Serif TC",
                "Source Han Serif TW",
                "PMingLiU",
                "MingLiU",
                "AR PL UMing TW",
            ],
            (Language::Korean, false) => &[
                "Noto Sans CJK KR",
                "Source Han Sans KR",
                "Source Han Sans K",
                "Malgun Gothic",
                "Apple SD Gothic Neo",
                "NanumGothic",
                "Dotum",
                "Gulim",
            ],
            (Language::Korean, true) => &[
                "Noto Serif CJK KR",
                "Source Han Serif KR",
                "Source Han Serif K",
                "Batang",
                "NanumMyeongjo",
                "AppleMyungjo",
            ],
        }
    }
}

/// The kind of Latin font asked for, for a stand-in.
#[derive(Clone, Copy, PartialEq)]
enum Kind {
    Sans,
    Serif,
    Mono,
}

impl Kind {
    /// Common fonts of the kind on Linux, Windows and macOS, best first. Liberation and
    /// its kin come first: they are narrower than DejaVu, so lines overrun less.
    fn families(self) -> &'static [&'static str] {
        match self {
            Kind::Sans => &[
                "Liberation Sans",
                "Arimo",
                "Arial",
                "Helvetica",
                "DejaVu Sans",
                "Noto Sans",
                "Segoe UI",
                "Helvetica Neue",
                "Roboto",
                "Open Sans",
                "Cantarell",
            ],
            Kind::Serif => &[
                "Liberation Serif",
                "Tinos",
                "Times New Roman",
                "Times",
                "DejaVu Serif",
                "Noto Serif",
                "Georgia",
                "Cambria",
            ],
            Kind::Mono => &[
                "Liberation Mono",
                "Cousine",
                "Courier New",
                "Courier",
                "DejaVu Sans Mono",
                "Noto Sans Mono",
                "Consolas",
                "Menlo",
            ],
        }
    }
}

/// Free fonts with the same character widths as common Windows fonts, as fontconfig
/// offers them, by the key of the Windows font's family.
fn substitutes(family: &str) -> &'static [&'static str] {
    match family {
        "arial" | "arialmt" => &["Liberation Sans", "Arimo"],
        "timesnewroman" | "timesnewromanps" | "timesnewromanpsmt" => &["Liberation Serif", "Tinos"],
        "couriernew" | "couriernewpsmt" => &["Liberation Mono", "Cousine"],
        "calibri" => &["Carlito"],
        "cambria" => &["Caladea"],
        _ => &[],
    }
}

/// A name with only its lower-case letters and digits, so "MS Mincho" is "MSMincho".
fn key(name: &str) -> String {
    name.chars()
        .filter(char::is_ascii_alphanumeric)
        .map(|c| c.to_ascii_lowercase())
        .collect()
}

/// A question for a font known only by its name.
fn query(name: &str) -> FallbackFontQuery {
    FallbackFontQuery {
        post_script_name: Some(name.to_string()),
        ..Default::default()
    }
}

/// The PDF's names for the font: "MSMincho", "MSGothic,Bold", "KozMinPr6N-Regular".
fn names(q: &FallbackFontQuery) -> Vec<&str> {
    [&q.post_script_name, &q.font_name]
        .into_iter()
        .flatten()
        .map(String::as_str)
        .filter(|n| !n.is_empty())
        .collect()
}

/// The font's name as a person would look for it: without the style after a comma, or
/// the CMap a Type 0 font's name ends with ("HeiseiMin-W3-UniJIS-UCS2-H" is
/// "HeiseiMin-W3").
pub fn display_name(q: &FallbackFontQuery) -> Option<String> {
    let name = names(q).into_iter().next()?;
    let name = name.split(',').next().unwrap_or(name);
    let is_cmap = |rest: &str| {
        matches!(rest, "Identity-H" | "Identity-V")
            || hayro_cmap::load_embedded(CMapName::from_bytes(rest.as_bytes())).is_some()
    };
    let cut = name
        .match_indices('-')
        .map(|(i, _)| i)
        .find(|&i| is_cmap(&name[i + 1..]));
    let name = cut.map_or(name, |i| &name[..i]);
    (!name.is_empty()).then(|| name.to_string())
}

impl InstalledFonts {
    fn new(db: Database) -> Self {
        Self {
            db,
            data: Mutex::new(HashMap::new()),
            by_language: Mutex::new(HashMap::new()),
        }
    }

    /// The fonts installed on this computer, found the first time they are needed.
    pub fn system() -> &'static InstalledFonts {
        static SYSTEM: OnceLock<InstalledFonts> = OnceLock::new();
        SYSTEM.get_or_init(|| {
            let mut db = Database::new();
            db.load_system_fonts();
            InstalledFonts::new(db)
        })
    }

    /// Only the fonts in these files (for tests). Files that can't be read are skipped.
    pub fn from_files(paths: &[&Path]) -> Self {
        let mut db = Database::new();
        for p in paths {
            let _ = db.load_font_file(p);
        }
        InstalledFonts::new(db)
    }

    /// The fonts, to offer for writing text (`typefaces.rs`).
    pub(crate) fn database(&self) -> &Database {
        &self.db
    }

    /// No fonts at all, so every font that isn't embedded gets hayro's stand-in.
    pub fn none() -> Self {
        InstalledFonts::new(Database::new())
    }

    /// The installed font to draw a font that isn't embedded with; `None` when no
    /// installed font fits, or none has the characters of the font's language.
    pub fn find(&self, q: &FallbackFontQuery) -> Option<Found> {
        let (id, stand_in) = self.find_id(q)?;
        self.found(id, stand_in)
    }

    /// The installed font a PDF's font name ("Arial", "Arial,Bold", "TimesNewRomanPS-BoldMT")
    /// stands for: that font, or one with its widths (Liberation Sans for Arial). `None`
    /// when only a stand-in is installed. For drawing text being typed in the look a line
    /// will keep.
    pub fn named(&self, name: &str) -> Option<Found> {
        match self.find_id(&query(name))? {
            (id, false) => self.found(id, false),
            (_, true) => None,
        }
    }

    fn found(&self, id: ID, stand_in: bool) -> Option<Found> {
        let face = self.db.face(id)?;
        Some(Found {
            data: self.data(id)?,
            index: face.index,
            stand_in: stand_in.then(|| {
                face.families
                    .first()
                    .map_or_else(|| face.post_script_name.clone(), |(n, _)| n.clone())
            }),
        })
    }

    /// The PostScript name of the installed font [`InstalledFonts::find`] would use.
    pub fn find_name(&self, q: &FallbackFontQuery) -> Option<String> {
        Some(self.db.face(self.find_id(q)?.0)?.post_script_name.clone())
    }

    /// The font to use, and whether it is a stand-in.
    fn find_id(&self, q: &FallbackFontQuery) -> Option<(ID, bool)> {
        let language = Language::of(q);
        let fits = |id: ID| language.is_none_or(|l| self.has(id, l.sample()));

        let names = names(q);
        let lower: Vec<String> = names.iter().map(|n| n.to_ascii_lowercase()).collect();
        let named = |words: &[&str]| lower.iter().any(|n| words.iter().any(|w| n.contains(w)));
        let weight = if q.is_bold || named(&["bold", "black", "heavy"]) {
            q.font_weight.max(700)
        } else {
            q.font_weight
        };
        let italic = q.is_italic || named(&["italic", "oblique"]);

        for name in &names {
            let k = key(name);
            if let Some(f) = self
                .db
                .faces()
                .find(|f| key(&f.post_script_name) == k && fits(f.id))
            {
                return Some((f.id, false));
            }
        }
        let families: Vec<String> = q
            .font_family
            .iter()
            .map(String::as_str)
            .chain(
                names
                    .iter()
                    .flat_map(|n| [n.split(',').next(), n.split(['-', ',']).next()])
                    .flatten(),
            )
            .map(key)
            .collect();
        for family in &families {
            if let Some(id) = self.in_family(family, weight, italic, &fits) {
                return Some((id, false));
            }
        }
        for family in families.iter().flat_map(|f| substitutes(f)) {
            if let Some(id) = self.in_family(&key(family), weight, italic, &fits) {
                return Some((id, false));
            }
        }

        let serif_names = [
            "times", "serif", "georgia", "garamond", "cambria", "palatino",
        ];
        let serif = q.is_serif
            || (named(&serif_names) && !named(&["sans"]))
            || named(&["mincho", "song", "ming", "batang", "myeongjo"]);
        if let Some(language) = language {
            let mut by_language = self.by_language.lock().unwrap_or_else(|e| e.into_inner());
            return by_language
                .entry((language, serif))
                .or_insert_with(|| self.for_language(language, serif))
                .map(|id| (id, true));
        }
        let kind = if q.is_fixed_pitch || named(&["mono", "courier", "consol", "typewriter"]) {
            Kind::Mono
        } else if serif {
            Kind::Serif
        } else {
            Kind::Sans
        };
        [kind, Kind::Sans, Kind::Serif, Kind::Mono]
            .into_iter()
            .flat_map(Kind::families)
            .find_map(|family| self.in_family(&key(family), weight, italic, &|_| true))
            .or_else(|| {
                // Some installed font with Latin letters, regular, close to the weight.
                self.db
                    .faces()
                    .filter(|f| f.style == Style::Normal && self.has(f.id, 'A'))
                    .min_by_key(|f| (f.weight.0 as i64 - weight as i64).abs())
                    .map(|f| f.id)
            })
            .map(|id| (id, true))
    }

    /// The face of a family closest to the weight and style asked for.
    fn in_family(
        &self,
        family: &str,
        weight: u32,
        italic: bool,
        fits: &dyn Fn(ID) -> bool,
    ) -> Option<ID> {
        if family.is_empty() {
            return None;
        }
        self.db
            .faces()
            .filter(|f| f.families.iter().any(|(n, _)| key(n) == family))
            .min_by_key(|f| {
                let slanted = f.style != Style::Normal;
                (slanted != italic, (f.weight.0 as i64 - weight as i64).abs())
            })
            .map(|f| f.id)
            .filter(|&id| fits(id))
    }

    fn for_language(&self, language: Language, serif: bool) -> Option<ID> {
        let sample = language.sample();
        [serif, !serif]
            .into_iter()
            .flat_map(|s| language.families(s))
            .find_map(|family| self.in_family(&key(family), 400, false, &|id| self.has(id, sample)))
            .or_else(|| {
                self.db
                    .faces()
                    .filter(|f| f.style == Style::Normal && self.has(f.id, sample))
                    .min_by_key(|f| (f.weight.0 as i64 - 400).abs())
                    .map(|f| f.id)
            })
    }

    /// Whether a font has a glyph for a character.
    fn has(&self, id: ID, c: char) -> bool {
        self.db
            .with_face_data(id, |data, index| {
                FontRef::from_index(data, index).is_ok_and(|f| f.charmap().map(c).is_some())
            })
            .unwrap_or(false)
    }

    fn data(&self, id: ID) -> Option<FontData> {
        let mut data = self.data.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(d) = data.get(&id).and_then(Weak::upgrade) {
            return Some(d);
        }
        let d = Arc::new(self.db.with_face_data(id, |bytes, _| bytes.to_vec())?);
        data.insert(id, Arc::downgrade(&d));
        Some(d)
    }
}

/// The installed faces' answers to [`maps_all`], by font file and index, then character.
type Mapped = HashMap<(std::path::PathBuf, u32), HashMap<char, bool>>;

/// Font files [`maps_all`] read itself, not being in the system's font list: kept, or
/// `None` where they can't be read.
type Others = HashMap<std::path::PathBuf, Option<Arc<Vec<u8>>>>;

/// Whether the face at `index` in the font file `path` maps every one of `chars` to a glyph:
/// read through the system's font list, which maps the file into memory only while it is
/// read, so looking through every installed font keeps none of them; a file not in the list
/// (the tests' fonts) is read once and kept. The answers are kept for the rest of the run.
pub(crate) fn maps_all(path: &Path, index: u32, chars: &[char]) -> bool {
    static ANSWERS: LazyLock<Mutex<Mapped>> = LazyLock::new(Mutex::default);
    static IDS: OnceLock<HashMap<(std::path::PathBuf, u32), ID>> = OnceLock::new();
    static OTHERS: LazyLock<Mutex<Others>> = LazyLock::new(Mutex::default);
    let key = (path.to_path_buf(), index);
    let mut answers = ANSWERS.lock().unwrap_or_else(|e| e.into_inner());
    let known = answers.entry(key.clone()).or_default();
    let ask: Vec<char> = (chars.iter().copied())
        .filter(|c| !known.contains_key(c))
        .collect();
    if !ask.is_empty() {
        let read = |data: &[u8], index: u32| -> Vec<bool> {
            match FontRef::from_index(data, index) {
                Ok(f) => {
                    let map = f.charmap();
                    ask.iter().map(|&c| map.map(c).is_some()).collect()
                }
                Err(_) => vec![false; ask.len()],
            }
        };
        let system = InstalledFonts::system();
        let ids = IDS.get_or_init(|| {
            (system.db.faces())
                .filter_map(|f| match &f.source {
                    fontdb::Source::File(p) | fontdb::Source::SharedFile(p, _) => {
                        Some(((p.clone(), f.index), f.id))
                    }
                    fontdb::Source::Binary(_) => None,
                })
                .collect()
        });
        let got = match ids.get(&key) {
            Some(&id) => system.db.with_face_data(id, read),
            None => {
                let mut others = OTHERS.lock().unwrap_or_else(|e| e.into_inner());
                let data = others
                    .entry(key.0.clone())
                    .or_insert_with(|| std::fs::read(path).ok().map(Arc::new));
                data.as_ref().map(|d| read(d, index))
            }
        }
        .unwrap_or_else(|| vec![false; ask.len()]);
        known.extend(ask.into_iter().zip(got));
    }
    chars.iter().all(|c| known.get(c) == Some(&true))
}

#[cfg(test)]
mod tests {
    use super::*;
    use hayro::hayro_interpret::hayro_cmap::CharacterCollection;

    fn bundled() -> InstalledFonts {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("assets/fonts");
        let caveat = dir.join("Caveat.ttf");
        let zeyada = dir.join("Zeyada.ttf");
        InstalledFonts::from_files(&[&caveat, &zeyada])
    }

    fn japanese(name: &str) -> FallbackFontQuery {
        FallbackFontQuery {
            character_collection: Some(CharacterCollection {
                family: CidFamily::AdobeJapan1,
                supplement: 2,
            }),
            ..query(name)
        }
    }

    #[test]
    fn finds_by_name_and_family() {
        let fonts = bundled();
        let found = |name: &str| {
            let f = fonts.find(&query(name)).unwrap();
            (fonts.find_name(&query(name)).unwrap(), f.stand_in)
        };
        assert_eq!(found("Zeyada"), ("Zeyada".into(), None));
        // A style after a comma, as Acrobat writes it, and a style after a hyphen.
        assert_eq!(found("Zeyada,Bold"), ("Zeyada".into(), None));
        assert_eq!(found("Zeyada-Italic"), ("Zeyada".into(), None));
        let (data, index) = {
            let f = fonts.find(&query("Zeyada")).unwrap();
            (f.data, f.index)
        };
        assert!(!data.as_ref().as_ref().is_empty());
        assert_eq!(index, 0);
    }

    /// A font file read for drawing is held only while a renderer holds it: the list keeps a
    /// weak reference, so a file that turns out not to be readable as a font goes too
    /// (PB-13).
    #[test]
    fn font_files_go_when_nothing_holds_them() {
        let fonts = bundled();
        let (id, _) = fonts.find_id(&query("Zeyada")).unwrap();
        let held = |f: &InstalledFonts| {
            let data = f.data.lock().unwrap();
            data.get(&id).and_then(Weak::upgrade).is_some()
        };
        let found = fonts.find(&query("Zeyada")).unwrap();
        assert!(held(&fonts));
        drop(found);
        assert!(!held(&fonts));
    }

    #[test]
    fn named_is_the_font_or_its_widths() {
        // The bundled Liberation fonts stand in for installed ones here.
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("assets/fonts");
        let regular = dir.join("LiberationSans-Regular.ttf");
        let bold = dir.join("LiberationSans-Bold.ttf");
        let fonts = InstalledFonts::from_files(&[&regular, &bold]);
        let name = |n: &str| {
            fonts.named(n)?;
            fonts.find_name(&query(n))
        };
        assert_eq!(
            name("LiberationSans-Bold").as_deref(),
            Some("LiberationSans-Bold")
        );
        assert_eq!(name("Liberation Sans").as_deref(), Some("LiberationSans"));
        // The same widths count as the font.
        assert_eq!(name("Arial").as_deref(), Some("LiberationSans"));
        assert_eq!(name("Arial,Bold").as_deref(), Some("LiberationSans-Bold"));
        assert_eq!(name("ArialMT").as_deref(), Some("LiberationSans"));
        // Only stand-ins: not the font.
        assert!(fonts.find(&query("Tahoma")).is_some());
        assert_eq!(name("Tahoma"), None);
        assert_eq!(name("Times New Roman"), None);
        assert!(InstalledFonts::none().named("Arial").is_none());
        let found = fonts.named("Arial").unwrap();
        assert!(!found.data.as_ref().as_ref().is_empty());
        assert_eq!(found.stand_in, None);
    }

    #[test]
    fn stand_ins_are_said_so() {
        // Nothing named Calibri, and none of the usual fonts: any font with letters.
        let fonts = bundled();
        let f = fonts.find(&query("Calibri")).unwrap();
        assert!(f.stand_in.is_some());
        assert!(fonts.find(&query("")).unwrap().stand_in.is_some());
        assert!(InstalledFonts::none().find(&query("Calibri")).is_none());
    }

    #[test]
    fn language_needs_its_characters() {
        // A Latin font named in a Japanese font isn't used: it has no Japanese.
        let fonts = bundled();
        assert!(fonts.find(&japanese("Zeyada")).is_none());
        assert!(InstalledFonts::none().find(&japanese("Zeyada")).is_none());
        assert_eq!(Language::of(&japanese("x")), Some(Language::Japanese));
        assert_eq!(Language::of(&query("x")), None);
        assert_eq!(Language::Japanese.suggestion(), "Noto Sans CJK JP");
    }

    #[test]
    fn display_names() {
        let name = |n: &str| display_name(&query(n)).unwrap_or_default();
        assert_eq!(name("HeiseiMin-W3-UniJIS-UCS2-H"), "HeiseiMin-W3");
        assert_eq!(name("MS-Mincho,Bold-UniJIS-UTF16-H"), "MS-Mincho");
        assert_eq!(name("Tahoma,Italic"), "Tahoma");
        assert_eq!(name("TimesNewRomanPSMT-Identity-H"), "TimesNewRomanPSMT");
        assert_eq!(name("KozMinPr6N-Regular"), "KozMinPr6N-Regular");
        assert_eq!(display_name(&query("")), None);
    }

    #[test]
    fn installed_fonts() {
        // Only meaningful where these fonts are installed; passes trivially otherwise.
        let fonts = InstalledFonts::system();
        let has_family = |f: &str| {
            fonts
                .db
                .faces()
                .any(|face| face.families.iter().any(|(n, _)| n == f))
        };
        if has_family("Liberation Sans") && !has_family("Arial") {
            assert_eq!(
                fonts.find_name(&query("Arial,Bold")).as_deref(),
                Some("LiberationSans-Bold")
            );
            assert_eq!(fonts.find(&query("Arial")).unwrap().stand_in, None);
        }
        if has_family("Liberation Sans") && !has_family("Tahoma") {
            let f = fonts.find(&query("Tahoma,Bold")).unwrap();
            assert_eq!(f.stand_in.as_deref(), Some("Liberation Sans"));
        }
        if let Some(f) = fonts.find(&japanese("MSMincho")) {
            let (id, _) = fonts.find_id(&japanese("MSMincho")).unwrap();
            assert!(fonts.has(id, 'あ'), "{:?}", f.stand_in);
        }
    }
}

//! Spelling (NEW-06): the dictionaries LimarePDF comes with and those added to it, the words of
//! a text that are checked, and checking them and suggesting others through `spellbook`, which
//! reads Hunspell's dictionaries (an `.aff` file of rules and a `.dic` file of words).
//!
//! Nothing here goes through a document: a caller hands in the text being typed, and a
//! [`Speller`] keeps each word's answer, so typing checks only the words that changed.

use std::collections::{HashMap, HashSet};
use std::io::{Cursor, Read};
use std::ops::Range;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use unicode_segmentation::UnicodeSegmentation;

/// The most suggestions offered for a word, as word processors' menus show a few.
pub const SUGGESTIONS: usize = 5;

/// The largest dictionary file read (the biggest Hunspell dictionaries are about 20 MB).
const MAX_FILE: u64 = 64 << 20;

/// A dictionary LimarePDF comes with.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BuiltIn {
    /// Its language, as a PDF's `/Lang` names it: "en-CA".
    pub tag: &'static str,
    /// Its name in menus: "English (Canada)".
    pub name: &'static str,
    /// Where it is in [`PACKED`].
    index: usize,
}

/// The dictionaries LimarePDF comes with, Canadian English first: SCOWL's English ones and
/// Grammalecte's French one (`assets/dictionaries/README.md`).
pub const BUILT_IN: [BuiltIn; 5] = [
    BuiltIn {
        tag: "en-CA",
        name: "English (Canada)",
        index: 0,
    },
    BuiltIn {
        tag: "en-US",
        name: "English (United States)",
        index: 1,
    },
    BuiltIn {
        tag: "en-GB",
        name: "English (United Kingdom)",
        index: 2,
    },
    BuiltIn {
        tag: "en-AU",
        name: "English (Australia)",
        index: 3,
    },
    BuiltIn {
        tag: "fr",
        name: "French",
        index: 4,
    },
];

/// A bundled dictionary file as stored: packed with zlib by `build.rs`.
macro_rules! packed {
    ($file:literal) => {
        include_bytes!(concat!(env!("OUT_DIR"), "/", $file, ".z"))
    };
}

/// The bundled dictionaries' `.aff` and `.dic` files, in [`BUILT_IN`]'s order, as stored.
static PACKED: [(&[u8], &[u8]); 5] = [
    (packed!("en_CA.aff"), packed!("en_CA.dic")),
    (packed!("en_US.aff"), packed!("en_US.dic")),
    (packed!("en_GB-ise.aff"), packed!("en_GB-ise.dic")),
    (packed!("en_AU.aff"), packed!("en_AU.dic")),
    (packed!("fr.aff"), packed!("fr.dic")),
];

impl BuiltIn {
    /// The bundled dictionary for `tag`, exactly as [`BUILT_IN`] names it.
    pub fn get(tag: &str) -> Option<BuiltIn> {
        BUILT_IN.iter().copied().find(|b| b.tag == tag)
    }

    /// Its `.aff` and `.dic` files, unpacked: byte for byte those in `assets/dictionaries`.
    pub fn files(&self) -> (Vec<u8>, Vec<u8>) {
        let (aff, dic) = PACKED[self.index];
        (unpack(aff), unpack(dic))
    }
}

fn unpack(packed: &[u8]) -> Vec<u8> {
    let mut data = Vec::new();
    flate2::read::ZlibDecoder::new(packed)
        .read_to_end(&mut data)
        .expect("a bundled dictionary unpacks");
    data
}

/// Why a dictionary couldn't be used, in words for the person who chose it.
#[derive(Debug, thiserror::Error)]
pub enum SpellingError {
    #[error("{0} couldn't be read: {1}")]
    Read(String, std::io::Error),
    #[error("{0} is written in {1}, a character set LimarePDF can't read")]
    Encoding(String, String),
    #[error("{0} isn't a dictionary LimarePDF can read: {1}")]
    Parse(String, String),
    #[error("{0} has no spelling dictionary in it")]
    NoDictionary(String),
    #[error("{0} is too large to be a dictionary")]
    TooLarge(String),
}

/// A spelling dictionary, ready to check words.
pub struct Dictionary(spellbook::Dictionary);

impl Dictionary {
    /// A bundled dictionary, unpacked and read (a few tens of milliseconds: not on a frame).
    pub fn built_in(b: BuiltIn) -> Result<Dictionary, SpellingError> {
        let (aff, dic) = b.files();
        Dictionary::from_bytes(b.name, &aff, &dic)
    }

    /// A dictionary from its two files' contents, in the character set its `.aff` file names
    /// (UTF-8, ISO 8859-1 or 15; Hunspell's default is ISO 8859-1). `name` names it in errors.
    pub fn from_bytes(name: &str, aff: &[u8], dic: &[u8]) -> Result<Dictionary, SpellingError> {
        let set = character_set(aff);
        let decode = |bytes: &[u8]| {
            decode(bytes, &set).ok_or_else(|| SpellingError::Encoding(name.into(), set.clone()))
        };
        let (aff, dic) = (decode(aff)?, decode(dic)?);
        spellbook::Dictionary::new(&aff, &dic)
            .map(Dictionary)
            .map_err(|e| SpellingError::Parse(name.into(), e.to_string()))
    }

    /// A dictionary from its two files on disk.
    pub fn from_files(aff: &Path, dic: &Path) -> Result<Dictionary, SpellingError> {
        let name = file_name(dic);
        let (aff, dic) = (read_limited(aff)?, read_limited(dic)?);
        Dictionary::from_bytes(&name, &aff, &dic)
    }

    /// Whether `word` is spelt as the dictionary has it.
    pub fn check(&self, word: &str) -> bool {
        self.0.check(word)
    }

    /// The dictionary's suggestions for `word`, the likeliest first, at most [`SUGGESTIONS`]
    /// (tens of milliseconds: asked for when the menu opens, never while typing). A word
    /// with a capital first ("Teh") is offered its small-letter form's, capitalised ("The"),
    /// as Hunspell offers them: spellbook alone misses some.
    pub fn suggest(&self, word: &str) -> Vec<String> {
        let mut out = Vec::new();
        let mut chars = word.chars();
        let first = chars.next();
        let rest = chars.as_str();
        let capital =
            first.is_some_and(char::is_uppercase) && !rest.chars().any(char::is_uppercase);
        if let (true, Some(first)) = (capital, first) {
            let mut small = Vec::new();
            self.0
                .suggest(&format!("{}{rest}", first.to_lowercase()), &mut small);
            out.extend(small.into_iter().map(|s| capitalised(&s)));
        }
        let mut own = Vec::new();
        self.0.suggest(word, &mut own);
        for s in own {
            if !out.contains(&s) {
                out.push(s);
            }
        }
        out.truncate(SUGGESTIONS);
        out
    }
}

/// `word` with its first letter a capital.
fn capitalised(word: &str) -> String {
    let mut chars = word.chars();
    chars
        .next()
        .map_or_else(String::new, |c| c.to_uppercase().chain(chars).collect())
}

fn file_name(path: &Path) -> String {
    path.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.display().to_string())
}

fn read_limited(path: &Path) -> Result<Vec<u8>, SpellingError> {
    let name = file_name(path);
    let file = std::fs::File::open(path).map_err(|e| SpellingError::Read(name.clone(), e))?;
    let mut data = Vec::new();
    file.take(MAX_FILE + 1)
        .read_to_end(&mut data)
        .map_err(|e| SpellingError::Read(name.clone(), e))?;
    if data.len() as u64 > MAX_FILE {
        return Err(SpellingError::TooLarge(name));
    }
    Ok(data)
}

/// The character set an `.aff` file names on its `SET` line, upper case; Hunspell's default,
/// ISO8859-1, without one.
fn character_set(aff: &[u8]) -> String {
    let aff = aff.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(aff);
    for line in aff.split(|&b| b == b'\n') {
        let line = String::from_utf8_lossy(line);
        let mut parts = line.split_whitespace();
        if parts.next() == Some("SET") {
            if let Some(set) = parts.next() {
                return set.to_ascii_uppercase();
            }
        }
    }
    "ISO8859-1".into()
}

/// `bytes` read in the character set `set`; `None` for a set not read here, or bytes that
/// aren't in it.
fn decode(bytes: &[u8], set: &str) -> Option<String> {
    let bytes = bytes.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(bytes);
    match set.replace(['-', '_'], "").as_str() {
        "UTF8" => String::from_utf8(bytes.to_vec()).ok(),
        "ISO88591" => Some(bytes.iter().map(|&b| char::from(b)).collect()),
        // ISO 8859-15 is 8859-1 with eight letters changed, the euro sign among them.
        "ISO885915" => Some(
            bytes
                .iter()
                .map(|&b| match b {
                    0xA4 => '€',
                    0xA6 => 'Š',
                    0xA8 => 'š',
                    0xB4 => 'Ž',
                    0xB8 => 'ž',
                    0xBC => 'Œ',
                    0xBD => 'œ',
                    0xBE => 'Ÿ',
                    b => char::from(b),
                })
                .collect(),
        ),
        _ => None,
    }
}

/// The words of `text` that are checked, by where they are in it (bytes). Words joined by a
/// hyphen are one ("well-known"), checked whole and then by part. Left out, as word processors
/// leave them out: words in capitals, words with digits, and web, email and file addresses.
pub fn words(text: &str) -> Vec<Range<usize>> {
    let skipped = addresses(text);
    let segments: Vec<(usize, &str)> = text.split_word_bound_indices().collect();
    let wordy = |s: &str| s.chars().any(char::is_alphabetic);
    let hyphen = |s: &str| matches!(s, "-" | "\u{2010}" | "\u{2011}");
    let mut out = Vec::new();
    let mut i = 0;
    while i < segments.len() {
        let (start, s) = segments[i];
        if !wordy(s) {
            i += 1;
            continue;
        }
        let mut end = start + s.len();
        while i + 2 < segments.len() && hyphen(segments[i + 1].1) && wordy(segments[i + 2].1) {
            i += 2;
            end = segments[i].0 + segments[i].1.len();
        }
        i += 1;
        let word = &text[start..end];
        let in_address = skipped.iter().any(|r| r.start < end && start < r.end);
        if !in_address && !word.chars().any(char::is_numeric) && !capitals(word) {
            out.push(start..end);
        }
    }
    out
}

/// A word of two or more letters, all capitals ("PDF", "NASA").
fn capitals(word: &str) -> bool {
    let letters = word.chars().filter(|c| c.is_alphabetic()).count();
    letters >= 2 && !word.chars().any(char::is_lowercase)
}

/// The web, email and file addresses in `text`: each run of text between spaces that reads as
/// one.
fn addresses(text: &str) -> Vec<Range<usize>> {
    let mut out = Vec::new();
    let mut start = None;
    for (i, c) in text
        .char_indices()
        .chain(std::iter::once((text.len(), ' ')))
    {
        match (c.is_whitespace(), start) {
            (false, None) => start = Some(i),
            (true, Some(s)) => {
                if address(&text[s..i]) {
                    out.push(s..i);
                }
                start = None;
            }
            _ => {}
        }
    }
    out
}

fn address(chunk: &str) -> bool {
    let chunk = chunk.trim_matches(|c: char| "()[]{}<>\"'“”‘’«».,;:!?".contains(c));
    let lower = chunk.to_lowercase();
    let email = chunk.split_once('@').is_some_and(|(name, domain)| {
        !name.is_empty() && domain.contains('.') && !domain.starts_with('.')
    });
    let drive = {
        let mut c = chunk.chars();
        c.next().is_some_and(|c| c.is_ascii_alphabetic())
            && c.next() == Some(':')
            && matches!(c.next(), Some('\\' | '/'))
    };
    lower.contains("://")
        || lower.starts_with("www.")
        || email
        || drive
        || chunk.contains('\\')
        || chunk.starts_with('/')
        || chunk.starts_with("~/")
}

/// Words a person has added to their dictionary, and those ignored for this session.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Exceptions {
    /// Added to the dictionary: kept between sessions, for every language, as word processors
    /// keep a custom dictionary.
    pub added: HashSet<String>,
    /// Ignore All: for this session only.
    pub ignored: HashSet<String>,
}

impl Exceptions {
    /// Whether `word` was added or ignored: as written, or added in small letters (which
    /// covers it in any capitals), as a word processor's custom dictionary does.
    pub fn covers(&self, word: &str) -> bool {
        let word = straight(word);
        let lower = word.to_lowercase();
        [&self.added, &self.ignored]
            .iter()
            .any(|set| set.contains(&word) || set.contains(&lower))
    }
}

/// A word with its curly apostrophes made straight, as a custom dictionary keeps it.
pub fn straight(word: &str) -> String {
    word.replace('’', "'")
}

/// The words of a word list file: one a line, blank lines left out.
pub fn read_word_list(text: &str) -> HashSet<String> {
    (text.lines())
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .map(straight)
        .collect()
}

/// A word list file for `words`: one a line, in order.
pub fn word_list(words: &HashSet<String>) -> String {
    let mut words: Vec<&String> = words.iter().collect();
    words.sort();
    words.iter().map(|w| format!("{w}\n")).collect()
}

/// A dictionary and the answers it has given, so a word is looked up once.
pub struct Speller {
    dictionary: Arc<Dictionary>,
    known: HashMap<String, bool>,
}

impl Speller {
    pub fn new(dictionary: Arc<Dictionary>) -> Speller {
        Speller {
            dictionary,
            known: HashMap::new(),
        }
    }

    pub fn dictionary(&self) -> &Arc<Dictionary> {
        &self.dictionary
    }

    /// Whether `word` is spelt correctly: in the dictionary, or covered by `except`.
    pub fn is_known(&mut self, word: &str, except: &Exceptions) -> bool {
        if except.covers(word) {
            return true;
        }
        if let Some(&known) = self.known.get(word) {
            return known;
        }
        let known = self.dictionary.check(word);
        self.known.insert(word.to_owned(), known);
        known
    }

    /// The words of `text` spelt wrongly, by where they are in it (bytes). Words joined by
    /// hyphens and not known together are known if each part is; else each part that isn't is
    /// wrong.
    pub fn misspelt(&mut self, text: &str, except: &Exceptions) -> Vec<Range<usize>> {
        let mut out = Vec::new();
        for r in words(text) {
            let word = &text[r.clone()];
            if self.is_known(word, except) {
                continue;
            }
            let parts: Vec<Range<usize>> = (word.split_word_bound_indices())
                .filter(|(_, s)| s.chars().any(char::is_alphabetic))
                .map(|(i, s)| r.start + i..r.start + i + s.len())
                .collect();
            if parts.len() < 2 {
                out.push(r);
                continue;
            }
            for p in parts {
                if !self.is_known(&text[p.clone()], except) {
                    out.push(p);
                }
            }
        }
        out
    }

    /// Suggestions for `word` (see [`Dictionary::suggest`]).
    pub fn suggest(&self, word: &str) -> Vec<String> {
        self.dictionary.suggest(word)
    }
}

/// A dictionary found in a folder: a `.dic` file with the `.aff` file of the same name.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Found {
    /// The file name without its extension: "de_DE_frami".
    pub stem: String,
    /// Its language, read from the file name: "de-DE"; the name itself when it doesn't start
    /// with one.
    pub tag: String,
    /// Its name in menus: "German (Germany)".
    pub name: String,
    pub aff: PathBuf,
    pub dic: PathBuf,
}

/// Where the system keeps Hunspell dictionaries: Linux's shared folders; none on Windows and
/// macOS, which have none.
pub fn system_folders() -> Vec<PathBuf> {
    if cfg!(all(unix, not(target_os = "macos"))) {
        [
            "/usr/share/hunspell",
            "/usr/share/myspell",
            "/usr/share/myspell/dicts",
            "/usr/local/share/hunspell",
        ]
        .iter()
        .map(PathBuf::from)
        .collect()
    } else {
        Vec::new()
    }
}

/// The dictionaries in `folders`, each a `.dic` file with its `.aff` beside it (a
/// hyphenation pattern file, also `.dic`, has none), by name; a folder that can't be read
/// has none.
pub fn find(folders: &[PathBuf]) -> Vec<Found> {
    let mut out: Vec<Found> = Vec::new();
    for folder in folders {
        let Ok(entries) = std::fs::read_dir(folder) else {
            continue;
        };
        let mut here: Vec<Found> = entries
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.extension().is_some_and(|x| x.eq_ignore_ascii_case("dic")))
            .filter_map(|dic| {
                let aff = dic.with_extension("aff");
                let stem = dic.file_stem()?.to_string_lossy().into_owned();
                aff.is_file().then(|| found(stem, aff, dic))
            })
            .collect();
        here.sort_by(|a, b| a.stem.cmp(&b.stem));
        // A folder linked from another (myspell's dicts to hunspell) lists the same files.
        here.retain(|f| !out.iter().any(|o| o.stem == f.stem));
        out.extend(here);
    }
    out
}

fn found(stem: String, aff: PathBuf, dic: PathBuf) -> Found {
    let tag = tag_of(&stem).unwrap_or_else(|| stem.clone());
    let name = language_name(&tag).unwrap_or_else(|| stem.clone());
    Found {
        stem,
        tag,
        name,
        aff,
        dic,
    }
}

/// The language a dictionary's file name starts with: "de_DE_frami" is "de-DE", "fr-classique"
/// "fr".
fn tag_of(stem: &str) -> Option<String> {
    let mut parts = stem.split(['_', '-']);
    let lang = parts.next()?;
    let lang_ok = (2..=3).contains(&lang.len()) && lang.chars().all(|c| c.is_ascii_lowercase());
    if !lang_ok {
        return None;
    }
    match parts.next() {
        Some(r) if r.len() == 2 && r.chars().all(|c| c.is_ascii_uppercase()) => {
            Some(format!("{lang}-{r}"))
        }
        _ => Some(lang.to_owned()),
    }
}

/// A language tag's name in menus: "fr-CA" is "French (Canada)"; `None` for a language not
/// listed here. A region not listed is shown as written: "German (LI)".
pub fn language_name(tag: &str) -> Option<String> {
    let tag = tag.replace('_', "-");
    let mut parts = tag.split('-');
    let lang = parts.next()?.to_ascii_lowercase();
    let name = LANGUAGES.iter().find(|(c, _)| *c == lang)?.1;
    match parts.next().filter(|r| r.len() == 2) {
        Some(r) => {
            let r = r.to_ascii_uppercase();
            let region = (REGIONS.iter().find(|(c, _)| *c == r)).map_or(r.as_str(), |(_, n)| n);
            Some(format!("{name} ({region})"))
        }
        None => Some(name.to_owned()),
    }
}

/// Where among `tags` (dictionaries' languages, in order of preference) the one for the
/// language `want` is: a PDF's `/Lang` ("fr-CA") or a system locale ("fr_CA.UTF-8"). The same
/// language and region first, else the same language.
pub fn best_match<'a>(want: &str, tags: impl IntoIterator<Item = &'a str>) -> Option<usize> {
    let norm = |t: &str| {
        let t = t.split(['.', '@']).next().unwrap_or("");
        t.replace('_', "-").to_ascii_lowercase()
    };
    let want = norm(want);
    let lang = |t: &str| t.split('-').next().unwrap_or("").to_owned();
    let tags: Vec<String> = tags.into_iter().map(norm).collect();
    if want.is_empty() {
        return None;
    }
    (tags.iter().position(|t| *t == want))
        .or_else(|| tags.iter().position(|t| lang(t) == lang(&want)))
}

/// A dictionary unpacked from a dictionary extension.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Unpacked {
    /// Its file name without the extension, made safe to keep as a file name.
    pub stem: String,
    pub aff: Vec<u8>,
    pub dic: Vec<u8>,
}

/// The spelling dictionaries in a LibreOffice or OpenOffice dictionary extension (`.oxt`, a
/// zip file): each `.dic` with an `.aff` of the same name. `name` names the file in errors.
pub fn in_extension(data: &[u8], name: &str) -> Result<Vec<Unpacked>, SpellingError> {
    let unreadable = |e: zip::result::ZipError| SpellingError::Parse(name.into(), e.to_string());
    let mut zip = zip::ZipArchive::new(Cursor::new(data)).map_err(unreadable)?;
    let mut affs: HashMap<String, Vec<u8>> = HashMap::new();
    let mut dics: Vec<(String, Vec<u8>)> = Vec::new();
    for i in 0..zip.len() {
        let mut file = zip.by_index(i).map_err(unreadable)?;
        if !file.is_file() {
            continue;
        }
        let path = file.name().map_err(unreadable)?.into_owned();
        let leaf = path.rsplit('/').next().unwrap_or(&path);
        let Some((stem, ext)) = leaf.rsplit_once('.') else {
            continue;
        };
        let (aff, dic) = (
            ext.eq_ignore_ascii_case("aff"),
            ext.eq_ignore_ascii_case("dic"),
        );
        if !aff && !dic {
            continue;
        }
        if file.size() > MAX_FILE {
            return Err(SpellingError::TooLarge(name.into()));
        }
        let mut bytes = Vec::new();
        (&mut file)
            .take(MAX_FILE + 1)
            .read_to_end(&mut bytes)
            .map_err(|e| SpellingError::Read(name.into(), e))?;
        if bytes.len() as u64 > MAX_FILE {
            return Err(SpellingError::TooLarge(name.into()));
        }
        let stem = safe_stem(stem);
        if aff {
            affs.insert(stem, bytes);
        } else {
            dics.push((stem, bytes));
        }
    }
    let mut out: Vec<Unpacked> = dics
        .into_iter()
        .filter_map(|(stem, dic)| {
            let aff = affs.remove(&stem)?;
            Some(Unpacked { stem, aff, dic })
        })
        .collect();
    out.sort_by(|a, b| a.stem.cmp(&b.stem));
    if out.is_empty() {
        return Err(SpellingError::NoDictionary(name.into()));
    }
    Ok(out)
}

/// A file name's stem with anything but letters, digits, `-`, `_` and `.` made `_`, and no
/// leading dot.
fn safe_stem(stem: &str) -> String {
    let s: String = stem
        .chars()
        .map(|c| match c {
            'a'..='z' | 'A'..='Z' | '0'..='9' | '-' | '_' | '.' => c,
            _ => '_',
        })
        .collect();
    let s = s.trim_start_matches('.');
    if s.is_empty() {
        "dictionary".into()
    } else {
        s.into()
    }
}

/// Languages by their two- or three-letter codes (ISO 639), for dictionaries' names.
const LANGUAGES: &[(&str, &str)] = &[
    ("af", "Afrikaans"),
    ("an", "Aragonese"),
    ("ar", "Arabic"),
    ("ast", "Asturian"),
    ("be", "Belarusian"),
    ("bg", "Bulgarian"),
    ("bn", "Bengali"),
    ("br", "Breton"),
    ("bs", "Bosnian"),
    ("ca", "Catalan"),
    ("cs", "Czech"),
    ("cy", "Welsh"),
    ("da", "Danish"),
    ("de", "German"),
    ("el", "Greek"),
    ("en", "English"),
    ("eo", "Esperanto"),
    ("es", "Spanish"),
    ("et", "Estonian"),
    ("eu", "Basque"),
    ("fa", "Persian"),
    ("fi", "Finnish"),
    ("fo", "Faroese"),
    ("fr", "French"),
    ("fy", "Western Frisian"),
    ("ga", "Irish"),
    ("gd", "Scottish Gaelic"),
    ("gl", "Galician"),
    ("gu", "Gujarati"),
    ("he", "Hebrew"),
    ("hi", "Hindi"),
    ("hr", "Croatian"),
    ("hu", "Hungarian"),
    ("hy", "Armenian"),
    ("id", "Indonesian"),
    ("is", "Icelandic"),
    ("it", "Italian"),
    ("ka", "Georgian"),
    ("kk", "Kazakh"),
    ("ko", "Korean"),
    ("ku", "Kurdish"),
    ("la", "Latin"),
    ("lb", "Luxembourgish"),
    ("lo", "Lao"),
    ("lt", "Lithuanian"),
    ("lv", "Latvian"),
    ("mk", "Macedonian"),
    ("mn", "Mongolian"),
    ("mr", "Marathi"),
    ("ms", "Malay"),
    ("mt", "Maltese"),
    ("nb", "Norwegian Bokmål"),
    ("ne", "Nepali"),
    ("nl", "Dutch"),
    ("nn", "Norwegian Nynorsk"),
    ("no", "Norwegian"),
    ("oc", "Occitan"),
    ("pa", "Punjabi"),
    ("pl", "Polish"),
    ("pt", "Portuguese"),
    ("ro", "Romanian"),
    ("ru", "Russian"),
    ("rw", "Kinyarwanda"),
    ("sk", "Slovak"),
    ("sl", "Slovenian"),
    ("sq", "Albanian"),
    ("sr", "Serbian"),
    ("sv", "Swedish"),
    ("sw", "Swahili"),
    ("ta", "Tamil"),
    ("te", "Telugu"),
    ("th", "Thai"),
    ("tl", "Tagalog"),
    ("tr", "Turkish"),
    ("uk", "Ukrainian"),
    ("ur", "Urdu"),
    ("uz", "Uzbek"),
    ("vi", "Vietnamese"),
    ("zu", "Zulu"),
];

/// Regions by their two-letter codes (ISO 3166), for dictionaries' names.
const REGIONS: &[(&str, &str)] = &[
    ("AR", "Argentina"),
    ("AT", "Austria"),
    ("AU", "Australia"),
    ("BE", "Belgium"),
    ("BO", "Bolivia"),
    ("BR", "Brazil"),
    ("BZ", "Belize"),
    ("CA", "Canada"),
    ("CH", "Switzerland"),
    ("CL", "Chile"),
    ("CO", "Colombia"),
    ("CR", "Costa Rica"),
    ("CU", "Cuba"),
    ("DE", "Germany"),
    ("DK", "Denmark"),
    ("DO", "Dominican Republic"),
    ("EC", "Ecuador"),
    ("ES", "Spain"),
    ("FR", "France"),
    ("GB", "United Kingdom"),
    ("GH", "Ghana"),
    ("GT", "Guatemala"),
    ("HN", "Honduras"),
    ("IE", "Ireland"),
    ("IN", "India"),
    ("IT", "Italy"),
    ("JM", "Jamaica"),
    ("LI", "Liechtenstein"),
    ("LU", "Luxembourg"),
    ("MC", "Monaco"),
    ("MX", "Mexico"),
    ("NA", "Namibia"),
    ("NG", "Nigeria"),
    ("NI", "Nicaragua"),
    ("NL", "Netherlands"),
    ("NO", "Norway"),
    ("NZ", "New Zealand"),
    ("PA", "Panama"),
    ("PE", "Peru"),
    ("PH", "Philippines"),
    ("PR", "Puerto Rico"),
    ("PT", "Portugal"),
    ("PY", "Paraguay"),
    ("SE", "Sweden"),
    ("SG", "Singapore"),
    ("SV", "El Salvador"),
    ("TT", "Trinidad and Tobago"),
    ("US", "United States"),
    ("UY", "Uruguay"),
    ("VE", "Venezuela"),
    ("ZA", "South Africa"),
    ("ZW", "Zimbabwe"),
];

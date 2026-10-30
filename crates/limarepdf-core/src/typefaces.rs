//! Installed fonts offered for text LimarePDF writes (the Unicode text design, section 7):
//! the families whose fonts allow embedding and have the Latin letters, each with up to four
//! faces (regular, bold, italic and bold italic). A face is a face of a font file, or a
//! variable font set at a weight. LimarePDF embeds them cut down to the characters used
//! (`embed.rs`), so a document set in one opens anywhere. Families without the Latin
//! letters (emoji fonts, some fonts for one script) aren't offered, but characters a chosen
//! font lacks may fall back to them (`layout.rs`).
//!
//! The list is read the first time it is needed (every installed font is opened, which
//! takes a moment) and kept for the rest of the run. It only grows, so the number in a
//! [`Family::Installed`] keeps naming the same family.

use crate::fonts::{Family, FontChoice};
use crate::installed_fonts::InstalledFonts;
use fontdb::{Database, Source, Style};
use rayon::prelude::*;
use skrifa::prelude::{FontRef, LocationRef, MetadataProvider, Size};
use skrifa::raw::TableProvider;
use skrifa::string::StringId;
use skrifa::Tag;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Once, RwLock};

/// A family of installed fonts LimarePDF can embed.
#[derive(Debug)]
pub struct Typeface {
    /// Its own name, as the font gives it: "Source Serif 4", "C059". Font lists show its
    /// [`Typeface::label`].
    pub name: String,
    /// Regular, bold, italic and bold italic, at `bold + 2 * italic`, where the family has
    /// them.
    faces: [Option<FaceFile>; 4],
    /// The standard font family nearest it: sans, serif or monospaced.
    standard: Family,
    /// Whether font lists offer it: it has the Latin letters. Others are only fallbacks for
    /// characters a chosen font lacks.
    listed: bool,
}

/// Where a face of a [`Typeface`] is, to read or draw it.
#[derive(Clone, Debug, PartialEq)]
pub struct FaceFile {
    pub path: PathBuf,
    /// Its index in the file (a collection holds several).
    pub index: u32,
    /// The variation settings of a variable font set at a weight (axis and value: "wght",
    /// 700); empty for other fonts.
    pub coords: Vec<([u8; 4], f32)>,
    /// Its PostScript name, as PDFs name it ("SourceSerif4-Regular", "Cantarell-Bold").
    pub ps: String,
}

/// The typefaces fonts installed under code names stand in for, by the fonts' own names: the
/// URW base-35 fonts Linux installs for Ghostscript, and their older (gsfonts) names. They
/// are metric-compatible equivalents, not the typefaces themselves, so their own name is
/// shown beside the familiar one.
const FAMILIAR: [(&str, &str); 17] = [
    ("C059", "New Century Schoolbook"),
    ("P052", "Palatino"),
    ("Z003", "Zapf Chancery"),
    ("URW Bookman", "Bookman"),
    ("URW Gothic", "Avant Garde Gothic"),
    ("Nimbus Sans", "Helvetica"),
    ("Nimbus Sans Narrow", "Helvetica Narrow"),
    ("Nimbus Roman", "Times"),
    ("Nimbus Mono PS", "Courier"),
    ("Century Schoolbook L", "New Century Schoolbook"),
    ("URW Palladio L", "Palatino"),
    ("URW Chancery L", "Zapf Chancery"),
    ("URW Bookman L", "Bookman"),
    ("URW Gothic L", "Avant Garde Gothic"),
    ("Nimbus Sans L", "Helvetica"),
    ("Nimbus Roman No9 L", "Times"),
    ("Nimbus Mono L", "Courier"),
];

/// The typeface a font named `name` with a code name stands in for.
fn familiar(name: &str) -> Option<&'static str> {
    FAMILIAR
        .iter()
        .find(|(own, _)| *own == name)
        .map(|(_, familiar)| *familiar)
}

/// A family's name as font lists show it: the familiar typeface first for a font with a
/// code name, its own after ("New Century Schoolbook (C059)"); else its own.
fn label(name: &str) -> String {
    match familiar(name) {
        Some(familiar) => format!("{familiar} ({name})"),
        None => name.to_string(),
    }
}

/// Sorts families as font lists show them: by label, any case.
fn by_label<T>(list: &mut [T], typeface: impl Fn(&T) -> &Typeface) {
    list.sort_by_cached_key(|t| typeface(t).label().to_lowercase());
}

impl Typeface {
    /// Its name as font lists show it: "Source Serif 4", "New Century Schoolbook (C059)".
    /// A search matches either name in it.
    pub fn label(&self) -> String {
        label(&self.name)
    }

    /// The typeface it stands in for, for a font with a code name: "New Century
    /// Schoolbook" for C059.
    pub fn familiar(&self) -> Option<&'static str> {
        familiar(&self.name)
    }

    /// Whether the family has the face; a bold or italic it lacks can't be chosen.
    pub fn has(&self, bold: bool, italic: bool) -> bool {
        self.face(bold, italic).is_some()
    }

    /// The face, where the family has it.
    pub fn face(&self, bold: bool, italic: bool) -> Option<&FaceFile> {
        self.faces[bold as usize + 2 * italic as usize].as_ref()
    }

    /// The style the family has nearest `bold` and `italic`: those, else the one that keeps
    /// the italic, else any. For a family whose regular face is missing, say.
    pub fn nearest(&self, bold: bool, italic: bool) -> (bool, bool) {
        [
            (bold, italic),
            (!bold, italic),
            (bold, !italic),
            (!bold, !italic),
        ]
        .into_iter()
        .find(|&(b, i)| self.has(b, i))
        .unwrap_or((bold, italic))
    }
}

/// Every family found, at its number; only grows.
static LIST: RwLock<Vec<Arc<Typeface>>> = RwLock::new(Vec::new());
/// Whether the installed fonts have been read into [`LIST`].
static SYSTEM: Once = Once::new();

/// The installed families LimarePDF can embed and font lists offer, by label, each with its
/// number for [`Family::Installed`]. The first call reads every installed font, which takes
/// a moment; see [`typefaces_ready`].
pub fn typefaces() -> Vec<(u16, Arc<Typeface>)> {
    let mut out = every_typeface();
    out.retain(|(_, t)| t.listed);
    out
}

/// Every installed family LimarePDF can embed, by label: those [`typefaces`] offers, and
/// those without the Latin letters, which characters a chosen font lacks may fall back to.
pub(crate) fn every_typeface() -> Vec<(u16, Arc<Typeface>)> {
    SYSTEM.call_once(|| {
        append(scan(InstalledFonts::system().database()));
    });
    let list = LIST.read().unwrap_or_else(|e| e.into_inner());
    let mut out: Vec<(u16, Arc<Typeface>)> = list
        .iter()
        .enumerate()
        .map(|(i, t)| (i as u16, Arc::clone(t)))
        .collect();
    by_label(&mut out, |(_, t)| t);
    out
}

/// How many families have been found so far, offered or not, without reading the installed
/// fonts when they haven't been: it changes only when more are found or added
/// ([`add_font_files`]), so what is kept about them (`layout.rs`) is kept while it stays.
pub(crate) fn typeface_count() -> usize {
    LIST.read().unwrap_or_else(|e| e.into_inner()).len()
}

/// The number of the installed family named `name` (any case, spaces aside), offered or
/// not.
pub(crate) fn family_named(name: &str) -> Option<u16> {
    SYSTEM.call_once(|| {
        append(scan(InstalledFonts::system().database()));
    });
    let k = key(name);
    let list = LIST.read().unwrap_or_else(|e| e.into_inner());
    list.iter()
        .position(|t| key(&t.name) == k)
        .map(|n| n as u16)
}

/// Whether [`typefaces`] has read the installed fonts, so it answers at once.
pub fn typefaces_ready() -> bool {
    SYSTEM.is_completed()
}

/// The family of a [`Family::Installed`] number.
pub fn typeface(number: u16) -> Option<Arc<Typeface>> {
    let list = LIST.read().unwrap_or_else(|e| e.into_inner());
    list.get(number as usize).cloned()
}

/// Offers the fonts in these files besides the installed ones (the tests use it), and gives
/// the number of each family found in them. Files that can't be read are skipped.
pub fn add_font_files(paths: &[&Path]) -> Vec<u16> {
    let mut db = Database::new();
    for p in paths {
        let _ = db.load_font_file(p);
    }
    append(scan(&db))
}

fn append(found: Vec<Typeface>) -> Vec<u16> {
    let mut list = LIST.write().unwrap_or_else(|e| e.into_inner());
    let mut numbers = Vec::new();
    for t in found {
        if list.len() > u16::MAX as usize {
            break;
        }
        numbers.push(list.len() as u16);
        list.push(Arc::new(t));
    }
    numbers
}

impl FontChoice {
    /// The installed family, for one.
    pub fn typeface(self) -> Option<Arc<Typeface>> {
        match self.family {
            Family::Installed(n) => typeface(n),
            _ => None,
        }
    }

    /// The font a line set in the PDF font `name` is written in when it can't keep its own:
    /// the installed family the font is from, when LimarePDF can embed it
    /// ("ABCDEF+SourceSerif4-Regular" is Source Serif 4, regular), else the nearest standard
    /// font ([`FontChoice::from_name`]). Reads the installed fonts the first time.
    pub fn of_font(name: &str) -> FontChoice {
        Self::installed_font(name, &typefaces()).unwrap_or_else(|| FontChoice::from_name(name))
    }

    /// The face of `list` the PDF font `name` is, by its PostScript name.
    pub fn installed_font(name: &str, list: &[(u16, Arc<Typeface>)]) -> Option<FontChoice> {
        let bare = match name.split_once('+') {
            Some((tag, rest)) if tag.len() == 6 && tag.bytes().all(|b| b.is_ascii_uppercase()) => {
                rest
            }
            _ => name,
        };
        let k = key(bare);
        // A standard font stays one, though macOS installs fonts by the same names.
        if k.is_empty() || standard_name(&k) {
            return None;
        }
        list.iter().find_map(|(n, t)| {
            let at = t
                .faces
                .iter()
                .position(|f| f.as_ref().is_some_and(|f| key(&f.ps) == k))?;
            Some(FontChoice {
                family: Family::Installed(*n),
                bold: at & 1 != 0,
                italic: at & 2 != 0,
            })
        })
    }

    /// The standard font nearest this one: itself, or for an installed family the standard
    /// family of its kind, in the same style. What the font's text falls back to.
    pub fn standard(self) -> FontChoice {
        let family = match self.family {
            Family::Installed(n) => typeface(n).map_or(Family::Helvetica, |t| t.standard),
            f => f,
        };
        FontChoice { family, ..self }
    }

    /// Whether this is one of the 12 standard fonts.
    pub fn is_standard(self) -> bool {
        !matches!(self.family, Family::Installed(_))
    }

    /// The family's name, as the font list shows it: "Helvetica", "Source Serif 4",
    /// "Palatino (P052)".
    pub fn family_name(self) -> String {
        match self.family {
            Family::Helvetica => "Helvetica".into(),
            Family::Times => "Times".into(),
            Family::Courier => "Courier".into(),
            Family::Installed(n) => {
                typeface(n).map_or_else(|| "Unknown font".into(), |t| t.label())
            }
        }
    }
}

/// A name with only its lower-case letters and digits.
fn key(name: &str) -> String {
    name.chars()
        .filter(char::is_ascii_alphanumeric)
        .map(|c| c.to_ascii_lowercase())
        .collect()
}

/// Families offered as Helvetica, Times and Courier, not again: the ones the standard fonts
/// are embedded in (`embed.rs`), and those macOS installs by the standard fonts' names.
const AS_STANDARD: [&str; 6] = [
    "Liberation Sans",
    "Liberation Serif",
    "Liberation Mono",
    "Helvetica",
    "Times",
    "Courier",
];

/// Whether a name's [`key`] is one of the 12 standard fonts' ("timesbold").
fn standard_name(k: &str) -> bool {
    [Family::Helvetica, Family::Times, Family::Courier]
        .into_iter()
        .flat_map(|family| {
            [(false, false), (true, false), (false, true), (true, true)].map(|(bold, italic)| {
                FontChoice {
                    family,
                    bold,
                    italic,
                }
            })
        })
        .any(|f| key(f.base_font()) == k)
}

/// A named instance of a variable font: its settings and PostScript name.
type Instance = (Vec<([u8; 4], f32)>, String);

/// A face of a font file, as found.
struct Found {
    family: String,
    path: PathBuf,
    index: u32,
    ps: String,
    italic: bool,
    /// Weights it can be set at: one for a static font, the weight axis's range for a
    /// variable one.
    weights: (f32, f32),
    /// How far from normal width it is (0 for normal).
    off_width: u16,
    mono: bool,
    /// Whether it is a variable font (set at a weight when embedded).
    variable: bool,
    has_weight_axis: bool,
    /// An `ital` axis, so it can be set upright or italic.
    ital_axis: bool,
    /// A `slnt` axis's most slanted value, so it can be set upright or slanted.
    slant: Option<f32>,
    /// The named instances of a variable font: their settings and PostScript names.
    instances: Vec<Instance>,
    /// The font's own PostScript name prefix for instances (name 25), else its family's.
    ps_prefix: String,
    /// Whether it has the Latin letters, so font lists offer its family.
    latin: bool,
}

/// Whether a font's `OS/2 fsType` forbids embedding it as LimarePDF does: a restricted
/// licence (and no less restrictive bit, which older fonts may set beside it), no
/// subsetting, or bitmaps only.
fn forbidden(fs_type: u16) -> bool {
    fs_type & 0x000E == 0x0002 || fs_type & 0x0300 != 0
}

/// The families in `db` LimarePDF can embed, by name.
fn scan(db: &Database) -> Vec<Typeface> {
    let faces: Vec<_> = db.faces().collect();
    let found: Vec<Found> = faces
        .par_iter()
        .filter_map(|f| {
            let family = f.families.first()?.0.clone();
            if family.starts_with('.') || AS_STANDARD.contains(&family.as_str()) {
                return None;
            }
            let path = match &f.source {
                Source::File(p) | Source::SharedFile(p, _) => p.clone(),
                Source::Binary(_) => return None,
            };
            let base = Found {
                family: family.clone(),
                path,
                index: f.index,
                ps: f.post_script_name.clone(),
                italic: f.style != Style::Normal,
                weights: (f.weight.0 as f32, f.weight.0 as f32),
                off_width: (f.stretch.to_number() as i32 - 5).unsigned_abs() as u16,
                mono: f.monospaced,
                variable: false,
                has_weight_axis: false,
                ital_axis: false,
                slant: None,
                instances: Vec::new(),
                ps_prefix: family.chars().filter(|c| !c.is_whitespace()).collect(),
                latin: true,
            };
            db.with_face_data(f.id, |data, index| read(data, index, base))
                .flatten()
        })
        .collect();
    let mut by_family: BTreeMap<String, Vec<Found>> = BTreeMap::new();
    for f in found {
        by_family.entry(f.family.clone()).or_default().push(f);
    }
    let mut out: Vec<Typeface> = by_family
        .into_iter()
        .filter_map(|(name, mut faces)| {
            // The same order whatever order the files were read in.
            faces.sort_by(|a, b| (&a.path, a.index).cmp(&(&b.path, b.index)));
            let listed = faces.iter().any(|f| f.latin);
            let standard = match FontChoice::from_name(&name).family {
                _ if faces.iter().any(|f| f.mono) => Family::Courier,
                f => f,
            };
            let faces = [(false, false), (true, false), (false, true), (true, true)]
                .map(|(bold, italic)| pick(&faces, bold, italic));
            if faces.iter().all(Option::is_none) {
                return None;
            }
            Some(Typeface {
                name,
                faces,
                standard,
                listed,
            })
        })
        .collect();
    by_label(&mut out, |t| t);
    out
}

/// What LimarePDF needs to know of a face of a font file, when it can embed it.
fn read(data: &[u8], index: u32, mut f: Found) -> Option<Found> {
    let font = FontRef::from_index(data, index).ok()?;
    let has = |t: &[u8; 4]| font.table_data(Tag::new(t)).is_some();
    if !(has(b"glyf") || has(b"CFF ") || has(b"CFF2")) {
        return None;
    }
    if forbidden(font.os2().map_or(0, |o| o.fs_type())) {
        return None;
    }
    let charmap = font.charmap();
    f.latin = ('A'..='Z')
        .chain('a'..='z')
        .all(|c| charmap.map(c).is_some());
    f.mono |= font
        .metrics(Size::unscaled(), LocationRef::default())
        .is_monospace;
    let axes = font.axes();
    if axes.is_empty() {
        return Some(f);
    }
    f.variable = true;
    if let Some(w) = axes.get_by_tag(Tag::new(b"wght")) {
        f.weights = (w.min_value(), w.max_value());
        f.has_weight_axis = true;
    }
    f.ital_axis = axes
        .get_by_tag(Tag::new(b"ital"))
        .is_some_and(|a| a.max_value() >= 1.0);
    f.slant = axes
        .get_by_tag(Tag::new(b"slnt"))
        .map(|a| a.min_value())
        .filter(|&s| s < 0.0);
    let string = |id: StringId| {
        font.localized_strings(id)
            .english_or_first()
            .map(|s| s.to_string())
    };
    if let Some(prefix) = string(StringId::VARIATIONS_POSTSCRIPT_NAME_PREFIX) {
        f.ps_prefix = prefix;
    }
    let tags: Vec<[u8; 4]> = axes.iter().map(|a| a.tag().to_be_bytes()).collect();
    f.instances = font
        .named_instances()
        .iter()
        .filter_map(|i| {
            let coords = tags.iter().copied().zip(i.user_coords()).collect();
            let ps = match i.postscript_name_id().and_then(string) {
                Some(ps) => ps,
                None => format!("{}-{}", f.ps_prefix, string(i.subfamily_name_id())?),
            };
            Some((coords, ps))
        })
        .collect();
    Some(f)
}

/// The face of `faces` for a style, as a face file: the face nearest the weight (400, or
/// 700 for bold) in the slant asked for, normal width first, set at that weight when it is
/// variable.
fn pick(faces: &[Found], bold: bool, italic: bool) -> Option<FaceFile> {
    let target: f32 = if bold { 700.0 } else { 400.0 };
    let mut best: Option<((u16, u32, bool), FaceFile)> = None;
    for f in faces {
        // Each slant the face can be set in, with the settings that give it, and whether
        // that is a slant axis rather than the face's own italic.
        let mut slants = vec![(f.italic, Vec::new(), false)];
        if !f.italic && f.ital_axis {
            slants = vec![
                (false, vec![(*b"ital", 0.0)], false),
                (true, vec![(*b"ital", 1.0)], false),
            ];
        } else if let (false, Some(s)) = (f.italic, f.slant) {
            slants = vec![
                (false, vec![(*b"slnt", 0.0)], false),
                (true, vec![(*b"slnt", s)], true),
            ];
        }
        for (slanted, extra, by_axis) in slants {
            if slanted != italic {
                continue;
            }
            let (low, high) = f.weights;
            if bold && high < 600.0 || !bold && low >= 600.0 {
                continue;
            }
            let weight = target.clamp(low, high);
            let rank = (f.off_width, (weight - target).abs() as u32, by_axis);
            if best.as_ref().is_some_and(|(r, _)| *r <= rank) {
                continue;
            }
            let mut coords = Vec::new();
            if f.variable {
                if f.has_weight_axis {
                    coords.push((*b"wght", weight));
                }
                coords.extend(extra);
            }
            let ps = if f.variable {
                instance_name(f, &coords)
            } else {
                f.ps.clone()
            };
            let file = FaceFile {
                path: f.path.clone(),
                index: f.index,
                coords,
                ps: ps
                    .chars()
                    .filter(|c| c.is_ascii_alphanumeric() || *c == '-')
                    .collect(),
            };
            best = Some((rank, file));
        }
    }
    best.map(|(_, f)| f)
}

/// The PostScript name of a variable font set at `coords`: its named instance's, else its
/// prefix with the settings ("Cantarell-wght650").
fn instance_name(f: &Found, coords: &[([u8; 4], f32)]) -> String {
    let at = |list: &[([u8; 4], f32)], tag: [u8; 4], default: f32| {
        list.iter()
            .find(|(t, _)| *t == tag)
            .map_or(default, |(_, v)| *v)
    };
    let same = |list: &[([u8; 4], f32)]| {
        coords
            .iter()
            .all(|&(t, v)| (at(list, t, f32::NAN) - v).abs() < 0.5)
            && list
                .iter()
                .all(|&(t, v)| (at(coords, t, v) - v).abs() < 0.5)
    };
    if let Some((_, ps)) = f.instances.iter().find(|(list, _)| same(list)) {
        return ps.clone();
    }
    let mut name = f.ps_prefix.clone();
    for (t, v) in coords {
        name.push('-');
        name.extend(t.iter().map(|&b| b as char));
        name.push_str(&format!("{}", v.round() as i64));
    }
    name
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(name: &str) -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join(name)
    }

    /// The test fonts' families, by name.
    fn fixtures() -> Vec<(u16, Arc<Typeface>)> {
        let files = [
            fixture("tests/data/fonts/SourceSerif4-Regular.otf"),
            fixture("tests/data/fonts/Cantarell-VF.otf"),
            fixture("assets/fonts/Caveat.ttf"),
            fixture("assets/fonts/LiberationSans-Regular.ttf"),
        ];
        let paths: Vec<&Path> = files.iter().map(PathBuf::as_path).collect();
        let numbers = add_font_files(&paths);
        numbers
            .into_iter()
            .map(|n| (n, typeface(n).unwrap()))
            .collect()
    }

    fn named(list: &[(u16, Arc<Typeface>)], name: &str) -> Arc<Typeface> {
        list.iter()
            .find(|(_, t)| t.name == name)
            .map(|(_, t)| Arc::clone(t))
            .unwrap_or_else(|| panic!("{name} in the list"))
    }

    #[test]
    fn families_and_their_faces() {
        let list = fixtures();
        let names: Vec<&str> = list.iter().map(|(_, t)| t.name.as_str()).collect();
        // Liberation is offered as Helvetica, not again.
        assert_eq!(names, ["Cantarell", "Caveat", "Source Serif 4"]);

        // A static font: its one face, nothing set.
        let serif = named(&list, "Source Serif 4");
        let regular = serif.face(false, false).unwrap();
        assert_eq!(regular.ps, "SourceSerif4-Regular");
        assert!(regular.coords.is_empty());
        assert!(!serif.has(true, false) && !serif.has(false, true) && !serif.has(true, true));
        assert_eq!(serif.standard, Family::Times);

        // A variable font: bold from its weight axis, named as its instance is.
        let cantarell = named(&list, "Cantarell");
        let regular = cantarell.face(false, false).unwrap();
        assert_eq!(regular.coords, [(*b"wght", 400.0)]);
        let bold = cantarell.face(true, false).unwrap();
        assert_eq!(bold.coords, [(*b"wght", 700.0)]);
        assert_eq!(bold.path, regular.path);
        assert_ne!(bold.ps, regular.ps);
        assert!(bold.ps.starts_with("Cantarell"), "{}", bold.ps);
        assert!(!cantarell.has(false, true), "no italic");
        assert_eq!(cantarell.standard, Family::Helvetica);

        // Caveat's weights are 400 to 700.
        let caveat = named(&list, "Caveat");
        assert_eq!(
            caveat.face(true, false).unwrap().coords,
            [(*b"wght", 700.0)]
        );
        assert_eq!(caveat.nearest(true, true), (true, false));
    }

    #[test]
    fn code_named_fonts_show_their_familiar_family_first() {
        let family = |name: &str| Typeface {
            name: name.into(),
            faces: Default::default(),
            standard: Family::Times,
            listed: true,
        };
        assert_eq!(family("C059").label(), "New Century Schoolbook (C059)");
        assert_eq!(family("Nimbus Mono PS").label(), "Courier (Nimbus Mono PS)");
        assert_eq!(
            family("URW Palladio L").label(),
            "Palatino (URW Palladio L)"
        );
        assert_eq!(family("Source Serif 4").label(), "Source Serif 4");
        assert_eq!(family("Z003").familiar(), Some("Zapf Chancery"));
        assert_eq!(family("Source Serif 4").familiar(), None);
        // Not a code name: only whole names are matched.
        assert_eq!(family("C0590").label(), "C0590");

        // Listed by the familiar name; the own name stays the font's.
        let mut list: Vec<Typeface> = ["Z003", "Cantarell", "P052", "URW Gothic", "Nimbus Sans"]
            .map(family)
            .into();
        by_label(&mut list, |t| t);
        let labels: Vec<String> = list.iter().map(Typeface::label).collect();
        assert_eq!(
            labels,
            [
                "Avant Garde Gothic (URW Gothic)",
                "Cantarell",
                "Helvetica (Nimbus Sans)",
                "Palatino (P052)",
                "Zapf Chancery (Z003)",
            ]
        );
        assert_eq!(list[3].name, "P052");
    }

    #[test]
    fn fonts_named_in_pdfs_are_found() {
        let list = fixtures();
        let find = |name: &str| {
            FontChoice::installed_font(name, &list).map(|f| (f.family_name(), f.bold, f.italic))
        };
        let serif = Some(("Source Serif 4".to_string(), false, false));
        assert_eq!(find("SourceSerif4-Regular"), serif);
        assert_eq!(find("ABCDEF+SourceSerif4-Regular"), serif);
        let bold = named(&list, "Cantarell")
            .face(true, false)
            .unwrap()
            .ps
            .clone();
        assert_eq!(
            find(&format!("QWERTY+{bold}")),
            Some(("Cantarell".to_string(), true, false))
        );
        assert_eq!(find("Helvetica"), None);
        assert_eq!(find("LiberationSans"), None);
        assert!(standard_name(&key("Times-Bold")) && standard_name(&key("Helvetica")));
        assert!(!standard_name(&key("SourceSerif4-Regular")));
        assert_eq!(find(""), None);
    }

    #[test]
    fn standard_fonts_and_stand_ins() {
        let list = fixtures();
        let serif = FontChoice::installed_font("SourceSerif4-Regular", &list).unwrap();
        assert!(!serif.is_standard());
        assert_eq!(serif.standard().family, Family::Times);
        assert_eq!(serif.standard().base_font(), "Times-Roman");
        assert!(FontChoice::default().is_standard());
        assert_eq!(FontChoice::default().family_name(), "Helvetica");
    }

    #[test]
    fn embedding_permissions() {
        // Installable, editable and preview-and-print fonts may be embedded.
        for fs_type in [0x0000, 0x0004, 0x0008, 0x000C] {
            assert!(!forbidden(fs_type), "{fs_type:#06x}");
        }
        // Restricted licence (alone), no subsetting, bitmaps only.
        for fs_type in [0x0002, 0x0102, 0x0100, 0x0200, 0x0204] {
            assert!(forbidden(fs_type), "{fs_type:#06x}");
        }
        // Old fonts that set several bits get the least restrictive.
        assert!(!forbidden(0x0006));
    }
}

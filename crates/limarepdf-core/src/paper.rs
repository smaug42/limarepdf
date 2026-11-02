//! Paper sizes: those a printer takes, as the printing service reports them, named as people
//! name them ("Letter (8.5 × 11 in)", "A4 (210 × 297 mm)"). On Linux and macOS they come
//! from CUPS over IPP (`ipp.rs`), with each size's margins, else from `lpoptions`'s names
//! alone; on Windows from the printer's driver (`print_win.rs`).
#![cfg_attr(windows, allow(dead_code))]

use crate::ipp::{values, Attribute, Value};

/// A paper size a printer takes.
#[derive(Clone, Debug, PartialEq)]
pub struct Paper {
    /// How it is named to the printer: the media name CUPS is sent (`na_letter_8.5x11in`,
    /// `A4`), or on Windows the driver's number for it.
    pub id: String,
    /// Its name for people, with its size: "Letter (8.5 × 11 in)".
    pub label: String,
    /// Width and height in points, as the printer takes the sheet.
    pub size: (f32, f32),
    /// How near each edge the printer can't print: top, right, bottom and left, in points;
    /// `None` when the printer doesn't say.
    pub margins: Option<[f32; 4]>,
}

/// The paper sizes a printer takes, and which is its default.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PaperList {
    pub papers: Vec<Paper>,
    /// The printer's default, when it is one of [`PaperList::papers`].
    pub default: Option<usize>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Unit {
    In,
    Mm,
}

impl Unit {
    fn points(self, v: f32) -> f32 {
        match self {
            Unit::In => v * 72.0,
            Unit::Mm => v * 72.0 / 25.4,
        }
    }
    /// `v` in hundredths of a millimetre, as IPP gives sizes.
    fn hundredths(self, v: f32) -> i32 {
        match self {
            Unit::In => (v * 2540.0).round() as i32,
            Unit::Mm => (v * 100.0).round() as i32,
        }
    }
    fn name(self) -> &'static str {
        match self {
            Unit::In => "in",
            Unit::Mm => "mm",
        }
    }
}

/// Points from hundredths of a millimetre.
fn points(hundredths: i32) -> f32 {
    hundredths as f32 / 2540.0 * 72.0
}

/// `v` written short: "8.5", "11", "4.125".
fn number(v: f32) -> String {
    let s = format!("{v:.3}");
    s.trim_end_matches('0').trim_end_matches('.').to_string()
}

/// A size in points as people write it: in inches when both sides are whole eighths of an
/// inch, else in millimetres. "8.5 × 11 in", "210 × 297 mm".
pub(crate) fn dims(w: f32, h: f32) -> String {
    // An eighth of an inch is 9 points; within 0.3 mm (0.85 points) of one counts.
    let eighths = |v: f32| {
        let e = v / 9.0;
        ((e - e.round()).abs() * 9.0 < 0.85 && e.round() > 0.0).then(|| e.round() / 8.0)
    };
    let mm = |v: f32| number((v / 72.0 * 254.0).round() / 10.0);
    match (eighths(w), eighths(h)) {
        (Some(a), Some(b)) => format!("{} × {} in", number(a), number(b)),
        _ => format!("{} × {} mm", mm(w), mm(h)),
    }
}

/// A PWG self-describing media name, as IPP names paper: its class, its own name, and its
/// width and height as written, with their unit ("na_letter_8.5x11in").
struct Pwg<'a> {
    class: &'a str,
    name: &'a str,
    dims: (&'a str, &'a str),
    size: (f32, f32),
    unit: Unit,
}

fn pwg(s: &str) -> Option<Pwg<'_>> {
    let (class, rest) = s.split_once('_')?;
    let (name, dims) = rest.rsplit_once('_')?;
    let (dims, unit) = match dims.strip_suffix("in") {
        Some(d) => (d, Unit::In),
        None => (dims.strip_suffix("mm")?, Unit::Mm),
    };
    let (w, h) = dims.split_once('x')?;
    let size = (w.parse::<f32>().ok()?, h.parse::<f32>().ok()?);
    (size.0 > 0.0 && size.1 > 0.0).then_some(Pwg {
        class,
        name,
        dims: (w, h),
        size,
        unit,
    })
}

/// The names people give the PWG sizes that aren't simply their own names.
const PWG_NAMES: &[(&str, &str)] = &[
    ("na_letter", "Letter"),
    ("na_legal", "Legal"),
    ("na_executive", "Executive"),
    ("na_ledger", "Tabloid"),
    ("na_invoice", "Statement"),
    ("na_foolscap", "Foolscap"),
    ("na_quarto", "Quarto"),
    ("na_oficio", "Oficio"),
    ("na_govt-letter", "Government Letter"),
    ("na_govt-legal", "Government Legal"),
    ("na_super-a", "Super A"),
    ("na_super-b", "Super B"),
    ("na_number-9", "Envelope #9"),
    ("na_number-10", "Envelope #10"),
    ("na_number-11", "Envelope #11"),
    ("na_number-12", "Envelope #12"),
    ("na_number-14", "Envelope #14"),
    ("na_monarch", "Envelope Monarch"),
    ("na_personal", "Envelope Personal"),
    ("na_a2", "Envelope A2"),
    ("na_index-3x5", "Index card"),
    ("na_index-4x6", "Index card"),
    ("na_index-5x8", "Index card"),
    ("na_5x7", "Photo"),
    ("na_arch-a", "Arch A"),
    ("na_arch-b", "Arch B"),
    ("na_arch-c", "Arch C"),
    ("na_arch-d", "Arch D"),
    ("na_arch-e", "Arch E"),
    ("iso_dl", "Envelope DL"),
    ("iso_c6c5", "Envelope C6/C5"),
    ("jpn_hagaki", "Postcard"),
    ("jpn_oufuku", "Double postcard"),
    ("jpn_chou3", "Envelope Chou 3"),
    ("jpn_chou4", "Envelope Chou 4"),
    ("jpn_kaku2", "Envelope Kaku 2"),
    ("om_16k", "16K"),
    ("roc_16k", "16K"),
    ("prc_16k", "16K"),
    ("roc_8k", "8K"),
    ("prc_32k", "32K"),
    ("om_folio", "Folio"),
    ("om_small-photo", "Small photo"),
    ("oe_photo-l", "Photo L"),
];

/// A PWG size's name for people, with its size as its name writes it.
fn pwg_label(p: &Pwg) -> String {
    let key = format!("{}_{}", p.class, p.name);
    let series = |s: &str| {
        let digits = p.name.strip_prefix(s)?;
        (!digits.is_empty() && digits.bytes().all(|b| b.is_ascii_digit())).then_some(digits)
    };
    let base = match PWG_NAMES.iter().find(|(k, _)| *k == key) {
        Some((_, n)) => n.to_string(),
        None if p.class == "custom" => "Custom".into(),
        None if p.class == "iso" && series("a").is_some() => p.name.to_uppercase(),
        None if p.class == "iso" && series("b").is_some() => p.name.to_uppercase(),
        None if p.class == "iso" && series("c").is_some() => {
            format!("Envelope {}", p.name.to_uppercase())
        }
        None if p.class == "jis" && series("b").is_some() => {
            format!("JIS {}", p.name.to_uppercase())
        }
        None => {
            let words = p.name.replace('-', " ");
            let mut c = words.chars();
            match c.next() {
                Some(f) => f.to_uppercase().chain(c).collect(),
                None => words,
            }
        }
    };
    format!("{base} ({} × {} {})", p.dims.0, p.dims.1, p.unit.name())
}

/// A size `media-col-database` lists, in hundredths of a millimetre, with its margins
/// (top, right, bottom, left) when given.
struct Entry {
    w: i32,
    h: i32,
    margins: Option<[i32; 4]>,
}

fn entry(col: &[Attribute]) -> Option<Entry> {
    let size = values(col, "media-size").first()?.members()?;
    let dim = |n: &str| -> Option<i32> { values(size, n).first()?.int() };
    let margin = |n: &str| -> Option<i32> { values(col, n).first()?.int() };
    let margins = (|| {
        Some([
            margin("media-top-margin")?,
            margin("media-right-margin")?,
            margin("media-bottom-margin")?,
            margin("media-left-margin")?,
        ])
    })();
    Some(Entry {
        w: dim("x-dimension")?,
        h: dim("y-dimension")?,
        margins,
    })
}

/// What IPP asks the printing service, for [`from_ipp`].
pub(crate) const WANTED: [&str; 3] = ["media-supported", "media-default", "media-col-database"];

/// The papers a printer's IPP attributes ([`WANTED`]) list, each with the size and margins
/// of the nearest size in `media-col-database` within a millimetre (the bordered one where a
/// size is also borderless). Custom sizes are left out, but for the default.
pub(crate) fn from_ipp(attributes: &[Attribute]) -> PaperList {
    let default = values(attributes, "media-default")
        .first()
        .and_then(Value::text);
    let database: Vec<Entry> = values(attributes, "media-col-database")
        .iter()
        .filter_map(Value::members)
        .filter_map(entry)
        .collect();
    let mut found = Vec::new();
    for name in values(attributes, "media-supported")
        .iter()
        .filter_map(Value::text)
    {
        let Some(p) = pwg(name) else { continue };
        if p.class == "custom" && Some(name) != default {
            continue;
        }
        let (w, h) = (p.unit.hundredths(p.size.0), p.unit.hundredths(p.size.1));
        let near = database
            .iter()
            .filter(|e| (e.w - w).abs() <= 100 && (e.h - h).abs() <= 100)
            .min_by_key(|e| {
                let borderless = e.margins == Some([0; 4]);
                let off = (e.w - w).abs() + (e.h - h).abs();
                (off, borderless, e.margins.is_none())
            });
        let (size, margins) = match near {
            Some(e) => ((points(e.w), points(e.h)), e.margins.map(|m| m.map(points))),
            None => ((p.unit.points(p.size.0), p.unit.points(p.size.1)), None),
        };
        found.push(Paper {
            id: name.to_string(),
            label: pwg_label(&p),
            size,
            margins,
        });
    }
    listed(found, default)
}

/// The standard PPD paper names `lpoptions` gives, with their names for people and their
/// sizes.
const PPD_NAMES: &[(&str, &str, f32, f32, Unit)] = &[
    ("Letter", "Letter", 8.5, 11.0, Unit::In),
    ("Legal", "Legal", 8.5, 14.0, Unit::In),
    ("Executive", "Executive", 7.25, 10.5, Unit::In),
    ("Tabloid", "Tabloid", 11.0, 17.0, Unit::In),
    ("Ledger", "Ledger", 17.0, 11.0, Unit::In),
    ("Statement", "Statement", 5.5, 8.5, Unit::In),
    ("Folio", "Folio", 8.5, 13.0, Unit::In),
    ("FLSA", "Foolscap", 8.5, 13.0, Unit::In),
    ("A0", "A0", 841.0, 1189.0, Unit::Mm),
    ("A1", "A1", 594.0, 841.0, Unit::Mm),
    ("A2", "A2", 420.0, 594.0, Unit::Mm),
    ("A3", "A3", 297.0, 420.0, Unit::Mm),
    ("A4", "A4", 210.0, 297.0, Unit::Mm),
    ("A5", "A5", 148.0, 210.0, Unit::Mm),
    ("A6", "A6", 105.0, 148.0, Unit::Mm),
    ("B4", "JIS B4", 257.0, 364.0, Unit::Mm),
    ("B5", "JIS B5", 182.0, 257.0, Unit::Mm),
    ("JB5", "JIS B5", 182.0, 257.0, Unit::Mm),
    ("ISOB4", "B4", 250.0, 353.0, Unit::Mm),
    ("ISOB5", "B5", 176.0, 250.0, Unit::Mm),
    ("Env9", "Envelope #9", 3.875, 8.875, Unit::In),
    ("Env10", "Envelope #10", 4.125, 9.5, Unit::In),
    ("EnvMonarch", "Envelope Monarch", 3.875, 7.5, Unit::In),
    ("EnvDL", "Envelope DL", 110.0, 220.0, Unit::Mm),
    ("EnvC4", "Envelope C4", 229.0, 324.0, Unit::Mm),
    ("EnvC5", "Envelope C5", 162.0, 229.0, Unit::Mm),
    ("EnvC6", "Envelope C6", 114.0, 162.0, Unit::Mm),
    ("EnvB5", "Envelope B5", 176.0, 250.0, Unit::Mm),
    ("Postcard", "Postcard", 100.0, 148.0, Unit::Mm),
    ("DoublePostcard", "Double postcard", 148.0, 200.0, Unit::Mm),
];

/// A PPD paper name's name for people and size in points: a standard one, or "w288h432"
/// (a size in points).
fn ppd_paper(name: &str) -> Option<(String, (f32, f32))> {
    let standard = PPD_NAMES.iter().find(|e| e.0.eq_ignore_ascii_case(name));
    if let Some(&(_, label, w, h, unit)) = standard {
        let label = format!("{label} ({} × {} {})", number(w), number(h), unit.name());
        return Some((label, (unit.points(w), unit.points(h))));
    }
    let (w, h) = name.strip_prefix('w')?.split_once('h')?;
    let (w, h) = (w.parse::<f32>().ok()?, h.parse::<f32>().ok()?);
    (w > 0.0 && h > 0.0).then(|| (dims(w, h), (w, h)))
}

/// The papers on the `PageSize` line of `lpoptions -l` ("PageSize/Media Size: A5 *Letter
/// A4 Letter.Duplex Custom.WIDTHxHEIGHT"), margins unknown. Variants of a size
/// (`Letter.Duplex`) and names LimarePDF doesn't know the size of are left out, but for
/// the default variant.
pub(crate) fn from_lpoptions(text: &str) -> PaperList {
    let Some((_, choices)) = text
        .lines()
        .find(|l| l.starts_with("PageSize/"))
        .and_then(|l| l.split_once(':'))
    else {
        return PaperList::default();
    };
    let mut default = None;
    let mut found = Vec::new();
    for choice in choices.split_whitespace() {
        let (name, is_default) = match choice.strip_prefix('*') {
            Some(n) => (n, true),
            None => (choice, false),
        };
        if is_default {
            default = Some(name);
        }
        let base = name.split('.').next().unwrap_or(name);
        if base != name && !is_default {
            continue;
        }
        if let Some((label, size)) = ppd_paper(base) {
            found.push(Paper {
                id: name.to_string(),
                label,
                size,
                margins: None,
            });
        }
    }
    listed(found, default)
}

/// `found` as a list with `default` (its id) marked: each label once, the default's kept
/// over others of its label.
pub(crate) fn listed(found: Vec<Paper>, default: Option<&str>) -> PaperList {
    let default_label = found
        .iter()
        .find(|p| Some(p.id.as_str()) == default)
        .map(|p| p.label.clone());
    let mut papers: Vec<Paper> = Vec::new();
    for p in found {
        let is_default = Some(p.id.as_str()) == default;
        let beside_default = !is_default && default_label.as_deref() == Some(p.label.as_str());
        if beside_default || papers.iter().any(|q| q.label == p.label) {
            continue;
        }
        papers.push(p);
    }
    let default = papers.iter().position(|p| Some(p.id.as_str()) == default);
    PaperList { papers, default }
}

/// The papers `printer` (a CUPS destination, perhaps `queue/instance`) takes: asked over IPP,
/// each with its margins, else read from `lpoptions`, margins unknown. A printer CUPS found on
/// the network gets its queue when one of CUPS's commands first uses it, so one IPP doesn't
/// know yet is asked again after `lpoptions` has run.
#[cfg(not(windows))]
pub(crate) fn cups_papers(printer: &str) -> crate::Result<PaperList> {
    use crate::print::{cups, within, ASK_WITHIN};
    let queue = printer.split('/').next().unwrap_or(printer).to_string();
    // The thread's limit a little over the request's own, so that its fuller message is the
    // one said.
    let ask = || {
        let queue = queue.clone();
        let limit = ASK_WITHIN + std::time::Duration::from_secs(2);
        within("the printer's paper sizes", limit, move || {
            let server = crate::ipp::server();
            crate::ipp::printer_attributes(&server, &queue, &WANTED, ASK_WITHIN)
        })
    };
    let asked = ask();
    if let Ok(Some(attrs)) = &asked {
        let list = from_ipp(attrs);
        if !list.papers.is_empty() {
            return Ok(list);
        }
    }
    let names = cups("lpoptions", &["-p", printer, "-l"]);
    if matches!(asked, Ok(None)) && names.is_ok() {
        if let Ok(Some(attrs)) = ask() {
            let list = from_ipp(&attrs);
            if !list.papers.is_empty() {
                return Ok(list);
            }
        }
    }
    let list = from_lpoptions(&names?);
    if list.papers.is_empty() {
        anyhow::bail!("the printing service doesn't list this printer's paper sizes");
    }
    Ok(list)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ipp::read_answer;
    use crate::ipp::tests::{answer, media_col, V};

    fn close(a: (f32, f32), b: (f32, f32)) -> bool {
        (a.0 - b.0).abs() < 0.05 && (a.1 - b.1).abs() < 0.05
    }

    #[test]
    fn pwg_names_read_as_people_name_them() {
        let label = |s: &str| pwg_label(&pwg(s).unwrap());
        assert_eq!(label("na_letter_8.5x11in"), "Letter (8.5 × 11 in)");
        assert_eq!(label("iso_a4_210x297mm"), "A4 (210 × 297 mm)");
        assert_eq!(label("iso_b5_176x250mm"), "B5 (176 × 250 mm)");
        assert_eq!(label("iso_c5_162x229mm"), "Envelope C5 (162 × 229 mm)");
        assert_eq!(label("jis_b5_182x257mm"), "JIS B5 (182 × 257 mm)");
        assert_eq!(
            label("na_number-10_4.125x9.5in"),
            "Envelope #10 (4.125 × 9.5 in)"
        );
        assert_eq!(label("om_16k_195x270mm"), "16K (195 × 270 mm)");
        assert_eq!(
            label("custom_210.06x292.69mm_210.06x292.69mm"),
            "Custom (210.06 × 292.69 mm)"
        );
        assert_eq!(label("oe_square-photo_5x5in"), "Square photo (5 × 5 in)");
        let a4 = pwg("iso_a4_210x297mm").unwrap();
        let a4 = (a4.unit.points(a4.size.0), a4.unit.points(a4.size.1));
        assert!(close(a4, (595.28, 841.89)), "{a4:?}");
        let bad = [
            "letter",
            "na_letter",
            "na_letter_8.5x11",
            "na_x_0x11in",
            "na_x_axbin",
        ];
        for bad in bad {
            assert!(pwg(bad).is_none(), "{bad}");
        }
    }

    #[test]
    fn sizes_are_written_in_inches_or_millimetres() {
        assert_eq!(dims(612.0, 792.0), "8.5 × 11 in");
        assert_eq!(dims(297.0, 684.0), "4.125 × 9.5 in");
        assert_eq!(dims(288.0, 432.0), "4 × 6 in");
        assert_eq!(dims(595.28, 841.89), "210 × 297 mm");
        assert_eq!(dims(311.81, 623.62), "110 × 220 mm");
        // A6: one side is near an eighth of an inch, the other not.
        assert_eq!(dims(297.64, 419.53), "105 × 148 mm");
    }

    /// The printer's sizes from its IPP attributes as CUPS gives them for a PPD printer:
    /// each named size takes the database's own size and margins; the duplex variants CUPS
    /// lists as custom sizes, and the custom range, are left out.
    #[test]
    fn ipp_papers_take_their_sizes_and_margins_from_the_database() {
        let bytes = answer(
            0,
            &[
                (
                    "media-supported",
                    vec![
                        V::Kw("iso_a6_105x148mm"),
                        V::Kw("na_letter_8.5x11in"),
                        V::Kw("iso_a4_210x297mm"),
                        V::Kw("custom_210.06x292.69mm_210.06x292.69mm"),
                        V::Kw("na_legal_8.5x14in"),
                        V::Kw("custom_min_3x5in"),
                    ],
                ),
                ("media-default", vec![V::Kw("na_letter_8.5x11in")]),
                (
                    "media-col-database",
                    vec![
                        media_col(10513, 14817, [423, 423, 423, 423]),
                        // Borderless first, then the bordered size: the bordered is taken.
                        media_col(21590, 27940, [0, 0, 0, 0]),
                        media_col(21590, 27940, [423, 635, 547, 635]),
                        media_col(21006, 29693, [423, 423, 423, 423]),
                        media_col(21006, 29269, [423, 423, 423, 423]),
                    ],
                ),
            ],
        );
        let a = read_answer(&bytes).unwrap();
        let list = from_ipp(&a.printer);
        let labels: Vec<&str> = list.papers.iter().map(|p| p.label.as_str()).collect();
        assert_eq!(
            labels,
            [
                "A6 (105 × 148 mm)",
                "Letter (8.5 × 11 in)",
                "A4 (210 × 297 mm)",
                "Legal (8.5 × 14 in)"
            ]
        );
        assert_eq!(list.default, Some(1));
        let letter = &list.papers[1];
        assert_eq!(letter.id, "na_letter_8.5x11in");
        assert!(close(letter.size, (612.0, 792.0)));
        let m = letter.margins.unwrap();
        // 4.23 mm and 6.35 mm: 12 and 18 points.
        let near = |a: f32, b: f32| (a - b).abs() < 0.01;
        assert!(near(m[0], 11.99) && near(m[1], 18.0), "{m:?}");
        // A4 takes the database's 210.06 × 296.93 mm, not the custom 292.69.
        let a4 = &list.papers[2];
        assert!(close(a4.size, (595.44, 841.69)), "{:?}", a4.size);
        // Legal has no size in the database: its own size, margins unknown.
        let legal = &list.papers[3];
        assert!(close(legal.size, (612.0, 1008.0)) && legal.margins.is_none());
    }

    #[test]
    fn a_custom_default_is_kept_and_unknown_names_left_out() {
        let bytes = answer(
            0,
            &[
                (
                    "media-supported",
                    vec![
                        V::Kw("na_letter_8.5x11in"),
                        V::Kw("custom_4x6in_4x6in"),
                        V::Kw("photo-ish"),
                    ],
                ),
                ("media-default", vec![V::Kw("custom_4x6in_4x6in")]),
            ],
        );
        let list = from_ipp(&read_answer(&bytes).unwrap().printer);
        assert_eq!(list.papers.len(), 2);
        assert_eq!(list.papers[1].label, "Custom (4 × 6 in)");
        assert_eq!(list.default, Some(1));
        assert!(list.papers.iter().all(|p| p.margins.is_none()));
        // No default listed, or one not supported.
        let none = answer(0, &[("media-supported", vec![V::Kw("na_letter_8.5x11in")])]);
        assert_eq!(from_ipp(&read_answer(&none).unwrap().printer).default, None);
    }

    #[test]
    fn lpoptions_names_are_read_with_their_sizes() {
        let text = "PageSize/Media Size: A6 Postcard 16k195x270 *Letter Letter.Duplex A4 \
                    A4.Duplex Legal EnvDL Env10 w288h432 Custom.WIDTHxHEIGHT\n\
                    Duplex/Double-Sided Printing: DuplexNoTumble *None\n";
        let list = from_lpoptions(text);
        let labels: Vec<&str> = list.papers.iter().map(|p| p.label.as_str()).collect();
        assert_eq!(
            labels,
            [
                "A6 (105 × 148 mm)",
                "Postcard (100 × 148 mm)",
                "Letter (8.5 × 11 in)",
                "A4 (210 × 297 mm)",
                "Legal (8.5 × 14 in)",
                "Envelope DL (110 × 220 mm)",
                "Envelope #10 (4.125 × 9.5 in)",
                "4 × 6 in",
            ]
        );
        assert_eq!(list.default, Some(2));
        assert_eq!(list.papers[2].id, "Letter");
        assert!(close(list.papers[3].size, (595.28, 841.89)));
        assert!(list.papers.iter().all(|p| p.margins.is_none()));
        // A default variant is kept as the default, its plain size left out.
        let list = from_lpoptions("PageSize/Media Size: Letter *Letter.Duplex A4\n");
        assert_eq!(list.papers.len(), 2);
        assert_eq!(list.papers[0].id, "Letter.Duplex");
        assert_eq!(list.default, Some(0));
        // No PageSize line, or a default LimarePDF doesn't know.
        assert_eq!(from_lpoptions("Duplex/Two: *None\n"), PaperList::default());
        assert_eq!(from_lpoptions("PageSize/Size: A4 *Odd\n").default, None);
    }
}

//! The answer a copy of the program gives with a Windows printer's paper sizes, and its check
//! (C-01). The copy reads them from the printer's driver (`print_win.rs`), so that a driver
//! writing past what it was asked for can end only the copy; the program asking reads at most
//! [`MAX_ANSWER`] of what it writes, and uses it only when all of it is as written here:
//!
//! ```text
//! limarepdf-paper-sizes 1
//! default 9
//! 1 2159 2794 12.5,12.5,12.5,12.5 Letter
//! 9 2100 2970 - A4
//! end 2
//! ```
//!
//! A version line; the default's number, or `-` for none; one line per paper with its number,
//! its width and height in tenths of a millimetre as the driver gave them, how near each edge
//! it can't be printed (top, right, bottom and left, in points) or `-`, and its name; then
//! `end` and how many papers there were. The fields are separated by tabs (spaces above), and
//! every line ends with a newline.

use crate::paper::{dims, listed, Paper, PaperList};
use crate::Result;
use anyhow::{anyhow, bail, Context};
use std::fmt::Write as _;
use std::str::FromStr;

/// The answer's first line.
const VERSION: &str = "limarepdf-paper-sizes 1";

/// The most papers an answer may list. No driver comes near.
pub(crate) const MAX_PAPERS: usize = 4096;

/// The most of an answer read, in bytes.
pub(crate) const MAX_ANSWER: usize = 1 << 20;

/// The most UTF-16 units of a paper's name the driver writes, and so the most characters of
/// it in an answer.
pub(crate) const MAX_NAME: usize = 64;

/// A paper as the driver lists it.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Listed {
    /// The driver's number for it.
    pub id: u16,
    /// Width and height in tenths of a millimetre.
    pub tenths: (i32, i32),
    /// How near each edge a sheet of it held portrait can't be printed: top, right, bottom and
    /// left, in points; `None` when it wasn't measured.
    pub margins: Option<[f32; 4]>,
    /// Its name ([`name`]), empty when the driver gives none.
    pub name: String,
}

/// A printer's papers, and its default.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Answer {
    pub papers: Vec<Listed>,
    /// The default paper's number, when it is one of [`Answer::papers`].
    pub default: Option<u16>,
}

/// Points from tenths of a millimetre.
pub(crate) fn points(tenths: i32) -> f32 {
    tenths as f32 / 254.0 * 72.0
}

/// Whether `margins` (top, right, bottom and left) are finite, not negative, and leave part
/// of a sheet of `size` (both in points).
pub(crate) fn fits(size: (f32, f32), margins: [f32; 4]) -> bool {
    let [top, right, bottom, left] = margins;
    margins.iter().all(|m| m.is_finite() && *m >= 0.0)
        && top + bottom < size.1
        && left + right < size.0
}

/// A paper's name from its slot of [`MAX_NAME`] UTF-16 units: up to the first NUL, or the
/// whole slot when there is none, trimmed, and with control characters replaced.
pub(crate) fn name(slot: &[u16]) -> String {
    let end = slot.iter().position(|&c| c == 0).unwrap_or(slot.len());
    String::from_utf16_lossy(&slot[..end])
        .trim()
        .chars()
        .map(|c| if c.is_control() { '\u{FFFD}' } else { c })
        .collect()
}

/// `answer` written as the copy writes it.
pub(crate) fn write(answer: &Answer) -> String {
    let default = answer.default.map_or("-".to_string(), |d| d.to_string());
    let mut s = format!("{VERSION}\ndefault\t{default}\n");
    for p in &answer.papers {
        let margins = match p.margins {
            Some([t, r, b, l]) => format!("{t},{r},{b},{l}"),
            None => "-".to_string(),
        };
        let (w, h) = p.tenths;
        let _ = writeln!(s, "{}\t{w}\t{h}\t{margins}\t{}", p.id, p.name);
    }
    let _ = writeln!(s, "end\t{}", answer.papers.len());
    s
}

/// The answer a copy wrote, if all of it is as [`write`] writes it and within the limits.
/// The errors don't repeat what the copy wrote.
pub(crate) fn read(answer: &[u8]) -> Result<Answer> {
    if answer.len() > MAX_ANSWER {
        bail!("it is longer than {} MiB", MAX_ANSWER >> 20);
    }
    let text = std::str::from_utf8(answer).map_err(|_| anyhow!("it isn't UTF-8"))?;
    let text = text
        .strip_suffix('\n')
        .ok_or_else(|| anyhow!("it is cut off"))?;
    let mut lines = text.split('\n');
    if lines.next() != Some(VERSION) {
        bail!("it doesn't start with \"{VERSION}\"");
    }
    let default = match lines.next().and_then(|l| l.strip_prefix("default\t")) {
        None => bail!("its second line isn't the default"),
        Some("-") => None,
        Some(d) => Some(number(d).ok_or_else(|| anyhow!("its default isn't a paper number"))?),
    };
    let mut papers = Vec::new();
    loop {
        let line = lines.next().ok_or_else(|| anyhow!("it is cut off"))?;
        if let Some(count) = line.strip_prefix("end\t") {
            if number::<usize>(count) != Some(papers.len()) {
                bail!("its end doesn't give the number of papers it lists");
            }
            break;
        }
        if papers.len() == MAX_PAPERS {
            bail!("it lists more than {MAX_PAPERS} papers");
        }
        let p = paper(line).with_context(|| format!("paper {}", papers.len() + 1))?;
        papers.push(p);
    }
    if lines.next().is_some() {
        bail!("it goes on after its end");
    }
    if let Some(d) = default {
        if !papers.iter().any(|p| p.id == d) {
            bail!("its default isn't one of its papers");
        }
    }
    Ok(Answer { papers, default })
}

/// One paper's line.
fn paper(line: &str) -> Result<Listed> {
    let fields: Vec<&str> = line.split('\t').collect();
    let &[id, w, h, margins, name] = fields.as_slice() else {
        bail!("it doesn't have five fields");
    };
    let id = number(id).ok_or_else(|| anyhow!("its number isn't a paper number"))?;
    let side = |s| number::<i32>(s).filter(|&v| v > 0);
    let (Some(w), Some(h)) = (side(w), side(h)) else {
        bail!("its size isn't two positive whole numbers");
    };
    let margins = match margins {
        "-" => None,
        m => {
            let m: Option<Vec<f32>> = m.split(',').map(|v| v.parse().ok()).collect();
            let m = m.and_then(|m| <[f32; 4]>::try_from(m).ok());
            let m = m.ok_or_else(|| anyhow!("its margins aren't four numbers"))?;
            if !fits((points(w), points(h)), m) {
                bail!("its margins don't leave part of the paper");
            }
            Some(m)
        }
    };
    if name.chars().count() > MAX_NAME || name.chars().any(char::is_control) {
        bail!("its name is longer than {MAX_NAME} characters or has control characters");
    }
    Ok(Listed {
        id,
        tenths: (w, h),
        margins,
        name: name.to_string(),
    })
}

/// A number written in digits alone, if it fits `T`.
fn number<T: FromStr>(s: &str) -> Option<T> {
    let digits = !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit());
    digits.then(|| s.parse().ok()).flatten()
}

/// The list the app and the CLI show: each paper named with its size ("Letter (8.5 × 11 in)",
/// or the size alone when it has no name), each label once ([`listed`]).
pub(crate) fn paper_list(answer: Answer) -> PaperList {
    let found = answer
        .papers
        .into_iter()
        .map(|p| {
            let size = (points(p.tenths.0), points(p.tenths.1));
            let label = match p.name.is_empty() {
                true => dims(size.0, size.1),
                false => format!("{} ({})", p.name, dims(size.0, size.1)),
            };
            Paper {
                id: p.id.to_string(),
                label,
                size,
                margins: p.margins,
            }
        })
        .collect();
    listed(found, answer.default.map(|d| d.to_string()).as_deref())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn two() -> Answer {
        Answer {
            papers: vec![
                Listed {
                    id: 1,
                    tenths: (2159, 2794),
                    margins: Some([12.24, 12.5, 0.1, 1.0 / 3.0]),
                    name: "Letter".to_string(),
                },
                Listed {
                    id: 9,
                    tenths: (2100, 2970),
                    margins: None,
                    name: "A4 – ISO 216 ✓".to_string(),
                },
            ],
            default: Some(9),
        }
    }

    /// What `two` is written as, apart from the first paper's margins.
    const TWO: &str = "limarepdf-paper-sizes 1\ndefault\t9\n\
                       1\t2159\t2794\t1,2,3,4\tLetter\n\
                       9\t2100\t2970\t-\tA4\n\
                       end\t2\n";

    #[test]
    fn an_answer_is_read_back_as_written() {
        let a = two();
        assert_eq!(read(write(&a).as_bytes()).unwrap(), a);
        let none = Answer {
            default: None,
            ..two()
        };
        assert_eq!(read(write(&none).as_bytes()).unwrap(), none);
        let empty = Answer::default();
        assert_eq!(read(write(&empty).as_bytes()).unwrap(), empty);
        let a = read(TWO.as_bytes()).unwrap();
        assert_eq!(a.papers[0].margins, Some([1.0, 2.0, 3.0, 4.0]));
        assert_eq!(a.papers[1].name, "A4");
    }

    /// Each kind of bad answer is refused, saying what is wrong with it.
    #[test]
    fn a_bad_answer_is_refused() {
        let line = |l: &str| TWO.replace("9\t2100\t2970\t-\tA4", l);
        let long_name = format!("9\t2100\t2970\t-\t{}", "x".repeat(MAX_NAME + 1));
        let cases: Vec<(String, &str)> = vec![
            (TWO.replace("sizes 1", "sizes 2"), "doesn't start with"),
            (
                TWO.replacen("limarepdf-paper-sizes 1\n", "", 1),
                "doesn't start with",
            ),
            (TWO.replace("default\t9\n", ""), "isn't the default"),
            (
                TWO.replace("default\t9", "default\t+9"),
                "default isn't a paper number",
            ),
            (
                TWO.replace("default\t9", "default\t65536"),
                "default isn't a paper number",
            ),
            (
                TWO.replace("default\t9", "default\t2"),
                "default isn't one of its papers",
            ),
            (line("+9\t2100\t2970\t-\tA4"), "number isn't a paper number"),
            (
                line("70000\t2100\t2970\t-\tA4"),
                "number isn't a paper number",
            ),
            (line("9\t0\t2970\t-\tA4"), "size isn't two positive"),
            (line("9\t-2100\t2970\t-\tA4"), "size isn't two positive"),
            (line("9\t2100\t2970.5\t-\tA4"), "size isn't two positive"),
            (
                line("9\t2100\t9999999999\t-\tA4"),
                "size isn't two positive",
            ),
            (line("9\t2100\t2970\tNaN,1,1,1\tA4"), "margins don't leave"),
            (line("9\t2100\t2970\tinf,1,1,1\tA4"), "margins don't leave"),
            (line("9\t2100\t2970\t-1,1,1,1\tA4"), "margins don't leave"),
            (
                line("9\t2100\t2970\t500,1,500,1\tA4"),
                "margins don't leave",
            ),
            (
                line("9\t2100\t2970\t1,300,1,300\tA4"),
                "margins don't leave",
            ),
            (line("9\t2100\t2970\t1,1,1\tA4"), "aren't four numbers"),
            (line("9\t2100\t2970\t1,1,1,1,1\tA4"), "aren't four numbers"),
            (line("9\t2100\t2970\t1,x,1,1\tA4"), "aren't four numbers"),
            (line("9\t2100\t2970\t-\tA\u{7}4"), "control characters"),
            (line(&long_name), "longer than 64"),
            (line("9\t2100\t2970\t-"), "five fields"),
            (line("9\t2100\t2970\t-\tA4\tx"), "five fields"),
            (TWO.replace("end\t2", "end\t3"), "number of papers"),
            (TWO.replace("end\t2", "end\t"), "number of papers"),
            (TWO.replace("end\t2\n", ""), "cut off"),
            (TWO.trim_end().to_string(), "cut off"),
            (TWO.to_string() + "end\t2\n", "goes on after its end"),
            (TWO.to_string() + "\n", "goes on after its end"),
        ];
        for (bad, why) in cases {
            let e = format!("{:#}", read(bad.as_bytes()).unwrap_err());
            assert!(e.contains(why), "{bad:?}: {e}");
        }
        let e = read(b"limarepdf-paper-sizes 1\n\xff\n").unwrap_err();
        assert!(e.to_string().contains("isn't UTF-8"), "{e}");
    }

    /// An answer may list up to [`MAX_PAPERS`] papers in up to [`MAX_ANSWER`] bytes.
    #[test]
    fn an_answer_is_held_to_its_limits() {
        let mut a = Answer::default();
        for id in 1..=MAX_PAPERS as u16 {
            a.papers.push(Listed {
                id,
                tenths: (2100, 2970),
                margins: None,
                name: String::new(),
            });
        }
        assert_eq!(read(write(&a).as_bytes()).unwrap(), a);
        a.papers.push(a.papers[0].clone());
        let e = read(write(&a).as_bytes()).unwrap_err();
        assert!(format!("{e:#}").contains("more than 4096 papers"), "{e:#}");

        let mut long = TWO.to_string();
        long.push_str(&" ".repeat(MAX_ANSWER + 1 - long.len()));
        let e = read(long.as_bytes()).unwrap_err();
        assert!(e.to_string().contains("longer than 1 MiB"), "{e}");
        long.pop();
        let e = read(long.as_bytes()).unwrap_err();
        assert!(e.to_string().contains("cut off"), "{e}");
    }

    /// The list shown is named and sized as before the copy read it (C-01): the name with the
    /// size, the size alone for a paper with no name, one of each label, the default marked.
    #[test]
    fn an_answer_gives_the_list_shown() {
        let mut a = two();
        a.papers.push(Listed {
            id: 260,
            tenths: (1000, 1500),
            margins: None,
            name: String::new(),
        });
        a.papers.push(Listed {
            id: 70,
            tenths: (2159, 2794),
            margins: None,
            name: "Letter".to_string(),
        });
        let list = paper_list(a);
        let labels: Vec<&str> = list.papers.iter().map(|p| p.label.as_str()).collect();
        assert_eq!(
            labels,
            [
                "Letter (8.5 × 11 in)",
                "A4 – ISO 216 ✓ (210 × 297 mm)",
                "100 × 150 mm"
            ]
        );
        assert_eq!(list.default, Some(1));
        assert_eq!(list.papers[1].id, "9");
        let p = &list.papers[0];
        assert_eq!(p.size, (2159.0 / 254.0 * 72.0, 2794.0 / 254.0 * 72.0));
        assert_eq!(p.margins, Some([12.24, 12.5, 0.1, 1.0 / 3.0]));
        let none = paper_list(Answer {
            default: None,
            ..two()
        });
        assert_eq!(none.default, None);
    }

    /// A name is read up to its NUL, or across the whole slot, trimmed, its control
    /// characters and broken UTF-16 replaced.
    #[test]
    fn names_are_read_from_their_slots() {
        let slot = |s: &str| {
            let mut v: Vec<u16> = s.encode_utf16().collect();
            v.resize(MAX_NAME, 0);
            v
        };
        assert_eq!(name(&slot(" Letter \n")), "Letter");
        assert_eq!(name(&slot("A4\tSmall")), "A4\u{FFFD}Small");
        assert_eq!(name(&slot("")), "");
        let full = slot(&"x".repeat(MAX_NAME));
        assert_eq!(name(&full), "x".repeat(MAX_NAME));
        let mut broken = slot("A5");
        broken[1] = 0xD800;
        assert_eq!(name(&broken), "A\u{FFFD}");
        assert!(fits((100.0, 100.0), [49.0, 49.0, 50.0, 50.0]));
        assert!(!fits((100.0, 100.0), [50.0, 0.0, 50.0, 0.0]));
    }
}

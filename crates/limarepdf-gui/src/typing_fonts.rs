//! Fonts for text being typed on the page, so it shows in the look it will have (the
//! word-processor design, section 4).
//!
//! New text and lines written anew are in one of the 12 standard fonts, drawn here in the
//! bundled Liberation fonts, which have the widths of Helvetica, Times and Courier; each face
//! is unpacked (core stores them compressed) and loaded the first time it is needed. A line
//! that keeps its own font is drawn in that font when the computer has it (found through
//! core's installed fonts and loaded the first time it is needed), else in the Liberation
//! face of its kind. Inter is the last resort, and the stand-in while a font loads, and
//! every family falls back to Inter for characters its font lacks.
//!
//! Text in an installed family (the Unicode text design, section 7) is drawn in the
//! family's font file, at the weight chosen when it is a variable font ([`coords`]); the
//! font list draws each family's name in it ([`preview`]).

use eframe::egui::epaint::text::VariationCoords;
use eframe::egui::{self, FontData, FontFamily};
use limarepdf_core::render::InstalledFonts;
use limarepdf_core::{bundled_face, liberation, FaceFile, FontChoice};
use std::borrow::Cow;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

/// The bundled face for a standard font: Helvetica is Liberation Sans, and so on. The faces
/// are core's (`LIBERATION`, `bundled_face`), which embeds them in PDFs.
fn face(f: FontChoice) -> &'static str {
    liberation(f)
}

fn named(name: &str) -> FontFamily {
    FontFamily::Name(name.into())
}

/// A family of one font, then `fallback` (Inter and egui's own fonts).
fn with_fallback(name: &str, fallback: &[String]) -> Vec<String> {
    let mut list = vec![name.to_string()];
    list.extend(fallback.iter().filter(|n| *n != name).cloned());
    list
}

/// What text being typed will be written in.
#[derive(Clone, Copy, Debug)]
pub enum Wanted<'a> {
    /// A standard font or an installed family's face: new text, and lines written anew.
    Standard(FontChoice),
    /// A line that keeps its own font, by the name the document gives it ("Arial,Bold").
    Kept(&'a str),
}

/// A font to draw with.
#[derive(Clone, Debug, PartialEq)]
enum Candidate {
    /// An installed font, by a PDF font name.
    Installed(String),
    /// An installed family's font file, and the face's index in it.
    File(PathBuf, u32),
    /// A bundled Liberation face, a family of its own named as in core's `LIBERATION`.
    Bundled(&'static str),
    Inter,
}

/// The fonts to try, best first.
fn candidates(w: Wanted) -> Vec<Candidate> {
    let (mut out, f) = match w {
        Wanted::Standard(f) => {
            let file = f
                .typeface()
                .and_then(|t| t.face(f.bold, f.italic).cloned())
                .map(|file| Candidate::File(file.path, file.index));
            (file.into_iter().collect(), f.standard())
        }
        Wanted::Kept(name) => (
            vec![Candidate::Installed(name.to_string())],
            FontChoice::from_name(name),
        ),
    };
    // The bundled face is always there, ready a frame or two after it is first asked for:
    // Inter meanwhile, rather than loading an installed font that would soon go unused.
    out.push(Candidate::Bundled(face(f)));
    out.push(Candidate::Inter);
    out
}

/// The family of the first candidate that is ready to draw with. `ready` may start
/// loading an installed font, which is then ready in a later frame.
fn pick(w: Wanted, mut ready: impl FnMut(&Candidate) -> bool) -> FontFamily {
    match candidates(w).into_iter().find(|c| ready(c)) {
        Some(Candidate::Installed(name)) => named(&installed_key(&name)),
        Some(Candidate::File(path, index)) => named(&file_key(&path, index)),
        Some(Candidate::Bundled(name)) => named(name),
        Some(Candidate::Inter) | None => FontFamily::Proportional,
    }
}

/// The egui font family to draw text being typed in.
pub fn family(ctx: &egui::Context, w: Wanted) -> FontFamily {
    pick(w, |c| match c {
        Candidate::Installed(name) => loaded(ctx, &installed_key(name), None, {
            let name = name.clone();
            move || {
                let f = InstalledFonts::system().named(&name)?;
                Some((Cow::Owned(f.data.as_ref().as_ref().to_vec()), f.index))
            }
        }),
        Candidate::File(path, index) => file(ctx, path, *index, None),
        Candidate::Bundled(name) => {
            let name = *name;
            loaded(ctx, name, None, move || {
                Some((Cow::Borrowed(bundled_face(name)?), 0))
            })
        }
        Candidate::Inter => true,
    })
}

/// The variation settings to draw text in `w` with: the weight of a variable font, as it
/// will be embedded; none for others (drawing ignores settings a font has no axis for, so
/// text drawn in a stand-in while the font loads is unchanged).
pub fn coords(w: Wanted) -> VariationCoords {
    let Wanted::Standard(f) = w else {
        return VariationCoords::default();
    };
    let face = f.typeface().and_then(|t| t.face(f.bold, f.italic).cloned());
    VariationCoords::new(face.into_iter().flat_map(|face| face.coords))
}

/// Largest font file loaded only to draw a family's name in the font list: Chinese,
/// Japanese and Korean fonts run to tens of megabytes, and their names show in the
/// interface font instead.
const MAX_PREVIEW: u64 = 4 << 20;

/// The egui font family to draw a family's name in for the font list, from the face
/// `face`; `None` until it is loaded (the first question starts loading it), or when the
/// font is too large to load only for that.
pub fn preview(ctx: &egui::Context, face: &FaceFile) -> Option<FontFamily> {
    file(ctx, &face.path, face.index, Some(MAX_PREVIEW))
        .then(|| named(&file_key(&face.path, face.index)))
}

fn has(ctx: &egui::Context, family: &FontFamily) -> bool {
    ctx.fonts(|f| f.definitions().families.contains_key(family))
}

/// The egui name of an installed font, by the PDF font name it was found for.
fn installed_key(name: &str) -> String {
    format!("installed {name}")
}

/// The egui name of a font file's face.
fn file_key(path: &std::path::Path, index: u32) -> String {
    format!("file {}#{index}", path.display())
}

/// Whether the face at `index` in the font file `path` is ready to draw with; the first
/// question starts loading it. With `max`, a larger file isn't loaded.
fn file(ctx: &egui::Context, path: &std::path::Path, index: u32, max: Option<u64>) -> bool {
    let path = path.to_path_buf();
    loaded(ctx, &file_key(&path, index), max, move || {
        if max.is_some_and(|max| std::fs::metadata(&path).map_or(true, |m| m.len() > max)) {
            return None;
        }
        Some((Cow::Owned(std::fs::read(&path).ok()?), index))
    })
}

/// Fonts loaded, by egui name, kept in the context's memory.
#[derive(Clone, Default)]
struct Loads(Arc<Mutex<HashMap<String, Load>>>);

enum Load {
    /// Being looked for or read on another thread: the first search reads every installed
    /// font.
    Looking,
    /// Found; `sent` once handed to egui, which adds it at the start of the next frame.
    Found { data: Arc<FontData>, sent: bool },
    /// Not installed, or not read: too large, when `limited`, which a question without a
    /// limit asks again.
    Missing { limited: bool },
}

/// Whether the font egui knows as `key` is ready to draw with; the first question starts
/// `fetch` (its bytes and index) on another thread. `max` says whether the question had a
/// size limit.
fn loaded(
    ctx: &egui::Context,
    key: &str,
    max: Option<u64>,
    fetch: impl FnOnce() -> Option<(Cow<'static, [u8]>, u32)> + Send + 'static,
) -> bool {
    if has(ctx, &named(key)) {
        return true;
    }
    let loads = ctx.data_mut(|d| {
        d.get_temp_mut_or_default::<Loads>(egui::Id::new("typing fonts"))
            .0
            .clone()
    });
    let mut l = loads.lock().unwrap_or_else(|e| e.into_inner());
    let start = match l.get(key) {
        None => true,
        Some(Load::Missing { limited: true }) => max.is_none(),
        _ => false,
    };
    match l.get(key) {
        _ if start => {
            l.insert(key.to_string(), Load::Looking);
            let (loads, ctx, key) = (loads.clone(), ctx.clone(), key.to_string());
            std::thread::spawn(move || {
                let load = match fetch() {
                    Some((bytes, index)) => Load::Found {
                        data: Arc::new(FontData {
                            font: bytes,
                            index,
                            tweak: Default::default(),
                        }),
                        sent: false,
                    },
                    None => Load::Missing {
                        limited: max.is_some(),
                    },
                };
                loads
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .insert(key, load);
                ctx.request_repaint();
            });
        }
        Some(Load::Found { sent: false, .. }) => {
            // Every font found so far goes in, so two arriving in one frame both stay.
            let mut defs = ctx.fonts(|f| f.definitions().clone());
            let fallback = defs.families[&FontFamily::Proportional].clone();
            for (key, load) in l.iter_mut() {
                if let Load::Found { data, sent } = load {
                    defs.font_data.insert(key.clone(), data.clone());
                    defs.families
                        .insert(named(key), with_fallback(key, &fallback));
                    *sent = true;
                }
            }
            drop(l);
            ctx.set_fonts(defs);
        }
        _ => {}
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use limarepdf_core::{Family, LIBERATION};
    use std::time::{Duration, Instant};

    fn choice(family: Family, bold: bool, italic: bool) -> FontChoice {
        FontChoice {
            family,
            bold,
            italic,
        }
    }

    #[test]
    fn every_standard_font_has_its_liberation_face() {
        let mut seen = Vec::new();
        for family in [Family::Helvetica, Family::Times, Family::Courier] {
            for (bold, italic) in [(false, false), (true, false), (false, true), (true, true)] {
                let f = choice(family, bold, italic);
                let want = format!(
                    "Liberation {}{}{}",
                    match family {
                        Family::Times => "Serif",
                        Family::Courier => "Mono",
                        Family::Helvetica | Family::Installed(_) => "Sans",
                    },
                    if bold { " Bold" } else { "" },
                    if italic { " Italic" } else { "" },
                );
                assert_eq!(face(f), want, "{}", f.base_font());
                // The file is that face: its PostScript name, "LiberationSerif-BoldItalic",
                // is in it, and no other style's.
                let bytes = bundled_face(&want).unwrap();
                let base = want.split(' ').take(2).collect::<String>();
                let style = want.split(' ').skip(2).collect::<String>();
                let ps = |style: &str| match style {
                    "" => base.clone(),
                    s => format!("{base}-{s}"),
                };
                assert!(contains(bytes, &ps(&style)), "{want}");
                for other in ["Bold", "Italic", "BoldItalic"] {
                    // "-Bold" is in "-BoldItalic".
                    if !ps(&style).starts_with(&ps(other)) {
                        assert!(!contains(bytes, &ps(other)), "{want} has {other}");
                    }
                }
                seen.push(want);
            }
        }
        seen.dedup();
        assert_eq!(seen.len(), 12);
    }

    /// Whether a font file has a name, in ASCII or UTF-16 (as name tables keep them).
    fn contains(bytes: &[u8], name: &str) -> bool {
        let wide: Vec<u8> = name.bytes().flat_map(|b| [0, b]).collect();
        [name.as_bytes(), &wide]
            .iter()
            .any(|n| bytes.windows(n.len()).any(|w| w == *n))
    }

    #[test]
    fn the_order_fonts_are_tried_in() {
        let bold_times = choice(Family::Times, true, false);
        assert_eq!(
            candidates(Wanted::Standard(bold_times)),
            [
                Candidate::Bundled("Liberation Serif Bold"),
                Candidate::Inter
            ]
        );
        // A kept font: the font itself first, then the Liberation face of its kind.
        let kept = candidates(Wanted::Kept("CourierNewPS-ItalicMT"));
        assert_eq!(
            kept,
            [
                Candidate::Installed("CourierNewPS-ItalicMT".into()),
                Candidate::Bundled("Liberation Mono Italic"),
                Candidate::Inter,
            ]
        );

        // Each falls to the next that is ready.
        let w = Wanted::Kept("Arial,Bold");
        assert_eq!(pick(w, |_| true), named("installed Arial,Bold"));
        assert_eq!(
            pick(w, |c| !matches!(c, Candidate::Installed(_))),
            named("Liberation Sans Bold")
        );
        assert_eq!(
            pick(w, |c| *c == Candidate::Inter),
            FontFamily::Proportional
        );
    }

    fn ctx() -> egui::Context {
        let ctx = egui::Context::default();
        crate::theme::install(&ctx);
        frame(&ctx);
        ctx
    }

    fn frame(ctx: &egui::Context) {
        let mut out = ctx.run_ui(Default::default(), |_| {});
        out.textures_delta.clear();
    }

    /// The family text in `w` is drawn in, after frames until it is `want` (a minute at
    /// most): fonts load on another thread.
    fn settled(ctx: &egui::Context, w: Wanted, want: &FontFamily) -> FontFamily {
        let done = Instant::now() + Duration::from_secs(60);
        let mut got = family(ctx, w);
        while got != *want && Instant::now() < done {
            frame(ctx);
            got = family(ctx, w);
            std::thread::sleep(Duration::from_millis(20));
        }
        got
    }

    #[test]
    fn the_bundled_faces_are_loaded_on_first_use() {
        // None at start: each is unpacked and added the first time text in it is drawn, in
        // Inter until then.
        let ctx = ctx();
        for name in LIBERATION.iter().flatten() {
            assert!(!has(&ctx, &named(name)), "{name} at start");
        }
        let w = Wanted::Standard(choice(Family::Courier, false, true));
        assert_eq!(family(&ctx, w), FontFamily::Proportional);
        let want = named("Liberation Mono Italic");
        assert_eq!(settled(&ctx, w, &want), want);
        for name in LIBERATION.iter().flatten() {
            if *name != "Liberation Mono Italic" {
                assert!(!has(&ctx, &named(name)), "{name} too");
            }
        }
        let list = ctx.fonts(|f| f.definitions().families[&want].clone());
        assert_eq!(list[0], "Liberation Mono Italic");
        assert!(list.contains(&"Inter".to_string()), "falls back to Inter");

        for f in [choice(Family::Courier, false, false), FontChoice::default()] {
            let want = named(face(f));
            assert_eq!(settled(&ctx, Wanted::Standard(f), &want), want);
        }
        // Drawn with the face: Courier's widths, so every letter is as wide.
        let width = |s: &str| {
            ctx.fonts_mut(|fonts| {
                fonts
                    .layout_no_wrap(
                        s.into(),
                        egui::FontId::new(20.0, named("Liberation Mono")),
                        egui::Color32::BLACK,
                    )
                    .size()
                    .x
            })
        };
        assert!((width("iiii") - width("MMMM")).abs() < 0.01);
        // egui sizes a font by its em, as PDF does: text typed in Liberation Sans is as
        // wide as core measures it in Helvetica, so the caret and line length match.
        let sans = |s: &str| {
            ctx.fonts_mut(|fonts| {
                fonts
                    .layout_no_wrap(
                        s.into(),
                        egui::FontId::new(100.0, named("Liberation Sans")),
                        egui::Color32::BLACK,
                    )
                    .size()
                    .x
            })
        };
        for s in ["Walrus like cold water", "MMMM iiii", "Hello, World!"] {
            let want = limarepdf_core::text_width(s, 100.0);
            assert!(
                (sans(s) - want).abs() < 0.01 * want,
                "{s}: {} vs {want}",
                sans(s)
            );
        }
    }

    #[test]
    fn a_kept_font_is_loaded_when_installed() {
        // A font no computer has: the Liberation face of its kind, and still that once
        // the search is done.
        let ctx = ctx();
        let w = Wanted::Kept("NoSuchFontAnywhere-BoldItalic");
        let stand_in = named("Liberation Sans Bold Italic");
        assert_eq!(settled(&ctx, w, &stand_in), stand_in);
        let done = Instant::now() + Duration::from_secs(60);
        loop {
            frame(&ctx);
            let loads = ctx.data_mut(|d| d.get_temp::<Loads>(egui::Id::new("typing fonts")));
            let looking = loads.is_some_and(|l| {
                matches!(
                    l.0.lock()
                        .unwrap()
                        .get(&installed_key("NoSuchFontAnywhere-BoldItalic")),
                    Some(Load::Looking)
                )
            });
            if !looking || Instant::now() > done {
                break;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        assert_eq!(family(&ctx, w), stand_in);

        // One that is installed here (most Linux systems have Liberation or DejaVu):
        // drawn in it a frame or two after the search ends.
        let fonts = InstalledFonts::system();
        let Some(name) = ["Liberation Sans", "DejaVu Sans", "Arial", "Helvetica"]
            .into_iter()
            .find(|n| fonts.named(n).is_some())
        else {
            return;
        };
        let w = Wanted::Kept(name);
        let got = settled(&ctx, w, &named(&installed_key(name)));
        assert_eq!(got, named(&installed_key(name)));
        let list = ctx.fonts(|f| f.definitions().families[&got].clone());
        assert!(list.contains(&"Inter".to_string()));
    }

    #[test]
    fn installed_faces_are_drawn_from_their_file_at_their_weight() {
        let bold = choice(crate::font_list::test_family("Caveat"), true, false);
        let t = bold.typeface().unwrap();
        let face = t.face(true, false).unwrap().clone();
        let w = Wanted::Standard(bold);
        // The family's file first, then the Liberation face of its kind.
        let list = candidates(w);
        assert_eq!(list[0], Candidate::File(face.path.clone(), face.index));
        assert_eq!(list[1], Candidate::Bundled("Liberation Sans Bold"));
        // At the weight it will be written in; standard fonts have no settings.
        assert_eq!(coords(w), VariationCoords::new([(*b"wght", 700.0)]));
        let helvetica = Wanted::Standard(FontChoice::default());
        assert_eq!(coords(helvetica), VariationCoords::default());

        // Drawn in a stand-in until the file is loaded, then in it; its name in the font
        // list too.
        let ctx = ctx();
        let want = named(&file_key(&face.path, face.index));
        assert_ne!(family(&ctx, w), want);
        assert_eq!(settled(&ctx, w, &want), want);
        assert_eq!(preview(&ctx, &face), Some(want));
    }
}

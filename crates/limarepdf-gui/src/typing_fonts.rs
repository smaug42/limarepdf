//! Fonts for text being typed on the page, so it shows in the look it will have (the
//! word-processor design, section 4).
//!
//! New text and lines written anew are in one of the 12 standard fonts, drawn here in the
//! bundled Liberation fonts, which have the widths of Helvetica, Times and Courier. A line
//! that keeps its own font is drawn in that font when the computer has it (found through
//! core's installed fonts and loaded the first time it is needed), else in the Liberation
//! face of its kind. Inter is the last resort, and every family falls back to Inter for
//! characters its font lacks.

use eframe::egui::{self, FontData, FontDefinitions, FontFamily};
use limarepdf_core::render::InstalledFonts;
use limarepdf_core::{liberation, Family, FontChoice, LIBERATION};
use std::borrow::Cow;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

/// The bundled face for a standard font: Helvetica is Liberation Sans, and so on. The faces
/// are core's (`LIBERATION`), which embeds them in PDFs.
fn face(f: FontChoice) -> &'static str {
    liberation(f).0
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

/// Registers the bundled faces, each a family of its own named as in [`LIBERATION`].
pub fn add_bundled(f: &mut FontDefinitions) {
    let fallback = f.families[&FontFamily::Proportional].clone();
    for (name, bytes) in LIBERATION.iter().flatten() {
        f.font_data
            .insert(name.to_string(), FontData::from_static(bytes).into());
        f.families
            .insert(named(name), with_fallback(name, &fallback));
    }
}

/// What text being typed will be written in.
#[derive(Clone, Copy, Debug)]
pub enum Wanted<'a> {
    /// One of the 12 standard fonts: new text, and lines written anew.
    Standard(FontChoice),
    /// A line that keeps its own font, by the name the document gives it ("Arial,Bold").
    Kept(&'a str),
}

/// A font to draw with.
#[derive(Clone, Debug, PartialEq)]
enum Candidate {
    /// An installed font, by a PDF font name.
    Installed(String),
    /// A bundled Liberation face.
    Bundled(&'static str),
    Inter,
}

/// The fonts to try, best first.
fn candidates(w: Wanted) -> Vec<Candidate> {
    let (mut out, f) = match w {
        Wanted::Standard(f) => (vec![], f),
        Wanted::Kept(name) => (
            vec![Candidate::Installed(name.to_string())],
            FontChoice::from_name(name),
        ),
    };
    out.push(Candidate::Bundled(face(f)));
    // Should the bundled face be missing: an installed copy, a font with its widths,
    // then the font it stands for.
    let style = match (f.bold, f.italic) {
        (false, false) => "",
        (true, false) => ",Bold",
        (false, true) => ",Italic",
        (true, true) => ",BoldItalic",
    };
    let names = match f.family {
        Family::Helvetica => ["Liberation Sans", "Arial", "Helvetica"],
        Family::Times => ["Liberation Serif", "Times New Roman", "Times"],
        Family::Courier => ["Liberation Mono", "Courier New", "Courier"],
    };
    out.extend(
        names
            .iter()
            .map(|n| Candidate::Installed(format!("{n}{style}"))),
    );
    out.push(Candidate::Inter);
    out
}

/// The family of the first candidate that is ready to draw with. `ready` may start
/// loading an installed font, which is then ready in a later frame.
fn pick(w: Wanted, mut ready: impl FnMut(&Candidate) -> bool) -> FontFamily {
    match candidates(w).into_iter().find(|c| ready(c)) {
        Some(Candidate::Installed(name)) => named(&installed_key(&name)),
        Some(Candidate::Bundled(name)) => named(name),
        Some(Candidate::Inter) | None => FontFamily::Proportional,
    }
}

/// The egui font family to draw text being typed in.
pub fn family(ctx: &egui::Context, w: Wanted) -> FontFamily {
    pick(w, |c| match c {
        Candidate::Installed(name) => installed(ctx, name),
        Candidate::Bundled(name) => has(ctx, &named(name)),
        Candidate::Inter => true,
    })
}

fn has(ctx: &egui::Context, family: &FontFamily) -> bool {
    ctx.fonts(|f| f.definitions().families.contains_key(family))
}

/// The egui name of an installed font, by the PDF font name it was found for.
fn installed_key(name: &str) -> String {
    format!("installed {name}")
}

/// Installed fonts looked for, by PDF font name, kept in the context's memory.
#[derive(Clone, Default)]
struct Loads(Arc<Mutex<HashMap<String, Load>>>);

enum Load {
    /// Being looked for on another thread: the first search reads every installed font.
    Looking,
    /// Found; `sent` once handed to egui, which adds it at the start of the next frame.
    Found { data: Arc<FontData>, sent: bool },
    /// Not installed.
    Missing,
}

/// Whether the installed font a PDF font name stands for is ready to draw with; the
/// first question starts looking for it.
fn installed(ctx: &egui::Context, name: &str) -> bool {
    let key = installed_key(name);
    if has(ctx, &named(&key)) {
        return true;
    }
    let loads = ctx.data_mut(|d| {
        d.get_temp_mut_or_default::<Loads>(egui::Id::new("typing fonts"))
            .0
            .clone()
    });
    let mut l = loads.lock().unwrap_or_else(|e| e.into_inner());
    match l.get(name) {
        None => {
            l.insert(name.to_string(), Load::Looking);
            let (loads, ctx, name) = (loads.clone(), ctx.clone(), name.to_string());
            std::thread::spawn(move || {
                let load = match InstalledFonts::system().named(&name) {
                    Some(f) => Load::Found {
                        data: Arc::new(FontData {
                            font: Cow::Owned(f.data.as_ref().as_ref().to_vec()),
                            index: f.index,
                            tweak: Default::default(),
                        }),
                        sent: false,
                    },
                    None => Load::Missing,
                };
                loads
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .insert(name, load);
                ctx.request_repaint();
            });
        }
        Some(Load::Found { sent: false, .. }) => {
            // Every font found so far goes in, so two arriving in one frame both stay.
            let mut defs = ctx.fonts(|f| f.definitions().clone());
            let fallback = defs.families[&FontFamily::Proportional].clone();
            for (name, load) in l.iter_mut() {
                if let Load::Found { data, sent } = load {
                    let key = installed_key(name);
                    defs.font_data.insert(key.clone(), data.clone());
                    defs.families
                        .insert(named(&key), with_fallback(&key, &fallback));
                    *sent = true;
                }
            }
            drop(l);
            ctx.set_fonts(defs);
        }
        Some(Load::Looking | Load::Found { sent: true, .. } | Load::Missing) => {}
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;
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
                        Family::Helvetica => "Sans",
                        Family::Times => "Serif",
                        Family::Courier => "Mono",
                    },
                    if bold { " Bold" } else { "" },
                    if italic { " Italic" } else { "" },
                );
                assert_eq!(face(f), want, "{}", f.base_font());
                // The file is that face: its PostScript name, "LiberationSerif-BoldItalic",
                // is in it, and no other style's.
                let (_, bytes) = LIBERATION
                    .iter()
                    .flatten()
                    .find(|(n, _)| *n == want)
                    .unwrap();
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
                Candidate::Installed("Liberation Serif,Bold".into()),
                Candidate::Installed("Times New Roman,Bold".into()),
                Candidate::Installed("Times,Bold".into()),
                Candidate::Inter,
            ]
        );
        // A kept font: the font itself first, then the Liberation face of its kind.
        let kept = candidates(Wanted::Kept("CourierNewPS-ItalicMT"));
        assert_eq!(
            kept[0],
            Candidate::Installed("CourierNewPS-ItalicMT".into())
        );
        assert_eq!(kept[1], Candidate::Bundled("Liberation Mono Italic"));
        assert_eq!(kept.last(), Some(&Candidate::Inter));

        // Each falls to the next that is ready.
        let w = Wanted::Kept("Arial,Bold");
        assert_eq!(pick(w, |_| true), named("installed Arial,Bold"));
        assert_eq!(
            pick(w, |c| !matches!(c, Candidate::Installed(_))),
            named("Liberation Sans Bold")
        );
        assert_eq!(
            pick(
                w,
                |c| matches!(c, Candidate::Installed(n) if n.starts_with("Helvetica"))
            ),
            named("installed Helvetica,Bold")
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

    #[test]
    fn the_bundled_faces_are_registered() {
        let ctx = ctx();
        for (name, _) in LIBERATION.iter().flatten() {
            let list = ctx.fonts(|f| f.definitions().families[&named(name)].clone());
            assert_eq!(list[0], *name);
            assert!(
                list.contains(&"Inter".to_string()),
                "{name} falls back to Inter"
            );
        }
        let f = choice(Family::Courier, false, true);
        assert_eq!(
            family(&ctx, Wanted::Standard(f)),
            named("Liberation Mono Italic")
        );
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
        assert_eq!(family(&ctx, w), named("Liberation Sans Bold Italic"));
        let done = Instant::now() + Duration::from_secs(60);
        loop {
            frame(&ctx);
            let loads = ctx.data_mut(|d| d.get_temp::<Loads>(egui::Id::new("typing fonts")));
            let looking = loads.is_some_and(|l| {
                matches!(
                    l.0.lock().unwrap().get("NoSuchFontAnywhere-BoldItalic"),
                    Some(Load::Looking)
                )
            });
            if !looking || Instant::now() > done {
                break;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        assert_eq!(family(&ctx, w), named("Liberation Sans Bold Italic"));

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
        let mut got = family(&ctx, w);
        while got != named(&installed_key(name)) && Instant::now() < done {
            frame(&ctx);
            got = family(&ctx, w);
            std::thread::sleep(Duration::from_millis(20));
        }
        assert_eq!(got, named(&installed_key(name)));
        let list = ctx.fonts(|f| f.definitions().families[&got].clone());
        assert!(list.contains(&"Inter".to_string()));
    }
}

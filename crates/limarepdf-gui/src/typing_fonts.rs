//! Fonts for text being typed on the page, so it shows in the look it will have (the
//! word-processor design, section 4).
//!
//! New text and lines written anew are in one of the 12 standard fonts, drawn here in the
//! bundled Liberation fonts, which have the widths of Helvetica, Times and Courier; each face
//! is unpacked (core stores them compressed) and loaded the first time it is needed. A line
//! that keeps its own embedded font is drawn in that font's own glyphs, in a small font core
//! makes of them with the PDF's widths ([`LineFont::preview`], made on another thread),
//! falling back to the Liberation face of its kind for what it lacks; a font the PDF doesn't
//! hold is drawn in the face LimarePDF's pages draw it in once kept ([`LineFont::stand_in`],
//! ENH-25): the Liberation face of the standard font the renderer takes for it, or for a CID
//! font the installed font it finds (loaded the first time it is needed). A line whose fonts
//! aren't read yet is drawn in the installed font of its font's name, else the Liberation
//! face of its kind by the name. Inter is the last resort, and the stand-in while a font
//! loads, and every family falls back to Inter for characters its font lacks.
//!
//! Text in an installed family (the Unicode text design, section 7) is drawn in the
//! family's font file, at the weight chosen when it is a variable font ([`coords`]); the
//! font list draws each family's name in it ([`preview`]).

use eframe::egui::epaint::text::VariationCoords;
use eframe::egui::{self, FontData, FontFamily};
use limarepdf_core::render::InstalledFonts;
use limarepdf_core::{
    bundled_face, liberation, FaceFile, FontChoice, InstalledQuery, LineFont, StandIn,
};
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

/// What text being typed will be written in.
#[derive(Clone, Copy, Debug)]
pub enum Wanted<'a> {
    /// A standard font or an installed family's face: new text, and lines written anew.
    Standard(FontChoice),
    /// A line that keeps its own font, by the name the document gives it ("Arial,Bold"),
    /// and the font itself when it is known ([`limarepdf_core::Pdf::line_fonts`]).
    Kept(&'a str, Option<&'a LineFont>),
}

/// A font to draw with.
#[derive(Clone, Debug, PartialEq)]
enum Candidate {
    /// A PDF font's own glyphs, in a font core makes of them ([`LineFont::preview`]).
    Preview(Own),
    /// An installed font, by a PDF font name.
    Installed(String),
    /// The installed font the pages draw a CID font the PDF doesn't hold in, by the
    /// renderer's question for it.
    Found(InstalledQuery),
    /// An installed family's font file, and the face's index in it.
    File(PathBuf, u32),
    /// A bundled Liberation face, a family of its own named as in core's `LIBERATION`.
    Bundled(&'static str),
    Inter,
}

/// A PDF font whose own glyphs are drawn ([`Candidate::Preview`]): what tells its preview
/// font from others ([`LineFont::preview_key`]), the font, and the bundled face of its kind,
/// which draws what its preview lacks.
#[derive(Clone, Debug)]
struct Own(u64, LineFont, &'static str);

impl PartialEq for Own {
    fn eq(&self, other: &Self) -> bool {
        (self.0, self.2) == (other.0, other.2)
    }
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
        Wanted::Kept(name, font) => match font.and_then(LineFont::stand_in) {
            // A font the PDF doesn't hold: the face the pages draw it in (ENH-25).
            Some(StandIn::Installed(q, f)) => (vec![Candidate::Found(q.clone())], *f),
            Some(StandIn::Standard(f)) => (Vec::new(), *f),
            None => {
                let f = FontChoice::from_name(name);
                let own =
                    font.and_then(|font| Some(Own(font.preview_key()?, font.clone(), face(f))));
                let mut out: Vec<Candidate> = own.map(Candidate::Preview).into_iter().collect();
                out.push(Candidate::Installed(name.to_string()));
                (out, f)
            }
        },
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
        Some(Candidate::Preview(own)) => named(&preview_key(own.0)),
        Some(Candidate::Installed(name)) => named(&installed_key(&name)),
        Some(Candidate::Found(q)) => named(&found_key(&q)),
        Some(Candidate::File(path, index)) => named(&file_key(&path, index)),
        Some(Candidate::Bundled(name)) => named(name),
        Some(Candidate::Inter) | None => FontFamily::Proportional,
    }
}

/// The egui font family to draw text being typed in.
pub fn family(ctx: &egui::Context, w: Wanted) -> FontFamily {
    pick(w, |c| match c {
        Candidate::Preview(Own(key, font, face)) => {
            // Its own glyphs, made on another thread; the face of its kind, for what they
            // lack, loaded too.
            bundled(ctx, face);
            let font = font.clone();
            let fetch = move || Some((Cow::Owned(font.preview()?), 0));
            loaded(ctx, &preview_key(*key), &[face], None, fetch)
        }
        Candidate::Installed(name) => loaded(ctx, &installed_key(name), &[], None, {
            let name = name.clone();
            move || {
                let f = InstalledFonts::system().named(&name)?;
                Some((Cow::Owned(f.data.as_ref().as_ref().to_vec()), f.index))
            }
        }),
        Candidate::Found(q) => loaded(ctx, &found_key(q), &[], None, {
            let q = q.clone();
            move || {
                let f = q.find(InstalledFonts::system())?;
                Some((Cow::Owned(f.data.as_ref().as_ref().to_vec()), f.index))
            }
        }),
        Candidate::File(path, index) => file(ctx, path, *index, None),
        Candidate::Bundled(name) => bundled(ctx, name),
        Candidate::Inter => true,
    })
}

/// Whether the bundled face `name` is ready to draw with; the first question starts
/// unpacking it.
fn bundled(ctx: &egui::Context, name: &'static str) -> bool {
    loaded(ctx, name, &[], None, move || {
        Some((Cow::Borrowed(bundled_face(name)?), 0))
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

/// The egui name of the installed font found for a CID font the PDF doesn't hold, by the
/// question it was found for.
fn found_key(q: &InstalledQuery) -> String {
    format!("found {}", q.key())
}

/// The egui name of a PDF font's preview font, by its key ([`LineFont::preview_key`]).
fn preview_key(key: u64) -> String {
    format!("preview {key:016x}")
}

/// The egui name of a font file's face.
fn file_key(path: &std::path::Path, index: u32) -> String {
    format!("file {}#{index}", path.display())
}

/// Whether the face at `index` in the font file `path` is ready to draw with; the first
/// question starts loading it. With `max`, a larger file isn't loaded.
fn file(ctx: &egui::Context, path: &std::path::Path, index: u32, max: Option<u64>) -> bool {
    let path = path.to_path_buf();
    loaded(ctx, &file_key(&path, index), &[], max, move || {
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
    /// `then` names the fonts it falls back to before Inter, once they are loaded. An
    /// `interface` font is one every font of the interface falls back to as well
    /// ([`interface_fallback`]).
    Found {
        data: Arc<FontData>,
        sent: bool,
        then: Vec<String>,
        interface: bool,
    },
    /// Not installed, or not read: too large, when `limited`, which a question without a
    /// limit asks again.
    Missing { limited: bool },
}

/// Whether the font egui knows as `key` is ready to draw with; the first question starts
/// `fetch` (its bytes and index) on another thread. It falls back to the fonts `then` once
/// they are loaded, then to Inter. `max` says whether the question had a size limit.
fn loaded(
    ctx: &egui::Context,
    key: &str,
    then: &[&str],
    max: Option<u64>,
    fetch: impl FnOnce() -> Option<(Cow<'static, [u8]>, u32)> + Send + 'static,
) -> bool {
    if has(ctx, &named(key)) {
        return true;
    }
    let loads = loads(ctx);
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
            let then: Vec<String> = then.iter().map(|n| n.to_string()).collect();
            std::thread::spawn(move || {
                let load = match fetch() {
                    Some((bytes, index)) => {
                        let data = match bytes {
                            Cow::Borrowed(b) => FontData::from_static(b),
                            Cow::Owned(v) => FontData::from_owned(v),
                        };
                        Load::Found {
                            data: Arc::new(FontData { index, ..data }),
                            sent: false,
                            then,
                            interface: false,
                        }
                    }
                    None => Load::Missing {
                        limited: max.is_some(),
                    },
                };
                let mut l = loads.lock().unwrap_or_else(|e| e.into_inner());
                arrived(&mut l, key, load);
                drop(l);
                ctx.request_repaint();
            });
        }
        Some(Load::Found { sent: false, .. }) => {
            let defs = install(ctx, &mut l);
            drop(l);
            ctx.set_fonts(defs);
        }
        _ => {}
    }
    false
}

/// Puts `load`, just read, in `l` as `key`. Typing and the interface's fallback
/// ([`interface_fallback`]) can read one face at once: what the other read arrived with
/// stays, so it is the interface's if either says so, and falls back to what typing asked.
fn arrived(l: &mut HashMap<String, Load>, key: String, mut load: Load) {
    if let (
        Load::Found {
            then, interface, ..
        },
        Some(Load::Found {
            then: had,
            interface: was,
            ..
        }),
    ) = (&mut load, l.get(&key))
    {
        *interface |= *was;
        if then.is_empty() {
            *then = had.clone();
        }
    }
    l.insert(key, load);
}

/// The fonts loaded so far, by egui name, kept in the context's memory.
fn loads(ctx: &egui::Context) -> Arc<Mutex<HashMap<String, Load>>> {
    ctx.data_mut(|d| {
        d.get_temp_mut_or_default::<Loads>(egui::Id::unique("typing fonts"))
            .0
            .clone()
    })
}

/// egui's fonts with every font found so far (`l`) in, so two arriving in one frame both
/// stay: each with a family of its own, which falls back to what it names that is in, then
/// to the interface's fonts; and each the interface falls back to at the end of every
/// family. Marks them handed to egui.
fn install(ctx: &egui::Context, l: &mut HashMap<String, Load>) -> egui::FontDefinitions {
    let mut defs = ctx.fonts(|f| f.definitions().clone());
    let fallback = defs.families[&FontFamily::Proportional].clone();
    for (key, load) in l.iter_mut() {
        if let Load::Found { data, sent, .. } = load {
            defs.font_data.insert(key.clone(), data.clone());
            *sent = true;
        }
    }
    for (key, load) in l.iter() {
        if let Load::Found { then, .. } = load {
            let then = then.iter().filter(|n| defs.font_data.contains_key(*n));
            let mut list = vec![key.clone()];
            list.extend(then.cloned());
            let rest: Vec<String> = (fallback.iter())
                .filter(|n| !list.contains(n))
                .cloned()
                .collect();
            list.extend(rest);
            defs.families.insert(named(key), list);
        }
    }
    for (key, load) in l.iter() {
        if let Load::Found {
            interface: true, ..
        } = load
        {
            for list in defs.families.values_mut() {
                if !list.contains(key) {
                    list.push(key.clone());
                }
            }
        }
    }
    defs
}

/// Where finding the fonts the interface falls back to is noted among the loads.
const INTERFACE: &str = "interface fallback";

/// Characters the interface's own fonts lack, whose installed fonts it falls back to
/// (ENH-33): Chinese, Japanese and Korean, and an emoji.
const SAMPLES: &str = "漢あ한😀";

/// Largest font file read for the interface to fall back to: collections of every weight
/// of a Chinese, Japanese and Korean family run past a hundred megabytes, and the
/// interface then shows boxes for what they would draw.
const MAX_INTERFACE: u64 = 64 << 20;

/// Interface text falls back to installed fonts for what Inter and egui's own fonts can't
/// draw (ENH-33): the first call starts finding, on another thread, the faces LimarePDF
/// writes Chinese, Japanese and Korean text and an emoji in
/// ([`limarepdf_core::fallback_faces`]); each is added at the end of every font of the
/// interface once read, and serves typing in that face too. Called each frame.
pub fn interface_fallback(ctx: &egui::Context) {
    let loads = loads(ctx);
    let mut l = loads.lock().unwrap_or_else(|e| e.into_inner());
    if !l.contains_key(INTERFACE) {
        l.insert(INTERFACE.into(), Load::Looking);
        let (loads, ctx) = (loads.clone(), ctx.clone());
        std::thread::spawn(move || {
            for face in limarepdf_core::fallback_faces(SAMPLES) {
                let key = file_key(&face.path, face.index);
                {
                    // Already loaded for typing: the interface falls back to it too.
                    let mut l = loads.lock().unwrap_or_else(|e| e.into_inner());
                    if let Some(Load::Found {
                        interface, sent, ..
                    }) = l.get_mut(&key)
                    {
                        (*interface, *sent) = (true, false);
                        continue;
                    }
                }
                let small = std::fs::metadata(&face.path).is_ok_and(|m| m.len() <= MAX_INTERFACE);
                let Some(bytes) = small.then(|| std::fs::read(&face.path).ok()).flatten() else {
                    continue;
                };
                let data = FontData {
                    index: face.index,
                    ..FontData::from_owned(bytes)
                };
                let found = Load::Found {
                    data: Arc::new(data),
                    sent: false,
                    then: Vec::new(),
                    interface: true,
                };
                let mut l = loads.lock().unwrap_or_else(|e| e.into_inner());
                arrived(&mut l, key, found);
            }
            let done = Load::Missing { limited: false };
            (loads.lock().unwrap_or_else(|e| e.into_inner())).insert(INTERFACE.into(), done);
            ctx.request_repaint();
        });
        return;
    }
    if l.values()
        .any(|v| matches!(v, Load::Found { sent: false, .. }))
    {
        let defs = install(ctx, &mut l);
        drop(l);
        ctx.set_fonts(defs);
    }
}

/// For tests: the fonts text in the PDF font `name` is drawn in, the installed font of its
/// name and the bundled face of its kind, held back as if still being read on another
/// thread, so that text is drawn in Inter, as on a busy computer before they are in.
#[cfg(test)]
pub fn hold(ctx: &egui::Context, name: &str) {
    let loads = loads(ctx);
    let mut l = loads.lock().unwrap_or_else(|e| e.into_inner());
    let bundled = face(FontChoice::from_name(name)).to_string();
    for key in [installed_key(name), bundled] {
        l.insert(key, Load::Looking);
    }
}

/// For tests: the fonts [`hold`] held back for the PDF font `name` let in, read on another
/// thread the next time text in them is drawn.
#[cfg(test)]
pub fn let_in(ctx: &egui::Context, name: &str) {
    let loads = loads(ctx);
    let mut l = loads.lock().unwrap_or_else(|e| e.into_inner());
    let bundled = face(FontChoice::from_name(name)).to_string();
    for key in [installed_key(name), bundled] {
        if matches!(l.get(&key), Some(Load::Looking)) {
            l.remove(&key);
        }
    }
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
        let kept = candidates(Wanted::Kept("CourierNewPS-ItalicMT", None));
        assert_eq!(
            kept,
            [
                Candidate::Installed("CourierNewPS-ItalicMT".into()),
                Candidate::Bundled("Liberation Mono Italic"),
                Candidate::Inter,
            ]
        );

        // Each falls to the next that is ready.
        let w = Wanted::Kept("Arial,Bold", None);
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
        let w = Wanted::Kept("NoSuchFontAnywhere-BoldItalic", None);
        let stand_in = named("Liberation Sans Bold Italic");
        assert_eq!(settled(&ctx, w, &stand_in), stand_in);
        let done = Instant::now() + Duration::from_secs(60);
        loop {
            frame(&ctx);
            let loads = ctx.data_mut(|d| d.get_temp::<Loads>(egui::Id::unique("typing fonts")));
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
        let w = Wanted::Kept(name, None);
        let got = settled(&ctx, w, &named(&installed_key(name)));
        assert_eq!(got, named(&installed_key(name)));
        let list = ctx.fonts(|f| f.definitions().families[&got].clone());
        assert!(list.contains(&"Inter".to_string()));
    }

    /// A page with a line, "Walrus", in an embedded font of its own: the bundled Liberation
    /// Sans by its WinAnsi codes, with Helvetica's widths (which it shares); and the line's
    /// fonts.
    fn embedded_line() -> Vec<LineFont> {
        use lopdf::{dictionary, Dictionary, Object, Stream};
        let mut pdf = limarepdf_core::Pdf::new();
        pdf.insert_blank(0, 612.0, 792.0).unwrap();
        let program = bundled_face("Liberation Sans").unwrap().to_vec();
        let file = pdf.doc.add_object(Stream::new(Dictionary::new(), program));
        let widths: Vec<Object> = (32u8..=126)
            .map(|b| Object::Real(limarepdf_core::text_width(&(b as char).to_string(), 1000.0)))
            .collect();
        let descriptor = pdf.doc.add_object(dictionary! {
            "Type" => "FontDescriptor", "FontName" => "LiberationSans", "Flags" => 32,
            "FontBBox" => vec![(-203).into(), (-303).into(), 1050.into(), 910.into()],
            "ItalicAngle" => 0, "Ascent" => 905, "Descent" => -212, "CapHeight" => 729,
            "StemV" => 80, "FontFile2" => file,
        });
        let font = pdf.doc.add_object(dictionary! {
            "Type" => "Font", "Subtype" => "TrueType", "BaseFont" => "LiberationSans",
            "FirstChar" => 32, "LastChar" => 126, "Widths" => widths,
            "Encoding" => "WinAnsiEncoding", "FontDescriptor" => descriptor,
        });
        let content = b"BT /F1 20 Tf 72 700 Td (Walrus) Tj ET".to_vec();
        let s = pdf.doc.add_object(Stream::new(Dictionary::new(), content));
        let page = pdf.page_ids()[0];
        let d = pdf.doc.get_dictionary_mut(page).unwrap();
        d.set("Contents", Object::Reference(s));
        d.set(
            "Resources",
            dictionary! { "Font" => dictionary! { "F1" => font } },
        );
        let objects = pdf.page_objects(0).unwrap();
        let line = objects.iter().find(|o| o.text.is_some()).unwrap();
        pdf.line_fonts(0, line.id).unwrap()
    }

    /// A line in an embedded font is typed in that font's own glyphs: a font core makes of
    /// them, with the PDF's widths, so the caret goes where the text will be written; what
    /// it lacks is drawn in the Liberation face of its kind.
    #[test]
    fn a_kept_embedded_font_is_drawn_in_its_own_glyphs() {
        let fonts = embedded_line();
        let w = Wanted::Kept("LiberationSans", Some(&fonts[0]));
        let list = candidates(w);
        assert!(matches!(list[0], Candidate::Preview(_)), "{list:?}");
        assert_eq!(
            list[1..],
            [
                Candidate::Installed("LiberationSans".into()),
                Candidate::Bundled("Liberation Sans"),
                Candidate::Inter
            ]
        );
        let ctx = ctx();
        let want = named(&preview_key(fonts[0].preview_key().unwrap()));
        assert_eq!(settled(&ctx, w, &want), want);
        // Falling back to the face of its kind once that is in, then Inter.
        let done = Instant::now() + Duration::from_secs(60);
        let families = |ctx: &egui::Context| ctx.fonts(|f| f.definitions().families[&want].clone());
        while families(&ctx).get(1).map(String::as_str) != Some("Liberation Sans")
            && Instant::now() < done
        {
            frame(&ctx);
            family(&ctx, w);
            std::thread::sleep(Duration::from_millis(20));
        }
        let list = families(&ctx);
        assert_eq!(list[1], "Liberation Sans", "{list:?}");
        assert!(list.contains(&"Inter".to_string()));
        let width = |s: &str, f: FontFamily| {
            ctx.fonts_mut(|fonts| {
                let id = egui::FontId::new(100.0, f);
                let laid = fonts.layout_no_wrap(s.into(), id, egui::Color32::BLACK);
                laid.size().x
            })
        };
        // Its letters as wide as the PDF writes them: the line's own and others typed, each
        // its width (text kept in a PDF's own font isn't kerned anew).
        for s in ["Walrus", "Hello, World!"] {
            let one = |c: char| limarepdf_core::text_width(&c.to_string(), 100.0);
            let pdf: f32 = s.chars().map(one).sum();
            let drawn = width(s, want.clone());
            assert!((drawn - pdf).abs() < 0.01 * pdf, "{s}: {drawn} vs {pdf}");
        }
        // A letter the PDF's font has no code for (WinAnsi has no Ω): the face of its kind.
        let face = width("Ω", named("Liberation Sans"));
        assert!((width("Ω", want) - face).abs() < 0.01, "{face}");
    }

    /// A page with a line, "Walrus", in a font the PDF names but doesn't hold: `subtype` and
    /// `base` its `/Subtype` and `/BaseFont`, `desc` its font descriptor's entries; and the
    /// line's fonts.
    fn unembedded_line(subtype: &str, base: &str, desc: lopdf::Dictionary) -> Vec<LineFont> {
        use lopdf::{dictionary, Dictionary, Object, Stream};
        let mut pdf = limarepdf_core::Pdf::new();
        pdf.insert_blank(0, 612.0, 792.0).unwrap();
        let widths: Vec<Object> = (32u8..=126)
            .map(|b| Object::Real(limarepdf_core::text_width(&(b as char).to_string(), 1000.0)))
            .collect();
        let mut fd = dictionary! { "Type" => "FontDescriptor", "FontName" => base };
        fd.extend(&desc);
        let descriptor = pdf.doc.add_object(fd);
        let font = pdf.doc.add_object(dictionary! {
            "Type" => "Font", "Subtype" => subtype, "BaseFont" => base,
            "FirstChar" => 32, "LastChar" => 126, "Widths" => widths,
            "Encoding" => "WinAnsiEncoding", "FontDescriptor" => descriptor,
        });
        let content = b"BT /F1 20 Tf 72 700 Td (Walrus) Tj ET".to_vec();
        let s = pdf.doc.add_object(Stream::new(Dictionary::new(), content));
        let page = pdf.page_ids()[0];
        let d = pdf.doc.get_dictionary_mut(page).unwrap();
        d.set("Contents", Object::Reference(s));
        d.set(
            "Resources",
            dictionary! { "Font" => dictionary! { "F1" => font } },
        );
        let objects = pdf.page_objects(0).unwrap();
        let line = objects.iter().find(|o| o.text.is_some()).unwrap();
        pdf.line_fonts(0, line.id).unwrap()
    }

    /// A line in a font the PDF doesn't hold is typed in the face LimarePDF's pages draw it
    /// in once kept (ENH-25), not in the installed font of its name: the standard font the
    /// renderer takes for it, by its name, then by its descriptor, in its Liberation face.
    #[test]
    fn an_unembedded_kept_font_is_drawn_as_the_page_draws_it() {
        use lopdf::dictionary;
        let cases = [
            // A word in the name gives the family, the descriptor's weight bold and its
            // italic angle italic.
            (
                "TrueType",
                "Arial",
                dictionary! { "Flags" => 32 },
                "Liberation Sans",
            ),
            (
                "TrueType",
                "Arial",
                dictionary! { "Flags" => 32, "FontWeight" => 700 },
                "Liberation Sans Bold",
            ),
            (
                "TrueType",
                "ArialMT",
                dictionary! { "Flags" => 32, "ItalicAngle" => -12 },
                "Liberation Sans Italic",
            ),
            (
                "Type1",
                "Times-Roman",
                dictionary! { "Flags" => 34 },
                "Liberation Serif",
            ),
            (
                "TrueType",
                "CourierNewPS-BoldMT",
                dictionary! { "Flags" => 35 },
                "Liberation Mono Bold",
            ),
            // No such word: the serif flag gives the family, "Bold" and "Italic" in the name
            // the style; the weight and the fixed-pitch flag aren't read.
            (
                "TrueType",
                "Cambria",
                dictionary! { "Flags" => 34 },
                "Liberation Serif",
            ),
            (
                "TrueType",
                "Calibri",
                dictionary! { "Flags" => 32, "FontWeight" => 700 },
                "Liberation Sans",
            ),
            (
                "TrueType",
                "Georgia,BoldItalic",
                dictionary! { "Flags" => 34 },
                "Liberation Serif Bold Italic",
            ),
            (
                "TrueType",
                "Consolas",
                dictionary! { "Flags" => 33 },
                "Liberation Sans",
            ),
        ];
        for (subtype, base, desc, face) in cases {
            let fonts = unembedded_line(subtype, base, desc);
            assert_eq!(
                candidates(Wanted::Kept(base, Some(&fonts[0]))),
                [Candidate::Bundled(face), Candidate::Inter],
                "{base}"
            );
        }
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

    /// A face read for typing while the fonts read at start read it for the interface too
    /// (ENH-33) serves both, whichever read arrives last: the interface's text falls back to
    /// it, and typing's family to what typing asked. (Typing's read, arriving last, took it
    /// from the interface, whose Chinese, Japanese and Korean then showed as boxes for the
    /// rest of the run: when typing began before the fonts at start were read, which a cold
    /// disk cache made likely.)
    #[test]
    fn a_face_read_for_typing_and_the_interface_at_once_serves_both() {
        let ctx = ctx();
        let bytes = limarepdf_core::bundled_face("Liberation Serif").expect("a face");
        let key = "file both";
        let then = ["Liberation Sans".to_string()];
        let for_interface = || Load::Found {
            data: Arc::new(FontData::from_static(bytes)),
            sent: false,
            then: Vec::new(),
            interface: true,
        };
        // Typing asks first: the file is read on another thread, not in yet.
        let (go, wait) = std::sync::mpsc::channel::<()>();
        let fetch = move || {
            wait.recv().ok()?;
            Some((Cow::Borrowed(bytes), 0))
        };
        assert!(!loaded(&ctx, key, &["Liberation Sans"], None, fetch));
        // Meanwhile the fonts read at start read it for the interface.
        let loads = loads(&ctx);
        arrived(&mut loads.lock().unwrap(), key.into(), for_interface());
        go.send(()).unwrap();
        let until = Instant::now() + Duration::from_secs(10);
        let typed = loop {
            let l = loads.lock().unwrap();
            if matches!(l.get(key), Some(Load::Found { then: t, .. }) if !t.is_empty()) {
                break l;
            }
            assert!(Instant::now() < until, "typing's read never arrived");
            drop(l);
            std::thread::sleep(Duration::from_millis(5));
        };
        let both = |l: &HashMap<String, Load>| match l.get(key) {
            Some(Load::Found {
                then: t, interface, ..
            }) => *interface && t[..] == then,
            _ => false,
        };
        assert!(both(&typed), "typing's read took it from the interface");
        drop(typed);
        // The other way round: the interface's read arriving last keeps typing's fallbacks.
        arrived(&mut loads.lock().unwrap(), key.into(), for_interface());
        assert!(both(&loads.lock().unwrap()), "typing's fallbacks lost");
        // Handed to egui, every interface family falls back to it.
        loaded(&ctx, key, &[], None, || None);
        let families = ctx.fonts(|f| f.definitions().families.clone());
        assert!(families[&FontFamily::Proportional].contains(&key.to_string()));
    }
}

//! The inspector's font list (the Unicode text design, section 7): Helvetica, Times and
//! Courier, then the installed families LimarePDF can embed, each name drawn in its own
//! font, with a search field; and the Bold and Italic buttons, which a family lacking that
//! face leaves off. A font installed under a code name is listed by the typeface it stands
//! in for, its own name after (ENH-04): "Palatino (P052)".
//!
//! The installed fonts are read once, on another thread, when the app starts ([`warm`]);
//! until then the list holds the three standard fonts.

use crate::{dialogs, typing_fonts};
use eframe::egui::{self, PopupCloseBehavior};
use limarepdf_core::{Family, FontChoice, Typeface};
use std::sync::Arc;

/// Starts reading the installed fonts on another thread, so the list has them when it is
/// first opened; a repaint follows.
pub fn warm(ctx: &egui::Context) {
    let ctx = ctx.clone();
    std::thread::spawn(move || {
        limarepdf_core::typefaces();
        ctx.request_repaint();
    });
}

/// A family in the list: its name, and an installed one's faces.
type Offer = (Family, String, Option<Arc<Typeface>>);

/// The families the list offers, standard ones first, with `search` in their names as the
/// list shows them (any case; the familiar name or a code name), and whether the installed
/// ones are still being read.
fn offered(search: &str) -> (Vec<Offer>, bool) {
    let search = search.trim().to_lowercase();
    let fits = |name: &str| name.to_lowercase().contains(&search);
    let mut out: Vec<_> = [Family::Helvetica, Family::Times, Family::Courier]
        .into_iter()
        .map(|family| {
            let f = FontChoice {
                family,
                ..Default::default()
            };
            (family, f.family_name(), None)
        })
        .filter(|(_, name, _)| fits(name))
        .collect();
    let ready = limarepdf_core::typefaces_ready();
    if ready {
        out.extend(
            limarepdf_core::typefaces()
                .into_iter()
                .filter_map(|(n, t)| {
                    let label = t.label();
                    fits(&label).then_some((Family::Installed(n), label, Some(t)))
                }),
        );
    }
    (out, !ready)
}

/// What the pointer on a family in the list, or on the list for text in it, says of it:
/// a standard font's look varies by app; a font with a code name is a stand-in, embedded.
fn about(family: Family) -> Option<String> {
    match family {
        Family::Installed(n) => {
            let t = limarepdf_core::typeface(n)?;
            let familiar = t.familiar()?;
            Some(format!(
                "{}: made to match {familiar}'s letter widths, not {familiar} itself. \
                 It is embedded in the PDF, so the text looks the same everywhere.",
                t.name
            ))
        }
        _ => Some("A standard PDF font: other apps may show it in their own version.".into()),
    }
}

/// `f` set in `family`, in the style nearest its own that the family has.
pub fn choose(f: &mut FontChoice, family: Family) {
    f.family = family;
    if let Some(t) = f.typeface() {
        (f.bold, f.italic) = t.nearest(f.bold, f.italic);
    }
}

/// The font list for the font `f`, across the space there is; with `blank`, it shows no
/// name, as for text in several families. The pointer on it says what the family is
/// ([`about`]) and, for text already in the PDF, the PDF's names for its fonts (`in_pdf`).
/// Returns whether a family was picked in the list, even `f`'s own.
pub fn ui(ui: &mut egui::Ui, f: &mut FontChoice, blank: bool, in_pdf: &[String]) -> bool {
    let id = ui.scope_id().with("fam");
    let search_id = id.with("search");
    let (name, mut hover) = if blank {
        (String::new(), Vec::new())
    } else {
        (f.family_name(), about(f.family).into_iter().collect())
    };
    if !in_pdf.is_empty() {
        hover.push(format!("In the PDF: {}", in_pdf.join(", ")));
    }
    let r = egui::ComboBox::from_id_salt(id)
        .selected_text(name)
        .width(ui.available_width())
        .height(320.0)
        .close_behavior(PopupCloseBehavior::CloseOnClickOutside)
        .show_ui(ui, |ui| list(ui, search_id, f, blank));
    dialogs::set_name(&r.response, "Font");
    if !hover.is_empty() {
        r.response.on_hover_text(hover.join("\n"));
    }
    if r.inner.is_none() {
        // Closed: the next opening starts with an empty search.
        ui.data_mut(|d| d.remove::<String>(search_id));
    }
    r.inner.unwrap_or(false)
}

/// The Bold and Italic buttons for the font `f`, off for a face its family lacks: a heavy
/// B and a slanted I with serifs.
pub fn styles(ui: &mut egui::Ui, f: &mut FontChoice) {
    let t = f.typeface();
    let has = |bold: bool, italic: bool| t.as_ref().is_none_or(|t| t.has(bold, italic));
    let mut bold = f.bold;
    let size = ui.style().text_styles[&egui::TextStyle::Button].size;
    // Outlined squares, as word processors' are.
    let square = |b: egui::Button<'static>, on: bool| {
        b.selected(on)
            .frame_when_inactive(true)
            .min_size(egui::vec2(30.0, 28.0))
    };
    let b = egui::RichText::new("B").family(crate::theme::semibold());
    let b = square(egui::Button::new(b), bold);
    let r = ui.add_enabled(has(!f.bold, f.italic), b);
    if r.clicked() {
        bold = !bold;
    }
    dialogs::set_name(&r, "Bold");
    // A slanted I with serifs, as word processors show it: the interface font's I,
    // slanted, reads as a slash. The icon is drawn inside some padding, so a little larger
    // to stand as tall as the B.
    let i = crate::theme::icon(crate::icon::TEXT_ITALIC).size(size + 3.0);
    let i = square(egui::Button::new(i), f.italic);
    let r = ui.add_enabled(has(bold, !f.italic), i);
    if r.clicked() {
        f.italic = !f.italic;
    }
    dialogs::set_name(&r, "Italic");
    f.bold = bold;
}

/// The open list: the search field, then the families that fit it, `f`'s marked unless
/// `blank`. Enter picks the first. Returns whether one was picked.
fn list(ui: &mut egui::Ui, search_id: egui::Id, f: &mut FontChoice, blank: bool) -> bool {
    let ctx = ui.ctx().clone();
    let mut search = ui.data(|d| d.get_temp::<String>(search_id));
    let fresh = search.is_none();
    let text = search.get_or_insert_with(String::new);
    let r = ui.add(
        egui::TextEdit::singleline(text)
            .hint_text("Search")
            .desired_width(f32::INFINITY),
    );
    dialogs::set_name(&r, "Search fonts");
    if fresh {
        r.request_focus();
    }
    let enter = r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
    let (families, reading) = offered(text);
    ui.data_mut(|d| d.insert_temp(search_id, search.clone().unwrap_or_default()));
    if enter {
        if let Some((family, _, _)) = families.first() {
            choose(f, *family);
            ui.close();
            return true;
        }
    }
    let mut picked = false;
    egui::ScrollArea::vertical()
        .max_height(280.0)
        .auto_shrink([false, true])
        .show(ui, |ui| {
            let row = ui.spacing().interact_size.y;
            for (i, (family, name, t)) in families.iter().enumerate() {
                if i > 0 && t.is_some() && families[i - 1].2.is_none() {
                    ui.separator();
                }
                // Each name in its own font, loaded once its row is in view.
                let at = egui::Rect::from_min_size(
                    ui.cursor().min,
                    egui::vec2(ui.available_width(), row),
                );
                let face = t.as_ref().and_then(|t| {
                    let (bold, italic) = t.nearest(false, false);
                    t.face(bold, italic).cloned()
                });
                let drawn = face
                    .filter(|_| ui.is_rect_visible(at))
                    .and_then(|face| Some((typing_fonts::preview(&ctx, &face)?, face)));
                let label: egui::WidgetText = match drawn {
                    Some((family, face)) => {
                        let format = egui::TextFormat {
                            font_id: egui::FontId::new(
                                ui.style().text_styles[&egui::TextStyle::Button].size + 2.0,
                                family,
                            ),
                            coords: egui::epaint::text::VariationCoords::new(face.coords),
                            color: ui.visuals().text_color(),
                            ..Default::default()
                        };
                        egui::text::LayoutJob::single_section(name.clone(), format).into()
                    }
                    None => name.clone().into(),
                };
                let selected = !blank && f.family == *family;
                let mut r = ui.add(egui::Button::selectable(selected, label));
                if let Some(about) = about(*family) {
                    r = r.on_hover_text(about);
                }
                if selected && fresh {
                    r.scroll_to_me(Some(egui::Align::Center));
                }
                if r.clicked() {
                    choose(f, *family);
                    picked = true;
                    ui.close();
                }
            }
            if reading {
                ui.weak("Finding installed fonts…");
            }
        });
    picked
}

/// A test font as an installed family, by name: Source Serif 4 (OpenType CFF, regular
/// only), Cantarell (variable, upright), Caveat (variable, weights 400 to 700, no Greek) or
/// Limare Emoji Test (made up: Latin letters and a few emoji), from core's test fonts.
#[cfg(test)]
pub fn test_family(name: &str) -> Family {
    static NUMBERS: std::sync::OnceLock<Vec<u16>> = std::sync::OnceLock::new();
    let numbers = NUMBERS.get_or_init(|| {
        let core = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../limarepdf-core");
        let files = [
            core.join("tests/data/fonts/SourceSerif4-Regular.otf"),
            core.join("tests/data/fonts/Cantarell-VF.otf"),
            core.join("assets/fonts/Caveat.ttf"),
            core.join("tests/data/fonts/LimareEmojiTest.ttf"),
        ];
        limarepdf_core::add_font_files(&files.iter().map(|p| p.as_path()).collect::<Vec<_>>())
    });
    let n = numbers
        .iter()
        .copied()
        .find(|&n| limarepdf_core::typeface(n).is_some_and(|t| t.name == name))
        .unwrap_or_else(|| panic!("{name} among the test fonts"));
    Family::Installed(n)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn standard_fonts_come_first_and_search_narrows_the_list() {
        let (all, _) = offered("");
        let names: Vec<&str> = all.iter().take(3).map(|(_, n, _)| n.as_str()).collect();
        assert_eq!(names, ["Helvetica", "Times", "Courier"]);
        let (times, _) = offered(" tIMes ");
        assert!(times.iter().any(|(f, _, _)| *f == Family::Times));
        assert!(!times.iter().any(|(f, _, _)| *f == Family::Helvetica));
        let (none, _) = offered("no font is called this");
        assert!(none.is_empty());
    }

    /// Italic shows a slanted capital I with serifs, as word processors do (the interface
    /// font's I slanted read as a slash); Bold a B in the semibold face (egui's strong text
    /// was only brighter). Screen readers still say Italic and Bold.
    #[test]
    fn italic_shows_a_slanted_i_with_serifs() {
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        crate::theme::install(&ctx);
        let mut f = FontChoice::default();
        let mut out = ctx.run_ui(egui::RawInput::default(), |ui| {
            ui.horizontal(|ui| styles(ui, &mut f));
        });
        out.textures_delta.clear();
        let painted: Vec<(String, egui::FontFamily)> = out
            .shapes
            .iter()
            .filter_map(|s| match &s.shape {
                egui::Shape::Text(t) => {
                    let family = t.galley.job.sections.first()?.format.font_id.family.clone();
                    Some((t.galley.text().to_string(), family))
                }
                _ => None,
            })
            .collect();
        let italic = (crate::icon::TEXT_ITALIC.to_string(), crate::theme::icons());
        assert!(painted.contains(&italic), "{painted:?}");
        // egui's strong text is only brighter: the B is in the semibold face.
        let bold = ("B".to_string(), crate::theme::semibold());
        assert!(painted.contains(&bold), "{painted:?}");
        let tree = out.platform_output.accesskit_update.expect("the buttons");
        for name in ["Bold", "Italic"] {
            assert!(
                tree.nodes.iter().any(|(_, n)| n.label() == Some(name)),
                "{name}"
            );
        }
    }

    #[test]
    fn a_family_is_chosen_in_a_style_it_has() {
        let mut f = FontChoice {
            family: Family::Times,
            bold: true,
            italic: true,
        };
        choose(&mut f, test_family("Source Serif 4"));
        assert_eq!((f.bold, f.italic), (false, false), "only a regular face");
        choose(&mut f, test_family("Caveat"));
        assert_eq!((f.bold, f.italic), (false, false));
        f.bold = true;
        choose(&mut f, test_family("Cantarell"));
        assert_eq!((f.bold, f.italic), (true, false), "bold kept");
        choose(&mut f, Family::Courier);
        assert_eq!(f.family, Family::Courier);
    }
}

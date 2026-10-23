//! The inspector's font list (the Unicode text design, section 7): Helvetica, Times and
//! Courier, then the installed families LimarePDF can embed, each name drawn in its own
//! font, with a search field; and the Bold and Italic buttons, which a family lacking that
//! face leaves off.
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

/// The families the list offers, standard ones first, with `search` in their names (any
/// case), and whether the installed ones are still being read.
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
                .filter(|(_, t)| fits(&t.name))
                .map(|(n, t)| (Family::Installed(n), t.name.clone(), Some(t))),
        );
    }
    (out, !ready)
}

/// `f` set in `family`, in the style nearest its own that the family has.
fn choose(f: &mut FontChoice, family: Family) {
    f.family = family;
    if let Some(t) = f.typeface() {
        (f.bold, f.italic) = t.nearest(f.bold, f.italic);
    }
}

/// The font list and the Bold and Italic buttons, for the font `f`.
pub fn ui(ui: &mut egui::Ui, f: &mut FontChoice) {
    let id = ui.id().with("fam");
    let search_id = id.with("search");
    let r = egui::ComboBox::from_id_salt(id)
        .selected_text(f.family_name())
        .height(320.0)
        .close_behavior(PopupCloseBehavior::CloseOnClickOutside)
        .show_ui(ui, |ui| list(ui, search_id, f));
    dialogs::set_name(&r.response, "Font");
    if r.inner.is_none() {
        // Closed: the next opening starts with an empty search.
        ui.data_mut(|d| d.remove::<String>(search_id));
    }
    let t = f.typeface();
    let has = |bold: bool, italic: bool| t.as_ref().is_none_or(|t| t.has(bold, italic));
    ui.horizontal(|ui| {
        let mut bold = f.bold;
        let b = egui::Button::selectable(bold, egui::RichText::new("B").strong());
        let r = ui.add_enabled(has(!f.bold, f.italic), b);
        if r.clicked() {
            bold = !bold;
        }
        dialogs::set_name(&r, "Bold");
        let i = egui::Button::selectable(f.italic, egui::RichText::new("I").italics());
        let r = ui.add_enabled(has(bold, !f.italic), i);
        if r.clicked() {
            f.italic = !f.italic;
        }
        dialogs::set_name(&r, "Italic");
        f.bold = bold;
    });
}

/// The open list: the search field, then the families that fit it. Enter picks the first.
fn list(ui: &mut egui::Ui, search_id: egui::Id, f: &mut FontChoice) {
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
            return;
        }
    }
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
                let selected = f.family == *family;
                let r = ui.add(egui::Button::selectable(selected, label));
                if selected && fresh {
                    r.scroll_to_me(Some(egui::Align::Center));
                }
                if r.clicked() {
                    choose(f, *family);
                    ui.close();
                }
            }
            if reading {
                ui.weak("Finding installed fonts…");
            }
        });
}

/// A test font as an installed family, by name: Source Serif 4 (OpenType CFF, regular
/// only), Cantarell (variable, upright) or Caveat (variable, weights 400 to 700, no Greek),
/// from core's test fonts.
#[cfg(test)]
pub fn test_family(name: &str) -> Family {
    static NUMBERS: std::sync::OnceLock<Vec<u16>> = std::sync::OnceLock::new();
    let numbers = NUMBERS.get_or_init(|| {
        let core = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../limarepdf-core");
        let files = [
            core.join("tests/data/fonts/SourceSerif4-Regular.otf"),
            core.join("tests/data/fonts/Cantarell-VF.otf"),
            core.join("assets/fonts/Caveat.ttf"),
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

//! The About window: version, "Made in Canada", links, contributors and credits.

use crate::theme;
use eframe::egui::{self, Align, Align2, Context, Layout, TextureHandle, TextureOptions, Ui};
use egui_phosphor::regular as icon;

const WEBSITE: &str = "https://github.com/smaug42/limarepdf";
const LICENCE: &str = "https://github.com/smaug42/limarepdf/blob/main/docs/licence.md";
const ISSUES: &str = "https://github.com/smaug42/limarepdf/issues";

/// The people who made LimarePDF, and what they did.
const CONTRIBUTORS: &[(&str, &str)] = &[
    ("Clayton Cornell", "Developer"),
    ("Dean Campbell", "Tester"),
];

/// Which page of the About window is shown.
#[derive(Clone, Copy, PartialEq, Eq, Default, Debug)]
pub enum Page {
    #[default]
    Main,
    Contributors,
}

/// The version line copied for bug reports.
fn version_text() -> String {
    format!(
        "LimarePDF {} on {} ({})",
        env!("CARGO_PKG_VERSION"),
        std::env::consts::OS,
        std::env::consts::ARCH
    )
}

/// A picture built into the app, made into a texture once and kept in egui's memory.
fn texture(ctx: &Context, name: &str, png: &'static [u8]) -> Option<TextureHandle> {
    let id = egui::Id::new(("about-texture", name));
    if let Some(t) = ctx.data(|d| d.get_temp::<TextureHandle>(id)) {
        return Some(t);
    }
    let data = eframe::icon_data::from_png_bytes(png).ok()?;
    let size = [data.width as usize, data.height as usize];
    let img = egui::ColorImage::from_rgba_unmultiplied(size, &data.rgba);
    // Mipmaps keep the pictures smooth when drawn much smaller than they are.
    let options = TextureOptions {
        mipmap_mode: Some(egui::TextureFilter::Linear),
        ..TextureOptions::LINEAR
    };
    let t = ctx.load_texture(name, img, options);
    ctx.data_mut(|d| d.insert_temp(id, t.clone()));
    Some(t)
}

/// The red maple leaf of the flag of Canada.
pub fn leaf(ctx: &Context) -> Option<TextureHandle> {
    texture(
        ctx,
        "maple-leaf",
        include_bytes!("../assets/maple-leaf.png"),
    )
}

fn app_icon(ctx: &Context) -> Option<TextureHandle> {
    texture(
        ctx,
        "app-icon",
        include_bytes!("../../../docs/images/icon.png"),
    )
}

/// A row of widgets centred in the width available. egui lays a row out from the left,
/// so the row's width from the last frame sets the space before it.
pub fn centred_row(ui: &mut Ui, id: &str, add: impl FnOnce(&mut Ui)) {
    let id = ui.id().with(id);
    let last: Option<f32> = ui.data(|d| d.get_temp(id));
    ui.horizontal(|ui| {
        let room = ui.available_width();
        ui.add_space(((room - last.unwrap_or(room)) / 2.0).max(0.0));
        let start = ui.cursor().min.x;
        add(ui);
        let width = ui.min_rect().right() - start;
        if last.is_none_or(|w| (w - width).abs() > 0.5) {
            ui.data_mut(|d| d.insert_temp(id, width));
            ui.ctx().request_discard("centred row width");
        }
    });
}

/// The leaf and "Made in Canada": quiet and the leaf the height of the text (`weak`), or
/// in the body colour with a larger leaf.
pub fn made_in_canada(ui: &mut Ui, id: &str, weak: bool) {
    let leaf = leaf(ui.ctx());
    centred_row(ui, id, |ui| {
        ui.spacing_mut().item_spacing.x = 6.0;
        if let Some(leaf) = leaf {
            let h = ui.text_style_height(&egui::TextStyle::Body);
            let h = if weak { h } else { h * 1.5 };
            ui.add(egui::Image::new((leaf.id(), egui::vec2(h, h))));
        }
        if weak {
            ui.weak("Made in Canada");
        } else {
            ui.label("Made in Canada");
        }
    });
}

/// Show the About window; true once it is closed.
pub fn show(ctx: &Context, page: &mut Page) -> bool {
    let mut open = true;
    let mut close = ctx.input(|i| i.key_pressed(egui::Key::Escape));
    egui::Window::new("About LimarePDF")
        .collapsible(false)
        .resizable(false)
        .anchor(Align2::CENTER_CENTER, [0.0, 0.0])
        .fixed_size([380.0, 0.0])
        .open(&mut open)
        .show(ctx, |ui| {
            ui.with_layout(Layout::top_down(Align::Center), |ui| match page {
                Page::Main => main_page(ui, page),
                Page::Contributors => contributors_page(ui),
            });
            ui.add_space(8.0);
            ui.separator();
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if ui.button("Close").clicked() {
                    close = true;
                }
                if *page == Page::Contributors && ui.button("Back").clicked() {
                    *page = Page::Main;
                }
            });
        });
    close || !open
}

fn main_page(ui: &mut Ui, page: &mut Page) {
    ui.add_space(4.0);
    if let Some(t) = app_icon(ui.ctx()) {
        ui.add(egui::Image::new((t.id(), egui::vec2(64.0, 64.0))));
    }
    ui.add_space(4.0);
    ui.label(
        egui::RichText::new("LimarePDF")
            .family(theme::semibold())
            .size(22.0),
    );
    centred_row(ui, "about-version", |ui| {
        ui.label(format!("Version {}", env!("CARGO_PKG_VERSION")));
        let copy = egui::Button::new(theme::icon(icon::COPY).size(14.0)).frame_when_inactive(false);
        if ui
            .add(copy)
            .on_hover_text("Copy the version and system, for a bug report")
            .clicked()
        {
            ui.ctx().copy_text(version_text());
        }
    });
    ui.label("A fast, pure-Rust PDF viewer and editor");
    ui.add_space(12.0);
    made_in_canada(ui, "about-canada", false);
    ui.add_space(12.0);
    centred_row(ui, "about-links", |ui| {
        let link = |ui: &mut Ui, text: &str, url: &str| {
            if ui.link(text).on_hover_text(url).clicked() {
                ui.ctx().open_url(egui::OpenUrl::new_tab(url));
            }
        };
        link(ui, "Website", WEBSITE);
        ui.weak("·");
        link(ui, "Licence", LICENCE);
        ui.weak("·");
        if ui.link("Contributors").clicked() {
            *page = Page::Contributors;
        }
        ui.weak("·");
        link(ui, "Report a problem", ISSUES);
    });
    ui.add_space(8.0);
    ui.separator();
    ui.with_layout(Layout::top_down(Align::Min), credits);
}

fn credits(ui: &mut Ui) {
    egui::CollapsingHeader::new("Credits")
        .default_open(false)
        .show(ui, |ui| {
            let row = |ui: &mut Ui, what: &str, who: &str| {
                ui.horizontal_wrapped(|ui| {
                    ui.label(egui::RichText::new(what).family(theme::semibold()));
                    ui.label(who);
                });
            };
            row(ui, "Rendering:", "hayro");
            row(ui, "Documents:", "lopdf");
            row(ui, "Interface:", "egui");
            row(
                ui,
                "Text recognition model:",
                "ocrs by Robert Knight, CC BY-SA 4.0, included unmodified",
            );
            row(
                ui,
                "Fonts:",
                "Inter; Caveat, Cedarville Cursive and Zeyada (SIL Open Font License 1.1); \
                 Homemade Apple (Apache License 2.0); all included unmodified",
            );
            row(ui, "Icons:", "Phosphor Icons (MIT)");
            ui.add_space(4.0);
            ui.weak("LimarePDF is licensed under MIT or Apache-2.0, at your option.");
        });
}

fn contributors_page(ui: &mut Ui) {
    ui.add_space(4.0);
    ui.label(
        egui::RichText::new("Contributors")
            .family(theme::semibold())
            .size(18.0),
    );
    ui.add_space(8.0);
    centred_row(ui, "about-contributors", |ui| {
        egui::Grid::new("about-contributors")
            .num_columns(2)
            .spacing([24.0, 6.0])
            .show(ui, |ui| {
                for (name, role) in CONTRIBUTORS {
                    ui.label(egui::RichText::new(*name).family(theme::semibold()));
                    ui.weak(*role);
                    ui.end_row();
                }
            });
    });
    ui.add_space(4.0);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pictures_decode() {
        for png in [
            &include_bytes!("../assets/maple-leaf.png")[..],
            &include_bytes!("../../../docs/images/icon.png")[..],
        ] {
            let d = eframe::icon_data::from_png_bytes(png).unwrap();
            assert_eq!(d.rgba.len(), (d.width * d.height * 4) as usize);
        }
    }

    #[test]
    fn version_names_the_system() {
        let v = version_text();
        assert!(v.contains(env!("CARGO_PKG_VERSION")));
        assert!(v.contains(std::env::consts::OS));
    }
}

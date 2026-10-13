//! The look of the app: fonts, colours, spacing and corner radii, for light and dark
//! mode. Light or dark follows the system unless chosen in View > Theme.
//!
//! Spacing uses a 4/8 px scale, corners are 6 px (8 px on windows), widgets are flat (no
//! borders), and there is one accent colour.

use eframe::egui::{
    self, Color32, CornerRadius, FontData, FontDefinitions, FontFamily, FontId, Margin, RichText,
    Shadow, Stroke, Style, TextStyle, Theme, Vec2,
};

/// Corner radius of buttons, fields and menus.
pub const RADIUS: u8 = 6;
/// Corner radius of windows and popups.
pub const WINDOW_RADIUS: u8 = 8;

/// Colours for one theme.
struct Palette {
    /// The accent: selection, focus, links, the active tool.
    accent: Color32,
    /// Background of selected items: the accent, faint.
    accent_bg: Color32,
    /// Panels and bars.
    panel: Color32,
    /// Windows, popups and text fields.
    surface: Color32,
    /// Background behind the pages.
    canvas: Color32,
    /// Buttons and other widgets at rest, hovered and pressed.
    widget: Color32,
    widget_hover: Color32,
    widget_active: Color32,
    /// Separators and window outlines.
    border: Color32,
    text: Color32,
    text_strong: Color32,
    shadow: Color32,
}

const LIGHT: Palette = Palette {
    accent: Color32::from_rgb(37, 99, 235),
    accent_bg: Color32::from_rgb(219, 231, 253),
    panel: Color32::from_rgb(250, 250, 251),
    surface: Color32::from_rgb(255, 255, 255),
    canvas: Color32::from_rgb(232, 233, 236),
    widget: Color32::from_rgb(241, 242, 244),
    widget_hover: Color32::from_rgb(230, 232, 236),
    widget_active: Color32::from_rgb(218, 221, 227),
    border: Color32::from_rgb(222, 224, 228),
    text: Color32::from_rgb(55, 58, 64),
    text_strong: Color32::from_rgb(20, 22, 26),
    shadow: Color32::from_black_alpha(28),
};

const DARK: Palette = Palette {
    accent: Color32::from_rgb(96, 152, 250),
    accent_bg: Color32::from_rgb(33, 52, 92),
    panel: Color32::from_rgb(32, 33, 36),
    surface: Color32::from_rgb(40, 41, 45),
    canvas: Color32::from_rgb(22, 23, 25),
    widget: Color32::from_rgb(46, 48, 52),
    widget_hover: Color32::from_rgb(58, 60, 65),
    widget_active: Color32::from_rgb(70, 72, 78),
    border: Color32::from_rgb(56, 58, 63),
    text: Color32::from_rgb(208, 210, 214),
    text_strong: Color32::from_rgb(240, 241, 243),
    shadow: Color32::from_black_alpha(110),
};

fn palette(dark: bool) -> &'static Palette {
    if dark {
        &DARK
    } else {
        &LIGHT
    }
}

/// Font family for headings and the document name (Inter SemiBold).
pub fn semibold() -> FontFamily {
    FontFamily::Name("semibold".into())
}

/// Font family for icons: Phosphor first. Inter has its own glyphs at many of the
/// private-use code points Phosphor uses, so icons in the text family can come out as
/// Inter's letters. Use this family for icon glyphs only: Phosphor also maps a–z to
/// blank glyphs.
pub fn icons() -> FontFamily {
    FontFamily::Name("icons".into())
}

/// An icon glyph in the icon family.
pub fn icon(glyph: &str) -> RichText {
    RichText::new(glyph).family(icons())
}

/// Install the fonts and the light and dark styles.
pub fn install(ctx: &egui::Context) {
    ctx.set_fonts(fonts());
    ctx.set_style_of(Theme::Light, style(false));
    ctx.set_style_of(Theme::Dark, style(true));
}

/// Inter for text, then egui's own fonts as fallbacks (symbols, emoji), with the
/// Phosphor icons after Inter.
fn fonts() -> FontDefinitions {
    let mut f = FontDefinitions::default();
    let regular = include_bytes!("../assets/fonts/Inter-Regular.ttf");
    let semi = include_bytes!("../assets/fonts/Inter-SemiBold.ttf");
    f.font_data
        .insert("Inter".into(), FontData::from_static(regular).into());
    f.font_data
        .insert("Inter-SemiBold".into(), FontData::from_static(semi).into());
    f.families
        .entry(FontFamily::Proportional)
        .or_default()
        .insert(0, "Inter".into());
    egui_phosphor::add_to_fonts(&mut f, egui_phosphor::Variant::Regular);
    // The same list with SemiBold in front, so icons and symbols still resolve.
    let mut bold = f.families[&FontFamily::Proportional].clone();
    bold.insert(0, "Inter-SemiBold".into());
    f.families.insert(semibold(), bold);
    let mut icons_list = f.families[&FontFamily::Proportional].clone();
    icons_list.retain(|n| n != "phosphor");
    icons_list.insert(0, "phosphor".into());
    f.families.insert(icons(), icons_list);
    f
}

fn style(dark: bool) -> Style {
    let p = palette(dark);
    let theme = if dark { Theme::Dark } else { Theme::Light };
    let mut s = theme.default_style();
    s.animation_time = 0.12;
    let text = |size| FontId::new(size, FontFamily::Proportional);
    s.text_styles = [
        (TextStyle::Small, text(11.0)),
        (TextStyle::Body, text(13.5)),
        (TextStyle::Button, text(13.5)),
        (TextStyle::Heading, FontId::new(16.0, semibold())),
        (TextStyle::Monospace, FontId::monospace(12.5)),
    ]
    .into();

    let sp = &mut s.spacing;
    sp.item_spacing = Vec2::new(8.0, 4.0);
    sp.button_padding = Vec2::new(8.0, 4.0);
    sp.interact_size.y = 24.0;
    sp.menu_margin = Margin::same(4);
    sp.window_margin = Margin::same(16);
    sp.indent = 16.0;

    let v = &mut s.visuals;
    let radius = CornerRadius::same(RADIUS);
    let w = &mut v.widgets;
    // Flat: no outlines on widgets, no growth on hover.
    for (wv, fill) in [
        (&mut w.noninteractive, p.panel),
        (&mut w.inactive, p.widget),
        (&mut w.hovered, p.widget_hover),
        (&mut w.active, p.widget_active),
        (&mut w.open, p.widget_hover),
    ] {
        wv.bg_fill = fill;
        wv.weak_bg_fill = fill;
        wv.bg_stroke = Stroke::NONE;
        wv.corner_radius = radius;
        wv.expansion = 0.0;
        wv.fg_stroke = Stroke::new(1.0, p.text);
    }
    // Separators and group outlines are drawn with this stroke.
    w.noninteractive.bg_stroke = Stroke::new(1.0, p.border);
    w.hovered.fg_stroke = Stroke::new(1.0, p.text_strong);
    w.active.fg_stroke = Stroke::new(1.0, p.text_strong);
    // Selected items (the active tool, a chosen tab) and focused text fields.
    v.selection.bg_fill = p.accent_bg;
    v.selection.stroke = Stroke::new(1.0, p.accent);
    v.text_cursor.stroke = Stroke::new(2.0, p.accent);
    v.hyperlink_color = p.accent;

    v.panel_fill = p.panel;
    v.window_fill = p.surface;
    v.extreme_bg_color = p.surface;
    v.faint_bg_color = p.widget;
    v.code_bg_color = p.widget;
    v.window_stroke = Stroke::new(1.0, p.border);
    v.window_corner_radius = CornerRadius::same(WINDOW_RADIUS);
    v.menu_corner_radius = CornerRadius::same(WINDOW_RADIUS);
    v.window_shadow = Shadow {
        offset: [0, 8],
        blur: 24,
        spread: 0,
        color: p.shadow,
    };
    v.popup_shadow = Shadow {
        offset: [0, 4],
        blur: 12,
        spread: 0,
        color: p.shadow,
    };
    v.slider_trailing_fill = true;
    s
}

/// Background behind the pages.
pub fn canvas(dark: bool) -> Color32 {
    palette(dark).canvas
}

/// The soft shadow under a page.
pub fn page_shadow(dark: bool) -> Shadow {
    Shadow {
        offset: [0, 2],
        blur: 12,
        spread: 0,
        color: Color32::from_black_alpha(if dark { 140 } else { 45 }),
    }
}

/// Frame for the bars at the top and bottom of the window.
pub fn bar_frame(style: &Style) -> egui::Frame {
    egui::Frame::side_top_panel(style).inner_margin(Margin::symmetric(8, 4))
}

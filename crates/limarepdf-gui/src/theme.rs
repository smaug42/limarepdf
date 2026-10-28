//! The look of the app: fonts, colours, spacing and corner radii, for light and dark
//! mode. Light or dark follows the system unless chosen in View > Theme.
//!
//! Spacing uses a 4/8 px scale, corners are 6 px (8 px on windows), widgets are flat (no
//! borders; dialogs outline theirs, [`dialog_style`]), and there is one accent colour: the
//! desktop's where it says one and the user follows it (View > Accent colour), else
//! LimarePDF's teal ([`set_accent`], `accent.rs`).

use eframe::egui::{
    self, Color32, CornerRadius, FontData, FontDefinitions, FontFamily, FontId, Margin, RichText,
    Shadow, Stroke, Style, TextStyle, Theme, Vec2,
};

/// Corner radius of buttons, fields and menus.
pub const RADIUS: u8 = 6;
/// Corner radius of windows and popups.
pub const WINDOW_RADIUS: u8 = 8;

/// Colours for one theme.
#[derive(Clone, Copy)]
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
    /// Outlines of fields and buttons in dialogs.
    edge: Color32,
    text: Color32,
    text_strong: Color32,
    /// Secondary text: group names, labels beside values, the status bar.
    muted: Color32,
    shadow: Color32,
}

// The approved workspace's colours (build plan 2.2): a restrained teal on light neutral
// surfaces, and their dark counterparts.
const LIGHT: Palette = Palette {
    accent: Color32::from_rgb(23, 106, 103),
    accent_bg: Color32::from_rgb(223, 240, 236),
    panel: Color32::from_rgb(248, 250, 251),
    surface: Color32::from_rgb(255, 255, 255),
    canvas: Color32::from_rgb(233, 238, 241),
    widget: Color32::from_rgb(237, 241, 243),
    widget_hover: Color32::from_rgb(223, 240, 236),
    widget_active: Color32::from_rgb(204, 228, 222),
    border: Color32::from_rgb(215, 224, 229),
    edge: Color32::from_rgb(196, 207, 214),
    text: Color32::from_rgb(32, 46, 56),
    text_strong: Color32::from_rgb(16, 26, 33),
    muted: Color32::from_rgb(91, 108, 121),
    shadow: Color32::from_black_alpha(28),
};

const DARK: Palette = Palette {
    accent: Color32::from_rgb(122, 202, 192),
    accent_bg: Color32::from_rgb(35, 72, 64),
    panel: Color32::from_rgb(28, 36, 43),
    surface: Color32::from_rgb(36, 47, 55),
    canvas: Color32::from_rgb(20, 28, 34),
    widget: Color32::from_rgb(44, 57, 66),
    widget_hover: Color32::from_rgb(35, 72, 64),
    widget_active: Color32::from_rgb(45, 90, 80),
    border: Color32::from_rgb(64, 80, 90),
    edge: Color32::from_rgb(84, 101, 112),
    text: Color32::from_rgb(237, 242, 245),
    text_strong: Color32::from_rgb(250, 252, 253),
    muted: Color32::from_rgb(180, 193, 202),
    shadow: Color32::from_black_alpha(110),
};

/// The desktop's accent colour, used for LimarePDF's teal when the user follows the
/// desktop and it says one ([`set_accent`]).
static DESKTOP_ACCENT: std::sync::RwLock<Option<Color32>> = std::sync::RwLock::new(None);

fn palette(dark: bool) -> Palette {
    let p = if dark { DARK } else { LIGHT };
    match DESKTOP_ACCENT.read().ok().and_then(|a| *a) {
        Some(a) => p.with_accent(a, dark),
        None => p,
    }
}

impl Palette {
    /// This palette with `accent` (the desktop's) for the teal: dark enough to read on the
    /// light theme's surfaces and to carry white text, or light enough to read on the dark
    /// theme's; the faint backgrounds of chosen and hovered things mixed from it.
    fn with_accent(self, accent: Color32, dark: bool) -> Palette {
        let toward = if dark { Color32::WHITE } else { Color32::BLACK };
        let ground = if dark { self.surface } else { Color32::WHITE };
        let mut a = accent;
        for _ in 0..20 {
            if contrast(a, ground) >= 4.5 {
                break;
            }
            a = mix(a, toward, 0.08);
        }
        let (faint, pressed) = if dark { (0.30, 0.42) } else { (0.14, 0.24) };
        Palette {
            accent: a,
            accent_bg: mix(self.surface, a, faint),
            widget_hover: mix(self.surface, a, faint),
            widget_active: mix(self.surface, a, pressed),
            ..self
        }
    }
}

/// `a` with `t` (0 to 1) of `b` mixed in.
fn mix(a: Color32, b: Color32, t: f32) -> Color32 {
    let m = |x: u8, y: u8| (f32::from(x) + (f32::from(y) - f32::from(x)) * t).round() as u8;
    Color32::from_rgb(m(a.r(), b.r()), m(a.g(), b.g()), m(a.b(), b.b()))
}

/// The contrast ratio of two colours, 1 to 21, as the Web Content Accessibility Guidelines
/// reckon it (4.5 for ordinary text).
fn contrast(a: Color32, b: Color32) -> f32 {
    let lum = |c: Color32| {
        let lin = |v: u8| {
            let v = f32::from(v) / 255.0;
            if v <= 0.04045 {
                v / 12.92
            } else {
                ((v + 0.055) / 1.055).powf(2.4)
            }
        };
        0.2126 * lin(c.r()) + 0.7152 * lin(c.g()) + 0.0722 * lin(c.b())
    };
    let (x, y) = (lum(a), lum(b));
    (x.max(y) + 0.05) / (x.min(y) + 0.05)
}

/// Use `accent` (the desktop's) for LimarePDF's teal, or the teal again (`None`), in both
/// themes' styles.
pub fn set_accent(ctx: &egui::Context, accent: Option<Color32>) {
    if let Ok(mut a) = DESKTOP_ACCENT.write() {
        if *a == accent {
            return;
        }
        *a = accent;
    }
    ctx.set_style_of(Theme::Light, style(false));
    ctx.set_style_of(Theme::Dark, style(true));
}

/// Font family for headings and the document name (Inter SemiBold).
pub fn semibold() -> FontFamily {
    FontFamily::Name("semibold".into())
}

/// Font family for dialog titles and the headings in dialogs (Inter Medium).
pub fn medium() -> FontFamily {
    FontFamily::Name("medium".into())
}

/// Font family for icons: Phosphor first. Inter has its own glyphs at many of the
/// private-use code points Phosphor uses, so icons in the text family can come out as
/// Inter's letters. Use this family for icon glyphs only: Phosphor also maps a–z to
/// blank glyphs.
pub fn icons() -> FontFamily {
    FontFamily::Name("icons".into())
}

/// Like [`icons`], with Phosphor's filled glyphs: the same code points, drawn solid.
pub fn icons_fill() -> FontFamily {
    FontFamily::Name("icons-fill".into())
}

/// An icon glyph in the icon family.
pub fn icon(glyph: &str) -> RichText {
    RichText::new(glyph).family(icons())
}

/// A button showing `glyph` before `text`; screen readers say the text alone.
pub fn glyph_button(ui: &mut egui::Ui, glyph: &str, text: &str) -> egui::Response {
    let r = ui.button((icon(glyph), text));
    crate::dialogs::set_name(&r, text);
    r
}

/// Install the fonts and the light and dark styles.
pub fn install(ctx: &egui::Context) {
    ctx.set_fonts(fonts());
    ctx.set_style_of(Theme::Light, style(false));
    ctx.set_style_of(Theme::Dark, style(true));
}

/// Inter for text, then egui's own fonts as fallbacks (symbols, emoji), with the
/// Phosphor icons after Inter. Fonts for text being typed on the page are added when first
/// needed (`typing_fonts`).
fn fonts() -> FontDefinitions {
    let mut f = FontDefinitions::default();
    let regular = include_bytes!("../assets/fonts/Inter-Regular.ttf");
    let semi = include_bytes!("../assets/fonts/Inter-SemiBold.ttf");
    let mid = include_bytes!("../assets/fonts/Inter-Medium.ttf");
    let phosphor = include_bytes!("../assets/fonts/Phosphor.ttf");
    let phosphor_fill = include_bytes!("../assets/fonts/Phosphor-Fill.ttf");
    f.font_data
        .insert("Inter".into(), FontData::from_static(regular).into());
    f.font_data
        .insert("Inter-SemiBold".into(), FontData::from_static(semi).into());
    f.font_data
        .insert("Inter-Medium".into(), FontData::from_static(mid).into());
    f.font_data
        .insert("phosphor".into(), FontData::from_static(phosphor).into());
    let text = f.families.entry(FontFamily::Proportional).or_default();
    text.insert(0, "Inter".into());
    // Second, so icons can sit in ordinary labels without Phosphor's blank a–z hiding Inter's.
    text.insert(1, "phosphor".into());
    // The same list with SemiBold or Medium in front, so icons and symbols still resolve.
    let mut bold = f.families[&FontFamily::Proportional].clone();
    bold.insert(0, "Inter-SemiBold".into());
    f.families.insert(semibold(), bold);
    let mut medium_list = f.families[&FontFamily::Proportional].clone();
    medium_list.insert(0, "Inter-Medium".into());
    f.families.insert(medium(), medium_list);
    let mut icons_list = f.families[&FontFamily::Proportional].clone();
    icons_list.retain(|n| n != "phosphor");
    icons_list.insert(0, "phosphor".into());
    f.families.insert(icons(), icons_list.clone());
    f.font_data.insert(
        "phosphor-fill".into(),
        FontData::from_static(phosphor_fill).into(),
    );
    icons_list[0] = "phosphor-fill".into();
    f.families.insert(icons_fill(), icons_list);
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
        (TextStyle::Body, text(14.0)),
        (TextStyle::Button, text(14.0)),
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
    v.weak_text_color = Some(p.muted);

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

/// The height of fields, lists and buttons in dialogs.
pub const DIALOG_CONTROL: f32 = 32.0;

/// The look inside dialogs: text and buttons at 14 points, and fields, lists and buttons
/// outlined, on the dialog's background, and [`DIALOG_CONTROL`] tall. Radio buttons and
/// checkboxes keep their rows (and are filled as the dialog is by
/// [`dialog_choices`]).
pub fn dialog_style(s: &mut Style) {
    let p = palette(s.visuals.dark_mode);
    for t in [TextStyle::Body, TextStyle::Button] {
        if let Some(f) = s.text_styles.get_mut(&t) {
            f.size = 14.0;
        }
    }
    let w = &mut s.visuals.widgets;
    w.inactive.weak_bg_fill = p.surface;
    for wv in [&mut w.inactive, &mut w.hovered, &mut w.active, &mut w.open] {
        wv.bg_stroke = Stroke::new(1.0, p.edge);
    }
    // Buttons, lists and number boxes are their text's height plus this padding.
    s.spacing.button_padding = Vec2::new(12.0, 8.0);
    // A scroll bar beside the fields, not over their outlines.
    s.spacing.scroll = egui::style::ScrollStyle::solid();
}

/// Radio buttons and checkboxes at rest on the dialog's background, outlined, as in the
/// mockup. egui fills scroll bars with the same colour, so this is for what is inside a
/// scrolling area, not the area itself.
pub fn dialog_choices(s: &mut Style) {
    s.visuals.widgets.inactive.bg_fill = palette(s.visuals.dark_mode).surface;
}

/// `edit` as a field in a dialog: as tall as the dialog's other controls, its text
/// centred in it.
pub fn field(edit: egui::TextEdit<'_>) -> egui::TextEdit<'_> {
    edit.margin(Margin::symmetric(12, 0))
        .min_size(Vec2::new(0.0, DIALOG_CONTROL))
        .align(egui::Align2::LEFT_CENTER)
}

/// The accent colour.
pub fn accent(dark: bool) -> Color32 {
    palette(dark).accent
}

/// The accent's faint background: what is chosen or pressed, and hovered buttons.
pub fn accent_bg(dark: bool) -> Color32 {
    palette(dark).accent_bg
}

/// Secondary text: group names, labels beside their values, the status bar.
pub fn muted(dark: bool) -> Color32 {
    palette(dark).muted
}

/// Thin dividers between the window's parts and its groups of commands.
pub fn border(dark: bool) -> Color32 {
    palette(dark).border
}

/// The command band, document tabs and inspector: white in the light theme.
pub fn surface(dark: bool) -> Color32 {
    palette(dark).surface
}

/// The shading over selected words on the page, which stays light in either theme: the
/// workspace's selected-text colour, the words showing through.
/// With the desktop's accent, its light tint.
pub fn text_shading(dark: bool) -> Color32 {
    let alpha = if dark { 150 } else { 140 };
    if let Some(a) = DESKTOP_ACCENT.read().ok().and_then(|a| *a) {
        let c = mix(Color32::WHITE, a, 0.4);
        return Color32::from_rgba_unmultiplied(c.r(), c.g(), c.b(), alpha);
    }
    if dark {
        Color32::from_rgba_unmultiplied(181, 214, 223, alpha)
    } else {
        Color32::from_rgba_unmultiplied(170, 208, 222, alpha)
    }
}

/// How good or bad a status is, for its colour: digital signature results and their
/// banner.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Tone {
    #[default]
    Neutral,
    Good,
    Caution,
    Bad,
}

/// The colour for a tone's icons and text; `None` for neutral (the usual text colour).
pub fn tone(dark: bool, t: Tone) -> Option<Color32> {
    Some(match (t, dark) {
        (Tone::Neutral, _) => return None,
        (Tone::Good, false) => Color32::from_rgb(21, 128, 61),
        (Tone::Good, true) => Color32::from_rgb(74, 222, 128),
        (Tone::Caution, false) => Color32::from_rgb(180, 83, 9),
        (Tone::Caution, true) => Color32::from_rgb(251, 191, 36),
        (Tone::Bad, false) => Color32::from_rgb(185, 28, 28),
        (Tone::Bad, true) => Color32::from_rgb(248, 113, 113),
    })
}

/// A button for the main thing to do on a screen: white text on the light theme's
/// accent in both themes (the dark theme's lighter accent is too pale for white text).
pub fn primary_button(text: &str) -> egui::Button<'static> {
    let label = RichText::new(text.to_owned())
        .color(Color32::WHITE)
        .family(semibold());
    egui::Button::new(label).fill(palette(false).accent)
}

/// A button for a main action that loses work or breaks signatures: white text on red in
/// both themes.
pub fn danger_button(text: &str) -> egui::Button<'static> {
    let label = RichText::new(text.to_owned())
        .color(Color32::WHITE)
        .family(semibold());
    egui::Button::new(label).fill(Color32::from_rgb(185, 28, 28))
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

/// Frame for the status bar at the bottom of the window.
pub fn bar_frame(style: &Style) -> egui::Frame {
    egui::Frame::side_top_panel(style).inner_margin(Margin::symmetric(12, 3))
}

/// Frame for the row of document tabs at the window's top, which stand on its lower edge.
pub fn tabs_frame(style: &Style) -> egui::Frame {
    egui::Frame::side_top_panel(style).inner_margin(Margin {
        left: 10,
        right: 10,
        top: 6,
        bottom: 0,
    })
}

/// Frame for the commands: the band's names and its groups, on the surface colour.
pub fn commands_frame(style: &Style) -> egui::Frame {
    egui::Frame::side_top_panel(style)
        .fill(palette(style.visuals.dark_mode).surface)
        .inner_margin(0)
}

/// The look inside the inspector and its pop-out, on the surface colour: text boxes filled as
/// its number boxes and lists are, where the surface's own colour would hide them.
pub fn inspector_style(s: &mut Style) {
    s.visuals.text_edit_bg_color = Some(palette(s.visuals.dark_mode).widget);
}

/// Frame for the inspector, as the workspace has it: on the surface colour, apart from the
/// side panel's, with room round its contents. `strip`: collapsed to its strip, as narrow
/// as egui's panels have it.
pub fn inspector_frame(style: &Style, strip: bool) -> egui::Frame {
    let room = match strip {
        true => egui::Margin::symmetric(8, 2),
        false => egui::Margin {
            left: 14,
            right: 14,
            top: 10,
            bottom: 8,
        },
    };
    egui::Frame::side_top_panel(style)
        .fill(palette(style.visuals.dark_mode).surface)
        .inner_margin(room)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A desktop accent too pale or too dark to read is darkened on the light theme (enough
    /// for white text on it too) or lightened on the dark one; one that reads is kept; the
    /// faint backgrounds come from it.
    #[test]
    fn a_desktop_accent_is_made_readable() {
        let pale = Color32::from_rgb(250, 220, 90);
        let light = LIGHT.with_accent(pale, false);
        assert!(contrast(light.accent, Color32::WHITE) >= 4.5);
        assert_ne!(light.accent, pale);
        let navy = Color32::from_rgb(20, 30, 90);
        let dark = DARK.with_accent(navy, true);
        assert!(contrast(dark.accent, DARK.surface) >= 4.5);
        let blue = Color32::from_rgb(0, 95, 184);
        assert_eq!(LIGHT.with_accent(blue, false).accent, blue);
        let faint = LIGHT.with_accent(blue, false).accent_bg;
        assert!(faint.b() > faint.r() && faint.r() > 200, "{faint:?}");
        // The teal itself reads on both.
        assert!(contrast(LIGHT.accent, Color32::WHITE) >= 4.5);
        assert!(contrast(DARK.accent, DARK.surface) >= 4.5);
    }
}

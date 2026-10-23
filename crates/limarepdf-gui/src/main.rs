// A release build on Windows is a GUI program, so no console window opens behind it.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod about;
mod actions;
mod app;
mod banners;
mod dialogs;
mod digital;
mod editor;
mod fields;
mod font_list;
mod inspector;
mod markup;
mod page_text;
mod panels;
mod recent;
mod recovery;
mod selection;
mod settings;
mod signatures;
mod signing;
mod tab;
mod theme;
mod typing_fonts;
mod worker;

/// Run document work that a malformed file could make panic, turning a panic into an error so the app stays
/// open. The caller must not trust any state the closure was changing when it fails.
pub fn guard<T>(f: impl FnOnce() -> limarepdf_core::Result<T>) -> limarepdf_core::Result<T> {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(f)).unwrap_or_else(|p| {
        let why = p
            .downcast_ref::<&str>()
            .map(|s| s.to_string())
            .or_else(|| p.downcast_ref::<String>().cloned())
            .unwrap_or_else(|| "unknown cause".into());
        Err(anyhow::anyhow!(
            "LimarePDF hit an internal error ({why}). The file may be damaged."
        ))
    })
}

fn main() -> eframe::Result {
    let mut files = Vec::new();
    let mut page: Option<usize> = None;
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        if a == "--page" {
            page = args.next().and_then(|v| v.parse().ok());
        } else if !a.starts_with("--") {
            files.push(std::path::PathBuf::from(a));
        }
    }
    let mut viewport = eframe::egui::ViewportBuilder::default()
        .with_title("LimarePDF")
        // Wayland takes the icon from the io.github.smaug42.limarepdf.desktop file this id names, not the window.
        .with_app_id("io.github.smaug42.limarepdf");
    match eframe::icon_data::from_png_bytes(include_bytes!("../../../docs/images/icon.png")) {
        Ok(icon) => viewport = viewport.with_icon(icon),
        Err(e) => eprintln!("limarepdf: app icon not loaded: {e}"),
    }
    // The window opens at the size it had when the app last closed.
    let settings = settings::Settings::load();
    let options = eframe::NativeOptions {
        viewport: viewport
            .with_inner_size(settings.window)
            .with_maximized(settings.window_maximized)
            .with_min_inner_size(settings::WINDOW_MIN)
            .with_drag_and_drop(true),
        ..Default::default()
    };
    eframe::run_native(
        "LimarePDF",
        options,
        Box::new(move |cc| Ok(Box::new(app::App::new(cc, settings, files, page)))),
    )
}

#[cfg(test)]
mod tests {
    #[test]
    fn guard_turns_panics_into_errors() {
        assert_eq!(super::guard(|| Ok(3)).unwrap(), 3);
        let e = super::guard::<()>(|| panic!("bad xref")).unwrap_err();
        assert!(e.to_string().contains("bad xref"));
        let n = 7;
        let e = super::guard::<()>(|| panic!("index {n}")).unwrap_err();
        assert!(e.to_string().contains("index 7"));
    }
}

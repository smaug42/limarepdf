// A release build on Windows is a GUI program, so no console window opens behind it.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod about;
mod actions;
mod app;
mod banners;
mod chosen;
mod dialogs;
mod digital;
mod editor;
mod fields;
mod font_list;
mod icon;
mod incoming;
mod inspector;
#[cfg(target_os = "macos")]
mod macos;
mod markup;
mod page_text;
mod panels;
mod recent;
mod recovery;
mod regions;
mod search;
mod selection;
mod settings;
mod signatures;
mod signing;
mod styled;
mod tab;
mod theme;
mod timing;
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

/// The files and `--page` number on the command line (without the program's name). Older
/// macOS gave an app started from Finder a `-psn_…` process number, which is no file.
fn read_args(
    args: impl IntoIterator<Item = String>,
    macos: bool,
) -> (Vec<std::path::PathBuf>, Option<usize>) {
    let mut files = Vec::new();
    let mut page: Option<usize> = None;
    let mut args = args.into_iter();
    while let Some(a) = args.next() {
        if a == "--page" {
            page = args.next().and_then(|v| v.parse().ok());
        } else if !a.starts_with("--") && !(macos && a.starts_with("-psn_")) {
            files.push(std::path::PathBuf::from(a));
        }
    }
    (files, page)
}

fn main() -> eframe::Result {
    timing::start();
    let (files, page) = read_args(std::env::args().skip(1), cfg!(target_os = "macos"));
    // Finder's Open With and double-click, from the first one, sent as the app launches.
    #[cfg(target_os = "macos")]
    macos::listen();
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
        // eframe's `system_fonts` is off (the workspace's Cargo.toml), so this would only
        // install a stand-in that logs a hint at the first character no font has.
        system_font_fallback: false,
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

    #[test]
    fn finder_process_numbers_are_not_files_on_macos() {
        let args = |a: &[&str]| a.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        let (files, page) =
            super::read_args(args(&["-psn_0_12345", "a b.pdf", "--page", "3"]), true);
        assert_eq!(files, [std::path::PathBuf::from("a b.pdf")]);
        assert_eq!(page, Some(3));
        // Elsewhere it is an ordinary (if odd) file name.
        let (files, _) = super::read_args(args(&["-psn_0_12345", "--other"]), false);
        assert_eq!(files, [std::path::PathBuf::from("-psn_0_12345")]);
    }
}

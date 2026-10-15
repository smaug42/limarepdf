// A release build on Windows is a GUI program, so no console window opens behind it.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod actions;
mod app;
mod banners;
mod dialogs;
mod editor;
mod fields;
mod inspector;
mod markup;
mod recent;
mod selection;
mod settings;
mod tab;
mod theme;
mod worker;

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
        .with_title("MPE")
        // Wayland takes the icon from the mpe.desktop file this id names, not the window.
        .with_app_id("mpe");
    match eframe::icon_data::from_png_bytes(include_bytes!("../../../docs/images/icon.png")) {
        Ok(icon) => viewport = viewport.with_icon(icon),
        Err(e) => eprintln!("mpe-gui: app icon not loaded: {e}"),
    }
    let options = eframe::NativeOptions {
        viewport: viewport
            .with_inner_size([1200.0, 850.0])
            .with_min_inner_size([640.0, 420.0])
            .with_drag_and_drop(true),
        ..Default::default()
    };
    eframe::run_native(
        "MPE",
        options,
        Box::new(move |cc| Ok(Box::new(app::App::new(cc, files, page)))),
    )
}

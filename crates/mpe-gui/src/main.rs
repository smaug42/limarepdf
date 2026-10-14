mod actions;
mod app;
mod dialogs;
mod editor;
mod inspector;
mod recent;
mod selection;
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
    let options = eframe::NativeOptions {
        viewport: eframe::egui::ViewportBuilder::default()
            .with_title("MPE")
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

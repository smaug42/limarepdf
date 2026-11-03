//! Links on the page (ENH-17): what hovering over one says, and what a click does.

use super::tools::line_at;
use super::*;
use lopdf::{dictionary, Object, StringFormat};

/// Where the links are, in displayed points on the first page.
const WEB: [f32; 4] = [100.0, 100.0, 250.0, 120.0];
const PROGRAM: [f32; 4] = [100.0, 200.0, 250.0, 220.0];

/// A two-page tab whose first page has a web link whose address hides a right-to-left
/// override (U+202E, which would show "exe.fdp" backwards as "pdf.exe"), and a link
/// that runs a program.
fn links_tab(ctx: &egui::Context) -> App {
    let mut app = app_with(ctx, 1);
    let t = app.tabs.last_mut().unwrap();
    assert!(t.edit(ctx, |p| {
        let page = p.page_ids()[0];
        // 792 points high: y measured up from the bottom.
        let at = |b: [f32; 4]| -> Object {
            let r = [b[0], 792.0 - b[3], b[2], 792.0 - b[1]];
            r.map(Object::Real).to_vec().into()
        };
        let mut uri = vec![0xEF, 0xBB, 0xBF];
        uri.extend("https://example.com/\u{202E}exe.fdp".as_bytes());
        let uri = Object::String(uri, StringFormat::Literal);
        let web = dictionary! { "Type" => "Annot", "Subtype" => "Link", "Rect" => at(WEB),
        "A" => dictionary! { "S" => "URI", "URI" => uri } };
        let run = dictionary! { "Type" => "Annot", "Subtype" => "Link", "Rect" => at(PROGRAM),
        "A" => dictionary! { "S" => "Launch", "F" => Object::string_literal("run.exe") } };
        let refs: Vec<Object> = [web, run]
            .into_iter()
            .map(|d| p.doc.add_object(d).into())
            .collect();
        p.doc.get_dictionary_mut(page)?.set("Annots", refs);
        Ok(())
    }));
    app
}

/// The middle of box `b` on the first page, on screen.
fn middle_of(ty: &Typist, app: &mut App, b: [f32; 4]) -> egui::Pos2 {
    let (min, k) = ty.page(app);
    min + egui::vec2((b[0] + b[2]) / 2.0, (b[1] + b[3]) / 2.0) * k
}

/// The tooltip shown with the pointer held over `at`, if it starts with `start`.
fn tip_over(ty: &Typist, app: &mut App, at: egui::Pos2, start: &str) -> Option<String> {
    ty.frame(app, vec![egui::Event::PointerMoved(at)]);
    let tree = ty.pause(app);
    line_at(&tree, start).map(|(text, _)| text)
}

/// A web link's address is shown on hover without the characters that could make it look
/// like another (the user guide's promise; the hover showed it as the file had it), and a
/// click asks before opening it.
#[test]
fn a_web_links_address_is_shown_cleaned_and_asked_about() {
    let (ctx, _) = paragraphs::new_app();
    let ty = Typist::new(&ctx);
    let mut app = links_tab(&ctx);
    let at = middle_of(&ty, &mut app, WEB);
    let tip = tip_over(&ty, &mut app, at, "https://example.com/").expect("no address shown");
    assert!(!tip.contains('\u{202E}'), "{tip:?}");
    ty.click(&mut app, at);
    assert!(matches!(&app.dialog, Dialog::OpenLink(u) if u.starts_with("https://example.com/")));
}

/// A link that runs a program says so on hover, and a click there doesn't offer to run it.
#[test]
fn a_link_that_runs_a_program_says_so_and_does_nothing() {
    let (ctx, _) = paragraphs::new_app();
    let ty = Typist::new(&ctx);
    let mut app = links_tab(&ctx);
    let at = middle_of(&ty, &mut app, PROGRAM);
    let tip = tip_over(&ty, &mut app, at, "Runs a program");
    assert_eq!(
        tip.as_deref(),
        Some("Runs a program or opens a file: run.exe. LimarePDF doesn't run them.")
    );
    ty.click(&mut app, at);
    assert!(matches!(app.dialog, Dialog::None));
    assert!(app.tabs.iter().all(|t| t.open_link.is_none()));
}

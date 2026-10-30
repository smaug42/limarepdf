//! Text typed on the page as core sets it (the text design, 5.1, and its acceptance, 10):
//! each word in the font core writes it in, falling back where the font chosen lacks it, and
//! the inspector saying which words go to another font.

use super::*;

/// Typed in a line, words its font lacks go to an installed font that has them, and the
/// inspector says which, and to what: "Not in Helvetica: 漢字. Those characters use …".
#[test]
fn the_inspector_says_which_words_go_to_another_font() {
    crate::font_list::test_family("Limare Han Test");
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    theme::install(&ctx);
    let mut app = App::with(Settings::from_file(None), Recent::from_file(None));
    lines_tab(&mut app, &ctx, 400.0, &paragraph(72.0, 100.0, 3, 1, false));
    edit_otter(&mut app, &ctx, 1400.0, "opened");
    let says = inspector_says(&settled_at(&mut app, &ctx, 1400.0));
    assert!(
        !says.iter().any(|s| s.starts_with("Not in Helvetica")),
        "{says:?}"
    );
    window(
        &mut app,
        &ctx,
        1400.0,
        vec![egui::Event::Text("漢字".into())],
    );
    let says = inspector_says(&settled_at(&mut app, &ctx, 1400.0));
    let gone = limarepdf_core::fallbacks(FontChoice::default(), "The 漢字 ate");
    let [(word, used)] = &gone[..] else {
        panic!("{gone:?}");
    };
    assert_eq!(word, "漢字");
    let used = used.family_name();
    let want = format!("Not in Helvetica: 漢字. Those characters use {used}.");
    assert!(says.contains(&want), "{says:?}");
}

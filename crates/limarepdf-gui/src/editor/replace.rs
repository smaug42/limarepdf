//! Replace (NEW-04a, batch 46's stage 2): a match of the search typed over on the page, as
//! typing over selected words is: its line opens with the match selected and the
//! replacement typed in its place, drawn in the line's fonts with the one overflow rule's
//! warning; across a paragraph's lines, the paragraph does. The line then waits, open, as
//! F6 leaves it, while the keyboard goes back to Search results: Replace again, or a click
//! away, keeps it as one change.

use super::*;
use limarepdf_core::HitPart;

/// Why Replace left a match as it was.
#[derive(Clone, Debug, PartialEq)]
pub enum NotReplaced {
    /// Something about the match itself, said at it on the page.
    Match(&'static str),
    /// The replacement has a character LimarePDF can't write there (the first, a grapheme
    /// cluster), said at Replace's field.
    Text(String),
}

const ON_TWO_PAGES: &str = "Not replaced: the match runs on to the next page";
const ACROSS_PARAGRAPHS: &str = "Not replaced: the match runs across paragraphs";
const HIDDEN: &str = "Not replaced: invisible recognised text";
const GROUPED: &str = "Not replaced: text inside a group";
const VERTICAL: &str = "Not replaced: vertical writing";

/// The characters `a..b` of a line's `text` deleting them takes, as a word processor deletes
/// a word: with the space after it when a space or the line's start is before it, or the
/// space before it at the line's end, so that no two spaces are left together.
fn with_a_space(text: &str, (a, b): (usize, usize)) -> (usize, usize) {
    let t: Vec<char> = text.chars().collect();
    let space = |i: usize| t.get(i) == Some(&' ');
    let after_space = a == 0 || space(a - 1);
    if space(b) && after_space {
        (a, b + 1)
    } else if b >= t.len() && a > 0 && space(a - 1) {
        (a - 1, b)
    } else {
        (a, b)
    }
}

impl Editor {
    /// Type `with` over the match of a search whose parts are `parts`: its line opens on the
    /// page with the match selected and `with` typed in its place (an empty `with` deletes
    /// it), or across a paragraph's lines (one LimarePDF is sure of, or any with paragraph
    /// editing on) the paragraph does. Once it is typed the line waits, open, and
    /// [`Self::typed_over`] says so, for the keyboard to go back to Search results. What
    /// isn't replaced says why.
    pub fn replace(
        &mut self,
        pdf: &limarepdf_core::Pdf,
        parts: &[HitPart],
        with: &str,
    ) -> Result<(), NotReplaced> {
        let Some(page) = parts.first().map(|h| h.page) else {
            return Err(NotReplaced::Match(ON_TWO_PAGES));
        };
        if parts.iter().any(|h| h.page != page) {
            return Err(NotReplaced::Match(ON_TWO_PAGES));
        }
        // Each line's characters, a line the match comes back to taken once.
        let mut lines: Vec<(usize, (usize, usize))> = Vec::new();
        for h in parts {
            let Some(id) = h.object else {
                return Err(NotReplaced::Match(GROUPED));
            };
            match lines.iter_mut().find(|(l, _)| *l == id) {
                Some((_, c)) => *c = (c.0.min(h.chars.0), c.1.max(h.chars.1)),
                None => lines.push((id, h.chars)),
            }
        }
        let objs = self.objects(pdf, page);
        for &(id, _) in &lines {
            let Some(o) = objs.get(id).filter(|o| o.kind == ObjectKind::Text) else {
                return Err(NotReplaced::Match(GROUPED));
            };
            if o.hidden {
                return Err(NotReplaced::Match(HIDDEN));
            }
            if o.vertical {
                return Err(NotReplaced::Match(VERTICAL));
            }
        }
        // What can't be written in the font the replacement goes into is refused first, as
        // typing it would leave it out.
        let (first, (a, _)) = lines[0];
        let font = objs.get(first).map_or_else(FontChoice::default, |o| {
            let at = (o.spans.iter()).find(|s| s.chars.0 <= a && a < s.chars.1);
            let span = at.or(o.spans.first()).map(|s| s.font_name.as_str());
            let name = span.or(o.font_name.as_deref());
            name.map_or_else(FontChoice::default, font_of)
        });
        if let Some(c) = page_text::untypable(with, font) {
            return Err(NotReplaced::Text(c));
        }
        // Deleted, a word on a line goes with a space beside it.
        if let ([(id, chars)], true) = (&mut lines[..], with.is_empty()) {
            let text = objs.get(*id).and_then(|o| o.text.as_deref());
            *chars = with_a_space(text.unwrap_or(""), *chars);
        }
        let typed = vec![match with.is_empty() {
            true => egui::Event::Key {
                key: egui::Key::Delete,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: egui::Modifiers::NONE,
            },
            false => egui::Event::Text(with.to_string()),
        }];
        match lines[..] {
            [(id, (a, b))] => self.start_text_edit(pdf, page, id, Pick::Chars(a, b)),
            [.., (_, (_, b))] => {
                let ids: Vec<usize> = lines.iter().map(|&(id, _)| id).collect();
                if !self.paragraph_over(pdf, page, &ids, (a, b), true) {
                    return Err(NotReplaced::Match(ACROSS_PARAGRAPHS));
                }
            }
            [] => return Err(NotReplaced::Match(GROUPED)),
        }
        if let Some(pe) = self.para.as_mut() {
            (pe.typed, pe.park) = (typed, true);
        } else if let Some(le) = self.line_edit.as_mut() {
            (le.typed, le.park) = (typed, true);
        }
        Ok(())
    }

    /// Say `why` Replace left the match whose first box is `at` (displayed-page points) on
    /// `page` as it was, under the match a while.
    pub fn note_at_match(
        &mut self,
        ctx: &egui::Context,
        page: usize,
        at: [f32; 4],
        why: &'static str,
    ) {
        self.match_note.mark(ctx, why.to_string());
        self.match_note_at = Some((page, at));
    }

    /// The note [`Self::note_at_match`] put on `page`, drawn there (at `r`, `k` pixels per
    /// point) while it lasts.
    pub(super) fn show_match_note(&mut self, ctx: &egui::Context, page: usize, r: Rect, k: f32) {
        let Some((_, b)) = self.match_note_at.filter(|m| m.0 == page) else {
            return;
        };
        let at = Pos2::new(r.min.x + b[0] * k, r.min.y + b[3] * k + 4.0);
        let id = self.scope.with(("match-note", page));
        self.match_note.show_at(ctx, at, id);
    }
}

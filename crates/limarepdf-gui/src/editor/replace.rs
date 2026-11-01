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

pub(super) const ON_TWO_PAGES: &str = "Not replaced: the match runs on to the next page";
pub(super) const ACROSS_PARAGRAPHS: &str = "Not replaced: the match runs across paragraphs";
pub(super) const HIDDEN: &str = "Not replaced: invisible recognised text";
pub(super) const GROUPED: &str = "Not replaced: text inside a group";
pub(super) const VERTICAL: &str = "Not replaced: vertical writing";

/// The page all of a match whose parts are `parts` is on, or why it isn't replaced.
pub(super) fn page_of(parts: &[HitPart]) -> Result<usize, &'static str> {
    let page = parts.first().map(|h| h.page).ok_or(ON_TWO_PAGES)?;
    match parts.iter().all(|h| h.page == page) {
        true => Ok(page),
        false => Err(ON_TWO_PAGES),
    }
}

/// A match's lines, by object id, in reading order, each with its characters (from, to).
pub(super) type MatchLines = Vec<(usize, (usize, usize))>;

/// The lines of the page's objects `objs` a match whose parts are `parts` is on, a line the
/// match comes back to taken once; or why it isn't replaced: text in a group, invisible
/// recognised text, vertical writing.
pub(super) fn lines_of(objs: &[PageObject], parts: &[HitPart]) -> Result<MatchLines, &'static str> {
    let mut lines: MatchLines = Vec::new();
    for h in parts {
        let id = h.object.ok_or(GROUPED)?;
        match lines.iter_mut().find(|(l, _)| *l == id) {
            Some((_, c)) => *c = (c.0.min(h.chars.0), c.1.max(h.chars.1)),
            None => lines.push((id, h.chars)),
        }
    }
    for &(id, _) in &lines {
        let o = (objs.get(id).filter(|o| o.kind == ObjectKind::Text)).ok_or(GROUPED)?;
        if o.hidden {
            return Err(HIDDEN);
        }
        if o.vertical {
            return Err(VERTICAL);
        }
    }
    match lines.is_empty() {
        true => Err(GROUPED),
        false => Ok(lines),
    }
}

/// The font a replacement typed over a match starting at character `a` of line `o` goes
/// into: that of the line's part there, as typing it would take.
pub(super) fn typed_font(o: Option<&PageObject>, a: usize) -> FontChoice {
    o.map_or_else(FontChoice::default, |o| {
        let at = (o.spans.iter()).find(|s| s.chars.0 <= a && a < s.chars.1);
        let span = at.or(o.spans.first()).map(|s| s.font_name.as_str());
        let name = span.or(o.font_name.as_deref());
        name.map_or_else(FontChoice::default, font_of)
    })
}

/// The characters `a..b` of a line's `text` deleting them takes, as a word processor deletes
/// a word: with the space after it when a space or the line's start is before it, or the
/// space before it at the line's end, so that no two spaces are left together.
pub(super) fn with_a_space(text: &str, (a, b): (usize, usize)) -> (usize, usize) {
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
        let page = page_of(parts).map_err(NotReplaced::Match)?;
        let objs = self.objects(pdf, page);
        let mut lines = lines_of(objs, parts).map_err(NotReplaced::Match)?;
        // What can't be written in the font the replacement goes into is refused first, as
        // typing it would leave it out.
        let (first, (a, _)) = lines[0];
        if let Some(c) = page_text::untypable(with, typed_font(objs.get(first), a)) {
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

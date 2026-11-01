//! Search results' Replace (NEW-04a, batch 46's stage 2): Edit > Replace (Ctrl+H) shows a
//! Replace with field under the search field. Replace types it over the current match on the
//! page ([`Editor::replace`]) and makes the next match current, the view staying at the
//! match replaced, whose line waits open. Replace again keeps that change, one undo step,
//! and replaces the next once the search, run again for the document as it is now, is
//! back.

use super::*;
use crate::editor::NotReplaced;
use crate::page_text;

/// The Replace button's key while the keyboard is in Search results, but on a Mac.
const ALT_R: egui::KeyboardShortcut =
    egui::KeyboardShortcut::new(egui::Modifiers::ALT, egui::Key::R);

/// Search results' Replace.
#[derive(Default)]
pub struct Replace {
    /// Its field and button are shown (Edit > Replace; Find leaves them out).
    pub shown: bool,
    /// What is typed over a match.
    pub with: String,
    /// Put the keyboard in its field when next drawn.
    pub focus: bool,
    /// Replace was asked while the search ran again for the document as it is now: the
    /// match current once it is back is replaced then.
    waiting: bool,
    /// The match typed over, its change not kept yet.
    over: Option<Over>,
    /// Why the replacement was refused, under its field a while.
    refused: page_text::Refused,
}

/// A match Replace typed over: its hit, its page, and how many matches the replacement
/// itself holds ([`Query::count_in`]), so that once the change is kept and the search runs
/// again the match current is still the one after it.
#[derive(Clone, Copy, Debug)]
struct Over {
    hit: usize,
    page: usize,
    holds: usize,
}

impl Tab {
    /// Whether the current match can be replaced now: the search shown is for the query and
    /// its options as they are, on the document as it is now, and its current match has
    /// come.
    pub fn match_ready(&self) -> bool {
        self.search.shows(&self.query, self.search_options)
            && self.to_hit.is_none()
            && self.hit_idx < self.hits.len()
    }

    /// Whether the search for the query as it is waits to be run or to come back for the
    /// document as it is now, or its current match to come.
    pub fn search_waits(&self) -> bool {
        !Query::new(&self.query).is_empty()
            && (!self.search.shows(&self.query, self.search_options) || self.to_hit.is_some())
            && self.search.failed.is_none()
    }

    /// Replace the current match with Replace's text (Search results' Replace, Enter in
    /// its field): typed over it on the page, the next match current. Just after the last
    /// replacement was kept, while the search runs again, it waits, and replaces the match
    /// current once the search is back. What isn't replaced says why: at the match, or, for
    /// a character LimarePDF can't write, at the field.
    pub fn replace_match(&mut self, ctx: &egui::Context) {
        if !self.match_ready() {
            self.replace.waiting = self.search_waits();
            return;
        }
        self.replace.waiting = false;
        let i = self.hit_idx;
        let hit = self.hits[i].clone();
        // Its line opens with the match in view, scrolled to only as far as it must be.
        self.reveal = hit.boxes.first().copied();
        match self.ed.replace(&self.pdf, &hit.parts, &self.replace.with) {
            Ok(()) => {
                let q = Query::with(&self.query, self.search_options);
                let holds = q.count_in(&self.replace.with);
                self.replace.over = Some(Over {
                    hit: i,
                    page: hit.page,
                    holds,
                });
                self.hit_idx = (i + 1) % self.hits.len();
                self.search.reveal = true;
            }
            Err(NotReplaced::Match(why)) => {
                let (page, b) = hit.boxes.first().copied().unwrap_or((hit.page, [0.0; 4]));
                self.ed.note_at_match(ctx, page, b, why);
            }
            Err(NotReplaced::Text(c)) => {
                let note = page_text::note(&c).to_string();
                self.replace.refused.mark(ctx, note);
            }
        }
    }

    /// The change Replace typed was kept and the search is to run again: the current match,
    /// `nth` of those on `page`, is counted again as the matches will be once the search is
    /// back, the replaced one gone and those its text holds in its place.
    pub(super) fn after_replacing(&mut self, page: usize, nth: usize) -> usize {
        match self.replace.over.take() {
            Some(o) if o.page == page && o.hit < self.hit_idx => (nth + o.holds).saturating_sub(1),
            _ => nth,
        }
    }

    /// The matches came in and the current one is placed: a Replace waiting for them
    /// replaces it now, unless other typing began meanwhile; with none left, it waits no
    /// more.
    pub(super) fn replace_when_placed(&mut self, ctx: &egui::Context, done: bool) {
        if !self.replace.waiting || self.to_hit.is_some() {
            return;
        }
        let typing = self.ed.typing_on_page() || self.ed.pending();
        let none = done && self.hits.is_empty();
        if typing || none || self.search.failed.is_some() {
            self.replace.waiting = false;
        } else if self.match_ready() {
            self.replace_match(ctx);
        }
    }

    /// What Replace typed ended without a change to keep: no match is typed over.
    pub(super) fn forget_replaced(&mut self) {
        if !self.ed.typing_on_page() && self.ed.actions.is_empty() {
            self.replace.over = None;
        }
    }

    /// Replace's field and its button, under the search field, while Replace is shown; the
    /// Replace pressed, Enter in the field, or Alt+R with the keyboard in Search results
    /// (`held`: the search field or an option has it; the field and button are added here),
    /// is returned for the app to run. Alt+R is the key Word's Find and Replace window gives
    /// its Replace button; on a Mac, where Option+R types "®" and Word has none, it isn't.
    pub(super) fn replace_ui(&mut self, ui: &mut egui::Ui, c: &Ctx, held: bool) -> Option<Act> {
        // Replace's text typed over a match, its line waiting: the keyboard comes back.
        let typed_over = std::mem::take(&mut self.ed.typed_over);
        if !self.replace.shown {
            return None;
        }
        let r = ui.add(
            egui::TextEdit::singleline(&mut self.replace.with)
                .hint_text("Replace with")
                .desired_width(f32::INFINITY)
                .return_key(None),
        );
        crate::dialogs::set_name(&r, "Replace with");
        if std::mem::take(&mut self.replace.focus) || typed_over {
            r.request_focus();
        }
        let id = r.id;
        self.replace
            .refused
            .show_at(ui.ctx(), r.rect.left_bottom() + Vec2::new(0.0, 4.0), id);
        let enter = r.has_focus()
            && ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Enter));
        // Its own word beside the field; the menus say which match it replaces.
        let a = actions::get(Act::ReplaceMatch);
        let enabled = (a.enabled)(c);
        let b = ui.add_enabled(enabled, egui::Button::new("Replace"));
        b.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, enabled, "Replace"));
        let ctx = ui.ctx().clone();
        let alt_r = (ctx.os() != egui::os::OperatingSystem::Mac).then_some(ALT_R);
        let held = held || r.has_focus() || b.has_focus();
        let keyed = alt_r.is_some_and(|k| held && ui.input_mut(|i| i.consume_shortcut(&k)));
        let tip = match alt_r {
            Some(k) => format!("Replace ({})\n{}", ctx.format_shortcut(&k), a.hint),
            None => format!("Replace\n{}", a.hint),
        };
        let clicked = b
            .on_hover_text(tip)
            .on_disabled_hover_text(actions::greyed_tip(&ctx, a, c))
            .clicked();
        (clicked || (enter || keyed) && enabled).then_some(Act::ReplaceMatch)
    }
}

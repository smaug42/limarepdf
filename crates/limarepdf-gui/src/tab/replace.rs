//! Search results' Replace (NEW-04a, batch 46's stage 2): Edit > Replace (Ctrl+H) shows a
//! Replace with field under the search field. Replace types it over the current match on the
//! page ([`Editor::replace`]) and makes the next match current, the view staying at the
//! match replaced, whose line waits open. Replace again keeps that change, one undo step,
//! and replaces the next once the search, run again for the document as it is now, is
//! back. Replace all (NEW-04b) replaces every match on a copy of the document
//! ([`crate::editor::replace_all`]), which the document takes, as one change, once its
//! review is answered.

use super::*;
use crate::dialogs::ReplaceReview;
use crate::editor::{AllJob, NotReplaced, Pass, Prepared};
use crate::page_text;

/// The most pages Replace all's review names where replacements don't fit.
const PAGES_NAMED: usize = 8;

/// The Replace button's key while the keyboard is in Search results, but on a Mac.
const ALT_R: egui::KeyboardShortcut =
    egui::KeyboardShortcut::new(egui::Modifiers::ALT, egui::Key::R);
/// Replace all's, as Word's Find and Replace window has it, but on a Mac.
const ALT_A: egui::KeyboardShortcut =
    egui::KeyboardShortcut::new(egui::Modifiers::ALT, egui::Key::A);

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

    /// Whether the search shown is still looking through the pages.
    pub fn search_running(&self) -> bool {
        let (q, o) = (&self.query, self.search_options);
        self.search.shows(q, o) && !self.search.complete(q, o)
    }

    /// Replace all's work: every match of the search shown, which has looked through every
    /// page, replaced with Replace's text on one line. `None` while it isn't so.
    pub fn replace_all_job(&self) -> Option<AllJob> {
        let done = self.search.complete(&self.query, self.search_options);
        (done && !self.hits.is_empty()).then(|| AllJob {
            pdf: self.pdf.clone(),
            matches: self.hits.iter().map(|h| h.parts.clone()).collect(),
            with: self.replace.with.replace(['\r', '\n', '\t'], " "),
            paragraphs: self.ed.paragraphs,
            words: self.ed.doc_words.clone(),
        })
    }

    /// What Replace all's review says of `p`, `query`'s matches replaced with `with`: the
    /// pages where some don't fit named as the view names them, the first few.
    pub fn replace_review(&self, p: &Prepared, query: &str, with: &str) -> ReplaceReview {
        let all = &p.all;
        let mut names: Vec<String> = (all.over_pages.iter().take(PAGES_NAMED))
            .map(|&i| self.page_name(i))
            .collect();
        let more = all.over_pages.len().saturating_sub(PAGES_NAMED);
        let pages = match (names.len(), more) {
            (0, _) => String::new(),
            (1, _) => format!("page {}", names[0]),
            (_, 0) => {
                let last = names.pop().unwrap_or_default();
                format!("pages {} and {last}", names.join(", "))
            }
            (_, n) => format!("pages {} and {n} more", names.join(", ")),
        };
        ReplaceReview {
            query: query.to_string(),
            with: with.to_string(),
            replaced: all.replaced,
            pages: all.pages.len(),
            over: all.over,
            over_pages: pages,
            other_font: all.other_font,
            left: (all.left.iter())
                .map(|&(why, n)| (n, why.said(n)))
                .collect(),
            fits: p.fits.as_ref().is_some_and(|f| f.replaced > 0),
        }
    }

    /// The document takes Replace all's copy `pass` (as reviewed), as one change, and says
    /// so with Undo; only the pages it changed are drawn again.
    pub fn take_replaced(&mut self, ctx: &egui::Context, pass: Pass) {
        let (n, unfit, similar) = (pass.replaced, pass.unfit, pass.similar);
        let pages = pass.pages.clone();
        let take = move |p: &mut Pdf| {
            *p = pass.pdf;
            Ok(())
        };
        if !(self.keep_page_typing(ctx) && self.edit_pages(ctx, &pages, take)) {
            return;
        }
        self.replace.over = None;
        self.replace.waiting = false;
        let mut said = match n {
            1 => "Replaced 1 match".to_string(),
            n => format!("Replaced {n} matches"),
        };
        if unfit > 0 {
            let are = match unfit {
                1 => "is left as it was",
                _ => "are left as they were",
            };
            said = format!("{said}; {unfit} that didn't fit {are}");
        }
        if similar {
            said = format!("{said}. {SIMILAR_FONT}");
        }
        self.notice = Some(said);
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

    /// Replace's field and its buttons, Replace and Replace all, under the search field,
    /// while Replace is shown; Replace pressed, Enter in the field, or Alt+R with the
    /// keyboard in Search results (`held`: the search field or an option has it; the field
    /// and buttons are added here), and Replace all pressed or Alt+A, are returned for the
    /// app to run. Alt+R and Alt+A are the keys Word's Find and Replace window gives its
    /// Replace and Replace All buttons; on a Mac, where Option types letters ("®", "å") and
    /// Word has none, they aren't.
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
        // Their own words, as in Word's window; the menus say what each replaces.
        struct Drawn {
            act: Act,
            enabled: bool,
            key: Option<egui::KeyboardShortcut>,
            clicked: bool,
            focused: bool,
        }
        let ctx = ui.ctx().clone();
        let mac = ctx.os() == egui::os::OperatingSystem::Mac;
        let buttons = [
            (Act::ReplaceMatch, "Replace", ALT_R),
            (Act::ReplaceAll, "Replace all", ALT_A),
        ];
        let drawn = ui.horizontal(|ui| {
            buttons.map(|(act, name, key)| {
                let a = actions::get(act);
                let enabled = (a.enabled)(c);
                let b = ui.add_enabled(enabled, egui::Button::new(name));
                b.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, enabled, name));
                let key = (!mac).then_some(key);
                let tip = match key {
                    Some(k) => format!("{name} ({})\n{}", ctx.format_shortcut(&k), a.hint),
                    None => format!("{name}\n{}", a.hint),
                };
                let focused = b.has_focus();
                let b = b.on_hover_text(tip);
                Drawn {
                    act,
                    enabled,
                    key,
                    clicked: b
                        .on_disabled_hover_text(actions::greyed_tip(&ctx, a, c))
                        .clicked(),
                    focused,
                }
            })
        });
        let held = held || r.has_focus() || drawn.inner.iter().any(|d| d.focused);
        let mut pressed = None;
        for d in drawn.inner {
            let keyed = d
                .key
                .is_some_and(|k| held && ui.input_mut(|i| i.consume_shortcut(&k)));
            let enter = enter && d.act == Act::ReplaceMatch;
            if d.clicked || (enter || keyed) && d.enabled {
                pressed = pressed.or(Some(d.act));
            }
        }
        pressed
    }
}

//! The Spelling view (NEW-06; Review > Spelling, F7): the words spelt wrongly in what was
//! written in the document, its comments and replies, text boxes and form fields, one at a
//! time with the dictionary's suggestions, as Word's Editor pane goes through a document's.
//! Change and Change all write the word chosen (Change all every one alike, as one step to
//! undo); Ignore leaves this one, Ignore all and Add to dictionary every one. The pages' own
//! text isn't gone through (PF-60).

use super::Tab;
use crate::selection::Selection;
use crate::theme;
use eframe::egui::{self, RichText};
use limarepdf_core::{AnnotKind, FieldKind, FieldValue, Pdf};
use std::collections::HashSet;
use std::ops::Range;

/// What a word spelt wrongly is in.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
enum Holder {
    Annot((u32, u16)),
    Field(String),
}

/// A word spelt wrongly, where it is.
#[derive(Clone, Debug, PartialEq)]
struct Wrong {
    holder: Holder,
    /// What holds it, as the view says it: "Note", "Reply", "Text box", "Field".
    what: String,
    page: usize,
    bbox: [f32; 4],
    /// The whole text it is in, and where it is in it (bytes).
    text: String,
    at: Range<usize>,
}

impl Wrong {
    fn word(&self) -> &str {
        &self.text[self.at.clone()]
    }
}

/// What the list of words spelt wrongly was worked out for: the document's revision, the
/// language and the spelling's revision, and how many words were ignored one by one.
type ListedFor = (u64, String, u64, usize);

/// The view's state, per tab.
#[derive(Default)]
pub struct SpellView {
    /// The words left as they are by Ignore: what holds each, where and the word.
    skipped: HashSet<(Holder, usize, String)>,
    /// The words spelt wrongly, and what they were worked out for.
    list: Option<(ListedFor, Vec<Wrong>)>,
    /// The suggestions for the word shown, and the one chosen.
    suggestions: Option<(Wrong, Vec<String>)>,
    chosen: usize,
}

/// A text written in the document that spelling goes through.
struct Written {
    holder: Holder,
    what: String,
    page: usize,
    bbox: [f32; 4],
    text: String,
}

/// What was written in `pdf`: its comments, replies and text boxes that can be changed, then
/// its text fields that can be typed in, by page.
fn written(pdf: &Pdf) -> Vec<Written> {
    let mut out: Vec<Written> = Vec::new();
    for a in pdf.annotations() {
        if a.contents.trim().is_empty() || a.locked() {
            continue;
        }
        let what = match (&a.kind, a.reply_to) {
            (_, Some(_)) => "Reply",
            (AnnotKind::Note, _) => "Note",
            (AnnotKind::TextBox, _) => "Text box",
            _ => "Comment",
        };
        out.push(Written {
            holder: Holder::Annot(a.id),
            what: what.into(),
            page: a.page,
            bbox: a.bbox,
            text: a.contents,
        });
    }
    for f in pdf.form_fields() {
        let (FieldKind::Text { password, .. }, FieldValue::Text(text)) = (&f.kind, &f.value) else {
            continue;
        };
        let Some(w) = f.widgets.first() else {
            continue;
        };
        if *password || f.read_only || text.trim().is_empty() {
            continue;
        }
        out.push(Written {
            holder: Holder::Field(f.name.clone()),
            what: "Field".into(),
            page: w.page,
            bbox: w.bbox,
            text: text.clone(),
        });
    }
    // By page, the comments before the fields on each, as written.
    out.sort_by_key(|w| w.page);
    out
}

impl Tab {
    /// The words spelt wrongly, worked out again only when the document or the spelling
    /// changed; `None` while the dictionary is being read.
    fn spelling_list(&mut self, ctx: &egui::Context) -> Option<&[Wrong]> {
        let (key, rev) = crate::spell::revision(ctx);
        let now = (self.revision, key, rev, self.spell_view.skipped.len());
        let list = self.spell_view.list.as_ref();
        if list.is_none_or(|(was, _)| *was != now) {
            let mut list = Vec::new();
            for w in written(&self.pdf) {
                let wrong = crate::spell::wrong_in(ctx, &w.text)?;
                for at in wrong {
                    let skip = (w.holder.clone(), at.start, w.text[at.clone()].to_owned());
                    if self.spell_view.skipped.contains(&skip) {
                        continue;
                    }
                    list.push(Wrong {
                        holder: w.holder.clone(),
                        what: w.what.clone(),
                        page: w.page,
                        bbox: w.bbox,
                        text: w.text.clone(),
                        at,
                    });
                }
            }
            self.spell_view.list = Some((now, list));
        }
        self.spell_view.list.as_ref().map(|(_, l)| l.as_slice())
    }

    /// The Spelling view: the word spelt wrongly first in the document, in its sentence,
    /// with what it is in, the suggestions, and what to do with it.
    pub fn show_spelling(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        let muted = theme::muted(ui.visuals().dark_mode);
        let (langs, active) = crate::spell::languages(&ctx);
        if let Some(l) = langs.get(active) {
            ui.label(RichText::new(&l.name).color(muted));
        }
        ui.add_space(6.0);
        if crate::spell::unreadable(&ctx) {
            ui.label(
                "This language's dictionary couldn't be read. Choose another in Spelling's menu.",
            );
            return;
        }
        let Some(list) = self.spelling_list(&ctx) else {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label("Reading the dictionary…");
            });
            return;
        };
        let Some(first) = list.first().cloned() else {
            ui.label("No words spelt wrongly in the comments, text boxes and form fields.");
            self.spell_view.suggestions = None;
            return;
        };
        let left = list.len();
        let shown = self.spell_view.suggestions.as_ref();
        if shown.is_none_or(|(w, _)| *w != first) {
            let got = crate::spell::suggest(&ctx, first.word());
            self.spell_view.suggestions = Some((first.clone(), got));
            self.spell_view.chosen = 0;
            self.show_wrong(&first);
        }
        let page = self.page_name(first.page);
        ui.label(RichText::new(format!("{} on page {page}", first.what)).color(muted));
        ui.add_space(2.0);
        sentence(ui, &first);
        ui.add_space(8.0);
        let suggestions = (self.spell_view.suggestions.as_ref())
            .map(|(_, s)| s.clone())
            .unwrap_or_default();
        let mut change = None;
        if suggestions.is_empty() {
            ui.label(RichText::new("No suggestions").color(muted));
        }
        for (i, s) in suggestions.iter().enumerate() {
            let r = ui.selectable_label(self.spell_view.chosen == i, s);
            if r.clicked() {
                self.spell_view.chosen = i;
            }
            if r.double_clicked() {
                change = Some(false);
            }
        }
        ui.add_space(8.0);
        let some = !suggestions.is_empty();
        ui.horizontal_wrapped(|ui| {
            if ui.add_enabled(some, egui::Button::new("Change")).clicked() {
                change = Some(false);
            }
            if ui
                .add_enabled(some, egui::Button::new("Change all"))
                .clicked()
            {
                change = Some(true);
            }
        });
        ui.horizontal_wrapped(|ui| {
            if ui.button("Ignore").clicked() {
                let skip = (
                    first.holder.clone(),
                    first.at.start,
                    first.word().to_owned(),
                );
                self.spell_view.skipped.insert(skip);
            }
            if ui.button("Ignore all").clicked() {
                crate::spell::ignore_word(&ctx, first.word());
            }
        });
        if ui.button("Add to dictionary").clicked() {
            crate::spell::add_word(&ctx, first.word());
        }
        ui.add_space(8.0);
        let count = match left {
            1 => "1 word to check".to_string(),
            n => format!("{n} words to check"),
        };
        ui.label(RichText::new(count).color(muted));
        let with = suggestions.get(self.spell_view.chosen).cloned();
        if let (Some(all), Some(with)) = (change, with) {
            self.change_spelling(&ctx, &first, &with, all);
        }
    }

    /// The comment or field holding `w` selected and its place shown on its page.
    fn show_wrong(&mut self, w: &Wrong) {
        if let Holder::Annot(id) = w.holder {
            self.ed.selection = Selection::Annot { page: w.page, id };
        }
        self.show_mark(w.page, w.bbox);
    }

    /// `w` changed to `with`; with `all`, every word alike in what was written, in any
    /// capitals, each keeping its first letter's (Word's Change all), as one step to undo.
    fn change_spelling(&mut self, ctx: &egui::Context, w: &Wrong, with: &str, all: bool) {
        let list = self.spelling_list(ctx).map(<[Wrong]>::to_vec);
        let same = |o: &Wrong| o.word().to_lowercase() == w.word().to_lowercase();
        let alike: Vec<Wrong> = match (all, list) {
            (true, Some(list)) => list.into_iter().filter(same).collect(),
            _ => vec![w.clone()],
        };
        // Each holder's text with its words changed, from the last so the places hold.
        let mut texts: Vec<(Holder, String)> = Vec::new();
        for o in alike.iter().rev() {
            let at = texts.iter().position(|(h, _)| *h == o.holder);
            let i = at.unwrap_or_else(|| {
                texts.push((o.holder.clone(), o.text.clone()));
                texts.len() - 1
            });
            let like = first_like(with, o.word());
            texts[i].1.replace_range(o.at.clone(), &like);
        }
        let annots = self.pdf.annotations();
        let changed = alike.len();
        let ok = self.edit(ctx, |pdf| {
            for (holder, text) in &texts {
                match holder {
                    Holder::Annot(id) => {
                        let Some(a) = annots.iter().find(|a| a.id == *id) else {
                            continue;
                        };
                        pdf.set_annotation(*id, a.color, a.opacity, text)?;
                    }
                    Holder::Field(name) => pdf.set_field(name, FieldValue::Text(text.clone()))?,
                }
            }
            Ok(())
        });
        if ok && all {
            self.notice = Some(match changed {
                1 => format!("Changed 1 word to “{with}”"),
                n => format!("Changed {n} words to “{with}”"),
            });
        }
    }
}

/// `with` with its first letter a capital or small as `like`'s is.
fn first_like(with: &str, like: &str) -> String {
    let upper = like.chars().next().is_some_and(char::is_uppercase);
    let mut chars = with.chars();
    let Some(first) = chars.next() else {
        return String::new();
    };
    let rest = chars.as_str();
    match upper {
        true => first.to_uppercase().collect::<String>() + rest,
        false => first.to_lowercase().collect::<String>() + rest,
    }
}

/// The text around `w`, the word itself in bold, wrapped to the view.
fn sentence(ui: &mut egui::Ui, w: &Wrong) {
    const AROUND: usize = 60;
    let before = &w.text[..w.at.start];
    let after = &w.text[w.at.end..];
    let cut = (before.char_indices().rev().nth(AROUND)).map_or(0, |(i, _)| i);
    let end = (after.char_indices().nth(AROUND)).map_or(after.len(), |(i, _)| i);
    let (lead, tail) = (cut > 0, end < after.len());
    let one_line = |s: &str| s.replace(['\n', '\r'], " ");
    let mut job = egui::text::LayoutJob::default();
    let body = egui::TextStyle::Body.resolve(ui.style());
    let color = ui.visuals().text_color();
    let plain = egui::TextFormat::simple(body.clone(), color);
    let strong = egui::TextFormat {
        underline: egui::Stroke::new(1.0, color),
        ..egui::TextFormat::simple(body, ui.visuals().strong_text_color())
    };
    let (lead, tail) = (if lead { "…" } else { "" }, if tail { "…" } else { "" });
    let before = format!("{lead}{}", one_line(&before[cut..]));
    let after = format!("{}{tail}", one_line(&after[..end]));
    job.append(&before, 0.0, plain.clone());
    job.append(w.word(), 0.0, strong);
    job.append(&after, 0.0, plain);
    job.wrap.max_width = ui.available_width();
    ui.label(job);
}

//! Spelling (NEW-06): the dictionary for the document in front, read off the frame; red wavy
//! lines under the words it doesn't know in the box being typed in, as word processors draw
//! them; and, on a right-click on one, its suggestions, Ignore all and Add to dictionary.
//!
//! The state is the context's (as the typing fonts' loads are), so each test's context has its
//! own. Only the box being drawn is checked, and only when its text changed: core's `Speller`
//! keeps each word's answer, so nothing else in the app is gone through as one types.

use eframe::egui::{self, Id, Pos2, Rect, Stroke};
use limarepdf_core::spelling::{self, BuiltIn, Dictionary, Exceptions, Found, Speller};
use std::collections::{HashMap, HashSet};
use std::ops::Range;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

/// Where a language's dictionary comes from.
#[derive(Clone, Debug, PartialEq)]
enum Source {
    BuiltIn(BuiltIn),
    Found(Found),
}

/// A language that can be checked: one of the bundled dictionaries, or one found in the
/// dictionaries folder or the system's.
#[derive(Clone, Debug, PartialEq)]
pub struct Language {
    /// What the setting keeps: a bundled dictionary's language ("en-CA"), or "file:" and a
    /// found one's file name ("file:de_DE").
    pub key: String,
    /// Its name in the Language menu.
    pub name: String,
    /// The language its words are in, as a PDF's `/Lang` names it.
    pub tag: String,
    source: Source,
}

/// What the app sets each frame from its settings and the document in front.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Config {
    /// Check spelling as you type.
    pub on: bool,
    /// The language chosen in the Language menu, by its key.
    pub chosen: Option<String>,
    /// The folder of dictionaries added to LimarePDF; `None` without a settings folder.
    pub folder: Option<PathBuf>,
    /// The language the document in front says it is in (`/Lang`).
    pub document: Option<String>,
}

enum Load {
    Reading,
    Ready(Speller),
    Failed,
}

/// The word under a right-click, for the box's menu.
struct Pick {
    id: Id,
    /// Where it is in the box's text, in characters.
    chars: Range<usize>,
    word: String,
    suggestions: Vec<String>,
}

/// A box's text as last checked, and the words marked in it (in characters).
struct Checked {
    key: String,
    revision: u64,
    text: String,
    marks: Vec<Range<usize>>,
}

#[derive(Default)]
struct State {
    config: Config,
    /// The words added and ignored, given by the app once (`added` from its word list).
    except: Option<Exceptions>,
    /// Words added since the app last saved the word list.
    added_changed: bool,
    /// The languages there are, listed when first asked for and after a dictionary is added.
    languages: Option<Vec<Language>>,
    /// The system's language ("en_CA.UTF-8"), asked once.
    system: Option<Option<String>>,
    loads: HashMap<String, Load>,
    boxes: HashMap<Id, Checked>,
    pick: Option<Pick>,
    /// A suggestion picked for a box: its id, the word's place (characters), the word as it
    /// was, and what replaces it.
    replace: Option<(Id, Range<usize>, String, String)>,
    /// Counts each change to what is wrong (a word added or ignored, another language), so
    /// that marks and Review > Spelling's list are worked out again.
    revision: u64,
    /// What went wrong reading a dictionary, for the app to say.
    failures: Vec<String>,
    /// The marks each box drew last (characters), for tests.
    #[cfg(test)]
    drawn: HashMap<Id, Vec<Range<usize>>>,
}

/// The words the box `id` marked when last drawn (characters); `None` before it marked
/// any, its dictionary not yet read.
#[cfg(test)]
pub fn marked(ctx: &egui::Context, id: Id) -> Option<Vec<Range<usize>>> {
    let s = shared(ctx);
    let drawn = lock(&s).drawn.get(&id).cloned();
    drawn
}

#[derive(Clone, Default)]
struct Shared(Arc<Mutex<State>>);

fn shared(ctx: &egui::Context) -> Arc<Mutex<State>> {
    ctx.data_mut(|d| {
        let s = d.get_temp_mut_or_default::<Shared>(Id::unique("spelling"));
        s.0.clone()
    })
}

fn lock(s: &Mutex<State>) -> MutexGuard<'_, State> {
    s.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Sets what spelling follows this frame. `added` (the word list saved last time) is taken
/// the first time only: from then on the words added are kept here, for [`take_added`].
pub fn configure(ctx: &egui::Context, config: Config, added: impl FnOnce() -> HashSet<String>) {
    let s = shared(ctx);
    let mut s = lock(&s);
    if s.except.is_none() {
        s.except = Some(Exceptions {
            added: added(),
            ignored: HashSet::new(),
        });
    }
    if s.config != config {
        if s.config.folder != config.folder {
            s.languages = None;
        }
        s.config = config;
        s.revision += 1;
    }
}

/// The words in the dictionary of one's own, when one was added since this was last asked.
pub fn take_added(ctx: &egui::Context) -> Option<HashSet<String>> {
    let s = shared(ctx);
    let mut s = lock(&s);
    if !std::mem::take(&mut s.added_changed) {
        return None;
    }
    s.except.as_ref().map(|e| e.added.clone())
}

/// What went wrong reading dictionaries since this was last asked, a sentence each.
pub fn take_failures(ctx: &egui::Context) -> Vec<String> {
    let s = shared(ctx);
    let failures = std::mem::take(&mut lock(&s).failures);
    failures
}

/// The dictionaries listed again, after one is added to the folder.
pub fn list_again(ctx: &egui::Context) {
    let s = shared(ctx);
    let mut s = lock(&s);
    s.languages = None;
    s.revision += 1;
}

impl State {
    fn languages(&mut self) -> &[Language] {
        let folder = self.config.folder.clone();
        self.languages.get_or_insert_with(|| {
            let mut out: Vec<Language> = (spelling::BUILT_IN.iter())
                .map(|b| Language {
                    key: b.tag.into(),
                    name: b.name.into(),
                    tag: b.tag.into(),
                    source: Source::BuiltIn(*b),
                })
                .collect();
            let own = folder.map(|f| spelling::find(&[f])).unwrap_or_default();
            // The system's, but not those LimarePDF has its own of (the same SCOWL lists).
            let system = spelling::find(&spelling::system_folders());
            let bundled = |f: &Found| spelling::BUILT_IN.iter().any(|b| b.tag == f.tag);
            let system = system.into_iter().filter(|f| !bundled(f));
            for f in own.into_iter().chain(system) {
                let key = format!("file:{}", f.stem);
                if out.iter().any(|l| l.key == key) {
                    continue;
                }
                let mut name = f.name.clone();
                if out.iter().any(|l| l.name == name) {
                    name = format!("{name} – {}", f.stem);
                }
                out.push(Language {
                    key,
                    name,
                    tag: f.tag.clone(),
                    source: Source::Found(f),
                });
            }
            out
        })
    }

    /// The language checked: the one chosen in the Language menu; before any is chosen, the
    /// document's, then the system's, then US English.
    fn active(&mut self) -> Language {
        let chosen = self.config.chosen.clone();
        let document = self.config.document.clone();
        let system = self
            .system
            .get_or_insert_with(sys_locale::get_locale)
            .clone();
        let langs = self.languages();
        let tags = || langs.iter().map(|l| l.tag.as_str());
        let chosen = chosen.and_then(|c| langs.iter().position(|l| l.key == c));
        let i = (chosen)
            .or_else(|| document.and_then(|d| spelling::best_match(&d, tags())))
            .or_else(|| system.and_then(|s| spelling::best_match(&s, tags())))
            .or_else(|| langs.iter().position(|l| l.key == "en-US"))
            .unwrap_or(0);
        langs[i].clone()
    }

    /// The speller for `lang`, once its dictionary is read; the first ask starts reading it
    /// on a thread of its own, which asks for a frame when it is done.
    fn speller(
        &mut self,
        ctx: &egui::Context,
        me: &Arc<Mutex<State>>,
        lang: &Language,
    ) -> Option<&mut Speller> {
        if !self.loads.contains_key(&lang.key) {
            self.loads.insert(lang.key.clone(), Load::Reading);
            let (ctx, me, lang) = (ctx.clone(), me.clone(), lang.clone());
            std::thread::spawn(move || {
                let read = match &lang.source {
                    Source::BuiltIn(b) => Dictionary::built_in(*b),
                    Source::Found(f) => Dictionary::from_files(&f.aff, &f.dic),
                };
                let mut s = lock(&me);
                let load = match read {
                    Ok(d) => Load::Ready(Speller::new(Arc::new(d))),
                    Err(e) => {
                        s.failures.push(format!(
                            "LimarePDF couldn't use the {} dictionary: {e}. Spelling isn't \
                             checked in it.",
                            lang.name
                        ));
                        Load::Failed
                    }
                };
                s.loads.insert(lang.key, load);
                s.revision += 1;
                drop(s);
                ctx.request_repaint();
            });
        }
        match self.loads.get_mut(&lang.key) {
            Some(Load::Ready(s)) => Some(s),
            _ => None,
        }
    }
}

/// Draws the marks under the words spelt wrongly in the box `id`, as its output `out` shows
/// `text`, and keeps the word under a right-click for the box's menu ([`menu`]). The word
/// being typed (the caret just after it) isn't marked until it is left, as in word
/// processors. Nothing while the dictionary is being read, or with spelling turned off.
pub fn marks(ui: &egui::Ui, id: Id, out: &egui::text_edit::TextEditOutput, text: &str) {
    let ctx = ui.ctx();
    let me = shared(ctx);
    let mut s = lock(&me);
    let right = out.response.response.secondary_clicked();
    if right {
        s.pick = None;
    }
    if !s.config.on {
        s.boxes.remove(&id);
        #[cfg(test)]
        s.drawn.insert(id, Vec::new());
        return;
    }
    let lang = s.active();
    let revision = s.revision;
    let fresh = (s.boxes.get(&id))
        .is_some_and(|c| c.key == lang.key && c.revision == revision && c.text == text);
    if !fresh {
        let except = s.except.clone().unwrap_or_default();
        let Some(speller) = s.speller(ctx, &me, &lang) else {
            #[cfg(test)]
            s.drawn.remove(&id);
            return;
        };
        let marks = to_chars(text, speller.misspelt(text, &except));
        let checked = Checked {
            key: lang.key.clone(),
            revision,
            text: text.to_owned(),
            marks,
        };
        s.boxes.insert(id, checked);
    }
    let marks = s.boxes[&id].marks.clone();
    let caret = (out.response.response.has_focus())
        .then_some(out.cursor_range)
        .flatten()
        .filter(|c| c.is_empty())
        .map(|c| c.primary.index.0);
    let ink = crate::theme::tone(ui.visuals().dark_mode, crate::theme::Tone::Bad)
        .unwrap_or(egui::Color32::RED);
    let shown: Vec<Range<usize>> = (marks.iter())
        .filter(|m| Some(m.end) != caret)
        .cloned()
        .collect();
    for m in &shown {
        for (from, to, y, height) in rows_of(&out.galley, m.clone()) {
            let at = out.galley_pos.to_vec2();
            wave(ui.painter(), from + at.x, to + at.x, y + at.y, height, ink);
        }
    }
    #[cfg(test)]
    s.drawn.insert(id, shown);
    let pointer = out.response.response.interact_pointer_pos();
    let (true, Some(p)) = (right, pointer) else {
        return;
    };
    let at = out.galley.cursor_from_pos(p - out.galley_pos).index.0;
    let Some(m) = marks.into_iter().find(|m| m.start <= at && at <= m.end) else {
        return;
    };
    let word: String = text.chars().skip(m.start).take(m.len()).collect();
    let suggestions = match s.loads.get(&lang.key) {
        Some(Load::Ready(sp)) => sp.suggest(&word),
        _ => Vec::new(),
    };
    s.pick = Some(Pick {
        id,
        chars: m,
        word,
        suggestions,
    });
}

/// Byte ranges of `text` as character ranges.
fn to_chars(text: &str, ranges: Vec<Range<usize>>) -> Vec<Range<usize>> {
    let at = |b: usize| text[..b].chars().count();
    ranges.into_iter().map(|r| at(r.start)..at(r.end)).collect()
}

/// Where the characters `chars` are in `galley`, a piece a row: from, to, the row's bottom
/// and its height, in the galley's points.
fn rows_of(galley: &egui::Galley, chars: Range<usize>) -> Vec<(f32, f32, f32, f32)> {
    use egui::epaint::text::CharIndex;
    use egui::text::CCursor;
    let mut out: Vec<(f32, f32, f32, f32)> = Vec::new();
    for i in chars {
        let start = galley.pos_from_cursor(CCursor {
            index: CharIndex(i),
            prefer_next_row: true,
        });
        let end = galley.pos_from_cursor(CCursor::new(CharIndex(i + 1)));
        let row: Rect = start;
        match out.last_mut() {
            Some(last) if last.2 == row.max.y && end.min.y == row.min.y => last.1 = end.min.x,
            _ if end.min.y == row.min.y => {
                out.push((start.min.x, end.min.x, row.max.y, row.height()));
            }
            _ => {}
        }
    }
    out
}

/// A wavy line from `x0` to `x1` just above `y` (a row's bottom), its waves sized to the
/// row's `height`, as word processors draw a word spelt wrongly.
fn wave(painter: &egui::Painter, x0: f32, x1: f32, y: f32, height: f32, ink: egui::Color32) {
    let amp = (height * 0.06).clamp(1.0, 2.5);
    let step = amp * 1.5;
    let y = y - amp - 0.5;
    let mut points = Vec::new();
    let mut x = x0;
    let mut up = true;
    while x < x1 {
        points.push(Pos2::new(x, if up { y - amp } else { y + amp }));
        x += step;
        up = !up;
    }
    points.push(Pos2::new(x1, if up { y - amp } else { y + amp }));
    if points.len() >= 2 {
        painter.add(egui::Shape::line(points, Stroke::new(1.0, ink)));
    }
}

/// Whether the word under the latest right-click is in the box `id`.
pub fn picked(ctx: &egui::Context, id: Id) -> bool {
    let s = shared(ctx);
    let picked = lock(&s).pick.as_ref().is_some_and(|p| p.id == id);
    picked
}

/// The top of a box's right-click menu, over a word spelt wrongly in the box `id`: its
/// suggestions, then Ignore all and Add to dictionary, as word processors put them. Whether
/// it drew anything (for a separator after it).
pub fn menu(ui: &mut egui::Ui, id: Id) -> bool {
    let me = shared(ui.ctx());
    let mut s = lock(&me);
    let Some(p) = s.pick.as_ref().filter(|p| p.id == id) else {
        return false;
    };
    let (word, chars, suggestions) = (p.word.clone(), p.chars.clone(), p.suggestions.clone());
    if suggestions.is_empty() {
        ui.add_enabled(false, egui::Button::new("No suggestions"));
    }
    for w in suggestions {
        if ui.button(egui::RichText::new(&w).strong()).clicked() {
            s.replace = Some((id, chars.clone(), word.clone(), w));
            s.pick = None;
            ui.ctx().request_repaint();
            ui.close();
        }
    }
    ui.separator();
    if ui.button("Ignore all").clicked() {
        ignore(&mut s, &word);
        ui.close();
    }
    if ui.button("Add to dictionary").clicked() {
        add(&mut s, &word);
        ui.close();
    }
    true
}

/// The right-click menu of a box that has none of its own (a comment's box in the inspector,
/// a reply's, a form field's): the spelling menu, over a word spelt wrongly.
pub fn menu_on(r: &egui::Response, id: Id) {
    if picked(&r.ctx, id) {
        r.context_menu(|ui| {
            menu(ui, id);
        });
    }
}

fn ignore(s: &mut State, word: &str) {
    let except = s.except.get_or_insert_with(Exceptions::default);
    except.ignored.insert(spelling::straight(word));
    s.pick = None;
    s.revision += 1;
}

fn add(s: &mut State, word: &str) {
    let except = s.except.get_or_insert_with(Exceptions::default);
    except.added.insert(spelling::straight(word));
    s.added_changed = true;
    s.pick = None;
    s.revision += 1;
}

/// Ignore all for `word`: from Review > Spelling.
pub fn ignore_word(ctx: &egui::Context, word: &str) {
    let s = shared(ctx);
    ignore(&mut lock(&s), word);
}

/// Add to dictionary for `word`: from Review > Spelling.
pub fn add_word(ctx: &egui::Context, word: &str) {
    let s = shared(ctx);
    add(&mut lock(&s), word);
}

/// A suggestion picked from the box `id`'s menu, put in its `text` as typing would put it,
/// the caret after it: asked before the box is drawn. Whether the text changed.
pub fn apply(ctx: &egui::Context, id: Id, text: &mut String) -> bool {
    let s = shared(ctx);
    let mut s = lock(&s);
    let Some((_, chars, was, with)) = s.replace.take_if(|r| r.0 == id) else {
        return false;
    };
    drop(s);
    let byte = |c: usize| text.char_indices().nth(c).map_or(text.len(), |(b, _)| b);
    let (from, to) = (byte(chars.start), byte(chars.end));
    // The word as it was right-clicked, or nothing: the text changed since.
    if text.get(from..to) != Some(was.as_str()) {
        return false;
    }
    text.replace_range(from..to, &with);
    let end = chars.start + with.chars().count();
    crate::page_text::select(ctx, id, (end, end));
    true
}

/// The languages there are, in the Language menu's order, and which is checked.
pub fn languages(ctx: &egui::Context) -> (Vec<Language>, usize) {
    let s = shared(ctx);
    let mut s = lock(&s);
    let active = s.active();
    let langs = s.languages().to_vec();
    let i = langs.iter().position(|l| l.key == active.key).unwrap_or(0);
    (langs, i)
}

/// What is spelt wrongly in `text` (bytes), in the language checked; `None` while its
/// dictionary is being read, or when it can't be.
pub fn wrong_in(ctx: &egui::Context, text: &str) -> Option<Vec<Range<usize>>> {
    let me = shared(ctx);
    let mut s = lock(&me);
    let lang = s.active();
    let except = s.except.clone().unwrap_or_default();
    let speller = s.speller(ctx, &me, &lang)?;
    Some(speller.misspelt(text, &except))
}

/// Suggestions for `word` in the language checked.
pub fn suggest(ctx: &egui::Context, word: &str) -> Vec<String> {
    let me = shared(ctx);
    let mut s = lock(&me);
    let lang = s.active();
    s.speller(ctx, &me, &lang)
        .map(|sp| sp.suggest(word))
        .unwrap_or_default()
}

/// What changes what is wrong: the language checked and the words added or ignored.
pub fn revision(ctx: &egui::Context) -> (String, u64) {
    let s = shared(ctx);
    let mut s = lock(&s);
    let key = s.active().key;
    (key, s.revision)
}

/// Whether the language checked has a dictionary that couldn't be read.
pub fn unreadable(ctx: &egui::Context) -> bool {
    let s = shared(ctx);
    let mut s = lock(&s);
    let key = s.active().key;
    matches!(s.loads.get(&key), Some(Load::Failed))
}

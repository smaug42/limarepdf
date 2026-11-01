//! Search as you type: a tab's searches run on a thread of its own, after a short pause in
//! the typing, over the document's text read once ([`TextIndex`]) and kept until it
//! changes. A newer search stops an older one, and what comes back is numbered, so matches
//! for an older query, or an older state of the document, are never shown.

use eframe::egui;
use limarepdf_core::{Pdf, Query, SearchHit, SearchOptions, TextIndex};
use std::collections::BTreeSet;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError, Weak};

/// How long typing pauses before the search runs, in seconds.
pub const PAUSE: f64 = 0.2;
/// How many pages are read and searched at a time; what they hold is sent after each group.
const GROUP: usize = 16;
/// The most memory every tab's search text may hold together, in bytes.
const BUDGET: usize = 256 << 20;

/// A search for the thread: its number, the query, and the document when its text changed
/// since the thread last had it, with the pages that changed (`None`: any).
struct Job {
    n: u64,
    query: Query,
    doc: Option<(Pdf, Option<Vec<usize>>)>,
}

/// What the search thread sends back about search number `n`.
pub enum Found {
    /// The matches on the pages before `upto` (0-based) not sent before, of `of` pages.
    Some {
        n: u64,
        hits: Vec<SearchHit>,
        upto: usize,
        of: usize,
    },
    /// Every page is searched.
    Done { n: u64 },
    /// The search stopped with an internal error: why.
    Failed { n: u64, why: String },
}

impl Found {
    fn n(&self) -> u64 {
        match self {
            Found::Some { n, .. } | Found::Done { n } | Found::Failed { n, .. } => *n,
        }
    }
}

struct Thread {
    tx: Sender<Job>,
    rx: Receiver<Found>,
    /// The number of the newest search: the thread stops one whose number it isn't.
    latest: Arc<AtomicU64>,
}

/// A tab's search: its thread, started with the first search, and where the search the tab
/// shows stands.
pub struct Search {
    ctx: egui::Context,
    thread: Option<Thread>,
    next: u64,
    /// The query and its options as the last frame had them, to notice typing.
    pub seen: (String, SearchOptions),
    /// The page being read when the query was begun: typing goes to the first match from it.
    pub anchor: usize,
    /// When the query last changed without being searched yet (`egui` time).
    pub typed: Option<f64>,
    /// The search shown: its number, query and options.
    pub asked: Option<(u64, String, SearchOptions)>,
    /// The document's text changed since the thread last had it: the pages that did, or
    /// `None` for any.
    changed: Option<Option<BTreeSet<usize>>>,
    /// While the search shown runs: the pages searched so far, and how many there are.
    pub progress: Option<(usize, usize)>,
    /// The search shown stopped with an internal error: why.
    pub failed: Option<String>,
    /// Scroll Search results to the current match's row.
    pub reveal: bool,
}

impl Search {
    pub fn new(ctx: &egui::Context) -> Self {
        Search {
            ctx: ctx.clone(),
            thread: None,
            next: 0,
            seen: Default::default(),
            anchor: 0,
            typed: None,
            asked: None,
            changed: Some(None),
            progress: None,
            failed: None,
            reveal: false,
        }
    }

    /// Whether the search shown is for an older state of the document's text, and no
    /// typing is waiting to search anyway.
    pub fn stale(&self) -> bool {
        self.asked.is_some() && self.changed.is_some() && self.typed.is_none()
    }

    /// The document's text changed: on `pages` (0-based), or on any page when `None`.
    pub fn text_changed(&mut self, pages: Option<&[usize]>) {
        match (&mut self.changed, pages) {
            (Some(None), _) => {}
            (Some(Some(had)), Some(p)) => had.extend(p),
            (c, None) => *c = Some(None),
            (c @ None, Some(p)) => *c = Some(Some(p.iter().copied().collect())),
        }
    }

    /// Whether the search shown is for `query` with `options` on the document as it is
    /// now, finished or still running, with no typing waiting to search.
    pub fn shows(&self, query: &str, options: SearchOptions) -> bool {
        self.asked
            .as_ref()
            .is_some_and(|(_, q, o)| q == query && *o == options)
            && self.failed.is_none()
            && self.typed.is_none()
            && self.changed.is_none()
    }

    /// Whether the search shown is the finished search for `query` with `options` on the
    /// document as it is now.
    pub fn complete(&self, query: &str, options: SearchOptions) -> bool {
        self.shows(query, options) && self.progress.is_none()
    }

    /// Start searching `pdf` for `query` with `options`, in place of any search running.
    /// The document goes to the thread when its text changed since the thread last had it.
    pub fn start(&mut self, pdf: &Pdf, query: &str, options: SearchOptions) {
        self.next += 1;
        let n = self.next;
        let job = Job {
            n,
            query: Query::with(query, options),
            doc: self
                .changed
                .take()
                .map(|c| (pdf.clone(), c.map(|s| s.into_iter().collect()))),
        };
        let thread = self.thread.get_or_insert_with(|| spawn(&self.ctx));
        thread.latest.store(n, Ordering::Relaxed);
        if let Err(job) = thread.tx.send(job) {
            // The thread is gone: a new one has nothing, so it gets the whole document.
            let mut job = job.0;
            if job.doc.is_none() {
                job.doc = Some((pdf.clone(), None));
            }
            let thread = self.thread.insert(spawn(&self.ctx));
            thread.latest.store(n, Ordering::Relaxed);
            let _ = thread.tx.send(job);
        }
        self.asked = Some((n, query.to_owned(), options));
        self.progress = Some((0, pdf.page_count()));
        self.failed = None;
        self.typed = None;
    }

    /// Stop the search running, if any, and show none.
    pub fn stop(&mut self) {
        self.next += 1;
        if let Some(t) = &self.thread {
            t.latest.store(self.next, Ordering::Relaxed);
        }
        self.asked = None;
        self.progress = None;
        self.failed = None;
        self.typed = None;
    }

    /// What the thread found for the search shown since the last call, in order; anything
    /// about other searches is dropped.
    pub fn take(&mut self) -> Vec<Found> {
        let Some(t) = &self.thread else {
            return Vec::new();
        };
        let shown = self.asked.as_ref().map(|a| a.0);
        let found: Vec<Found> = t.rx.try_iter().filter(|f| Some(f.n()) == shown).collect();
        for f in &found {
            match f {
                Found::Some { upto, of, .. } => self.progress = Some((*upto, *of)),
                Found::Done { .. } => self.progress = None,
                Found::Failed { why, .. } => {
                    self.progress = None;
                    self.failed = Some(why.clone());
                }
            }
        }
        found
    }
}

fn spawn(ctx: &egui::Context) -> Thread {
    let (tx, jobs) = channel();
    let (found, rx) = channel();
    let latest = Arc::new(AtomicU64::new(0));
    let (l, ctx) = (latest.clone(), ctx.clone());
    std::thread::spawn(move || run(jobs, found, l, ctx));
    Thread { tx, rx, latest }
}

/// The search thread: runs each newest search until the tab goes.
fn run(jobs: Receiver<Job>, tx: Sender<Found>, latest: Arc<AtomicU64>, ctx: egui::Context) {
    let index = Arc::new(Mutex::new(TextIndex::default()));
    lock(&HELD).push(Held {
        index: Arc::downgrade(&index),
        bytes: 0,
        used: 0,
    });
    let mut doc: Option<Pdf> = None;
    while let Ok(mut job) = jobs.recv() {
        // Only the newest search matters; the document of one passed over is still taken.
        while let Ok(newer) = jobs.try_recv() {
            job = Job {
                doc: merge(job.doc, newer.doc),
                ..newer
            };
        }
        let n = job.n;
        let ran = crate::guard(|| {
            search(job, &mut doc, &index, &latest, &tx, &ctx);
            Ok(())
        });
        if let Err(e) = ran {
            // Read everything again next time, rather than trust what was half done.
            *lock(&index) = doc.as_ref().map(TextIndex::new).unwrap_or_default();
            let _ = tx.send(Found::Failed {
                n,
                why: format!("{e:#}"),
            });
            ctx.request_repaint();
        }
    }
}

/// The document of two jobs, the older first: the newer one's, with the pages changed in
/// either.
type Doc = Option<(Pdf, Option<Vec<usize>>)>;
fn merge(older: Doc, newer: Doc) -> Doc {
    match (older, newer) {
        (Some((_, a)), Some((pdf, b))) => {
            let pages = a.zip(b).map(|(mut a, b)| {
                a.extend(b);
                a
            });
            Some((pdf, pages))
        }
        (older, None) => older,
        (None, newer) => newer,
    }
}

/// Run search `job` over `doc` (the document, replaced by the job's when it has one),
/// sending the matches a group of pages at a time, until it is done or a newer search comes.
fn search(
    job: Job,
    doc: &mut Option<Pdf>,
    index: &Arc<Mutex<TextIndex>>,
    latest: &AtomicU64,
    tx: &Sender<Found>,
    ctx: &egui::Context,
) {
    let mut ix = lock(index);
    if let Some((pdf, changed)) = job.doc {
        ix.update(&pdf, changed.as_deref());
        *doc = Some(pdf);
    }
    let Some(pdf) = doc.as_ref() else {
        return;
    };
    let n = job.n;
    let stop = || latest.load(Ordering::Relaxed) != n;
    let pages = ix.page_count();
    let (mut start, mut spill) = (0, 0);
    while start < pages {
        let end = (start + GROUP).min(pages);
        // The page after the group too, for a phrase across the page break.
        if !ix.read(pdf, start..(end + 1).min(pages), &stop) {
            return;
        }
        let m = ix.find(&job.query, start..end, spill);
        spill = m.spill;
        fit(index, &mut ix, start);
        let some = Found::Some {
            n,
            hits: m.hits,
            upto: end,
            of: pages,
        };
        if tx.send(some).is_err() || stop() {
            return;
        }
        ctx.request_repaint();
        start = end;
    }
    crate::timing::mark("search index", || format!("{} bytes", ix.bytes()));
    let _ = tx.send(Found::Done { n });
    ctx.request_repaint();
}

/// A search thread's index, for keeping them all within [`BUDGET`]: how much it held when
/// last counted, and when it was last searched (a count of searches).
struct Held {
    index: Weak<Mutex<TextIndex>>,
    bytes: usize,
    used: u64,
}

static HELD: Mutex<Vec<Held>> = Mutex::new(Vec::new());
static TICKS: AtomicU64 = AtomicU64::new(0);

/// Keep every tab's search text within [`BUDGET`] after this thread's index (`mine`, locked
/// from `me`) has grown: see [`fit_within`].
fn fit(me: &Arc<Mutex<TextIndex>>, mine: &mut TextIndex, searched: usize) {
    fit_within(&mut lock(&HELD), me, mine, searched, BUDGET);
}

/// Keep the search text of the threads `held` within `budget` bytes after this thread's
/// index (`mine`, locked from `me`) has grown: the other indexes searched longest ago are
/// let go first, then, if this one alone is too big, its pages before `searched` (already
/// searched).
fn fit_within(
    held: &mut Vec<Held>,
    me: &Arc<Mutex<TextIndex>>,
    mine: &mut TextIndex,
    searched: usize,
    budget: usize,
) {
    held.retain(|h| h.index.strong_count() > 0);
    let is_me = |h: &Held| std::ptr::eq(h.index.as_ptr(), Arc::as_ptr(me));
    let tick = TICKS.fetch_add(1, Ordering::Relaxed) + 1;
    for h in held.iter_mut().filter(|h| is_me(h)) {
        (h.bytes, h.used) = (mine.bytes(), tick);
    }
    let mut total: usize = held.iter().map(|h| h.bytes).sum();
    held.sort_by_key(|h| h.used);
    for h in held.iter_mut() {
        if total <= budget {
            return;
        }
        if is_me(h) {
            continue;
        }
        // One being searched is busy; it counts itself when it is done.
        if let Some(other) = h.index.upgrade() {
            if let Ok(mut i) = other.try_lock() {
                let n = i.page_count();
                i.forget(0..n);
                total -= h.bytes;
                h.bytes = 0;
            }
        }
    }
    if total > budget {
        mine.forget(0..searched);
        for h in held.iter_mut().filter(|h| is_me(h)) {
            h.bytes = mine.bytes();
        }
    }
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pages_changed_add_up_until_the_thread_has_the_document() {
        let mut s = Search::new(&egui::Context::default());
        // A new tab's thread has nothing yet: the whole document goes.
        assert_eq!(s.changed, Some(None));
        s.changed = None;
        s.text_changed(Some(&[2]));
        s.text_changed(Some(&[5, 2]));
        assert_eq!(s.changed, Some(Some(BTreeSet::from([2, 5]))));
        s.text_changed(None);
        s.text_changed(Some(&[1]));
        assert_eq!(s.changed, Some(None));
    }

    /// Two lines a page, "walrus" on the first and "otter" on the second, `n` times.
    fn animals(n: usize) -> Pdf {
        let text = "the walrus ate fish\nthe otter swam\n".repeat(n);
        Pdf::from_text(&text, (400.0, 65.0)).unwrap()
    }

    /// What the search shown finds, once it is done.
    fn all_found(s: &mut Search) -> Vec<SearchHit> {
        let mut hits = Vec::new();
        for _ in 0..1000 {
            for f in s.take() {
                match f {
                    Found::Some { hits: h, .. } => hits.extend(h),
                    Found::Done { .. } => return hits,
                    Found::Failed { why, .. } => panic!("{why}"),
                }
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        panic!("the search didn't finish");
    }

    /// ENH-02: a search started while another runs takes its place: only its matches come.
    #[test]
    fn a_newer_search_replaces_an_older_one() {
        let pdf = animals(300);
        let mut s = Search::new(&egui::Context::default());
        s.start(&pdf, "walrus", ANY);
        s.start(&pdf, "otter", ANY);
        let hits = all_found(&mut s);
        assert_eq!(hits.len(), 300);
        assert!(hits.iter().all(|h| h.snippet.found == "otter"));
        assert!(s.complete("otter", ANY));
    }

    const ANY: SearchOptions = SearchOptions {
        match_case: false,
        whole_words: false,
    };

    /// NEW-04a: the thread searches with the options asked, and a search shown is for its
    /// query with its options alone.
    #[test]
    fn a_search_is_for_its_options_too() {
        let pdf = Pdf::from_text("the Otter and the otters", (400.0, 65.0)).unwrap();
        let mut s = Search::new(&egui::Context::default());
        let case = SearchOptions {
            match_case: true,
            ..ANY
        };
        s.start(&pdf, "Otter", case);
        assert_eq!(all_found(&mut s).len(), 1);
        assert!(s.complete("Otter", case) && !s.complete("Otter", ANY));
        let whole = SearchOptions {
            whole_words: true,
            ..ANY
        };
        s.start(&pdf, "otter", whole);
        let hits = all_found(&mut s);
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].snippet.found, "Otter");
        s.start(&pdf, "otter", ANY);
        assert_eq!(all_found(&mut s).len(), 2);
    }

    /// Past the budget, the other tabs' search text goes first, then this one's pages
    /// already searched.
    #[test]
    fn search_text_is_kept_within_the_budget() {
        let pdf = animals(4);
        let read = || {
            let mut ix = TextIndex::new(&pdf);
            assert!(ix.read(&pdf, 0..4, &|| false));
            Arc::new(Mutex::new(ix))
        };
        let (other, me) = (read(), read());
        let mut held: Vec<Held> = [&other, &me]
            .into_iter()
            .map(|ix| Held {
                index: Arc::downgrade(ix),
                bytes: lock(ix).bytes(),
                used: 0,
            })
            .collect();
        let mut mine = lock(&me);
        let size = mine.bytes();
        // Room for this tab's text alone: the other's goes.
        fit_within(&mut held, &me, &mut mine, 2, size);
        assert!((0..4).all(|p| !lock(&other).has(p)));
        assert!((0..4).all(|p| mine.has(p)));
        // No room even for that: its pages searched already go too.
        fit_within(&mut held, &me, &mut mine, 2, 1);
        assert_eq!(
            (0..4).map(|p| mine.has(p)).collect::<Vec<_>>(),
            [false, false, true, true]
        );
    }

    /// ENH-02: what an older search found and sent is never taken, even when it is waiting.
    #[test]
    fn what_an_older_search_found_is_never_shown() {
        let pdf = animals(3);
        let mut s = Search::new(&egui::Context::default());
        s.start(&pdf, "walrus", ANY);
        // It is done by now, its matches waiting to be taken.
        std::thread::sleep(std::time::Duration::from_millis(300));
        s.start(&pdf, "otter", ANY);
        let hits = all_found(&mut s);
        assert_eq!(hits.len(), 3);
        assert!(hits.iter().all(|h| h.snippet.found == "otter"));
        // A search stopped shows nothing more.
        s.start(&pdf, "fish", ANY);
        s.stop();
        std::thread::sleep(std::time::Duration::from_millis(100));
        assert!(s.take().is_empty() && s.asked.is_none());
    }

    #[test]
    fn a_passed_over_job_still_brings_its_document() {
        let (a, b) = (Pdf::new(), Pdf::new());
        let m = merge(Some((a, Some(vec![1]))), None).unwrap();
        assert_eq!(m.1, Some(vec![1]));
        let m = merge(Some((m.0, Some(vec![1]))), Some((b, Some(vec![3])))).unwrap();
        assert_eq!(m.1, Some(vec![1, 3]));
        let m = merge(Some((m.0, None)), Some((Pdf::new(), Some(vec![3])))).unwrap();
        assert_eq!(m.1, None);
    }
}

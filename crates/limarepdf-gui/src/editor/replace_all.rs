//! Replace all (NEW-04b, batch 46's stage 3): every match of a search replaced at once on a
//! copy of the document, each as Replace types it over its match ([`Editor::replace`]): in
//! its line, or across a paragraph's lines (one LimarePDF is sure of, or any with paragraph
//! editing on) in its paragraph, set again by the line and paragraph writers. What Replace
//! leaves as it was is left so here, and counted. Each line and paragraph changed is then
//! judged by the one overflow rule ([`page_text::overflow`]) where the copy has it, and its
//! replacements' fonts read, for the review before the document takes the copy as one
//! change. A replacement holding what it replaces is replaced once only, as the matches are
//! those of one search.

use super::paragraph::{refused, PARAGRAPH_DEFAULT};
use super::replace::{lines_of, page_of, typed_font, with_a_space, MatchLines};
use super::replace::{GROUPED, HIDDEN, ON_TWO_PAGES, VERTICAL};
use super::*;
use limarepdf_core::{Block, HitPart, Paragraph, Pdf, SetLine};
use rayon::prelude::*;
use std::collections::{HashSet, VecDeque};

/// Why Replace all leaves a match as it was, in the order the review says them.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum LeftOut {
    /// It runs on to the next page.
    NextPage,
    /// It runs across paragraphs, or across lines LimarePDF doesn't edit as one.
    AcrossParagraphs,
    /// Invisible recognised text (batch 46's Q4).
    Hidden,
    /// Text inside a group (a form XObject), which is no line of its own.
    Grouped,
    /// Vertical writing.
    Vertical,
    /// The replacement has a character LimarePDF can't write in the font it goes into.
    Unwritable,
    /// Its line or paragraph couldn't be written again.
    Failed,
}

impl LeftOut {
    /// What the review says of `n` matches left for this reason, after their number.
    pub fn said(self, n: usize) -> &'static str {
        let (one, many) = match self {
            LeftOut::NextPage => ("runs on to the next page", "run on to the next page"),
            LeftOut::AcrossParagraphs => ("runs across paragraphs", "run across paragraphs"),
            LeftOut::Hidden => (
                "is invisible recognised text",
                "are invisible recognised text",
            ),
            LeftOut::Grouped => ("is text inside a group", "are text inside a group"),
            LeftOut::Vertical => ("is in vertical writing", "are in vertical writing"),
            LeftOut::Unwritable => (
                "needs a character LimarePDF can't write there",
                "need a character LimarePDF can't write there",
            ),
            LeftOut::Failed => (
                "is on a line LimarePDF couldn't write again",
                "are on lines LimarePDF couldn't write again",
            ),
        };
        if n == 1 {
            one
        } else {
            many
        }
    }

    /// The reason for Replace's note `why` ([`super::replace::lines_of`]).
    fn of(why: &'static str) -> LeftOut {
        match why {
            ON_TWO_PAGES => LeftOut::NextPage,
            HIDDEN => LeftOut::Hidden,
            GROUPED => LeftOut::Grouped,
            VERTICAL => LeftOut::Vertical,
            _ => LeftOut::AcrossParagraphs,
        }
    }
}

/// What Replace all is asked: the document as it is, each match's parts in the search's
/// order, what replaces them, and how text opens for editing.
pub struct AllJob {
    pub pdf: Pdf,
    pub matches: Vec<Vec<HitPart>>,
    /// The replacement, on one line.
    pub with: String,
    /// Paragraph editing is on ([`Editor::paragraphs`]).
    pub paragraphs: bool,
    /// The document's words, for the hyphens at a paragraph's line ends.
    pub words: Option<Arc<DocWords>>,
}

/// A copy of the document with replacements made, and what they did.
pub struct Pass {
    pub pdf: Pdf,
    /// Matches replaced, and the pages (0-based, in order) they are on.
    pub replaced: usize,
    pub pages: Vec<usize>,
    /// Of those, the matches whose line or paragraph runs past the page's edge or over
    /// other text, a picture or a shape there (the one overflow rule), and their pages.
    pub over: usize,
    pub over_pages: Vec<usize>,
    /// Of those, the matches whose replacement is written in a font other than its text's
    /// own, as that has no letters for it.
    pub other_font: usize,
    /// The matches left as they were, how many for each reason, in [`LeftOut`]'s order.
    pub left: Vec<(LeftOut, usize)>,
    /// The matches left as they were as they don't fit (Replace where it fits).
    pub unfit: usize,
    /// Words beside a replacement couldn't keep their own font and are in a similar one.
    pub similar: bool,
    /// The changes that run over, to leave out where it fits.
    over_changes: HashSet<Key>,
}

/// Replace all prepared: every replacement made, and, when some don't fit and others do,
/// those that fit alone.
pub struct Prepared {
    pub all: Pass,
    pub fits: Option<Pass>,
}

/// Make every replacement of `job` on a copy of its document, then, when some don't fit
/// and others do, on another copy those that fit.
pub fn replace_all(job: &AllJob) -> Prepared {
    let (plan, left) = plan(job);
    let all = settle(job, &plan, &left, &HashSet::new());
    let fits = (all.over > 0 && all.over < all.replaced)
        .then(|| settle(job, &plan, &left, &all.over_changes));
    Prepared { all, fits }
}

/// What a change is of: a line (by its object id) or a paragraph (by its block's place
/// among the page's blocks).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum Of {
    Line(usize),
    Paragraph(usize),
}

/// A change by its page.
type Key = (usize, Of);

/// A line or paragraph changed, and the matches in it, each by its lines.
struct Change {
    of: Of,
    matches: Vec<MatchLines>,
}

/// A page's changes, with its objects and blocks as they were read for them, and the
/// paragraphs of the blocks that open as one, by block, as they were set up.
struct PagePlan {
    page: usize,
    objs: Vec<PageObject>,
    blocks: Vec<Block>,
    changes: Vec<Change>,
    paragraphs: HashMap<usize, (Paragraph, ParagraphSetting)>,
}

/// The changes `job`'s matches make, page by page, decided on the document as it is, as
/// Replace decides them, the pages read in parallel; and why each match left as it was is
/// left.
fn plan(job: &AllJob) -> (Vec<PagePlan>, Vec<LeftOut>) {
    let words = job.words.clone().unwrap_or_default();
    let mut left = Vec::new();
    let mut by_page: BTreeMap<usize, Vec<&[HitPart]>> = BTreeMap::new();
    for parts in &job.matches {
        match page_of(parts) {
            Ok(p) => by_page.entry(p).or_default().push(parts),
            Err(why) => left.push(LeftOut::of(why)),
        }
    }
    let by_page: Vec<(usize, Vec<&[HitPart]>)> = by_page.into_iter().collect();
    let planned: Vec<(Option<PagePlan>, Vec<LeftOut>)> = (by_page.into_par_iter())
        .map(|(page, matches)| plan_page(job, &words, page, &matches))
        .collect();
    let mut plans = Vec::new();
    for (pp, why) in planned {
        plans.extend(pp);
        left.extend(why);
    }
    (plans, left)
}

/// [`plan`] for page `page`'s `matches`: its changes, if any, and why the matches left as
/// they were are left.
fn plan_page(
    job: &AllJob,
    words: &DocWords,
    page: usize,
    matches: &[&[HitPart]],
) -> (Option<PagePlan>, Vec<LeftOut>) {
    let mut left = Vec::new();
    let Ok(objs) = job.pdf.page_objects(page) else {
        return (None, vec![LeftOut::Failed; matches.len()]);
    };
    let mut blocks: Option<Vec<Block>> = None;
    // The blocks read that open as a paragraph, as Replace opens one, set up; `None` for
    // one that doesn't.
    let mut opens: HashMap<usize, Option<(Paragraph, ParagraphSetting)>> = HashMap::new();
    let mut changes: Vec<Change> = Vec::new();
    for &parts in matches {
        let lines = match lines_of(&objs, parts) {
            Ok(l) => l,
            Err(why) => {
                left.push(LeftOut::of(why));
                continue;
            }
        };
        let (first, (a, _)) = lines[0];
        if page_text::untypable(&job.with, typed_font(objs.get(first), a)).is_some() {
            left.push(LeftOut::Unwritable);
            continue;
        }
        // One line is edited alone unless it may open as its paragraph; the page's blocks
        // are read only then.
        let alone = lines.len() == 1
            && !((job.paragraphs || PARAGRAPH_DEFAULT) && drawn_in_place(&objs[first]));
        let of = if alone {
            Of::Line(first)
        } else {
            let blocks =
                blocks.get_or_insert_with(|| job.pdf.page_blocks(page).unwrap_or_default());
            let block = (blocks.iter()).position(|b| b.pieces().any(|p| p == first));
            let mut can = |k: usize| {
                let set = opens.entry(k).or_insert_with(|| {
                    let b = &blocks[k];
                    let para = job.pdf.paragraph(page, b, words).ok()?;
                    let setting = job.pdf.paragraph_setting(&para).ok()?;
                    refused(b).is_none().then_some((para, setting))
                });
                set.is_some()
            };
            match lines[..] {
                // One line: its paragraph with paragraph editing on, else the line.
                [_] => match block {
                    Some(k)
                        if (job.paragraphs || PARAGRAPH_DEFAULT && blocks[k].confident())
                            && can(k) =>
                    {
                        Of::Paragraph(k)
                    }
                    _ => Of::Line(first),
                },
                // Across lines: their paragraph, when they are all in one it opens as.
                _ => {
                    let inside = |k: usize| {
                        let b = &blocks[k];
                        let sure = job.paragraphs || b.confident();
                        sure && lines.iter().all(|(id, _)| b.pieces().any(|p| p == *id))
                    };
                    match block.filter(|&k| inside(k)) {
                        Some(k) if can(k) => Of::Paragraph(k),
                        _ => {
                            left.push(LeftOut::AcrossParagraphs);
                            continue;
                        }
                    }
                }
            }
        };
        match changes.iter_mut().find(|c| c.of == of) {
            Some(c) => c.matches.push(lines),
            None => changes.push(Change {
                of,
                matches: vec![lines],
            }),
        }
    }
    // A line in a paragraph that is changed is changed with it.
    let blocks = blocks.unwrap_or_default();
    let para_of = |id: usize| {
        (changes.iter()).find_map(|c| match c.of {
            Of::Paragraph(k) if blocks[k].pieces().any(|p| p == id) => Some(c.of),
            _ => None,
        })
    };
    let moves: Vec<(usize, Of)> = (changes.iter().enumerate())
        .filter_map(|(i, c)| match c.of {
            Of::Line(id) => para_of(id).map(|to| (i, to)),
            Of::Paragraph(_) => None,
        })
        .collect();
    for &(i, to) in moves.iter().rev() {
        let moved = changes.remove(i);
        if let Some(c) = changes.iter_mut().find(|c| c.of == to) {
            c.matches.extend(moved.matches);
        }
    }
    let paragraphs = (opens.into_iter())
        .filter_map(|(k, set)| Some((k, set?)))
        .collect();
    let pp = (!changes.is_empty()).then_some(PagePlan {
        page,
        objs,
        blocks,
        changes,
        paragraphs,
    });
    (pp, left)
}

/// [`write`], again without the changes that failed, until none does: a failed change may
/// have left a font it added to its page in the copy.
fn settle(job: &AllJob, plan: &[PagePlan], left: &[LeftOut], unfit: &HashSet<Key>) -> Pass {
    let mut failed = HashSet::new();
    loop {
        let (pass, newly) = write(job, plan, left, unfit, &failed);
        if newly.is_empty() {
            return pass;
        }
        failed.extend(newly);
    }
}

/// A change written on the copy: how many matches it replaced and how many of those went
/// to another font, where the text it changed was, and where its text now is.
struct Done {
    of: Of,
    matches: usize,
    other_font: usize,
    old: [f32; 4],
    at: Written,
}

/// Where a change's text is written: from where a line started, or on a paragraph's lines
/// written anew.
enum Written {
    Line(Box<PageObject>),
    Paragraph(Vec<SetLine>),
}

/// Write `plan`'s changes, but those `unfit` or `failed`, on a copy of `job`'s document,
/// and judge each written; `left` the matches left before. Returns the copy and what it
/// holds, and the changes that failed now.
fn write(
    job: &AllJob,
    plan: &[PagePlan],
    left: &[LeftOut],
    unfit: &HashSet<Key>,
    failed: &HashSet<Key>,
) -> (Pass, Vec<Key>) {
    let mut pass = Pass {
        pdf: job.pdf.clone(),
        replaced: 0,
        pages: Vec::new(),
        over: 0,
        over_pages: Vec::new(),
        other_font: 0,
        left: Vec::new(),
        unfit: 0,
        similar: false,
        over_changes: HashSet::new(),
    };
    let pdf = &mut pass.pdf;
    let words = job.words.clone().unwrap_or_default();
    let sizes = pdf.page_sizes();
    let mut left = left.to_vec();
    let mut newly = Vec::new();
    for pp in plan {
        let mut done = Vec::new();
        let mut lines = Vec::new();
        let mut touched = false;
        for c in &pp.changes {
            let key = (pp.page, c.of);
            if unfit.contains(&key) {
                pass.unfit += c.matches.len();
                continue;
            }
            if failed.contains(&key) {
                left.extend(c.matches.iter().map(|_| LeftOut::Failed));
                continue;
            }
            // The paragraphs one at a time, then the lines together.
            let Of::Paragraph(k) = c.of else {
                lines.push(c);
                continue;
            };
            match paragraph(pdf, job, &words, pp, (c, k), touched) {
                Ok((d, lost)) => {
                    pass.similar |= lost;
                    done.push(d);
                }
                Err(_) => newly.push(key),
            }
            touched = true;
        }
        if !lines.is_empty() {
            match line_changes(pdf, job, pp, &lines, touched) {
                Ok((written, gone, lost)) => {
                    pass.similar |= lost;
                    done.extend(written);
                    newly.extend(gone.into_iter().map(|of| (pp.page, of)));
                }
                Err(_) => newly.extend(lines.iter().map(|c| (pp.page, c.of))),
            }
        }
        if done.is_empty() {
            continue;
        }
        let size = sizes.get(pp.page).copied().unwrap_or((612.0, 792.0));
        // A page that can't be read back can't be judged: its changes are left out.
        let Ok(after) = pdf.page_objects(pp.page) else {
            newly.extend(done.iter().map(|d| (pp.page, d.of)));
            continue;
        };
        let over = judge(&pp.objs, &after, &done, size);
        pass.pages.push(pp.page);
        for (d, over) in done.iter().zip(over) {
            pass.replaced += d.matches;
            pass.other_font += d.other_font;
            if over {
                pass.over += d.matches;
                pass.over_changes.insert((pp.page, d.of));
                if pass.over_pages.last() != Some(&pp.page) {
                    pass.over_pages.push(pp.page);
                }
            }
        }
    }
    left.sort_unstable();
    for why in left {
        match pass.left.last_mut() {
            Some((w, n)) if *w == why => *n += 1,
            _ => pass.left.push((why, 1)),
        }
    }
    (pass, newly)
}

/// Write paragraph change `c` (of block `k` of `pp`'s page as it was) on `pdf`: as it was
/// set up while the page is as it was (`touched`: something on it was written), else its
/// lines found again on the page as it is now. Returns what was written, and whether words
/// beside the replacements went to a similar font.
fn paragraph(
    pdf: &mut Pdf,
    job: &AllJob,
    words: &DocWords,
    pp: &PagePlan,
    (c, k): (&Change, usize),
    touched: bool,
) -> limarepdf_core::Result<(Done, bool)> {
    let changed = || anyhow::anyhow!("the page changed");
    let (now, found);
    let (para, setting) = match pp.paragraphs.get(&k).filter(|_| !touched) {
        Some((para, setting)) => {
            now = (0..pp.objs.len()).map(Some).collect();
            (para, setting)
        }
        None => {
            now = follow(&pp.objs, &pdf.page_objects(pp.page)?);
            let mut block = pp.blocks[k].clone();
            for l in &mut block.lines {
                for id in &mut l.pieces {
                    *id = now.get(*id).copied().flatten().ok_or_else(changed)?;
                }
                l.pieces.sort_unstable();
            }
            let para = pdf.paragraph(pp.page, &block, words)?;
            let setting = pdf.paragraph_setting(&para)?;
            found = (para, setting);
            (&found.0, &found.1)
        }
    };
    let now: &Vec<Option<usize>> = &now;
    let at = |id: usize| now.get(id).copied().flatten().ok_or_else(changed);
    // Where each line's characters are in the paragraph's text.
    let mut places: HashMap<(usize, usize), usize> = HashMap::new();
    for i in 0..para.text.chars().count() {
        if let Some(s) = para.source(i) {
            places.entry(s).or_insert(i);
        }
    }
    let len = |id: usize| (pp.objs[id].text.as_deref()).map_or(0, |t| t.chars().count());
    let place = |id: usize, j: usize| -> limarepdf_core::Result<usize> {
        let (now, n) = (at(id)?, len(id));
        let found = match j >= n && n > 0 {
            true => places.get(&(now, n - 1)).map(|i| i + 1),
            false => places.get(&(now, j)).copied(),
        };
        found.ok_or_else(changed)
    };
    let mut ranges = Vec::new();
    for lines in &c.matches {
        let ((first, (a, _)), (last, (_, b))) = (lines[0], lines[lines.len() - 1]);
        ranges.push((place(first, a)?, place(last, b)?));
    }
    let spaced: Vec<bool> = c.matches.iter().map(|l| l.len() == 1).collect();
    let mut text = Styled::of_paragraph(para, font_of);
    let orig = text.clone();
    let put_at = put(&mut text, &ranges, &job.with, &spaced);
    let parts = text.parts();
    let set = setting.set(&parts);
    let mut lost = false;
    let mut other_font = 0;
    if !text.same_as(&orig) && !set.unchanged() {
        // Each part's start in the text, for the pieces' places.
        let starts: Vec<usize> = (parts.iter())
            .scan(0, |at, p| {
                let s = *at;
                *at += p.text.chars().count();
                Some(s)
            })
            .collect();
        let away: Vec<(usize, usize)> = (setting.pieces(&parts).into_iter())
            .filter(|p| p.kept.is_none())
            .map(|p| (starts[p.part] + p.chars.0, starts[p.part] + p.chars.1))
            .collect();
        other_font = into_other_font(&put_at, &away);
        lost = !pdf.edit_paragraph(setting, &parts)?.is_empty();
    }
    let old = (pp.blocks[k].lines.iter().map(|l| l.bbox)).reduce(union);
    let written = set.lines.into_iter().filter(|l| l.kept.is_none()).collect();
    let d = Done {
        of: c.of,
        matches: c.matches.len(),
        other_font,
        old: old.unwrap_or_default(),
        at: Written::Paragraph(written),
    };
    Ok((d, lost))
}

/// Write line changes `lines` of `pp`'s page on `pdf` together, each line found again on
/// the page as it is now when something on it was written (`touched`). Returns what was
/// written, the changes that couldn't be, and whether words beside the replacements went to
/// a similar font.
fn line_changes(
    pdf: &mut Pdf,
    job: &AllJob,
    pp: &PagePlan,
    lines: &[&Change],
    touched: bool,
) -> limarepdf_core::Result<(Vec<Done>, Vec<Of>, bool)> {
    let read;
    let objs: &[PageObject] = match touched {
        true => {
            read = pdf.page_objects(pp.page)?;
            &read
        }
        false => &pp.objs,
    };
    let now = follow(&pp.objs, objs);
    let fonts = pdf.page_line_fonts(pp.page).unwrap_or_default();
    let (mut edits, mut made, mut gone) = (Vec::new(), Vec::new(), Vec::new());
    for c in lines {
        let Of::Line(id) = c.of else {
            continue;
        };
        let Some(i) = now.get(id).copied().flatten() else {
            gone.push(c.of);
            continue;
        };
        let o = &objs[i];
        let mut text = Styled::of(o, font_of);
        let orig = text.clone();
        let ranges: Vec<(usize, usize)> = c.matches.iter().map(|l| l[0].1).collect();
        let put_at = put(&mut text, &ranges, &job.with, &vec![true; ranges.len()]);
        let mut d = Done {
            of: c.of,
            matches: c.matches.len(),
            other_font: 0,
            old: o.bbox,
            at: Written::Line(Box::new(o.clone())),
        };
        if text.same_as(&orig) {
            made.push((None, d));
            continue;
        }
        let te = text_change(&text, o);
        if let Some(fonts) = fonts.get(i).and_then(|f| f.as_deref()) {
            let mut away = Vec::new();
            let mut start = 0;
            for part in &te.parts {
                for p in o.kept_pieces(part, fonts).into_iter().filter(|p| !p.kept) {
                    away.push((start + p.chars.0, start + p.chars.1));
                }
                start += part.text.chars().count();
            }
            d.other_font = into_other_font(&put_at, &away);
        }
        edits.push((i, te.clone()));
        made.push((Some((id, te)), d));
    }
    let written = match edits.is_empty() {
        true => Ok(Vec::new()),
        false => pdf.edit_texts(pp.page, &edits),
    };
    if let Ok(lost) = written {
        let done = made.into_iter().map(|(_, d)| d).collect();
        return Ok((done, gone, !lost.is_empty()));
    }
    // Some line couldn't be written: each alone, to know which.
    let (mut done, mut lost) = (Vec::new(), false);
    for (edit, d) in made {
        let Some((id, te)) = edit else {
            done.push(d);
            continue;
        };
        let now = follow(&pp.objs, &pdf.page_objects(pp.page)?);
        let written = (now.get(id).copied().flatten()).map(|i| pdf.edit_texts(pp.page, &[(i, te)]));
        match written {
            Some(Ok(l)) => {
                lost |= !l.is_empty();
                done.push(d);
            }
            _ => gone.push(d.of),
        }
    }
    Ok((done, gone, lost))
}

/// Put `with` in place of each of `ranges` (characters, from and to, in order, apart) of
/// `text`, as typing over each would, from the last: an empty `with` deletes them, a word
/// with a space beside it where `spaced` says ([`with_a_space`]). Returns where each
/// replacement is in the text now: its first character and how many it has.
fn put(
    text: &mut Styled,
    ranges: &[(usize, usize)],
    with: &str,
    spaced: &[bool],
) -> Vec<(usize, usize)> {
    let n = with.chars().count();
    let mut at = vec![(0, 0); ranges.len()];
    let mut order: Vec<usize> = (0..ranges.len()).collect();
    order.sort_by_key(|&i| std::cmp::Reverse(ranges[i].0));
    let mut end = usize::MAX;
    for (step, &i) in order.iter().enumerate() {
        let (mut a, mut b) = ranges[i];
        if with.is_empty() && spaced[i] {
            (a, b) = with_a_space(text.text(), (a, b));
        }
        let chars: Vec<char> = text.text().chars().collect();
        // Never into the replacement after it.
        let b = b.min(chars.len()).min(end);
        let a = a.min(b);
        let new: String = (chars[..a].iter().copied())
            .chain(with.chars())
            .chain(chars[b..].iter().copied())
            .collect();
        text.sync(&new, Some(a + n));
        // Those after it move by what it changed (each starts at `b` or later).
        for &j in &order[..step] {
            at[j].0 = (at[j].0 + n + a) - b;
        }
        at[i] = (a, n);
        end = a;
    }
    at
}

/// How many of the replacements `at` (each's first character and how many it has) have a
/// character in `away`, the stretches (from, to) written in another font than their own.
fn into_other_font(at: &[(usize, usize)], away: &[(usize, usize)]) -> usize {
    (at.iter())
        .filter(|&&(s, n)| n > 0 && away.iter().any(|&(a, b)| a < s + n && s < b))
        .count()
}

/// Where each of a page's objects as they were, `before`, is among them as they are now,
/// `after`, once some were changed: the first left of the same kind, text and box; `None`
/// for one changed or gone.
fn follow(before: &[PageObject], after: &[PageObject]) -> Vec<Option<usize>> {
    type Same = (u8, Option<String>, [i64; 4]);
    let same = |o: &PageObject| -> Same {
        let kind = match o.kind {
            ObjectKind::Text => 0,
            ObjectKind::Image => 1,
            ObjectKind::Form => 2,
            ObjectKind::Path => 3,
        };
        (
            kind,
            o.text.clone(),
            o.bbox.map(|v| (v * 100.0).round() as i64),
        )
    };
    let mut left: HashMap<Same, VecDeque<usize>> = HashMap::new();
    for (i, o) in after.iter().enumerate() {
        left.entry(same(o)).or_default().push_back(i);
    }
    (before.iter())
        .map(|o| left.get_mut(&same(o)).and_then(VecDeque::pop_front))
        .collect()
}

/// How far along line `o`'s baseline, from where it starts, line `n` starts, when it starts
/// on that baseline from there on: where a line edited from `o` is written.
fn along(o: &PageObject, n: &PageObject) -> Option<f32> {
    let ((x, y), (nx, ny)) = (o.baseline?, n.baseline?);
    let (dx, dy) = o.dir.unwrap_or((1.0, 0.0));
    let across = (ny - y) * dx - (nx - x) * dy;
    let along = (nx - x) * dx + (ny - y) * dy;
    (across.abs() < 0.5 && along > -0.5).then_some(along)
}

/// Whether each change written on a page, `done`, runs past the edge of the page (`size`
/// points across and down) or over other text, a picture or a shape on it, by the one
/// overflow rule ([`page_text::overflow`] over [`obstacles_in`]), as the typing editors judge
/// it: its text where the page as it is now, `after`, has it, against the page's other
/// things but those its text as it was, `before`, already met.
fn judge(
    before: &[PageObject],
    after: &[PageObject],
    done: &[Done],
    size: (f32, f32),
) -> Vec<bool> {
    let mut kept = vec![false; after.len()];
    for i in follow(before, after).into_iter().flatten() {
        kept[i] = true;
    }
    // Each line written anew to the change that wrote it: a paragraph's line where it was
    // set, else the line edited starting nearest before it on its baseline.
    let mut owner: Vec<Option<usize>> = vec![None; after.len()];
    for (i, n) in after.iter().enumerate() {
        if kept[i] || n.kind != ObjectKind::Text {
            continue;
        }
        let Some((x, y)) = n.baseline else {
            continue;
        };
        let set_at =
            |l: &SetLine| (y - l.baseline).abs() < 0.5 && x > l.x - 0.5 && x < l.x + l.width + 0.5;
        let para = (done.iter()).position(|d| match &d.at {
            Written::Paragraph(lines) => lines.iter().any(set_at),
            Written::Line(_) => false,
        });
        let line = || {
            (done.iter().enumerate())
                .filter_map(|(k, d)| match &d.at {
                    Written::Line(o) => along(o, n).map(|a| (k, a)),
                    Written::Paragraph(_) => None,
                })
                .min_by(|a, b| a.1.total_cmp(&b.1))
                .map(|(k, _)| k)
        };
        owner[i] = para.or_else(line);
    }
    let rect = |b: [f32; 4]| Rect::from_min_max(Pos2::new(b[0], b[1]), Pos2::new(b[2], b[3]));
    let page = Rect::from_min_size(Pos2::ZERO, Vec2::new(size.0, size.1));
    (0..done.len())
        .map(|k| {
            let rows: Vec<Rect> = (after.iter().enumerate())
                .filter(|(i, _)| owner[*i] == Some(k))
                .map(|(_, o)| rect(o.bbox))
                .collect();
            let old = done[k].old;
            let leave = |i: usize, o: &PageObject| owner[i] == Some(k) || meet(o.bbox, old);
            let others: Vec<(Rect, &'static str)> = (obstacles_in(after, size.0 * size.1, leave))
                .into_iter()
                .map(|(b, what)| (rect(b), what))
                .collect();
            page_text::overflow(page, &rows, &others).is_some()
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(text: &str) -> Styled {
        Styled::plain(text, crate::styled::CharStyle::default())
    }

    /// Each match is replaced from the last, and where each replacement is said as the text
    /// is now; deleting one takes a space with it, as deleting a word does.
    #[test]
    fn put_replaces_each_match_and_says_where() {
        let mut t = line("the cat sat on the cat mat");
        let at = put(&mut t, &[(4, 7), (19, 22)], "otter", &[true, true]);
        assert_eq!(t.text(), "the otter sat on the otter mat");
        assert_eq!(at, [(4, 5), (21, 5)]);
        let mut t = line("a cat a cat");
        let at = put(&mut t, &[(2, 5), (8, 11)], "", &[true, true]);
        assert_eq!(t.text(), "a a");
        assert_eq!(at, [(2, 0), (3, 0)]);
        // Holding what it replaces, it is put in once.
        let mut t = line("cat");
        put(&mut t, &[(0, 3)], "cats", &[true]);
        assert_eq!(t.text(), "cats");
    }

    /// A replacement goes to another font when a stretch written in one touches it.
    #[test]
    fn a_replacement_in_a_stretch_away_counts() {
        let at = [(4, 5), (21, 5), (30, 0)];
        assert_eq!(into_other_font(&at, &[]), 0);
        assert_eq!(into_other_font(&at, &[(0, 4)]), 0);
        assert_eq!(into_other_font(&at, &[(3, 5), (25, 40)]), 2);
    }

    /// A made-up document of 300 Letter pages, each with ten lines of 12-point Helvetica 20
    /// points apart, with 1,100 matches of "walrus": three lines a page with one, every
    /// third page a line with two more.
    fn walrus_pages() -> Pdf {
        let mut pdf = Pdf::new();
        for p in 0..300 {
            pdf.insert_blank(p, 612.0, 792.0).unwrap();
            for l in 0..10 {
                let text = match l {
                    0 | 3 | 6 => "the walrus ate the fish by the quay",
                    9 if p % 3 == 0 => "a walrus and a walrus",
                    _ => "a quiet line of other words",
                };
                let at = (72.0, 100.0 + 20.0 * l as f32);
                (pdf.add_text(p, at, text, FontChoice::default(), 12.0, [0.0; 3])).unwrap();
            }
        }
        pdf
    }

    /// Batch 46's stage 3 measure: Replace all's preparation on [`walrus_pages`], every
    /// line replaced alone and with paragraph editing on, timed and printed. Run with
    /// `--ignored --nocapture`, on an optimised build (`--profile ci-test`).
    #[test]
    #[ignore]
    fn replace_all_speed_on_300_pages() {
        let pdf = walrus_pages();
        let matches: Vec<Vec<HitPart>> = (pdf.search("walrus").unwrap().into_iter())
            .map(|h| h.parts)
            .collect();
        assert_eq!(matches.len(), 1100);
        for paragraphs in [false, true] {
            let job = AllJob {
                pdf: pdf.clone(),
                matches: matches.clone(),
                with: "otter".into(),
                paragraphs,
                words: None,
            };
            let start = std::time::Instant::now();
            let planned = plan(&job).0.len();
            let planning = start.elapsed();
            let start = std::time::Instant::now();
            let done = replace_all(&job);
            let took = start.elapsed();
            let all = &done.all;
            println!(
                "paragraphs {paragraphs}: {} replaced on {} pages ({planned} planned in \
                 {planning:?}), {} over, {} left, in {took:?}",
                all.replaced,
                all.pages.len(),
                all.over,
                all.left.iter().map(|l| l.1).sum::<usize>(),
            );
            assert_eq!(all.replaced, 1100);
        }
    }
}

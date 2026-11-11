//! Text attached to a stretch of a page's letters for other apps to read in their place, for
//! copying, search and screen readers: a tag's `/ActualText`, and `/Alt` and `/E` with it
//! ([`reach::ALT`]). An edit that changes a stretch's letters takes it off, so they read as
//! typed; a line written anew around letters it leaves as they are writes a copy of the tag
//! around them; where it can't, the edit is refused (PB-71).
//!
//! The structure tree's own attached text for a tag (by its `/MCID`) isn't changed here.

use crate::objects::{Obj, TextData};
use crate::{reach, Pdf, Result};
use anyhow::bail;
use lopdf::content::Operation;
use lopdf::{Dictionary, Object, ObjectId};
use std::cmp::Reverse;
use std::collections::HashMap;

use crate::edit::TextPart;

/// Why an edit is refused around letters it leaves as they are.
pub(crate) const REFUSED: &str = "letters the edit leaves as they are carry text other apps \
                                  read in their place, which this edit can't keep with them; \
                                  nothing was changed";

/// A stretch of a page's content in a tag with attached text.
#[derive(Clone, Debug)]
pub(crate) struct Attached {
    /// Its `BDC`'s index in the page's operators, and its `EMC`'s (the operators' end when
    /// it has none).
    pub bdc: usize,
    pub emc: usize,
    /// Its tag, and its properties (named ones looked up).
    tag: Object,
    props: Dictionary,
}

/// A copy of a stretch's tag around its letters in a line written anew ([`line_body`]): the
/// stretch's index in the page's, and the `BDC` that opens it.
///
/// [`line_body`]: crate::edit::line_body
#[derive(Clone, Debug)]
pub(crate) struct Tag {
    pub of: usize,
    pub bdc: Operation,
}

/// What edits do to the stretches with attached text on the lines they write anew
/// ([`Carried::line`]).
#[derive(Default)]
pub(crate) struct Carried {
    /// The tags their text is taken off: their `BDC`s by operator, as written again.
    pub stripped: HashMap<usize, Operation>,
    /// For each line, the copies written around the letters it keeps: their characters in
    /// its new text, from and to, and the tag.
    pub copies: Vec<Vec<((usize, usize), Tag)>>,
}

impl Attached {
    /// The stretches of operators `ops` in tags with attached text, in the order they open,
    /// named properties looked up by `named`. A tag left open runs to the end.
    pub fn of(ops: &[Operation], named: impl Fn(&[u8]) -> Dictionary) -> Vec<Attached> {
        let mut out: Vec<Attached> = Vec::new();
        // The tags open, innermost last: each one's stretch in `out`, when it has text.
        let mut open: Vec<Option<usize>> = Vec::new();
        for (i, o) in ops.iter().enumerate() {
            match o.operator.as_str() {
                "BDC" => {
                    let props = match o.operands.get(1) {
                        Some(Object::Dictionary(d)) => d.clone(),
                        Some(Object::Name(n)) => named(n),
                        _ => Dictionary::new(),
                    };
                    let alt = reach::ALT.iter().any(|k| props.has(k));
                    open.push(alt.then_some(out.len()));
                    if alt {
                        out.push(Attached {
                            bdc: i,
                            emc: ops.len(),
                            tag: o.operands.first().cloned().unwrap_or(Object::Null),
                            props,
                        });
                    }
                }
                "BMC" => open.push(None),
                "EMC" => {
                    if let Some(Some(k)) = open.pop() {
                        out[k].emc = i;
                    }
                }
                _ => {}
            }
        }
        out
    }

    /// Its `BDC` with the attached text taken off: its properties inline, without it.
    pub fn stripped(&self) -> Operation {
        let mut d = self.props.clone();
        for k in reach::ALT {
            d.remove(k);
        }
        Operation::new("BDC", vec![self.tag.clone(), Object::Dictionary(d)])
    }

    /// A copy of its `BDC` for its letters written anew: its properties inline, without its
    /// place in the structure tree (`/MCID`), which stays with the old tag.
    fn copied(&self) -> Operation {
        let mut d = self.props.clone();
        d.remove(b"MCID");
        Operation::new("BDC", vec![self.tag.clone(), Object::Dictionary(d)])
    }

    /// Whether operator `op` is inside it.
    pub fn holds(&self, op: usize) -> bool {
        self.bdc < op && op < self.emc
    }

    /// Its letters on line `t`, from and to in the line's text: its shows' characters there,
    /// less the space read in the gap after the last. `None` when it has none there.
    pub fn letters(&self, t: &TextData) -> Option<(usize, usize)> {
        let mut inside = (0..t.runs.len()).filter(|&k| self.holds(t.runs[k].op));
        let first = inside.next()?;
        let last = inside.next_back().unwrap_or(first);
        let gap = usize::from(t.runs[last].gap > 0.0);
        let (a, b) = (t.pieces[first].chars.0, t.pieces[last].chars.1 - gap);
        (a < b).then_some((a, b))
    }

    /// Whether it holds shows of lines of `objs` other than `t`.
    fn elsewhere(&self, objs: &[Obj], t: &TextData) -> bool {
        let mine = |op: usize| t.runs.iter().any(|r| r.op == op);
        (objs.iter().filter_map(|o| o.text.as_ref()))
            .flat_map(|o| &o.runs)
            .any(|r| self.holds(r.op) && !mine(r.op))
    }
}

impl Carried {
    /// Line `t` edited to `new`, each of whose characters was where `origins` says in the
    /// line (`None` for one typed), `all` being its page's stretches and `objs` its lines:
    /// each stretch with letters on it loses its attached text, and those whose letters the
    /// edit keeps, in order and as they were, get a copy around them in the new text. Only
    /// when `copies` says the line is written in pieces that can have one, else (one new
    /// show of the whole line) the edit is refused around letters kept; and when the stretch
    /// holds letters of another line, which can't go with them.
    pub fn line(
        &mut self,
        (all, objs): (&[Attached], &[Obj]),
        t: &TextData,
        new: &str,
        origins: &[Option<usize>],
        copies: bool,
    ) -> Result<()> {
        let (old, new): (Vec<char>, Vec<char>) = (t.text.chars().collect(), new.chars().collect());
        let origins: Vec<Option<usize>> = (origins.iter().enumerate())
            .map(|(j, o)| o.filter(|&o| old.get(o).is_some_and(|c| new.get(j) == Some(c))))
            .collect();
        let mut line = Vec::new();
        for (s, a) in all.iter().enumerate() {
            let Some((from, to)) = a.letters(t) else {
                continue;
            };
            self.stripped.insert(a.bdc, a.stripped());
            let Some(j) = kept_at(&origins, (from, to)) else {
                continue;
            };
            if !copies || a.elsewhere(objs, t) {
                bail!(REFUSED);
            }
            let tag = Tag {
                of: s,
                bdc: a.copied(),
            };
            line.push(((j, j + to - from), tag));
        }
        self.copies.push(line);
        Ok(())
    }
}

/// Where an edit keeps characters `(a, b)` of what it edits consecutively, in order: where
/// the first is in the new text, each of whose characters came from where `origins` says.
fn kept_at(origins: &[Option<usize>], (a, b): (usize, usize)) -> Option<usize> {
    let j = origins.iter().position(|&o| o == Some(a))?;
    (a..b)
        .all(|o| origins.get(j + o - a) == Some(&Some(o)))
        .then_some(j)
}

/// An edit's `parts` split where the copies `copies` start and end (their characters in its
/// text, from and to), each piece with the copies around it, outermost first.
pub(crate) fn split(
    parts: &[TextPart],
    copies: &[((usize, usize), Tag)],
) -> (Vec<TextPart>, Vec<Vec<Tag>>) {
    let cuts: Vec<usize> = copies.iter().flat_map(|((a, b), _)| [*a, *b]).collect();
    let (mut out, mut tags) = (Vec::new(), Vec::new());
    let mut at = 0;
    for p in parts {
        let text: Vec<char> = p.text.chars().collect();
        let n = text.len();
        let mut ends: Vec<usize> = (cuts.iter())
            .filter(|&&c| at < c && c < at + n)
            .map(|c| c - at)
            .collect();
        ends.push(n);
        ends.sort_unstable();
        ends.dedup();
        let mut from = 0;
        for to in ends {
            let mut around: Vec<_> = (copies.iter())
                .filter(|((a, b), _)| *a <= at + from && at + to <= *b)
                .collect();
            around.sort_by_key(|((a, b), _)| (*a, Reverse(*b)));
            tags.push(around.into_iter().map(|(_, t)| t.clone()).collect());
            out.push(TextPart {
                text: text[from..to].iter().collect(),
                from: None,
                origins: (from..to).map(|j| p.origin(j)).collect(),
                ..p.clone()
            });
            from = to;
        }
        at += n;
    }
    (out, tags)
}

/// What a paragraph edit does to the stretches with attached text over the rows it writes
/// anew (their lines `gone`, of `objs`): the tags it takes the text off (their `BDC`s by
/// operator, as written again), where it changes their letters, the paragraph's `n`
/// characters having been drawn as `source` says (`Paragraph::source`) and the
/// edit's coming from where `origins` says. A row written anew has no copy of a tag, so
/// the edit is refused around letters it keeps.
pub(crate) fn in_rows(
    (all, objs): (&[Attached], &[Obj]),
    gone: &[usize],
    (n, source): (usize, impl Fn(usize) -> Option<(usize, usize)>),
    origins: &[Option<usize>],
) -> Result<HashMap<usize, Operation>> {
    let text = |id: usize| objs.get(id).and_then(|o| o.text.as_ref());
    let mut stripped = HashMap::new();
    for a in all {
        let touched =
            (gone.iter().filter_map(|&id| text(id))).any(|t| t.runs.iter().any(|r| a.holds(r.op)));
        if !touched {
            continue;
        }
        stripped.insert(a.bdc, a.stripped());
        // Its letters in the paragraph, from its first to its last.
        let mut at = (0..n).filter(|&k| {
            source(k).is_some_and(|(id, c)| {
                let letters = text(id).and_then(|t| a.letters(t));
                letters.is_some_and(|(from, to)| from <= c && c < to)
            })
        });
        let Some(first) = at.next() else {
            continue;
        };
        let last = at.last().unwrap_or(first);
        if kept_at(origins, (first, last + 1)).is_some() {
            bail!(REFUSED);
        }
    }
    Ok(stripped)
}

impl Pdf {
    /// The stretches of page `page`'s operators `ops` with attached text ([`Attached::of`]).
    pub(crate) fn attached(&self, page: ObjectId, ops: &[Operation]) -> Vec<Attached> {
        let res = self.resolved_dict(self.inherited(page, b"Resources").as_ref());
        let named = self.resolved_dict(res.get(b"Properties").ok());
        Attached::of(ops, |n| self.resolved_dict(named.get(n).ok()))
    }
}

//! An edited paragraph written in place (NEW-01; the paragraph design's section 7): each
//! line set anew where its old line was in the content, inside its text object, with the
//! text state put back after it; the lines that keep their breaks left as they were.

use crate::decorate::{carried, Decorated, Redraw};
use crate::edit::{line_body, move_only, op, put_code, r, tj, Check, Following, Ink, Written};
use crate::edit::{PartFont, TextPart, Way};
use crate::fonts::FontChoice;
use crate::layout::{shown, Setter};
use crate::matrix::{self as mx, apply, mul, M};
use crate::objects::{Obj, Parsed, Show};
use crate::para_frames::KeptFrame;
use crate::paragraph::Align;
use crate::rewrap::{
    away_pieces, decorations_of, ParagraphSetting, Planned, SetLine, SetParagraph,
};
use crate::{Pdf, Result};
use anyhow::{anyhow, bail};
use lopdf::content::Operation;
use lopdf::{Object, ObjectId, StringFormat};
use std::collections::HashMap;

impl Pdf {
    /// The paragraph of `setting` ([`Pdf::paragraph_setting`]) edited to `parts`, as
    /// [`ParagraphSetting::set`] sets them, written in one rewrite of its page's content: each
    /// line set anew in place of its old line's first show (the lines added after the last
    /// one's last show), inside that text object, the text state put back after it; the
    /// old lines' shows taken out, their other operators kept; the lines kept left as they
    /// were. So what draws over what, clipping and the text's tags stay. Each character
    /// keeps its own code in its part's font where it can, across the paragraph's lines, the
    /// rest a word at a time in the font they go to. Each line written is read back: one
    /// that doesn't read as it should is written again with its letters in the font they
    /// go to; one that still doesn't refuses the whole edit, nothing changed. Nothing is
    /// done when no line changes. Returns the lines ([`crate::SetParagraph::lines`]) whose
    /// letters were written in another font after all.
    pub fn edit_paragraph(
        &mut self,
        setting: &ParagraphSetting,
        parts: &[TextPart],
    ) -> Result<Vec<usize>> {
        let f = &setting.paragraph().frame;
        self.edit_paragraph_between(setting, parts, (f.left, f.right))
    }

    /// [`Pdf::edit_paragraph`] with the frame's left and right edges moved to `edges`, as
    /// [`ParagraphSetting::set_between`] sets the paragraph between them.
    pub fn edit_paragraph_between(
        &mut self,
        setting: &ParagraphSetting,
        parts: &[TextPart],
        edges: (f32, f32),
    ) -> Result<Vec<usize>> {
        let align = setting.paragraph().frame.align;
        self.paragraph_with(setting, parts, edges, align, None)
    }

    /// [`Pdf::edit_paragraph_between`] with the paragraph aligned as `align` says, as
    /// [`ParagraphSetting::set_in`] sets it (NEW-15).
    pub fn edit_paragraph_in(
        &mut self,
        setting: &ParagraphSetting,
        parts: &[TextPart],
        edges: (f32, f32),
        align: Align,
    ) -> Result<Vec<usize>> {
        self.paragraph_with(setting, parts, edges, align, None)
    }

    /// [`Pdf::edit_paragraph_in`] with the paragraph's underlines and strike-throughs as
    /// given (over the text of `parts`), in place of its lines' old ones.
    pub fn edit_paragraph_decorated(
        &mut self,
        setting: &ParagraphSetting,
        parts: &[TextPart],
        edges: (f32, f32),
        align: Align,
        decorations: &[Decorated],
    ) -> Result<Vec<usize>> {
        self.paragraph_with(setting, parts, edges, align, Some(decorations))
    }

    /// A paragraph edited ([`Pdf::edit_paragraph_in`]) between its frame's edges, aligned
    /// as given, its lines' decorations drawn again for its new text: as `asked`, else
    /// carried with their characters ([`crate::decorate::carried`]), each line's stretch
    /// where the line is written.
    fn paragraph_with(
        &mut self,
        setting: &ParagraphSetting,
        parts: &[TextPart],
        edges: (f32, f32),
        align: Align,
        asked: Option<&[Decorated]>,
    ) -> Result<Vec<usize>> {
        let set = setting.set_in(parts, edges, align);
        if set.unchanged() && asked.is_none() {
            return Ok(Vec::new());
        }
        let para = setting.paragraph();
        let (page, mut parsed) = self.load_edit(para.page)?;
        for (id, text) in para.rows.iter().flat_map(|r| &r.pieces) {
            let now = parsed.objs.get(*id).and_then(|o| o.text.as_ref());
            if now.map(|t| &t.text) != Some(text) {
                bail!("the page changed");
            }
        }
        // Its lines' decorations on its text, carried to the new text; the old ones stop
        // painting, all drawn again where their characters are written.
        let old = decorations_of(para, &parsed.decorations);
        let want: Vec<Decorated> = match asked {
            Some(asked) => asked.to_vec(),
            None => {
                let origins: Vec<Option<usize>> = (parts.iter())
                    .flat_map(|p| (0..p.text.chars().count()).map(move |j| p.origin(j)))
                    .collect();
                carried(&old.iter().collect::<Vec<_>>(), &origins)
            }
        };
        let rows: Vec<usize> = para
            .rows
            .iter()
            .flat_map(|r| &r.pieces)
            .map(|(id, _)| *id)
            .collect();
        let paths: Vec<usize> = (parsed.decorations.iter())
            .filter(|(line, _)| rows.contains(line))
            .flat_map(|(_, d)| d.paths.clone())
            .collect();
        for p in paths {
            let last = parsed.objs[p].last;
            parsed.ops[last] = op("n", vec![]);
        }
        let mut own = vec![true; set.lines.len()];
        let mut lost = Vec::new();
        loop {
            let (ops, checks) =
                self.paragraph_ops(page, &parsed, setting, parts, &set.lines, &own)?;
            let misread = self.misread(page, &parsed.inline, &ops, &checks)?;
            if misread.is_empty() {
                let chars: Vec<char> = parts.iter().flat_map(|p| p.text.chars()).collect();
                let lines = Set {
                    lines: &set.lines,
                    chars: &chars,
                    checks: &checks,
                };
                let (ops, depth) =
                    self.paragraph_decorated(page, &parsed, setting, &lines, &want, ops)?;
                self.commit(page, ops, depth, &parsed.inline)?;
                self.keep_set_frame(page, setting, &set, edges, align)?;
                return Ok(lost);
            }
            for n in misread {
                if !own[n] {
                    bail!("a line of the edited paragraph doesn't read back as it was typed; nothing was changed");
                }
                own[n] = false;
                lost.push(n);
            }
        }
    }

    /// Record on page `page` the frame paragraph `setting` was written in as `set`, between
    /// `edges` and aligned as `align` says ([`crate::para_frames`]), where it isn't the one
    /// its lines give: its edges or alignment set, or its frame read from the record, which
    /// follows its lines as they are written.
    fn keep_set_frame(
        &mut self,
        page: ObjectId,
        setting: &ParagraphSetting,
        set: &SetParagraph,
        edges: (f32, f32),
        align: Align,
    ) -> Result<()> {
        let para = setting.paragraph();
        let f = &para.frame;
        let moved = (edges.0 - f.left).abs() > 1e-3 || (edges.1 - f.right).abs() > 1e-3;
        let (Some(first), Some(last)) = (set.lines.first(), set.lines.last()) else {
            return Ok(());
        };
        if !(para.kept || moved || align != f.align) {
            return Ok(());
        }
        let frame = setting.frame_in(edges, align);
        let rows = (para.rows.first(), para.rows.last());
        let was = match rows {
            (Some(a), Some(b)) => (a.baseline, b.baseline),
            _ => (first.baseline, last.baseline),
        };
        let kept = KeptFrame {
            top: first.baseline,
            bottom: last.baseline,
            left: frame.left,
            right: frame.right,
            indent: frame.indent,
        };
        self.keep_frame(page, kept, was)
    }

    /// The edited paragraph's operators `ops`, with decorations `want` (over its new text)
    /// drawn on top for each of its set lines: where a line written is (its read-back
    /// place), or where a line kept is, as `parsed` read it.
    fn paragraph_decorated(
        &self,
        page: ObjectId,
        parsed: &Parsed,
        setting: &ParagraphSetting,
        lines: &Set,
        want: &[Decorated],
        ops: Vec<Operation>,
    ) -> Result<(Vec<Operation>, usize)> {
        let (set, chars, checks) = (lines.lines, lines.chars, lines.checks);
        if want.is_empty() {
            return Ok((ops, parsed.depth_left));
        }
        let bytes = (parsed.inline.encode(ops.clone()))
            .map_err(|e| anyhow!("cannot encode the page content: {e}"))?;
        let back = self.parse_as_page(page, &bytes)?;
        let para = setting.paragraph();
        let mut added = Vec::new();
        for (n, l) in set.iter().enumerate() {
            let (a, mut end) = l.chars;
            while end > a && chars[end - 1].is_whitespace() {
                end -= 1;
            }
            let here: Vec<Decorated> = (want.iter())
                .filter(|d| d.chars.0 < end && d.chars.1 > a)
                .map(|d| Decorated {
                    chars: (d.chars.0.max(a) - a, d.chars.1.min(end) - a),
                    ..d.clone()
                })
                .collect();
            if here.is_empty() {
                continue;
            }
            let (source, place) = match l.kept {
                Some(i) => {
                    let ids = para.rows[i].pieces.iter().map(|(id, _)| *id);
                    let objs: Vec<&Obj> = ids.filter_map(|id| parsed.objs.get(id)).collect();
                    let from = objs.iter().map(|o| o.first).min().unwrap_or(0);
                    let to = objs.iter().map(|o| o.last + 1).max().unwrap_or(0);
                    (parsed, Check::Within(from, to))
                }
                None => match checks.iter().find(|(m, ..)| *m == n) {
                    Some((_, place, _)) => (&back, *place),
                    None => continue,
                },
            };
            let text: String = chars[a..end].iter().collect();
            let (more, _) = self.decoration_ops(page, source, place, &Redraw::new(&text, &here));
            added.extend(more);
        }
        if added.is_empty() {
            return Ok((ops, parsed.depth_left));
        }
        let mut ops = crate::edit::closed(ops, parsed.depth_left);
        ops.extend(added);
        Ok((ops, 0))
    }

    /// The page's operators with paragraph `setting` edited to `parts` set as `lines`, each
    /// line's own letters in their fonts where `own` says ([`Pdf::edit_paragraph`]); and the
    /// lines written, to read back: each one's index, where it is, and what it should read.
    fn paragraph_ops(
        &mut self,
        page: ObjectId,
        parsed: &Parsed,
        setting: &ParagraphSetting,
        parts: &[TextPart],
        lines: &[SetLine],
        own: &[bool],
    ) -> Result<(Vec<Operation>, Checks)> {
        let para = setting.paragraph();
        let chars: Vec<char> = parts.iter().flat_map(|p| p.text.chars()).collect();
        // What each character is written as, a line not read back as it should be written
        // all in the fonts its words go to.
        let mut plan = setting.planned(parts);
        for (_, l) in lines.iter().enumerate().filter(|(n, _)| !own[*n]) {
            for pl in &mut plan[l.chars.0..l.chars.1] {
                pl.kept = None;
            }
        }
        let rows = para.rows.len();
        let written: Vec<usize> = (0..lines.len())
            .filter(|&n| lines[n].kept.is_none())
            .collect();
        let replaced: Vec<usize> = (0..rows)
            .filter(|&i| lines.get(i).and_then(|l| l.kept) != Some(i))
            .collect();
        let objects = |i: usize| para.rows[i].pieces.iter().map(|(id, _)| *id);
        let text_of = |id: usize| {
            (parsed.objs.get(id).and_then(|o| o.text.as_ref()))
                .ok_or_else(|| anyhow!("the page changed"))
        };
        // Each written line's characters, without the spaces it ends with.
        let content = |l: &SetLine| {
            let mut end = l.chars.1;
            while end > l.chars.0 && chars[end - 1].is_whitespace() {
                end -= 1;
            }
            (l.chars.0, end)
        };
        // The words going to another font, each font's in order, written by one pen a font
        // with all of its text set as it was measured.
        let away = away_pieces(&plan);
        let setters = setting.setters(&chars, &away);
        let mut words: Vec<(FontChoice, Vec<String>)> = Vec::new();
        // For each written line, its stretches: kept ones (from, to) or a word (font, its
        // place in that font's list), and what follows each word (its spaces).
        let mut stretches: Vec<Vec<Stretch>> = Vec::new();
        for &n in &written {
            let (a, end) = content(&lines[n]);
            let mut line: Vec<Stretch> = Vec::new();
            let mut j = a;
            while j < end {
                let pl = plan[j];
                if pl.kept.is_some() {
                    let mut k = j;
                    while k < end && plan[k].kept.is_some() && plan[k].part == pl.part {
                        k += 1;
                    }
                    line.push(Stretch::Kept(j, k));
                    j = k;
                    continue;
                }
                let mut k = j;
                while k < end
                    && plan[k].kept.is_none()
                    && plan[k].part == pl.part
                    && !chars[k].is_whitespace()
                {
                    k += 1;
                }
                if k == j {
                    // A space between words in another font: a move.
                    line.push(Stretch::Space(j));
                    j += 1;
                    continue;
                }
                let mut word: String = (chars[j..k].iter()).filter(|&&c| c != '\u{AD}').collect();
                if chars[k - 1] == '\u{AD}' && k == end {
                    word.push('-');
                }
                let list = match words.iter_mut().find(|(f, _)| *f == pl.away) {
                    Some((_, list)) => list,
                    None => {
                        words.push((pl.away, Vec::new()));
                        &mut words.last_mut().expect("pushed").1
                    }
                };
                list.push(word);
                line.push(Stretch::Word(j, pl.away, list.len() - 1));
                j = k;
            }
            stretches.push(line);
        }
        let mut pens = Vec::new();
        for (f, list) in &words {
            let setter = match setters.iter().find(|(g, _)| g == f) {
                Some((_, s)) => s,
                None => bail!("no setter for a font words go to"),
            };
            let texts: Vec<&str> = list.iter().map(String::as_str).collect();
            let (names, pen) = self.text_pen(page, setter, &texts)?;
            pens.push((*f, names, pen));
        }
        let (vd, _, _) = self.vis_down(page);
        // Each written line's operators, the state put back after it, by where it goes: in
        // place of a show, or after one.
        let mut at: HashMap<usize, Vec<Placed>> = HashMap::new();
        let mut after: HashMap<usize, Vec<Placed>> = HashMap::new();
        let last_replaced = replaced.last().copied();
        for (w, &n) in written.iter().enumerate() {
            let l = &lines[n];
            // Where it goes: its old line's first show, or after the last old line's last.
            let (show, put_after) = match n < rows {
                true => {
                    let first = (objects(n).map(|id| text_of(id).map(|t| t.runs[0].op)))
                        .collect::<Result<Vec<_>>>()?;
                    let first = first.into_iter().min();
                    (first.ok_or_else(|| anyhow!("a line with no text"))?, false)
                }
                false => {
                    let i = last_replaced.ok_or_else(|| anyhow!("no line to write after"))?;
                    let ends =
                        objects(i).map(|id| text_of(id).map(|t| t.runs[t.runs.len() - 1].op));
                    let last = ends.collect::<Result<Vec<_>>>()?.into_iter().max();
                    (last.ok_or_else(|| anyhow!("a line with no text"))?, true)
                }
            };
            let shows_it =
                |o: &&Obj| (o.text.as_ref()).is_some_and(|t| t.runs.iter().any(|r| r.op == show));
            let obj = (parsed.objs.iter())
                .find(shows_it)
                .ok_or_else(|| anyhow!("the page changed"))?;
            let t = obj.text.as_ref().expect("text");
            let run = t.runs.iter().find(|r| r.op == show).expect("its show");
            let span = (t.spans.iter())
                .find(|s| (s.runs.0..s.runs.1).any(|k| t.runs[k].op == show))
                .expect("its part");
            let parts_written = self.written_line(
                setting,
                parts,
                (&plan, &chars),
                l,
                &stretches[w],
                (&pens, &setters),
            )?;
            // The line in page space, then in the text object's.
            let (dx, dy) = t.dir;
            let (ox, oy) = apply(vd, l.x, l.baseline);
            let user = [dx, dy, -dy, dx, ox, oy];
            let mut body = line_body(user, &parts_written, true);
            if parts_written.iter().any(|w| w.flipped) {
                let mut whole = vec![op("BT", vec![])];
                whole.extend(body);
                whole.push(op("ET", vec![]));
                body = self.turned(page, whole, &parts_written)?;
                body.retain(|o| o.operator != "BT" && o.operator != "ET");
            }
            let inv = mx::inv(obj.ctm).ok_or_else(|| anyhow!("degenerate page transform"))?;
            for o in body.iter_mut().filter(|o| o.operator == "Tm") {
                let m: Vec<f32> = (o.operands.iter())
                    .filter_map(|x| x.as_float().ok())
                    .collect();
                if let [a, b, c, d, e, f] = m[..] {
                    o.operands = matrix(mul([a, b, c, d, e, f], inv));
                }
            }
            // The state as the show had it, for what follows.
            let s = &run.state;
            let mut back = vec![
                op("Tf", vec![Object::Name(span.font.clone()), r(s.size)]),
                op("Tc", vec![r(s.tc)]),
                op("Tw", vec![r(s.tw)]),
                op("Tz", vec![r(s.th * 100.0)]),
                op("Ts", vec![r(s.rise)]),
            ];
            back.extend(span.paint.ops());
            // What the line left as it was needn't be set again.
            let said = |o: &Operation| format!("{:?}", o.operands);
            back.retain(|b| {
                let last = body.iter().rev().find(|o| o.operator == b.operator);
                last.is_none_or(|o| said(o) != said(b))
            });
            back.push(op("Tm", matrix(s.tlm)));
            let place = if put_after { &mut after } else { &mut at };
            place.entry(show).or_default().push((n, body, back));
        }
        // The old lines' shows out, what follows each kept where it was.
        let mut removed: Vec<usize> = replaced.iter().flat_map(|&i| objects(i)).collect();
        removed.sort_unstable();
        let mut shows: HashMap<usize, Show> = HashMap::new();
        let mut following = Vec::new();
        for &id in &removed {
            let obj = &parsed.objs[id];
            let t = text_of(id)?;
            shows.extend(t.runs.iter().map(|r| (r.op, r.show)));
            following.push(Following::of(&parsed.ops, obj));
        }
        let fixes = Following::all(following);
        let mut out = Vec::with_capacity(parsed.ops.len() + 16 * written.len());
        let mut checks = Vec::new();
        let mut put = |out: &mut Vec<Operation>, list: Option<&Vec<Placed>>| {
            for (n, body, back) in list.into_iter().flatten() {
                let from = out.len();
                out.extend(body.iter().cloned());
                let (a, end) = content(&lines[*n]);
                let want = shown(&chars[a..end].iter().collect::<String>());
                checks.push((*n, Check::Within(from, out.len()), want));
                out.extend(back.iter().cloned());
            }
        };
        for (i, o) in parsed.ops.iter().enumerate() {
            out.extend(fixes.get(&i).into_iter().flatten().cloned());
            match shows.get(&i) {
                Some(&show) => {
                    out.extend(move_only(show));
                    put(&mut out, at.get(&i));
                    put(&mut out, after.get(&i));
                }
                None => out.push(o.clone()),
            }
        }
        Ok((out, checks))
    }

    /// Line `l` of an edit (`parts`, written as `plan` says, its characters `chars`) in its
    /// stretches, each as it is written: the kept ones with their own codes where they keep
    /// them, the words in other fonts by `pens` (each font's, with its fonts' names), the
    /// spaces between those by `setters`' spaces.
    fn written_line(
        &self,
        setting: &ParagraphSetting,
        parts: &[TextPart],
        (plan, chars): (&[Planned], &[char]),
        l: &SetLine,
        stretches: &[Stretch],
        (pens, setters): (&[Pen], &[(FontChoice, Setter)]),
    ) -> Result<Vec<Written>> {
        let para = setting.paragraph();
        let end = {
            let mut end = l.chars.1;
            while end > l.chars.0 && chars[end - 1].is_whitespace() {
                end -= 1;
            }
            end
        };
        // The colour a part is written in: what set it in the paragraph when it is its
        // part's colour there and `rg` wouldn't write it as it was.
        let paint = |p: &TextPart| {
            let i = match p.font {
                PartFont::Span(i) => i,
                PartFont::Choice(_) => return None,
            };
            let (s, part) = (para.spans.get(i)?, para.parts.get(i)?);
            (s.fill == Some(p.color) && !part.paint.plain()).then(|| part.paint.clone())
        };
        let mut out: Vec<Written> = Vec::new();
        for st in stretches {
            match *st {
                Stretch::Kept(a, b) => {
                    let pl = plan[a];
                    let p = &parts[pl.part];
                    let (i, _) = pl.kept.expect("kept");
                    let part = &para.parts[i];
                    let size = p.size.max(1.0) / part.em;
                    let spacing = setting.spacing(p);
                    let items =
                        kept_items(setting, plan, chars, a..b, end, l.stretch, size, spacing);
                    // A letter TeX's accent was drawn over, from another text object, keeps
                    // the move back to it under the accent, as the measure does (PB-73).
                    let last = (out.last_mut()).and_then(|w| Some((w.size, w.shows.last_mut()?)));
                    if let (Some(d), Some((size, (spacing, show)))) =
                        (under_accent(setting, parts, plan, chars, a), last)
                    {
                        let [tc, _, th] = *spacing;
                        let units = (size * th).max(1e-3);
                        push_items(show, vec![r(-(d - tc * th) * 1000.0 / units)]);
                    }
                    out.push(Written {
                        font: part.font.clone(),
                        size,
                        color: p.color,
                        paint: paint(p),
                        rise: p.rise,
                        flipped: setting.font(i).flipped(),
                        shows: vec![(spacing, tj(items))],
                    });
                }
                Stretch::Word(a, f, k) => {
                    let p = &parts[plan[a].part];
                    let size = p.size.max(1.0);
                    let spacing = setting.spacing(p);
                    let (_, names, pen) = pens.iter().find(|(g, ..)| *g == f).expect("a pen");
                    for s in pen.shows(k) {
                        out.push(Written {
                            font: names[s.font].as_bytes().to_vec(),
                            size,
                            color: p.color,
                            paint: paint(p),
                            rise: p.rise + s.rise * size / 1000.0,
                            flipped: false,
                            shows: vec![([spacing[0], 0.0, spacing[2]], tj(s.items.clone()))],
                        });
                    }
                }
                Stretch::Space(j) => {
                    // A move at the end of what is written before it: a space in the font
                    // the words go to, and the line's stretch.
                    let p = &parts[plan[j].part];
                    let [tc, _, th] = setting.spacing(p);
                    let space = (setters.iter().find(|(g, _)| *g == plan[j].away))
                        .map_or(0.25 * p.size, |(_, s)| s.width(" ", p.size));
                    let stretch = if chars[j] == ' ' { l.stretch } else { 0.0 };
                    let d = space * th + tc * th + stretch;
                    if let Some(w) = out.last_mut() {
                        let units = (w.size * w.shows.last().map_or(1.0, |s| s.0[2])).max(1e-3);
                        if let Some((_, show)) = w.shows.last_mut() {
                            push_items(show, vec![r(-d * 1000.0 / units)]);
                        }
                    }
                }
            }
        }
        Ok(out)
    }
}

/// A pen writing the words that go to a font: the font, its fonts' names in the page's
/// resources, and the pen.
type Pen = (FontChoice, Vec<String>, crate::layout::Pen);

/// A line written in place ([`Pdf::paragraph_ops`]): its index, its operators, and those
/// that put the text state back after it.
type Placed = (usize, Vec<Operation>, Vec<Operation>);

/// The lines written, to read back ([`Pdf::misread`]): each one's index, where it is, and
/// what it should read.
type Checks = Vec<(usize, Check, String)>;

/// An edited paragraph's set lines, for their decorations ([`Pdf::paragraph_decorated`]):
/// the lines, the edited text's characters, and the lines written's read-back places.
struct Set<'a> {
    lines: &'a [SetLine],
    chars: &'a [char],
    checks: &'a Checks,
}

/// A stretch of a line as it is written ([`Pdf::written_line`]): characters kept in their
/// part's font (from, to), a word in another font (its first character, the font, and its
/// place in that font's words), or a space between such words.
#[derive(Clone, Copy, Debug)]
enum Stretch {
    Kept(usize, usize),
    Word(usize, FontChoice, usize),
    Space(usize),
}

/// How far from the end of the letter before it the stretch of kept letters starting at
/// character `a` of an edit (`parts`, written as `plan` says, its characters `chars`) starts,
/// in points as the measure sets it ([`ParagraphSetting::measure`]), when that letter is
/// TeX's accent drawn over it from another text object ([`crate::Paragraph::accent_over`]), both
/// kept in their own codes and still next to each other: the move back under the accent.
fn under_accent(
    setting: &ParagraphSetting,
    parts: &[TextPart],
    plan: &[Planned],
    chars: &[char],
    a: usize,
) -> Option<f32> {
    let para = setting.paragraph();
    let own = |pl: &Planned| matches!(pl.kept, Some((_, Way::Own(_) | Way::WithBefore)));
    let (before, pl) = (&plan[a.checked_sub(1)?], &plan[a]);
    let k = before.from?;
    let next = own(before) && own(pl) && pl.from == Some(k + 1);
    if !next || chars[a].is_whitespace() || !para.accent_over(k) {
        return None;
    }
    let (i, _) = before.kept?;
    let scale = parts[before.part].size / para.spans[i].font_size.max(1e-3);
    Some(para.gap_after(k) * scale)
}

/// `items` added at the end of the text show `show` (a `TJ`).
fn push_items(show: &mut Operation, items: Vec<Object>) {
    let shown = show.operands.first().and_then(|o| o.as_array().ok());
    let mut all = shown.cloned().unwrap_or_default();
    all.extend(items);
    *show = tj(all);
}

/// The strings and moves of characters `range` of an edit (`chars`, written as `plan`
/// says, each kept in its part's font) in a line whose characters end at `end` (a soft
/// hyphen there shows), its spaces stretched by `stretch` points: each character's own
/// code where it keeps it, with what was drawn between it and its neighbour while that is
/// still its neighbour; the font's code for one typed; a space its code, or a move where the
/// font has none. `size` is the text size the show is written at, `spacing` its spacing.
#[allow(clippy::too_many_arguments)]
fn kept_items(
    setting: &ParagraphSetting,
    plan: &[Planned],
    chars: &[char],
    range: std::ops::Range<usize>,
    end: usize,
    stretch: f32,
    size: f32,
    spacing: [f32; 3],
) -> Vec<Object> {
    let para = setting.paragraph();
    let [tc, _, th] = spacing;
    let units = (size * th).max(1e-3);
    let mut items: Vec<Object> = Vec::new();
    let mut last: Option<usize> = None;
    let code = |items: &mut Vec<Object>, bytes: &[u8]| {
        let format = match bytes.len() {
            1 => StringFormat::Literal,
            _ => StringFormat::Hexadecimal,
        };
        put_code(items, bytes, format);
    };
    for j in range {
        let c = chars[j];
        let (i, way) = plan[j].kept.expect("kept");
        let font = setting.font(i);
        if c == '\u{AD}' {
            if j + 1 == end {
                match (way, plan[j].from.and_then(|k| setting.drawn(k))) {
                    (Way::Own(_), Some(d)) => {
                        if let Ink::Code { bytes, format, .. } = &d.ink {
                            put_code(&mut items, bytes, *format);
                        }
                    }
                    _ => {
                        if let Ok(b) = font.code('-') {
                            code(&mut items, &b);
                        }
                    }
                }
            }
            last = None;
            continue;
        }
        if c.is_whitespace() {
            match font.code(' ') {
                Ok(b) => code(&mut items, &b),
                Err(_) => {
                    let d = setting.space_em() * size * para_em(setting, i) + tc * th;
                    items.push(r(-d * 1000.0 / units));
                }
            }
            if stretch != 0.0 && c == ' ' {
                items.push(r(-stretch * 1000.0 / units));
            }
            last = None;
            continue;
        }
        match way {
            Way::Own(o) => {
                let Some(d) = setting.drawn(o) else {
                    continue;
                };
                if o > 0 && last == Some(o - 1) && para.neighbours(o - 1) {
                    items.extend(d.after_last.iter().cloned());
                    items.extend(d.before.iter().cloned());
                }
                match &d.ink {
                    Ink::Code {
                        bytes, format, n, ..
                    } => {
                        put_code(&mut items, bytes, *format);
                        last = Some(o + n - 1);
                    }
                    Ink::Move(x) => {
                        items.push(r(*x));
                        last = Some(o);
                    }
                }
            }
            Way::Code => {
                if let Ok(b) = font.code(c) {
                    code(&mut items, &b);
                }
                last = None;
            }
            Way::WithBefore | Way::Space | Way::Missing => {}
        }
    }
    items
}

/// The em of the paragraph's part `i` ([`crate::fonts::FontCtx::em`]): what a size in
/// points is over the text size its shows are written at.
fn para_em(setting: &ParagraphSetting, i: usize) -> f32 {
    setting.paragraph().parts.get(i).map_or(1.0, |p| p.em)
}

/// A matrix as an operator's operands (no "-0").
fn matrix(m: M) -> Vec<Object> {
    let plain = |x: f32| if x == 0.0 { 0.0 } else { x };
    m.iter().map(|&x| r(plain(x))).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use lopdf::{dictionary, Dictionary, Stream};

    /// A line that didn't read back as typed in its own font is written again with its
    /// letters in the font its words go to ([`Pdf::edit_paragraph`]'s second try): it reads
    /// as typed, and the other lines stay as they were.
    #[test]
    fn a_line_written_again_in_the_font_its_words_go_to_reads_as_typed() {
        let mut p = Pdf::new();
        p.insert_blank(0, 612.0, 792.0).unwrap();
        let font = p.doc.add_object(dictionary! {
            "Type" => "Font", "Subtype" => "Type1", "BaseFont" => "Helvetica",
            "Encoding" => "WinAnsiEncoding",
        });
        let lines = [
            "The harbour was quiet that morning and",
            "the fishing boats sat low in the water",
            "while gulls argued over the nets that",
            "dried along the wall of the old quay.",
        ];
        let mut content = String::new();
        for (k, l) in lines.iter().enumerate() {
            let y = 700.0 - 13.2 * k as f32;
            content.push_str(&format!("BT /F1 11 Tf 72 {y} Td ({l}) Tj ET\n"));
        }
        let s = p
            .doc
            .add_object(Stream::new(Dictionary::new(), content.into_bytes()));
        let page = p.page_ids()[0];
        let res = dictionary! { "Font" => dictionary! { "F1" => font } };
        let d = p.doc.get_dictionary_mut(page).unwrap();
        d.set("Contents", s);
        d.set("Resources", res);
        let blocks = p.page_blocks(0).unwrap();
        let para = p.paragraph(0, &blocks[0], &p.document_words()).unwrap();
        let setting = p.paragraph_setting(&para).unwrap();
        // The second line's "low" typed as "lot", as wide: that line alone set again.
        let mut parts = para.parts();
        parts[1].text = parts[1].text.replacen("low", "lot", 1);
        let k = parts[1].text.find("lot").unwrap() + 2;
        let from = parts[1].from.unwrap();
        parts[1].origins = (0..parts[1].text.chars().count())
            .map(|j| (j != k).then_some(from + j))
            .collect();
        let set = setting.set(&parts);
        let kept: Vec<Option<usize>> = set.lines.iter().map(|l| l.kept).collect();
        assert_eq!(kept, [Some(0), None, Some(2), Some(3)]);
        let (id, parsed) = p.load_edit(0).unwrap();
        let own = [true, false, true, true];
        let (ops, checks) = p
            .paragraph_ops(id, &parsed, &setting, &parts, &set.lines, &own)
            .unwrap();
        let misread = p.misread(id, &parsed.inline, &ops, &checks).unwrap();
        assert!(misread.is_empty(), "{misread:?}");
        // Its show in a font added to the page, the others' in /F1 as they were.
        let fonts: Vec<Vec<u8>> = (ops.iter().filter(|o| o.operator == "Tf"))
            .map(|o| o.operands[0].as_name().unwrap().to_vec())
            .collect();
        assert!(fonts.iter().any(|f| f != b"F1"), "{fonts:?}");
        let depth = parsed.depth_left;
        p.commit(id, ops, depth, &parsed.inline).unwrap();
        let texts: Vec<String> = (p.page_objects(0).unwrap().into_iter())
            .filter_map(|o| o.text)
            .collect();
        let mut want: Vec<String> = lines.iter().map(|l| l.to_string()).collect();
        want[1] = want[1].replacen("low", "lot", 1);
        assert_eq!(texts, want);
    }
}

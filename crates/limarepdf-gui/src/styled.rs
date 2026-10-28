//! A line of text being typed with a style for each character (the line styles design,
//! section 6): typed text takes the style of the text around it; Bold, Italic, the font,
//! size and colour change what is selected, the word at the caret, or what is typed next,
//! as in a word processor; and the line goes to core in parts.

use crate::editor::{same_color, word_around};
use crate::{font_list, page_text};
use limarepdf_core::{Family, FontChoice, PageObject, PartFont, TextPart, TextSpan};

/// How a character is set.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CharStyle {
    /// The font as the inspector shows it.
    pub font: FontChoice,
    /// The line's part ([`PageObject::spans`]) the character came from, or was typed in:
    /// it is written in that part's own font while `font` is still the one the part was
    /// found in.
    pub span: Option<usize>,
    pub size: f32,
    pub color: [f32; 3],
    /// How far it is raised above the line's baseline, in points (lowered when below
    /// zero).
    pub rise: f32,
}

impl Default for CharStyle {
    fn default() -> Self {
        Self {
            font: FontChoice::default(),
            span: None,
            size: 12.0,
            color: [0.0; 3],
            rise: 0.0,
        }
    }
}

/// What the inspector shows for a selection: each value, or `None` where the selection
/// mixes them.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Shown {
    pub font: Option<FontChoice>,
    pub size: Option<f32>,
    pub color: Option<[f32; 3]>,
    /// All of it is bold.
    pub bold: bool,
    /// All of it is italic.
    pub italic: bool,
}

/// Text with a style for each character.
#[derive(Clone, Debug, Default)]
pub struct Styled {
    text: String,
    /// One per character of `text`.
    styles: Vec<CharStyle>,
    /// The style set for what is typed next at this character, until the caret moves.
    next: Option<(usize, CharStyle)>,
    /// The style typed text takes in an empty line.
    base: CharStyle,
    /// The line's parts as they were read, each with the font it is shown as.
    spans: Vec<(TextSpan, FontChoice)>,
    /// A style command was used.
    restyled: bool,
    /// One per character of `text`: where it is in the line as it was read, `None` for
    /// one typed.
    origins: Vec<Option<usize>>,
    /// The line's text as it was read.
    read: Vec<char>,
}

impl Styled {
    /// `text` all in `style`: new text, typed in the inspector's choices.
    pub fn plain(text: &str, style: CharStyle) -> Self {
        let n = text.chars().count();
        Self {
            text: text.to_string(),
            styles: vec![style; n],
            base: style,
            origins: vec![None; n],
            ..Default::default()
        }
    }

    /// Line `o` as it is, each character in the style of its part; `font_of` gives the
    /// font a part's PDF font is shown as.
    pub fn of(o: &PageObject, font_of: impl Fn(&str) -> FontChoice) -> Self {
        let text = o.text.clone().unwrap_or_default();
        let n = text.chars().count();
        let spans: Vec<_> = o
            .spans
            .iter()
            .map(|s| (s.clone(), font_of(&s.font_name)))
            .collect();
        let mut found = vec![None; n];
        for (i, (s, font)) in spans.iter().enumerate() {
            let style = CharStyle {
                font: *font,
                span: Some(i),
                size: s.font_size,
                color: s.fill.unwrap_or([0.0; 3]),
                rise: s.rise,
            };
            for c in found.iter_mut().take(s.chars.1.min(n)).skip(s.chars.0) {
                *c = Some(style);
            }
        }
        let whole = spans
            .first()
            .map(|(s, font)| CharStyle {
                font: *font,
                span: Some(0),
                size: s.font_size,
                color: s.fill.unwrap_or([0.0; 3]),
                rise: s.rise,
            })
            .unwrap_or_else(|| CharStyle {
                font: font_of(o.font_name.as_deref().unwrap_or("")),
                span: None,
                size: o.font_size.unwrap_or(12.0),
                color: o.fill.unwrap_or([0.0; 3]),
                rise: 0.0,
            });
        // A character in no part takes the style of the one before it.
        let mut last = found.iter().flatten().next().copied().unwrap_or(whole);
        let styles = found
            .into_iter()
            .map(|s| {
                last = s.unwrap_or(last);
                last
            })
            .collect();
        Self {
            read: text.chars().collect(),
            origins: (0..n).map(Some).collect(),
            text,
            styles,
            next: None,
            base: whole,
            spans,
            restyled: false,
        }
    }

    /// Whether a style command was used, so more than the words may have changed.
    pub fn restyled(&self) -> bool {
        self.restyled
    }

    pub fn text(&self) -> &str {
        &self.text
    }

    /// Each character's style.
    pub fn styles(&self) -> &[CharStyle] {
        &self.styles
    }

    /// Take the text as an editor changed it to `new`, with the caret at character
    /// `caret` when known. Characters removed take their styles with them; those typed
    /// take the style set for the next typing, else that of the first character typed over,
    /// else that of the one before them (after them at the start). Returns whether the
    /// text changed.
    pub fn sync(&mut self, new: &str, caret: Option<usize>) -> bool {
        if new == self.text {
            return false;
        }
        let old: Vec<char> = self.text.chars().collect();
        let now: Vec<char> = new.chars().collect();
        let room = old.len().min(now.len());
        // What is typed ends at the caret, which tells "ab" + "b" typed after the "a"
        // from "b" typed after the "b".
        let mut suffix = old
            .iter()
            .rev()
            .zip(now.iter().rev())
            .take_while(|(a, b)| a == b)
            .count()
            .min(room);
        if let Some(c) = caret {
            suffix = suffix.min(now.len().saturating_sub(c));
        }
        let prefix = old
            .iter()
            .zip(&now)
            .take_while(|(a, b)| a == b)
            .count()
            .min(room - suffix);
        let removed = prefix..old.len() - suffix;
        let typed = match self.next {
            Some((_, s)) => s,
            None if !removed.is_empty() => self.styles[prefix],
            None => self.typing_style(prefix),
        };
        let added = now.len() - suffix - prefix;
        self.origins
            .splice(removed.clone(), std::iter::repeat_n(None, added));
        self.styles
            .splice(removed, std::iter::repeat_n(typed, added));
        if now.is_empty() {
            self.base = typed;
        }
        self.text = new.to_string();
        self.next = None;
        true
    }

    /// The text as an editor has it this frame, `new`, styled as [`Self::sync`] would
    /// style it, for drawing before the caret is known.
    pub fn preview(&self, new: &str) -> Styled {
        let mut s = self.clone();
        s.sync(new, None);
        s
    }

    /// The caret or selection is now `sel` (characters): the style set for the next typing
    /// is forgotten once the caret moves away from where it was set.
    pub fn caret_at(&mut self, sel: (usize, usize)) {
        if self.next.is_some_and(|(at, _)| sel != (at, at)) {
            self.next = None;
        }
    }

    /// The style text typed at character `at` takes.
    fn typing_style(&self, at: usize) -> CharStyle {
        match self.next {
            Some((p, s)) if p == at => s,
            _ => at
                .checked_sub(1)
                .and_then(|i| self.styles.get(i))
                .or(self.styles.get(at))
                .copied()
                .unwrap_or(self.base),
        }
    }

    /// The characters (from, to) a style command with `sel` selected changes: the
    /// selection, else the word the caret is inside; `None` between words or at either end
    /// of one, where it sets what is typed next.
    fn range(&self, sel: (usize, usize)) -> Option<(usize, usize)> {
        let n = self.styles.len();
        let (a, b) = (sel.0.min(sel.1).min(n), sel.0.max(sel.1).min(n));
        if a < b {
            return Some((a, b));
        }
        let (wa, wb) = word_around(&self.text, a);
        (wa < a && a < wb).then_some((wa, wb))
    }

    /// The styles a style command with `sel` selected looks at.
    fn looked_at(&self, sel: (usize, usize)) -> Vec<CharStyle> {
        match self.range(sel) {
            Some((a, b)) => self.styles[a..b].to_vec(),
            None => vec![self.typing_style(sel.0.min(sel.1))],
        }
    }

    /// Change the style of what a style command with `sel` selected changes ([`Self::range`]),
    /// or of what is typed next at the caret.
    fn restyle(&mut self, sel: (usize, usize), f: impl Fn(CharStyle) -> CharStyle) {
        self.restyled = true;
        match self.range(sel) {
            Some((a, b)) => {
                for s in &mut self.styles[a..b] {
                    *s = f(*s);
                }
            }
            None => {
                let at = sel.0.min(sel.1).min(self.styles.len());
                self.next = Some((at, f(self.typing_style(at))));
            }
        }
    }

    /// Bold on or off, by Word's rule: all bold becomes not bold, otherwise all becomes
    /// bold, each character only when its family has the face.
    pub fn bold(&mut self, sel: (usize, usize)) {
        self.switch(sel, true);
    }

    /// Italic on or off, as [`Self::bold`].
    pub fn italic(&mut self, sel: (usize, usize)) {
        self.switch(sel, false);
    }

    fn switch(&mut self, sel: (usize, usize), bold: bool) {
        let all = self.looked_at(sel).iter().all(|s| is_on(s.font, bold));
        self.turn(sel, bold, !all);
    }

    /// Bold (or with `bold` false, Italic) turned `on` or off for what a style command
    /// with `sel` selected changes, each character only when its family has the face.
    pub fn turn(&mut self, sel: (usize, usize), bold: bool, on: bool) {
        self.restyle(sel, |mut s| {
            if is_on(s.font, bold) != on {
                s.font = page_text::switched(s.font, bold, !bold);
            }
            s
        });
    }

    /// The font family set to `family`, each character in the face nearest its own that
    /// the family has.
    pub fn set_family(&mut self, sel: (usize, usize), family: Family) {
        self.restyle(sel, |mut s| {
            font_list::choose(&mut s.font, family);
            s
        });
    }

    pub fn set_size(&mut self, sel: (usize, usize), size: f32) {
        self.restyle(sel, |s| CharStyle { size, ..s });
    }

    pub fn set_color(&mut self, sel: (usize, usize), color: [f32; 3]) {
        self.restyle(sel, |s| CharStyle { color, ..s });
    }

    /// The font what is typed with `sel` selected goes into: that of the first character
    /// typed over, else the caret's typing style.
    pub fn typing_font(&self, sel: (usize, usize)) -> FontChoice {
        let a = sel.0.min(sel.1);
        match self.styles.get(a) {
            Some(s) if sel.0 != sel.1 => s.font,
            _ => self.typing_style(a).font,
        }
    }

    /// The font `part` (of [`Self::parts`]) is shown as.
    pub fn font_of(&self, part: &TextPart) -> FontChoice {
        match part.font {
            PartFont::Span(i) => self.spans.get(i).map_or(self.base.font, |(_, f)| *f),
            PartFont::Choice(f) => f,
        }
    }

    /// The parts as drawn: [`Self::parts`], or for no text an empty part in the style
    /// typed text will take.
    pub fn drawn_parts(&self) -> Vec<TextPart> {
        let parts = self.parts();
        if !parts.is_empty() {
            return parts;
        }
        let s = self.typing_style(0);
        vec![TextPart {
            text: String::new(),
            font: match self.kept(&s) {
                Some(i) => PartFont::Span(i),
                None => PartFont::Choice(s.font),
            },
            size: s.size,
            color: s.color,
            rise: s.rise,
            from: None,
        }]
    }

    /// The style of the first character a style command with `sel` selected looks at.
    pub fn first(&self, sel: (usize, usize)) -> CharStyle {
        self.looked_at(sel)[0]
    }

    /// The colours of what a style command with `sel` selected looks at, each once, in
    /// order.
    pub fn colors(&self, sel: (usize, usize)) -> Vec<[f32; 3]> {
        let mut out: Vec<[f32; 3]> = Vec::new();
        for s in self.looked_at(sel) {
            if !out.iter().any(|c| same_color(*c, s.color)) {
                out.push(s.color);
            }
        }
        out
    }

    /// The style text typed at the end takes: what new text carries on in.
    pub fn end_style(&self) -> CharStyle {
        self.typing_style(self.styles.len())
    }

    /// What the inspector shows with `sel` selected: the selection's values, or those of
    /// what is typed at the caret.
    pub fn shown(&self, sel: (usize, usize)) -> Shown {
        let (a, b) = (sel.0.min(sel.1), sel.0.max(sel.1));
        let looked = if a < b {
            self.styles[a.min(self.styles.len())..b.min(self.styles.len())].to_vec()
        } else {
            vec![self.typing_style(a)]
        };
        let first = looked.first().copied().unwrap_or(self.base);
        let all = |same: &dyn Fn(&CharStyle) -> bool| looked.iter().all(same);
        Shown {
            font: all(&|s| self.same_font(s, &first)).then_some(first.font),
            size: all(&|s| (s.size - first.size).abs() < 0.01 * first.size.max(0.01))
                .then_some(first.size),
            color: all(&|s| same_color(s.color, first.color)).then_some(first.color),
            bold: all(&|s| s.font.bold),
            italic: all(&|s| s.font.italic),
        }
    }

    /// Whether two characters look to be in the same font: two in their parts' own fonts
    /// when those are one font (two subsets of it, say), others when they are the same
    /// font.
    fn same_font(&self, a: &CharStyle, b: &CharStyle) -> bool {
        self.same_font_as(a, self, b)
    }

    /// [`Self::same_font`] for `a` here and `b` in `other`, another line.
    fn same_font_as(&self, a: &CharStyle, other: &Styled, b: &CharStyle) -> bool {
        match (self.kept(a), other.kept(b)) {
            (Some(i), Some(j)) => self.spans[i].0.same_font(&other.spans[j].0),
            (None, None) => a.font == b.font,
            _ => false,
        }
    }

    /// Where each character came from in the line as it was read, for those whose own
    /// shows can be kept ([`TextPart::from`]): runs of characters still as they were read,
    /// cut back to whole words (or to the ends of the line's parts), as a word changed in
    /// part is written again whole. A run's ends part no word as the line was read, nor
    /// as it is now: letters typed onto a word change it.
    fn owned(&self) -> Vec<Option<usize>> {
        let now: Vec<char> = self.text.chars().collect();
        let n = self.origins.len();
        let space = |i: usize| now.get(i).is_some_and(|c| c.is_whitespace());
        let apart = |c: usize| c == 0 || c == n || space(c) || space(c - 1);
        let mut out = vec![None; n];
        let mut i = 0;
        while i < n {
            let Some(o) = self.origins[i] else {
                i += 1;
                continue;
            };
            let mut j = i + 1;
            while j < n && self.origins[j] == Some(o + (j - i)) {
                j += 1;
            }
            let (mut a, mut b) = (o, o + (j - i));
            if !(self.cuttable(a) && apart(i)) {
                a += 1;
                while a < b && !self.cuttable(a) {
                    a += 1;
                }
            }
            if b > a && !(self.cuttable(b) && apart(j)) {
                b -= 1;
                while b > a && !self.cuttable(b) {
                    b -= 1;
                }
            }
            for k in a..b {
                out[i + k - o] = Some(k);
            }
            i = j;
        }
        out
    }

    /// Whether the line as read may be cut at character `at` without parting a word: at
    /// its ends, next to a space, or where one of its parts starts or ends.
    fn cuttable(&self, at: usize) -> bool {
        let space = |i: usize| self.read.get(i).is_some_and(|c| c.is_whitespace());
        at == 0
            || at == self.read.len()
            || space(at)
            || at.checked_sub(1).is_some_and(space)
            || (self.spans.iter()).any(|(s, _)| s.chars.0 == at || s.chars.1 == at)
    }

    /// The part whose own font a character in `s` is written in.
    fn kept(&self, s: &CharStyle) -> Option<usize> {
        s.span
            .filter(|&i| self.spans.get(i).is_some_and(|(_, f)| *f == s.font))
    }

    /// The text in parts of one style each, as core writes it.
    pub fn parts(&self) -> Vec<TextPart> {
        self.parts_in(0..self.styles.len())
    }

    /// The lines of new text to write (Enter starts one), each in parts, with how far its
    /// baseline is below the first line's, in points: each line [`page_text::line_gap`]
    /// below the one before, by the two lines' largest sizes, as they are drawn while
    /// typing. Blank lines are left out but keep their space; spaces ending a line go.
    pub fn lines(&self) -> Vec<(f32, Vec<TextPart>)> {
        let chars: Vec<char> = self.text.chars().collect();
        let mut out = Vec::new();
        let (mut start, mut dy, mut above) = (0, 0.0, None);
        while start <= chars.len() {
            let end = chars[start..]
                .iter()
                .position(|&c| c == '\n')
                .map_or(chars.len(), |i| start + i);
            // Its largest size, the line break ending it included.
            let size = self.styles[start..(end + 1).min(chars.len())]
                .iter()
                .map(|s| s.size)
                .fold(0.0, f32::max);
            if let Some(a) = above {
                dy += page_text::line_gap(a, size);
            }
            above = Some(size);
            let mut last = end;
            while last > start && chars[last - 1].is_whitespace() {
                last -= 1;
            }
            if last > start {
                out.push((dy, self.parts_in(start..last)));
            }
            start = end + 1;
        }
        out
    }

    /// Characters `range` in parts of one style each. Characters in their part's own font
    /// that are still as they were read ([`Self::owned`]) are parts of their own, which
    /// say where they were.
    fn parts_in(&self, range: std::ops::Range<usize>) -> Vec<TextPart> {
        let owned = self.owned();
        let mut out: Vec<(TextPart, usize)> = Vec::new();
        let chars = self.text.chars().zip(&self.styles).zip(owned);
        for ((c, s), at) in chars.skip(range.start).take(range.len()) {
            let (font, from) = match self.kept(s) {
                Some(i) => (PartFont::Span(i), at),
                None => (PartFont::Choice(s.font), None),
            };
            let style = (font, s.size, s.color, s.rise);
            match out.last_mut() {
                Some((p, n))
                    if (p.font, p.size, p.color, p.rise) == style
                        && p.from.map(|f| f + *n) == from =>
                {
                    p.text.push(c);
                    *n += 1;
                }
                _ => out.push((
                    TextPart {
                        text: c.to_string(),
                        font,
                        size: s.size,
                        color: s.color,
                        rise: s.rise,
                        from,
                    },
                    1,
                )),
            }
        }
        out.into_iter().map(|(p, _)| p).collect()
    }

    /// Whether this is `other` to the eye: the same text in the same styles.
    pub fn same_as(&self, other: &Styled) -> bool {
        self.text == other.text
            && self.styles.iter().zip(&other.styles).all(|(a, b)| {
                a.font == b.font
                    && self.kept(a) == other.kept(b)
                    && a.size == b.size
                    && same_color(a.color, b.color)
                    && a.rise == b.rise
            })
    }
}

/// Whether font `f` is bold (with `bold` false, italic).
fn is_on(f: FontChoice, bold: bool) -> bool {
    if bold {
        f.bold
    } else {
        f.italic
    }
}

/// Text that the inspector's style controls change: a line with what is selected in it
/// ([`InLine`]), or words selected across lines ([`Across`]).
pub trait Restyle {
    /// What the inspector shows: each value, or `None` where the text mixes them.
    fn shown(&self) -> Shown;
    /// The style of the first character changed.
    fn first(&self) -> CharStyle;
    /// The colours of what is changed, each once, in order.
    fn colors(&self) -> Vec<[f32; 3]>;
    /// Bold on or off by Word's rule ([`Styled::bold`]).
    fn bold(&mut self);
    /// Italic on or off, as [`Self::bold`].
    fn italic(&mut self);
    fn set_family(&mut self, family: Family);
    fn set_size(&mut self, size: f32);
    fn set_color(&mut self, color: [f32; 3]);
}

/// A line's text with `sel` selected in it: a style command changes what is selected,
/// the word at the caret, or what is typed next ([`Styled`]).
pub struct InLine<'a> {
    pub text: &'a mut Styled,
    pub sel: (usize, usize),
}

impl Restyle for InLine<'_> {
    fn shown(&self) -> Shown {
        self.text.shown(self.sel)
    }

    fn first(&self) -> CharStyle {
        self.text.first(self.sel)
    }

    fn colors(&self) -> Vec<[f32; 3]> {
        self.text.colors(self.sel)
    }

    fn bold(&mut self) {
        self.text.bold(self.sel);
    }

    fn italic(&mut self) {
        self.text.italic(self.sel);
    }

    fn set_family(&mut self, family: Family) {
        self.text.set_family(self.sel, family);
    }

    fn set_size(&mut self, size: f32) {
        self.text.set_size(self.sel, size);
    }

    fn set_color(&mut self, color: [f32; 3]) {
        self.text.set_color(self.sel, color);
    }
}

/// Words selected across lines, as the read tool selects them: each line's text, and the
/// characters selected in it (from, to; never empty). Style commands treat them as one
/// selection, as a word processor treats a selection across paragraphs.
#[derive(Clone, Debug, Default)]
pub struct Across {
    pub lines: Vec<(Styled, Vec<(usize, usize)>)>,
}

impl Across {
    /// Each selected character's line and style.
    fn each(&self) -> impl Iterator<Item = (&Styled, &CharStyle)> {
        (self.lines.iter()).flat_map(|(t, ranges)| {
            ranges
                .iter()
                .flat_map(move |&(a, b)| &t.styles[a..b])
                .map(move |s| (t, s))
        })
    }

    /// Change each selected range of each line with `f`.
    fn each_range(&mut self, f: impl Fn(&mut Styled, (usize, usize))) {
        for (t, ranges) in &mut self.lines {
            for &r in ranges.iter() {
                f(t, r);
            }
        }
    }

    fn switch(&mut self, bold: bool) {
        let all = self.each().all(|(_, s)| is_on(s.font, bold));
        self.each_range(|t, r| t.turn(r, bold, !all));
    }
}

impl Restyle for Across {
    fn shown(&self) -> Shown {
        let Some((t0, first)) = self.each().next() else {
            return Shown {
                font: None,
                size: None,
                color: None,
                bold: false,
                italic: false,
            };
        };
        let all = |same: &dyn Fn(&Styled, &CharStyle) -> bool| self.each().all(|(t, s)| same(t, s));
        Shown {
            font: all(&|t, s| t.same_font_as(s, t0, first)).then_some(first.font),
            size: all(&|_, s| (s.size - first.size).abs() < 0.01 * first.size.max(0.01))
                .then_some(first.size),
            color: all(&|_, s| same_color(s.color, first.color)).then_some(first.color),
            bold: all(&|_, s| s.font.bold),
            italic: all(&|_, s| s.font.italic),
        }
    }

    fn first(&self) -> CharStyle {
        self.each().next().map(|(_, s)| *s).unwrap_or_default()
    }

    fn colors(&self) -> Vec<[f32; 3]> {
        let mut out: Vec<[f32; 3]> = Vec::new();
        for (_, s) in self.each() {
            if !out.iter().any(|c| same_color(*c, s.color)) {
                out.push(s.color);
            }
        }
        out
    }

    fn bold(&mut self) {
        self.switch(true);
    }

    fn italic(&mut self) {
        self.switch(false);
    }

    fn set_family(&mut self, family: Family) {
        self.each_range(|t, r| t.set_family(r, family));
    }

    fn set_size(&mut self, size: f32) {
        self.each_range(|t, r| t.set_size(r, size));
    }

    fn set_color(&mut self, color: [f32; 3]) {
        self.each_range(|t, r| t.set_color(r, color));
    }
}

/// Typing within this many seconds of the last, where it left the caret, joins its undo
/// step, as text editors group typing.
const JOIN: f64 = 1.0;
/// The most steps kept.
const STEPS: usize = 100;

/// Undo and redo inside a line being typed (the design, section 6.5): typing and style
/// changes in order. It watches the text once a frame ([`Self::observe`]): typing joins
/// the step before it as text editors join it, and each style change is a step of its own,
/// changes made while a list or colour picker is open joining into one.
#[derive(Clone, Debug, Default)]
pub struct History {
    undo: Vec<(Styled, (usize, usize))>,
    redo: Vec<(Styled, (usize, usize))>,
    /// The text when last watched, and the selection then.
    last: Styled,
    sel: (usize, usize),
    /// The last change was typing, at this time, leaving the caret here.
    typing: Option<(f64, usize)>,
    /// The last change was a style change not yet finished.
    styling: bool,
}

impl History {
    /// Nothing to undo yet in `text`.
    pub fn new(text: &Styled) -> Self {
        Self {
            last: text.clone(),
            ..Default::default()
        }
    }

    /// Watch `text` this frame, at time `now`, with `sel` selected when known: a change
    /// since the last frame becomes a step to undo, unless it carries on the one before.
    /// `settled` says no list, picker or button is in use.
    pub fn observe(&mut self, text: &Styled, sel: Option<(usize, usize)>, now: f64, settled: bool) {
        let before = self.sel;
        self.sel = sel.unwrap_or(before);
        if text.text != self.last.text {
            let joins = self
                .typing
                .is_some_and(|(at, caret)| now - at < JOIN && before == (caret, caret));
            if !joins {
                self.push(before);
            }
            self.typing = Some((now, self.sel.1));
            self.styling = false;
        } else if !text.same_as(&self.last) {
            if !self.styling {
                self.push(before);
            }
            self.typing = None;
            self.styling = !settled;
        } else {
            self.styling &= !settled;
            return;
        }
        self.last = text.clone();
        self.redo.clear();
    }

    /// The text when last watched becomes a step to undo, with the selection `sel`.
    fn push(&mut self, sel: (usize, usize)) {
        let mut was = self.last.clone();
        was.next = None;
        self.undo.push((was, sel));
        if self.undo.len() > STEPS {
            self.undo.remove(0);
        }
    }

    /// Whether there is a step to take back.
    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }

    /// Whether there is a step taken back to redo.
    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }

    /// Take `text` back a step: returns the selection it had then, or `None` with nothing
    /// to undo.
    pub fn undo(&mut self, text: &mut Styled) -> Option<(usize, usize)> {
        let (was, sel) = self.undo.pop()?;
        self.redo.push((std::mem::replace(text, was), self.sel));
        Some(self.went_to(text, sel))
    }

    /// Take `text` forward a step undone: returns the selection it had then.
    pub fn redo(&mut self, text: &mut Styled) -> Option<(usize, usize)> {
        let (next, sel) = self.redo.pop()?;
        self.undo.push((std::mem::replace(text, next), self.sel));
        Some(self.went_to(text, sel))
    }

    /// `text` is now a step taken back or forward, with `sel` selected.
    fn went_to(&mut self, text: &Styled, sel: (usize, usize)) -> (usize, usize) {
        self.last = text.clone();
        self.sel = sel;
        self.typing = None;
        self.styling = false;
        sel
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bold() -> CharStyle {
        CharStyle {
            font: FontChoice {
                bold: true,
                ..Default::default()
            },
            ..Default::default()
        }
    }

    /// `text` in plain style, with the characters `from..to` bold.
    fn with_bold(text: &str, from: usize, to: usize) -> Styled {
        let mut s = Styled::plain(text, CharStyle::default());
        for c in &mut s.styles[from..to] {
            *c = bold();
        }
        s
    }

    /// Each character's boldness, as `.` and `B`.
    fn marks(s: &Styled) -> String {
        s.styles
            .iter()
            .map(|c| if c.font.bold { 'B' } else { '.' })
            .collect()
    }

    #[test]
    fn typed_text_takes_the_style_before_it() {
        let mut s = with_bold("ab", 1, 2);
        // "b" typed after the "a": plain, though the text alone can't tell it from "b"
        // typed after the "b".
        assert!(s.sync("abb", Some(2)));
        assert_eq!(marks(&s), "..B");
        let mut s = with_bold("ab", 1, 2);
        s.sync("abb", Some(3));
        assert_eq!(marks(&s), ".BB");
        // At the start, the style after it.
        let mut s = with_bold("ab", 0, 1);
        s.sync("xab", Some(1));
        assert_eq!(marks(&s), "BB.");
        // Typing over a selection takes its first character's style.
        let mut s = with_bold("a bold word", 2, 6);
        s.sync("a big word", Some(5));
        assert_eq!(marks(&s), "..BBB.....");
        // Deleting keeps the others' styles.
        let mut s = with_bold("abc", 1, 2);
        s.sync("ac", Some(1));
        assert_eq!(marks(&s), "..");
        assert!(!s.sync("ac", Some(1)));
    }

    #[test]
    fn an_emptied_line_types_in_its_first_style() {
        let mut s = with_bold("ab", 0, 1);
        s.sync("", Some(0));
        s.sync("x", Some(1));
        assert_eq!(marks(&s), "B");
    }

    #[test]
    fn style_commands_follow_word_processors() {
        // A selection.
        let mut s = Styled::plain("one two three", CharStyle::default());
        s.bold((4, 7));
        assert_eq!(marks(&s), "....BBB......");
        // Partly bold becomes all bold; all bold becomes plain.
        s.bold((2, 6));
        assert_eq!(marks(&s), "..BBBBB......");
        s.bold((6, 2));
        assert_eq!(marks(&s), "......B......");
        // The caret inside a word: that word.
        let mut s = Styled::plain("one two three", CharStyle::default());
        s.bold((9, 9));
        assert_eq!(marks(&s), "........BBBBB");
        // Between words, or at a word's end: what is typed next, until the caret moves.
        let mut s = Styled::plain("one two", CharStyle::default());
        s.bold((3, 3));
        assert_eq!(marks(&s), ".......");
        assert!(s.shown((3, 3)).bold);
        s.sync("oneX two", Some(4));
        assert_eq!(marks(&s), "...B....");
        s.sync("oneXY two", Some(5));
        assert_eq!(marks(&s), "...BB....");
        let mut s = Styled::plain("one two", CharStyle::default());
        s.bold((7, 7));
        s.caret_at((2, 2));
        s.caret_at((7, 7));
        s.sync("one twoX", Some(8));
        assert_eq!(marks(&s), "........");
    }

    #[test]
    fn size_and_colour_change_the_selection_and_show_blank_when_mixed() {
        let mut s = Styled::plain("one two", CharStyle::default());
        s.set_size((0, 3), 20.0);
        s.set_color((5, 5), [1.0, 0.0, 0.0]);
        let sizes: Vec<f32> = s.styles.iter().map(|c| c.size).collect();
        assert_eq!(sizes, [20.0, 20.0, 20.0, 12.0, 12.0, 12.0, 12.0]);
        assert_eq!(s.styles[4].color, [1.0, 0.0, 0.0]);
        assert_eq!(s.styles[3].color, [0.0; 3]);
        let all = s.shown((0, 7));
        assert_eq!((all.size, all.color), (None, None));
        assert_eq!(all.font, Some(FontChoice::default()));
        let some = s.shown((0, 2));
        assert_eq!((some.size, some.color), (Some(20.0), Some([0.0; 3])));
        // At the caret: the character before it.
        assert_eq!(s.shown((3, 3)).size, Some(20.0));
        assert_eq!(s.first((4, 7)).color, [1.0, 0.0, 0.0]);
    }

    #[test]
    fn a_line_goes_to_core_in_parts_of_one_style() {
        let mut s = Styled::plain("one two three", CharStyle::default());
        s.bold((4, 7));
        let parts = s.parts();
        let texts: Vec<&str> = parts.iter().map(|p| p.text.as_str()).collect();
        assert_eq!(texts, ["one ", "two", " three"]);
        assert_eq!(parts[1].font, PartFont::Choice(bold().font));
        assert!(Styled::plain("", CharStyle::default()).parts().is_empty());
    }

    #[test]
    fn new_text_is_written_a_line_at_a_time_spaced_by_its_sizes() {
        let mut s = Styled::plain("", CharStyle::default());
        s.sync("Big  \n\n  small\n ", None);
        // "Big" 30 points, its line break included; the rest 10.
        s.set_size((0, 6), 30.0);
        let lines = s.lines();
        let texts: Vec<Vec<&str>> = lines
            .iter()
            .map(|(_, parts)| parts.iter().map(|p| p.text.as_str()).collect())
            .collect();
        // Spaces ending a line go; blank lines are left out but keep their space.
        assert_eq!(texts, [vec!["Big"], vec!["  small"]]);
        let gap = page_text::line_gap(30.0, 12.0) + page_text::line_gap(12.0, 12.0);
        assert_eq!(lines[1].0, gap);
        // A line in two styles is one line in two parts.
        let mut s = Styled::plain("one two", CharStyle::default());
        s.bold((4, 7));
        let lines = s.lines();
        assert_eq!((lines.len(), lines[0].1.len()), (1, 2));
        assert!(Styled::plain(" \n ", CharStyle::default())
            .lines()
            .is_empty());
        assert_eq!(s.end_style().font, bold().font);
    }

    #[test]
    fn undo_takes_back_typing_and_styles_in_order() {
        let mut s = Styled::plain("one two", CharStyle::default());
        let mut h = History::new(&s);
        let mut at = 0.0;
        let mut seen = |s: &Styled, h: &mut History, sel, settled| {
            at += 0.1;
            h.observe(s, Some(sel), at, settled);
        };
        // Typing at the end, a letter a frame: one step. Each command follows a frame
        // where its selection was seen, as in the editor.
        seen(&s, &mut h, (7, 7), true);
        for (i, t) in ["one twos", "one twosx"].iter().enumerate() {
            s.sync(t, Some(8 + i));
            seen(&s, &mut h, (8 + i, 8 + i), true);
        }
        // Bold on a word, then more typing: three steps in all.
        seen(&s, &mut h, (0, 3), true);
        s.bold((0, 3));
        seen(&s, &mut h, (0, 3), true);
        seen(&s, &mut h, (9, 9), true);
        s.sync("one twosxy", Some(10));
        seen(&s, &mut h, (10, 10), true);
        assert_eq!(marks(&s), "BBB.......");
        // Each step back puts back the selection it was made with.
        assert_eq!(h.undo(&mut s), Some((9, 9)));
        assert_eq!((s.text(), marks(&s).as_str()), ("one twosx", "BBB......"));
        assert_eq!(h.undo(&mut s), Some((0, 3)));
        assert_eq!(marks(&s), ".........");
        assert_eq!(h.undo(&mut s), Some((7, 7)));
        assert_eq!(s.text(), "one two");
        assert_eq!(h.undo(&mut s), None);
        // Redo goes forward again, each step with the selection it was undone with.
        assert_eq!(h.redo(&mut s), Some((0, 3)));
        assert_eq!(h.redo(&mut s), Some((9, 9)));
        assert_eq!(marks(&s), "BBB......");
        // A change after undoing drops what could be redone.
        h.undo(&mut s);
        s.set_size((0, 3), 20.0);
        seen(&s, &mut h, (0, 3), true);
        assert_eq!(h.redo(&mut s), None);
        // Typing after a pause, or elsewhere, is a step of its own.
        let mut s = Styled::plain("ab", CharStyle::default());
        let mut h = History::new(&s);
        s.sync("abc", Some(3));
        h.observe(&s, Some((3, 3)), 1.0, true);
        s.sync("abcd", Some(4));
        h.observe(&s, Some((4, 4)), 3.0, true);
        h.observe(&s, Some((0, 0)), 3.05, true);
        s.sync("xabcd", Some(1));
        h.observe(&s, Some((1, 1)), 3.1, true);
        let mut texts = Vec::new();
        while h.undo(&mut s).is_some() {
            texts.push(s.text().to_string());
        }
        assert_eq!(texts, ["abcd", "abc", "ab"]);
        // Colours picked while the picker is open are one step.
        let mut s = Styled::plain("ab", CharStyle::default());
        let mut h = History::new(&s);
        for (i, c) in [0.2, 0.4, 0.6].iter().enumerate() {
            s.set_color((0, 2), [*c, 0.0, 0.0]);
            h.observe(&s, Some((0, 2)), i as f64 * 0.1, false);
        }
        h.observe(&s, Some((0, 2)), 0.5, true);
        s.set_color((0, 2), [1.0, 0.0, 0.0]);
        h.observe(&s, Some((0, 2)), 0.6, true);
        h.undo(&mut s);
        assert_eq!(s.styles()[0].color, [0.6, 0.0, 0.0]);
        h.undo(&mut s);
        assert_eq!(s.styles()[0].color, [0.0; 3]);
    }

    #[test]
    fn a_read_line_keeps_its_parts_fonts_until_changed() {
        let span = |chars, font: &str, size| TextSpan {
            chars,
            font_name: font.into(),
            font_size: size,
            fill: None,
            font_reusable: true,
            rise: 0.0,
        };
        let o = PageObject {
            id: 0,
            kind: limarepdf_core::ObjectKind::Text,
            bbox: [0.0; 4],
            text: Some("Big bold end".into()),
            font_name: Some("Helvetica".into()),
            font_size: Some(12.0),
            fill: None,
            stroke: None,
            line_width: None,
            has_fill: true,
            has_stroke: false,
            inline: false,
            font_reusable: true,
            hidden: false,
            baseline: None,
            dir: None,
            stops: Vec::new(),
            upright: true,
            vertical: false,
            spans: vec![
                span((0, 4), "Helvetica", 12.0),
                span((4, 8), "ABCDEF+Helvetica-Bold", 12.0),
                span((8, 12), "Helvetica", 12.0),
            ],
        };
        let mut s = Styled::of(&o, FontChoice::from_name);
        let fonts: Vec<PartFont> = s.parts().iter().map(|p| p.font).collect();
        assert_eq!(
            fonts,
            [PartFont::Span(0), PartFont::Span(1), PartFont::Span(2)]
        );
        assert!(s.same_as(&Styled::of(&o, FontChoice::from_name)));
        // Typed text goes into its part.
        s.sync("Big bolder end", Some(10));
        assert_eq!(s.parts()[1].text, "bolder");
        // Bold off and on again: the part's own font once more.
        s.bold((4, 10));
        assert!(matches!(s.parts()[0].font, PartFont::Span(0)));
        assert_eq!(s.parts()[0].text, "Big ");
        assert!(matches!(s.parts()[1].font, PartFont::Choice(_)));
        s.bold((4, 10));
        assert_eq!(s.parts()[1].font, PartFont::Span(1));
        assert!(!s.same_as(&Styled::of(&o, FontChoice::from_name)));
        assert!(s.shown((0, 14)).font.is_none());
        // Two subsets of one font are one font to the inspector.
        let mut o2 = o.clone();
        o2.spans[1].font_name = "ABCDEF+Helvetica".into();
        o2.spans[2].font_name = "GHIJKL+Helvetica".into();
        let s2 = Styled::of(&o2, FontChoice::from_name);
        assert!(s2.shown((0, 12)).font.is_some());
    }

    /// A line `text` as read, in one part in a subset of Helvetica, 12 points.
    fn read_line(text: &str) -> Styled {
        let o = PageObject {
            id: 0,
            kind: limarepdf_core::ObjectKind::Text,
            bbox: [0.0; 4],
            text: Some(text.into()),
            font_name: Some("ABCDEF+Helvetica".into()),
            font_size: Some(12.0),
            fill: None,
            stroke: None,
            line_width: None,
            has_fill: true,
            has_stroke: false,
            inline: false,
            font_reusable: false,
            hidden: false,
            baseline: None,
            dir: None,
            stops: Vec::new(),
            upright: true,
            vertical: false,
            spans: vec![TextSpan {
                chars: (0, text.chars().count()),
                font_name: "ABCDEF+Helvetica".into(),
                font_size: 12.0,
                fill: None,
                font_reusable: false,
                rise: 0.0,
            }],
        };
        Styled::of(&o, FontChoice::from_name)
    }

    /// Each part's text and where it says it was in the line as read.
    fn from(s: &Styled) -> Vec<(String, Option<usize>)> {
        s.parts().into_iter().map(|p| (p.text, p.from)).collect()
    }

    /// Words still as they were read say where they were, so their own shows can be kept:
    /// a restyled word too, while it keeps its font. A word typed into, or typed onto,
    /// doesn't, and is written again whole.
    #[test]
    fn untouched_words_say_where_they_were_read() {
        let at = |v: &[(&str, Option<usize>)]| -> Vec<(String, Option<usize>)> {
            v.iter().map(|(t, f)| (t.to_string(), *f)).collect()
        };
        let mut s = read_line("one two three");
        s.set_size((4, 7), 30.0);
        assert_eq!(
            from(&s),
            at(&[("one ", Some(0)), ("two", Some(4)), (" three", Some(7))])
        );
        let mut s = read_line("one two three");
        s.sync("one twXo three", Some(7));
        let typed_into = at(&[("one ", Some(0)), ("twXo", None), (" three", Some(7))]);
        assert_eq!(from(&s), typed_into);
        let mut s = read_line("one two three");
        s.sync("oneer two three", Some(5));
        assert_eq!(from(&s), at(&[("oneer", None), (" two three", Some(3))]));
        // In another font, a word is written anew whatever it was.
        let mut s = read_line("one two three");
        s.bold((4, 7));
        assert_eq!(from(&s)[1], ("two".to_string(), None));
        // New text was never read.
        assert_eq!(from(&with_bold("new", 0, 1))[0].1, None);
    }

    /// Words selected across lines take Bold by Word's rule as one selection, and show
    /// blank where the lines mix a value.
    #[test]
    fn words_across_lines_are_one_selection() {
        let mut across = Across {
            lines: vec![
                (with_bold("aa bb", 3, 5), vec![(3, 5)]),
                (with_bold("cc", 0, 0), vec![(0, 2)]),
            ],
        };
        // Whether every selected character is bold, or none is.
        let all = |a: &Across, bold: bool| {
            let line = |(t, r): &(Styled, Vec<(usize, usize)>)| {
                let selected = &t.styles()[r[0].0..r[0].1];
                selected.iter().all(|s| s.font.bold == bold)
            };
            a.lines.iter().all(line)
        };
        assert!(!across.shown().bold);
        across.bold();
        assert!(across.shown().bold, "partly bold becomes all bold");
        assert!(all(&across, true));
        across.bold();
        assert!(all(&across, false));
        // The first line's unselected "aa " is left as it was.
        assert!(!across.lines[0].0.styles()[0].font.bold);
        assert_eq!(across.shown().size, Some(12.0));
        across.lines[1].0.set_size((0, 2), 20.0);
        assert_eq!(across.shown().size, None);
        across.set_size(14.0);
        assert_eq!(across.shown().size, Some(14.0));
    }
}

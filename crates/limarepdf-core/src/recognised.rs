//! Taking recognised text ([`TextData::recognised`]) off a page: out of its content, and out
//! of the forms it draws at any depth, as when an app arranging pages puts a whole page in a
//! form. A form drawing nothing but the lines taken off isn't drawn any more. One drawing
//! more is drawn as a copy without them, by a name of its own, where it was drawn: the
//! original stays as it was for whatever else draws it, and saving (which prunes unused
//! objects) drops it when nothing does. A copy of a form drawing more forms draws copies of
//! those it needs.
//!
//! The fonts and forms only the lines taken off used leave the resources that held them, the
//! page's and each copy's, so none of the words stay in the file through them. A form
//! without resources of its own draws with those of where it is drawn: what it, or a Type 3
//! font, pattern or soft mask drawing with them, still uses stays there, and nothing leaves
//! them when one of those can't be read.

use crate::budget::{self, Work};
use crate::edit::{encoded, form_copy, numbered};
use crate::matrix::M;
use crate::objects::{Parsed, TextData, STEPS};
use crate::reach::{Decoded, Uses};
use crate::{ObjectKind, Pdf, Result};
use anyhow::{anyhow, bail};
use lopdf::content::Operation;
use lopdf::{Dictionary, Object, ObjectId, Stream};
use std::collections::{HashMap, HashSet};

/// Why recognised text isn't taken out of content with an inline picture.
pub(crate) const INLINE_IMAGE: &str =
    "the page has an inline image that would be lost in rewriting it";

/// What taking recognised text off a page is called when its budget refuses it.
const WHAT: &str = "taking the recognised text off the page";

/// The names a form's copies are given: `LimareOcr1`, `LimareOcr2`…
const COPY: &str = "LimareOcr";

/// A page without the recognised lines chosen, as [`Pdf::stripped`] plans it.
pub(crate) struct Stripped {
    /// How many lines it takes off.
    pub lines: usize,
    /// The page's content, written.
    content: Vec<u8>,
    /// The page's resources, when they change.
    res: Option<Dictionary>,
    /// The copies of forms, with their ids.
    added: Vec<(ObjectId, Object)>,
    /// The first id free when it was planned, the copies' first.
    first: u32,
}

/// What the content drawn with one resource dictionary stops using of it, and what it still
/// uses.
#[derive(Default)]
struct Pruned {
    fonts: HashSet<Vec<u8>>,
    xobjects: HashSet<Vec<u8>>,
    kept: Uses,
}

struct Stripper<'a> {
    pdf: &'a Pdf,
    choose: &'a dyn Fn(&TextData) -> bool,
    work: Work,
    /// What the drawings read for the names they use use ([`Pdf::borrowed_uses`]).
    uses: Decoded,
    /// The forms the content being rewritten is inside of, as it is read.
    forms: Vec<Option<ObjectId>>,
    next_id: u32,
    added: Vec<(ObjectId, Object)>,
    /// The copies made of forms with resources of their own, by form and which of its lines
    /// are taken off: one drawn twice the same way is copied once.
    made: HashMap<(ObjectId, Vec<bool>), ObjectId>,
}

impl Stripper<'_> {
    /// Whether line `t` is taken off.
    fn chosen(&self, t: &TextData) -> bool {
        t.recognised() && (self.choose)(t)
    }

    /// Content `parsed`, drawn with `res`, without its chosen lines: what it draws instead,
    /// and whether that is nothing. The forms it draws holding chosen lines go, when they
    /// draw nothing else, or are drawn as copies without them, named in `res`. What it stops
    /// using of `res`, and still uses, goes in `pruned`.
    fn content(
        &mut self,
        parsed: &mut Parsed,
        res: &mut Dictionary,
        pruned: &mut Pruned,
    ) -> Result<(Vec<Operation>, bool)> {
        if parsed.ops.iter().any(|o| o.operator == "BI") {
            bail!(INLINE_IMAGE);
        }
        // What it uses before, less what it still uses after.
        let mut unused = Uses::of_ops(&parsed.ops);
        let mut gone: Vec<usize> = (parsed.objs.iter().enumerate())
            .filter(|(_, o)| o.text.as_ref().is_some_and(|t| self.chosen(t)))
            .map(|(id, _)| id)
            .collect();
        let mut drawn = Vec::new();
        for f in &parsed.forms_read {
            let chosen: Vec<bool> = (parsed.form_text[f.lines.clone()].iter())
                .map(|t| self.chosen(t))
                .collect();
            if chosen.contains(&true) {
                let obj = (parsed.objs.iter())
                    .position(|o| o.kind == ObjectKind::Form && o.first == f.op);
                drawn.push((f.op, f.ctm, chosen, obj));
            }
        }
        for (op, ctm, chosen, obj) in drawn {
            let name = name_operand(&parsed.ops[op]).ok_or_else(|| anyhow!("a form unnamed"))?;
            match self.form(&name, ctm, chosen, obj.is_some(), res, pruned)? {
                Some(copy) => parsed.ops[op].operands[0] = Object::Name(copy),
                None => gone.extend(obj),
            }
        }
        let mut ops = Pdf::without_objects(parsed, &gone)?;
        // The fonts only the lines taken off are in, and what chooses them.
        let gone: HashSet<usize> = gone.into_iter().collect();
        let (off, kept): (Vec<_>, Vec<_>) = (parsed.objs.iter().enumerate())
            .filter_map(|(id, o)| Some((id, o.text.as_ref()?)))
            .partition(|(id, _)| gone.contains(id));
        let spans = |lines: Vec<(usize, &TextData)>| -> Vec<Vec<u8>> {
            (lines.into_iter().flat_map(|(_, t)| &t.spans))
                .map(|s| s.font.clone())
                .collect()
        };
        let mut fonts: HashSet<Vec<u8>> = spans(off).into_iter().collect();
        for f in spans(kept) {
            fonts.remove(&f);
        }
        ops.retain(|o| o.operator != "Tf" || !name_operand(o).is_some_and(|n| fonts.contains(&n)));
        let after = Uses::of_ops(&ops);
        unused.fonts.retain(|n| !after.fonts.contains(n));
        unused.xobjects.retain(|n| !after.xobjects.contains(n));
        pruned.fonts.extend(unused.fonts);
        pruned.xobjects.extend(unused.xobjects);
        add(&mut pruned.kept, after);
        let shades = parsed.ops.iter().any(|o| o.operator == "sh");
        Ok((ops, gone.len() == parsed.objs.len() && !shades))
    }

    /// The form named `name` in `res`, drawn with transform `ctm`, without the lines of it
    /// `chosen` picks (its own, then those of the forms it draws, as they are read): the name
    /// in `res` of a copy of it drawing the rest, or `None` when that is nothing and it
    /// `can_go`. What a form without resources of its own stops using of `res`, and still
    /// uses, goes in `pruned`.
    fn form(
        &mut self,
        name: &[u8],
        ctm: M,
        chosen: Vec<bool>,
        can_go: bool,
        res: &mut Dictionary,
        pruned: &mut Pruned,
    ) -> Result<Option<Vec<u8>>> {
        let pdf = self.pdf;
        let unread = || anyhow!("a form the page draws can't be read again");
        let xobjects = pdf.resolved_dict(res.get(b"XObject").ok());
        let (id, s) = (xobjects.get(name).ok())
            .and_then(|o| pdf.doc.dereference(o).ok())
            .and_then(|(id, o)| Some((id, o.as_stream().ok()?)))
            .ok_or_else(unread)?;
        let own = (s.dict.get(b"Resources").ok()).map(|o| pdf.resolved_dict(Some(o)));
        let key = id.filter(|_| own.is_some()).map(|id| (id, chosen));
        if let Some(&copy) = key.as_ref().and_then(|k| self.made.get(k)) {
            return Ok(Some(self.named(res, copy)));
        }
        self.work.take(1)?;
        let data = (s.get_plain_content_with_limit(crate::MAX_CONTENT)).map_err(|_| unread())?;
        self.work.take(data.len() as u64 / 1024)?;
        let with = Object::Dictionary(own.clone().unwrap_or_else(|| res.clone()));
        let (work, forms) = (&mut self.work, &mut self.forms);
        forms.push(id);
        let mut parsed = pdf.parse_content(&data, Some(&with), ctm, work, forms)?;
        // A copy's resources are its form's, unless they change.
        let (ops, empty, own) = match own {
            Some(before) => {
                let (mut own, mut inner) = (before.clone(), Pruned::default());
                let (ops, empty) = self.content(&mut parsed, &mut own, &mut inner)?;
                self.prune(&mut own, inner);
                (ops, empty, (own != before).then_some(own))
            }
            None => {
                let (ops, empty) = self.content(&mut parsed, res, pruned)?;
                (ops, empty, None)
            }
        };
        self.forms.pop();
        if empty && can_go {
            return Ok(None);
        }
        let bytes = encoded(ops, parsed.depth_left, &parsed.inline)?;
        let copy = (self.next_id, 0);
        self.next_id += 1;
        let stream = form_copy(s, own, bytes);
        self.added.push((copy, Object::Stream(stream)));
        if let Some(k) = key {
            self.made.insert(k, copy);
        }
        Ok(Some(self.named(res, copy)))
    }

    /// Takes out of `res` what `pruned` says the content drawn with it stops using, but what
    /// the drawings drawn with it still use; nothing when one of those can't be read.
    fn prune(&mut self, res: &mut Dictionary, pruned: Pruned) {
        let Some(more) = self.pdf.borrowed_uses(res, &pruned.kept, &mut self.uses) else {
            return;
        };
        let kept = &pruned.kept;
        let cats = [
            ("Font", pruned.fonts, &kept.fonts, &more.fonts),
            ("XObject", pruned.xobjects, &kept.xobjects, &more.xobjects),
        ];
        for (cat, gone, used, more) in cats {
            let unused: Vec<Vec<u8>> = (gone.into_iter())
                .filter(|n| !used.contains(n) && !more.contains(n))
                .collect();
            if unused.is_empty() {
                continue;
            }
            let mut d = self.pdf.resolved_dict(res.get(cat.as_bytes()).ok());
            for n in &unused {
                d.remove(n);
            }
            res.set(cat, d);
        }
    }

    /// The name of drawing `id` among `res`'s forms, a new one given it there if it has none.
    fn named(&self, res: &mut Dictionary, id: ObjectId) -> Vec<u8> {
        let mut xobjects = self.pdf.resolved_dict(res.get(b"XObject").ok());
        let had = (xobjects.iter()).find(|(_, o)| o.as_reference().ok() == Some(id));
        if let Some((n, _)) = had {
            return n.clone();
        }
        let name = (numbered(COPY).map(String::into_bytes))
            .find(|n| !xobjects.has(n))
            .unwrap_or_default();
        xobjects.set(name.clone(), id);
        res.set("XObject", xobjects);
        name
    }
}

/// `more` added to `uses`.
fn add(uses: &mut Uses, more: Uses) {
    for (to, from) in [
        (&mut uses.xobjects, more.xobjects),
        (&mut uses.properties, more.properties),
        (&mut uses.fonts, more.fonts),
        (&mut uses.patterns, more.patterns),
        (&mut uses.states, more.states),
    ] {
        to.extend(from);
    }
}

/// The name an operator names first: a font for `Tf`, a picture or a form for `Do`.
fn name_operand(o: &Operation) -> Option<Vec<u8>> {
    Some(o.operands.first()?.as_name().ok()?.to_vec())
}

impl Pdf {
    /// The page with object `page` without the recognised lines `choose` picks, in its
    /// content and in the forms it draws, planned: nothing changes until it is put
    /// ([`Pdf::put_stripped`]). `None` when it has none of them; why not when they can't be
    /// taken off.
    pub(crate) fn stripped(
        &self,
        page: ObjectId,
        choose: &dyn Fn(&TextData) -> bool,
    ) -> std::result::Result<Option<Stripped>, String> {
        let mut parsed =
            (self.parse_page_of(page)).map_err(|e| format!("cannot read the page: {e}"))?;
        let lines = (parsed.text_lines())
            .filter(|t| t.recognised() && choose(t))
            .count();
        if lines == 0 {
            return Ok(None);
        }
        let first = self.doc.max_id + 1;
        let mut s = Stripper {
            pdf: self,
            choose,
            work: Work::new(WHAT, budget::limit(STEPS)),
            uses: Decoded::default(),
            forms: Vec::new(),
            next_id: first,
            added: Vec::new(),
            made: HashMap::new(),
        };
        let before = self.resolved_dict(self.inherited(page, b"Resources").as_ref());
        let mut res = before.clone();
        let content = (|| {
            let mut pruned = Pruned::default();
            let (ops, _) = s.content(&mut parsed, &mut res, &mut pruned)?;
            s.prune(&mut res, pruned);
            encoded(ops, parsed.depth_left, &parsed.inline)
        })();
        Ok(Some(Stripped {
            lines,
            content: content.map_err(|e| format!("{e:#}"))?,
            res: (res != before).then_some(res),
            added: s.added,
            first,
        }))
    }

    /// Put `s`, planned for the page with object `page` ([`Pdf::stripped`]) with the
    /// document as it is.
    pub(crate) fn put_stripped(&mut self, page: ObjectId, s: Stripped) -> Result<()> {
        if s.first != self.doc.max_id + 1 {
            bail!("the document changed after the page's recognised text was read");
        }
        for (id, o) in s.added {
            self.doc.objects.insert(id, o);
            self.doc.max_id = self.doc.max_id.max(id.0);
        }
        let content = Stream::new(Dictionary::new(), s.content);
        let content = self.doc.add_object(content);
        let d = self.doc.get_dictionary_mut(page)?;
        d.set("Contents", content);
        if let Some(res) = s.res {
            d.set("Resources", res);
        }
        Ok(())
    }
}

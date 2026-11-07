//! Removing hidden information before a document is shared: what isn't on any page but
//! travels with the file (metadata, scripts, attachments, the content of hidden layers,
//! private application data), and optionally comments, form values, bookmarks and what
//! lies outside the visible page area.

use crate::budget::{self, TooComplex, Work};
use crate::fonts::FontCtx;
use crate::inline::Inline;
use crate::reach;
use crate::{Pdf, Result};
use anyhow::anyhow;
use lopdf::content::{Content, Operation};
use lopdf::{Dictionary, Document, Object, ObjectId};
use std::collections::{HashMap, HashSet};

/// A kind of hidden information.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Hidden {
    Metadata,
    Scripts,
    Attachments,
    HiddenLayers,
    PrivateData,
    Comments,
    FormValues,
    Bookmarks,
    OutsideCrop,
}

impl Hidden {
    pub const ALL: [Hidden; 9] = [
        Hidden::Metadata,
        Hidden::Scripts,
        Hidden::Attachments,
        Hidden::HiddenLayers,
        Hidden::PrivateData,
        Hidden::Comments,
        Hidden::FormValues,
        Hidden::Bookmarks,
        Hidden::OutsideCrop,
    ];

    /// The name the command line takes.
    pub fn name(self) -> &'static str {
        match self {
            Hidden::Metadata => "metadata",
            Hidden::Scripts => "scripts",
            Hidden::Attachments => "attachments",
            Hidden::HiddenLayers => "hidden-layers",
            Hidden::PrivateData => "private-data",
            Hidden::Comments => "comments",
            Hidden::FormValues => "form-values",
            Hidden::Bookmarks => "bookmarks",
            Hidden::OutsideCrop => "outside-crop",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|h| h.name() == s.trim())
    }

    /// A label for people.
    pub fn label(self) -> &'static str {
        match self {
            Hidden::Metadata => "Document information and XMP metadata",
            Hidden::Scripts => "JavaScript and actions that run by themselves",
            Hidden::Attachments => "Attached files",
            Hidden::HiddenLayers => "Hidden layers and their content",
            Hidden::PrivateData => "Thumbnails and private application data",
            Hidden::Comments => "Comments and markup",
            Hidden::FormValues => "Form field values",
            Hidden::Bookmarks => "Bookmarks",
            Hidden::OutsideCrop => "Content outside the visible page area",
        }
    }

    /// Whether it is removed unless asked not to. Comments, form values and bookmarks are
    /// often wanted; cutting outside the crop box changes the pages.
    pub fn default_on(self) -> bool {
        matches!(
            self,
            Hidden::Metadata
                | Hidden::Scripts
                | Hidden::Attachments
                | Hidden::HiddenLayers
                | Hidden::PrivateData
        )
    }
}

/// How many things of each kind [`Pdf::remove_hidden`] removed (kinds with none are left
/// out), in [`Hidden::ALL`] order.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct HiddenReport {
    pub removed: Vec<(Hidden, usize)>,
    /// Compressed data looked into for hidden layers was complete but had a wrong checksum;
    /// it was read in full, as other readers do.
    pub faults_overlooked: bool,
    /// The pages (zero-based, in order) where cutting what lies outside the crop box left a
    /// drawing that still draws an original picture or form it cut, as
    /// [`crate::RedactReport::pictures_elsewhere`].
    pub pictures_elsewhere: Vec<usize>,
}

impl HiddenReport {
    pub fn count(&self, h: Hidden) -> usize {
        self.removed
            .iter()
            .find(|(k, _)| *k == h)
            .map_or(0, |(_, n)| *n)
    }

    /// What to tell people about the removal besides the counts, a sentence each.
    pub fn notes(&self) -> Vec<String> {
        let mut out = Vec::new();
        if self.faults_overlooked {
            out.push(
                "Some of this document's data had a small fault in it (other PDF readers \
                 overlook it too). It was all read, and nothing was skipped."
                    .to_string(),
            );
        }
        let pictures = crate::RedactReport {
            pictures_elsewhere: self.pictures_elsewhere.clone(),
            ..Default::default()
        };
        out.extend(pictures.picture_notices());
        out
    }
}

/// [`Pdf::remove_hidden`]'s refusal when compressed data that can hold hidden layers is cut
/// off or damaged part way, so not all of it could be checked. The message is the app's.
#[derive(Debug, thiserror::Error)]
#[error(
    "Part of this document is damaged, so LimarePDF can't check all of it for hidden layers. \
     Nothing was changed. To remove the other kinds of information, untick Hidden layers and \
     try again."
)]
pub struct LayersDamaged;

/// Actions that run something, send data or open other files by themselves.
const ACTIVE: [&[u8]; 6] = [
    b"JavaScript",
    b"Launch",
    b"SubmitForm",
    b"ImportData",
    b"Rendition",
    b"GoToE",
];

/// Deepest nesting of drawings (forms, patterns, glyphs, soft masks) looked into for hidden
/// layers, and of the expressions in a layer rule. Deeper ones make the removal refuse
/// rather than leave them.
const MAX_NESTING: usize = 32;

/// Most steps working out a document's layer rules may take: each operator and each operand
/// of an expression is one, and each layer a rule without an expression lists.
const RULE_STEPS: u64 = 1 << 20;

/// A layer rule (an optional content membership dictionary, `/Type /OCMD`): one stored as
/// an object of its own, or one written in place, by what it is.
#[derive(Clone, PartialEq, Eq, Hash)]
enum RuleKey {
    Stored(ObjectId),
    Direct(String),
}

/// How a layer rule reads with the layers that are off: whether it hides its content, and
/// whether it names a layer that's off; or why it can't be read, to follow "can't be read".
type Reading = std::result::Result<(bool, bool), String>;

/// Why a layer rule couldn't be worked out: a fault in it, or the steps ran out.
enum Fault {
    Bad(String),
    Too(TooComplex),
}

fn bad(why: &str) -> Fault {
    Fault::Bad(why.to_string())
}

/// An expression of a layer rule with the layers that are off taken as off for good.
#[derive(Clone)]
enum Folded {
    /// The same whatever the layers that stay are.
    Const(bool),
    /// What to write in its place: it still depends on layers that stay.
    Live(Object),
}

/// The layers switched off in the default configuration, and how the layer rules read with
/// them (ISO 32000-1, 8.11.2.2: `/VE` first, else `/OCGs` and `/P`). Every other layer
/// counts as on. Each rule and each stored expression is worked out once, within one budget
/// of [`RULE_STEPS`]; what the scan found the rules something uses do is kept here too.
struct Layers {
    off: HashSet<ObjectId>,
    /// Stored expressions worked out: whether each is true, how many levels deep it
    /// reaches, and whether it names a layer that's off.
    exprs: HashMap<ObjectId, (bool, usize, bool)>,
    rules: HashMap<RuleKey, Reading>,
    /// The stored expressions being worked out, to find one that contains itself.
    open: HashSet<ObjectId>,
    work: Work,
    /// The steps ran out.
    too: Option<TooComplex>,
    /// Some rule something uses hides its content.
    hides: bool,
    /// Some rule something uses can't be read.
    unread: bool,
    /// The rules counted as found: those hiding their content though every layer they name
    /// is on, and those that can't be read.
    counted: HashSet<RuleKey>,
    /// Stored expressions with the layers that are off taken out, and what each becomes.
    folds: HashMap<ObjectId, Folded>,
    exprs_new: HashMap<ObjectId, Vec<Object>>,
    /// Rules kept that name a layer that's off, and what each becomes.
    rewrites: HashMap<RuleKey, Dictionary>,
}

/// The rule `oc` is, if it is one: by reference, or written in place.
fn rule_of<'a>(doc: &'a Document, oc: &'a Object) -> Option<(RuleKey, &'a Dictionary)> {
    let (id, d) = match oc {
        Object::Reference(id) => (Some(*id), doc.get_dictionary(*id).ok()?),
        Object::Dictionary(d) => (None, d),
        _ => return None,
    };
    if d.get(b"Type").and_then(Object::as_name).ok() != Some(b"OCMD") {
        return None;
    }
    let key = id.map_or_else(|| RuleKey::Direct(format!("{d:?}")), RuleKey::Stored);
    Some((key, d))
}

/// The layers a rule's `/OCGs` lists: one, or an array of them (others are left out).
fn rule_groups(d: &Dictionary) -> Vec<ObjectId> {
    match d.get(b"OCGs") {
        Ok(Object::Reference(id)) => vec![*id],
        Ok(Object::Array(a)) => a.iter().filter_map(|o| o.as_reference().ok()).collect(),
        _ => Vec::new(),
    }
}

impl Layers {
    fn new(off: HashSet<ObjectId>) -> Layers {
        Layers {
            off,
            exprs: HashMap::new(),
            rules: HashMap::new(),
            open: HashSet::new(),
            work: Work::new("checking the layers", budget::limit(RULE_STEPS)),
            too: None,
            hides: false,
            unread: false,
            counted: HashSet::new(),
            folds: HashMap::new(),
            exprs_new: HashMap::new(),
            rewrites: HashMap::new(),
        }
    }

    /// Whether there is anything to remove: a layer switched off, or content a rule hides.
    fn to_remove(&self) -> bool {
        !self.off.is_empty() || self.hides
    }

    /// How many are found: the layers switched off and the rules counted.
    fn found(&self) -> usize {
        self.off.len() + self.counted.len()
    }

    /// Whether the removal refuses, by what `scan` found: some rule can't be read, or there
    /// is something to remove and some drawing couldn't be looked into.
    fn refuses(&self, scan: &LayerScan) -> bool {
        self.unread || (self.to_remove() && !scan.blocked.is_empty())
    }

    fn step(&mut self) -> std::result::Result<(), Fault> {
        self.work.take(1).map_err(Fault::Too)
    }

    /// How rule `d` (`key`) reads. When the steps run out, that is kept for the scan to
    /// refuse, and the rule reads as showing its content.
    fn read(&mut self, doc: &Document, key: &RuleKey, d: &Dictionary) -> Reading {
        if let Some(r) = self.rules.get(key) {
            return r.clone();
        }
        let r = match self.work_out(doc, d) {
            Ok(r) => Ok(r),
            Err(Fault::Bad(why)) => Err(why),
            Err(Fault::Too(t)) => {
                self.too.get_or_insert(t);
                return Ok((false, false));
            }
        };
        self.rules.insert(key.clone(), r.clone());
        r
    }

    fn work_out(
        &mut self,
        doc: &Document,
        d: &Dictionary,
    ) -> std::result::Result<(bool, bool), Fault> {
        let groups = rule_groups(d);
        let names_off = groups.iter().any(|g| self.off.contains(g));
        if let Ok(ve) = d.get(b"VE") {
            let (shown, _, off) = match ve {
                Object::Array(a) => self.expr(doc, a, 1)?,
                Object::Reference(id)
                    if doc.get_object(*id).is_ok_and(|o| o.as_array().is_ok()) =>
                {
                    self.stored(doc, *id, 1)?
                }
                _ => return Err(bad("its /VE isn't an expression")),
            };
            return Ok((!shown, off || names_off));
        }
        for _ in &groups {
            self.step()?;
        }
        if groups.is_empty() {
            return Ok((false, false));
        }
        let on: Vec<bool> = groups.iter().map(|g| !self.off.contains(g)).collect();
        let shown = match d.get(b"P").and_then(Object::as_name).unwrap_or(b"AnyOn") {
            b"AllOn" => on.iter().all(|o| *o),
            b"AnyOff" => on.iter().any(|o| !o),
            b"AllOff" => on.iter().all(|o| !o),
            _ => on.iter().any(|o| *o),
        };
        Ok((!shown, names_off))
    }

    /// Expression `a`, at `level` (the top of a rule's `/VE` is 1): whether it is true, how
    /// many levels deep it reaches from itself, and whether it names a layer that's off.
    fn expr(
        &mut self,
        doc: &Document,
        a: &[Object],
        level: usize,
    ) -> std::result::Result<(bool, usize, bool), Fault> {
        if level > MAX_NESTING {
            return Err(bad("it is nested more than 32 deep"));
        }
        self.step()?;
        let Some(Object::Name(op)) = a.first() else {
            return Err(bad(
                "an expression in it doesn't start with /And, /Or or /Not",
            ));
        };
        let args = &a[1..];
        match op.as_slice() {
            b"Not" if args.len() != 1 => {
                return Err(bad("a /Not in it doesn't have exactly one operand"))
            }
            b"And" | b"Or" if args.is_empty() => {
                return Err(bad("an /And or /Or in it has nothing to combine"))
            }
            b"Not" | b"And" | b"Or" => {}
            _ => {
                return Err(Fault::Bad(format!(
                    "it combines layers with /{}, which isn't a way to combine them",
                    String::from_utf8_lossy(op)
                )))
            }
        }
        let neither = "something in it is neither a layer nor an expression";
        let (mut all, mut any, mut height, mut names_off) = (true, false, 1, false);
        for x in args {
            self.step()?;
            let (v, h, off) = match x {
                Object::Reference(id) => match doc.get_object(*id) {
                    Ok(Object::Dictionary(_)) => {
                        let off = self.off.contains(id);
                        (!off, 0, off)
                    }
                    Ok(Object::Array(_)) => self.stored(doc, *id, level + 1)?,
                    _ => return Err(bad(neither)),
                },
                Object::Array(inner) => self.expr(doc, inner, level + 1)?,
                _ => return Err(bad(neither)),
            };
            all &= v;
            any |= v;
            height = height.max(h + 1);
            names_off |= off;
        }
        let v = match op.as_slice() {
            b"Not" => !all,
            b"And" => all,
            _ => any,
        };
        Ok((v, height, names_off))
    }

    /// The expression stored as object `id`, at `level`: worked out once, and checked for
    /// how deep it reaches at each place it is used. A fault isn't kept, as how deep it
    /// reaches depends on where it was first used.
    fn stored(
        &mut self,
        doc: &Document,
        id: ObjectId,
        level: usize,
    ) -> std::result::Result<(bool, usize, bool), Fault> {
        if let Some(&(v, h, off)) = self.exprs.get(&id) {
            if level - 1 + h > MAX_NESTING {
                return Err(bad("it is nested more than 32 deep"));
            }
            return Ok((v, h, off));
        }
        if !self.open.insert(id) {
            return Err(bad("it contains itself"));
        }
        let got = match doc.get_object(id) {
            Ok(Object::Array(a)) => self.expr(doc, a, level),
            _ => Err(bad("something in it is neither a layer nor an expression")),
        };
        self.open.remove(&id);
        let got = got?;
        self.exprs.insert(id, got);
        Ok(got)
    }

    /// Note how rule `d` (`key`), which something uses, reads: content it hides, counting
    /// it when every layer it names is on; that it can't be read, counting it too; and when
    /// it is kept but names a layer that's off, what it becomes. Returns how it reads.
    fn used(&mut self, doc: &Document, key: &RuleKey, d: &Dictionary) -> Reading {
        let r = self.read(doc, key, d);
        match r {
            Ok((true, names_off)) => {
                self.hides = true;
                if !names_off {
                    self.counted.insert(key.clone());
                }
            }
            Ok((false, true)) if !self.rewrites.contains_key(key) => match self.rewritten(doc, d) {
                Ok(new) => {
                    self.rewrites.insert(key.clone(), new);
                }
                Err(Fault::Too(t)) => {
                    self.too.get_or_insert(t);
                }
                Err(Fault::Bad(_)) => {}
            },
            Ok(_) => {}
            Err(_) => {
                self.unread = true;
                self.counted.insert(key.clone());
            }
        }
        r
    }

    /// Rule `d`, kept and naming a layer that's off, with the layers that are off taken as
    /// off for good, so the layers that stay switch its content as before and no layer
    /// that's off is named in it. One shown for good has no effect: `<< /Type /OCMD >>`.
    fn rewritten(
        &mut self,
        doc: &Document,
        d: &Dictionary,
    ) -> std::result::Result<Dictionary, Fault> {
        let mut out = Dictionary::new();
        out.set("Type", "OCMD");
        if let Ok(ve) = d.get(b"VE") {
            let folded = match ve {
                Object::Reference(id) => self.fold_stored(doc, *id)?,
                Object::Array(a) => self.fold(doc, a)?,
                _ => return Err(bad("its /VE isn't an expression")),
            };
            match folded {
                Folded::Const(_) => {}
                Folded::Live(Object::Reference(id)) if doc.get_dictionary(id).is_ok() => {
                    out.set("OCGs", Object::Reference(id));
                }
                Folded::Live(x) => out.set("VE", x),
            }
            return Ok(out);
        }
        let kept: Vec<Object> = (rule_groups(d).into_iter())
            .filter(|g| !self.off.contains(g))
            .map(Object::Reference)
            .collect();
        match d.get(b"P").and_then(Object::as_name).unwrap_or(b"AnyOn") {
            // A layer in it is off for good, so its content shows for good.
            b"AnyOff" => {}
            b"AllOff" if kept.is_empty() => {}
            _ => {
                out = d.clone();
                out.set("OCGs", kept);
            }
        }
        Ok(out)
    }

    /// Expression `a` folded: each layer that's off is the constant "off", and constants
    /// fold. What isn't changed stays as it was.
    fn fold(&mut self, doc: &Document, a: &[Object]) -> std::result::Result<Folded, Fault> {
        self.step()?;
        let Some(Object::Name(op)) = a.first() else {
            return Err(bad(
                "an expression in it doesn't start with /And, /Or or /Not",
            ));
        };
        if op == b"Not" {
            self.step()?;
            let x = a
                .get(1)
                .ok_or_else(|| bad("a /Not in it doesn't have exactly one operand"))?;
            return Ok(match self.fold_operand(doc, x)? {
                Folded::Const(v) => Folded::Const(!v),
                Folded::Live(x) => Folded::Live(Object::Array(vec![a[0].clone(), x])),
            });
        }
        // What leaves an `/And` as it is (true), or an `/Or` (false).
        let same = op == b"And";
        let mut kept = vec![a[0].clone()];
        let mut dropped = false;
        for x in &a[1..] {
            self.step()?;
            match self.fold_operand(doc, x)? {
                Folded::Const(v) if v == same => dropped = true,
                Folded::Const(v) => return Ok(Folded::Const(v)),
                Folded::Live(x) => kept.push(x),
            }
        }
        Ok(match kept.len() {
            1 => Folded::Const(same),
            2 if dropped => Folded::Live(kept.remove(1)),
            _ => Folded::Live(Object::Array(kept)),
        })
    }

    fn fold_operand(&mut self, doc: &Document, x: &Object) -> std::result::Result<Folded, Fault> {
        match x {
            Object::Reference(id) => match doc.get_object(*id) {
                Ok(Object::Array(_)) => self.fold_stored(doc, *id),
                _ if self.off.contains(id) => Ok(Folded::Const(false)),
                _ => Ok(Folded::Live(x.clone())),
            },
            Object::Array(a) => self.fold(doc, a),
            _ => Ok(Folded::Live(x.clone())),
        }
    }

    /// The expression stored as object `id` folded, once: one that changes, but still
    /// depends on layers that stay, is rewritten where it is (so every rule using it sees
    /// the same, and nothing is copied out of it), and is used as it was, by reference.
    fn fold_stored(&mut self, doc: &Document, id: ObjectId) -> std::result::Result<Folded, Fault> {
        let live = Folded::Live(Object::Reference(id));
        if let Some(f) = self.folds.get(&id) {
            return Ok(f.clone());
        }
        let Ok(Object::Array(a)) = doc.get_object(id) else {
            return Ok(live);
        };
        if self.exprs.get(&id).is_some_and(|e| !e.2) {
            return Ok(live);
        }
        let f = match self.fold(doc, a)? {
            Folded::Const(v) => Folded::Const(v),
            Folded::Live(Object::Array(new)) => {
                if new != *a {
                    self.exprs_new.insert(id, new);
                }
                live
            }
            Folded::Live(x) => {
                self.exprs_new
                    .insert(id, vec![Object::Name(b"And".to_vec()), x]);
                live
            }
        };
        self.folds.insert(id, f.clone());
        Ok(f)
    }
}

/// A rule written into a drawing's own content that can't be read: why, to follow the name
/// of the drawing.
#[derive(Debug, thiserror::Error)]
#[error("has a layer rule that can't be read ({0})")]
struct InlineRule(String);

/// A drawing hidden layers are taken out of.
enum Drawn {
    Page(ObjectId),
    Stream(ObjectId),
}

/// The drawings that held content of hidden layers, without it, and what couldn't be looked
/// into.
#[derive(Default)]
struct LayerScan {
    edits: Vec<(Drawn, Vec<Operation>, Inline)>,
    blocked: Vec<String>,
    /// Some of what is in `blocked` is damaged data.
    damaged: bool,
    /// Some data read had a wrong checksum.
    faults: bool,
}

/// A drawing the layer scan has looked into. One without resources of its own draws with
/// those of where it is used, so its layer names can mean other layers there: for such a
/// drawing, the resources it was first looked into with, and the names each set of resources
/// it has been checked against gives hidden layers.
#[derive(Default)]
struct Seen {
    first: Option<Dictionary>,
    checked: Vec<HiddenNames>,
}

/// The names resources give content in hidden layers: in `/Properties`, and in `/XObject`
/// (pictures and forms in a hidden layer), each sorted; and their fonts (the width of hidden
/// text depends on them), each written out. A drawing loses the same content with any
/// resources that give the same.
type HiddenNames = (Vec<Vec<u8>>, Vec<Vec<u8>>, Vec<(Vec<u8>, String)>);

/// Hidden content that can't be taken out without changing what shows after it: why, to
/// follow the name of the drawing it is in.
#[derive(Debug, thiserror::Error)]
#[error("{0}")]
struct Unsure(&'static str);

impl Unsure {
    const WIDTH: Unsure = Unsure(
        "has hidden text whose width LimarePDF can't work out, and visible text on the same \
         line comes after it",
    );
    const CLIP: Unsure = Unsure("has hidden text that clips what is drawn after it");
    const SHAPE: Unsure = Unsure("has a hidden shape that is finished outside its layer");
}

/// The text state [`Pdf::without_hidden`] follows, to keep what comes after hidden text
/// where it was. A value is `None` until the drawing sets it, when it is that of where the
/// drawing is used.
#[derive(Clone)]
struct TextState {
    /// The font's resource name, and its size.
    font: Option<(Vec<u8>, f32)>,
    /// Character and word spacing.
    tc: Option<f32>,
    tw: Option<f32>,
    /// The rendering mode: 4 to 7 add the text to the clip. Taken as 0 until set (a mode
    /// that clips, set outside a form for text inside it, isn't looked for).
    render: i64,
}

impl TextState {
    /// The state a page's content (`page`) or another drawing starts with.
    fn new(page: bool) -> Self {
        let zero = page.then_some(0.0);
        TextState {
            font: None,
            tc: zero,
            tw: zero,
            render: 0,
        }
    }

    /// Follow operator `o`; `saved` is the stack `q` and `Q` keep the state on.
    fn follow(&mut self, o: &Operation, saved: &mut Vec<TextState>) {
        let num = |i: usize| o.operands.get(i).and_then(|x| x.as_float().ok());
        match o.operator.as_str() {
            "q" => saved.push(self.clone()),
            "Q" => {
                if let Some(s) = saved.pop() {
                    *self = s;
                }
            }
            "Tf" => {
                if let (Some(Object::Name(n)), Some(size)) = (o.operands.first(), num(1)) {
                    self.font = Some((n.clone(), size));
                }
            }
            "Tc" => self.tc = num(0).or(self.tc),
            "Tw" => self.tw = num(0).or(self.tw),
            "Tr" => self.render = num(0).map_or(self.render, |r| r as i64),
            "\"" => {
                self.tw = num(0).or(self.tw);
                self.tc = num(1).or(self.tc);
            }
            _ => {}
        }
    }
}

/// What hidden text-showing operator `o`, drawn in text state `t` with `font`, leaves: its
/// move to the next line (`'` and `"`, with the spacing `"` sets), then spacing as wide as
/// its text, as a `TJ` without text. And whether the width was known: it isn't without the
/// font, its size or spacing, or when LimarePDF can't split its strings into characters
/// for certain.
fn hidden_show(o: &Operation, t: &TextState, font: Option<&FontCtx>) -> (Vec<Operation>, bool) {
    let mut out = Vec::new();
    let mut items: Vec<&Object> = o.operands.iter().collect();
    match o.operator.as_str() {
        "TJ" => {
            items = o
                .operands
                .first()
                .and_then(|a| a.as_array().ok())
                .map(|a| a.iter().collect())
                .unwrap_or_default();
        }
        "\"" => {
            out.extend(
                ["Tw", "Tc"]
                    .into_iter()
                    .zip(&o.operands)
                    .map(|(n, v)| Operation::new(n, vec![v.clone()])),
            );
            out.push(Operation::new("T*", vec![]));
            items = o.operands.iter().skip(2).collect();
        }
        "'" => out.push(Operation::new("T*", vec![])),
        _ => {}
    }
    let known = |s: &[u8]| font.is_some_and(|f| f.codes_known && f.drawn_alike(s));
    let (Some((_, size)), Some(tc), Some(tw)) = (&t.font, t.tc, t.tw) else {
        return (out, false);
    };
    // In thousandths of the font size, as `TJ` counts: each character moves on by its
    // glyph's width and the spacing, each number back by itself.
    let mut moved = 0.0f32;
    for it in items {
        match it {
            Object::String(s, _) => {
                let Some(f) = font.filter(|_| known(s)) else {
                    return (out, false);
                };
                for (code, space, raw) in f.codes(s) {
                    let spacing = tc + if space { tw } else { 0.0 };
                    if spacing != 0.0 && *size == 0.0 {
                        return (out, false);
                    }
                    let spacing = if spacing == 0.0 {
                        0.0
                    } else {
                        spacing * 1000.0 / size
                    };
                    moved += f.width(code, raw.len()) + spacing;
                }
            }
            x => moved -= x.as_float().unwrap_or(0.0),
        }
    }
    if moved.abs() > 1e-4 {
        out.push(Operation::new(
            "TJ",
            vec![Object::Array(vec![Object::Real(-moved)])],
        ));
    }
    (out, true)
}

/// Why a stream's content couldn't be read.
enum Unread {
    /// Its compressed data is cut off or damaged part way.
    Damaged,
    Other(anyhow::Error),
}

/// A stream's content, decoded, and whether it had a wrong checksum; or why it couldn't be
/// read.
type Read = std::result::Result<(Vec<u8>, bool), Unread>;

impl Pdf {
    /// What [`Pdf::remove_hidden`] would remove of every kind, worked out on a copy. Hidden
    /// layers and content outside the crop box are counted even when their removal would be
    /// refused.
    pub fn find_hidden(&self) -> Result<HiddenReport> {
        self.clone().remove_kinds(&Hidden::ALL, false)
    }

    /// Remove the kinds of hidden information in `what`. Unused objects go when the
    /// document is saved, which rewrites the whole file, so nothing removed is kept in it.
    /// Refuses, with nothing changed, when it couldn't remove all of a kind: hidden layers
    /// in drawings it can't read or nested too deep, a layer rule it can't read, a form
    /// nested too deep, content outside the crop box redaction's check finds left.
    pub fn remove_hidden(&mut self, what: &[Hidden]) -> Result<HiddenReport> {
        let before = self.before();
        let got = self.remove_kinds(what, true);
        if got.is_err() {
            self.go_back(before);
        }
        got
    }

    /// Why [`Pdf::remove_hidden`] would refuse to remove `what`, if it would; changes nothing.
    /// For checking before anything else is done for the removal. Content outside the crop
    /// box is only found left by redacting it.
    pub fn check_remove_hidden(&self, what: &[Hidden]) -> Result<()> {
        if what.contains(&Hidden::HiddenLayers) && self.layers_to_remove()? {
            // What is no longer drawn is found by following the drawings afterwards.
            self.reach(&mut reach::work(), &mut reach::Decoded::new())?;
        }
        if what.contains(&Hidden::FormValues) {
            self.orphan_widgets(&self.value_nodes()?)?;
        }
        Ok(())
    }

    /// Whether removing hidden layers has anything to do; refuses when it can't be done for
    /// certain. Its budget of steps is done with when it returns.
    fn layers_to_remove(&self) -> Result<bool> {
        let Some((_, _, off)) = self.layer_list() else {
            return Ok(false);
        };
        let mut layers = Layers::new(off);
        let scan = self.scan_layers(&mut layers)?;
        if layers.refuses(&scan) {
            refuse_blocked(&scan)?;
        }
        Ok(layers.to_remove())
    }

    fn remove_kinds(&mut self, what: &[Hidden], strict: bool) -> Result<HiddenReport> {
        if strict {
            self.check_remove_hidden(what)?;
        }
        self.own_direct_annots()?;
        let mut report = HiddenReport::default();
        for h in Hidden::ALL {
            if !what.contains(&h) {
                continue;
            }
            let n = match h {
                Hidden::OutsideCrop => {
                    let (n, pictures) = self.cut_outside_crop(strict)?;
                    report.pictures_elsewhere = pictures;
                    n
                }
                Hidden::HiddenLayers => {
                    let (n, faults) = self.drop_hidden_layers(strict)?;
                    report.faults_overlooked |= faults;
                    n
                }
                Hidden::Comments => {
                    self.remove_annotations(|a| !matches!(a.kind, crate::AnnotKind::Redact))?
                }
                Hidden::Attachments => self.drop_attachments()?,
                Hidden::Scripts => self.drop_scripts(),
                Hidden::Metadata => self.drop_metadata(),
                Hidden::PrivateData => self.drop_private_data(),
                Hidden::Bookmarks => {
                    let n = self.outline().len();
                    if let Ok(c) = self.doc.catalog_mut() {
                        c.remove(b"Outlines");
                    }
                    n
                }
                Hidden::FormValues => self.clear_form_values()?,
            };
            if n > 0 {
                report.removed.push((h, n));
            }
        }
        Ok(report)
    }

    /// Remove `keys` from every dictionary in the file. Returns how many were removed.
    fn drop_keys_everywhere(&mut self, keys: &[&[u8]]) -> usize {
        let mut n = 0;
        for obj in self.doc.objects.values_mut() {
            let d = match obj {
                Object::Dictionary(d) => d,
                Object::Stream(s) => &mut s.dict,
                _ => continue,
            };
            for k in keys {
                if d.remove(k).is_some() {
                    n += 1;
                }
            }
        }
        n
    }

    /// Remove `keys` from the catalog's `/Names` dictionary. Returns how many were there.
    fn drop_names(&mut self, keys: &[&[u8]]) -> usize {
        let names = self
            .doc
            .catalog()
            .ok()
            .and_then(|c| c.get(b"Names").ok())
            .cloned();
        let d = match names {
            Some(Object::Reference(id)) => self.doc.get_dictionary_mut(id).ok(),
            Some(Object::Dictionary(_)) => self
                .doc
                .catalog_mut()
                .ok()
                .and_then(|c| c.get_mut(b"Names").ok())
                .and_then(|o| o.as_dict_mut().ok()),
            _ => None,
        };
        let Some(d) = d else { return 0 };
        keys.iter().filter(|k| d.remove(k).is_some()).count()
    }

    fn drop_metadata(&mut self) -> usize {
        let info = usize::from(self.doc.trailer.remove(b"Info").is_some());
        info + self.drop_keys_everywhere(&[b"Metadata"])
    }

    fn drop_private_data(&mut self) -> usize {
        // A page-piece's date of change goes with it (LimarePDF's paragraph frames date
        // their page's).
        let mut n = self.drop_keys_everywhere(&[b"PieceInfo", b"LastModified", b"Thumb"]);
        if let Ok(c) = self.doc.catalog_mut() {
            n += usize::from(c.remove(b"SpiderInfo").is_some());
        }
        n + self.drop_names(&[b"IDS", b"URLS"])
    }

    fn drop_attachments(&mut self) -> Result<usize> {
        let mut n = self.drop_names(&[b"EmbeddedFiles"]);
        if let Ok(c) = self.doc.catalog_mut() {
            n += usize::from(c.remove(b"Collection").is_some());
        }
        n += self.drop_keys_everywhere(&[b"AF"]);
        let files: HashSet<ObjectId> = self
            .every_annotation()
            .into_iter()
            .filter(|a| matches!(&a.kind, crate::AnnotKind::Other(s) if s == "FileAttachment"))
            .map(|a| a.id)
            .collect();
        if !files.is_empty() {
            n += self.drop_annotations(files)?;
        }
        Ok(n)
    }

    fn drop_scripts(&mut self) -> usize {
        let bad: HashSet<ObjectId> = self
            .doc
            .objects
            .iter()
            .filter(|(_, o)| o.as_dict().is_ok_and(is_active))
            .map(|(id, _)| *id)
            .collect();
        // Lists of next actions stored as objects of their own (`/Next 12 0 R`).
        fn next_lists(o: &Object, out: &mut Vec<ObjectId>) {
            let d = match o {
                Object::Dictionary(d) => d,
                Object::Stream(s) => &s.dict,
                Object::Array(a) => return a.iter().for_each(|x| next_lists(x, out)),
                _ => return,
            };
            if let Ok(id) = d.get(b"Next").and_then(Object::as_reference) {
                out.push(id);
            }
            d.iter().for_each(|(_, v)| next_lists(v, out));
        }
        let mut lists = Vec::new();
        for o in self.doc.objects.values() {
            next_lists(o, &mut lists);
        }
        let lists: HashSet<ObjectId> = (lists.into_iter())
            .filter(|id| self.doc.get_object(*id).is_ok_and(|o| o.as_array().is_ok()))
            .collect();
        let mut n = self.drop_names(&[b"JavaScript"]);
        let xfa_only = self.xfa_only();
        for (id, obj) in self.doc.objects.iter_mut() {
            if let (true, Object::Array(a)) = (lists.contains(id), &mut *obj) {
                let before = a.len();
                a.retain(|o| !is_script(o, &bad));
                n += before - a.len();
            }
            n += without_scripts(obj, &bad, xfa_only);
        }
        n
    }

    /// Delete what lies outside each page's crop box, then make the crop box the page.
    /// Returns how many pages were cut, and the pages where a drawing left as it was still
    /// draws an original picture or form cut ([`crate::RedactReport::pictures_elsewhere`]).
    /// When `strict`, refuses, after redacting, when redaction's check finds something left.
    fn cut_outside_crop(&mut self, strict: bool) -> Result<(usize, Vec<usize>)> {
        let mut marks = Vec::new();
        let mut cut = Vec::new();
        for (i, page) in self.page_ids().into_iter().enumerate() {
            let (media, crop) = (self.media_box(page), self.page_box(page));
            let inside = crop[0] > media[0] + 0.5
                || crop[1] > media[1] + 0.5
                || crop[2] < media[2] - 0.5
                || crop[3] < media[3] - 0.5;
            if !inside {
                continue;
            }
            // Around the shown page (displayed points), far enough to cover the media box.
            let (_, w, h) = self.vis_down(page);
            let m = (media[2] - media[0]).abs() + (media[3] - media[1]).abs();
            for r in [
                [-m, -m, w + m, 0.0],
                [-m, h, w + m, h + m],
                [-m, 0.0, 0.0, h],
                [w, 0.0, w + m, h],
            ] {
                marks.push((i, r));
            }
            cut.push((page, crop));
        }
        if marks.is_empty() {
            return Ok((0, Vec::new()));
        }
        let rep = self.redact(&marks)?;
        if strict && !rep.left.is_empty() {
            return Err(anyhow!(
                "Content outside the visible page area can't be removed for certain: the check \
                 after redacting found {}. Nothing was changed; leave out content outside the \
                 visible page area to remove the rest.",
                rep.left.join("; ")
            ));
        }
        for (page, crop) in &cut {
            let d = self.doc.get_dictionary_mut(*page)?;
            d.set(
                "MediaBox",
                crop.iter().map(|&v| Object::Real(v)).collect::<Vec<_>>(),
            );
            for k in [&b"CropBox"[..], b"TrimBox", b"BleedBox", b"ArtBox"] {
                d.remove(k);
            }
        }
        Ok((cut.len(), rep.pictures_elsewhere))
    }

    /// The layer properties, their default configuration, and the layers (optional content
    /// groups) switched off in it; `None` when the document has no layer list.
    fn layer_list(&self) -> Option<(Dictionary, Dictionary, HashSet<ObjectId>)> {
        let props = self
            .doc
            .catalog()
            .ok()
            .and_then(|c| c.get(b"OCProperties").ok())
            .map(|o| self.resolved_dict(Some(o)))?;
        let refs = |o: Option<&Object>| -> Vec<ObjectId> {
            o.and_then(|o| self.doc.dereference(o).ok())
                .and_then(|(_, o)| o.as_array().ok())
                .map(|a| a.iter().filter_map(|x| x.as_reference().ok()).collect())
                .unwrap_or_default()
        };
        let all = refs(props.get(b"OCGs").ok());
        let d = self.resolved_dict(props.get(b"D").ok());
        let base_off = d.get(b"BaseState").and_then(|o| o.as_name()).ok() == Some(b"OFF");
        let hidden: HashSet<ObjectId> = if base_off {
            let on: HashSet<ObjectId> = refs(d.get(b"ON").ok()).into_iter().collect();
            all.iter().copied().filter(|g| !on.contains(g)).collect()
        } else {
            refs(d.get(b"OFF").ok()).into_iter().collect()
        };
        Some((props, d, hidden))
    }

    /// Delete the content of the layers (optional content groups) that are switched off,
    /// and what layer rules hide, and take those layers out of the layer list; rules kept
    /// that named them are rewritten without them. Returns how many were found (the layers
    /// switched off, and the rules hiding content with every layer they name on), and
    /// whether data with a wrong checksum was read. When `strict`, refuses with nothing
    /// changed if some rule couldn't be read, or there is something to remove and some
    /// drawing couldn't be looked into; otherwise (finding) counts them and changes nothing
    /// then. With nothing to remove, nothing changes.
    fn drop_hidden_layers(&mut self, strict: bool) -> Result<(usize, bool)> {
        let Some((props, d, hidden)) = self.layer_list() else {
            return Ok((0, false));
        };
        let mut layers = Layers::new(hidden.clone());
        let scan = match self.scan_layers(&mut layers) {
            Ok(scan) => scan,
            Err(e) if strict => return Err(e.into()),
            // Found, so Remove can say why it stops.
            Err(_) => return Ok((hidden.len().max(1), false)),
        };
        let faults = scan.faults;
        if layers.refuses(&scan) {
            if strict {
                refuse_blocked(&scan)?;
            }
            return Ok((layers.found(), faults));
        }
        if !layers.to_remove() {
            return Ok((0, false));
        }
        for (drawn, ops, inline) in scan.edits {
            match drawn {
                Drawn::Page(page) => self.commit(page, ops, 0, &inline)?,
                Drawn::Stream(id) => {
                    let bytes = inline
                        .encode(ops)
                        .map_err(|e| anyhow!("cannot encode a form: {e}"))?;
                    if let Ok(s) = self.doc.get_object_mut(id).and_then(|o| o.as_stream_mut()) {
                        s.dict.remove(b"Filter");
                        s.dict.remove(b"DecodeParms");
                        s.set_content(bytes);
                        let _ = s.compress();
                    }
                }
            }
        }
        // Pictures and forms in a hidden layer are no longer drawn: out of the resources.
        let mut layered = HashSet::new();
        for (id, o) in &self.doc.objects {
            let oc = o.as_stream().ok().and_then(|s| s.dict.get(b"OC").ok());
            if oc.is_some_and(|oc| self.oc_hidden(oc, &mut layers)) {
                layered.insert(*id);
            }
        }
        if !layered.is_empty() {
            // The streams were changed in place, so they are read afresh. Finding (on a
            // copy) leaves them listed when the drawings are too complex to follow.
            match self.reach(&mut reach::work_after(0), &mut reach::Decoded::new()) {
                Ok(reach) => self.drop_unused_xobjects(&layered, &reach)?,
                Err(e) if strict => return Err(e.into()),
                Err(_) => {}
            }
        }
        // Annotations in a hidden layer, and their drawings in one: a drawing for a state
        // goes, and the annotation when its normal drawing does.
        let mut gone = HashSet::new();
        let mut looks = Vec::new();
        for page in self.page_ids() {
            for id in self.all_annot_refs(page) {
                let Ok(annot) = self.doc.get_dictionary(id) else {
                    continue;
                };
                if annot
                    .get(b"OC")
                    .is_ok_and(|o| self.oc_hidden(o, &mut layers))
                {
                    gone.insert(id);
                    continue;
                }
                let Ok(ap) = annot.get(b"AP") else {
                    continue;
                };
                let mut ap = self.resolved_dict(Some(ap));
                let mut changed = false;
                for (k, v) in ap.clone().iter() {
                    match self.doc.dereference(v) {
                        Ok((_, Object::Stream(_))) => {
                            if self.look_hidden(v, &mut layers) {
                                ap.remove(k);
                                changed = true;
                            }
                        }
                        Ok((_, Object::Dictionary(states))) => {
                            let mut states = states.clone();
                            let hidden: Vec<Vec<u8>> = (states.iter())
                                .filter(|(_, s)| self.look_hidden(s, &mut layers))
                                .map(|(n, _)| n.clone())
                                .collect();
                            if hidden.is_empty() {
                                continue;
                            }
                            changed = true;
                            for n in hidden {
                                states.remove(&n);
                            }
                            if states.is_empty() {
                                ap.remove(k);
                            } else {
                                ap.set(k.clone(), states);
                            }
                        }
                        _ => {}
                    }
                }
                if changed && ap.has(b"N") {
                    looks.push((id, ap));
                } else if changed {
                    gone.insert(id);
                }
            }
        }
        for (id, ap) in looks {
            if let Ok(d) = self.doc.get_dictionary_mut(id) {
                d.set("AP", ap);
            }
        }
        if !gone.is_empty() {
            self.drop_annotations(gone)?;
        }
        self.drop_hidden_properties(&layers);
        self.rewrite_rules(&layers);

        // The layer list: hidden layers out, the rest on.
        let keep = |o: &Object| o.as_reference().map_or(true, |id| !hidden.contains(&id));
        fn prune(a: &[Object], keep: &dyn Fn(&Object) -> bool) -> Vec<Object> {
            a.iter()
                .filter(|o| keep(o))
                .map(|o| match o {
                    Object::Array(inner) => Object::Array(prune(inner, keep)),
                    o => o.clone(),
                })
                .collect()
        }
        let mut props = props;
        if let Ok(Object::Array(a)) = props.get(b"OCGs").cloned() {
            props.set("OCGs", prune(&a, &keep));
        }
        let mut d = d;
        for k in [&b"OFF"[..], b"BaseState", b"AS", b"Locked", b"RBGroups"] {
            d.remove(k);
        }
        for k in [&b"ON"[..], b"Order"] {
            if let Ok(Object::Array(a)) = d.get(k).cloned() {
                d.set(k.to_vec(), prune(&a, &keep));
            }
        }
        props.set("D", d);
        props.remove(b"Configs");
        self.doc
            .catalog_mut()
            .map_err(|e| anyhow!("{e}"))?
            .set("OCProperties", props);
        Ok((layers.found(), faults))
    }

    /// Whether `look`, an annotation's drawing, is in a hidden layer of its own.
    fn look_hidden(&self, look: &Object, layers: &mut Layers) -> bool {
        let oc = (self.doc.dereference(look).ok())
            .and_then(|(_, s)| s.as_stream().ok())
            .and_then(|s| s.dict.get(b"OC").ok());
        oc.is_some_and(|oc| self.oc_hidden(oc, layers))
    }

    /// Put in place what `layers` found the rules kept that named a layer that's off, and
    /// the expressions stored apart that they use, become.
    fn rewrite_rules(&mut self, layers: &Layers) {
        for (id, e) in &layers.exprs_new {
            self.doc.objects.insert(*id, Object::Array(e.clone()));
        }
        let mut direct = false;
        for (key, new) in &layers.rewrites {
            match key {
                RuleKey::Stored(id) => {
                    self.doc
                        .objects
                        .insert(*id, Object::Dictionary(new.clone()));
                }
                RuleKey::Direct(_) => direct = true,
            }
        }
        if !direct {
            return;
        }
        fn walk(o: &mut Object, rewrites: &HashMap<RuleKey, Dictionary>) {
            let d = match o {
                Object::Dictionary(d) => d,
                Object::Stream(s) => &mut s.dict,
                Object::Array(a) => return a.iter_mut().for_each(|x| walk(x, rewrites)),
                _ => return,
            };
            if d.get(b"Type").and_then(Object::as_name).ok() == Some(b"OCMD") {
                if let Some(new) = rewrites.get(&RuleKey::Direct(format!("{d:?}"))) {
                    *d = new.clone();
                    return;
                }
            }
            d.iter_mut().for_each(|(_, v)| walk(v, rewrites));
        }
        for o in self.doc.objects.values_mut() {
            walk(o, &layers.rewrites);
        }
    }

    /// Every drawing that can hold content `layers` hide, without it: the pages' content,
    /// then what their resources and their annotations' appearances draw (forms, tiling
    /// patterns, Type 3 glyphs, soft masks), nested up to [`MAX_NESTING`], each once (layer
    /// states are the document's, so a drawing with its own resources looks the same
    /// wherever it is used). One without resources of its own is checked against every
    /// place it is used from. What couldn't be read (a layer rule too), is nested deeper,
    /// or would lose different content in different places, goes in `blocked`. Refuses when
    /// working out the layer rules would take too many steps.
    fn scan_layers(&self, layers: &mut Layers) -> std::result::Result<LayerScan, TooComplex> {
        let mut scan = LayerScan::default();
        self.read_rules(layers, &mut scan);
        let mut seen = HashMap::new();
        for (i, page) in self.page_ids().into_iter().enumerate() {
            let res = self.resolved_dict(self.inherited(page, b"Resources").as_ref());
            let content = self.page_content(page);
            let name = format!("page {}'s content", i + 1);
            self.scan_read(Drawn::Page(page), content, &res, layers, &name, &mut scan);
            self.scan_resources(&res, layers, &mut seen, 0, &mut scan);
            for annot in self.all_annot_refs(page) {
                let ap = self
                    .doc
                    .get_dictionary(annot)
                    .ok()
                    .and_then(|d| d.get(b"AP").ok())
                    .map(|o| self.resolved_dict(Some(o)));
                for (_, look) in ap.iter().flat_map(|ap| ap.iter()) {
                    // A drawing, or a dictionary of drawings by state.
                    let ids: Vec<ObjectId> = match self.doc.dereference(look) {
                        Ok((_, Object::Dictionary(states))) => states
                            .iter()
                            .filter_map(|(_, s)| s.as_reference().ok())
                            .collect(),
                        _ => look.as_reference().ok().into_iter().collect(),
                    };
                    for id in ids {
                        self.scan_drawing(id, &Dictionary::new(), layers, &mut seen, 0, &mut scan);
                    }
                }
            }
        }
        let mut once = HashSet::new();
        scan.blocked.retain(|b| once.insert(b.clone()));
        match layers.too.take() {
            Some(too) => Err(too),
            None => Ok(scan),
        }
    }

    /// Note in `layers` how each layer rule something in the file uses reads (the `/OC` of
    /// a drawing or an annotation, or an entry of `/Properties`), and in `scan.blocked`
    /// each that can't be read.
    fn read_rules(&self, layers: &mut Layers, scan: &mut LayerScan) {
        fn uses<'a>(doc: &'a Document, o: &'a Object, out: &mut Vec<&'a Object>) {
            let d = match o {
                Object::Dictionary(d) => d,
                Object::Stream(s) => &s.dict,
                Object::Array(a) => return a.iter().for_each(|x| uses(doc, x, out)),
                _ => return,
            };
            out.extend(d.get(b"OC").ok());
            let props = match d.get(b"Properties") {
                Ok(Object::Reference(id)) => doc.get_dictionary(*id).ok(),
                Ok(Object::Dictionary(p)) => Some(p),
                _ => None,
            };
            out.extend(props.iter().flat_map(|p| p.iter().map(|(_, v)| v)));
            d.iter().for_each(|(_, v)| uses(doc, v, out));
        }
        for (holder, o) in &self.doc.objects {
            let mut found = Vec::new();
            uses(&self.doc, o, &mut found);
            for oc in found {
                let Some((key, d)) = rule_of(&self.doc, oc) else {
                    continue;
                };
                if let Err(why) = layers.used(&self.doc, &key, d) {
                    scan.blocked.push(match key {
                        RuleKey::Stored(id) => {
                            format!("a layer rule (object {}) can't be read ({why})", id.0)
                        }
                        RuleKey::Direct(_) => {
                            format!("a layer rule in object {} can't be read ({why})", holder.0)
                        }
                    });
                }
            }
        }
    }

    /// [`Pdf::scan_layers`] for the drawings resources `res` hold, at nesting `depth`.
    fn scan_resources(
        &self,
        res: &Dictionary,
        layers: &mut Layers,
        seen: &mut HashMap<ObjectId, Seen>,
        depth: usize,
        scan: &mut LayerScan,
    ) {
        let entries = |key: &[u8]| self.resolved_dict(res.get(key).ok());
        let stream = |o: &Object| -> Option<(ObjectId, &lopdf::Stream)> {
            let id = o.as_reference().ok()?;
            Some((id, self.doc.get_object(id).ok()?.as_stream().ok()?))
        };
        let name_is = |d: &Dictionary, key: &[u8], v: &[u8]| {
            d.get(key).and_then(|o| o.as_name()).ok() == Some(v)
        };
        let mut drawings: Vec<ObjectId> = Vec::new();
        for (_, e) in entries(b"XObject").iter() {
            if let Some((id, s)) = stream(e) {
                if name_is(&s.dict, b"Subtype", b"Form") {
                    drawings.push(id);
                }
            }
        }
        for (_, e) in entries(b"Pattern").iter() {
            if let Some((id, _)) = stream(e) {
                drawings.push(id);
            }
        }
        for (_, e) in entries(b"ExtGState").iter() {
            let gs = self.resolved_dict(Some(e));
            let mask = self.resolved_dict(gs.get(b"SMask").ok());
            if let Some((id, _)) = mask.get(b"G").ok().and_then(stream) {
                drawings.push(id);
            }
        }
        let mut glyphs: Vec<(ObjectId, Dictionary)> = Vec::new();
        for (_, e) in entries(b"Font").iter() {
            let font = self.resolved_dict(Some(e));
            if !name_is(&font, b"Subtype", b"Type3") {
                continue;
            }
            let fres = font
                .get(b"Resources")
                .ok()
                .map_or_else(|| res.clone(), |o| self.resolved_dict(Some(o)));
            let procs = self.resolved_dict(font.get(b"CharProcs").ok());
            for (_, g) in procs.iter() {
                if let Some((id, _)) = stream(g) {
                    glyphs.push((id, fres.clone()));
                }
            }
        }
        let todo = drawings
            .into_iter()
            .map(|id| (id, res.clone()))
            .chain(glyphs);
        for (id, outer) in todo {
            if !seen.contains_key(&id) && depth >= MAX_NESTING {
                scan.blocked
                    .push(format!("drawings are nested more than {MAX_NESTING} deep"));
                return;
            }
            self.scan_drawing(id, &outer, layers, seen, depth + 1, scan);
        }
    }

    /// [`Pdf::scan_layers`] for drawing `id`, which draws with its own resources, else with
    /// `outer`. One looked into already is only checked against `outer`, when it draws with
    /// it.
    fn scan_drawing(
        &self,
        id: ObjectId,
        outer: &Dictionary,
        layers: &mut Layers,
        seen: &mut HashMap<ObjectId, Seen>,
        depth: usize,
        scan: &mut LayerScan,
    ) {
        let Ok(s) = self.doc.get_object(id).and_then(|o| o.as_stream()) else {
            seen.entry(id).or_default();
            return;
        };
        let own = s.dict.get(b"Resources").ok();
        if let Some(was) = seen.get_mut(&id) {
            if own.is_none() {
                self.check_shared(id, s, outer, layers, was, scan);
            }
            return;
        }
        let res = own.map_or_else(|| outer.clone(), |o| self.resolved_dict(Some(o)));
        let mut was = Seen::default();
        if own.is_none() {
            was.first = Some(res.clone());
            was.checked.push(self.hidden_names(&res, layers));
        }
        seen.insert(id, was);
        let name = format!("a drawing in the file (object {})", id.0);
        self.scan_read(Drawn::Stream(id), readable(s), &res, layers, &name, scan);
        self.scan_resources(&res, layers, seen, depth, scan);
    }

    /// Check drawing `id` (stream `s`), which has no resources of its own and was looked
    /// into with `was.first`, against resources `res` of another place it is used from.
    /// Where it would lose other content, the removal can't be done on the one drawing:
    /// that goes in `blocked`.
    fn check_shared(
        &self,
        id: ObjectId,
        s: &lopdf::Stream,
        res: &Dictionary,
        layers: &mut Layers,
        was: &mut Seen,
        scan: &mut LayerScan,
    ) {
        let names = self.hidden_names(res, layers);
        if was.checked.contains(&names) {
            return;
        }
        was.checked.push(names);
        let (Some(first), Ok((data, _))) = (&was.first, readable(s)) else {
            return;
        };
        let looks = [first, res].map(|r| self.without_hidden(&data, r, layers, false));
        let name = format!("a drawing in the file (object {})", id.0);
        if let Some(why) = looks
            .iter()
            .find_map(|l| l.as_ref().err()?.downcast_ref::<Unsure>())
        {
            scan.blocked.push(format!("{name} {why}"));
            return;
        }
        // What can't be read is in `blocked` already, from the first look.
        let [Ok(a), Ok(b)] = looks else {
            return;
        };
        let same = match (a, b) {
            (None, None) => true,
            (Some((a, _)), Some((b, _))) => {
                a.len() == b.len()
                    && a.iter()
                        .zip(&b)
                        .all(|(x, y)| x.operator == y.operator && x.operands == y.operands)
            }
            _ => false,
        };
        if !same {
            scan.blocked.push(format!(
                "{name} is shared between places that set its layers or fonts differently"
            ));
        }
    }

    /// The names resources `res` give content `layers` hide, and their fonts, as
    /// [`Pdf::without_hidden`] looks them up.
    fn hidden_names(&self, res: &Dictionary, layers: &mut Layers) -> HiddenNames {
        let props = self.resolved_dict(res.get(b"Properties").ok());
        let mut names: Vec<Vec<u8>> = props
            .iter()
            .filter(|(_, o)| self.oc_hidden(o, layers))
            .map(|(n, _)| n.clone())
            .collect();
        let xobjs = self.resolved_dict(res.get(b"XObject").ok());
        let mut drawings: Vec<Vec<u8>> = xobjs
            .iter()
            .filter(|(_, e)| {
                self.doc
                    .dereference(e)
                    .ok()
                    .and_then(|(_, s)| s.as_stream().ok())
                    .and_then(|s| s.dict.get(b"OC").ok())
                    .is_some_and(|oc| self.oc_hidden(oc, layers))
            })
            .map(|(n, _)| n.clone())
            .collect();
        // Fonts by what they are: the same font can be a reference in one place and copied
        // in another.
        let mut fonts: Vec<(Vec<u8>, String)> = self
            .resolved_dict(res.get(b"Font").ok())
            .iter()
            .map(|(n, f)| {
                let font = self.doc.dereference(f).map(|(_, f)| f).unwrap_or(f);
                (n.clone(), format!("{font:?}"))
            })
            .collect();
        names.sort();
        drawings.sort();
        fonts.sort();
        (names, drawings, fonts)
    }

    /// Page `page`'s content streams, read with [`readable`] and joined as lopdf joins them,
    /// at most [`crate::MAX_CONTENT`] bytes in all.
    fn page_content(&self, page: ObjectId) -> Read {
        let mut all = Vec::new();
        let mut faults = false;
        for c in self.doc.get_page_contents(page) {
            let Ok(s) = self.doc.get_object(c).and_then(|o| o.as_stream()) else {
                continue;
            };
            let (data, fault) = readable(s)?;
            all.extend_from_slice(&data);
            all.push(b'\n');
            faults |= fault;
            if all.len() > crate::MAX_CONTENT {
                return Err(Unread::Other(anyhow!("it is too large")));
            }
        }
        Ok((all, faults))
    }

    /// Record in `scan` what [`Pdf::without_hidden`] makes of drawing `drawn`, read as
    /// `read` and drawn with resources `res`; `name` names it when it couldn't be read.
    fn scan_read(
        &self,
        drawn: Drawn,
        read: Read,
        res: &Dictionary,
        layers: &mut Layers,
        name: &str,
        scan: &mut LayerScan,
    ) {
        let page = matches!(drawn, Drawn::Page(_));
        let edit = read.and_then(|(data, fault)| {
            scan.faults |= fault;
            self.without_hidden(&data, res, layers, page)
                .map_err(Unread::Other)
        });
        match edit {
            Ok(Some((ops, inline))) => scan.edits.push((drawn, ops, inline)),
            Ok(None) => {}
            Err(Unread::Damaged) => {
                scan.damaged = true;
                scan.blocked.push(format!("{name} is damaged"));
            }
            Err(Unread::Other(e)) => {
                let why = (e.downcast_ref::<Unsure>().map(|u| u.to_string()))
                    .or_else(|| e.downcast_ref::<InlineRule>().map(|r| r.to_string()));
                scan.blocked.push(match why {
                    Some(why) => format!("{name} {why}"),
                    None => format!("{name} can't be read ({e})"),
                })
            }
        }
    }

    /// Take the entries naming hidden layers, or rules hiding their content, out of every
    /// `/Properties` resource dictionary: the content that used them is gone.
    fn drop_hidden_properties(&mut self, layers: &Layers) {
        let hides = |key: RuleKey| matches!(layers.rules.get(&key), Some(Ok((true, _))));
        let gone = |o: &Object| match o {
            Object::Reference(id) => layers.off.contains(id) || hides(RuleKey::Stored(*id)),
            Object::Dictionary(d) => {
                d.get(b"Type").and_then(Object::as_name).ok() == Some(b"OCMD")
                    && hides(RuleKey::Direct(format!("{d:?}")))
            }
            _ => false,
        };
        fn walk(o: &mut Object, gone: &dyn Fn(&Object) -> bool, lists: &mut Vec<ObjectId>) {
            let d = match o {
                Object::Dictionary(d) => d,
                Object::Stream(s) => &mut s.dict,
                Object::Array(a) => return a.iter_mut().for_each(|x| walk(x, gone, lists)),
                _ => return,
            };
            match d.get_mut(b"Properties") {
                Ok(Object::Dictionary(props)) => {
                    let names: Vec<Vec<u8>> = props
                        .iter()
                        .filter(|(_, v)| gone(v))
                        .map(|(k, _)| k.clone())
                        .collect();
                    for k in names {
                        props.remove(&k);
                    }
                }
                Ok(Object::Reference(id)) => lists.push(*id),
                _ => {}
            }
            for (_, v) in d.iter_mut() {
                walk(v, gone, lists);
            }
        }
        let mut lists = Vec::new();
        for o in self.doc.objects.values_mut() {
            walk(o, &gone, &mut lists);
        }
        for id in lists {
            if let Ok(props) = self.doc.get_dictionary_mut(id) {
                let names: Vec<Vec<u8>> = props
                    .iter()
                    .filter(|(_, v)| gone(v))
                    .map(|(k, _)| k.clone())
                    .collect();
                for k in names {
                    props.remove(&k);
                }
            }
        }
    }

    /// Whether optional content `oc` (a layer, or a layer rule) is hidden, by `layers`. A
    /// rule that can't be read hides nothing here; the scan refuses for it.
    fn oc_hidden(&self, oc: &Object, layers: &mut Layers) -> bool {
        if oc.as_reference().is_ok_and(|id| layers.off.contains(&id)) {
            return true;
        }
        rule_of(&self.doc, oc)
            .is_some_and(|(key, d)| matches!(layers.read(&self.doc, &key, d), Ok((true, _))))
    }

    /// A content stream without what `layers` hide: marked content tagged with a hidden
    /// layer or a rule hiding it, and pictures and forms that belong to one; a rule written
    /// in it that is kept but names a layer that's off is rewritten. `None` when nothing
    /// changed. `page`: the stream is a page's content, which starts with the default text
    /// state (a form or other drawing starts with that of where it is used).
    ///
    /// Hidden content still sets what is drawn after it (ISO 32000-1, 8.11.3.2), so only
    /// its painting goes: settings (colour, position, line style, text state) are kept, a
    /// clipping path still clips, and hidden text becomes spacing as wide. Refuses with
    /// [`Unsure`] where the result wouldn't look the same, and with [`InlineRule`] where a
    /// rule written in it can't be read.
    #[allow(clippy::type_complexity)]
    fn without_hidden(
        &self,
        data: &[u8],
        res: &Dictionary,
        layers: &mut Layers,
        page: bool,
    ) -> Result<Option<(Vec<Operation>, Inline)>> {
        let (data, inline) = crate::inline::extract(data).map_err(|e| anyhow!("{e}"))?;
        let ops = Content::decode(&data)
            .map_err(|e| anyhow!("cannot parse the page content: {e}"))?
            .operations;
        let props = self.resolved_dict(res.get(b"Properties").ok());
        let xobjs = self.resolved_dict(res.get(b"XObject").ok());
        let fonts = self.resolved_dict(res.get(b"Font").ok());
        let mut cache: HashMap<Vec<u8>, Option<FontCtx>> = HashMap::new();
        let mut out = Vec::with_capacity(ops.len());
        let mut changed = false;
        let mut text = TextState::new(page);
        let mut saved: Vec<TextState> = Vec::new();
        // Hidden text of unknown width moved the text position by an unknown amount.
        let mut lost = false;
        // While in hidden content: its marked-content depth, and the path being built.
        let mut skip: Option<usize> = None;
        let mut path: Vec<Operation> = Vec::new();
        // A path begun in what is kept and not yet painted, which hidden content may paint.
        let mut open = false;
        for mut o in ops {
            let name = o.operator.as_str();
            text.follow(&o, &mut saved);
            if matches!(name, "BT" | "ET" | "Td" | "TD" | "T*" | "Tm" | "'" | "\"") {
                lost = false;
            }
            if let Some(depth) = skip.as_mut() {
                match name {
                    "BDC" | "BMC" => *depth += 1,
                    "EMC" => {
                        *depth -= 1;
                        if *depth == 0 {
                            if !path.is_empty() {
                                return Err(Unsure::SHAPE.into());
                            }
                            skip = None;
                        }
                    }
                    "m" | "l" | "c" | "v" | "y" | "h" | "re" | "W" | "W*" => path.push(o),
                    "S" | "s" | "f" | "F" | "f*" | "B" | "B*" | "b" | "b*" | "n" => {
                        // The path ends unpainted: a clipping path still clips, and a path
                        // begun before the hidden content ends here, not at the next painting.
                        if open || path.iter().any(|p| p.operator.starts_with('W')) {
                            out.append(&mut path);
                            out.push(Operation::new("n", vec![]));
                        }
                        path.clear();
                        open = false;
                    }
                    "Tj" | "TJ" | "'" | "\"" => {
                        if text.render >= 4 {
                            return Err(Unsure::CLIP.into());
                        }
                        let font = text.font.as_ref().and_then(|(f, _)| {
                            cache
                                .entry(f.clone())
                                .or_insert_with(|| {
                                    let (_, d) = self.doc.dereference(fonts.get(f).ok()?).ok()?;
                                    d.as_dict().ok().map(|d| FontCtx::new(&self.doc, d))
                                })
                                .as_ref()
                        });
                        let (ops, width) = hidden_show(&o, &text, font);
                        out.extend(ops);
                        lost = !width;
                    }
                    // Painting.
                    "sh" | "Do" | "MP" | "DP" => {}
                    t if inline.raw(t).is_some() => {}
                    _ => out.push(o),
                }
                continue;
            }
            if lost && matches!(name, "Tj" | "TJ") {
                return Err(Unsure::WIDTH.into());
            }
            match o.operator.as_str() {
                "BDC" if o.operands.first().and_then(|t| t.as_name().ok()) == Some(b"OC") => {
                    let mut rewrite = None;
                    let hides = match o.operands.get(1) {
                        Some(Object::Name(n)) => {
                            props.get(n).is_ok_and(|p| self.oc_hidden(p, layers))
                        }
                        Some(oc @ Object::Dictionary(_)) => match rule_of(&self.doc, oc) {
                            Some((key, d)) => match layers.used(&self.doc, &key, d) {
                                Err(why) => return Err(InlineRule(why).into()),
                                Ok((hides, _)) => {
                                    rewrite = layers.rewrites.get(&key).cloned();
                                    hides
                                }
                            },
                            None => false,
                        },
                        Some(oc) => self.oc_hidden(oc, layers),
                        None => false,
                    };
                    if hides {
                        skip = Some(1);
                        changed = true;
                        continue;
                    }
                    if let Some(new) = rewrite {
                        o.operands[1] = Object::Dictionary(new);
                        changed = true;
                    }
                }
                "Do" => {
                    let oc = o
                        .operands
                        .first()
                        .and_then(|n| n.as_name().ok())
                        .and_then(|n| xobjs.get(n).ok())
                        .and_then(|e| self.doc.dereference(e).ok())
                        .and_then(|(_, s)| s.as_stream().ok())
                        .and_then(|s| s.dict.get(b"OC").ok());
                    if oc.is_some_and(|oc| self.oc_hidden(oc, layers)) {
                        changed = true;
                        continue;
                    }
                }
                "m" | "l" | "c" | "v" | "y" | "h" | "re" | "W" | "W*" => open = true,
                "S" | "s" | "f" | "F" | "f*" | "B" | "B*" | "b" | "b*" | "n" => open = false,
                _ => {}
            }
            out.push(o);
        }
        Ok(changed.then_some((out, inline)))
    }
}

/// A stream's content, decoded with its filters in turn. lopdf returns what Flate data
/// decodes to up to a fault rather than an error, and can lose the end of data whose
/// checksum is wrong, so Flate data is checked to its end first: complete data with a wrong
/// or missing checksum is read in full (as other readers do) and reported; data cut off or
/// damaged part way is [`Unread::Damaged`]. Data in other filters that decodes to nothing
/// is an error, as lopdf gives no error for it either.
fn readable(s: &lopdf::Stream) -> Read {
    let max = crate::MAX_CONTENT;
    let filters = s.filters().unwrap_or_default();
    if filters.is_empty() {
        let data = s.get_plain_content_with_limit(max);
        return data
            .map(|d| (d, false))
            .map_err(|e| Unread::Other(anyhow!("{e}")));
    }
    let params = s.dict.get(b"DecodeParms").and_then(Object::as_dict).ok();
    let mut data = s.content.clone();
    let mut fault = false;
    for f in &filters {
        if *f == b"FlateDecode" && !data.is_empty() {
            match inflate(&data, max).map_err(Unread::Other)? {
                Inflated::Whole => {}
                Inflated::BadChecksum(fixed) => {
                    data = fixed;
                    fault = true;
                }
                Inflated::Damaged => return Err(Unread::Damaged),
            }
        }
        let mut d = Dictionary::new();
        d.set("Filter", Object::Name(f.to_vec()));
        if let Some(p) = params {
            d.set("DecodeParms", p.clone());
        }
        data = lopdf::Stream::new(d, data)
            .get_plain_content_with_limit(max)
            .map_err(|e| Unread::Other(anyhow!("{e}")))?;
    }
    let flate = filters.iter().any(|f| *f == b"FlateDecode");
    if data.is_empty() && !s.content.trim_ascii_end().is_empty() && !flate {
        return Err(Unread::Other(anyhow!("its compressed data doesn't decode")));
    }
    Ok((data, fault))
}

/// What [`inflate`] found.
enum Inflated {
    Whole,
    /// Complete, but its checksum is wrong or missing: the data with the right one.
    BadChecksum(Vec<u8>),
    /// Cut off, or damaged part way.
    Damaged,
}

/// Check zlib data `z` (Flate data) to its end, decoding at most `limit` bytes of it.
fn inflate(z: &[u8], limit: usize) -> Result<Inflated> {
    use flate2::{Decompress, FlushDecompress, Status};
    // A zlib header: deflate, a valid window, no preset dictionary, its check bits right.
    let header = z.len() >= 2
        && z[0] & 0x0f == 8
        && z[0] >> 4 <= 7
        && z[1] & 0x20 == 0
        && (u16::from(z[0]) << 8 | u16::from(z[1])) % 31 == 0;
    if !header {
        return Ok(Inflated::Damaged);
    }
    let body = &z[2..];
    let mut d = Decompress::new(false);
    let mut out = Vec::with_capacity(body.len().saturating_mul(2).min(limit + 1));
    loop {
        if out.len() == out.capacity() {
            if out.len() > limit {
                return Err(anyhow!("it is too large"));
            }
            out.reserve(out.len().max(1024).min(limit + 1 - out.len()));
        }
        let (used, made) = (d.total_in(), d.total_out());
        match d.decompress_vec(&body[used as usize..], &mut out, FlushDecompress::None) {
            Ok(Status::StreamEnd) => break,
            // Nothing more comes out with room for it: the data stops short.
            Ok(_) if d.total_in() == used && d.total_out() == made => return Ok(Inflated::Damaged),
            Ok(_) => {}
            Err(_) => return Ok(Inflated::Damaged),
        }
    }
    if out.len() > limit {
        return Err(anyhow!("it is too large"));
    }
    let end = 2 + d.total_in() as usize;
    let sum = adler32(&out).to_be_bytes();
    if z.get(end..end + 4) == Some(&sum[..]) {
        return Ok(Inflated::Whole);
    }
    let mut fixed = z[..end].to_vec();
    fixed.extend_from_slice(&sum);
    Ok(Inflated::BadChecksum(fixed))
}

/// zlib's checksum (RFC 1950).
fn adler32(data: &[u8]) -> u32 {
    let (mut a, mut b) = (1u32, 0u32);
    // The most bytes that can be added before the sums must be reduced.
    for chunk in data.chunks(5552) {
        for &x in chunk {
            a += u32::from(x);
            b += a;
        }
        a %= 65521;
        b %= 65521;
    }
    b << 16 | a
}

/// A refusal naming what couldn't be looked into, when anything couldn't. Damaged data is
/// told in plain words; it is the likely cause people meet.
fn refuse_blocked(scan: &LayerScan) -> Result<()> {
    if scan.damaged {
        return Err(LayersDamaged.into());
    }
    if scan.blocked.is_empty() {
        return Ok(());
    }
    Err(anyhow!(
        "hidden layers can't be removed for certain: {}. Nothing was changed; leave out \
         hidden layers to remove the rest",
        scan.blocked.join("; ")
    ))
}

/// Whether an action dictionary runs something by itself.
fn is_active(d: &Dictionary) -> bool {
    d.get(b"S")
        .and_then(|o| o.as_name())
        .is_ok_and(|s| ACTIVE.contains(&s))
}

/// Whether `o` is an action that runs something by itself: one of `bad` (the objects that
/// are such actions), or one written in place.
fn is_script(o: &Object, bad: &HashSet<ObjectId>) -> bool {
    match o {
        Object::Reference(id) => bad.contains(id),
        Object::Dictionary(d) => is_active(d),
        _ => false,
    }
}

/// Take the actions that run something out of `o` and out of every dictionary written
/// directly inside it (an ordinary link's action whose `/Next` holds JavaScript, an
/// annotation written into a page's list), where `bad` are the objects that are such
/// actions. Ordinary actions stay. Returns how many were removed. Direct objects can't loop,
/// and lopdf's parser bounds how deeply they nest.
fn without_scripts(o: &mut Object, bad: &HashSet<ObjectId>, xfa_only: bool) -> usize {
    let d = match o {
        Object::Dictionary(d) => d,
        Object::Stream(s) => &mut s.dict,
        Object::Array(a) => {
            return a
                .iter_mut()
                .map(|x| without_scripts(x, bad, xfa_only))
                .sum()
        }
        _ => return 0,
    };
    let is_bad = |o: &Object| is_script(o, bad);
    // Additional actions: on opening, closing, printing, keystrokes, focus….
    let mut n = usize::from(d.remove(b"AA").is_some());
    for key in [&b"A"[..], b"OpenAction"] {
        if d.get(key).is_ok_and(is_bad) {
            d.remove(key);
            n += 1;
        }
    }
    match d.get_mut(b"Next") {
        Ok(Object::Array(a)) => {
            let before = a.len();
            a.retain(|o| !is_bad(o));
            n += before - a.len();
        }
        Ok(o) if is_bad(o) => {
            d.remove(b"Next");
            n += 1;
        }
        _ => {}
    }
    // A hybrid form's XFA part holds scripts of its own; the ordinary fields stay.
    if !xfa_only && d.has(b"Fields") && d.remove(b"XFA").is_some() {
        n += 1;
    }
    for (_, v) in d.iter_mut() {
        n += without_scripts(v, bad, xfa_only);
    }
    n
}

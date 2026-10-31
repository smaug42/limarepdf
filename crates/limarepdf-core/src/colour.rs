//! Colours as a page's content sets them (PB-65): what set a colour, kept to be written
//! back as it was (a spot colour stays the same ink), and the colour it shows as, for the
//! inspector and for telling a line's parts apart. Colour spaces are read from the page's
//! resources: the device spaces, the calibrated and ICC-based ones by their components, Lab,
//! Indexed, and Separation and DeviceN through their tint transforms (functions of types
//! 0, 2, 3 and 4). Shown colours are approximate where a profile would decide.

use lopdf::content::Operation;
use lopdf::{Dictionary, Document, Object};
use std::sync::Arc;

/// How deep colour spaces and functions are read inside each other: an Indexed space's
/// base, a Separation's alternate space, a stitching function's parts.
const MAX_NESTING: usize = 4;
/// Most inputs a sampled function is read with: its corners are 2 to this power.
const MAX_INPUTS: usize = 8;
/// Most values a calculator function's stack holds, as PDF allows.
const MAX_STACK: usize = 100;

/// What set a fill colour in a page's content: the operators to write it again with.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Paint {
    /// The colour space's name a `cs` gave, when one did (not `g`, `rg` or `k`).
    space: Option<Object>,
    /// The operator that set the colour and its operands: `g`, `rg` or `k`, or `sc` or
    /// `scn` in `space`; none after `cs` alone, which sets the space's first colour.
    set: Option<(String, Vec<Object>)>,
}

impl Default for Paint {
    /// Black, as a page starts.
    fn default() -> Self {
        Self {
            space: None,
            set: Some(("g".into(), vec![Object::Integer(0)])),
        }
    }
}

impl Paint {
    /// Set by `g`, `rg` or `k` with `operands`.
    pub(crate) fn device(op: &str, operands: &[Object]) -> Self {
        Self {
            space: None,
            set: Some((op.into(), operands.to_vec())),
        }
    }

    /// The colour space `name` chosen by `cs`, in its first colour.
    pub(crate) fn space(name: &Object) -> Self {
        Self {
            space: Some(name.clone()),
            set: None,
        }
    }

    /// This paint's space set to the colour `sc` or `scn` (`op`) gives with `operands`. In a
    /// device space `g`, `rg` or `k` chose, it is that operator's colour.
    pub(crate) fn with(&self, op: &str, operands: &[Object]) -> Self {
        match &self.space {
            Some(_) => Self {
                space: self.space.clone(),
                set: Some((op.into(), operands.to_vec())),
            },
            None => match operands.len() {
                1 => Self::device("g", operands),
                3 => Self::device("rg", operands),
                4 => Self::device("k", operands),
                // Values the space can't take leave the colour as it was.
                _ => self.clone(),
            },
        }
    }

    /// Grey or RGB, as `g` or `rg` set it (or `cs` named): written as `rg` of the colour it
    /// shows as, which draws the same.
    pub(crate) fn plain(&self) -> bool {
        match &self.space {
            Some(Object::Name(n)) => matches!(&n[..], b"DeviceGray" | b"DeviceRGB"),
            Some(_) => false,
            None => self
                .set
                .as_ref()
                .is_some_and(|(op, _)| op == "g" || op == "rg"),
        }
    }

    /// Whether two paints draw the same, those written as `rg` judged by the colour they
    /// show as (compared apart).
    pub(crate) fn same(&self, other: &Paint) -> bool {
        (self.plain() && other.plain()) || self == other
    }

    /// The operators that set this colour.
    pub(crate) fn ops(&self) -> Vec<Operation> {
        let space = self
            .space
            .iter()
            .map(|n| Operation::new("cs", vec![n.clone()]));
        let set = (self.set.iter()).map(|(op, operands)| Operation::new(op, operands.clone()));
        space.chain(set).collect()
    }
}

/// A colour space, read from a page's resources ([`Space::named`]).
#[derive(Debug)]
pub(crate) enum Space {
    Gray,
    Rgb,
    Cmyk,
    Lab {
        /// a* and b*'s ranges.
        range: [f32; 4],
    },
    Indexed {
        base: Arc<Space>,
        /// The base space's components of each colour, as bytes.
        table: Vec<u8>,
        last: usize,
    },
    /// Separation and DeviceN: tints of `n` inks, shown through the tint transform in the
    /// alternate space; never drawn when the ink is `None`.
    Inks {
        n: usize,
        alt: Arc<Space>,
        tint: Function,
        none: bool,
    },
    /// Patterns, whose colour isn't one; and spaces that can't be read.
    Other,
}

impl Space {
    /// The space `name` (the operand of `cs`) names, with `spaces` the page's
    /// `/ColorSpace` resources.
    pub(crate) fn named(doc: &Document, spaces: &Dictionary, name: &[u8]) -> Space {
        match name {
            b"DeviceGray" | b"G" => Space::Gray,
            b"DeviceRGB" | b"RGB" => Space::Rgb,
            b"DeviceCMYK" | b"CMYK" => Space::Cmyk,
            _ => match spaces.get(name) {
                Ok(o) => Space::of(doc, o, 0),
                Err(_) => Space::Other,
            },
        }
    }

    fn of(doc: &Document, o: &Object, depth: usize) -> Space {
        let Ok((_, o)) = doc.dereference(o) else {
            return Space::Other;
        };
        if depth > MAX_NESTING {
            return Space::Other;
        }
        let a = match o {
            Object::Name(n) => return Space::named(doc, &Dictionary::new(), n),
            Object::Array(a) => a,
            _ => return Space::Other,
        };
        let at = |i: usize| a.get(i).and_then(|o| doc.dereference(o).ok()).map(|x| x.1);
        let inner = |i: usize| at(i).map_or(Space::Other, |o| Space::of(doc, o, depth + 1));
        let Some(Ok(head)) = a.first().map(Object::as_name) else {
            return Space::Other;
        };
        match head {
            b"DeviceGray" | b"CalGray" => Space::Gray,
            b"DeviceRGB" | b"CalRGB" => Space::Rgb,
            b"DeviceCMYK" => Space::Cmyk,
            b"Lab" => {
                let range = (at(1).and_then(|d| d.as_dict().ok()))
                    .and_then(|d| numbers(doc, d.get(b"Range").ok()?))
                    .filter(|r| r.len() == 4);
                Space::Lab {
                    range: range
                        .map_or([-100.0, 100.0, -100.0, 100.0], |r| [r[0], r[1], r[2], r[3]]),
                }
            }
            b"ICCBased" => {
                let Some(Ok(s)) = at(1).map(Object::as_stream) else {
                    return Space::Other;
                };
                match s.dict.get(b"N").ok().and_then(|n| n.as_i64().ok()) {
                    Some(1) => Space::Gray,
                    Some(3) => Space::Rgb,
                    Some(4) => Space::Cmyk,
                    _ => (s.dict.get(b"Alternate").ok())
                        .map_or(Space::Other, |o| Space::of(doc, o, depth + 1)),
                }
            }
            b"Indexed" | b"I" => {
                let base = Arc::new(inner(1));
                let last = at(2)
                    .and_then(|o| o.as_i64().ok())
                    .unwrap_or(0)
                    .clamp(0, 255);
                let table = match at(3) {
                    Some(Object::String(b, _)) => b.clone(),
                    Some(Object::Stream(s)) => s
                        .get_plain_content_with_limit(crate::MAX_CONTENT)
                        .unwrap_or_default(),
                    _ => Vec::new(),
                };
                Space::Indexed {
                    base,
                    table,
                    last: last as usize,
                }
            }
            b"Separation" | b"DeviceN" => {
                let (n, none) = match at(1) {
                    Some(Object::Name(ink)) => (1, &ink[..] == b"None"),
                    Some(Object::Array(inks)) => {
                        let none = |o: &Object| o.as_name().ok() == Some(b"None");
                        (inks.len(), !inks.is_empty() && inks.iter().all(none))
                    }
                    _ => return Space::Other,
                };
                let Some(tint) = at(3).and_then(|f| Function::of(doc, f, depth + 1)) else {
                    return Space::Other;
                };
                Space::Inks {
                    n,
                    alt: Arc::new(inner(2)),
                    tint,
                    none,
                }
            }
            _ => Space::Other,
        }
    }

    /// Its values when `cs` chooses it: black, or full ink.
    pub(crate) fn first(&self) -> Vec<f32> {
        match self {
            Space::Gray | Space::Indexed { .. } => vec![0.0],
            Space::Rgb | Space::Lab { .. } => vec![0.0; 3],
            Space::Cmyk => vec![0.0, 0.0, 0.0, 1.0],
            Space::Inks { n, .. } => vec![1.0; *n],
            Space::Other => Vec::new(),
        }
    }

    /// The colour `values` in this space show as, in RGB; `None` for a pattern, an ink
    /// that is never drawn, or values the space can't take.
    pub(crate) fn rgb(&self, values: &[f32]) -> Option<[f32; 3]> {
        let v = values;
        let unit = |x: f32| x.clamp(0.0, 1.0);
        match self {
            Space::Gray => v.first().map(|&g| [unit(g); 3]),
            Space::Rgb => (v.len() >= 3).then(|| [unit(v[0]), unit(v[1]), unit(v[2])]),
            Space::Cmyk => (v.len() >= 4).then(|| {
                let k = 1.0 - unit(v[3]);
                let c = |x: f32| (1.0 - unit(x)) * k;
                [c(v[0]), c(v[1]), c(v[2])]
            }),
            Space::Lab { range } => (v.len() >= 3).then(|| {
                let l = v[0].clamp(0.0, 100.0);
                let a = v[1].clamp(range[0], range[1]);
                let b = v[2].clamp(range[2], range[3]);
                lab_rgb(l, a, b)
            }),
            Space::Indexed { base, table, last } => {
                let i = (v.first()?.round().max(0.0) as usize).min(*last);
                let n = base.first().len();
                let bytes = table.get(i * n..(i + 1) * n)?;
                let values: Vec<f32> = match **base {
                    // Lab's bytes span its ranges.
                    Space::Lab { range } => {
                        let span = [(0.0, 100.0), (range[0], range[1]), (range[2], range[3])];
                        (bytes.iter().zip(span))
                            .map(|(&b, (lo, hi))| lo + b as f32 / 255.0 * (hi - lo))
                            .collect()
                    }
                    _ => bytes.iter().map(|&b| b as f32 / 255.0).collect(),
                };
                base.rgb(&values)
            }
            Space::Inks { n, alt, tint, none } => {
                if *none || v.len() < *n {
                    return None;
                }
                let tints: Vec<f32> = v[..*n].iter().map(|&x| unit(x)).collect();
                alt.rgb(&tint.eval(&tints)?)
            }
            Space::Other => None,
        }
    }
}

/// CIE L*a*b* shown in sRGB, the space's white taken as sRGB's.
fn lab_rgb(l: f32, a: f32, b: f32) -> [f32; 3] {
    let fy = (l + 16.0) / 116.0;
    let (fx, fz) = (fy + a / 500.0, fy - b / 200.0);
    let back = |t: f32| {
        const D: f32 = 6.0 / 29.0;
        if t > D {
            t * t * t
        } else {
            3.0 * D * D * (t - 4.0 / 29.0)
        }
    };
    let (x, y, z) = (0.9505 * back(fx), back(fy), 1.089 * back(fz));
    let linear = [
        3.2406 * x - 1.5372 * y - 0.4986 * z,
        -0.9689 * x + 1.8758 * y + 0.0415 * z,
        0.0557 * x - 0.2040 * y + 1.0570 * z,
    ];
    linear.map(|c| {
        let c = c.clamp(0.0, 1.0);
        if c <= 0.003_130_8 {
            12.92 * c
        } else {
            1.055 * c.powf(1.0 / 2.4) - 0.055
        }
    })
}

/// A PDF function (PDF 2.0, section 7.10), as a tint transform uses one.
#[derive(Debug)]
pub(crate) enum Function {
    /// Type 0: a table of samples, read between them.
    Sampled {
        size: Vec<usize>,
        bits: usize,
        encode: Vec<f32>,
        decode: Vec<f32>,
        outputs: usize,
        data: Vec<u8>,
        domain: Vec<f32>,
        range: Vec<f32>,
    },
    /// Type 2: `c0 + x^n (c1 - c0)`.
    Exponential {
        c0: Vec<f32>,
        c1: Vec<f32>,
        n: f32,
        domain: Vec<f32>,
        range: Vec<f32>,
    },
    /// Type 3: the domain in parts, each read by a function of its own.
    Stitching {
        parts: Vec<Function>,
        bounds: Vec<f32>,
        encode: Vec<f32>,
        domain: Vec<f32>,
        range: Vec<f32>,
    },
    /// Type 4: a program in PDF's PostScript calculator language.
    Calculator {
        program: Vec<Ps>,
        domain: Vec<f32>,
        range: Vec<f32>,
    },
}

/// The numbers of an array object, references followed.
fn numbers(doc: &Document, o: &Object) -> Option<Vec<f32>> {
    let (_, o) = doc.dereference(o).ok()?;
    (o.as_array().ok()?.iter())
        .map(|x| doc.dereference(x).ok()?.1.as_float().ok())
        .collect()
}

/// `x` from the range `a..b` to the same place in `c..d`.
fn between(x: f32, a: f32, b: f32, c: f32, d: f32) -> f32 {
    if (b - a).abs() < f32::EPSILON {
        c
    } else {
        c + (x - a) * (d - c) / (b - a)
    }
}

/// `values` each clipped to its pair of `limits`, where there is one.
fn clip(values: &mut [f32], limits: &[f32]) {
    for (v, [lo, hi]) in values.iter_mut().zip(limits.as_chunks::<2>().0) {
        *v = v.clamp(lo.min(*hi), hi.max(*lo));
    }
}

impl Function {
    fn of(doc: &Document, o: &Object, depth: usize) -> Option<Function> {
        if depth > MAX_NESTING {
            return None;
        }
        let (_, o) = doc.dereference(o).ok()?;
        let (dict, stream) = match o {
            Object::Dictionary(d) => (d, None),
            Object::Stream(s) => (&s.dict, Some(s)),
            _ => return None,
        };
        let get = |k: &[u8]| numbers(doc, dict.get(k).ok()?);
        let domain = get(b"Domain").filter(|d| d.len() >= 2 && d.len() % 2 == 0)?;
        let range = get(b"Range").unwrap_or_default();
        let int = |k: &[u8]| {
            let (_, o) = doc.dereference(dict.get(k).ok()?).ok()?;
            o.as_i64().ok()
        };
        Some(match int(b"FunctionType")? {
            0 => {
                let size: Vec<usize> = get(b"Size")?.iter().map(|&s| s.max(1.0) as usize).collect();
                let inputs = domain.len() / 2;
                let outputs = range.len() / 2;
                if size.len() != inputs || inputs == 0 || inputs > MAX_INPUTS || outputs == 0 {
                    return None;
                }
                let bits = int(b"BitsPerSample")?;
                if ![1, 2, 4, 8, 12, 16, 24, 32].contains(&bits) {
                    return None;
                }
                let encode = get(b"Encode")
                    .filter(|e| e.len() == 2 * inputs)
                    .unwrap_or_else(|| size.iter().flat_map(|&s| [0.0, (s - 1) as f32]).collect());
                let decode = get(b"Decode")
                    .filter(|d| d.len() == range.len())
                    .unwrap_or_else(|| range.clone());
                let data = stream?
                    .get_plain_content_with_limit(crate::MAX_CONTENT)
                    .ok()?;
                Function::Sampled {
                    size,
                    bits: bits as usize,
                    encode,
                    decode,
                    outputs,
                    data,
                    domain,
                    range,
                }
            }
            2 => {
                let c0 = get(b"C0").unwrap_or_else(|| vec![0.0]);
                let c1 = get(b"C1").unwrap_or_else(|| vec![1.0]);
                let n = (dict.get(b"N").ok())
                    .and_then(|o| doc.dereference(o).ok()?.1.as_float().ok())?;
                if c0.len() != c1.len() {
                    return None;
                }
                Function::Exponential {
                    c0,
                    c1,
                    n,
                    domain,
                    range,
                }
            }
            3 => {
                let (_, fs) = doc.dereference(dict.get(b"Functions").ok()?).ok()?;
                let parts: Vec<Function> = (fs.as_array().ok()?.iter())
                    .map(|f| Function::of(doc, f, depth + 1))
                    .collect::<Option<_>>()?;
                let bounds = get(b"Bounds").unwrap_or_default();
                let encode = get(b"Encode")?;
                if parts.is_empty()
                    || bounds.len() + 1 != parts.len()
                    || encode.len() != 2 * parts.len()
                {
                    return None;
                }
                Function::Stitching {
                    parts,
                    bounds,
                    encode,
                    domain,
                    range,
                }
            }
            4 => {
                let code = stream?
                    .get_plain_content_with_limit(crate::MAX_CONTENT)
                    .ok()?;
                Function::Calculator {
                    program: Ps::program(&code)?,
                    domain,
                    range,
                }
            }
            _ => return None,
        })
    }

    /// The function's outputs for `input`, or `None` when it can't give them.
    pub(crate) fn eval(&self, input: &[f32]) -> Option<Vec<f32>> {
        match self {
            Function::Sampled {
                size,
                bits,
                encode,
                decode,
                outputs,
                data,
                domain,
                range,
            } => {
                let m = size.len();
                if input.len() < m {
                    return None;
                }
                // Where the input falls in the table, input by input: the sample below it
                // and how far towards the next.
                let mut at: Vec<(usize, f32)> = Vec::with_capacity(m);
                for i in 0..m {
                    let (d0, d1) = (domain[2 * i], domain[2 * i + 1]);
                    let x = input[i].clamp(d0.min(d1), d1.max(d0));
                    let e = between(x, d0, d1, encode[2 * i], encode[2 * i + 1]);
                    let e = e.clamp(0.0, (size[i] - 1) as f32);
                    let lo = (e.floor() as usize).min(size[i] - 1);
                    at.push((lo, e - lo as f32));
                }
                let max = ((1u64 << bits) - 1) as f32;
                let mut out = vec![0.0f32; *outputs];
                // Each corner of the cell the input is in, weighted by its nearness.
                for corner in 0..1usize << m {
                    let (mut index, mut stride, mut weight) = (0usize, 1usize, 1.0f32);
                    for (i, &(lo, f)) in at.iter().enumerate() {
                        let up = corner >> i & 1 == 1;
                        let k = if up { (lo + 1).min(size[i] - 1) } else { lo };
                        weight *= if up { f } else { 1.0 - f };
                        index = index.checked_add(k.checked_mul(stride)?)?;
                        stride = stride.checked_mul(size[i])?;
                    }
                    if weight == 0.0 {
                        continue;
                    }
                    for (j, o) in out.iter_mut().enumerate() {
                        let n = index.checked_mul(*outputs)?.checked_add(j)?;
                        *o += weight * sample(data, n, *bits)? as f32;
                    }
                }
                for (j, o) in out.iter_mut().enumerate() {
                    *o = between(*o, 0.0, max, decode[2 * j], decode[2 * j + 1]);
                }
                clip(&mut out, range);
                Some(out)
            }
            Function::Exponential {
                c0,
                c1,
                n,
                domain,
                range,
            } => {
                let mut x = vec![*input.first()?];
                clip(&mut x, domain);
                let p = x[0].powf(*n);
                let mut out: Vec<f32> = c0.iter().zip(c1).map(|(a, b)| a + p * (b - a)).collect();
                clip(&mut out, range);
                out.iter().all(|v| v.is_finite()).then_some(out)
            }
            Function::Stitching {
                parts,
                bounds,
                encode,
                domain,
                range,
            } => {
                let mut x = vec![*input.first()?];
                clip(&mut x, domain);
                let x = x[0];
                let k = bounds.iter().take_while(|&&b| x >= b).count();
                let lo = if k == 0 { domain[0] } else { bounds[k - 1] };
                let hi = bounds.get(k).copied().unwrap_or(domain[1]);
                let e = between(x, lo, hi, encode[2 * k], encode[2 * k + 1]);
                let mut out = parts[k].eval(&[e])?;
                clip(&mut out, range);
                Some(out)
            }
            Function::Calculator {
                program,
                domain,
                range,
            } => {
                let mut x = input.to_vec();
                clip(&mut x, domain);
                let mut stack: Vec<Value> = x.iter().map(|&v| Value::Num(v as f64)).collect();
                Ps::run(program, &mut stack)?;
                let n = range.len() / 2;
                if stack.len() < n {
                    return None;
                }
                let mut out = (stack[stack.len() - n..].iter())
                    .map(|v| v.num().map(|x| x as f32))
                    .collect::<Option<Vec<f32>>>()?;
                clip(&mut out, range);
                Some(out)
            }
        }
    }
}

/// Sample `n` of a table of `bits`-bit samples, read big-endian.
fn sample(data: &[u8], n: usize, bits: usize) -> Option<u32> {
    let start = n.checked_mul(bits)?;
    let mut v: u32 = 0;
    for b in start..start + bits {
        let byte = *data.get(b / 8)?;
        v = v << 1 | u32::from(byte >> (7 - b % 8) & 1);
    }
    Some(v)
}

/// A value on a calculator function's stack.
#[derive(Clone, Copy, Debug)]
enum Value {
    Num(f64),
    Bool(bool),
}

impl Value {
    fn num(self) -> Option<f64> {
        match self {
            Value::Num(x) => Some(x),
            Value::Bool(_) => None,
        }
    }
}

/// A step of a calculator function's program (PDF 2.0, section 7.10.5).
#[derive(Clone, Debug)]
pub(crate) enum Ps {
    Num(f64),
    Op(String),
    If(Vec<Ps>),
    IfElse(Vec<Ps>, Vec<Ps>),
}

impl Ps {
    /// The program `{ ... }` in `code`.
    fn program(code: &[u8]) -> Option<Vec<Ps>> {
        let text = std::str::from_utf8(code).ok()?;
        let spaced = text.replace('{', " { ").replace('}', " } ");
        let mut words = spaced.split_ascii_whitespace();
        if words.next()? != "{" {
            return None;
        }
        Ps::block(&mut words, 0)
    }

    /// The steps up to the `}` that closes a block, the `{` read.
    fn block<'a>(words: &mut impl Iterator<Item = &'a str>, depth: usize) -> Option<Vec<Ps>> {
        if depth > MAX_STACK {
            return None;
        }
        let mut steps: Vec<Ps> = Vec::new();
        // Blocks read and not yet taken: one by the `if` straight after it, two by an
        // `ifelse`.
        let mut blocks: Vec<Vec<Ps>> = Vec::new();
        loop {
            match words.next()? {
                "{" if blocks.len() < 2 => blocks.push(Ps::block(words, depth + 1)?),
                "if" if blocks.len() == 1 => steps.push(Ps::If(blocks.pop()?)),
                "ifelse" if blocks.len() == 2 => {
                    let otherwise = blocks.pop()?;
                    let then = blocks.pop()?;
                    steps.push(Ps::IfElse(then, otherwise));
                }
                _ if !blocks.is_empty() => return None,
                "}" => return Some(steps),
                w => match w.parse::<f64>() {
                    Ok(x) => steps.push(Ps::Num(x)),
                    Err(_) => steps.push(Ps::Op(w.to_string())),
                },
            }
        }
    }

    /// Run `program` on `stack`; `None` on an error.
    fn run(program: &[Ps], stack: &mut Vec<Value>) -> Option<()> {
        for step in program {
            match step {
                Ps::Num(x) => stack.push(Value::Num(*x)),
                Ps::If(then) => {
                    if let Value::Bool(true) = stack.pop()? {
                        Ps::run(then, stack)?;
                    }
                }
                Ps::IfElse(then, otherwise) => match stack.pop()? {
                    Value::Bool(true) => Ps::run(then, stack)?,
                    Value::Bool(false) => Ps::run(otherwise, stack)?,
                    Value::Num(_) => return None,
                },
                Ps::Op(op) => Ps::op(op, stack)?,
            }
            if stack.len() > MAX_STACK {
                return None;
            }
        }
        Some(())
    }

    fn op(op: &str, stack: &mut Vec<Value>) -> Option<()> {
        let num = |stack: &mut Vec<Value>| stack.pop()?.num();
        let int = |x: f64| x.trunc() as i64;
        let push =
            |stack: &mut Vec<Value>, x: f64| x.is_finite().then(|| stack.push(Value::Num(x)));
        match op {
            "abs" | "ceiling" | "cos" | "cvi" | "cvr" | "floor" | "ln" | "log" | "neg"
            | "round" | "sin" | "sqrt" | "truncate" => {
                let x = num(stack)?;
                let y = match op {
                    "abs" => x.abs(),
                    "ceiling" => x.ceil(),
                    "cos" => x.to_radians().cos(),
                    "cvi" | "truncate" => x.trunc(),
                    "cvr" => x,
                    "floor" => x.floor(),
                    "ln" => x.ln(),
                    "log" => x.log10(),
                    "neg" => -x,
                    "round" => (x + 0.5).floor(),
                    "sin" => x.to_radians().sin(),
                    _ => x.sqrt(),
                };
                push(stack, y)?;
            }
            "add" | "sub" | "mul" | "div" | "idiv" | "mod" | "exp" | "atan" | "bitshift" => {
                let b = num(stack)?;
                let a = num(stack)?;
                let y = match op {
                    "add" => a + b,
                    "sub" => a - b,
                    "mul" => a * b,
                    "div" => a / b,
                    "idiv" => int(a).checked_div(int(b))? as f64,
                    "mod" => int(a).checked_rem(int(b))? as f64,
                    "exp" => a.powf(b),
                    "atan" => {
                        let d = a.atan2(b).to_degrees();
                        if d < 0.0 {
                            d + 360.0
                        } else {
                            d
                        }
                    }
                    _ => {
                        let (a, s) = (int(a), int(b));
                        match s >= 0 {
                            true => a.checked_shl(s.min(63) as u32)? as f64,
                            false => a.checked_shr((-s).min(63) as u32)? as f64,
                        }
                    }
                };
                push(stack, y)?;
            }
            "eq" | "ne" | "gt" | "ge" | "lt" | "le" => {
                let b = stack.pop()?;
                let a = stack.pop()?;
                let y = match (a, b) {
                    (Value::Num(a), Value::Num(b)) => match op {
                        "eq" => a == b,
                        "ne" => a != b,
                        "gt" => a > b,
                        "ge" => a >= b,
                        "lt" => a < b,
                        _ => a <= b,
                    },
                    (Value::Bool(a), Value::Bool(b)) if op == "eq" || op == "ne" => {
                        (a == b) == (op == "eq")
                    }
                    _ => return None,
                };
                stack.push(Value::Bool(y));
            }
            "and" | "or" | "xor" => {
                let b = stack.pop()?;
                let a = stack.pop()?;
                stack.push(match (a, b) {
                    (Value::Bool(a), Value::Bool(b)) => Value::Bool(match op {
                        "and" => a && b,
                        "or" => a || b,
                        _ => a ^ b,
                    }),
                    (Value::Num(a), Value::Num(b)) => {
                        let (a, b) = (int(a), int(b));
                        Value::Num(match op {
                            "and" => a & b,
                            "or" => a | b,
                            _ => a ^ b,
                        } as f64)
                    }
                    _ => return None,
                });
            }
            "not" => {
                let v = match stack.pop()? {
                    Value::Bool(b) => Value::Bool(!b),
                    Value::Num(x) => Value::Num(!int(x) as f64),
                };
                stack.push(v);
            }
            "true" => stack.push(Value::Bool(true)),
            "false" => stack.push(Value::Bool(false)),
            "pop" => {
                stack.pop()?;
            }
            "dup" => stack.push(*stack.last()?),
            "exch" => {
                let n = stack.len();
                if n < 2 {
                    return None;
                }
                stack.swap(n - 1, n - 2);
            }
            "copy" => {
                let n = usize::try_from(int(num(stack)?)).ok()?;
                let from = stack.len().checked_sub(n)?;
                stack.extend_from_within(from..);
            }
            "index" => {
                let n = usize::try_from(int(num(stack)?)).ok()?;
                let i = stack.len().checked_sub(n.checked_add(1)?)?;
                stack.push(stack[i]);
            }
            "roll" => {
                let j = int(num(stack)?);
                let n = usize::try_from(int(num(stack)?)).ok()?;
                let from = stack.len().checked_sub(n)?;
                if n > 0 {
                    let by = j.rem_euclid(n as i64) as usize;
                    stack[from..].rotate_right(by);
                }
            }
            _ => return None,
        }
        Some(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use lopdf::{dictionary, Stream};

    fn near(a: &[f32], b: &[f32]) -> bool {
        a.len() == b.len() && a.iter().zip(b).all(|(x, y)| (x - y).abs() < 0.01)
    }

    fn function(doc: &mut Document, o: Object) -> Function {
        let id = doc.add_object(o);
        Function::of(doc, &Object::Reference(id), 0).expect("a function")
    }

    #[test]
    fn exponential_and_stitching_functions() {
        let mut doc = Document::with_version("1.7");
        let red = dictionary! {
            "FunctionType" => 2, "Domain" => vec![0.into(), 1.into()],
            "C0" => vec![0.into(), 0.into(), 0.into(), 0.into()],
            "C1" => vec![0.into(), 1.into(), 0.8.into(), 0.into()], "N" => 1,
        };
        let f = function(&mut doc, red.clone().into());
        assert!(near(&f.eval(&[0.5]).unwrap(), &[0.0, 0.5, 0.4, 0.0]));
        assert!(near(&f.eval(&[3.0]).unwrap(), &[0.0, 1.0, 0.8, 0.0]));
        let half = dictionary! {
            "FunctionType" => 2, "Domain" => vec![0.into(), 1.into()],
            "C0" => vec![1.into()], "C1" => vec![0.into()], "N" => 2,
        };
        let f = function(&mut doc, half.into());
        assert!(near(&f.eval(&[0.5]).unwrap(), &[0.75]));
        let a = doc.add_object(dictionary! {
            "FunctionType" => 2, "Domain" => vec![0.into(), 1.into()],
            "C0" => vec![0.into()], "C1" => vec![1.into()], "N" => 1,
        });
        let stitched = dictionary! {
            "FunctionType" => 3, "Domain" => vec![0.into(), 1.into()],
            "Functions" => vec![a.into(), a.into()], "Bounds" => vec![0.5.into()],
            "Encode" => vec![0.into(), 1.into(), 1.into(), 0.into()],
        };
        let f = function(&mut doc, stitched.into());
        assert!(near(&f.eval(&[0.25]).unwrap(), &[0.5]));
        assert!(near(&f.eval(&[0.75]).unwrap(), &[0.5]));
        assert!(near(&f.eval(&[1.0]).unwrap(), &[0.0]));
    }

    #[test]
    fn sampled_functions_read_between_samples() {
        let mut doc = Document::with_version("1.7");
        // One input, two outputs of 8 bits: from (0, 255) to (255, 0) in three samples.
        let s = Stream::new(
            dictionary! {
                "FunctionType" => 0, "Domain" => vec![0.into(), 1.into()],
                "Range" => vec![0.into(), 1.into(), 0.into(), 1.into()],
                "Size" => vec![3.into()], "BitsPerSample" => 8,
            },
            vec![0, 255, 255, 255, 255, 0],
        );
        let f = function(&mut doc, s.into());
        assert!(near(&f.eval(&[0.0]).unwrap(), &[0.0, 1.0]));
        assert!(near(&f.eval(&[0.25]).unwrap(), &[0.5, 1.0]));
        assert!(near(&f.eval(&[1.0]).unwrap(), &[1.0, 0.0]));
        // Two inputs of 4 bits each sample, one output: the corners 0, 15, 15, 0.
        let s = Stream::new(
            dictionary! {
                "FunctionType" => 0,
                "Domain" => vec![0.into(), 1.into(), 0.into(), 1.into()],
                "Range" => vec![0.into(), 1.into()],
                "Size" => vec![2.into(), 2.into()], "BitsPerSample" => 4,
            },
            vec![0x0f, 0xf0],
        );
        let f = function(&mut doc, s.into());
        assert!(near(&f.eval(&[1.0, 0.0]).unwrap(), &[1.0]));
        assert!(near(&f.eval(&[0.5, 0.5]).unwrap(), &[0.5]));
        // Too few samples for the table: no value, not a guess.
        let s = Stream::new(
            dictionary! {
                "FunctionType" => 0, "Domain" => vec![0.into(), 1.into()],
                "Range" => vec![0.into(), 1.into()], "Size" => vec![300.into()],
                "BitsPerSample" => 8,
            },
            vec![1, 2],
        );
        assert_eq!(function(&mut doc, s.into()).eval(&[1.0]), None);
    }

    #[test]
    fn calculator_functions_run_their_programs() {
        let mut doc = Document::with_version("1.7");
        let mut calc = |code: &str, outputs: usize| {
            let range: Vec<Object> = (0..outputs).flat_map(|_| [0.into(), 1.into()]).collect();
            let s = Stream::new(
                dictionary! {
                    "FunctionType" => 4, "Domain" => vec![0.into(), 1.into()], "Range" => range,
                },
                code.as_bytes().to_vec(),
            );
            function(&mut doc, s.into())
        };
        // A spot colour's tint to CMYK, as design programs write them.
        let f = calc("{dup 0 mul exch dup 0.84 mul exch 0.5 mul 0}", 4);
        assert!(near(&f.eval(&[1.0]).unwrap(), &[0.0, 0.84, 0.5, 0.0]));
        assert!(near(&f.eval(&[0.5]).unwrap(), &[0.0, 0.42, 0.25, 0.0]));
        let f = calc("{ dup 0.5 gt { pop 1 } { 2 mul } ifelse }", 1);
        assert!(near(&f.eval(&[0.25]).unwrap(), &[0.5]));
        assert!(near(&f.eval(&[0.75]).unwrap(), &[1.0]));
        let f = calc("{ 3 1 roll pop pop }", 1);
        assert_eq!(f.eval(&[0.5]), None, "too few values to roll");
        let f = calc("{ 1 2 3 3 1 roll pop pop }", 2);
        assert!(near(&f.eval(&[0.5]).unwrap(), &[0.5, 1.0]));
        let f = calc("{ pop pop }", 1);
        assert_eq!(f.eval(&[0.5]), None);
        assert!(Ps::program(b"{ 1 { 2 } }").is_none(), "a block no if takes");
    }

    #[test]
    fn spaces_show_their_colours() {
        let mut doc = Document::with_version("1.7");
        let tint = doc.add_object(dictionary! {
            "FunctionType" => 2, "Domain" => vec![0.into(), 1.into()],
            "C0" => vec![0.into(), 0.into(), 0.into(), 0.into()],
            "C1" => vec![0.into(), 1.into(), 1.into(), 0.into()], "N" => 1,
        });
        let red = Object::Array(vec![
            "Separation".into(),
            Object::Name(b"PANTONE Red".to_vec()),
            "DeviceCMYK".into(),
            tint.into(),
        ]);
        let none = Object::Array(vec![
            "Separation".into(),
            "None".into(),
            "DeviceCMYK".into(),
            tint.into(),
        ]);
        let table = Object::string_literal(vec![255, 255, 255, 0, 0, 255]);
        let indexed = Object::Array(vec!["Indexed".into(), "DeviceRGB".into(), 1.into(), table]);
        let lab = Object::Array(vec![
            "Lab".into(),
            dictionary! {
                "WhitePoint" => vec![0.9505.into(), 1.into(), 1.089.into()],
            }
            .into(),
        ]);
        let spaces = dictionary! {
            "CS0" => red, "CS1" => none, "CS2" => indexed, "CS3" => lab,
        };
        let space = |n: &str| Space::named(&doc, &spaces, n.as_bytes());
        let full = space("CS0");
        assert_eq!(full.first(), [1.0]);
        assert!(near(&full.rgb(&full.first()).unwrap(), &[1.0, 0.0, 0.0]));
        assert!(near(&full.rgb(&[0.0]).unwrap(), &[1.0, 1.0, 1.0]));
        assert_eq!(space("CS1").rgb(&[1.0]), None, "never drawn");
        assert!(near(&space("CS2").rgb(&[1.0]).unwrap(), &[0.0, 0.0, 1.0]));
        assert!(near(
            &space("CS3").rgb(&[100.0, 0.0, 0.0]).unwrap(),
            &[1.0, 1.0, 1.0]
        ));
        assert!(near(
            &space("CS3").rgb(&[0.0, 0.0, 0.0]).unwrap(),
            &[0.0, 0.0, 0.0]
        ));
        assert!(near(
            &space("DeviceCMYK").rgb(&[0.0, 0.0, 0.0, 1.0]).unwrap(),
            &[0.0; 3]
        ));
        assert!(matches!(space("Missing"), Space::Other));
    }

    #[test]
    fn paints_write_back_what_set_them() {
        let cs = Paint::space(&Object::Name(b"CS0".to_vec()));
        let ink = cs.with("scn", &[Object::Real(0.5)]);
        let ops: Vec<String> = ink.ops().iter().map(|o| o.operator.clone()).collect();
        assert_eq!(ops, ["cs", "scn"]);
        assert!(!ink.plain());
        // `sc` after `rg` is the device colour it sets.
        let rgb = Paint::device("rg", &[0.into(), 0.into(), 0.into()]);
        let after = rgb.with("sc", &[1.into(), 0.into(), 0.into()]);
        assert_eq!(after, Paint::device("rg", &[1.into(), 0.into(), 0.into()]));
        assert!(after.plain() && after.same(&Paint::default()));
        assert!(!Paint::device("k", &[0.into(), 0.into(), 0.into(), 1.into()]).plain());
    }
}

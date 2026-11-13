//! Shapes filled with a shading made of triangles or patches (a mesh: shading types 4 to 7),
//! drawn as the shading itself (PB-11). To fill a shape with a shading pattern, hayro 0.9
//! samples the shading into a picture of all the shape covers on the pass, mesh or not, and
//! holds the picture until the pass ends; `sh` samples only what the mesh covers. A figure
//! converted from PostScript can paint each triangle of a shading as a pattern filling the
//! whole figure: pdf.js's bug1260585, 2,640 of them, took 6 GB and 2 minutes at 288 dpi. In
//! the copies pages are drawn from, such a fill is drawn as the PDF reference describes it:
//! the shape set as the clip, then the shading painted with `sh` in the pattern's space. A
//! shading with a background, which `sh` leaves out, and a pattern with a graphics state of
//! its own are left as they are, and so are fills in Type 3 glyphs and in drawings without
//! resources of their own.

use crate::matrix::{self as mx, M};
use crate::unfillable::{inherited_at, int, numbers, MAX_CONTENT};
use lopdf::content::{Content, Operation};
use lopdf::{Dictionary, Document, Object, ObjectId, Stream};
use std::collections::btree_map::Entry;
use std::collections::{BTreeMap, BTreeSet, HashMap};

/// A mesh pattern a drawing's resources name: its shading, the name the shading is given
/// among the drawing's shadings, and the pattern's matrix.
struct Mesh {
    shading: ObjectId,
    name: Vec<u8>,
    matrix: M,
}

/// Where shadings are added to a drawing's resources: the object holding them, and the keys
/// from it down to the shadings, made when missing.
type Target = (ObjectId, Vec<&'static [u8]>);

/// The shadings to add: where, and each one's name and object.
type Adds = Vec<(Target, Vec<(Vec<u8>, ObjectId)>)>;

/// Draw every shape filled with a mesh pattern in `doc`'s pages, forms, annotations'
/// appearances and tiling patterns' cells as `sh`, in `swaps`: what is put in the copy of
/// `doc`, by object, where the drawings are read from first. A drawing sharing a stream
/// with another is left as it is: the stream is drawn with the resources of each.
pub(crate) fn draw_as_shadings(doc: &Document, swaps: &mut BTreeMap<ObjectId, Object>) {
    let shading = |o: &Object| o.as_stream().is_ok_and(|s| is_mesh(doc, &s.dict));
    if !doc.objects.values().any(shading) {
        return;
    }
    // Each drawing: the object holding its resources, the resources, its streams.
    let mut drawings = Vec::new();
    let mut others = BTreeSet::new();
    for page in doc.get_pages().into_values() {
        if let Some((holder, res)) = inherited_at(doc, page, b"Resources") {
            drawings.push((holder, res.clone(), doc.get_page_contents(page)));
        }
        let annots = (doc.get_dictionary(page).ok())
            .and_then(|p| doc.dereference(p.get(b"Annots").ok()?).ok())
            .and_then(|(_, a)| a.as_array().ok());
        for a in annots.into_iter().flatten() {
            others.extend(appearances(doc, a));
        }
    }
    for (&id, o) in &doc.objects {
        let Ok(s) = o.as_stream() else { continue };
        let form = matches!(
            s.dict.get(b"Subtype").and_then(Object::as_name),
            Ok(b"Form")
        );
        if form || int(doc, &s.dict, b"PatternType") == Some(1) {
            others.insert(id);
        }
    }
    for id in others {
        let s = swaps.get(&id).or_else(|| doc.objects.get(&id));
        if let Some(res) = s.and_then(|s| s.as_stream().ok()?.dict.get(b"Resources").ok()) {
            drawings.push((id, res.clone(), vec![id]));
        }
    }
    let mut uses = HashMap::new();
    for &id in drawings.iter().flat_map(|(_, _, streams)| streams) {
        *uses.entry(id).or_insert(0) += 1;
    }
    let mut adds = Vec::new();
    for (holder, res, streams) in drawings {
        if streams.iter().all(|id| uses.get(id) == Some(&1)) {
            redraw(doc, swaps, holder, &res, &streams, &mut adds);
        }
    }
    for ((at, keys), names) in adds {
        let o = match swaps.entry(at) {
            Entry::Occupied(e) => e.into_mut(),
            Entry::Vacant(e) => match doc.objects.get(&at) {
                Some(o) => e.insert(o.clone()),
                None => continue,
            },
        };
        let d = match o {
            Object::Dictionary(d) => d,
            Object::Stream(s) => &mut s.dict,
            _ => continue,
        };
        if let Some(d) = under(d, &keys) {
            for (name, shading) in names {
                d.set(name, Object::Reference(shading));
            }
        }
    }
}

/// The dictionary `keys` lead to from `d`, those missing made.
fn under<'a>(mut d: &'a mut Dictionary, keys: &[&[u8]]) -> Option<&'a mut Dictionary> {
    for &key in keys {
        if !d.has(key) {
            d.set(key, Dictionary::new());
        }
        d = d.get_mut(key).ok()?.as_dict_mut().ok()?;
    }
    Some(d)
}

/// The streams annotation `a` draws as its normal appearance, each state's.
fn appearances(doc: &Document, a: &Object) -> Vec<ObjectId> {
    let ap = dict(doc, a).and_then(|a| dict(doc, a.get(b"AP").ok()?));
    let Some(normal) = ap.and_then(|ap| ap.get(b"N").ok()) else {
        return Vec::new();
    };
    let stream = |o: &Object| {
        let id = o.as_reference().ok()?;
        doc.get_object(id).ok()?.as_stream().ok().map(|_| id)
    };
    match (stream(normal), dict(doc, normal)) {
        (Some(id), _) => vec![id],
        (None, Some(states)) => states.iter().filter_map(|(_, s)| stream(s)).collect(),
        _ => Vec::new(),
    }
}

/// Dictionary `o`, or the one it refers to.
fn dict<'a>(doc: &'a Document, o: &'a Object) -> Option<&'a Dictionary> {
    doc.dereference(o).ok()?.1.as_dict().ok()
}

/// Redraw the drawing in `streams` (a page's, in order, or a form's or a cell's one), with
/// resources `res` found in object `holder`, in `swaps`, and note in `adds` the shadings its
/// resources are to be given, when it fills shapes with mesh patterns.
fn redraw(
    doc: &Document,
    swaps: &mut BTreeMap<ObjectId, Object>,
    holder: ObjectId,
    res: &Object,
    streams: &[ObjectId],
    adds: &mut Adds,
) {
    let Some(r) = dict(doc, res) else { return };
    let meshes = meshes(doc, r);
    let (Some(target), false) = (target(doc, holder, res), meshes.is_empty()) else {
        return;
    };
    let mut dicts = Vec::new();
    let mut data = Vec::new();
    for id in streams {
        let s = swaps.get(id).or_else(|| doc.objects.get(id));
        let Some(s) = s.and_then(|s| s.as_stream().ok()) else {
            return;
        };
        let Ok(d) = s.get_plain_content_with_limit(MAX_CONTENT.saturating_sub(data.len())) else {
            return;
        };
        if !data.is_empty() {
            data.push(b'\n');
        }
        data.extend(d);
        dicts.push(s.dict.clone());
    }
    let Ok((stripped, inline)) = crate::inline::extract(&data) else {
        return;
    };
    let Ok(content) = Content::decode(&stripped) else {
        return;
    };
    let Some(ops) = rewrite(content.operations, &meshes) else {
        return;
    };
    let Ok(mut bytes) = inline.encode(ops) else {
        return;
    };
    // The first stream draws it all, the others nothing.
    for (&id, mut dict) in streams.iter().zip(dicts) {
        dict.remove(b"Filter");
        dict.remove(b"DecodeParms");
        swaps.insert(
            id,
            Object::Stream(Stream::new(dict, std::mem::take(&mut bytes))),
        );
    }
    adds.push((
        target,
        meshes.into_values().map(|m| (m.name, m.shading)).collect(),
    ));
}

/// Where shadings are added to resources `res` found in object `holder`: in them, or in
/// their shadings when those are an object of their own.
fn target(doc: &Document, holder: ObjectId, res: &Object) -> Option<Target> {
    let (at, mut keys, d) = match res {
        Object::Reference(r) => (*r, Vec::new(), doc.get_dictionary(*r).ok()?),
        Object::Dictionary(d) => (holder, vec![&b"Resources"[..]], d),
        _ => return None,
    };
    match d.get(b"Shading") {
        Ok(Object::Reference(s)) => doc.get_dictionary(*s).is_ok().then(|| (*s, Vec::new())),
        Ok(Object::Dictionary(_)) | Err(_) => {
            keys.push(b"Shading");
            Some((at, keys))
        }
        Ok(_) => None,
    }
}

/// The mesh patterns resources `res` name, by name, each with the name its shading is given:
/// one no other shading there has.
fn meshes(doc: &Document, res: &Dictionary) -> HashMap<Vec<u8>, Mesh> {
    let Some(patterns) = res.get(b"Pattern").ok().and_then(|p| dict(doc, p)) else {
        return HashMap::new();
    };
    let shadings = res.get(b"Shading").ok().and_then(|s| dict(doc, s));
    let mut out = HashMap::new();
    for (key, o) in patterns.iter() {
        let Some((shading, matrix)) = mesh(doc, o) else {
            continue;
        };
        let name = format!("LimarePDFMesh{}_{}", shading.0, shading.1).into_bytes();
        let held = shadings.and_then(|s| s.get(&name).ok());
        if held.is_none_or(|o| o.as_reference().ok() == Some(shading)) {
            out.insert(
                key.clone(),
                Mesh {
                    shading,
                    name,
                    matrix,
                },
            );
        }
    }
    out
}

/// The shading and the matrix of pattern `o`, when hayro would sample it over all a shape
/// covers and `sh` can draw it the same: a shading pattern without a graphics state of its
/// own, its shading a mesh without a background.
fn mesh(doc: &Document, o: &Object) -> Option<(ObjectId, M)> {
    let p = doc.dereference(o).ok()?.1.as_dict().ok()?;
    if int(doc, p, b"PatternType") != Some(2) || p.has(b"ExtGState") {
        return None;
    }
    let shading = p.get(b"Shading").ok()?.as_reference().ok()?;
    let s = doc.get_object(shading).ok()?.as_stream().ok()?;
    if !is_mesh(doc, &s.dict) || s.dict.has(b"Background") {
        return None;
    }
    let m = p.get(b"Matrix").ok().and_then(|m| numbers(doc, m));
    let matrix = match m.as_deref() {
        Some(&[a, b, c, d, e, f]) => [a, b, c, d, e, f].map(|n| n as f32),
        _ => mx::ID,
    };
    Some((shading, matrix))
}

/// Whether shading `d` is a mesh: of triangles (types 4 and 5) or patches (6 and 7).
fn is_mesh(doc: &Document, d: &Dictionary) -> bool {
    int(doc, d, b"ShadingType").is_some_and(|t| (4..=7).contains(&t))
}

/// Content `ops` with every shape filled with a mesh pattern `meshes` name drawn as `sh`: in
/// a saved state, the shape set as the clip, the transform set to the pattern's space, which
/// is from the drawing's own as it began, and the shading painted. A shape outlined too is
/// outlined after, and one setting a clip sets it after. `None` when no shape is.
fn rewrite(ops: Vec<Operation>, meshes: &HashMap<Vec<u8>, Mesh>) -> Option<Vec<Operation>> {
    let mut out = Vec::with_capacity(ops.len());
    let mut any = false;
    // The transform since the drawing began and the mesh pattern filling shapes, as `q`
    // saves them.
    let mut g: (M, Option<&Mesh>) = (mx::ID, None);
    let mut saved = Vec::new();
    // The shape being built, with its clip, until it is painted; `None` once something else
    // came between.
    let mut shape = Some(Vec::new());
    for op in ops {
        let o = op.operator.as_str();
        if matches!(o, "m" | "l" | "c" | "v" | "y" | "h" | "re" | "W" | "W*") {
            match &mut shape {
                Some(s) => s.push(op),
                None => out.push(op),
            }
            continue;
        }
        if matches!(
            o,
            "S" | "s" | "f" | "F" | "f*" | "B" | "B*" | "b" | "b*" | "n"
        ) {
            let built = shape.replace(Vec::new()).unwrap_or_default();
            let fills = !matches!(o, "S" | "s" | "n") && !built.is_empty();
            let mesh = g.1.filter(|_| fills);
            // From the transform now to the pattern's space.
            let to = mesh.and_then(|m| Some(mx::mul(m.matrix, mx::inv(g.0)?)));
            match (mesh, to.filter(|&to| mx::inv(to).is_some())) {
                (Some(m), Some(to)) => {
                    fill(&mut out, built, o, m, to);
                    any = true;
                }
                _ => {
                    out.extend(built);
                    out.push(op);
                }
            }
            continue;
        }
        if let Some(s) = shape.take_if(|s| !s.is_empty()) {
            out.extend(s);
        }
        match o {
            "q" => saved.push(g),
            "Q" => g = saved.pop().unwrap_or(g),
            "cm" => {
                let n: Vec<f32> = op
                    .operands
                    .iter()
                    .filter_map(|n| n.as_float().ok())
                    .collect();
                if let &[a, b, c, d, e, f] = &n[..] {
                    g.0 = mx::mul([a, b, c, d, e, f], g.0);
                }
            }
            "cs" | "sc" | "g" | "rg" | "k" => g.1 = None,
            "scn" => {
                let name = op.operands.last().and_then(|n| n.as_name().ok());
                g.1 = name.and_then(|n| meshes.get(n));
            }
            _ => {}
        }
        out.push(op);
    }
    out.extend(shape.into_iter().flatten());
    any.then_some(out)
}

/// Shape `built` (its clip among it) painted by `paint` with mesh `m`, the pattern's space
/// `to` from the transform now.
fn fill(out: &mut Vec<Operation>, built: Vec<Operation>, paint: &str, m: &Mesh, to: M) {
    let (clip, path): (Vec<_>, Vec<_>) =
        (built.into_iter()).partition(|op| matches!(op.operator.as_str(), "W" | "W*"));
    let op = |name: &str| Operation::new(name, Vec::new());
    out.push(op("q"));
    out.extend(path.iter().cloned());
    out.push(op(if matches!(paint, "f*" | "B*" | "b*") {
        "W*"
    } else {
        "W"
    }));
    out.push(op("n"));
    out.push(Operation::new(
        "cm",
        to.iter().map(|&n| Object::Real(n)).collect(),
    ));
    out.push(Operation::new("sh", vec![Object::Name(m.name.clone())]));
    out.push(op("Q"));
    let after = match paint {
        "B" | "B*" => "S",
        "b" | "b*" => "s",
        _ if !clip.is_empty() => "n",
        _ => return,
    };
    out.extend(path);
    out.extend(clip.into_iter().last());
    out.push(op(after));
}

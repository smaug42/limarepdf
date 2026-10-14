//! Interactive forms (AcroForm): the fields, their values and where their widgets are.

use crate::doc::decode_text;
use crate::matrix::{self as mx, M};
use crate::Pdf;
use lopdf::{Dictionary, Object, ObjectId};
use std::collections::{HashMap, HashSet};

/// Limits that keep a malformed or hostile file from running away.
const MAX_FIELDS: usize = 10_000;
const MAX_DEPTH: usize = 32;

// Field flags (`/Ff`), as bit masks.
const READ_ONLY: i64 = 1;
const REQUIRED: i64 = 1 << 1;
const MULTILINE: i64 = 1 << 12;
const PASSWORD: i64 = 1 << 13;
const RADIO: i64 = 1 << 15;
const PUSH_BUTTON: i64 = 1 << 16;
const COMBO: i64 = 1 << 17;
const EDIT: i64 = 1 << 18;
const MULTI_SELECT: i64 = 1 << 21;
const COMB: i64 = 1 << 24;

/// Annotation flag (`/F`) for an annotation that is not shown.
const HIDDEN: i64 = 1 << 1;

/// What kind of field it is.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FieldKind {
    Text {
        multiline: bool,
        /// Typed characters are hidden, and the value isn't kept in the file.
        password: bool,
        /// Spread over `max_len` equal boxes, one character in each.
        comb: bool,
        max_len: Option<usize>,
    },
    Check,
    Radio,
    /// A drop-down list (`combo`), which may also take typed text (`editable`), or a list
    /// box, which may allow more than one choice (`multi`).
    Choice {
        combo: bool,
        editable: bool,
        multi: bool,
    },
    /// A push button. It only runs JavaScript, so it has no value.
    Button,
    Signature,
}

/// A field's value.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FieldValue {
    Text(String),
    /// A checkbox: ticked or not.
    Check(bool),
    /// The chosen options' export values: at most one, except in a multi-select list
    /// box. For a radio group, the chosen button's export value.
    Choice(Vec<String>),
    /// Push buttons and signature fields.
    None,
}

/// One field, with everything inherited from its parents filled in.
#[derive(Clone, Debug, PartialEq)]
pub struct Field {
    /// The fully qualified name, with parents' names joined by dots.
    pub name: String,
    pub kind: FieldKind,
    pub value: FieldValue,
    /// For choices and radio groups: (export value, text shown).
    pub options: Vec<(String, String)>,
    pub read_only: bool,
    pub required: bool,
    /// The description the file gives for the field (`/TU`), if any.
    pub tooltip: Option<String>,
    /// Where the field is drawn and clicked. Widgets that aren't on any page are left out.
    pub widgets: Vec<Widget>,
}

/// One place a field is shown on a page.
#[derive(Clone, Debug, PartialEq)]
pub struct Widget {
    /// The page (zero-based).
    pub page: usize,
    /// The widget's box in displayed-page points (y down), like
    /// [`crate::PageObject::bbox`].
    pub bbox: [f32; 4],
    /// For checkboxes and radio buttons: the state name meaning "on" for this widget.
    pub on_state: Option<String>,
    /// For radio buttons: the value choosing this one gives the field (the export value
    /// of its "on" state).
    pub export: Option<String>,
    pub hidden: bool,
}

/// Values inherited from parent fields.
#[derive(Clone, Default)]
struct Inherited {
    ft: Option<Vec<u8>>,
    ff: i64,
    v: Option<Object>,
    dv: Option<Object>,
    max_len: Option<usize>,
    opt: Option<Object>,
    da: Option<Vec<u8>>,
    q: Option<i64>,
}

/// Where a field is in the file, and what writing its value needs besides [`Field`].
pub(crate) struct Node {
    /// The terminal field's dictionary, which holds `/V`.
    pub id: ObjectId,
    /// All its widget annotations, on a page or not.
    pub widgets: Vec<ObjectId>,
    /// The default appearance (`/DA`): font, size and colour, inherited or from the form.
    pub da: Option<Vec<u8>>,
    /// Alignment: 0 left, 1 centred, 2 right.
    pub q: i64,
    /// The value a reset goes back to.
    pub dv: Option<Object>,
    /// For a choice or a button with `/Opt`: (export value, text shown).
    pub opt: Vec<(String, String)>,
}

/// State kept while walking the field tree.
struct Walk {
    /// Each annotation's page, from the pages' `/Annots`.
    pages: HashMap<ObjectId, usize>,
    /// Each page's matrix from user space to displayed-page points.
    to_vis: HashMap<usize, Option<M>>,
    seen: HashSet<ObjectId>,
    out: Vec<(Field, Node)>,
}

impl Pdf {
    /// The form's fields, in the order the form lists them; empty when there is no form.
    /// Broken entries are skipped, never fatal.
    pub fn form_fields(&self) -> Vec<Field> {
        self.form_nodes().into_iter().map(|(f, _)| f).collect()
    }

    /// The fields with where each is in the file.
    pub(crate) fn form_nodes(&self) -> Vec<(Field, Node)> {
        let Some(form) = self.acro_form() else {
            return Vec::new();
        };
        let top = Inherited {
            da: form.get(b"DA").ok().and_then(|o| self.string_bytes(o)),
            q: form.get(b"Q").ok().and_then(|o| self.int_value(o)),
            ..Inherited::default()
        };
        let Some(fields) = form
            .get(b"Fields")
            .ok()
            .and_then(|o| self.doc.dereference(o).ok())
            .and_then(|(_, o)| o.as_array().ok())
        else {
            return Vec::new();
        };
        let mut walk = Walk {
            pages: self.annot_pages(),
            to_vis: HashMap::new(),
            seen: HashSet::new(),
            out: Vec::new(),
        };
        for f in fields {
            if let Ok(id) = f.as_reference() {
                self.walk_field(id, "", &top, 0, &mut walk);
            }
        }
        walk.out
    }

    /// The catalog's `/AcroForm` dictionary.
    pub(crate) fn acro_form(&self) -> Option<&Dictionary> {
        let form = self.doc.catalog().ok()?.get(b"AcroForm").ok()?;
        self.doc.dereference(form).ok()?.1.as_dict().ok()
    }

    /// Each annotation listed on a page, with that page.
    fn annot_pages(&self) -> HashMap<ObjectId, usize> {
        let mut map = HashMap::new();
        for (i, id) in self.page_ids().into_iter().enumerate() {
            let annots = self
                .doc
                .get_dictionary(id)
                .ok()
                .and_then(|d| d.get(b"Annots").ok())
                .and_then(|o| self.doc.dereference(o).ok())
                .and_then(|(_, o)| o.as_array().ok());
            for a in annots.into_iter().flatten() {
                if let Ok(a) = a.as_reference() {
                    map.entry(a).or_insert(i);
                }
            }
        }
        map
    }

    fn walk_field(&self, id: ObjectId, parent: &str, inh: &Inherited, depth: usize, w: &mut Walk) {
        if depth > MAX_DEPTH || w.out.len() >= MAX_FIELDS || !w.seen.insert(id) {
            return;
        }
        let Ok(d) = self.doc.get_dictionary(id) else {
            return;
        };
        let name = match d.get(b"T").ok().and_then(|o| self.text_value(o)) {
            Some(t) if parent.is_empty() => t,
            Some(t) => format!("{parent}.{t}"),
            None => parent.to_string(),
        };
        let mut inh = inh.clone();
        if let Some(ft) = d.get(b"FT").ok().and_then(|o| o.as_name().ok()) {
            inh.ft = Some(ft.to_vec());
        }
        if let Some(ff) = d.get(b"Ff").ok().and_then(|o| self.int_value(o)) {
            inh.ff = ff;
        }
        if let Ok(v) = d.get(b"V") {
            inh.v = Some(v.clone());
        }
        if let Ok(dv) = d.get(b"DV") {
            inh.dv = Some(dv.clone());
        }
        if let Some(da) = d.get(b"DA").ok().and_then(|o| self.string_bytes(o)) {
            inh.da = Some(da);
        }
        if let Some(q) = d.get(b"Q").ok().and_then(|o| self.int_value(o)) {
            inh.q = Some(q);
        }
        if let Some(n) = d.get(b"MaxLen").ok().and_then(|o| self.int_value(o)) {
            inh.max_len = usize::try_from(n).ok();
        }
        if let Ok(opt) = d.get(b"Opt") {
            inh.opt = Some(opt.clone());
        }

        // Kids with a name are fields of their own; kids without one are this field's
        // widgets.
        let kids: Vec<ObjectId> = d
            .get(b"Kids")
            .ok()
            .and_then(|o| self.doc.dereference(o).ok())
            .and_then(|(_, o)| o.as_array().ok())
            .map(|a| a.iter().filter_map(|k| k.as_reference().ok()).collect())
            .unwrap_or_default();
        let (fields, widgets): (Vec<ObjectId>, Vec<ObjectId>) = kids.into_iter().partition(|k| {
            self.doc
                .get_dictionary(*k)
                .is_ok_and(|k| k.has(b"T") || !k.has(b"Rect"))
        });
        if !fields.is_empty() {
            for k in fields {
                self.walk_field(k, &name, &inh, depth + 1, w);
            }
            return;
        }
        let widgets = if widgets.is_empty() && d.has(b"Rect") {
            vec![id]
        } else {
            widgets
        };
        if let Some(field) = self.make_field(name, d, &inh, &widgets, w) {
            let node = Node {
                id,
                widgets,
                da: inh.da,
                q: inh.q.unwrap_or(0),
                dv: inh.dv,
                opt: inh.opt.map(|o| self.options(&o)).unwrap_or_default(),
            };
            w.out.push((field, node));
        }
    }

    /// A terminal field from its dictionary, what it inherits and its widgets.
    fn make_field(
        &self,
        name: String,
        d: &Dictionary,
        inh: &Inherited,
        widgets: &[ObjectId],
        w: &mut Walk,
    ) -> Option<Field> {
        let ff = inh.ff;
        let kind = match inh.ft.as_deref()? {
            b"Tx" => FieldKind::Text {
                multiline: ff & MULTILINE != 0,
                password: ff & PASSWORD != 0,
                comb: ff & COMB != 0,
                max_len: inh.max_len,
            },
            b"Btn" if ff & PUSH_BUTTON != 0 => FieldKind::Button,
            b"Btn" if ff & RADIO != 0 => FieldKind::Radio,
            b"Btn" => FieldKind::Check,
            b"Ch" => FieldKind::Choice {
                combo: ff & COMBO != 0,
                editable: ff & EDIT != 0,
                multi: ff & MULTI_SELECT != 0,
            },
            b"Sig" => FieldKind::Signature,
            _ => return None,
        };
        let opt = inh
            .opt
            .as_ref()
            .map(|o| self.options(o))
            .unwrap_or_default();
        let mut ws: Vec<Widget> = widgets
            .iter()
            .filter_map(|&id| self.widget(id, w))
            .collect();
        for wd in &mut ws {
            wd.export = wd.on_state.as_deref().map(|s| button_export(s, &opt));
        }
        let value = self.value_of(&kind, inh.v.as_ref(), &opt, widgets);
        let options = match &kind {
            FieldKind::Check | FieldKind::Radio => {
                let mut options: Vec<(String, String)> = Vec::new();
                for s in widgets.iter().filter_map(|&id| self.on_state(id)) {
                    let e = button_export(&s, &opt);
                    if !options.iter().any(|o| o.0 == e) {
                        options.push((e.clone(), e));
                    }
                }
                options
            }
            FieldKind::Choice { .. } => opt,
            _ => Vec::new(),
        };
        Some(Field {
            name,
            kind,
            value,
            options,
            read_only: ff & READ_ONLY != 0,
            required: ff & REQUIRED != 0,
            tooltip: d.get(b"TU").ok().and_then(|o| self.text_value(o)),
            widgets: ws,
        })
    }

    /// A widget annotation, if it is on a page.
    fn widget(&self, id: ObjectId, w: &mut Walk) -> Option<Widget> {
        let d = self.doc.get_dictionary(id).ok()?;
        let page = *w.pages.get(&id)?;
        let rect = self.rect(d.get(b"Rect").ok()?)?;
        let to_vis = *w.to_vis.entry(page).or_insert_with(|| {
            let pid = self.page_id_at(page).ok()?;
            mx::inv(self.vis_down(pid).0)
        });
        let on_state = self.on_state(id);
        let flags = d
            .get(b"F")
            .ok()
            .and_then(|o| self.int_value(o))
            .unwrap_or(0);
        Some(Widget {
            page,
            bbox: mx::transform_rect(to_vis?, rect),
            on_state,
            export: None,
            hidden: flags & HIDDEN != 0,
        })
    }

    /// A field's value from its `/V` (or `/DV`) entry. For buttons, `widgets` give the
    /// states, and `opt` the export values when the states are numbers.
    pub(crate) fn value_of(
        &self,
        kind: &FieldKind,
        v: Option<&Object>,
        opt: &[(String, String)],
        widgets: &[ObjectId],
    ) -> FieldValue {
        match kind {
            FieldKind::Text { .. } => {
                FieldValue::Text(v.and_then(|o| self.text_value(o)).unwrap_or_default())
            }
            FieldKind::Check | FieldKind::Radio => {
                let on = v
                    .and_then(|o| self.doc.dereference(o).ok())
                    .and_then(|(_, o)| o.as_name().ok())
                    .map(|n| String::from_utf8_lossy(n).into_owned())
                    .filter(|n| n != "Off");
                // A checkbox is ticked only when its value is one of its widgets' states
                // (or, when they have no drawings to name states, when it isn't Off).
                if *kind == FieldKind::Check {
                    let states: Vec<String> =
                        widgets.iter().filter_map(|&w| self.on_state(w)).collect();
                    let ticked = on.is_some_and(|s| states.is_empty() || states.contains(&s));
                    FieldValue::Check(ticked)
                } else {
                    FieldValue::Choice(on.map(|s| button_export(&s, opt)).into_iter().collect())
                }
            }
            FieldKind::Choice { .. } => {
                let chosen = match v.map(|o| self.doc.dereference(o).map(|x| x.1)) {
                    Some(Ok(Object::Array(a))) => {
                        a.iter().filter_map(|o| self.text_value(o)).collect()
                    }
                    Some(Ok(o)) => self.text_value(o).into_iter().collect(),
                    _ => Vec::new(),
                };
                FieldValue::Choice(chosen)
            }
            FieldKind::Button | FieldKind::Signature => FieldValue::None,
        }
    }

    /// A checkbox or radio widget's "on" state: the name in its normal appearance other
    /// than `Off`.
    pub(crate) fn on_state(&self, widget: ObjectId) -> Option<String> {
        self.doc
            .get_dictionary(widget)
            .ok()?
            .get(b"AP")
            .ok()
            .and_then(|o| self.doc.dereference(o).ok())
            .and_then(|(_, o)| o.as_dict().ok())
            .and_then(|ap| ap.get(b"N").ok())
            .and_then(|o| self.doc.dereference(o).ok())
            .and_then(|(_, o)| o.as_dict().ok())
            .and_then(|n| {
                n.iter()
                    .map(|(k, _)| k)
                    .find(|k| k.as_slice() != b"Off")
                    .map(|k| String::from_utf8_lossy(k).into_owned())
            })
    }

    /// A choice field's `/Opt`: each entry a string, or an [export value, text shown]
    /// pair.
    fn options(&self, opt: &Object) -> Vec<(String, String)> {
        let Some(a) = self
            .doc
            .dereference(opt)
            .ok()
            .and_then(|(_, o)| o.as_array().ok())
        else {
            return Vec::new();
        };
        a.iter()
            .filter_map(|o| match self.doc.dereference(o).ok()?.1 {
                Object::Array(pair) if pair.len() >= 2 => {
                    Some((self.text_value(&pair[0])?, self.text_value(&pair[1])?))
                }
                o => {
                    let t = self.text_value(o)?;
                    Some((t.clone(), t))
                }
            })
            .take(MAX_FIELDS)
            .collect()
    }

    /// A text string (or a name, as some files write), decoded.
    fn text_value(&self, o: &Object) -> Option<String> {
        match self.doc.dereference(o).ok()?.1 {
            Object::String(s, _) => Some(decode_text(s)),
            Object::Name(n) => Some(String::from_utf8_lossy(n).into_owned()),
            Object::Stream(s) => {
                let data = s.decompressed_content().ok()?;
                Some(decode_text(&data))
            }
            _ => None,
        }
    }

    pub(crate) fn int_value(&self, o: &Object) -> Option<i64> {
        self.doc.dereference(o).ok()?.1.as_i64().ok()
    }

    /// A string's raw bytes.
    pub(crate) fn string_bytes(&self, o: &Object) -> Option<Vec<u8>> {
        match self.doc.dereference(o).ok()?.1 {
            Object::String(s, _) => Some(s.clone()),
            _ => None,
        }
    }
}

/// A button state's export value: the state itself, or, when the field has `/Opt` and
/// the state is a number, that entry of `/Opt`.
pub(crate) fn button_export(state: &str, opt: &[(String, String)]) -> String {
    state
        .parse::<usize>()
        .ok()
        .and_then(|i| opt.get(i))
        .map_or_else(|| state.to_string(), |o| o.0.clone())
}

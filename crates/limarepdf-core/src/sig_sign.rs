//! Signing with a digital ID: an ordinary (approval) signature or a certification, in an
//! empty signature field or a new box, written as Acrobat writes it (`adbe.pkcs7.detached`,
//! SHA-256 or SHA-384) in a revision appended to the file (see [`crate::incremental`]),
//! so signatures already in it stay valid. See docs/signatures.md.
//!
//! The document's own rules are honoured: a certification that allows no changes, or one
//! that allows only signing in existing fields, and fields locked by earlier signers. A
//! field whose `/Lock` names other fields locks them, as Acrobat does. Before the result is
//! handed out LimarePDF checks it like any signed file: the new signature must be valid and
//! cover the whole file, and every earlier signature must check as it did before.

use crate::cms_check::CertInfo;
use crate::cms_write;
use crate::doc::encode_text;
use crate::embed::unwritable;
use crate::form_ap::nums;
use crate::forms::FieldKind;
use crate::ids::UnlockedId;
use crate::incremental;
use crate::matrix as mx;
use crate::overlay::text_width;
use crate::sig_verify::{Lexer, Tok};
use crate::sign::{signature_art, typed_signature};
use crate::{
    verify_signatures, FontChoice, InkColor, LockAction, Pdf, Permissions, Result, SigKind,
    SigStatus, SignArt, SignRole, SignStyle, SignatureReport, TrustList,
};
use anyhow::{anyhow, bail, Context};
use chrono::{DateTime, FixedOffset, Utc};
use lopdf::{dictionary, Dictionary, Object, ObjectId, Stream, StringFormat};
use std::collections::HashSet;

/// Where a digital signature goes.
#[derive(Clone, Debug, PartialEq)]
pub enum SignPlace {
    /// An empty signature field in the document, by its full name.
    Field(String),
    /// A new signature field: a box on page `page` (zero-based), in displayed-page points
    /// with y down, like [`crate::Widget::bbox`].
    NewBox { page: usize, rect: [f32; 4] },
}

/// A digital signature to add with [`sign_document`].
pub struct SignRequest<'a> {
    pub id: &'a UnlockedId,
    pub place: SignPlace,
    /// Why and where it was signed, as the signer states them (shown by every reader).
    pub reason: Option<String>,
    pub location: Option<String>,
    /// Certify the document (allowed only before anyone has signed it), with what it
    /// allows afterwards, instead of an ordinary signature.
    pub certify: Option<Permissions>,
    /// The left part of the signature's look: the signer's own signature. `None` writes
    /// the ID's name in handwriting.
    pub art: Option<SignArt<'a>>,
    /// The caller showed the signer that their edits will show as disallowed changes to
    /// earlier signatures, and the signer went ahead. Without it, such signing is refused.
    pub accept_disallowed: bool,
}

/// A signed document.
#[derive(Clone, Debug)]
pub struct SignedDocument {
    pub bytes: Vec<u8>,
    /// The full name of the field signed.
    pub field: String,
    /// Fill-and-sign items drawn into the pages. When the document already had signatures
    /// they are locked in place instead (drawing them in would change page content).
    pub finished_items: usize,
    /// The document had no signatures and its structure couldn't be added to (LimarePDF repaired
    /// it to open it, or other PDF apps may read it differently from LimarePDF), so it was
    /// written out in full before the signature was added.
    pub rewritten: bool,
}

/// What `/ByteRange` holds until the real range is known: wide enough for files up to
/// 10 GB.
const PLACEHOLDER: i64 = 9_999_999_999;
/// Annotation flags: printed, and locked (can't be moved or deleted).
const PRINT: i64 = 4;
const LOCKED: i64 = 128;
/// Field flag: read-only.
const READ_ONLY: i64 = 1;
/// The smallest box a signature can be put in, in points.
pub const MIN_SIGNATURE_BOX: f32 = 12.0;
/// Room left in `/Contents` beyond a trial signature's size, for ECDSA values a few bytes
/// longer than the trial's.
const ROOM_MARGIN: usize = 256;
/// The handwriting a name is written in when the signer gives no signature of their own.
const NAME_STYLE: SignStyle = SignStyle::Caveat;
/// The appearance's resource names.
const FONT: &str = "Helv";
const ART: &str = "Sig";

const ENCRYPTED: &str =
    "LimarePDF can't sign encrypted (password-protected) documents; sign a copy \
                         without the password instead";

/// Signs `original` (the file as it was opened) with the request's digital ID. `edited`
/// is the document as edited since it was opened, if it was; its edits go into the same
/// revision as the signature. The result has been checked as a signed file before it is
/// returned; nothing is ever returned that didn't check.
pub fn sign_document(
    original: &[u8],
    edited: Option<&Pdf>,
    req: &SignRequest,
) -> Result<SignedDocument> {
    sign_at(original, edited, req, chrono::Local::now().fixed_offset())
}

pub(crate) fn sign_at(
    original: &[u8],
    edited: Option<&Pdf>,
    req: &SignRequest,
    now: DateTime<FixedOffset>,
) -> Result<SignedDocument> {
    check_id(&req.id.cert, now.with_timezone(&Utc))?;
    let opened = Pdf::from_bytes(original, None).map_err(|e| {
        // A password-protected document: the reason it can't be signed is the encryption.
        if format!("{e:#}").contains("encrypted") {
            anyhow!("{ENCRYPTED}")
        } else {
            e
        }
    })?;
    if opened.encrypted() || edited.is_some_and(Pdf::encrypted) {
        bail!("{ENCRYPTED}");
    }
    let before = verify_signatures(original, None, &TrustList::default())?;
    check_rules(&before, &opened, req)?;

    let problem = incremental::base_problem(original, &opened);
    let mut pdf = edited.cloned().unwrap_or(opened);
    let mut base = std::borrow::Cow::Borrowed(original);
    let mut rewritten = false;
    if let Some(why) = problem {
        if !before.signatures.is_empty() {
            bail!(
                "{why}, so LimarePDF can't add a signature without breaking the ones already in it"
            );
        }
        base = std::borrow::Cow::Owned(pdf.to_bytes()?);
        pdf = Pdf::from_bytes(&base, None)?;
        rewritten = true;
    }

    let earlier = !before.signatures.is_empty();
    let finished_items = if earlier {
        pdf.lock_sign_items()?;
        0
    } else {
        settle_appearances(&mut pdf)?;
        pdf.finish_signing()?
    };
    let room = cms_write::signed_data(req.id, &[b""], now.with_timezone(&Utc))?.len() + ROOM_MARGIN;
    let (field, sig_id) = place_signature(&mut pdf, &before, req, room, now)?;

    let appended = incremental::append_revision(&base, &pdf.doc)?;
    let mut bytes = appended.bytes;
    let at = *appended
        .offsets
        .get(&sig_id)
        .context("the signature wasn't written")?;
    let spots = placeholders(&bytes, at, sig_id, room)
        .context("the signature's placeholders weren't found where they were written")?;
    let (b, c) = (spots.contents.0, spots.contents.1);
    let range = format!("0 {b} {c} {}", bytes.len() - c);
    let slot = spots.byte_range.1 - spots.byte_range.0 - 2;
    if range.len() > slot {
        bail!("the file is too large to sign");
    }
    bytes[spots.byte_range.0 + 1..spots.byte_range.1 - 1]
        .copy_from_slice(format!("{range:<slot$}").as_bytes());
    let cms = cms_write::signed_data(req.id, &[&bytes[..b], &bytes[c..]], now.with_timezone(&Utc))?;
    if cms.len() > room {
        bail!("the signature came out larger than the room kept for it");
    }
    let hex: String = cms.iter().map(|x| format!("{x:02X}")).collect();
    bytes[b + 1..b + 1 + hex.len()].copy_from_slice(hex.as_bytes());

    self_check(&base, &bytes, &before, &field, req)?;
    Ok(SignedDocument {
        bytes,
        field,
        finished_items,
        rewritten,
    })
}

/// LimarePDF signs only with an ID valid at the signing time.
fn check_id(cert: &CertInfo, now: DateTime<Utc>) -> Result<()> {
    let day = |t: DateTime<Utc>| t.format("%Y-%m-%d").to_string();
    if now > cert.not_after {
        bail!(
            "the digital ID expired on {}; LimarePDF doesn't sign with expired IDs (make or import a \
             new one)",
            day(cert.not_after)
        );
    }
    if now < cert.not_before {
        bail!(
            "the digital ID isn't valid until {} (check this computer's clock)",
            day(cert.not_before)
        );
    }
    Ok(())
}

/// Refuses what the document's signatures and certification don't allow, with why.
fn check_rules(rep: &SignatureReport, pdf: &Pdf, req: &SignRequest) -> Result<()> {
    let signed = rep.signatures.len();
    if req.certify.is_some() && signed > 0 {
        bail!(
            "a document can be certified only before anyone signs it, and this one has {signed} \
             signature{}",
            if signed == 1 { "" } else { "s" }
        );
    }
    if let Some((_, p)) = rep.certification() {
        if p == Permissions::NoChanges {
            bail!("the author certified this document allowing no changes, so it can't be signed");
        }
        if matches!(req.place, SignPlace::NewBox { .. }) {
            bail!(
                "the author certified this document, which allows signing only in the signature \
                 fields it already has"
            );
        }
    }
    let locked_all = rep.signatures.iter().find(|s| {
        s.locks
            .as_ref()
            .is_some_and(|l| l.permissions == Some(Permissions::NoChanges))
    });
    if let Some(s) = locked_all {
        bail!(
            "{} locked the document when signing, so it can't be signed again",
            signer_name(s)
        );
    }
    match &req.place {
        SignPlace::Field(name) => {
            if rep.signatures.iter().any(|s| s.fields.contains(name)) {
                bail!("the field \"{name}\" is already signed");
            }
            let Some(field) = pdf.form_fields().into_iter().find(|f| &f.name == name) else {
                bail!("the document has no field named \"{name}\"");
            };
            if field.kind != FieldKind::Signature {
                bail!("\"{name}\" isn't a signature field");
            }
            if field.read_only {
                bail!("the signature field \"{name}\" is read-only");
            }
            if let Some(s) = rep
                .signatures
                .iter()
                .find(|s| s.locks.as_ref().is_some_and(|l| l.locks(name)))
            {
                bail!(
                    "{} locked the field \"{name}\" when signing, so it can't be signed",
                    signer_name(s)
                );
            }
        }
        SignPlace::NewBox { page, rect } => {
            let page_id = pdf.page_id_at(*page)?;
            let (_, vw, vh) = pdf.vis_down(page_id);
            let [x0, y0, x1, y1] = *rect;
            if !rect.iter().all(|v| v.is_finite())
                || x1 - x0 < MIN_SIGNATURE_BOX
                || y1 - y0 < MIN_SIGNATURE_BOX
            {
                bail!("a signature box must be at least {MIN_SIGNATURE_BOX} points wide and high");
            }
            let slack = 0.5;
            if x0 < -slack || y0 < -slack || x1 > vw + slack || y1 > vh + slack {
                bail!("the signature box must be on the page");
            }
        }
    }
    Ok(())
}

fn signer_name(s: &crate::DigitalSignature) -> String {
    match s.signer.as_ref().map(|c| c.display_name().to_string()) {
        Some(n) => format!("\"{n}\""),
        None => "an earlier signer".into(),
    }
}

/// When the form asks readers to draw fields themselves (`/NeedAppearances`), each reader
/// shows the fields its own way, so what is signed would look different in each. LimarePDF
/// draws them and turns the request off, so every reader shows what LimarePDF showed.
fn settle_appearances(pdf: &mut Pdf) -> Result<()> {
    let asks = pdf
        .acro_form()
        .and_then(|f| f.get(b"NeedAppearances").ok())
        .and_then(|o| pdf.doc.dereference(o).ok())
        .is_some_and(|(_, o)| o.as_bool().unwrap_or(false));
    if asks {
        pdf.refresh_appearances();
        let form = pdf.acro_form_id()?;
        pdf.doc.get_dictionary_mut(form)?.remove(b"NeedAppearances");
    }
    Ok(())
}

/// Writes the signature dictionary (with placeholders), its field or widget and its
/// appearance, locks and certification. Returns the field's name and the dictionary's id.
fn place_signature(
    pdf: &mut Pdf,
    rep: &SignatureReport,
    req: &SignRequest,
    room: usize,
    now: DateTime<FixedOffset>,
) -> Result<(String, ObjectId)> {
    let root = pdf.doc.trailer.get(b"Root")?.as_reference()?;
    let mut refs: Vec<Object> = Vec::new();
    if let Some(p) = req.certify {
        refs.push(
            dictionary! {
                "Type" => "SigRef",
                "TransformMethod" => "DocMDP",
                "TransformParams" => dictionary! {
                    "Type" => "TransformParams",
                    "P" => permission_number(p),
                    "V" => Object::Name(b"1.2".to_vec()),
                },
            }
            .into(),
        );
    }
    let name = match &req.place {
        SignPlace::Field(n) => n.clone(),
        SignPlace::NewBox { .. } => free_field_name(pdf),
    };
    // Fields this signature locks, from the field's `/Lock`.
    if let Some(lock) = rep
        .empty_fields
        .iter()
        .find(|f| f.name == name)
        .and_then(|f| f.lock.clone())
    {
        let mut params = dictionary! {
            "Type" => "TransformParams",
            "Action" => match lock.action {
                LockAction::All => "All",
                LockAction::Include => "Include",
                LockAction::Exclude => "Exclude",
            },
            "V" => Object::Name(b"1.2".to_vec()),
        };
        if lock.action != LockAction::All {
            let fields: Vec<Object> = lock
                .fields
                .iter()
                .map(|f| Object::String(encode_text(f), StringFormat::Literal))
                .collect();
            params.set("Fields", fields);
        }
        if let Some(p) = lock.permissions {
            params.set("P", permission_number(p));
        }
        refs.push(
            dictionary! {
                "Type" => "SigRef",
                "TransformMethod" => "FieldMDP",
                "TransformParams" => params,
                "Data" => root,
            }
            .into(),
        );
        // Read-only in every reader, as Acrobat does. Signature fields stay as they are:
        // the lock itself covers signing them.
        for (field, node) in pdf.form_nodes() {
            if field.name != name
                && field.kind != FieldKind::Signature
                && !field.read_only
                && lock.locks(&field.name)
            {
                let ff = effective_ff(pdf, node.id);
                pdf.doc
                    .get_dictionary_mut(node.id)?
                    .set("Ff", ff | READ_ONLY);
            }
        }
    }

    let cert = &req.id.cert;
    let mut sig = dictionary! {
        "Type" => "Sig",
        "Filter" => "Adobe.PPKLite",
        "SubFilter" => "adbe.pkcs7.detached",
        "ByteRange" => vec![0.into(), PLACEHOLDER.into(), PLACEHOLDER.into(), PLACEHOLDER.into()],
        "Contents" => Object::String(vec![0; room], StringFormat::Hexadecimal),
        "M" => Object::string_literal(pdf_date(now)),
        "Name" => text_object(cert.display_name()),
        "Prop_Build" => dictionary! {
            "App" => dictionary! {
                "Name" => "LimarePDF",
                "REx" => Object::string_literal(env!("CARGO_PKG_VERSION")),
            },
            "Filter" => dictionary! { "Name" => "Adobe.PPKLite" },
        },
    };
    for (key, value) in [("Reason", &req.reason), ("Location", &req.location)] {
        if let Some(v) = value.as_deref().map(str::trim).filter(|v| !v.is_empty()) {
            if v.chars().any(char::is_control) {
                bail!(
                    "the {} can't hold line breaks or control characters",
                    key.to_lowercase()
                );
            }
            sig.set(key, text_object(v));
        }
    }
    if !refs.is_empty() {
        sig.set("Reference", refs);
    }
    let sig_id = pdf.doc.add_object(sig);

    let shown = shown_name(cert);
    match &req.place {
        SignPlace::Field(_) => {
            let (_, node) = pdf
                .form_nodes()
                .into_iter()
                .find(|(f, _)| f.name == name)
                .ok_or_else(|| anyhow!("the field \"{name}\" disappeared"))?;
            if node.widgets.is_empty() {
                bail!("the signature field \"{name}\" isn't shown anywhere, so it can't be signed");
            }
            pdf.doc.get_dictionary_mut(node.id)?.set("V", sig_id);
            for w in node.widgets {
                let d = pdf.doc.get_dictionary(w)?;
                let r = d
                    .get(b"Rect")
                    .ok()
                    .and_then(|o| pdf.rect(o))
                    .ok_or_else(|| anyhow!("the signature field \"{name}\" has no place"))?;
                let rot = d
                    .get(b"MK")
                    .ok()
                    .and_then(|o| pdf.dict_of(o))
                    .and_then(|mk| mk.get(b"R").ok())
                    .and_then(|o| pdf.int_value(o))
                    .unwrap_or(0)
                    .rem_euclid(360);
                let ap = appearance(
                    pdf,
                    (r[2] - r[0]).abs(),
                    (r[3] - r[1]).abs(),
                    rot,
                    &shown,
                    req.art,
                    now,
                )?;
                let d = pdf.doc.get_dictionary_mut(w)?;
                let flags = d.get(b"F").and_then(Object::as_i64).unwrap_or(0);
                d.set("F", flags | LOCKED);
                d.set("AP", dictionary! { "N" => ap });
            }
        }
        SignPlace::NewBox { page, rect } => {
            let page_id = pdf.page_id_at(*page)?;
            let (to_user, _, _) = pdf.vis_down(page_id);
            let r = mx::transform_rect(to_user, *rect);
            let rot = pdf.page_rotation(page_id).rem_euclid(360);
            let ap = appearance(pdf, r[2] - r[0], r[3] - r[1], rot, &shown, req.art, now)?;
            let mut widget = dictionary! {
                "Type" => "Annot",
                "Subtype" => "Widget",
                "FT" => "Sig",
                "T" => text_object(&name),
                "V" => sig_id,
                "F" => PRINT | LOCKED,
                "Rect" => nums(&r),
                "P" => page_id,
                "AP" => dictionary! { "N" => ap },
            };
            if rot != 0 {
                widget.set("MK", dictionary! { "R" => rot });
            }
            let widget = pdf.doc.add_object(widget);
            pdf.push_annot(page_id, widget)?;
            add_to_fields(pdf, widget)?;
        }
    }
    let form = pdf.acro_form_id()?;
    let form = pdf.doc.get_dictionary_mut(form)?;
    let flags = form.get(b"SigFlags").and_then(Object::as_i64).unwrap_or(0);
    form.set("SigFlags", flags | 3);
    if req.certify.is_some() {
        set_perms(pdf, root, sig_id)?;
    }
    Ok((name, sig_id))
}

fn permission_number(p: Permissions) -> i64 {
    match p {
        Permissions::NoChanges => 1,
        Permissions::FormFilling => 2,
        Permissions::FormFillingAndComments => 3,
    }
}

/// A text string: plain when it is ASCII, UTF-16 otherwise.
fn text_object(s: &str) -> Object {
    Object::String(encode_text(s), StringFormat::Literal)
}

/// A field flags value as it applies to a field, inherited from its parents when it has
/// none of its own.
fn effective_ff(pdf: &Pdf, mut id: ObjectId) -> i64 {
    for _ in 0..64 {
        let Ok(d) = pdf.doc.get_dictionary(id) else {
            break;
        };
        if let Some(ff) = d.get(b"Ff").ok().and_then(|o| pdf.int_value(o)) {
            return ff;
        }
        match d.get(b"Parent").and_then(Object::as_reference) {
            Ok(p) => id = p,
            Err(_) => break,
        }
    }
    0
}

/// "Signature1", "Signature2", ...: the first name no top-level field has.
fn free_field_name(pdf: &Pdf) -> String {
    let mut taken: HashSet<String> = pdf
        .form_fields()
        .into_iter()
        .filter_map(|f| f.name.split('.').next().map(str::to_string))
        .collect();
    let top = pdf
        .acro_form()
        .and_then(|f| f.get(b"Fields").ok())
        .and_then(|o| pdf.doc.dereference(o).ok())
        .and_then(|(_, o)| o.as_array().ok().cloned())
        .unwrap_or_default();
    for f in top {
        if let Some(t) = pdf
            .dict_of(&f)
            .and_then(|d| d.get(b"T").ok())
            .and_then(|t| pdf.text_value(t))
        {
            taken.insert(t);
        }
    }
    (1..)
        .map(|i| format!("Signature{i}"))
        .find(|n| !taken.contains(n))
        .expect("some number is free")
}

/// Adds a new top-level field to the form, making the form if there is none.
fn add_to_fields(pdf: &mut Pdf, field: ObjectId) -> Result<()> {
    let form = pdf.acro_form_id()?;
    let current = pdf.doc.get_dictionary(form)?.get(b"Fields").ok().cloned();
    match current {
        Some(Object::Reference(r)) if pdf.doc.get_object(r).is_ok_and(|o| o.as_array().is_ok()) => {
            pdf.doc
                .get_object_mut(r)?
                .as_array_mut()?
                .push(field.into());
        }
        Some(Object::Array(mut a)) => {
            a.push(field.into());
            pdf.doc.get_dictionary_mut(form)?.set("Fields", a);
        }
        None => pdf
            .doc
            .get_dictionary_mut(form)?
            .set("Fields", vec![Object::from(field)]),
        Some(_) => bail!("the document's form list is damaged"),
    }
    Ok(())
}

/// Points the catalog's `/Perms /DocMDP` at the certification signature.
fn set_perms(pdf: &mut Pdf, root: ObjectId, sig: ObjectId) -> Result<()> {
    let current = pdf.doc.get_dictionary(root)?.get(b"Perms").ok().cloned();
    match current {
        Some(Object::Reference(r)) if pdf.doc.get_dictionary(r).is_ok() => {
            pdf.doc.get_dictionary_mut(r)?.set("DocMDP", sig);
        }
        Some(Object::Dictionary(mut d)) => {
            d.set("DocMDP", sig);
            pdf.doc.get_dictionary_mut(root)?.set("Perms", d);
        }
        None => pdf
            .doc
            .get_dictionary_mut(root)?
            .set("Perms", dictionary! { "DocMDP" => sig }),
        Some(_) => bail!("the document's permissions are damaged"),
    }
    Ok(())
}

/// A PDF date in the signer's time zone, as Acrobat writes it: `D:20261016142305-04'00'`.
fn pdf_date(t: DateTime<FixedOffset>) -> String {
    format!("D:{}{}", t.format("%Y%m%d%H%M%S"), zone(t))
}

/// The time zone as Adobe writes it: `-04'00'`.
fn zone(t: DateTime<FixedOffset>) -> String {
    let secs = t.offset().local_minus_utc();
    let sign = if secs < 0 { '-' } else { '+' };
    let m = secs.unsigned_abs() / 60;
    format!("{sign}{:02}'{:02}'", m / 60, m % 60)
}

/// The signer's name as the appearance writes it, or `None` when it has characters the
/// appearance's font (Helvetica, or its embedded face) can't show: the line then leaves
/// the name out rather than show it wrongly. Readers show the full name from the
/// certificate.
fn shown_name(cert: &CertInfo) -> Option<String> {
    let n = cert.display_name().trim();
    let shows = unwritable(FontChoice::default(), n).is_empty();
    (shows && !n.is_empty()).then(|| n.to_string())
}

// ---------------------------------------------------------------- appearance

/// The signature's look in a `w` by `h` widget turned `rot` degrees: on the left (or top,
/// in a tall box) the signer's signature, or their name in handwriting; on the right
/// "Digitally signed by NAME" and the date, as Acrobat shows them.
fn appearance(
    pdf: &mut Pdf,
    w: f32,
    h: f32,
    rot: i64,
    name: &Option<String>,
    art: Option<SignArt>,
    now: DateTime<FixedOffset>,
) -> Result<ObjectId> {
    let (w, h, matrix) = match rot {
        90 => (h, w, Some([0.0, 1.0, -1.0, 0.0, 0.0, 0.0])),
        180 => (w, h, Some([-1.0, 0.0, 0.0, -1.0, 0.0, 0.0])),
        270 => (h, w, Some([0.0, -1.0, 1.0, 0.0, 0.0, 0.0])),
        _ => (w, h, None),
    };
    let mut form = dictionary! {
        "Type" => "XObject",
        "Subtype" => "Form",
        "BBox" => nums(&[0.0, 0.0, w, h]),
    };
    if let Some(m) = matrix {
        form.set("Matrix", nums(&m));
    }
    if w < 1.0 || h < 1.0 {
        // An invisible signature field: nothing to draw.
        form.set("Resources", Dictionary::new());
        return Ok(pdf.doc.add_object(Stream::new(form, Vec::new())));
    }

    let typed;
    let art = match art {
        Some(a) => Some(a),
        None => match name.as_deref().map(|n| typed_signature(n, NAME_STYLE)) {
            Some(Ok(t)) => {
                typed = t;
                Some(SignArt::Typed {
                    text: &typed,
                    style: NAME_STYLE,
                })
            }
            _ => None,
        },
    };
    const PAD: f32 = 2.0;
    let (art_box, text_box) = match art {
        None => (None, [PAD, PAD, w - PAD, h - PAD]),
        Some(_) if w >= 1.6 * h => (
            Some([PAD, PAD, w / 2.0 - PAD, h - PAD]),
            [w / 2.0 + PAD, PAD, w - PAD, h - PAD],
        ),
        Some(_) => (
            Some([PAD, h / 2.0 + PAD, w - PAD, h - PAD]),
            [PAD, PAD, w - PAD, h / 2.0 - PAD],
        ),
    };

    let mut ops = String::new();
    let mut res = Dictionary::new();
    if let (Some(a), Some([x0, y0, x1, y1])) = (art, art_box) {
        let drawn = signature_art(&a, SignRole::Signature, InkColor::Black, false)?;
        let (aw, ah) = (drawn.w, drawn.h);
        let k = ((x1 - x0) / aw).min((y1 - y0) / ah);
        if k > 0.0 && k.is_finite() {
            let tx = x0 + ((x1 - x0) - aw * k) / 2.0;
            let ty = y0 + ((y1 - y0) - ah * k) / 2.0;
            let id = pdf.add_art(drawn);
            ops += &format!(
                "q {} 0 0 {} {} {} cm /{ART} Do Q\n",
                f(k),
                f(k),
                f(tx),
                f(ty)
            );
            res.set("XObject", dictionary! { ART => id });
        }
    }

    let first = match name {
        Some(n) => format!("Digitally signed by {n}"),
        None => "Digitally signed".to_string(),
    };
    let date = format!("Date: {} {}", now.format("%Y.%m.%d %H:%M:%S"), zone(now));
    let [x0, y0, x1, y1] = text_box;
    let (tw, th) = (x1 - x0, y1 - y0);
    if tw > 2.0 && th > 2.0 {
        let (size, lines) = fit_text(&[&first, &date], tw, th);
        let lead = size * LEADING;
        let block = lead * lines.len() as f32;
        let mut y = y0 + (th + block) / 2.0 - size * ASCENT;
        let pen = pdf.pen(FontChoice::default(), &format!("{first}{date}"))?;
        ops += &format!("0 g BT /{FONT} {} Tf\n", f(size));
        for l in &lines {
            ops += &format!("1 0 0 1 {} {} Tm {} Tj\n", f(x0), f(y), pen.token(l));
            y -= lead;
        }
        ops += "ET\n";
        let font = pdf.pen_font(&pen);
        res.set("Font", dictionary! { FONT => font });
    }
    form.set("Resources", res);
    Ok(pdf.doc.add_object(Stream::new(form, ops.into_bytes())))
}

/// Line spacing and the height above the baseline, as shares of the text size.
const LEADING: f32 = 1.15;
const ASCENT: f32 = 0.8;

/// The largest text size (at most 10 points, at least 3) at which `paras`, each wrapped at
/// spaces, fit in `w` by `h`, and the wrapped lines.
fn fit_text(paras: &[&str], w: f32, h: f32) -> (f32, Vec<String>) {
    let mut size = 10.0;
    loop {
        let lines: Vec<String> = paras.iter().flat_map(|p| wrap(p, w, size)).collect();
        let fits = lines.len() as f32 * size * LEADING <= h
            && lines.iter().all(|l| text_width(l, size) <= w);
        if fits || size <= 3.0 {
            return (size, lines);
        }
        size -= 0.25;
    }
}

/// `text` broken at spaces into lines no wider than `w` at `size` where words allow.
fn wrap(text: &str, w: f32, size: f32) -> Vec<String> {
    let mut lines: Vec<String> = Vec::new();
    for word in text.split(' ').filter(|s| !s.is_empty()) {
        match lines.last_mut() {
            Some(l) if text_width(&format!("{l} {word}"), size) <= w => {
                l.push(' ');
                l.push_str(word);
            }
            _ => lines.push(word.to_string()),
        }
    }
    lines
}

/// A number for a content stream.
fn f(v: f32) -> String {
    let s = format!("{v:.3}");
    let s = s.trim_end_matches('0').trim_end_matches('.');
    if s == "-0" {
        "0".into()
    } else {
        s.into()
    }
}

// ---------------------------------------------------------------- finishing

/// Where the placeholders of the signature dictionary written at `at` are: the
/// `/Contents` hex string and the `/ByteRange` array, each as start and end.
struct Spots {
    contents: (usize, usize),
    byte_range: (usize, usize),
}

fn placeholders(bytes: &[u8], at: usize, id: ObjectId, room: usize) -> Option<Spots> {
    let mut lx = Lexer::new(bytes, at);
    let num = |t: Option<(usize, Tok, usize)>| match t {
        Some((_, Tok::Word(w), _)) => std::str::from_utf8(w).ok()?.parse::<u32>().ok(),
        _ => None,
    };
    if num(lx.next())? != id.0 || num(lx.next())? != u32::from(id.1) {
        return None;
    }
    if !matches!(lx.next()?, (_, Tok::Word(b"obj"), _))
        || !matches!(lx.next()?, (_, Tok::DictOpen, _))
    {
        return None;
    }
    let (mut contents, mut byte_range) = (None, None);
    let mut depth = 1usize;
    let mut last_name: Option<&[u8]> = None;
    while depth > 0 {
        let (start, tok, end) = lx.next()?;
        match tok {
            Tok::Hex if depth == 1 && last_name == Some(b"Contents") => {
                contents = Some((start, end));
            }
            Tok::ArrayOpen if depth == 1 && last_name == Some(b"ByteRange") => {
                let mut close = None;
                while let Some((_, t, e)) = lx.next() {
                    match t {
                        Tok::ArrayClose => {
                            close = Some(e);
                            break;
                        }
                        Tok::Word(_) => {}
                        _ => return None,
                    }
                }
                byte_range = Some((start, close?));
            }
            Tok::DictOpen | Tok::ArrayOpen => depth += 1,
            Tok::DictClose | Tok::ArrayClose => depth -= 1,
            _ => {}
        }
        last_name = match tok {
            Tok::Name(n) if depth == 1 => Some(n),
            _ => None,
        };
    }
    let (contents, byte_range) = (contents?, byte_range?);
    let hex_ok = contents.1 - contents.0 == 2 * room + 2
        && bytes[contents.0 + 1..contents.1 - 1]
            .iter()
            .all(|&b| b == b'0');
    let range_text = format!("[0 {PLACEHOLDER} {PLACEHOLDER} {PLACEHOLDER}]");
    let range_ok = &bytes[byte_range.0..byte_range.1] == range_text.as_bytes();
    (hex_ok && range_ok).then_some(Spots {
        contents,
        byte_range,
    })
}

/// Checks the signed file as any signed file is checked, before it is handed out: the new
/// signature valid, of the kind asked for, by the ID's certificate, covering the whole
/// file with nothing changed, and every earlier signature as it was.
fn self_check(
    base: &[u8],
    bytes: &[u8],
    before: &SignatureReport,
    field: &str,
    req: &SignRequest,
) -> Result<()> {
    let fail = |why: String| {
        anyhow!("the signed document didn't check correctly ({why}); nothing was written")
    };
    if !bytes.starts_with(base) {
        return Err(fail("earlier bytes changed".into()));
    }
    let after = verify_signatures(bytes, None, &TrustList::default())
        .map_err(|e| fail(format!("{e:#}")))?;
    if after.signatures.len() != before.signatures.len() + 1 {
        return Err(fail("it doesn't have exactly one more signature".into()));
    }
    let new = after
        .signatures
        .iter()
        .find(|s| s.fields.iter().any(|f| f == field))
        .ok_or_else(|| fail("the new signature wasn't found".into()))?;
    if new.status != SigStatus::Valid {
        return Err(fail(format!("the new signature is {:?}", new.status)));
    }
    if !new.covers_whole_file || !new.changes.is_empty() || !new.warnings.is_empty() {
        return Err(fail(format!(
            "the new signature has warnings or later changes: {:?} {:?}",
            new.warnings, new.changes
        )));
    }
    if new.signer.as_ref().map(|c| &c.der) != Some(&req.id.cert.der) {
        return Err(fail("the new signature names another certificate".into()));
    }
    let kind = match req.certify {
        Some(p) => SigKind::Certification(p),
        None => SigKind::Approval,
    };
    if new.kind != kind {
        return Err(fail(format!("the new signature is a {:?}", new.kind)));
    }
    let disallowed = newly_disallowed(before, &after).map_err(fail)?;
    if !disallowed.is_empty() && !req.accept_disallowed {
        return Err(DisallowedChanges {
            signers: disallowed,
        }
        .into());
    }
    Ok(())
}

/// Compares the earlier signatures (`before`) with the same signatures in a later version
/// of the file (`after`): each must be there and check as it did. Returns each signer
/// whose signature reports changes it doesn't allow that it didn't report before, with
/// those changes. `Err` says which signature didn't check as before.
pub(crate) fn newly_disallowed(
    before: &SignatureReport,
    after: &SignatureReport,
) -> std::result::Result<Vec<(String, Vec<String>)>, String> {
    let mut out = Vec::new();
    for old in &before.signatures {
        let now = after
            .signatures
            .iter()
            .find(|s| s.fields == old.fields && s.byte_range == old.byte_range)
            .ok_or_else(|| format!("the signature by {} is missing", signer_name(old)))?;
        if now.status != old.status {
            return Err(format!(
                "the signature by {} changed from {:?} to {:?}",
                signer_name(old),
                old.status,
                now.status
            ));
        }
        let known: HashSet<String> = old
            .changes
            .iter()
            .filter(|c| !c.allowed())
            .map(|c| c.to_string())
            .collect();
        let mut new: Vec<String> = Vec::new();
        for c in now.changes.iter().filter(|c| !c.allowed()) {
            let c = c.to_string();
            if !known.contains(&c) && !new.contains(&c) {
                new.push(c);
            }
        }
        if !new.is_empty() {
            out.push((signer_name(old), new));
        }
    }
    Ok(out)
}

/// Signing was refused because the edits going into the signed revision would make
/// earlier signatures report changes they don't allow. Nothing was written. The caller
/// can tell the signer, and sign again with [`SignRequest::accept_disallowed`] if they
/// go ahead. Found with `error.downcast_ref::<DisallowedChanges>()`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DisallowedChanges {
    /// Each earlier signer (quoted name, or "an earlier signer") and the changes their
    /// signature would report.
    pub signers: Vec<(String, Vec<String>)>,
}

impl std::fmt::Display for DisallowedChanges {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        let parts: Vec<String> = self
            .signers
            .iter()
            .map(|(who, what)| format!("{who}'s signature would report {}", what.join("; ")))
            .collect();
        write!(
            f,
            "signing now would make earlier signatures report changes they don't allow ({}); \
             nothing was written",
            parts.join(", and ")
        )
    }
}

impl std::error::Error for DisallowedChanges {}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    #[test]
    fn dates_are_written_as_acrobat_writes_them() {
        let west = FixedOffset::west_opt(4 * 3600).unwrap();
        let t = west.with_ymd_and_hms(2026, 10, 16, 14, 23, 5).unwrap();
        assert_eq!(pdf_date(t), "D:20261016142305-04'00'");
        let east = FixedOffset::east_opt(5 * 3600 + 30 * 60).unwrap();
        let t = east.with_ymd_and_hms(2026, 1, 2, 3, 4, 5).unwrap();
        assert_eq!(pdf_date(t), "D:20260102030405+05'30'");
        assert_eq!(
            zone(
                Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0)
                    .unwrap()
                    .fixed_offset()
            ),
            "+00'00'"
        );
    }

    #[test]
    fn text_is_wrapped_and_sized_to_fit() {
        let (size, lines) = fit_text(
            &["Digitally signed by Jane Doe", "Date: 2026.10.16"],
            80.0,
            40.0,
        );
        assert!(lines.len() >= 2, "{lines:?}");
        assert!((3.0..=10.0).contains(&size));
        assert!(lines.iter().all(|l| text_width(l, size) <= 80.0));
        assert!(lines.len() as f32 * size * LEADING <= 40.0);
        // Too little room: the smallest size, as many lines as needed.
        let (size, _) = fit_text(&["Digitally signed by Jane Doe"], 20.0, 4.0);
        assert_eq!(size, 3.0);
        assert_eq!(wrap("a  b", 1000.0, 10.0), ["a b"]);
    }

    #[test]
    fn names_the_font_cant_show_are_left_out() {
        let cert = |name: &str| CertInfo {
            subject: String::new(),
            common_name: Some(name.into()),
            organisation: None,
            email: None,
            issuer: String::new(),
            issuer_common_name: None,
            serial: String::new(),
            not_before: Utc::now(),
            not_after: Utc::now(),
            sha256: String::new(),
            self_issued: true,
            key: String::new(),
            key_bits: 0,
            der: Vec::new(),
        };
        assert_eq!(
            shown_name(&cert("Zoë Müller")).as_deref(),
            Some("Zoë Müller")
        );
        // Beyond WinAnsi, in the embedded face.
        assert_eq!(
            shown_name(&cert("Šárka Dvořák")).as_deref(),
            Some("Šárka Dvořák")
        );
        assert_eq!(shown_name(&cert("Ζωή")).as_deref(), Some("Ζωή"));
        assert_eq!(shown_name(&cert("Юлия")).as_deref(), Some("Юлия"));
        // Scripts the face can't write, and control characters.
        assert_eq!(shown_name(&cert("山田")), None);
        assert_eq!(shown_name(&cert("שרה")), None);
        assert_eq!(shown_name(&cert("Jo\u{7}e")), None);
        // Emoji, single or in sequences: the face has none.
        assert_eq!(shown_name(&cert("Jo 😀")), None);
        assert_eq!(shown_name(&cert("Jo 👍🏽")), None);
    }

    #[test]
    fn only_the_placeholders_in_the_signature_object_are_found() {
        let room = 4;
        let zeros = "0".repeat(2 * room);
        let range = format!("[0 {PLACEHOLDER} {PLACEHOLDER} {PLACEHOLDER}]");
        let decoy = format!("5 0 obj\n<</ByteRange{range}/Contents<{zeros}>>>\nendobj\n");
        let real = format!(
            "6 0 obj\n<</Type/Sig/Reference[<</Contents<{zeros}>>>]/ByteRange{range}/Contents<{zeros}>/M(x)>>\nendobj\n"
        );
        let bytes = format!("{decoy}{real}").into_bytes();
        let at = decoy.len();
        let s = placeholders(&bytes, at, (6, 0), room).unwrap();
        // The top-level entries, not the decoy object's or the nested dictionary's.
        let top = at + real.rfind("/Contents<").unwrap() + "/Contents".len();
        assert_eq!(s.contents, (top, top + 2 * room + 2));
        let br = at + real.find("/ByteRange[").unwrap() + "/ByteRange".len();
        assert_eq!(s.byte_range, (br, br + range.len()));
        // The wrong object, or the wrong size: not found.
        assert!(placeholders(&bytes, 0, (6, 0), room).is_none());
        assert!(placeholders(&bytes, at, (6, 0), room + 1).is_none());
    }
}

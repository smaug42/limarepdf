//! What changed after signing, and whether the signer allowed it: the allowed-changes
//! analysis (Acrobat's rules), the shadow and incremental-saving attack shapes, malformed
//! appended revisions, and the signed version of each signature.

mod sigkit;

use lopdf::{dictionary, Dictionary, Document, Object, ObjectId, Stream};
use mpe_core::{
    verify_signatures, Change, ChangeKind, DigitalSignature, Permissions, Refusal, SigStatus,
    SignatureReport, TrustList,
};
use sigkit::*;

fn report(b: &[u8]) -> SignatureReport {
    verify_signatures(b, None, &TrustList::default()).unwrap()
}

/// The signature in field `name`.
fn sig(b: &[u8], name: &str) -> DigitalSignature {
    report(b)
        .signatures
        .into_iter()
        .find(|s| s.fields.iter().any(|f| f == name))
        .unwrap_or_else(|| panic!("no signature in {name}"))
}

/// The changes after the signature in `Signature1`, as (kind, refusal).
fn changes(b: &[u8]) -> Vec<(ChangeKind, Option<Refusal>)> {
    summary(&sig(b, "Signature1").changes)
}

fn summary(c: &[Change]) -> Vec<(ChangeKind, Option<Refusal>)> {
    c.iter().map(|c| (c.kind.clone(), c.refused)).collect()
}

#[track_caller]
fn assert_refused(b: &[u8], want: &ChangeKind) {
    let got = changes(b);
    assert!(
        got.iter().any(|(k, r)| k == want && r.is_some()),
        "expected {want:?} refused, got {got:#?}"
    );
    assert!(!report(b).all_ok());
}

// ---------------------------------------------------------------- building documents

/// Edits the latest version in an appended revision.
fn edit(b: &[u8], f: impl FnOnce(&Document, &mut Document)) -> Vec<u8> {
    append_update(b, |inc| {
        let prev = inc.get_prev_documents().clone();
        f(&prev, &mut inc.new_document);
    })
}

fn root(d: &Document) -> ObjectId {
    d.trailer.get(b"Root").unwrap().as_reference().unwrap()
}

fn page(d: &Document) -> ObjectId {
    *d.get_pages().get(&1).unwrap()
}

fn dict(d: &Document, id: ObjectId) -> Dictionary {
    d.get_object(id).unwrap().as_dict().unwrap().clone()
}

/// The object of the field (or annotation) titled `name`.
fn titled(d: &Document, name: &str) -> ObjectId {
    *d.objects
        .iter()
        .find(|(_, o)| {
            o.as_dict()
                .ok()
                .and_then(|d| d.get(b"T").ok())
                .and_then(|t| t.as_str().ok())
                .is_some_and(|t| t == name.as_bytes())
        })
        .unwrap()
        .0
}

fn appearance(new: &mut Document, content: &str) -> ObjectId {
    new.add_object(Stream::new(
        dictionary! {
            "Type" => "XObject", "Subtype" => "Form",
            "BBox" => vec![0.into(), 0.into(), 100.into(), 20.into()],
        },
        content.as_bytes().to_vec(),
    ))
}

/// The page's annotations with `add` appended.
fn add_annot(prev: &Document, new: &mut Document, add: ObjectId) {
    let p = page(prev);
    let mut d = dict(prev, p);
    let mut annots = d
        .get(b"Annots")
        .ok()
        .and_then(|a| a.as_array().ok())
        .cloned()
        .unwrap_or_default();
    annots.push(add.into());
    d.set("Annots", annots);
    new.set_object(p, Object::Dictionary(d));
}

/// The base document with text fields named `names`.
fn form(names: &[&str]) -> Vec<u8> {
    edit(&base_pdf(), |prev, new| {
        let p = page(prev);
        let mut fields: Vec<Object> = Vec::new();
        for (i, n) in names.iter().enumerate() {
            let ap = appearance(new, "");
            let y = 100 + 30 * i as i64;
            let id = new.add_object(dictionary! {
                "Type" => "Annot", "Subtype" => "Widget", "FT" => "Tx",
                "T" => Object::string_literal(*n), "F" => 4, "P" => p,
                "Rect" => vec![150.into(), y.into(), 250.into(), (y + 20).into()],
                "AP" => dictionary! { "N" => ap },
            });
            fields.push(id.into());
        }
        let mut d = dict(prev, p);
        d.set("Annots", fields.clone());
        new.set_object(p, Object::Dictionary(d));
        let r = root(prev);
        let mut c = dict(prev, r);
        c.set(
            "AcroForm",
            dictionary! { "Fields" => fields, "DA" => Object::string_literal("/Helv 0 Tf 0 g") },
        );
        new.set_object(r, Object::Dictionary(c));
    })
}

/// Fills in the text field `name` with `value` and a new appearance.
fn fill(b: &[u8], name: &str, value: &str) -> Vec<u8> {
    edit(b, |prev, new| {
        let id = titled(prev, name);
        let mut d = dict(prev, id);
        let ap = appearance(new, &format!("BT ({value}) Tj ET"));
        d.set("V", Object::string_literal(value));
        d.set("AP", dictionary! { "N" => ap });
        new.set_object(id, Object::Dictionary(d));
    })
}

/// Adds a note on page 1, with `extra` entries.
fn comment(b: &[u8], extra: Dictionary) -> Vec<u8> {
    edit(b, |prev, new| {
        let mut d = dictionary! {
            "Type" => "Annot", "Subtype" => "Text", "Contents" => Object::string_literal("Hi"),
            "Rect" => vec![10.into(), 10.into(), 30.into(), 30.into()], "P" => page(prev),
        };
        d.extend(&extra);
        let id = new.add_object(d);
        add_annot(prev, new, id);
    })
}

fn approve(input: &[u8]) -> Vec<u8> {
    sign(input, &id("rsa3072"), &Field::default(), &Cms::default())
}

fn doc_mdp(p: i64) -> Object {
    vec![Object::from(dictionary! {
        "Type" => "SigRef", "TransformMethod" => "DocMDP",
        "TransformParams" => dictionary! { "Type" => "TransformParams", "V" => "1.2", "P" => p },
    })]
    .into()
}

fn certify(input: &[u8], p: i64) -> Vec<u8> {
    let f = Field {
        reference: Some(doc_mdp(p)),
        perms: true,
        ..Field::default()
    };
    sign(input, &id("rsa3072"), &f, &Cms::default())
}

/// Signs `input` in a new field `name` with another key.
fn countersign(input: &[u8], name: &'static str, invisible: bool) -> Vec<u8> {
    let f = Field {
        name,
        invisible,
        ..Field::default()
    };
    sign(input, &id("p256"), &f, &Cms::default())
}

fn timestamp(input: &[u8], invisible: bool) -> Vec<u8> {
    let f = Field {
        name: "Timestamp1",
        sub_filter: "ETSI.RFC3161",
        doc_timestamp: true,
        invisible,
        m: None,
        reason: None,
        ..Field::default()
    };
    sign_with(input, &f, |r| {
        timestamp_token(&tsa(), r, "20260601130000Z", Hash::Sha256)
    })
}

fn lock(action: &str, fields: &[&str], p: Option<i64>) -> Object {
    let mut params = dictionary! {
        "Type" => "TransformParams", "V" => "1.2", "Action" => Object::Name(action.into()),
        "Fields" => fields.iter().map(|f| Object::string_literal(*f)).collect::<Vec<_>>(),
    };
    if let Some(p) = p {
        params.set("P", p);
    }
    vec![Object::from(dictionary! {
        "Type" => "SigRef", "TransformMethod" => "FieldMDP", "TransformParams" => params,
    })]
    .into()
}

fn locked_by(input: &[u8], reference: Object) -> Vec<u8> {
    let f = Field {
        reference: Some(reference),
        ..Field::default()
    };
    sign(input, &id("rsa3072"), &f, &Cms::default())
}

/// The page's content stream.
fn contents(b: &[u8]) -> ObjectId {
    let d = Document::load_mem(b).unwrap();
    dict(&d, page(&d))
        .get(b"Contents")
        .unwrap()
        .as_reference()
        .unwrap()
}

// ---------------------------------------------------------------- allowed changes

#[test]
fn nothing_changed_after_signing() {
    let b = approve(&base_pdf());
    let s = sig(&b, "Signature1");
    assert!(s.changes.is_empty());
    assert_eq!(s.signed_len, Some(b.len()));
    assert!(report(&b).all_ok());
    // White space after the end changes nothing.
    let mut w = b.clone();
    w.extend_from_slice(b"\r\n\n");
    assert!(sig(&w, "Signature1").changes.is_empty());
}

#[test]
fn filling_in_a_form_is_allowed() {
    let b = fill(&approve(&form(&["Name", "Date"])), "Name", "Jane");
    let s = sig(&b, "Signature1");
    assert_eq!(
        summary(&s.changes),
        [(ChangeKind::FieldFilled("Name".into()), None)]
    );
    assert_eq!(s.changes[0].page, Some(0));
    assert_eq!(s.status, SigStatus::Valid);
    assert!(report(&b).all_ok());
}

#[test]
fn another_signature_is_allowed() {
    let b = countersign(&approve(&base_pdf()), "Signature2", false);
    assert_eq!(
        changes(&b),
        [(ChangeKind::Signed("Signature2".into()), None)]
    );
    assert!(sig(&b, "Signature2").changes.is_empty());
    assert!(report(&b).all_ok());

    // Signing an empty signature field added by the first signer.
    let with_field = edit(&approve(&base_pdf()), |prev, new| {
        let id = new.add_object(dictionary! {
            "Type" => "Annot", "Subtype" => "Widget", "FT" => "Sig",
            "T" => Object::string_literal("Approver"), "P" => page(prev),
            "Rect" => vec![0.into(), 100.into(), 50.into(), 120.into()],
        });
        add_annot(prev, new, id);
        let r = root(prev);
        let mut c = dict(prev, r);
        let mut form = c.get(b"AcroForm").unwrap().as_dict().unwrap().clone();
        let mut fields = form.get(b"Fields").unwrap().as_array().unwrap().clone();
        fields.push(id.into());
        form.set("Fields", fields);
        c.set("AcroForm", form);
        new.set_object(r, Object::Dictionary(c));
    });
    assert_eq!(
        changes(&with_field),
        [(ChangeKind::SignatureFieldAdded("Approver".into()), None)]
    );
    let f = Field {
        name: "Approver",
        existing: true,
        ..Field::default()
    };
    let b = sign(&with_field, &id("p256"), &f, &Cms::default());
    assert_eq!(changes(&b), [(ChangeKind::Signed("Approver".into()), None)]);
    assert!(report(&b).all_ok());
}

#[test]
fn comments_are_allowed_unless_certified_without_them() {
    let added = comment(&approve(&base_pdf()), Dictionary::new());
    assert_eq!(changes(&added), [(ChangeKind::CommentAdded, None)]);
    // Removing a comment made before signing, and editing one, are comment changes too.
    let removed = edit(
        &approve(&comment(&base_pdf(), Dictionary::new())),
        |prev, new| {
            let p = page(prev);
            let mut d = dict(prev, p);
            let mut annots = d.get(b"Annots").unwrap().as_array().unwrap().clone();
            // The comment, then the signature's widget.
            let widget = annots.pop().unwrap();
            annots[0] = widget;
            d.set("Annots", annots);
            new.set_object(p, Object::Dictionary(d));
        },
    );
    assert_eq!(changes(&removed), [(ChangeKind::CommentRemoved, None)]);
    // Taking the signature's own widget off the page hides the signature.
    let hidden = edit(&approve(&base_pdf()), |prev, new| {
        let p = page(prev);
        let mut d = dict(prev, p);
        d.set("Annots", Vec::<Object>::new());
        new.set_object(p, Object::Dictionary(d));
    });
    assert_refused(&hidden, &ChangeKind::SignatureChanged("Signature1".into()));
    let edited = edit(
        &approve(&comment(&base_pdf(), Dictionary::new())),
        |prev, new| {
            let id = *prev
                .objects
                .iter()
                .find(|(_, o)| {
                    o.as_dict()
                        .is_ok_and(|d| d.has(b"Contents") && d.has(b"Rect"))
                })
                .unwrap()
                .0;
            let mut d = dict(prev, id);
            d.set("Contents", Object::string_literal("Changed"));
            new.set_object(id, Object::Dictionary(d));
        },
    );
    assert_eq!(changes(&edited), [(ChangeKind::CommentChanged, None)]);

    let p2 = comment(&certify(&base_pdf(), 2), Dictionary::new());
    assert_eq!(
        changes(&p2),
        [(
            ChangeKind::CommentAdded,
            Some(Refusal::Certification(Permissions::FormFilling))
        )]
    );
    let p3 = comment(&certify(&base_pdf(), 3), Dictionary::new());
    assert_eq!(changes(&p3), [(ChangeKind::CommentAdded, None)]);
}

#[test]
fn certification_limits() {
    // No changes: validation data and an invisible document timestamp only.
    let p1 = certify(&form(&["Name"]), 1);
    let dss = edit(&p1, |prev, new| {
        let cert = new.add_object(Stream::new(Dictionary::new(), cert_der("ca")));
        let r = root(prev);
        let mut c = dict(prev, r);
        c.set("DSS", dictionary! { "Certs" => vec![Object::from(cert)] });
        new.set_object(r, Object::Dictionary(c));
    });
    let stamped = timestamp(&dss, true);
    assert_eq!(
        changes(&stamped),
        [
            (ChangeKind::DocumentTimestamp("Timestamp1".into()), None),
            (ChangeKind::ValidationData, None),
        ]
    );
    assert!(report(&stamped).all_ok());
    let visible = timestamp(&p1, false);
    assert!(changes(&visible)[0].1.is_some());
    let filled = fill(&p1, "Name", "Jane");
    assert_eq!(
        changes(&filled),
        [(
            ChangeKind::FieldFilled("Name".into()),
            Some(Refusal::Certification(Permissions::NoChanges))
        )]
    );

    // Form filling and signing: a new signature field must be invisible.
    let p2 = certify(&form(&["Name"]), 2);
    assert!(report(&fill(&p2, "Name", "Jane")).all_ok());
    assert!(report(&countersign(&p2, "Signature2", true)).all_ok());
    assert_eq!(
        changes(&countersign(&p2, "Signature2", false)),
        [(
            ChangeKind::Signed("Signature2".into()),
            Some(Refusal::Certification(Permissions::FormFilling))
        )]
    );
}

#[test]
fn field_locks() {
    let b = locked_by(&form(&["Name", "Date"]), lock("Include", &["Name"], None));
    assert_eq!(
        changes(&fill(&b, "Name", "Jane")),
        [(
            ChangeKind::FieldFilled("Name".into()),
            Some(Refusal::Locked)
        )]
    );
    assert_eq!(
        changes(&fill(&b, "Date", "Today")),
        [(ChangeKind::FieldFilled("Date".into()), None)]
    );
    let all_but = locked_by(&form(&["Name", "Date"]), lock("Exclude", &["Date"], None));
    assert!(!report(&fill(&all_but, "Name", "Jane")).all_ok());
    assert!(report(&fill(&all_but, "Date", "Today")).all_ok());
    // "Lock document": nothing may change.
    let doc = locked_by(&base_pdf(), lock("All", &[], Some(1)));
    assert_eq!(
        changes(&comment(&doc, Dictionary::new())),
        [(ChangeKind::CommentAdded, Some(Refusal::Locked))]
    );
}

#[test]
fn document_information_may_change() {
    let b = approve(&base_pdf());
    let num = Document::load_mem(&b).unwrap().max_id + 1;
    let b = append_raw(
        &b,
        &[(num, b"<< /Title (New title) >>")],
        &[],
        &[],
        &format!("/Info {num} 0 R"),
    );
    assert_eq!(changes(&b), [(ChangeKind::Metadata, None)]);
}

#[test]
fn rewriting_strings_differently_is_no_change() {
    let b = approve(&base_pdf());
    let d = Document::load_mem(&b).unwrap();
    let field = titled(&d, "Signature1");
    let value = sig_id(&b, "Signature1");
    let p = page(&d);
    // The same field with its name as a hex string.
    let body = format!(
        "<< /Type /Annot /Subtype /Widget /FT /Sig /T <5369676E617475726531> /V {} 0 R \
         /F 132 /Rect [20 20 120 60] /P {} 0 R >>",
        value.0, p.0
    );
    let b = append_raw(&b, &[(field.0, body.as_bytes())], &[], &[], "");
    let s = sig(&b, "Signature1");
    assert!(!s.covers_whole_file);
    assert!(s.changes.is_empty(), "{:#?}", s.changes);
}

// ---------------------------------------------------------------- disallowed changes

#[test]
fn page_content_changes_are_refused() {
    let b = approve(&base_pdf());
    let c = contents(&b);
    let in_place = edit(&b, |_, new| {
        new.set_object(
            c,
            Stream::new(Dictionary::new(), b"0 0 m 100 100 l S".to_vec()),
        );
    });
    assert_refused(&in_place, &ChangeKind::PageContent);
    let replaced = edit(&b, |prev, new| {
        let s = new.add_object(Stream::new(Dictionary::new(), b"0 0 m 1 1 l S".to_vec()));
        let p = page(prev);
        let mut d = dict(prev, p);
        d.set("Contents", s);
        new.set_object(p, Object::Dictionary(d));
    });
    assert_refused(&replaced, &ChangeKind::PageContent);
    let rotated = edit(&b, |prev, new| {
        let p = page(prev);
        let mut d = dict(prev, p);
        d.set("Rotate", 90);
        new.set_object(p, Object::Dictionary(d));
    });
    assert_refused(&rotated, &ChangeKind::PageContent);
}

#[test]
fn adding_a_page_is_refused() {
    let b = edit(&approve(&base_pdf()), |prev, new| {
        let pages = dict(prev, root(prev))
            .get(b"Pages")
            .unwrap()
            .as_reference()
            .unwrap();
        let mut d = dict(prev, pages);
        let extra = new.add_object(dictionary! {
            "Type" => "Page", "Parent" => pages,
            "MediaBox" => vec![0.into(), 0.into(), 300.into(), 200.into()],
        });
        let mut kids = d.get(b"Kids").unwrap().as_array().unwrap().clone();
        kids.push(extra.into());
        d.set("Kids", kids);
        d.set("Count", 2);
        new.set_object(pages, Object::Dictionary(d));
    });
    assert_refused(&b, &ChangeKind::PagesChanged);
}

#[test]
fn changing_a_signature_is_refused() {
    let b = approve(&base_pdf());
    let id = sig_id(&b, "Signature1");
    let reason = edit(&b, |prev, new| {
        let mut d = dict(prev, id);
        d.set("Reason", Object::string_literal("Forged"));
        new.set_object(id, Object::Dictionary(d));
    });
    assert_refused(&reason, &ChangeKind::SignatureChanged("Signature1".into()));
    let look = edit(&b, |prev, new| {
        let f = titled(prev, "Signature1");
        let ap = appearance(new, "BT (Someone else) Tj ET");
        let mut d = dict(prev, f);
        d.set("AP", dictionary! { "N" => ap });
        new.set_object(f, Object::Dictionary(d));
    });
    assert_refused(&look, &ChangeKind::SignatureChanged("Signature1".into()));
}

#[test]
fn new_fields_and_field_changes_are_refused() {
    let b = approve(&form(&["Name"]));
    let added = edit(&b, |prev, new| {
        let id = new.add_object(dictionary! {
            "Type" => "Annot", "Subtype" => "Widget", "FT" => "Tx",
            "T" => Object::string_literal("Extra"), "P" => page(prev),
            "Rect" => vec![0.into(), 0.into(), 50.into(), 20.into()],
        });
        add_annot(prev, new, id);
        let r = root(prev);
        let mut c = dict(prev, r);
        let mut form = c.get(b"AcroForm").unwrap().as_dict().unwrap().clone();
        let mut fields = form.get(b"Fields").unwrap().as_array().unwrap().clone();
        fields.push(id.into());
        form.set("Fields", fields);
        c.set("AcroForm", form);
        new.set_object(r, Object::Dictionary(c));
    });
    assert_refused(&added, &ChangeKind::FieldAdded("Extra".into()));
    // Moving a field, or hiding it, isn't filling it in.
    let moved = edit(&b, |prev, new| {
        let f = titled(prev, "Name");
        let mut d = dict(prev, f);
        d.set("F", 6);
        new.set_object(f, Object::Dictionary(d));
    });
    assert_refused(&moved, &ChangeKind::FieldChanged("Name".into()));
}

#[test]
fn actions_and_scripts_are_refused() {
    let b = approve(&base_pdf());
    let js = dictionary! { "S" => "JavaScript", "JS" => Object::string_literal("app.alert(1)") };
    let open = edit(&b, |prev, new| {
        let r = root(prev);
        let mut c = dict(prev, r);
        c.set("OpenAction", js.clone());
        new.set_object(r, Object::Dictionary(c));
    });
    assert_refused(&open, &ChangeKind::Actions);
    // A comment that runs a script when the page opens isn't just a comment.
    let note = comment(&b, dictionary! { "AA" => dictionary! { "PO" => js } });
    assert_refused(&note, &ChangeKind::Actions);
    // A link isn't a comment.
    let link = edit(&b, |prev, new| {
        let id = new.add_object(dictionary! {
            "Type" => "Annot", "Subtype" => "Link",
            "Rect" => vec![0.into(), 0.into(), 300.into(), 200.into()],
            "A" => dictionary! { "S" => "URI", "URI" => Object::string_literal("https://example.com") },
        });
        add_annot(prev, new, id);
    });
    assert_refused(&link, &ChangeKind::AnnotationChanged);
}

#[test]
fn a_change_undone_before_the_next_signature_still_counts() {
    let b = approve(&base_pdf());
    let rotate = |b: &[u8], r: i64| {
        edit(b, |prev, new| {
            let p = page(prev);
            let mut d = dict(prev, p);
            d.set("Rotate", r);
            new.set_object(p, Object::Dictionary(d));
        })
    };
    // Turned, countersigned, turned back: the file ends as the first signer signed it,
    // but the second signer signed something else.
    let b = rotate(&countersign(&rotate(&b, 90), "Signature2", false), 0);
    assert_refused(&b, &ChangeKind::PageContent);
    assert!(!sig(&b, "Signature2").changes_allowed());
}

// ---------------------------------------------------------------- attacks

/// Shadow attack, hide: an update that only frees an object (lopdf itself ignores free
/// entries, so this is caught by MPE's own reader).
#[test]
fn freeing_an_object_still_in_use_is_refused() {
    let b = approve(&base_pdf());
    let c = contents(&b);
    let freed = append_raw(&b, &[], &[c.0], &[], "");
    let got = changes(&freed);
    assert!(
        matches!(&got[..], [(ChangeKind::Unrecognised(w), Some(Refusal::Never))] if w.contains("deleted but is still used")),
        "{got:#?}"
    );
}

/// Shadow attack, hide-and-replace: an update re-pointing an object at bytes inside the
/// signed part (an unused object planted before signing).
#[test]
fn pointing_back_into_the_signed_part_is_refused() {
    let b = approve(&base_pdf());
    let c = contents(&b);
    let moved = append_raw(&b, &[], &[], &[(c.0, 9)], "");
    assert!(matches!(
        &changes(&moved)[..],
        [(ChangeKind::Malformed(w), _)] if w.contains("outside the revision")
    ));
    // An entry pointing at another object's header (DSS hide.pdf). The kit starts the
    // revision on a new line.
    let at = b.len() + usize::from(!b.ends_with(b"\n"));
    let num = Document::load_mem(&b).unwrap().max_id + 1;
    let wrong = append_raw(&b, &[(num, b"<< >>")], &[], &[(c.0, at)], "");
    assert!(matches!(
        &changes(&wrong)[..],
        [(ChangeKind::Malformed(w), _)] if w.contains("isn't where")
    ));
}

#[test]
fn data_after_the_last_revision_is_refused() {
    let mut b = fill(&approve(&form(&["Name"])), "Name", "Jane");
    b.extend_from_slice(b"9 0 obj << /Junk true >> endobj\n");
    assert!(matches!(
        &changes(&b)[..],
        [(ChangeKind::Malformed(w), _)] if w.contains("complete revision")
    ));
}

/// Shadow attack shape: an object the page draws is also listed as an annotation, so
/// changing it might pass as "comment changed".
#[test]
fn page_content_posing_as_a_comment_is_still_page_content() {
    let base = edit(&base_pdf(), |prev, new| {
        let p = page(prev);
        let c = dict(prev, p)
            .get(b"Contents")
            .unwrap()
            .as_reference()
            .unwrap();
        let mut d = dict(prev, p);
        d.set("Annots", vec![Object::from(c)]);
        new.set_object(p, Object::Dictionary(d));
    });
    let b = approve(&base);
    let c = contents(&b);
    let changed = edit(&b, |_, new| {
        new.set_object(c, Stream::new(Dictionary::new(), b"0 0 m 9 9 l S".to_vec()));
    });
    assert_refused(&changed, &ChangeKind::PageContent);
}

/// An object the signed version referred to but didn't have, filled in later.
#[test]
fn filling_in_a_missing_object_is_refused() {
    let base = edit(&base_pdf(), |prev, new| {
        let p = page(prev);
        let mut d = dict(prev, p);
        d.set("Thumb", Object::Reference((900, 0)));
        new.set_object(p, Object::Dictionary(d));
    });
    let b = approve(&base);
    let b = append_raw(&b, &[(900, b"<< /Late true >>")], &[], &[], "");
    let got = changes(&b);
    assert!(
        got.iter()
            .any(|(k, r)| matches!(k, ChangeKind::Unrecognised(_)) && r.is_some()),
        "{got:#?}"
    );
}

/// Truncated or damaged appended data never panics and never passes as unchanged.
#[test]
fn damaged_appended_data_is_never_clean() {
    let b = fill(&approve(&form(&["Name"])), "Name", "Jane");
    let end = sig(&b, "Signature1").signed_len.unwrap();
    let clean = |x: &[u8]| -> Option<bool> {
        let r = verify_signatures(x, None, &TrustList::default()).ok()?;
        let s = r.signatures.iter().find(|s| s.fields == ["Signature1"])?;
        Some(s.status == SigStatus::Valid && s.changes.is_empty())
    };
    for cut in (end + 1..b.len()).step_by(3) {
        let x = &b[..cut];
        if clean(x) == Some(true) {
            assert!(x[end..].iter().all(u8::is_ascii_whitespace), "cut at {cut}");
        }
    }
    for at in (end..b.len()).step_by(5) {
        for flip in [0x01u8, 0x20, 0x80] {
            let mut x = b.clone();
            x[at] ^= flip;
            assert_ne!(clean(&x), Some(true), "byte {at} ^ {flip:#x}");
        }
    }
}

// ---------------------------------------------------------------- signed versions

#[test]
fn the_signed_version_is_the_file_as_signed() {
    let signed = approve(&form(&["Name"]));
    let b = comment(&fill(&signed, "Name", "Jane"), Dictionary::new());
    let s = sig(&b, "Signature1");
    let len = s.signed_len.unwrap();
    assert_eq!(&b[..len], &signed[..]);
    let then = sig(&b[..len], "Signature1");
    assert_eq!(then.status, SigStatus::Valid);
    assert!(then.covers_whole_file && then.changes.is_empty());
}

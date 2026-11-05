//! The tests' one meaning of "the document is unchanged" (cycle plan section 3, rule 3), for
//! checking that an operation which fails leaves the document as it was. Tests only: built
//! with `cfg(test)` or the `test-seams` feature, never in a release build.
//!
//! The document's objects and its header (version, binary mark, trailer) are compared as
//! written, by the undo history's own comparison ([`history::same`]); so are the [`Pdf`]'s
//! fields an edit's history doesn't hold: the encryption to write, lopdf's encryption state,
//! whether a password was needed, why other apps may read the file differently, and whether
//! it was repaired. Left out on purpose: a raised highest object number (kept by
//! [`Pdf::go_back`] and [`Pdf::restore`], so an object number isn't reused in a session), the
//! fonts LimarePDF embedded (`made_fonts`, kept by `go_back`) and the cache of streams found
//! fine (`checked`).

use crate::history::{same, same_dict};
use crate::{Encryption, Pdf};
use lopdf::{Document, ObjectId};

/// A document's state before an operation, for [`Pdf::state_since`].
pub struct StateBefore {
    doc: Document,
    encryption: Option<Encryption>,
    encryption_state: String,
    password_needed: bool,
    differs: Option<String>,
    repaired: bool,
}

/// One way a document differs from its [`StateBefore`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum StateDifference {
    /// An object added, removed or changed.
    Object(ObjectId),
    Version,
    BinaryMark,
    Trailer,
    /// The encryption the document will be written with.
    Encryption,
    /// lopdf's state for the encryption the document was read with.
    EncryptionState,
    PasswordNeeded,
    Differs,
    Repaired,
    /// The highest object number lowered (raising it is allowed).
    MaxIdLowered,
}

impl Pdf {
    /// The document's state now, before an operation.
    pub fn state_before(&self) -> StateBefore {
        StateBefore {
            doc: self.doc.clone(),
            encryption: self.encryption.clone(),
            encryption_state: format!("{:?}", self.doc.encryption_state),
            password_needed: self.password_needed,
            differs: self.differs.clone(),
            repaired: self.repaired,
        }
    }

    /// How the document differs from `before`: empty when it is unchanged.
    pub fn state_since(&self, before: &StateBefore) -> Vec<StateDifference> {
        use StateDifference as D;
        let (was, now) = (&before.doc, &self.doc);
        let mut out: Vec<D> = (was.objects.keys().chain(now.objects.keys()))
            .filter(|id| match (was.objects.get(id), now.objects.get(id)) {
                (Some(a), Some(b)) => !same(a, b),
                _ => true,
            })
            .copied()
            .collect::<std::collections::BTreeSet<_>>()
            .into_iter()
            .map(D::Object)
            .collect();
        let mut add = |differs: bool, d: D| {
            if differs {
                out.push(d);
            }
        };
        add(was.version != now.version, D::Version);
        add(was.binary_mark != now.binary_mark, D::BinaryMark);
        add(!same_dict(&was.trailer, &now.trailer), D::Trailer);
        add(now.max_id < was.max_id, D::MaxIdLowered);
        add(before.encryption != self.encryption, D::Encryption);
        add(
            before.encryption_state != format!("{:?}", now.encryption_state),
            D::EncryptionState,
        );
        add(
            before.password_needed != self.password_needed,
            D::PasswordNeeded,
        );
        add(before.differs != self.differs, D::Differs);
        add(before.repaired != self.repaired, D::Repaired);
        out
    }
}

#[cfg(test)]
mod tests {
    use super::StateDifference as D;
    use crate::{Cipher, Encryption, Pdf, Restrictions};
    use lopdf::{dictionary, Object};

    #[test]
    fn an_untouched_document_is_unchanged() {
        let p = Pdf::new();
        assert_eq!(p.state_since(&p.state_before()), []);
    }

    #[test]
    fn a_changed_trailer_entry_is_reported() {
        let mut p = Pdf::new();
        let before = p.state_before();
        p.doc.trailer.set("ID", Object::Array(vec![]));
        assert_eq!(p.state_since(&before), [D::Trailer]);
    }

    #[test]
    fn a_new_info_dictionary_is_reported() {
        let mut p = Pdf::new();
        let before = p.state_before();
        let info = p
            .doc
            .add_object(dictionary! { "Title" => Object::string_literal("x") });
        p.doc.trailer.set("Info", info);
        assert_eq!(p.state_since(&before), [D::Object(info), D::Trailer]);
    }

    #[test]
    fn a_changed_info_entry_is_reported() {
        let mut p = Pdf::new();
        let info = p
            .doc
            .add_object(dictionary! { "Title" => Object::string_literal("x") });
        p.doc.trailer.set("Info", info);
        let before = p.state_before();
        p.doc
            .get_dictionary_mut(info)
            .unwrap()
            .set("Title", Object::string_literal("y"));
        assert_eq!(p.state_since(&before), [D::Object(info)]);
    }

    #[test]
    fn a_new_and_a_removed_object_are_reported() {
        let mut p = Pdf::new();
        let before = p.state_before();
        let added = p.doc.add_object(Object::Integer(1));
        let root = p.doc.trailer.get(b"Root").unwrap().as_reference().unwrap();
        p.doc.objects.remove(&root);
        assert_eq!(p.state_since(&before), [D::Object(root), D::Object(added)]);
    }

    #[test]
    fn a_dictionary_reordered_is_a_change() {
        let mut p = Pdf::new();
        let id = p.doc.add_object(dictionary! { "A" => 1, "B" => 2 });
        let before = p.state_before();
        *p.doc.objects.get_mut(&id).unwrap() =
            Object::Dictionary(dictionary! { "B" => 2, "A" => 1 });
        assert_eq!(p.state_since(&before), [D::Object(id)]);
    }

    #[test]
    fn a_changed_encryption_setting_is_reported() {
        let mut p = Pdf::new();
        let before = p.state_before();
        p.encryption = Some(Encryption {
            user_password: String::new(),
            owner_password: "owner".into(),
            cipher: Cipher::Aes256,
            restrictions: Restrictions::default(),
        });
        assert_eq!(p.state_since(&before), [D::Encryption]);
    }

    #[test]
    fn a_changed_encryption_state_is_reported() {
        let mut p = Pdf::new();
        let before = p.state_before();
        p.doc.encryption_state = Some(lopdf::EncryptionState::default());
        assert_eq!(p.state_since(&before), [D::EncryptionState]);
    }

    #[test]
    fn the_fields_outside_the_history_are_each_reported() {
        let mut p = Pdf::new();
        let before = p.state_before();
        p.password_needed = true;
        p.differs = Some("a reason".into());
        p.repaired = true;
        p.doc.version = "2.0".into();
        assert_eq!(
            p.state_since(&before),
            [D::Version, D::PasswordNeeded, D::Differs, D::Repaired]
        );
    }

    #[test]
    fn a_raised_highest_object_number_alone_is_not_a_change() {
        let mut p = Pdf::new();
        let before = p.state_before();
        p.doc.max_id += 5;
        assert_eq!(p.state_since(&before), []);
        p.doc.max_id -= 6;
        assert_eq!(p.state_since(&before), [D::MaxIdLowered]);
    }

    #[test]
    fn the_stated_exceptions_are_not_changes() {
        let mut p = Pdf::new();
        let before = p.state_before();
        p.made_fonts.insert((7, 0));
        assert_eq!(p.state_since(&before), []);
    }
}

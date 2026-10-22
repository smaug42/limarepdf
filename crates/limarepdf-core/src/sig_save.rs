//! Saving a signed document without breaking its signatures: the edits made since it was
//! opened go into a revision appended after its bytes (see [`crate::incremental`]), as
//! Acrobat saves signed files. Before the result is handed out every signature is checked
//! again: each must check as it did before, and the changes it reports are compared with
//! what it reported before, so the caller can ask first when the edits are changes a
//! signer didn't allow. See docs/signatures.md.
//!
//! Everything in the opened file stays in it, so whatever the edits delete can still be
//! read from the earlier version. Edits meant to remove information for good (applied
//! redactions, removed hidden information) must be saved by rewriting the file instead,
//! which breaks the signatures; the callers decide that.

use crate::sig_sign::{newly_disallowed, DisallowedChanges};
use crate::{incremental, verify_signatures, Pdf, Result, TrustList};
use anyhow::{anyhow, bail};

/// A signed document with its edits appended, from [`append_edits`].
#[derive(Clone, Debug)]
pub struct AppendedEdits {
    /// The file to write: the opened bytes, then the new revision. Just the opened bytes
    /// when nothing changed.
    pub bytes: Vec<u8>,
    /// How many objects the new revision holds; 0 when nothing changed, or only the
    /// trailer did (say, the document information was removed).
    pub objects: usize,
    /// The earlier signers whose signatures will report changes they don't allow, which
    /// they didn't report before. The file can still be saved (each signature stays valid
    /// for what it signed, and readers list the changes), but the caller asks first.
    pub disallowed: Option<DisallowedChanges>,
}

/// `original` (the file as it was opened or last saved) with the edits that make it
/// `edited` added as a new revision. Every signature in the result has been checked: each
/// one checks as it did in `original`, and [`AppendedEdits::disallowed`] names those that
/// now report changes they don't allow. An error says why the edits can't be added this
/// way (the file is encrypted or damaged, a password was set, an edit removes a
/// signature, or the result didn't read back correctly); nothing is returned then.
pub fn append_edits(original: &[u8], edited: &Pdf) -> Result<AppendedEdits> {
    let opened = Pdf::from_bytes(original, None).map_err(|e| {
        if format!("{e:#}").contains("encrypted") {
            anyhow!("{ENCRYPTED}")
        } else {
            e
        }
    })?;
    if opened.encrypted() {
        bail!("{ENCRYPTED}");
    }
    if edited.encrypted() {
        bail!(
            "a password was set for it, and LimarePDF can add a password only by writing the whole \
             file again"
        );
    }
    // Also refuses a damaged file, unless nothing changed.
    let appended = incremental::append_revision(original, &edited.doc)?;
    if appended.bytes.len() == original.len() {
        return Ok(AppendedEdits {
            bytes: appended.bytes,
            objects: 0,
            disallowed: None,
        });
    }
    let before = verify_signatures(original, None, &TrustList::default())?;

    let fail = |why: String| {
        anyhow!("the saved document didn't check correctly ({why}); nothing was written")
    };
    let after = verify_signatures(&appended.bytes, None, &TrustList::default())
        .map_err(|e| fail(format!("{e:#}")))?;
    let (was, now) = (before.signatures.len(), after.signatures.len());
    if now < was {
        bail!(
            "the changes remove {} of its signatures (such as flattening the form)",
            was - now
        );
    }
    if now != was {
        return Err(fail(format!("it has {now} signatures instead of {was}")));
    }
    let signers = newly_disallowed(&before, &after).map_err(fail)?;
    Ok(AppendedEdits {
        objects: appended.offsets.len(),
        bytes: appended.bytes,
        disallowed: (!signers.is_empty()).then_some(DisallowedChanges { signers }),
    })
}

const ENCRYPTED: &str =
    "the document is encrypted (password-protected), and LimarePDF can't add to \
                         encrypted documents";

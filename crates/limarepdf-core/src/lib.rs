//! limarepdf-core: pure-Rust PDF document operations and rendering.
//!
//! * Editing and structure: [`lopdf`]
//! * Rasterisation: [`hayro`] (CPU, no unsafe, no C/C++ dependencies)

mod annot_ap;
mod annot_write;
mod annots;
mod asn1;
mod atomic;
mod base_encodings;
mod blank;
mod cms_check;
mod cms_write;
pub mod disk;
mod display;
mod doc;
mod edit;
mod embed;
mod excerpt;
mod fonts;
mod form_ap;
mod form_flatten;
mod form_write;
mod forms;
mod history;
mod ids;
mod imgs;
mod incremental;
mod inline;
mod installed_fonts;
mod labels;
mod layout;
mod links;
mod matrix;
mod objects;
mod ocr;
mod outline;
mod overlay;
mod own_encoding;
mod own_font;
pub mod page_colours;
mod pagerange;
mod pages_gone;
mod pkcs12;
mod preview_font;
pub mod print;
#[cfg(windows)]
mod print_win;
mod radicals;
mod reading;
mod redact;
mod redact_img;
pub mod render;
mod repair;
mod sanitise;
mod scan;
mod security;
mod sig_changes;
mod sig_save;
mod sig_sign;
mod sig_verify;
mod sign;
mod stamps;
mod std14;
mod text;
mod tounicode;
mod trees;
mod trust;
mod typefaces;
mod viewprefs;

pub use annots::{Annot, AnnotKind, ShapeLook, TextStyle, MAX_ANNOTS};
pub use atomic::NotConfirmed;
pub use blank::BLANK_TOLERANCE;
pub use cms_check::CertInfo;
pub use display::{display_line, display_text, local_date, local_time, quoted_list};
pub use doc::{existing, part_file_name, write_file, Info, Pdf, SplitBy};
pub use edit::{KeptPiece, PartFont, PathStyle, Shape, TextEdit, TextPart};
pub use embed::{
    bundled_face, emoji_sequence, installed_could_write, liberation, refusal, unwritten_script,
    writable, LIBERATION,
};
pub use fonts::{winansi_byte, Family, FontChoice};
pub use form_ap::BoxSetting;
pub use forms::{Field, FieldKind, FieldValue, Widget};
pub use history::{Before, Restore};
pub use ids::{
    password_problem, DigitalId, IdList, IdStore, NewId, Stored, UnlockedId, WrongPassword,
    MIN_PASSWORD,
};
pub use imgs::{image_dimensions, picture_preview, ImageImport};
pub use labels::{LabelRange, LabelStyle};
pub use layout::{fallback_faces, fallbacks, set_in, warm_fallbacks};
pub use links::{Link, LinkTarget};
pub use objects::{words_text, ObjectKind, PageObject, PageWord, TextSpan};
pub use ocr::{Ocr, OcrLine, OcrResult, OcrWord};
pub use outline::{BookmarkPlace, OutlineItem};
pub use overlay::{
    picture_size, text_width, today_iso, Bates, HeaderFooter, ImageWatermark, Placed, Watermark,
};
pub use own_font::{CharRefusal, FontRefusal, LineFont};
pub use pagerange::{format_page_range, parse_page_range};
pub use redact::{PageRedaction, RedactReport};
pub use sanitise::{Hidden, HiddenReport, LayersDamaged};
pub use scan::{ScanMode, ScanOptions, ScanResult};
pub use security::{Cipher, Encryption, Restrictions};
pub use sig_changes::{Change, ChangeKind, Refusal};
pub use sig_save::{append_edits, AppendedEdits};
pub use sig_sign::{
    sign_document, DisallowedChanges, SignPlace, SignRequest, SignedDocument, MIN_SIGNATURE_BOX,
};
pub use sig_verify::{
    verify_signatures, DigitalSignature, EmptySignatureField, FieldLock, LockAction, Permissions,
    SigKind, SigStatus, SigWarning, SignatureReport, SigningTime, TimeSource, Timestamp,
};
pub use sign::{
    picture_view, sign_label, typed_signature, DateStyle, InkColor, Mark, SignArt, SignRole,
    SignStyle, SIGN_TEXT,
};
pub use stamps::{stamp_wording, Stamp, StampColor, StampKind, StampPlace};
pub use text::{match_count, Matches, Query, SearchHit, Snippet, TextIndex};
pub use trust::{is_authority, read_certificates, Added, Trust, TrustEntry, TrustList, TrustRole};
pub use typefaces::{add_font_files, typeface, typefaces, typefaces_ready, FaceFile, Typeface};
pub use viewprefs::{OpenLayout, OpenPanel, ViewPrefs};

pub type Result<T> = anyhow::Result<T>;

/// LimarePDF's settings folder, from the environment; `None` when it can't be found:
///
/// - Linux and other Unix: `$XDG_CONFIG_HOME/limarepdf/`, else `~/.config/limarepdf/`
/// - macOS: `~/Library/Application Support/limarepdf/`
/// - Windows: `%APPDATA%\limarepdf\`
pub fn settings_dir() -> Option<std::path::PathBuf> {
    use std::path::PathBuf;
    let env = |k: &str| {
        let v = std::env::var_os(k)?;
        (!v.is_empty()).then(|| PathBuf::from(v))
    };
    let base = if cfg!(windows) {
        env("APPDATA")?
    } else if cfg!(target_os = "macos") {
        env("HOME")?.join("Library/Application Support")
    } else {
        env("XDG_CONFIG_HOME").or_else(|| Some(env("HOME")?.join(".config")))?
    };
    Some(base.join("limarepdf"))
}

/// Writes `bytes` to `path` for this user only (mode 600 on Unix), replacing any file
/// there in one step, so a failure leaves the old file whole. Missing folders are made,
/// private to this user on Unix.
pub fn write_private(path: &std::path::Path, bytes: &[u8]) -> Result<()> {
    use anyhow::Context;
    if let Some(dir) = path.parent().filter(|d| !d.as_os_str().is_empty()) {
        let mut b = std::fs::DirBuilder::new();
        b.recursive(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::DirBuilderExt;
            b.mode(0o700);
        }
        b.create(dir)
            .with_context(|| format!("making the folder {}", dir.display()))?;
    }
    // A context, so that a `NotConfirmed` can still be told apart.
    atomic::replace(path, atomic::Access::Private, |w| {
        use std::io::Write;
        Ok(w.write_all(bytes)?)
    })
    .with_context(|| format!("saving {}", path.display()))
}

/// The most a page's content, a drawing's content or a text stream may decompress to, so a
/// small crafted stream can't use up memory. Fonts and scanned images have their own limits.
pub(crate) const MAX_CONTENT: usize = 256 << 20;

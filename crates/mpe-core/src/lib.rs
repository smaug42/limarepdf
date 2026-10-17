//! mpe-core: pure-Rust PDF document operations and rendering.
//!
//! * Editing and structure: [`lopdf`]
//! * Rasterisation: [`hayro`] (CPU, no unsafe, no C/C++ dependencies)

mod annot_ap;
mod annot_write;
mod annots;
mod asn1;
mod blank;
mod cms_check;
mod display;
mod doc;
mod edit;
mod excerpt;
mod fonts;
mod form_ap;
mod form_flatten;
mod form_write;
mod forms;
mod imgs;
mod inline;
mod labels;
mod links;
mod matrix;
mod objects;
mod ocr;
mod outline;
mod overlay;
pub mod page_colours;
mod pagerange;
pub mod print;
#[cfg(windows)]
mod print_win;
mod reading;
mod redact;
mod redact_img;
pub mod render;
mod sanitise;
mod scan;
mod security;
mod sig_changes;
mod sig_verify;
mod sign;
mod stamps;
mod std14;
mod text;
mod tounicode;
mod trust;
mod viewprefs;

pub use annots::{Annot, AnnotKind, ShapeLook, TextStyle};
pub use blank::BLANK_TOLERANCE;
pub use cms_check::CertInfo;
pub use display::{display_line, display_text};
pub use doc::{part_file_name, Info, Pdf};
pub use edit::{PathStyle, Shape, TextEdit};
pub use fonts::{Family, FontChoice};
pub use forms::{Field, FieldKind, FieldValue, Widget};
pub use imgs::{image_dimensions, ImageImport};
pub use labels::{LabelRange, LabelStyle};
pub use links::{Link, LinkTarget};
pub use objects::{words_text, ObjectKind, PageObject, PageWord};
pub use ocr::{Ocr, OcrLine, OcrResult, OcrWord};
pub use outline::{BookmarkPlace, OutlineItem};
pub use overlay::{text_width, Bates, HeaderFooter, ImageWatermark, Watermark};
pub use pagerange::{format_page_range, parse_page_range};
pub use redact::{PageRedaction, RedactReport};
pub use sanitise::{Hidden, HiddenReport};
pub use scan::{ScanMode, ScanOptions, ScanResult};
pub use security::{Cipher, Encryption, Restrictions};
pub use sig_changes::{Change, ChangeKind, Refusal};
pub use sig_verify::{
    verify_signatures, DigitalSignature, EmptySignatureField, FieldLock, LockAction, Permissions,
    SigKind, SigStatus, SigWarning, SignatureReport, SigningTime, TimeSource, Timestamp,
};
pub use sign::{
    sign_label, typed_signature, DateStyle, InkColor, Mark, SignArt, SignRole, SignStyle, SIGN_TEXT,
};
pub use stamps::{stamp_wording, Stamp, StampColor, StampKind, StampPlace};
pub use text::SearchHit;
pub use trust::{is_authority, read_certificates, Added, Trust, TrustEntry, TrustList, TrustRole};
pub use viewprefs::{OpenLayout, OpenPanel, ViewPrefs};

pub type Result<T> = anyhow::Result<T>;

/// MPE's settings folder, from the environment; `None` when it can't be found:
///
/// - Linux and other Unix: `$XDG_CONFIG_HOME/mpe/`, else `~/.config/mpe/`
/// - macOS: `~/Library/Application Support/mpe/`
/// - Windows: `%APPDATA%\mpe\`
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
    Some(base.join("mpe"))
}

/// The most a page's content, a drawing's content or a text stream may decompress to, so a
/// small crafted stream can't use up memory. Fonts and scanned images have their own limits.
pub(crate) const MAX_CONTENT: usize = 256 << 20;

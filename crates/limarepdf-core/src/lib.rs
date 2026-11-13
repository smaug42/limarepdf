//! limarepdf-core: pure-Rust PDF document operations and rendering.
//!
//! * Editing and structure: [`lopdf`]
//! * Rasterisation: [`hayro`] (CPU, no unsafe, no C/C++ dependencies)

mod annot_ap;
mod annot_write;
mod annots;
mod asn1;
mod atomic;
mod attached;
mod base_encodings;
mod blank;
mod budget;
mod child;
mod cms_check;
mod cms_write;
mod colour;
mod compact;
mod decorate;
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
mod ipp;
mod labels;
mod layout;
mod links;
mod matrix;
mod merge;
mod mesh_fill;
mod objects;
mod ocr;
mod outline;
mod overlay;
mod own_encoding;
mod own_font;
pub mod page_colours;
mod pagerange;
mod pages_gone;
mod paper;
#[cfg(any(windows, test))]
mod paper_answer;
mod para_frames;
mod para_write;
mod paragraph;
mod passes;
mod pictures;
mod pkcs12;
mod preview_font;
pub mod print;
#[cfg(windows)]
mod print_win;
mod radicals;
mod reach;
mod reading;
mod redact;
mod redact_img;
mod reduce;
pub mod render;
mod repair;
mod retained;
mod rewrap;
mod sanitise;
mod scan;
mod security;
mod sheets;
mod sig_changes;
mod sig_save;
mod sig_sign;
mod sig_verify;
mod sign;
pub mod spelling;
mod stamps;
mod stand_in;
#[cfg(any(test, feature = "test-seams"))]
mod state;
mod std14;
mod text;
mod tounicode;
mod trees;
mod trust;
mod typefaces;
mod unfillable;
mod viewprefs;

pub use annot_ap::TextBoxSetting;
pub use annots::{Annot, AnnotKind, Flattened, ShapeLook, TextStyle, MAX_ANNOTS};
#[cfg(any(test, feature = "test-seams"))]
pub use atomic::{commit_seam, sync_seam};
pub use atomic::{ChangedOnDisk, NotConfirmed};
pub use blank::BLANK_TOLERANCE;
#[cfg(any(test, feature = "test-seams"))]
pub use budget::budget_seam;
pub use budget::TooComplex;
#[cfg(any(test, feature = "test-seams"))]
pub use child::child_seam;
pub use cms_check::CertInfo;
#[cfg(any(test, feature = "test-seams"))]
pub use compact::MAX_XMP;
pub use decorate::{decoration_lines, DecoKind, Decorated, Decoration, FontLines, Look};
pub use display::{display_line, display_text, local_date, local_time, quoted_list, short_list};
pub use doc::{
    existing, part_file_name, write_file, write_file_expecting, Info, NeedsPassword, Pdf, SplitBy,
};
pub use edit::{KeptPiece, PartFont, PathStyle, Shape, TextEdit, TextPart};
pub use embed::{
    bundled_face, emoji_sequence, liberation, refusal, unwritten_script, writable, LIBERATION,
};
pub use fonts::{winansi_byte, Family, FontChoice};
pub use form_ap::BoxSetting;
pub use forms::{Field, FieldKind, FieldValue, Widget};
pub use history::{Before, Restore};
#[cfg(any(test, feature = "test-seams"))]
pub use ids::password_seam;
pub use ids::{
    password_problem, DigitalId, IdList, IdStore, NewId, Stored, UnlockedId, WrongPassword,
    MIN_PASSWORD,
};
pub use imgs::{image_dimensions, picture_preview, ImageImport, PicturesTooLarge};
pub use labels::{LabelRange, LabelStyle};
pub use layout::{fallback_faces, fallbacks, set_in, warm_fallbacks, SetIn};
pub use links::{email_link, web_link, Link, LinkId, LinkTarget, Unfollowed};
pub use objects::{words_text, ObjectKind, PageObject, PageWord, TextSpan};
pub use ocr::{Ocr, OcrLine, OcrResult, OcrWord};
#[cfg(any(test, feature = "test-seams"))]
pub use outline::outline_seam;
pub use outline::{bookmark_moves, bookmark_step, BookmarkPlace, BookmarkStep, OutlineItem};
pub use overlay::{
    picture_size, text_width, today_iso, Bates, HeaderFooter, ImageWatermark, Placed, Watermark,
};
pub use own_font::{CharRefusal, FontRefusal, LineFont};
pub use pagerange::{format_page_range, parse_page_range};
pub use paragraph::{Align, Block, Detect, Doubt, Frame, VisualLine};
pub use pictures::{png_file, PictureFormat, SavedPictures};
#[cfg(all(windows, any(test, feature = "test-seams")))]
pub use print_win::papers_seam;
pub use redact::{Leftovers, PageRedaction, RedactReport};
pub use reduce::{
    KeptWhy, Most, PictureQuality, ReduceOptions, ReducePreview, Reduced, PREVIEW_SIDE,
};
pub use rewrap::{
    DocWords, Hyphen, LineEndHyphen, Paragraph, ParagraphPiece, ParagraphSetting, SetLine,
    SetParagraph,
};
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
pub use stand_in::{InstalledQuery, StandIn};
#[cfg(any(test, feature = "test-seams"))]
pub use state::{StateBefore, StateDifference};
pub use text::{
    match_count, HitPart, Matches, Query, SearchHit, SearchOptions, Snippet, TextIndex,
    SEARCH_GROUP,
};
pub use trust::{is_authority, read_certificates, Added, Trust, TrustEntry, TrustList, TrustRole};
pub use typefaces::{add_font_files, typeface, typefaces, typefaces_ready, FaceFile, Typeface};
pub use viewprefs::{OpenLayout, OpenPanel, ViewPrefs};

pub type Result<T> = anyhow::Result<T>;

/// The readers fuzz/'s targets reach that aren't public: digital ID files and the printing
/// service's answers. Built only with `--cfg fuzzing`, as `cargo fuzz` builds; never in the app.
#[cfg(fuzzing)]
pub mod fuzz_seam {
    /// Reads a digital ID file as LimarePDF does: its certificates without the password, as
    /// the list of IDs shows them, then all of it with `password`.
    pub fn digital_id(file: &[u8], password: &str) {
        let _ = crate::pkcs12::peek(file);
        let _ = crate::pkcs12::read(file, password);
    }

    /// Reads an answer from the printing service (IPP), then the paper sizes in it.
    pub fn printer_answer(data: &[u8]) {
        if let Ok(answer) = crate::ipp::read_answer(data) {
            let _ = crate::paper::from_ipp(&answer.printer);
        }
    }
}

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
    save_private(path, bytes, false)
}

/// [`write_private`], for a new file: one at `path` is never replaced. When something is there
/// by the time the file is written, the error is an [`std::io::ErrorKind::AlreadyExists`]
/// (PB-134).
pub(crate) fn write_private_new(path: &std::path::Path, bytes: &[u8]) -> Result<()> {
    save_private(path, bytes, true)
}

fn save_private(path: &std::path::Path, bytes: &[u8], new: bool) -> Result<()> {
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
    let write = |w: &mut std::io::BufWriter<std::fs::File>| {
        use std::io::Write;
        Ok(w.write_all(bytes)?)
    };
    let written = match new {
        true => atomic::create_new(path, write),
        false => atomic::replace(path, atomic::Access::Private, None, write),
    };
    // A context, so that a `NotConfirmed` can still be told apart.
    written.with_context(|| format!("saving {}", path.display()))
}

/// The most a page's content, a drawing's content or a text stream may decompress to, so a
/// small crafted stream can't use up memory. Fonts and scanned images have their own limits.
pub(crate) const MAX_CONTENT: usize = 256 << 20;

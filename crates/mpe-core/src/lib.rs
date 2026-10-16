//! mpe-core: pure-Rust PDF document operations and rendering.
//!
//! * Editing and structure: [`lopdf`]
//! * Rasterisation: [`hayro`] (CPU, no unsafe, no C/C++ dependencies)

mod annot_ap;
mod annot_write;
mod annots;
mod blank;
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
mod pagerange;
pub mod print;
#[cfg(windows)]
mod print_win;
mod reading;
pub mod render;
mod scan;
mod security;
mod stamps;
mod std14;
mod text;
mod tounicode;
mod viewprefs;

pub use annots::{Annot, AnnotKind, ShapeLook, TextStyle};
pub use blank::BLANK_TOLERANCE;
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
pub use scan::{ScanMode, ScanOptions, ScanResult};
pub use security::{Cipher, Encryption, Restrictions};
pub use stamps::{stamp_wording, Stamp, StampColor, StampKind, StampPlace};
pub use text::SearchHit;
pub use viewprefs::{OpenLayout, OpenPanel, ViewPrefs};

pub type Result<T> = anyhow::Result<T>;

/// The most a page's content, a drawing's content or a text stream may decompress to, so a
/// small crafted stream can't use up memory. Fonts and scanned images have their own limits.
pub(crate) const MAX_CONTENT: usize = 256 << 20;

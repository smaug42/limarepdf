//! mpe-core: pure-Rust PDF document operations and rendering.
//!
//! * Editing and structure: [`lopdf`]
//! * Rasterisation: [`hayro`] (CPU, no unsafe, no C/C++ dependencies)

mod doc;
mod edit;
mod fonts;
mod form_ap;
mod form_flatten;
mod form_write;
mod forms;
mod imgs;
mod inline;
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
mod std14;
mod text;
mod tounicode;

pub use doc::{Info, Pdf};
pub use edit::{PathStyle, Shape, TextEdit};
pub use fonts::{Family, FontChoice};
pub use forms::{Field, FieldKind, FieldValue, Widget};
pub use imgs::{image_dimensions, ImageImport};
pub use links::{Link, LinkTarget};
pub use objects::{words_text, ObjectKind, PageObject, PageWord};
pub use ocr::{Ocr, OcrLine, OcrResult, OcrWord};
pub use outline::OutlineItem;
pub use overlay::{text_width, Bates, HeaderFooter, Watermark};
pub use pagerange::parse_page_range;
pub use scan::{ScanMode, ScanOptions, ScanResult};
pub use security::{Cipher, Encryption, Restrictions};
pub use text::SearchHit;

pub type Result<T> = anyhow::Result<T>;

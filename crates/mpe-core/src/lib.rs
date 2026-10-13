//! mpe-core: pure-Rust PDF document operations and rendering.
//!
//! * Editing and structure: [`lopdf`]
//! * Rasterisation: [`hayro`] (CPU, no unsafe, no C/C++ dependencies)

mod doc;
mod edit;
mod fonts;
mod imgs;
mod inline;
mod matrix;
mod objects;
mod ocr;
mod overlay;
mod pagerange;
pub mod render;
mod scan;
mod security;
mod text;

pub use doc::{Info, Pdf};
pub use edit::{PathStyle, Shape, TextEdit};
pub use fonts::{Family, FontChoice};
pub use imgs::{image_dimensions, ImageImport};
pub use objects::{ObjectKind, PageObject};
pub use ocr::{Ocr, OcrLine, OcrResult, OcrWord};
pub use overlay::{text_width, Bates, HeaderFooter, Watermark};
pub use pagerange::parse_page_range;
pub use scan::{ScanMode, ScanOptions, ScanResult};
pub use security::{Cipher, Encryption, Restrictions};
pub use text::SearchHit;

pub type Result<T> = anyhow::Result<T>;

//! mpe-core: pure-Rust PDF document operations and rendering.
//!
//! * Editing and structure: [`lopdf`]
//! * Rasterisation: [`hayro`] (CPU, no unsafe, no C/C++ dependencies)

mod doc;
mod overlay;
mod pagerange;
mod security;
pub mod render;
mod text;

pub use doc::{Info, Pdf};
pub use overlay::{text_width, Bates, HeaderFooter, Watermark};
pub use pagerange::parse_page_range;
pub use security::{Cipher, Encryption, Restrictions};
pub use text::SearchHit;

pub type Result<T> = anyhow::Result<T>;

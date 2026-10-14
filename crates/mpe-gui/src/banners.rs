//! Document banners: short notices above the pages about something MPE noticed when the
//! document was opened, each with at most one action and a way to dismiss it.

use crate::actions::Act;
use egui_phosphor::regular as icon;
use mpe_core::{ObjectKind, Pdf};

/// Pages looked at for scans; enough to tell, and quick when opening a large file.
const SCAN_PAGES: usize = 20;
/// A document with more pages than this, or a file larger than this, is large.
const LARGE_PAGES: usize = 500;
const LARGE_BYTES: u64 = 100_000_000;

/// What a banner is about, in order of importance.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub enum Kind {
    /// The file was encrypted: saving drops the protection unless it is set again.
    Protected,
    /// Pages are images without text.
    Scanned,
    /// The document has form fields, which MPE cannot fill in.
    Forms,
    /// Many pages or a big file.
    Large,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Banner {
    pub kind: Kind,
    pub text: String,
}

impl Banner {
    pub fn icon(&self) -> &'static str {
        match self.kind {
            Kind::Protected => icon::LOCK_KEY,
            Kind::Scanned => icon::SCAN,
            Kind::Forms => icon::TEXTBOX,
            Kind::Large => icon::STACK,
        }
    }

    /// The button the banner offers: its label and command.
    pub fn action(&self) -> Option<(&'static str, Act)> {
        match self.kind {
            Kind::Protected => Some(("Password security", Act::Security)),
            Kind::Scanned => Some(("Recognise text", Act::Ocr)),
            Kind::Forms | Kind::Large => None,
        }
    }
}

/// The banners for a document just opened, most important first. `bytes` is the file's
/// size, when it came from a file.
pub fn detect(pdf: &Pdf, bytes: Option<u64>) -> Vec<Banner> {
    let mut v = Vec::new();
    if pdf.doc.was_encrypted() {
        v.push(Banner {
            kind: Kind::Protected,
            text: "This document was password-protected. Saving removes the protection unless \
                   you set it again."
                .into(),
        });
    }
    let scans = scanned_pages(pdf);
    if scans > 0 {
        let what = if scans == pdf.page_count() {
            "Its pages are"
        } else {
            "Some of its pages are"
        };
        v.push(Banner {
            kind: Kind::Scanned,
            text: format!("{what} scans without text, so they cannot be searched or copied."),
        });
    }
    if has_form_fields(pdf) {
        v.push(Banner {
            kind: Kind::Forms,
            text: "This document has form fields. MPE cannot fill them in yet.".into(),
        });
    }
    let pages = pdf.page_count();
    let big = bytes.is_some_and(|b| b > LARGE_BYTES);
    if pages > LARGE_PAGES || big {
        let size = bytes.map_or(String::new(), |b| {
            format!(", {:.0} MB", b as f64 / 1_000_000.0)
        });
        v.push(Banner {
            kind: Kind::Large,
            text: format!(
                "Large document ({pages} pages{size}): some operations may take a while."
            ),
        });
    }
    v
}

/// How many of the first pages are scans: no text (visible or recognised) and an image
/// covering at least half the page.
fn scanned_pages(pdf: &Pdf) -> usize {
    (0..pdf.page_count().min(SCAN_PAGES))
        .filter(|&i| {
            let (Ok(objs), Ok((w, h))) = (pdf.page_objects(i), pdf.page_size(i)) else {
                return false;
            };
            let area = |b: [f32; 4]| (b[2] - b[0]).abs() * (b[3] - b[1]).abs();
            let text = pdf.page_has_text(i);
            let image = objs
                .iter()
                .any(|o| o.kind == ObjectKind::Image && area(o.bbox) >= 0.5 * w * h);
            !text && image
        })
        .count()
}

/// Whether the document's form (the catalogue's `/AcroForm`) lists any fields.
fn has_form_fields(pdf: &Pdf) -> bool {
    let doc = &pdf.doc;
    let Ok(form) = doc.catalog().and_then(|c| c.get(b"AcroForm")) else {
        return false;
    };
    let Ok((_, form)) = doc.dereference(form) else {
        return false;
    };
    let Ok(fields) = form.as_dict().and_then(|d| d.get(b"Fields")) else {
        return false;
    };
    doc.dereference(fields)
        .ok()
        .and_then(|(_, f)| f.as_array().ok())
        .is_some_and(|a| !a.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;
    use lopdf::{dictionary, Object};
    use mpe_core::{Cipher, Encryption, ImageImport, Restrictions};

    fn kinds(pdf: &Pdf, bytes: Option<u64>) -> Vec<Kind> {
        detect(pdf, bytes).iter().map(|b| b.kind).collect()
    }

    fn blank(pages: usize) -> Pdf {
        let mut p = Pdf::new();
        for i in 0..pages {
            p.insert_blank(i, 300.0, 300.0).unwrap();
        }
        p
    }

    /// A 1 x 1 pixel grey PNG.
    const PNG: &[u8] = &[
        0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0x00, 0x00, 0x00, 0x0D, 0x49, 0x48, 0x44,
        0x52, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x00, 0x00, 0x00, 0x00, 0x3A,
        0x7E, 0x9B, 0x55, 0x00, 0x00, 0x00, 0x0A, 0x49, 0x44, 0x41, 0x54, 0x78, 0x9C, 0x63, 0x68,
        0x00, 0x00, 0x00, 0x82, 0x00, 0x81, 0x77, 0xCD, 0x72, 0xB6, 0x00, 0x00, 0x00, 0x00, 0x49,
        0x45, 0x4E, 0x44, 0xAE, 0x42, 0x60, 0x82,
    ];

    #[test]
    fn plain_documents_get_no_banner() {
        assert!(kinds(&blank(3), Some(1000)).is_empty());
    }

    #[test]
    fn image_pages_are_scans() {
        let p = Pdf::from_images(&[PNG.to_vec()], &ImageImport::default()).unwrap();
        assert_eq!(kinds(&p, None), [Kind::Scanned]);
        let b = &detect(&p, None)[0];
        assert!(b.text.starts_with("Its pages are"), "{}", b.text);
        assert_eq!(b.action().map(|a| a.1), Some(Act::Ocr));
    }

    #[test]
    fn encrypted_files_are_flagged() {
        let mut p = blank(1);
        p.set_encryption(Some(Encryption {
            user_password: "user".into(),
            owner_password: "owner".into(),
            cipher: Cipher::Aes128,
            restrictions: Restrictions::default(),
        }));
        let bytes = p.to_bytes().unwrap();
        let opened = Pdf::from_bytes(&bytes, Some("user")).unwrap();
        assert_eq!(kinds(&opened, None), [Kind::Protected]);
        // A copy saved without protection is no longer flagged.
        let plain = Pdf::from_bytes(&blank(1).to_bytes().unwrap(), None).unwrap();
        assert!(kinds(&plain, None).is_empty());
    }

    #[test]
    fn form_fields_are_found() {
        let mut p = blank(1);
        let name = Object::string_literal("name");
        let field = p.doc.add_object(dictionary! { "FT" => "Tx", "T" => name });
        let fields = vec![Object::Reference(field)];
        let form = p.doc.add_object(dictionary! { "Fields" => fields });
        let catalog = p.doc.catalog_mut().unwrap();
        catalog.set("AcroForm", Object::Reference(form));
        assert_eq!(kinds(&p, None), [Kind::Forms]);
        // An empty form is not.
        let mut q = blank(1);
        let empty: Vec<Object> = Vec::new();
        let catalog = q.doc.catalog_mut().unwrap();
        catalog.set("AcroForm", dictionary! { "Fields" => empty });
        assert!(kinds(&q, None).is_empty());
    }

    #[test]
    fn large_by_pages_or_size() {
        assert_eq!(kinds(&blank(LARGE_PAGES + 1), None), [Kind::Large]);
        let big = detect(&blank(2), Some(LARGE_BYTES + 1));
        assert_eq!(big[0].kind, Kind::Large);
        assert!(big[0].text.contains("100 MB"), "{}", big[0].text);
    }
}

//! Document banners: short notices above the pages about something MPE noticed when the
//! document was opened, each with at most one action and a way to dismiss it.

use crate::actions::Act;
use crate::theme::Tone;
use egui_phosphor::regular as icon;
use mpe_core::{FieldKind, ObjectKind, Pdf};

/// Pages looked at for scans; enough to tell, and quick when opening a large file.
const SCAN_PAGES: usize = 20;
/// A document with more pages than this, or a file larger than this, is large.
const LARGE_PAGES: usize = 500;
const LARGE_BYTES: u64 = 100_000_000;

/// What a banner is about, in order of importance.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub enum Kind {
    /// The document's digital signatures: being checked, or how they checked.
    Signatures,
    /// The file was encrypted: saving drops the protection unless it is set again.
    Protected,
    /// Areas are marked for redaction but nothing under them is removed yet.
    Redactions,
    /// Signatures and other fill-and-sign items can still be moved.
    Signing,
    /// An XFA-only form: only Adobe software builds its pages.
    Xfa,
    /// Pages are images without text.
    Scanned,
    /// The document has form fields.
    Forms,
    /// Many pages or a big file.
    Large,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Banner {
    pub kind: Kind,
    pub text: String,
    /// Colours the banner by how good or bad its news is (signatures only).
    pub tone: Tone,
}

impl Banner {
    /// A banner in the usual colours.
    fn plain(kind: Kind, text: impl Into<String>) -> Self {
        Self {
            kind,
            text: text.into(),
            tone: Tone::Neutral,
        }
    }

    pub fn icon(&self) -> &'static str {
        match self.kind {
            Kind::Signatures => match self.tone {
                Tone::Good => icon::SEAL_CHECK,
                Tone::Caution => icon::SEAL_QUESTION,
                Tone::Bad => icon::SEAL_WARNING,
                Tone::Neutral => icon::SEAL,
            },
            Kind::Protected => icon::LOCK_KEY,
            Kind::Redactions => icon::EYE_SLASH,
            Kind::Signing => icon::SIGNATURE,
            Kind::Xfa => icon::WARNING,
            Kind::Scanned => icon::SCAN,
            Kind::Forms => icon::TEXTBOX,
            Kind::Large => icon::STACK,
        }
    }

    /// The button the banner offers: its label and command.
    pub fn action(&self) -> Option<(&'static str, Act)> {
        match self.kind {
            Kind::Signatures => Some(("Signatures", Act::Signatures)),
            Kind::Protected => Some(("Password security", Act::Security)),
            Kind::Redactions => Some(("Apply redactions…", Act::ApplyRedactions)),
            Kind::Signing => Some(("Finish signing…", Act::FinishSigning)),
            Kind::Scanned => Some(("Recognise text", Act::Ocr)),
            Kind::Forms => Some(("Highlight fields", Act::ToggleFields)),
            Kind::Xfa | Kind::Large => None,
        }
    }
}

/// The banner for `marks` areas marked for redaction and not yet applied. Unlike the
/// others it follows the document as it changes (see `Tab::refresh_marks_banner`).
pub fn redactions(marks: usize) -> Banner {
    Banner::plain(
        Kind::Redactions,
        format!(
            "{} marked for redaction (outlined in red) but not yet applied: nothing under them \
             has been removed.",
            if marks == 1 {
                "An area is".to_string()
            } else {
                format!("{marks} areas are")
            }
        ),
    )
}

/// The banner for `items` signatures, marks and other fill-and-sign items that can still
/// be moved; it follows the document as the redaction banner does.
pub fn signing(items: usize) -> Banner {
    Banner::plain(
        Kind::Signing,
        format!(
            "{} can still be moved or removed. Finish signing to draw {} into the pages for \
             good.",
            if items == 1 {
                "A signature or other item you put down".to_string()
            } else {
                format!("{items} signatures and other items you put down")
            },
            if items == 1 { "it" } else { "them" }
        ),
    )
}

/// The banner for a signed document's digital signatures, coloured by `tone`.
pub fn signatures(tone: Tone, text: String) -> Banner {
    Banner {
        kind: Kind::Signatures,
        text,
        tone,
    }
}

/// The banners for a document just opened, most important first. `bytes` is the file's
/// size, when it came from a file.
pub fn detect(pdf: &Pdf, bytes: Option<u64>) -> Vec<Banner> {
    let mut v = Vec::new();
    if pdf.doc.was_encrypted() {
        v.push(Banner::plain(
            Kind::Protected,
            "This document was password-protected. Saving removes the protection unless you \
             set it again.",
        ));
    }
    let xfa = pdf.xfa_only();
    if xfa {
        v.push(Banner::plain(
            Kind::Xfa,
            "This form was made for Adobe Reader (an XFA form). Only Adobe software can show \
             or fill it in; what you see is the placeholder page the file contains.",
        ));
    }
    let scans = scanned_pages(pdf);
    if scans > 0 {
        let what = if scans == pdf.page_count() {
            "Its pages are"
        } else {
            "Some of its pages are"
        };
        v.push(Banner::plain(
            Kind::Scanned,
            format!("{what} scans without text, so they cannot be searched or copied."),
        ));
    }
    if !xfa && has_form_fields(pdf) {
        v.push(Banner::plain(
            Kind::Forms,
            "This document has a form. Click a field to fill it in, or press Tab.",
        ));
    }
    let pages = pdf.page_count();
    let big = bytes.is_some_and(|b| b > LARGE_BYTES);
    if pages > LARGE_PAGES || big {
        let size = bytes.map_or(String::new(), |b| {
            format!(", {:.0} MB", b as f64 / 1_000_000.0)
        });
        v.push(Banner::plain(
            Kind::Large,
            format!("Large document ({pages} pages{size}): some operations may take a while."),
        ));
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

/// Whether the document's form has fields to fill in. Signature fields don't count: the
/// signatures banner and the Sign tool cover them.
fn has_form_fields(pdf: &Pdf) -> bool {
    pdf.form_fields()
        .iter()
        .any(|f| f.kind != FieldKind::Signature)
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
        // Nor is a form of signature fields alone: there is nothing to fill in.
        let mut s = blank(1);
        let name = Object::string_literal("sig");
        let field = s.doc.add_object(dictionary! { "FT" => "Sig", "T" => name });
        let fields = vec![Object::Reference(field)];
        let form = s.doc.add_object(dictionary! { "Fields" => fields });
        let catalog = s.doc.catalog_mut().unwrap();
        catalog.set("AcroForm", Object::Reference(form));
        assert!(kinds(&s, None).is_empty());
        // An empty form is not.
        let mut q = blank(1);
        let empty: Vec<Object> = Vec::new();
        let catalog = q.doc.catalog_mut().unwrap();
        catalog.set("AcroForm", dictionary! { "Fields" => empty });
        assert!(kinds(&q, None).is_empty());
    }

    #[test]
    fn xfa_only_forms_are_explained() {
        let mut p = blank(1);
        let empty: Vec<Object> = Vec::new();
        let xfa = Object::string_literal("<xdp:xdp/>");
        let catalog = p.doc.catalog_mut().unwrap();
        catalog.set("AcroForm", dictionary! { "Fields" => empty, "XFA" => xfa });
        assert_eq!(kinds(&p, None), [Kind::Xfa]);
    }

    #[test]
    fn large_by_pages_or_size() {
        assert_eq!(kinds(&blank(LARGE_PAGES + 1), None), [Kind::Large]);
        let big = detect(&blank(2), Some(LARGE_BYTES + 1));
        assert_eq!(big[0].kind, Kind::Large);
        assert!(big[0].text.contains("100 MB"), "{}", big[0].text);
    }
}

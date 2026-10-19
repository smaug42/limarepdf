//! Document banners: short notices above the pages about something LimarePDF noticed when the
//! document was opened, each with at most one action and a way to dismiss it.

use crate::actions::Act;
use crate::theme::Tone;
use egui_phosphor::regular as icon;
use limarepdf_core::render::Missing;
use limarepdf_core::{FieldKind, ObjectKind, Pdf};
use std::collections::BTreeSet;

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
    /// The document was restored from a recovery copy.
    Recovered,
    /// A recovery copy made before content was removed couldn't be deleted.
    LeftoverCopy,
    /// Recovery copies of the document can't be written right now.
    NoCopies,
    /// The file was damaged, and LimarePDF repaired it to open it.
    Repaired,
    /// The file was encrypted: saving drops the protection unless it is set again.
    Protected,
    /// Areas are marked for redaction but nothing under them is removed yet.
    Redactions,
    /// Signatures and other fill-and-sign items can still be moved.
    Signing,
    /// An XFA-only form: only Adobe software builds its pages.
    Xfa,
    /// Parts of pages couldn't be drawn.
    Missing,
    /// Fonts the document names aren't installed: which, and what to install.
    Fonts,
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
            Kind::Recovered => icon::LIFEBUOY,
            Kind::LeftoverCopy => icon::FILE_X,
            Kind::NoCopies => icon::WARNING_CIRCLE,
            Kind::Repaired => icon::WRENCH,
            Kind::Protected => icon::LOCK_KEY,
            Kind::Redactions => icon::EYE_SLASH,
            Kind::Signing => icon::SIGNATURE,
            Kind::Xfa => icon::WARNING,
            Kind::Missing => icon::IMAGE_BROKEN,
            Kind::Fonts => icon::TEXT_AA,
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
            Kind::Repaired | Kind::Recovered => Some(("Save as…", Act::SaveAs)),
            Kind::NoCopies => Some(("Save", Act::Save)),
            Kind::LeftoverCopy => Some(("Show file", Act::ShowRecoveryFile)),
            Kind::Xfa | Kind::Missing | Kind::Fonts | Kind::Large => None,
        }
    }
}

/// The notice on a document restored from a recovery copy made at `time` (local time);
/// `signed`: it holds digital signatures, which aren't checked (the design's section 7).
pub fn recovered(time: &str, signed: bool) -> Banner {
    let mut text = format!(
        "Recovered copy, as it was at {time}, before LimarePDF stopped without closing \
         normally. It isn't saved: save it to keep it."
    );
    if signed {
        text.push_str(
            " Its digital signatures can't be checked here, because they cover the original \
             file, not this copy. Open the original to check them. Saving this copy makes them \
             invalid.",
        );
    }
    Banner::plain(Kind::Recovered, text)
}

/// The notice when a recovery copy of the document can't be written (`why`).
pub fn no_copies(why: &str) -> Banner {
    Banner::plain(
        Kind::NoCopies,
        format!(
            "Recovery copies of this document can't be kept right now ({}). Save it to keep \
             your changes.",
            limarepdf_core::display_line(why, 200)
        ),
    )
}

/// The notice when a recovery copy made before content was removed couldn't be deleted,
/// with where it is.
pub fn leftover_copy(file: &std::path::Path) -> Banner {
    Banner::plain(
        Kind::LeftoverCopy,
        format!(
            "An earlier recovery copy of this document, made before the change, couldn't be \
             deleted. It can't be offered for recovery, but its file is still on disk at {} \
             until LimarePDF manages to delete it, or you do.",
            file.display()
        ),
    )
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

/// The banners for what the renderer couldn't draw as the file asks, found as pages are
/// drawn: one about fonts that aren't installed, saying what to install, and one about
/// other parts that couldn't be drawn.
pub fn missing(what: &BTreeSet<Missing>) -> Vec<Banner> {
    let mut stand_ins = BTreeSet::new();
    let mut unnamed_stand_ins = false;
    let mut languages = BTreeSet::new();
    let mut unusable = BTreeSet::new();
    let mut parts = Vec::new();
    for m in what {
        match m {
            Missing::StandIn(Some(f), _) => {
                stand_ins.insert(f.as_str());
            }
            Missing::StandIn(None, _) => unnamed_stand_ins = true,
            Missing::Language(l, _) => {
                languages.insert(*l);
            }
            Missing::Font(Some(f)) => {
                unusable.insert(f.as_str());
            }
            Missing::Font(None) | Missing::Picture => parts.push(m.describe()),
        }
    }

    let mut fonts = Vec::new();
    if !languages.is_empty() {
        let names: Vec<&str> = languages.iter().map(|l| l.name()).collect();
        let suggest: Vec<&str> = languages.iter().map(|l| l.suggestion()).collect();
        fonts.push(if languages.len() == 1 {
            format!(
                "This document has {0} text, but no {0} font is installed, so that text can't \
                 be shown. Install a {0} font, such as {1}, to read it.",
                names[0], suggest[0]
            )
        } else {
            format!(
                "This document has {} text, but no font for them is installed, so that text \
                 can't be shown. Install fonts for them, such as {}, to read it.",
                list(&names),
                list(&suggest)
            )
        });
    }
    if !unusable.is_empty() {
        let names: Vec<&str> = unusable.into_iter().collect();
        fonts.push(format!(
            "{} {} installed and no font could stand in, so {} text may show wrong \
             characters. Install {} to read it.",
            fonts_named(&names),
            if names.len() == 1 { "isn't" } else { "aren't" },
            if names.len() == 1 { "its" } else { "their" },
            if names.len() == 1 { "it" } else { "them" },
        ));
    }
    if !stand_ins.is_empty() {
        let names: Vec<&str> = stand_ins.into_iter().collect();
        fonts.push(if names.len() == 1 {
            format!(
                "The font {0} isn't installed, so a similar font is shown in its place and \
                 some text may not look right. Install {0} to see the document as it was made.",
                names[0]
            )
        } else {
            format!(
                "{} aren't installed, so similar fonts are shown in their place and some text \
                 may not look right. Install them to see the document as it was made.",
                fonts_named(&names)
            )
        });
    } else if unnamed_stand_ins {
        fonts.push(
            "Some fonts this document uses aren't installed, so similar fonts are shown in \
             their place and some text may not look right."
                .into(),
        );
    }

    let mut v = Vec::new();
    if !parts.is_empty() {
        v.push(Banner::plain(
            Kind::Missing,
            format!(
                "Parts of this document couldn't be drawn ({}), so some pages may look \
                 incomplete.",
                parts.join(", ")
            ),
        ));
    }
    if !fonts.is_empty() {
        v.push(Banner::plain(Kind::Fonts, fonts.join(" ")));
    }
    v
}

/// "The font A" or "The fonts A, B and C".
fn fonts_named(names: &[&str]) -> String {
    if names.len() == 1 {
        format!("The font {}", names[0])
    } else {
        format!("The fonts {}", list(names))
    }
}

/// "A", "A and B", "A, B and C"; after four, "A, B, C, D and 3 more".
fn list(items: &[&str]) -> String {
    const SHOWN: usize = 4;
    match items {
        [] => String::new(),
        [one] => one.to_string(),
        _ if items.len() > SHOWN => {
            let more = items.len() - SHOWN;
            format!("{} and {more} more", items[..SHOWN].join(", "))
        }
        [rest @ .., last] => format!("{} and {last}", rest.join(", ")),
    }
}

/// The banners for a document just opened, most important first. `bytes` is the file's
/// size, when it came from a file.
pub fn detect(pdf: &Pdf, bytes: Option<u64>) -> Vec<Banner> {
    let mut v = Vec::new();
    if pdf.repaired() {
        let signed = if pdf.signature_count() > 0 {
            " Its signatures can't vouch for what is shown, and saving the repaired copy breaks \
             them."
        } else {
            ""
        };
        v.push(Banner::plain(
            Kind::Repaired,
            format!(
                "This file is damaged, and LimarePDF repaired it to open it; some content may \
                 be missing. The original stays as it is unless you choose to replace it when \
                 saving.{signed}"
            ),
        ));
    }
    if pdf.doc.was_encrypted() {
        v.push(Banner::plain(
            Kind::Protected,
            if pdf.password_needed() {
                "This document was password-protected. Saving removes the protection unless \
                 you set it again."
            } else {
                "This document opens without a password, but it has a permissions password, \
                 which can limit printing, copying or changing it. Saving removes that password \
                 unless you set it again."
            },
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
/// covering at least half the page. A page whose picture can't be read isn't one: text
/// recognition couldn't read it either, and the missing-picture notice covers it.
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
            !text && image && !pdf.page_has_unreadable_picture(i)
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
    use limarepdf_core::render::Language;
    use limarepdf_core::{Cipher, Encryption, ImageImport, Restrictions};
    use lopdf::{dictionary, Object};

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
    fn repaired_files_say_so() {
        // A line after the header moves every object away from where the table says.
        let mut bytes = blank(1).to_bytes().unwrap();
        let line = bytes.iter().position(|&c| c == b'\n').unwrap() + 1;
        bytes.splice(line..line, format!("%{}\n", " ".repeat(200)).into_bytes());
        let p = Pdf::from_bytes(&bytes, None).unwrap();
        assert_eq!(kinds(&p, None), [Kind::Repaired]);
        let b = &detect(&p, None)[0];
        assert!(b.text.contains("repaired"));
        assert_eq!(b.action().map(|a| a.1), Some(Act::SaveAs));
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
    fn pages_with_unreadable_pictures_are_not_scans() {
        let mut p = Pdf::from_images(&[PNG.to_vec()], &ImageImport::default()).unwrap();
        let mut doc = lopdf::Document::load_mem(&p.to_bytes().unwrap()).unwrap();
        for o in doc.objects.values_mut() {
            if let Object::Stream(s) = o {
                if s.dict.get(b"Subtype").and_then(Object::as_name).ok() == Some(b"Image") {
                    s.dict.set("Filter", "FlateDecode");
                    s.set_content((0..400u32).map(|i| (i * 37 + 11) as u8).collect());
                }
            }
        }
        let mut bytes = Vec::new();
        doc.save_to(&mut bytes).unwrap();
        let p = Pdf::from_bytes(&bytes, None).unwrap();
        assert!(p.page_has_unreadable_picture(0));
        assert!(!kinds(&p, None).contains(&Kind::Scanned));
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
        assert!(detect(&opened, None)[0]
            .text
            .starts_with("This document was password-protected"));
        // With only a permissions password, it doesn't read as if one had been typed.
        p.set_encryption(Some(Encryption {
            user_password: "".into(),
            owner_password: "owner".into(),
            cipher: Cipher::Aes128,
            restrictions: Restrictions::default(),
        }));
        let open = Pdf::from_bytes(&p.to_bytes().unwrap(), None).unwrap();
        assert_eq!(kinds(&open, None), [Kind::Protected]);
        let text = &detect(&open, None)[0].text;
        assert!(
            text.starts_with("This document opens without a password"),
            "{text}"
        );
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

    #[test]
    fn missing_parts_are_listed() {
        let b = missing(&[Missing::Picture, Missing::Font(None)].into());
        assert_eq!(b.len(), 1);
        assert_eq!(b[0].kind, Kind::Missing);
        assert!(b[0].text.contains("(a font, a picture)"), "{}", b[0].text);
        assert_eq!(b[0].action(), None);
        assert!(missing(&BTreeSet::new()).is_empty());
    }

    #[test]
    fn fonts_say_what_to_install() {
        let stand_in = |f: &str| Missing::StandIn(Some(f.into()), "Liberation Sans".into());
        let b = missing(&[stand_in("Tahoma"), Missing::Picture].into());
        assert_eq!(b.len(), 2);
        assert_eq!(b[1].kind, Kind::Fonts);
        assert_eq!(
            b[1].text,
            "The font Tahoma isn't installed, so a similar font is shown in its place and \
             some text may not look right. Install Tahoma to see the document as it was made."
        );

        let b = missing(&[stand_in("Tahoma"), stand_in("Verdana"), stand_in("Calibri")].into());
        let start = "The fonts Calibri, Tahoma and Verdana aren't installed";
        assert!(b[0].text.starts_with(start), "{}", b[0].text);

        let japanese = Missing::Language(Language::Japanese, Some("MS-Mincho".into()));
        let b = missing(&[japanese].into());
        assert_eq!(
            b[0].text,
            "This document has Japanese text, but no Japanese font is installed, so that text \
             can't be shown. Install a Japanese font, such as Noto Sans CJK JP, to read it."
        );
        let b = missing(
            &[
                Missing::Language(Language::Japanese, None),
                Missing::Language(Language::Korean, None),
            ]
            .into(),
        );
        assert!(
            b[0].text.contains("Japanese and Korean text")
                && b[0].text.contains("Noto Sans CJK JP and Noto Sans CJK KR"),
            "{}",
            b[0].text
        );

        let b = missing(&[Missing::Font(Some("Tahoma".into()))].into());
        assert_eq!(b[0].kind, Kind::Fonts);
        assert!(b[0].text.contains("Install it"), "{}", b[0].text);
    }

    #[test]
    fn long_lists_are_cut() {
        let six = ["A", "B", "C", "D", "E", "F"];
        assert_eq!(list(&six), "A, B, C, D and 2 more");
        assert_eq!(list(&["A", "B"]), "A and B");
    }
}

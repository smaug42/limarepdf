//! Text attached to letters for other apps to read in their place (`/ActualText`, and `/Alt`
//! and `/E` with it) after an edit (PB-71): where an edit changes letters in a stretch, its
//! attached text is removed, so other apps read what was typed; a stretch whose letters
//! aren't changed keeps it around them; where it can't be kept, the edit is refused and
//! nothing changes.
//!
//! The core text reader doesn't read attached text, so the oracle here is the content:
//! which letters sit inside which tag after the edit. Opt-in, Poppler reads it as other apps
//! would: `cargo test -p limarepdf-core --test actual_text -- --ignored cross_check`.

mod common;

use common::saved::tools;
use limarepdf_core::{FontChoice, ObjectKind, PartFont, Pdf, TextEdit, TextPart, TextSpan};
use lopdf::content::Content;
use lopdf::{dictionary, Dictionary, Object, Stream};

/// A 300 by 200 point page drawing `content`, with Helvetica as `/F1` and `props` as its
/// named marked-content properties.
fn make_with(content: &str, props: Dictionary) -> Pdf {
    let mut p = Pdf::new();
    p.insert_blank(0, 300.0, 200.0).unwrap();
    let font = p.doc.add_object(dictionary! {
        "Type" => "Font", "Subtype" => "Type1", "BaseFont" => "Helvetica",
        "Encoding" => "WinAnsiEncoding",
    });
    let s = p
        .doc
        .add_object(Stream::new(Dictionary::new(), content.as_bytes().to_vec()));
    let page = p.page_ids()[0];
    let d = p.doc.get_dictionary_mut(page).unwrap();
    d.set("Contents", s);
    d.set(
        "Resources",
        dictionary! { "Font" => dictionary! { "F1" => font }, "Properties" => props },
    );
    p
}

fn make(content: &str) -> Pdf {
    make_with(content, Dictionary::new())
}

/// "Old words Kept", the last word's letters drawn as "Kept" with "KEPT" attached, as small
/// capitals are.
const AFTER: &str = "BT /F1 20 Tf 20 100 Td (Old words ) Tj \
                     /Span <</ActualText (KEPT)>> BDC (Kept) Tj EMC ET";

/// The same, its first show outside the stretch in another character spacing, so that the
/// line is written on top, not in place.
const SPACED: &str = "BT /F1 20 Tf 20 100 Td (Old words ) Tj 0.5 Tc \
                      /Span <</ActualText (KEPT)>> BDC (Kept) Tj EMC ET";

/// "Drop words", its first letter a drop capital with "DROP" attached.
const FIRST: &str = "BT /F1 20 Tf 20 100 Td /Span <</ActualText (DROP)>> BDC (D) Tj EMC \
                     (rop words) Tj ET";

/// A tag as the content has it: its name, its properties (named ones looked up), and the
/// letters its shows draw (WinAnsi, read as Latin-1), those of tags inside it too.
#[derive(Debug)]
struct Tag {
    name: String,
    props: Dictionary,
    letters: String,
}

impl Tag {
    /// Its attached text, if it has any.
    fn alt(&self) -> Option<String> {
        let o = self.props.get(b"ActualText").ok()?;
        let b = o.as_str().ok()?;
        Some(match b {
            [0xfe, 0xff, rest @ ..] => {
                let units: Vec<u16> = (rest.chunks(2))
                    .map(|c| u16::from_be_bytes([c[0], *c.get(1).unwrap_or(&0)]))
                    .collect();
                String::from_utf16_lossy(&units)
            }
            _ => b.iter().map(|&c| c as char).collect(),
        })
    }

    fn mcid(&self) -> Option<i64> {
        self.props.get(b"MCID").ok()?.as_i64().ok()
    }
}

/// Page 0's tags in the order they open, and the letters drawn outside any tag with
/// attached text.
fn tags(p: &Pdf) -> (Vec<Tag>, String) {
    let page = p.page_ids()[0];
    let ops = Content::decode(&p.doc.get_page_content(page))
        .unwrap()
        .operations;
    let res = p
        .doc
        .get_dictionary(page)
        .unwrap()
        .get(b"Resources")
        .unwrap();
    let res = match res {
        Object::Reference(id) => p.doc.get_dictionary(*id).unwrap(),
        o => o.as_dict().unwrap(),
    };
    let named = res
        .get(b"Properties")
        .ok()
        .and_then(|o| o.as_dict().ok())
        .cloned()
        .unwrap_or_default();
    let (mut tags, mut outside, mut open) = (Vec::new(), String::new(), Vec::new());
    for o in ops {
        match o.operator.as_str() {
            "BDC" | "BMC" => {
                let props = match o.operands.get(1) {
                    Some(Object::Dictionary(d)) => d.clone(),
                    Some(Object::Name(n)) => match named.get(n) {
                        Ok(Object::Dictionary(d)) => d.clone(),
                        Ok(Object::Reference(id)) => p.doc.get_dictionary(*id).unwrap().clone(),
                        _ => Dictionary::new(),
                    },
                    _ => Dictionary::new(),
                };
                let name = o.operands[0]
                    .as_name()
                    .map_or(String::new(), |n| String::from_utf8_lossy(n).into_owned());
                open.push(tags.len());
                tags.push(Tag {
                    name,
                    props,
                    letters: String::new(),
                });
            }
            "EMC" => {
                open.pop();
            }
            "Tj" | "TJ" | "'" | "\"" => {
                let mut drawn = String::new();
                for x in &o.operands {
                    let strings = match x {
                        Object::Array(a) => a.iter().collect(),
                        x => vec![x],
                    };
                    for s in strings {
                        if let Object::String(b, _) = s {
                            drawn.extend(b.iter().map(|&c| c as char));
                        }
                    }
                }
                for &k in &open {
                    tags[k].letters.push_str(&drawn);
                }
                if !open.iter().any(|&k| tags[k].alt().is_some()) {
                    outside.push_str(&drawn);
                }
            }
            _ => {}
        }
    }
    (tags, outside)
}

/// Page 0's stretches with attached text that draw letters: the text, and the letters,
/// spaces left out.
fn stretches(p: &Pdf) -> Vec<(String, String)> {
    let (tags, _) = tags(p);
    (tags.iter())
        .filter(|t| !t.letters.trim().is_empty())
        .filter_map(|t| Some((t.alt()?, t.letters.replace(' ', ""))))
        .collect()
}

/// Every tag of page 0 with attached text, letters or none.
fn alts(p: &Pdf) -> Vec<String> {
    tags(p).0.iter().filter_map(Tag::alt).collect()
}

/// Page 0's letters outside any stretch with attached text, spaces left out.
fn outside(p: &Pdf) -> String {
    tags(p).1.replace(' ', "")
}

/// Page 0's content as it is drawn now.
fn drawn(p: &Pdf) -> Vec<u8> {
    p.doc.get_page_content(p.page_ids()[0])
}

/// The line `old` changed to `new` as typing changes it: the characters it starts and ends
/// with unchanged kept where they were, in their parts' styles; those typed take the style
/// of the character before them.
fn typed(old: &str, spans: &[TextSpan], new: &str) -> Vec<TextPart> {
    let (old, new): (Vec<char>, Vec<char>) = (old.chars().collect(), new.chars().collect());
    let head = old.iter().zip(&new).take_while(|(a, b)| a == b).count();
    let room = old.len().min(new.len()) - head;
    let tail = (old.iter().rev().zip(new.iter().rev()))
        .take_while(|(a, b)| a == b)
        .count()
        .min(room);
    let mut parts: Vec<TextPart> = Vec::new();
    let mut span = 0;
    for (j, &c) in new.iter().enumerate() {
        let origin = match j {
            _ if j < head => Some(j),
            _ if j >= new.len() - tail => Some(old.len() + j - new.len()),
            _ => None,
        };
        if let Some(k) = origin {
            let at = spans.iter().position(|s| s.chars.0 <= k && k < s.chars.1);
            span = at.unwrap_or(span);
        }
        let s = &spans[span];
        match parts.last_mut() {
            Some(p) if p.font == PartFont::Span(span) => {
                p.text.push(c);
                p.origins.push(origin);
            }
            _ => parts.push(TextPart {
                text: c.to_string(),
                font: PartFont::Span(span),
                size: s.font_size,
                color: s.fill.unwrap_or([0.0; 3]),
                rise: s.rise,
                from: None,
                origins: vec![origin],
            }),
        }
    }
    parts
}

/// Page 0's line holding `has`.
fn line(p: &Pdf, has: &str) -> limarepdf_core::PageObject {
    let objs = p.page_objects(0).unwrap().into_iter();
    (objs.filter(|o| o.kind == ObjectKind::Text))
        .find(|o| o.text.as_deref().is_some_and(|t| t.contains(has)))
        .unwrap_or_else(|| panic!("no line with {has:?}"))
}

/// Page 0's line holding `from` with it typed as `to`, in its parts as the app edits them.
fn retype(p: &mut Pdf, from: &str, to: &str) -> limarepdf_core::Result<()> {
    let o = line(p, from);
    let old = o.text.clone().unwrap();
    let e = TextEdit::in_parts(typed(&old, &o.spans, &old.replacen(from, to, 1)));
    p.edit_text(0, o.id, &e)
}

/// Page 0's line holding `from` with it typed as `to`, in one style: in `font` when given
/// (the line written anew), else as a change of words alone.
fn retype_plain(
    p: &mut Pdf,
    from: &str,
    to: &str,
    font: Option<&str>,
) -> limarepdf_core::Result<()> {
    let o = line(p, from);
    let old = o.text.clone().unwrap();
    let e = TextEdit {
        text: old.replacen(from, to, 1),
        font: font.map(FontChoice::from_name),
        size: None,
        color: None,
        parts: Vec::new(),
    };
    p.edit_text(0, o.id, &e)
}

/// The edit was refused as one that can't keep a stretch's attached text, and the page is as
/// it was.
fn refused(p: &Pdf, was: &[u8], r: limarepdf_core::Result<()>, what: &str) {
    let e = r.err().unwrap_or_else(|| panic!("{what}: not refused"));
    let msg = e.to_string();
    assert!(
        msg.contains("other apps read") && msg.contains("nothing was changed"),
        "{what}: {msg}"
    );
    assert!(drawn(p) == was, "{what}: the page changed");
}

#[test]
fn the_fixtures_read_as_drawn() {
    for (content, text, want) in [
        (AFTER, "Old words Kept", ("KEPT", "Kept")),
        (SPACED, "Old words Kept", ("KEPT", "Kept")),
        (FIRST, "Drop words", ("DROP", "D")),
    ] {
        let p = make(content);
        let objs = p.page_objects(0).unwrap();
        let texts: Vec<_> = objs.iter().filter_map(|o| o.text.as_deref()).collect();
        assert_eq!(texts, [text]);
        assert_eq!(stretches(&p), [(want.0.into(), want.1.into())]);
    }
}

/// A stretch whose letters an edit leaves as they are keeps its attached text around them,
/// whether the line is written in place of its first show or on top, and whether the edit
/// is before or after it; the old tag, which no longer draws them, loses it, so nothing
/// else reads as it.
#[test]
fn a_stretch_an_edit_leaves_as_it_is_keeps_its_text_around_its_letters() {
    for (what, content, from, to, want, rest) in [
        (
            "in place",
            AFTER,
            "Old",
            "New",
            ("KEPT", "Kept"),
            "Newwords",
        ),
        ("on top", SPACED, "Old", "New", ("KEPT", "Kept"), "Newwords"),
        (
            "after it",
            FIRST,
            "words",
            "wards",
            ("DROP", "D"),
            "ropwards",
        ),
    ] {
        let mut p = make(content);
        let was = line(&p, from).text.unwrap();
        retype(&mut p, from, to).unwrap_or_else(|e| panic!("{what}: {e}"));
        assert_eq!(
            line(&p, to).text.unwrap(),
            was.replacen(from, to, 1),
            "{what}"
        );
        assert_eq!(
            stretches(&p),
            [(want.0.to_string(), want.1.to_string())],
            "{what}: {:?}",
            tags(&p).0
        );
        assert_eq!(alts(&p), [want.0], "{what}: {:?}", tags(&p).0);
        assert_eq!(outside(&p), rest, "{what}");
    }
}

/// A stretch whose letters an edit changes, wholly or in part, or removes, loses its
/// attached text, its tag and other properties staying; the letters typed read as typed.
#[test]
fn a_stretch_whose_letters_are_changed_loses_its_text() {
    let both = "BT /F1 20 Tf 20 100 Td (Old ) Tj \
                /Span <</ActualText (KEPT HERE) /Lang (en)>> BDC (Kept here) Tj EMC ET";
    for (what, content, from, to) in [
        ("changed", AFTER, "Kept", "Kapt"),
        ("in part", both, "here", "there"),
        ("removed", AFTER, "Old words Kept", ""),
        ("the drop capital", FIRST, "Drop", "Crop"),
    ] {
        let mut p = make(content);
        retype(&mut p, from, to).unwrap_or_else(|e| panic!("{what}: {e}"));
        assert!(alts(&p).is_empty(), "{what}: {:?}", tags(&p).0);
        let spans: Vec<_> = tags(&p)
            .0
            .into_iter()
            .filter(|t| t.name == "Span")
            .collect();
        assert_eq!(spans.len(), 1, "{what}: {spans:?}");
        if what == "in part" {
            assert!(spans[0].props.has(b"Lang"), "{what}: {spans:?}");
        }
    }
}

/// The copy around the letters kept doesn't claim the structure tree's place for them
/// (`/MCID`): the old tag keeps it, so the tree points at one tag only.
#[test]
fn a_kept_stretchs_tag_is_copied_without_its_place_in_the_structure() {
    let content = "BT /F1 20 Tf 20 100 Td (Old words ) Tj \
                   /Span <</MCID 0 /ActualText (KEPT)>> BDC (Kept) Tj EMC ET";
    let mut p = make(content);
    retype(&mut p, "Old", "New").unwrap();
    assert_eq!(stretches(&p), [("KEPT".into(), "Kept".into())]);
    let (tags, _) = tags(&p);
    let claims: Vec<_> = tags.iter().filter(|t| t.mcid() == Some(0)).collect();
    assert_eq!(claims.len(), 1, "{tags:?}");
    assert!(claims[0].alt().is_none(), "{tags:?}");
    let kept = tags.iter().find(|t| t.alt().is_some()).unwrap();
    assert_eq!(kept.mcid(), None, "{tags:?}");
}

/// A stretch whose properties are named in the page's resources (`/Span /P1 BDC`) is kept
/// as one written inline is; changed, it loses its text, the page's named properties staying
/// as they were for anything else that names them.
#[test]
fn a_stretch_with_named_properties_is_kept_or_loses_its_text_as_one_inline() {
    let content = "BT /F1 20 Tf 20 100 Td (Old words ) Tj /Span /P1 BDC (Kept) Tj EMC ET";
    let props =
        || dictionary! { "P1" => dictionary! { "ActualText" => Object::string_literal("KEPT") } };
    let mut p = make_with(content, props());
    assert_eq!(stretches(&p), [("KEPT".into(), "Kept".into())]);
    retype(&mut p, "Old", "New").unwrap();
    assert_eq!(
        stretches(&p),
        [("KEPT".into(), "Kept".into())],
        "{:?}",
        tags(&p).0
    );
    assert_eq!(alts(&p), ["KEPT"], "{:?}", tags(&p).0);

    let mut p = make_with(content, props());
    retype(&mut p, "Kept", "Kapt").unwrap();
    assert!(alts(&p).is_empty(), "{:?}", tags(&p).0);
    let page = p.page_ids()[0];
    let res = p
        .doc
        .get_dictionary(page)
        .unwrap()
        .get(b"Resources")
        .unwrap();
    let named = res
        .as_dict()
        .unwrap()
        .get(b"Properties")
        .unwrap()
        .as_dict()
        .unwrap();
    assert!(named
        .get(b"P1")
        .unwrap()
        .as_dict()
        .unwrap()
        .has(b"ActualText"));
}

/// Edits that write the line as one new show, in its own style or another, can't keep a
/// stretch's text around letters they leave as they are: invisible text (an OCR layer) and
/// a line set in another font are refused then. Changing the stretch's letters, they go
/// ahead and the stretch loses its text.
#[test]
fn a_line_written_as_one_new_show_is_refused_around_a_stretch_it_keeps() {
    let hidden = "BT /F1 20 Tf 20 100 Td 3 Tr (Old words ) Tj \
                  /Span <</ActualText (KEPT)>> BDC (Kept) Tj EMC ET";
    for (what, content, font) in [
        ("invisible", hidden, None),
        ("another font", AFTER, Some("Courier")),
    ] {
        let mut p = make(content);
        let was = drawn(&p);
        let r = retype_plain(&mut p, "Old", "New", font);
        refused(&p, &was, r, what);

        let mut p = make(content);
        retype_plain(&mut p, "Kept", "Kapt", font).unwrap_or_else(|e| panic!("{what}: {e}"));
        assert!(alts(&p).is_empty(), "{what}: {:?}", tags(&p).0);
    }
}

/// A stretch drawing letters of two lines (a word broken across them) can't be kept around
/// one line's letters when that line is written anew: the edit is refused. Changing its
/// letters on the line edited, it loses its text for both lines.
#[test]
fn a_stretch_over_two_lines_is_refused_kept_and_loses_its_text_changed() {
    let content = "BT /F1 20 Tf 20 100 Td (Old words ) Tj \
                   /Span <</ActualText (KEPT LINES)>> BDC (Kept) Tj 0 -30 Td (lines) Tj EMC \
                   ( end) Tj ET";
    let p = make(content);
    let texts: Vec<_> = (p.page_objects(0).unwrap().into_iter())
        .filter_map(|o| o.text)
        .collect();
    assert_eq!(texts, ["Old words Kept", "lines end"]);

    let mut p = make(content);
    let was = drawn(&p);
    let r = retype(&mut p, "Old", "New");
    refused(&p, &was, r, "kept");

    let mut p = make(content);
    retype(&mut p, "Kept", "Kapt").unwrap();
    assert!(alts(&p).is_empty(), "{:?}", tags(&p).0);
}

/// A paragraph's first line with "harbour" drawn with "HARBOUR" attached. Rows rewritten
/// can't keep a stretch's text around letters left as they are: the edit is refused then.
/// Changing its letters, the stretch loses its text; a row kept as it was keeps its stretch
/// as it was.
#[test]
fn a_paragraph_edit_keeps_a_stretch_in_a_row_it_keeps_and_refuses_one_it_rewrites() {
    let lines = [
        "The harbour was quiet that morning, and the fishing boats sat",
        "low in the water while gulls argued over the nets drying along",
        "the wall. Nobody on the quay was in a hurry; the tide would",
        "turn when it turned, and the old men who mended the nets had",
    ];
    let mut content = String::new();
    for (k, l) in lines.iter().enumerate() {
        let y = 180.0 - 13.2 * k as f32;
        match l.strip_prefix("The harbour") {
            Some(rest) => content.push_str(&format!(
                "BT /F1 11 Tf 10 {y} Td (The ) Tj /Span <</ActualText (HARBOUR)>> BDC \
                 (harbour) Tj EMC ({rest}) Tj ET\n"
            )),
            None => content.push_str(&format!("BT /F1 11 Tf 10 {y} Td ({l}) Tj ET\n")),
        }
    }
    let make = || {
        let mut p = Pdf::new();
        p.insert_blank(0, 400.0, 200.0).unwrap();
        let font = p.doc.add_object(dictionary! {
            "Type" => "Font", "Subtype" => "Type1", "BaseFont" => "Helvetica",
            "Encoding" => "WinAnsiEncoding",
        });
        let s = (p.doc).add_object(Stream::new(Dictionary::new(), content.as_bytes().to_vec()));
        let page = p.page_ids()[0];
        let d = p.doc.get_dictionary_mut(page).unwrap();
        d.set("Contents", s);
        d.set(
            "Resources",
            dictionary! { "Font" => dictionary! { "F1" => font } },
        );
        p
    };
    let edit = |p: &mut Pdf, from: &str, to: &str| {
        let words = p.document_words();
        let blocks = p.page_blocks(0).unwrap();
        assert_eq!(blocks.len(), 1, "{blocks:?}");
        let para = p.paragraph(0, &blocks[0], &words).unwrap();
        assert_eq!(para.text, lines.join(" "));
        let setting = p.paragraph_setting(&para).unwrap();
        let new = para.text.replacen(from, to, 1);
        let parts = paragraph_typed(&para, &new);
        p.edit_paragraph(&setting, &parts).map(|_| ())
    };
    assert_eq!(stretches(&make()), [("HARBOUR".into(), "harbour".into())]);

    // A row kept: the stretch as it was.
    let mut p = make();
    edit(&mut p, "mended", "mend").unwrap();
    assert_eq!(stretches(&p), [("HARBOUR".into(), "harbour".into())]);

    // Its row rewritten, its letters kept: refused.
    let mut p = make();
    let was = drawn(&p);
    let r = edit(&mut p, "quiet", "calm");
    refused(&p, &was, r, "its row rewritten");

    // Its letters changed.
    let mut p = make();
    edit(&mut p, "harbour", "harbor").unwrap();
    assert!(alts(&p).is_empty(), "{:?}", tags(&p).0);
}

/// [`typed`] for a paragraph, its parts by [`limarepdf_core::Paragraph::span_at`].
fn paragraph_typed(para: &limarepdf_core::Paragraph, new: &str) -> Vec<TextPart> {
    let (old, new): (Vec<char>, Vec<char>) = (para.text.chars().collect(), new.chars().collect());
    let head = old.iter().zip(&new).take_while(|(a, b)| a == b).count();
    let room = old.len().min(new.len()) - head;
    let tail = (old.iter().rev().zip(new.iter().rev()))
        .take_while(|(a, b)| a == b)
        .count()
        .min(room);
    let mut parts: Vec<TextPart> = Vec::new();
    let mut span = 0;
    for (j, &c) in new.iter().enumerate() {
        let origin = match j {
            _ if j < head => Some(j),
            _ if j >= new.len() - tail => Some(old.len() + j - new.len()),
            _ => None,
        };
        if let Some(k) = origin {
            span = para.span_at(k).unwrap();
        }
        let s = &para.spans[span];
        match parts.last_mut() {
            Some(p) if p.font == PartFont::Span(span) => {
                p.text.push(c);
                p.origins.push(origin);
            }
            _ => parts.push(TextPart {
                text: c.to_string(),
                font: PartFont::Span(span),
                size: s.font_size,
                color: s.fill.unwrap_or([0.0; 3]),
                rise: s.rise,
                from: None,
                origins: vec![origin],
            }),
        }
    }
    parts
}

/// What Poppler reads after the edits, as other apps would: the kept stretch's text in
/// place of its letters, the changed one's letters as typed.
/// `cargo test -p limarepdf-core --test actual_text -- --ignored cross_check`.
#[test]
#[ignore]
fn cross_check_attached_text_after_edits_with_poppler() {
    let read = |p: &mut Pdf| tools::pdftotext(&p.to_bytes().unwrap());
    for content in [AFTER, SPACED] {
        let mut p = make(content);
        assert!(read(&mut p).contains("Old words KEPT"), "{}", read(&mut p));
        retype(&mut p, "Old", "New").unwrap();
        let text = read(&mut p);
        assert!(text.contains("New words KEPT"), "{text}");

        let mut p = make(content);
        retype(&mut p, "Kept", "Kapt").unwrap();
        let text = read(&mut p);
        assert!(text.contains("Old words Kapt"), "{text}");
        assert!(!text.contains("KEPT"), "{text}");
    }
    let mut p = make(FIRST);
    retype(&mut p, "words", "wards").unwrap();
    let text = read(&mut p);
    assert!(text.contains("DROProp wards"), "{text}");
}

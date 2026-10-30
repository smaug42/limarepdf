//! The memory an undo step says it holds (`Restore::size`, which the app's undo budget
//! counts) against what it really holds, measured by a counting allocator (batch 43).

use limarepdf_core::{FontChoice, Pdf, Restore, TextEdit, Watermark};
use lopdf::{dictionary, Dictionary, Object, Stream};
use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicUsize, Ordering::Relaxed};

/// The heap's live bytes, counted on every allocation.
struct Counting;

static LIVE: AtomicUsize = AtomicUsize::new(0);

unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, l: Layout) -> *mut u8 {
        let p = System.alloc(l);
        if !p.is_null() {
            LIVE.fetch_add(l.size(), Relaxed);
        }
        p
    }

    unsafe fn alloc_zeroed(&self, l: Layout) -> *mut u8 {
        let p = System.alloc_zeroed(l);
        if !p.is_null() {
            LIVE.fetch_add(l.size(), Relaxed);
        }
        p
    }

    unsafe fn dealloc(&self, p: *mut u8, l: Layout) {
        System.dealloc(p, l);
        LIVE.fetch_sub(l.size(), Relaxed);
    }

    unsafe fn realloc(&self, p: *mut u8, l: Layout, size: usize) -> *mut u8 {
        let q = System.realloc(p, l, size);
        if !q.is_null() {
            LIVE.fetch_add(size, Relaxed);
            LIVE.fetch_sub(l.size(), Relaxed);
        }
        q
    }
}

#[global_allocator]
static HEAP: Counting = Counting;

const W: f32 = 300.0;
const H: f32 = 200.0;

/// The live heap a copy of `r` takes.
fn held(r: &Restore) -> usize {
    let start = LIVE.load(Relaxed);
    let copy = r.clone();
    let used = LIVE.load(Relaxed).saturating_sub(start);
    drop(copy);
    used
}

/// `n` pages, each with a line of text.
fn pages(n: usize) -> Pdf {
    let mut p = Pdf::new();
    for i in 0..n {
        p.insert_blank(i, W, H).unwrap();
    }
    let font = FontChoice::default();
    for i in 0..n {
        let line = format!("Page {} of the walrus survey", i + 1);
        p.add_text(i, (20.0, 40.0), &line, font, 14.0, [0.0; 3])
            .unwrap();
    }
    p
}

/// One page of forty lines of text, so a line edit rewrites a content stream of some size.
fn page_of_lines() -> Pdf {
    let mut p = Pdf::new();
    p.insert_blank(0, 600.0, 800.0).unwrap();
    let font = FontChoice::default();
    for i in 0..40 {
        let line = format!("Line {i} of the harbour survey, counting walruses on the rocks");
        let y = 20.0 + i as f32 * 18.0;
        p.add_text(0, (20.0, y), &line, font, 12.0, [0.0; 3])
            .unwrap();
    }
    p
}

/// `n` pages under `Pages` nodes of ten each: the first page change flattens the tree.
fn nested(n: usize) -> Pdf {
    let mut p = Pdf::new();
    let root = (p.doc.catalog().unwrap().get(b"Pages").unwrap())
        .as_reference()
        .unwrap();
    let mut groups = Vec::new();
    for g in 0..n.div_ceil(10) {
        let node = p.doc.new_object_id();
        let mut kids = Vec::new();
        for i in g * 10..((g + 1) * 10).min(n) {
            let text = format!("BT /F1 12 Tf 20 150 Td (Page {}) Tj ET", i + 1);
            let c = p
                .doc
                .add_object(Stream::new(Dictionary::new(), text.into_bytes()));
            let page = p.doc.add_object(dictionary! {
                "Type" => "Page", "Parent" => node,
                "MediaBox" => vec![0.into(), 0.into(), W.into(), H.into()],
                "Contents" => c,
            });
            kids.push(Object::Reference(page));
        }
        let count = kids.len() as i64;
        p.doc.objects.insert(
            node,
            Object::Dictionary(dictionary! {
                "Type" => "Pages", "Parent" => root, "Kids" => kids, "Count" => count,
            }),
        );
        groups.push(Object::Reference(node));
    }
    let d = p.doc.get_dictionary_mut(root).unwrap();
    d.set("Kids", groups);
    d.set("Count", n as i64);
    p
}

/// A PNG of noise, which compresses poorly.
fn noise_png(w: u32, h: u32) -> Vec<u8> {
    let mut seed = 0x2545_f491_u32;
    let img = image::RgbImage::from_fn(w, h, |_, _| {
        seed ^= seed << 13;
        seed ^= seed >> 17;
        seed ^= seed << 5;
        let [r, g, b, _] = seed.to_le_bytes();
        image::Rgb([r, g, b])
    });
    let mut out = std::io::Cursor::new(Vec::new());
    img.write_to(&mut out, image::ImageFormat::Png).unwrap();
    out.into_inner()
}

/// The Restore that undoes `edit` on `p`.
fn undo_of(p: &mut Pdf, edit: impl FnOnce(&mut Pdf)) -> Restore {
    let before = p.before();
    edit(p);
    before.undo(p)
}

/// What an undo step says it holds is within a third of what it really holds, for small
/// and large steps alike. One test, so no other test's allocations are counted.
#[test]
fn a_restore_says_about_what_it_holds() {
    let mut cases: Vec<(&str, Restore)> = Vec::new();

    let mut p = page_of_lines();
    let id = (p.page_objects(0).unwrap())
        .into_iter()
        .find(|o| o.text.is_some())
        .unwrap()
        .id;
    let edit = TextEdit {
        text: "Line 0 of the harbour survey, counting seals instead".into(),
        font: None,
        size: None,
        color: None,
        parts: vec![],
    };
    let r = undo_of(&mut p, |p| {
        p.edit_texts(0, &[(id, edit)]).unwrap();
    });
    cases.push(("the words of a line", r));

    let mut p = pages(50);
    let all: Vec<usize> = (0..50).collect();
    let r = undo_of(&mut p, |p| {
        p.add_watermark(&all, &Watermark::default()).unwrap()
    });
    cases.push(("a watermark on 50 pages", r));

    let mut p = pages(1);
    let png = noise_png(600, 400);
    let r = undo_of(&mut p, |p| {
        p.add_image(0, [20.0, 20.0, 280.0, 180.0], &png).unwrap()
    });
    let redo = p.restore(r.clone());
    cases.push(("a picture added", r));
    cases.push(("a picture's Redo", redo));

    let mut p = nested(200);
    let r = undo_of(&mut p, |p| p.rotate_pages(&[0], 90).unwrap());
    cases.push(("200 nested pages flattened", r));

    let mut off = Vec::new();
    for (what, r) in &cases {
        let real = held(r);
        let said = r.size();
        let ratio = said as f64 / real.max(1) as f64;
        println!(
            "{what}: says {said} bytes, holds {real} ({ratio:.2}), {} objects",
            r.objects()
        );
        if !(0.75..=1.33).contains(&ratio) {
            off.push(format!("{what}: says {said}, holds {real} ({ratio:.2})"));
        }
    }
    assert!(off.is_empty(), "{off:#?}");
}

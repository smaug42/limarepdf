//! The memory hayro 0.9's glyph cache keeps (PB-11, `passes`), measured by a counting
//! allocator, in a file of its own as the count is the whole process's. hayro keeps a picture
//! of every glyph it fills, at every size, for as long as its render cache, and has no way to
//! let them go.

use limarepdf_core::render;
use limarepdf_core::Pdf;
use lopdf::{dictionary, Stream};
use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicUsize, Ordering::Relaxed};
use std::sync::Mutex;

/// Held by each test, so that one counts only its own.
static ONE: Mutex<()> = Mutex::new(());

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

/// A page 300 points square showing `n` letters of Helvetica, each at a size of its own from
/// 40 points, a hundredth of a point apart, so that no two share a picture in the cache.
fn page(n: usize) -> Vec<u8> {
    let mut p = Pdf::new();
    p.insert_blank(0, 300.0, 300.0).unwrap();
    let letters = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz";
    let mut content = String::from("BT\n");
    for i in 0..n {
        let (x, y) = (10 + (i % 5) * 55, 10 + (i / 5 % 5) * 55);
        let size = 40.0 + i as f32 * 0.01;
        let c = letters[i % letters.len()] as char;
        content += &format!("/F {size:.2} Tf 1 0 0 1 {x} {y} Tm ({c}) Tj\n");
    }
    content += "ET\n";
    let font = dictionary! { "Type" => "Font", "Subtype" => "Type1", "BaseFont" => "Helvetica" };
    let content = p
        .doc
        .add_object(Stream::new(dictionary! {}, content.into_bytes()));
    let pid = p.page_ids()[0];
    let d = p.doc.get_dictionary_mut(pid).unwrap();
    d.set("Contents", content);
    d.set(
        "Resources",
        dictionary! { "Font" => dictionary! { "F" => font } },
    );
    p.snapshot().unwrap()
}

/// The heap a renderer of `bytes` keeps over what it began with, after drawing its page at
/// 3×, large enough for the page to be weighed, and then at each of `scales`: after the first
/// draw, and after the last. `tile`: draw only the top left 512 pixels square, as the app
/// draws a page zoomed in.
fn kept(bytes: Vec<u8>, scales: &[f32], tile: bool) -> (usize, usize) {
    let hp = render::load(bytes, None).unwrap();
    let r = render::Renderer::new(&hp);
    let draw = |s: f32| {
        if tile {
            let at = render::Region {
                x: 0,
                y: 0,
                width: 512,
                height: 512,
            };
            r.render_region(0, s, at).unwrap();
        } else {
            r.render(0, s).unwrap();
        }
    };
    let start = LIVE.load(Relaxed);
    draw(3.0);
    let first = LIVE.load(Relaxed).saturating_sub(start);
    for s in scales {
        draw(*s);
    }
    (first, LIVE.load(Relaxed).saturating_sub(start))
}

const MB: f64 = 1e6;

/// Drawn at 40 zooms, whole or a tile at a time, 200 glyphs leave a set of pictures for each;
/// with the cache held to 16 MB, it's let go when the zooms' glyphs would pass that.
#[test]
fn glyphs_kept_from_zoom_to_zoom_are_held_to_a_budget() {
    let _one = ONE.lock().unwrap_or_else(|e| e.into_inner());
    render::glyph_seam::hold_to(16.0 * MB);
    for tile in [false, true] {
        let scales: Vec<f32> = (0..40).map(|i| 3.0 + i as f32 * 0.01).collect();
        let (first, last) = kept(page(200), &scales, tile);
        println!(
            "tile {tile}: first {:.1} MB, last {:.1} MB",
            first as f64 / MB,
            last as f64 / MB
        );
        // The first page's glyphs are kept: the cache is used.
        assert!(first as f64 > 0.5 * MB, "tile {tile}: first {first}");
        assert!((last as f64) < 16.0 * MB, "tile {tile}: last {last}");
    }
}

/// 1,000 glyphs, drawn once, would take more than the 4 MB the cache is held to: the page is
/// drawn without it, and leaves none of them.
#[test]
fn a_page_whose_glyphs_pass_the_budget_keeps_none() {
    let _one = ONE.lock().unwrap_or_else(|e| e.into_inner());
    render::glyph_seam::hold_to(4.0 * MB);
    let (first, _) = kept(page(1000), &[], false);
    println!("kept {:.1} MB", first as f64 / MB);
    assert!((first as f64) < 2.0 * MB, "kept {first}");
}

//! The memory drawing a page filled many times with mesh patterns takes (PB-11,
//! `mesh_fill.rs`), measured by a counting allocator, in a file of its own as the count is
//! the whole process's. hayro 0.9 samples a mesh pattern into a picture of all a fill
//! covers and holds each until the pass ends; drawn as `sh`, only the mesh's bounds.

use limarepdf_core::render;
use limarepdf_core::Pdf;
use lopdf::{dictionary, Dictionary, Object, Stream};
use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicUsize, Ordering::Relaxed};

/// The heap's live bytes, and the most since last set, counted on every allocation.
struct Counting;

static LIVE: AtomicUsize = AtomicUsize::new(0);
static PEAK: AtomicUsize = AtomicUsize::new(0);

fn grew(by: usize) {
    let now = LIVE.fetch_add(by, Relaxed) + by;
    PEAK.fetch_max(now, Relaxed);
}

unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, l: Layout) -> *mut u8 {
        let p = System.alloc(l);
        if !p.is_null() {
            grew(l.size());
        }
        p
    }

    unsafe fn alloc_zeroed(&self, l: Layout) -> *mut u8 {
        let p = System.alloc_zeroed(l);
        if !p.is_null() {
            grew(l.size());
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
            grew(size);
            LIVE.fetch_sub(l.size(), Relaxed);
        }
        q
    }
}

#[global_allocator]
static HEAP: Counting = Counting;

const FILLS: usize = 30;

/// A Letter page filled `FILLS` times, whole, each time with a pattern of one triangle 80
/// by 120 points, as figures converted from PostScript paint their shadings.
fn page() -> Pdf {
    let mut p = Pdf::new();
    p.insert_blank(0, 612.0, 792.0).unwrap();
    let mut patterns = Dictionary::new();
    let mut content = String::new();
    for i in 0..FILLS {
        let (x, y) = (20.0 + (i % 6) as f32 * 95.0, 20.0 + (i / 6) as f32 * 150.0);
        let mut data = Vec::new();
        for (cx, cy, rgb) in [
            (x, y, [255, 0, 0]),
            (x + 80.0, y, [0, 255, 0]),
            (x, y + 120.0, [0, 0, 255]),
        ] {
            data.push(0);
            data.extend(((cx / 612.0 * 65535.0) as u16).to_be_bytes());
            data.extend(((cy / 792.0 * 65535.0) as u16).to_be_bytes());
            data.extend(rgb);
        }
        let decode = [0, 612, 0, 792, 0, 1, 0, 1, 0, 1]
            .map(Object::from)
            .to_vec();
        let shading = dictionary! {
            "ShadingType" => 4,
            "ColorSpace" => "DeviceRGB",
            "BitsPerCoordinate" => 16,
            "BitsPerComponent" => 8,
            "BitsPerFlag" => 8,
            "Decode" => decode,
        };
        let shading = p.doc.add_object(Stream::new(shading, data));
        let pattern = dictionary! { "Type" => "Pattern", "PatternType" => 2, "Shading" => shading };
        patterns.set(format!("M{i}"), pattern);
        content += &format!("/Pattern cs /M{i} scn 0 0 612 792 re f\n");
    }
    let content = p
        .doc
        .add_object(Stream::new(dictionary! {}, content.into_bytes()));
    let pid = p.page_ids()[0];
    let d = p.doc.get_dictionary_mut(pid).unwrap();
    d.set("Contents", content);
    d.set("Resources", dictionary! { "Pattern" => patterns });
    p
}

/// The most heap drawing the first page of `bytes` at 144 dpi took, over what it began with.
fn drawing(bytes: Vec<u8>) -> usize {
    let hp = render::load(bytes, None).unwrap();
    let r = render::Renderer::new(&hp);
    let start = LIVE.load(Relaxed);
    PEAK.store(start, Relaxed);
    let b = r.render(0, 2.0).unwrap();
    let peak = PEAK.load(Relaxed) - start;
    assert!(b.rgba.chunks(4).any(|p| p[..3] != [255; 3]));
    peak
}

/// Drawn as it is, the page takes a picture of all of it for each fill, 30 of them; drawn
/// from the copy pages are drawn from, the fills redrawn as `sh`, one of each triangle.
#[test]
fn fills_with_mesh_patterns_take_memory_for_their_meshes_only() {
    let mut p = page();
    let before = drawing(p.snapshot().unwrap());
    let after = drawing(p.drawing_snapshot().unwrap());
    let mb = |n: usize| n as f64 / 1e6;
    println!("as it is {:.1} MB, redrawn {:.1} MB", mb(before), mb(after));
    assert!(
        after * 8 < before,
        "as it is {:.1} MB, redrawn {:.1} MB",
        mb(before),
        mb(after)
    );
}

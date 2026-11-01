//! Lining objects up on a page (NEW-08, batch 46's stage 5): Align and Distribute for the
//! objects selected with the arrow, and the snapping of an object moved or resized to the
//! page's and the other objects' edges and middles, as word processors line up shapes and
//! pictures. Boxes are page points as shown, y down: `[left, top, right, bottom]`.

use super::{shifted, union};

/// Where Align puts the objects: their left edges, middles across or right edges
/// together, or their tops, middles down or bottoms.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Align {
    Left,
    Centre,
    Right,
    Top,
    Middle,
    Bottom,
}

/// Which way Distribute spaces the objects out: across the page, or down it.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Axis {
    Across,
    Down,
}

/// How close to a line an edge or middle snaps to it, in pixels on screen.
pub const SNAP_PX: f32 = 6.0;

/// Less than this far (points) is where it is already.
const STILL: f32 = 0.01;

/// The middle of box `b` across (`i` 0) or down (`i` 1).
fn mid(b: [f32; 4], i: usize) -> f32 {
    (b[i] + b[i + 2]) / 2.0
}

/// Box `b` of object `id` moved by `(dx, dy)`, if that moves it.
fn moved(id: usize, b: [f32; 4], dx: f32, dy: f32) -> Option<(usize, [f32; 4])> {
    (dx.abs() > STILL || dy.abs() > STILL).then(|| (id, shifted(b, dx, dy)))
}

/// The objects' boxes `boxes` aligned as `how` says: two or more by the box round them all,
/// as word processors align objects to each other; one alone by the page (`page`, its width
/// and height). Only the boxes that move are given.
pub fn align(boxes: &[(usize, [f32; 4])], how: Align, page: (f32, f32)) -> Vec<(usize, [f32; 4])> {
    let frame = match boxes {
        [_] => [0.0, 0.0, page.0, page.1],
        _ => match boxes.iter().map(|b| b.1).reduce(union) {
            Some(round) => round,
            None => return Vec::new(),
        },
    };
    (boxes.iter())
        .filter_map(|&(id, b)| {
            let (dx, dy) = match how {
                Align::Left => (frame[0] - b[0], 0.0),
                Align::Centre => (mid(frame, 0) - mid(b, 0), 0.0),
                Align::Right => (frame[2] - b[2], 0.0),
                Align::Top => (0.0, frame[1] - b[1]),
                Align::Middle => (0.0, mid(frame, 1) - mid(b, 1)),
                Align::Bottom => (0.0, frame[3] - b[3]),
            };
            moved(id, b, dx, dy)
        })
        .collect()
}

/// The objects' boxes spaced out evenly along `axis`, three or more: the first and the last
/// (by their middles) stay where they are, and those between go with equal gaps, as word
/// processors distribute objects. Only the boxes that move are given.
pub fn distribute(boxes: &[(usize, [f32; 4])], axis: Axis) -> Vec<(usize, [f32; 4])> {
    if boxes.len() < 3 {
        return Vec::new();
    }
    let (lo, hi) = match axis {
        Axis::Across => (0, 2),
        Axis::Down => (1, 3),
    };
    let mut order = boxes.to_vec();
    let middle = |b: [f32; 4]| b[lo] + b[hi];
    order.sort_by(|a, b| (middle(a.1).total_cmp(&middle(b.1))).then(a.0.cmp(&b.0)));
    let (first, last) = (order[0].1, order[order.len() - 1].1);
    let sizes: f32 = order.iter().map(|(_, b)| b[hi] - b[lo]).sum();
    let gap = (last[hi] - first[lo] - sizes) / (order.len() - 1) as f32;
    let mut at = first[lo];
    (order.iter())
        .filter_map(|&(id, b)| {
            let d = at - b[lo];
            at += b[hi] - b[lo] + gap;
            match axis {
                Axis::Across => moved(id, b, d, 0.0),
                Axis::Down => moved(id, b, 0.0, d),
            }
        })
        .collect()
}

/// A line a move or resize snapped to, drawn while dragging: upright at `at` across, or
/// level at `at` down, from `from` to `to` along it (page points).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Guide {
    pub upright: bool,
    pub at: f32,
    pub from: f32,
    pub to: f32,
}

/// What a move or resize on a page snaps to: the page's edges and middles, and the edges
/// and middles of the page's other objects, each a line upright (where across) or level
/// (where down), with the box it is of.
#[derive(Default)]
pub struct Snaps {
    upright: Vec<(f32, [f32; 4])>,
    level: Vec<(f32, [f32; 4])>,
}

impl Snaps {
    /// The lines of a page `page` (its width and height) and of the boxes `others` on it.
    pub fn new(page: (f32, f32), others: impl IntoIterator<Item = [f32; 4]>) -> Self {
        let mut s = Snaps::default();
        for b in std::iter::once([0.0, 0.0, page.0, page.1]).chain(others) {
            for x in [b[0], mid(b, 0), b[2]] {
                s.upright.push((x, b));
            }
            for y in [b[1], mid(b, 1), b[3]] {
                s.level.push((y, b));
            }
        }
        s
    }

    fn lines(&self, level: bool) -> &[(f32, [f32; 4])] {
        if level {
            &self.level
        } else {
            &self.upright
        }
    }

    /// How far to move the places `at` (where across, or with `level` where down) for the
    /// one nearest a line to meet it, if that is within `reach`.
    fn nearest(&self, level: bool, at: &[f32], reach: f32) -> Option<f32> {
        let mut best: Option<f32> = None;
        for &f in at {
            for &(line, _) in self.lines(level) {
                let d = line - f;
                if d.abs() <= reach && best.is_none_or(|b| d.abs() < b.abs()) {
                    best = Some(d);
                }
            }
        }
        best
    }

    /// How far to move box `b` further, across and down, for one of its edges or middles
    /// to meet the nearest line within `reach` each way.
    pub fn snap_move(&self, b: [f32; 4], reach: f32) -> (f32, f32) {
        let across = self.nearest(false, &[b[0], mid(b, 0), b[2]], reach);
        let down = self.nearest(true, &[b[1], mid(b, 1), b[3]], reach);
        (across.unwrap_or(0.0), down.unwrap_or(0.0))
    }

    /// The pointer's move `(dx, dy)` for handle `h` of box `orig` (as `resize` numbers
    /// them), changed for the edges the handle moves to meet the nearest lines within
    /// `reach`. With `keep` on a corner (Shift, keeping the proportions), the nearer of the
    /// two meets its line and the other follows.
    pub fn snap_resize(
        &self,
        orig: [f32; 4],
        h: usize,
        (dx, dy): (f32, f32),
        keep: bool,
        reach: f32,
    ) -> (f32, f32) {
        let (x, y) = moving_edges(h);
        if let (true, Some(x), Some(y)) = (keep, x, y) {
            let (ow, oh) = (orig[2] - orig[0], orig[3] - orig[1]);
            if ow < 0.5 || oh < 0.5 {
                return (dx, dy);
            }
            let b = super::resize(orig, h, dx, dy, true);
            // The scale each snap would give, the nearer one chosen.
            let grow = |i: usize, d: f32| if i < 2 { -d } else { d };
            let wide = (self.nearest(false, &[b[x]], reach))
                .map(|d| (d.abs(), (b[2] - b[0] + grow(x, d)) / ow));
            let tall = (self.nearest(true, &[b[y]], reach))
                .map(|d| (d.abs(), (b[3] - b[1] + grow(y, d)) / oh));
            let s = match (wide, tall) {
                (Some(w), Some(t)) => Some(if w.0 <= t.0 { w.1 } else { t.1 }),
                (w, t) => w.or(t).map(|w| w.1),
            };
            return match s.filter(|&s| s > 0.0) {
                // The pointer's move that gives that scale.
                Some(s) => (grow(x, ow * (s - 1.0)), grow(y, oh * (s - 1.0))),
                None => (dx, dy),
            };
        }
        let snap = |i: Option<usize>, d: f32| {
            let Some(i) = i else {
                return d;
            };
            let more = self.nearest(i % 2 == 1, &[orig[i] + d], reach);
            d + more.unwrap_or(0.0)
        };
        (snap(x, dx), snap(y, dy))
    }

    /// The guides for box `b`: a line through each of the places `xs` (across) and `ys`
    /// (down) that meets a line here, drawn along `b` and the boxes it meets there.
    pub fn guides(&self, b: [f32; 4], xs: &[f32], ys: &[f32]) -> Vec<Guide> {
        let mut out: Vec<Guide> = Vec::new();
        for (level, places) in [(false, xs), (true, ys)] {
            let (lo, hi) = if level { (0, 2) } else { (1, 3) };
            for &at in places {
                let meets = (self.lines(level).iter()).filter(|(line, _)| (line - at).abs() < 0.05);
                let mut span: Option<(f32, f32)> = None;
                for (_, o) in meets {
                    let (from, to) = span.unwrap_or((b[lo], b[hi]));
                    span = Some((from.min(o[lo]), to.max(o[hi])));
                }
                let Some((from, to)) = span else {
                    continue;
                };
                let same = |g: &Guide| g.upright != level && (g.at - at).abs() < 0.05;
                if !out.iter().any(same) {
                    out.push(Guide {
                        upright: !level,
                        at,
                        from,
                        to,
                    });
                }
            }
        }
        out
    }
}

/// The edges handle `h` moves: the one across (0 left, 2 right) and the one down (1 top, 3
/// bottom), as `resize` numbers the handles from the top left, clockwise.
pub fn moving_edges(h: usize) -> (Option<usize>, Option<usize>) {
    let x = match h {
        0 | 6 | 7 => Some(0),
        2..=4 => Some(2),
        _ => None,
    };
    let y = match h {
        0..=2 => Some(1),
        4..=6 => Some(3),
        _ => None,
    };
    (x, y)
}

/// The places across and down of box `b` that a move lines up: its edges and middles.
pub fn features(b: [f32; 4]) -> ([f32; 3], [f32; 3]) {
    ([b[0], mid(b, 0), b[2]], [b[1], mid(b, 1), b[3]])
}

#[cfg(test)]
mod tests {
    use super::*;

    const PAGE: (f32, f32) = (612.0, 792.0);
    /// A photo, and a narrower caption under it, off to the right.
    const PHOTO: [f32; 4] = [100.0, 100.0, 300.0, 250.0];
    const CAPTION: [f32; 4] = [150.0, 260.0, 330.0, 274.0];

    fn close(a: [f32; 4], b: [f32; 4]) -> bool {
        a.iter().zip(&b).all(|(x, y)| (x - y).abs() < 1e-3)
    }

    fn after(boxes: &[(usize, [f32; 4])], moved: &[(usize, [f32; 4])]) -> Vec<[f32; 4]> {
        (boxes.iter())
            .map(|&(id, b)| moved.iter().find(|m| m.0 == id).map_or(b, |m| m.1))
            .collect()
    }

    /// Two or more objects line up by the box round them all, keeping their sizes; one
    /// alone by the page.
    #[test]
    fn objects_align_to_each_other_and_one_to_the_page() {
        let both = [(0, PHOTO), (1, CAPTION)];
        // How far the photo and the caption move, across and down.
        let cases = [
            (Align::Left, (0.0, 0.0), (-50.0, 0.0)),
            (Align::Right, (30.0, 0.0), (0.0, 0.0)),
            (Align::Centre, (15.0, 0.0), (-25.0, 0.0)),
            (Align::Top, (0.0, 0.0), (0.0, -160.0)),
            (Align::Bottom, (0.0, 24.0), (0.0, 0.0)),
            (Align::Middle, (0.0, 12.0), (0.0, -80.0)),
        ];
        for (how, (px, py), (cx, cy)) in cases {
            let now = after(&both, &align(&both, how, PAGE));
            assert!(close(now[0], shifted(PHOTO, px, py)), "{how:?}: {now:?}");
            assert!(close(now[1], shifted(CAPTION, cx, cy)), "{how:?}: {now:?}");
        }
        // Only what moves is given: aligned left, the photo stays.
        let left = align(&both, Align::Left, PAGE);
        assert_eq!(left.iter().map(|m| m.0).collect::<Vec<_>>(), [1]);
        assert!(align(&[(0, PHOTO), (1, PHOTO)], Align::Centre, PAGE).is_empty());

        let one = [(4, PHOTO)];
        let page = |how| after(&one, &align(&one, how, PAGE))[0];
        assert!(close(page(Align::Left), [0.0, 100.0, 200.0, 250.0]));
        assert!(close(page(Align::Centre), [206.0, 100.0, 406.0, 250.0]));
        assert!(close(page(Align::Right), [412.0, 100.0, 612.0, 250.0]));
        assert!(close(page(Align::Top), [100.0, 0.0, 300.0, 150.0]));
        assert!(close(page(Align::Middle), [100.0, 321.0, 300.0, 471.0]));
        assert!(close(page(Align::Bottom), [100.0, 642.0, 300.0, 792.0]));
        assert!(align(&[], Align::Left, PAGE).is_empty());
    }

    /// Distribute leaves the first and last (by their middles) where they are and spaces
    /// the others between with equal gaps, whatever order they were selected in; fewer
    /// than three, nothing moves.
    #[test]
    fn distribute_keeps_the_outer_two_and_evens_the_gaps() {
        let boxes = [
            (0, [0.0, 0.0, 50.0, 10.0]),
            (1, [300.0, 40.0, 400.0, 60.0]),
            (2, [60.0, 20.0, 80.0, 30.0]),
            (3, [500.0, 0.0, 520.0, 10.0]),
        ];
        let moved = distribute(&boxes, Axis::Across);
        let now = after(&boxes, &moved);
        assert!(close(now[0], boxes[0].1), "{now:?}");
        assert!(close(now[3], boxes[3].1), "{now:?}");
        // 520 across, 190 of it the boxes: three gaps of 110.
        assert!(close(now[2], [160.0, 20.0, 180.0, 30.0]), "{now:?}");
        assert!(close(now[1], [290.0, 40.0, 390.0, 60.0]), "{now:?}");
        assert!(!moved.iter().any(|m| m.0 == 0 || m.0 == 3));
        // Already even: nothing moves.
        let even: Vec<_> = now.into_iter().enumerate().collect();
        assert!(distribute(&even, Axis::Across).is_empty());

        // Down, the same.
        let down = [
            (0, [0.0, 0.0, 10.0, 10.0]),
            (1, [0.0, 15.0, 10.0, 25.0]),
            (2, [0.0, 90.0, 10.0, 100.0]),
        ];
        let now = after(&down, &distribute(&down, Axis::Down));
        assert!(close(now[1], [0.0, 45.0, 10.0, 55.0]), "{now:?}");
        assert!(distribute(&down[..2], Axis::Down).is_empty());
    }

    /// A move snaps the nearest of the box's edges and middles to the nearest line within
    /// reach, each way apart; beyond reach it doesn't.
    #[test]
    fn a_move_snaps_to_the_nearest_line_within_reach() {
        let s = Snaps::new(PAGE, [PHOTO]);
        // The caption's left edge 3 points right of the photo's: back onto it.
        let caption = [103.0, 260.0, 283.0, 274.0];
        assert_eq!(s.snap_move(caption, 4.0), (-3.0, 0.0));
        assert_eq!(s.snap_move(caption, 2.0), (0.0, 0.0));
        // Its middle (195) nearer the photo's (200) than its edges are to any line.
        let off = [110.0, 500.0, 280.0, 514.0];
        assert_eq!(s.snap_move(off, 6.0), (5.0, 0.0));
        // Down, its top near the photo's bottom.
        let under = [400.0, 252.0, 450.0, 270.0];
        assert_eq!(s.snap_move(under, 4.0), (0.0, -2.0));
        // The page's own edges and middle.
        assert_eq!(s.snap_move([2.0, 500.0, 50.0, 510.0], 4.0), (-2.0, 0.0));
    }

    /// A resize snaps the edges its handle moves, the others staying; with the proportions
    /// kept, the nearer of the two and the other following.
    #[test]
    fn a_resize_snaps_the_edges_it_moves() {
        let s = Snaps::new(PAGE, [PHOTO]);
        let small = [400.0, 400.0, 420.0, 410.0];
        // The right handle (3) dragged to 298: onto the photo's right edge? Too far (122).
        assert_eq!(s.snap_resize(small, 3, (5.0, 0.0), false, 4.0), (5.0, 0.0));
        let near = [290.0, 400.0, 297.0, 410.0];
        assert_eq!(s.snap_resize(near, 3, (1.0, 7.0), false, 4.0), (3.0, 7.0));
        // The top-left corner (0), both edges near lines.
        let corner = [102.0, 252.0, 400.0, 500.0];
        let both = s.snap_resize(corner, 0, (0.0, 0.0), false, 4.0);
        assert_eq!(both, (-2.0, -2.0));
        // The bottom handle (5) moves the bottom edge alone.
        let low = [400.0, 200.0, 420.0, 248.0];
        assert_eq!(s.snap_resize(low, 5, (0.0, 0.0), false, 4.0), (0.0, 2.0));

        // Kept proportions, the bottom-right corner (4) of a 90×45 box: dragged to 297
        // across, 3 points from the photo's right edge, and no line near down.
        let b = [200.0, 500.0, 290.0, 545.0];
        let (dx, dy) = s.snap_resize(b, 4, (7.0, 3.5), true, 4.0);
        let r = crate::editor::resize(b, 4, dx, dy, true);
        assert!(close(r, [200.0, 500.0, 300.0, 550.0]), "{r:?}");
    }

    /// The guides run along the box and every box it lines up with there, one a line.
    #[test]
    fn guides_run_along_what_lines_up() {
        let other = [100.0, 400.0, 150.0, 420.0];
        let s = Snaps::new(PAGE, [PHOTO, other]);
        let caption = [100.0, 260.0, 280.0, 274.0];
        let (xs, ys) = features(caption);
        let g = s.guides(caption, &xs, &ys);
        let left = Guide {
            upright: true,
            at: 100.0,
            from: 100.0,
            to: 420.0,
        };
        assert_eq!(g, [left]);
        // On the page's middle across, from its top to its bottom.
        let centred = [206.0, 300.0, 406.0, 320.0];
        let (xs, ys) = features(centred);
        let g = s.guides(centred, &xs, &ys);
        assert_eq!(g.len(), 1);
        assert_eq!((g[0].at, g[0].from, g[0].to), (306.0, 0.0, 792.0));
        let away = [400.0, 600.0, 410.0, 610.0];
        assert!(s.guides(away, &[400.0], &[600.0]).is_empty());
    }
}

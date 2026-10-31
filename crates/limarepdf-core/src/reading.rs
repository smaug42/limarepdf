//! Reading order for a page's words, worked out from where they sit on the page rather
//! than the order the PDF draws them in. Forms and many other documents draw blocks out
//! of order (a footer first, a column of labels last), which would make a selection
//! jump about and copied text come out shuffled.
//!
//! The page is cut recursively ("XY cut"): first at the widest empty horizontal band if
//! it is clearly wider than the space between lines, else at the widest empty vertical
//! strip that separates two columns of text. What can't be cut is read as rows, top to
//! bottom, each left to right. Narrow columns, such as the page numbers of a table of
//! contents or labels side by side on a form, are read with their rows instead.

/// How wide an empty band across a region must be, in text heights, to separate blocks
/// of rows: more than the leading between lines of a paragraph.
const ROW_GAP: f32 = 0.8;
/// How wide an empty strip down a region must be, in text heights, to separate columns:
/// more than a space between words.
const COL_GAP: f32 = 1.0;
/// How wide the text on each side of such a strip must be, in text heights, to count as
/// a column.
const COL_WIDTH: f32 = 10.0;

/// The reading order of word boxes (displayed-page points, y down) as (index into
/// `boxes`, row) pairs. Rows number the visual lines from 0, in reading order.
pub(crate) fn reading_order(boxes: &[[f32; 4]]) -> Vec<(usize, usize)> {
    let mut heights: Vec<f32> = boxes
        .iter()
        .map(|b| b[3] - b[1])
        .filter(|h| *h > 0.0)
        .collect();
    heights.sort_by(f32::total_cmp);
    let h = heights.get(heights.len() / 2).copied().unwrap_or(10.0);
    let mut out = Vec::with_capacity(boxes.len());
    let mut rows = 0;
    // Regions still to read, the next on top: a list rather than recursion, as a page
    // can have tens of thousands of blocks (one per line) and run out of stack.
    let mut todo: Vec<Vec<usize>> = vec![(0..boxes.len()).collect()];
    while let Some(idx) = todo.pop() {
        match cut(boxes, idx, h) {
            Ok(parts) => todo.extend(parts.into_iter().rev()),
            Err(idx) => read_rows(boxes, idx, &mut out, &mut rows),
        }
    }
    out
}

/// The empty gaps across the boxes `idx` along `axis` (0: x, 1: y), as (start, end),
/// in order.
pub(crate) fn gaps(boxes: &[[f32; 4]], idx: &[usize], axis: usize) -> Vec<(f32, f32)> {
    let mut spans: Vec<(f32, f32)> = idx
        .iter()
        .map(|&i| (boxes[i][axis], boxes[i][axis + 2]))
        .collect();
    spans.sort_by(|a, b| a.0.total_cmp(&b.0));
    let mut out = Vec::new();
    let Some(&(_, mut end)) = spans.first() else {
        return out;
    };
    for &(s, e) in &spans[1..] {
        if s > end {
            out.push((end, s));
        }
        end = end.max(e);
    }
    out
}

/// The boxes `idx` of text height `h` cut into blocks in reading order, or `idx` back if
/// they can't be cut. Wide bands across are cut first, all at once: cutting at the widest
/// and then within each side, as the method goes, ends the same way, since a side's
/// bands are the region's bands on that side.
fn cut(boxes: &[[f32; 4]], idx: Vec<usize>, h: f32) -> Result<Vec<Vec<usize>>, Vec<usize>> {
    if idx.len() < 2 {
        return Err(idx);
    }
    let at: Vec<f32> = gaps(boxes, &idx, 1)
        .into_iter()
        .filter(|g| g.1 - g.0 >= ROW_GAP * h)
        .map(|g| (g.0 + g.1) / 2.0)
        .collect();
    if !at.is_empty() {
        // No box crosses a gap, so each lies wholly in one block.
        let mut parts = vec![Vec::new(); at.len() + 1];
        for i in idx {
            parts[at.partition_point(|&y| y <= boxes[i][1])].push(i);
        }
        return Ok(parts);
    }
    match column_at(boxes, &idx, h) {
        Some(x) => {
            let (a, b) = idx.into_iter().partition(|&i| boxes[i][0] < x);
            Ok(vec![a, b])
        }
        None => Err(idx),
    }
}

/// Where to cut the boxes `idx` of text height `h` into two columns, if anywhere: the
/// widest strip down them that separates columns.
fn column_at(boxes: &[[f32; 4]], idx: &[usize], h: f32) -> Option<f32> {
    let wider = |a: &&(f32, f32), b: &&(f32, f32)| (a.1 - a.0).total_cmp(&(b.1 - b.0));
    let cols = gaps(boxes, idx, 0);
    let (left, right) = idx.iter().fold((f32::MAX, f32::MIN), |(l, r), &i| {
        (l.min(boxes[i][0]), r.max(boxes[i][2]))
    });
    cols.iter()
        .filter(|g| {
            g.1 - g.0 >= COL_GAP * h && g.0 - left >= COL_WIDTH * h && right - g.1 >= COL_WIDTH * h
        })
        .max_by(wider)
        .map(|g| (g.0 + g.1) / 2.0)
}

/// Boxes with no wide gaps between them: group them into rows by their vertical middles
/// and read each row left to right.
fn read_rows(
    boxes: &[[f32; 4]],
    mut idx: Vec<usize>,
    out: &mut Vec<(usize, usize)>,
    rows: &mut usize,
) {
    let mid = |i: usize| (boxes[i][1] + boxes[i][3]) / 2.0;
    idx.sort_by(|&a, &b| mid(a).total_cmp(&mid(b)));
    let mut row: Vec<usize> = Vec::new();
    let mut bottom = f32::MIN;
    for i in idx {
        if !row.is_empty() && mid(i) > bottom {
            flush(boxes, &mut row, out, rows);
        }
        bottom = if row.is_empty() {
            boxes[i][3]
        } else {
            bottom.max(boxes[i][3])
        };
        row.push(i);
    }
    flush(boxes, &mut row, out, rows);
}

fn flush(
    boxes: &[[f32; 4]],
    row: &mut Vec<usize>,
    out: &mut Vec<(usize, usize)>,
    rows: &mut usize,
) {
    if row.is_empty() {
        return;
    }
    row.sort_by(|&a, &b| boxes[a][0].total_cmp(&boxes[b][0]));
    out.extend(row.drain(..).map(|i| (i, *rows)));
    *rows += 1;
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A word box 10 high: x from `x` to `x + w`, top at `y`.
    fn word(x: f32, y: f32, w: f32) -> [f32; 4] {
        [x, y, x + w, y + 10.0]
    }

    fn order(boxes: &[[f32; 4]]) -> Vec<usize> {
        reading_order(boxes).into_iter().map(|(i, _)| i).collect()
    }

    #[test]
    fn a_footer_drawn_first_is_read_last() {
        let boxes = [
            word(500.0, 750.0, 50.0), // footer
            word(30.0, 100.0, 40.0),
            word(75.0, 100.0, 40.0),
            word(30.0, 112.0, 40.0),
        ];
        assert_eq!(order(&boxes), [1, 2, 3, 0]);
        let rows: Vec<usize> = reading_order(&boxes).iter().map(|r| r.1).collect();
        assert_eq!(rows, [0, 0, 1, 2]);
    }

    #[test]
    fn a_form_is_read_row_by_row() {
        // Labels in two columns, drawn left column first, rows well apart.
        let boxes = [
            word(30.0, 100.0, 40.0),
            word(30.0, 140.0, 40.0),
            word(300.0, 100.0, 60.0),
            word(300.0, 140.0, 60.0),
        ];
        assert_eq!(order(&boxes), [0, 2, 1, 3]);
    }

    #[test]
    fn two_columns_of_text_are_read_column_by_column() {
        // Lines 12 apart (2 of leading), columns 30 apart.
        let mut boxes = Vec::new();
        for line in 0..4 {
            let y = 100.0 + 12.0 * line as f32;
            boxes.push(word(300.0, y, 200.0));
            boxes.push(word(70.0, y, 200.0));
        }
        assert_eq!(order(&boxes), [1, 3, 5, 7, 0, 2, 4, 6]);
    }

    #[test]
    fn page_numbers_stay_with_their_entries() {
        // A table of contents: titles with dot leaders, page numbers 12 to the right,
        // all page numbers drawn last.
        let mut boxes = Vec::new();
        for line in 0..3 {
            boxes.push(word(70.0, 100.0 + 12.0 * line as f32, 460.0));
        }
        for line in 0..3 {
            boxes.push(word(542.0, 100.0 + 12.0 * line as f32, 10.0));
        }
        assert_eq!(order(&boxes), [0, 3, 1, 4, 2, 5]);
    }

    #[test]
    fn a_paragraph_keeps_its_words_and_lines() {
        // Tight leading: lines overlap slightly, so no band separates them.
        let boxes = [
            word(80.0, 109.0, 30.0),
            word(30.0, 100.0, 45.0),
            word(80.0, 100.0, 30.0),
            word(30.0, 109.0, 45.0),
        ];
        let got = reading_order(&boxes);
        assert_eq!(got, [(1, 0), (2, 0), (3, 1), (0, 1)]);
    }

    #[test]
    fn nothing_or_one_word() {
        assert!(reading_order(&[]).is_empty());
        assert_eq!(reading_order(&[word(0.0, 0.0, 5.0)]), [(0, 0)]);
    }

    #[test]
    fn many_lines_far_apart_fit_a_small_stack() {
        // Each line its own block, as on a crafted page: cutting them off one at a time
        // went 30,000 calls deep, more than a 2 MiB thread (rayon's, the app's) holds.
        let boxes: Vec<[f32; 4]> = (0..30_000)
            .map(|i| word(30.0, 20.0 * i as f32, 40.0))
            .collect();
        let got = std::thread::Builder::new()
            .stack_size(2 << 20)
            .spawn(move || order(&boxes))
            .unwrap()
            .join()
            .unwrap();
        assert!(got.into_iter().eq(0..30_000));
    }

    /// The method as first written: cut at the widest gap, then each side in turn.
    fn by_widest_cut(boxes: &[[f32; 4]]) -> Vec<(usize, usize)> {
        fn go(
            b: &[[f32; 4]],
            idx: Vec<usize>,
            h: f32,
            out: &mut Vec<(usize, usize)>,
            r: &mut usize,
        ) {
            if idx.len() > 1 {
                let wider = |a: &&(f32, f32), c: &&(f32, f32)| (a.1 - a.0).total_cmp(&(c.1 - c.0));
                let row = gaps(b, &idx, 1)
                    .iter()
                    .max_by(wider)
                    .filter(|g| g.1 - g.0 >= ROW_GAP * h)
                    .map(|g| (1, (g.0 + g.1) / 2.0));
                if let Some((axis, at)) = row.or_else(|| column_at(b, &idx, h).map(|x| (0, x))) {
                    let (l, m): (Vec<usize>, Vec<usize>) =
                        idx.into_iter().partition(|&i| b[i][axis] < at);
                    go(b, l, h, out, r);
                    go(b, m, h, out, r);
                    return;
                }
            }
            read_rows(b, idx, out, r);
        }
        let mut heights: Vec<f32> = boxes
            .iter()
            .map(|b| b[3] - b[1])
            .filter(|h| *h > 0.0)
            .collect();
        heights.sort_by(f32::total_cmp);
        let h = heights.get(heights.len() / 2).copied().unwrap_or(10.0);
        let (mut out, mut rows) = (Vec::new(), 0);
        go(boxes, (0..boxes.len()).collect(), h, &mut out, &mut rows);
        out
    }

    #[test]
    fn cutting_all_wide_bands_at_once_reads_as_cutting_the_widest_first() {
        // Pseudo-random pages: rows with gaps from a few sizes (so many tie), words in
        // a few columns, some taller, some overlapping their neighbours.
        let mut seed = 0x2545_f491_u64;
        let mut next = |n: u64| {
            seed = seed.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1);
            (seed >> 33) % n
        };
        for _ in 0..500 {
            let mut boxes = Vec::new();
            let mut y = 0.0;
            for _ in 0..1 + next(40) {
                y += [2.0, 9.0, 12.0, 25.0, 40.0][next(5) as usize];
                for _ in 0..next(6) {
                    let x = [30.0, 80.0, 150.0, 300.0, 420.0][next(5) as usize] + next(20) as f32;
                    let tall = if next(8) == 0 { 14.0 } else { 0.0 };
                    boxes.push([x, y, x + 10.0 + next(120) as f32, y + 10.0 + tall]);
                }
            }
            assert_eq!(reading_order(&boxes), by_widest_cut(&boxes), "{boxes:?}");
        }
    }
}

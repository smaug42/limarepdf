//! Page labels: reading, writing, and keeping them with their pages.

use mpe_core::{LabelRange, LabelStyle, Pdf};

fn pages(n: usize) -> Pdf {
    let mut p = Pdf::new();
    for i in 0..n {
        p.insert_blank(i, 300.0, 200.0).unwrap();
    }
    p
}

fn range(page: usize, style: Option<LabelStyle>, prefix: &str, first: u32) -> LabelRange {
    LabelRange {
        page,
        style,
        prefix: prefix.into(),
        first,
    }
}

/// i, ii, then 1, 2, 3.
fn front_matter() -> Pdf {
    let mut p = pages(5);
    p.set_label_ranges(&[
        range(0, Some(LabelStyle::LowerRoman), "", 1),
        range(2, Some(LabelStyle::Decimal), "", 1),
    ])
    .unwrap();
    p
}

fn labels(p: &Pdf) -> Vec<String> {
    p.page_labels().unwrap()
}

#[test]
fn no_labels_by_default() {
    assert_eq!(pages(2).page_labels(), None);
}

#[test]
fn writes_and_reads_back_after_saving() {
    let mut p = pages(6);
    p.set_label_ranges(&[
        range(0, None, "Cover", 1),
        range(1, Some(LabelStyle::UpperRoman), "", 4),
        range(3, Some(LabelStyle::UpperLetters), "Annex-", 1),
        range(5, Some(LabelStyle::Decimal), "§ ", 10),
    ])
    .unwrap();
    let mut back = Pdf::from_bytes(&p.to_bytes().unwrap(), None).unwrap();
    assert_eq!(
        labels(&back),
        ["Cover", "IV", "V", "Annex-A", "Annex-B", "§ 10"]
    );
    back.set_label_ranges(&[]).unwrap();
    assert_eq!(back.page_labels(), None);
}

#[test]
fn ranges_that_only_carry_on_are_dropped() {
    let mut p = pages(4);
    p.set_label_ranges(&[
        range(0, Some(LabelStyle::Decimal), "", 1),
        range(2, Some(LabelStyle::Decimal), "", 3),
    ])
    .unwrap();
    assert_eq!(p.label_ranges().len(), 1);
}

#[test]
fn numbering_from_a_page_keeps_plain_numbers_before_it() {
    let mut p = pages(5);
    p.set_label_range(range(3, Some(LabelStyle::UpperLetters), "", 1))
        .unwrap();
    assert_eq!(labels(&p), ["1", "2", "3", "A", "B"]);
    // A range set inside another lasts until the next one.
    p.set_label_range(range(1, Some(LabelStyle::LowerRoman), "", 1))
        .unwrap();
    assert_eq!(labels(&p), ["1", "i", "ii", "A", "B"]);
}

#[test]
fn labels_follow_deleted_inserted_and_moved_pages() {
    let mut p = front_matter();
    assert_eq!(labels(&p), ["i", "ii", "1", "2", "3"]);

    p.delete_pages(&[2]).unwrap();
    assert_eq!(labels(&p), ["i", "ii", "2", "3"]);

    // A new page carries on the numbering of the page before it.
    p.insert_blank(2, 300.0, 200.0).unwrap();
    assert_eq!(labels(&p), ["i", "ii", "iii", "2", "3"]);

    p.reorder(&[3, 4, 0, 1, 2]).unwrap();
    assert_eq!(labels(&p), ["2", "3", "i", "ii", "iii"]);
}

#[test]
fn extracted_pages_keep_their_labels_and_appended_pages_carry_on() {
    let p = front_matter();
    assert_eq!(labels(&p.extract(&[1, 3]).unwrap()), ["ii", "2"]);

    let mut q = front_matter();
    q.append(vec![pages(2)]).unwrap();
    assert_eq!(labels(&q), ["i", "ii", "1", "2", "3", "4", "5"]);
}

#[test]
fn hostile_start_numbers_stay_small() {
    let mut p = pages(3);
    p.set_label_ranges(&[range(0, Some(LabelStyle::UpperRoman), "", u32::MAX - 1)])
        .unwrap();
    let l = labels(&p);
    assert_eq!(l[0], (u32::MAX - 1).to_string());
    assert_eq!(l[2], u32::MAX.to_string());
    assert!(p.set_label_ranges(&[range(3, None, "x", 1)]).is_err());
}

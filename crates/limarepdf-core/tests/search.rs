//! Search (batch 22): snippets cut at words, counts in words, phrases across a page break
//! (PF-38), and the index the app searches as you type with (ENH-02, ENH-03); Match case,
//! Whole words only and each hit's lines, for Replace (NEW-04a).

use limarepdf_core::{match_count, FontChoice, HitPart, Pdf, Query, SearchOptions, TextIndex};
use std::collections::BTreeSet;

/// `text` on pages that hold two lines each.
fn paged(text: &str) -> Pdf {
    Pdf::from_text(text, (400.0, 65.0)).unwrap()
}

/// The pages a hit's boxes are on.
fn pages_of(boxes: &[(usize, [f32; 4])]) -> BTreeSet<usize> {
    boxes.iter().map(|b| b.0).collect()
}

#[test]
fn snippets_are_whole_words_with_dots_where_words_are_left_out() {
    let words = "one two three four five six seven eight nine ten eleven twelve walrus \
                 thirteen fourteen fifteen sixteen seventeen eighteen nineteen twenty";
    let p = Pdf::from_text(words, (600.0, 400.0)).unwrap();
    let hits = p.search("walrus").unwrap();
    assert_eq!(hits.len(), 1);
    let s = hits[0].snippet.to_string();
    assert!(s.starts_with('…') && s.ends_with('…'), "{s:?}");
    assert!(s.contains("twelve walrus thirteen"), "{s:?}");
    let all: BTreeSet<&str> = words.split_whitespace().collect();
    for w in s.trim_matches('…').split_whitespace() {
        assert!(all.contains(w), "{w:?} is part of a word: {s:?}");
    }
    // The parts: the match by itself, the words around it, at most 40 characters a side.
    let s = &hits[0].snippet;
    assert_eq!(s.found, "walrus");
    assert!(s.before.ends_with("twelve ") && s.after.starts_with(" thirteen"));
    assert!(s.before.chars().count() <= 40 && s.after.chars().count() <= 40);
    // Shorter, still cut at words.
    assert_eq!(s.around(10, 10).to_string(), "…twelve walrus thirteen…");
    assert_eq!(s.around(6, 8).to_string(), "…walrus…");
}

#[test]
fn a_short_page_shows_all_its_words_with_no_dots() {
    let p = Pdf::from_text("the walrus ate", (400.0, 300.0)).unwrap();
    let s = &p.search("WALRUS").unwrap()[0].snippet;
    let parts = (s.before.as_str(), s.found.as_str(), s.after.as_str());
    assert_eq!(parts, ("the ", "walrus", " ate"));
    assert_eq!(s.to_string(), "the walrus ate");
}

#[test]
fn match_counts_are_in_words() {
    assert_eq!(match_count(0), "No matches");
    assert_eq!(match_count(1), "1 match");
    assert_eq!(match_count(3), "3 matches");
}

/// PF-38: a phrase running from one page to the next is one match, on the page it starts
/// on, highlighted on both.
#[test]
fn a_phrase_across_a_page_break_is_found_and_shown_on_both_pages() {
    let p = paged("first line\nthe walrus\nate fish\nlast line");
    assert_eq!(p.page_count(), 2);
    let hits = p.search("walrus ate").unwrap();
    assert_eq!(hits.len(), 1, "{hits:?}");
    assert_eq!(hits[0].page, 0);
    assert_eq!(pages_of(&hits[0].boxes), BTreeSet::from([0, 1]));
    let s = &hits[0].snippet;
    assert_eq!(s.found, "walrus ate");
    assert!(
        s.before.ends_with("the ") && s.after.starts_with(" fish"),
        "{s:?}"
    );
    // Looking at one page alone doesn't find it.
    assert!(p.find_text_matches(0, "walrus ate").unwrap().is_empty());
    assert!(p.find_text_matches(1, "walrus ate").unwrap().is_empty());
}

#[test]
fn a_word_hyphenated_at_a_page_end_is_found_whole() {
    let p = paged("one\nthe infor-\nmation age\nend");
    assert_eq!(p.page_count(), 2);
    for q in ["information", "the information age", "infor- mation"] {
        let hits = p.search(q).unwrap();
        assert_eq!(hits.len(), 1, "{q}: {hits:?}");
        assert_eq!(pages_of(&hits[0].boxes), BTreeSet::from([0, 1]), "{q}");
    }
}

/// A phrase that takes the start of the next page leaves it to that page's matches, as one
/// reading of the whole text would: "a a" here is found once, from the first page.
#[test]
fn a_phrase_across_a_page_break_is_not_found_twice() {
    let p = paged("x\nx a\na a b\ny");
    assert_eq!(p.page_count(), 2);
    let hits = p.search("a a").unwrap();
    assert_eq!(hits.len(), 1, "{hits:?}");
    assert_eq!(hits[0].page, 0);
}

/// Some text over `n` pages of two lines, with "walrus ate" across page breaks and within
/// pages.
fn walrus_pages(n: usize) -> Pdf {
    let mut text = String::new();
    for i in 0..n {
        match i % 3 {
            0 => text += &format!("page {i}\nthe walrus\n"),
            1 => text += "ate fish and the walrus ate\nmore fish\n",
            _ => text += "nothing here\nat all\n",
        }
    }
    let p = paged(&text);
    assert_eq!(p.page_count(), n);
    p
}

/// The app reads and searches a group of pages at a time, the page after each group read
/// too: that finds what searching the whole document at once finds, in the same order.
#[test]
fn the_index_finds_a_group_at_a_time_what_search_finds() {
    let p = walrus_pages(20);
    let whole = p.search("walrus ate").unwrap();
    assert!(whole.len() > 10, "{}", whole.len());
    assert!(whole.iter().any(|h| pages_of(&h.boxes).len() == 2));
    let (q, n) = (Query::new("walrus ate"), p.page_count());
    let mut ix = TextIndex::new(&p);
    let (mut hits, mut spill) = (Vec::new(), 0);
    for start in (0..n).step_by(3) {
        let end = (start + 3).min(n);
        assert!(ix.read(&p, start..(end + 1).min(n), &|| false));
        let m = ix.find(&q, start..end, spill);
        spill = m.spill;
        hits.extend(m.hits);
    }
    assert_eq!(hits, whole);
    // Another query needs nothing read again.
    let fish = ix.find(&Query::new("fish"), 0..n, 0).hits;
    assert_eq!(fish, p.search("fish").unwrap());
    assert!(ix.bytes() > 0);
}

/// Hits carry the same boxes looking at their page alone finds.
#[test]
fn hits_have_the_boxes_of_the_page_matches() {
    let p = walrus_pages(6);
    for page in 0..p.page_count() {
        let alone: Vec<Vec<[f32; 4]>> = p.find_text_matches(page, "fish").unwrap();
        let hits: Vec<Vec<[f32; 4]>> = p
            .search("fish")
            .unwrap()
            .into_iter()
            .filter(|h| h.page == page)
            .map(|h| h.boxes.into_iter().map(|b| b.1).collect())
            .collect();
        assert_eq!(hits, alone, "page {page}");
    }
}

#[test]
fn reading_stops_when_asked_and_keeps_what_was_read() {
    let p = walrus_pages(6);
    let n = p.page_count();
    let mut ix = TextIndex::new(&p);
    assert!(!ix.read(&p, 0..n, &|| true));
    assert!((0..n).all(|i| !ix.has(i)));
    assert!(ix.read(&p, 0..2, &|| false));
    assert!(ix.has(0) && ix.has(1) && !ix.has(2));
    ix.forget(0..1);
    assert!(!ix.has(0) && ix.has(1));
}

/// After an edit, the pages it changed are read again and the rest kept; once the pages
/// themselves change, everything is read again.
#[test]
fn the_index_follows_the_document() {
    let mut p = walrus_pages(6);
    let n = p.page_count();
    let mut ix = TextIndex::new(&p);
    assert!(ix.read(&p, 0..n, &|| false));
    p.add_text(
        2,
        (50.0, 30.0),
        "otter",
        FontChoice::default(),
        11.0,
        [0.0; 3],
    )
    .unwrap();
    ix.update(&p, Some(&[2]));
    assert!(!ix.has(2) && (0..n).filter(|&i| i != 2).all(|i| ix.has(i)));
    assert!(ix.read(&p, 0..n, &|| false));
    let otter = ix.find(&Query::new("otter"), 0..n, 0).hits;
    assert_eq!(otter.len(), 1);
    assert_eq!(otter[0].page, 2);
    p.insert_blank(0, 400.0, 65.0).unwrap();
    ix.update(&p, Some(&[0]));
    assert_eq!(ix.page_count(), n + 1);
    assert!((0..=n).all(|i| !ix.has(i)));
}

/// What a search with `options` finds, as the words found.
fn found(p: &Pdf, q: &str, options: SearchOptions) -> Vec<String> {
    let hits = p.search_with(q, options).unwrap();
    hits.into_iter().map(|h| h.snippet.found).collect()
}

const CASE: SearchOptions = SearchOptions {
    match_case: true,
    whole_words: false,
};
const WHOLE: SearchOptions = SearchOptions {
    match_case: false,
    whole_words: true,
};
const BOTH: SearchOptions = SearchOptions {
    match_case: true,
    whole_words: true,
};

/// NEW-04a: Match case and Whole words only, each and together, as a word processor's
/// Find has them; whitespace still matches any run of it.
#[test]
fn match_case_and_whole_words_only() {
    let p = Pdf::from_text("The Walrus and the walrus-like walruses", (600.0, 400.0)).unwrap();
    let any = SearchOptions::default();
    assert_eq!(found(&p, "walrus", any), ["Walrus", "walrus", "walrus"]);
    assert_eq!(found(&p, "Walrus", CASE), ["Walrus"]);
    assert_eq!(found(&p, "walrus", WHOLE), ["Walrus", "walrus"]);
    assert_eq!(found(&p, "walrus", BOTH), ["walrus"]);
    let spaced = found(&p, "the  WALRUS", WHOLE);
    assert_eq!(spaced, ["The Walrus", "the walrus"]);
    assert!(found(&p, "the  WALRUS", CASE).is_empty());
    // Part of a word, or a word that runs on, isn't whole.
    assert!(found(&p, "alrus", WHOLE).is_empty());
    assert!(found(&p, "walrus-lik", WHOLE).is_empty());
    assert_eq!(found(&p, "walrus-", WHOLE), ["walrus-"]);
    // The same options through the index the app searches with.
    let mut ix = TextIndex::new(&p);
    assert!(ix.read(&p, 0..1, &|| false));
    let q = Query::with("walrus", WHOLE);
    assert_eq!(q.options(), WHOLE);
    let hits = p.search_with("walrus", WHOLE).unwrap();
    assert_eq!(ix.find(&q, 0..1, 0).hits, hits);
}

/// A word hyphenated at a line end is one word, within a page and across a page break.
#[test]
fn a_word_hyphenated_at_a_line_end_is_one_whole_word() {
    let lines = Pdf::from_text("the infor-\nmation age", (400.0, 300.0)).unwrap();
    let pages = paged("one\nthe infor-\nmation age\nend");
    assert_eq!(pages.page_count(), 2);
    for p in [&lines, &pages] {
        assert_eq!(found(p, "information", WHOLE).len(), 1);
        assert!(found(p, "infor", WHOLE).is_empty());
        assert!(found(p, "mation", WHOLE).is_empty());
        assert_eq!(found(p, "mation", SearchOptions::default()).len(), 1);
    }
}

/// Each hit says what it is of the page's lines: their objects and characters, the
/// spaces between its words with them.
#[test]
fn hits_carry_their_lines_and_characters() {
    let p = Pdf::from_text("the walrus ate fish\nthe otter swam", (400.0, 300.0)).unwrap();
    let objs = p.page_objects(0).unwrap();
    let text_of = |part: &HitPart| -> String {
        let line = objs[part.object.unwrap()].text.as_deref().unwrap();
        let (a, b) = part.chars;
        line.chars().skip(a).take(b - a).collect()
    };
    let hits = p.search("ate fish the").unwrap();
    assert_eq!(hits.len(), 1);
    let parts = &hits[0].parts;
    assert_eq!(parts.len(), 2, "{parts:?}");
    assert!(parts.iter().all(|h| h.page == 0));
    assert_eq!(text_of(&parts[0]), "ate fish");
    assert_eq!(text_of(&parts[1]), "the");
    assert_ne!(parts[0].object, parts[1].object);
    // A phrase across a page break has a part on each page.
    let p = paged("first line\nthe walrus\nate fish\nlast line");
    let hits = p.search("walrus ate").unwrap();
    let pages: Vec<usize> = hits[0].parts.iter().map(|h| h.page).collect();
    assert_eq!(pages, [0, 1]);
}

/// What a replacement holds of what it replaces, counted as a search counts.
#[test]
fn a_query_counts_itself_in_other_text() {
    assert_eq!(Query::new("cat").count_in("cats and  Cat"), 2);
    assert_eq!(Query::with("cat", CASE).count_in("cats and Cat"), 1);
    assert_eq!(Query::with("cat", WHOLE).count_in("cats and Cat"), 1);
    assert_eq!(Query::new("a b").count_in(" a  b "), 1);
    assert_eq!(Query::new("dog").count_in("cat"), 0);
}

/// The sizes and crop margins of all pages at once are each page's.
#[test]
fn page_sizes_and_margins_are_each_pages() {
    let mut p = Pdf::new();
    for (i, (w, h)) in [(612.0, 792.0), (300.0, 200.0), (595.0, 842.0)]
        .into_iter()
        .enumerate()
    {
        p.insert_blank(i, w, h).unwrap();
    }
    p.rotate_pages(&[1], 90).unwrap();
    let one: Vec<(f32, f32)> = (0..3).map(|i| p.page_size(i).unwrap()).collect();
    assert_eq!(p.page_sizes(), one);
    assert_eq!(one[1], (200.0, 300.0));
    let full: Vec<(f32, f32)> = (0..3).map(|i| p.full_page_size(i).unwrap()).collect();
    assert_eq!(p.full_page_sizes(), full);
    p.set_crop_margins(1, [10.0, 20.0, 30.0, 40.0]).unwrap();
    let margins: Vec<[f32; 4]> = (0..3).map(|i| p.crop_margins(i).unwrap()).collect();
    assert_eq!(p.all_crop_margins(), margins);
    assert_eq!(margins[1], [10.0, 20.0, 30.0, 40.0]);
}

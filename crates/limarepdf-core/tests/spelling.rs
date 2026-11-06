//! Spelling (NEW-06): the bundled dictionaries, the words checked, the checking and the
//! suggestions, words added and ignored, and dictionaries found in folders and extensions.

use limarepdf_core::spelling::{
    best_match, find, in_extension, in_extension_within, language_name, read_within,
    read_word_list, word_list, words, BuiltIn, Dictionary, DictionaryBudget, Exceptions, Speller,
    SpellingError, BUILT_IN, SUGGESTIONS,
};
use std::io::Write;
use std::path::PathBuf;
use std::sync::{Arc, OnceLock};

fn scratch(name: &str) -> PathBuf {
    let d = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("spelling-{name}"));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

/// Canadian English, read once for the tests that use it.
fn canadian() -> Arc<Dictionary> {
    static D: OnceLock<Arc<Dictionary>> = OnceLock::new();
    let b = BuiltIn::get("en-CA").unwrap();
    D.get_or_init(|| Arc::new(Dictionary::built_in(b).unwrap()))
        .clone()
}

fn wrong(text: &str, except: &Exceptions) -> Vec<String> {
    let mut s = Speller::new(canadian());
    let found = s.misspelt(text, except);
    found.into_iter().map(|r| text[r].to_owned()).collect()
}

fn listed(text: &str) -> Vec<&str> {
    words(text).into_iter().map(|r| &text[r]).collect()
}

#[test]
fn each_bundled_dictionary_unpacks_as_released_and_reads() {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("assets/dictionaries");
    let files = ["en_CA", "en_US", "en_GB-ise", "en_AU", "fr"];
    let samples = [
        ("colour", "colorr"),
        ("color", "colorr"),
        ("organise", "organisse"),
        ("colour", "colorr"),
        ("fenêtre", "fenetre"),
    ];
    for ((b, file), (right, wrong)) in BUILT_IN.iter().zip(files).zip(samples) {
        let (aff, dic) = b.files();
        assert_eq!(aff, std::fs::read(dir.join(format!("{file}.aff"))).unwrap());
        assert_eq!(dic, std::fs::read(dir.join(format!("{file}.dic"))).unwrap());
        let d = Dictionary::built_in(*b).unwrap();
        assert!(d.check(right), "{}: {right}", b.name);
        assert!(!d.check(wrong), "{}: {wrong}", b.name);
    }
    assert_eq!(BUILT_IN[0].tag, "en-CA", "Canadian English comes first");
}

#[test]
fn canadian_english_is_scowls_own() {
    // As released: -our and -re, and -ize (the user: "Leave it as is").
    let d = canadian();
    for w in [
        "colour",
        "centre",
        "neighbour",
        "organize",
        "don’t",
        "don't",
    ] {
        assert!(d.check(w), "{w}");
    }
    for w in ["color", "center", "organise"] {
        assert!(!d.check(w), "{w}");
    }
}

#[test]
fn words_leave_out_capitals_digits_and_addresses() {
    assert_eq!(
        listed("The PDF has 3rd-party pages."),
        ["The", "has", "pages"]
    );
    assert_eq!(
        listed("See https://example.com/teh-page or www.exampel.ca, mail me@exampel.org"),
        ["See", "or", "mail"]
    );
    assert_eq!(
        listed(r"Open C:\Users\teh or ~/docs/teh.pdf now"),
        ["Open", "or", "now"]
    );
    // Apostrophes inside words stay; hyphens join words into one.
    assert_eq!(
        listed("don’t l’école well-known"),
        ["don’t", "l’école", "well-known"]
    );
    assert_eq!(listed("I A"), ["I", "A"], "one capital letter is a word");
}

#[test]
fn wrong_words_are_found_where_they_are() {
    let text = "Teh colour of the wagon is grey, not gray.";
    let mut s = Speller::new(canadian());
    let found = s.misspelt(text, &Exceptions::default());
    let at: Vec<(usize, &str)> = found.iter().map(|r| (r.start, &text[r.clone()])).collect();
    assert_eq!(at, [(0, "Teh"), (37, "gray")]);
    // Words joined by a hyphen: known parts make a known whole; only the wrong part is marked.
    assert!(wrong("a well-known, up-to-date page", &Exceptions::default()).is_empty());
    assert_eq!(
        wrong("a well-knwon page", &Exceptions::default()),
        ["knwon"]
    );
    // Letters beyond ASCII, measured in bytes.
    let text = "Le café est «fermé», teh end";
    let mut s = Speller::new(canadian());
    let found = s.misspelt(text, &Exceptions::default());
    assert!(found.iter().all(|r| text.is_char_boundary(r.start)));
}

#[test]
fn added_and_ignored_words_count_as_spelt_correctly() {
    let mut except = Exceptions::default();
    assert_eq!(
        wrong("Limare wrote limarepdf", &except),
        ["Limare", "limarepdf"]
    );
    except.added.insert("limarepdf".into());
    // Added in small letters, it covers capitals; added with one, only as written.
    assert_eq!(
        wrong("Limarepdf wrote limarepdf", &except),
        Vec::<String>::new()
    );
    except.added.insert("Limare".into());
    assert_eq!(wrong("Limare limare", &except), ["limare"]);
    except.ignored.insert("teh".into());
    assert!(wrong("teh Teh", &except).is_empty());
    // A curly apostrophe matches a straight one added.
    except.added.insert("o'flaherty's".into());
    assert!(wrong("O’Flaherty’s", &except).is_empty());
}

#[test]
fn suggestions_put_the_likeliest_first_and_are_few() {
    let s = Speller::new(canadian());
    let got = s.suggest("recieve");
    assert_eq!(got.first().map(String::as_str), Some("receive"));
    assert!(s.suggest("teh").len() <= SUGGESTIONS);
    // A word with a capital first gets its small-letter form's, capitalised: spellbook
    // alone offered "Eh", "Th", "Te", "Tet" and "Ted" for "Teh", and no "The".
    let got = s.suggest("Teh");
    assert_eq!(got.first().map(String::as_str), Some("The"), "{got:?}");
    assert!(
        got.iter().all(|w| w.starts_with(char::is_uppercase)),
        "{got:?}"
    );
    let fr = Dictionary::built_in(BuiltIn::get("fr").unwrap()).unwrap();
    assert_eq!(
        fr.suggest("fenetre").first().map(String::as_str),
        Some("fenêtre")
    );
}

#[test]
fn a_word_list_keeps_one_word_a_line() {
    let words = read_word_list("Limare\r\n\n  o’clocks \nlimarepdf\n");
    let expected: std::collections::HashSet<String> =
        ["Limare", "o'clocks", "limarepdf"].map(String::from).into();
    assert_eq!(words, expected);
    assert_eq!(word_list(&words), "Limare\nlimarepdf\no'clocks\n");
}

#[test]
fn a_dictionary_in_latin_1_reads_and_another_character_set_is_refused() {
    let aff = b"SET ISO8859-1\nTRY e\n";
    let dic = b"2\ncaf\xe9\nna\xefve\n";
    let d = Dictionary::from_bytes("test", aff, dic).unwrap();
    assert!(d.check("café") && d.check("naïve") && !d.check("cafe"));
    // Without a SET line, Hunspell reads ISO 8859-1.
    let d = Dictionary::from_bytes("test", b"TRY e\n", dic).unwrap();
    assert!(d.check("café"));
    let refused = Dictionary::from_bytes("ru_RU.dic", b"SET KOI8-R\n", b"1\n\xc1\n");
    let Err(SpellingError::Encoding(name, set)) = refused else {
        panic!("KOI8-R read");
    };
    assert_eq!((name.as_str(), set.as_str()), ("ru_RU.dic", "KOI8-R"));
    let message = SpellingError::Encoding(name, set).to_string();
    assert_eq!(
        message,
        "ru_RU.dic is written in KOI8-R, a character set LimarePDF can't read"
    );
}

#[test]
fn dictionaries_are_found_in_folders_by_their_pairs() {
    let a = scratch("found-a");
    let b = scratch("found-b");
    for (dir, file) in [
        (&a, "de_DE.aff"),
        (&a, "de_DE.dic"),
        (&a, "hyph_de_DE.dic"),
        (&a, "notes.txt"),
        (&a, "fr-classique.aff"),
        (&a, "fr-classique.dic"),
        (&b, "de_DE.aff"),
        (&b, "de_DE.dic"),
        (&b, "Custom.aff"),
        (&b, "Custom.dic"),
    ] {
        std::fs::write(dir.join(file), "SET UTF-8\n").unwrap();
    }
    let missing = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("spelling-not-there");
    let found = find(&[missing, a.clone(), b.clone()]);
    let got: Vec<(&str, &str, &str)> = (found.iter())
        .map(|f| (f.stem.as_str(), f.tag.as_str(), f.name.as_str()))
        .collect();
    assert_eq!(
        got,
        [
            ("de_DE", "de-DE", "German (Germany)"),
            ("fr-classique", "fr", "French"),
            ("Custom", "Custom", "Custom"),
        ]
    );
    assert_eq!(found[0].aff, a.join("de_DE.aff"));
}

#[test]
fn languages_match_by_region_then_language() {
    let tags: Vec<&str> = BUILT_IN.iter().map(|b| b.tag).collect();
    assert_eq!(best_match("en-GB", tags.iter().copied()), Some(2));
    assert_eq!(best_match("fr-CA", tags.iter().copied()), Some(4));
    assert_eq!(best_match("en_AU.UTF-8", tags.iter().copied()), Some(3));
    assert_eq!(best_match("EN-us", tags.iter().copied()), Some(1));
    assert_eq!(best_match("en-NZ", tags.iter().copied()), Some(0));
    assert_eq!(best_match("de-DE", tags.iter().copied()), None);
    assert_eq!(best_match("", tags.iter().copied()), None);
    assert_eq!(language_name("fr-CA").as_deref(), Some("French (Canada)"));
    assert_eq!(language_name("de_XY").as_deref(), Some("German (XY)"));
    assert_eq!(language_name("nl").as_deref(), Some("Dutch"));
    assert_eq!(language_name("xx-YY"), None);
}

fn extension(files: &[(&str, &[u8])]) -> Vec<u8> {
    let mut zip = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    let opts = zip::write::SimpleFileOptions::default();
    for (name, data) in files {
        zip.start_file(*name, opts).unwrap();
        zip.write_all(data).unwrap();
    }
    zip.finish().unwrap().into_inner()
}

#[test]
fn an_extension_gives_its_dictionaries() {
    let oxt = extension(&[
        ("description.xml", b"<description/>"),
        ("dictionaries/de_DE.aff", b"SET UTF-8\n"),
        ("dictionaries/de_DE.dic", b"1\nHaus\n"),
        ("dictionaries/hyph_de_DE.dic", b"ISO8859-1\n"),
        ("th_de_DE.dat", b"UTF-8\n"),
        ("dictionaries/../evil name.aff", b"SET UTF-8\n"),
        ("dictionaries/../evil name.dic", b"1\nx\n"),
    ]);
    let got = in_extension(&oxt, "de.oxt").unwrap();
    let stems: Vec<&str> = got.iter().map(|u| u.stem.as_str()).collect();
    assert_eq!(stems, ["de_DE", "evil_name"]);
    let d = Dictionary::from_bytes("de", &got[0].aff, &got[0].dic).unwrap();
    assert!(d.check("Haus"));
    let none = extension(&[("dictionaries/hyph_de_DE.dic", b"x")]);
    let message = in_extension(&none, "hyph.oxt").unwrap_err().to_string();
    assert_eq!(message, "hyph.oxt has no spelling dictionary in it");
    assert!(matches!(
        in_extension(b"not a zip", "x.oxt"),
        Err(SpellingError::Parse(..))
    ));
}

/// `oxt` with each file's unpacked size stated as `size`, whatever it is.
fn stating(mut oxt: Vec<u8>, size: u32) -> Vec<u8> {
    for i in 0..oxt.len() - 4 {
        let at = match &oxt[i..i + 4] {
            b"PK\x03\x04" => i + 22,
            b"PK\x01\x02" => i + 24,
            _ => continue,
        };
        oxt[at..at + 4].copy_from_slice(&size.to_le_bytes());
    }
    oxt
}

fn small() -> DictionaryBudget {
    DictionaryBudget::new(4, 64 << 10)
}

/// The files and bytes `b` has left.
fn left(b: &DictionaryBudget) -> (usize, u64) {
    (b.files_left(), b.bytes_left())
}

/// An extension's `.aff` and `.dic` files are counted and measured as they are unpacked, those
/// with no partner and those of the same name too, against what one choice of dictionaries may
/// unpack; past it the extension gives nothing and the budget is as it was.
#[test]
fn an_extension_unpacks_no_more_than_its_budget() {
    let kib = vec![b'x'; 1 << 10];
    let k = kib.as_slice();
    let five = extension(&[
        ("a.aff", k),
        ("a.dic", k),
        ("b.aff", k),
        ("b.dic", k),
        ("c.dic", k),
    ]);
    let mut b = small();
    let e = in_extension_within(&five, "five.oxt", &mut b).unwrap_err();
    assert!(matches!(e, SpellingError::TooMany(..)), "{e:?}");
    assert_eq!(
        e.to_string(),
        "five.oxt has too many dictionary files: LimarePDF adds at most 4 at once"
    );
    assert_eq!(b, small(), "a refusal took from the budget");
    let twice = extension(&[
        ("a.aff", k),
        ("old/a.aff", k),
        ("a.dic", k),
        ("hyph_a.dic", k),
        ("b.dic", k),
    ]);
    let e = in_extension_within(&twice, "twice.oxt", &mut small()).unwrap_err();
    assert!(matches!(e, SpellingError::TooMany(..)), "{e:?}");
    let four = extension(&[
        ("a.aff", k),
        ("a.dic", k),
        ("b.aff", k),
        ("b.dic", k),
        ("README.txt", k),
        ("th_a.dat", k),
    ]);
    let mut b = small();
    let got = in_extension_within(&four, "four.oxt", &mut b).unwrap();
    assert_eq!(got.len(), 2);
    assert_eq!(left(&b), (0, 60 << 10));

    let (big, less) = (vec![b'x'; 32 << 10], vec![b'x'; 31 << 10]);
    let over = extension(&[("a.aff", &big), ("a.dic", &big), ("b.dic", k)]);
    let mut b = small();
    let e = in_extension_within(&over, "over.oxt", &mut b).unwrap_err();
    assert!(matches!(e, SpellingError::TooMuch(..)), "{e:?}");
    assert_eq!(
        e.to_string(),
        "over.oxt has too much in it: LimarePDF adds at most 64 KiB of dictionaries at once"
    );
    assert_eq!(b, small());
    let full = extension(&[("a.aff", &big), ("a.dic", &less), ("b.dic", k)]);
    let mut b = small();
    let got = in_extension_within(&full, "full.oxt", &mut b).unwrap();
    assert_eq!(got.len(), 1);
    assert_eq!(left(&b), (1, 0));

    // One budget for several extensions chosen together.
    let pair = extension(&[("a.aff", k), ("a.dic", k)]);
    let three = extension(&[("a.aff", k), ("a.dic", k), ("b.dic", k)]);
    let mut b = small();
    in_extension_within(&pair, "1.oxt", &mut b).unwrap();
    let e = in_extension_within(&three, "2.oxt", &mut b).unwrap_err();
    assert!(matches!(e, SpellingError::TooMany(..)), "{e:?}");
    in_extension_within(&pair, "3.oxt", &mut b).unwrap();
    assert_eq!(left(&b), (0, 60 << 10));
}

/// An extension of 256 dictionary files is unpacked, of 257 refused.
#[test]
fn an_extension_unpacks_at_most_256_files() {
    let names: Vec<String> = (0..257)
        .map(|i| format!("d{}.{}", i / 2, ["aff", "dic"][i % 2]))
        .collect();
    let files: Vec<(&str, &[u8])> = names.iter().map(|n| (n.as_str(), &b"x"[..])).collect();
    let got = in_extension(&extension(&files[..256]), "256.oxt").unwrap();
    assert_eq!(got.len(), 128);
    let e = in_extension(&extension(&files), "257.oxt").unwrap_err();
    assert_eq!(
        e.to_string(),
        "257.oxt has too many dictionary files: LimarePDF adds at most 256 at once"
    );
}

/// The sizes an extension states are checked before anything is unpacked, and what is
/// unpacked is still measured, so a file that says it is smaller than it is isn't believed.
#[test]
fn an_extensions_stated_sizes_are_checked_and_not_trusted() {
    let pair = extension(&[
        ("a.aff", b"SET UTF-8\n"),
        ("a.dic", b"1\nx\n"),
        ("b.dic", b"1\ny\n"),
    ]);
    let e = in_extension(&stating(pair.clone(), 50 << 20), "huge.oxt").unwrap_err();
    assert!(matches!(e, SpellingError::TooMuch(..)), "{e:?}");
    let e = in_extension(&stating(pair, 100 << 20), "huge.oxt").unwrap_err();
    assert!(matches!(e, SpellingError::TooLarge(..)), "{e:?}");
    let some = vec![b'x'; 40 << 10];
    let pair = extension(&[("a.aff", &some), ("a.dic", &some)]);
    // The zip reader stops at the stated size too, and says so.
    let mut b = small();
    let e = in_extension_within(&stating(pair, 1), "small.oxt", &mut b).unwrap_err();
    assert!(
        matches!(e, SpellingError::TooMuch(..) | SpellingError::Read(..)),
        "{e:?}"
    );
    assert_eq!(b, small());
}

/// A `.dic` or `.aff` file chosen on its own counts against the same budget as an extension's.
#[test]
fn a_chosen_file_is_read_within_the_budget() {
    let dir = scratch("budget");
    let file = dir.join("a.dic");
    std::fs::write(&file, vec![b'x'; 40 << 10]).unwrap();
    let mut b = small();
    assert_eq!(read_within(&file, &mut b).unwrap().len(), 40 << 10);
    assert_eq!(left(&b), (3, 24 << 10));
    let e = read_within(&file, &mut b).unwrap_err();
    assert!(matches!(e, SpellingError::TooMuch(..)), "{e:?}");
    assert_eq!(left(&b), (3, 24 << 10));
    let e = read_within(&file, &mut DictionaryBudget::new(0, 64 << 10)).unwrap_err();
    assert!(matches!(e, SpellingError::TooMany(..)), "{e:?}");
    let e = read_within(&dir.join("gone.dic"), &mut small()).unwrap_err();
    assert!(matches!(e, SpellingError::Read(..)), "{e:?}");
}

//! Spelling (NEW-06): the bundled dictionaries, the words checked, the checking and the
//! suggestions, words added and ignored, and dictionaries found in folders and extensions.

use limarepdf_core::spelling::{
    best_match, find, in_extension, language_name, read_word_list, word_list, words, BuiltIn,
    Dictionary, Exceptions, Speller, SpellingError, BUILT_IN, SUGGESTIONS,
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

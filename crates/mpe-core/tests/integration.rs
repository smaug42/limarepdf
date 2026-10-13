use mpe_core::{render, Bates, Cipher, Encryption, HeaderFooter, Pdf, Restrictions, Watermark};

fn doc_with_text() -> Pdf {
    let mut p = Pdf::new();
    for i in 0..2 {
        p.insert_blank(i, 300.0, 200.0).unwrap();
    }
    p.add_header_footer(
        &[0, 1],
        &HeaderFooter {
            header: ["Left".into(), "needle in haystack".into(), "".into()],
            bates: Some(Bates::default()),
            ..Default::default()
        },
    )
    .unwrap();
    p
}

fn find(hay: &[u8], needle: &[u8]) -> Option<usize> {
    hay.windows(needle.len()).position(|w| w == needle)
}

#[test]
fn renders_overlay_pixels() {
    let mut p = doc_with_text();
    p.add_watermark(
        &[0],
        &Watermark {
            font_size: 60.0,
            opacity: 1.0,
            ..Default::default()
        },
    )
    .unwrap();
    let bytes = p.to_bytes().unwrap();
    let hp = render::load(bytes, None).unwrap();
    let r = render::Renderer::new(&hp);
    assert_eq!(r.page_count(), 2);
    let bmp = r.render(0, 1.0).unwrap();
    assert_eq!((bmp.width, bmp.height), (300, 200));
    let dark = bmp
        .rgba
        .chunks_exact(4)
        .filter(|px| px[0] < 200 || px[1] < 200)
        .count();
    assert!(
        dark > 200,
        "expected visible text/watermark pixels, got {dark}"
    );
    let all = render::render_pages_parallel(&hp, &[0, 1], 0.5);
    assert!(all.iter().all(|b| b.is_ok()));
}

#[test]
fn encryption_roundtrip_all_ciphers() {
    for cipher in [
        Cipher::Rc4_40,
        Cipher::Rc4_128,
        Cipher::Aes128,
        Cipher::Aes256,
    ] {
        let mut p = doc_with_text();
        p.set_encryption(Some(Encryption {
            user_password: "user".into(),
            owner_password: "owner".into(),
            cipher,
            restrictions: Restrictions {
                copy: false,
                ..Default::default()
            },
        }));
        let bytes = p.to_bytes().unwrap();
        // Hex password hashes, as hayro misreads some binary literal strings; and the key
        // length for AES-256, which poppler needs.
        let at = find(&bytes, b"/Filter/Standard").expect("no encryption dictionary");
        let end = at + find(&bytes[at..], b"endobj").unwrap();
        let enc = &bytes[at..end];
        for key in ["/O", "/U"] {
            let i = find(enc, key.as_bytes()).unwrap() + key.len();
            assert_eq!(enc[i], b'<', "{cipher:?}: {key} is not hex");
        }
        if cipher == Cipher::Aes256 {
            assert!(find(enc, b"/Length 256").is_some(), "no /Length");
        }
        assert!(
            Pdf::from_bytes(&bytes, Some("wrong")).is_err(),
            "{cipher:?}: wrong password accepted"
        );
        let ok =
            Pdf::from_bytes(&bytes, Some("user")).unwrap_or_else(|e| panic!("{cipher:?}: {e}"));
        assert_eq!(ok.page_count(), 2, "{cipher:?}");
        assert!(
            ok.page_text(0).contains("needle"),
            "{cipher:?}: text not recoverable"
        );
        // The renderer must also open it with the password.
        let hp =
            render::load(bytes, Some("user")).unwrap_or_else(|e| panic!("{cipher:?} render: {e}"));
        assert_eq!(render::Renderer::new(&hp).page_count(), 2);
    }
}

#[test]
fn search_finds_text() {
    let p = doc_with_text();
    let hits = p.search("NEEDLE").unwrap();
    assert_eq!(hits.len(), 2, "{hits:?}");
    assert_eq!(hits[0].page, 0);
}

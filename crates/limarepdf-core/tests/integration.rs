use limarepdf_core::{
    render, Bates, Cipher, Encryption, HeaderFooter, Pdf, Restrictions, Watermark,
};

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
        .as_chunks::<4>()
        .0
        .iter()
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

/// `bytes` with the encryption dictionary's `/U` cut to its first 16 bytes, as some
/// writers store it for revisions 3 and 4 (readers compare only those). Spaces take the
/// place of the cut, so every offset holds.
fn short_user_hash(mut bytes: Vec<u8>) -> Vec<u8> {
    let filter = find(&bytes, b"/Filter/Standard").expect("no encryption dictionary");
    let at = bytes[..filter]
        .windows(3)
        .rposition(|w| w == b"obj")
        .unwrap();
    let end = filter + find(&bytes[filter..], b"endobj").unwrap();
    let u = at + find(&bytes[at..end], b"/U<").unwrap() + 3;
    assert_eq!(bytes[u + 64], b'>', "/U is not 32 bytes of hex");
    bytes[u + 32] = b'>';
    bytes[u + 33..u + 65].fill(b' ');
    bytes
}

#[test]
fn empty_password_files_with_a_short_user_hash_open() {
    // RC4 at revision 3, written by LimarePDF.
    let mut p = doc_with_text();
    p.set_encryption(Some(Encryption {
        user_password: "".into(),
        owner_password: "owner".into(),
        cipher: Cipher::Rc4_128,
        restrictions: Restrictions::default(),
    }));
    let r3 = short_user_hash(p.to_bytes().unwrap());

    // RC4 at revision 4 through a crypt filter, with the metadata left unencrypted: the
    // traits of PDFBox's test file 1910.
    let mut doc = doc_with_text().doc;
    let s = lopdf::Object::String(vec![7; 16], lopdf::StringFormat::Hexadecimal);
    doc.trailer.set("ID", vec![s.clone(), s]);
    doc.compress();
    let f: std::sync::Arc<dyn lopdf::encryption::crypt_filters::CryptFilter> =
        std::sync::Arc::new(lopdf::encryption::crypt_filters::Rc4CryptFilter);
    let state = lopdf::EncryptionState::try_from(lopdf::EncryptionVersion::V4 {
        document: &doc,
        encrypt_metadata: false,
        crypt_filters: std::collections::BTreeMap::from([(b"StdCF".to_vec(), f)]),
        stream_filter: b"StdCF".to_vec(),
        string_filter: b"StdCF".to_vec(),
        owner_password: "owner",
        user_password: "",
        permissions: lopdf::Permissions::all(),
    })
    .unwrap();
    doc.encrypt(&state).unwrap();
    let id = doc.trailer.get(b"Encrypt").unwrap().as_reference().unwrap();
    let enc = doc.get_object_mut(id).unwrap().as_dict_mut().unwrap();
    for key in [b"O", b"U"] {
        if let Ok(lopdf::Object::String(_, format)) = enc.get_mut(key) {
            *format = lopdf::StringFormat::Hexadecimal;
        }
    }
    let mut r4 = Vec::new();
    doc.save_to(&mut r4).unwrap();
    let r4 = short_user_hash(r4);
    assert!(find(&r4, b"/EncryptMetadata false").is_some());

    for (name, bytes) in [("revision 3", r3), ("revision 4", r4)] {
        let p = Pdf::from_bytes(&bytes, None).unwrap_or_else(|e| panic!("{name}: {e:#}"));
        assert_eq!(p.page_count(), 2, "{name}");
        assert!(p.page_text(0).contains("needle"), "{name}: text not read");
        assert!(
            p.doc.encryption_state.is_some(),
            "{name}: not seen as protected"
        );
        assert!(!p.repaired(), "{name}: not damaged");
        assert!(!p.password_needed(), "{name}");
        assert!(Pdf::from_bytes(&bytes, Some("owner")).is_ok(), "{name}");
    }
}

#[test]
fn files_tell_whether_they_needed_a_password() {
    let protect = |user: &str| {
        let mut p = doc_with_text();
        p.set_encryption(Some(Encryption {
            user_password: user.into(),
            owner_password: "owner".into(),
            cipher: Cipher::Aes128,
            restrictions: Restrictions::default(),
        }));
        p.to_bytes().unwrap()
    };
    let locked = protect("user");
    for password in ["user", "owner"] {
        let p = Pdf::from_bytes(&locked, Some(password)).unwrap();
        assert!(p.password_needed(), "{password}");
    }
    // Only a permissions password: it opens without one, even when one is given.
    let open = protect("");
    for password in [None, Some(""), Some("owner"), Some("anything")] {
        let p = Pdf::from_bytes(&open, password).unwrap();
        assert!(p.doc.was_encrypted(), "{password:?}");
        assert!(!p.password_needed(), "{password:?}");
    }
    let plain = Pdf::from_bytes(&doc_with_text().to_bytes().unwrap(), None).unwrap();
    assert!(!plain.password_needed());
}

#[test]
fn crop_margins_follow_rotation() {
    let mut p = Pdf::new();
    p.insert_blank(0, 300.0, 200.0).unwrap();
    assert_eq!(p.crop_margins(0).unwrap(), [0.0; 4]);

    p.set_crop_margins(0, [10.0, 20.0, 30.0, 40.0]).unwrap();
    assert_eq!(p.crop_margins(0).unwrap(), [10.0, 20.0, 30.0, 40.0]);
    assert_eq!(p.page_size(0).unwrap(), (240.0, 160.0));

    // Turned clockwise, the old left margin is at the top.
    p.rotate_pages(&[0], 90).unwrap();
    assert_eq!(p.crop_margins(0).unwrap(), [40.0, 10.0, 20.0, 30.0]);
    assert_eq!(p.page_size(0).unwrap(), (160.0, 240.0));

    // Margins are set as displayed: left on the turned page is the bottom unturned.
    p.set_crop_margins(0, [0.0, 0.0, 0.0, 50.0]).unwrap();
    p.rotate_pages(&[0], -90).unwrap();
    assert_eq!(p.crop_margins(0).unwrap(), [0.0, 0.0, 50.0, 0.0]);

    p.set_crop_margins(0, [0.0; 4]).unwrap();
    assert_eq!(p.page_size(0).unwrap(), (300.0, 200.0));

    assert!(p.set_crop_margins(0, [-1.0, 0.0, 0.0, 0.0]).is_err());
    assert!(p.set_crop_margins(0, [100.0, 0.0, 100.0, 0.0]).is_err());
    assert!(p.crop_margins(1).is_err());
}

#[test]
fn search_finds_text() {
    let p = doc_with_text();
    let hits = p.search("NEEDLE").unwrap();
    assert_eq!(hits.len(), 2, "{hits:?}");
    assert_eq!(hits[0].page, 0);
}

#[test]
fn search_joins_words_hyphenated_at_a_line_end() {
    let p = Pdf::from_text(
        "the infor-\nmation age, self-\nemployed\nnot - joined",
        (300.0, 200.0),
    )
    .unwrap();
    for q in [
        "information",
        "INFOR- MATION",
        "self-employed",
        "selfemployed",
        "the information age",
    ] {
        assert_eq!(p.search(q).unwrap().len(), 1, "{q}");
        assert_eq!(p.find_text_matches(0, q).unwrap().len(), 1, "{q}");
    }
    // Both halves are highlighted.
    assert_eq!(p.find_text_matches(0, "information").unwrap()[0].len(), 2);
    // Only a hyphen between letters at a line end joins; other line breaks and hyphens stay.
    for q in ["employednot", "not-joined", "notjoined"] {
        assert!(p.search(q).unwrap().is_empty(), "{q}");
        assert!(p.find_text_matches(0, q).unwrap().is_empty(), "{q}");
    }
}

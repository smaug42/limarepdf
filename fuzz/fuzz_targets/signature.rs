//! Fuzzed bytes as a signature's `/Contents`, in a one-page PDF whose byte range covers the rest
//! of the file, checked as the app checks signatures. The first byte picks the kind of signature
//! (`/SubFilter`), so each reader sees the bytes it takes: CMS for the detached and SHA-1 kinds, a
//! timestamp token for a document timestamp, and a PKCS#1 signature beside a test certificate for
//! `adbe.x509.rsa_sha1`.
#![no_main]

use libfuzzer_sys::fuzz_target;
use limarepdf_core::{verify_signatures, TrustList};

const SUB_FILTERS: [&str; 5] = [
    "adbe.pkcs7.detached",
    "ETSI.CAdES.detached",
    "adbe.pkcs7.sha1",
    "ETSI.RFC3161",
    "adbe.x509.rsa_sha1",
];
const CERT: &[u8] =
    include_bytes!("../../crates/limarepdf-core/tests/data/signatures/keys/t-leaf.crt.der");
/// The byte range's last three numbers, each as wide as it may be written, so that writing them
/// once they're known moves nothing.
fn range(start: usize, end: usize, rest: usize) -> String {
    format!("{start:<10} {end:<10} {rest:<10}")
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn signed(sub_filter: &str, contents: &[u8]) -> Vec<u8> {
    let cert = match sub_filter {
        "adbe.x509.rsa_sha1" => format!(" /Cert <{}>", hex(CERT)),
        _ => String::new(),
    };
    let objects: Vec<Vec<u8>> = vec![
        b"<< /Type /Catalog /Pages 2 0 R /AcroForm << /Fields [4 0 R] /SigFlags 3 >> >>".to_vec(),
        b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec(),
        b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 200 200] /Annots [4 0 R] >>".to_vec(),
        b"<< /FT /Sig /T (Signature1) /Type /Annot /Subtype /Widget /Rect [0 0 0 0] /F 132 \
           /P 3 0 R /V 5 0 R >>"
            .to_vec(),
        [
            format!("<< /Type /Sig /Filter /Adobe.PPKLite /SubFilter /{sub_filter}{cert} ")
                .as_bytes(),
            b"/M (D:20261111120000Z) /ByteRange [0 ",
            range(0, 0, 0).as_bytes(),
            b"] /Contents <",
            hex(contents).as_bytes(),
            b"> >>",
        ]
        .concat(),
    ];
    let mut pdf = b"%PDF-1.7\n".to_vec();
    let mut offsets = Vec::new();
    for (i, o) in objects.iter().enumerate() {
        offsets.push(pdf.len());
        pdf.extend_from_slice(format!("{} 0 obj\n", i + 1).as_bytes());
        pdf.extend_from_slice(o);
        pdf.extend_from_slice(b"\nendobj\n");
    }
    let xref = pdf.len();
    let size = objects.len() + 1;
    pdf.extend_from_slice(format!("xref\n0 {size}\n0000000000 65535 f \n").as_bytes());
    for o in offsets {
        pdf.extend_from_slice(format!("{o:010} 00000 n \n").as_bytes());
    }
    pdf.extend_from_slice(
        format!("trailer\n<< /Size {size} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n").as_bytes(),
    );
    // The gap is the hex string, `<` to `>`; the signed part is all the rest.
    let after = |what: &[u8]| pdf.windows(what.len()).position(|w| w == what).unwrap() + what.len();
    let at = after(b"/ByteRange [0 ");
    let start = after(b"/Contents <") - 1;
    let end = start + 2 * contents.len() + 2;
    let numbers = range(start, end, pdf.len() - end);
    pdf[at..at + numbers.len()].copy_from_slice(numbers.as_bytes());
    pdf
}

fuzz_target!(|data: &[u8]| {
    let Some((&pick, contents)) = data.split_first() else {
        return;
    };
    let sub_filter = SUB_FILTERS[usize::from(pick) % SUB_FILTERS.len()];
    let _ = verify_signatures(&signed(sub_filter, contents), None, &TrustList::default());
});

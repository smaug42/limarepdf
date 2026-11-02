//! `limarepdf-cli print` refuses, before anything reaches the printer, when filled fields
//! wouldn't show on paper. `lp` is replaced by a script that records the job, so no
//! printer is needed and none is ever used.
#![cfg(unix)]

use std::path::{Path, PathBuf};
use std::process::Command;

/// A one-page PDF with a printed text field holding Hebrew text (which LimarePDF can't write
/// yet: right to left) and no drawing, as an app that leaves the drawing to the reader writes
/// it.
fn make_pdf(dir: &Path) -> PathBuf {
    let objects = [
        "<< /Type /Catalog /Pages 2 0 R /AcroForm << /Fields [4 0 R] /DA (/Helv 12 Tf 0 g) >> >>",
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>",
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 200 200] /Annots [4 0 R] >>",
        "<< /Type /Annot /Subtype /Widget /FT /Tx /F 4 /T (Name) /Rect [20 20 180 50] \
         /V <FEFF05E905DC05D505DD> >>",
    ];
    let mut pdf = b"%PDF-1.7\n".to_vec();
    let mut offsets = Vec::new();
    for (i, o) in objects.iter().enumerate() {
        offsets.push(pdf.len());
        pdf.extend_from_slice(format!("{} 0 obj\n{o}\nendobj\n", i + 1).as_bytes());
    }
    let xref = pdf.len();
    pdf.extend_from_slice(b"xref\n0 5\n0000000000 65535 f \n");
    for o in offsets {
        pdf.extend_from_slice(format!("{o:010} 00000 n \n").as_bytes());
    }
    pdf.extend_from_slice(
        format!("trailer\n<< /Size 5 /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n").as_bytes(),
    );
    let path = dir.join("form.pdf");
    std::fs::write(&path, pdf).unwrap();
    path
}

/// Stand-ins for `lp`, which saves the job, and `lpoptions`, which knows no paper size.
fn fake_cups(dir: &Path) {
    use std::os::unix::fs::PermissionsExt;
    let lp = format!(
        "#!/bin/sh\ncat > '{}/job.pdf'\necho 'request id is Test-1 (1 file(s))'\n",
        dir.display()
    );
    for (name, script) in [("lp", lp.as_str()), ("lpoptions", "#!/bin/sh\n")] {
        let path = dir.join(name);
        std::fs::write(&path, script).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
}

/// Stand-ins for `lpstat`, listing one printer, the default, and `lpoptions`, naming its
/// paper sizes as a PPD does.
fn fake_lpstat(dir: &Path) {
    use std::os::unix::fs::PermissionsExt;
    let lpstat = "#!/bin/sh\ncase \"$1\" in\n  -e) echo limarepdf-test ;;\n  \
                  -d) echo 'system default destination: limarepdf-test' ;;\nesac\n";
    let lpoptions = "#!/bin/sh\necho 'PageSize/Media Size: *A4 Letter Custom.WIDTHxHEIGHT'\n";
    for (name, script) in [("lpstat", lpstat), ("lpoptions", lpoptions)] {
        let path = dir.join(name);
        std::fs::write(&path, script).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
}

/// An IPP answer describing a printer with Letter (the default, with margins) and A4.
fn ipp_answer() -> Vec<u8> {
    fn put(b: &mut Vec<u8>, tag: u8, name: &str, value: &[u8]) {
        b.push(tag);
        b.extend_from_slice(&(name.len() as u16).to_be_bytes());
        b.extend_from_slice(name.as_bytes());
        b.extend_from_slice(&(value.len() as u16).to_be_bytes());
        b.extend_from_slice(value);
    }
    let mut b = vec![1, 1, 0, 0, 0, 0, 0, 1, 0x01];
    put(&mut b, 0x47, "attributes-charset", b"utf-8");
    put(&mut b, 0x48, "attributes-natural-language", b"en");
    b.push(0x04);
    put(&mut b, 0x44, "media-supported", b"na_letter_8.5x11in");
    put(&mut b, 0x44, "", b"iso_a4_210x297mm");
    put(&mut b, 0x44, "media-default", b"na_letter_8.5x11in");
    // media-col-database: Letter with 4.23 mm margins.
    put(&mut b, 0x34, "media-col-database", b"");
    put(&mut b, 0x4A, "", b"media-size");
    put(&mut b, 0x34, "", b"");
    put(&mut b, 0x4A, "", b"x-dimension");
    put(&mut b, 0x21, "", &21590i32.to_be_bytes());
    put(&mut b, 0x4A, "", b"y-dimension");
    put(&mut b, 0x21, "", &27940i32.to_be_bytes());
    put(&mut b, 0x37, "", b"");
    for side in ["top", "right", "bottom", "left"] {
        put(&mut b, 0x4A, "", format!("media-{side}-margin").as_bytes());
        put(&mut b, 0x21, "", &423i32.to_be_bytes());
    }
    put(&mut b, 0x37, "", b"");
    b.push(0x03);
    b
}

/// A stand-in printing service on the socket `path`, answering one request with
/// [`ipp_answer`].
fn fake_ipp(path: &Path) -> std::thread::JoinHandle<()> {
    use std::io::{Read, Write};
    let _ = std::fs::remove_file(path);
    let listener = std::os::unix::net::UnixListener::bind(path).unwrap();
    std::thread::spawn(move || {
        let (mut s, _) = listener.accept().unwrap();
        s.set_read_timeout(Some(std::time::Duration::from_secs(5)))
            .unwrap();
        let mut got = Vec::new();
        let mut buf = [0u8; 4096];
        // The request's head and body: the body ends with the end-of-attributes tag.
        while let Ok(n) = s.read(&mut buf) {
            got.extend_from_slice(&buf[..n]);
            if n == 0 || (got.windows(4).any(|w| w == b"\r\n\r\n") && got.last() == Some(&3)) {
                break;
            }
        }
        let body = ipp_answer();
        let head = format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n", body.len());
        let _ = s.write_all(head.as_bytes());
        let _ = s.write_all(&body);
    })
}

/// `printers --paper` lists the printer's paper sizes as the printing service describes
/// them over IPP, and from `lpoptions`'s names when it can't be asked.
#[test]
fn printers_list_their_paper_sizes() {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join("printers-paper");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    fake_lpstat(&dir);
    let path = format!(
        "{}:{}",
        dir.display(),
        std::env::var("PATH").unwrap_or_default()
    );
    let socket = dir.join("ipp.sock");
    let run = |socket: &Path| {
        let out = Command::new(env!("CARGO_BIN_EXE_limarepdf-cli"))
            .args(["printers", "--paper"])
            .env("PATH", &path)
            .env("CUPS_SERVER", socket)
            .output()
            .unwrap();
        let said = String::from_utf8_lossy(&out.stdout).to_string();
        assert!(
            out.status.success(),
            "{said}{}",
            String::from_utf8_lossy(&out.stderr)
        );
        said
    };

    let serve = fake_ipp(&socket);
    let said = run(&socket);
    serve.join().unwrap();
    assert_eq!(
        said,
        "limarepdf-test (default)\n  Letter (8.5 × 11 in), default\n  A4 (210 × 297 mm)\n"
    );

    // No service on the socket: `lpoptions`'s names, without margins.
    let said = run(&dir.join("nothing.sock"));
    assert_eq!(
        said,
        "limarepdf-test (default)\n  A4 (210 × 297 mm), default\n  Letter (8.5 × 11 in)\n  \
         (the printer doesn't say how near the edges it can print)\n"
    );
}

/// Stand-ins for a printing service that can't be asked over IPP: `lpstat` (one printer, the
/// default), `lpoptions` (A4, the default, and Letter) and `lp`, which saves its arguments and
/// the job. Returns the PATH and the socket path naming no service, for `CUPS_SERVER`.
fn fake_printing(dir: &Path) -> (String, PathBuf) {
    use std::os::unix::fs::PermissionsExt;
    fake_lpstat(dir);
    let lp = format!(
        "#!/bin/sh\nprintf '%s\\n' \"$@\" > '{d}/args.txt'\ncat > '{d}/job.pdf'\n\
         echo 'request id is Test-1 (1 file(s))'\n",
        d = dir.display()
    );
    let path = dir.join("lp");
    std::fs::write(&path, lp).unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    let path = format!(
        "{}:{}",
        dir.display(),
        std::env::var("PATH").unwrap_or_default()
    );
    (path, dir.join("nothing.sock"))
}

/// A PDF of two pages: Letter, and one 1,000 points square.
fn two_pages(dir: &Path) -> PathBuf {
    let mut p = limarepdf_core::Pdf::new();
    p.insert_blank(0, 612.0, 792.0).unwrap();
    p.insert_blank(1, 1000.0, 1000.0).unwrap();
    let path = dir.join("two.pdf");
    p.save(&path).unwrap();
    path
}

/// `print` sends sheets of the paper named with `--paper` (else the printer's default), with
/// CUPS's scaling turned off; a paper the printer doesn't take is refused before anything
/// is sent; pages reaching past the printable area at the scale asked for are warned about.
#[test]
fn print_lays_the_pages_out_on_the_paper() {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join("print-paper");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let (path, socket) = fake_printing(&dir);
    let file = two_pages(&dir);
    let run = |args: &[&str]| {
        let _ = std::fs::remove_file(dir.join("job.pdf"));
        Command::new(env!("CARGO_BIN_EXE_limarepdf-cli"))
            .arg("print")
            .args(args)
            .arg(&file)
            .env("PATH", &path)
            .env("CUPS_SERVER", &socket)
            .output()
            .unwrap()
    };
    let sheets = || {
        let job = limarepdf_core::Pdf::open(dir.join("job.pdf"), None).unwrap();
        (0..job.page_count())
            .map(|i| {
                let (w, h) = job.page_size(i).unwrap();
                (w.round(), h.round())
            })
            .collect::<Vec<_>>()
    };
    let args = || std::fs::read_to_string(dir.join("args.txt")).unwrap();
    let err = |out: &std::process::Output| String::from_utf8_lossy(&out.stderr).to_string();

    // The printer's default paper, A4: both pages on A4 sheets.
    let out = run(&[]);
    assert!(out.status.success(), "{}", err(&out));
    assert_eq!(sheets(), [(595.0, 842.0), (595.0, 842.0)]);
    let said = args();
    assert!(said.contains("media=A4\n"), "{said}");
    assert!(said.contains("print-scaling=none\n"), "{said}");
    assert!(out.stderr.is_empty(), "{}", err(&out));

    // Letter, named as people name it, upright, at 50 %.
    let upright = ["--orientation", "portrait", "--scale", "50%"];
    let out = run(&[&["--paper", "letter"][..], &upright].concat());
    assert!(out.status.success(), "{}", err(&out));
    assert_eq!(sheets(), [(612.0, 792.0), (612.0, 792.0)]);
    assert!(args().contains("media=Letter\n"));

    // At actual size both pages reach past the printable area: said, and printed.
    let out = run(&["--paper", "Letter", "--scale", "actual"]);
    assert!(out.status.success());
    let said = err(&out);
    let warned = "warning: at this scale pages 1, 2 reach past the printable area of Letter \
                  (8.5 × 11 in), so their edges won't print\n";
    assert!(said.contains(warned), "{said}");

    // A paper the printer doesn't take: refused, naming those it does, nothing sent.
    let out = run(&["--paper", "A3"]);
    assert!(!out.status.success());
    let said = err(&out);
    assert!(said.contains("no paper size called \"A3\""), "{said}");
    let listed = said.contains("A4 (210 × 297 mm), Letter (8.5 × 11 in)");
    assert!(listed, "{said}");
    assert!(!dir.join("job.pdf").exists(), "a job reached lp");

    // A scale out of range is refused by the command line itself.
    let out = run(&["--scale", "401"]);
    assert!(!out.status.success());
    let said = err(&out);
    assert!(said.contains("a percentage from 1 to 400"), "{said}");
}

#[test]
fn fields_that_would_print_empty_stop_printing() {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join("print-fields");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    fake_cups(&dir);
    let file = make_pdf(&dir);
    let path = format!(
        "{}:{}",
        dir.display(),
        std::env::var("PATH").unwrap_or_default()
    );

    let out = Command::new(env!("CARGO_BIN_EXE_limarepdf-cli"))
        .args(["print", "--printer", "limarepdf-test"])
        .arg(&file)
        .env("PATH", path)
        .output()
        .unwrap();
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(!out.status.success(), "printed: {err}");
    assert!(
        err.contains("\"Name\"") && err.contains("Nothing was sent to the printer"),
        "{err}"
    );
    assert!(!dir.join("job.pdf").exists(), "a job reached lp");
}

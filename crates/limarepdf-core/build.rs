// Stores the bundled Liberation faces and dictionaries compressed (about 2 MB and 2.5 MB less
// in each binary): each file in assets/fonts and assets/dictionaries is packed here with zlib, and
// embed.rs and spelling.rs unpack it, byte for byte, the first time it is used. The files in
// assets stay as released: the Reserved Font Name needs the fonts unmodified.
use flate2::write::ZlibEncoder;
use flate2::Compression;
use std::io::Write;
use std::path::{Path, PathBuf};

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    let out = PathBuf::from(std::env::var_os("OUT_DIR").expect("cargo sets OUT_DIR"));
    for family in ["Sans", "Serif", "Mono"] {
        for style in ["Regular", "Bold", "Italic", "BoldItalic"] {
            pack(
                &out,
                "assets/fonts",
                &format!("Liberation{family}-{style}.ttf"),
            );
        }
    }
    for name in ["en_CA", "en_US", "en_GB-ise", "en_AU", "fr"] {
        for kind in ["aff", "dic"] {
            pack(&out, "assets/dictionaries", &format!("{name}.{kind}"));
        }
    }
}

/// Packs `dir/file` with zlib as `file.z` in `out`.
fn pack(out: &Path, dir: &str, file: &str) {
    let path = format!("{dir}/{file}");
    println!("cargo:rerun-if-changed={path}");
    let data = std::fs::read(&path).unwrap_or_else(|e| panic!("{path}: {e}"));
    let mut z = ZlibEncoder::new(Vec::new(), Compression::best());
    z.write_all(&data).expect("packing in memory");
    let packed = z.finish().expect("packing in memory");
    std::fs::write(out.join(format!("{file}.z")), packed)
        .unwrap_or_else(|e| panic!("{file}.z: {e}"));
}

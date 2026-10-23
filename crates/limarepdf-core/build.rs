// Stores the bundled Liberation faces compressed (about 2 MB less in each binary): each file
// in assets/fonts is packed here with zlib, and embed.rs unpacks it, byte for byte, the first
// time it is used. The files in assets/fonts stay as released: the Reserved Font Name needs
// them unmodified.
use flate2::write::ZlibEncoder;
use flate2::Compression;
use std::io::Write;
use std::path::PathBuf;

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    let out = PathBuf::from(std::env::var_os("OUT_DIR").expect("cargo sets OUT_DIR"));
    for family in ["Sans", "Serif", "Mono"] {
        for style in ["Regular", "Bold", "Italic", "BoldItalic"] {
            let file = format!("Liberation{family}-{style}.ttf");
            let path = format!("assets/fonts/{file}");
            println!("cargo:rerun-if-changed={path}");
            let data = std::fs::read(&path).unwrap_or_else(|e| panic!("{path}: {e}"));
            let mut z = ZlibEncoder::new(Vec::new(), Compression::best());
            z.write_all(&data).expect("packing in memory");
            let packed = z.finish().expect("packing in memory");
            std::fs::write(out.join(format!("{file}.z")), packed)
                .unwrap_or_else(|e| panic!("{file}.z: {e}"));
        }
    }
}

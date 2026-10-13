//! Print the text found in an image file, one line per text line.
//!
//! Usage: cargo run --release -p mpe-core --example ocr -- MODEL.rten IMAGE

fn main() -> mpe_core::Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let [model, image] = &args[..] else {
        anyhow::bail!("usage: ocr MODEL.rten IMAGE");
    };
    let ocr = mpe_core::Ocr::new(std::fs::read(model)?)?;
    let img = image::open(image)?.to_luma8();
    for line in ocr.read(img.as_raw(), img.width(), img.height())? {
        println!("{}", line.text());
    }
    Ok(())
}

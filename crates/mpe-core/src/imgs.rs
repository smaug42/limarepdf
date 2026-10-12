//! Building image XObjects from PNG / JPEG / other raster files.

use crate::{Pdf, Result};
use anyhow::{anyhow, bail};
use lopdf::{dictionary, Object, ObjectId, Stream};

/// Pixel size of a PNG/JPEG/other supported raster file.
pub fn image_dimensions(bytes: &[u8]) -> Result<(u32, u32)> {
    image::load_from_memory(bytes)
        .map(|i| (i.width(), i.height()))
        .map_err(|e| anyhow!("cannot decode the image: {e}"))
}

impl Pdf {
    /// Decode an image file and add it to the document as an image XObject.
    /// Returns the object id and the pixel size. JPEG data is embedded as is (no recompression).
    pub(crate) fn add_image_object(&mut self, bytes: &[u8]) -> Result<(ObjectId, u32, u32)> {
        let fmt = image::guess_format(bytes)
            .map_err(|_| anyhow!("unrecognised image format (PNG and JPEG are supported)"))?;
        let img = image::load_from_memory_with_format(bytes, fmt)
            .map_err(|e| anyhow!("cannot decode the image: {e}"))?;
        let (w, h) = (img.width(), img.height());
        if w == 0 || h == 0 {
            bail!("empty image");
        }
        let mut dict = dictionary! {
            "Type" => "XObject", "Subtype" => "Image", "Width" => w as i64, "Height" => h as i64,
            "BitsPerComponent" => 8,
        };
        let content;
        if fmt == image::ImageFormat::Jpeg {
            let gray = img.color().channel_count() < 3;
            dict.set("ColorSpace", if gray { "DeviceGray" } else { "DeviceRGB" });
            dict.set("Filter", "DCTDecode");
            content = bytes.to_vec();
        } else {
            let rgba = img.to_rgba8();
            let mut rgb = Vec::with_capacity(w as usize * h as usize * 3);
            let mut alpha = Vec::with_capacity(w as usize * h as usize);
            let mut translucent = false;
            for p in rgba.pixels() {
                rgb.extend_from_slice(&p.0[..3]);
                alpha.push(p.0[3]);
                translucent |= p.0[3] != 255;
            }
            dict.set("ColorSpace", "DeviceRGB");
            if translucent {
                let sm = dictionary! {
                    "Type" => "XObject", "Subtype" => "Image", "Width" => w as i64, "Height" => h as i64,
                    "ColorSpace" => "DeviceGray", "BitsPerComponent" => 8,
                };
                let sid = self.doc.add_object(Stream::new(sm, alpha));
                dict.set("SMask", Object::Reference(sid));
            }
            content = rgb;
        }
        let id = self.doc.add_object(Stream::new(dict, content));
        Ok((id, w, h))
    }
}

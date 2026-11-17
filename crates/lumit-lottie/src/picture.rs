//! A still as the bytes a file carries: PNG, lossless, or JPEG when a
//! quality is asked for and nothing in the picture is see-through (JPEG has
//! no alpha, so a picture that needs one stays PNG).
//!
//! Runs on the export's thread.

use crate::Still;

/// An encoded picture and what to call it.
pub(crate) struct Encoded {
    pub bytes: Vec<u8>,
    /// The file extension, without the dot.
    pub ext: &'static str,
    /// The media type, for a data URI.
    pub mime: &'static str,
}

/// `still` as PNG, or as JPEG at `quality` (1 to 100) when it is opaque.
/// `None` when the encoder refuses it (a size that doesn't match the pixels).
pub(crate) fn encode(still: &Still, quality: Option<u8>) -> Option<Encoded> {
    let opaque = still.rgba.chunks_exact(4).all(|p| p.get(3) == Some(&255));
    match quality {
        Some(q) if opaque => {
            let rgb: Vec<u8> = still
                .rgba
                .chunks_exact(4)
                .flat_map(|p| p.iter().take(3).copied())
                .collect();
            let mut bytes = Vec::new();
            image::codecs::jpeg::JpegEncoder::new_with_quality(&mut bytes, q.clamp(1, 100))
                .encode(
                    &rgb,
                    still.width,
                    still.height,
                    image::ExtendedColorType::Rgb8,
                )
                .ok()?;
            Some(Encoded {
                bytes,
                ext: "jpg",
                mime: "image/jpeg",
            })
        }
        _ => {
            let mut bytes = Vec::new();
            let mut encoder = png::Encoder::new(&mut bytes, still.width, still.height);
            encoder.set_color(png::ColorType::Rgba);
            encoder.set_depth(png::BitDepth::Eight);
            let mut writer = encoder.write_header().ok()?;
            writer.write_image_data(&still.rgba).ok()?;
            writer.finish().ok()?;
            Some(Encoded {
                bytes,
                ext: "png",
                mime: "image/png",
            })
        }
    }
}

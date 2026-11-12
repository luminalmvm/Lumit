//! The bytes that name a [`TextBlock`]'s picture.

use crate::TextBlock;

impl TextBlock<'_> {
    /// Every field of the block as bytes, for the name of a kept raster. The
    /// fields are named one by one, so a new one will not build until it is
    /// added here too.
    #[must_use]
    pub fn key_bytes(&self) -> Vec<u8> {
        let TextBlock {
            text,
            size,
            fill,
            style,
            paragraph,
            boxed,
        } = *self;
        let mut out = Vec::new();
        let mut part = |bytes: &[u8]| {
            out.extend_from_slice(&(bytes.len() as u64).to_le_bytes());
            out.extend_from_slice(bytes);
        };
        part(text.as_bytes());
        part(&size.to_le_bytes());
        part(&fill.0.map(f32::to_le_bytes).concat());
        part(&style.key_bytes());
        part(&paragraph.key_bytes());
        part(&[u8::from(boxed)]);
        out
    }
}

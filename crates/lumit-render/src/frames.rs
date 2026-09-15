//! The one way a baked per-frame analysis reads a clip.
//!
//! Two tiers read frames the same way - the Roto brush's propagation and the
//! planes tier's model runs - and both want the same thing: the source's own
//! raster, as RGBA bytes, one frame at a time, opened on the worker thread. So
//! the trait lives here rather than on whichever of them was written first, and
//! each names it from its own module.

/// One frame of a clip as **encoded RGBA8** at the source's own raster,
/// everything a baked analysis reads.
///
/// A trait for [`LumaFrames`](crate::track::LumaFrames)'s reason: the engine
/// tests feed a synthetic shot with an answer they wrote down, since asking them
/// to encode a video first would be measuring ffmpeg. Whichever it is, it is
/// opened on the analysis thread and never on the caller's.
pub trait RotoFrames {
    /// `(frames, width, height, frames per second)`.
    fn info(&self) -> (usize, u32, u32, f64);
    /// Frame `n` as row-major RGBA8, `width · height · 4` long. `None` ends the
    /// run early - a clip that stops decoding part-way is analysed as far as it
    /// went, which is the same honesty a partial track has.
    fn rgba(&mut self, n: usize) -> Option<Vec<u8>>;
}

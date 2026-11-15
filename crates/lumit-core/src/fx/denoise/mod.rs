//! Denoise's maths: the CPU twin of `fx_denoise*.wgsl`, and the noise
//! measurement the Learn button runs.
//!
//! **In plain terms.** The picture is put in Y'CbCr. Frames either side, each
//! moved along the measured motion so it lines up, are averaged in wherever a
//! 3x3 patch of them looks like the current frame give or take the noise. What
//! is left is split into four bands of detail by size, and each band is kept
//! in proportion to how far its local energy stands above the noise expected
//! there (a local Wiener gain). Detail, Sharpen and Grain then give back some
//! of what the shrink took.
//!
//! Every function here is the one a kernel pass mirrors, pass for pass, so the
//! oracle test can hold the two together. They run on the worker that called
//! them and keep no state.

pub mod atrous;
pub mod learn;
pub mod space;
pub mod spatial;
pub mod temporal;

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests;

/// One resolved Denoise, reduced to the numbers the passes read.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DenoiseParams {
    /// 0 the result, 1 the removed noise, 2 how many frames were blended.
    pub view: u32,
    /// Luma noise, a sigma of encoded Y in 0..1 units, before the tone scale.
    pub luma_sigma: f32,
    /// Chroma noise, a sigma of Cb and Cr together.
    pub chroma_sigma: f32,
    /// Luma noise multipliers at shadows, midtones and highlights.
    pub tone: [f32; 3],
    /// Each band's luma noise relative to `luma_sigma`, finest first.
    pub luma_bands: [f32; 4],
    /// Each band's chroma noise relative to `chroma_sigma`.
    pub chroma_bands: [f32; 4],
    /// Frames either side averaged in.
    pub frames: u32,
    /// Whether the neighbours are moved along the measured motion first.
    pub motion: bool,
    /// How different a patch may be and still be averaged in: below `t0` it
    /// counts fully, and its weight falls to nothing over `t1` more.
    pub t0: f32,
    pub t1: f32,
    /// Temporal strength, 0..2. Nothing is averaged at 0.
    pub strength: f32,
    /// How much of the noise each channel loses, 0..1.
    pub luma_amount: f32,
    pub chroma_amount: f32,
    /// How much of each band's noise is taken, 0..1, finest first.
    pub band_amounts: [f32; 4],
    /// 0..1: texture given back where the band stands above the noise.
    pub detail: f32,
    /// 0..2: the fine luma bands lifted where they are signal.
    pub sharpen: f32,
    /// 0..1: the fine part of the removed luma noise put back.
    pub grain: f32,
    /// 0..1, blended against the input.
    pub mix: f32,
}

/// The picture Denoise makes of `input` (premultiplied linear RGBA) with the
/// `aligned` neighbours averaged in: each `w * h * 4`, Y'CbCr with the
/// field's confidence in the fourth lane, as [`temporal::warp`] makes them.
/// With none it is the spatial pass alone, which is what the fallback runs.
/// `matte` (RGBA, or empty for none) scales the reduction per pixel.
#[must_use]
pub fn run(
    input: &[f32],
    w: u32,
    h: u32,
    aligned: &[&[f32]],
    p: &DenoiseParams,
    matte: &[f32],
) -> Vec<f32> {
    let n = (w as usize) * (h as usize) * 4;
    if input.len() != n || w == 0 || h == 0 {
        return input.to_vec();
    }
    let current = space::picture_to_ycc(input);
    let averaged = if p.frames == 0 {
        temporal::plain(&current)
    } else {
        temporal::accumulate(&current, aligned, w, h, p)
    };
    let (details, coarse) = spatial::shrink(&averaged, &current, w, h, p);
    spatial::finish(
        input, &current, &details, &coarse, &averaged, w, h, p, matte,
    )
}

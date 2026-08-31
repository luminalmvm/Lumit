//! Warp and seed: carrying a solved matte on to the next frame
//! (docs/impl/roto.md §3).
//!
//! The flow arrives as **plain borrowed slices** rather than through
//! `lumit-flow`: that crate pulls wgpu in, and this one is CPU arithmetic with
//! no graphics device anywhere near it — the stance `lumit-track` takes for the
//! same reason. Whoever owns the flow engine hands the numbers over.

use crate::{check_plane, RotoError, Seed, Seeds};

/// Above this warped alpha a pixel seeds foreground.
const WARP_FG: f32 = 0.9;
/// Below this warped alpha a pixel seeds background.
const WARP_BG: f32 = 0.1;
/// How far both seed sets are shrunk before the solve reads them.
const ERODE_PX: u32 = 2;

/// One frame pair's flow, borrowed: where each pixel of the **new** frame came
/// from in the previous one, whether that answer exists at all, and how much
/// the forward and backward flows agreed about it.
#[derive(Debug, Clone, Copy)]
pub struct FlowField<'a> {
    /// Two f32 per pixel — the offset in source pixels from this frame back to
    /// the previous one, which is the direction a backward warp needs.
    flow: &'a [f32],
    /// Zero where there is no flow for this pixel at all.
    validity: &'a [u8],
    /// Forward–backward agreement, 0..1.
    confidence: &'a [f32],
    width: u32,
    height: u32,
}

impl<'a> FlowField<'a> {
    /// Borrow one frame pair's flow, checking every plane against the size it
    /// claims — the trust boundary between this crate and whoever computed it.
    pub fn new(
        flow: &'a [f32],
        validity: &'a [u8],
        confidence: &'a [f32],
        width: u32,
        height: u32,
    ) -> Result<Self, RotoError> {
        check_plane(flow.len(), 2, width, height)?;
        check_plane(validity.len(), 1, width, height)?;
        check_plane(confidence.len(), 1, width, height)?;
        Ok(Self {
            flow,
            validity,
            confidence,
            width,
            height,
        })
    }

    pub fn width(&self) -> u32 {
        self.width
    }

    pub fn height(&self) -> u32 {
        self.height
    }
}

/// Derive the next frame's seeds from the previous frame's matte.
///
/// `out` is cleared, then filled: the previous matte is backward-warped by the
/// flow, its confident interiors become seeds (`α_w > 0.9` foreground,
/// `α_w < 0.1` background), and both sets are eroded by two pixels. A pixel
/// whose flow is missing, untrusted, or points off the frame **seeds nothing** —
/// occlusions, reveals and flow failures are exactly the pixels that must be
/// re-decided from the new frame's own colours.
///
/// The frame's own correction strokes are stamped on top afterwards by the
/// caller, which is how the user outranks the machine.
pub fn warp_and_seed(
    prev_matte: &[f32],
    flow: &FlowField<'_>,
    confidence_floor: f32,
    out: &mut Seeds,
) -> Result<(), RotoError> {
    let (w, h) = (flow.width, flow.height);
    check_plane(prev_matte.len(), 1, w, h)?;
    if out.width() != w || out.height() != h {
        return Err(RotoError::SizeMismatch {
            a_width: w,
            a_height: h,
            b_width: out.width(),
            b_height: out.height(),
        });
    }
    out.clear();
    for y in 0..h {
        for x in 0..w {
            let i = (y as usize) * (w as usize) + (x as usize);
            if flow.validity.get(i).copied().unwrap_or(0) == 0 {
                continue;
            }
            if flow.confidence.get(i).copied().unwrap_or(0.0) < confidence_floor {
                continue;
            }
            let fx = flow.flow.get(i * 2).copied().unwrap_or(0.0);
            let fy = flow.flow.get(i * 2 + 1).copied().unwrap_or(0.0);
            if !(fx.is_finite() && fy.is_finite()) {
                continue;
            }
            let sx = x as f32 + fx;
            let sy = y as f32 + fy;
            // Off the frame is not a seed: extending the border matte outwards
            // would invent a subject where nothing was ever seen.
            if sx < 0.0 || sy < 0.0 || sx > (w - 1) as f32 || sy > (h - 1) as f32 {
                continue;
            }
            let a = sample(prev_matte, w, h, sx, sy);
            if a > WARP_FG {
                out.set(i, Seed::Foreground);
            } else if a < WARP_BG {
                out.set(i, Seed::Background);
            }
        }
    }
    out.erode(ERODE_PX);
    Ok(())
}

/// Bilinear read of the previous matte, the same backward-warp sampling
/// synthesis uses — holes and z-fighting avoided by construction.
fn sample(matte: &[f32], w: u32, h: u32, x: f32, y: f32) -> f32 {
    let (w, h) = (w as usize, h as usize);
    let x0 = x.floor().max(0.0) as usize;
    let y0 = y.floor().max(0.0) as usize;
    let x1 = (x0 + 1).min(w.saturating_sub(1));
    let y1 = (y0 + 1).min(h.saturating_sub(1));
    let x0 = x0.min(w.saturating_sub(1));
    let y0 = y0.min(h.saturating_sub(1));
    let tx = (x - x0 as f32).clamp(0.0, 1.0);
    let ty = (y - y0 as f32).clamp(0.0, 1.0);
    let at = |xi: usize, yi: usize| matte.get(yi * w + xi).copied().unwrap_or(0.0);
    let top = at(x0, y0) + (at(x1, y0) - at(x0, y0)) * tx;
    let bottom = at(x0, y1) + (at(x1, y1) - at(x0, y1)) * tx;
    top + (bottom - top) * ty
}

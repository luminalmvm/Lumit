//! `lumit-roto` — the roto brush's arithmetic: stroke seeding, the geodesic
//! segmentation, the guided-filter edge, and carrying a matte along a flow
//! field. The algorithms are pinned in [docs/impl/roto.md]; this crate is
//! package RB1 of that note and holds nothing else.
//!
//! # In plain terms
//!
//! Rotoscoping is cutting a moving thing out of its shot: for every frame, a
//! grey picture (a **matte**) that is white where the subject is and black
//! where it is not. Doing it by hand means drawing round the subject on every
//! frame. This crate does the arithmetic that shortens that job to a few
//! scribbles.
//!
//! Three ideas carry the whole thing.
//!
//! *A scribble is a claim, not an outline.* The user drags a stroke through the
//! subject and another through the background; the pixels under those strokes
//! become **seeds** — pixels whose answer is known. Nothing else is known.
//!
//! *Every other pixel joins the seed it can reach most cheaply.* Travel costs
//! distance, and it also costs **colour change**: stepping across an edge where
//! the picture changes hue is expensive, walking along a flat region is nearly
//! free. So a pixel deep inside the subject reaches the subject's seed cheaply
//! and the background's seed only by paying for the crossing, and the matte
//! falls out of comparing the two prices. This is the *geodesic distance
//! transform*, and it is computed by six sweeps over the picture — three
//! left-to-right-and-down, three right-to-left-and-up — each pixel taking the
//! best price its already-visited neighbours can offer.
//!
//! *The next frame is not solved from nothing.* Given the optical flow — how
//! far each pixel moved — the previous frame's matte is dragged into place and
//! its confident interiors become this frame's seeds, shrunk by two pixels so
//! a smeared motion boundary never seeds the wrong side. Then the same solve
//! re-decides the edge from this frame's own colours, so drift cannot pile up.
//! Where the flow was not trusted, nothing is seeded at all: those pixels are
//! exactly the ones that must be decided afresh.
//!
//! Last comes the **refine edge** pass: real edges are soft — hair, motion
//! blur, smoke — and the segmentation's answer is hard. A *guided filter*
//! reshapes the matte inside a narrow band around the boundary so it follows
//! the colours actually there, and everything outside that band is snapped to
//! solid or empty so a textured interior can never go grey.
//!
//! # Thread role and contract
//!
//! Pure computation: no IO, no clocks, no threads, no GPU, no interior
//! mutability. Frames arrive as borrowed encoded-RGB planes ([`FrameRgb`]) and
//! flow as borrowed slices ([`FlowField`]), one frame at a time, so
//! cancellation is the caller's frame loop (14-ENGINEERING-RULES §1.4).
//! [`RotoSolver`] owns every working buffer and reuses it across frames — the
//! per-frame allocation is zero once the frame size settles.
//!
//! Everything is deterministic: fixed scan orders throughout, a fixed pass
//! count (no convergence test), f32 arithmetic in that order, no `HashMap` on
//! any path that reaches a result. Two runs over the same frames produce
//! bit-identical mattes.
//!
//! Coordinates are **source raster pixels**: strokes describe the full,
//! unaltered footage, so one shot's mattes serve every comp that cuts it.
//!
//! ```
//! use lumit_roto::{base_seeds, FrameRgb, RotoSolver, RotoSettings, RotoStroke, StrokeKind};
//!
//! // A 32×32 frame: dark, with a bright square in the middle.
//! let (w, h) = (32u32, 32u32);
//! let mut rgb = vec![0.1f32; (w * h * 3) as usize];
//! for y in 10..22 {
//!     for x in 10..22 {
//!         let i = ((y * w + x) * 3) as usize;
//!         rgb[i] = 0.9;
//!         rgb[i + 1] = 0.9;
//!         rgb[i + 2] = 0.9;
//!     }
//! }
//! let stroke = RotoStroke {
//!     id: uuid::Uuid::nil(),
//!     points: vec![(14.0, 16.0), (18.0, 16.0)],
//!     radius: 1.5,
//!     kind: StrokeKind::Foreground,
//!     frame: 0,
//! };
//! // No background stroke, so the frame border seeds background by default.
//! let seeds = base_seeds(w, h, &[stroke])?;
//! let mut solver = RotoSolver::new(RotoSettings::default());
//! let mut matte = vec![0.0f32; (w * h) as usize];
//! solver.solve(FrameRgb::new(&rgb, w, h)?, &seeds, &mut matte)?;
//! assert!(matte[(16 * w + 16) as usize] > 0.9); // inside the square
//! assert!(matte[(2 * w + 2) as usize] < 0.1); // out in the dark
//! # Ok::<(), lumit_roto::RotoError>(())
//! ```
//!
//! [docs/impl/roto.md]: https://github.com/lumit/lumit/blob/main/docs/impl/roto.md

mod gdt;
mod guided;
mod solve;
mod stroke;
mod warp;

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests;

pub use solve::RotoSolver;
pub use stroke::{base_seeds, RotoStroke, Seed, Seeds, StrokeKind};
pub use warp::{warp_and_seed, FlowField};

/// Everything this crate refuses, each a named error and never a fault
/// (14-ENGINEERING-RULES §4).
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum RotoError {
    /// A frame size that cannot describe a picture, or one whose pixel count
    /// does not fit in a `usize`.
    #[error("frame size {width}×{height} is not a picture")]
    BadSize { width: u32, height: u32 },
    /// A borrowed plane whose length disagrees with the frame size it was
    /// handed with — the trust boundary of every entry point here.
    #[error("expected {want} values for {width}×{height}, got {got}")]
    PlaneSize {
        width: u32,
        height: u32,
        want: usize,
        got: usize,
    },
    /// Two planes handed to the same call describe different pictures.
    #[error("planes disagree: {a_width}×{a_height} against {b_width}×{b_height}")]
    SizeMismatch {
        a_width: u32,
        a_height: u32,
        b_width: u32,
        b_height: u32,
    },
    /// A solve with no foreground seed, or none in the background, has no
    /// answer to give — §2's reason the frame border seeds background by
    /// default on a base frame.
    #[error("a solve needs at least one foreground seed and one background seed")]
    NoSeeds,
}

/// The knobs the roto solve exposes. Everything the note pins as a constant —
/// the three pass pairs, the 0.9/0.1 seed thresholds, the 2 px erosion — is a
/// constant here rather than a field, because a setting nobody may change is a
/// setting that only invalidates caches.
#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct RotoSettings {
    /// How dearly the geodesic walk pays for colour change against distance
    /// (docs/impl/roto.md §2). At the default, a full-scale step in one encoded
    /// channel costs about as much as fifty pixels of travel.
    pub gamma: f32,
    /// The guided filter's window radius in source pixels, and the half-width
    /// of the band its answer is allowed into.
    pub guide_radius: u32,
    /// The guided filter's regulariser: bigger is smoother and flatter.
    pub guide_eps: f32,
    /// The refine band: the filter's answer replaces the matte where
    /// `|α_raw − ½|` is under this, dilated by [`guide_radius`](Self::guide_radius).
    pub band: f32,
    /// Warped seeds are taken only from pixels whose flow confidence reaches
    /// this. Below it, nothing is seeded and the pixel is re-decided from the
    /// frame's own colours.
    pub confidence_floor: f32,
}

impl Default for RotoSettings {
    fn default() -> Self {
        Self {
            gamma: 50.0,
            guide_radius: 8,
            guide_eps: 1e-3,
            band: 0.45,
            confidence_floor: 0.5,
        }
    }
}

/// One frame's **encoded** RGB, borrowed: three interleaved f32 per pixel.
///
/// Encoded rather than linear, and that is a choice rather than an oversight:
/// the geodesic cost and the guided filter both want colour distance to mean
/// what the eye means by it, which is the same reading the optical flow's
/// correlation already made and measured (docs/impl/optical-flow.md §1).
#[derive(Debug, Clone, Copy)]
pub struct FrameRgb<'a> {
    rgb: &'a [f32],
    width: u32,
    height: u32,
}

impl<'a> FrameRgb<'a> {
    /// Borrow a plane, checking it against the size it claims.
    pub fn new(rgb: &'a [f32], width: u32, height: u32) -> Result<Self, RotoError> {
        let want = pixel_count(width, height)?
            .checked_mul(3)
            .ok_or(RotoError::BadSize { width, height })?;
        if rgb.len() != want {
            return Err(RotoError::PlaneSize {
                width,
                height,
                want,
                got: rgb.len(),
            });
        }
        Ok(Self { rgb, width, height })
    }

    pub fn width(&self) -> u32 {
        self.width
    }

    pub fn height(&self) -> u32 {
        self.height
    }

    /// The pixel at flat index `i`. Out of range reads as black rather than
    /// panicking; every caller in this crate walks `0..w*h`.
    #[inline]
    pub(crate) fn px(&self, i: usize) -> [f32; 3] {
        let base = i * 3;
        match self.rgb.get(base..base + 3) {
            Some([r, g, b]) => [*r, *g, *b],
            _ => [0.0; 3],
        }
    }
}

/// `width × height` as a `usize`, or a typed error — the one place this crate
/// multiplies a frame size.
pub(crate) fn pixel_count(width: u32, height: u32) -> Result<usize, RotoError> {
    if width == 0 || height == 0 {
        return Err(RotoError::BadSize { width, height });
    }
    (width as usize)
        .checked_mul(height as usize)
        .ok_or(RotoError::BadSize { width, height })
}

/// Check a borrowed plane of `per_pixel` values against a frame size.
pub(crate) fn check_plane(
    len: usize,
    per_pixel: usize,
    width: u32,
    height: u32,
) -> Result<usize, RotoError> {
    let n = pixel_count(width, height)?;
    let want = n
        .checked_mul(per_pixel)
        .ok_or(RotoError::BadSize { width, height })?;
    if len != want {
        return Err(RotoError::PlaneSize {
            width,
            height,
            want,
            got: len,
        });
    }
    Ok(n)
}

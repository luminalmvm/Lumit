//! Strokes, and the seeds they stamp (docs/impl/roto.md §1, §2).
//!
//! A stroke is the user's edit and lives in the document; seeds are what the
//! solve reads. Turning one into the other is the whole of this file: dabs
//! walked along a polyline exactly as paint strokes are stamped
//! (docs/impl/paint.md), later stroke winning wherever two overlap.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{pixel_count, RotoError};

/// A dab every quarter radius, the spacing the paint rasteriser has always
/// used: close enough that a run of discs reads as a line.
const DAB_SPACING: f32 = 0.25;

/// The most dabs one segment may ask for, so a stroke with an absurd length or
/// a hair-thin radius cannot ask for an unbounded walk (14 §5, budgeted).
const MAX_DABS_PER_SEGMENT: usize = 4096;

/// How wide the default background ring round the frame is, in pixels.
const BORDER_RING_PX: u32 = 2;

/// What a stroke claims about the pixels under it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum StrokeKind {
    /// These pixels are the subject.
    Foreground,
    /// These pixels are not.
    Background,
    /// These pixels want the refine band, whatever the segmentation decided —
    /// the one lock of hair that needs more room than the automatic band.
    Refine,
}

/// One roto stroke: the path the pointer took, and what it claims.
///
/// A **polyline** rather than a bezier, for the reason paint strokes are one:
/// it is a record of a gesture, sampled as it happened, not a shape anyone
/// will edit vertex by vertex. Points are **source raster pixels** on the
/// full, unaltered footage, so the matte describes the file's frames
/// and survives every comp-side transform, retime and preview tier.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RotoStroke {
    pub id: Uuid,
    /// The pointer's path in source raster pixels, in the order it was drawn.
    pub points: Vec<(f32, f32)>,
    /// Half the brush's width, in source raster pixels.
    pub radius: f32,
    pub kind: StrokeKind,
    /// The **source** frame index the stroke was drawn on.
    pub frame: i64,
}

/// What is known about one pixel before the solve runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Seed {
    /// Nothing is known; the solve decides.
    #[default]
    None,
    Foreground,
    Background,
}

/// The seed field for one frame: what the strokes and the warped matte claim,
/// plus where a `Refine` stroke has asked for the band.
///
/// Reused across frames — [`clear`](Self::clear) rather than a fresh
/// allocation per frame (14 §5).
#[derive(Debug, Clone)]
pub struct Seeds {
    width: u32,
    height: u32,
    cells: Vec<Seed>,
    /// Erosion reads the pre-erosion state while writing the new one; this is
    /// that copy, kept so a per-frame erosion allocates nothing.
    scratch: Vec<Seed>,
    refine: Vec<bool>,
}

impl Seeds {
    /// An empty seed field for a frame of this size.
    pub fn new(width: u32, height: u32) -> Result<Self, RotoError> {
        let n = pixel_count(width, height)?;
        Ok(Self {
            width,
            height,
            cells: vec![Seed::None; n],
            scratch: Vec::new(),
            refine: vec![false; n],
        })
    }

    pub fn width(&self) -> u32 {
        self.width
    }

    pub fn height(&self) -> u32 {
        self.height
    }

    /// Forget everything, keeping the buffers.
    pub fn clear(&mut self) {
        self.cells.fill(Seed::None);
        self.refine.fill(false);
    }

    /// The seed at flat index `i`; out of range reads as [`Seed::None`].
    #[inline]
    pub fn at(&self, i: usize) -> Seed {
        self.cells.get(i).copied().unwrap_or_default()
    }

    /// Whether a `Refine` stroke painted this pixel.
    #[inline]
    pub fn refine_at(&self, i: usize) -> bool {
        self.refine.get(i).copied().unwrap_or(false)
    }

    /// Set one pixel's seed, ignoring an index off the frame.
    #[inline]
    pub fn set(&mut self, i: usize, seed: Seed) {
        if let Some(slot) = self.cells.get_mut(i) {
            *slot = seed;
        }
    }

    /// How many foreground and background seeds there are — the check that
    /// turns a hopeless solve into [`RotoError::NoSeeds`] rather than a NaN.
    pub fn counts(&self) -> (usize, usize) {
        let mut fg = 0;
        let mut bg = 0;
        for cell in &self.cells {
            match cell {
                Seed::Foreground => fg += 1,
                Seed::Background => bg += 1,
                Seed::None => {}
            }
        }
        (fg, bg)
    }

    /// Seed a ring of background round the frame border.
    ///
    /// The base frame's default when the user has drawn no background stroke:
    /// nobody paints the background first, and a solve with no background seed
    /// has no answer (docs/impl/roto.md §2).
    pub fn seed_border_ring(&mut self, thickness: u32) {
        let t = thickness.min(self.width / 2).min(self.height / 2).max(1);
        for y in 0..self.height {
            let edge_row = y < t || y >= self.height.saturating_sub(t);
            for x in 0..self.width {
                if edge_row || x < t || x >= self.width.saturating_sub(t) {
                    let i = (y as usize) * (self.width as usize) + (x as usize);
                    self.set(i, Seed::Background);
                }
            }
        }
    }

    /// Stamp every stroke in document order. **Later wins** wherever two
    /// overlap: the user's most recent word is the verdict.
    pub fn stamp_all(&mut self, strokes: &[RotoStroke]) {
        for stroke in strokes {
            self.stamp(stroke);
        }
    }

    /// Stamp one stroke: dabs along its polyline, the radius honoured.
    pub fn stamp(&mut self, stroke: &RotoStroke) {
        let radius = if stroke.radius.is_finite() {
            stroke.radius.max(0.5)
        } else {
            return;
        };
        let Some(&first) = stroke.points.first() else {
            return;
        };
        self.disc(first, radius, stroke.kind);
        for pair in stroke.points.windows(2) {
            let (x0, y0) = pair[0];
            let (x1, y1) = pair[1];
            if !(x0.is_finite() && y0.is_finite() && x1.is_finite() && y1.is_finite()) {
                continue;
            }
            let (dx, dy) = (x1 - x0, y1 - y0);
            let length = (dx * dx + dy * dy).sqrt();
            let step = (radius * DAB_SPACING).max(0.25);
            if length <= step {
                self.disc((x1, y1), radius, stroke.kind);
                continue;
            }
            let count = ((length / step).ceil() as usize).clamp(1, MAX_DABS_PER_SEGMENT);
            for j in 1..=count {
                let t = j as f32 / count as f32;
                self.disc((x0 + dx * t, y0 + dy * t), radius, stroke.kind);
            }
        }
    }

    /// One dab: every pixel whose centre is inside the disc takes the claim.
    fn disc(&mut self, centre: (f32, f32), radius: f32, kind: StrokeKind) {
        let (cx, cy) = centre;
        if !(cx.is_finite() && cy.is_finite()) {
            return;
        }
        let x0 = (cx - radius).floor().max(0.0) as u32;
        let y0 = (cy - radius).floor().max(0.0) as u32;
        let x1 = (cx + radius).ceil().min(f32::from(u16::MAX)) as u32;
        let y1 = (cy + radius).ceil().min(f32::from(u16::MAX)) as u32;
        let x1 = x1.min(self.width.saturating_sub(1));
        let y1 = y1.min(self.height.saturating_sub(1));
        if x0 > x1 || y0 > y1 {
            return;
        }
        let r2 = radius * radius;
        for y in y0..=y1 {
            for x in x0..=x1 {
                let dx = x as f32 + 0.5 - cx;
                let dy = y as f32 + 0.5 - cy;
                if dx * dx + dy * dy > r2 {
                    continue;
                }
                let i = (y as usize) * (self.width as usize) + (x as usize);
                match kind {
                    StrokeKind::Foreground => self.set(i, Seed::Foreground),
                    StrokeKind::Background => self.set(i, Seed::Background),
                    StrokeKind::Refine => {
                        if let Some(slot) = self.refine.get_mut(i) {
                            *slot = true;
                        }
                    }
                }
            }
        }
    }

    /// Shrink both seed sets by `radius` pixels: a seed survives only where
    /// every neighbour within the square agrees with it.
    ///
    /// Load-bearing rather than tidy (docs/impl/roto.md §12): warped seeds sit
    /// wherever the flow put them, and a motion boundary the flow blurred would
    /// otherwise plant foreground seeds on the background's side of the edge.
    /// At the frame border the window is clipped rather than counted as
    /// disagreement — the erosion is aimed at motion boundaries, not at the
    /// picture's own edge.
    pub fn erode(&mut self, radius: u32) {
        if radius == 0 {
            return;
        }
        self.scratch.clear();
        self.scratch.extend_from_slice(&self.cells);
        let (w, h) = (self.width as i64, self.height as i64);
        let r = radius as i64;
        for y in 0..h {
            for x in 0..w {
                let i = (y * w + x) as usize;
                let seed = self.scratch.get(i).copied().unwrap_or_default();
                if seed == Seed::None {
                    continue;
                }
                let mut keep = true;
                'window: for ny in (y - r).max(0)..=(y + r).min(h - 1) {
                    for nx in (x - r).max(0)..=(x + r).min(w - 1) {
                        let j = (ny * w + nx) as usize;
                        if self.scratch.get(j).copied().unwrap_or_default() != seed {
                            keep = false;
                            break 'window;
                        }
                    }
                }
                if !keep {
                    self.set(i, Seed::None);
                }
            }
        }
    }
}

/// The seed field for a **base** frame: the user's strokes, with a background
/// ring round the border when the user has drawn no background stroke at all.
pub fn base_seeds(width: u32, height: u32, strokes: &[RotoStroke]) -> Result<Seeds, RotoError> {
    let mut seeds = Seeds::new(width, height)?;
    if !strokes.iter().any(|s| s.kind == StrokeKind::Background) {
        seeds.seed_border_ring(BORDER_RING_PX);
    }
    seeds.stamp_all(strokes);
    Ok(seeds)
}

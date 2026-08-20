//! Image pyramids and sampling for the tracker (docs/impl/tracking.md §2).
//!
//! In plain terms: a pyramid is the same picture stored several times over, each
//! copy half the width and half the height of the one before. A patch that moves
//! forty pixels between two frames moves five pixels in the copy that is eight
//! times smaller, and five pixels is a distance a local search can cross. So the
//! tracker answers the question on the small picture first and carries that
//! answer down to the big one, where it only has to be refined.
//!
//! The downsample is the same 2×2 box `lumit-flow` uses for its own pyramid, so
//! the two crates see the same coarse pictures. It is copied rather than shared
//! because `lumit-flow`'s is `pub(crate)` and that crate pulls in `wgpu`
//! (docs/05: engine crates do not depend on a GPU crate to borrow four lines of
//! arithmetic).

/// One single-channel image level, row-major, in whatever units the caller's
/// luma is (0..1 encoded luma is what the tracker is tuned for).
pub(crate) struct Plane {
    pub(crate) w: usize,
    pub(crate) h: usize,
    pub(crate) data: Vec<f32>,
}

impl Plane {
    fn at(&self, x: usize, y: usize) -> f32 {
        self.data[y * self.w + x]
    }

    /// Bilinear sample with edge clamp, accumulated in `f64` (§2: all `f64`
    /// accumulation over `f32` pixels).
    pub(crate) fn sample(&self, x: f64, y: f64) -> f64 {
        if self.w == 0 || self.h == 0 {
            return 0.0;
        }
        let x = x.clamp(0.0, (self.w - 1) as f64);
        let y = y.clamp(0.0, (self.h - 1) as f64);
        let x0 = x.floor() as usize;
        let y0 = y.floor() as usize;
        let x1 = (x0 + 1).min(self.w - 1);
        let y1 = (y0 + 1).min(self.h - 1);
        let fx = x - x0 as f64;
        let fy = y - y0 as f64;
        let a = f64::from(self.at(x0, y0)) * (1.0 - fx) + f64::from(self.at(x1, y0)) * fx;
        let b = f64::from(self.at(x0, y1)) * (1.0 - fx) + f64::from(self.at(x1, y1)) * fx;
        a * (1.0 - fy) + b * fy
    }

    /// Whether `(x, y)` sits at least `margin` pixels inside every edge.
    pub(crate) fn inside(&self, x: f64, y: f64, margin: f64) -> bool {
        if self.w == 0 || self.h == 0 {
            return false;
        }
        x >= margin
            && y >= margin
            && x <= (self.w - 1) as f64 - margin
            && y <= (self.h - 1) as f64 - margin
    }
}

/// A frame at several scales: level 0 is the source raster, each level after it
/// half the size of the one before.
pub(crate) struct Pyramid {
    pub(crate) levels: Vec<Plane>,
}

impl Pyramid {
    pub(crate) fn new() -> Self {
        Pyramid { levels: Vec::new() }
    }

    /// How many levels a `w × h` frame supports without the coarsest dropping
    /// under `min_dim` in either dimension, capped at `want`.
    pub(crate) fn usable_levels(w: usize, h: usize, want: usize, min_dim: usize) -> usize {
        let mut n = 1;
        let (mut lw, mut lh) = (w, h);
        while n < want.max(1) {
            let (nw, nh) = (lw / 2, lh / 2);
            if nw.min(nh) < min_dim {
                break;
            }
            lw = nw;
            lh = nh;
            n += 1;
        }
        n
    }

    /// Rebuild every level from `luma`. The buffers are allocated once for a
    /// given size and level count and overwritten on every later frame
    /// (14-ENGINEERING-RULES §5: frame-sized allocations are budgeted). Two
    /// pyramids and the detector's `Scratch` are a tracker's whole frame-sized
    /// budget; nothing else here is allocated per frame.
    pub(crate) fn fill(&mut self, luma: &[f32], w: usize, h: usize, levels: usize) {
        let shape_matches = self.levels.len() == levels
            && self.levels.first().is_some_and(|l| l.w == w && l.h == h);
        if !shape_matches {
            self.levels.clear();
            let (mut lw, mut lh) = (w, h);
            for _ in 0..levels {
                self.levels.push(Plane {
                    w: lw,
                    h: lh,
                    data: vec![0.0; lw * lh],
                });
                lw = (lw / 2).max(1);
                lh = (lh / 2).max(1);
            }
        }
        if let Some(l0) = self.levels.first_mut() {
            l0.data.copy_from_slice(&luma[..w * h]);
        }
        for i in 1..self.levels.len() {
            let (lower, upper) = self.levels.split_at_mut(i);
            let src = &lower[i - 1];
            let dst = &mut upper[0];
            downsample_into(src, dst);
        }
    }
}

/// Box-downsample by 2 — the same average of the four source pixels
/// `lumit-flow` takes, clamped at the right and bottom edges of an odd source.
fn downsample_into(src: &Plane, dst: &mut Plane) {
    for y in 0..dst.h {
        for x in 0..dst.w {
            let x0 = (2 * x).min(src.w - 1);
            let y0 = (2 * y).min(src.h - 1);
            let x1 = (2 * x + 1).min(src.w - 1);
            let y1 = (2 * y + 1).min(src.h - 1);
            dst.data[y * dst.w + x] =
                0.25 * (src.at(x0, y0) + src.at(x1, y0) + src.at(x0, y1) + src.at(x1, y1));
        }
    }
}

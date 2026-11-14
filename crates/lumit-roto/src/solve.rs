//! One frame's matte, end to end: the geodesic segmentation, then the refine
//! band (docs/impl/roto.md §2 and §4).

use crate::gdt;
use crate::guided::{box_mean, BoxScratch, Guided};
use crate::{
    check_plane,
    stroke::{Seed, Seeds},
    FrameRgb, RotoError, RotoSettings,
};

/// The long side the distance step aims for. Gamma was tuned on HD, and on a
/// larger frame the same subject is crossed in more, noisier steps, so its
/// edges counted for less and the cut drifted toward the nearest stroke. A
/// frame of 2880 or more on its long side is averaged down by a whole number,
/// for that step only, to bring the long side near this.
const DISTANCE_SIDE: u32 = 1920;

/// Solves frames, one at a time, reusing every buffer.
///
/// Allocation happens when the frame size changes and at no other moment: a
/// six-hundred-frame propagation run allocates once (14 §5). Nothing here is
/// shared, so a caller who wants two solves at once holds two solvers.
#[derive(Debug)]
pub struct RotoSolver {
    settings: RotoSettings,
    len: usize,
    width: u32,
    height: u32,
    steps: Vec<[f32; 4]>,
    d_f: Vec<f32>,
    d_b: Vec<f32>,
    alpha_raw: Vec<f32>,
    filtered: Vec<f32>,
    band: Vec<f32>,
    hard: Vec<f32>,
    boxes: BoxScratch,
    guided: Guided,
    /// The averaged-down picture, seeds and answer of the distance step, on a
    /// frame large enough to be averaged down.
    small_rgb: Vec<f32>,
    small_seeds: Option<Seeds>,
    small_alpha: Vec<f32>,
}

impl RotoSolver {
    pub fn new(settings: RotoSettings) -> Self {
        Self {
            settings,
            len: 0,
            width: 0,
            height: 0,
            steps: Vec::new(),
            d_f: Vec::new(),
            d_b: Vec::new(),
            alpha_raw: Vec::new(),
            filtered: Vec::new(),
            band: Vec::new(),
            hard: Vec::new(),
            boxes: BoxScratch::default(),
            guided: Guided::default(),
            small_rgb: Vec::new(),
            small_seeds: None,
            small_alpha: Vec::new(),
        }
    }

    pub fn settings(&self) -> &RotoSettings {
        &self.settings
    }

    /// The segmentation's own answer for the frame last solved, before the
    /// refine band reshaped it — what the tests compare the filtered edge
    /// against, and what a Boundary view would draw.
    pub fn alpha_raw(&self) -> &[f32] {
        &self.alpha_raw
    }

    /// Solve one frame into `out` (one f32 per pixel, 0..1).
    pub fn solve(
        &mut self,
        frame: FrameRgb<'_>,
        seeds: &Seeds,
        out: &mut [f32],
    ) -> Result<(), RotoError> {
        let (w, h) = (frame.width(), frame.height());
        if seeds.width() != w || seeds.height() != h {
            return Err(RotoError::SizeMismatch {
                a_width: w,
                a_height: h,
                b_width: seeds.width(),
                b_height: seeds.height(),
            });
        }
        let n = check_plane(out.len(), 1, w, h)?;
        let (fg, bg) = seeds.counts();
        if fg == 0 || bg == 0 {
            return Err(RotoError::NoSeeds);
        }
        self.resize(w, h, n);
        self.distance(frame, seeds)?;

        // Each pixel goes to the nearer seed, and the filter reshapes that
        // answer. Not `alpha_raw` itself, which is a ratio of two distances
        // and drifts across the whole frame, so filtered it comes back grey
        // everywhere. Under a Refine stroke neither side is claimed and the
        // colours decide.
        for (i, slot) in self.hard.iter_mut().enumerate() {
            let a = self.alpha_raw.get(i).copied().unwrap_or(0.0);
            *slot = if seeds.refine_at(i) {
                0.5
            } else if a > 0.5 {
                1.0
            } else {
                0.0
            };
        }
        let radius = self.settings.guide_radius;
        self.guided.filter(
            frame,
            &self.hard,
            radius,
            self.settings.guide_eps,
            &mut self.filtered,
        );

        // The band: within the filter's radius of the edge, where the window
        // holds both sides. Everywhere else the answer is snapped, because a
        // filter run over a solid interior greys it wherever the guide happens
        // to have texture (§12's fence).
        box_mean(w, h, radius, &mut self.boxes, &mut self.band, |i| {
            self.hard.get(i).copied().unwrap_or(0.0)
        });
        for i in 0..n {
            let raw = self.alpha_raw.get(i).copied().unwrap_or(0.0);
            let mixed = self.band.get(i).copied().unwrap_or(0.0);
            let in_band = mixed > 0.0 && mixed < 1.0;
            let refine = seeds.refine_at(i);
            let value = if in_band || refine {
                self.filtered.get(i).copied().unwrap_or(raw).clamp(0.0, 1.0)
            } else if raw > 0.5 {
                1.0
            } else {
                0.0
            };
            if let Some(slot) = out.get_mut(i) {
                *slot = value;
            }
        }
        Ok(())
    }

    /// `alpha_raw` for the whole frame. The factor is the long side over
    /// [`DISTANCE_SIDE`], rounded. At 1 it is worked out at the frame's own
    /// size. Above it, the frame and its seeds are averaged down by that
    /// factor, solved there, and scaled back up, with the seeds' own pixels
    /// kept to their claim.
    fn distance(&mut self, frame: FrameRgb<'_>, seeds: &Seeds) -> Result<(), RotoError> {
        let (w, h) = (frame.width(), frame.height());
        let gamma = self.settings.gamma;
        let k = ((w.max(h) + DISTANCE_SIDE / 2) / DISTANCE_SIDE).max(1);
        if k > 1 {
            let (sw, sh) = (w.div_ceil(k), h.div_ceil(k));
            let small = match self.small_seeds.take() {
                Some(s) if s.width() == sw && s.height() == sh => s,
                _ => Seeds::new(sw, sh)?,
            };
            let small = self.small_seeds.insert(small);
            shrink(frame, seeds, k, &mut self.small_rgb, small);
            let (fg, bg) = small.counts();
            // Strokes so close that every cell holding one side also holds the
            // other: only the frame's own size can tell them apart.
            if fg > 0 && bg > 0 {
                let sn = check_plane(self.small_rgb.len(), 3, sw, sh)?;
                self.steps.resize(sn, [0.0; 4]);
                self.d_f.resize(sn, 0.0);
                self.d_b.resize(sn, 0.0);
                self.small_alpha.resize(sn, 0.0);
                gdt::alpha_raw(
                    FrameRgb::new(&self.small_rgb, sw, sh)?,
                    small,
                    gamma,
                    &mut self.steps,
                    &mut self.d_f,
                    &mut self.d_b,
                    &mut self.small_alpha,
                );
                grow(&self.small_alpha, (sw, sh), k, seeds, &mut self.alpha_raw);
                return Ok(());
            }
        }
        let n = self.len;
        self.steps.resize(n, [0.0; 4]);
        self.d_f.resize(n, 0.0);
        self.d_b.resize(n, 0.0);
        gdt::alpha_raw(
            frame,
            seeds,
            gamma,
            &mut self.steps,
            &mut self.d_f,
            &mut self.d_b,
            &mut self.alpha_raw,
        );
        Ok(())
    }

    fn resize(&mut self, width: u32, height: u32, len: usize) {
        if self.len == len && self.width == width && self.height == height {
            return;
        }
        self.len = len;
        self.width = width;
        self.height = height;
        for plane in [
            &mut self.alpha_raw,
            &mut self.filtered,
            &mut self.band,
            &mut self.hard,
        ] {
            plane.clear();
            plane.resize(len, 0.0);
        }
        self.guided.resize(len);
    }
}

/// Average `frame` down by `k` into `rgb`, and its seeds into `small`. A cell
/// is foreground when one of its pixels is and none is background, and the
/// same the other way; a cell holding both is left for the solve.
fn shrink(frame: FrameRgb<'_>, seeds: &Seeds, k: u32, rgb: &mut Vec<f32>, small: &mut Seeds) {
    let (w, h) = (frame.width() as usize, frame.height() as usize);
    let (sw, sh) = (small.width() as usize, small.height() as usize);
    let k = k as usize;
    rgb.clear();
    rgb.resize(sw * sh * 3, 0.0);
    small.clear();
    for sy in 0..sh {
        for sx in 0..sw {
            let (mut sum, mut count) = ([0.0f32; 3], 0.0f32);
            let (mut fg, mut bg) = (false, false);
            for y in sy * k..((sy + 1) * k).min(h) {
                for x in sx * k..((sx + 1) * k).min(w) {
                    let i = y * w + x;
                    let p = frame.px(i);
                    sum[0] += p[0];
                    sum[1] += p[1];
                    sum[2] += p[2];
                    count += 1.0;
                    match seeds.at(i) {
                        Seed::Foreground => fg = true,
                        Seed::Background => bg = true,
                        Seed::None => {}
                    }
                }
            }
            let c = sy * sw + sx;
            if let Some(slot) = rgb.get_mut(c * 3..c * 3 + 3) {
                for (s, v) in slot.iter_mut().zip(sum) {
                    *s = v / count.max(1.0);
                }
            }
            match (fg, bg) {
                (true, false) => small.set(c, Seed::Foreground),
                (false, true) => small.set(c, Seed::Background),
                _ => {}
            }
        }
    }
}

/// Scale the small answer back up to `w × h`, bilinear at pixel centres, with
/// every seeded pixel kept to its own claim.
fn grow(small: &[f32], (sw, sh): (u32, u32), k: u32, seeds: &Seeds, out: &mut [f32]) {
    let (sw, sh) = (sw as usize, sh as usize);
    let (w, h) = (seeds.width() as usize, seeds.height() as usize);
    let k = k as f32;
    let at = |x: usize, y: usize| small.get(y * sw + x).copied().unwrap_or(0.5);
    // Where a pixel centre falls between the small cells' centres.
    let place = |p: usize, side: usize| {
        let s = ((p as f32 + 0.5) / k - 0.5).clamp(0.0, (side - 1) as f32);
        let lo = s.floor() as usize;
        (lo, (lo + 1).min(side - 1), s - lo as f32)
    };
    for y in 0..h {
        let (y0, y1, ty) = place(y, sh);
        for x in 0..w {
            let i = y * w + x;
            let value = match seeds.at(i) {
                Seed::Foreground => 1.0,
                Seed::Background => 0.0,
                Seed::None => {
                    let (x0, x1, tx) = place(x, sw);
                    let top = at(x0, y0) + (at(x1, y0) - at(x0, y0)) * tx;
                    let bottom = at(x0, y1) + (at(x1, y1) - at(x0, y1)) * tx;
                    top + (bottom - top) * ty
                }
            };
            if let Some(slot) = out.get_mut(i) {
                *slot = value;
            }
        }
    }
}

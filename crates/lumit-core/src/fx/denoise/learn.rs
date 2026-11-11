//! Learn: how noisy the picture is, measured on its flattest patch.
//!
//! **In plain terms.** On a patch with nothing in it but noise, every band of
//! the a-trous transform is noise, so the spread of each band is the noise at
//! that size. The spread is taken as a median (1.4826 times the median
//! absolute deviation), so a stray speck or an edge clipping the corner does
//! not move it. Without a patch from the user, the frame is cut into blocks and
//! the one with the least coarse structure among the ordinarily noisy ones is
//! taken; how the noise of the blocks changes with their brightness sets the
//! tone rows.
//!
//! Pure and deterministic: the same picture gives the same answer on any
//! machine. Runs on whichever thread asked, and takes about a second on a 4K
//! frame, so never from a rebuild path.

use super::atrous::{dilation, level_of, LEVELS};
use super::space::{oetf, pixel_to_ycc, tone, CLEAR};

/// Why a measurement should not be trusted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LearnIssue {
    /// The patch is too small to measure the coarse bands.
    Small,
    /// The patch has picture in it, not only noise.
    NotFlat,
    /// Part of the patch is clipped to black or white, where there is no noise.
    Clipped,
    /// There is too little noise to measure.
    TooLittleNoise,
}

/// What Learn found: the row values to write, where it measured, and how far
/// to trust it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Learned {
    /// Luma noise, in eight-bit code values of encoded Y.
    pub luma_noise: f32,
    /// Chroma noise, in eight-bit code values of Cb and Cr together.
    pub chroma_noise: f32,
    /// Each luma band's noise as a per cent of Luma noise, finest first.
    pub luma_bands: [f32; 4],
    /// Each chroma band's noise as a per cent of Chroma noise.
    pub chroma_bands: [f32; 4],
    /// Shadows, Midtones and Highlights, per cent.
    pub tone: [f32; 3],
    /// Where it measured: centre x, centre y and side, in the picture's pixels.
    pub sample: [f32; 3],
    /// 0..1.
    pub quality: f32,
    /// Why the quality is poor, when it is.
    pub reason: Option<LearnIssue>,
}

impl Learned {
    /// The rows to write, by id, in their own units.
    #[must_use]
    pub fn rows(&self) -> Vec<(&'static str, f32)> {
        let l = self.luma_bands;
        let c = self.chroma_bands;
        vec![
            ("sample_x", self.sample[0]),
            ("sample_y", self.sample[1]),
            ("sample_size", self.sample[2]),
            ("luma_noise", self.luma_noise),
            ("chroma_noise", self.chroma_noise),
            ("luma_fine", l[0]),
            ("luma_medium", l[1]),
            ("luma_coarse", l[2]),
            ("luma_broad", l[3]),
            ("chroma_fine", c[0]),
            ("chroma_medium", c[1]),
            ("chroma_coarse", c[2]),
            ("chroma_broad", c[3]),
            ("shadows", self.tone[0]),
            ("midtones", self.tone[1]),
            ("highlights", self.tone[2]),
        ]
    }
}

/// A median absolute deviation's scale to a Gaussian sigma.
const MAD_SIGMA: f32 = 1.4826;

/// How far the deepest band reaches either side, in pixels: 2 x (1 + 2 + 4 + 8).
const REACH: u32 = 32;

/// The quality below which a reason is given.
const POOR: f32 = 0.6;

/// A sigma, from values whose middle is near zero but not quite.
fn robust_sigma(values: &mut [f32]) -> f32 {
    if values.is_empty() {
        return 0.0;
    }
    let mid = median(values);
    for v in values.iter_mut() {
        *v = (*v - mid).abs();
    }
    MAD_SIGMA * median(values)
}

/// The middle value, reordering `values`.
fn median(values: &mut [f32]) -> f32 {
    if values.is_empty() {
        return 0.0;
    }
    let mid = values.len() / 2;
    values.select_nth_unstable_by(mid, f32::total_cmp);
    values[mid]
}

/// The `q` quantile (0..1) of `values`, reordering them.
fn quantile(values: &mut [f32], q: f32) -> f32 {
    if values.is_empty() {
        return 0.0;
    }
    let at = ((values.len() - 1) as f32 * q).round() as usize;
    values.select_nth_unstable_by(at, f32::total_cmp);
    values[at]
}

/// A rectangle of the frame: left, top, width, height.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Rect {
    x: u32,
    y: u32,
    w: u32,
    h: u32,
}

/// The four bands and the coarsest level of one region, Y'CbCr, every lane.
struct Bands {
    rect: Rect,
    /// `levels[0]` is the region itself, `levels[4]` the coarsest.
    levels: Vec<Vec<f32>>,
}

impl Bands {
    fn of(ycc: &[f32], frame_w: u32, rect: Rect) -> Self {
        let mut first = Vec::with_capacity((rect.w * rect.h * 3) as usize);
        for y in rect.y..rect.y + rect.h {
            for x in rect.x..rect.x + rect.w {
                let i = ((y * frame_w + x) * 4) as usize;
                first.extend_from_slice(&ycc[i..i + 3]);
            }
        }
        let mut levels = vec![first];
        for b in 0..LEVELS {
            let next = level_of::<3>(&levels[b], rect.w, rect.h, dilation(b));
            levels.push(next);
        }
        Self { rect, levels }
    }

    /// The values of `band` (or, with `None`, the picture less its coarsest
    /// level) in lane `c`, inside `inner` (frame coordinates).
    fn values(&self, band: Option<usize>, c: usize, inner: Rect) -> Vec<f32> {
        let (a, b) = match band {
            Some(b) => (&self.levels[b], &self.levels[b + 1]),
            None => (&self.levels[0], &self.levels[LEVELS]),
        };
        let mut out = Vec::with_capacity((inner.w * inner.h) as usize);
        for y in inner.y..inner.y + inner.h {
            for x in inner.x..inner.x + inner.w {
                let i = (((y - self.rect.y) * self.rect.w + (x - self.rect.x)) * 3) as usize + c;
                out.push(a[i] - b[i]);
            }
        }
        out
    }

    /// Mean squared gradient of lane `c` of level `l` inside `inner`.
    fn gradient_energy(&self, l: usize, c: usize, inner: Rect) -> f32 {
        let lv = &self.levels[l];
        let at = |x: u32, y: u32| {
            lv[(((y - self.rect.y) * self.rect.w + (x - self.rect.x)) * 3) as usize + c]
        };
        let (mut e, mut n) = (0.0f64, 0u32);
        for y in inner.y..inner.y + inner.h.saturating_sub(1) {
            for x in inner.x..inner.x + inner.w.saturating_sub(1) {
                let gx = at(x + 1, y) - at(x, y);
                let gy = at(x, y + 1) - at(x, y);
                e += f64::from(gx * gx + gy * gy);
                n += 1;
            }
        }
        if n == 0 {
            0.0
        } else {
            (e / f64::from(n)) as f32
        }
    }
}

/// `inner` grown by the transform's reach, inside the frame.
fn padded(inner: Rect, w: u32, h: u32) -> Rect {
    let x = inner.x.saturating_sub(REACH);
    let y = inner.y.saturating_sub(REACH);
    let right = (inner.x + inner.w + REACH).min(w);
    let bottom = (inner.y + inner.h + REACH).min(h);
    Rect {
        x,
        y,
        w: right - x,
        h: bottom - y,
    }
}

/// Whether an encoded value is within one code value of black or white.
fn clips(v: f32) -> bool {
    !(1.0 / 255.0..=254.0 / 255.0).contains(&v)
}

/// What one block of the frame looks like to the search.
#[derive(Clone, Copy)]
struct Block {
    rect: Rect,
    fine: f32,
    structure: f32,
    clipped: f32,
    luma: f32,
}

/// The structure score of a region: the coarse level's gradient energy over
/// the third band's noise, so it reads the same for fine and coarse grain.
fn structure(bands: &Bands, inner: Rect) -> f32 {
    let s3 = robust_sigma(&mut bands.values(Some(2), 0, inner));
    bands.gradient_energy(3, 0, inner) / (s3 * s3).max(1e-12)
}

/// What the structure score of pure noise comes to (measured on white and on
/// blurred noise; both land near it). Twice this is still read as flat.
const NOISE_STRUCTURE: f32 = 0.6;

/// Measure the noise of `rgba_linear` (premultiplied linear RGBA, `w * h`),
/// on the patch `sample` (centre x, centre y, side, in its pixels) or, with
/// none, on the flattest patch the search finds.
#[must_use]
pub fn learn(rgba_linear: &[f32], w: u32, h: u32, sample: Option<[f32; 3]>) -> Learned {
    let unusable = Learned {
        luma_noise: 0.0,
        chroma_noise: 0.0,
        luma_bands: [100.0; 4],
        chroma_bands: [100.0; 4],
        tone: [100.0; 3],
        sample: sample.unwrap_or([w as f32 * 0.5, h as f32 * 0.5, 0.0]),
        quality: 0.0,
        reason: Some(LearnIssue::Small),
    };
    let n = (w as usize) * (h as usize);
    if w < 8 || h < 8 || rgba_linear.len() != n * 4 {
        return unusable;
    }
    let ycc: Vec<f32> = rgba_linear
        .chunks_exact(4)
        .flat_map(|p| pixel_to_ycc([p[0], p[1], p[2], p[3]]))
        .collect();
    let clipped: Vec<bool> = rgba_linear
        .chunks_exact(4)
        .map(|p| {
            let k = if p[3] > CLEAR { 1.0 / p[3] } else { 1.0 };
            (0..3).any(|c| clips(oetf(p[c] * k)))
        })
        .collect();
    let blocks = survey(&ycc, &clipped, w, h);
    let inner = match sample {
        Some([cx, cy, side]) => {
            let side = side.round().clamp(1.0, w.min(h) as f32) as u32;
            let x = (cx - side as f32 * 0.5)
                .round()
                .clamp(0.0, (w - side) as f32) as u32;
            let y = (cy - side as f32 * 0.5)
                .round()
                .clamp(0.0, (h - side) as f32) as u32;
            Rect {
                x,
                y,
                w: side,
                h: side,
            }
        }
        None => match choose(&blocks) {
            Some(b) => b.rect,
            None => return unusable,
        },
    };
    measure(&ycc, &clipped, w, h, inner, &blocks)
}

/// Every block of the frame, scored. Blocks are 64 px a side every 32 px at
/// HD, scaled with the frame so 4K uses 128.
fn survey(ycc: &[f32], clipped: &[bool], w: u32, h: u32) -> Vec<Block> {
    let scale = ((w.max(h) as f32 / 1920.0).round() as u32).max(1);
    let side = (64 * scale).min(w.min(h));
    let stride = (side / 2).max(1);
    // The whole frame's luma, so the blocks' bands have no edges of their own.
    let mut luma = Vec::with_capacity((w * h) as usize);
    luma.extend(ycc.chunks_exact(4).map(|p| p[0]));
    let mut levels = vec![luma];
    for b in 0..3 {
        let next = level_of::<1>(&levels[b], w, h, dilation(b));
        levels.push(next);
    }
    let mut out = Vec::new();
    let mut by = 0;
    while by + side <= h {
        let mut bx = 0;
        while bx + side <= w {
            let rect = Rect {
                x: bx,
                y: by,
                w: side,
                h: side,
            };
            let band = |l: usize| {
                let mut v = Vec::with_capacity((side * side) as usize);
                for y in by..by + side {
                    for x in bx..bx + side {
                        let i = (y * w + x) as usize;
                        v.push(levels[l][i] - levels[l + 1][i]);
                    }
                }
                v
            };
            let fine = robust_sigma(&mut band(0));
            let s3 = robust_sigma(&mut band(2));
            let mut e = 0.0f64;
            let mut cut = 0u32;
            let mut sum = 0.0f64;
            for y in by..by + side {
                for x in bx..bx + side {
                    let i = (y * w + x) as usize;
                    sum += f64::from(ycc[i * 4]);
                    cut += u32::from(clipped[i]);
                    if x + 1 < bx + side && y + 1 < by + side {
                        let c3 = &levels[3];
                        let gx = c3[i + 1] - c3[i];
                        let gy = c3[i + w as usize] - c3[i];
                        e += f64::from(gx * gx + gy * gy);
                    }
                }
            }
            let cells = f64::from(side * side);
            let grads = f64::from((side - 1) * (side - 1)).max(1.0);
            out.push(Block {
                rect,
                fine,
                structure: (e / grads) as f32 / (s3 * s3).max(1e-12),
                clipped: cut as f32 / cells as f32,
                luma: (sum / cells) as f32,
            });
            bx += stride;
        }
        by += stride;
    }
    out
}

/// The block to measure: the least structure among the unclipped blocks
/// whose fine noise is between the 10th and 50th percentile.
fn choose(blocks: &[Block]) -> Option<Block> {
    let open: Vec<&Block> = blocks.iter().filter(|b| b.clipped <= 0.02).collect();
    let mut fines: Vec<f32> = open.iter().map(|b| b.fine).collect();
    let lo = quantile(&mut fines, 0.1);
    let hi = quantile(&mut fines, 0.5);
    open.into_iter()
        .filter(|b| b.fine >= lo && b.fine <= hi)
        .min_by(|a, b| a.structure.total_cmp(&b.structure))
        .copied()
}

/// The tone rows: per tone, the 20th percentile of the blocks' fine noise over
/// the sample's, put to 1 at the sample's own brightness.
fn tones(blocks: &[Block], fine: f32, luma: f32) -> [f32; 3] {
    let bin = |y: f32| {
        if y < 0.275 {
            0
        } else if y < 0.65 {
            1
        } else {
            2
        }
    };
    let mut found: [Option<f32>; 3] = [None; 3];
    for (t, slot) in found.iter_mut().enumerate() {
        let mut v: Vec<f32> = blocks
            .iter()
            .filter(|b| b.clipped <= 0.02 && bin(b.luma) == t)
            .map(|b| b.fine)
            .collect();
        if !v.is_empty() && fine > 0.0 {
            *slot = Some(quantile(&mut v, 0.2) / fine);
        }
    }
    // A tone with no blocks takes its nearest neighbour's.
    let near = |t: usize| -> f32 {
        let order: [usize; 3] = match t {
            0 => [0, 1, 2],
            1 => [1, 0, 2],
            _ => [2, 1, 0],
        };
        order.iter().find_map(|&o| found[o]).unwrap_or(1.0)
    };
    let raw = [near(0), near(1), near(2)];
    let at = tone(luma, raw).max(1e-6);
    raw.map(|r| (r / at).clamp(0.25, 4.0))
}

/// Measure the patch `inner`.
fn measure(
    ycc: &[f32],
    clipped: &[bool],
    w: u32,
    h: u32,
    inner: Rect,
    blocks: &[Block],
) -> Learned {
    let bands = Bands::of(ycc, w, padded(inner, w, h));
    let pixel: Vec<f32> = (0..3)
        .map(|c| robust_sigma(&mut bands.values(None, c, inner)))
        .collect();
    let band = |b: usize, c: usize| robust_sigma(&mut bands.values(Some(b), c, inner));
    let rms = |a: f32, b: f32| ((a * a + b * b) * 0.5).sqrt();
    let luma = pixel[0];
    let chroma = rms(pixel[1], pixel[2]);
    let share = |s: f32, of: f32| {
        if of > 0.0 {
            (100.0 * s / of).clamp(0.0, 400.0)
        } else {
            100.0
        }
    };
    let luma_bands = [0, 1, 2, 3].map(|b| share(band(b, 0), luma));
    let chroma_bands = [0, 1, 2, 3].map(|b| share(rms(band(b, 1), band(b, 2)), chroma));

    let mut mean_luma = 0.0f64;
    let mut cut = 0u32;
    for y in inner.y..inner.y + inner.h {
        for x in inner.x..inner.x + inner.w {
            let i = (y * w + x) as usize;
            mean_luma += f64::from(ycc[i * 4]);
            cut += u32::from(clipped[i]);
        }
    }
    let cells = (inner.w * inner.h).max(1) as f32;
    let mean_luma = (mean_luma / f64::from(cells)) as f32;
    let clipped_share = cut as f32 / cells;
    let tone = tones(blocks, band(0, 0), mean_luma).map(|t| t * 100.0);

    let side = inner.w.min(inner.h) as f32;
    let size = (0.3 + 0.7 * (side - 16.0) / 80.0).clamp(0.3, 1.0);
    let flatness = (2.0 * NOISE_STRUCTURE / structure(&bands, inner).max(1e-12)).min(1.0);
    let open = 1.0 - clipped_share;
    let enough = luma * 255.0 >= 0.2;
    let quality = if enough { size * flatness * open } else { 0.0 };
    let reason = if !enough {
        Some(LearnIssue::TooLittleNoise)
    } else if quality < POOR {
        [
            (size, LearnIssue::Small),
            (flatness, LearnIssue::NotFlat),
            (open, LearnIssue::Clipped),
        ]
        .into_iter()
        .min_by(|a, b| a.0.total_cmp(&b.0))
        .map(|(_, why)| why)
    } else {
        None
    };
    Learned {
        luma_noise: luma * 255.0,
        chroma_noise: chroma * 255.0,
        luma_bands,
        chroma_bands,
        tone,
        sample: [
            inner.x as f32 + inner.w as f32 * 0.5,
            inner.y as f32 + inner.h as f32 * 0.5,
            side,
        ],
        quality,
        reason,
    }
}

//! Motion to frames further away, made from the motion between neighbours.
//!
//! A frame's motion to the frame two along is its motion to the next frame,
//! then that frame's motion on from where the pixel landed. A reader of several
//! frames either side measures only the pairs next to each other and chains
//! them, so playing forward costs one new pair a frame however far it reads.
//!
//! The per-pixel passes share rows out across the cores, as the luma and
//! confidence passes do.

use crate::rows::rows;
use crate::{Bytes, Floats, FlowField, FlowSettings, Gray, Texel};

/// A dense field with a confidence for every pixel, laid out as the texture an
/// effect reads it from: `[u, v, conf, 0]` per pixel, row-major, `(u, v)` in
/// the field's own pixels as [`FlowField`] has them and `conf` the 0..1 trust
/// in that vector. Kept in that layout so it goes to the card as it is.
#[derive(Clone)]
pub struct Leg {
    pub w: usize,
    pub h: usize,
    pub texels: Vec<f32>,
}

impl Leg {
    /// `f` with its [`crate::confidence`] against `g`, the field measured the
    /// other way between the same two frames.
    #[must_use]
    pub fn measured(f: &FlowField, g: &FlowField) -> Self {
        Self::from_planes(&f.u, &f.v, &crate::confidence(f, g), f.w, f.h)
    }

    /// A leg from separate planes, each `w × h`. A plane that is short reads
    /// as no motion where it ends, and a short confidence as full trust, so an
    /// older caller degrades to the plain field.
    #[must_use]
    pub fn from_planes(u: &[f32], v: &[f32], conf: &[f32], w: usize, h: usize) -> Self {
        let mut texels = vec![0f32; w * h * 4];
        rows(&mut texels, w.max(1) * 4, |y, row| {
            for (x, t) in row.chunks_exact_mut(4).enumerate() {
                let i = y * w + x;
                t[0] = u.get(i).copied().unwrap_or(0.0);
                t[1] = v.get(i).copied().unwrap_or(0.0);
                t[2] = conf.get(i).copied().unwrap_or(1.0);
            }
        });
        Self { w, h, texels }
    }

    /// What it holds, for a cache's accounting.
    #[must_use]
    pub fn bytes(&self) -> usize {
        self.texels.len() * 4 + 32
    }

    /// The motion from this leg's first frame to the far frame of `next`,
    /// which starts where this one ends: each pixel follows this leg, then
    /// `next` from where it landed, sampled bilinear with the edges held as
    /// [`crate::confidence`] samples them. The trust is the product of the two
    /// along the way, so a pixel is only as sure as each step it took. A `next`
    /// of another size cannot be followed and comes back trusted nowhere,
    /// which leaves its frame out rather than misplacing it.
    #[must_use]
    pub fn then(&self, next: &Leg) -> Leg {
        let (w, h) = (self.w, self.h);
        let n = w * h * 4;
        if next.w != w || next.h != h || self.texels.len() != n || next.texels.len() != n {
            let mut out = self.clone();
            out.texels.chunks_exact_mut(4).for_each(|t| t[2] = 0.0);
            return out;
        }
        let mut texels = vec![0f32; n];
        rows(&mut texels, w.max(1) * 4, |y, row| {
            for (x, t) in row.chunks_exact_mut(4).enumerate() {
                let i = (y * w + x) * 4;
                let (fu, fv, fc) = (self.texels[i], self.texels[i + 1], self.texels[i + 2]);
                let [nu, nv, nc] = sample(&next.texels, w, h, x as f32 + fu, y as f32 + fv);
                t[0] = fu + nu;
                t[1] = fv + nv;
                t[2] = fc * nc;
            }
        });
        Leg { w, h, texels }
    }
}

/// The first three channels of `texels` (`w × h`, four to a pixel) at
/// `(x, y)`, bilinear with the edges held: the sampling [`crate::confidence`]
/// does, on all three at once. The caller has checked the length.
fn sample(texels: &[f32], w: usize, h: usize, x: f32, y: f32) -> [f32; 3] {
    let x = x.clamp(0.0, (w - 1) as f32);
    let y = y.clamp(0.0, (h - 1) as f32);
    let (x0, y0) = (x.floor() as usize, y.floor() as usize);
    let (x1, y1) = ((x0 + 1).min(w - 1), (y0 + 1).min(h - 1));
    let (fx, fy) = (x - x0 as f32, y - y0 as f32);
    let at = |x: usize, y: usize, c: usize| texels[(y * w + x) * 4 + c];
    let mut out = [0f32; 3];
    for (c, o) in out.iter_mut().enumerate() {
        let a = at(x0, y0, c) * (1.0 - fx) + at(x1, y0, c) * fx;
        let b = at(x0, y1, c) * (1.0 - fx) + at(x1, y1, c) * fx;
        *o = a * (1.0 - fy) + b * fy;
    }
    out
}

/// One frame's luma at the settings' working size, exactly as
/// [`crate::flow_grays`] (or [`crate::flow_grays_f32`], with `floats`) makes
/// each picture of a pair. A frame met again in the next pair need not be read
/// again.
#[must_use]
pub fn flow_gray(rgba: &[u8], w: usize, h: usize, floats: bool, set: &FlowSettings) -> Gray {
    if floats {
        gray_at::<Floats>(rgba, w, h, set)
    } else {
        gray_at::<Bytes>(rgba, w, h, set)
    }
}

fn gray_at<T: Texel>(rgba: &[u8], w: usize, h: usize, set: &FlowSettings) -> Gray {
    let mut g = crate::to_gray_as::<T>(rgba, w, h);
    let (ww, _) = set.working_size(w, h);
    while g.w > ww {
        g = crate::downsample(&g);
    }
    g
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The chained field is the sum of the two steps along the way, and the
    /// single-frame luma is the one a pair would have made: a luma kept from
    /// the last pair must not change the next one.
    #[test]
    fn a_chain_follows_each_step_and_a_frame_reads_as_in_a_pair() {
        let (w, h) = (16usize, 8usize);
        let leg = |u: f32, c: f32| {
            Leg::from_planes(&vec![u; w * h], &vec![0.5; w * h], &vec![c; w * h], w, h)
        };
        let both = leg(2.0, 0.5).then(&leg(1.0, 0.8));
        for t in both.texels.chunks_exact(4) {
            assert!((t[0] - 3.0).abs() < 1e-6 && (t[1] - 1.0).abs() < 1e-6);
            assert!((t[2] - 0.4).abs() < 1e-6);
        }

        let (fw, fh) = (64usize, 40usize);
        let a: Vec<u8> = (0..fw * fh * 4).map(|i| (i * 37 % 251) as u8).collect();
        let b: Vec<u8> = (0..fw * fh * 4).map(|i| (i * 11 % 241) as u8).collect();
        let set = FlowSettings {
            divisor: 2,
            ..FlowSettings::default()
        };
        let (ga, gb, _) = crate::flow_grays(&a, &b, fw, fh, &set);
        assert_eq!(flow_gray(&a, fw, fh, false, &set).data, ga.data);
        assert_eq!(flow_gray(&b, fw, fh, false, &set).data, gb.data);
    }
}

//! How long the sensor took to read a frame: the rolling shutter, measured
//! from what the analysis already found.
//!
//! # In plain terms
//!
//! Most phone and mirrorless cameras read the picture one row after another,
//! so a speck low in the frame is seen a little later than one near the top.
//! While the camera's motion holds steady that only skews every frame the same
//! way. When it changes speed, the late rows have moved on further than the
//! early ones, and a speck misses where the frame's slide, turn and growth put
//! it by an amount that grows with how far down the frame it is. Summed over
//! every step, that miss gives one number for the clip: the share of a frame
//! interval the read takes.
//!
//! Only each step's slide, turn and growth (its similarity) is read. Never its
//! four-corner fit: a skew row by row is a stretch the four-corner fit takes in
//! whole, so nothing would be left to measure. One number per axis is fitted
//! for the whole clip, by closed-form least squares, in a fixed loop order.

use lumit_core::stabilise::analysis::{Run, Step};

use super::StabiliseError;

/// Below this many usable steps the clip says too little, and nothing is
/// corrected.
const MIN_STEPS: usize = 12;

/// Below this many agreeing specks a step is left out of the fit.
const MIN_PAIRS: usize = 8;

/// The least squared predicted miss, in pixels of the raster the specks were
/// followed on, per speck per step: a camera that barely changes speed shows
/// no rolling shutter to measure.
const MIN_SIGNAL: f64 = 0.02;

/// A read shorter than this share of a frame interval is too little to be
/// worth moving pixels for.
const MIN_READOUT: f64 = 0.1;

/// The step's similarity as `(a, b, tx, ty)`, where it can be trusted.
fn usable(steps: &[Step], k: usize) -> Option<[f64; 4]> {
    steps
        .get(k)
        .filter(|s| s.trusted_similarity())
        .map(|s| s.similarity)
}

/// How far the similarity `s` moves the point `q`.
fn moved(s: [f64; 4], q: [f64; 2]) -> [f64; 2] {
    [
        s[0] * q[0] - s[1] * q[1] + s[2] - q[0],
        s[1] * q[0] + s[0] * q[1] + s[3] - q[1],
    ]
}

/// The sums one axis gathers over every step.
#[derive(Default, Clone, Copy)]
struct Sums {
    /// Σ b⊥·e⊥: the predicted miss against the measured one.
    be: f64,
    /// Σ b⊥·b⊥: the predicted miss against itself.
    bb: f64,
}

/// What one step adds to an axis: the predicted misses `b` and the measured
/// ones `e`, each with the part the step's own similarity can take in removed,
/// since that fit absorbed it from the measurement already. The four
/// similarity fields about the specks' centroid are orthogonal, so the removal
/// is four projections taken one at a time.
fn step_sums(pts: &[[f64; 2]], b: &[[f64; 2]], e: &[[f64; 2]]) -> Sums {
    let n = pts.len() as f64;
    let (sx, sy) = pts
        .iter()
        .fold((0.0, 0.0), |(x, y), p| (x + p[0], y + p[1]));
    let (cx, cy) = (sx / n, sy / n);
    // Dot products with the fields (1, 0), (0, 1), (x, y) and (−y, x).
    let (mut bg, mut eg) = ([0.0f64; 4], [0.0f64; 4]);
    let (mut be, mut bb, mut r2) = (0.0, 0.0, 0.0);
    for ((p, bi), ei) in pts.iter().zip(b).zip(e) {
        let (x, y) = (p[0] - cx, p[1] - cy);
        let fields = [[1.0, 0.0], [0.0, 1.0], [x, y], [-y, x]];
        for (k, f) in fields.iter().enumerate() {
            bg[k] += bi[0] * f[0] + bi[1] * f[1];
            eg[k] += ei[0] * f[0] + ei[1] * f[1];
        }
        be += bi[0] * ei[0] + bi[1] * ei[1];
        bb += bi[0] * bi[0] + bi[1] * bi[1];
        r2 += x * x + y * y;
    }
    let norms = [n, n, r2, r2];
    for k in 0..4 {
        if norms[k] > 1e-12 {
            be -= bg[k] * eg[k] / norms[k];
            bb -= bg[k] * bg[k] / norms[k];
        }
    }
    Sums {
        be,
        bb: bb.max(0.0),
    }
}

/// The share of a frame interval the sensor took to read a frame, along x and
/// along y, from the analysis's steps and specks on a `size` raster, `scale`
/// of its pixels to one the specks were followed on. Positive
/// for a read from the top (or the left), negative for one the other way. At
/// most one axis is non-zero, and both are zero when the clip shows too little
/// to say.
///
/// While frame `t` is read the picture moves with the mean of the steps either
/// side of it, so a speck the step out of `t` carries to `t + 1` misses that
/// step's similarity by the read time times its place along the axis times the
/// change of motion across the step, `(D(t + 1) − D(t − 1)) / 2`.
///
/// # Errors
///
/// [`StabiliseError::Cancelled`] when `stop` said so, checked between steps.
pub fn readout(
    steps: &[Step],
    runs: &[Run],
    size: [f64; 2],
    scale: f64,
    stop: &dyn Fn() -> bool,
) -> Result<[f32; 2], StabiliseError> {
    if size[0] <= 0.0 || size[1] <= 0.0 || steps.len() < 3 {
        return Ok([0.0; 2]);
    }
    // Runs by the frame they start on, so each step only looks at the specks
    // alive across it.
    let mut order: Vec<(u32, usize)> = runs.iter().enumerate().map(|(i, r)| (r.first, i)).collect();
    order.sort_unstable();
    let mut next = 0usize;
    let mut alive: Vec<&Run> = Vec::new();
    let (mut pts, mut e, mut b) = (Vec::new(), Vec::new(), Vec::new());
    let mut a = Vec::new();
    let mut sums = [Sums::default(); 2];
    let (mut used, mut pairs) = (0usize, 0usize);
    for t in 1..steps.len() - 1 {
        if stop() {
            return Err(StabiliseError::Cancelled);
        }
        let f = t as u32;
        while let Some(&(first, i)) = order.get(next) {
            if first > f {
                break;
            }
            if let Some(r) = runs.get(i) {
                alive.push(r);
            }
            next += 1;
        }
        alive.retain(|r| r.last() > f);
        let (Some(before), Some(step), Some(after)) =
            (usable(steps, t - 1), usable(steps, t), usable(steps, t + 1))
        else {
            continue;
        };
        pts.clear();
        e.clear();
        a.clear();
        for r in alive.iter().filter(|r| r.agreed_at(f)) {
            let (Some(p), Some(q)) = (r.at(f), r.at(f + 1)) else {
                continue;
            };
            let p = [f64::from(p[0]), f64::from(p[1])];
            let to = moved(step, p);
            pts.push(p);
            e.push([
                f64::from(q[0]) - p[0] - to[0],
                f64::from(q[1]) - p[1] - to[1],
            ]);
            let (d0, d1) = (moved(before, p), moved(after, p));
            a.push([0.5 * (d1[0] - d0[0]), 0.5 * (d1[1] - d0[1])]);
        }
        if pts.len() < MIN_PAIRS {
            continue;
        }
        for (axis, sum) in sums.iter_mut().enumerate() {
            b.clear();
            b.extend(pts.iter().zip(&a).map(|(p, ai)| {
                let u = p[axis] / size[axis] - 0.5;
                [u * ai[0], u * ai[1]]
            }));
            let s = step_sums(&pts, &b, &e);
            sum.be += s.be;
            sum.bb += s.bb;
        }
        used += 1;
        pairs += pts.len();
    }
    if used < MIN_STEPS {
        return Ok([0.0; 2]);
    }
    // The axis whose fit explains more of the misses; the other is left at
    // nought.
    let signal = MIN_SIGNAL * scale * scale;
    let explained = |s: &Sums| {
        if s.bb > signal * pairs as f64 {
            s.be * s.be / s.bb
        } else {
            0.0
        }
    };
    let axis = usize::from(explained(&sums[1]) >= explained(&sums[0]));
    let mut out = [0.0f32; 2];
    if let Some(s) = sums.get(axis).filter(|s| explained(s) > 0.0) {
        let r = s.be / s.bb;
        if r.abs() >= MIN_READOUT {
            if let Some(o) = out.get_mut(axis) {
                *o = r.clamp(-1.0, 1.0) as f32;
            }
        }
    }
    Ok(out)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use lumit_core::stabilise::analysis::pack_agreed;
    use lumit_core::stabilise::motion::fit_similarity;

    /// The camera's similarity on frame `t`, taking a scene point to where a
    /// global shutter would see it: a jittery slide and a little turn.
    fn camera(t: usize) -> [f64; 4] {
        let t = t as f64;
        let turn = 0.004 * (0.9 * t).sin() + 0.002 * (2.3 * t + 0.5).cos();
        let grow = 1.0 + 0.002 * (1.3 * t).sin();
        let dx = 9.0 * (1.7 * t).sin() + 5.0 * (3.1 * t + 1.0).sin() + 0.8 * t;
        let dy = 7.0 * (2.1 * t + 0.4).cos() + 4.0 * (3.7 * t).sin();
        [grow * turn.cos(), grow * turn.sin(), dx, dy]
    }

    fn apply(s: [f64; 4], q: [f64; 2]) -> [f64; 2] {
        [
            s[0] * q[0] - s[1] * q[1] + s[2],
            s[1] * q[0] + s[0] * q[1] + s[3],
        ]
    }

    /// The step from frame `t` to `t + 1`: `C(t + 1) · C(t)⁻¹`.
    fn step_between(c0: [f64; 4], c1: [f64; 4]) -> [f64; 4] {
        let k = c0[0] * c0[0] + c0[1] * c0[1];
        let (ia, ib) = (c0[0] / k, -c0[1] / k);
        let it = [-(ia * c0[2] - ib * c0[3]), -(ib * c0[2] + ia * c0[3])];
        let a = c1[0] * ia - c1[1] * ib;
        let b = c1[1] * ia + c1[0] * ib;
        let o = apply(c1, it);
        [a, b, o[0], o[1]]
    }

    /// A clip of static scene points seen through `camera`, each read `r` of
    /// a frame interval along `axis`, and the steps fitted to what was seen.
    fn clip(r: f64, axis: usize) -> (Vec<Step>, Vec<Run>) {
        let frames = 72;
        let size = [1280.0, 720.0];
        let truth: Vec<[f64; 4]> = (0..frames - 1)
            .map(|t| step_between(camera(t), camera(t + 1)))
            .collect();
        let shift = |s: [f64; 4], q: [f64; 2]| moved(s, q);
        let scene: Vec<[f64; 2]> = (0..320)
            .map(|i| {
                let (gx, gy) = ((i % 20) as f64, (i / 20) as f64);
                [
                    120.0 + gx * 52.0 + 7.0 * (i as f64 * 1.3).sin(),
                    90.0 + gy * 34.0 + 5.0 * (i as f64 * 0.7).cos(),
                ]
            })
            .collect();
        let seen: Vec<Vec<[f64; 2]>> = scene
            .iter()
            .map(|x| {
                (0..frames)
                    .map(|t| {
                        let q = apply(camera(t), *x);
                        // The motion while the frame is read: the mean of
                        // the steps either side.
                        let v = |at: [f64; 2]| {
                            let parts: Vec<[f64; 2]> = [t.checked_sub(1), Some(t)]
                                .into_iter()
                                .flatten()
                                .filter_map(|k| truth.get(k))
                                .map(|s| shift(*s, at))
                                .collect();
                            let n = parts.len() as f64;
                            parts
                                .iter()
                                .fold([0.0, 0.0], |m, d| [m[0] + d[0] / n, m[1] + d[1] / n])
                        };
                        let vq = v(q);
                        let mut out = q;
                        for _ in 0..6 {
                            let rho = r * (out[axis] / size[axis] - 0.5);
                            out = [q[0] + rho * vq[0], q[1] + rho * vq[1]];
                        }
                        out
                    })
                    .collect()
            })
            .collect();
        let steps = (0..frames - 1)
            .map(|t| {
                let pairs: Vec<_> = seen.iter().map(|s| (s[t], s[t + 1])).collect();
                let (sim, _) = fit_similarity(&pairs, 2.0).unwrap();
                Step {
                    similarity: sim,
                    homography: [1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0],
                    kept: 320,
                    kept_similarity: 320,
                    seen: 320,
                    cut: false,
                }
            })
            .collect();
        let runs = seen
            .iter()
            .enumerate()
            .map(|(i, s)| Run {
                id: i as u32,
                first: 0,
                points: s.iter().map(|p| [p[0] as f32, p[1] as f32]).collect(),
                agreed: pack_agreed(&vec![true; frames - 1]),
            })
            .collect();
        (steps, runs)
    }

    #[test]
    fn the_read_time_comes_back_on_its_own_axis() {
        let size = [1280.0, 720.0];
        let (steps, runs) = clip(0.6, 1);
        let got = readout(&steps, &runs, size, 1.0, &|| false).unwrap();
        assert!((got[1] - 0.6).abs() < 0.1 && got[0] == 0.0, "down: {got:?}");
        let (steps, runs) = clip(0.6, 0);
        let got = readout(&steps, &runs, size, 1.0, &|| false).unwrap();
        assert!(
            (got[0] - 0.6).abs() < 0.1 && got[1] == 0.0,
            "across: {got:?}"
        );
        let (steps, runs) = clip(0.0, 1);
        let got = readout(&steps, &runs, size, 1.0, &|| false).unwrap();
        assert_eq!(got, [0.0, 0.0], "a global shutter");
    }
}

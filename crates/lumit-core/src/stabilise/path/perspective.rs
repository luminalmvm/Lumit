//! The path of the four-corner stretch.

use super::{
    ends, finite, hops, motions, place, placing, reach, segments, wanted, weighted, Kernel, Placed,
    Pose, Smoothing, UNMOVED,
};
use crate::stabilise::analysis::Step;
use crate::stabilise::motion::{apply, four_point, invert, mul, Mat3, IDENTITY};

/// How many frames in a row share one anchor in the perspective path.
const BLOCK: usize = 32;

/// The four-corner path is the similarity path with the rest of the
/// four-corner motion on top. For frame `t` and a frame `k` round it, the
/// four-corner motion chained from `t` to `k`, `H`, is split into the
/// similarity the similarity path chains for the pair, `S`, and what is
/// left, `R = S⁻¹·H`, which stays close to no motion however far the camera
/// turns or grows. Where each `R` takes the frame's corners is safe to
/// average; whole pictures turned tens of degrees apart are not, as their
/// corners average to a shrunken picture. `R` acts on frame `t`'s own pixels
/// before `S` carries them to frame `k`, so the smooth camera takes frame `t`
/// by the averaged `R̄` first and then by the inverse of the similarity path's
/// pose `S̄`, and output pixel `y` shows input pixel `R̄⁻¹(S̄(y))`.
///
/// Both are read through an anchor `a` a few frames away, `R = S(a → t)·
/// S(k → a)·H(a → k)·H(t → a)`, and `S̄` from the frames placed in `a` and
/// brought to `t` by `S(a → t)`, so the chains out to the frames round a
/// block of frames are made once for the block rather than once a frame.
pub(super) fn perspective_path(
    steps: &[Step],
    frames: usize,
    s: &Smoothing<'_>,
    c: [f64; 2],
    corners: [[f64; 2]; 4],
) -> Vec<Mat3> {
    let seg = segments(steps, frames);
    let end = ends(&seg);
    let (hops, turn) = hops(steps, frames, false, c, &seg);
    let fwd: Vec<Mat3> = motions(steps, frames, true, c)
        .into_iter()
        .map(|m| m.unwrap_or(IDENTITY))
        .collect();
    let back: Vec<Mat3> = fwd.iter().map(|m| invert(m).unwrap_or(IDENTITY)).collect();

    // No motion: the warp from the held frame to each frame, chained out from
    // it in both directions within its shot. Another shot holds its own first
    // frame, as the similarity path does.
    let locked: Option<Vec<Mat3>> = s.lock.map(|r| {
        let r = (r as usize).min(frames - 1);
        let anchor = |t: usize| if seg[t] == seg[r] { r } else { seg[t] };
        let mut to = vec![IDENTITY; frames];
        for t in 1..frames {
            if t > anchor(t) {
                to[t] = normalise(mul(&fwd[t - 1], &to[t - 1]));
            }
        }
        for t in (0..frames.saturating_sub(1)).rev() {
            if t < anchor(t) {
                to[t] = normalise(mul(&back[t], &to[t + 1]));
            }
        }
        to
    });

    let sigma = |t: usize| s.sigma.get(t).copied().unwrap_or(0.0);
    let mut kernel = Kernel::new();
    // The block's anchor, the first frame it reaches, where the frames from
    // there are placed in the anchor, the rest of the motion from the anchor
    // to each, and the four-corner motion from the anchor to this frame.
    let (mut a, mut base) = (0usize, 0usize);
    let mut placed: Vec<Placed> = Vec::new();
    let mut rest: Vec<Mat3> = Vec::new();
    let mut moved: Vec<[f64; 12]> = Vec::new();
    let mut to_t = IDENTITY;
    let mut out = Vec::with_capacity(frames);
    for t in 0..frames {
        // Where this frame's corners should be, and the smooth similarity
        // to follow after that.
        let (target, pose) = if let Some(to) = &locked {
            // M(t, ref) = to(ref → t)⁻¹.
            let m = invert(&to[t]).unwrap_or(IDENTITY);
            (corners.map(|p| apply(&m, p).unwrap_or(p)), None)
        } else {
            let span = (seg[t], end[t]);
            if t == span.0 || t - a >= BLOCK {
                a = t;
                let most = (t..(t + BLOCK).min(span.1 + 1))
                    .map(|k| reach(sigma(k)))
                    .max()
                    .unwrap_or(0);
                base = a.saturating_sub(most).max(span.0);
                let top = (a + BLOCK - 1 + most).min(span.1);
                place(&hops, &turn, (base, top), a, &mut placed);
                anchor(&fwd, &back, &placed, (base, top), a, c, &mut rest);
                to_t = IDENTITY;
            } else {
                to_t = normalise(mul(fwd.get(t - 1).unwrap_or(&IDENTITY), &to_t));
            }
            let r = kernel.set(sigma(t));
            let (lo, hi) = (t.saturating_sub(r).max(span.0), (t + r).min(span.1));
            // Each frame round this one as the anchor sees it: its growth and
            // turn, where it has the centre, and where the rest of the motion
            // to it takes this frame's corners. Averaged, and brought back.
            let from = invert(&to_t).unwrap_or(IDENTITY);
            let q = corners.map(|p| apply(&from, p).unwrap_or(p));
            let q = [
                q[0][0], q[1][0], q[2][0], q[3][0], q[0][1], q[1][1], q[2][1], q[3][1],
            ];
            let (from_lo, to_hi) = (lo.saturating_sub(base), hi.saturating_sub(base));
            let round = rest.get(from_lo..=to_hi).unwrap_or(&[]);
            let places = placed.get(from_lo..=to_hi).unwrap_or(&[]);
            moved.clear();
            moved.extend(round.iter().zip(places).map(|(m, at)| {
                let p = carry(m, &q);
                [
                    at[0], at[1], at[2], at[3], p[0], p[1], p[2], p[3], p[4], p[5], p[6], p[7],
                ]
            }));
            let here = placed
                .get(t.saturating_sub(base))
                .copied()
                .unwrap_or(UNMOVED);
            let home = invert(&placing(&here, c)).unwrap_or(IDENTITY);
            let mean = if r == 0 {
                None
            } else {
                weighted(&moved, |v| *v, span, t, (r, &kernel.w))
            };
            let (pose, target) = match mean {
                Some(v) => {
                    let p = apply(&home, [c[0] + v[2], c[1] + v[3]]).unwrap_or(c);
                    let pose = Pose {
                        ls: v[0] - here[0],
                        th: v[1] - here[1],
                        p: [p[0] - c[0], p[1] - c[1]],
                    };
                    // A frame round it with a corner past the horizon leaves
                    // only the similarity to follow.
                    let avg = [[v[4], v[8]], [v[5], v[9]], [v[6], v[10]], [v[7], v[11]]];
                    let target = if avg.iter().flatten().all(|x| x.is_finite()) {
                        avg.map(|p| apply(&home, p).unwrap_or(p))
                    } else {
                        corners
                    };
                    (pose, target)
                }
                None => (Pose::STILL, corners),
            };
            (target, Some(wanted(pose, s)))
        };
        let mut want = target;
        if s.keep_scale {
            want = keep_area(corners, want);
        }
        // P̃ takes this frame's corners to where they should be; the output
        // shows input pixel P̃⁻¹(y), after the smooth similarity if there is one.
        let w = four_point(corners, want)
            .and_then(|p| invert(&p))
            .unwrap_or(IDENTITY);
        out.push(match pose {
            Some(p) => finite(mul(&w, &p.matrix(c))),
            None => w,
        });
    }
    out
}

/// Where `m` takes the four points whose x and y are `q`, as the four x
/// then the four y: not finite for a point past its horizon.
fn carry(m: &Mat3, q: &[f64; 8]) -> [f64; 8] {
    let mut out = [0.0f64; 8];
    let (xs, ys) = out.split_at_mut(4);
    let (qx, qy) = q.split_at(4);
    for (((x, y), px), py) in xs.iter_mut().zip(ys.iter_mut()).zip(qx).zip(qy) {
        let k = 1.0 / (m[2][0] * px + m[2][1] * py + m[2][2]);
        *x = (m[0][0] * px + m[0][1] * py + m[0][2]) * k;
        *y = (m[1][0] * px + m[1][1] * py + m[1][2]) * k;
    }
    out
}

/// The rest of the motion from anchor frame `a` to each frame `base..=top`
/// of its shot, as frame `a` sees it, into `rest`: the four-corner motion
/// chained from `a` to the frame, brought back by the similarity path's
/// placing of that frame in `a`.
fn anchor(
    fwd: &[Mat3],
    back: &[Mat3],
    placed: &[Placed],
    (base, top): (usize, usize),
    a: usize,
    c: [f64; 2],
    rest: &mut Vec<Mat3>,
) {
    rest.clear();
    rest.resize(placed.len(), IDENTITY);
    let i = a - base;
    let mut m = IDENTITY;
    let ahead = fwd.get(a..top).unwrap_or(&[]).iter();
    for (f, (at, slot)) in ahead.zip(placed.iter().zip(rest.iter_mut()).skip(i + 1)) {
        m = normalise(mul(f, &m));
        *slot = mul(&placing(at, c), &m);
    }
    let mut m = IDENTITY;
    let behind = back.get(base..a).unwrap_or(&[]).iter().rev();
    for (b, (at, slot)) in behind.zip(placed.iter().zip(rest.iter_mut()).take(i).rev()) {
        m = normalise(mul(b, &m));
        *slot = mul(&placing(at, c), &m);
    }
}

/// A homography scaled so its last entry is one.
fn normalise(m: Mat3) -> Mat3 {
    let d = m[2][2];
    if !d.is_finite() || d.abs() < 1e-12 {
        return m;
    }
    let mut out = m;
    for row in &mut out {
        for v in row.iter_mut() {
            *v /= d;
        }
    }
    out
}

/// `want` grown or shrunk about its own centre to the size of `corners`, so the
/// smooth camera turns and leans but does not zoom.
fn keep_area(corners: [[f64; 2]; 4], want: [[f64; 2]; 4]) -> [[f64; 2]; 4] {
    let centre = |q: [[f64; 2]; 4]| {
        [
            q.iter().map(|p| p[0]).sum::<f64>() / 4.0,
            q.iter().map(|p| p[1]).sum::<f64>() / 4.0,
        ]
    };
    let spread = |q: [[f64; 2]; 4], c: [f64; 2]| {
        q.iter()
            .map(|p| (p[0] - c[0]).hypot(p[1] - c[1]))
            .sum::<f64>()
            / 4.0
    };
    let (c0, c1) = (centre(corners), centre(want));
    let (s0, s1) = (spread(corners, c0), spread(want, c1));
    if s1 <= 1e-9 {
        return want;
    }
    let k = s0 / s1;
    want.map(|p| [c1[0] + (p[0] - c1[0]) * k, c1[1] + (p[1] - c1[1]) * k])
}

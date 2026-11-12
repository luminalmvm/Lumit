//! The path of a slide, or of a slide, a turn and a growth.

use super::{
    cmul, ends, finite, hops, place, segments, wanted, weighted, Hop, Kernel, Model, Placed, Pose,
    Smoothing,
};
use crate::stabilise::analysis::Step;
use crate::stabilise::motion::Mat3;

/// The path is smoothed as each frame sees it: the frames round frame `t`
/// are placed by chaining their steps outward from `t`, never through a frame
/// far away. Averaging in one fixed frame per shot gives the same answer in
/// exact sums, but a shot that grows a long way (a camera moving forward
/// reads as a steady zoom) loses every digit there.
pub(super) fn similarity_path(
    steps: &[Step],
    frames: usize,
    model: Model,
    s: &Smoothing<'_>,
    c: [f64; 2],
) -> Vec<Mat3> {
    let seg = segments(steps, frames);
    let end = ends(&seg);
    let (hops, turn) = hops(steps, frames, model == Model::Position, c, &seg);

    // No motion: where the held frame's centre is on each frame, chained out
    // from it within its shot. Another shot holds its own first frame.
    let held: Option<(Vec<usize>, Vec<[f64; 2]>)> = s.lock.map(|r| {
        let r = r as usize;
        let anchor: Vec<usize> = (0..frames)
            .map(|t| {
                if r < frames && seg[r] == seg[t] {
                    r
                } else {
                    seg[t]
                }
            })
            .collect();
        let mut x = vec![[0.0f64; 2]; frames];
        for t in 1..frames {
            if t > anchor[t] {
                let h = hops[t];
                let sx = cmul(h.s, x[t - 1]);
                x[t] = [sx[0] + h.e[0], sx[1] + h.e[1]];
            }
        }
        for t in (0..frames.saturating_sub(1)).rev() {
            if t < anchor[t] {
                let h = hops[t + 1];
                x[t] = cmul(h.g, [x[t + 1][0] - h.e[0], x[t + 1][1] - h.e[1]]);
            }
        }
        (anchor, x)
    });

    let mut kernel = Kernel::new();
    let mut near = Vec::new();
    let mut out = Vec::with_capacity(frames);
    for t in 0..frames {
        // The smooth camera as frame t sees it: its growth and turn from
        // frame t, and where it has the centre.
        let target = match &held {
            Some((anchor, x)) => {
                let a = anchor[t];
                Pose {
                    ls: turn[t][0] - turn[a][0],
                    th: turn[t][1] - turn[a][1],
                    p: x[t],
                }
            }
            None => {
                let r = kernel.set(s.sigma.get(t).copied().unwrap_or(0.0));
                smooth_pose(&hops, &turn, (seg[t], end[t]), t, (r, &kernel.w), &mut near)
            }
        };
        // Output pixel y shows the input pixel the smooth camera has there.
        out.push(finite(wanted(target, s).matrix(c)));
    }
    out
}

/// The Gaussian-weighted pose round frame `t` of the shot `first..=last`, as
/// frame `t` sees it, never reaching across a cut, and reflected past the
/// shot's ends. `near` is room for where the frames round `t` are placed.
fn smooth_pose(
    hops: &[Hop],
    turn: &[[f64; 2]],
    (first, last): (usize, usize),
    t: usize,
    (r, weights): (usize, &[f64]),
    near: &mut Vec<Placed>,
) -> Pose {
    if r == 0 {
        return Pose::STILL;
    }
    let (lo, hi) = (t.saturating_sub(r).max(first), (t + r).min(last));
    place(hops, turn, (lo, hi), t, near);
    weighted(
        near,
        |v| [v[0], v[1], v[2], v[3]],
        (first, last),
        t,
        (r, weights),
    )
    .map_or(Pose::STILL, |a| Pose {
        ls: a[0],
        th: a[1],
        p: [a[2], a[3]],
    })
}

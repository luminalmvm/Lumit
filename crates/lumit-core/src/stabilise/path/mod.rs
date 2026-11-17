//! The camera's path through a clip, smoothed, and what each frame has to be
//! moved by to follow the smooth path instead of the shaky one.
//!
//! In plain terms: chaining each frame's motion onto the last gives where the
//! camera pointed on every frame. A smooth version of that path is where it
//! should have pointed. Each frame is then moved by the difference, so the shot
//! looks as if it was filmed along the smooth path. "No motion" is the smoothest
//! path of all: the camera never moves from the reference frame.
//!
//! Everything comes out as a map from a pixel of the output to the pixel of the
//! input it shows, which is the form the warp kernel reads.

mod perspective;
mod similarity;

use crate::stabilise::analysis::Step;
use crate::stabilise::motion::{apply, from_rows, from_similarity, Mat3, IDENTITY};

/// What the frame's motion is fitted as.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Model {
    /// A slide.
    Position,
    /// A slide, a turn and a growth.
    Similarity,
    /// The four-corner stretch.
    Perspective,
}

/// How the path is smoothed, frame by frame.
pub struct Smoothing<'a> {
    /// The Gaussian's standard deviation in frames, one per analysed frame.
    /// Nought leaves the frame where it was.
    pub sigma: &'a [f64],
    /// No motion: lock every frame to this analysed frame instead of smoothing.
    pub lock: Option<u32>,
    /// Leave the picture's growth alone, so walking towards something does not
    /// read as a zoom to be undone.
    pub keep_scale: bool,
}

/// The centre-anchored form of a similarity: `z ↦ m·(z − c) + c + p`, with `m`
/// a complex scale and turn and `p` where the centre went.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Pose {
    /// ln |m|.
    ls: f64,
    /// arg m, unwrapped along the path.
    th: f64,
    p: [f64; 2],
}

impl Pose {
    const STILL: Pose = Pose {
        ls: 0.0,
        th: 0.0,
        p: [0.0, 0.0],
    };

    /// The pose as a matrix about centre `c`.
    fn matrix(&self, c: [f64; 2]) -> Mat3 {
        let s = self.ls.exp();
        let (a, b) = (s * self.th.cos(), s * self.th.sin());
        [
            [a, -b, c[0] + self.p[0] - a * c[0] + b * c[1]],
            [b, a, c[1] + self.p[1] - b * c[0] - a * c[1]],
            [0.0, 0.0, 1.0],
        ]
    }
}

/// Which analysed frames share a shot: the index of each one's first frame.
fn segments(steps: &[Step], frames: usize) -> Vec<usize> {
    let mut start = vec![0usize; frames];
    let mut s = 0usize;
    for (t, slot) in start.iter_mut().enumerate().skip(1) {
        if steps.get(t - 1).is_some_and(|st| st.cut) {
            s = t;
        }
        *slot = s;
    }
    start
}

/// The last analysed frame of each frame's shot, given each one's first.
fn ends(seg: &[usize]) -> Vec<usize> {
    let mut end = vec![0usize; seg.len()];
    let mut e = seg.len().saturating_sub(1);
    for t in (0..seg.len()).rev() {
        end[t] = e;
        if seg[t] == t {
            e = t.saturating_sub(1);
        }
    }
    end
}

/// Where the smoothing reads frame `k` of the shot `first..=last`: `k` itself
/// inside it, and past either end the frame as far inside it, reflected
/// through the end (`p(first − j) = 2·p(first) − p(first + j)`). A path that
/// keeps moving at the shot's ends then smooths to itself there, rather than
/// lagging the camera because the frames past the end are missing. `None`
/// inside the shot, else the end and the frame mirrored in it.
fn mirrored(k: i64, first: usize, last: usize) -> Option<(usize, usize)> {
    let (f, l) = (first as i64, last as i64);
    if k < f {
        Some((first, (2 * f - k).min(l) as usize))
    } else if k > l {
        Some((last, (2 * l - k).max(f) as usize))
    } else {
        None
    }
}

/// The smoothing's standard deviation in frames for a Smoothness of
/// `percent` at `fps`.
///
/// Nought is no smoothing at all. The curve rises gently through the low
/// settings, where a shake is a few frames long, and steeply past 100, where
/// a smooth path is seconds long: 50 is about two thirds of a second either
/// side, which leaves about the slow wander other editors leave at their 50,
/// 100 about two seconds and 300 most of a minute.
#[must_use]
pub fn sigma_frames(percent: f64, fps: f64) -> f64 {
    let s = (percent / 100.0).max(0.0);
    let seconds = 0.6 * ((1.5 * s).exp() - 1.0);
    if seconds.is_finite() {
        seconds * fps.max(1.0)
    } else {
        0.0
    }
}

/// The Gaussian weight of a frame `k` away, for `sigma` in frames.
fn gauss(k: f64, sigma: f64) -> f64 {
    (-(k * k) / (2.0 * sigma * sigma)).exp()
}

/// How far either side a Gaussian of `sigma` frames is worth reading.
fn reach(sigma: f64) -> usize {
    if sigma.is_finite() && sigma > 0.0 {
        (3.0 * sigma).ceil().min(100_000.0) as usize
    } else {
        0
    }
}

/// For each analysed frame, the map from an output pixel to the input pixel it
/// shows, before any zoom, in source pixels. `centre` is the frame's centre and
/// `corners` its four corners, upper left, upper right, lower left, lower right.
#[must_use]
pub fn corrections(
    steps: &[Step],
    frames: usize,
    model: Model,
    s: &Smoothing<'_>,
    centre: [f64; 2],
    corners: [[f64; 2]; 4],
) -> Vec<Mat3> {
    if frames == 0 {
        return Vec::new();
    }
    let mut out = match model {
        Model::Position | Model::Similarity => {
            similarity::similarity_path(steps, frames, model, s, centre)
        }
        Model::Perspective => perspective::perspective_path(steps, frames, s, centre, corners),
    };
    if s.lock.is_none() {
        fade(&mut out, steps, model == Model::Perspective, s.sigma);
    }
    out
}

/// Take each frame's correction only as far as the motion round it was
/// measured. The share of the steps the smoothing reads for the frame,
/// weighed as it weighs them, that were usable rather than carried over from
/// their neighbours, is squared and the correction taken that far. A carried
/// step is a guess, and the camera is lost exactly where it moves fastest, so
/// the guesses miss the most motion where they are most needed and their
/// errors add up along the path: where half a stretch is guessed, a quarter of
/// its correction is kept. Where every step was measured the corrections are
/// left exactly as they are.
fn fade(out: &mut [Mat3], steps: &[Step], perspective: bool, sigma: &[f64]) {
    let usable = |st: &Step| st.trusted_similarity() || (perspective && st.trusted());
    let seg = segments(steps, out.len());
    let end = ends(&seg);
    let mut kernel = Kernel::new();
    for (t, m) in out.iter_mut().enumerate() {
        let r = kernel.set(sigma.get(t).copied().unwrap_or(0.0));
        // The steps of the shot within reach, each weighed by its nearer
        // frame's distance from this one.
        let (lo, hi) = (t.saturating_sub(r).max(seg[t]), (t + r).min(end[t]));
        let (mut measured, mut all) = (0.0f64, 0.0f64);
        for k in lo..hi {
            let w = kernel
                .w
                .get(if k >= t { k - t } else { t - k - 1 })
                .copied()
                .unwrap_or(0.0);
            all += w;
            if steps.get(k).is_some_and(usable) {
                measured += w;
            }
        }
        if all > 0.0 && measured < all {
            let kept = (measured / all).powi(2);
            *m = std::array::from_fn(|i| {
                std::array::from_fn(|j| IDENTITY[i][j] + (m[i][j] - IDENTITY[i][j]) * kept)
            });
        }
    }
}

/// A step's slide, turn and growth about the frame's centre `c`: what it does
/// to the camera's pose.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Rate {
    /// ln of the growth.
    ls: f64,
    /// The turn.
    th: f64,
    /// Where the centre went.
    e: [f64; 2],
}

impl Rate {
    /// The slide, turn and growth of `m` about `c`: exact for a similarity,
    /// and its nearest for a four-corner fit.
    fn of(m: &Mat3, c: [f64; 2]) -> Rate {
        let (a, b) = (0.5 * (m[0][0] + m[1][1]), 0.5 * (m[1][0] - m[0][1]));
        let moved = apply(m, c).unwrap_or(c);
        Rate {
            ls: a.hypot(b).max(1e-9).ln(),
            th: b.atan2(a),
            e: [moved[0] - c[0], moved[1] - c[1]],
        }
    }

    /// The similarity doing this about `c`.
    fn matrix(&self, c: [f64; 2]) -> Mat3 {
        Pose {
            ls: self.ls,
            th: self.th,
            p: self.e,
        }
        .matrix(c)
    }

    /// `u` of the way from `self` to `other`.
    fn towards(&self, other: &Rate, u: f64) -> Rate {
        let mix = |a: f64, b: f64| a + (b - a) * u;
        Rate {
            ls: mix(self.ls, other.ls),
            th: mix(self.th, other.th),
            e: [mix(self.e[0], other.e[0]), mix(self.e[1], other.e[1])],
        }
    }
}

/// The motion the path follows across each step: the step's own where it is
/// usable (its four-corner fit for the perspective path where that is
/// trusted, and its similarity otherwise), and elsewhere the slide, turn and
/// growth of the usable steps either side within the shot, blended by where
/// the step falls between them. A step past the last usable one of its shot
/// carries that one's on. `None` for a cut, and for every step of a shot with
/// no usable step at all, which stands still.
fn motions(steps: &[Step], frames: usize, perspective: bool, c: [f64; 2]) -> Vec<Option<Mat3>> {
    let count = frames.saturating_sub(1);
    let own = |st: &Step| -> Option<Mat3> {
        if perspective && st.trusted() {
            Some(from_rows(st.homography))
        } else if st.trusted_similarity() {
            Some(from_similarity(st.similarity))
        } else {
            None
        }
    };
    let mut out: Vec<Option<Mat3>> = (0..count).map(|k| steps.get(k).and_then(own)).collect();
    // The nearest usable step before each one, within its shot.
    let mut before: Vec<Option<usize>> = vec![None; count];
    let mut last: Option<usize> = None;
    for k in 0..count {
        let st = steps.get(k);
        if st.is_none_or(|s| s.cut) {
            last = None;
            continue;
        }
        before[k] = last;
        if out[k].is_some() {
            last = Some(k);
        }
    }
    let mut next: Option<usize> = None;
    for k in (0..count).rev() {
        let st = steps.get(k);
        if st.is_none_or(|s| s.cut) {
            next = None;
            continue;
        }
        if out[k].is_some() {
            next = Some(k);
            continue;
        }
        let rate = |i: usize| out.get(i).copied().flatten().map(|m| Rate::of(&m, c));
        let carried = match (before[k], next) {
            (Some(p), Some(n)) => rate(p)
                .zip(rate(n))
                .map(|(a, b)| a.towards(&b, (k - p) as f64 / (n - p) as f64)),
            (Some(i), None) | (None, Some(i)) => rate(i),
            (None, None) => None,
        };
        out[k] = carried.map(|r| r.matrix(c));
    }
    out
}

/// The product of two complex numbers.
fn cmul(a: [f64; 2], b: [f64; 2]) -> [f64; 2] {
    [a[0] * b[0] - a[1] * b[1], a[0] * b[1] + a[1] * b[0]]
}

/// One frame's step from the frame before, about the centre: `z ↦ s·(z − c) + c + e`.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Hop {
    /// The complex growth and turn.
    s: [f64; 2],
    /// Its inverse.
    g: [f64; 2],
    /// Where the centre went.
    e: [f64; 2],
}

impl Hop {
    const STILL: Hop = Hop {
        s: [1.0, 0.0],
        g: [1.0, 0.0],
        e: [0.0, 0.0],
    };

    fn new(s: [f64; 2], e: [f64; 2]) -> Hop {
        let d = (s[0] * s[0] + s[1] * s[1]).max(1e-18);
        Hop {
            s,
            g: [s[0] / d, -s[1] / d],
            e,
        }
    }
}

/// The Gaussian's weights, worked out once for each Smoothness rather than
/// once for every frame it reaches.
struct Kernel {
    sigma: f64,
    w: Vec<f64>,
}

impl Kernel {
    fn new() -> Kernel {
        Kernel {
            sigma: f64::NAN,
            w: Vec::new(),
        }
    }

    /// The weights for `sigma`, nearest first, and how far they reach.
    fn set(&mut self, sigma: f64) -> usize {
        let r = reach(sigma);
        if sigma.to_bits() != self.sigma.to_bits() {
            self.sigma = sigma;
            self.w.clear();
            self.w.extend((0..=r).map(|k| gauss(k as f64, sigma)));
        }
        r
    }
}

/// Each frame's step from the one before as the similarity path takes it,
/// and the growth and turn since the start of its shot, as [ln growth, turn].
fn hops(
    steps: &[Step],
    frames: usize,
    slide_only: bool,
    c: [f64; 2],
    seg: &[usize],
) -> (Vec<Hop>, Vec<[f64; 2]>) {
    let moves = motions(steps, frames, false, c);
    let mut hops = vec![Hop::STILL; frames];
    let mut turn = vec![[0.0f64; 2]; frames];
    for t in 1..frames {
        if seg[t] == t {
            continue;
        }
        let m = moves.get(t - 1).copied().flatten().unwrap_or(IDENTITY);
        let moved = apply(&m, c).unwrap_or(c);
        let e = [moved[0] - c[0], moved[1] - c[1]];
        let (a, b) = if slide_only {
            (1.0, 0.0)
        } else {
            (m[0][0], m[1][0])
        };
        hops[t] = Hop::new([a, b], e);
        turn[t] = [
            turn[t - 1][0] + a.hypot(b).max(1e-9).ln(),
            turn[t - 1][1] + b.atan2(a),
        ];
    }
    (hops, turn)
}

/// How much of the smooth camera `target` frame `t` follows: Amount of it,
/// and none of its growth under Keep scale.
fn wanted(target: Pose, s: &Smoothing<'_>) -> Pose {
    Pose {
        ls: if s.keep_scale { 0.0 } else { target.ls },
        ..target
    }
}

/// `m`, or no correction at all where an entry is not finite.
fn finite(m: Mat3) -> Mat3 {
    if m.iter().flatten().all(|v| v.is_finite()) {
        m
    } else {
        IDENTITY
    }
}

/// Where the similarity path places a frame `k` round frame `t`, as frame
/// `t` sees it: `[ln growth, turn, x, y, m.re, m.im]`, with
/// `z ↦ m·(z − c) + c + (x, y)` taking frame `k`'s pixels to frame `t`'s.
/// The growth and turn are the unwrapped sums the path averages; `m` is the
/// same growth and turn as the chained product.
type Placed = [f64; 6];

/// A frame placed in itself.
const UNMOVED: Placed = [0.0, 0.0, 0.0, 0.0, 1.0, 0.0];

/// Frames `lo..=hi` of one shot placed in frame `t`, chained outward from
/// `t`, into `near`, from `lo` on.
fn place(
    hops: &[Hop],
    turn: &[[f64; 2]],
    (lo, hi): (usize, usize),
    t: usize,
    near: &mut Vec<Placed>,
) {
    let hop = |k: usize| hops.get(k).copied().unwrap_or(Hop::STILL);
    let here = turn.get(t).copied().unwrap_or([0.0, 0.0]);
    let since = |k: usize| {
        let tn = turn.get(k).copied().unwrap_or(here);
        [here[0] - tn[0], here[1] - tn[1]]
    };
    near.clear();
    near.resize(hi.saturating_sub(lo) + 1, UNMOVED);
    let (mut f, mut x) = ([1.0, 0.0], [0.0, 0.0]);
    for k in (t + 1)..=hi {
        let h = hop(k);
        f = cmul(f, h.g);
        let fe = cmul(f, h.e);
        x = [x[0] - fe[0], x[1] - fe[1]];
        let d = since(k);
        if let Some(slot) = near.get_mut(k - lo) {
            *slot = [d[0], d[1], x[0], x[1], f[0], f[1]];
        }
    }
    let (mut f, mut x) = ([1.0, 0.0], [0.0, 0.0]);
    for k in (lo..t).rev() {
        let h = hop(k + 1);
        let fe = cmul(f, h.e);
        x = [x[0] + fe[0], x[1] + fe[1]];
        f = cmul(f, h.s);
        let d = since(k);
        if let Some(slot) = near.get_mut(k - lo) {
            *slot = [d[0], d[1], x[0], x[1], f[0], f[1]];
        }
    }
}

/// The similarity `at` places a frame by, as a matrix about centre `c`.
fn placing(at: &Placed, c: [f64; 2]) -> Mat3 {
    let (a, b) = (at[4], at[5]);
    [
        [a, -b, c[0] + at[2] - a * c[0] + b * c[1]],
        [b, a, c[1] + at[3] - b * c[0] - a * c[1]],
        [0.0, 0.0, 1.0],
    ]
}

/// The Gaussian-weighted average round frame `t` of the shot `first..=last`
/// of what `of` reads from `vals`, one for each frame the window reaches
/// from the first. Past the shot's ends it reads the frame as far inside,
/// reflected through the end. `None` when nothing is weighed.
fn weighted<T, const N: usize>(
    vals: &[T],
    of: impl Fn(&T) -> [f64; N],
    (first, last): (usize, usize),
    t: usize,
    (r, weights): (usize, &[f64]),
) -> Option<[f64; N]> {
    let lo = t.saturating_sub(r).max(first);
    let i = t.saturating_sub(lo);
    let mut acc = [0.0f64; N];
    let mut wsum = 0.0;
    let mut add = |w: f64, v: [f64; N]| {
        for (s, v) in acc.iter_mut().zip(v) {
            *s += w * v;
        }
        wsum += w;
    };
    // Inside the shot, nearest first on either side.
    for (w, v) in weights.iter().zip(vals.iter().skip(i)) {
        add(*w, of(v));
    }
    for (w, v) in weights.iter().skip(1).zip(vals.iter().take(i).rev()) {
        add(*w, of(v));
    }
    // Past the shot's ends, reflected.
    let at = |j: usize| j.checked_sub(lo).and_then(|i| vals.get(i)).map(&of);
    let ti = t as i64;
    let past = ((ti - r as i64)..(first as i64)).chain((last as i64 + 1)..=(ti + r as i64));
    for k in past {
        let Some((edge, other)) = mirrored(k, first, last) else {
            continue;
        };
        if let (Some(a), Some(b)) = (at(edge), at(other)) {
            let mut v = a;
            for (v, b) in v.iter_mut().zip(b) {
                *v = 2.0 * *v - b;
            }
            add(
                weights.get(k.abs_diff(ti) as usize).copied().unwrap_or(0.0),
                v,
            );
        }
    }
    (wsum > 0.0).then(|| acc.map(|a| a / wsum))
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use crate::stabilise::motion::{invert, mul};

    fn slide(dx: f64, dy: f64) -> Step {
        Step {
            similarity: [1.0, 0.0, dx, dy],
            homography: [1.0, 0.0, dx, 0.0, 1.0, dy, 0.0, 0.0, 1.0],
            kept: 100,
            kept_similarity: 100,
            seen: 100,
            cut: false,
        }
    }

    const C: [f64; 2] = [960.0, 540.0];
    const CORNERS: [[f64; 2]; 4] = [[0.0, 0.0], [1920.0, 0.0], [0.0, 1080.0], [1920.0, 1080.0]];

    #[test]
    fn smoothness_nought_leaves_every_frame_alone() {
        let steps: Vec<Step> = (0..30)
            .map(|i| slide(f64::from(i % 3) - 1.0, 2.0))
            .collect();
        for model in [Model::Position, Model::Similarity, Model::Perspective] {
            let zero = vec![0.0; 31];
            let w = corrections(
                &steps,
                31,
                model,
                &Smoothing {
                    sigma: &zero,
                    lock: None,
                    keep_scale: false,
                },
                C,
                CORNERS,
            );
            for m in &w {
                let p = apply(m, [100.0, 200.0]).unwrap();
                assert!(
                    (p[0] - 100.0).abs() < 1e-6 && (p[1] - 200.0).abs() < 1e-6,
                    "{model:?}"
                );
            }
        }
    }

    #[test]
    fn no_motion_puts_every_frame_back_on_the_reference() {
        // The picture slides 3 px right a frame, so on frame 10 a point of
        // frame 0 is 30 px to the right; locked to frame 0, frame 10's output
        // shows that point where frame 0 had it.
        let steps: Vec<Step> = (0..20).map(|_| slide(3.0, 0.0)).collect();
        let sigma = vec![0.0; 21];
        for model in [Model::Position, Model::Similarity, Model::Perspective] {
            let w = corrections(
                &steps,
                21,
                model,
                &Smoothing {
                    sigma: &sigma,
                    lock: Some(0),
                    keep_scale: false,
                },
                C,
                CORNERS,
            );
            let p = apply(&w[10], [500.0, 500.0]).unwrap();
            assert!(
                (p[0] - 530.0).abs() < 1e-6 && (p[1] - 500.0).abs() < 1e-6,
                "{model:?} {p:?}"
            );
        }
    }

    /// Past a cut, No motion holds the other shot on its own first frame
    /// under every Method, rather than leaving it shaking as shot.
    #[test]
    fn no_motion_holds_the_shot_after_a_cut_on_its_first_frame() {
        let mut steps: Vec<Step> = (0..20).map(|_| slide(3.0, 0.0)).collect();
        steps[9].cut = true;
        let sigma = vec![0.0; 21];
        for model in [Model::Position, Model::Similarity, Model::Perspective] {
            let w = corrections(
                &steps,
                21,
                model,
                &Smoothing {
                    sigma: &sigma,
                    lock: Some(0),
                    keep_scale: false,
                },
                C,
                CORNERS,
            );
            // Frame 15 is five slides past frame 10, the shot's first.
            let p = apply(&w[15], [500.0, 500.0]).unwrap();
            assert!(
                (p[0] - 515.0).abs() < 1e-6 && (p[1] - 500.0).abs() < 1e-6,
                "{model:?} {p:?}"
            );
        }
    }

    #[test]
    fn a_steady_pan_is_kept_and_its_jitter_is_not() {
        // 4 px a frame to the right with a ±2 px rattle on top. Smoothed, the
        // correction undoes the rattle and leaves the pan.
        let steps: Vec<Step> = (0..120)
            .map(|i| slide(4.0 + if i % 2 == 0 { 2.0 } else { -2.0 }, 0.0))
            .collect();
        let sigma = vec![6.0; 121];
        let w = corrections(
            &steps,
            121,
            Model::Position,
            &Smoothing {
                sigma: &sigma,
                lock: None,
                keep_scale: false,
            },
            C,
            CORNERS,
        );
        // Mid-clip, a frame's correction is about the rattle's size, never the
        // pan's accumulated distance.
        for m in &w[30..90] {
            let p = apply(m, C).unwrap();
            assert!((p[0] - C[0]).abs() < 3.0, "{p:?}");
        }
    }

    #[test]
    fn a_step_nothing_agreed_on_carries_its_neighbours_motion() {
        // A steady 4 px pan with one step no speck agreed on. Held to frame 0,
        // frame 30 must still show the point frame 0 had, so the lost step is
        // taken as the pan, never as no motion.
        let mut steps: Vec<Step> = (0..40).map(|_| slide(4.0, 0.0)).collect();
        steps[20] = Step {
            seen: 3,
            ..Step::STILL
        };
        let sigma = vec![0.0; 41];
        for model in [Model::Position, Model::Similarity, Model::Perspective] {
            let w = corrections(
                &steps,
                41,
                model,
                &Smoothing {
                    sigma: &sigma,
                    lock: Some(0),
                    keep_scale: false,
                },
                C,
                CORNERS,
            );
            let p = apply(&w[30], [500.0, 500.0]).unwrap();
            assert!(
                (p[0] - 620.0).abs() < 1e-6 && (p[1] - 500.0).abs() < 1e-6,
                "{model:?} {p:?}"
            );
        }
    }

    #[test]
    fn a_camera_turning_fast_keeps_its_turn_and_loses_its_jitter() {
        // The camera turns 3° a frame and drifts 2 px a frame, with a rattle
        // of up to 1.5 px on top. Smoothed over about 20 frames either side,
        // which spans a half turn, the correction only takes the rattle out.
        let jitter = |t: f64| [1.5 * (2.3 * t).sin(), 1.5 * (1.7 * t).cos()];
        // Where the camera has a point of the first frame on frame t.
        let camera = |t: usize| -> Mat3 {
            let (th, j) = ((3.0 * t as f64).to_radians(), jitter(t as f64));
            let d = [C[0] + 2.0 * t as f64 + j[0], C[1] + j[1]];
            let (a, b) = (th.cos(), th.sin());
            [
                [a, -b, C[0] - a * d[0] + b * d[1]],
                [b, a, C[1] - b * d[0] - a * d[1]],
                [0.0, 0.0, 1.0],
            ]
        };
        let steps: Vec<Step> = (1..121)
            .map(|t| {
                let m = mul(&camera(t), &invert(&camera(t - 1)).unwrap());
                Step {
                    similarity: [m[0][0], m[1][0], m[0][2], m[1][2]],
                    homography: [
                        m[0][0], m[0][1], m[0][2], m[1][0], m[1][1], m[1][2], 0.0, 0.0, 1.0,
                    ],
                    ..slide(0.0, 0.0)
                }
            })
            .collect();
        let sigma = vec![20.0; 121];
        for model in [Model::Similarity, Model::Perspective] {
            let w = corrections(
                &steps,
                121,
                model,
                &Smoothing {
                    sigma: &sigma,
                    lock: None,
                    keep_scale: false,
                },
                C,
                CORNERS,
            );
            // Every corner moves by a few times the rattle at most, so the
            // picture needs next to no zoom to cover the frame.
            for (t, m) in w.iter().enumerate() {
                for q in CORNERS {
                    let p = apply(m, q).unwrap();
                    assert!(
                        (p[0] - q[0]).hypot(p[1] - q[1]) < 6.0,
                        "{model:?} frame {t}: {q:?} to {p:?}"
                    );
                }
            }
        }
    }

    #[test]
    fn a_steady_pan_needs_no_correction_at_the_ends_of_the_shot() {
        // Smoothing a pan that never changes speed changes nothing, at the
        // first and last frames as much as in the middle.
        let steps: Vec<Step> = (0..59).map(|_| slide(4.0, -1.0)).collect();
        let sigma = vec![10.0; 60];
        for model in [Model::Similarity, Model::Perspective] {
            let w = corrections(
                &steps,
                60,
                model,
                &Smoothing {
                    sigma: &sigma,
                    lock: None,
                    keep_scale: false,
                },
                C,
                CORNERS,
            );
            for t in [0, 1, 58, 59] {
                let p = apply(&w[t], C).unwrap();
                assert!(
                    (p[0] - C[0]).abs() < 1e-6 && (p[1] - C[1]).abs() < 1e-6,
                    "{model:?} frame {t}: {p:?}"
                );
            }
        }
    }
}

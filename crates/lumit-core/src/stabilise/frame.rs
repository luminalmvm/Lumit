//! What happens at the edges once the picture has been moved.
//!
//! In plain terms: moving each frame to follow a smooth path drags its edges
//! into view, so the corners of the output would show nothing. The picture can
//! be left like that, cropped to the part that always has something in it,
//! scaled up until the empty edges are pushed out of frame, or filled from the
//! frames either side. This works out how far each frame has to be scaled for
//! its edges to vanish, and from that the one scale for the shot, or a scale
//! that eases along with the shot when the crop is allowed to move.

use super::motion::{apply, Mat3};

/// The largest scale the bisection looks at. Past it a frame counts as not
/// coverable at all.
const SCALE_CEILING: f64 = 20.0;

/// The smallest scale, one or more, at which every output pixel of the frame,
/// inside a margin of `margin` of its size on each side, shows a pixel of the
/// input. `w` takes an output pixel to the input pixel it shows; `size` is the
/// frame's width and height.
#[must_use]
pub fn needed_scale(w: &Mat3, size: [f64; 2], margin: f64) -> f64 {
    let covered = |z: f64| covers(w, size, margin, z);
    if covered(1.0) {
        return 1.0;
    }
    if !covered(SCALE_CEILING) {
        return SCALE_CEILING;
    }
    let (mut lo, mut hi) = (1.0f64, SCALE_CEILING);
    for _ in 0..40 {
        let mid = 0.5 * (lo + hi);
        if covered(mid) {
            hi = mid;
        } else {
            lo = mid;
        }
    }
    hi
}

/// Whether the output, scaled up by `z` about its centre and inset by `margin`,
/// shows only input pixels. A homography takes the rectangle to a convex quad,
/// so its four corners settle it; the edge middles are checked as well.
fn covers(w: &Mat3, size: [f64; 2], margin: f64, z: f64) -> bool {
    let c = [0.5 * size[0], 0.5 * size[1]];
    let keep = (1.0 - 2.0 * margin.clamp(0.0, 0.45)) / z;
    let (hx, hy) = (0.5 * size[0] * keep, 0.5 * size[1] * keep);
    let probes = [
        [-hx, -hy],
        [0.0, -hy],
        [hx, -hy],
        [hx, 0.0],
        [hx, hy],
        [0.0, hy],
        [-hx, hy],
        [-hx, 0.0],
    ];
    probes.iter().all(|d| {
        apply(w, [c[0] + d[0], c[1] + d[1]])
            .is_some_and(|p| p[0] >= 0.0 && p[1] >= 0.0 && p[0] <= size[0] && p[1] <= size[1])
    })
}

/// How the shot's edges are dealt with.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Edges {
    /// Leave the moving edges in view.
    Show,
    /// Cut the picture to the part that always has something in it.
    Crop,
    /// Scale up until the edges are out of frame.
    CropAndScale,
    /// Fill the edges from the frames either side.
    Fill,
}

/// The rows the edge handling reads.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Framing {
    pub edges: Edges,
    /// The most the picture is scaled up, 1 or more.
    pub max_scale: f64,
    /// A share of the frame on each side that is allowed to stay empty.
    pub margin: f64,
    /// 0 lets the crop follow the shot frame by frame and crop least; 1 holds
    /// one crop for the whole shot, the smoothest.
    pub smooth_more: f64,
    /// Frames per second, for how quickly a moving crop may ease.
    pub fps: f64,
}

/// What the edge handling decided for the shot.
#[derive(Debug, Clone, PartialEq)]
pub struct Framed {
    /// The scale each analysed frame is drawn at, before the extra scale: what
    /// pushes the empty edges out of frame. One for Show, Crop and Fill.
    pub scale: Vec<f64>,
    /// For Crop, the scale whose frame the picture is cut to on each analysed
    /// frame; one elsewhere.
    pub crop: Vec<f64>,
    /// The largest scale any shown frame needed, which the status line reports.
    pub needed: f64,
    /// Whether some shown frame needed more than the maximum.
    pub short: bool,
}

/// Decide the edges for the shot from each analysed frame's map, considering
/// only the analysed frames `shown` (the ones the layer actually shows): they
/// alone decide the scale and feed the ease, and a frame outside them holds
/// the nearest shown frame's.
///
/// Crop and scale reads the Maximum scale and the Safe margin. Crop reads
/// neither: it cuts to the part every shown frame covers, eased by Steady
/// crop, and scales nothing, so it is never short.
#[must_use]
pub fn frame(maps: &[Mat3], size: [f64; 2], f: &Framing, shown: (usize, usize)) -> Framed {
    let n = maps.len();
    let ones = vec![1.0; n];
    if n == 0 || matches!(f.edges, Edges::Show | Edges::Fill) {
        return Framed {
            scale: ones.clone(),
            crop: ones,
            needed: 1.0,
            short: false,
        };
    }
    let crop = f.edges == Edges::Crop;
    let (max_scale, margin) = if crop {
        (SCALE_CEILING, 0.0)
    } else {
        (f.max_scale.max(1.0), f.margin)
    };
    let (a, b) = (
        shown.0.min(n - 1),
        shown.1.min(n - 1).max(shown.0.min(n - 1)),
    );
    let per: Vec<f64> = maps
        .iter()
        .enumerate()
        .map(|(t, m)| {
            if (a..=b).contains(&t) {
                needed_scale(m, size, margin)
            } else {
                1.0
            }
        })
        .collect();
    let needed = per[a..=b].iter().copied().fold(1.0, f64::max);
    let short = needed > max_scale + 1e-9;
    let fixed = needed.min(max_scale);

    // The moving crop: each shown frame's own need, held up over a second
    // either side so the ease never dips under a frame that needs more, then
    // eased. Both windows stop at the shown frames.
    let k = f.smooth_more.clamp(0.0, 1.0);
    let window = (f.fps.max(1.0)).round() as usize;
    let held: Vec<f64> = (0..n)
        .map(|t| {
            let t = t.clamp(a, b);
            let lo = t.saturating_sub(window).max(a);
            let hi = (t + window).min(b);
            per[lo..=hi]
                .iter()
                .copied()
                .fold(1.0, f64::max)
                .min(max_scale)
        })
        .collect();
    let sigma = (window as f64 / 2.0).max(1.0);
    let reach = (3.0 * sigma).ceil() as usize;
    let eased: Vec<f64> = (0..n)
        .map(|t| {
            let t = t.clamp(a, b);
            let lo = t.saturating_sub(reach).max(a);
            let hi = (t + reach).min(b);
            let (mut s, mut ws) = (0.0, 0.0);
            for (j, v) in held.iter().enumerate().take(hi + 1).skip(lo) {
                let d = j as f64 - t as f64;
                let wgt = (-(d * d) / (2.0 * sigma * sigma)).exp();
                s += wgt * v;
                ws += wgt;
            }
            let e = if ws > 0.0 { s / ws } else { held[t] };
            // The ease can only round a peak off from above.
            e.max(per[t].min(max_scale))
        })
        .collect();
    // Between the two by `k`, geometrically, since a scale is a ratio.
    let scale: Vec<f64> = eased
        .iter()
        .map(|&e| (fixed.ln() * k + e.ln() * (1.0 - k)).exp())
        .collect();
    if crop {
        Framed {
            scale: ones,
            crop: scale,
            needed: 1.0,
            short: false,
        }
    } else {
        Framed {
            scale,
            crop: ones,
            needed,
            short,
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use crate::stabilise::motion::IDENTITY;

    #[test]
    fn a_frame_moved_ten_pixels_needs_the_scale_that_hides_ten_pixels() {
        // Output pixel y shows input pixel y + 10 in x: the right ten columns
        // are empty, and scaling by w / (w − 20) about the centre hides them.
        let w = [[1.0, 0.0, 10.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];
        let z = needed_scale(&w, [1920.0, 1080.0], 0.0);
        assert!((z - 1920.0 / 1900.0).abs() < 1e-6, "{z}");
        assert!((needed_scale(&IDENTITY, [1920.0, 1080.0], 0.0) - 1.0).abs() < 1e-12);
    }

    #[test]
    fn the_static_scale_is_the_worst_shown_frame_capped_at_the_maximum() {
        let shift = |dx: f64| [[1.0, 0.0, dx], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];
        let maps = vec![shift(0.0), shift(40.0), shift(-10.0), shift(300.0)];
        let f = Framing {
            edges: Edges::CropAndScale,
            max_scale: 1.5,
            margin: 0.0,
            smooth_more: 1.0,
            fps: 30.0,
        };
        // Only the first three frames are shown: the 300 px frame is trimmed
        // away and must not decide the scale.
        let out = frame(&maps, [1920.0, 1080.0], &f, (0, 2));
        let want = 1920.0 / (1920.0 - 80.0);
        assert!((out.needed - want).abs() < 1e-6);
        assert!(!out.short);
        assert!(out.scale.iter().all(|&s| (s - want).abs() < 1e-6));
    }
}

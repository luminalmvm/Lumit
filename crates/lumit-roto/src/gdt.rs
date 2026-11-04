//! The geodesic distance transform (docs/impl/roto.md §2, pinned).
//!
//! For every pixel, how dearly it can reach the nearest foreground seed and
//! the nearest background seed, where a step's price is
//! `sqrt(‖Δx‖² + γ²·‖ΔI‖²)` — distance *and* colour change. The matte is the
//! ratio of the two prices, so it is already soft where the two are close.
//!
//! Computed by Toivanen's raster-scan chamfer passes: a forward scan
//! (top-left to bottom-right, each pixel relaxing over its four causal
//! 8-neighbours) and the mirrored backward scan, **three pass pairs, always**.
//! No convergence test, no priority queue, no per-pixel allocation — the fixed
//! count is what makes two runs bit-identical, and it is conservative for
//! seeds this dense.

use crate::{stroke::Seed, stroke::Seeds, FrameRgb};

/// Three pairs of scans, always (docs/impl/roto.md §12: a convergence test
/// trades determinism for nothing).
const PASS_PAIRS: usize = 3;

/// Keeps `α_raw` finite where both distances are zero — a pixel seeded both
/// ways by two overlapping strokes cannot exist (later wins), but a divide
/// that could produce NaN has no business in a matte.
const ALPHA_EPS: f32 = 1e-6;

/// Where each of a pixel's four forward steps sits in its row of the step
/// table: the side step right, the side step down, and the two diagonals down.
/// Every step the scans take is one of these, read from one end or the other.
const RIGHT: usize = 0;
const DOWN: usize = 1;
const DOWN_RIGHT: usize = 2;
const DOWN_LEFT: usize = 3;

/// `D_F`, `D_B` and then `α_raw = D_B / (D_F + D_B + ε)` into `out`.
///
/// `steps`, `d_f`, `d_b` and `out` are the solver's reused buffers and must
/// already be `width × height` long; anything else is left alone rather than
/// resized here, because the solver owns that decision.
pub(crate) fn alpha_raw(
    frame: FrameRgb<'_>,
    seeds: &Seeds,
    gamma: f32,
    steps: &mut [[f32; 4]],
    d_f: &mut [f32],
    d_b: &mut [f32],
    out: &mut [f32],
) {
    let w = frame.width() as usize;
    let h = frame.height() as usize;
    let n = out.len();
    if w * h != n || steps.len() != n || d_f.len() != n || d_b.len() != n {
        return;
    }
    step_prices(frame, gamma * gamma, steps);
    for (i, (f, b)) in d_f.iter_mut().zip(d_b.iter_mut()).enumerate() {
        let seed = seeds.at(i);
        *f = if seed == Seed::Foreground {
            0.0
        } else {
            f32::INFINITY
        };
        *b = if seed == Seed::Background {
            0.0
        } else {
            f32::INFINITY
        };
    }
    // Both distances ride the same scans: they never read each other, so
    // walking them side by side changes neither, and each step's price is
    // fetched once for the two of them.
    for _ in 0..PASS_PAIRS {
        forward(steps, w, h, d_f, d_b);
        backward(steps, w, h, d_f, d_b);
    }
    for (i, slot) in out.iter_mut().enumerate() {
        let f = d_f[i];
        let b = d_b[i];
        let denom = f + b + ALPHA_EPS;
        // Both unreachable means the picture is disconnected from every seed,
        // which 8-connectivity makes impossible; half is the honest answer if
        // it ever happens rather than a NaN travelling downstream.
        *slot = if denom.is_finite() && denom > 0.0 {
            (b / denom).clamp(0.0, 1.0)
        } else {
            0.5
        };
    }
}

/// The price of every step, `sqrt(geo + γ²·‖ΔI‖²)`, worked out once a frame.
///
/// It depends on the frame alone, and the twelve scans over the two distances
/// would otherwise take the same square root twelve times over. `geo` is the
/// squared geometric length of the step, 1 for a side and 2 for a diagonal,
/// kept squared because the colour term is squared too and the one root
/// covers both. A step off the frame is priced at infinity and never taken.
///
/// ponytail: the ceiling here is **leaks through low-contrast gaps** — a
/// subject touching a same-coloured wall costs nothing to walk into, so the
/// distance runs straight through the join and the matte swallows the wall.
/// That is the classical algorithm's known failure and the reason the
/// correction loop exists; the upgrade path is an evidence-bearing pairwise
/// term (graph cut, or a learned cost) on the seed seam of §9, not more passes.
/// Observable trigger: a solve whose matte includes a region joined to the
/// subject by a boundary with no colour step, as the dumbbell test pins.
fn step_prices(frame: FrameRgb<'_>, g2: f32, steps: &mut [[f32; 4]]) {
    let w = frame.width() as usize;
    let h = frame.height() as usize;
    // Off the frame there is no neighbour to price.
    let price = |on: bool, i: usize, j: usize, geo: f32| {
        if !on {
            return f32::INFINITY;
        }
        let a = frame.px(i);
        let b = frame.px(j);
        let dr = a[0] - b[0];
        let dg = a[1] - b[1];
        let db = a[2] - b[2];
        (geo + g2 * (dr * dr + dg * dg + db * db)).sqrt()
    };
    for y in 0..h {
        for x in 0..w {
            let i = y * w + x;
            let (right, down) = (x + 1 < w, y + 1 < h);
            steps[i] = [
                price(right, i, i + 1, 1.0),
                price(down, i, i + w, 1.0),
                price(right && down, i, i + w + 1, 2.0),
                price(x > 0 && down, i, i + w - 1, 2.0),
            ];
        }
    }
}

/// Top-left to bottom-right, each pixel relaxing over its four causal
/// neighbours above and to the left.
fn forward(steps: &[[f32; 4]], w: usize, h: usize, d_f: &mut [f32], d_b: &mut [f32]) {
    for y in 0..h {
        for x in 0..w {
            let i = y * w + x;
            let (mut f, mut b) = (d_f[i], d_b[i]);
            if y > 0 {
                let up = i - w;
                relax(&mut f, &mut b, d_f[up], d_b[up], steps[up][DOWN]);
                if x > 0 {
                    let j = up - 1;
                    relax(&mut f, &mut b, d_f[j], d_b[j], steps[j][DOWN_RIGHT]);
                }
                if x + 1 < w {
                    let j = up + 1;
                    relax(&mut f, &mut b, d_f[j], d_b[j], steps[j][DOWN_LEFT]);
                }
            }
            if x > 0 {
                let j = i - 1;
                relax(&mut f, &mut b, d_f[j], d_b[j], steps[j][RIGHT]);
            }
            d_f[i] = f;
            d_b[i] = b;
        }
    }
}

/// The mirror of [`forward`], over the four neighbours below and to the right.
fn backward(steps: &[[f32; 4]], w: usize, h: usize, d_f: &mut [f32], d_b: &mut [f32]) {
    for y in (0..h).rev() {
        for x in (0..w).rev() {
            let i = y * w + x;
            let (mut f, mut b) = (d_f[i], d_b[i]);
            let step = steps[i];
            if y + 1 < h {
                let down = i + w;
                relax(&mut f, &mut b, d_f[down], d_b[down], step[DOWN]);
                if x + 1 < w {
                    let j = down + 1;
                    relax(&mut f, &mut b, d_f[j], d_b[j], step[DOWN_RIGHT]);
                }
                if x > 0 {
                    let j = down - 1;
                    relax(&mut f, &mut b, d_f[j], d_b[j], step[DOWN_LEFT]);
                }
            }
            if x + 1 < w {
                relax(&mut f, &mut b, d_f[i + 1], d_b[i + 1], step[RIGHT]);
            }
            d_f[i] = f;
            d_b[i] = b;
        }
    }
}

/// Offer a pixel the price of reaching each kind of seed through one
/// neighbour. An unreachable neighbour offers infinity, which never wins.
#[inline]
fn relax(f: &mut f32, b: &mut f32, from_f: f32, from_b: f32, step: f32) {
    let via_f = from_f + step;
    if via_f < *f {
        *f = via_f;
    }
    let via_b = from_b + step;
    if via_b < *b {
        *b = via_b;
    }
}

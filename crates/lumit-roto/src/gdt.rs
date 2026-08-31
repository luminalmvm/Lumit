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

use crate::{FrameRgb, Seed, Seeds};

/// Three pairs of scans, always (docs/impl/roto.md §12: a convergence test
/// trades determinism for nothing).
const PASS_PAIRS: usize = 3;

/// Keeps `α_raw` finite where both distances are zero — a pixel seeded both
/// ways by two overlapping strokes cannot exist (later wins), but a divide
/// that could produce NaN has no business in a matte.
const ALPHA_EPS: f32 = 1e-6;

/// `D_F`, `D_B` and then `α_raw = D_B / (D_F + D_B + ε)` into `out`.
///
/// `d_f`, `d_b` and `out` are the solver's reused buffers and must already be
/// `width × height` long; anything shorter is left alone rather than resized
/// here, because the solver owns that decision.
pub(crate) fn alpha_raw(
    frame: FrameRgb<'_>,
    seeds: &Seeds,
    gamma: f32,
    d_f: &mut [f32],
    d_b: &mut [f32],
    out: &mut [f32],
) {
    transform(frame, seeds, Seed::Foreground, gamma, d_f);
    transform(frame, seeds, Seed::Background, gamma, d_b);
    for (i, slot) in out.iter_mut().enumerate() {
        let f = d_f.get(i).copied().unwrap_or(f32::INFINITY);
        let b = d_b.get(i).copied().unwrap_or(f32::INFINITY);
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

/// The geodesic distance to the nearest seed of one kind.
///
/// ponytail: the ceiling here is **leaks through low-contrast gaps** — a
/// subject touching a same-coloured wall costs nothing to walk into, so the
/// distance runs straight through the join and the matte swallows the wall.
/// That is the classical algorithm's known failure and the reason the
/// correction loop exists; the upgrade path is an evidence-bearing pairwise
/// term (graph cut, or a learned cost) on the seed seam of §9, not more passes.
/// Observable trigger: a solve whose matte includes a region joined to the
/// subject by a boundary with no colour step, as the dumbbell test pins.
fn transform(frame: FrameRgb<'_>, seeds: &Seeds, target: Seed, gamma: f32, d: &mut [f32]) {
    let w = frame.width() as usize;
    let h = frame.height() as usize;
    let g2 = gamma * gamma;
    for (i, slot) in d.iter_mut().enumerate() {
        *slot = if seeds.at(i) == target {
            0.0
        } else {
            f32::INFINITY
        };
    }
    for _ in 0..PASS_PAIRS {
        // Forward: top-left to bottom-right, over the four causal neighbours
        // above and to the left.
        for y in 0..h {
            for x in 0..w {
                let i = y * w + x;
                if y > 0 {
                    let up = i - w;
                    relax(d, &frame, i, up, 1.0, g2);
                    if x > 0 {
                        relax(d, &frame, i, up - 1, 2.0, g2);
                    }
                    if x + 1 < w {
                        relax(d, &frame, i, up + 1, 2.0, g2);
                    }
                }
                if x > 0 {
                    relax(d, &frame, i, i - 1, 1.0, g2);
                }
            }
        }
        // Backward: the mirror, over the four neighbours below and to the right.
        for y in (0..h).rev() {
            for x in (0..w).rev() {
                let i = y * w + x;
                if y + 1 < h {
                    let down = i + w;
                    relax(d, &frame, i, down, 1.0, g2);
                    if x + 1 < w {
                        relax(d, &frame, i, down + 1, 2.0, g2);
                    }
                    if x > 0 {
                        relax(d, &frame, i, down - 1, 2.0, g2);
                    }
                }
                if x + 1 < w {
                    relax(d, &frame, i, i + 1, 1.0, g2);
                }
            }
        }
    }
}

/// Offer pixel `i` the price of reaching a seed through its neighbour `j`.
///
/// `geo` is the squared geometric length of the step — 1 for a side, 2 for a
/// diagonal — kept squared because the colour term is squared too and the one
/// square root covers both.
#[inline]
fn relax(d: &mut [f32], frame: &FrameRgb<'_>, i: usize, j: usize, geo: f32, g2: f32) {
    let Some(&from) = d.get(j) else {
        return;
    };
    if !from.is_finite() {
        return;
    }
    let a = frame.px(i);
    let b = frame.px(j);
    let dr = a[0] - b[0];
    let dg = a[1] - b[1];
    let db = a[2] - b[2];
    let cand = from + (geo + g2 * (dr * dr + dg * dg + db * db)).sqrt();
    if let Some(slot) = d.get_mut(i) {
        if cand < *slot {
            *slot = cand;
        }
    }
}

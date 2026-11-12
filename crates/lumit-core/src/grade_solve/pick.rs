//! Picks on the picture: the qualifier's ranges from the pixels under a
//! circle (Set, Add or Subtract), and where a hue curve's points go.
//!
//! The qualifier keys on the picture as it came, in the node's grading space
//! (`v_in`), so the samples are read there. Hue is the vectorscope's angle,
//! saturation `max − min`, luma Rec. 709's, as
//! [`qualify`](crate::fx::effects::correction::maths::qualify) reads them.

use super::{colours, percentile, RowValue, Rows, SolveError};
use crate::fx::effects::correction::{maths, Correction};

/// What a qualifier pick does to the ranges already there.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PickMode {
    /// Ranges that cover the samples and nothing else.
    Set,
    /// The ranges widened to cover the samples too.
    Add,
    /// The nearest edge of one range pulled in past the samples.
    Subtract,
}

/// Which way a hue curve's x axis runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CurveAxis {
    Hue,
    Sat,
    Lum,
}

/// Where a hue curve pick lands on the curve's x axis, 0..1, and how far
/// the samples spread either side of it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CurvePick {
    pub centre: f32,
    pub spread: f32,
}

/// Samples with less chroma than this have no hue worth reading.
const GREY: f32 = 0.02;

/// The share of samples a range covers, from each end.
const TAIL: f32 = 0.02;

/// The axis of a hue curve's row id, or `None` for a row that isn't one.
pub fn curve_axis(id: &str) -> Option<CurveAxis> {
    match id {
        "hue_hue" | "hue_sat" | "hue_lum" => Some(CurveAxis::Hue),
        "lum_sat" => Some(CurveAxis::Lum),
        "sat_sat" | "sat_lum" => Some(CurveAxis::Sat),
        _ => None,
    }
}

/// The colours (scene linear, unpremultiplied) inside the circle at `centre`
/// with `radius`, raster pixels; at least the pixel under the centre.
pub fn circle_samples(
    rgba: &[f32],
    width: u32,
    height: u32,
    centre: [f32; 2],
    radius: f32,
) -> Vec<[f32; 3]> {
    if width == 0 || height == 0 || !centre.iter().all(|c| c.is_finite()) {
        return Vec::new();
    }
    let r = if radius.is_finite() {
        radius.max(0.5)
    } else {
        0.5
    };
    let span = |c: f32, max: u32| {
        let lo = (c - r).floor().clamp(0.0, (max - 1) as f32) as u32;
        let hi = (c + r).ceil().clamp(0.0, (max - 1) as f32) as u32;
        lo..=hi
    };
    let mut out = Vec::new();
    for y in span(centre[1], height) {
        for x in span(centre[0], width) {
            let (dx, dy) = (x as f32 + 0.5 - centre[0], y as f32 + 0.5 - centre[1]);
            if dx * dx + dy * dy > r * r {
                continue;
            }
            let k = ((y * width + x) * 4) as usize;
            if let Some(px) = rgba.get(k..k + 4) {
                out.extend(colours(px));
            }
        }
    }
    if out.is_empty() {
        let x = (centre[0].max(0.0) as u32).min(width - 1);
        let y = (centre[1].max(0.0) as u32).min(height - 1);
        let k = ((y * width + x) * 4) as usize;
        if let Some(px) = rgba.get(k..k + 4) {
            out.extend(colours(px));
        }
    }
    out
}

/// A range on a 0..1 axis: low, high and softness.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Range {
    lo: f32,
    hi: f32,
    soft: f32,
}

impl Range {
    /// The samples' own range, `TAIL` trimmed from each end, softness a
    /// quarter of its width within reason.
    fn of(mut values: Vec<f32>) -> Range {
        let lo = percentile(&mut values, TAIL);
        let hi = percentile(&mut values, 1.0 - TAIL);
        Range {
            lo,
            hi,
            soft: (0.25 * (hi - lo)).clamp(0.02, 0.15),
        }
    }

    fn union(self, o: Range) -> Range {
        Range {
            lo: self.lo.min(o.lo),
            hi: self.hi.max(o.hi),
            soft: self.soft.max(o.soft),
        }
    }

    /// This range with its nearer edge pulled past `cut`, and the share of
    /// the range that costs. `None` when `cut` lies outside it already, its
    /// softness included.
    fn without(self, cut: Range) -> Option<(Range, f32)> {
        if cut.hi < self.lo - self.soft || cut.lo > self.hi + self.soft {
            return None;
        }
        let width = (self.hi - self.lo).max(1e-6);
        let raised = cut.hi + self.soft;
        let lowered = cut.lo - self.soft;
        let up = (raised - self.lo).max(0.0);
        let down = (self.hi - lowered).max(0.0);
        Some(if up <= down {
            (
                Range {
                    lo: raised.min(self.hi),
                    ..self
                },
                up / width,
            )
        } else {
            (
                Range {
                    hi: lowered.max(self.lo),
                    ..self
                },
                down / width,
            )
        })
    }
}

/// The qualifier rows that key the samples `picked` (scene linear, from
/// [`circle_samples`] on the picture coming into `node`), by the node's
/// qualifier mode, Off reading as HSL, and 3D adding the circle's average
/// as a sample. The node's current rows are what Add widens and Subtract
/// cuts into.
pub fn qualifier_pick(
    picked: &[[f32; 3]],
    mode: PickMode,
    node: &Correction,
) -> Result<Rows, SolveError> {
    if picked.is_empty() {
        return Err(SolveError::Empty);
    }
    // A 3D key takes the circle's average as one more sample.
    if node.qual_mode == 4 {
        let one = super::mean(picked.iter().copied()).ok_or(SolveError::Empty)?;
        return super::pick3d::qualifier_pick_3d(&[one], mode, node);
    }
    let space = node.packed(1.0).space;
    let v: Vec<[f32; 3]> = picked
        .iter()
        .map(|u| u.map(|c| maths::encode(c, space)))
        .collect();
    let was_off = node.qual_mode == 0 || node.qual_mode > 3;
    // An Off qualifier has nothing to widen or cut into.
    let mode = match (was_off, mode) {
        (true, PickMode::Subtract) => return Ok(Vec::new()),
        (true, _) => PickMode::Set,
        (false, m) => m,
    };
    let pct = |v: f32| v / 100.0;
    let lum_now = Range {
        lo: pct(node.qual_lum_low),
        hi: pct(node.qual_lum_high),
        soft: pct(node.qual_lum_soft),
    };
    let lum = Range::of(v.iter().map(|v| maths::luma(*v)).collect());
    let mut rows: Rows = Vec::new();
    let put_range = |rows: &mut Rows, ids: [&'static str; 3], r: Range| {
        rows.push((ids[0], RowValue::Float(100.0 * r.lo.clamp(0.0, 1.0))));
        rows.push((ids[1], RowValue::Float(100.0 * r.hi.clamp(0.0, 1.0))));
        rows.push((ids[2], RowValue::Float(100.0 * r.soft.clamp(0.0, 1.0))));
    };
    const LUM: [&str; 3] = ["qual_lum_low", "qual_lum_high", "qual_lum_soft"];
    const SAT: [&str; 3] = ["qual_sat_low", "qual_sat_high", "qual_sat_soft"];

    match if was_off { 1 } else { node.qual_mode } {
        2 => {
            let now = [
                (node.qual_red_low, node.qual_red_high),
                (node.qual_green_low, node.qual_green_high),
                (node.qual_blue_low, node.qual_blue_high),
            ]
            .map(|(lo, hi)| Range {
                lo: pct(lo),
                hi: pct(hi),
                soft: pct(node.qual_rgb_soft),
            });
            let got: [Range; 3] =
                std::array::from_fn(|c| Range::of(v.iter().map(|v| v[c]).collect()));
            let next = edit_ranges(&now, &got, mode);
            const IDS: [[&str; 2]; 3] = [
                ["qual_red_low", "qual_red_high"],
                ["qual_green_low", "qual_green_high"],
                ["qual_blue_low", "qual_blue_high"],
            ];
            for (ids, r) in IDS.iter().zip(next.iter()) {
                rows.push((ids[0], RowValue::Float(100.0 * r.lo.clamp(0.0, 1.0))));
                rows.push((ids[1], RowValue::Float(100.0 * r.hi.clamp(0.0, 1.0))));
            }
            let soft = next.iter().map(|r| r.soft).fold(0.0, f32::max);
            rows.push((
                "qual_rgb_soft",
                RowValue::Float(100.0 * soft.clamp(0.0, 1.0)),
            ));
        }
        3 => {
            let next = edit_ranges(&[lum_now], &[lum], mode);
            put_range(&mut rows, LUM, next[0]);
        }
        _ => {
            let sat_now = Range {
                lo: pct(node.qual_sat_low),
                hi: pct(node.qual_sat_high),
                soft: pct(node.qual_sat_soft),
            };
            let sat = Range::of(v.iter().map(|v| maths::chroma(*v)).collect());
            let hues: Vec<f32> = v
                .iter()
                .filter(|v| maths::chroma(**v) > GREY)
                .map(|v| hue(*v))
                .collect();
            let chromatic = hues.len() * 4 >= v.len();
            let centre_now = (node.qual_hue / 360.0).rem_euclid(1.0);
            let half = (node.qual_hue_width / 720.0).max(0.0);
            let sym = node.qual_hue_sym / 100.0;
            // The hue range in turns about the current centre, so it reads
            // as an ordinary range and edits as one.
            let hue_now = Range {
                lo: -half * (1.0 - sym),
                hi: half * (1.0 + sym),
                soft: (node.qual_hue_soft / 360.0).max(0.0),
            };
            let (centre, hue_got) = match (mode, chromatic) {
                (PickMode::Set, true) => {
                    let c = circular_mean(&hues);
                    (c, Some(hue_range(&hues, c)))
                }
                (_, true) => (centre_now, Some(hue_range(&hues, centre_now))),
                (_, false) => (centre_now, None),
            };
            let use_hue = match mode {
                PickMode::Set => chromatic,
                _ => node.qual_use_hue,
            };

            let mut now = vec![sat_now, lum_now];
            let mut got = vec![sat, lum];
            if let Some(h) = hue_got.filter(|_| node.qual_use_hue || mode == PickMode::Set) {
                now.push(hue_now);
                got.push(h);
            }
            let next = edit_ranges(&now, &got, mode);
            put_range(&mut rows, SAT, next[0]);
            put_range(&mut rows, LUM, next[1]);
            if let Some(h) = next.get(2) {
                // Back to a centre and a width, symmetric.
                let width = (h.hi - h.lo).clamp(0.0, 1.0);
                let middle = (centre + 0.5 * (h.lo + h.hi)).rem_euclid(1.0);
                rows.push(("qual_hue", RowValue::Float(360.0 * middle)));
                rows.push(("qual_hue_width", RowValue::Float(360.0 * width)));
                rows.push((
                    "qual_hue_soft",
                    RowValue::Float((360.0 * h.soft).clamp(4.0, 60.0)),
                ));
                rows.push(("qual_hue_sym", RowValue::Float(0.0)));
            }
            rows.push(("qual_use_hue", RowValue::Bool(use_hue)));
        }
    }
    if was_off {
        rows.push(("qual_mode", RowValue::Choice(1)));
    }
    Ok(rows)
}

/// The ranges after the pick: the samples' own for Set, both covered for
/// Add, and for Subtract one range cut, the one that loses least.
fn edit_ranges(now: &[Range], got: &[Range], mode: PickMode) -> Vec<Range> {
    match mode {
        PickMode::Set => got.to_vec(),
        PickMode::Add => now.iter().zip(got).map(|(n, g)| n.union(*g)).collect(),
        PickMode::Subtract => {
            let mut out = now.to_vec();
            let best = now
                .iter()
                .zip(got)
                .enumerate()
                .filter_map(|(i, (n, g))| n.without(*g).map(|(r, cost)| (i, r, cost)))
                .min_by(|a, b| a.2.total_cmp(&b.2));
            if let Some((i, r, _)) = best {
                if let Some(slot) = out.get_mut(i) {
                    *slot = r;
                }
            }
            out
        }
    }
}

/// The vectorscope hue of a value, 0..1 from red.
fn hue(v: [f32; 3]) -> f32 {
    let c = maths::ycc(v);
    maths::hue_of(c[1], c[2])
}

/// The mean of hues as angles, 0..1.
fn circular_mean(hues: &[f32]) -> f32 {
    let (mut s, mut c) = (0.0f64, 0.0f64);
    for h in hues {
        let a = f64::from(*h) * std::f64::consts::TAU;
        s += a.sin();
        c += a.cos();
    }
    ((s.atan2(c) / std::f64::consts::TAU) as f32).rem_euclid(1.0)
}

/// The hues as a range of turns about `centre`: their own ends, trimmed,
/// made symmetric so the centre stays put.
fn hue_range(hues: &[f32], centre: f32) -> Range {
    let d: Vec<f32> = hues
        .iter()
        .map(|h| (h - centre + 0.5).rem_euclid(1.0) - 0.5)
        .collect();
    let r = Range::of(d);
    let half = r.lo.abs().max(r.hi.abs()).max(2.0 / 360.0);
    Range {
        lo: -half,
        hi: half,
        soft: (0.5 * half).clamp(4.0 / 360.0, 40.0 / 360.0),
    }
}

/// Where the samples `picked` sit on a hue curve's axis, read where the
/// node's hue curves read: after the rest of its grade.
pub fn curve_pick(
    picked: &[[f32; 3]],
    axis: CurveAxis,
    node: &Correction,
) -> Result<CurvePick, SolveError> {
    // The node's grade up to the hue curves, all of it keyed in.
    let mut pk = node.packed(1.0);
    pk.hue_curves_on = false;
    pk.mix = 1.0;
    let space = pk.space;
    let v: Vec<[f32; 3]> = picked
        .iter()
        .map(|u| {
            let vin = maths::v_in([u[0], u[1], u[2], 1.0], space);
            let out = maths::grade([u[0], u[1], u[2], 1.0], 1.0, maths::luma(vin), &pk);
            [0, 1, 2].map(|c| maths::encode(out[c], space))
        })
        .collect();
    if v.is_empty() {
        return Err(SolveError::Empty);
    }
    Ok(match axis {
        CurveAxis::Hue => {
            let hues: Vec<f32> = v
                .iter()
                .filter(|v| maths::chroma(**v) > GREY)
                .map(|v| hue(*v))
                .collect();
            if hues.is_empty() {
                return Err(SolveError::Empty);
            }
            let centre = circular_mean(&hues);
            CurvePick {
                centre,
                spread: hue_range(&hues, centre).hi,
            }
        }
        CurveAxis::Sat | CurveAxis::Lum => {
            let read = |v: &[f32; 3]| match axis {
                CurveAxis::Sat => maths::chroma(*v),
                _ => maths::luma(*v),
            };
            let mut x: Vec<f32> = v.iter().map(|v| read(v).clamp(0.0, 1.0)).collect();
            let centre = percentile(&mut x, 0.5);
            let r = Range::of(x);
            CurvePick {
                centre,
                spread: (0.5 * (r.hi - r.lo)).max(0.01),
            }
        }
    })
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use crate::grade_solve::test_support::{declared, with_rows};

    /// A pick on an orange band keys the band and leaves a blue one alone,
    /// and Subtract over part of it cuts that part out.
    #[test]
    fn a_pick_over_a_hue_band_keys_that_band() {
        let (w, h) = (64u32, 32u32);
        let mut rgba = Vec::new();
        for y in 0..h {
            for x in 0..w {
                let t = (y as f32 + 0.5) / h as f32;
                // Left half orange, right half blue, brighter down the
                // picture.
                let base = if x < w / 2 {
                    [0.9, 0.45, 0.12]
                } else {
                    [0.1, 0.2, 0.8]
                };
                let level = 0.3 + 0.6 * t;
                rgba.extend_from_slice(&[base[0] * level, base[1] * level, base[2] * level, 1.0]);
            }
        }
        let picked = circle_samples(&rgba, w, h, [16.0, 16.0], 10.0);
        assert!(picked.len() > 200);
        let node = with_rows(&[], &Vec::new());
        let rows = qualifier_pick(&picked, PickMode::Set, &node).unwrap();
        assert!(declared(&rows));
        let keyed = with_rows(&[], &rows);
        let q = keyed.packed(1.0).qual;
        let key = |x: u32, y: u32| {
            let k = ((y * w + x) * 4) as usize;
            let px = [rgba[k], rgba[k + 1], rgba[k + 2], rgba[k + 3]];
            maths::qualify(maths::v_in(px, 0), &q)
        };
        assert!(key(16, 16) > 0.99);
        assert!(key(10, 12) > 0.99);
        assert!(key(48, 16) < 0.01);

        // Subtract the bright end of the band: the bottom rows go, the top
        // ones stay.
        let bright = circle_samples(&rgba, w, h, [16.0, 30.0], 2.0);
        let cut = qualifier_pick(&bright, PickMode::Subtract, &keyed).unwrap();
        let mut both = rows.clone();
        both.extend(cut);
        let q = with_rows(&[], &both).packed(1.0).qual;
        let key = |x: u32, y: u32| {
            let k = ((y * w + x) * 4) as usize;
            let px = [rgba[k], rgba[k + 1], rgba[k + 2], rgba[k + 3]];
            maths::qualify(maths::v_in(px, 0), &q)
        };
        assert!(key(16, 31) < 0.01);
        assert!(key(16, 10) > 0.99);
    }
}

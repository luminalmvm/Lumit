//! The 3D qualifier's samples from strokes on the picture.
//!
//! **In plain terms.** A stroke is read as a row of small circles along its
//! path, each one's average colour a sample. The samples join the ones the
//! node already keeps (or drops), and when there are more than a Shape row
//! holds, the two closest are merged until they fit. Colours are read where
//! the key reads them: the picture coming into the node, in its grading
//! space, as `Y, Cb, Cr`.

use super::pick::{circle_samples, PickMode};
use super::{mean, RowValue, Rows, SolveError};
use crate::fx::effects::correction::{maths, Correction};
use crate::fx::shape_points::{ShapePoints, SHAPE_MAX_POINTS};

/// The average colours (scene linear) of circles of `radius` along the
/// stroke through `points`, raster pixels: at most [`SHAPE_MAX_POINTS`] of
/// them, spread evenly along it.
pub fn stroke_samples(
    rgba: &[f32],
    width: u32,
    height: u32,
    points: &[[f32; 2]],
    radius: f32,
) -> Vec<[f32; 3]> {
    let points: Vec<[f32; 2]> = points
        .iter()
        .copied()
        .filter(|p| p.iter().all(|c| c.is_finite()))
        .collect();
    let length: f32 = points
        .windows(2)
        .map(|s| (s[1][0] - s[0][0]).hypot(s[1][1] - s[0][1]))
        .sum();
    let step = radius.max(1.0).max(length / (SHAPE_MAX_POINTS - 1) as f32);
    let mut centres = Vec::new();
    let mut carried = 0.0f32;
    for s in points.windows(2) {
        let (a, b) = (s[0], s[1]);
        let len = (b[0] - a[0]).hypot(b[1] - a[1]);
        let mut t = carried;
        while t < len {
            let f = t / len;
            centres.push([a[0] + (b[0] - a[0]) * f, a[1] + (b[1] - a[1]) * f]);
            t += step;
        }
        carried = t - len;
    }
    centres.extend(points.last().copied());
    centres
        .into_iter()
        .take(SHAPE_MAX_POINTS)
        .filter_map(|c| mean(circle_samples(rgba, width, height, c, radius).into_iter()))
        .collect()
}

/// Distance between two samples as the key measures it: `Y` counted at
/// half.
fn distance(a: [f32; 3], b: [f32; 3]) -> f32 {
    let d = [0.5 * (a[0] - b[0]), a[1] - b[1], a[2] - b[2]];
    (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt()
}

/// The samples cut down to what a Shape row holds: the closest two merged,
/// weighted by how many each stands for, until they fit.
fn merged(mut s: Vec<([f32; 3], f32)>) -> Vec<[f32; 3]> {
    while s.len() > SHAPE_MAX_POINTS {
        let mut best = (f32::MAX, 0, 1);
        for i in 0..s.len() {
            for j in i + 1..s.len() {
                let d = distance(s[i].0, s[j].0);
                if d < best.0 {
                    best = (d, i, j);
                }
            }
        }
        let (_, i, j) = best;
        let (b, wb) = s.remove(j);
        let (a, wa) = s[i];
        let w = wa + wb;
        s[i] = ([0, 1, 2].map(|c| (a[c] * wa + b[c] * wb) / w), w);
    }
    s.into_iter().map(|(v, _)| v).collect()
}

/// The two Shape rows that store `samples`: Cb and Cr doubled, then luma.
fn shape_rows(samples: &[[f32; 3]]) -> (ShapePoints, ShapePoints) {
    let chroma: Vec<[f32; 2]> = samples.iter().map(|s| [2.0 * s[1], 2.0 * s[2]]).collect();
    let luma: Vec<[f32; 2]> = samples.iter().map(|s| [s[0], 0.0]).collect();
    (
        ShapePoints::sanitised(&chroma),
        ShapePoints::sanitised(&luma),
    )
}

/// The 3D rows after a pick of `picked` (scene linear, from
/// [`stroke_samples`] or a circle's average): Set keeps only these and
/// drops nothing, Add keeps these too, Subtract drops them. A node not yet
/// in 3D is switched to it, and Subtract on it does nothing.
pub fn qualifier_pick_3d(
    picked: &[[f32; 3]],
    mode: PickMode,
    node: &Correction,
) -> Result<Rows, SolveError> {
    if picked.is_empty() {
        return Err(SolveError::Empty);
    }
    let was_3d = node.qual_mode == 4;
    let mode = match (was_3d, mode) {
        (false, PickMode::Subtract) => return Ok(Vec::new()),
        (false, _) => PickMode::Set,
        (true, m) => m,
    };
    let pk = node.packed(1.0);
    let new: Vec<([f32; 3], f32)> = picked
        .iter()
        .map(|u| (maths::ycc(u.map(|c| maths::encode(c, pk.space))), 1.0))
        .collect();
    let q = &pk.qual;
    let had = |at: &[[f32; 3]], n: u32| -> Vec<([f32; 3], f32)> {
        at.iter().take(n as usize).map(|s| (*s, 1.0)).collect()
    };
    let (keep, drop) = match mode {
        PickMode::Set => (new, Vec::new()),
        PickMode::Add => {
            let mut keep = had(&q.keep_at, q.samples[0]);
            keep.extend(new);
            (keep, had(&q.drop_at, q.samples[1]))
        }
        PickMode::Subtract => {
            let mut drop = had(&q.drop_at, q.samples[1]);
            drop.extend(new);
            (had(&q.keep_at, q.samples[0]), drop)
        }
    };
    let (keep_c, keep_l) = shape_rows(&merged(keep));
    let (drop_c, drop_l) = shape_rows(&merged(drop));
    let mut rows: Rows = vec![
        ("qual_3d_keep", RowValue::Shape(keep_c)),
        ("qual_3d_keep_luma", RowValue::Shape(keep_l)),
        ("qual_3d_drop", RowValue::Shape(drop_c)),
        ("qual_3d_drop_luma", RowValue::Shape(drop_l)),
    ];
    if !was_3d {
        rows.push(("qual_mode", RowValue::Choice(4)));
    }
    Ok(rows)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use crate::grade_solve::test_support::{declared, with_rows};

    /// A stroke over an orange band keys the band and not the blue beside
    /// it, and a drop stroke over the band's dark end takes that end out.
    #[test]
    fn a_stroke_over_a_band_keys_that_band() {
        let (w, h) = (64u32, 32u32);
        let mut rgba = Vec::new();
        for y in 0..h {
            for x in 0..w {
                let base = if x < w / 2 {
                    [0.9, 0.45, 0.12]
                } else {
                    [0.1, 0.2, 0.8]
                };
                let level = 0.2 + 0.7 * (y as f32 + 0.5) / h as f32;
                rgba.extend_from_slice(&[base[0] * level, base[1] * level, base[2] * level, 1.0]);
            }
        }
        let key = |node: &Correction, x: u32, y: u32| {
            let k = ((y * w + x) * 4) as usize;
            let px = [rgba[k], rgba[k + 1], rgba[k + 2], rgba[k + 3]];
            maths::qualify(maths::v_in(px, 0), &node.packed(1.0).qual)
        };
        let stroke = stroke_samples(&rgba, w, h, &[[16.0, 2.0], [16.0, 30.0]], 2.0);
        assert!(stroke.len() > 4 && stroke.len() <= SHAPE_MAX_POINTS);
        let rows = qualifier_pick_3d(&stroke, PickMode::Set, &with_rows(&[], &Vec::new())).unwrap();
        assert!(declared(&rows));
        let keyed = with_rows(&[], &rows);
        assert_eq!(keyed.qual_mode, 4);
        assert!(key(&keyed, 10, 16) > 0.99);
        assert!(key(&keyed, 48, 16) < 0.01);

        let dark = stroke_samples(&rgba, w, h, &[[10.0, 1.0], [20.0, 1.0]], 1.0);
        let cut = qualifier_pick_3d(&dark, PickMode::Subtract, &keyed).unwrap();
        let mut both = rows.clone();
        both.extend(cut);
        let cut = with_rows(&[], &both);
        assert!(key(&cut, 16, 0) < 0.01);
        assert!(key(&cut, 16, 30) > 0.99);
    }
}

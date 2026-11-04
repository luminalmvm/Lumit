//! Keeping a shot's depth steady. Each plane is scaled to its own frame's nearest
//! and furthest, so something dark walking in rescales everything behind it. Each
//! frame gets a scale and offset that put it back on the frames before it, worked
//! out in order over the whole run, so it reads the same scrubbed or played.

use super::{expand, FrameRecord, PlaneKind};

/// How many points across and down are compared.
const GRID: (usize, usize) = (48, 27);
/// How much of the frames so far the next frame is measured against.
const MEMORY: f32 = 0.8;

/// A scale and offset per record, for a run of depth planes.
pub(super) fn fit(records: &[FrameRecord]) -> Vec<[f32; 2]> {
    let mut reference: Vec<f32> = Vec::new();
    records
        .iter()
        .filter(|record| record.kind == PlaneKind::Depth)
        .map(|record| {
            let samples = sampled(record);
            let fit = robust(&samples, &reference);
            let steady = samples.iter().map(|d| fit[0] * d + fit[1]);
            if reference.len() == samples.len() {
                for (kept, d) in reference.iter_mut().zip(steady) {
                    *kept = *kept * MEMORY + d * (1.0 - MEMORY);
                }
            } else {
                reference = steady.collect();
            }
            fit
        })
        .collect()
}

/// The plane at the grid's points, nearer larger, 0 to 1.
fn sampled(record: &FrameRecord) -> Vec<f32> {
    let plane = expand(record);
    let (w, h) = (record.width as usize, record.height as usize);
    let at = |x: usize, y: usize| {
        let i = (y * h / GRID.1 * w + x * w / GRID.0) * 2;
        let bytes = [
            plane.get(i).copied().unwrap_or(0),
            plane.get(i + 1).copied().unwrap_or(0),
        ];
        f32::from(u16::from_le_bytes(bytes)) / f32::from(u16::MAX)
    };
    (0..GRID.1)
        .flat_map(|y| (0..GRID.0).map(move |x| (x, y)))
        .map(|(x, y)| at(x, y))
        .collect()
}

/// The scale and offset that put `x` on `y`: the middle slope over many pairs of
/// points, then the middle offset, so a subject entering doesn't drag either. No
/// `y` yet is the first frame, which stays as it is.
fn robust(x: &[f32], y: &[f32]) -> [f32; 2] {
    if x.len() != y.len() || x.is_empty() {
        return [1.0, 0.0];
    }
    let n = x.len();
    let mut slopes: Vec<f32> = (0..n)
        .flat_map(|i| [(i, (i + n / 2) % n), (i, (i + n / 4) % n)])
        .filter(|&(a, b)| (x[a] - x[b]).abs() > 0.05)
        .map(|(a, b)| (y[a] - y[b]) / (x[a] - x[b]))
        .collect();
    // A flat frame has no slope to find.
    let scale = middle(&mut slopes).unwrap_or(1.0).clamp(0.2, 5.0);
    let mut offsets: Vec<f32> = x.iter().zip(y).map(|(x, y)| y - scale * x).collect();
    [scale, middle(&mut offsets).unwrap_or(0.0)]
}

fn middle(values: &mut [f32]) -> Option<f32> {
    let at = values.len() / 2;
    let (_, value, _) =
        (!values.is_empty()).then(|| values.select_nth_unstable_by(at, f32::total_cmp))?;
    Some(*value)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    /// A dark box entering rescales the plane. The fit puts the same background
    /// back where it was.
    #[test]
    fn a_subject_entering_leaves_the_background_where_it_was() {
        let base: Vec<f32> = (0..GRID.0 * GRID.1)
            .map(|i| 0.2 + 0.6 * (i % GRID.0) as f32 / GRID.0 as f32)
            .collect();
        let mut reference = base.clone();
        for (lo, hi) in [(0.2, 0.9), (0.0, 1.5), (-0.3, 0.8)] {
            // The plane's own squash, with the box taking a fifth of the frame
            // at the new nearest.
            let frame: Vec<f32> = base
                .iter()
                .enumerate()
                .map(|(i, d)| {
                    if i % 5 == 0 {
                        1.0
                    } else {
                        (d - lo) / (hi - lo)
                    }
                })
                .collect();
            let [a, b] = robust(&frame, &reference);
            for (i, (seen, was)) in frame.iter().zip(&base).enumerate() {
                if i % 5 != 0 {
                    assert!((a * seen + b - was).abs() < 0.03, "{a} {b} at {i}");
                }
            }
            reference = base.clone();
        }
    }
}

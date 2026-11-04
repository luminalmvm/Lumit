//! Keeping a shot's depth steady. Each plane is scaled to its own frame's nearest
//! and furthest, so something dark walking in rescales everything behind it. Each
//! frame gets a scale and offset that put it back on the frames before it, worked
//! out in order over the whole run, so it reads the same scrubbed or played.

use super::{expand, runs, FrameRecord, PlaneKind, PlaneRun};
use std::collections::HashMap;
use std::sync::{OnceLock, RwLock};
use uuid::Uuid;

/// How many points across and down are compared.
const GRID: (usize, usize) = (48, 27);
/// How much of the frames so far the next frame is measured against.
const MEMORY: f32 = 0.8;
/// How far a frame's middle sample can sit from the frame before it, after a fit
/// of its own, and still be the same shot. Measured on the real model: a box
/// walking in reaches 0.057 and a cut 0.08. Past it the depth starts afresh, or
/// the rest of the run would lock to a squashed scale.
const NEW_SHOT: f32 = 0.07;

/// Each instance's fits, with the name of the run they were made from.
type Fits = HashMap<Uuid, (u64, Vec<[f32; 2]>)>;

fn fits() -> &'static RwLock<Fits> {
    static FITS: OnceLock<RwLock<Fits>> = OnceLock::new();
    FITS.get_or_init(Default::default)
}

/// Work out and keep `run`'s fits, on whichever thread is publishing it.
pub(super) fn hold(instance: Uuid, run: &PlaneRun) {
    let fit = fit(&run.records);
    if let Ok(mut held) = fits().write() {
        held.insert(instance, (run.content, fit));
    }
}

/// What steadies `instance`'s plane at `frame`, as a scale and an offset.
#[must_use]
pub fn of(instance: Uuid, frame: i64) -> [f32; 2] {
    let found = || {
        let (runs, fits) = (runs().read().ok()?, fits().read().ok()?);
        let (run, (content, fit)) = (runs.get(&instance)?, fits.get(&instance)?);
        fit.get(run.index(frame)?)
            .copied()
            .filter(|_| *content == run.content)
    };
    found().unwrap_or([1.0, 0.0])
}

/// A scale and offset per record, for a run of depth planes.
pub(super) fn fit(records: &[FrameRecord]) -> Vec<[f32; 2]> {
    let mut reference: Vec<f32> = Vec::new();
    let mut previous: Vec<f32> = Vec::new();
    records
        .iter()
        .filter(|record| record.kind == PlaneKind::Depth)
        .map(|record| {
            let samples = sampled(record);
            if apart(&samples, &previous, robust(&samples, &previous)) > NEW_SHOT {
                reference.clear();
            }
            let fit = robust(&samples, &reference);
            let steady = samples.iter().map(|d| fit[0] * d + fit[1]);
            if reference.len() == samples.len() {
                for (kept, d) in reference.iter_mut().zip(steady) {
                    *kept = *kept * MEMORY + d * (1.0 - MEMORY);
                }
            } else {
                reference = steady.collect();
            }
            previous = samples;
            fit
        })
        .collect()
}

/// How far `x` sits from `y` after a fit, at the middle sample.
fn apart(x: &[f32], y: &[f32], [a, b]: [f32; 2]) -> f32 {
    let mut off: Vec<f32> = x
        .iter()
        .zip(y)
        .map(|(x, y)| (a * x + b - y).abs())
        .collect();
    middle(&mut off).unwrap_or(0.0)
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
            assert!(
                apart(&frame, &reference, [a, b]) < NEW_SHOT,
                "the same shot"
            );
            for (i, (seen, was)) in frame.iter().zip(&base).enumerate() {
                if i % 5 != 0 {
                    assert!((a * seen + b - was).abs() < 0.03, "{a} {b} at {i}");
                }
            }
            reference = base.clone();
        }
    }

    /// A cut has nothing to fit to, so it reads raw and starts afresh, however
    /// odd the frames before it had the scale.
    #[test]
    fn a_cut_does_not_lock_the_depth() {
        let plane = |d: &dyn Fn(usize, usize) -> f32| {
            let (w, h) = (96_usize, 54_usize);
            let mut data = Vec::with_capacity(w * h * 2);
            for y in 0..h {
                for x in 0..w {
                    let v = (d(x, y).clamp(0.0, 1.0) * 65535.0) as u16;
                    data.extend_from_slice(&v.to_le_bytes());
                }
            }
            super::super::PlaneOut {
                width: w as u32,
                height: h as u32,
                kind: PlaneKind::Depth,
                data,
            }
        };
        let across = plane(&|x, _| x as f32 / 96.0);
        let squashed = plane(&|x, _| 0.5 + x as f32 / 96.0 * 0.5);
        let down = plane(&|_, y| y as f32 / 54.0);
        let records: Vec<_> = [&across, &squashed, &down, &down]
            .iter()
            .enumerate()
            .map(|(i, out)| super::super::record_of(i as i64, out))
            .collect();
        let fits = fit(&records);
        assert!(
            (fits[1][0] - 2.0).abs() < 0.05,
            "the same shot is fitted: {fits:?}"
        );
        assert_eq!(fits[2], [1.0, 0.0], "the cut reads as it is");
        assert!((fits[3][0] - 1.0).abs() < 0.01, "and stays there: {fits:?}");
    }
}

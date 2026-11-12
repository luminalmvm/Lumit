//! Dropping points: the ones picked, and the ones that move with them.
//!
//! In plain terms: someone walks through a shot and the stabiliser follows
//! them as much as the background. Picking the points on them and dropping
//! those is the fix, but the points on a person are born and die all the time,
//! so the few picked on one frame are a fraction of them. So a drop spreads:
//! any point that moves the way a dropped one does (differently from the
//! picture as a whole, and the same way), near it while both are alive, goes
//! too, and so on from those, until nothing new moves with them.

use std::collections::BTreeSet;

use super::analysis::{Analysis, Run, Step};
use super::motion::apply;
use super::path::Model;
use super::plan::judged_by;

/// A dropped point spreads only to points moving their own way by at least
/// this much a frame, in pixels of the raster the points were followed on:
/// points that move with the picture are never taken with it.
const OWN_MOTION_PX: f64 = 0.5;

/// How closely two points' own motions must agree, frame by frame on average,
/// in pixels of the raster the points were followed on.
const AGREE_PX: f64 = 0.375;

/// How near two points must come while both are alive, as a share of the
/// frame's diagonal.
const NEAR: f64 = 0.15;

/// The most points one drop reaches, so a pick on a background that is itself
/// moving cannot take the whole shot.
const MOST: usize = 4000;

/// How a point moved on each step of its life beyond what the picture did:
/// the step's fitted motion's prediction subtracted from where it went.
fn own_motion(run: &Run, steps: &[Step]) -> Vec<Option<[f64; 2]>> {
    (run.first..run.last())
        .map(|f| {
            let m = judged_by(steps.get(f as usize)?, Model::Perspective)?;
            let (p, q) = (run.at(f)?, run.at(f + 1)?);
            let want = apply(&m, [f64::from(p[0]), f64::from(p[1])])?;
            Some([f64::from(q[0]) - want[0], f64::from(q[1]) - want[1]])
        })
        .collect()
}

/// Frames to a bucket of the index the search walks: a point is only ever
/// compared with points alive in the same buckets as it.
const BUCKET: u32 = 16;

/// `picked`, and every point that moves with them, transitively. Ascending.
#[must_use]
pub fn spread(analysis: &Analysis, steps: &[Step], picked: &[u32]) -> Vec<u32> {
    let by_id = |id: u32| analysis.runs.binary_search_by_key(&id, |r| r.id).ok();
    let diag = f64::from(analysis.width).hypot(f64::from(analysis.height));
    let own_floor = OWN_MOTION_PX * analysis.scale;
    let agree = AGREE_PX * analysis.scale;
    let near2 = (NEAR * diag).powi(2);
    // Which runs are alive in each stretch of frames.
    let mut alive: Vec<Vec<usize>> = vec![Vec::new(); (analysis.frames / BUCKET + 1) as usize];
    for (i, r) in analysis.runs.iter().enumerate() {
        for b in (r.first / BUCKET)..=(r.last() / BUCKET) {
            if let Some(list) = alive.get_mut(b as usize) {
                list.push(i);
            }
        }
    }
    let mut motions: std::collections::HashMap<usize, Vec<Option<[f64; 2]>>> =
        std::collections::HashMap::new();
    let mut motion = |i: usize| -> Vec<Option<[f64; 2]>> {
        motions
            .entry(i)
            .or_insert_with(|| {
                analysis
                    .runs
                    .get(i)
                    .map_or_else(Vec::new, |r| own_motion(r, steps))
            })
            .clone()
    };
    let mut out: BTreeSet<u32> = picked.iter().copied().collect();
    let mut queue: Vec<usize> = picked.iter().filter_map(|&id| by_id(id)).collect();
    while let Some(d) = queue.pop() {
        if out.len() >= MOST {
            break;
        }
        let Some(dr) = analysis.runs.get(d) else {
            continue;
        };
        let dm = motion(d);
        // A point moving with the picture spreads to nothing.
        let own: Vec<f64> = dm.iter().flatten().map(|m| m[0].hypot(m[1])).collect();
        if own.is_empty() || own.iter().sum::<f64>() / (own.len() as f64) < own_floor {
            continue;
        }
        let mut candidates: Vec<usize> = ((dr.first / BUCKET)..=(dr.last() / BUCKET))
            .filter_map(|b| alive.get(b as usize))
            .flatten()
            .copied()
            .collect();
        candidates.sort_unstable();
        candidates.dedup();
        for t in candidates {
            let Some(tr) = analysis.runs.get(t) else {
                continue;
            };
            if out.contains(&tr.id) {
                continue;
            }
            let (lo, hi) = (dr.first.max(tr.first), dr.last().min(tr.last()));
            if hi < lo + 3 {
                continue;
            }
            let close = (lo..=hi).step_by(4).any(|f| match (dr.at(f), tr.at(f)) {
                (Some(a), Some(b)) => {
                    let (dx, dy) = (f64::from(a[0] - b[0]), f64::from(a[1] - b[1]));
                    dx * dx + dy * dy < near2
                }
                _ => false,
            });
            if !close {
                continue;
            }
            let tm = motion(t);
            let (mut sum, mut n) = (0.0, 0usize);
            for f in lo..hi {
                let a = dm.get((f - dr.first) as usize).copied().flatten();
                let b = tm.get((f - tr.first) as usize).copied().flatten();
                if let (Some(a), Some(b)) = (a, b) {
                    sum += (a[0] - b[0]).hypot(a[1] - b[1]);
                    n += 1;
                }
            }
            if n >= 3 && sum / (n as f64) < agree {
                out.insert(tr.id);
                queue.push(t);
            }
        }
    }
    out.into_iter().collect()
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use crate::stabilise::analysis::pack_agreed;

    fn run(id: u32, first: u32, pts: Vec<[f32; 2]>) -> Run {
        let n = pts.len().saturating_sub(1);
        Run {
            id,
            first,
            points: pts,
            agreed: pack_agreed(&vec![true; n]),
        }
    }

    #[test]
    fn a_drop_takes_the_points_moving_with_it_and_leaves_the_picture() {
        // A still picture, and something crossing it 5 px a frame. Points 0
        // and 1 ride the mover at different times; 2 sits on the picture.
        let still = Step {
            similarity: [1.0, 0.0, 0.0, 0.0],
            homography: [1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0],
            kept: 100,
            kept_similarity: 100,
            seen: 100,
            cut: false,
        };
        let steps = vec![still; 30];
        let mover = |first: u32, n: u32, y: f32| -> Vec<[f32; 2]> {
            (0..n)
                .map(|k| [100.0 + 5.0 * (first + k) as f32, y])
                .collect()
        };
        let analysis = Analysis {
            fps: 30.0,
            rate_scale: 1.0,
            width: 640,
            height: 360,
            scale: 2.0,
            first: 0,
            frames: 31,
            asked: 31,
            steps: steps.clone(),
            runs: vec![
                run(0, 0, mover(0, 12, 100.0)),
                run(1, 6, mover(6, 20, 110.0)),
                run(2, 0, vec![[300.0, 300.0]; 31]),
            ],
            readout: [0.0; 2],
        };
        assert_eq!(spread(&analysis, &steps, &[0]), vec![0, 1]);
        // Dropping a point that moves with the picture takes only itself.
        assert_eq!(spread(&analysis, &steps, &[2]), vec![2]);
    }
}

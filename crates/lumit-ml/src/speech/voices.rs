//! Telling voices apart: a fingerprint of every second and a half of speech
//! from the speaker model, and the fingerprints grouped by how alike they
//! are.
//!
//! Fingerprints are compared by the angle between them (cosine distance).
//! Grouping is agglomerative with complete linkage: start with every
//! fingerprint on its own and keep joining the two nearest groups, where two
//! groups are as far apart as their two furthest members, until the nearest
//! two are further apart than a fixed distance, or until the number of
//! voices asked for is left. The nearest-neighbour chain finds the same joins
//! as the obvious search in quadratic rather than cubic time, which is what
//! lets an hour of speech be grouped at all.
//!
//! # Thread role and contract
//!
//! [`Voices`] owns its session and is never shared: each run is an FFI call
//! (14-ENGINEERING-RULES §1.3). [`group`] is pure and deterministic: the same
//! fingerprints in the same order always give the same numbers.

use std::path::Path;

use super::fbank::{Fbank, BANDS};
use crate::{
    error::MlError,
    manifest,
    session::{Feed, Place, Prefer, Session},
};

/// The speaker model, open.
#[derive(Debug)]
pub struct Voices {
    session: Session,
    names: manifest::Whisper,
    fbank: Fbank,
}

impl Voices {
    /// The speaker model from the pack's folder, or `None` when the pack has
    /// none.
    ///
    /// # Errors
    ///
    /// Whatever [`Session::open`] answers for a file that is named and will
    /// not open.
    pub fn open(dir: &Path, names: &manifest::Whisper) -> Result<Option<Self>, MlError> {
        let Some(file) = &names.voices else {
            return Ok(None);
        };
        Ok(Some(Voices {
            session: Session::open(dir, file, Prefer::Cpu)?,
            names: names.clone(),
            fbank: Fbank::new(),
        }))
    }

    /// The fingerprints of some stretches of speech at 16 kHz, each of
    /// length one, in order. Stretches of the same length go through the
    /// model together.
    ///
    /// # Errors
    ///
    /// Whatever a run refuses.
    pub fn fingerprints(&mut self, stretches: &[&[f32]]) -> Result<Vec<Vec<f32>>, MlError> {
        let features: Vec<(Vec<f32>, usize)> =
            stretches.iter().map(|s| self.fbank.fbank(s)).collect();
        let mut out = vec![Vec::new(); stretches.len()];
        let mut lengths: Vec<usize> = features.iter().map(|(_, frames)| *frames).collect();
        lengths.sort_unstable();
        lengths.dedup();
        for frames in lengths.into_iter().filter(|frames| *frames > 0) {
            let members: Vec<usize> = (0..features.len())
                .filter(|at| features[*at].1 == frames)
                .collect();
            let batch: Vec<f32> = members
                .iter()
                .flat_map(|at| features[*at].0.iter().copied())
                .collect();
            let ran = self.session.bound(
                &[(
                    self.names.voices_input.as_str(),
                    Feed::F32(&[members.len(), frames, BANDS], &batch),
                )],
                &[(self.names.voices_output.as_str(), Place::Host)],
            )?;
            let (shape, prints) = ran.first().ok_or(MlError::ShapeMismatch)?.read()?;
            let width = shape.last().copied().unwrap_or(0);
            for (row, at) in members.iter().enumerate() {
                let print = prints
                    .get(row * width..(row + 1) * width)
                    .ok_or(MlError::ShapeMismatch)?;
                out[*at] = unit(print.to_vec());
            }
        }
        Ok(out)
    }
}

/// `vector` scaled to length one, or left alone when it has no length.
#[must_use]
pub fn unit(mut vector: Vec<f32>) -> Vec<f32> {
    let length = vector.iter().map(|v| v * v).sum::<f32>().sqrt();
    if length > 0.0 {
        for value in &mut vector {
            *value /= length;
        }
    }
    vector
}

/// How unalike two fingerprints of length one are: 0 the same, 2 opposite.
fn distance(a: &[f32], b: &[f32]) -> f32 {
    1.0 - a.iter().zip(b).map(|(x, y)| x * y).sum::<f32>()
}

/// The most fingerprints grouped against each other. The distances between
/// 4,000 take 32 MB, where an eight-hour recording's would take 3 GB.
const MOST: usize = 4000;

/// Group fingerprints of length one: a group number for each, numbered in
/// the order groups are first met. Stops at `count` groups when asked for a
/// count, and otherwise when the two nearest groups are further apart than
/// `threshold`.
///
/// Past [`MOST`] of them, every nth is grouped and each of the rest goes
/// with the group it is nearest the middle of.
#[must_use]
pub fn group(prints: &[&[f32]], count: Option<usize>, threshold: f32) -> Vec<u32> {
    if prints.len() <= MOST {
        return linked(prints, count, threshold);
    }
    let step = prints.len().div_ceil(MOST);
    let some: Vec<&[f32]> = prints.iter().step_by(step).copied().collect();
    let numbers = linked(&some, count, threshold);
    let groups = numbers.iter().max().map_or(0, |top| *top as usize + 1);
    let width = some.first().map_or(0, |print| print.len());
    let mut middles = vec![vec![0.0f32; width]; groups];
    for (print, number) in some.iter().zip(&numbers) {
        if let Some(middle) = middles.get_mut(*number as usize) {
            for (sum, value) in middle.iter_mut().zip(*print) {
                *sum += value;
            }
        }
    }
    let middles: Vec<Vec<f32>> = middles.into_iter().map(unit).collect();
    let nearest = |print: &[f32]| {
        middles
            .iter()
            .enumerate()
            .min_by(|a, b| distance(print, a.1).total_cmp(&distance(print, b.1)))
            .map_or(0, |(group, _)| group as u32)
    };
    // Numbered again in the order the groups are met among all of them.
    let mut renumbered: Vec<Option<u32>> = vec![None; groups.max(1)];
    let mut next = 0u32;
    prints
        .iter()
        .enumerate()
        .map(|(at, print)| {
            let group = match numbers.get(at / step) {
                Some(number) if at % step == 0 => *number,
                _ => nearest(print),
            };
            let slot = renumbered.get_mut(group as usize);
            slot.map_or(group, |slot| {
                *slot.get_or_insert_with(|| {
                    next += 1;
                    next - 1
                })
            })
        })
        .collect()
}

/// [`group`], with every fingerprint measured against every other.
fn linked(prints: &[&[f32]], count: Option<usize>, threshold: f32) -> Vec<u32> {
    let n = prints.len();
    if n == 0 {
        return Vec::new();
    }
    // The upper triangle of the distances, row by row.
    let index = |i: usize, j: usize| {
        let (a, b) = if i < j { (i, j) } else { (j, i) };
        a * n - a * (a + 1) / 2 + (b - a - 1)
    };
    let mut far = vec![0.0f32; n * (n - 1) / 2];
    for i in 0..n {
        for j in i + 1..n {
            far[index(i, j)] = distance(prints[i], prints[j]);
        }
    }

    // The nearest-neighbour chain: every join complete linkage would make,
    // in some order, with its distance.
    let mut alive = vec![true; n];
    let mut joins: Vec<(f32, usize, usize)> = Vec::with_capacity(n.saturating_sub(1));
    let mut chain: Vec<usize> = Vec::new();
    let mut left = n;
    while left > 1 {
        if chain.is_empty() {
            if let Some(first) = alive.iter().position(|a| *a) {
                chain.push(first);
            }
        }
        let Some(&a) = chain.last() else { break };
        let before = chain.len().checked_sub(2).map(|at| chain[at]);
        let mut best: Option<(f32, usize)> = before.map(|b| (far[index(a, b)], b));
        for (k, live) in alive.iter().enumerate() {
            if !*live || k == a {
                continue;
            }
            let d = far[index(a, k)];
            if best.is_none_or(|(bd, _)| d < bd) {
                best = Some((d, k));
            }
        }
        let Some((d, b)) = best else { break };
        if Some(b) == before {
            chain.pop();
            chain.pop();
            joins.push((d, a.min(b), a.max(b)));
            let (keep, gone) = (a.min(b), a.max(b));
            alive[gone] = false;
            for k in 0..n {
                if alive[k] && k != keep {
                    let joined = far[index(keep, k)].max(far[index(gone, k)]);
                    far[index(keep, k)] = joined;
                }
            }
            left -= 1;
        } else {
            chain.push(b);
        }
    }

    // The joins in distance order, applied until the stopping rule says so.
    joins.sort_by(|x, y| x.0.total_cmp(&y.0).then(x.1.cmp(&y.1)));
    let mut parent: Vec<usize> = (0..n).collect();
    let root = |parent: &mut Vec<usize>, mut i: usize| {
        while parent[i] != i {
            parent[i] = parent[parent[i]];
            i = parent[i];
        }
        i
    };
    let mut groups = n;
    for (d, a, b) in joins {
        let done = match count {
            Some(want) => groups <= want.max(1),
            None => d > threshold,
        };
        if done {
            break;
        }
        let (ra, rb) = (root(&mut parent, a), root(&mut parent, b));
        if ra != rb {
            parent[ra.max(rb)] = ra.min(rb);
            groups -= 1;
        }
    }

    let mut numbers: Vec<Option<u32>> = vec![None; n];
    let mut next = 0u32;
    (0..n)
        .map(|i| {
            let r = root(&mut parent, i);
            *numbers[r].get_or_insert_with(|| {
                next += 1;
                next - 1
            })
        })
        .collect()
}

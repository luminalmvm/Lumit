//! A stabiliser's analysis: how the whole picture moved from each frame to the
//! next, measured from the specks [`crate::Tracker`] followed.
//!
//! # In plain terms
//!
//! Each pair of neighbouring frames shares hundreds of specks, and they may
//! move two or three ways at once: the background, and someone walking through
//! it. Each way is found by a robust search (the same LO-RANSAC the planar
//! tracker uses), and one of them is chosen as the picture for the whole shot
//! ([`layers`]). That layer's specks are then fitted two ways, as a slide with
//! a turn and a growth and as a four-corner stretch, and each speck is marked
//! on every step as belonging to it or not, which is what the Viewer colours
//! them by. Last, the rolling shutter is measured from those fits.
//!
//! Pure computation, deterministic, cancellable between frame pairs.

use lumit_core::stabilise::analysis::{pack_agreed, Analysis, Run, Step, MIN_KEPT};
use lumit_core::stabilise::motion::{
    agree_px, close_px, fit_similarity, fit_similarity_from, four_corner, from_rows,
    from_similarity, within, Pair,
};

use crate::{Correspondence, Track, TrackSet};

mod layers;
mod readout;

/// What the analysis was of, beyond the tracks themselves.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Shape {
    /// The rate the frames were read at: the file's own, with no Interpret
    /// footage rate on it.
    pub fps: f64,
    /// Analysed frames per frame of the item's rate: see
    /// [`Analysis::rate_scale`].
    pub rate_scale: f64,
    /// The frame `jumps[0]` steps out of: the first frame decoded, which can
    /// come before the first frame any track starts on.
    pub decoded_from: i64,
    /// The source frame the tracks' frame zero is.
    pub first: i64,
    /// Frames asked for: the clip, or the Frames row's span.
    pub asked: u32,
    /// The layer's raster, which the stored positions are in.
    pub width: u32,
    pub height: u32,
    /// Layer pixels per tracked pixel: one when the tracker read the frames at
    /// their own size, more when it read them smaller.
    pub scale: f64,
}

/// Why no analysis came back.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum StabiliseError {
    /// Fewer than two frames were followed.
    #[error("too few frames were followed")]
    TooShort,
    /// The caller stopped it.
    #[error("the analysis was cancelled")]
    Cancelled,
}

/// Fit every step of `set` and pack its specks into an [`Analysis`].
///
/// `jumps` marks the frame pairs whose overall brightness jumped as a cut does,
/// one per step from [`Shape::decoded_from`]; a step is a cut when it jumped
/// and the specks crossing it collapsed.
///
/// # Errors
///
/// [`StabiliseError::TooShort`] for fewer than two frames,
/// [`StabiliseError::Cancelled`] when `stop` said so.
pub fn analyse(
    set: &TrackSet,
    shape: &Shape,
    jumps: &[bool],
    stop: &dyn Fn() -> bool,
) -> Result<Analysis, StabiliseError> {
    let (first, last) = set.frame_range().ok_or(StabiliseError::TooShort)?;
    if last <= first {
        return Err(StabiliseError::TooShort);
    }
    let frames = u32::try_from(last - first + 1).map_err(|_| StabiliseError::TooShort)?;
    let s = shape.scale;
    let (agree, close) = (agree_px(s), close_px(s));
    let layer = [f64::from(shape.width), f64::from(shape.height)];
    let size = set.source_size();
    let count = frames as usize - 1;

    // Every step's ways of moving, and how far each is one of the step
    // before's.
    let mut found: Vec<Vec<layers::Layer>> = Vec::with_capacity(count);
    let mut share: Vec<[[Option<f64>; layers::MOST]; layers::MOST]> = Vec::with_capacity(count);
    let mut seen_by: Vec<(u32, bool)> = Vec::with_capacity(count);
    let mut ids_then: Vec<u32> = Vec::new();
    let mut previous_seen = 0u32;
    let mut walk = Walk::new(set.tracks());
    for f in first..last {
        if stop() {
            return Err(StabiliseError::Cancelled);
        }
        let pts = walk.step(f);
        let seen = u32::try_from(pts.len()).unwrap_or(u32::MAX);
        let jumped = usize::try_from(f - shape.decoded_from)
            .ok()
            .and_then(|i| jumps.get(i).copied())
            .unwrap_or(false);
        let collapsed = seen < MIN_KEPT.max(previous_seen / 4);
        let cut = jumped && collapsed;
        previous_seen = seen;
        let here = if cut {
            Vec::new()
        } else {
            layers::split(&pts, size, f)
        };
        let ids_now: Vec<u32> = pts.iter().map(|c| c.id).collect();
        let mut links = [[None; layers::MOST]; layers::MOST];
        if let Some(then) = found.last() {
            for (a, now) in here.iter().enumerate().take(layers::MOST) {
                for (b, before) in then.iter().enumerate().take(layers::MOST) {
                    links[a][b] =
                        layers::shared(&now.members, &before.members, &ids_now, &ids_then);
                }
            }
        }
        found.push(here);
        share.push(links);
        seen_by.push((seen, cut));
        ids_then = ids_now;
    }
    let chosen = layers::choose(&found, &share, stop).ok_or(StabiliseError::Cancelled)?;

    let mut steps = Vec::with_capacity(count);
    // Per step, the ids of the layer it was measured from, ascending.
    let mut agreed: Vec<Vec<u32>> = Vec::with_capacity(count);
    let mut walk = Walk::new(set.tracks());
    for (k, f) in (first..last).enumerate() {
        if stop() {
            return Err(StabiliseError::Cancelled);
        }
        let (seen, cut) = seen_by.get(k).copied().unwrap_or((0, false));
        let members = chosen
            .get(k)
            .copied()
            .flatten()
            .and_then(|i| found.get_mut(k)?.get_mut(i))
            .map(|l| std::mem::take(&mut l.members))
            .unwrap_or_default();
        let pts = walk.step(f);
        let fitted = (!members.is_empty())
            .then(|| fit_step(&pts, &members, (s, layer), (agree, close)))
            .flatten();
        match fitted {
            Some((step, read)) => {
                steps.push(Step { seen, ..step });
                agreed.push(read);
            }
            None => {
                steps.push(Step {
                    seen,
                    cut,
                    ..Step::STILL
                });
                agreed.push(Vec::new());
            }
        }
        if let Some(l) = found.get_mut(k) {
            l.clear();
        }
    }

    let mut runs = Vec::with_capacity(set.tracks().len());
    for track in set.tracks() {
        if track.points.len() < 2 {
            continue;
        }
        let start = u32::try_from(track.first_frame() - first).unwrap_or(0);
        let points: Vec<[f32; 2]> = track
            .points
            .iter()
            .map(|p| {
                let q = to_layer([p.x, p.y], s);
                [q[0] as f32, q[1] as f32]
            })
            .collect();
        let flags: Vec<bool> = (0..track.points.len() - 1)
            .map(|k| {
                agreed
                    .get(start as usize + k)
                    .is_some_and(|ids| ids.binary_search(&track.id).is_ok())
            })
            .collect();
        runs.push(Run {
            id: track.id,
            first: start,
            points,
            agreed: pack_agreed(&flags),
        });
    }

    let readout = readout::readout(&steps, &runs, layer, s, stop)?;

    Ok(Analysis {
        fps: shape.fps,
        rate_scale: shape.rate_scale,
        width: shape.width,
        height: shape.height,
        scale: s,
        first: shape.first + first,
        frames,
        asked: shape.asked.max(frames),
        steps,
        runs,
        readout,
    })
}

/// The tracks alive across each step in turn, walked once in birth order, so
/// the pairs out of every frame are found without reading every track of the
/// clip for each: linear in the clip's length.
struct Walk<'a> {
    tracks: &'a [Track],
    /// Track indices by the frame each starts on.
    order: Vec<usize>,
    next: usize,
    /// The tracks started and not yet ended, in id order.
    alive: Vec<usize>,
}

impl<'a> Walk<'a> {
    fn new(tracks: &'a [Track]) -> Self {
        let mut order: Vec<usize> = (0..tracks.len()).collect();
        order.sort_by_key(|&i| (tracks[i].first_frame(), i));
        Walk {
            tracks,
            order,
            next: 0,
            alive: Vec::new(),
        }
    }

    /// Every track present on both `f` and `f + 1`, in id order: what
    /// [`TrackSet::correspondences`] answers. `f` must rise from call to call.
    fn step(&mut self, f: i64) -> Vec<Correspondence> {
        let tracks = self.tracks;
        let mut born = false;
        while let Some(&i) = self.order.get(self.next) {
            if tracks[i].first_frame() > f {
                break;
            }
            self.alive.push(i);
            self.next += 1;
            born = true;
        }
        self.alive.retain(|&i| tracks[i].last_frame() > f);
        // The set is in id order, so index order is id order.
        if born {
            self.alive.sort_unstable();
        }
        self.alive
            .iter()
            .filter_map(|&i| {
                let t = &tracks[i];
                let (a, b) = (t.point_at(f)?, t.point_at(f + 1)?);
                Some(Correspondence {
                    id: t.id,
                    from: [a.x, a.y],
                    to: [b.x, b.y],
                })
            })
            .collect()
    }
}

/// A point of the tracked raster in the layer's pixels, `s` of them to a
/// tracked one. The tracker counts a pixel's centre as its whole coordinate,
/// and tracked pixel `j` covers layer pixels `j·s` to `(j + 1)·s`, whose
/// centre is `(j + ½)·s` in the layer's own convention.
#[must_use]
pub fn to_layer(p: [f64; 2], s: f64) -> [f64; 2] {
    [(p[0] + 0.5) * s, (p[1] + 0.5) * s]
}

/// The way back: a point of the layer on the tracked raster.
#[must_use]
pub fn to_tracked(p: [f64; 2], s: f64) -> [f64; 2] {
    let s = if s > 0.0 { s } else { 1.0 };
    [p[0] / s - 0.5, p[1] / s - 0.5]
}

/// One step's fits, in layer pixels (`s` of them to a tracked pixel), and the
/// ids they read, ascending: the layer `members` of `pts` it is measured from,
/// and every other pair within twice `agree` of the layer's own motion, so the
/// parts of one background at slightly different depths, which a larger step
/// splits apart, are read alike on every step. `s` is the layer's pixels to a
/// tracked one and `size` the layer's raster. The last plain fits are over
/// the pairs within `close` layer pixels; agreement is counted over every
/// pair, within `agree`. `None` when the layer is too small to fit.
fn fit_step(
    pts: &[crate::Correspondence],
    members: &[u32],
    (s, size): (f64, [f64; 2]),
    (agree, close): (f64, f64),
) -> Option<(Step, Vec<u32>)> {
    let scale = |c: &crate::Correspondence| -> Pair { (to_layer(c.from, s), to_layer(c.to, s)) };
    let core: Vec<Pair> = pts
        .iter()
        .filter(|c| members.binary_search(&c.id).is_ok())
        .map(scale)
        .collect();
    let (own, _) = fit_similarity(&core, close)?;
    let layer = from_similarity(own);
    let read: Vec<u32> = pts
        .iter()
        .filter(|c| members.binary_search(&c.id).is_ok() || within(&layer, &scale(c), 2.0 * agree))
        .map(|c| c.id)
        .collect();
    let core: Vec<Pair> = pts
        .iter()
        .filter(|c| read.binary_search(&c.id).is_ok())
        .map(scale)
        .collect();
    // Started from the layer's own motion, so what was read beside it only
    // counts as far as it agrees.
    let (sim, _) = fit_similarity_from(&core, own, close)?;
    // A four-corner fit that fails its own checks is the step's similarity.
    let hom = four_corner(&core, sim, size, close);
    let (m, ms) = (from_rows(hom), from_similarity(sim));
    let (mut kept, mut kept_similarity) = (0u32, 0u32);
    for p in pts.iter().map(scale) {
        kept += u32::from(within(&m, &p, agree));
        kept_similarity += u32::from(within(&ms, &p, agree));
    }
    Some((
        Step {
            similarity: sim,
            homography: hom,
            kept,
            kept_similarity,
            seen: 0,
            cut: false,
        },
        read,
    ))
}

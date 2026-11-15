//! Carrying every live track from one frame to the next
//! (docs/impl/tracking.md §2).
//!
//! # In plain terms
//!
//! Each live speck is looked for in the new frame, followed back again, and
//! compared with how it looked when it was first seen. A speck that fails any
//! of the three is ended, never moved somewhere plausible.
//!
//! Looking for hundreds of specks one by one is the cost of a whole analysis,
//! and most of that cost is the search from the coarsest picture down, which
//! is how a speck that moved a long way is found. When the frame as a whole
//! moved, nearly every speck moved with it, so a tracker can be asked to
//! measure that first: follow a few dozen specks the long way, fit the one warp
//! they agree on, and start every other speck where that warp puts it. Each of
//! those then only has to be refined on the finer pictures. A speck on
//! something moving by itself is not where the warp puts it, and on the finer
//! pictures alone it is usually not found at all, so it ends. A camera solve
//! or a stabiliser was going to set it aside anyway.

use crate::exclude;
use crate::geom::{project, Mat3};
use crate::klt::{self, Step};
use crate::pairs::{homography_ransac, GeometrySettings};
use crate::{
    Correspondence, FlowSeed, LiveTrack, Track, TrackPoint, TrackState, TrackStep, Tracker,
};

/// How many specks are followed the long way to measure the frame's motion.
/// Enough for the homography fit to outvote a moving object, few enough that
/// the long way stays a small share of the frame's cost.
const PROBES: usize = 48;

/// Below this many agreeing probes the frame's motion is not trusted, and
/// every speck is followed the long way as it always was.
const MIN_AGREEING: usize = 12;

/// How far, in whole pixels of the coarsest pyramid level, the whole frame's
/// slide is looked for when the tracks have lost it: eight pixels there is
/// sixty-four on a four-level pyramid, more than a fast handheld pan moves
/// between two frames.
const REACH: i64 = 8;

/// How many pixels wider on each side the window is for the second try at a
/// track on a step that lost hold of the frame's motion.
const WIDER: i64 = 2;

/// One speck followed forward and back, where it was followed at all: where
/// it landed, and how far the return missed where it started.
type Followed = Option<(Step, f64)>;

/// End the track at `index` and drop it from the live list (the `false` this
/// hands back to `retain_mut`). Every phase-1 failure (a singular solve, an
/// exclusion mask, forward-backward, NCC) lands here: a track that fails is
/// never teleported, it stops (docs/impl/tracking.md §2).
fn end_track(tracks: &mut [Track], index: usize) -> bool {
    if let Some(t) = tracks.get_mut(index) {
        t.state = TrackState::Ended;
    }
    false
}

/// One speck followed forward and back, through the finest `levels` of the
/// pyramid in a window `half` pixels either side: where it landed, and how far
/// the return missed where it started. `None` for a solve that found nothing.
fn follow(
    tracker: &Tracker,
    from: [f64; 2],
    start: [f64; 2],
    levels: usize,
    half: i64,
) -> Followed {
    let s = &tracker.settings;
    let solve = |a, b, at: [f64; 2], seed| {
        if s.affine {
            klt::solve(a, b, at, seed, half, s.max_iters, s.epsilon, levels)
        } else {
            klt::slide(a, b, at, seed, half, s.max_iters, s.epsilon, levels)
        }
    };
    let fwd = solve(&tracker.prev, &tracker.cur, from, start)?;
    // Forward–backward: follow the settled patch back and see whether it
    // returns to where it started. Seeded with the negated forward
    // displacement, so the backward solve starts from the right answer and the
    // test measures drift rather than the search's luck.
    let back_seed = [from[0] - fwd.pos[0], from[1] - fwd.pos[1]];
    let back = solve(&tracker.cur, &tracker.prev, fwd.pos, back_seed)?;
    let fb = (back.pos[0] - from[0]).hypot(back.pos[1] - from[1]);
    Some((fwd, fb))
}

impl Tracker {
    /// The frame's own motion, measured from a sample of live tracks followed
    /// through the whole pyramid, each started from `start` (its guess at the
    /// track's displacement): the warp most of them agree on, and what each
    /// sampled track found, by its place in the live list, so the main pass
    /// does not follow it twice.
    ///
    /// `None` when the tracker was not asked to seed, or too few samples
    /// agreed on any warp.
    fn measure(
        &self,
        frame: i64,
        start: &dyn Fn(&LiveTrack) -> [f64; 2],
    ) -> Option<(Mat3, Vec<Followed>)> {
        let s = &self.settings;
        if s.seeded_levels == 0 || self.live.len() < MIN_AGREEING {
            return None;
        }
        let every = (self.live.len() / PROBES).max(1);
        let mut found: Vec<Followed> = vec![None; self.live.len()];
        let mut pts: Vec<Correspondence> = Vec::with_capacity(PROBES + 1);
        for (i, l) in self.live.iter().enumerate().step_by(every) {
            let Some((fwd, fb)) = follow(self, l.pos, start(l), s.levels, s.half_window as i64)
            else {
                continue;
            };
            if fb < s.fb_max {
                let id = self.tracks.get(l.index).map_or(0, |t| t.id);
                pts.push(Correspondence {
                    id,
                    from: l.pos,
                    to: fwd.pos,
                });
            }
            found[i] = Some((fwd, fb));
        }
        if pts.len() < MIN_AGREEING {
            return None;
        }
        let size = self.size.unwrap_or((1, 1));
        let last = self.last_frame.unwrap_or(frame - 1);
        let (h, inliers) =
            homography_ransac(&pts, size, last, frame, &GeometrySettings::default())?;
        (inliers.len() >= MIN_AGREEING).then_some((h, found))
    }

    /// The whole frame's slide from `prev` to `cur`, for when the tracks'
    /// own guesses have lost it: every whole-pixel shift within [`REACH`] tried
    /// on the coarsest level, by the mean absolute difference of the middle of
    /// the frame, then refined a pixel either way on the level below. In
    /// level-0 pixels; `None` on a pyramid too small to search.
    fn coarse_slide(&self) -> Option<[f64; 2]> {
        let top = self
            .prev
            .levels
            .len()
            .min(self.cur.levels.len())
            .checked_sub(1)?;
        let best = |level: usize, around: [i64; 2], reach: i64| -> Option<[i64; 2]> {
            let (a, b) = (self.prev.levels.get(level)?, self.cur.levels.get(level)?);
            let (w, h) = (a.w.min(b.w) as i64, a.h.min(b.h) as i64);
            let margin = reach + around[0].abs().max(around[1].abs());
            if w <= 2 * margin + 4 || h <= 2 * margin + 4 {
                return None;
            }
            let mut found: Option<([i64; 2], f64)> = None;
            for dy in -reach..=reach {
                for dx in -reach..=reach {
                    let (sx, sy) = (around[0] + dx, around[1] + dy);
                    let mut sum = 0.0f64;
                    for y in margin..h - margin {
                        let (ra, rb) = ((y * w) as usize, ((y + sy) * w) as usize);
                        for x in margin..w - margin {
                            let pa = a.data.get(ra + x as usize).copied().unwrap_or(0.0);
                            let pb = b.data.get(rb + (x + sx) as usize).copied().unwrap_or(0.0);
                            sum += f64::from((pa - pb).abs());
                        }
                    }
                    // Strictly better only, so the scan order breaks ties.
                    if found.is_none_or(|(_, s)| sum < s) {
                        found = Some(([sx, sy], sum));
                    }
                }
            }
            found.map(|(d, _)| d)
        };
        let coarse = best(top, [0, 0], REACH)?;
        let (level, d) = match top.checked_sub(1) {
            Some(below) => (
                below,
                best(below, [2 * coarse[0], 2 * coarse[1]], 1)
                    .unwrap_or([2 * coarse[0], 2 * coarse[1]]),
            ),
            None => (top, coarse),
        };
        let scale = (1usize << level) as f64;
        Some([d[0] as f64 * scale, d[1] as f64 * scale])
    }

    /// Where one live track went, by its place `i` in the live list: what the
    /// measurement already found for it, or followed from where the frame's
    /// motion `h` puts it on the finer levels, or from the flow seed, or the
    /// long way from `start`.
    #[allow(clippy::too_many_arguments)]
    fn outcome(
        &self,
        i: usize,
        l: &LiveTrack,
        measured: Option<&(Mat3, Vec<Followed>)>,
        seed: Option<FlowSeed<'_>>,
        start: [f64; 2],
        fewer: usize,
    ) -> Followed {
        let half = self.settings.half_window as i64;
        let sampled = measured.and_then(|(_, found)| found.get(i).copied().flatten());
        match (sampled, measured, seed) {
            (Some(done), _, _) => Some(done),
            (None, Some((h, _)), _) => {
                let start = project(h, l.pos).map_or(l.vel, |p| [p[0] - l.pos[0], p[1] - l.pos[1]]);
                follow(self, l.pos, start, fewer, half)
            }
            (None, None, Some(f)) => follow(self, l.pos, f.at(l.pos[0], l.pos[1]), fewer, half),
            (None, None, None) => follow(self, l.pos, start, self.settings.levels, half),
        }
    }

    /// Carry every live track from `prev` to `cur`, verifying each
    /// (§2: forward–backward, then NCC; a failure ends the track).
    ///
    /// Where the frame's motion is measured, a step that lost hold of it is
    /// tried again from the whole frame's slide: when the measurement found
    /// no warp, and when most tracks failed under the one it found. Only those
    /// steps pay for the search, and only the tracks that failed are followed
    /// again, once, in a wider window.
    pub(crate) fn advance(&mut self, frame: i64, seed: Option<FlowSeed<'_>>) {
        let all = self.settings.levels;
        let fewer = match self.settings.seeded_levels {
            0 => all,
            n => n.min(all),
        };
        let measuring = seed.is_none() && self.settings.seeded_levels > 0;
        let mut slide: Option<[f64; 2]> = None;
        let mut measured = if measuring {
            self.measure(frame, &|l| l.vel)
        } else {
            None
        };
        if measuring && measured.is_none() && self.live.len() >= MIN_AGREEING {
            slide = self.coarse_slide();
            if let Some(d) = slide {
                measured = self.measure(frame, &|_| d);
            }
        }
        let held = |o: &Followed, fb_max: f64| o.is_some_and(|(_, fb)| fb < fb_max);
        let fb_max = self.settings.fb_max;
        let mut outcomes: Vec<Followed> = self
            .live
            .iter()
            .enumerate()
            .map(|(i, l)| {
                self.outcome(i, l, measured.as_ref(), seed, slide.unwrap_or(l.vel), fewer)
            })
            .collect();
        let kept = outcomes.iter().filter(|o| held(o, fb_max)).count();
        let collapsed = 2 * kept < self.live.len() && self.live.len() >= MIN_AGREEING;
        if measuring && slide.is_none() && collapsed {
            slide = self.coarse_slide();
            if let Some(d) = slide {
                measured = self.measure(frame, &|_| d);
            }
        }
        // A step that lost hold of the frame's motion: the tracks that failed
        // are tried once more, from the motion found again, in a wider window.
        if measuring && slide.is_some() {
            let half = self.settings.half_window as i64 + WIDER;
            for (i, l) in self.live.iter().enumerate() {
                if outcomes.get(i).is_none_or(|o| held(o, fb_max)) {
                    continue;
                }
                let (start, levels) = match &measured {
                    Some((h, _)) => (
                        project(h, l.pos).map_or(l.vel, |p| [p[0] - l.pos[0], p[1] - l.pos[1]]),
                        fewer,
                    ),
                    None => (slide.unwrap_or(l.vel), all),
                };
                let retry = follow(self, l.pos, start, levels, half);
                if held(&retry, fb_max) {
                    outcomes[i] = retry;
                }
            }
        }
        let mut live = std::mem::take(&mut self.live);
        let mut index = 0usize;
        // `retain_mut` keeps the survivors in order, so ids stay ascending and
        // the next frame walks them in the same sequence (§1's determinism).
        live.retain_mut(|l| {
            let i = index;
            index += 1;
            let Some((fwd, fb)) = outcomes.get(i).copied().flatten() else {
                return end_track(&mut self.tracks, l.index);
            };
            if exclude::excluded(&self.masks, fwd.pos[0], fwd.pos[1]) || fb >= fb_max {
                return end_track(&mut self.tracks, l.index);
            }
            let half = self.settings.half_window as i64;
            let Some(now) = self
                .cur
                .levels
                .first()
                .and_then(|p| klt::patch(p, fwd.pos, half))
            else {
                return end_track(&mut self.tracks, l.index);
            };
            let ncc = klt::ncc(&l.reference, &now);
            if ncc < self.settings.ncc_floor {
                return end_track(&mut self.tracks, l.index);
            }
            if ncc < self.settings.ncc_refresh {
                l.reference = now;
            }
            let Some(t) = self.tracks.get_mut(l.index) else {
                return false;
            };
            t.points.push(TrackPoint {
                frame,
                x: fwd.pos[0],
                y: fwd.pos[1],
            });
            t.steps.push(TrackStep { a: fwd.a, ncc, fb });
            l.vel = [fwd.pos[0] - l.pos[0], fwd.pos[1] - l.pos[1]];
            l.pos = fwd.pos;
            true
        });
        self.live = live;
    }
}

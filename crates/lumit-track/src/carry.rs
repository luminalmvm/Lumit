//! Carrying every live track from one frame to the next
//! (docs/impl/tracking.md §2).

use crate::exclude;
use crate::klt;
use crate::{FlowSeed, Track, TrackPoint, TrackState, TrackStep, Tracker};

/// End the track at `index` and drop it from the live list (the `false` this
/// hands back to `retain_mut`). Every phase-1 failure — a singular solve, an
/// exclusion mask, forward–backward, NCC — lands here: a track that fails is
/// never teleported, it stops (docs/impl/tracking.md §2).
fn end_track(tracks: &mut [Track], index: usize) -> bool {
    if let Some(t) = tracks.get_mut(index) {
        t.state = TrackState::Ended;
    }
    false
}

impl Tracker {
    /// Carry every live track from `prev` to `cur`, verifying each
    /// (§2: forward–backward, then NCC; a failure ends the track).
    pub(crate) fn advance(&mut self, frame: i64, seed: Option<FlowSeed<'_>>) {
        let Tracker {
            settings,
            masks,
            prev,
            cur,
            tracks,
            live,
            ..
        } = self;
        let half = settings.half_window as i64;
        // `retain_mut` keeps the survivors in order, so ids stay ascending and
        // the next frame walks them in the same sequence (§1's determinism).
        live.retain_mut(|l| {
            let start = seed.map_or(l.vel, |f| f.at(l.pos[0], l.pos[1]));
            let Some(fwd) = klt::solve(
                prev,
                cur,
                l.pos,
                start,
                half,
                settings.max_iters,
                settings.epsilon,
            ) else {
                return end_track(tracks, l.index);
            };
            if exclude::excluded(masks, fwd.pos[0], fwd.pos[1]) {
                return end_track(tracks, l.index);
            }
            // Forward–backward: follow the settled patch back and see whether it
            // returns to where it started. Seeded with the negated forward
            // displacement, so the backward solve starts from the right answer
            // and the test measures drift rather than the search's luck.
            let back_seed = [l.pos[0] - fwd.pos[0], l.pos[1] - fwd.pos[1]];
            let Some(back) = klt::solve(
                cur,
                prev,
                fwd.pos,
                back_seed,
                half,
                settings.max_iters,
                settings.epsilon,
            ) else {
                return end_track(tracks, l.index);
            };
            let fb = (back.pos[0] - l.pos[0]).hypot(back.pos[1] - l.pos[1]);
            if fb >= settings.fb_max {
                return end_track(tracks, l.index);
            }
            let Some(now) = cur
                .levels
                .first()
                .and_then(|p| klt::patch(p, fwd.pos, half))
            else {
                return end_track(tracks, l.index);
            };
            let ncc = klt::ncc(&l.reference, &now);
            if ncc < settings.ncc_floor {
                return end_track(tracks, l.index);
            }
            if ncc < settings.ncc_refresh {
                l.reference = now;
            }
            let Some(t) = tracks.get_mut(l.index) else {
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
    }
}

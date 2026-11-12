//! Writing what the live controls did into the document: one value let go
//! with the transport stopped, or a whole automation pass as keys.
//!
//! # In plain terms
//!
//! A pass is a list of `(time, value)` points per control, one for every
//! move of the pointer. Most of them say nothing a straight line between
//! their neighbours does not, so they are thinned first. What is left
//! becomes linear keys at exact sample times, replacing the keys the pass
//! covered. The curve either side is left exactly as it was, with a key at
//! each edge holding the old value where the new stretch does not start on
//! it. In Trim the points are changes, so the keys inside the stretch move by
//! the change at their time and keep the ride already written.

use crate::api::mixer::BridgeMixTarget;
use lumit_core::anim::{Animation, Keyframe, Property, SideInterp};
use lumit_core::mixer::AutomationMode;
use lumit_core::model::Document;
use lumit_core::ops::Op;
use lumit_core::time::Rational;
use std::sync::{Arc, RwLock};
use uuid::Uuid;

use super::automation::{layer_in, layer_of};

/// One control's points in a pass, in comp seconds.
pub(crate) struct Track {
    pub target: BridgeMixTarget,
    pub mode: AutomationMode,
    /// The points are changes from where the control was taken, as Trim
    /// writes them, rather than levels.
    pub trim: bool,
    /// The value heard at each time: dB or pan units, or the change.
    pub points: Vec<(f64, f64)>,
    /// Still being written.
    pub open: bool,
}

/// Points closer together than this are one point.
const CLOSEST_S: f64 = 0.020;

/// How far a dropped point may sit from the line that replaces it: dB for
/// a Volume, a send or the master, pan units for a Pan.
fn tolerance(target: BridgeMixTarget) -> f64 {
    match target {
        BridgeMixTarget::Pan { .. } => 0.5,
        _ => 0.2,
    }
}

/// Write a pass's `tracks` on `comp` as one undo step, "Record automation",
/// and switch a strip that wrote in Write to Touch inside it. `rate` is the
/// grid key times land on. A control whose layer is gone, or whose value is
/// driven by an expression, is passed over. True when anything was written.
pub(crate) fn write_pass(
    project: &Arc<RwLock<crate::api::state::LumitBridgeState>>,
    comp: Uuid,
    tracks: &[Track],
    rate: u32,
) -> bool {
    let Ok(state) = project.write() else {
        return false;
    };
    let store = &state.store;
    let own = store.begin_own_undo_group();
    let mut wrote = false;
    for track in tracks {
        let doc = store.snapshot();
        if let Some(op) = pass_op(&doc, comp, track, rate) {
            wrote |= store.commit(op).is_ok();
        }
    }
    let mut switched: Vec<Option<Uuid>> = Vec::new();
    for track in tracks.iter().filter(|k| k.mode == AutomationMode::Write) {
        let layer = layer_in(track.target);
        if switched.contains(&layer) {
            continue;
        }
        switched.push(layer);
        let op = match layer {
            Some(layer) => Op::SetLayerAutomation {
                comp,
                layer,
                mode: AutomationMode::Touch,
            },
            None => Op::SetMasterAutomationMode {
                comp,
                mode: AutomationMode::Touch,
            },
        };
        wrote |= store.commit(op).is_ok();
    }
    if own {
        store.name_undo_group("Record automation");
        store.end_undo_group();
    }
    wrote
}

/// The op that writes one track of a pass over `doc`.
fn pass_op(doc: &Document, comp: Uuid, track: &Track, rate: u32) -> Option<Op> {
    let c = doc.comp(comp)?;
    let trim = track.trim;
    let tol = tolerance(track.target);
    let points = thin(&track.points, tol);
    if let BridgeMixTarget::Master = track.target {
        let old = c
            .master_keys
            .clone()
            .unwrap_or(Animation::Static(c.master_volume_db));
        return Some(Op::SetMasterAutomation {
            comp,
            keys: Some(splice(&old, &points, trim, tol, rate)?),
        });
    }
    let l = layer_of(c, layer_in(track.target)?)?;
    // Keys are in layer time.
    let offset = l.start_offset.0.to_f64();
    let points: Vec<(f64, f64)> = points.iter().map(|&(t, v)| (t - offset, v)).collect();
    match track.target {
        BridgeMixTarget::Volume { layer } => Some(Op::SetLayerVolume {
            comp,
            layer,
            animation: splice(&l.volume_db.animation, &points, trim, tol, rate)?,
        }),
        BridgeMixTarget::Pan { layer } => Some(Op::SetLayerPan {
            comp,
            layer,
            animation: splice(&l.pan.animation, &points, trim, tol, rate)?,
        }),
        BridgeMixTarget::Send { layer, bus } => {
            let mut routes = l.audio_routes.clone();
            let send = routes.sends.iter_mut().find(|s| s.bus == bus)?;
            let keys = splice(&send.level_db.animation, &points, trim, tol, rate)?;
            send.level_db.set_animation(keys);
            Some(Op::SetLayerRoutes {
                comp,
                layer,
                routes,
            })
        }
        BridgeMixTarget::Master => None,
    }
}

/// `points` with the ones that add nothing taken out: any closer than 20 ms
/// to the one kept before it, then any within `tol` of the straight line
/// between the points either side of the run it sits in. The first and last
/// always stay.
fn thin(points: &[(f64, f64)], tol: f64) -> Vec<(f64, f64)> {
    let mut sorted = points.to_vec();
    sorted.sort_by(|a, b| a.0.total_cmp(&b.0));
    let (Some(&first), Some(&last)) = (sorted.first(), sorted.last()) else {
        return Vec::new();
    };
    if sorted.len() < 2 {
        return sorted;
    }
    // Of the moves inside 20 ms of one kept, the last stays in its place: it
    // is where the control got to, which a hold after it goes on from. The
    // very first point stays as it is.
    let mut spaced = vec![first];
    let mut since = first.0;
    for &p in &sorted[1..sorted.len() - 1] {
        if p.0 - since >= CLOSEST_S {
            spaced.push(p);
            since = p.0;
        } else if spaced.len() > 1 {
            if let Some(k) = spaced.last_mut() {
                *k = p;
            }
        }
    }
    if spaced.len() > 1 && spaced.last().is_some_and(|k| last.0 - k.0 < CLOSEST_S) {
        spaced.pop();
    }
    if last.0 > first.0 {
        spaced.push(last);
    }
    // Where a control comes to rest, and where it leaves it: a hold's two
    // ends are kept, so it replays flat from the moment it stopped moving.
    let held = |i: usize| spaced.get(i).map(|p| p.1);
    let still: Vec<bool> = (0..spaced.len())
        .map(|i| {
            let here = held(i);
            let before = i.checked_sub(1).and_then(held);
            (held(i + 1) == here) != (before == here)
        })
        .collect();
    // Each kept point reaches as far ahead as a straight line can go with
    // every point it passes over inside the tolerance, and no hold's end.
    let fits = |a: usize, b: usize| {
        let (pa, pb) = (spaced[a], spaced[b]);
        !still[a + 1..b].iter().any(|s| *s)
            && spaced[a + 1..b].iter().all(|p| {
                let line = pa.1 + (pb.1 - pa.1) * (p.0 - pa.0) / (pb.0 - pa.0);
                (p.1 - line).abs() <= tol
            })
    };
    let mut kept = vec![spaced[0]];
    let mut a = 0;
    let mut b = 2;
    while b < spaced.len() {
        if fits(a, b) {
            b += 1;
        } else {
            a = b - 1;
            kept.push(spaced[a]);
            b = a + 2;
        }
    }
    if spaced.len() > 1 {
        kept.push(spaced[spaced.len() - 1]);
    }
    kept
}

/// The value of the straight lines through `points` at `t`, held flat
/// beyond them.
fn between(points: &[(f64, f64)], t: f64) -> f64 {
    let i = points.partition_point(|p| p.0 <= t);
    match (i.checked_sub(1).and_then(|i| points.get(i)), points.get(i)) {
        (Some(a), Some(b)) if b.0 > a.0 => a.1 + (b.1 - a.1) * (t - a.0) / (b.0 - a.0),
        (Some(a), _) => a.1,
        (None, Some(b)) => b.1,
        (None, None) => 0.0,
    }
}

/// A linear key.
fn key(time: Rational, value: f64) -> Keyframe {
    Keyframe {
        time,
        value,
        interp_in: SideInterp::Linear,
        interp_out: SideInterp::Linear,
    }
}

/// `old` with the stretch `points` covers (in its own time) replaced by
/// linear keys through them, or, with `trim`, moved by them. The curve
/// outside the stretch is unchanged. None for an expression, which keys
/// cannot be written into.
fn splice(
    old: &Animation,
    points: &[(f64, f64)],
    trim: bool,
    tol: f64,
    rate: u32,
) -> Option<Animation> {
    if matches!(old, Animation::Expression(_)) {
        return None;
    }
    let (&(t0, _), &(t1, _)) = (points.first()?, points.last()?);
    if t1 <= t0 {
        // One moment is no ride.
        return Some(old.clone());
    }
    let was = Property {
        animation: old.clone(),
        extra: serde_json::Map::new(),
    };
    let grid = i64::from(rate.max(1));
    let time = |t: f64| Rational::from_f64_on_grid(t, grid).ok();
    let step = 1.0 / grid as f64;

    // The new values over the stretch.
    let mut new: Vec<(f64, f64)> = if trim {
        let mut moved: Vec<(f64, f64)> = points
            .iter()
            .map(|&(t, change)| (t, was.value_at(t) + change))
            .collect();
        if let Animation::Keyframed(keys) = old {
            moved.extend(
                keys.iter()
                    .map(|k| k.time.to_f64())
                    .filter(|&t| t > t0 && t < t1)
                    .map(|t| (t, was.value_at(t) + between(points, t))),
            );
        }
        moved.sort_by(|a, b| a.0.total_cmp(&b.0));
        moved
    } else {
        points.to_vec()
    };
    let (&(_, first), &(_, last)) = (new.first()?, new.last()?);
    // Where the stretch does not start (or end) on the old curve, the old
    // value is kept at the edge and the step is made over a short ramp just
    // inside it, so the curve outside is unchanged and no key sits a sample
    // from another.
    let step0 = (first - was.value_at(t0)).abs() > tol;
    let step1 = (last - was.value_at(t1)).abs() > tol;
    let n = new.len();
    if n >= 2 && step0 {
        let t = t0 + CLOSEST_S.min((new[1].0 - t0) / 2.0);
        new[0] = (t, between(&new, t));
    }
    if n >= 2 && step1 {
        let t = t1 - CLOSEST_S.min((t1 - new[n - 2].0) / 2.0);
        new[n - 1] = (t, between(&new, t));
    }
    let (r0, r1) = (time(t0)?, time(t1)?);

    // The old keys split at both edges, so the curve outside keeps its shape.
    let mut split = was.clone();
    split.insert_key_preserving_shape(r0);
    split.insert_key_preserving_shape(r1);
    let old_keys = match split.animation {
        Animation::Keyframed(keys) => keys,
        _ => Vec::new(),
    };
    let (mut outer_in, mut outer_out) = (SideInterp::Linear, SideInterp::Linear);
    let mut kept: Vec<Keyframe> = Vec::new();
    let (mut have0, mut have1) = (false, false);
    for mut k in old_keys {
        if k.time < r0 || k.time > r1 {
            kept.push(k);
        } else if k.time == r0 {
            if step0 {
                k.interp_out = SideInterp::Linear;
                kept.push(k);
                have0 = true;
            } else {
                outer_in = k.interp_in;
            }
        } else if k.time == r1 {
            if step1 {
                k.interp_in = SideInterp::Linear;
                kept.push(k);
                have1 = true;
            } else {
                outer_out = k.interp_out;
            }
        }
    }
    if step0 && !have0 {
        kept.push(key(r0, was.value_at(t0)));
    }
    if step1 && !have1 {
        kept.push(key(r1, was.value_at(t1)));
    }

    // Each key, and whether it is outside the stretch.
    let mut keys: Vec<(Keyframe, bool)> = Vec::with_capacity(n + kept.len());
    for (i, (t, v)) in new.drain(..).enumerate() {
        let mut k = key(time(t)?, v);
        if i == 0 && !step0 {
            k.interp_in = outer_in;
        }
        if i + 1 == n && !step1 {
            k.interp_out = outer_out;
        }
        keys.push((k, false));
    }
    keys.extend(kept.into_iter().map(|k| (k, true)));
    keys.sort_by_key(|(k, _)| k.time);
    // Keys a sample or less apart are one key. Where one of them is outside
    // the stretch it stays, so the curve there is unchanged, and its side
    // facing the stretch turns linear like the stretch's own; of two new
    // ones the first stays.
    let mut merged: Vec<(Keyframe, bool)> = Vec::with_capacity(keys.len());
    for (k, outside) in keys {
        let Some((last, last_outside)) = merged.last_mut() else {
            merged.push((k, outside));
            continue;
        };
        // So are an outside key and a new one closer than the thinning
        // keeps points that say the same within its tolerance.
        let gap = k.time.to_f64() - last.time.to_f64();
        let same = gap < CLOSEST_S && (k.value - last.value).abs() <= tol;
        if (gap > step * 1.000_001 && !(same && outside != *last_outside))
            || (outside && *last_outside)
        {
            merged.push((k, outside));
        } else if outside {
            let mut k = k;
            k.interp_in = SideInterp::Linear;
            *last = k;
            *last_outside = true;
        } else if *last_outside {
            last.interp_out = SideInterp::Linear;
        }
    }
    // A step made as a ramp leaves a new key just inside the edge. Where
    // the step was small enough that the line from the edge to the next new
    // key passes within the tolerance of it, it says nothing and goes.
    let within = |a: &Keyframe, b: &Keyframe, c: &Keyframe| {
        let (ta, tb, tc) = (a.time.to_f64(), b.time.to_f64(), c.time.to_f64());
        tc > ta && (b.value - (a.value + (c.value - a.value) * (tb - ta) / (tc - ta))).abs() <= tol
    };
    if step0 {
        if let Some(i) = merged.iter().position(|(k, o)| *o && k.time == r0) {
            let quiet = matches!(
                merged.get(i..i + 3),
                Some([(a, _), (b, false), (c, false)]) if within(a, b, c)
            );
            if quiet {
                merged.remove(i + 1);
            }
        }
    }
    if step1 {
        if let Some(i) = merged.iter().rposition(|(k, o)| *o && k.time == r1) {
            let quiet = i >= 2
                && matches!(
                    merged.get(i - 2..=i),
                    Some([(a, false), (b, false), (c, _)]) if within(a, b, c)
                );
            if quiet {
                merged.remove(i - 1);
            }
        }
    }
    let keys: Vec<Keyframe> = merged.into_iter().map(|(k, _)| k).collect();
    Some(Animation::Keyframed(keys))
}

/// The op that writes `value` for `target` the way the mixer writes a fader
/// let go: a key at the playhead (comp second `t`, on the comp's frame) when
/// the property is keyed, else its static value. None when there is nothing
/// to write it into.
pub(crate) fn value_op(
    doc: &Document,
    comp: Uuid,
    target: BridgeMixTarget,
    t: f64,
    value: f64,
) -> Option<Op> {
    if !value.is_finite() {
        return None;
    }
    let c = doc.comp(comp)?;
    let frame = (t.max(0.0) * c.frame_rate.fps()).round() as i64;
    let at = c.frame_rate.time_of_frame(frame).ok()?.0;
    if let BridgeMixTarget::Master = target {
        return Some(match &c.master_keys {
            Some(keys @ Animation::Keyframed(_)) => Op::SetMasterAutomation {
                comp,
                keys: Some(key_at(keys, at, value)?),
            },
            _ => Op::SetMasterVolume { comp, db: value },
        });
    }
    let l = layer_of(c, layer_in(target)?)?;
    let at = at.checked_sub(l.start_offset.0).ok()?;
    match target {
        BridgeMixTarget::Volume { layer } => Some(Op::SetLayerVolume {
            comp,
            layer,
            animation: key_at(&l.volume_db.animation, at, value)?,
        }),
        BridgeMixTarget::Pan { layer } => Some(Op::SetLayerPan {
            comp,
            layer,
            animation: key_at(&l.pan.animation, at, value)?,
        }),
        BridgeMixTarget::Send { layer, bus } => {
            let mut routes = l.audio_routes.clone();
            let send = routes.sends.iter_mut().find(|s| s.bus == bus)?;
            let keys = key_at(&send.level_db.animation, at, value)?;
            send.level_db.set_animation(keys);
            Some(Op::SetLayerRoutes {
                comp,
                layer,
                routes,
            })
        }
        BridgeMixTarget::Master => None,
    }
}

/// `old` holding `value` at `t`: the key there moved, or a new one, when it
/// is keyed; the value itself when it is still.
fn key_at(old: &Animation, t: Rational, value: f64) -> Option<Animation> {
    match old {
        Animation::Static(_) => Some(Animation::Static(value)),
        Animation::Keyframed(keys) => {
            let mut keys = keys.clone();
            match keys.iter_mut().find(|k| k.time == t) {
                Some(k) => k.value = value,
                None => {
                    let i = keys.partition_point(|k| k.time < t);
                    keys.insert(i, key(t, value));
                }
            }
            Some(Animation::Keyframed(keys))
        }
        Animation::Expression(_) => None,
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use crate::api::state::LumitBridgeState;

    /// **A recorded pass is not lost.** A Touch ride over a keyed Volume is
    /// written as keys that give back the ride within the thinning
    /// tolerance, the curve either side of it is exactly what it was, and
    /// one undo takes the whole pass back.
    #[test]
    fn a_recorded_pass_commits_keys_that_give_the_ride_back_and_undo_in_one_step() {
        let project = LumitBridgeState::new_project(None).expect("a project");
        let comp = project.new_composition("Mix".into(), None).expect("comp");
        let layer = comp.add_solid_layer(None).expect("a layer");
        let state = project.state().expect("state");
        let old = Animation::Keyframed(vec![
            key(Rational::new(0, 1).unwrap(), 0.0),
            key(Rational::new(10, 1).unwrap(), -10.0),
        ]);
        state
            .read()
            .unwrap()
            .store
            .commit(Op::SetLayerVolume {
                comp: comp.id,
                layer: layer.layer_id,
                animation: old.clone(),
            })
            .expect("keyed");
        let volume = || {
            let doc = state.read().unwrap().store.snapshot();
            let c = doc.comp(comp.id).unwrap();
            layer_of(c, layer.layer_id).unwrap().volume_db.clone()
        };
        let was = volume();

        // Two seconds of a ride that leaves the keys and comes back to them,
        // a point every ten milliseconds as the pointer moves.
        let ride = |t: f64| was.value_at(t) + 3.0 * (std::f64::consts::PI * (t - 2.0) / 2.0).sin();
        let points: Vec<(f64, f64)> = (0..=200)
            .map(|i| 2.0 + f64::from(i) * 0.01)
            .map(|t| (t, ride(t)))
            .collect();
        let track = Track {
            target: BridgeMixTarget::Volume {
                layer: layer.layer_id,
            },
            mode: AutomationMode::Touch,
            trim: false,
            points: points.clone(),
            open: false,
        };
        assert!(write_pass(&state, comp.id, &[track], 48_000));

        let now = volume();
        for &(t, v) in &points {
            assert!(
                (now.value_at(t) - v).abs() <= 0.2 + 1e-6,
                "at {t} s the keys give {} for a ride of {v}",
                now.value_at(t)
            );
        }
        for t in [0.0, 0.5, 1.0, 1.99, 4.01, 6.0, 9.5, 12.0] {
            assert!(
                (now.value_at(t) - was.value_at(t)).abs() < 1e-9,
                "outside the pass at {t} s the curve moved"
            );
        }
        let Animation::Keyframed(keys) = &now.animation else {
            panic!("keyed");
        };
        assert!(keys.len() < points.len() / 4, "the ride was thinned");

        project.undo().expect("undone");
        assert_eq!(volume(), was, "one undo takes the whole pass back");

        // In Trim the points are changes: the keys already there are heard
        // moved by them. A control taken before playback began holds levels
        // even on a strip in Trim, and those are written as levels.
        let change = |t: f64| 2.0 * (std::f64::consts::PI * (t - 5.0) / 2.0).sin();
        let trims: Vec<(f64, f64)> = (0..=200)
            .map(|i| 5.0 + f64::from(i) * 0.01)
            .map(|t| (t, change(t)))
            .collect();
        let levels: Vec<(f64, f64)> = (0..=100)
            .map(|i| 8.0 + f64::from(i) * 0.01)
            .map(|t| (t, -20.0 + t))
            .collect();
        let target = BridgeMixTarget::Volume {
            layer: layer.layer_id,
        };
        let passes = [
            Track {
                target,
                mode: AutomationMode::Trim,
                trim: true,
                points: trims.clone(),
                open: false,
            },
            Track {
                target,
                mode: AutomationMode::Trim,
                trim: false,
                points: levels.clone(),
                open: false,
            },
        ];
        assert!(write_pass(&state, comp.id, &passes, 48_000));
        let now = volume();
        for &(t, c) in &trims {
            let want = was.value_at(t) + c;
            assert!(
                (now.value_at(t) - want).abs() <= 0.2 + 1e-6,
                "at {t} s the trim gives {} for {want}",
                now.value_at(t)
            );
        }
        for &(t, v) in &levels {
            // They start and end off the curve: each step is a short ramp
            // just inside them.
            if !(8.02..=8.98).contains(&t) {
                continue;
            }
            assert!(
                (now.value_at(t) - v).abs() <= 0.2 + 1e-6,
                "at {t} s a level taken before playback gives {} for {v}",
                now.value_at(t)
            );
        }
        project.undo().expect("undone");
        assert_eq!(volume(), was, "and one undo takes the trim pass back");

        // A ride taken a sample after a key, on the curve, and a hold that
        // starts and ends off it: one key per sample, the curve outside as
        // it was, and the hold given back away from its edges.
        let step = 1.0 / 48_000.0;
        let near: Vec<(f64, f64)> = (0..=200)
            .map(|i| step + f64::from(i) * 0.01)
            .map(|t| {
                let ride = 3.0 * (std::f64::consts::PI * (t - step) / 2.0).sin();
                (t, was.value_at(t) - ride)
            })
            .collect();
        let held: Vec<(f64, f64)> = (0..=100)
            .map(|i| 6.0 + f64::from(i) * 0.01)
            .map(|t| (t, was.value_at(t) + 3.0))
            .collect();
        let rides = [near.clone(), held.clone()].map(|points| Track {
            target,
            mode: AutomationMode::Latch,
            trim: false,
            points,
            open: false,
        });
        assert!(write_pass(&state, comp.id, &rides, 48_000));
        let now = volume();
        let Animation::Keyframed(keys) = &now.animation else {
            panic!("keyed");
        };
        for pair in keys.windows(2) {
            assert!(
                pair[1].time.to_f64() - pair[0].time.to_f64() > step * 1.5,
                "two keys at {} s and {} s",
                pair[0].time.to_f64(),
                pair[1].time.to_f64()
            );
        }
        for t in [0.0, 5.0, 5.99, 6.0, 7.0, 7.01, 9.5, 12.0] {
            assert!(
                (now.value_at(t) - was.value_at(t)).abs() < 1e-9,
                "outside the passes at {t} s the curve moved"
            );
        }
        for &(t, v) in near.iter().chain(&held) {
            if (6.0..6.02).contains(&t) || t > 6.98 && t <= 7.0 {
                continue;
            }
            assert!(
                (now.value_at(t) - v).abs() <= 0.2 + 1e-6,
                "at {t} s the keys give {} for {v}",
                now.value_at(t)
            );
        }
        project.undo().expect("undone");
        assert_eq!(volume(), was, "and the last pass goes back too");

        // A Touch ride as the mixer records it: taken on the curve and held
        // still a moment, pulled down to -12 dB by 3.05 s, held there until
        // let go at 3.51 s (the clock noting the hold), then the glide back.
        // The hold replays flat from where the fader stopped, and the still
        // start leaves one key.
        let mut ride: Vec<(f64, f64)> = vec![(2.0, -2.0), (2.01, -2.01), (2.05, -2.05)];
        ride.extend((0..=99).map(|i| {
            let t = 2.06 + f64::from(i) * 0.01;
            // The last move is to -12 dB itself, as the held value is.
            let v = -2.06 + (t - 2.06) / 0.99 * (-12.0 + 2.06);
            (t, if i == 99 { -12.0 } else { v })
        }));
        ride.extend([(3.066, -12.0), (3.5, -12.0), (3.51, -12.0), (3.76, -3.76)]);
        let track = Track {
            target,
            mode: AutomationMode::Touch,
            trim: false,
            points: ride,
            open: false,
        };
        assert!(write_pass(&state, comp.id, &[track], 48_000));
        let now = volume();
        for i in 0..=46 {
            let t = 3.05 + f64::from(i) * 0.01;
            assert!(
                (now.value_at(t) + 12.0).abs() < 0.05,
                "held at -12 dB, the keys give {} at {t} s",
                now.value_at(t)
            );
        }
        let Animation::Keyframed(keys) = &now.animation else {
            panic!("keyed");
        };
        let start = keys
            .iter()
            .filter(|k| (1.99..2.055).contains(&k.time.to_f64()))
            .count();
        assert_eq!(start, 1, "the still start leaves one key: {keys:?}");
        project.undo().expect("undone");
        assert_eq!(volume(), was, "and the ride goes back");

        // A ride taken 2.35 ms after a key, on its value: the key holding
        // the curve outside stays, and the ride's first point goes.
        let late = 0.00235;
        let ride: Vec<(f64, f64)> = (0..=100)
            .map(|i| late + f64::from(i) * 0.01)
            .map(|t| {
                (
                    t,
                    was.value_at(t) - 2.0 * (std::f64::consts::PI * (t - late)).sin(),
                )
            })
            .collect();
        let track = Track {
            target,
            mode: AutomationMode::Touch,
            trim: false,
            points: ride,
            open: false,
        };
        assert!(write_pass(&state, comp.id, &[track], 48_000));
        let now = volume();
        let Animation::Keyframed(keys) = &now.animation else {
            panic!("keyed");
        };
        assert!(
            keys.iter()
                .any(|k| k.time.to_f64() == 0.0 && k.value == 0.0),
            "the key at 0 s stays: {keys:?}"
        );
        for pair in keys.windows(2) {
            let (a, b) = (&pair[0], &pair[1]);
            assert!(
                b.time.to_f64() - a.time.to_f64() >= 0.02 || (b.value - a.value).abs() > 0.2,
                "two keys saying the same at {} s and {} s",
                a.time.to_f64(),
                b.time.to_f64()
            );
        }
    }
}

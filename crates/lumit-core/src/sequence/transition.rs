//! What an overlap on a picture row draws as. A dissolve unless the incoming
//! clip names one of the Transition effects, and then that effect runs over the
//! outgoing clip with the incoming clip beneath it, swept from nothing to
//! everything across the overlap.
//!
//! The sweep is read off the same gain the dissolve reads ([`super::shown_at`]),
//! so a transition takes its curve from the clip's fade shape as a dissolve does.

use super::{with, Clip};
use crate::anim::{Animation, Property};
use crate::model::{EffectInstance, EffectValue};
use crate::time::Rational;
use uuid::Uuid;

/// The effects an overlap can be, by match name, in the order the menus list
/// them.
pub const KINDS: [&str; 6] = [
    "linear_wipe",
    "radial_wipe",
    "iris_wipe",
    "card_wipe",
    "venetian_blinds",
    "luma_fade",
];

/// The rows of a kind that the overlap decides for it, which an interface has
/// no reason to show: what the sweep writes, and the Mix, which would leave some
/// of the outgoing clip standing when the overlap ends.
#[must_use]
pub fn swept_rows(match_name: &str) -> &'static [&'static str] {
    match match_name {
        "iris_wipe" => &["outer_radius", "inner_radius", "mix"],
        _ => &["completion", "mix"],
    }
}

/// `effect` as it draws at `u` through its overlap, on a picture `width` by
/// `height`, its own keys read at `lt`. At 0 it has taken nothing away, and at 1
/// nothing of the outgoing clip is left.
///
/// An Iris wipe has no Completion to sweep, so its hole grows until the corners
/// of its polygon have passed every corner of the picture.
#[must_use]
pub fn swept(effect: &EffectInstance, u: f64, width: f64, height: f64, lt: f64) -> EffectInstance {
    // Every bound here is a constant, so `clamp` has no reversed pair to panic on.
    let u = u.clamp(0.0, 1.0);
    let mut out = effect.clone();
    if out.effect.match_name != "iris_wipe" {
        put(&mut out, "completion", u * 100.0);
        return out;
    }
    let float = |id: &str, or: f64| {
        effect
            .params
            .iter()
            .find(|p| p.id == id)
            .and_then(|p| match &p.value {
                EffectValue::Float(v) => Some(v.value_at(lt)),
                _ => None,
            })
            .unwrap_or(or)
    };
    let starred = effect
        .params
        .iter()
        .any(|p| p.id == "use_inner_radius" && matches!(p.value, EffectValue::Bool(true)));
    let (cx, cy) = (
        float("centre_x", width / 2.0),
        float("centre_y", height / 2.0),
    );
    let corner = (cx.max(width - cx)).hypot(cy.max(height - cy));
    let corners = float("points", 6.0).max(3.0);
    let inner = float("inner_radius", 165.0);
    let outer = float("outer_radius", 330.0).max(1.0);
    // The narrowest part of the shape has to clear the corner too: a polygon's
    // sides, or a star's pulled-in points.
    let waist = if starred {
        (inner / outer).clamp(0.1, 1.0)
    } else {
        (std::f64::consts::PI / corners).cos().max(0.1)
    };
    let reach = (corner + float("feather", 0.0)) / waist;
    put(&mut out, "outer_radius", u * reach);
    if starred {
        put(&mut out, "inner_radius", u * reach * inner / outer);
    }
    out
}

/// One Float row holding `value` at every moment.
fn put(effect: &mut EffectInstance, id: &str, value: f64) {
    for param in effect.params.iter_mut().filter(|p| p.id == id) {
        if let EffectValue::Float(v) = &mut param.value {
            *v = Property {
                animation: Animation::Static(value),
                extra: serde_json::Map::new(),
            };
        }
    }
}

/// Make the two clips at an edit point overlap by `frames` steps of `frame`,
/// centred on it. On a layer that draws, the overlap is a dissolve, and on an
/// audio-only one a crossfade.
///
/// `outgoing` ends where `incoming` starts, or the two already overlap and
/// the edit point is the middle of that. Each is carried on by half the
/// length into its own source, the outgoing clip's end later and the incoming
/// clip's start earlier, and the odd step goes to the incoming clip. `room`
/// is how far each edge may be carried from where it stands, the outgoing
/// clip's first, with None for no limit ([`Clip::spare`]). A clip with less
/// than its half gives what it has, in whole steps, so the overlap is the
/// longest the two allow. Neither edge passes the far end of the other clip
/// or reaches a third, so two clips at most cover any moment.
///
/// No `frames` trims an overlap back to its middle, and the clips abut again.
///
/// None when the two neither meet nor overlap, when a length was asked for
/// and neither clip has any to give, or when a map cannot be trimmed or
/// carried on.
pub fn dissolve(
    clips: &[Clip],
    outgoing: Uuid,
    incoming: Uuid,
    frames: i64,
    frame: Rational,
    room: (Option<Rational>, Option<Rational>),
) -> Option<Vec<Clip>> {
    let out = clips.iter().find(|c| c.id == outgoing)?;
    let inc = clips.iter().find(|c| c.id == incoming)?;
    let (start, end) = (inc.place_start, out.place_end());
    if frames < 0
        || frame <= Rational::ZERO
        || out.place_start > start
        || start > end
        || end > inc.place_end()
    {
        return None;
    }
    // The whole steps in a length, and the length of that many steps.
    let steps = |length: Rational| {
        let count = length.checked_div(frame).ok()?;
        Some(count.num().div_euclid(count.den()))
    };
    let span = |steps: i64| frame.checked_mul(Rational::new(steps, 1).ok()?).ok();
    let overlap = steps(end.checked_sub(start).ok()?)?;
    let point = end.checked_sub(span(overlap / 2)?).ok()?;

    // How far each edge may go: to the other clip's far end, to a third
    // clip, and as far as its own source runs.
    let others = || {
        clips
            .iter()
            .filter(|c| c.id != outgoing && c.id != incoming)
    };
    let later = others().map(|c| c.place_start).filter(|s| *s >= point);
    let mut latest = later.chain([inc.place_end()]).min()?;
    let earlier = others().map(Clip::place_end).filter(|e| *e <= point);
    let mut earliest = earlier.chain([out.place_start]).max()?;
    if let Some(room) = room.0 {
        latest = latest.min(end.checked_add(room).ok()?);
    }
    if let Some(room) = room.1 {
        earliest = earliest.max(start.checked_sub(room).ok()?);
    }
    let after = (frames / 2).min(steps(latest.checked_sub(point).ok()?)?);
    let before = (frames - frames / 2).min(steps(point.checked_sub(earliest).ok()?)?);
    if frames > 0 && after + before == 0 {
        return None;
    }
    let new_end = point.checked_add(span(after)?).ok()?;
    let new_start = point.checked_sub(span(before)?).ok()?;
    let out = match new_end.cmp(&end) {
        std::cmp::Ordering::Less => out.trim_end(new_end)?,
        std::cmp::Ordering::Greater => out.extend_end(new_end)?,
        std::cmp::Ordering::Equal => out.clone(),
    };
    let inc = match new_start.cmp(&start) {
        std::cmp::Ordering::Less => inc.extend_start(new_start)?,
        std::cmp::Ordering::Greater => inc.trim_start(new_start)?,
        std::cmp::Ordering::Equal => inc.clone(),
    };
    Some(with(clips, &[out, inc]))
}

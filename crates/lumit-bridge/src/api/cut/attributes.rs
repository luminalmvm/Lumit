//! Paste attributes, Vegas's Paste Event Attributes: the look of the clip
//! last copied given to other clips, which keep their media and their place.

use super::*;
use lumit_core::anim::{Animation, Keyframe, Property};
use lumit_core::sequence::Fade;

/// `to` wearing `from`'s attributes: its effects as instances of their own,
/// its fx switch, gain, opacity and blend mode, its fades held to `to`'s
/// length, and its speed line laid along `to`'s length at the same speeds.
/// None where the speed line cannot be laid: an expression, or overflow.
#[frb(ignore)]
fn given(from: &Clip, to: &Clip) -> Option<Clip> {
    let fade = |f: Fade| Fade {
        seconds: f.seconds.min(to.place_duration),
        ..f
    };
    let (retime, source_out) = match &from.retime {
        None => (None, to.source_in.checked_add(to.place_duration).ok()?),
        Some(map) => laid(map, from.place_duration, to)?,
    };
    Some(Clip {
        effects: from.duplicate().effects,
        fx: from.fx,
        gain_db: from.gain_db,
        opacity: from.opacity,
        blend: from.blend,
        fade_in: fade(from.fade_in),
        fade_out: fade(from.fade_out),
        retime,
        source_out,
        ..to.clone()
    })
}

/// A clip's map `length` long laid along `to`: each key at the same share of
/// the length, keeping its speed, starting from `to`'s own trim. The map and
/// the source moment it ends on.
#[frb(ignore)]
fn laid(map: &Property, length: Rational, to: &Clip) -> Option<(Option<Property>, Rational)> {
    let k = to.place_duration.checked_div(length).ok()?;
    let start = to.source_in.to_f64();
    let animation = match &map.animation {
        // A held frame holds `to`'s first.
        Animation::Static(_) => Animation::Static(start),
        Animation::Keyframed(keys) => {
            let first = keys.first()?.value;
            let mut laid = Vec::with_capacity(keys.len());
            for key in keys {
                laid.push(Keyframe {
                    time: key.time.checked_mul(k).ok()?,
                    // Times and values both scaled by `k`, so speeds stay.
                    value: start + (key.value - first) * k.to_f64(),
                    ..*key
                });
            }
            Animation::Keyframed(laid)
        }
        Animation::Expression(_) => return None,
    };
    let map = Property {
        animation,
        extra: map.extra.clone(),
    };
    let end = map.value_at(to.place_duration.to_f64());
    let out = Rational::from_f64_on_grid(end, Rational::FLICK_DEN).ok()?;
    Some((Some(map), out))
}

impl Edit<'_> {
    /// Give each named clip the attributes of the first clip copied.
    /// `Nothing` when nothing is copied in this project.
    pub(super) fn paste_attributes(mut self, clips: &[Uuid]) -> Cut {
        let from = {
            let copied = COPIED.lock().map_err(|_| BridgeError::ReadFailed)?;
            match copied.as_ref() {
                Some((project, held)) if *project == self.of.project_id() => {
                    held.first().map(|h| h.clip.clone())
                }
                _ => None,
            }
        }
        .ok_or(BridgeCutResult::Nothing)?;
        for id in clips {
            let (r, c) = self.find(*id)?;
            if self.rows[r].locked {
                return Err(BridgeCutResult::Locked.into());
            }
            let to = &mut self.rows[r].clips[c];
            *to = given(&from, to).ok_or(BridgeCutResult::Uncuttable)?;
        }
        self.commit()
    }
}

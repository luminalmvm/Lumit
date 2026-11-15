//! A transition between two layers of a composition that overlap in time: the
//! same kinds and looks a cut's overlap takes, keyed onto the layers.

use flutter_rust_bridge::frb;
use lumit_core::anim::{Animation, Keyframe, Property, SideInterp};
use lumit_core::model::{EffectInstance, EffectValue, Layer, LayerKind};
use lumit_core::ops::Op;
use lumit_core::sequence::transition::{self, Side, Sweep};
use lumit_core::time::{CompTime, Rational, TimeError};
use uuid::Uuid;

use crate::api::{composition::CompositionReference, BridgeError};

impl CompositionReference {
    /// Put a transition between two layers that overlap in time, as one undo
    /// step: the kind `kind` at `look`, its first with none. False when they
    /// don't overlap by two frames, when one sits inside the other, or when
    /// either has no picture or is locked. One put there before is replaced.
    ///
    /// The layer that starts first is going. A wipe runs on the upper layer
    /// only, and the rest on both. Where the two pictures cross, the upper
    /// layer's Opacity is keyed, unless it is already keyed.
    #[frb(sync)]
    pub fn transition_between(
        &self,
        first: Uuid,
        second: Uuid,
        kind: String,
        look: Option<u32>,
    ) -> Result<bool, BridgeError> {
        let comp = self.composition()?;
        let place = |id: Uuid| {
            comp.layers
                .iter()
                .position(|l| l.id == id)
                .ok_or(BridgeError::InvalidLayer)
        };
        let (a, b) = (place(first)?, place(second)?);
        let (Some(la), Some(lb)) = (comp.layers.get(a), comp.layers.get(b)) else {
            return Err(BridgeError::InvalidLayer);
        };
        let pictureless = |l: &Layer| {
            l.audio_only
                || matches!(
                    l.kind,
                    LayerKind::Null | LayerKind::Camera { .. } | LayerKind::Light { .. }
                )
        };
        if a == b || [la, lb].iter().any(|l| pictureless(l) || l.switches.locked) {
            return Ok(false);
        }
        // Going is the one that starts first, or with the same start, ends
        // first. Index 0 is the top of the stack.
        let ((going, gi), (coming, ci)) =
            if (la.in_point, la.out_point) <= (lb.in_point, lb.out_point) {
                ((la, a), (lb, b))
            } else {
                ((lb, b), (la, a))
            };
        // One inside the other would leave the outer one gone to its end.
        if coming.out_point < going.out_point {
            return Ok(false);
        }
        let start = coming.in_point.0;
        let end = going.out_point.0.min(coming.out_point.0);
        let two = comp
            .frame_rate
            .time_of_frame(2)
            .map_err(|_| BridgeError::InvalidFrameRate)?;
        let frame = comp
            .frame_rate
            .frame_duration()
            .map_err(|_| BridgeError::InvalidFrameRate)?
            .0;
        if end.checked_sub(start).map_or(true, |len| len < two.0) {
            return Ok(false);
        }
        let (w, h) = (f64::from(comp.width), f64::from(comp.height));
        // One set of rows for both layers, so a seeded order is the same on each.
        let fresh = transition::fresh(&kind, look.map(|l| l as usize), w, h)
            .ok_or(BridgeError::UnknownEffectName)?;
        let sweep = transition::kind(&kind)
            .ok_or(BridgeError::UnknownEffectName)?
            .sweep;
        let upper_going = gi < ci;
        let (upper, upper_side) = if upper_going {
            (going, Side::Out)
        } else {
            (coming, Side::In)
        };
        // From composition time into a layer's own, where its keys live.
        let local = |l: &Layer, t: Rational| {
            CompTime(t)
                .delta(l.start_offset)
                .map(|d| d.0)
                .map_err(|_| BridgeError::InvalidTime)
        };
        // A transition put here before is one whose keys start where this
        // one's do, and the new one takes its place.
        let earlier = |e: &EffectInstance, at: Rational| {
            transition::kind(&e.effect.match_name).is_some()
                && e.params.iter().any(|p| match &p.value {
                    EffectValue::Float(Property {
                        animation: Animation::Keyframed(keys),
                        ..
                    }) => keys.first().is_some_and(|k| k.time == at),
                    _ => false,
                })
        };
        let mut ops = Vec::new();
        let mut add = |l: &Layer, side: Option<Side>| -> Result<(), BridgeError> {
            let span = (local(l, start)?, local(l, end)?);
            let mut effects = l.effects.clone();
            effects.retain(|e| !earlier(e, span.0));
            if effects.len() == l.effects.len() && side.is_none() {
                return Ok(());
            }
            if let Some(side) = side {
                let mut keyed = transition::keyed(&fresh, side, span, frame, w, h);
                keyed.id = Uuid::now_v7();
                effects.push(keyed);
            }
            ops.push(Op::SetLayerEffects {
                comp: self.id,
                layer: l.id,
                effects,
            });
            Ok(())
        };
        let cross = match sweep {
            Sweep::Out | Sweep::Iris => {
                let lower = if upper_going { coming } else { going };
                add(upper, Some(upper_side))?;
                add(lower, None)?;
                0.0
            }
            Sweep::Both | Sweep::Peak { .. } => {
                add(going, Some(Side::Out))?;
                add(coming, Some(Side::In))?;
                match sweep {
                    Sweep::Peak { cross, .. } => cross,
                    _ => match fresh.param("cross") {
                        Some(EffectValue::Float(c)) => c.value_at(0.0) / 100.0,
                        _ => 0.0,
                    },
                }
            }
        };
        // The upper layer fades across `cross` of the overlap about its middle,
        // so the two pictures add up to a whole one.
        if let (true, Animation::Static(full)) = (cross > 0.0, &upper.transform.opacity.animation) {
            let bad = |_: TimeError| BridgeError::InvalidTime;
            let len = end.checked_sub(start).map_err(bad)?;
            let mid = Rational::new(1, 2)
                .and_then(|h| start.checked_add(len.checked_mul(h)?))
                .map_err(bad)?;
            let half = Rational::new((cross.min(1.0) * 1000.0).round() as i64, 2000)
                .and_then(|part| len.checked_mul(part))
                .map_err(bad)?;
            let times = (
                mid.checked_sub(half).map_err(bad)?,
                mid.checked_add(half).map_err(bad)?,
            );
            let (from, to) = if upper_going {
                (*full, 0.0)
            } else {
                (0.0, *full)
            };
            let key = |time, value| Keyframe {
                time,
                value,
                interp_in: SideInterp::Linear,
                interp_out: SideInterp::Linear,
            };
            ops.push(Op::SetTransformProperty {
                comp: self.id,
                layer: upper.id,
                prop: lumit_core::model::TransformProp::Opacity,
                animation: Animation::Keyframed(vec![
                    key(local(upper, times.0)?, from),
                    key(local(upper, times.1)?, to),
                ]),
            });
        }
        let proj = self.project()?;
        let proj = proj.write().map_err(|_| BridgeError::WriteFailed)?;
        proj.store
            .commit(Op::Batch { ops })
            .map_err(BridgeError::OpError)?;
        Ok(true)
    }
}

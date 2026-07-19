//! Keyframe-lane drag: turning a lane keyframe selection and its drag
//! delta into one batched shift operation across the affected layers.

use super::*;

/// One transform channel's bucket of local key times to shift (lane drag).
type TfShift = (
    (uuid::Uuid, lumit_core::model::TransformProp),
    Vec<lumit_core::Rational>,
);
/// One layer's bucket of (effect index, param index, local time) shifts.
type FxShift = (uuid::Uuid, Vec<(usize, usize, lumit_core::Rational)>);

/// The Y partner of a linked pair's X channel (Anchor/Position/Scale), or None.
/// A linked lane row keys both axes together, so a drag on it moves both.
pub(crate) fn linked_partner(
    p: lumit_core::model::TransformProp,
) -> Option<lumit_core::model::TransformProp> {
    use lumit_core::model::TransformProp::{
        AnchorX, AnchorY, PositionX, PositionY, ScaleX, ScaleY,
    };
    match p {
        AnchorX => Some(AnchorY),
        PositionX => Some(PositionY),
        ScaleX => Some(ScaleY),
        _ => None,
    }
}

/// Build the one op that slides every selected lane keyframe by `delta` seconds
/// (note 2.1). Transform-channel selections become one `SetTransformProperty`
/// per channel — a linked Anchor/Position/Scale row (listed in `linked`) also
/// moves its partner axis's key at the same time; effect-parameter selections
/// fold into one `SetLayerEffects` per layer. Several ops wrap in a Batch so the
/// whole slide is a single undo step. Returns None when nothing moves.
pub(crate) fn build_lane_drag_op(
    comp: &lumit_core::model::Composition,
    selection: &[crate::app_state::LaneKeySel],
    linked: &[(uuid::Uuid, lumit_core::model::TransformProp)],
    delta: f64,
    fps: f64,
) -> Option<lumit_core::Op> {
    use crate::app_state::PropRow;
    use lumit_core::anim::Animation;
    use lumit_core::model::TransformProp;

    // (layer, transform channel) -> the local times to shift on it.
    let mut tf: Vec<TfShift> = Vec::new();
    // layer -> (effect index, param index, local time) shifts.
    let mut fx: Vec<FxShift> = Vec::new();
    {
        let mut add_tf = |layer: uuid::Uuid, prop: TransformProp, t: lumit_core::Rational| match tf
            .iter_mut()
            .find(|((l, p), _)| *l == layer && *p == prop)
        {
            Some((_, ts)) => ts.push(t),
            None => tf.push(((layer, prop), vec![t])),
        };
        let mut add_fx =
            |layer: uuid::Uuid, effect: usize, param: usize, t: lumit_core::Rational| match fx
                .iter_mut()
                .find(|(l, _)| *l == layer)
            {
                Some((_, v)) => v.push((effect, param, t)),
                None => fx.push((layer, vec![(effect, param, t)])),
            };
        for s in selection {
            match s.row {
                PropRow::Transform(prop) => {
                    add_tf(s.layer, prop, s.time);
                    if linked.iter().any(|(l, p)| *l == s.layer && *p == prop) {
                        if let Some(partner) = linked_partner(prop) {
                            add_tf(s.layer, partner, s.time);
                        }
                    }
                }
                PropRow::Effect { effect, param } => add_fx(s.layer, effect, param, s.time),
                // The Retime channel has no draggable lane glyphs, so it never
                // appears in a lane selection.
                PropRow::Retime => {}
            }
        }
    }

    let mut ops: Vec<lumit_core::Op> = Vec::new();

    for ((layer_id, prop), times) in &tf {
        let Some(layer) = comp.layers.iter().find(|l| l.id == *layer_id) else {
            continue;
        };
        let Animation::Keyframed(keys) = &layer.transform.get(*prop).animation else {
            continue;
        };
        let new_keys = shift_keys_time(keys, times, delta, fps);
        ops.push(lumit_core::Op::SetTransformProperty {
            comp: comp.id,
            layer: *layer_id,
            prop: *prop,
            animation: Animation::Keyframed(new_keys),
        });
    }

    for (layer_id, shifts) in &fx {
        let Some(layer) = comp.layers.iter().find(|l| l.id == *layer_id) else {
            continue;
        };
        let mut effects = layer.effects.clone();
        let mut touched = false;
        // Distinct (effect, param) pairs, each shifted once with all its times.
        let mut seen: Vec<(usize, usize)> = Vec::new();
        for (e, p, _) in shifts {
            if !seen.contains(&(*e, *p)) {
                seen.push((*e, *p));
            }
        }
        for (e, p) in seen {
            let times: Vec<lumit_core::Rational> = shifts
                .iter()
                .filter(|(ee, pp, _)| *ee == e && *pp == p)
                .map(|(_, _, t)| *t)
                .collect();
            if let Some(param) = effects.get_mut(e).and_then(|inst| inst.params.get_mut(p)) {
                if let lumit_core::model::EffectValue::Float(prop) = &mut param.value {
                    if let Animation::Keyframed(keys) = &prop.animation {
                        prop.animation =
                            Animation::Keyframed(shift_keys_time(keys, &times, delta, fps));
                        touched = true;
                    }
                }
            }
        }
        if touched {
            ops.push(lumit_core::Op::SetLayerEffects {
                comp: comp.id,
                layer: *layer_id,
                effects,
            });
        }
    }

    match ops.len() {
        0 => None,
        1 => ops.pop(),
        _ => Some(lumit_core::Op::Batch { ops }),
    }
}

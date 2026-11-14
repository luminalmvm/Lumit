//! A layer's motion path as the Viewer draws it.

use lumit_core::anim::{Animation, Keyframe, Property};
use lumit_core::model::Layer;
use lumit_core::time::{CompTime, Rational};

use crate::api::layer::{rational_of, BridgeMotionKey, BridgeMotionPath};

/// The motion path of `layer` in `comp`, or `None` while its Position is
/// still on both axes — which is most layers, and the answer that costs
/// nothing.
pub(crate) fn motion_path_of(
    comp: &lumit_core::model::Composition,
    layer: &Layer,
) -> Option<BridgeMotionPath> {
    use lumit_core::anim::{resolved_side, SideInterp, MAX_INFLUENCE, MIN_INFLUENCE};

    fn keys_of(property: &Property) -> Option<&[Keyframe]> {
        match &property.animation {
            Animation::Keyframed(keys) if !keys.is_empty() => Some(keys.as_slice()),
            _ => None,
        }
    }
    let (px, py) = (&layer.transform.position_x, &layer.transform.position_y);
    let (kx, ky) = (keys_of(px), keys_of(py));
    // A 3D layer moving only in depth still has a path to show.
    let pz = layer
        .switches
        .three_d
        .then_some(&layer.transform.position_z);
    let kz = pz.and_then(keys_of);
    if kx.is_none() && ky.is_none() && kz.is_none() {
        return None;
    }
    let offset = layer.start_offset.0;
    let comp_time = |t: Rational| CompTime(t.checked_add(offset).unwrap_or(t));

    // Every time either axis has a key at, on the layer's clock, oldest first.
    let mut times: Vec<Rational> = kx
        .into_iter()
        .chain(ky)
        .chain(kz)
        .flatten()
        .map(|k| k.time)
        .collect();
    times.sort_by(|a, b| a.to_f64().total_cmp(&b.to_f64()));
    times.dedup();
    let first_frame = comp.frame_rate.frame_at(comp_time(*times.first()?));
    let last_frame = comp.frame_rate.frame_at(comp_time(*times.last()?));

    let mut samples = Vec::new();
    let mut depths = Vec::new();
    for frame in first_frame..=last_frame {
        let t = comp.frame_rate.time_of_frame(frame).ok()?.0;
        let local = lumit_core::time::layer_time(t.to_f64(), offset);
        samples.push(px.value_at(local));
        samples.push(py.value_at(local));
        depths.extend(pz.map(|z| z.value_at(local)));
    }

    // How far one axis' handle sits from its key on one side: the cubic's
    // first control point less the key, in that axis' own units. A straight
    // side is a handle a third of the way along the chord, as the evaluator
    // treats it; a held side has no span to draw a handle across.
    let reach = |keys: &[Keyframe], i: usize, out: bool| -> Option<f64> {
        let j = if out {
            i.checked_add(1)?
        } else {
            i.checked_sub(1)?
        };
        let neighbour = keys.get(j)?;
        let key = keys.get(i)?;
        let dt = (neighbour.time.to_f64() - key.time.to_f64()).abs();
        match resolved_side(keys, i, out) {
            SideInterp::Bezier { speed, influence } => {
                let along = speed * influence.clamp(MIN_INFLUENCE, MAX_INFLUENCE) * dt;
                Some(if out { along } else { -along })
            }
            SideInterp::Linear => Some((neighbour.value - key.value) / 3.0),
            SideInterp::Hold | SideInterp::Auto { .. } => None,
        }
    };
    let eased = |keys: &[Keyframe], i: usize, out: bool| {
        matches!(resolved_side(keys, i, out), SideInterp::Bezier { .. })
    };

    let keys = times
        .iter()
        .map(|&time| {
            let local = time.to_f64();
            let (x, y) = (px.value_at(local), py.value_at(local));
            let index = |keys: Option<&[Keyframe]>| {
                keys.and_then(|k| k.iter().position(|key| key.time == time))
            };
            let (ix, iy) = (index(kx), index(ky));
            let handle = |out: bool| -> Option<[f64; 2]> {
                let (kx, ky, ix, iy) = (kx?, ky?, ix?, iy?);
                if !eased(kx, ix, out) && !eased(ky, iy, out) {
                    return None;
                }
                Some([x + reach(kx, ix, out)?, y + reach(ky, iy, out)?])
            };
            BridgeMotionKey {
                time: rational_of(comp_time(time).0),
                frame: comp.frame_rate.frame_at(comp_time(time)),
                x,
                y,
                z: pz.map_or(0.0, |z| z.value_at(local)),
                x_index: ix.and_then(|i| u32::try_from(i).ok()),
                y_index: iy.and_then(|i| u32::try_from(i).ok()),
                handle_in: handle(false),
                handle_out: handle(true),
            }
        })
        .collect();
    Some(BridgeMotionPath {
        first_frame,
        samples,
        depths,
        keys,
    })
}

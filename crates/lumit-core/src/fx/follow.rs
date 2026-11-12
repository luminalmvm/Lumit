//! **Time effects that follow their layer's motion.** Echo, Motion blur,
//! Timewarp and Posterize time read their layer at other moments. Read from
//! the layer's own picture, those moments hold nothing its Position, Scale or
//! Rotation keys move, so a Text layer moved by keys left no trail, no blur,
//! no retimed or stepped motion until it was precomposed.
//!
//! The render draws such a layer into a comp-sized picture, placed by its own
//! transform and its parents', as the effects above the time effect leave it,
//! and builds that picture again at each moment the effect reads. The effect
//! and everything after it run on those pictures, in comp space. This module
//! says which layers do that, and at which moments, so the render, the
//! planner and the frame cache all ask the same thing.

use crate::model::{Composition, EffectInstance, EffectNamespace, Layer, LayerKind};

/// How a time effect reads its layer at other moments.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Follow {
    /// Echo and Motion blur: whole frames a set number of frames away, beside
    /// the frame itself.
    Neighbours,
    /// Timewarp: the moments it shows in place of the frame.
    Moments,
    /// Posterize time: the whole layer, transform and all, held on its grid.
    Hold,
}

const ECHO: &str = "echo";
const MOTION_BLUR: &str = "motion_blur";
const TIMEWARP: &str = "timewarp";
const POSTERIZE: &str = "posterize_time";

fn live(e: &EffectInstance, name: &str) -> bool {
    e.enabled && e.effect.namespace == EffectNamespace::Builtin && e.effect.match_name == name
}

/// Where the time effect that follows `layer`'s motion sits in its stack, and
/// how it reads. A Posterize time anywhere holds the whole layer; otherwise
/// the first live Echo, Motion blur or Timewarp. `None` on a layer that draws
/// no picture of its own, on an adjustment layer (whose time effects read the
/// composite below, motion and all), and on a stack with none of them.
#[must_use]
pub fn split(layer: &Layer) -> Option<(usize, Follow)> {
    if !layer.switches.fx
        || layer.is_adjustment()
        || matches!(
            layer.kind,
            LayerKind::Camera { .. } | LayerKind::Light { .. } | LayerKind::Null
        )
    {
        return None;
    }
    if let Some(i) = layer.effects.iter().position(|e| live(e, POSTERIZE)) {
        return Some((i, Follow::Hold));
    }
    layer.effects.iter().enumerate().find_map(|(i, e)| {
        if live(e, ECHO) || live(e, MOTION_BLUR) {
            Some((i, Follow::Neighbours))
        } else if live(e, TIMEWARP) {
            Some((i, Follow::Moments))
        } else {
            None
        }
    })
}

/// The layer as it is drawn into the picture: the effects above the time
/// effect, or under a hold every effect but the Posterize time itself.
/// Its styles run after its effects, so outside unless the whole layer is
/// held.
#[must_use]
pub fn inner(layer: &Layer, at: usize, follow: Follow) -> Layer {
    let mut inner = layer.clone();
    if follow == Follow::Hold {
        inner.effects.remove(at);
    } else {
        inner.effects.truncate(at);
        inner.styles.clear();
    }
    inner
}

/// The time effect and every effect after it, with the layer's styles: what
/// runs on the picture. Nothing under a hold.
#[must_use]
pub fn tail(layer: &Layer, at: usize, follow: Follow) -> Layer {
    let mut tail = layer.clone();
    if follow == Follow::Hold {
        tail.effects.clear();
        tail.styles.clear();
    } else {
        tail.effects.drain(..at);
    }
    tail
}

/// The frames, from the frame itself, that a Neighbours `tail` reads at layer
/// time `lt`, each with the offset a motion field measured against it is filed
/// under. An Echo reads only as many as it shows.
#[must_use]
pub fn neighbour_offsets(tail: &Layer, lt: f64, dt: f64) -> Vec<(i32, Option<i32>)> {
    let fx = &tail.effects;
    let flow = crate::fx::temporal::stack_flow_sides(fx, true, |_| false);
    let mut offsets: Vec<i32> = flow.iter().map(|&(_, side)| side).collect();
    if crate::fx::temporal::stack_is_temporal(fx, true) {
        let shown = fx
            .first()
            .filter(|e| live(e, ECHO))
            .map_or(16, |e| crate::fx::effects::echo::shown(e, lt));
        offsets.extend(
            crate::fx::temporal::stack_temporal_window(fx, true, lt / dt)
                .into_iter()
                .filter(|&o| o >= -shown),
        );
    }
    offsets.retain(|&o| o != 0);
    offsets.sort_unstable();
    offsets.dedup();
    offsets
        .into_iter()
        .map(|o| {
            (
                o,
                flow.iter().find(|&&(_, side)| side == o).map(|&(f, _)| f),
            )
        })
        .collect()
}

/// [`neighbour_offsets`] with the comp time each is built at, around comp
/// time `t`: a frame away, or for an Echo its Echo time once per echo, ahead
/// or back, so the echoes are as far apart as it says. The offset stays the
/// echo's place in the trail, which is what the kernel reads it by.
#[must_use]
pub fn neighbour_times(tail: &Layer, t: f64, lt: f64, dt: f64) -> Vec<(i32, f64, Option<i32>)> {
    let spacing = tail
        .effects
        .first()
        .filter(|e| live(e, ECHO))
        .and_then(|e| crate::fx::effects::echo::echo_time(e, lt));
    neighbour_offsets(tail, lt, dt)
        .into_iter()
        .map(|(o, measure)| {
            let tau = match spacing {
                Some(s) if measure.is_none() && o < 0 => t + f64::from(-o) * s,
                _ => crate::time::frames_on(t, o, dt),
            };
            (o, tau, measure)
        })
        .collect()
}

/// The comp time a hold shows `layer` at, around comp time `t`.
#[must_use]
pub fn held_time(layer: &Layer, t: f64) -> f64 {
    let lt = crate::time::layer_time(t, layer.start_offset.0);
    crate::fx::temporal::this_layer_effect_time(
        &layer.effects,
        layer.switches.fx,
        lt,
        layer.start_offset.0,
    ) + layer.start_offset.0.to_f64()
}

/// The comp times the picture of `layer` is built at around comp time `t`,
/// other than `t` itself: its neighbours, its moments, or the time it is held
/// at. Empty for a layer that follows nothing.
#[must_use]
pub fn times(comp: &Composition, layer: &Layer, t: f64) -> Vec<f64> {
    let Some((at, follow)) = split(layer) else {
        return Vec::new();
    };
    let dt = 1.0 / comp.frame_rate.fps().max(1.0);
    let lt = crate::time::layer_time(t, layer.start_offset.0);
    match follow {
        Follow::Neighbours => neighbour_times(&tail(layer, at, follow), t, lt, dt)
            .into_iter()
            .map(|(_, tau, _)| tau)
            .collect(),
        Follow::Moments => {
            let markers = crate::fx::markers::MarkerContext::for_layer(comp, layer);
            let start = layer.start_offset.0.to_f64();
            crate::fx::moments::stack_own_moments(
                &tail(layer, at, follow).effects,
                true,
                lt,
                &markers,
            )
            .map_or_else(Vec::new, |(_, m)| {
                m.times().into_iter().map(|tau| tau + start).collect()
            })
        }
        Follow::Hold => vec![held_time(layer, t)],
    }
}

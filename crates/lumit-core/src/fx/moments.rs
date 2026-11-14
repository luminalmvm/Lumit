//! Effects that show their own input at another moment: Timewarp, and Jolt's
//! time jolts.
//!
//! The input is the layer as the effects above this one leave it, or on an
//! adjustment layer the composite below. The render builds it again at the
//! layer times [`OwnMoments`] names and hands the pictures to the effect as its
//! layer pictures, first moment first. The planner fetches footage for the same
//! moments and the frame key reads them, all from [`stack_own_moments`], so the
//! three can never disagree about which moments they are.

use crate::fx::registry::EffectDef;
use crate::model::{EffectInstance, EffectNamespace};

/// The moments of its own input an effect shows at one frame.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct OwnMoments {
    /// The first moment, layer time in seconds.
    pub a: f64,
    /// A second moment and how far towards it the picture sits, 0 to 1. For an
    /// effect that mixes or interpolates between two frames.
    pub b: Option<(f64, f32)>,
}

impl OwnMoments {
    /// The moments as layer times, first first.
    #[must_use]
    pub fn times(&self) -> Vec<f64> {
        std::iter::once(self.a)
            .chain(self.b.map(|(b, _)| b))
            .collect()
    }
}

/// The first live effect in `effects` that shows its input at other moments,
/// its index in `effects` and those moments at layer time `lt`, with the
/// layer's markers and the comp's rate in `markers`. `None` when the
/// stack is switched off or has none, which is nearly every stack, and on a
/// frame where that effect shows the frame itself. One such effect a stack: a
/// second one shows its input at the frame's own time.
#[must_use]
pub fn stack_own_moments(
    owner: uuid::Uuid,
    effects: &[EffectInstance],
    fx_on: bool,
    lt: f64,
    markers: &crate::fx::markers::MarkerContext,
) -> Option<(usize, OwnMoments)> {
    if !fx_on {
        return None;
    }
    let (i, e, def) = effects.iter().enumerate().find_map(|(i, e)| {
        if !e.enabled || e.effect.namespace != EffectNamespace::Builtin {
            return None;
        }
        let def = crate::fx::builtins::def(&e.effect.match_name)?;
        def.wants_own_moments(e, owner).then_some((i, e, def))
    })?;
    def.own_moments(e, lt, markers).map(|m| (i, m))
}

/// Whether an op of `def` takes a slot in its layer's list of layer pictures:
/// it has a Layer row of its own, or it shows its own input at other moments.
/// The one predicate the draw builder fills the list by and the op walk reads
/// it by, so the two stay one for one.
#[must_use]
pub fn takes_layer_pictures(def: &dyn EffectDef) -> bool {
    def.schema().layer_input().is_some() || def.shows_own_moments()
}

/// The comp times `layer` of `comp` is built again at for an effect on it that
/// shows its own input at other moments, at comp time `t_comp`. Empty for
/// nearly every layer, and for one out of its span now.
#[must_use]
pub fn own_moment_comp_times(
    comp: &crate::model::Composition,
    layer: &crate::model::Layer,
    t_comp: f64,
) -> Vec<f64> {
    if t_comp < layer.in_point.0.to_f64() || t_comp >= layer.out_point.0.to_f64() {
        return Vec::new();
    }
    let start = layer.start_offset.0.to_f64();
    let lt = crate::time::layer_time(t_comp, layer.start_offset.0);
    let markers = crate::fx::markers::MarkerContext::for_layer(comp, layer);
    stack_own_moments(layer.id, &layer.effects, layer.switches.fx, lt, &markers)
        .map_or_else(Vec::new, |(_, m)| {
            m.times().into_iter().map(|at| at + start).collect()
        })
}

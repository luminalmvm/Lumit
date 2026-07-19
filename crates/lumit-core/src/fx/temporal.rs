use super::*;
use crate::model::{EffectInstance, EffectNamespace, EffectValue, Layer};

/// Which layers a Posterize Time effect (docs/08 §3.25) holds in time — the
/// owner's Scope choice.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PosterizeScope {
    /// Adjustment behaviour: the composite of everything below the effect's
    /// layer renders at the held time (the owner's global stop-motion pass).
    EverythingBelow,
    /// Only the layer's own source and effect stack sample the held time.
    ThisLayer,
}

/// A Posterize Time effect resolved at a layer time (docs/08 §3.25,
/// docs/impl/temporal-rerender.md): the coarse grid it snaps time to and the
/// scope it covers.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PosterizeParams {
    /// Posterised frame rate in fps — the grid the current time snaps down to.
    pub rate: f64,
    /// Grid phase offset in comp seconds (shifts where the steps land).
    pub phase: f64,
    pub scope: PosterizeScope,
}

/// The held comp time for Posterize Time (docs/08 §3.25): the current comp time
/// `t` snapped down to the coarser `rate`-fps grid, offset by `phase` —
/// `floor((t − phase)·rate)/rate + phase`. A degenerate grid (`rate <= 0`)
/// holds nothing and returns `t` unchanged, never dividing by zero (the engine
/// no-panic rule, docs/14). Pure and deterministic, so the two comp times that
/// share a held frame re-render to the same pixels (docs/impl/temporal-rerender
/// §6).
pub fn posterize_held_time(t: f64, rate: f64, phase: f64) -> f64 {
    if rate <= 0.0 {
        return t;
    }
    ((t - phase) * rate).floor() / rate + phase
}

/// The layer time a *This layer's effects* Posterize Time holds this layer's
/// own effect stack at (docs/08 §3.25): the coarse-grid held time in the layer's
/// own time base. Returns `lt` unchanged when the stack has no live Posterize or
/// its scope is *Everything below* (that scope re-renders the layers beneath
/// instead — the adjustment path, not this per-layer time substitution). `lt` is
/// the layer time the stack would otherwise resolve at and `start_offset` is the
/// layer's own offset, so the hold is computed on the comp time `lt +
/// start_offset` (matching the *Everything below* path, which holds on comp
/// time) and mapped back into the layer's base. Only the effect stack is held —
/// the caller keeps the layer's transform and source live, so the effects step
/// on the grid while the layer itself moves smoothly. Pure and deterministic, so
/// the preview and export derive the identical held time (K-031); shared by both
/// so a *This layer's effects* frame is identical in the viewport and the file.
pub fn this_layer_effect_time(
    effects: &[EffectInstance],
    fx_on: bool,
    lt: f64,
    start_offset: f64,
) -> f64 {
    match stack_posterize(effects, fx_on, lt) {
        Some(p) if p.scope == PosterizeScope::ThisLayer => {
            posterize_held_time(lt + start_offset, p.rate, p.phase) - start_offset
        }
        _ => lt,
    }
}

/// The first enabled built-in Posterize Time effect in a live stack, resolved
/// at layer time `lt`. None when the stack is bypassed or carries none — so a
/// layer with no Posterize pays nothing and renders normally. A stack with more
/// than one takes the first in order (a single time-hold per layer in v1).
pub fn stack_posterize(
    effects: &[EffectInstance],
    fx_on: bool,
    lt: f64,
) -> Option<PosterizeParams> {
    if !fx_on {
        return None;
    }
    effects
        .iter()
        .filter(|e| e.enabled && e.effect.namespace == EffectNamespace::Builtin)
        .find(|e| e.effect.match_name == "posterize_time")
        .map(|e| {
            let rate = e.float_at("rate", lt).unwrap_or(12.0);
            let phase = e.float_at("phase", lt).unwrap_or(0.0);
            let scope = match e.param("scope") {
                Some(EffectValue::Choice(1)) => PosterizeScope::ThisLayer,
                _ => PosterizeScope::EverythingBelow,
            };
            PosterizeParams { rate, phase, scope }
        })
}

/// The comp time each layer in a stack is *sampled* at — the time its source
/// footage is decoded and its transform/effects read — once the live Posterize
/// Time effects covering it have held their input on the coarse grid (docs/08
/// §3.25, docs/impl/temporal-rerender.md). The vector is 1:1 with `layers` and
/// in the same top-to-bottom document order (index 0 is the topmost layer, so a
/// layer at a higher index is *below*).
///
/// Two holds compose onto a running sample time as the walk descends:
/// * a **This layer** Posterize on a layer holds that layer's own sample time
///   (so its footage playback and transform step — the owner's per-layer
///   stop-motion), affecting only itself;
/// * an **Everything below** Posterize on an adjustment layer holds the sample
///   time of every layer beneath it (the owner's global stop-motion pass).
///
/// This is the piece that makes Posterize Time visibly step *footage playback*,
/// not only comp-driven animation: the decode planner reads this to snap which
/// source frame each covered layer decodes to the held grid, matching the held
/// re-render the draw builder already performs. Nested/stacked Posterize
/// adjustments compose by snapping the already-held time again, so a coarser
/// grid above dominates. Pure and deterministic, and shared by the preview
/// decode planner and export so the two hold the identical frame (K-031).
pub fn posterize_sample_times(layers: &[Layer], t_comp: f64) -> Vec<f64> {
    // The time imposed on the current layer by every Everything-below Posterize
    // adjustment seen above it, composed. Starts at the true playhead.
    let mut below_hold = t_comp;
    let mut out = Vec::with_capacity(layers.len());
    for layer in layers {
        // Start from the time the adjustments above hold this layer at.
        let mut sample_t = below_hold;
        let lt = below_hold - layer.start_offset.0.to_f64();
        let here = stack_posterize(&layer.effects, layer.switches.fx, lt);
        // A Posterize on this layer holds its own source sampling at the reduced
        // rate, whatever the scope — so applying Posterize to a footage layer
        // steps that footage (T12; before, nothing held unless the Posterize sat
        // on an adjustment layer above).
        if let Some(p) = &here {
            sample_t = posterize_held_time(below_hold, p.rate, p.phase);
        }
        out.push(sample_t);
        // An Everything-below Posterize holds every layer beneath it too, whatever
        // the carrying layer's kind (an adjustment stop-motion pass, or a plain
        // layer with content below it) — compose its grid onto the running
        // below-hold so nested holds snap the already-held time again.
        if let Some(p) = &here {
            if p.scope == PosterizeScope::EverythingBelow {
                below_hold = posterize_held_time(below_hold, p.rate, p.phase);
            }
        }
    }
    out
}

/// An accumulation motion blur effect resolved at a layer time (docs/08 §3.26,
/// docs/impl/temporal-rerender.md §3): the sub-frame shutter it samples the
/// below-stack across, and the Mix blending the averaged result against the
/// frame-time composite.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AccumulationMbParams {
    /// Sub-frame renders of the scene below across the open shutter.
    pub samples: u32,
    /// Shutter angle in degrees (the open fraction is `shutter_angle / 360`).
    pub shutter_angle: f64,
    /// Shutter phase in degrees (where the open interval sits; -90 centres it).
    pub shutter_phase: f64,
    /// Averaged-over-original blend, 0..1 (1 = full accumulation blur).
    pub mix: f64,
    /// Force per-layer motion blur (K-120) on every layer during the sub-frame
    /// sample renders (docs/08 §3.26): the effect's shutter stands in for the
    /// comp master and every layer's own switch, so one effect blurs every
    /// moving layer without toggling each one and each sample is itself
    /// transform-smeared. The comp is never mutated — the forced shutter rides
    /// on the sample-render's cloned comp only.
    pub force_all: bool,
}

impl AccumulationMbParams {
    /// The sub-frame sample offsets in *frames* across the open shutter, reusing
    /// the shared per-layer motion-blur shutter maths ([`crate::model::
    /// MotionBlur::sample_offsets`]) so the two derive the identical centred
    /// samples. Empty when `samples < 2` (a single sample is no blur — the caller
    /// then falls back to the plain frame-time composite). A caller turns each
    /// offset into a comp-time sample by `t + offset · dt` (dt = one frame in comp
    /// seconds).
    pub fn sample_offsets(&self) -> Vec<f64> {
        self.shutter().sample_offsets()
    }

    /// The shutter as a [`crate::model::MotionBlur`] — the shared centred-shutter
    /// maths the per-layer switch (K-120) uses, always enabled with this effect's
    /// angle/phase/samples.
    fn shutter(&self) -> crate::model::MotionBlur {
        crate::model::MotionBlur {
            enabled: true,
            shutter_angle: self.shutter_angle,
            shutter_phase: self.shutter_phase,
            samples: self.samples,
        }
    }

    /// The per-layer motion-blur shutter to force on every layer during the
    /// sample renders when *Force on all layers* is set (docs/08 §3.26), or None
    /// otherwise. Some carries this effect's own shutter (angle/phase/samples),
    /// so the caller drops it onto the sample render's cloned comp master and
    /// every layer's own switch — never the original comp.
    pub fn forced_layer_mb(&self) -> Option<crate::model::MotionBlur> {
        self.force_all.then(|| self.shutter())
    }
}

/// The first enabled built-in accumulation motion blur effect in a live stack,
/// resolved at layer time `lt`. None when the stack is bypassed or carries none
/// — so a layer with no accumulation blur pays nothing. A stack with more than
/// one takes the first in order (a single accumulation pass per layer in v1).
pub fn stack_accumulation_mb(
    effects: &[EffectInstance],
    fx_on: bool,
    lt: f64,
) -> Option<AccumulationMbParams> {
    if !fx_on {
        return None;
    }
    effects
        .iter()
        .filter(|e| e.enabled && e.effect.namespace == EffectNamespace::Builtin)
        .find(|e| e.effect.match_name == "accumulation_mb")
        .map(|e| {
            // Samples is a Float row (no integer kind); round and clamp to the
            // same 2..64 the schema declares, so a hand-edited project cannot
            // demand an unbounded number of full comp re-renders.
            let samples = e
                .float_at("samples", lt)
                .unwrap_or(8.0)
                .round()
                .clamp(2.0, 64.0) as u32;
            let shutter_angle = e.float_at("shutter_angle", lt).unwrap_or(180.0);
            let shutter_phase = e.float_at("shutter_phase", lt).unwrap_or(-90.0);
            let mix = (e.float_at("mix", lt).unwrap_or(100.0) / 100.0).clamp(0.0, 1.0);
            // Static bool (v1); an older project saved before the parameter
            // existed reads as the default (false).
            let force_all = e.bool_of("force_all").unwrap_or(false);
            AccumulationMbParams {
                samples,
                shutter_angle,
                shutter_phase,
                mix,
                force_all,
            }
        })
}

/// The union of source-relative frame offsets a layer's live effect stack
/// needs (docs/08 §1.3 `temporal`), always sorted and always containing 0
/// (the current frame). `&[0]` when the stack is bypassed, empty, or every
/// effect is a plain single-frame one — so a layer with no temporal effect
/// pays nothing. The render pipeline decodes the layer's source at each of
/// these offsets so a temporal effect (echo, flow motion blur, datamosh)
/// can read its neighbours.
pub fn stack_temporal_window(effects: &[EffectInstance], fx_on: bool) -> Vec<i32> {
    let mut offsets = vec![0i32];
    if fx_on {
        for e in effects.iter().filter(|e| e.enabled) {
            if e.effect.namespace != EffectNamespace::Builtin {
                continue;
            }
            if let Some(s) = schema(&e.effect.match_name) {
                offsets.extend_from_slice(s.traits.temporal);
            }
        }
    }
    offsets.sort_unstable();
    offsets.dedup();
    offsets
}

/// True when any live effect in the stack reads frames other than the
/// current one — the cheap gate the render/cache paths check before doing
/// any neighbour-frame work.
pub fn stack_is_temporal(effects: &[EffectInstance], fx_on: bool) -> bool {
    fx_on
        && effects
            .iter()
            .filter(|e| e.enabled && e.effect.namespace == EffectNamespace::Builtin)
            .any(|e| {
                schema(&e.effect.match_name)
                    .is_some_and(|s| s.traits.temporal.iter().any(|&o| o != 0))
            })
}

/// The neighbour offset a live effect wants a dense **flow field** measured
/// against (per-pixel motion vectors between the current source frame and
/// that neighbour), computed in the decode worker and handed to the kernel
/// as a texture — the gate mirroring [`stack_is_temporal`] that the render/
/// decode paths check before doing any flow work. Flow motion blur (docs/08
/// §3.2) wants `1` (the +1 neighbour); Datamosh (§3.12, K-107) wants `-1` —
/// both purely static reads of the schema's own match name now (K-107
/// dropped the dynamic per-instance check a combined Glitch effect used to
/// need). Both effects are also temporal (their windows reach that same
/// offset), so the neighbour machinery already fetches the source frame the
/// flow is measured against.
///
/// A layer can carry only one flow field per frame in v1
/// ([`crate`]-external callers store it in a single `Option` slot) — if a
/// stack somehow has both a live Motion blur and a live Datamosh, the first
/// one encountered in stack order wins and the other's flow-dependent
/// behaviour degrades to its own missing-field passthrough (never a fault;
/// pinned by test, K-104).
pub fn stack_flow_neighbour(effects: &[EffectInstance], fx_on: bool) -> Option<i32> {
    if !fx_on {
        return None;
    }
    for e in effects
        .iter()
        .filter(|e| e.enabled && e.effect.namespace == EffectNamespace::Builtin)
    {
        if e.effect.match_name == "motion_blur" {
            return Some(1);
        }
        if e.effect.match_name == "datamosh" {
            return Some(-1);
        }
    }
    None
}

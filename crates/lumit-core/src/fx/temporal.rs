use super::*;
use crate::model::{EffectInstance, EffectNamespace, Layer};

/// A Posterize Time effect resolved at a layer time (docs/08 §3.25,
/// docs/impl/temporal-rerender.md): the coarse grid it snaps time to. Its
/// reach is implied by the carrier (superseding the old Scope choice):
/// a plain layer holds its own source and effect stack; an adjustment layer
/// holds everything below it — that composite IS its effect input.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PosterizeParams {
    /// Posterised frame rate in fps — the grid the current time snaps down to.
    pub rate: f64,
    /// Grid phase offset in comp seconds (shifts where the steps land).
    pub phase: f64,
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

/// The layer time a Posterize Time holds this layer's own effect stack at
/// (docs/08 §3.25): the coarse-grid held time in the layer's own time
/// base, whenever the stack carries a live Posterize. `lt` is the layer time
/// the stack would otherwise resolve at and `start_offset` the layer's own
/// offset, so the hold is computed on the comp time `lt + start_offset`
/// (matching the adjustment below-render path, which holds on comp time) and
/// mapped back into the layer's base. Only the effect stack is held — the
/// caller keeps the layer's transform and source live, so the effects step on
/// the grid while the layer itself moves smoothly. On an adjustment layer the
/// below-render seam holds the input on the same grid, so this snap is
/// consistent with it. Pure and deterministic, shared by preview and export.
pub fn this_layer_effect_time(
    effects: &[EffectInstance],
    fx_on: bool,
    lt: f64,
    start_offset: crate::time::Rational,
) -> f64 {
    match stack_posterize(effects, fx_on, lt) {
        Some(p) => crate::time::layer_time(
            posterize_held_time(lt + start_offset.to_f64(), p.rate, p.phase),
            start_offset,
        ),
        None => lt,
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
            // A stored `scope` from an older project is simply unread: the
            // reach is implied by the carrier now.
            PosterizeParams { rate, phase }
        })
}

/// The comp time each layer in a stack is *sampled* at — the time its source
/// footage is decoded and its transform/effects read — once the live Posterize
/// Time effects covering it have held their input on the coarse grid (docs/08
/// §3.25, docs/impl/temporal-rerender.md). The vector is 1:1 with `layers` and
/// in the same top-to-bottom document order (index 0 is the topmost layer, so a
/// layer at a higher index is *below*).
///
/// Two holds compose onto a running sample time as the walk descends (the
/// reach is implied by the carrier, no Scope choice):
/// * a Posterize on a plain layer holds that layer's own sample time (so its
///   footage playback and transform step — the owner's per-layer stop-motion),
///   affecting only itself;
/// * a Posterize on an ADJUSTMENT layer holds the sample time of every layer
///   beneath it (the owner's global stop-motion pass) — the composite below is
///   that layer's effect input, so "this layer's effects" reaches it.
///
/// This is the piece that makes Posterize Time visibly step *footage playback*,
/// not only comp-driven animation: the decode planner reads this to snap which
/// source frame each covered layer decodes to the held grid, matching the held
/// re-render the draw builder already performs. Nested/stacked Posterize
/// adjustments compose by snapping the already-held time again, so a coarser
/// grid above dominates. Pure and deterministic, and shared by the preview
/// decode planner and export so the two hold the identical frame.
pub fn posterize_sample_times(layers: &[Layer], t_comp: f64) -> Vec<f64> {
    // The time imposed on the current layer by every Everything-below Posterize
    // adjustment seen above it, composed. Starts at the true playhead.
    let mut below_hold = t_comp;
    let mut out = Vec::with_capacity(layers.len());
    for layer in layers {
        // Start from the time the adjustments above hold this layer at.
        let mut sample_t = below_hold;
        let lt = crate::time::layer_time(below_hold, layer.start_offset.0);
        let here = stack_posterize(&layer.effects, layer.switches.fx, lt);
        // A Posterize on this layer holds its own source sampling at the reduced
        // rate, whatever the scope — so applying Posterize to a footage layer
        // steps that footage (T12; before, nothing held unless the Posterize sat
        // on an adjustment layer above).
        if let Some(p) = &here {
            sample_t = posterize_held_time(below_hold, p.rate, p.phase);
        }
        out.push(sample_t);
        // A Posterize carried by an ADJUSTMENT layer holds every layer beneath
        // it too (the composite below is its effect input) — compose its
        // grid onto the running below-hold so nested holds snap the
        // already-held time again.
        if let Some(p) = &here {
            if layer.is_adjustment() {
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
    /// Force per-layer motion blur on every layer during the sub-frame
    /// sample renders (docs/08 §3.26): the effect's shutter stands in for the
    /// comp master and every layer's own switch, so one effect blurs every
    /// moving layer without toggling each one and each sample is itself
    /// transform-smeared. The comp is never mutated — the forced shutter rides
    /// on the sample-render's cloned comp only.
    pub force_all: bool,
    /// Which channel of the Matte layer drives the per-pixel shutter
    /// (`CHANNEL_OPTIONS` index: 0 luminance, 1 alpha, 2 R, 3 G, 4 B), and
    /// whether it is read the other way round. This effect draws no pass of
    /// its own, so nothing prepares its matte at the dispatch seam — the
    /// combine does it itself, once, before reading.
    pub matte_channel: u32,
    /// See [`matte_channel`](Self::matte_channel).
    pub matte_invert: bool,
}

impl AccumulationMbParams {
    /// Where the **frame's own time** sits across the open shutter, 0..1: the
    /// point the matte shrinks the shutter toward.
    ///
    /// **In plain terms.** The samples are spread across the time the shutter
    /// is open, and one moment in that span is the frame the viewer asked for
    /// — with the standard −90° phase on a 180° shutter, the middle. A matte
    /// that scales the shutter angle has to shrink the span toward *that*
    /// moment and no other, because black must mean "no motion blur here",
    /// which is the frame itself and not some sub-frame instant a quarter of a
    /// frame early.
    ///
    /// Clamped to 0..1: a phase that puts the frame outside the open shutter
    /// altogether (the shutter opens after the frame, or shuts before it)
    /// shrinks toward the nearest end of the span it has. A closed shutter has
    /// no span at all and answers the middle, which costs nothing because
    /// every sample is at the same instant anyway.
    #[must_use]
    pub fn shutter_anchor(&self) -> f64 {
        let open = self.shutter_angle / 360.0;
        if open == 0.0 {
            return 0.5;
        }
        (-(self.shutter_phase / 360.0) / open).clamp(0.0, 1.0)
    }

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
    /// maths the per-layer switch uses, always enabled with this effect's
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
                // The Matte's Channel and Invert. Read here rather than
                // at the dispatch seam because this effect has no dispatch: it
                // orchestrates a re-render, and the combine prepares its own
                // matte.
                matte_channel: match e.param(crate::fx::MATTE_CHANNEL_PARAM) {
                    Some(crate::model::EffectValue::Choice(c)) => {
                        (*c).min(crate::fx::CHANNEL_OPTIONS.len() as u32 - 1)
                    }
                    _ => 0,
                },
                matte_invert: e.bool_of(crate::fx::MATTE_INVERT_PARAM).unwrap_or(false),
            }
        })
}

/// The sub-frame offsets, in comp frames, each layer's **footage** is wanted at
/// for the accumulation motion blur adjustments above it (docs/08 §3.26,
/// docs/impl/temporal-rerender.md §2). 1:1 with `layers`, top-to-bottom; empty
/// for a layer no live accumulation adjustment covers, which is every layer in
/// an ordinary comp, so the decode planner pays nothing there.
///
/// **In plain terms.** Accumulation motion blur re-renders the scene below it
/// at several moments inside one frame. Transforms and effects re-resolve at
/// those moments for free; footage does not, because a decoded frame is a
/// decoded frame. This is the list of moments the decode planner has to fetch
/// (or synthesise) each covered clip at, so that a clip playing under the
/// adjustment smears the way a moving layer does.
///
/// Every live accumulation adjustment above a layer contributes its offsets,
/// as a union: with two stacked, the inner one's own re-render wants its
/// samples of the footage and the outer one's re-render (inside which the inner
/// is held to a still) wants its own. A **plain layer carrying the effect**
/// wants its own offsets too, for itself alone: there it averages its own clip
/// over the shutter rather than re-rendering anything beneath. Sorted and
/// deduplicated on the exact value, since each consumer looks its offsets up
/// by the same maths that made them. Visibility and span are read at `t_comp`,
/// as the draw builder reads them for the draw itself.
pub fn accumulation_shutter_offsets(layers: &[Layer], t_comp: f64) -> Vec<Vec<f64>> {
    let mut above: Vec<f64> = Vec::new();
    let mut out = Vec::with_capacity(layers.len());
    for layer in layers {
        let live = layer.switches.visible
            && t_comp >= layer.in_point.0.to_f64()
            && t_comp < layer.out_point.0.to_f64();
        let lt = crate::time::layer_time(t_comp, layer.start_offset.0);
        let own = live
            .then(|| stack_accumulation_mb(&layer.effects, layer.switches.fx, lt))
            .flatten()
            .map(|p| p.sample_offsets())
            .unwrap_or_default();
        let merge = |into: &mut Vec<f64>| {
            for &off in &own {
                if !into.iter().any(|o| o.to_bits() == off.to_bits()) {
                    into.push(off);
                }
            }
            into.sort_by(f64::total_cmp);
        };
        if layer.is_adjustment() {
            // The adjustment has no clip of its own; everything beneath it
            // gets its moments.
            out.push(above.clone());
            merge(&mut above);
        } else {
            let mut mine = above.clone();
            merge(&mut mine);
            out.push(mine);
        }
    }
    out
}

/// Whether an instance is one the catalogue can answer for — a built-in, or a
/// plugin that registered at run time. A placeholder is not: it is a
/// name this build does not understand, kept so the project round-trips, and it
/// renders as identity.
fn is_catalogued(e: &EffectInstance) -> bool {
    matches!(
        e.effect.namespace,
        EffectNamespace::Builtin | EffectNamespace::Ofx
    )
}

/// The union of source-relative frame offsets a layer's live effect stack
/// needs at layer time `lt` (docs/08 §1.3 `temporal`), always sorted and always
/// containing 0 (the current frame). `&[0]` when the stack is bypassed, empty,
/// or every effect is a plain single-frame one — so a layer with no temporal
/// effect pays nothing. The render pipeline decodes the layer's source at each
/// of these offsets so a temporal effect (echo, flow motion blur, datamosh)
/// can read its neighbours.
///
/// **Per instance, not only per effect**. A built-in's window is a fact
/// about the effect and comes off its declaration, exactly as it always did. A
/// plugin's is a fact about *this copy of it at this frame*: a retimer answers
/// `getFramesNeeded` per instance and per time (docs/12 §2.1), so the definition
/// is asked ([`EffectDef::frames_needed`](super::EffectDef::frames_needed)) and
/// answers for itself, falling back to the declaration when it has nothing more
/// specific to say. The frames it names are then in the frame key and in the
/// prefetch, which is what keeps a cached frame from outliving what it sampled.
pub fn stack_temporal_window(effects: &[EffectInstance], fx_on: bool, lt: f64) -> Vec<i32> {
    let mut offsets = vec![0i32];
    if fx_on {
        for e in effects.iter().filter(|e| e.enabled && is_catalogued(e)) {
            let Some(def) = super::BUILTIN_DEFS.get(&e.effect.match_name) else {
                continue;
            };
            match def.frames_needed(e, lt) {
                Some(own) => offsets.extend_from_slice(&own),
                None => offsets.extend_from_slice(def.schema().traits.temporal),
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
///
/// Deliberately the **declaration** rather than the per-instance answer: this is
/// the gate in front of the window, it runs on every layer of every frame, and a
/// plugin that reads other frames at all says so at describe time (its declared
/// window is widened to ±1 for exactly this reason, docs/12 §2.1). A plugin that
/// declared no temporal access and then asked for another frame would be
/// answered with the frame in hand, which is what the spec says it gets.
pub fn stack_is_temporal(effects: &[EffectInstance], fx_on: bool) -> bool {
    fx_on
        && effects
            .iter()
            .filter(|e| e.enabled && is_catalogued(e))
            .any(|e| {
                schema(&e.effect.match_name)
                    .is_some_and(|s| s.traits.temporal.iter().any(|&o| o != 0))
            })
}

/// The neighbour offset **one effect** wants a dense flow field measured
/// against, by match name — the single table both the decode worker (which
/// measures) and the render (which binds a field to an op) read, so the two
/// cannot disagree about which field belongs to whom.
///
/// Flow motion blur (docs/08 §3.2) wants `1` (the +1 neighbour); Datamosh
/// (§3.12) wants `-1`. Both are purely static reads of the schema's own
/// match name (the dynamic per-instance check a combined Glitch effect used
/// to need is gone), and both are also temporal — their windows reach that
/// same offset — so the neighbour machinery already fetches the source frame the
/// flow is measured against.
pub fn effect_flow_neighbour(match_name: &str) -> Option<i32> {
    match match_name {
        "motion_blur" => Some(1),
        "datamosh" => Some(-1),
        _ => None,
    }
}

/// Every neighbour offset a live effect in the stack wants a dense **flow
/// field** measured against (per-pixel motion vectors between the current
/// source frame and that neighbour), computed in the decode worker and handed
/// to the kernels as textures — the gate mirroring [`stack_is_temporal`] that
/// the render/decode paths check before doing any flow work. Sorted and
/// deduplicated; empty when the stack wants none, so a plain layer pays nothing.
///
/// **One offset per consumer, not one per layer.** Until this, the
/// layer carried a single field and the first flow-consuming effect in stack
/// order took it, leaving the other silently doing nothing. The two want
/// *different measurements* — forward to the next frame versus back to the
/// previous — so there was never one field to share; measuring both is the only
/// honest answer. They are separate entries in the same flow cache, keyed by the
/// frame pair they were measured over, so a stack with one of them costs exactly
/// what it always did.
/// Which of the file's channels this stack asks to be decoded into red, green,
/// blue and alpha (docs/08 §3.97), or `None` for a stack that does not ask.
///
/// Read by the decode planner, beside [`stack_flow_neighbours`] and for the
/// same reason: both are effect settings that have to be known *before* the
/// pixels exist, so the plan reads them rather than the stack.
///
/// **The last enabled one wins.** Two Extract channels on one layer is a
/// contradiction rather than a chain — there is only one decode — and the
/// bottom of the stack is the one nearer the picture, so it is the one that
/// gets asked for. Effects off is every layer decoding as itself.
#[must_use]
pub fn stack_extracted_channels(
    effects: &[EffectInstance],
    fx_on: bool,
) -> Option<[Option<String>; 4]> {
    if !fx_on {
        return None;
    }
    effects
        .iter()
        .filter(|e| e.effect.namespace == EffectNamespace::Builtin)
        .filter(|e| e.effect.match_name == "extract_channels")
        .filter_map(crate::fx::effects::extract_channels::selection)
        .next_back()
}

pub fn stack_flow_neighbours(effects: &[EffectInstance], fx_on: bool) -> Vec<i32> {
    let mut offsets: Vec<i32> = if fx_on {
        effects
            .iter()
            .filter(|e| e.enabled && e.effect.namespace == EffectNamespace::Builtin)
            .filter_map(|e| effect_flow_neighbour(&e.effect.match_name))
            .collect()
    } else {
        Vec::new()
    };
    offsets.sort_unstable();
    offsets.dedup();
    offsets
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod extracted_channel_tests {
    use super::*;
    use crate::fx::effects::extract_channels as ec;

    fn extractor(channels: &[&str], slot: usize, option: u32) -> EffectInstance {
        let names: Vec<String> = channels.iter().map(|c| (*c).to_owned()).collect();
        let mut inst = crate::fx::builtins::instantiate("extract_channels").unwrap();
        inst.extra
            .insert(ec::EXTRA_KEY.to_owned(), ec::channels_extra(&names));
        inst.params.push(crate::model::EffectParam {
            id: ec::SLOT_IDS[slot].to_owned(),
            value: crate::model::EffectValue::Choice(option),
            extra: serde_json::Map::new(),
        });
        inst
    }

    /// The plan reads the selection off the stack, which is the whole way the
    /// decode ever hears about it.
    #[test]
    fn the_stack_hands_over_the_channels_it_asks_for() {
        let stack = vec![extractor(&["R", "Z"], 0, 2)];
        assert_eq!(
            stack_extracted_channels(&stack, true),
            Some([Some("Z".into()), None, None, None])
        );
    }

    /// Effects off is every layer decoding as itself — the switch has to reach
    /// the decode as well as the stack, or turning effects off would leave the
    /// layer showing an extracted channel with no effect to explain it.
    #[test]
    fn effects_off_decodes_the_picture_the_file_opens_as() {
        let stack = vec![extractor(&["R", "Z"], 0, 2)];
        assert_eq!(stack_extracted_channels(&stack, false), None);
    }

    /// One decode, so a second copy is a contradiction rather than a chain: the
    /// one nearer the picture is the one that gets asked for.
    #[test]
    fn two_extractors_take_the_lower_one() {
        let stack = vec![extractor(&["R", "Z"], 0, 2), extractor(&["R", "Z"], 0, 1)];
        assert_eq!(
            stack_extracted_channels(&stack, true),
            Some([Some("R".into()), None, None, None])
        );
    }

    /// A layer with no such effect asks for nothing, which is nearly every
    /// layer and has to stay free.
    #[test]
    fn an_ordinary_stack_asks_for_nothing() {
        assert_eq!(stack_extracted_channels(&[], true), None);
    }
}

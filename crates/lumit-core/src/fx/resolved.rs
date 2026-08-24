use std::sync::Arc;

use super::*;
use crate::{
    expression::ExpressionContext,
    model::{EffectInstance, EffectNamespace, EffectValue},
};
use uuid::Uuid;

/// The Fast motion blur output view (docs/08 §3.2, FX-19): the finished blurred
/// picture, or a diagnostic look at the motion field, the confidence that
/// steers the streak, or the dominant motion the reconstruction borrows in the
/// places confidence is low. A per-pixel choice the kernel branches on last.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MbView {
    /// The blurred picture (the default).
    Rendered,
    /// The per-pixel flow vectors, colour-coded (red = +x, green = +y, grey =
    /// still) — for checking the motion the smear follows.
    MotionVectors,
    /// The per-pixel confidence as greyscale (white = trusted, black = suspect)
    /// — for seeing where the streak is steered by its neighbourhood instead of
    /// by its own vector.
    Confidence,
    /// The tile/neighbour-max dominant motion, colour-coded on the same scale as
    /// [`MbView::MotionVectors`] (docs/impl/optical-flow.md §4.5 item 3) — the
    /// blocky field an uncertain pixel borrows its direction from. Flipping
    /// between this and Motion vectors is how the borrow is checked by eye.
    TileMax,
}

impl MbView {
    /// The kernel's integer code for this view (0 Rendered, 1 Motion vectors, 2
    /// Confidence, 3 Dominant motion), so the CPU oracle and the WGSL uniform
    /// agree.
    pub fn code(self) -> i32 {
        match self {
            MbView::Rendered => 0,
            MbView::MotionVectors => 1,
            MbView::Confidence => 2,
            MbView::TileMax => 3,
        }
    }
}

/// The Fast motion blur reconstruction tier (docs/impl/optical-flow.md §4.5
/// "Tiers", K-390). **The only choice a user sees** — there is no method
/// picker; one method adapts internally, and this buys it more work per pixel.
///
/// # In plain terms
///
/// Normal spaces the samples along a streak about two pixels apart and draws
/// each streak straight. High halves the spacing (smoother long streaks) and
/// re-reads the motion field partway along each streak so a streak can *bend* —
/// which is what a spinning or swinging object's smear actually does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MbQuality {
    /// Straight streaks, samples ~2 px apart (the default).
    Normal,
    /// Curved streaks (the field is re-sampled along the trail) and samples
    /// ~1 px apart.
    High,
}

impl MbQuality {
    /// The kernel's integer code (0 Normal, 1 High), so the CPU oracle and the
    /// WGSL uniform agree.
    pub fn code(self) -> i32 {
        match self {
            MbQuality::Normal => 0,
            MbQuality::High => 1,
        }
    }

    /// Pixels between adjacent taps along a streak — §4's `S = ceil(‖v‖ / 2)`
    /// adaptive-tap rule, with High halving the step.
    pub fn tap_spacing(self) -> f32 {
        match self {
            MbQuality::Normal => 2.0,
            MbQuality::High => 1.0,
        }
    }

    /// Whether the field is re-sampled partway along each streak (curved
    /// trails, §4's destination-flow fixed point applied per tap).
    pub fn curved(self) -> bool {
        matches!(self, MbQuality::High)
    }
}

/// The Matte key output view (docs/08 §3.21, K-154): the finished keyed picture,
/// or a diagnostic look at the screen matte the key derives. A per-op choice the
/// kernel and CPU reference branch on (identically) at the end. The integer codes
/// are the wire form the WGSL uniform reads: 0 Final, 1 Screen matte, 2 Status.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MatteKeyView {
    /// The keyed, despilled, matte-applied picture (the default).
    Final,
    /// The screen matte itself as greyscale (white kept, black keyed) — for
    /// seeing exactly what the key is holding out.
    ScreenMatte,
    /// A continuous heat view of the matte: greyscale, with the uncertain
    /// mid-tones tinted so at-risk edges and holes stand out.
    Status,
}

impl MatteKeyView {
    /// The kernel's integer code (0 Final, 1 Screen matte, 2 Status), so the CPU
    /// oracle and the WGSL uniform agree.
    pub fn code(self) -> u32 {
        match self {
            MatteKeyView::Final => 0,
            MatteKeyView::ScreenMatte => 1,
            MatteKeyView::Status => 2,
        }
    }

    /// The view for a stored Choice index, clamped to the known set (unknown
    /// codes fall back to Final — a safe, non-diagnostic default).
    pub fn from_code(code: u32) -> Self {
        match code {
            1 => MatteKeyView::ScreenMatte,
            2 => MatteKeyView::Status,
            _ => MatteKeyView::Final,
        }
    }
}

/// How the Matte key recolours pixels where despill removed screen tint (docs/08
/// §3.21, K-154, Keylight's Replace method). Codes are the WGSL wire form: 0
/// Source, 1 Hard colour, 2 Soft colour, 3 None.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReplaceMethod {
    /// Keep the original source colour (no colour replacement; alpha still keys).
    Source,
    /// Blend in the flat replace colour where spill was removed.
    HardColour,
    /// Blend in the replace colour scaled by the pixel's own brightness (the
    /// default — it settles into shading rather than reading as a flat patch).
    SoftColour,
    /// Leave the despilled colour untouched.
    None,
}

impl ReplaceMethod {
    /// The kernel's integer code (0 Source, 1 Hard colour, 2 Soft colour, 3 None).
    pub fn code(self) -> u32 {
        match self {
            ReplaceMethod::Source => 0,
            ReplaceMethod::HardColour => 1,
            ReplaceMethod::SoftColour => 2,
            ReplaceMethod::None => 3,
        }
    }

    /// The method for a stored Choice index, clamped to the known set (unknown
    /// codes fall back to Soft colour, the tasteful default).
    pub fn from_code(code: u32) -> Self {
        match code {
            0 => ReplaceMethod::Source,
            1 => ReplaceMethod::HardColour,
            3 => ReplaceMethod::None,
            _ => ReplaceMethod::SoftColour,
        }
    }
}

/// The Matte key's full resolved parameter bundle (docs/08 §3.21, K-154): the
/// Keylight-style colour-difference keyer, flattened to plain numbers that the CPU
/// reference ([`cpu::matte_key`](crate::fx::cpu::matte_key)) and the WGSL kernel
/// both read, so preview and export match op-for-op (K-031). Every field is
/// already unit-normalised by the resolve step; the maths derive the screen's
/// primary channel and reference from `key` identically on both paths.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MatteKeyParams {
    /// Which picture to output (Final / Screen matte / Status), as a wire code.
    pub view: u32,
    /// Scene-linear RGBA screen (key) colour; alpha ignored. Its largest channel
    /// picks the primary screen axis.
    pub key: [f32; 4],
    /// Screen gain (Keylight's Screen strength): scales the matte's fall-off. 1.0
    /// keys the exact screen to zero; > 1 keys more aggressively. `≥ 0`.
    pub gain: f32,
    /// Screen balance, 0..1: how the two non-screen channels are weighted into the
    /// reference the primary is measured against (0 = their min, 1 = their max,
    /// 0.5 = their average, the default).
    pub balance: f32,
    /// Despill bias (scene-linear RGBA, alpha ignored): shifts the reference the
    /// unspill clamps the primary down to. A neutral grey is a no-op.
    pub despill_bias: [f32; 4],
    /// Alpha bias (scene-linear RGBA, alpha ignored): shifts what colour counts as
    /// neutral for the matte. A neutral grey is a no-op.
    pub alpha_bias: [f32; 4],
    /// Despill amount, 0..1: fraction of the primary's screen excess pulled out of
    /// kept pixels (Keylight's screen despill).
    pub spill: f32,
    /// Clip black, 0..1: screen-matte values at/below this map to 0 (fully keyed).
    pub clip_black: f32,
    /// Clip white, 0..1: screen-matte values at/above this map to 1 (fully kept).
    pub clip_white: f32,
    /// Clip rollback, 0..1: pulls the clipped matte back toward the un-clipped
    /// matte, recovering fine edge detail the clips would erode (0 = full clip).
    pub clip_rollback: f32,
    /// Replace method wire code (0 Source, 1 Hard, 2 Soft, 3 None).
    pub replace_method: u32,
    /// Scene-linear RGBA replace colour used by the Hard/Soft replace methods.
    pub replace_colour: [f32; 4],
    /// Screen pre-blur radius in **raster** pixels (K-446): how far the picture
    /// the key is *judged from* is softened. The colour that comes out is still
    /// the sharp original. 0 is the neutral, and the whole stage is skipped.
    pub pre_blur: f32,
    /// Screen matte shrink (−) / grow (+) in **raster** pixels (K-446): a
    /// morphological march of the matte's edge, inward or outward. 0 is the
    /// neutral.
    pub shrink_grow: f32,
    /// Screen matte softness in **raster** pixels (K-446): a Gaussian blur of
    /// the matte, and only the matte. 0 is the neutral.
    pub softness: f32,
    /// Despot black, 0..1 (K-446): how far an isolated dark speck is lifted to
    /// its neighbours. 0 is the neutral.
    pub despot_black: f32,
    /// Despot white, 0..1 (K-446): how far an isolated bright speck is dropped
    /// to its neighbours. 0 is the neutral.
    pub despot_white: f32,
    /// 0..1, blended against the untouched premultiplied input; 0 is the identity.
    pub mix: f32,
}

impl MatteKeyParams {
    /// Whether any **spatial** stage is asked for (K-446) — the one predicate
    /// both render paths branch on, so the fast pointwise kernel and the staged
    /// pipeline are chosen identically. The garbage masks are asked separately,
    /// because they come from the mask carriage rather than from these numbers.
    #[must_use]
    pub fn spatial(&self) -> bool {
        self.pre_blur > 0.0
            || self.shrink_grow != 0.0
            || self.softness > 0.0
            || self.despot_black > 0.0
            || self.despot_white > 0.0
    }
}

/// Resolve a layer's live stack at layer time `lt` for a raster whose
/// diagonal is `diag_px` pixels; `px_scale` is raster pixels per comp pixel
/// (the §2.3 preview-resolution factor — 1.0 at full resolution), which
/// converts px@comp parameters exactly as `diag_px` converts % diag ones.
/// `markers` is the layer's §1.4 marker context ([`MarkerContext::for_layer`],
/// or [`MarkerContext::NONE`] where no comp is in play), consumed by the
/// marker-driven modes (Flash's Trigger and Strobe, §3.7). Placeholders,
/// unknown names and bypassed effects resolve to nothing (they render as
/// identity, docs/03 §8).
pub fn resolve_stack(
    effects: &[EffectInstance],
    lt: f64,
    diag_px: f32,
    px_scale: f32,
    markers: &MarkerContext,
    context: Arc<ExpressionContext>,
) -> ResolvedStack {
    // The same walk, handed one time for every effect: when `sample_lt ==
    // frame_lt` the temporal branch cannot fire, so this is the plain resolve.
    resolve_stack_temporal_named(effects, lt, lt, diag_px, px_scale, markers, context).1
}

/// Resolve a layer's live stack for a held/sub-frame re-render (docs/impl/
/// temporal-rerender.md §5): an effect flagged `sample_temporally == false`
/// resolves at the true frame time `frame_lt` (so a particle system or other
/// costly/stochastic effect is not re-run per held sample), while every other
/// effect resolves at the held/sample time `sample_lt`. When `sample_lt ==
/// frame_lt` this is byte-identical to [`resolve_stack`], so an ordinary
/// (non-temporal) render is unchanged — the two share one walk, differing only
/// in which layer time each effect is handed.
pub fn resolve_stack_temporal(
    effects: &[EffectInstance],
    sample_lt: f64,
    frame_lt: f64,
    diag_px: f32,
    px_scale: f32,
    markers: &MarkerContext,
    context: Arc<ExpressionContext>,
) -> ResolvedStack {
    resolve_stack_temporal_named(
        effects, sample_lt, frame_lt, diag_px, px_scale, markers, context,
    )
    .1
}

/// [`resolve_stack_temporal`] with the id of the effect instance behind each
/// op, 1:1 and in order with `ops`.
///
/// **Why the ids matter.** The render-time indicator has to land a measured
/// millisecond on the right row of the effect stack, and the mapping cannot be
/// reconstructed afterwards by filtering the effect list, because the walk also
/// drops placeholders, unknown names and the orchestration-only effects. So the
/// one walk that knows both answers reports both, and everything else stays 1:1
/// by construction.
#[allow(clippy::too_many_arguments)]
pub fn resolve_stack_temporal_named(
    effects: &[EffectInstance],
    sample_lt: f64,
    frame_lt: f64,
    diag_px: f32,
    px_scale: f32,
    markers: &MarkerContext,
    context: Arc<ExpressionContext>,
) -> (Vec<Uuid>, ResolvedStack) {
    let mut ids = Vec::new();
    let mut out = ResolvedStack::new();
    for e in effects
        .iter()
        .filter(|e| e.enabled && e.effect.namespace == EffectNamespace::Builtin)
    {
        let lt = if e.sample_temporally {
            sample_lt
        } else {
            frame_lt
        };
        // Every built-in is looked up in the catalogue and resolved by the one
        // generic loop below (docs/impl/effect-registry.md §6); a name this
        // build does not know resolves to nothing, which is how an unknown
        // effect stays an inert placeholder (K-065). The arena's own op order
        // *is* the stack order, so there is no second list to keep in step.
        if let Some(def) = BUILTIN_DEFS.get(&e.effect.match_name) {
            // An orchestration-only effect (Posterize time, Accumulation motion
            // blur) resolves to nothing at all: it changes *what time* the layers
            // it covers render at, which the frame walk reads straight off the
            // instance, and it has no per-pixel op to order among the others.
            // Skipping it here leaves it no op, no bag, and no id in the
            // indicator's list.
            if !def.is_image_op() {
                continue;
            }
            resolve_into_arena(
                def,
                e,
                lt,
                diag_px,
                px_scale,
                markers,
                &mut out,
                context.clone(),
            );
            ids.push(e.id);
        }
    }
    (ids, out)
}

/// Evaluate every parameter a migrated effect declares into the stack's arena
/// (docs/impl/effect-registry.md §3, step 1).
///
/// **In plain terms.** This is the loop that replaces thirty-odd hand-written
/// resolve arms. It walks the effect's own declaration, asks the instance for
/// each control's value at this frame — through the expression context, exactly
/// as the hand-written arms did — converts it by the unit the declaration
/// states, and drops the pair into the bag. No effect has resolve code of its
/// own any more; what used to sit in its arm (the clamps, the `exp2`, the hue
/// matrix) now sits in the effect's `packed` method, called once at dispatch by
/// whichever of the two render paths is running.
///
/// The bag carries **schema-space** numbers — Saturation 100 means 100 per
/// cent, not a factor of 1 — because that is what the effect's typed reader and
/// its declared default are written in. Only two conversions happen here, and
/// both are about *rasters* rather than about the effect:
///
/// - `Px` is px@comp on the way in (docs/08 §2.3 forbids anything else), so it
///   is multiplied by `px_scale` to reach the raster in play. Every distance
///   a built-in declares is this (K-419).
/// - `PctDiag` would become `v / 100 × diag_px` (the caller has already scaled
///   `diag_px` by the preview factor). No built-in parameter declares it any
///   more; the arm stays so the enum resolves completely.
///
/// Both are what [`ResolvedStack::rescale_spatial`] moves again if the stack is
/// later reused at another size, which is the symmetry the old `rescale_px`
/// had.
///
/// After the declared parameters comes the one thing the declaration cannot
/// carry: values derived from layer time, the marker context or a whole
/// keyframed track ([`EffectDef::resolve_derived`], K-385). Almost every effect
/// pushes nothing there.
#[allow(clippy::too_many_arguments)]
fn resolve_into_arena(
    def: &'static dyn EffectDef,
    e: &EffectInstance,
    lt: f64,
    diag_px: f32,
    px_scale: f32,
    markers: &MarkerContext,
    bags: &mut ResolvedStack,
    context: Arc<ExpressionContext>,
) {
    bags.begin(def, e.id);
    for p in def.schema().params {
        // A number the instance does not carry reads its declared default: a
        // project saved before the parameter existed simply renders (K-258).
        let spatial = |v: f64| -> f32 {
            let v = v as f32;
            match p.unit {
                Unit::PctDiag => v / 100.0 * diag_px,
                Unit::Px => v * px_scale,
                Unit::Raw | Unit::Degrees | Unit::Seconds => v,
            }
        };
        let value = match p.kind {
            // A Slider is a Float with a closed range (K-414): the kind is the
            // control, so it resolves through exactly the Float path and an
            // adopting parameter's output cannot move.
            ParamKind::Float { default, .. }
            | ParamKind::Slider { default, .. }
            | ParamKind::Angle { default, .. } => Value::Float(spatial(
                e.float_at_with_context(p.id, lt, context.clone())
                    .unwrap_or(default),
            )),
            ParamKind::Int { default, .. } => Value::Int(
                e.float_at_with_context(p.id, lt, context.clone())
                    .map_or(default as i32, |v| v.round() as i32),
            ),
            ParamKind::Bool { default } => Value::Bool(e.bool_of(p.id).unwrap_or(default)),
            ParamKind::Choice { default, .. } => Value::Choice(match e.param(p.id) {
                Some(EffectValue::Choice(c)) => *c,
                _ => default,
            }),
            ParamKind::Colour { default, .. } => {
                Value::Colour(e.colour_at(p.id, lt).unwrap_or(default).map(|c| c as f32))
            }
            ParamKind::Seed => Value::Int(match e.param(p.id) {
                Some(EffectValue::Seed(s)) => *s as i32,
                _ => 0,
            }),
            // A File slot, a Layer binding and a mask's geometry are decided by
            // the *caller*, which is the only thing that knows which cube
            // loaded, which layer was rendered or which masks the layer carries
            // (docs/impl/layer-input.md); they are threaded beside the op as an
            // aux slot instead (K-387, K-408, docs/impl/effect-registry.md
            // §2.5a), so the bag deliberately carries nothing for them. An
            // effect declaring one must declare the list it consumes:
            // `a_side_table_effect_declares_the_list_it_consumes` in
            // lumit-render fails the moment one does not, and a silent default
            // here would be a picture quietly rendering without its LUT.
            //
            // An **Action** skips for a stronger reason still (K-417): those
            // three carry their payload beside the op, and a button carries
            // nothing anywhere. It is not a value, so it is not in the bag,
            // and so it is not in the frame key either — pressing Analyse
            // renames no frame.
            ParamKind::File { .. }
            | ParamKind::Layer { .. }
            | ParamKind::MaskPath { .. }
            | ParamKind::Action => continue,
            // A curve is small enough to ride in the bag itself (K-412), so
            // unlike the three above it needs no slot beside the op. It does
            // not animate, so there is nothing to evaluate at `lt` — only the
            // straightening every read applies.
            ParamKind::Curve => Value::Curve(match e.param(p.id) {
                Some(EffectValue::Curve(points)) => CurvePoints::sanitised(points),
                _ => CurvePoints::IDENTITY,
            }),
        };
        bags.push(ParamId::new(p.id), value);
    }
    def.resolve_derived(
        &ResolveCx {
            inst: e,
            lt,
            diag_px,
            px_scale,
            markers,
            context,
        },
        &mut |id, value| bags.push(id, value),
    );
}

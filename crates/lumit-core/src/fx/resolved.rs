use std::sync::Arc;

use super::*;
use crate::{
    expression::ExpressionContext,
    model::{EffectInstance, EffectNamespace, EffectValue},
};
use uuid::Uuid;

/// The Fast motion blur output view (docs/08 §3.2, FX-19): the finished blurred
/// picture, or a diagnostic look at the motion field or the confidence that
/// tapers the streak length. A per-pixel choice the kernel branches on last.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MbView {
    /// The blurred picture (the default).
    Rendered,
    /// The per-pixel flow vectors, colour-coded (red = +x, green = +y, grey =
    /// still) — for checking the motion the smear follows.
    MotionVectors,
    /// The per-pixel confidence as greyscale (white = trusted, black = suspect)
    /// — for seeing where the streak fades out.
    Confidence,
}

impl MbView {
    /// The kernel's integer code for this view (0 Rendered, 1 Motion vectors, 2
    /// Confidence), so the CPU oracle and the WGSL uniform agree.
    pub fn code(self) -> i32 {
        match self {
            MbView::Rendered => 0,
            MbView::MotionVectors => 1,
            MbView::Confidence => 2,
        }
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
    /// 0..1, blended against the untouched premultiplied input; 0 is the identity.
    pub mix: f32,
}

/// One sub-frame state of a shake's own motion blur (T18, K-165): the wobble
/// sampled at one point in the shutter, in the same `(offset_px, rotation_deg,
/// zoom)` form the frame-time shake carries. The dispatch turns each into an
/// affine through [`shake_affine`] and averages the resamples.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ShakeSample {
    /// Wobble offset at this sub-frame, raster pixels.
    pub offset_px: [f32; 2],
    /// Rotation wobble at this sub-frame, degrees.
    pub rotation_deg: f32,
    /// Zoom factor at this sub-frame; 1 = no depth (z) shake.
    pub zoom: f32,
}

impl ShakeSample {
    /// The neutral (identity) sample — the fixed-size array's initialiser.
    pub const IDENTITY: Self = Self {
        offset_px: [0.0, 0.0],
        rotation_deg: 0.0,
        zoom: 1.0,
    };
}

/// One effect, resolved to plain numbers at a frame — the flat form both the
/// WGSL kernels (lumit-gpu) and the CPU references below consume.
#[derive(Debug, Clone, Copy, PartialEq)]
// Clippy would have the largest variant boxed. It cannot be: `Resolved` is
// `Copy` by design — plain old data with no owned allocations, so a resolved
// stack can be hashed byte-for-byte into the frame key (K-143) and copied into
// a GPU uniform without a clone. `Box` is not `Copy`, so taking the advice
// would cost the determinism guarantee to save a few hundred bytes on a
// per-frame, per-layer value that never lives in a long array.
#[allow(clippy::large_enum_variant)]
pub enum Resolved {
    /// A migrated effect: its parameters live in the stack's
    /// [`ResolvedStack`] arena (`op` is the index of its op there), not in a
    /// variant of their own. The Vec keeps ordering authority — the arena is
    /// only the bag — so the two halves of a [`ResolvedOps`] stay in step
    /// (docs/impl/effect-registry.md §2.3).
    ///
    /// Nearly every built-in resolves to this now (34 of them); only Shake
    /// still has a variant of its own, and the migration ends when it loses it
    /// and this enum goes with it.
    Registry { op: u32 },
    /// A shake, already sampled at this frame (the noise runs at resolve
    /// time, host-side): the current wobble, dispatched through the Transform
    /// kernel via [`shake_affine`] — no kernel of its own. `edge` (P3, K-145)
    /// governs the border the resample reveals; there is no Auto-scale cover
    /// any more (FX-11/K-146 replaced it with this Edges control).
    Shake {
        /// This frame's wobble offset, raster pixels.
        offset_px: [f32; 2],
        /// This frame's rotation wobble, degrees.
        rotation_deg: f32,
        /// This frame's zoom factor; 1 = no depth (z) shake.
        zoom: f32,
        /// Edge policy for the revealed border: 0 Transparent, 1 Repeat,
        /// 2 Mirror ([`EdgesMode`]).
        edge: u32,
        /// 0..1.
        mix: f32,
        /// The shake's own motion blur (T18, K-165): `Some` when the toggle is
        /// on and the amount is non-zero — the wobble sampled at
        /// [`SHAKE_MB_SAMPLES`] sub-frame placements across the shutter, which
        /// the dispatch resamples and averages in premultiplied linear space
        /// (the accumulation-motion-blur philosophy, applied to this effect
        /// alone). `None` is the plain single resample, the bit-exact
        /// passthrough. The centre sample equals the frame-time wobble above.
        /// Sampled host-side because the noise lattice needs 64-bit integers
        /// the GPU has not got (docs/08 §3.12).
        mb: Option<[ShakeSample; SHAKE_MB_SAMPLES]>,
    },
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
/// Rescale every pixel-dimensioned field of already-resolved ops by `f` —
/// the repair for a stack resolved against one raster and run on another
/// (K-266). The Adjust arm of the draw builder resolves with `px_scale` 1
/// because its stack runs on "the comp-sized intermediate" — which is only
/// true at full preview resolution. Under reduced-resolution preview the
/// intermediate is the preview raster, and every px@comp parameter (the
/// flare's light, DoF apertures, blur radii) landed too far right and too
/// big by exactly the preview factor; the owner measured the flare's light
/// hitting the frame edge at 1500 of a 1920 comp. The realise walk calls
/// this with `render_width / comp_width` before running an adjustment
/// stack.
///
/// Exhaustive on purpose: a new op must decide here whether it owns pixel
/// fields, so the bug cannot quietly return with the next effect.
pub fn rescale_px(ops: &mut [Resolved], f: f32) {
    if (f - 1.0).abs() < 1e-6 {
        return;
    }
    for op in ops {
        match op {
            Resolved::Shake { offset_px, mb, .. } => {
                offset_px[0] *= f;
                offset_px[1] *= f;
                if let Some(samples) = mb {
                    for s in samples.iter_mut() {
                        s.offset_px[0] *= f;
                        s.offset_px[1] *= f;
                    }
                }
            }
            // A migrated effect keeps no pixel field here: its parameters sit
            // in the arena, which declares its own units, so
            // `ResolvedStack::rescale_spatial` moves them —
            // `ResolvedOps::rescale_px` calls both halves together so neither
            // can be forgotten.
            Resolved::Registry { .. } => {}
        }
    }
}

/// A layer's stack resolved at one frame, in both the forms the hybrid period
/// needs: the `Vec` of flat variants that still carries the ordering, and the
/// arena the migrated effects keep their parameters in
/// (docs/impl/effect-registry.md §2.3). `bags` is empty until the first effect
/// migrates, so nothing downstream changes yet.
#[derive(Debug, Clone, Default)]
pub struct ResolvedOps {
    pub ops: Vec<Resolved>,
    pub bags: ResolvedStack,
}

impl ResolvedOps {
    /// Rescale both halves for a stack resolved against one raster and run on
    /// another (K-266) — the one entry point, so a caller cannot rescale the
    /// variants and forget the arena.
    pub fn rescale_px(&mut self, f: f32) {
        rescale_px(&mut self.ops, f);
        self.bags.rescale_spatial(f);
    }
}

pub fn resolve_stack(
    effects: &[EffectInstance],
    lt: f64,
    diag_px: f32,
    px_scale: f32,
    markers: &MarkerContext,
    context: Arc<ExpressionContext>,
) -> ResolvedOps {
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
/// (non-temporal) render is unchanged — the two share [`resolve_one`], differing
/// only in which layer time each effect is handed.
pub fn resolve_stack_temporal(
    effects: &[EffectInstance],
    sample_lt: f64,
    frame_lt: f64,
    diag_px: f32,
    px_scale: f32,
    markers: &MarkerContext,
    context: Arc<ExpressionContext>,
) -> ResolvedOps {
    resolve_stack_temporal_named(
        effects, sample_lt, frame_lt, diag_px, px_scale, markers, context,
    )
    .1
}

/// [`resolve_stack_temporal`] with the id of the effect instance behind each
/// op, 1:1 and in order with `ops`.
///
/// **Why the ids matter.** A [`Resolved`] op is a flat bag of numbers: by
/// design it has forgotten which effect wrote it, because the kernels do not
/// care. The render-time indicator does care — a measured millisecond has to
/// land on the right row of the effect stack — and the mapping cannot be
/// reconstructed afterwards by filtering the effect list, because
/// [`resolve_one`] also drops placeholders, unknown names and the
/// orchestration-only effects. So the one walk that knows both answers reports
/// both, and everything else stays 1:1 by construction.
#[allow(clippy::too_many_arguments)]
pub fn resolve_stack_temporal_named(
    effects: &[EffectInstance],
    sample_lt: f64,
    frame_lt: f64,
    diag_px: f32,
    px_scale: f32,
    markers: &MarkerContext,
    context: Arc<ExpressionContext>,
) -> (Vec<Uuid>, ResolvedOps) {
    let mut ids = Vec::new();
    let mut out = ResolvedOps::default();
    for e in effects
        .iter()
        .filter(|e| e.enabled && e.effect.namespace == EffectNamespace::Builtin)
    {
        let lt = if e.sample_temporally {
            sample_lt
        } else {
            frame_lt
        };
        // A migrated effect (docs/impl/effect-registry.md §6) is looked up in
        // the catalogue and resolved by the one generic loop below; anything
        // still carrying a variant of its own goes through `resolve_one`. The
        // `Vec` keeps the ordering either way, so the two halves of the stack
        // cannot get out of step.
        if let Some(def) = BUILTIN_DEFS.get(&e.effect.match_name) {
            // An orchestration-only effect (Posterize time, Accumulation motion
            // blur) resolves to nothing at all: it changes *what time* the layers
            // it covers render at, which the frame walk reads straight off the
            // instance, and it has no per-pixel op to order among the others.
            // Skipping it here is exactly what `resolve_one` returning `None`
            // meant for it — no op, no bag, and no id in the indicator's list.
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
                &mut out.bags,
                context.clone(),
            );
            let op = (out.bags.len() - 1) as u32;
            out.ops.push(Resolved::Registry { op });
            ids.push(e.id);
        } else if let Some(op) = resolve_one(e, lt, diag_px, context.clone()) {
            out.ops.push(op);
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
/// - `PctDiag` becomes pixels of the raster in play, `v / 100 × diag_px`. The
///   caller has already scaled `diag_px` by the preview factor, which is why
///   `px_scale` does not appear again — the old arms did exactly this.
/// - `Px` is px@comp on the way in (docs/08 §2.3 forbids anything else), so it
///   is multiplied by `px_scale` to reach the same raster.
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
            ParamKind::Float { default, .. } | ParamKind::Angle { default, .. } => {
                Value::Float(spatial(
                    e.float_at_with_context(p.id, lt, context.clone())
                        .unwrap_or(default),
                ))
            }
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
            // A File slot and a Layer binding are decided by the *caller*, which
            // is the only thing that knows which cube loaded or which layer was
            // rendered (docs/impl/layer-input.md); they are threaded beside the
            // op as an aux slot instead (K-387, docs/impl/effect-registry.md
            // §2.5a), so the bag deliberately carries nothing for them. An
            // effect declaring one must declare the list it consumes:
            // `a_side_table_effect_declares_the_list_it_consumes` in
            // lumit-render fails the moment one does not, and a silent default
            // here would be a picture quietly rendering without its LUT.
            ParamKind::File { .. } | ParamKind::Layer { .. } => continue,
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

/// Resolve one effect instance to its flat [`Resolved`] op at layer time `lt`,
/// or None when it is a placeholder, an unknown name, or an orchestration-only
/// effect (Posterize time, accumulation motion blur) that has no per-pixel op.
/// The shared core of [`resolve_stack`] and [`resolve_stack_temporal`].
///
/// It takes neither a marker context nor the preview factor: Flash was the only
/// arm that read markers and it reads them through [`ResolveCx`] now (K-385),
/// and the Lens flare was the last arm with a px@comp parameter to scale — Shake
/// measures its wobble against the diagonal instead.
fn resolve_one(
    e: &EffectInstance,
    lt: f64,
    diag_px: f32,
    expression_context: Arc<ExpressionContext>,
) -> Option<Resolved> {
    // Every float parameter reads through the expression context. Bound once
    // here so the sixty-odd call sites below stay as short as they were before
    // expressions existed.
    let fl = |id: &str| e.float_at_with_context(id, lt, expression_context.clone());
    match e.effect.match_name.as_str() {
        "shake" => {
            let amp_pct = (fl("amplitude").unwrap_or(1.5) as f32).max(0.0);
            let freq = fl("frequency").unwrap_or(8.0).max(0.0);
            let rot_amount = (fl("rotation").unwrap_or(1.0) as f32).max(0.0);
            // Per-axis wobble (twirl group, K-146): amount multipliers scale
            // the master Amplitude, frequency multipliers the master rate.
            // Defaults of 1 reproduce the old uniform x/y shake exactly.
            let x_amp = (fl("x_amp").unwrap_or(1.0) as f32).max(0.0);
            let y_amp = (fl("y_amp").unwrap_or(1.0) as f32).max(0.0);
            let x_freq = fl("x_freq").unwrap_or(1.0).max(0.0);
            let y_freq = fl("y_freq").unwrap_or(1.0).max(0.0);
            let z_freq = fl("z_freq").unwrap_or(1.0).max(0.0);
            // z (depth/scale) amount: the new id, else the old `zoom_pump`
            // (migration — a project saved before FX-11 keeps its pump), a
            // scale-pump per cent either way.
            let z_pct = fl("z_amp").or_else(|| fl("zoom_pump")).unwrap_or(0.0) as f32;
            let z_amp = (z_pct / 100.0).clamp(0.0, 1.0);
            // Edges (P3, K-145): the new `edge` Choice, else migrate the old
            // Auto-scale bool (on → Repeat hides the border as the cover once
            // did; off → Transparent), else the schema default Repeat.
            let edge = match e.param("edge") {
                Some(EffectValue::Choice(c)) => {
                    EdgesMode::from_code((*c).min(2)).unwrap_or(EdgesMode::Repeat)
                }
                _ => match e.param("auto_scale") {
                    Some(EffectValue::Bool(false)) => EdgesMode::Transparent,
                    _ => EdgesMode::Repeat,
                },
            };
            let seed = match e.param("seed") {
                Some(EffectValue::Seed(s)) => *s,
                _ => 0,
            };
            let mix = (fl("mix").unwrap_or(100.0) as f32 / 100.0).clamp(0.0, 1.0);
            // The wobble: independent noise channels sampled at local time ×
            // frequency (per axis, §3.4) — deterministic, hop-free, identical
            // on every machine (§2.4). One sampler drives the frame-time wobble
            // and the motion-blur sub-frames, so they agree bit-for-bit.
            let base = lt * freq;
            let amp_px = (amp_pct / 100.0 * diag_px).max(0.0);
            let wobble = ShakeWobble {
                seed,
                amp_px,
                x_amp,
                y_amp,
                rot_amount,
                z_amp,
                x_freq,
                y_freq,
                z_freq,
            };
            let (offset_px, rotation_deg, zoom) = wobble.at(base);
            // The shake's own motion blur (T18, K-165): when the toggle is on
            // and the amount is non-zero, sample the wobble across the shutter
            // for the dispatch to average; off is the plain single resample
            // (the bit-exact passthrough). The centre offset is 0, so the middle
            // sample equals the frame-time wobble exactly.
            let motion_blur = e.bool_of("motion_blur").unwrap_or(false);
            let mb_amount = fl("mb_amount").unwrap_or(0.5);
            let mb = (motion_blur && mb_amount > 0.0).then(|| {
                let mut samples = [ShakeSample::IDENTITY; SHAKE_MB_SAMPLES];
                for (s, db) in samples.iter_mut().zip(shake_mb_offsets(mb_amount)) {
                    let (offset_px, rotation_deg, zoom) = wobble.at(base + db);
                    *s = ShakeSample {
                        offset_px,
                        rotation_deg,
                        zoom,
                    };
                }
                samples
            });
            Some(Resolved::Shake {
                offset_px,
                rotation_deg,
                zoom,
                edge: edge.code(),
                mix,
                mb,
            })
        }
        _ => None,
    }
}

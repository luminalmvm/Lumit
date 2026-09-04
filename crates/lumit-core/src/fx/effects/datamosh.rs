//! Datamosh (docs/08 §3.12): the compression-artefact look, done honestly —
//! the previous frame dragged along this frame's motion instead of being
//! redrawn.
//!
//! **In plain terms.** Two of this effect's inputs are pictures, not numbers:
//! the layer's −1 neighbour frame and its current→previous flow field, both off
//! the decode. They arrive beside the resolved op as the whole lists the render
//! prepared — Datamosh names the flow field, and picks its one neighbour
//! out of the decoded window. Either missing (a non-footage layer, or a dropped
//! decode) is a passthrough, never a fault.
//!
//! Two of the numbers are not controls either. The melt ramps from a clean frame
//! just after each simulated I-frame up to full by the next, which is a function
//! of the clock; and an older project stores its reach under the old
//! `streak_length` id. Both are worked out at resolve time through the one hook
//! that sees the clock and the stored instance ([`EffectDef::resolve_derived`]),
//! exactly as the hand-written resolve arm worked them out.
//!
//! There is no CPU reference through the single-buffer dispatcher, which carries
//! neither the neighbour nor the field, so `apply_cpu` keeps its identity default
//! — the passthrough the old `Resolved::Datamosh` arm of `cpu::apply` was. The
//! §1.6 oracle is [`crate::fx::cpu::datamosh`], exercised directly from the
//! lumit-gpu test, which can upload both.

use crate::fx::{EffectDef, EffectMetadata, EffectSchema, ParamId, Params, ResolveCx, Value};
use lumit_fx_macros::Effect;

/// Datamosh's controls.
#[derive(Debug, Clone, Copy, PartialEq, Effect)]
#[effect(
    match_name = "datamosh",
    label = "Datamosh",
    version = 3,
    // Beside Shake and RGB split (a seeded positional wobble, a channel split) —
    // but Datamosh itself reads no hash or seed, so `seeded` stays false.
    category = Distortion,
    // A streamline of up to 64 bilinear taps (like Motion blur's own streak
    // integral), not the single tap the earlier version took.
    cost = Moderate,
    // The flow can point anywhere in the frame — the same unbounded-read
    // reasoning Motion blur's own full-frame ROI carries.
    roi = FullFrame,
    temporal = &[0, -1],
)]
pub struct Datamosh {
    /// Blends between the ordinary frame and the moshed one. 0 is the bit-exact
    /// passthrough (pinned by test); the hard ceiling is open (FX-14), so a
    /// value above 1 extrapolates past the moshed frame for a punchier tear.
    /// Default 1 (owner, 2026-07-19): below 1 it reads like a lowered Mix, so the
    /// full melt is the out-of-the-box look.
    #[slider(min = 0.0, max = 1.0, default = 1.0, hard_min = 0.0, unit = Raw)]
    pub intensity: f32,

    /// Frames of predicted motion the streamline walk reaches (each step advances
    /// ~1 frame of flow): 1 is a one-frame prediction, higher reaches further so
    /// more smearing accumulates (the P-frame run length between clean reference
    /// frames). Open above.
    ///
    /// The id `displacement` supersedes the older `streak_length`, which is still
    /// read as a fallback — and *that* is why the number the kernel reads is
    /// [`Datamosh::DERIVED_DISPLACEMENT`] rather than this row: an old project's
    /// reach lives under an id the schema no longer declares, so the bag cannot
    /// carry it.
    #[slider(min = 1.0, max = 16.0, default = 4.0, hard_min = 1.0, unit = Frames)]
    pub displacement: f32,

    /// How much of the reach accumulates into the smear: 0 keeps only the nearest
    /// step (a short, quickly-resetting trail), 1 averages the whole walk evenly
    /// (a long melting bloom). A pure 0..1 ratio — the natural unit for a blend
    /// weight.
    #[slider(min = 0.0, max = 1.0, default = 0.6, hard_min = 0.0, hard_max = 1.0, unit = Raw)]
    pub bloom: f32,

    /// Seconds between simulated I-frames: the melt ramps from a clean frame just
    /// after each reset up to full by the next, the accumulating-P-frame look. 0
    /// turns the periodic reset off (a constant melt); the content-driven reset —
    /// stills and cuts, where the flow is zero — still fires regardless. In
    /// seconds (not frames) because the resolve step is frame-rate-agnostic; a
    /// frame-count interval needs the comp frame index threaded through resolve,
    /// a deferred broad change.
    ///
    /// What it *produces* — where the ramp has got to this frame — is
    /// [`Datamosh::DERIVED_RAMP`].
    #[slider(min = 0.0, max = 5.0, default = 0.0, hard_min = 0.0, unit = Seconds)]
    pub reset_interval: f32,

    /// The host-uniform Mix every effect ends with (docs/08 §1.5), per cent.
    #[slider(
        min = 0.0,
        max = 100.0,
        default = 100.0,
        hard_min = 0.0,
        hard_max = 100.0,
        unit = Percent
    )]
    pub mix: f32,
}

impl Datamosh {
    /// Where the melt has got to at this frame, 0..1 — a pure function of the
    /// layer time and Reset interval, and 1 when the periodic reset is
    /// off. Never a panel row: it is what the stored parameters *produce*.
    pub const DERIVED_RAMP: ParamId = ParamId::new("derived.ramp");

    /// The reach this instance actually walks, before the ramp — the
    /// `displacement` row, or an older project's `streak_length` where that row
    /// does not exist. Never a panel row.
    pub const DERIVED_DISPLACEMENT: ParamId = ParamId::new("derived.displacement");

    /// The ramp and the reach out of a resolved bag: [`Datamosh::packed`]'s two
    /// missing arguments, so no caller has to know the ids.
    pub fn derived_of(p: Params<'_>) -> (f32, f32) {
        (
            p.float(Self::DERIVED_RAMP, 1.0),
            p.float(Self::DERIVED_DISPLACEMENT, 4.0),
        )
    }

    /// What the kernel wants (docs/impl/effect-registry.md §2.4):
    /// `(intensity, displacement, bloom, steps, mix)`, derived exactly as the old
    /// resolve arm derived them. The ramp scales the intensity and the reach
    /// together, so a frame just after a reset is nearly clean; the step count
    /// tracks the reach because each step advances about one frame of flow, and
    /// is clamped to the same 2..=64 Motion blur's streak loops (or 1 at a
    /// sub-frame reach, where a single tap is exact). Both render paths read this
    /// one method, so the CPU reference and the WGSL kernel cannot drift apart.
    pub fn packed(self, ramp: f32, displacement: f32) -> (f32, f32, f32, i32, f32) {
        let eff_displacement = (displacement * ramp).max(0.0);
        let steps = if eff_displacement < 1.0 {
            1
        } else {
            (eff_displacement.round() as i32).clamp(2, 64)
        };
        (
            self.intensity.max(0.0) * ramp,
            eff_displacement,
            self.bloom.clamp(0.0, 1.0),
            steps,
            (self.mix / 100.0).clamp(0.0, 1.0),
        )
    }
}

/// Datamosh's behaviour: no CPU reference through the single-buffer dispatcher
/// (the neighbour and the flow field are textures), so `apply_cpu` keeps its
/// identity default — the passthrough the old `Resolved::Datamosh` arm was.
pub struct DatamoshDef;

impl EffectDef for DatamoshDef {
    fn schema(&self) -> &'static EffectSchema {
        &<Datamosh as EffectMetadata>::SCHEMA
    }

    /// The melt's ramp and the migrated reach — the whole of what the old resolve
    /// arm did beyond reading its rows, moved unchanged.
    ///
    /// The ramp is `(lt ÷ interval) mod 1`, in `f64` and `rem_euclid` so a
    /// negative layer time still lands inside the period rather than reflecting;
    /// a zero or negative interval turns the periodic reset off and the ramp is
    /// exactly 1, which multiplies through as the identity.
    ///
    /// The reach reads `displacement`, falling back to the older `streak_length`
    /// when the newer row is absent — floored at one frame in `f64` before the
    /// cast, exactly as the arm ordered it.
    fn resolve_derived(&self, cx: &ResolveCx<'_>, push: &mut dyn FnMut(ParamId, Value)) {
        let (e, lt) = (cx.inst, cx.lt);
        let fl = |id: &str| e.float_at_with_context(id, lt, cx.context.clone());
        let interval = fl("reset_interval").unwrap_or(0.0).max(0.0);
        let ramp = if interval > 0.0 {
            (lt / interval).rem_euclid(1.0) as f32
        } else {
            1.0
        };
        push(Datamosh::DERIVED_RAMP, Value::Float(ramp));
        let displacement = fl("displacement")
            .or_else(|| fl("streak_length"))
            .unwrap_or(4.0)
            .max(1.0) as f32;
        push(Datamosh::DERIVED_DISPLACEMENT, Value::Float(displacement));
    }
}

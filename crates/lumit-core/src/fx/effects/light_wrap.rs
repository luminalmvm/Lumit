//! Light wrap (docs/08 §3.28): the oldest trick in compositing.
//!
//! **In plain terms.** A keyed subject reads as pasted on because in a real
//! camera the light behind it spills round its edges. This takes the referenced
//! Background layer, blurs it over Width, and screens that blur back only into
//! the band just inside the foreground's own outline — found from the
//! foreground's alpha, so the effect needs no mask of its own.
//!
//! The background is a whole picture, not a number, so it arrives beside the
//! resolved op as this effect's aux slot: the render enumerates every
//! layer-input-taking effect in stack order and `run_ops` counts along the same
//! list, which is why Light wrap and Depth of field share one counter. An unset,
//! missing or cyclic Background leaves the slot empty and the effect is the
//! labelled no-op every layer-input effect follows.
//!
//! There is no CPU reference through the single-buffer dispatcher: the second
//! picture never reaches it, so `apply_cpu` keeps its identity default, exactly
//! as the old `Resolved::LightWrap` arm of `cpu::apply` was a passthrough. The
//! §1.6 oracle is [`crate::fx::cpu::light_wrap`], exercised directly from the
//! lumit-gpu test, which can upload a background.

use crate::fx::{EffectDef, EffectMetadata, EffectSchema};
use lumit_fx_macros::Effect;

/// Light wrap's controls.
#[derive(Debug, Clone, Copy, PartialEq, Effect)]
#[effect(
    match_name = "light_wrap",
    label = "Light wrap",
    version = 1,
    category = Stylise,
    cost = Moderate,
    // The wrap reaches Width inside the edge and the blur reads Width out.
    // Width's hard maximum is open, so the padding is its 200 px@comp slider
    // doubled — which also covers both halves of that reach at the slider.
    roi = PaddedPx(400.0),
    // The band is read off the foreground's own alpha, which only means
    // anything premultiplied.
    premultiplied = true,
)]
pub struct LightWrap {
    /// The plate whose light spills round the foreground's edge. Unset until the
    /// owner picks one — a labelled no-op. No `self_default`: a layer is
    /// never its own background, so starting pointed at itself would be a wrap of
    /// nothing.
    ///
    /// **Always `false` here, by design.** A Layer binding is decided by the
    /// caller — only the render knows which layer was actually rendered — so
    /// `resolve_into_arena` carries no `Value::Layer`, and the picture arrives at
    /// the GPU pass as its aux slot instead. The row exists because the
    /// panel needs it.
    #[layer(self_default = false)]
    pub background: bool,

    /// px@comp, and the same distance twice: how far the wrap reaches
    /// inside the edge, and the radius the background is softened by. Declared
    /// `Px`, so the resolve step scales it by the §2.3 preview factor and
    /// [`ResolvedStack::rescale_spatial`](crate::fx::ResolvedStack::
    /// rescale_spatial) moves it again if the stack is reused at another size —
    /// which is what the old `rescale_px`'s `LightWrap` arm did by hand.
    #[slider(min = 0.0, max = 200.0, default = 0.0, hard_min = 0.0, unit = Px)]
    pub width: f32,

    /// Gain on the spill before it is screened on. Open above for a
    /// deliberately hot wrap.
    #[slider(min = 0.0, max = 3.0, default = 1.0, hard_min = 0.0, unit = Raw)]
    pub intensity: f32,

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

impl LightWrap {
    /// The width, gain and mix the kernel blends by, floored and clamped exactly
    /// as the old resolve arm did. The width arrives already scaled by the §2.3
    /// preview factor, so a half-resolution preview wraps by the same *visible*
    /// width the export will. Both render paths read this one method, so the CPU
    /// reference and the WGSL kernel cannot drift apart.
    pub fn packed(self) -> (f32, f32, f32) {
        (
            self.width.max(0.0),
            self.intensity.max(0.0),
            (self.mix / 100.0).clamp(0.0, 1.0),
        )
    }
}

/// Light wrap's behaviour: no CPU reference through the single-image dispatcher
/// (the background is a second picture), so `apply_cpu` keeps its identity
/// default — the passthrough the old `Resolved::LightWrap` arm was.
pub struct LightWrapDef;

impl EffectDef for LightWrapDef {
    fn schema(&self) -> &'static EffectSchema {
        &<LightWrap as EffectMetadata>::SCHEMA
    }
}

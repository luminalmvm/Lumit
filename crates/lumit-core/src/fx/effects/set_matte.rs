//! Set matte (docs/08 §3.44): another layer's channel becomes this layer's
//! alpha — AE's Set Matte.
//!
//! **In plain terms.** Pick a layer, pick one of its channels, and that channel
//! is now this layer's transparency: bright where the layer shows, dark where it
//! does not. It is how a title is cut out of a cloud, how a fill is shaped by a
//! ramp, and how one picture wears another's silhouette.
//!
//! **It is a K-395 matte consumer by nature**, which is what settled the open
//! question docs/impl/ae-effect-parity.md carried (K-400): the effect lives in
//! Utility *and* its source is the universal Matte row, because those were never
//! two answers. Its matte does not scale a strength — it **is** the alpha — so
//! this is the sixth effect to claim the matte inside its own maths, and the
//! generic dissolve does not also run.
//!
//! There is no CPU reference through the single-buffer dispatcher, which carries
//! no second picture, so `apply_cpu` keeps its identity default — the labelled
//! no-op an unset row renders anyway. The §1.6 oracle is
//! [`crate::fx::cpu::set_matte`], exercised directly from the lumit-gpu test,
//! which can upload a matte.

use crate::fx::{EffectDef, EffectMetadata, EffectSchema, CHANNEL_OPTIONS};
use lumit_fx_macros::Effect;

/// Set matte's controls.
#[derive(Debug, Clone, Copy, PartialEq, Effect)]
#[effect(
    match_name = "set_matte",
    label = "Set matte",
    version = 1,
    category = Utility,
    cost = Trivial,
    roi = Exact,
    // §2.2: the effect changes COVERAGE and must leave colour alone, so it runs
    // on straight values — a premultiplied colour multiplied by a new alpha
    // would have been scaled twice. The round trip is fused into the one pass.
    premultiplied = false,
    // K-395: the injected Matte row IS this effect's source, not a strength.
    matte = (
        "matte",
        "is the alpha: the chosen channel of the matte layer becomes this \
         layer's coverage, which is the whole effect rather than a strength \
         applied to one",
    ),
)]
pub struct SetMatte {
    /// Which channel of the matte layer carries the shape. **Luminance by
    /// default, where AE's is the alpha**: a layer picked as a matte is very
    /// often an opaque grey picture — a Fractal noise, a ramp, a luma pass —
    /// whose alpha is 1 everywhere, and an effect that did nothing until a
    /// second control was also changed is the no-op default docs/08 §1.2
    /// forbids. The import writes the value, so nothing is lost.
    #[choice(options = *CHANNEL_OPTIONS, default = 0)]
    pub channel: u32,

    /// Intersect with the layer's own alpha instead of replacing it — AE's
    /// "Composite Matte with Original". Off by default, because "set" is what
    /// the effect is called.
    #[toggle(label = "Combine with existing alpha", default = false)]
    pub combine: bool,

    /// The host-uniform Mix every effect ends with (docs/08 §1.5), per cent.
    #[slider(
        min = 0.0,
        max = 100.0,
        default = 100.0,
        hard_min = 0.0,
        hard_max = 100.0
    )]
    pub mix: f32,
}

impl SetMatte {
    /// The three numbers both kernels consume (docs/impl/effect-registry.md
    /// §2.4). `invert` is not here: it is the injected Matte row's own switch,
    /// read out of the bag beside the layer binding by whoever has the texture.
    #[must_use]
    pub fn packed(self) -> (u32, bool, f32) {
        (
            self.channel.min(CHANNEL_OPTIONS.len() as u32 - 1),
            self.combine,
            (self.mix / 100.0).clamp(0.0, 1.0),
        )
    }
}

/// Set matte's behaviour: no CPU reference through the single-image dispatcher
/// (the matte is a second picture), so `apply_cpu` keeps its identity default.
pub struct SetMatteDef;

impl EffectDef for SetMatteDef {
    fn schema(&self) -> &'static EffectSchema {
        &<SetMatte as EffectMetadata>::SCHEMA
    }
}

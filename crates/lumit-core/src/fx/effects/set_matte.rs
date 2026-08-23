//! Set matte (docs/08 §3.44): another layer's channel becomes this layer's
//! alpha — AE's Set Matte.
//!
//! **In plain terms.** Pick a layer, pick one of its channels, and that channel
//! is now this layer's transparency: bright where the layer shows, dark where it
//! does not. It is how a title is cut out of a cloud, how a fill is shaped by a
//! ramp, and how one picture wears another's silhouette.
//!
//! **It carries no Matte row** (K-429, the owner's rule for mattes). It used to
//! claim the universal one (K-395/K-400) because its source and a matte looked
//! like the same picture; they are not the same *idea*. Every other effect's
//! Matte row answers "how much of me happens here", and Set matte has no answer
//! to give: what it takes from another layer is the coverage itself, which is
//! the whole effect rather than an amount of one. So the row it shows is its
//! **own** source picker — an ordinary auxiliary layer on the K-387 carriage,
//! beside Light wrap's Background and Texturize's Texture — and the universal
//! row is gone, with the Channel that duplicated the one below it. The stored
//! ids are unchanged, so a project saved before this reads back exactly as it
//! did (K-065, K-258).
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
    // K-429 (the owner's rule for mattes): the effect that IS a matte carries no
    // Matte row. The row below is its own source, declared here rather than
    // injected, and it rides the ordinary auxiliary-layer carriage.
    matte = false,
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

    /// The layer this one wears the silhouette of. **This effect's own source
    /// row**, not the universal Matte (K-429): the id and the label are the ones
    /// the injected row had, so a saved project reads back unchanged (K-065),
    /// but nothing about it is generic any more — no dissolve stands beside the
    /// kernel, and the Channel above is the only channel pick there is.
    ///
    /// **Always `false` here, by design**, as every Layer row is: a layer
    /// binding is decided by the caller, so `resolve_into_arena` carries no
    /// `Value::Layer` and the picture arrives at the GPU pass as its aux slot
    /// instead (K-387). The row exists because the panel needs it. Unset is the
    /// labelled no-op — a coverage nobody has supplied cannot have a tasteful
    /// default (§1.2).
    #[layer(label = "Matte", self_default = false)]
    pub matte: bool,

    /// Read the source the other way round: opaque where it was clear. Applied
    /// once, in the kernel, since nothing prepares this picture at the seam.
    #[toggle(label = "Invert", default = false)]
    pub matte_invert: bool,
}

impl SetMatte {
    /// The three numbers both kernels consume (docs/impl/effect-registry.md
    /// §2.4). `invert` is not here: it is the source row's own switch, read out
    /// of the bag beside the layer binding by whoever has the texture.
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

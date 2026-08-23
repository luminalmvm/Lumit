//! Displacement map (docs/08 §3.49): another layer's channels push this one —
//! AE's Displacement Map.
//!
//! **In plain terms.** Pick a layer. Where it is bright, this layer's pixels are
//! pushed one way; where it is dark, the other way; where it is mid-grey they do
//! not move at all. One channel of the map steers the sideways push and another
//! the up-and-down one, and the two Amounts say how far a push can go.
//!
//! **It is a K-395 matte consumer by nature**, the seventh, and the second after
//! Set matte (§3.44) whose matte is the *subject* rather than a modifier: the
//! layer on the Matte row **is** the map. AE has a picker of its own for it;
//! Lumit already has one row that names a layer and renders it at this raster,
//! and a second beside it saying the same thing would be a seam for nothing.
//!
//! There is no CPU reference through the single-buffer dispatcher, which carries
//! no second picture, so `apply_cpu` keeps its identity default — the labelled
//! no-op an unset row renders anyway. The §1.6 oracle is
//! [`crate::fx::cpu::displacement_map`], exercised directly from the lumit-gpu
//! test, which can upload a map.

use crate::fx::{cpu, EffectDef, EffectMetadata, EffectSchema, CHANNEL_OPTIONS, EDGE_OPTIONS};
use lumit_fx_macros::Effect;

/// Displacement map's controls.
#[derive(Debug, Clone, Copy, PartialEq, Effect)]
#[effect(
    match_name = "displacement_map",
    label = "Displacement map",
    version = 1,
    category = Distortion,
    // One map read and one bilinear tap a pixel.
    cost = Cheap,
    // The Amount sliders' own reach. Their hard maximum is open (a push may be
    // typed past the slider), so the padding is the slider's 500 px@comp
    // doubled, exactly as Turbulent displace's is.
    roi = PaddedPx(1000.0),
    premultiplied = true,
    // K-395: the matte is not a strength here, it is the map — the whole input
    // the effect exists to read. The generic strength dissolve does not also run.
    matte = (
        "matte",
        "is the displacement map: the chosen channels of the matte layer say \
         which way and how far each pixel is pushed, mid-grey meaning no push at \
         all — the effect's subject rather than a strength applied to one",
    ),
    // K-425: the two channel choices below are this effect's own channel pick,
    // so the seam injects none and hands the kernel the raw RGBA map.
    matte_channel = false,
)]
pub struct DisplacementMap {
    /// Which channel of the map steers the sideways push. **Red, as AE's is** —
    /// a map authored for this effect puts x in red and y in green, and the two
    /// defaults together read a colour map the way its author meant it.
    #[choice(label = "Horizontal channel", options = *CHANNEL_OPTIONS, default = 2)]
    pub horizontal_channel: u32,

    /// px@comp: the farthest a pixel can be pushed sideways, at map white.
    /// Signed — negative simply reads the map the other way round on this axis.
    #[slider(
        label = "Horizontal amount",
        min = -500.0,
        max = 500.0,
        default = 60.0,
        unit = Px
    )]
    pub horizontal_amount: f32,

    /// Which channel of the map steers the up-and-down push. **Green, as AE's
    /// is**; see [`horizontal_channel`](Self::horizontal_channel).
    #[choice(label = "Vertical channel", options = *CHANNEL_OPTIONS, default = 3)]
    pub vertical_channel: u32,

    /// px@comp; see [`horizontal_amount`](Self::horizontal_amount).
    #[slider(
        label = "Vertical amount",
        min = -500.0,
        max = 500.0,
        default = 60.0,
        unit = Px
    )]
    pub vertical_amount: f32,

    /// What a sample pushed off the frame reads. **Repeat by default**, which is
    /// AE's behaviour with "Wrap Pixels Around" off and the one that keeps a
    /// rippled frame full rather than eating holes in its border.
    #[choice(label = "Edges", options = *EDGE_OPTIONS, default = 1)]
    pub edge: u32,

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

impl DisplacementMap {
    /// The bundle both kernels consume (docs/impl/effect-registry.md §2.4).
    /// `invert` is not here: it is the injected Matte row's own switch, read out
    /// of the bag beside the layer binding by whoever has the texture, exactly
    /// as Set matte's is.
    #[must_use]
    pub fn packed(self) -> cpu::DisplacementMapParams {
        let last = CHANNEL_OPTIONS.len() as u32 - 1;
        cpu::DisplacementMapParams {
            channels: [
                self.horizontal_channel.min(last),
                self.vertical_channel.min(last),
            ],
            amount: [self.horizontal_amount, self.vertical_amount],
            edge: self.edge.min(2),
            mix: (self.mix / 100.0).clamp(0.0, 1.0),
        }
    }
}

/// Displacement map's behaviour: no CPU reference through the single-image
/// dispatcher (the map is a second picture), so `apply_cpu` keeps its identity
/// default — which is also what an unbound Matte row renders.
pub struct DisplacementMapDef;

impl EffectDef for DisplacementMapDef {
    fn schema(&self) -> &'static EffectSchema {
        &<DisplacementMap as EffectMetadata>::SCHEMA
    }
}

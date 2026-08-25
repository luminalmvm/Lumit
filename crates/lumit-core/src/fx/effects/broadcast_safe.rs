//! Broadcast safe (docs/08 §3.69): the signal clamped to a legal amplitude —
//! AE's Broadcast Colors.
//!
//! **In plain terms.** Analogue television carries brightness and colour in one
//! wire, added together, and a transmitter will distort or refuse a signal whose
//! total swings too far. A saturated red on a bright background can be perfectly
//! legal on its own and illegal once the two are added. This measures that total
//! for every pixel and either pulls the pixel down until it is legal, drains the
//! colour out of it until it is, or shows you which pixels the problem is in.
//!
//! **It is a delivery tool, not a look** — hence Utility — and it is named for
//! what it does rather than for what AE calls it (docs/01).

use crate::fx::{cpu, EffectDef, EffectMetadata, EffectSchema, Params};
use lumit_fx_macros::Effect;

/// Broadcast safe's controls.
#[derive(Debug, Clone, Copy, PartialEq, Effect)]
#[effect(
    match_name = "broadcast_safe",
    label = "Broadcast safe",
    version = 1,
    category = Utility,
    cost = Cheap,
    roi = Exact,
    // §2.2: the amplitude is a statement about the pixel's own colour, not about
    // the colour times its coverage.
    premultiplied = false,
)]
pub struct BroadcastSafe {
    /// Which system's composite signal is being measured. NTSC carries 7.5 IRE
    /// of setup below black and 92.5 of active range; PAL has no setup and 100.
    #[choice(options = ["NTSC", "PAL"], default = 0)]
    pub standard: u32,

    /// What to do with a pixel that is over the limit. The first two repair it,
    /// the last two are diagnostic views — Key out unsafe leaves only the legal
    /// picture, Key out safe leaves only the problem, which composites over the
    /// frame as an overlay.
    #[choice(
        label = "How to treat",
        options = [
            "Reduce brightness",
            "Reduce saturation",
            "Key out unsafe",
            "Key out safe",
        ],
        default = 0
    )]
    pub how_to_treat: u32,

    /// IRE: the largest total amplitude allowed. 110 is AE's default and the
    /// figure most broadcasters ask for; 100 is the strict limit.
    #[slider(
        label = "Maximum signal",
        min = 90.0,
        max = 120.0,
        default = 110.0,
        hard_min = 90.0,
        hard_max = 120.0,
        unit = Raw
    )]
    pub maximum_signal: f32,

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

impl BroadcastSafe {
    /// NTSC's setup pedestal, as a fraction of the full signal: 7.5 IRE of 100.
    pub const NTSC_SETUP: f32 = 0.075;

    /// The bundle both kernels consume (docs/impl/effect-registry.md §2.4).
    /// **The whole of the standard's difference is one number** — the setup —
    /// and it is folded into the target amplitude here, so neither kernel
    /// branches on NTSC versus PAL at all.
    #[must_use]
    pub fn packed(self) -> cpu::BroadcastSafeParams {
        let setup = if self.standard == 0 {
            Self::NTSC_SETUP
        } else {
            0.0
        };
        cpu::BroadcastSafeParams {
            // ire = 100·(setup + (1 − setup)·(Y + C)), so the amplitude the
            // pixel's own luma and chroma must not exceed is this.
            target: (self.maximum_signal.clamp(90.0, 120.0) / 100.0 - setup) / (1.0 - setup),
            mode: self.how_to_treat.min(3),
            mix: (self.mix / 100.0).clamp(0.0, 1.0),
        }
    }
}

/// Broadcast safe's behaviour.
pub struct BroadcastSafeDef;

impl EffectDef for BroadcastSafeDef {
    fn schema(&self) -> &'static EffectSchema {
        &<BroadcastSafe as EffectMetadata>::SCHEMA
    }

    fn apply_cpu(&self, rgba: &mut [f32], _w: u32, _h: u32, p: Params<'_>) {
        cpu::broadcast_safe(rgba, &BroadcastSafe::read(p).packed());
    }
}

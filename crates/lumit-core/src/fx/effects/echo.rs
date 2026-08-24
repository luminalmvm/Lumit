//! Echo / trails (docs/08 §3.13): the montage speed-line staple, and the first
//! temporal effect — its window reaches back to previous frames.
//!
//! **In plain terms.** Echo does not read its neighbours' *parameters*, it reads
//! their *pictures*: the render decodes the layer's source at offsets −1…−16 and
//! hands the whole list to the GPU pass beside the resolved op (K-387). Nothing
//! about which frames those are is a control — the window follows from the
//! `temporal` trait declared here — so what the effect resolves to is the trail
//! itself: one weight per offset, the blend that combines them, and Mix.
//!
//! v1 status, unchanged by the migration: echoes are spaced one comp frame apart
//! (a Spacing control is a later refinement), echo *k* sits at offset −k with
//! intensity `decay^k`, and the trail is built from the layer's **source**
//! frames rather than the upstream stack's output at those times.
//!
//! There is no CPU reference. The neighbour frames never reach the single-buffer
//! CPU dispatcher, so the degradation rung renders Echo as identity, exactly as
//! the old `Resolved::Echo` arm did; the §1.6 oracle is [`crate::fx::cpu::echo`],
//! exercised directly from the lumit-gpu test, which can upload the neighbours.

use crate::fx::{EffectDef, EffectMetadata, EffectSchema};
use lumit_fx_macros::Effect;

/// Echo's controls.
#[derive(Debug, Clone, Copy, PartialEq, Effect)]
#[effect(
    match_name = "echo",
    label = "Echo",
    version = 2,
    category = Temporal,
    // It reads whole neighbour frames.
    cost = Cheap,
    roi = FullFrame,
    // The 16-frame window (FX-17/K-149, raised from 8) the render decodes for.
    temporal = &[0, -1, -2, -3, -4, -5, -6, -7, -8, -9, -10, -11, -12, -13, -14, -15, -16],
    // K-429: the matte scales the amount, inside the kernel (the owner's rule
    // for mattes); the generic strength dissolve does not also run.
    matte = (
        "matte",
        "scales Decay per pixel: the trail dies away sooner where the matte is \
         dark and reaches its full length where it is white, so the ghosts are \
         genuinely shorter rather than faded back",
    ),
)]
pub struct Echo {
    /// Count of trailing frames; each is one comp frame further back (v1 fixed
    /// spacing). Capped at the 16-frame window (FX-17/K-149, raised from 8).
    #[slider(min = 1.0, max = 16.0, default = 4.0, hard_min = 1.0, hard_max = 16.0, unit = Raw)]
    pub echoes: f32,

    /// Per-echo intensity falloff: echo *k* has intensity `decay^k`.
    #[slider(min = 0.0, max = 1.0, default = 0.6, hard_min = 0.0, hard_max = 1.0, unit = Raw)]
    pub decay: f32,

    /// Two effect-only compositing ORDERS first, then a divider (T21), then the
    /// order-independent light-combine blend modes. Behind draws each echo behind
    /// the trail (ghosting); In front over it (the old "Normal"). The HSL / burn
    /// / dodge modes a layer offers are omitted: they are ill-defined on a
    /// premultiplied light trail (see §3.13 Open questions). Pre-release, no
    /// migration — old stored indices simply re-map.
    #[choice(
        options = [
            "Behind",
            "In front",
            "Add",
            "Screen",
            "Multiply",
            "Overlay",
            "Soft light",
            "Hard light",
            "Lighten",
            "Darken",
            "Difference",
            "Exclusion",
            "Subtract",
            "Divide"
        ],
        default = 3, // Screen
        dividers_after = &[1] // divider after In front
    )]
    pub mode: u32,

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

impl Echo {
    /// The geometric trail, the combine blend and the mix — clamped exactly as
    /// the old resolve arm clamped them (docs/impl/effect-registry.md §2.4):
    /// `weights[i]` is the intensity of the echo at frame offset `-(i+1)`, zero
    /// past the count, the count rounds and clamps to the 1..=16 window, and the
    /// mode clamps to the 0..=13 range the CPU oracle and WGSL kernel branch
    /// over. Both render paths read this one method, so the CPU reference and the
    /// WGSL kernel cannot drift apart.
    pub fn packed(self) -> ([f32; 16], u32, f32) {
        let count = (self.echoes.round() as i32).clamp(1, 16);
        let decay = self.decay.clamp(0.0, 1.0);
        let mut weights = [0.0f32; 16];
        for (i, w) in weights.iter_mut().enumerate() {
            if (i as i32) < count {
                *w = decay.powi(i as i32 + 1);
            }
        }
        (
            weights,
            self.mode.min(13),
            (self.mix / 100.0).clamp(0.0, 1.0),
        )
    }
}

/// Echo's behaviour: no CPU reference (the neighbours are textures), so
/// `apply_cpu` keeps its identity default — the passthrough the old
/// `Resolved::Echo` arm was.
pub struct EchoDef;

impl EffectDef for EchoDef {
    fn schema(&self) -> &'static EffectSchema {
        &<Echo as EffectMetadata>::SCHEMA
    }
}

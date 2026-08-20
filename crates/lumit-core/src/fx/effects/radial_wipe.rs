//! Radial wipe (docs/08 §3.47): a wedge swept round a centre — AE's Radial
//! Wipe.
//!
//! **In plain terms.** A hand sweeps round a clock face and takes the picture
//! with it. Start angle is where the hand begins, Wipe which way it turns (or
//! both ways at once, opening like a pair of curtains), Completion how far round
//! it has got, and Feather how soft the trailing edge is.

use crate::fx::{cpu, EffectDef, EffectMetadata, EffectSchema, Params};
use lumit_fx_macros::Effect;

/// The three sweep directions, in schema order.
pub const WIPE_OPTIONS: &[&str] = &["Clockwise", "Anticlockwise", "Both"];

/// Radial wipe's controls.
#[derive(Debug, Clone, Copy, PartialEq, Effect)]
#[effect(
    match_name = "radial_wipe",
    label = "Radial wipe",
    version = 1,
    category = Transition,
    // One `atan2` a pixel — docs/08 §3.42's admission again, recorded by K-399:
    // the angle IS a function of the pixel and cannot be lifted host-side.
    cost = Cheap,
    roi = Exact,
    premultiplied = true,
)]
pub struct RadialWipe {
    /// Where the hand pivots, px@comp (K-260: point parameters are PIXELS). The
    /// schema default is nominal 1080p centre; `instantiate_for_raster` centres
    /// a fresh instance on the actual comp.
    #[slider(label = "Wipe centre x", min = 0.0, max = 3840.0, default = 960.0, unit = Px)]
    pub centre_x: f32,

    /// px@comp; see [`centre_x`](Self::centre_x).
    #[slider(label = "Wipe centre y", min = 0.0, max = 2160.0, default = 540.0, unit = Px)]
    pub centre_y: f32,

    /// How far round the sweep has got, per cent. **50 by default, where AE's
    /// is 0**, for docs/08 §3.39's reason (§1.2: no no-op defaults).
    #[slider(
        min = 0.0,
        max = 100.0,
        default = 50.0,
        hard_min = 0.0,
        hard_max = 100.0
    )]
    pub completion: f32,

    /// Where the hand begins, degrees, measured from straight up and turning
    /// clockwise (§3.43's convention, and AE's).
    #[dial(label = "Start angle", default = 0.0, step = 15.0)]
    pub start_angle: f32,

    /// Which way the wedge opens. Both opens it symmetrically about Start
    /// angle, like a pair of curtains, and removes the same fraction of the
    /// circle at the same Completion as either single direction does.
    #[choice(options = *WIPE_OPTIONS, default = 0)]
    pub wipe: u32,

    /// How soft the sweeping edge is, px@comp — **a width measured at the arc**,
    /// so it stays the same thickness as it sweeps outward rather than fanning
    /// open. Keeps AE's 0, for §3.46's reason.
    #[slider(min = 0.0, max = 500.0, default = 0.0, hard_min = 0.0, unit = Px)]
    pub feather: f32,

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

impl RadialWipe {
    /// The bundle both kernels consume (docs/impl/effect-registry.md §2.4).
    ///
    /// The three modes collapse to one number here: which way the wedge's middle
    /// sits from Start angle. +1 clockwise, −1 anticlockwise, 0 for Both, which
    /// leaves the middle *on* the start ray — one expression in both kernels
    /// rather than three branches (docs/08 §3.47).
    #[must_use]
    pub fn packed(self) -> cpu::RadialWipeParams {
        cpu::RadialWipeParams {
            centre: [self.centre_x, self.centre_y],
            start: self.start_angle.to_radians(),
            dir: match self.wipe {
                1 => -1.0,
                2 => 0.0,
                _ => 1.0,
            },
            completion: (self.completion / 100.0).clamp(0.0, 1.0),
            feather: self.feather.max(0.0).max(1e-3),
            mix: (self.mix / 100.0).clamp(0.0, 1.0),
        }
    }
}

/// Radial wipe's behaviour.
pub struct RadialWipeDef;

impl EffectDef for RadialWipeDef {
    fn schema(&self) -> &'static EffectSchema {
        &<RadialWipe as EffectMetadata>::SCHEMA
    }

    fn apply_cpu(&self, rgba: &mut [f32], w: u32, h: u32, p: Params<'_>) {
        cpu::radial_wipe(rgba, w, h, &RadialWipe::read(p).packed());
    }
}

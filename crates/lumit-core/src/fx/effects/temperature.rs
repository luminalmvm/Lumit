//! Temperature (docs/08 §3.16): the warm/cool channel gain pair.

use crate::fx::{cpu, EffectDef, EffectMetadata, EffectSchema, Params};
use lumit_fx_macros::Effect;

/// Temperature's controls.
#[derive(Debug, Clone, Copy, PartialEq, Effect)]
#[effect(
    match_name = "temperature",
    label = "Temperature",
    version = 1,
    category = Colour,
    cost = Cheap,
    roi = Exact,
    // K-395: the matte scales the amount, inside the kernel (the owner's
    // rule for mattes); the generic strength dissolve does not also run.
    matte = (
        "matte",
        "scales Temperature toward 0 per pixel: white applies the full \
         shift, grey a milder one, black none",
    ),
)]
pub struct Temperature {
    /// A plain number: negative cools (blue up, red down), positive warms (red
    /// up, blue down). 0 is neutral. The slider reaches ±150 and the hard range
    /// ±200 (K-135): with the stronger ±0.75·k gain, ±150 already pushes one
    /// channel toward black, so a user rarely runs out of headroom wanting more.
    #[slider(
        min = -150.0,
        max = 150.0,
        default = 0.0,
        hard_min = -200.0,
        hard_max = 200.0
    )]
    pub temperature: f32,

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

impl Temperature {
    /// The two channel gains, computed on the host so the CPU reference and the
    /// WGSL kernel multiply by byte-identical f32 factors (docs/08 §1.6).
    ///
    /// k = Temperature / 100, clamped to the ±2 hard range. The stronger ±0.75·k
    /// gain (K-135) makes full deflection a decisive orange/blue; the gains floor
    /// at 0 so an extreme never drives a channel negative. Temperature 0 → k 0 →
    /// gains exactly (1.0, 1.0), the neutral point.
    pub fn gains(self) -> (f32, f32) {
        cpu::temperature_gains(self.t())
    }

    /// Temperature ÷ 100, clamped to the ±2 hard range: the one number the
    /// gains are made from, and what the matted kernel rebuilds them from per
    /// pixel (K-395).
    #[must_use]
    pub fn t(self) -> f32 {
        (self.temperature / 100.0).clamp(-2.0, 2.0)
    }

    /// The two gains the kernel multiplies by, and the mix
    /// (docs/impl/effect-registry.md §2.4). Both render paths read this one
    /// method, so the CPU reference and the WGSL kernel cannot drift apart.
    pub fn packed(self) -> (f32, f32, f32) {
        let (gain_r, gain_b) = self.gains();
        (gain_r, gain_b, (self.mix / 100.0).clamp(0.0, 1.0))
    }
}

/// Temperature's behaviour.
pub struct TemperatureDef;

impl EffectDef for TemperatureDef {
    fn schema(&self) -> &'static EffectSchema {
        &<Temperature as EffectMetadata>::SCHEMA
    }

    fn apply_cpu(&self, rgba: &mut [f32], _w: u32, _h: u32, p: Params<'_>) {
        let (gain_r, gain_b, mix) = Temperature::read(p).packed();
        cpu::temperature(rgba, gain_r, gain_b, mix);
    }
}

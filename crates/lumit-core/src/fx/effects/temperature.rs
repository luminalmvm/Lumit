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
        let k = (self.temperature / 100.0).clamp(-2.0, 2.0);
        ((1.0 + 0.75 * k).max(0.0), (1.0 - 0.75 * k).max(0.0))
    }
}

/// Temperature's behaviour.
pub struct TemperatureDef;

impl EffectDef for TemperatureDef {
    fn schema(&self) -> &'static EffectSchema {
        &<Temperature as EffectMetadata>::SCHEMA
    }

    fn apply_cpu(&self, rgba: &mut [f32], _w: u32, _h: u32, p: Params<'_>) {
        let v = Temperature::read(p);
        let (gain_r, gain_b) = v.gains();
        cpu::temperature(rgba, gain_r, gain_b, (v.mix / 100.0).clamp(0.0, 1.0));
    }
}

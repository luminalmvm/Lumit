//! Temperature (docs/08 §3.16): the warm/cool channel gain pair, and the
//! green/magenta tint that goes with it.

use crate::fx::{
    cpu, params::Params, registry::EffectDef, registry::EffectMetadata, schema::EffectSchema,
};
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
    // The matte scales the amount, inside the kernel (the owner's rule for
    // mattes); the generic strength dissolve does not also run.
    matte = (
        "matte",
        "scales Temperature toward 0 per pixel: white applies the full \
         shift, grey a milder one, black none",
    ),
)]
pub struct Temperature {
    /// A plain number: negative cools (blue up, red down), positive warms (red
    /// up, blue down). 0 is neutral. The slider reaches ±150 and the hard range
    /// ±200: with the stronger ±0.75·k gain, ±150 already pushes one
    /// channel toward black, so a user rarely runs out of headroom wanting more.
    #[slider(
        min = -150.0,
        max = 150.0,
        default = 0.0,
        hard_min = -200.0,
        hard_max = 200.0,
        unit = Raw
    )]
    pub temperature: f32,

    /// The other axis of white balance: negative goes green (green up),
    /// positive magenta (green down). 0 is neutral, and a project saved
    /// before this row existed reads 0.
    #[slider(
        min = -150.0,
        max = 150.0,
        default = 0.0,
        hard_min = -200.0,
        hard_max = 200.0,
        unit = Raw
    )]
    pub tint: f32,

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

impl Temperature {
    /// The two channel gains, computed on the host so the CPU reference and the
    /// WGSL kernel multiply by byte-identical f32 factors (docs/08 §1.6).
    ///
    /// k = Temperature / 100, clamped to the ±2 hard range. The stronger ±0.75·k
    /// gain makes full deflection a decisive orange/blue; the gains floor
    /// at 0 so an extreme never drives a channel negative. Temperature 0 → k 0 →
    /// gains exactly (1.0, 1.0), the neutral point.
    pub fn gains(self) -> (f32, f32) {
        cpu::temperature_gains(self.t())
    }

    /// Temperature ÷ 100, clamped to the ±2 hard range: the one number the
    /// gains are made from, and what the matted kernel rebuilds them from per
    /// pixel.
    #[must_use]
    pub fn t(self) -> f32 {
        (self.temperature / 100.0).clamp(-2.0, 2.0)
    }

    /// Tint ÷ 100, clamped the same way: what the green gain is made from.
    #[must_use]
    pub fn tint_k(self) -> f32 {
        (self.tint / 100.0).clamp(-2.0, 2.0)
    }

    /// The three gains the kernel multiplies by (red, green, blue), and the
    /// mix (docs/impl/effect-registry.md §2.4). Both render paths read this
    /// one method, so the CPU reference and the WGSL kernel cannot drift
    /// apart.
    pub fn packed(self) -> ([f32; 3], f32) {
        let (gain_r, gain_b) = self.gains();
        let gain_g = cpu::tint_gain(self.tint_k());
        ([gain_r, gain_g, gain_b], (self.mix / 100.0).clamp(0.0, 1.0))
    }
}

/// The Temperature and Tint that turn `picked` grey, as the sliders read them.
///
/// `picked` is a colour off a picture already showing the `now` (Temperature,
/// Tint), so those gains come off it first and a second pick starts from the
/// shot rather than from the first pick. Red and blue meet first, on the two
/// gains Temperature moves in opposite directions, then green comes to where
/// they met. Black has no colour to read, so it leaves `now` as it is.
#[must_use]
pub fn neutral_for(picked: [f32; 3], now: (f32, f32)) -> (f32, f32) {
    let ([gain_r, gain_g, gain_b], _) = Temperature {
        temperature: now.0,
        tint: now.1,
        mix: 100.0,
    }
    .packed();
    // A gain floored at nought has lost its channel, so there is nothing to undo.
    let shot = |channel: f32, gain: f32| if gain > 0.0 { channel / gain } else { channel };
    let (r, g, b) = (
        shot(picked[0], gain_r),
        shot(picked[1], gain_g),
        shot(picked[2], gain_b),
    );
    if r + b <= 1e-5 || g <= 1e-5 {
        return now;
    }
    let t = (b - r) / (0.75 * (b + r));
    let met = r * cpu::temperature_gains(t).0;
    let tint = (1.0 - met / g) / 0.75;
    (
        (t * 100.0).clamp(-200.0, 200.0),
        (tint * 100.0).clamp(-200.0, 200.0),
    )
}

/// Temperature's behaviour.
pub struct TemperatureDef;

impl EffectDef for TemperatureDef {
    fn schema(&self) -> &'static EffectSchema {
        &<Temperature as EffectMetadata>::SCHEMA
    }

    fn apply_cpu(&self, rgba: &mut [f32], _w: u32, _h: u32, p: Params<'_>) {
        let (gains, mix) = Temperature::read(p).packed();
        cpu::temperature_matted(rgba, gains, 0.0, 0.0, mix, &[]);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A pick has to leave the picked colour grey, however far the row had
    /// already moved, or white balance lands on the wrong picture.
    #[test]
    fn a_pick_turns_the_picked_colour_grey() {
        let shot = [0.42_f32, 0.3, 0.18];
        for now in [(0.0, 0.0), (40.0, -25.0), (-70.0, 30.0)] {
            let gains = |t: f32, tint: f32| {
                Temperature {
                    temperature: t,
                    tint,
                    mix: 100.0,
                }
                .packed()
                .0
            };
            let shown = gains(now.0, now.1);
            let picked = [shot[0] * shown[0], shot[1] * shown[1], shot[2] * shown[2]];
            let (t, tint) = neutral_for(picked, now);
            let fixed = gains(t, tint);
            let out = [shot[0] * fixed[0], shot[1] * fixed[1], shot[2] * fixed[2]];
            assert!((out[0] - out[1]).abs() < 1e-4, "{out:?} from {now:?}");
            assert!((out[2] - out[1]).abs() < 1e-4, "{out:?} from {now:?}");
        }
        assert_eq!(neutral_for([0.0, 0.0, 0.0], (12.0, 3.0)), (12.0, 3.0));
    }
}

//! sRGB curve: the picture onto the sRGB curve, or back off it.
//!
//! Lumit works in linear light. Some grades were made on the encoded picture
//! instead, the way Photoshop makes them, and give a different picture in
//! linear light from the same numbers. Put one of these either side of such a
//! grade, Linear to sRGB before and sRGB to linear after, and the grade sees
//! the numbers it was made on.

use crate::fx::{
    params::Params, registry::EffectDef, registry::EffectMetadata, schema::EffectSchema,
};
use lumit_fx_macros::Effect;

/// sRGB curve's controls.
#[derive(Debug, Clone, Copy, PartialEq, Effect)]
#[effect(
    match_name = "srgb_curve",
    label = "sRGB curve",
    version = 1,
    category = Colour,
    cost = Cheap,
    roi = Exact,
    // A curve on premultiplied colour would bend a soft edge by its coverage.
    premultiplied = false,
)]
pub struct SrgbCurve {
    #[choice(options = ["Linear to sRGB", "sRGB to linear"], default = 0)]
    pub direction: u32,

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

impl SrgbCurve {
    /// True to decode, and the mix. Both render paths read this one method.
    pub fn packed(self) -> (bool, f32) {
        (self.direction == 1, (self.mix / 100.0).clamp(0.0, 1.0))
    }
}

/// One channel onto the sRGB curve, or back off it. A value under zero is
/// mirrored and a value over one carries on along the curve, so the two
/// directions undo each other everywhere.
#[must_use]
pub fn srgb_curve_at(v: f32, decode: bool) -> f32 {
    let x = v.abs();
    let y = match decode {
        false if x <= 0.003_130_8 => x * 12.92,
        false => 1.055 * x.powf(1.0 / 2.4) - 0.055,
        true if x <= 0.040_45 => x / 12.92,
        true => ((x + 0.055) / 1.055).powf(2.4),
    };
    y.copysign(v)
}

/// sRGB curve's behaviour.
pub struct SrgbCurveDef;

impl EffectDef for SrgbCurveDef {
    fn schema(&self) -> &'static EffectSchema {
        &<SrgbCurve as EffectMetadata>::SCHEMA
    }

    fn apply_cpu(&self, rgba: &mut [f32], _w: u32, _h: u32, p: Params<'_>) {
        let (decode, mix) = SrgbCurve::read(p).packed();
        for px in rgba.chunks_exact_mut(4) {
            let a = px[3];
            if a <= 0.0 {
                continue;
            }
            for c in &mut px[..3] {
                let curved = srgb_curve_at(*c / a, decode) * a;
                *c = *c * (1.0 - mix) + curved * mix;
            }
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use crate::model::EffectValue;

    fn curve(direction: u32) -> crate::model::EffectInstance {
        let mut curve = crate::fx::builtins::instantiate("srgb_curve").unwrap();
        let param = curve
            .params
            .iter_mut()
            .find(|p| p.id == "direction")
            .unwrap();
        param.value = EffectValue::Choice(direction);
        curve
    }

    fn run(effects: &[crate::model::EffectInstance], rgba: &mut [f32]) {
        let ops = crate::fx::resolved::resolve_stack(
            effects,
            0.0,
            1.0,
            1.0,
            &crate::fx::markers::MarkerContext::default(),
            std::sync::Arc::new(crate::expression::ExpressionContext::detached()),
        );
        crate::fx::cpu::apply_stack(rgba, 1, (rgba.len() / 4) as u32, &ops);
    }

    // An imported Photoshop adjustment sits between these two, so a wrong
    // curve is a wrong picture.
    #[test]
    fn the_curve_lands_on_srgb_and_the_two_directions_undo_each_other() {
        let start = [0.214, 0.001, -0.5, 1.0, 0.25, 0.5, 2.0, 0.5];
        let mut encoded = start;
        run(&[curve(0)], &mut encoded);
        let want = [0.5, 0.012_92, -0.735_4, 1.0, 0.367_7, 0.5, 0.912_4, 0.5];
        for (got, want) in encoded.iter().zip(want) {
            assert!((got - want).abs() < 1e-3, "{encoded:?}");
        }
        let mut back = start;
        run(&[curve(0), curve(1)], &mut back);
        for (got, want) in back.iter().zip(start) {
            assert!((got - want).abs() < 1e-5, "{back:?}");
        }
    }
}

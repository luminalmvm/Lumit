//! Light leak: a warm band of light across the picture, as if the film had
//! been fogged at the edge of the gate. It is brightest in a soft band, turns
//! from Colour at its heart to Edge colour at its fringe, and is broken up by
//! two layers of noise that drift with time.
//!
//! The light is screened onto the picture, so it lifts the dark parts most and
//! a part already at full brightness gains nothing, rather than piling up into
//! flat colour. Where the picture is clear nothing is added.

use crate::fx::{
    noise::value3, params::ParamId, params::Params, params::Value, presets::EffectPreset,
    presets::PresetValue, registry::EffectDef, registry::EffectMetadata, registry::ResolveCx,
    schema::EffectSchema,
};
use lumit_fx_macros::Effect;

/// Light leak's controls.
#[derive(Debug, Clone, Copy, PartialEq, Effect)]
#[effect(
    match_name = "light_leak",
    label = "Light leak",
    version = 1,
    category = Stylise,
    // Two noise taps a pixel.
    cost = Cheap,
    roi = Exact,
    // The noise drifts with time under fixed controls.
    seeded = true,
    premultiplied = true,
)]
pub struct LightLeak {
    /// How strong the light is, per cent.
    #[slider(
        min = 0.0,
        max = 100.0,
        default = 70.0,
        hard_min = 0.0,
        hard_max = 400.0,
        unit = Percent
    )]
    pub amount: f32,

    /// The light at the heart of the band, scene-linear.
    #[colour(default = [1.0, 0.45, 0.12, 1.0], max = 4.0)]
    pub colour: [f32; 4],

    /// The light at the band's fringe, scene-linear.
    #[colour(label = "Edge colour", default = [1.0, 0.08, 0.03, 1.0], max = 4.0)]
    pub colour_2: [f32; 4],

    /// Which way the band lies, degrees. 0 is upright.
    #[dial(default = 20.0, step = 15.0)]
    pub angle: f32,

    /// How wide the band is, per cent of the picture's height.
    #[slider(
        min = 10.0,
        max = 200.0,
        default = 60.0,
        hard_min = 1.0,
        hard_max = 1000.0,
        unit = Percent
    )]
    pub size: f32,

    /// How fast the light changes, in noise cells a second.
    #[slider(min = 0.0, max = 4.0, default = 1.0, hard_min = 0.0, unit = Raw)]
    pub drift: f32,

    /// Per instance, so two leaks do not match.
    #[seed]
    pub seed: u32,

    /// The host-uniform Mix every effect ends with, per cent.
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

/// One frame of Light leak, as both render paths draw it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LightLeakParams {
    /// The band's normal over its width, so `dot(uv, normal)` is the distance
    /// across it in band widths.
    pub normal: [f32; 2],
    /// The picture's centre, raster pixels.
    pub centre: [f32; 2],
    /// One over the picture's height, so `uv` is in heights.
    pub inv_h: f32,
    /// The depth of the coarse and the fine noise.
    pub z: [f32; 2],
    /// Colour at the heart, times Amount.
    pub colour: [f32; 3],
    /// Edge colour, times Amount.
    pub colour_2: [f32; 3],
    pub seed: u32,
    /// 0..1, blended against the unprocessed input.
    pub mix: f32,
}

impl LightLeakParams {
    /// No light at all: the picture goes through untouched, bit for bit.
    #[must_use]
    pub fn is_identity(&self) -> bool {
        self.mix == 0.0 || (self.colour == [0.0; 3] && self.colour_2 == [0.0; 3])
    }
}

impl LightLeak {
    /// Layer time, seconds. Never a panel row.
    pub const DERIVED_TIME: ParamId = ParamId::new("derived.time");

    /// The numbers both kernels read, from the rows and the time in `p`, on a
    /// `w` by `h` raster.
    #[must_use]
    pub fn packed(self, p: Params<'_>, w: u32, h: u32) -> LightLeakParams {
        let t = p.float(Self::DERIVED_TIME, 0.0);
        let amount = self.amount.max(0.0) / 100.0;
        let size = (self.size / 100.0).max(1e-3);
        let (sin, cos) = f64::from(self.angle).to_radians().sin_cos();
        let z = t * self.drift.max(0.0);
        let tint = |c: [f32; 4]| [c[0] * amount, c[1] * amount, c[2] * amount];
        LightLeakParams {
            normal: [cos as f32 / size, sin as f32 / size],
            centre: [w as f32 * 0.5, h as f32 * 0.5],
            inv_h: 1.0 / (h.max(1) as f32),
            z: [z, z * 1.7],
            colour: tint(self.colour),
            colour_2: tint(self.colour_2),
            seed: self.seed,
            mix: (self.mix / 100.0).clamp(0.0, 1.0),
        }
    }
}

/// How much light falls on the pixel at `x`, `y`, 0..1. The WGSL twin is the
/// same expression in the same order.
#[must_use]
pub fn light_leak_light(x: u32, y: u32, p: &LightLeakParams) -> f32 {
    let u = (x as f32 + 0.5 - p.centre[0]) * p.inv_h;
    let v = (y as f32 + 0.5 - p.centre[1]) * p.inv_h;
    let g = u * p.normal[0] + v * p.normal[1];
    let band = (-(g * g)).exp();
    let n1 = value3(p.seed, 0, u * 2.0, v * 2.0, p.z[0], 0);
    let n2 = value3(p.seed, 1, u * 5.0, v * 5.0, p.z[1], 0);
    (band * (0.55 + 0.9 * n1) + 0.35 * n2 * band).clamp(0.0, 1.0)
}

/// The CPU oracle. The light is screened onto the premultiplied picture: each
/// channel gains `leak × (alpha − channel)`, never less than nothing.
pub fn light_leak(rgba: &mut [f32], w: u32, p: &LightLeakParams) {
    if p.is_identity() || w == 0 {
        return;
    }
    for (i, px) in rgba.chunks_exact_mut(4).enumerate() {
        let (x, y) = (i as u32 % w, i as u32 / w);
        let l = light_leak_light(x, y, p);
        let a = px[3];
        for (c, (hot, edge)) in px.iter_mut().zip(p.colour.iter().zip(&p.colour_2)) {
            let leak = (edge + (hot - edge) * l) * l;
            *c += p.mix * leak * (a - *c).max(0.0);
        }
    }
}

/// Light leak's looks, which the Transition menu offers too.
pub const LIGHT_LEAK_LOOKS: &[EffectPreset] = &[
    EffectPreset {
        name: "Light leak",
        values: &[],
    },
    EffectPreset {
        name: "Film burn",
        values: &[
            ("amount", PresetValue::Float(200.0)),
            ("size", PresetValue::Float(110.0)),
            ("drift", PresetValue::Float(2.5)),
        ],
    },
];

/// Light leak's behaviour.
pub struct LightLeakDef;

impl EffectDef for LightLeakDef {
    fn schema(&self) -> &'static EffectSchema {
        &<LightLeak as EffectMetadata>::SCHEMA
    }

    fn presets(&self) -> &'static [EffectPreset] {
        LIGHT_LEAK_LOOKS
    }

    fn resolve_derived(&self, cx: &ResolveCx<'_>, push: &mut dyn FnMut(ParamId, Value)) {
        push(LightLeak::DERIVED_TIME, Value::Float(cx.lt as f32));
    }

    fn apply_cpu(&self, rgba: &mut [f32], w: u32, h: u32, p: Params<'_>) {
        light_leak(rgba, w, &LightLeak::read(p).packed(p, w, h));
    }
}

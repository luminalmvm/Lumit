//! CDL: the ASC Color Decision List's slope, offset, power and saturation,
//! exactly as version 1.2 of the list writes them.
//!
//! **In plain terms.** The grade a set or another grading tool hands over as
//! ten numbers. It is worked in its Space, as a Correction is, because the
//! numbers only mean what they meant where they were made: on a video signal
//! or a log one. The list's own clamps hold every value to 0..1, so a CDL
//! node clips highlights a Correction would keep. One with every row at rest
//! hands the picture on untouched, clamps and all.
//!
//! # The formulas
//!
//! `v = E(u)`, `u` the unpremultiplied pixel and `E` the Correction's encode
//! for the Space ([`maths::encode`]). Per channel
//! `x = clamp(v·slope + offset, 0, 1)^power`, power held above nought.
//! Then `Y = 0.2126x.r + 0.7152x.g + 0.0722x.b` and
//! `out = clamp(Y + saturation·(x − Y), 0, 1)`.
//! Last `w = v·(1 − mix) + out·mix`, decoded and premultiplied again.

use crate::fx::effects::correction::{maths, SPACE_OPTIONS};
use crate::fx::{
    params::Params, registry::EffectDef, registry::EffectMetadata, schema::EffectSchema,
};
use lumit_fx_macros::Effect;

/// The match name.
pub const CDL: &str = "cdl";

/// How many `vec4`s [`CdlPacked::words`] writes: `fx_grade.wgsl`'s `Params`.
pub const WORDS: usize = 16;

/// CDL's controls.
#[derive(Debug, Clone, Copy, PartialEq, Effect)]
#[effect(
    match_name = "cdl",
    label = "CDL",
    version = 1,
    category = Colour,
    cost = Cheap,
    roi = Exact,
    // A grade shifts matte edges when worked premultiplied.
    premultiplied = false,
)]
pub struct Cdl {
    /// The signal the numbers were made on: gamma 2.4, ACEScct, or scene
    /// linear.
    #[choice(options = SPACE_OPTIONS, default = 0)]
    pub space: u32,
    /// Multiplies each channel.
    #[colour(default = [1.0, 1.0, 1.0, 1.0], min = 0.0, max = 4.0)]
    pub slope: [f32; 4],
    /// Adds to each channel, after Slope.
    #[colour(default = [0.0, 0.0, 0.0, 1.0], min = -1.0, max = 1.0)]
    pub offset: [f32; 4],
    /// The exponent each channel is raised to, last.
    #[colour(default = [1.0, 1.0, 1.0, 1.0], min = 0.0, max = 4.0)]
    pub power: [f32; 4],
    /// Colourfulness about Rec. 709 luma.
    #[slider(min = 0.0, max = 2.0, default = 1.0, hard_min = 0.0, hard_max = 10.0, unit = Raw)]
    pub saturation: f32,
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

/// CDL's numbers, ready to draw.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CdlPacked {
    pub space: u32,
    pub slope: [f32; 3],
    pub offset: [f32; 3],
    pub power: [f32; 3],
    pub saturation: f32,
    pub mix: f32,
    /// Every row at rest, or Mix 0.
    pub neutral: bool,
}

impl Cdl {
    /// What both render paths draw with.
    #[must_use]
    pub fn packed(&self) -> CdlPacked {
        let rgb = |c: [f32; 4]| [c[0], c[1], c[2]];
        let mut pk = CdlPacked {
            space: if self.space <= 2 { self.space } else { 0 },
            slope: rgb(self.slope),
            offset: rgb(self.offset),
            // The list wants a power above nought.
            power: rgb(self.power).map(|p| p.max(1e-4)),
            saturation: self.saturation.max(0.0),
            mix: (self.mix / 100.0).clamp(0.0, 1.0),
            neutral: false,
        };
        pk.neutral = pk.mix == 0.0
            || (pk.slope == [1.0; 3]
                && pk.offset == [0.0; 3]
                && pk.power == [1.0; 3]
                && pk.saturation == 1.0);
        pk
    }
}

impl CdlPacked {
    /// The uniform `fx_grade.wgsl`'s `cdl` reads.
    #[must_use]
    pub fn words(&self) -> [[f32; 4]; WORDS] {
        let v3 = |v: [f32; 3]| [v[0], v[1], v[2], 0.0];
        let mut out = [[0.0f32; 4]; WORDS];
        out[0] = [self.space as f32, self.saturation, self.mix, 0.0];
        out[1] = v3(self.slope);
        out[2] = v3(self.offset);
        out[3] = v3(self.power);
        out
    }

    /// One premultiplied pixel through the list.
    #[must_use]
    pub fn pixel(&self, o: [f32; 4]) -> [f32; 4] {
        let v = maths::v_in(o, self.space);
        let x = [0, 1, 2].map(|c| {
            let x = (v[c] * self.slope[c] + self.offset[c]).clamp(0.0, 1.0);
            if x > 0.0 {
                x.powf(self.power[c])
            } else {
                0.0
            }
        });
        let y = maths::luma(x);
        let a = o[3];
        let out = [0, 1, 2].map(|c| {
            let g = (y + self.saturation * (x[c] - y)).clamp(0.0, 1.0);
            let w = v[c] * (1.0 - self.mix) + g * self.mix;
            maths::decode(w, self.space) * a
        });
        [out[0], out[1], out[2], a]
    }
}

/// CDL's behaviour.
pub struct CdlDef;

impl EffectDef for CdlDef {
    fn schema(&self) -> &'static EffectSchema {
        &<Cdl as EffectMetadata>::SCHEMA
    }

    fn apply_cpu(&self, rgba: &mut [f32], _w: u32, _h: u32, p: Params<'_>) {
        let pk = Cdl::read(p).packed();
        if pk.neutral {
            return;
        }
        for px in rgba.chunks_exact_mut(4) {
            let out = pk.pixel([px[0], px[1], px[2], px[3]]);
            px.copy_from_slice(&out);
        }
    }
}

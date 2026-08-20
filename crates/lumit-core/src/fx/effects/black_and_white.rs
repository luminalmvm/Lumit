//! Black and white (docs/08 §3.62): six weights, one grey — AE's Black & White.
//!
//! **In plain terms.** Turning a colour picture grey throws information away, and
//! *which* information decides whether the result is a photograph or a mush. A
//! flat conversion gives a red jumper and the green grass behind it the same
//! grey, and the jumper vanishes. This effect asks six questions instead — how
//! bright should reds be, and yellows, and greens, and cyans, and blues, and
//! magentas — so a photographer can darken a sky until the clouds stand out, the
//! way a red filter on black-and-white film always did.
//!
//! The maths under it is a decomposition, not a weighted sum: every colour is
//! written exactly as a grey plus one *secondary* (yellow, cyan or magenta) plus
//! one *primary* (red, green or blue), and the two weights that apply are the
//! two those parts name. On a grey pixel both parts are zero, so the six sliders
//! do nothing at all — which is what makes them trustworthy.

use crate::fx::{cpu, EffectDef, EffectMetadata, EffectSchema, EnabledCond, EnabledWhen, Params};
use lumit_fx_macros::Effect;

/// A tint colour means nothing until the tint is on.
pub const BLACK_AND_WHITE_ENABLED_WHEN: &[EnabledWhen] = &[EnabledWhen {
    param: "tint_colour",
    on: "tint",
    cond: EnabledCond::BoolIs(true),
}];

/// Black and white's controls. The six defaults are AE's, and they are a real
/// conversion rather than a no-op (§3.10).
#[derive(Debug, Clone, Copy, PartialEq, Effect)]
#[effect(
    match_name = "black_and_white",
    label = "Black and white",
    version = 1,
    category = Colour,
    cost = Cheap,
    roi = Exact,
    // §2.2: the decomposition is about the pixel's own colour, not about how
    // much of it there is.
    premultiplied = false,
    enabled_when = BLACK_AND_WHITE_ENABLED_WHEN,
)]
pub struct BlackAndWhite {
    /// Per cent: how bright the red part of a colour comes out. Raise it to
    /// lift skin and brick, lower it to darken them.
    #[slider(min = -200.0, max = 300.0, default = 40.0, hard_min = -200.0)]
    pub reds: f32,

    /// Per cent; see [`reds`](Self::reds).
    #[slider(min = -200.0, max = 300.0, default = 60.0, hard_min = -200.0)]
    pub yellows: f32,

    /// Per cent; see [`reds`](Self::reds).
    #[slider(min = -200.0, max = 300.0, default = 40.0, hard_min = -200.0)]
    pub greens: f32,

    /// Per cent; see [`reds`](Self::reds).
    #[slider(min = -200.0, max = 300.0, default = 60.0, hard_min = -200.0)]
    pub cyans: f32,

    /// Per cent; see [`reds`](Self::reds). Lower it to darken a sky.
    #[slider(min = -200.0, max = 300.0, default = 20.0, hard_min = -200.0)]
    pub blues: f32,

    /// Per cent; see [`reds`](Self::reds).
    #[slider(min = -200.0, max = 300.0, default = 80.0, hard_min = -200.0)]
    pub magentas: f32,

    /// On, the grey is coloured by [`tint_colour`](Self::tint_colour) —
    /// the sepia print, the cyanotype.
    #[toggle(default = false)]
    pub tint: bool,

    /// Scene-linear RGBA; the alpha is ignored. Divided by its own luma before
    /// use, so it changes the picture's hue and not its exposure (§3.62).
    #[colour(label = "Tint colour", default = [0.62, 0.44, 0.26, 1.0], max = 4.0)]
    pub tint_colour: [f32; 4],

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

impl BlackAndWhite {
    /// The six weights as fractions, the tint already divided through by its own
    /// luma, and the mix (docs/impl/effect-registry.md §2.4). Every division
    /// happens here, once, so neither kernel divides.
    #[must_use]
    pub fn packed(self) -> cpu::BlackAndWhiteParams {
        let t = [
            self.tint_colour[0],
            self.tint_colour[1],
            self.tint_colour[2],
        ];
        let tl = t[0] * cpu::LUMA[0] + t[1] * cpu::LUMA[1] + t[2] * cpu::LUMA[2];
        // A tint of pure black has no hue to give, so it falls back to neutral
        // rather than dividing by zero (docs/14 §4).
        let inv = if tl > 1e-6 { 1.0 / tl } else { 0.0 };
        cpu::BlackAndWhiteParams {
            // Red, yellow, green, cyan, blue, magenta — the order the
            // decomposition indexes them in.
            weights: [
                self.reds / 100.0,
                self.yellows / 100.0,
                self.greens / 100.0,
                self.cyans / 100.0,
                self.blues / 100.0,
                self.magentas / 100.0,
            ],
            tint: [t[0] * inv, t[1] * inv, t[2] * inv],
            // A float rather than a bool so the kernel multiplies instead of
            // branching.
            tint_on: f32::from(u8::from(self.tint)),
            mix: (self.mix / 100.0).clamp(0.0, 1.0),
        }
    }
}

/// Black and white's behaviour.
pub struct BlackAndWhiteDef;

impl EffectDef for BlackAndWhiteDef {
    fn schema(&self) -> &'static EffectSchema {
        &<BlackAndWhite as EffectMetadata>::SCHEMA
    }

    fn apply_cpu(&self, rgba: &mut [f32], _w: u32, _h: u32, p: Params<'_>) {
        cpu::black_and_white(rgba, &BlackAndWhite::read(p).packed());
    }
}

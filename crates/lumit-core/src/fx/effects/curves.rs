//! Curves (docs/08 §3.30, K-396): the per-channel tone curve, as five
//! animatable knots a channel rather than AE's arbitrary point blob.

use crate::fx::{cpu, EffectDef, EffectMetadata, EffectSchema, ParamGroup, Params};
use lumit_fx_macros::Effect;

/// Curves' four channel groups (docs/08 §3.30). Each group's ids are a
/// contiguous run of the schema's `params`, which is what lets the panel draw
/// the twirl in place. Master is the one you reach for first, so it opens; the
/// three colour channels are the second reach and start closed.
pub const CURVES_GROUPS: &[ParamGroup] = &[
    ParamGroup {
        label: "Master",
        params: &[
            "master_black",
            "master_shadows",
            "master_midtones",
            "master_highlights",
            "master_white",
        ],
        collapsed: false,
        visible_when: None,
        visible_when_lens_elements: None,
    },
    ParamGroup {
        label: "Red",
        params: &[
            "red_black",
            "red_shadows",
            "red_midtones",
            "red_highlights",
            "red_white",
        ],
        collapsed: true,
        visible_when: None,
        visible_when_lens_elements: None,
    },
    ParamGroup {
        label: "Green",
        params: &[
            "green_black",
            "green_shadows",
            "green_midtones",
            "green_highlights",
            "green_white",
        ],
        collapsed: true,
        visible_when: None,
        visible_when_lens_elements: None,
    },
    ParamGroup {
        label: "Blue",
        params: &[
            "blue_black",
            "blue_shadows",
            "blue_midtones",
            "blue_highlights",
            "blue_white",
        ],
        collapsed: true,
        visible_when: None,
        visible_when_lens_elements: None,
    },
];

/// Curves' controls: five knots on each of Master, Red, Green and Blue, at the
/// fixed inputs 0, 0.25, 0.5, 0.75 and 1. Each knot is that channel's curve
/// **output** at its input — an ordinary animatable number, unlike AE's
/// arbitrary-data point list, which only ever steps (docs/08 §3.30, K-396).
///
/// Defaults are the identity curve — every output is its own input — so a
/// fresh Curves is the bit-exact passthrough: the grade family's sanctioned
/// exception to the "no no-op default" rule (docs/08 §3.10).
///
/// The knot rows repeat their labels across the four groups on purpose: the
/// group header says which channel, so the row can say what the knot is.
#[derive(Debug, Clone, Copy, PartialEq, Effect)]
#[effect(
    match_name = "curves",
    label = "Curves",
    version = 1,
    category = Colour,
    cost = Cheap,
    roi = Exact,
    // §2.2: a tone curve is non-linear, so it does not commute with
    // premultiplied alpha — grading premult would shift matte edges.
    premultiplied = false,
    groups = CURVES_GROUPS,
)]
pub struct Curves {
    /// Master's output at input 0 — the black point.
    #[slider(min = 0.0, max = 1.0, default = 0.0, hard_min = 0.0, label = "Black")]
    pub master_black: f32,
    /// Master's output at input 0.25.
    #[slider(
        min = 0.0,
        max = 1.0,
        default = 0.25,
        hard_min = 0.0,
        label = "Shadows"
    )]
    pub master_shadows: f32,
    /// Master's output at input 0.5.
    #[slider(
        min = 0.0,
        max = 1.0,
        default = 0.5,
        hard_min = 0.0,
        label = "Midtones"
    )]
    pub master_midtones: f32,
    /// Master's output at input 0.75.
    #[slider(
        min = 0.0,
        max = 1.0,
        default = 0.75,
        hard_min = 0.0,
        label = "Highlights"
    )]
    pub master_highlights: f32,
    /// Master's output at input 1 — the white point.
    #[slider(min = 0.0, max = 1.0, default = 1.0, hard_min = 0.0, label = "White")]
    pub master_white: f32,

    /// Red's output at input 0.
    #[slider(min = 0.0, max = 1.0, default = 0.0, hard_min = 0.0, label = "Black")]
    pub red_black: f32,
    /// Red's output at input 0.25.
    #[slider(
        min = 0.0,
        max = 1.0,
        default = 0.25,
        hard_min = 0.0,
        label = "Shadows"
    )]
    pub red_shadows: f32,
    /// Red's output at input 0.5.
    #[slider(
        min = 0.0,
        max = 1.0,
        default = 0.5,
        hard_min = 0.0,
        label = "Midtones"
    )]
    pub red_midtones: f32,
    /// Red's output at input 0.75.
    #[slider(
        min = 0.0,
        max = 1.0,
        default = 0.75,
        hard_min = 0.0,
        label = "Highlights"
    )]
    pub red_highlights: f32,
    /// Red's output at input 1.
    #[slider(min = 0.0, max = 1.0, default = 1.0, hard_min = 0.0, label = "White")]
    pub red_white: f32,

    /// Green's output at input 0.
    #[slider(min = 0.0, max = 1.0, default = 0.0, hard_min = 0.0, label = "Black")]
    pub green_black: f32,
    /// Green's output at input 0.25.
    #[slider(
        min = 0.0,
        max = 1.0,
        default = 0.25,
        hard_min = 0.0,
        label = "Shadows"
    )]
    pub green_shadows: f32,
    /// Green's output at input 0.5.
    #[slider(
        min = 0.0,
        max = 1.0,
        default = 0.5,
        hard_min = 0.0,
        label = "Midtones"
    )]
    pub green_midtones: f32,
    /// Green's output at input 0.75.
    #[slider(
        min = 0.0,
        max = 1.0,
        default = 0.75,
        hard_min = 0.0,
        label = "Highlights"
    )]
    pub green_highlights: f32,
    /// Green's output at input 1.
    #[slider(min = 0.0, max = 1.0, default = 1.0, hard_min = 0.0, label = "White")]
    pub green_white: f32,

    /// Blue's output at input 0.
    #[slider(min = 0.0, max = 1.0, default = 0.0, hard_min = 0.0, label = "Black")]
    pub blue_black: f32,
    /// Blue's output at input 0.25.
    #[slider(
        min = 0.0,
        max = 1.0,
        default = 0.25,
        hard_min = 0.0,
        label = "Shadows"
    )]
    pub blue_shadows: f32,
    /// Blue's output at input 0.5.
    #[slider(
        min = 0.0,
        max = 1.0,
        default = 0.5,
        hard_min = 0.0,
        label = "Midtones"
    )]
    pub blue_midtones: f32,
    /// Blue's output at input 0.75.
    #[slider(
        min = 0.0,
        max = 1.0,
        default = 0.75,
        hard_min = 0.0,
        label = "Highlights"
    )]
    pub blue_highlights: f32,
    /// Blue's output at input 1.
    #[slider(min = 0.0, max = 1.0, default = 1.0, hard_min = 0.0, label = "White")]
    pub blue_white: f32,

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

impl Curves {
    /// The knots and their monotone-cubic tangents, indexed `[knot][channel]`
    /// with channel 0 Master and 1..3 R/G/B, plus the mix
    /// (docs/impl/effect-registry.md §2.4).
    ///
    /// The spline fit is host maths on purpose: both render paths take the
    /// tangents this produced, so neither fits a curve per pixel and the two
    /// cannot disagree about the shape.
    #[must_use]
    pub fn packed(self) -> ([[f32; 4]; 5], [[f32; 4]; 5], f32) {
        let channels = [
            [
                self.master_black,
                self.master_shadows,
                self.master_midtones,
                self.master_highlights,
                self.master_white,
            ],
            [
                self.red_black,
                self.red_shadows,
                self.red_midtones,
                self.red_highlights,
                self.red_white,
            ],
            [
                self.green_black,
                self.green_shadows,
                self.green_midtones,
                self.green_highlights,
                self.green_white,
            ],
            [
                self.blue_black,
                self.blue_shadows,
                self.blue_midtones,
                self.blue_highlights,
                self.blue_white,
            ],
        ];
        let mut y = [[0.0f32; 4]; 5];
        let mut m = [[0.0f32; 4]; 5];
        for (c, knots) in channels.iter().enumerate() {
            let tangents = cpu::curve_tangents(*knots);
            for i in 0..5 {
                y[i][c] = knots[i];
                m[i][c] = tangents[i];
            }
        }
        (y, m, (self.mix / 100.0).clamp(0.0, 1.0))
    }
}

/// Curves' behaviour.
pub struct CurvesDef;

impl EffectDef for CurvesDef {
    fn schema(&self) -> &'static EffectSchema {
        &<Curves as EffectMetadata>::SCHEMA
    }

    fn apply_cpu(&self, rgba: &mut [f32], _w: u32, _h: u32, p: Params<'_>) {
        let (y, m, mix) = Curves::read(p).packed();
        cpu::curves(rgba, y, m, mix);
    }
}

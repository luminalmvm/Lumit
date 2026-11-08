//! Gradient (docs/08 §3.35): a linear or radial ramp of two to five colours, with scatter.
//!
//! **In plain terms.** Two points and two colours. Everything on the line
//! through the first point is the first colour, everything at the second point
//! is the second, and in between the two are mixed by how far along you are —
//! or, in Radial, by how far *out*. Scatter jiggles that distance by a hair per
//! pixel, which is the standard cure for the contour rings a long, shallow ramp
//! shows on an 8-bit display.

use crate::fx::{
    cpu, params::Params, registry::EffectDef, registry::EffectMetadata, schema::EffectSchema,
    schema::ParamGroup,
};
use lumit_fx_macros::Effect;

/// Each colour in between shows only once Colours counts it.
pub const GRADIENT_GROUPS: &[ParamGroup] = &[
    ParamGroup {
        label: "",
        params: &["colour_2", "position_2"],
        collapsed: false,
        visible_when: Some(("colours", &[1, 2, 3])),
        visible_when_lens_elements: None,
    },
    ParamGroup {
        label: "",
        params: &["colour_3", "position_3"],
        collapsed: false,
        visible_when: Some(("colours", &[2, 3])),
        visible_when_lens_elements: None,
    },
    ParamGroup {
        label: "",
        params: &["colour_4", "position_4"],
        collapsed: false,
        visible_when: Some(("colours", &[3])),
        visible_when_lens_elements: None,
    },
];

/// Gradient's controls.
///
/// The two points are px@comp (§2.3), declared `Px` so the resolve step converts
/// them to the raster in play and
/// [`ResolvedStack::rescale_spatial`](crate::fx::params::ResolvedStack::rescale_spatial)
/// moves them **together** if the stack is reused at another size — a
/// ramp that slid when the preview resolution changed would be a ramp nobody
/// could grade against.
#[derive(Debug, Clone, Copy, PartialEq, Effect)]
#[effect(
    match_name = "gradient",
    label = "Gradient",
    version = 1,
    category = Generate,
    cost = Cheap,
    roi = Exact,
    // The ramp replaces the frame outright, so nothing of the input's colour is
    // read and there is nothing to unpremultiply (§2.2).
    premultiplied = true,
    seeded = true,
    groups = GRADIENT_GROUPS,
)]
pub struct Gradient {
    /// Linear projects onto the axis; Radial measures distance from Start, with
    /// End sitting on the outer edge.
    #[choice(options = ["Linear", "Radial"], default = 0)]
    pub shape: u32,

    /// px@comp. The default pair runs top-to-bottom down a 1080p frame.
    #[slider(label = "Start x", min = 0.0, max = 3840.0, default = 960.0, unit = Px)]
    pub start_x: f32,

    /// px@comp; see [`start_x`](Self::start_x).
    #[slider(label = "Start y", min = 0.0, max = 2160.0, default = 0.0, unit = Px)]
    pub start_y: f32,

    /// The colour at Start. Scene-linear and open above 1 (§2.1).
    #[colour(label = "Start colour", default = [1.0, 1.0, 1.0, 1.0], max = 4.0)]
    pub start_colour: [f32; 4],

    /// px@comp; see [`start_x`](Self::start_x).
    #[slider(label = "End x", min = 0.0, max = 3840.0, default = 960.0, unit = Px)]
    pub end_x: f32,

    /// px@comp; see [`start_x`](Self::start_x).
    #[slider(label = "End y", min = 0.0, max = 2160.0, default = 1080.0, unit = Px)]
    pub end_y: f32,

    /// The colour at End.
    #[colour(label = "End colour", default = [0.0, 0.0, 0.0, 1.0], max = 4.0)]
    pub end_colour: [f32; 4],

    /// How many colours the ramp runs through: Start colour, the colours
    /// in between, End colour.
    #[choice(label = "Colours", options = ["Two", "Three", "Four", "Five"], default = 0)]
    pub colours: u32,

    /// The first colour after Start colour.
    #[colour(label = "Colour 2", default = [1.0, 0.5, 0.0, 1.0], max = 4.0)]
    pub colour_2: [f32; 4],

    /// Where Colour 2 sits along the ramp, per cent.
    #[slider(
        label = "Colour 2 position",
        min = 0.0,
        max = 100.0,
        default = 50.0,
        hard_min = 0.0,
        hard_max = 100.0,
        unit = Percent
    )]
    pub position_2: f32,

    /// The next colour, with Four or Five.
    #[colour(label = "Colour 3", default = [0.0, 0.5, 1.0, 1.0], max = 4.0)]
    pub colour_3: [f32; 4],

    /// Where Colour 3 sits, per cent.
    #[slider(
        label = "Colour 3 position",
        min = 0.0,
        max = 100.0,
        default = 75.0,
        hard_min = 0.0,
        hard_max = 100.0,
        unit = Percent
    )]
    pub position_3: f32,

    /// The last colour before End colour, with Five.
    #[colour(label = "Colour 4", default = [0.0, 1.0, 1.0, 1.0], max = 4.0)]
    pub colour_4: [f32; 4],

    /// Where Colour 4 sits, per cent.
    #[slider(
        label = "Colour 4 position",
        min = 0.0,
        max = 100.0,
        default = 90.0,
        hard_min = 0.0,
        hard_max = 100.0,
        unit = Percent
    )]
    pub position_4: f32,

    /// Per cent: how far a pixel's position along the ramp may be dithered. A
    /// few per cent breaks banding without softening the ramp's ends.
    #[slider(
        min = 0.0,
        max = 100.0,
        default = 0.0,
        hard_min = 0.0,
        hard_max = 100.0,
        unit = Percent
    )]
    pub scatter: f32,

    /// Which draw the scatter follows (§2.4).
    #[seed]
    pub seed: u32,

    /// The host-uniform Mix every effect ends with (docs/08 §1.5), per cent —
    /// where an imported AE Ramp's "Blend with original" lands.
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

impl Gradient {
    /// The bundle both kernels consume (docs/impl/effect-registry.md §2.4).
    ///
    /// Both reciprocals are taken here, once, and floored against the same
    /// epsilon: Start and End at the same place give a zero-length axis, and the
    /// floor collapses the ramp to one flat colour rather than dividing by zero
    /// (docs/14 §4). The two points
    /// arrive already scaled to the raster by the declared `Px` unit. Both render
    /// paths read this one method, so the CPU reference and the WGSL kernel
    /// cannot drift apart.
    #[must_use]
    pub fn packed(self) -> cpu::GradientParams {
        let axis = [self.end_x - self.start_x, self.end_y - self.start_y];
        let len2 = axis[0] * axis[0] + axis[1] * axis[1];
        // One epsilon, applied to the squared length, so the linear and radial
        // reciprocals degenerate at exactly the same point.
        let len2 = len2.max(1e-6);
        cpu::GradientParams {
            radial: self.shape == 1,
            start: [self.start_x, self.start_y],
            axis,
            inv_len2: 1.0 / len2,
            inv_len: 1.0 / len2.sqrt(),
            c0: [
                self.start_colour[0],
                self.start_colour[1],
                self.start_colour[2],
            ],
            c1: [self.end_colour[0], self.end_colour[1], self.end_colour[2]],
            scatter: (self.scatter / 100.0).clamp(0.0, 1.0),
            seed: self.seed,
            mix: (self.mix / 100.0).clamp(0.0, 1.0),
            // A generator floods the frame (§2.2's opaque write); clipping the
            // ramp to the layer's coverage is the Gradient overlay *style*'s,
            // and false here is the picture this effect always drew.
            clip_to_alpha: false,
            mids: self.colours.min(3),
            mid: self.mids(),
        }
    }

    /// The colours between Start colour and End colour, each with its place
    /// along the ramp (0..1) in the last lane. Kept in order: a colour placed
    /// before the one ahead of it sits where that one does.
    fn mids(self) -> [[f32; 4]; 3] {
        let mut at = 0.0f32;
        [
            (self.colour_2, self.position_2),
            (self.colour_3, self.position_3),
            (self.colour_4, self.position_4),
        ]
        .map(|(c, pos)| {
            at = (pos / 100.0).clamp(at, 1.0);
            [c[0], c[1], c[2], at]
        })
    }
}

/// Gradient's behaviour.
pub struct GradientDef;

impl EffectDef for GradientDef {
    fn schema(&self) -> &'static EffectSchema {
        &<Gradient as EffectMetadata>::SCHEMA
    }

    fn apply_cpu(&self, rgba: &mut [f32], w: u32, h: u32, p: Params<'_>) {
        cpu::gradient(rgba, w, h, &Gradient::read(p).packed());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plain() -> Gradient {
        Gradient::read(Params::EMPTY)
    }

    #[test]
    fn three_colours_pass_through_the_middle_one() {
        let g = Gradient {
            colours: 1,
            start_colour: [1.0; 4],
            colour_2: [1.0, 0.5, 0.0, 1.0],
            end_colour: [1.0; 4],
            ..plain()
        }
        .packed();
        assert_eq!(cpu::gradient_ramp(&g, 0.5), [1.0, 0.5, 0.0]);
        assert_eq!(cpu::gradient_ramp(&g, 0.0), [1.0, 1.0, 1.0]);
        assert_eq!(cpu::gradient_ramp(&g, 1.0), [1.0, 1.0, 1.0]);
        let quarter = cpu::gradient_ramp(&g, 0.25);
        assert!((quarter[1] - 0.75).abs() < 1e-6);
        // Two colours ignore the ones in between.
        let two = Gradient {
            colours: 0,
            ..plain()
        }
        .packed();
        assert_eq!(cpu::gradient_ramp(&two, 0.5), [0.5, 0.5, 0.5]);
    }
}

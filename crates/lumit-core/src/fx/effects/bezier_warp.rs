//! Bezier warp (docs/08 §3.55): the frame's four edges bent — AE's Bezier Warp,
//! and the answer to §3.48's "not in v1".
//!
//! **In plain terms.** Corner pin (§3.48) lets you drag the four corners of the
//! picture and keeps the edges between them straight. This one gives each edge
//! two handles as well, so the edges can bow, and fills the inside smoothly
//! between them. It is how a flat picture is wrapped onto a curved surface —
//! a bottle, a flag, a page turning.
//!
//! The four corners carry §3.48's own names and units, so swapping one effect
//! for the other costs no dragging.

use crate::fx::{cpu, EffectDef, EffectMetadata, EffectSchema, ParamGroup, Params};
use lumit_fx_macros::Effect;

/// The panel's four disclosures: the corners stay in the open (they are what
/// most projects touch), and each edge's pair of handles sits behind the edge's
/// own heading.
pub const BEZIER_WARP_GROUPS: &[ParamGroup] = &[
    ParamGroup {
        label: "Top edge",
        params: &[
            "top_left_tangent_x",
            "top_left_tangent_y",
            "top_right_tangent_x",
            "top_right_tangent_y",
        ],
        collapsed: true,
        visible_when: None,
        visible_when_lens_elements: None,
    },
    ParamGroup {
        label: "Right edge",
        params: &[
            "right_top_tangent_x",
            "right_top_tangent_y",
            "right_bottom_tangent_x",
            "right_bottom_tangent_y",
        ],
        collapsed: true,
        visible_when: None,
        visible_when_lens_elements: None,
    },
    ParamGroup {
        label: "Bottom edge",
        params: &[
            "bottom_left_tangent_x",
            "bottom_left_tangent_y",
            "bottom_right_tangent_x",
            "bottom_right_tangent_y",
        ],
        collapsed: true,
        visible_when: None,
        visible_when_lens_elements: None,
    },
    ParamGroup {
        label: "Left edge",
        params: &[
            "left_top_tangent_x",
            "left_top_tangent_y",
            "left_bottom_tangent_x",
            "left_bottom_tangent_y",
        ],
        collapsed: true,
        visible_when: None,
        visible_when_lens_elements: None,
    },
];

/// Bezier warp's controls: twelve points and a solver budget.
///
/// Every point is px@comp (§2.3), declared `Px`. The schema defaults are
/// a nominal 1080p frame with its handles at the thirds — the patch that is
/// exactly the identity — and
/// [`instantiate_for_raster`](crate::fx::instantiate_for_raster) puts a fresh
/// instance's twelve points on the actual comp.
#[derive(Debug, Clone, Copy, PartialEq, Effect)]
#[effect(
    match_name = "bezier_warp",
    label = "Bezier warp",
    version = 1,
    category = Distortion,
    // Up to twelve Newton steps, each a patch evaluation and a Jacobian.
    cost = Moderate,
    // Any output pixel can be drawn from anywhere in the frame.
    roi = FullFrame,
    premultiplied = true,
    groups = BEZIER_WARP_GROUPS,
    // The matte scales the displacement, inside the kernel (the owner's rule
    // for mattes); the generic strength dissolve does not also run.
    matte = (
        "matte",
        "scales the bend from the straight frame per pixel, read where the \
         pixel lands: white bends it all the way, grey part way, black leaves \
         it where it was",
    ),
)]
pub struct BezierWarp {
    /// px@comp: the picture's upper-left corner goes here. §3.48's name.
    #[slider(min = 0.0, max = 3840.0, default = 0.0, unit = Px)]
    pub upper_left_x: f32,
    /// px@comp; see [`upper_left_x`](Self::upper_left_x).
    #[slider(min = 0.0, max = 2160.0, default = 0.0, unit = Px)]
    pub upper_left_y: f32,
    /// px@comp: the upper-right corner.
    #[slider(min = 0.0, max = 3840.0, default = 1920.0, unit = Px)]
    pub upper_right_x: f32,
    /// px@comp; see [`upper_right_x`](Self::upper_right_x).
    #[slider(min = 0.0, max = 2160.0, default = 0.0, unit = Px)]
    pub upper_right_y: f32,
    /// px@comp: the lower-right corner.
    #[slider(min = 0.0, max = 3840.0, default = 1920.0, unit = Px)]
    pub lower_right_x: f32,
    /// px@comp; see [`lower_right_x`](Self::lower_right_x).
    #[slider(min = 0.0, max = 2160.0, default = 1080.0, unit = Px)]
    pub lower_right_y: f32,
    /// px@comp: the lower-left corner.
    #[slider(min = 0.0, max = 3840.0, default = 0.0, unit = Px)]
    pub lower_left_x: f32,
    /// px@comp; see [`lower_left_x`](Self::lower_left_x).
    #[slider(min = 0.0, max = 2160.0, default = 1080.0, unit = Px)]
    pub lower_left_y: f32,

    /// px@comp: the top edge's first handle, a third of the way along.
    #[slider(min = 0.0, max = 3840.0, default = 640.0, unit = Px)]
    pub top_left_tangent_x: f32,
    /// px@comp; see [`top_left_tangent_x`](Self::top_left_tangent_x).
    #[slider(min = 0.0, max = 2160.0, default = 0.0, unit = Px)]
    pub top_left_tangent_y: f32,
    /// px@comp: the top edge's second handle.
    #[slider(min = 0.0, max = 3840.0, default = 1280.0, unit = Px)]
    pub top_right_tangent_x: f32,
    /// px@comp; see [`top_right_tangent_x`](Self::top_right_tangent_x).
    #[slider(min = 0.0, max = 2160.0, default = 0.0, unit = Px)]
    pub top_right_tangent_y: f32,

    /// px@comp: the right edge's upper handle.
    #[slider(min = 0.0, max = 3840.0, default = 1920.0, unit = Px)]
    pub right_top_tangent_x: f32,
    /// px@comp; see [`right_top_tangent_x`](Self::right_top_tangent_x).
    #[slider(min = 0.0, max = 2160.0, default = 360.0, unit = Px)]
    pub right_top_tangent_y: f32,
    /// px@comp: the right edge's lower handle.
    #[slider(min = 0.0, max = 3840.0, default = 1920.0, unit = Px)]
    pub right_bottom_tangent_x: f32,
    /// px@comp; see [`right_bottom_tangent_x`](Self::right_bottom_tangent_x).
    #[slider(min = 0.0, max = 2160.0, default = 720.0, unit = Px)]
    pub right_bottom_tangent_y: f32,

    /// px@comp: the bottom edge's left handle.
    #[slider(min = 0.0, max = 3840.0, default = 640.0, unit = Px)]
    pub bottom_left_tangent_x: f32,
    /// px@comp; see [`bottom_left_tangent_x`](Self::bottom_left_tangent_x).
    #[slider(min = 0.0, max = 2160.0, default = 1080.0, unit = Px)]
    pub bottom_left_tangent_y: f32,
    /// px@comp: the bottom edge's right handle.
    #[slider(min = 0.0, max = 3840.0, default = 1280.0, unit = Px)]
    pub bottom_right_tangent_x: f32,
    /// px@comp; see [`bottom_right_tangent_x`](Self::bottom_right_tangent_x).
    #[slider(min = 0.0, max = 2160.0, default = 1080.0, unit = Px)]
    pub bottom_right_tangent_y: f32,

    /// px@comp: the left edge's upper handle.
    #[slider(min = 0.0, max = 3840.0, default = 0.0, unit = Px)]
    pub left_top_tangent_x: f32,
    /// px@comp; see [`left_top_tangent_x`](Self::left_top_tangent_x).
    #[slider(min = 0.0, max = 2160.0, default = 360.0, unit = Px)]
    pub left_top_tangent_y: f32,
    /// px@comp: the left edge's lower handle.
    #[slider(min = 0.0, max = 3840.0, default = 0.0, unit = Px)]
    pub left_bottom_tangent_x: f32,
    /// px@comp; see [`left_bottom_tangent_x`](Self::left_bottom_tangent_x).
    #[slider(min = 0.0, max = 2160.0, default = 720.0, unit = Px)]
    pub left_bottom_tangent_y: f32,

    /// How many Newton steps each pixel takes to invert the patch (§3.55
    /// decision 2 — AE's Quality buys smaller triangles; there are no triangles
    /// here, so it buys convergence). Eight is well past where an ordinary warp
    /// stops moving.
    #[counter(min = 1, max = 12, default = 8, hard_min = 1, hard_max = 12, unit = Raw)]
    pub quality: i32,

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

impl BezierWarp {
    /// The bundle both kernels consume (docs/impl/effect-registry.md §2.4): the
    /// twelve points in **AE's clockwise walk** — corner, two handles, corner,
    /// two handles, … starting at the upper left — which is the order the four
    /// boundary curves are read out of, and the order the import writes.
    #[must_use]
    pub fn packed(self) -> cpu::BezierWarpParams {
        cpu::BezierWarpParams {
            pts: [
                [self.upper_left_x, self.upper_left_y],
                [self.top_left_tangent_x, self.top_left_tangent_y],
                [self.top_right_tangent_x, self.top_right_tangent_y],
                [self.upper_right_x, self.upper_right_y],
                [self.right_top_tangent_x, self.right_top_tangent_y],
                [self.right_bottom_tangent_x, self.right_bottom_tangent_y],
                [self.lower_right_x, self.lower_right_y],
                [self.bottom_right_tangent_x, self.bottom_right_tangent_y],
                [self.bottom_left_tangent_x, self.bottom_left_tangent_y],
                [self.lower_left_x, self.lower_left_y],
                [self.left_bottom_tangent_x, self.left_bottom_tangent_y],
                [self.left_top_tangent_x, self.left_top_tangent_y],
            ],
            steps: self.quality.clamp(1, 12) as u32,
            mix: (self.mix / 100.0).clamp(0.0, 1.0),
        }
    }
}

/// Bezier warp's behaviour.
pub struct BezierWarpDef;

impl EffectDef for BezierWarpDef {
    fn schema(&self) -> &'static EffectSchema {
        &<BezierWarp as EffectMetadata>::SCHEMA
    }

    fn apply_cpu(&self, rgba: &mut [f32], w: u32, h: u32, p: Params<'_>) {
        cpu::bezier_warp(rgba, w, h, &BezierWarp::read(p).packed());
    }
}

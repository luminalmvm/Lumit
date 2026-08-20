//! Vegas (docs/08 §3.76): marching lights along the picture's contours — AE's
//! Vegas, its Image Contours half.
//!
//! **In plain terms.** The effect finds a contour in the picture — the line
//! where its brightness (or its alpha) crosses a level you choose — and runs a
//! dashed stroke along it. Turn Rotation and the dashes march, which is the
//! chasing-lights look the effect is named for.
//!
//! Two things are worth knowing about how it works, because they are what make
//! the controls mean what they say. The contour is a **level set**, not an edge
//! detector's output, so Width is a width in pixels rather than "however steep
//! the picture happens to be here". And the dashes are laid out in screen space
//! along the contour's own direction, because an effect that never traces a path
//! has no arc length to count segments around — §3.76's second decision, and
//! where AE's Segments becomes Lumit's Segment length.
//!
//! **AE's Mask/Path source is carried too**, since K-408: pick Mask/Path and the
//! dashes march round a mask you have drawn instead of round a contour the
//! effect found. That half is a different kernel — the shared path drawing
//! §3.78 and §3.79 also use — and on it Segment length means what AE's Segments
//! means, because there is a real arc length to count round.

use crate::fx::{cpu, EffectDef, EffectMetadata, EffectSchema, ParamId, Params, ResolveCx, Value};
use crate::mask::MaskPolyline;
use lumit_fx_macros::Effect;

/// Vegas' controls.
#[derive(Debug, Clone, Copy, PartialEq, Effect)]
#[effect(
    match_name = "vegas",
    label = "Vegas",
    version = 1,
    category = Generate,
    cost = Cheap,
    // A 3×3 Sobel.
    roi = PaddedPctDiag(1.0),
    premultiplied = true,
    enabled_when = VEGAS_ENABLED_WHEN,
)]
pub struct Vegas {
    /// Where the line comes from. Luminance is AE's Image Contours; Alpha
    /// outlines the layer's own shape, which is what a logo wants; **Mask/Path**
    /// is AE's other half — a mask you have drawn (K-408).
    #[choice(options = ["Luminance", "Alpha", "Mask/Path"], default = 0)]
    pub source: u32,

    /// Which of the layer's masks to march round, while Source is Mask/Path.
    /// Unset is **First mask**.
    #[mask_path(label = "Mask")]
    pub path: bool,

    /// Where the contour is, per cent of the range. 50 is the middle of the tone
    /// range on the perceptual curve (§3.58 decision 1), and the edge of a matte
    /// under Alpha.
    #[slider(
        min = 0.0,
        max = 100.0,
        default = 50.0,
        hard_min = 0.0,
        hard_max = 100.0
    )]
    pub threshold: f32,

    /// How thick the stroke is, px@comp (§2.3).
    #[slider(min = 0.0, max = 50.0, default = 3.0, hard_min = 0.0, unit = Px)]
    pub width: f32,

    /// How crisp its edges are, per cent. 100 is a hard stroke, 0 a soft one
    /// that fades over its own width.
    #[slider(
        min = 0.0,
        max = 100.0,
        default = 50.0,
        hard_min = 0.0,
        hard_max = 100.0
    )]
    pub hardness: f32,

    /// One dash and its gap, px@comp — AE's Segments turned into a length, for
    /// §3.76's second reason.
    #[slider(
        label = "Segment length",
        min = 1.0,
        max = 1000.0,
        default = 80.0,
        hard_min = 1.0,
        unit = Px
    )]
    pub segment_length: f32,

    /// How much of one segment is lit, per cent — AE's Length, unchanged. 100
    /// draws a continuous outline.
    #[slider(
        min = 0.0,
        max = 100.0,
        default = 55.0,
        hard_min = 0.0,
        hard_max = 100.0
    )]
    pub length: f32,

    /// Marches the dashes along the contour: one full turn moves them on by
    /// exactly one segment, so a linear keyframe is the marching-lights
    /// animation.
    #[dial(default = 0.0, step = 15.0)]
    pub rotation: f32,

    /// The stroke's colour. Scene-linear and open above 1 (§2.1).
    #[colour(default = [1.0, 0.80, 0.25, 1.0], max = 4.0)]
    pub colour: [f32; 4],

    /// How strong the stroke is, per cent.
    #[slider(
        min = 0.0,
        max = 100.0,
        default = 100.0,
        hard_min = 0.0,
        hard_max = 100.0
    )]
    pub opacity: f32,

    /// On, the layer that arrived stays under the stroke; off, the stroke is all
    /// there is — which is how an outline becomes its own element.
    #[toggle(label = "Composite on original", default = true)]
    pub composite_on_original: bool,

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

/// The two rows that answer to Source: a mask is not a control while the line
/// is being found in the picture, and a level is not a control while it is not.
pub const VEGAS_ENABLED_WHEN: &[crate::fx::EnabledWhen] = &[
    crate::fx::EnabledWhen {
        param: "path",
        on: "source",
        cond: crate::fx::EnabledCond::ChoiceIs(Vegas::SOURCE_MASK_PATH),
    },
    crate::fx::EnabledWhen {
        param: "threshold",
        on: "source",
        cond: crate::fx::EnabledCond::ChoiceIsNot(Vegas::SOURCE_MASK_PATH),
    },
];

impl Vegas {
    /// The [`source`](Self::source) option that reads a mask instead of the
    /// picture (K-408). Named because three places test for it and a bare 2
    /// in any of them would be the one that went stale.
    pub const SOURCE_MASK_PATH: u32 = 2;

    /// Raster pixels per comp pixel (§2.3), pushed at resolve because the seam
    /// hands its vertices over in px@comp (K-408). Never a panel row.
    pub const DERIVED_PX_SCALE: ParamId = ParamId::new("derived.px_scale");

    /// This instance's raster factor, read back out of a resolved bag.
    #[must_use]
    pub fn px_scale_of(p: Params<'_>) -> f32 {
        p.float(Self::DERIVED_PX_SCALE, 1.0)
    }

    /// Whether this instance marches round a mask rather than round a contour.
    #[must_use]
    pub fn on_a_path(self) -> bool {
        self.source == Self::SOURCE_MASK_PATH
    }

    /// The Mask/Path half's bundle: the same stroke, laid on a mask's own line
    /// (K-408). Shares every control with the contour half — Width, Hardness,
    /// Segment length, Length, Rotation, Colour, Opacity — because it is the
    /// same stroke, and drops only Threshold, which has no meaning without a
    /// picture to take a level of.
    ///
    /// **Segment length is AE's Segments here.** The dashes are spaced by
    /// measured distance *round the path*, so they stay evenly spaced however
    /// hard it curves — the price §3.76's third decision pays on a contour, and
    /// which is not paid on a path because there is an arc length to count.
    #[must_use]
    pub fn path_packed(self, poly: &MaskPolyline, px_scale: f32) -> cpu::PathDrawParams {
        let v = self.packed();
        let mut p = cpu::PathDrawParams::blank();
        cpu::path_chain(
            &cpu::path_points(poly, px_scale.max(1e-6)),
            0.0,
            100.0,
            &mut p,
        );
        p.half_width = v.half_width;
        p.band = v.band;
        p.inv_segment = v.inv_segment;
        p.duty = v.duty;
        p.phase = v.phase;
        p.colour = v.colour;
        p.opacity = v.opacity;
        p.style = if v.composite {
            cpu::PAINT_ON_ORIGINAL
        } else {
            cpu::PAINT_ON_TRANSPARENT
        };
        p.mix = v.mix;
        p
    }

    /// The bundle both kernels consume (docs/impl/effect-registry.md §2.4). The
    /// one reciprocal and the soft band are taken here and floored, so neither
    /// path divides by a zero-width stroke or a zero-length segment (docs/14 §4).
    #[must_use]
    pub fn packed(self) -> cpu::VegasParams {
        let half = self.width.max(0.0) * 0.5;
        cpu::VegasParams {
            from_alpha: self.source == 1,
            level: (self.threshold / 100.0).clamp(0.0, 1.0),
            half_width: half,
            band: ((1.0 - (self.hardness / 100.0).clamp(0.0, 1.0)) * half).max(0.5),
            inv_segment: 1.0 / self.segment_length.max(1.0),
            // Length 100 is a *continuous* outline, so the duty is pushed past
            // the fraction's own range rather than sitting on its edge — at
            // exactly 1 the wrap point would scallop the stroke once a segment.
            duty: if self.length >= 100.0 {
                2.0
            } else {
                (self.length / 100.0).clamp(0.0, 1.0)
            },
            // Degrees to turns: one full turn is one segment (§3.76 decision 3).
            phase: self.rotation / 360.0,
            colour: [self.colour[0], self.colour[1], self.colour[2]],
            opacity: (self.opacity / 100.0).clamp(0.0, 1.0),
            composite: self.composite_on_original,
            mix: (self.mix / 100.0).clamp(0.0, 1.0),
        }
    }
}

/// Vegas' behaviour.
pub struct VegasDef;

impl EffectDef for VegasDef {
    fn schema(&self) -> &'static EffectSchema {
        &<Vegas as EffectMetadata>::SCHEMA
    }

    fn resolve_derived(&self, cx: &ResolveCx<'_>, push: &mut dyn FnMut(ParamId, Value)) {
        push(Vegas::DERIVED_PX_SCALE, Value::Float(cx.px_scale));
    }

    /// The contour half only. On Mask/Path the geometry arrives beside the op
    /// rather than in the bag, so there is nothing here to draw and the identity
    /// stands — the same shape Set matte and the LUT have (docs/08 §1.6), and
    /// the same picture an unset mask row renders anyway. That half's oracle is
    /// [`cpu::path_draw`], exercised directly from the lumit-gpu test.
    fn apply_cpu(&self, rgba: &mut [f32], w: u32, h: u32, p: Params<'_>) {
        let v = Vegas::read(p);
        if v.on_a_path() {
            return;
        }
        cpu::vegas(rgba, w, h, &v.packed());
    }
}

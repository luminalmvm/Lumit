//! Find edges (docs/08 §3.66): the picture as a pencil drawing — AE's Find
//! Edges.
//!
//! **In plain terms.** Everywhere the picture changes quickly — an outline, a
//! rim, a hard shadow — a line is drawn; everywhere it is flat, nothing is. The
//! default is dark lines on white, which reads as a pencil drawing; **Invert**
//! turns it into glowing lines on black, which is where the neon look starts.
//!
//! The one thing worth knowing is *where* the lines are found. Lumit works in
//! scene-linear light (§2.1), and in light the step from 3.0 to 4.0 in a sunlit
//! sky is a stronger change than the step from 0.01 to 0.05 in a shadow — though
//! the eye sees the second and not the first. The differences are therefore
//! taken in a square root of the light, the same curve §3.58's rungs are spaced
//! on, which puts the lines where a person would draw them.

use crate::fx::{cpu, EffectDef, EffectMetadata, EffectSchema, Params};
use lumit_fx_macros::Effect;

/// Find edges' controls.
#[derive(Debug, Clone, Copy, PartialEq, Effect)]
#[effect(
    match_name = "find_edges",
    label = "Find edges",
    version = 1,
    category = Stylise,
    // Eight taps a pixel.
    cost = Cheap,
    // One pixel of reach (a 3x3 kernel); the padding never resolves below one
    // raster pixel, so Quarter preview still gets its neighbour.
    roi = PaddedPx(1.0),
    // §2.2: a gradient of premultiplied colour is a gradient of the coverage
    // wherever the coverage moves, which puts a line round every soft edge.
    premultiplied = false,
)]
pub struct FindEdges {
    /// Off (AE's default) the frame is white with dark lines on it; on, black
    /// with bright ones.
    ///
    /// **"Invert edges", not AE's bare "Invert"**: every effect now carries the
    /// K-395 Matte row, whose own switch is called Invert, and two rows of the
    /// same name in one panel is a control nobody can point at. The label is
    /// distinct, the id and the import mapping are unchanged.
    #[toggle(label = "Invert edges", default = false)]
    pub invert: bool,

    /// The host-uniform Mix every effect ends with (docs/08 §1.5), per cent.
    /// This is AE's "Blend With Original".
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

impl FindEdges {
    /// The invert flag as the number both kernels multiply by, and the mix
    /// (docs/impl/effect-registry.md §2.4).
    #[must_use]
    pub fn packed(self) -> (f32, f32) {
        (
            f32::from(u8::from(self.invert)),
            (self.mix / 100.0).clamp(0.0, 1.0),
        )
    }
}

/// Find edges' behaviour.
pub struct FindEdgesDef;

impl EffectDef for FindEdgesDef {
    fn schema(&self) -> &'static EffectSchema {
        &<FindEdges as EffectMetadata>::SCHEMA
    }

    fn apply_cpu(&self, rgba: &mut [f32], w: u32, h: u32, p: Params<'_>) {
        let (invert, mix) = FindEdges::read(p).packed();
        cpu::find_edges(rgba, w, h, invert, mix);
    }
}

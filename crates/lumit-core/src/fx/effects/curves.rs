//! Curves (docs/08 §3.30, K-412): the per-channel tone curve, as a real
//! curve — an ordered list of control points a channel, the shape an editor
//! edits.
//!
//! **In plain terms.** Five curves: one for the picture as a whole (Master),
//! one each for red, green and blue, and one for alpha. Each is a handful of
//! points in a unit square with a smooth line drawn through them; the line
//! says what an input brightness comes out as.
//!
//! **The line is drawn here, not in the kernel** (K-412, Lightning's
//! discipline in §3.74). [`Curves::packed`] fits the spline once and writes it
//! down as a 257-entry table a channel; both render paths are handed the
//! identical tables and do nothing but look up and interpolate, which is what
//! leaves the §1.6 oracle checking the *lookup* rather than two spline fits
//! agreeing by luck.

use crate::fx::{cpu, CurvePoints, EffectDef, EffectMetadata, EffectSchema, Params};
use lumit_fx_macros::Effect;

/// Curves' controls: one curve on each of Master, Red, Green, Blue and
/// Alpha — After Effects' own five (K-412).
///
/// Each curve is an ordered list of 2..=16 points in the unit square, the
/// identity diagonal by default, so a fresh Curves is the bit-exact
/// passthrough: the grade family's sanctioned exception to the "no no-op
/// default" rule (docs/08 §3.10).
///
/// The rows carry only their channel's name because the panel draws them as
/// channel tabs over one editor (K-412), not as five stacked widgets.
#[derive(Debug, Clone, Copy, PartialEq, Effect)]
#[effect(
    match_name = "curves",
    label = "Curves",
    // K-412 replaced K-396's twenty fixed knots outright rather than
    // migrating them: the effect is days old and unreleased, and a version
    // bump is what keeps a cached frame from the knot generation out of the
    // curve generation's picture (docs/08 §1.1).
    version = 2,
    category = Colour,
    cost = Cheap,
    roi = Exact,
    // §2.2: a tone curve is non-linear, so it does not commute with
    // premultiplied alpha — grading premult would shift matte edges.
    premultiplied = false,
)]
pub struct Curves {
    /// The whole picture's curve, applied after the per-channel ones. It does
    /// not touch alpha, which has its own row — After Effects' arrangement.
    #[curve(label = "Master")]
    pub master: CurvePoints,
    /// Red's own curve, applied before Master.
    #[curve(label = "Red")]
    pub red: CurvePoints,
    /// Green's own curve, applied before Master.
    #[curve(label = "Green")]
    pub green: CurvePoints,
    /// Blue's own curve, applied before Master.
    #[curve(label = "Blue")]
    pub blue: CurvePoints,
    /// Coverage's own curve. The graded colour is re-premultiplied by the
    /// graded alpha, so bending this bends the matte and the picture together.
    #[curve(label = "Alpha")]
    pub alpha: CurvePoints,

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

impl Curves {
    /// The five baked tables and the mix — the bundle both kernels consume
    /// (docs/impl/effect-registry.md §2.4).
    ///
    /// The spline fit is host maths on purpose: both render paths take the
    /// table this produced, so neither fits a curve per pixel and the two
    /// cannot disagree about the shape.
    #[must_use]
    pub fn packed(self) -> cpu::CurveTables {
        let channels = [self.master, self.red, self.green, self.blue, self.alpha];
        let mut t = [[0.0f32; cpu::CURVE_TABLE]; 5];
        for (slot, points) in t.iter_mut().zip(channels) {
            *slot = cpu::curve_table(&points);
        }
        cpu::CurveTables {
            t,
            // Neutrality is decided on the *points*, not on the baked
            // numbers: it is the same comparison the fixed-knot version made,
            // it costs five equalities instead of 1285, and a curve somebody
            // dragged into a straight line by hand is a curve, not a
            // short-circuit.
            neutral: channels.iter().all(|c| *c == CurvePoints::IDENTITY),
            mix: (self.mix / 100.0).clamp(0.0, 1.0),
        }
    }
}

/// Curves' behaviour.
pub struct CurvesDef;

impl EffectDef for CurvesDef {
    fn schema(&self) -> &'static EffectSchema {
        &<Curves as EffectMetadata>::SCHEMA
    }

    fn apply_cpu(&self, rgba: &mut [f32], _w: u32, _h: u32, p: Params<'_>) {
        cpu::curves(rgba, &Curves::read(p).packed());
    }
}

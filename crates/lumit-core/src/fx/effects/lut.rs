//! LUT (docs/08 §3.11, docs/impl/lut.md, K-114): a 3D colour look-up from a
//! `.cube` file — a colourist's baked grade dropped onto a layer.
//!
//! **In plain terms.** The grade itself is not a number: it is a file, parsed
//! and uploaded as a 3D texture by whoever is rendering. So the declaration
//! carries the File row (for the panel, and so the render knows to go looking),
//! but the cube travels *beside* the resolved op in the render's parallel LUT
//! list — the k-th `lut` effect in the stack binds the k-th slot (K-387). What
//! the effect itself resolves to is the Mix alone.
//!
//! There is no CPU reference. The parsed cube never reaches the single-buffer
//! CPU dispatcher, so the degradation rung renders a LUT as identity, exactly as
//! the old `Resolved::Lut` arm did; the §1.6 oracle is [`crate::lut::Lut3d::
//! sample`], exercised directly from the lumit-gpu test.

use crate::fx::{EffectDef, EffectMetadata, EffectSchema};
use lumit_fx_macros::Effect;

/// LUT's controls.
#[derive(Debug, Clone, Copy, PartialEq, Effect)]
#[effect(
    match_name = "lut",
    label = "LUT",
    version = 1,
    category = Colour,
    // A per-pixel 3D lookup.
    cost = Moderate,
    roi = Exact,
    // §2.2: an arbitrary colour map must see straight colour.
    premultiplied = false,
)]
pub struct Lut {
    /// The `.cube` file (K-111); animatable only by stepping between paths with
    /// hold keys, since two files cannot be blended.
    ///
    /// **Always `None` here, by design.** A file slot is decided by the caller —
    /// only the render knows which cube actually loaded — so `resolve_into_arena`
    /// carries no `Value::File`, and the cube arrives at the GPU pass as its aux
    /// slot instead (K-387). The row exists because the panel needs it.
    #[file(filter = ["cube"], filter_name = "Cube LUT")]
    pub file: Option<u32>,

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

impl Lut {
    /// The mix the kernel blends the graded result by, clamped exactly as the
    /// old resolve arm clamped it — the whole of LUT's host maths, since the
    /// grade itself is the file.
    pub fn packed(self) -> f32 {
        (self.mix / 100.0).clamp(0.0, 1.0)
    }
}

/// LUT's behaviour: no CPU reference (the cube is a texture), so `apply_cpu`
/// keeps its identity default — the passthrough the old `Resolved::Lut` arm was.
pub struct LutDef;

impl EffectDef for LutDef {
    fn schema(&self) -> &'static EffectSchema {
        &<Lut as EffectMetadata>::SCHEMA
    }
}

//! Fill (docs/08 §3.34): flood the layer's own coverage with one colour.
//!
//! **In plain terms.** The layer already says *where* it is — its alpha carries
//! the shape, the antialiased edge, the feather. This effect keeps all of that
//! and replaces only the colour, which is why a shape, a title or a keyed matte
//! becomes a flat block of colour with its edges intact rather than a hard
//! stencil.

use crate::fx::{cpu, EffectDef, EffectMetadata, EffectSchema, Params};
use lumit_fx_macros::Effect;

/// Fill's controls.
#[derive(Debug, Clone, Copy, PartialEq, Effect)]
#[effect(
    match_name = "fill",
    label = "Fill",
    version = 1,
    category = Generate,
    cost = Trivial,
    roi = Exact,
    // §2.2: `colour · a` IS the premultiplied form of "this colour at this
    // coverage", and the source colour is never read — so there is nothing to
    // unpremultiply and no round trip to lose precision in.
    premultiplied = true,
)]
pub struct Fill {
    /// The colour the coverage is flooded with. Open above 1 so an HDR fill is
    /// typable (§2.1); the alpha lane is ignored, as it is on every colour
    /// parameter in the catalogue — a colour says what light, the layer says how
    /// much of it.
    #[colour(default = [1.0, 1.0, 1.0, 1.0], max = 4.0)]
    pub colour: [f32; 4],

    /// The host-uniform Mix every effect ends with (docs/08 §1.5), per cent.
    /// This is also where an imported AE Fill's Opacity lands: the two are the
    /// same number, and §3.34 deliberately does not carry both.
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

impl Fill {
    /// The colour's three channels and the mix (docs/impl/effect-registry.md
    /// §2.4). Both render paths read this one method, so the CPU reference and
    /// the WGSL kernel cannot drift apart.
    #[must_use]
    pub fn packed(self) -> ([f32; 3], f32) {
        (
            [self.colour[0], self.colour[1], self.colour[2]],
            (self.mix / 100.0).clamp(0.0, 1.0),
        )
    }
}

/// Fill's behaviour.
pub struct FillDef;

impl EffectDef for FillDef {
    fn schema(&self) -> &'static EffectSchema {
        &<Fill as EffectMetadata>::SCHEMA
    }

    fn apply_cpu(&self, rgba: &mut [f32], _w: u32, _h: u32, p: Params<'_>) {
        let (colour, mix) = Fill::read(p).packed();
        cpu::fill(rgba, colour, mix);
    }
}

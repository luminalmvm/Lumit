//! Texturize (docs/08 §3.68): another layer pressed into this one as relief —
//! AE's Texturize.
//!
//! **In plain terms.** Pick a layer — a canvas, a sheet of paper, a wall, a
//! crumpled foil — and this layer comes back looking as though it were printed
//! on it. The chosen layer is embossed exactly as §3.67 embosses a picture, and
//! the light and shade that come out are multiplied into this layer's colour.
//!
//! **It takes a layer of its own, not the Matte row.** §3.49's displacement map
//! *is* its matte, because a map has nothing else it could be; a texture is not,
//! because an editor will want to press a canvas into a layer **and** limit the
//! pressing to a region, and one row cannot say both. So the Texture row is this
//! effect's own — Light wrap's Background (§3.28) is the precedent — and the
//! generic §2.6 strength matte stays, which is what "only over the sky" means
//! here.
//!
//! There is no CPU reference through the single-buffer dispatcher, which carries
//! no second picture, so `apply_cpu` keeps its identity default — the labelled
//! no-op an unset row renders anyway. The §1.6 oracle is
//! [`crate::fx::cpu::texturize`], exercised directly from the lumit-gpu test,
//! which can upload a texture.

use crate::fx::{cpu, EffectDef, EffectMetadata, EffectSchema, EnabledCond, EnabledWhen};
use lumit_fx_macros::Effect;

/// A texture nobody has chosen has nothing to place, light or press.
pub const TEXTURIZE_ENABLED_WHEN: &[EnabledWhen] = &[
    EnabledWhen {
        param: "light_direction",
        on: "texture",
        cond: EnabledCond::LayerSet,
    },
    EnabledWhen {
        param: "relief",
        on: "texture",
        cond: EnabledCond::LayerSet,
    },
    EnabledWhen {
        param: "texture_contrast",
        on: "texture",
        cond: EnabledCond::LayerSet,
    },
    EnabledWhen {
        param: "placement",
        on: "texture",
        cond: EnabledCond::LayerSet,
    },
    EnabledWhen {
        param: "scale",
        on: "texture",
        cond: EnabledCond::LayerSet,
    },
];

/// Texturize's controls.
#[derive(Debug, Clone, Copy, PartialEq, Effect)]
#[effect(
    match_name = "texturize",
    label = "Texturize",
    version = 1,
    category = Stylise,
    // Two bilinear taps a pixel.
    cost = Cheap,
    // Relief's own reach, as §3.67's is. Its hard maximum is open, so the
    // padding is the slider's 20 px@comp doubled.
    roi = PaddedPx(40.0),
    // The relief is a multiply, and multiplying premultiplied colour by a scalar
    // is the same operation as multiplying straight colour by it — so no round
    // trip, and the shape is untouched.
    premultiplied = true,
    enabled_when = TEXTURIZE_ENABLED_WHEN,
    // K-428: the matte scales the amount, inside the kernel (the owner's rule
    // for mattes); the generic strength dissolve does not also run.
    matte = (
        "matte",
        "scales Relief per pixel: white lights the full relief, grey a shallower          one, black leaves the surface flat",
    ),
)]
pub struct Texturize {
    /// The layer pressed into this one. Unset until the owner picks one — the
    /// labelled no-op every layer row renders (docs/impl/layer-input.md). No
    /// `self_default` (K-288): a layer textured with itself is an edge-detect
    /// with extra steps, not a default anybody wants.
    #[layer(self_default = false)]
    pub texture: bool,

    /// Degrees, from straight up and clockwise — §3.67's convention, which is
    /// AE's. The slope of the texture facing this direction is the lit one.
    #[dial(label = "Light direction", default = 45.0, step = 15.0)]
    pub light_direction: f32,

    /// px@comp: how far apart the texture's two taps are, which is how thick the
    /// relief reads. **AE has no control here** — its relief is one pixel of
    /// whatever raster it was handed, which §2.3 forbids — so the default of 1
    /// is AE's behaviour at full resolution and the control is Lumit's own.
    #[slider(min = 0.0, max = 20.0, default = 1.0, hard_min = 0.0, unit = Px)]
    pub relief: f32,

    /// Per cent: the gain on the relief before it multiplies the picture. 0
    /// leaves the picture alone.
    #[slider(
        label = "Texture contrast",
        min = 0.0,
        max = 200.0,
        default = 100.0,
        hard_min = 0.0
    )]
    pub texture_contrast: f32,

    /// What happens **outside** one copy of the texture: Stretch holds its edge
    /// outward, Tile repeats it, Centre leaves the rest of the frame untextured.
    /// At Scale 100 all three coincide, and that one case is AE's "Stretch
    /// Texture to Fit" exactly (§3.68 decision 2).
    #[choice(options = ["Stretch", "Tile", "Centre"], default = 0)]
    pub placement: u32,

    /// Per cent: how big one copy of the texture is, as a fraction of the frame.
    /// Lumit's own control — the layer carriage renders a referenced layer at
    /// this raster, so 100 is the fitting AE calls Stretch.
    #[slider(min = 10.0, max = 400.0, default = 100.0, hard_min = 1.0)]
    pub scale: f32,

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

impl Texturize {
    /// The bundle both kernels consume (docs/impl/effect-registry.md §2.4). The
    /// trigonometry, the per-cent divisions and the reciprocal of the scale all
    /// happen here, once, so the CPU reference and the WGSL kernel sample the
    /// texture at the identical coordinates.
    #[must_use]
    pub fn packed(self) -> cpu::TexturizeParams {
        let t = self.light_direction.to_radians();
        let r = self.relief.max(0.0);
        cpu::TexturizeParams {
            // Straight up is −y in the raster, and clockwise from there is +x.
            offset: [r * t.sin(), -r * t.cos()],
            contrast: (self.texture_contrast / 100.0).max(0.0),
            inv_scale: 100.0 / self.scale.max(1.0),
            placement: self.placement.min(2),
            mix: (self.mix / 100.0).clamp(0.0, 1.0),
        }
    }
}

/// Texturize's behaviour: no CPU reference through the single-image dispatcher
/// (the texture is a second picture), so `apply_cpu` keeps its identity default
/// — which is also what an unbound Texture row renders.
pub struct TexturizeDef;

impl EffectDef for TexturizeDef {
    fn schema(&self) -> &'static EffectSchema {
        &<Texturize as EffectMetadata>::SCHEMA
    }
}

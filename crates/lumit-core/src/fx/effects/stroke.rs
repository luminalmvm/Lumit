//! Stroke (docs/08 §3.79): a brush walked round a mask path — AE's Stroke.
//!
//! **In plain terms.** You draw a mask; this runs a round brush along the line
//! of it, from one per cent of the way round to another. Keyframe End and the
//! line draws itself on, which is what the effect is mostly used for.
//!
//! It reads the mask's **shape**, not its coverage (K-408, docs/08 §1.2): the
//! hole a mask cuts cannot say which way is *along* it, and a brush that starts
//! at 20 % and stops at 80 % has to know.
//!
//! One thing about how it works. A brush stroke is a row of round stamps, and
//! while the stamps overlap their union is exactly the shape the brush sweeps —
//! so a stroke whose Spacing is under half its width is drawn as the swept path
//! itself, and only a stroke whose stamps have come apart is drawn as separate
//! dots (§3.79's second decision). Same picture, a fraction of the pieces, and
//! it is the only form that fits a long path with a fine brush inside the
//! geometry budget.

use crate::fx::{cpu, EffectDef, EffectMetadata, EffectSchema, ParamId, Params, ResolveCx, Value};
use crate::mask::MaskPolyline;
use lumit_fx_macros::Effect;

/// Stroke's controls.
#[derive(Debug, Clone, Copy, PartialEq, Effect)]
#[effect(
    match_name = "stroke",
    label = "Stroke",
    version = 1,
    category = Generate,
    // The kernel reads its own pixel and nothing else: the brush arrives as
    // geometry, not as a neighbourhood of the picture.
    roi = Exact,
    cost = Moderate,
    premultiplied = true,
    // K-428: the matte scales the amount, inside the kernel (the owner's rule
    // for mattes); the generic strength dissolve does not also run.
    matte = (
        "matte",
        "scales Opacity per pixel: white lays the brush down in full, grey          faintly, black not at all",
    ),
)]
pub struct Stroke {
    /// Which of the layer's masks to walk (K-408). Unset is **First mask**,
    /// because the effect is usually added before the mask is drawn.
    #[mask_path(label = "Mask")]
    pub path: bool,

    /// The brush's colour. Scene-linear and open above 1 (§2.1).
    #[colour(default = [1.0, 1.0, 1.0, 1.0], max = 4.0)]
    pub colour: [f32; 4],

    /// How wide the brush is, px@comp (§2.3) — a **width**, as Vegas' is, not
    /// a radius.
    #[slider(
        label = "Brush size",
        min = 0.5,
        max = 100.0,
        default = 8.0,
        hard_min = 0.1,
        unit = Px
    )]
    pub brush_size: f32,

    /// How crisp the brush's edge is, per cent. 100 is a hard-edged marker, 0 an
    /// airbrush that fades over its own width.
    #[slider(
        label = "Brush hardness",
        min = 0.0,
        max = 100.0,
        default = 75.0,
        hard_min = 0.0,
        hard_max = 100.0
    )]
    pub hardness: f32,

    /// How far apart the stamps are, per cent of the brush's width. Under 50 the
    /// stroke is continuous; well over it, a dotted line.
    #[slider(min = 1.0, max = 500.0, default = 15.0, hard_min = 1.0)]
    pub spacing: f32,

    /// Where the brush starts, per cent of the way round the path.
    #[slider(
        min = 0.0,
        max = 100.0,
        default = 0.0,
        hard_min = 0.0,
        hard_max = 100.0
    )]
    pub start: f32,

    /// Where it stops; see [`start`](Self::start). Keyframe it from 0 and the
    /// line draws itself on.
    #[slider(
        min = 0.0,
        max = 100.0,
        default = 100.0,
        hard_min = 0.0,
        hard_max = 100.0
    )]
    pub end: f32,

    /// What the stroke is painted onto. **On original** lays it over the picture
    /// that arrived; **On transparent** drops the picture and keeps the stroke;
    /// **Reveal original** turns the stroke into a hole in reverse — the picture
    /// survives only where the brush went, which is how a line reveals a title.
    #[choice(
        label = "Paint style",
        options = ["On original", "On transparent", "Reveal original"],
        default = 0
    )]
    pub paint_style: u32,

    /// How strong the brush is, per cent.
    #[slider(
        min = 0.0,
        max = 100.0,
        default = 100.0,
        hard_min = 0.0,
        hard_max = 100.0
    )]
    pub opacity: f32,

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

impl Stroke {
    /// Raster pixels per comp pixel (§2.3), pushed at resolve because the seam
    /// hands its vertices over in px@comp and the brush walks the raster
    /// (K-408). Never a panel row.
    pub const DERIVED_PX_SCALE: ParamId = ParamId::new("derived.px_scale");

    /// This instance's raster factor, read back out of a resolved bag.
    #[must_use]
    pub fn px_scale_of(p: Params<'_>) -> f32 {
        p.float(Self::DERIVED_PX_SCALE, 1.0)
    }

    /// The bundle both kernels consume (docs/impl/effect-registry.md §2.4): the
    /// brush's trail, already laid out and already trimmed.
    ///
    /// An empty or absent mask leaves the count at zero, which the kernel draws
    /// as nothing — the documented no-op both paths take (docs/08 §1.2).
    #[must_use]
    pub fn packed(self, poly: &MaskPolyline, px_scale: f32) -> cpu::PathDrawParams {
        let mut p = cpu::PathDrawParams::blank();
        let width = self.brush_size.max(0.0);
        let spacing = width * (self.spacing.max(0.0) / 100.0);
        cpu::stroke_geometry(poly, px_scale, width, spacing, self.start, self.end, &mut p);
        let half = width * 0.5;
        p.half_width = half;
        // Hardness reads exactly as Vegas' does, and the floor is the same one:
        // a band under half a pixel is an edge no raster can show.
        p.band = ((1.0 - (self.hardness / 100.0).clamp(0.0, 1.0)) * half).max(0.5);
        p.colour = [self.colour[0], self.colour[1], self.colour[2]];
        p.opacity = (self.opacity / 100.0).clamp(0.0, 1.0);
        // The three options are declared in the order the kernel numbers them,
        // so the choice index *is* the style — one place for the mapping.
        p.style = self.paint_style.min(cpu::PAINT_REVEAL_ORIGINAL);
        p.mix = (self.mix / 100.0).clamp(0.0, 1.0);
        p
    }
}

/// Stroke's behaviour: no CPU reference through the trait, because the mask's
/// geometry arrives beside the op rather than in the bag — the same shape Set
/// matte and the LUT have, and for the same reason. `apply_cpu` keeps its
/// identity default, which is exactly what an unset mask row renders anyway; the
/// §1.6 oracle is [`crate::fx::cpu::path_draw`], exercised directly from the
/// lumit-gpu test, which can build a polyline.
pub struct StrokeDef;

impl EffectDef for StrokeDef {
    fn schema(&self) -> &'static EffectSchema {
        &<Stroke as EffectMetadata>::SCHEMA
    }

    fn resolve_derived(&self, cx: &ResolveCx<'_>, push: &mut dyn FnMut(ParamId, Value)) {
        push(Stroke::DERIVED_PX_SCALE, Value::Float(cx.px_scale));
    }
}

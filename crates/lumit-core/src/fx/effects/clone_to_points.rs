//! Clone to points (K-600): a layer's picture stamped at every point of a
//! stream.
//!
//! **In plain terms.** Wire a producer's teal Points socket into this effect,
//! pick a layer, and that layer's picture is stamped once per point — at the
//! point's place, turned by the point's own rotation, sized by the point's own
//! size and tinted by its colour. A hundred snowflakes from Particulate, a
//! lattice of thumbnails from Grid, a logo scattered inside a silhouette: the
//! rig people build by hand out of repeaters and expressions, as one wire.
//!
//! **It is Particulate's Sprite mode, pointed at somebody else's particles.**
//! Not a second implementation of it — literally the same instanced quad, the
//! same bilinear tap, the same premultiplied tint, reached through the shared
//! points draw (K-598). What changes is only where the points came from.
//!
//! **Painter's order is `id` order** and nothing else. The stream arrives
//! ordered by birth index ascending, which is a fact of the evaluation rather
//! than an artefact of how it was scheduled (particulate.md §5), and the stamps
//! are laid down in that order so a later point covers an earlier one. Two
//! renders of one frame therefore lay the same picture down in the same order,
//! on any machine (K-031).
//!
//! **Nothing wired draws nothing** — the picture passes through unchanged, and
//! the box wears the "no stream" mark K-509 gave the family. So does an unset
//! Clone layer row: this effect exists to stamp a layer, and with none to stamp the
//! honest answer is the identity, not a fallback shape somebody has to notice
//! and undo.

use crate::fx::points::{self, PointsStream};
use crate::fx::{
    EffectDef, EffectMetadata, EffectSchema, ParamId, Params, Port, PortType, ResolveCx, Signature,
    Value,
};
use lumit_fx_macros::Effect;

/// The wire-only data input (points-stream.md §4.1): no stored value, nothing
/// to keyframe, no panel row. The **first** such input on a stack effect — the
/// port the note said `Signature::Image` would grow to answer for.
pub const POINTS_PORT: &str = "points";

/// What this effect consumes. Not `three_d`: a stamp is a picture laid on the
/// layer's own flat rectangle, turned by one angle, so what it needs of a point
/// is where the camera puts it and how much it foreshortens — which is exactly
/// what a 2D reading answers (K-561).
const POINTS_IN: &[Port] = &[Port::new(POINTS_PORT, "Points", PortType::Points)];

/// Clone to points' controls.
#[derive(Debug, Clone, Copy, PartialEq, Effect)]
#[effect(
    match_name = "clone_to_points",
    label = "Clone to points",
    version = 1,
    category = Generate,
    cost = Moderate,
    // A point may be anywhere, and a stamp reaches half its own size past it.
    roi = FullFrame,
    premultiplied = true,
    // Not seeded: nothing here is a function of time under constant parameters.
    // The *stream* may well be, and that is the producer's own declaration —
    // its clock folds into its own carriage, and this op's key is chained
    // behind it because a producer sits strictly upstream (K-492).
    seeded = false,
)]
pub struct CloneToPoints {
    /// The layer stamped at every point (K-123, K-142). **Unset draws
    /// nothing**, the ordinary unset-is-identity reading — deliberately unlike
    /// Particulate's Sprite mode, which falls back to discs because a *render
    /// mode* must always draw something. Here there is no mode, only a source.
    #[layer(label = "Clone layer")]
    pub clone_layer: bool,

    /// Multiplies each point's own size, per cent. At 100 a stamp is a square
    /// of the point's diameter, which is what Particulate's Sprite mode draws.
    #[slider(min = 0.0, max = 1000.0, default = 100.0, hard_min = 0.0, unit = Percent)]
    pub scale: f32,

    /// Added to each point's own rotation, degrees.
    #[dial(label = "Rotation", default = 0.0)]
    pub rotation: f32,

    /// Tint each stamp by its point's colour, per cent (0 leaves the layer's
    /// own colours alone, 100 multiplies them by the point's).
    ///
    /// A dial rather than a switch because the stream's colour usually carries
    /// the *fade* as well as the hue — Particulate's Opacity over life lives in
    /// that alpha — so "all of it" and "none of it" are both wanted and so is
    /// everything between. At 0 a stamp is opaque wherever the layer is.
    #[slider(min = 0.0, max = 100.0, default = 100.0, hard_min = 0.0, hard_max = 100.0, unit = Percent)]
    pub tint: f32,

    /// **The budget dial** (K-475), the family's row: the most stamps that may
    /// be drawn at once. A stream longer than this is trimmed to its **newest**
    /// by birth index — the producer's own cap rule applied a second time, so
    /// what vanishes is what a smaller cap would have taken. Not animatable: it
    /// is a capacity declaration.
    #[counter(
        label = "Max clones",
        min = 1,
        max = 200_000,
        default = 2_000,
        hard_min = 1,
        hard_max = points::CAP_HARD,
        unit = Raw
    )]
    pub max_clones: i32,

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

impl CloneToPoints {
    /// The raster factor, for the one input the declaration cannot scale
    /// (K-385): a stream read off a wire is in px@comp, like a mask path, and
    /// has to be rearranged into the pixels the frame is drawn at.
    pub const DERIVED_PX_SCALE: ParamId = ParamId::new("derived.px_scale");

    /// This instance's raster factor, read back out of a resolved bag.
    #[must_use]
    pub fn px_scale_of(p: Params<'_>) -> f32 {
        p.float(Self::DERIVED_PX_SCALE, 1.0)
    }

    /// The stream this effect actually stamps: the wired one, with the two
    /// dials and the cap applied.
    ///
    /// **Every decision is here**, in one expression both render paths read, so
    /// the CPU oracle and the instanced draw cannot come to stamp different
    /// squares. `in_stream` is in the units the caller wants out — px@comp for
    /// a reader, raster pixels for a draw ([`PointsStream::rescaled`]).
    #[must_use]
    pub fn stamps(self, in_stream: &PointsStream) -> PointsStream {
        let mut out = in_stream.clone();
        // The newest by birth index, which is the cap rule the whole family
        // applies (K-475) — deterministic, and the same from any scrub
        // direction.
        out.keep_newest(self.max_clones.clamp(0, points::CAP_HARD as i32) as usize);
        let scale = (self.scale / 100.0).max(0.0);
        let turn = self.rotation.to_radians();
        let tint = (self.tint / 100.0).clamp(0.0, 1.0);
        for s in &mut out.size {
            *s *= scale;
        }
        for r in &mut out.rotation {
            *r += turn;
        }
        for c in &mut out.colour {
            // Towards opaque white, which is the identity of a premultiplied
            // tint: at Tint 0 a stamp is the layer's own picture, at 100 it is
            // the picture times the point's colour and alpha.
            for ch in c.iter_mut() {
                *ch = 1.0 + (*ch - 1.0) * tint;
            }
        }
        out
    }

    /// How the stamps are drawn — Sprite mode, and the host Mix (K-425).
    #[must_use]
    pub fn draw_style(self) -> points::DrawStyle {
        points::DrawStyle {
            mode: points::RenderMode::Sprite,
            feather: 0.0,
            streak_seconds: 0.0,
            mix: (self.mix / 100.0).clamp(0.0, 1.0),
        }
    }
}

/// Clone to points' behaviour.
///
/// **No CPU reference through the trait**, for the reason every points effect
/// has one fewer than it looks: what it draws is a stream and a camera, neither
/// of which is a number in the bag [`apply_cpu`](EffectDef::apply_cpu) is
/// handed. Both ride the carriage beside the op. The §1.6 oracle is
/// [`CloneToPoints::stamps`] with [`points::draw_stream`], exercised directly
/// from the test suite — and it is the very stream the GPU draw is handed.
pub struct CloneToPointsDef;

impl EffectDef for CloneToPointsDef {
    fn schema(&self) -> &'static EffectSchema {
        &<CloneToPoints as EffectMetadata>::SCHEMA
    }

    /// A picture in, a picture out, and a **stream in** beside it (K-492,
    /// K-600) — the first stack effect to declare a data input.
    fn signature(&self) -> Signature {
        Signature::Image {
            inputs: POINTS_IN,
            extra: &[],
        }
    }

    /// The raster factor, so a px@comp stream reaches the pixels this frame is
    /// drawn at (K-385).
    fn resolve_derived(&self, cx: &ResolveCx<'_>, push: &mut dyn FnMut(ParamId, Value)) {
        push(CloneToPoints::DERIVED_PX_SCALE, Value::Float(cx.px_scale));
    }
}

//! Emit from image: points thrown at a picture, kept where it is bright.
//!
//! **In plain terms.** Grid puts points where the arithmetic says and Scatter
//! keeps the ones that land on something *opaque*; this one keeps the ones that
//! land on something *bright*. Point it at a layer — a title, a logo, a noise
//! pattern, a luma matte somebody painted — and its highlights become a cloud
//! of points in the shape of themselves. Threshold is where "bright" starts.
//!
//! **The acceptance is rejection sampling**, exactly as Scatter's is:
//! candidate *i* falls at a place its own seeded dice chose, and stands if the
//! field under it beats a second die. Where the field is 1 every candidate
//! stands; where it is a half, half of them do; where it is nothing, none. That
//! is what makes a soft gradient come out as a thinning of the crowd rather
//! than as a hard cut, and it is why the dial is a *threshold* rather than a
//! switch.
//!
//! **Brightness is measured on the light, not on the coverage.** The picture is
//! premultiplied, so a half-covered white pixel carries half the light of a
//! covered one and would read as grey; the honest answer to "how bright is what
//! is *there*" divides the coverage back out first. Threshold is then the floor
//! and full white the ceiling, so the field is a proper chance again.
//!
//! **Its stream cannot be sampled by a driver or by a stack consumer**, which
//! is points-stream.md §2.2's recorded constraint answered the same way
//! Scatter answers it: the stream is a function of a picture, and at
//! resolve time — when the driver walk runs, and when the draw builder fills a
//! consumer's carriage — no picture exists. Both read the documented empty
//! stream rather than a guess.

use crate::fx::effects::scatter::DENSITY_CELL;
use crate::fx::points::{self, DrawStyle, PointsStream, Projection, RenderMode};
use crate::fx::{
    EffectDef, EffectMetadata, EffectSchema, ParamGroup, ParamId, Params, Port, PortType,
    ResolveCx, Signature, Value,
};
use lumit_fx_macros::Effect;

/// The declared **data** output — the same port Particulate, Grid and Scatter
/// declare, so a wire does not know which producer it came from.
const POINTS_OUT: &[Port] = &[Port::new("points", "Points", PortType::Points)];

const fn group(label: &'static str, params: &'static [&'static str]) -> ParamGroup {
    ParamGroup {
        label,
        params,
        collapsed: false,
        visible_when: None,
        visible_when_lens_elements: None,
    }
}

/// Two kickers, as the other generators have: where the points come from, and
/// what one looks like.
pub const EMIT_FROM_IMAGE_GROUPS: &[ParamGroup] = &[
    group("Source", &["source", "threshold", "density", "seed"]),
    group("Point", &["size", "feather", "colour", "max_points"]),
];

/// Which per-candidate draw is being made.
mod attr {
    /// Where across the frame the candidate falls.
    pub const PLACE_U: u32 = 0;
    /// Where down it falls.
    pub const PLACE_V: u32 = 1;
    /// **The acceptance die** — the number the field under the candidate has to
    /// beat. The same id Scatter's is (`A_ACCEPT` in `fx_particulate.wgsl`),
    /// because it is the one die the WGSL draw re-rolls for itself and there is
    /// one such draw for both rejections.
    pub const ACCEPT: u32 = 16;
}

/// Emit from image's controls.
#[derive(Debug, Clone, Copy, PartialEq, Effect)]
#[effect(
    match_name = "emit_from_image",
    label = "Emit from image",
    version = 1,
    category = Generate,
    cost = Moderate,
    roi = FullFrame,
    premultiplied = true,
    // Not seeded, for Scatter's reason: `seeded` says the pixels are a
    // function of *time* under constant parameters. These are a function of a
    // picture, which the frame key already covers.
    seeded = false,
    groups = EMIT_FROM_IMAGE_GROUPS,
)]
pub struct EmitFromImage {
    /// The layer whose brightness the points land in. **Unset
    /// reads this effect's own input picture**, which is the documented
    /// default and the same reading an unset Matte gives Scatter.
    #[layer(label = "Source layer")]
    pub source: bool,

    /// How bright a pixel must be before a candidate on it stands any chance at
    /// all, per cent of full white. At 0 the field is the brightness itself, so
    /// a dim area is thinly populated rather than empty; at 100 only a pixel
    /// brighter than white keeps anything, which is the documented no-op.
    #[slider(
        label = "Threshold",
        min = 0.0,
        max = 100.0,
        default = 50.0,
        hard_min = 0.0,
        hard_max = 100.0,
        unit = Percent
    )]
    pub threshold: f32,

    /// How many candidate points are thrown per hundred-pixel square of the
    /// composition — a count per area, so the same dial means the same spacing
    /// whatever size the comp is.
    #[slider(min = 0.0, max = 100.0, default = 20.0, hard_min = 0.0, unit = Raw)]
    pub density: f32,

    /// Which points (§2.4). The reseed button rolls it — and rolls both dice,
    /// so a reseed moves the crowd *and* changes which of it survives.
    #[seed]
    pub seed: u32,

    /// The diameter of the disc a point is drawn as, px@comp.
    #[slider(min = 0.0, max = 200.0, default = 6.0, hard_min = 0.0, unit = Px)]
    pub size: f32,

    /// How soft that disc's edge is, per cent.
    #[slider(
        min = 0.0,
        max = 100.0,
        default = 100.0,
        hard_min = 0.0,
        hard_max = 100.0,
        unit = Percent
    )]
    pub feather: f32,

    /// The colour a point is drawn in. Scene-linear, and values above 1 are
    /// legal and useful over a glow.
    #[colour(default = [1.0, 1.0, 1.0, 1.0], max = 4.0)]
    pub colour: [f32; 4],

    /// **The budget dial**, as Scatter's is: the most **candidates**
    /// that may be thrown, and the peak scratch the governor grants against.
    /// What stands is a subset of that, so the cap is a ceiling on the work and
    /// not on the look. Not animatable — it is a capacity declaration.
    #[counter(
        label = "Max points",
        min = 1,
        max = 200_000,
        default = points::CAP_DEFAULT,
        hard_min = 1,
        hard_max = points::CAP_HARD,
        unit = Raw
    )]
    pub max_points: i32,

    /// The host-uniform Mix every effect ends with (docs/08 §1.5), per cent.
    /// **At nought the stream is still emitted and nothing is drawn**, the
    /// emit-only mode Grid documents.
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

impl EmitFromImage {
    /// The raster factor, for the two things the declaration cannot scale: the
    /// composition's camera, and the **count** — Density is per area of the
    /// composition, so it is measured against the comp's own size and not
    /// against whatever raster is previewing.
    pub const DERIVED_PX_SCALE: ParamId = ParamId::new("derived.px_scale");

    /// This instance's raster factor, read back out of a resolved bag.
    #[must_use]
    pub fn px_scale_of(p: Params<'_>) -> f32 {
        p.float(Self::DERIVED_PX_SCALE, 1.0)
    }

    /// How many candidates are thrown at a `w × h` raster whose comp pixels are
    /// `px_scale` of a raster pixel each — Scatter's rule and Scatter's cell
    /// ([`DENSITY_CELL`]), because a density that meant two things would be two
    /// densities.
    #[must_use]
    pub fn candidate_count(self, w: u32, h: u32, px_scale: f32) -> usize {
        let s = px_scale.max(1e-6);
        let area = (w as f32 / s) * (h as f32 / s);
        let n = (self.density.max(0.0) * area / DENSITY_CELL).round();
        let cap = self.max_points.clamp(0, points::CAP_HARD as i32) as f32;
        n.clamp(0.0, cap) as usize
    }

    /// Every candidate, accepted or not — where each falls, and how it would be
    /// drawn.
    ///
    /// **The candidate set is what the card is given** (Scatter's shape): the
    /// rejection happens in the vertex stage, because that is the only place a
    /// host-built set can meet a picture that exists only on the card. The
    /// accepted subset — the *stream* — is [`stream`](Self::stream).
    #[must_use]
    pub fn candidates(self, w: u32, h: u32, px_scale: f32, projection: Projection) -> PointsStream {
        let n = self.candidate_count(w, h, px_scale);
        let mut out = PointsStream {
            projection,
            ..PointsStream::default()
        };
        let size = self.size.max(0.0);
        let a = self.colour[3];
        let colour = [
            self.colour[0] * a,
            self.colour[1] * a,
            self.colour[2] * a,
            a,
        ];
        for i in 0..n as u64 {
            out.position.push([
                points::draw(self.seed, i, attr::PLACE_U) * w as f32,
                points::draw(self.seed, i, attr::PLACE_V) * h as f32,
                0.0,
            ]);
            out.speed.push([0.0; 3]);
            out.age.push(0.0);
            out.life.push(1.0);
            out.size.push(size);
            out.rotation.push(0.0);
            out.colour.push(colour);
            out.id.push(i);
        }
        out
    }

    /// Whether candidate `id` stands where the field reads `field`.
    ///
    /// Scatter's rule, and deliberately the same one: a field of 1 keeps every
    /// candidate, a field of a half keeps half of them, a field of nothing
    /// keeps none. The die is the candidate's own, so the answer is the same at
    /// every frame and from any scrub direction.
    #[must_use]
    pub fn accepts(self, field: f32, id: u64) -> bool {
        field > points::draw(self.seed, id, attr::ACCEPT)
    }

    /// The **brightness** field under a point: the unpremultiplied luminance of
    /// `rgba` at the pixel it lands on, with this instance's Threshold as the
    /// floor and full white as the ceiling.
    ///
    /// **Nearest, not bilinear**, and named rather than assumed: this is the
    /// arithmetic the vertex stage does with one `textureLoad`, and a reference
    /// that filtered where the kernel does not would be a reference that
    /// disagreed. Off the raster reads as nothing, which is the honest answer
    /// for a point that is not on the picture.
    #[must_use]
    pub fn field_at(self, at: [f32; 3], w: u32, h: u32, rgba: &[f32]) -> f32 {
        let x = at[0].floor();
        let y = at[1].floor();
        let inside = x >= 0.0 && y >= 0.0 && (x as u32) < w && (y as u32) < h;
        let mut texel = [0.0f32; 4];
        if inside {
            let d = ((y as u32 * w + x as u32) * 4) as usize;
            if let Some(t) = rgba.get(d..d + 4) {
                texel.copy_from_slice(t);
            }
        }
        // Unpremultiply first: a half-covered white pixel is white, not grey.
        // No coverage is no light, which is what an empty pixel means.
        let bright = if texel[3] > 0.0 {
            let w = crate::fx::cpu::LUMA;
            (texel[0] * w[0] + texel[1] * w[1] + texel[2] * w[2]) / texel[3]
        } else {
            0.0
        };
        let floor = (self.threshold / 100.0).clamp(0.0, 1.0);
        ((bright - floor) / (1.0 - floor).max(1e-3)).clamp(0.0, 1.0)
    }

    /// The points that stand: the **stream**.
    ///
    /// `rgba` is the field's own picture, premultiplied linear — the Source
    /// layer's frame, or this effect's input when the row is unset — at the
    /// same `w × h` raster the candidates were thrown at.
    #[must_use]
    pub fn stream(
        self,
        w: u32,
        h: u32,
        px_scale: f32,
        rgba: &[f32],
        projection: Projection,
    ) -> PointsStream {
        let all = self.candidates(w, h, px_scale, projection);
        let mut out = PointsStream {
            projection,
            ..PointsStream::default()
        };
        for i in 0..all.len() {
            let at = all.position[i];
            if !self.accepts(self.field_at(at, w, h, rgba), all.id[i]) {
                continue;
            }
            out.position.push(at);
            out.speed.push(all.speed[i]);
            out.age.push(all.age[i]);
            out.life.push(all.life[i]);
            out.size.push(all.size[i]);
            out.rotation.push(all.rotation[i]);
            out.colour.push(all.colour[i]);
            out.id.push(all.id[i]);
        }
        out
    }

    /// How the stream is drawn — a feathered disc per point, and the host Mix.
    #[must_use]
    pub fn draw_style(self) -> DrawStyle {
        DrawStyle {
            mode: RenderMode::Disc,
            feather: (self.feather / 100.0).clamp(0.0, 1.0),
            streak_seconds: 0.0,
            mix: (self.mix / 100.0).clamp(0.0, 1.0),
        }
    }
}

/// Emit from image's behaviour.
///
/// **No CPU reference through the trait**, for Scatter's two reasons: the
/// composition's camera is not in the bag, and neither is the picture the field
/// is read from. Both ride beside the op. The §1.6 oracle is
/// [`EmitFromImage::stream`] with [`points::draw_stream`], exercised directly
/// from the test suite against the same picture the kernel samples.
pub struct EmitFromImageDef;

impl EffectDef for EmitFromImageDef {
    fn schema(&self) -> &'static EffectSchema {
        &<EmitFromImage as EffectMetadata>::SCHEMA
    }

    /// The picture *and* the data, exactly as the other three producers
    /// declare it — and see this file's header on why a **driver** and a stack
    /// consumer may not sample it.
    fn signature(&self) -> Signature {
        Signature::Image {
            inputs: &[],
            extra: POINTS_OUT,
        }
    }

    /// The raster factor: the camera, and the count Density means per
    /// composition area.
    fn resolve_derived(&self, cx: &ResolveCx<'_>, push: &mut dyn FnMut(ParamId, Value)) {
        push(EmitFromImage::DERIVED_PX_SCALE, Value::Float(cx.px_scale));
    }
}

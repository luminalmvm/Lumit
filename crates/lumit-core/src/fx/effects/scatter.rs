//! Scatter (K-599): points thrown at the picture, kept where there is alpha.
//!
//! **In plain terms.** Grid puts points where the arithmetic says; Scatter
//! throws them at random and keeps the ones that land on something. "Something"
//! is the layer's own alpha — or a matte layer's, if one is bound — so a
//! silhouette, a piece of text, a keyed subject becomes a cloud of points in
//! the shape of itself. Density is a dial, and the acceptance is **rejection
//! sampling**: candidate *i* falls at a place its own seeded dice chose, and
//! stands if the alpha under it beats a second die. Where the alpha is 1 every
//! candidate stands; where it is a half, half of them do; where it is nothing,
//! none. That is what makes a soft edge come out as a thinning of the crowd
//! rather than as a hard cut.
//!
//! **Deterministic and scrub-safe**, with one thing named rather than hidden:
//! a candidate's *place* is a function of the seed and its index alone, so it
//! is the same place at every frame and at every preview resolution. What can
//! change with the preview resolution is whether a candidate on a **soft edge**
//! is accepted, because the alpha it reads is the picture at the raster being
//! drawn, and a half-resolution picture is a different picture. At full
//! resolution preview and export are the same by construction (K-031), which is
//! where that guarantee is actually made.
//!
//! **Its stream cannot be sampled by a driver**, and that is the recorded
//! answer to points-stream.md §2.2's constraint: the stream is a function of
//! the input picture, and at resolve time — when the driver walk runs — no
//! picture exists. A points wire from Scatter into a Points sample reads the
//! documented empty stream rather than a guess.

use crate::fx::points::{self, DrawStyle, PointsStream, Projection, RenderMode};
use crate::fx::{
    EffectDef, EffectMetadata, EffectSchema, ParamGroup, ParamId, Params, Port, PortType,
    ResolveCx, Signature, Value,
};
use lumit_fx_macros::Effect;

/// Scatter's declared **data** output — the same port Particulate and Grid
/// declare (K-472, K-492).
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

/// Two kickers, as Grid has: how many points, and what one looks like.
pub const SCATTER_GROUPS: &[ParamGroup] = &[
    group("Scatter", &["density", "seed"]),
    group("Point", &["size", "feather", "colour", "max_points"]),
];

/// The area, in **px@comp**, that Density counts its points against.
///
/// A hundred pixels square. A density is a count per area or it is not a
/// density — the same dial on a 4K comp and on a 1080 one has to mean the same
/// spacing, or the effect would thin out the moment somebody changed the
/// composition's size.
pub const DENSITY_CELL: f32 = 100.0 * 100.0;

/// Which per-candidate draw is being made.
mod attr {
    /// Where across the frame the candidate falls.
    pub const PLACE_U: u32 = 0;
    /// Where down it falls.
    pub const PLACE_V: u32 = 1;
    /// **The acceptance die** — the number the alpha under the candidate has
    /// to beat. Deliberately 16 and not 2: it is the one die the WGSL draw
    /// re-rolls for itself (`A_ACCEPT` in `fx_particulate.wgsl`), so it sits
    /// clear of every neighbour rather than next to them.
    pub const ACCEPT: u32 = 16;
}

/// Scatter's controls.
#[derive(Debug, Clone, Copy, PartialEq, Effect)]
#[effect(
    match_name = "scatter",
    label = "Scatter",
    version = 1,
    category = Generate,
    cost = Moderate,
    roi = FullFrame,
    premultiplied = true,
    // Not seeded, for Grid's reason (K-598): `seeded` says the pixels are a
    // function of *time* under constant parameters. Scatter's are a function
    // of its **input**, which the frame key already covers.
    seeded = false,
    // **The K-395 override.** The matte is not a strength here — it is *where
    // the points go*, consumed inside the effect's own maths, so the generic
    // dissolve must not run as well.
    matte = (
        "matte",
        "chooses which layer's alpha the points land inside, instead of this layer's own"
    ),
    // No Channel row: this effect reads alpha and only alpha, on purpose. A
    // luma matte becomes an alpha one through Matte key or Set matte, which is
    // what those exist for, and one rule here is one sentence in the manual.
    matte_channel = false,
    groups = SCATTER_GROUPS,
)]
pub struct Scatter {
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

    /// **The budget dial** (K-475), as Grid's is: the most **candidates** that
    /// may be thrown, and the peak scratch the governor grants against. What
    /// stands is a subset of that, so the cap is a ceiling on the work and not
    /// on the look. Not animatable — it is a capacity declaration.
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

impl Scatter {
    /// The raster factor, for the two things the declaration cannot scale
    /// (K-385): the composition's camera, and the **count** — Density is per
    /// area of the composition, so it is measured against the comp's own size
    /// and not against whatever raster is previewing.
    pub const DERIVED_PX_SCALE: ParamId = ParamId::new("derived.px_scale");

    /// This instance's raster factor, read back out of a resolved bag.
    #[must_use]
    pub fn px_scale_of(p: Params<'_>) -> f32 {
        p.float(Self::DERIVED_PX_SCALE, 1.0)
    }

    /// How many candidates are thrown at a `w × h` raster whose comp pixels are
    /// `px_scale` of a raster pixel each.
    ///
    /// Rounded, and worked out in composition pixels on purpose: two previews
    /// of one frame at different resolutions throw the **same** candidates, so
    /// changing the preview divisor never re-rolls the crowd.
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
    /// **The candidate set is what the card is given** (K-599): the rejection
    /// happens in the vertex stage, because that is the only place a host-built
    /// set can meet a picture that exists only on the card. The accepted subset
    /// — the *stream* — is [`stream`](Self::stream), and it is what the CPU
    /// reference draws and what a stack consumer will read.
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
    /// **Rejection sampling**, which is what makes the crowd's density track
    /// the alpha rather than its edge: a field of 1 keeps every candidate, a
    /// field of a half keeps half of them, a field of nothing keeps none. The
    /// die is the candidate's own, so the answer is the same at every frame and
    /// from any scrub direction.
    #[must_use]
    pub fn accepts(self, field: f32, id: u64) -> bool {
        field > points::draw(self.seed, id, attr::ACCEPT)
    }

    /// The field under a point: the alpha of `rgba` at the pixel it lands on.
    ///
    /// **Nearest, not bilinear**, and named rather than assumed: this is the
    /// arithmetic the vertex stage does with one `textureLoad`, and a
    /// reference that filtered where the kernel does not would be a reference
    /// that disagreed. Off the raster reads as nothing, which is the honest
    /// answer for a point that is not on the picture.
    #[must_use]
    pub fn field_at(at: [f32; 3], w: u32, h: u32, rgba: &[f32], invert: bool) -> f32 {
        let x = at[0].floor();
        let y = at[1].floor();
        let inside = x >= 0.0 && y >= 0.0 && (x as u32) < w && (y as u32) < h;
        let a = if inside {
            let d = ((y as u32 * w + x as u32) * 4 + 3) as usize;
            rgba.get(d).copied().unwrap_or(0.0)
        } else {
            0.0
        };
        if invert {
            1.0 - a
        } else {
            a
        }
    }

    /// The points that stand: the **stream** (K-599).
    ///
    /// `rgba` is the field's own picture, premultiplied linear — the effect's
    /// input, or the bound matte's frame — at the same `w × h` raster the
    /// candidates were thrown at.
    #[must_use]
    #[allow(clippy::too_many_arguments)]
    pub fn stream(
        self,
        w: u32,
        h: u32,
        px_scale: f32,
        rgba: &[f32],
        invert: bool,
        projection: Projection,
    ) -> PointsStream {
        let all = self.candidates(w, h, px_scale, projection);
        let mut out = PointsStream {
            projection,
            ..PointsStream::default()
        };
        for i in 0..all.len() {
            let at = all.position[i];
            if !self.accepts(Self::field_at(at, w, h, rgba, invert), all.id[i]) {
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

/// Scatter's behaviour.
///
/// **No CPU reference through the trait**, for Grid's reason and one more of
/// its own: the composition's camera is not in the bag, and neither is the
/// picture the field is read from when a matte is bound. Both ride beside the
/// op. The §1.6 oracle is [`Scatter::stream`] with [`points::draw_stream`],
/// exercised directly from the test suite against the same picture the kernel
/// samples.
pub struct ScatterDef;

impl EffectDef for ScatterDef {
    fn schema(&self) -> &'static EffectSchema {
        &<Scatter as EffectMetadata>::SCHEMA
    }

    /// The picture *and* the data (K-472), exactly as Particulate and Grid
    /// declare it — and see this file's header on why a **driver** may not
    /// sample it.
    fn signature(&self) -> Signature {
        Signature::Image {
            inputs: &[],
            extra: POINTS_OUT,
        }
    }

    /// The raster factor: the camera, and the count Density means per
    /// composition area (K-385).
    fn resolve_derived(&self, cx: &ResolveCx<'_>, push: &mut dyn FnMut(ParamId, Value)) {
        push(Scatter::DERIVED_PX_SCALE, Value::Float(cx.px_scale));
    }
}

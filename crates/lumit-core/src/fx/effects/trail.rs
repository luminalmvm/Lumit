//! Trail (K-601): where every point of a stream has been, drawn by asking the
//! producer again rather than by remembering.
//!
//! **In plain terms.** Wire a producer's teal Points socket into this effect and
//! each point grows a tail: a line of dots, or one connected ribbon, running
//! back through the places it was a moment ago and fading as it goes.
//!
//! **Nothing is stored, ever.** A trail is the obvious place to keep a history —
//! and keeping one would cost this engine everything it has: a frame that
//! depended on the frame before it cannot be scrubbed to, cannot be rendered out
//! of order, and cannot promise that two renders agree (K-474). So Trail does
//! what Streak does and does it further: it evaluates the producer's stream
//! **again**, at `t − k·Spacing`, once per sample, and reads each point's older
//! self out of the answer. Frame 500's trail costs the same as frame 3's, from a
//! cold start, from either scrub direction.
//!
//! **Points are matched by `id`.** A stream is ordered by birth index ascending,
//! and the past stream is too, so "where was point 4 172 a moment ago?" is a
//! walk that only ever moves forwards — no map, no search, no allocation per
//! point. A point with no older self simply has a shorter tail: it was not born
//! yet, which is exactly the honest picture.
//!
//! **Painter's order is oldest sample first**, then `id` inside each sample, so
//! the near end of a tail lands on top of the far end and one point's tail lands
//! on the next point's in a fixed order (K-031).
//!
//! **Nothing wired draws nothing** — the picture passes through, and the box
//! wears the "no stream" mark K-509 gave the family.

use crate::fx::points::{self, PointsStream};
use crate::fx::{
    EffectDef, EffectMetadata, EffectSchema, ParamId, Params, Port, PortType, ResolveCx, Signature,
    Value,
};
use lumit_fx_macros::Effect;

/// The wire-only data input (points-stream.md §4.1).
pub const POINTS_PORT: &str = "points";

/// What this effect consumes. Not `three_d`: a trail is a line drawn on the
/// layer's own flat picture, so what it wants of a point is where the camera
/// puts it — which is what a 2D reading answers (K-561).
const POINTS_IN: &[Port] = &[Port::new(POINTS_PORT, "Points", PortType::Points)];

/// What a tail is drawn as.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TrailStyle {
    /// One dab per sample: a dotted tail, and the honest picture of what was
    /// actually evaluated.
    #[default]
    Dots,
    /// A capsule from each sample to the one before it: a continuous ribbon,
    /// which is the same kernel with the tail somewhere other than the head.
    Segments,
}

impl TrailStyle {
    /// The Choice option labels, in code order.
    pub const OPTIONS: &'static [&'static str] = &["Dots", "Segments"];

    /// The style for a stored Choice index; anything unknown is Dots, the
    /// declared default (a document from a newer build renders, K-065).
    #[must_use]
    pub const fn from_code(code: u32) -> Self {
        match code {
            1 => TrailStyle::Segments,
            _ => TrailStyle::Dots,
        }
    }
}

/// Trail's controls.
#[derive(Debug, Clone, Copy, PartialEq, Effect)]
#[effect(
    match_name = "trail",
    label = "Trail",
    version = 1,
    category = Generate,
    // Heavier than its siblings, and honestly so: every sample is another whole
    // evaluation of the producer's stream. Samples is the dial that says how
    // many, and it is the budget.
    cost = Heavy,
    roi = FullFrame,
    premultiplied = true,
    // Not seeded: nothing here is a function of time under constant parameters.
    // The producer's stream is, and that is the producer's own declaration.
    seeded = false,
)]
pub struct Trail {
    /// How many places back a tail is drawn through, **including where the
    /// point is now** — so 1 is the point itself and no tail at all.
    ///
    /// **This is the budget dial** as much as it is a look: each sample is a
    /// second, third, fourth evaluation of the whole producer stream, on the
    /// host, at a different moment. The default is short on purpose.
    #[counter(
        label = "Samples",
        min = 1,
        max = 64,
        default = 8,
        hard_min = 1,
        hard_max = 256,
        unit = Raw
    )]
    pub back_samples: i32,

    /// How far apart in time those places are, seconds of layer time. A comp
    /// frame is the natural setting and the default is close to one.
    #[slider(
        label = "Spacing",
        min = 0.001,
        max = 1.0,
        default = 0.033,
        hard_min = 0.001,
        unit = Seconds
    )]
    pub back_step: f32,

    /// Dots or one connected ribbon ([`TrailStyle`]).
    ///
    /// The option list is [`TrailStyle::OPTIONS`] rather than a second copy of
    /// the words, so the labels and `from_code` cannot come to disagree about
    /// which index means what.
    #[choice(label = "Style", options = *TrailStyle::OPTIONS, default = 0)]
    pub style: u32,

    /// Multiplies each point's own size, per cent — the diameter of a dot, or
    /// the thickness of a segment.
    #[slider(min = 0.0, max = 400.0, default = 60.0, hard_min = 0.0, unit = Percent)]
    pub scale: f32,

    /// How soft a dab's edge is, per cent.
    #[slider(
        min = 0.0,
        max = 100.0,
        default = 100.0,
        hard_min = 0.0,
        hard_max = 100.0,
        unit = Percent
    )]
    pub feather: f32,

    /// How far the far end of a tail fades, per cent: at 100 the oldest sample
    /// is invisible and the tail dies away, at 0 the whole tail is as solid as
    /// the point.
    #[slider(
        min = 0.0,
        max = 100.0,
        default = 100.0,
        hard_min = 0.0,
        hard_max = 100.0,
        unit = Percent
    )]
    pub fade: f32,

    /// **The budget dial** (K-475), the family's row: the most **points** that
    /// may grow a tail. A stream longer than this is trimmed to its newest by
    /// birth index, the producer's own cap rule applied a second time. Not
    /// animatable — it is a capacity declaration.
    #[counter(
        label = "Max trails",
        min = 1,
        max = 200_000,
        default = 2_000,
        hard_min = 1,
        hard_max = points::CAP_HARD,
        unit = Raw
    )]
    pub max_trails: i32,

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

impl Trail {
    /// The raster factor, for the one input the declaration cannot scale
    /// (K-385): a stream read off a wire is in px@comp and has to be rearranged
    /// into the pixels the frame is drawn at.
    pub const DERIVED_PX_SCALE: ParamId = ParamId::new("derived.px_scale");

    /// This instance's raster factor, read back out of a resolved bag.
    #[must_use]
    pub fn px_scale_of(p: Params<'_>) -> f32 {
        p.float(Self::DERIVED_PX_SCALE, 1.0)
    }

    /// How many samples back the carriage should evaluate, and how far apart —
    /// the two numbers the **draw builder** reads off this effect's stored rows
    /// before any bag exists (points-stream.md §3.3).
    ///
    /// Named by their parameter ids rather than by this effect's name, the way
    /// a producer's birth scan is found by its `emit_rate` row (K-598): a later
    /// consumer that wants the same back-samples declares the same two ids and
    /// needs no edit to the builder.
    pub const SAMPLES_PARAM: &'static str = "back_samples";
    /// See [`SAMPLES_PARAM`](Self::SAMPLES_PARAM).
    pub const STEP_PARAM: &'static str = "back_step";

    /// The tail, drawn: every sample of every point that has one, and — for
    /// Segments — where each dab's capsule runs back to.
    ///
    /// `samples[0]` is this frame and `samples[k]` is `k` steps into the past,
    /// all in the units the caller wants out (px@comp for a reader, raster
    /// pixels for a draw). Fewer samples than Samples asks for is not an error:
    /// the carriage hands over what it could evaluate, and a shorter tail is
    /// what a shorter list means.
    ///
    /// **Every decision is here**, in one expression both render paths read, so
    /// the CPU oracle and the instanced draw cannot come to draw different
    /// tails.
    #[must_use]
    pub fn tail(self, samples: &[PointsStream]) -> (PointsStream, Vec<[f32; 3]>) {
        let mut out = PointsStream {
            projection: samples
                .first()
                .map_or_else(Default::default, |s| s.projection),
            ..PointsStream::default()
        };
        let mut tails: Vec<[f32; 3]> = Vec::new();
        let Some(head) = samples.first() else {
            return (out, tails);
        };
        // The points that grow a tail at all: the newest by birth index, which
        // is the cap rule the whole family applies (K-475).
        let mut heads = head.clone();
        heads.keep_newest(self.max_trails.clamp(0, points::CAP_HARD as i32) as usize);
        let wanted =
            (self.back_samples.clamp(1, points::CAP_HARD as i32) as usize).min(samples.len());
        let scale = (self.scale / 100.0).max(0.0);
        let fade = (self.fade / 100.0).clamp(0.0, 1.0);
        let segments = TrailStyle::from_code(self.style) == TrailStyle::Segments;
        // The far end's own share of the point's alpha; the near end keeps all
        // of it. One sample is the near end and nothing else, so it never
        // divides by nought.
        let last = (wanted.saturating_sub(1)).max(1) as f32;

        // **Oldest first**, so the near end of a tail lands on top of the far
        // end. Inside a sample, `id` order, which the stream already carries.
        for k in (0..wanted).rev() {
            let Some(past) = samples.get(k) else { continue };
            // Where each dab's capsule runs back to: the sample before this one
            // in time, which for Dots and for the far end is the dab itself.
            let older = segments.then(|| samples.get(k + 1)).flatten();
            let dim = 1.0 - fade * (k as f32 / last).min(1.0);
            let mut cursor = 0usize;
            let mut older_cursor = 0usize;
            for i in 0..heads.len() {
                let id = heads.id[i];
                let Some(j) = PointsStream::seek_id(past, id, &mut cursor) else {
                    // Not alive then: the tail simply stops there.
                    continue;
                };
                let at = past.position[j];
                let colour = past.colour[j];
                out.position.push(at);
                out.speed.push(past.speed[j]);
                out.age.push(past.age[j]);
                out.life.push(past.life[j]);
                out.size.push(past.size[j] * scale);
                out.rotation.push(past.rotation[j]);
                out.colour.push([
                    colour[0] * dim,
                    colour[1] * dim,
                    colour[2] * dim,
                    colour[3] * dim,
                ]);
                out.id.push(id);
                if segments {
                    let back = older
                        .and_then(|o| {
                            PointsStream::seek_id(o, id, &mut older_cursor).map(|m| o.position[m])
                        })
                        .unwrap_or(at);
                    tails.push(back);
                }
            }
        }
        (out, tails)
    }

    /// How the tail is drawn — dabs and capsules through the one kernel, and
    /// the host Mix (K-425).
    #[must_use]
    pub fn draw_style(self) -> points::DrawStyle {
        points::DrawStyle {
            // A capsule is a disc when its tail is its head, so the mode is the
            // disc's either way and the geometry is in the tails.
            mode: points::RenderMode::Disc,
            feather: (self.feather / 100.0).clamp(0.0, 1.0),
            // Not Particulate's Streak: that one asks the *evaluation* for a
            // tail at an age offset. This one already has its own.
            streak_seconds: 0.0,
            mix: (self.mix / 100.0).clamp(0.0, 1.0),
        }
    }
}

/// Trail's behaviour.
///
/// **No CPU reference through the trait**, the shape every points effect has:
/// what it draws is a stream and a camera, neither of which is a number in the
/// bag [`apply_cpu`](EffectDef::apply_cpu) is handed. Both ride the carriage
/// beside the op. The §1.6 oracle is [`Trail::tail`] with
/// [`points::draw_stream`], exercised directly from the test suite.
pub struct TrailDef;

impl EffectDef for TrailDef {
    fn schema(&self) -> &'static EffectSchema {
        &<Trail as EffectMetadata>::SCHEMA
    }

    /// A picture in, a picture out, and a **stream in** beside it (K-492,
    /// K-601).
    fn signature(&self) -> Signature {
        Signature::Image {
            inputs: POINTS_IN,
            extra: &[],
        }
    }

    /// The raster factor, so a px@comp stream reaches the pixels this frame is
    /// drawn at (K-385).
    fn resolve_derived(&self, cx: &ResolveCx<'_>, push: &mut dyn FnMut(ParamId, Value)) {
        push(Trail::DERIVED_PX_SCALE, Value::Float(cx.px_scale));
    }
}

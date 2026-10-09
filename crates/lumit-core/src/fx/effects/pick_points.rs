//! Pick points keeps some of a stream's points and drops the rest.
//!
//! Each point gets a number from 0 to 1 from the same pattern Vary points
//! reads, and is kept while that number lies between From and To. Every nth
//! thins the stream by count as well. Kept points keep their `id`, so a Trail
//! below still follows them. They go out on the Points socket, and are drawn
//! as discs unless Mix is 0. Nothing wired draws nothing.

use crate::fx::effects::vary_points::{Pattern, PatternKind, POINTS_IN, POINTS_OUT};
use crate::fx::points::PointsStream;
use crate::fx::{
    EffectDef, EffectMetadata, EffectSchema, ParamGroup, ParamId, Params, ResolveCx, Signature,
    Value,
};
use lumit_fx_macros::Effect;

const fn group(
    label: &'static str,
    params: &'static [&'static str],
    visible_when: Option<(&'static str, &'static [u32])>,
) -> ParamGroup {
    ParamGroup {
        label,
        params,
        collapsed: false,
        visible_when,
        visible_when_lens_elements: None,
    }
}

/// The pattern and its range, the rows only some patterns read, and the count.
pub const PICK_GROUPS: &[ParamGroup] = &[
    group("Pattern", &["pattern", "from", "to"], None),
    group("", &["seed"], Some(("pattern", &[1, 2]))),
    group("", &["noise_scale", "noise_speed"], Some(("pattern", &[2]))),
    group(
        "",
        &["centre_x", "centre_y", "radius"],
        Some(("pattern", &[3])),
    ),
    group("Count", &["every_nth", "nth_offset", "invert"], None),
    group("Point", &["feather"], None),
];

/// Pick points' controls.
#[derive(Debug, Clone, Copy, PartialEq, Effect)]
#[effect(
    match_name = "pick_points",
    label = "Pick points",
    version = 1,
    category = Generate,
    cost = Moderate,
    roi = FullFrame,
    premultiplied = true,
    // Seeded, since a Noise pattern with a speed moves under constant
    // parameters.
    seeded = true,
    groups = PICK_GROUPS,
)]
pub struct PickPoints {
    /// What gives each point its number.
    #[choice(label = "Pattern", options = *PatternKind::OPTIONS, default = 1)]
    pub pattern: u32,

    /// The lowest number a kept point may have, per cent.
    #[slider(
        min = 0.0,
        max = 100.0,
        default = 0.0,
        hard_min = 0.0,
        hard_max = 100.0,
        unit = Percent
    )]
    pub from: f32,

    /// The highest number a kept point may have, per cent. On a Random
    /// pattern this is the share kept.
    #[slider(
        min = 0.0,
        max = 100.0,
        default = 100.0,
        hard_min = 0.0,
        hard_max = 100.0,
        unit = Percent
    )]
    pub to: f32,

    /// Which dice, and which noise.
    #[seed]
    pub seed: u32,

    /// How far apart two points have to be before the noise gives them
    /// different numbers, px@comp.
    #[slider(
        label = "Noise scale",
        min = 10.0,
        max = 1000.0,
        default = 200.0,
        hard_min = 1.0,
        unit = Px
    )]
    pub noise_scale: f32,

    /// How fast the noise evolves, per second. 0 holds it still.
    #[slider(
        label = "Noise speed",
        min = 0.0,
        max = 5.0,
        default = 0.0,
        hard_min = 0.0,
        unit = Raw
    )]
    pub noise_speed: f32,

    /// Where Distance is measured from, px@comp.
    #[slider(label = "Centre X", min = 0.0, max = 3840.0, default = 960.0, unit = Px)]
    pub centre_x: f32,

    /// px@comp. See [`centre_x`](Self::centre_x).
    #[slider(label = "Centre Y", min = 0.0, max = 2160.0, default = 540.0, unit = Px)]
    pub centre_y: f32,

    /// The distance from Centre at which the pattern reads 1, px@comp.
    #[slider(
        label = "Radius",
        min = 0.0,
        max = 2000.0,
        default = 400.0,
        hard_min = 0.0,
        unit = Px
    )]
    pub radius: f32,

    /// Keep one point in this many, counted by `id` so a particle is in or out
    /// for its whole life. 1 keeps them all.
    #[counter(
        label = "Every nth",
        min = 1,
        max = 100,
        default = 1,
        hard_min = 1,
        hard_max = 1_000_000,
        unit = Raw
    )]
    pub every_nth: i32,

    /// Which of each run is the one kept.
    #[counter(
        label = "Offset",
        min = 0,
        max = 100,
        default = 0,
        hard_min = 0,
        hard_max = 1_000_000,
        unit = Raw
    )]
    pub nth_offset: i32,

    /// Keep what would have been dropped, and drop what would have been kept.
    #[toggle(label = "Invert", default = false)]
    pub invert: bool,

    /// How soft the disc a point is drawn as is, per cent.
    #[slider(
        min = 0.0,
        max = 100.0,
        default = 100.0,
        hard_min = 0.0,
        hard_max = 100.0,
        unit = Percent
    )]
    pub feather: f32,

    /// The Mix every effect ends with, per cent. At 0 the stream is still
    /// handed on and nothing is drawn.
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

impl PickPoints {
    /// The raster factor, since a stream off a wire arrives in px@comp.
    pub const DERIVED_PX_SCALE: ParamId = ParamId::new("derived.px_scale");

    /// The pattern's own rows.
    #[must_use]
    pub fn pattern(self) -> Pattern {
        Pattern {
            kind: PatternKind::from_code(self.pattern),
            seed: self.seed,
            noise_scale: self.noise_scale,
            noise_speed: self.noise_speed,
            centre: [self.centre_x, self.centre_y],
            radius: self.radius,
        }
    }

    /// The wired stream with only the kept points in it, still in `id` order.
    #[must_use]
    pub fn apply(self, in_stream: &PointsStream, t: f64) -> PointsStream {
        let pattern = self.pattern();
        let (from, to) = (self.from / 100.0, self.to / 100.0);
        let nth = u64::from(self.every_nth.max(1).unsigned_abs());
        let offset = u64::from(self.nth_offset.max(0).unsigned_abs());
        let mut out = in_stream.clone();
        out.retain(|i| {
            let id = in_stream.id.get(i).copied().unwrap_or(0);
            let v = pattern.value(in_stream, i, t);
            let kept = v >= from && v <= to && id.wrapping_add(offset) % nth == 0;
            kept != self.invert
        });
        out
    }
}

/// Pick points' behaviour, the same shape as Vary points'.
pub struct PickPointsDef;

impl EffectDef for PickPointsDef {
    fn schema(&self) -> &'static EffectSchema {
        &<PickPoints as EffectMetadata>::SCHEMA
    }

    /// A stream in and a stream out, beside the picture.
    fn signature(&self) -> Signature {
        Signature::Image {
            inputs: POINTS_IN,
            extra: POINTS_OUT,
        }
    }

    fn resolve_derived(&self, cx: &ResolveCx<'_>, push: &mut dyn FnMut(ParamId, Value)) {
        push(PickPoints::DERIVED_PX_SCALE, Value::Float(cx.px_scale));
    }

    fn modify_points(&self, p: Params<'_>, input: &PointsStream, t: f64) -> Option<PointsStream> {
        Some(PickPoints::read(p).apply(input, t))
    }
}

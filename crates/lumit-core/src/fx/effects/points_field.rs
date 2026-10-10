//! Points field draws a picture of where a stream's points are.
//!
//! Every pixel finds the point nearest to it on the frame, and Output says
//! what it draws from that point: how close it is, which way it lies, its
//! colour, or the number it carries. Other effects read the picture through
//! their Matte row or a layer row, and so does the Image pattern of Vary
//! points and Pick points.
//!
//! Direction is written the way Displacement map reads a map: red steers the
//! sideways push and green the up-and-down one, and mid-grey is no push.
//! Nothing wired draws nothing.

use crate::fx::points::{self, PointsStream};
use crate::fx::{
    EffectDef, EffectMetadata, EffectSchema, EnabledCond, EnabledWhen, ParamId, Params, Port,
    PortType, ResolveCx, ShortText, Signature, Value,
};
use lumit_fx_macros::Effect;

/// The wire-only data input.
pub const POINTS_PORT: &str = "points";

/// What this effect takes in. Nearness is judged on the frame, so not `three_d`.
const POINTS_IN: &[Port] = &[Port::new(POINTS_PORT, "Points", PortType::Points)];

/// Invert only changes Distance, and Number range only divides Number.
pub const FIELD_ENABLED_WHEN: &[EnabledWhen] = &[
    EnabledWhen {
        param: "invert",
        on: "output",
        cond: EnabledCond::ChoiceIs(0),
    },
    EnabledWhen {
        param: "number_range",
        on: "output",
        cond: EnabledCond::ChoiceIs(3),
    },
    EnabledWhen {
        param: "number_name",
        on: "output",
        cond: EnabledCond::ChoiceIs(3),
    },
    EnabledWhen {
        param: "colour_name",
        on: "output",
        cond: EnabledCond::ChoiceIs(2),
    },
];

/// What each pixel draws from its nearest point.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum FieldOutput {
    /// Grey: 1 at a point, falling to 0 at Radius away.
    #[default]
    Distance,
    /// Which way the point lies, as a displacement map.
    Direction,
    /// The point's own colour.
    Colour,
    /// The number the point carries, over Number range, as a grey.
    Number,
}

impl FieldOutput {
    /// The Choice option labels, in code order. A Choice is stored as its
    /// index, so a new output goes on the end.
    pub const OPTIONS: &'static [&'static str] = &["Distance", "Direction", "Colour", "Number"];

    /// The output for a stored Choice index. Anything unknown is Distance.
    #[must_use]
    pub const fn from_code(code: u32) -> Self {
        match code {
            1 => FieldOutput::Direction,
            2 => FieldOutput::Colour,
            3 => FieldOutput::Number,
            _ => FieldOutput::Distance,
        }
    }
}

/// One point as the field reads it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Seed {
    /// Where the point is seen on the frame.
    pub at: [f32; 2],
    /// Premultiplied scene-linear RGBA.
    pub colour: [f32; 4],
    /// The number the point carries.
    pub number: f32,
}

/// Points field's controls.
#[derive(Debug, Clone, Copy, PartialEq, Effect)]
#[effect(
    match_name = "points_field",
    label = "Points field",
    version = 1,
    category = Generate,
    cost = Moderate,
    // A point anywhere decides pixels everywhere.
    roi = FullFrame,
    premultiplied = true,
    // Nothing here changes over time by itself. The stream may, and that is
    // its producer's declaration.
    seeded = false,
    enabled_when = FIELD_ENABLED_WHEN,
)]
pub struct PointsField {
    /// What each pixel draws from its nearest point.
    #[choice(label = "Output", options = *FieldOutput::OPTIONS, default = 0)]
    pub output: u32,

    /// How far from a point Distance takes to fall to 0, px@comp measured on
    /// the frame.
    #[slider(
        label = "Radius",
        min = 0.0,
        max = 1000.0,
        default = 100.0,
        hard_min = 0.0,
        unit = Px
    )]
    pub radius: f32,

    /// Leave Colour and Number transparent further than Radius from every
    /// point. Off fills the frame, each point owning the pixels nearest it.
    #[toggle(label = "Limit to radius", default = false)]
    pub limit: bool,

    /// Turn Distance over: 0 at a point, rising to 1 at Radius away.
    #[toggle(label = "Invert", default = false)]
    pub invert: bool,

    /// The number that draws as white when Output is Number.
    #[slider(
        label = "Number range",
        min = 1.0,
        max = 1000.0,
        default = 100.0,
        hard_min = 0.001,
        unit = Raw
    )]
    pub number_range: f32,

    /// Which of a point's numbers Number draws: a name written above, or
    /// one of the `@` names. Empty is the number it carries.
    #[text(label = "Name", default = "")]
    pub number_name: ShortText,

    /// The name Colour draws, where the points carry a colour under one.
    /// A number or an offset under the name draws as a grey. Empty is the
    /// point's own colour.
    #[text(label = "Colour name", default = "")]
    pub colour_name: ShortText,

    /// The family's budget row: the most points the field is made from. A
    /// longer stream is trimmed to its newest.
    #[counter(
        label = "Max points",
        min = 1,
        max = 200_000,
        default = 2_000,
        hard_min = 1,
        hard_max = points::CAP_HARD,
        unit = Raw
    )]
    pub max_points: i32,

    /// The Mix every effect ends with, per cent.
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

impl PointsField {
    /// The raster factor, so a px@comp stream off a wire can be put into the
    /// pixels the frame is drawn at.
    pub const DERIVED_PX_SCALE: ParamId = ParamId::new("derived.px_scale");

    /// This instance's raster factor, read back out of a resolved bag.
    #[must_use]
    pub fn px_scale_of(p: Params<'_>) -> f32 {
        p.float(Self::DERIVED_PX_SCALE, 1.0)
    }

    /// The points the field is made from: the newest Max points of the
    /// stream, each where it is seen on the frame. A point that is not a
    /// number is left out.
    #[must_use]
    pub fn seeds(self, stream: &PointsStream) -> Vec<Seed> {
        let mut s = stream.clone();
        s.keep_newest(self.max_points.clamp(0, points::CAP_HARD as i32) as usize);
        let number = s.column(self.number_name.as_str());
        let named = s.column(self.colour_name.as_str());
        (0..s.len())
            .filter_map(|i| {
                let at = s.projected(i);
                let colour = match (self.colour_name.is_empty(), s.whole(named, i)) {
                    (true, _) => s.colour.get(i).copied().unwrap_or([0.0; 4]),
                    (false, Some((colour, 4))) => colour,
                    // Anything narrower is one number, drawn as a grey.
                    (false, _) => s.read(named, i).map_or([0.0; 4], |v| [v, v, v, 1.0]),
                };
                (at[0].is_finite() && at[1].is_finite()).then(|| Seed {
                    at,
                    colour,
                    number: s.read(number, i).unwrap_or(0.0),
                })
            })
            .collect()
    }

    /// What the pixel centred on `p` draws, given the seed nearest to it.
    /// The kernel's `shade` does the same sums in the same order.
    #[must_use]
    pub fn shade(self, p: [f32; 2], seed: &Seed) -> [f32; 4] {
        let (dx, dy) = (seed.at[0] - p[0], seed.at[1] - p[1]);
        let d = (dx * dx + dy * dy).sqrt();
        let radius = self.radius.max(0.0);
        let grey = |v: f32| [v, v, v, 1.0];
        match FieldOutput::from_code(self.output) {
            FieldOutput::Distance => {
                let v = if radius > 0.0 {
                    (1.0 - d / radius).clamp(0.0, 1.0)
                } else {
                    0.0
                };
                grey(if self.invert { 1.0 - v } else { v })
            }
            // Displacement map's own reading: 0.5 is no push, 1 a full push
            // one way and 0 the other. A pixel on its point pushes nowhere.
            FieldOutput::Direction => {
                let (ux, uy) = if d > 0.0 {
                    (dx / d, dy / d)
                } else {
                    (0.0, 0.0)
                };
                [0.5 + 0.5 * ux, 0.5 + 0.5 * uy, 0.5, 1.0]
            }
            _ if self.limit && d > radius => [0.0; 4],
            FieldOutput::Colour => seed.colour,
            FieldOutput::Number => grey(seed.number / self.number_range.max(1e-6)),
        }
    }

    /// The CPU reference: the field mixed over `rgba`, a `w` by `h` picture
    /// of premultiplied RGBA floats. `stream` and Radius are in its pixels.
    ///
    /// Every pixel asks every seed, and the earlier seed wins a tie.
    /// (ponytail: brute force, pixels times points. It is the oracle, so it
    /// only ever sees a test-sized frame. The card's pass is the fast one.)
    pub fn draw(self, rgba: &mut [f32], w: u32, h: u32, stream: &PointsStream) {
        let seeds = self.seeds(stream);
        let mix = (self.mix / 100.0).clamp(0.0, 1.0);
        if seeds.is_empty() || mix <= 0.0 {
            return;
        }
        let width = w as usize;
        for (i, px) in rgba
            .chunks_exact_mut(4)
            .take(width * h as usize)
            .enumerate()
        {
            let p = [(i % width) as f32 + 0.5, (i / width) as f32 + 0.5];
            let d2 = |s: &Seed| (s.at[0] - p[0]).powi(2) + (s.at[1] - p[1]).powi(2);
            let Some(nearest) = seeds.iter().min_by(|a, b| d2(a).total_cmp(&d2(b))) else {
                continue;
            };
            for (c, f) in px.iter_mut().zip(self.shade(p, nearest)) {
                *c += (f - *c) * mix;
            }
        }
    }
}

/// Points field's behaviour.
///
/// No CPU reference through the trait, as with every points effect: the
/// stream rides beside the op, not in the bag. The oracle is
/// [`PointsField::draw`].
pub struct PointsFieldDef;

impl EffectDef for PointsFieldDef {
    fn schema(&self) -> &'static EffectSchema {
        &<PointsField as EffectMetadata>::SCHEMA
    }

    /// A picture in, a picture out, and a stream in beside it.
    fn signature(&self) -> Signature {
        Signature::Image {
            inputs: POINTS_IN,
            extra: &[],
        }
    }

    /// The raster factor, so a px@comp stream reaches the pixels this frame
    /// is drawn at.
    fn resolve_derived(&self, cx: &ResolveCx<'_>, push: &mut dyn FnMut(ParamId, Value)) {
        push(PointsField::DERIVED_PX_SCALE, Value::Float(cx.px_scale));
    }
}

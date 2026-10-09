//! Vary points makes a stream's points differ from one another.
//!
//! Each point gets a number from 0 to 1 from a pattern: its place in the
//! stream, its own dice, a noise at its position, its distance from a centre,
//! or its age. That number sets how much of each change the point takes.
//! The changed stream goes out on the Points socket, and is drawn as discs
//! unless Mix is 0. Nothing wired draws nothing.

use crate::fx::cpu;
use crate::fx::noise::value3;
use crate::fx::points::{self, PointsStream};
use crate::fx::{
    CurvePoints, EffectDef, EffectMetadata, EffectSchema, ParamGroup, ParamId, Params, Port,
    PortType, ResolveCx, Signature, Value,
};
use lumit_fx_macros::Effect;

/// The wire-only data input.
pub const POINTS_PORT: &str = "points";

/// What a modifier takes in.
pub(crate) const POINTS_IN: &[Port] = &[Port::new(POINTS_PORT, "Points", PortType::Points)];

/// What it hands on, the same port every producer declares.
pub(crate) const POINTS_OUT: &[Port] = &[Port::new("points", "Points", PortType::Points)];

/// The dice a Random pattern rolls and the lattice a Noise one reads, kept off
/// the numbers the producers use for their own jitter.
const RANDOM_ATTR: u32 = 32;
const NOISE_CHANNEL: u32 = 80;

/// What gives each point its number.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PatternKind {
    /// Its place in the stream: 0 for the first point, 1 for the last.
    Index,
    /// A roll of its own dice, from its `id`, so the same number every frame.
    #[default]
    Random,
    /// A smooth noise read at its position.
    Noise,
    /// How far it is from Centre, as a share of Radius.
    Distance,
    /// How far through its life it is.
    Age,
}

impl PatternKind {
    /// The Choice option labels, in code order. A Choice is stored as its
    /// index, so a new pattern goes on the end.
    pub const OPTIONS: &'static [&'static str] = &["Index", "Random", "Noise", "Distance", "Age"];

    /// The pattern for a stored Choice index. Anything unknown is Random.
    #[must_use]
    pub const fn from_code(code: u32) -> Self {
        match code {
            0 => PatternKind::Index,
            2 => PatternKind::Noise,
            3 => PatternKind::Distance,
            4 => PatternKind::Age,
            _ => PatternKind::Random,
        }
    }
}

/// A pattern and the rows it reads. Shared by Vary points and Pick points, so
/// both number a stream the same way.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Pattern {
    pub kind: PatternKind,
    pub seed: u32,
    /// Spatial wavelength of the noise, in the stream's own units.
    pub noise_scale: f32,
    /// How fast the noise evolves, per second.
    pub noise_speed: f32,
    /// Where Distance is measured from, in the stream's own units.
    pub centre: [f32; 2],
    /// The distance that reads as 1.
    pub radius: f32,
}

impl Pattern {
    /// Point `i`'s number, `0..=1`, at layer time `t`.
    ///
    /// Everything but Index is keyed on the point itself, so a neighbour
    /// coming or going doesn't reshuffle it.
    #[must_use]
    pub fn value(&self, s: &PointsStream, i: usize, t: f64) -> f32 {
        let p = s.position.get(i).copied().unwrap_or([0.0; 3]);
        let v = match self.kind {
            PatternKind::Index => {
                // A single point reads as the first.
                let last = s.len().saturating_sub(1).max(1) as f32;
                i as f32 / last
            }
            PatternKind::Random => {
                points::draw(self.seed, s.id.get(i).copied().unwrap_or(0), RANDOM_ATTR)
            }
            PatternKind::Noise => {
                let k = 1.0 / self.noise_scale.max(1e-3);
                let z = p[2] * k + (t as f32) * self.noise_speed;
                value3(self.seed, NOISE_CHANNEL, p[0] * k, p[1] * k, z, 0) * 0.5 + 0.5
            }
            PatternKind::Distance => {
                (p[0] - self.centre[0]).hypot(p[1] - self.centre[1]) / self.radius.max(1e-3)
            }
            PatternKind::Age => {
                let life = s.life.get(i).copied().unwrap_or(1.0);
                s.age.get(i).copied().unwrap_or(0.0) / life.max(1e-6)
            }
        };
        // A NaN reads as 0 so it never reaches a position.
        if v.is_finite() {
            v.clamp(0.0, 1.0)
        } else {
            0.0
        }
    }
}

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

/// The pattern, the rows only some patterns read, and what it changes.
pub const VARY_GROUPS: &[ParamGroup] = &[
    group("Pattern", &["pattern", "curve"], None),
    group("", &["seed"], Some(("pattern", &[1, 2]))),
    group("", &["noise_scale", "noise_speed"], Some(("pattern", &[2]))),
    group(
        "",
        &["centre_x", "centre_y", "radius"],
        Some(("pattern", &[3])),
    ),
    group(
        "Change",
        &[
            "size", "opacity", "rotation", "colour", "offset_x", "offset_y", "offset_z",
        ],
        None,
    ),
    group("Point", &["feather"], None),
];

/// Vary points' controls.
#[derive(Debug, Clone, Copy, PartialEq, Effect)]
#[effect(
    match_name = "vary_points",
    label = "Vary points",
    version = 1,
    category = Generate,
    cost = Moderate,
    // A point may be anywhere, and may be moved anywhere.
    roi = FullFrame,
    premultiplied = true,
    // Seeded, since a Noise pattern with a speed moves under constant
    // parameters.
    seeded = true,
    groups = VARY_GROUPS,
)]
pub struct VaryPoints {
    /// What gives each point its number.
    #[choice(label = "Pattern", options = *PatternKind::OPTIONS, default = 1)]
    pub pattern: u32,

    /// Reshapes the pattern's number before the changes read it. The diagonal
    /// changes nothing.
    #[curve(label = "Curve", default = [[0.0, 0.0], [1.0, 1.0]])]
    pub curve: CurvePoints,

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

    /// The size a point takes where the pattern reads 1, as a share of its
    /// own, per cent. Where it reads 0 the point keeps its size.
    #[slider(min = 0.0, max = 400.0, default = 100.0, hard_min = 0.0, unit = Percent)]
    pub size: f32,

    /// The opacity a point takes where the pattern reads 1, as a share of its
    /// own, per cent.
    #[slider(
        min = 0.0,
        max = 100.0,
        default = 100.0,
        hard_min = 0.0,
        hard_max = 100.0,
        unit = Percent
    )]
    pub opacity: f32,

    /// The turn added where the pattern reads 1, degrees.
    #[dial(label = "Rotation", default = 0.0)]
    pub rotation: f32,

    /// The colour a point is multiplied by where the pattern reads 1. White
    /// changes nothing.
    #[colour(default = [1.0, 1.0, 1.0, 1.0], max = 4.0)]
    pub colour: [f32; 4],

    /// How far a point is moved across where the pattern reads 1, px@comp.
    #[slider(label = "Offset x", min = -1000.0, max = 1000.0, default = 0.0, unit = Px)]
    pub offset_x: f32,

    /// px@comp, down. See [`offset_x`](Self::offset_x).
    #[slider(label = "Offset y", min = -1000.0, max = 1000.0, default = 0.0, unit = Px)]
    pub offset_y: f32,

    /// px@comp, through the layer's plane. See [`offset_x`](Self::offset_x).
    #[slider(label = "Offset z", min = -1000.0, max = 1000.0, default = 0.0, unit = Px)]
    pub offset_z: f32,

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

impl VaryPoints {
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

    /// The wired stream with the changes made. The draw and every reader
    /// downstream go through this one function. `in_stream` and the bag must
    /// be in the same units, and the answer is in those units too.
    #[must_use]
    pub fn apply(self, in_stream: &PointsStream, t: f64) -> PointsStream {
        let mut out = in_stream.clone();
        let pattern = self.pattern();
        let table = cpu::curve_table(&self.curve);
        let size = (self.size / 100.0).max(0.0);
        let opacity = (self.opacity / 100.0).clamp(0.0, 1.0);
        let turn = self.rotation.to_radians();
        // Premultiplied, as every colour in the working space is.
        let a = self.colour[3];
        let tint = [
            self.colour[0] * a,
            self.colour[1] * a,
            self.colour[2] * a,
            a,
        ];
        for i in 0..out.len() {
            let v = cpu::curve_at(pattern.value(in_stream, i, t), &table).clamp(0.0, 1.0);
            let towards = |to: f32| 1.0 + (to - 1.0) * v;
            if let Some(s) = out.size.get_mut(i) {
                *s *= towards(size);
            }
            if let Some(r) = out.rotation.get_mut(i) {
                *r += turn * v;
            }
            if let Some(c) = out.colour.get_mut(i) {
                let fade = towards(opacity);
                for (ch, to) in c.iter_mut().zip(tint) {
                    *ch *= towards(to) * fade;
                }
            }
            if let Some(p) = out.position.get_mut(i) {
                p[0] += self.offset_x * v;
                p[1] += self.offset_y * v;
                p[2] += self.offset_z * v;
            }
        }
        out
    }
}

/// Vary points' behaviour. No CPU reference through the trait, as with the
/// rest of the family: the stream and the camera aren't in the bag.
pub struct VaryPointsDef;

impl EffectDef for VaryPointsDef {
    fn schema(&self) -> &'static EffectSchema {
        &<VaryPoints as EffectMetadata>::SCHEMA
    }

    /// A stream in and a stream out, beside the picture.
    fn signature(&self) -> Signature {
        Signature::Image {
            inputs: POINTS_IN,
            extra: POINTS_OUT,
        }
    }

    fn resolve_derived(&self, cx: &ResolveCx<'_>, push: &mut dyn FnMut(ParamId, Value)) {
        push(VaryPoints::DERIVED_PX_SCALE, Value::Float(cx.px_scale));
    }

    fn modify_points(&self, p: Params<'_>, input: &PointsStream, t: f64) -> Option<PointsStream> {
        Some(VaryPoints::read(p).apply(input, t))
    }
}

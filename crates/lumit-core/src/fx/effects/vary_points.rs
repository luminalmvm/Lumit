//! Vary points makes a stream's points differ from one another.
//!
//! Each point gets a number from 0 to 1 from a pattern: its place in the
//! stream, its own dice, a noise at its position, its distance from a centre,
//! its age, where it is across or down the frame, how bright a picture is
//! under it, whether it is picked, the number it carries, or which stripe,
//! square or wave of a drawn pattern it sits in. That number sets how much of
//! each change the point takes.
//! The changed stream goes out on the Points socket, and is drawn as discs
//! unless Mix is 0. Nothing wired draws nothing.

use crate::fx::cpu;
use crate::fx::noise::value3;
use crate::fx::points::{self, PointsStream};
use crate::fx::{
    CurvePoints, EffectDef, EffectMetadata, EffectSchema, EnabledCond, EnabledWhen, ParamGroup,
    ParamId, Params, Port, PortType, ResolveCx, Signature, Value,
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
    /// Always 1, so every point takes the whole of each change.
    Constant,
    /// Where it is from left to right: 0 at Centre X less Radius, 1 at Centre
    /// X plus Radius.
    Across,
    /// The same from top to bottom, about Centre Y.
    Down,
    /// How bright the picture under it is.
    Image,
    /// 1 if it is picked, 0 if not.
    Picked,
    /// The number it carries, held to 0 to 1.
    Number,
    /// Bands across the frame: 1 inside a band, 0 between.
    Stripes,
    /// A chequerboard: 1 and 0 in alternate squares.
    Checker,
    /// A wave that repeats over space and can travel over time.
    Waves,
}

impl PatternKind {
    /// The Choice option labels, in code order. A Choice is stored as its
    /// index, so a new pattern goes on the end.
    pub const OPTIONS: &'static [&'static str] = &[
        "Index", "Random", "Noise", "Distance", "Age", "Constant", "Across", "Down", "Image",
        "Picked", "Number", "Stripes", "Checker", "Waves",
    ];

    /// The pattern for a stored Choice index. Anything unknown is Random.
    #[must_use]
    pub const fn from_code(code: u32) -> Self {
        match code {
            0 => PatternKind::Index,
            2 => PatternKind::Noise,
            3 => PatternKind::Distance,
            4 => PatternKind::Age,
            5 => PatternKind::Constant,
            6 => PatternKind::Across,
            7 => PatternKind::Down,
            8 => PatternKind::Image,
            9 => PatternKind::Picked,
            10 => PatternKind::Number,
            11 => PatternKind::Stripes,
            12 => PatternKind::Checker,
            13 => PatternKind::Waves,
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
    /// How fast the noise evolves, and how many waves pass, per second.
    pub noise_speed: f32,
    /// Where Distance, Across and Down are measured from, and where the
    /// stripes, squares and waves start, in the stream's own units.
    pub centre: [f32; 2],
    /// The distance that reads as 1.
    pub radius: f32,
    /// The number that reads as 1 under the Number pattern.
    pub number_range: f32,
    /// How far it is from one stripe, square or wave to the next, in the
    /// stream's own units.
    pub spacing: f32,
    /// How far the stripes and squares are turned about Centre, and the way
    /// the waves travel, degrees.
    pub angle: f32,
    /// The share of each Spacing a stripe fills, 0 to 1.
    pub width: f32,
    /// The shape of a wave: sine, triangle, square or sawtooth.
    pub wave: u32,
    /// The waves run out from Centre as rings.
    pub rings: bool,
}

impl Pattern {
    /// Point `i`'s number, `0..=1`, at layer time `t`.
    ///
    /// Everything but Index is keyed on the point itself, so a neighbour
    /// coming or going doesn't reshuffle it. `sampled` is the picture's
    /// colour under each point, which only Image reads. `axis` says which
    /// roll a Random or Noise pattern gives: 0 is the usual one, and the
    /// others let one effect push a point a different way along each axis.
    #[must_use]
    pub fn value(
        &self,
        s: &PointsStream,
        i: usize,
        t: f64,
        sampled: Option<&[[f32; 4]]>,
        axis: u32,
    ) -> f32 {
        let p = s.position.get(i).copied().unwrap_or([0.0; 3]);
        // 0 at the centre less the radius, 1 at the centre plus it.
        let along = |at: f32, centre: f32| (at - centre) / (2.0 * self.radius.max(1e-3)) + 0.5;
        // Where it is from Centre in the pattern's own turned frame, along
        // Angle and then across it, counted in Spacings.
        let turned = || {
            let (sin, cos) = self.angle.to_radians().sin_cos();
            let (x, y) = (p[0] - self.centre[0], p[1] - self.centre[1]);
            let k = 1.0 / self.spacing.max(1e-3);
            [(x * cos + y * sin) * k, (y * cos - x * sin) * k]
        };
        let on = |yes: bool| if yes { 1.0 } else { 0.0 };
        let v = match self.kind {
            PatternKind::Index => {
                // A single point reads as the first.
                let last = s.len().saturating_sub(1).max(1) as f32;
                i as f32 / last
            }
            PatternKind::Random => {
                let id = s.id.get(i).copied().unwrap_or(0);
                points::draw(self.seed, id, RANDOM_ATTR + axis)
            }
            PatternKind::Noise => {
                let k = 1.0 / self.noise_scale.max(1e-3);
                let z = p[2] * k + (t as f32) * self.noise_speed;
                value3(self.seed, NOISE_CHANNEL + axis, p[0] * k, p[1] * k, z, 0) * 0.5 + 0.5
            }
            PatternKind::Distance => {
                (p[0] - self.centre[0]).hypot(p[1] - self.centre[1]) / self.radius.max(1e-3)
            }
            PatternKind::Age => {
                let life = s.life.get(i).copied().unwrap_or(1.0);
                s.age.get(i).copied().unwrap_or(0.0) / life.max(1e-6)
            }
            PatternKind::Constant => 1.0,
            PatternKind::Across => along(p[0], self.centre[0]),
            PatternKind::Down => along(p[1], self.centre[1]),
            PatternKind::Image => {
                let c = sampled.and_then(|c| c.get(i)).copied().unwrap_or([0.0; 4]);
                // The picture is premultiplied, so the coverage is divided
                // back out first: a half-covered white pixel is white, not
                // grey. No coverage is no light.
                if c[3] > 0.0 {
                    (c[0] * cpu::LUMA[0] + c[1] * cpu::LUMA[1] + c[2] * cpu::LUMA[2]) / c[3]
                } else {
                    0.0
                }
            }
            PatternKind::Picked => {
                if s.picked(i) {
                    1.0
                } else {
                    0.0
                }
            }
            PatternKind::Number => s.index_of(i) / self.number_range.max(1e-3),
            // At no Angle the stripes lie across, one below another.
            PatternKind::Stripes => on(turned()[1].rem_euclid(1.0) < self.width),
            PatternKind::Checker => {
                let [u, v] = turned();
                on((u.floor() + v.floor()).rem_euclid(2.0) < 1.0)
            }
            PatternKind::Waves => {
                let far = if self.rings {
                    (p[0] - self.centre[0]).hypot(p[1] - self.centre[1]) / self.spacing.max(1e-3)
                } else {
                    turned()[0]
                };
                // Less the time, so the waves move the way Angle points and
                // the rings move outwards.
                let x = (far - t as f32 * self.noise_speed).rem_euclid(1.0);
                match self.wave {
                    1 => 1.0 - (2.0 * x - 1.0).abs(),
                    2 => on(x < 0.5),
                    3 => x,
                    _ => 0.5 + 0.5 * (x * std::f32::consts::TAU).sin(),
                }
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
    group("", &["noise_scale"], Some(("pattern", &[2]))),
    group("", &["noise_speed"], Some(("pattern", &[2, 13]))),
    group(
        "",
        &["centre_x", "centre_y"],
        Some(("pattern", &[3, 6, 7, 11, 12, 13])),
    ),
    group("", &["radius"], Some(("pattern", &[3, 6, 7]))),
    group("", &["number_range"], Some(("pattern", &[10]))),
    group("", &["spacing", "angle"], Some(("pattern", &[11, 12, 13]))),
    group("", &["band_width"], Some(("pattern", &[11]))),
    group("", &["wave", "rings"], Some(("pattern", &[13]))),
    // Always shown, since the Image pattern and Colour from image both read
    // the layer and a group can only follow one row.
    group("Image", &["image_layer", "image_colour"], None),
    group(
        "Change",
        &[
            "apply_to",
            "size",
            "opacity",
            "stretch_x",
            "stretch_y",
            "colour_start",
            "use_colour_mid",
            "colour_mid",
            "colour",
            "both_ways",
            "rotation",
            "offset_x",
            "offset_y",
            "offset_z",
            "offset_forward",
            "offset_side",
        ],
        None,
    ),
    group("Number", &["set_number", "number_from", "number_to"], None),
    group("Point", &["feather"], None),
];

/// Number from and Number to do nothing until Set number is on, nor Middle
/// colour until its own switch is.
pub const VARY_ENABLED_WHEN: &[EnabledWhen] = &[
    EnabledWhen {
        param: "colour_mid",
        on: "use_colour_mid",
        cond: EnabledCond::BoolIs(true),
    },
    EnabledWhen {
        param: "number_from",
        on: "set_number",
        cond: EnabledCond::BoolIs(true),
    },
    EnabledWhen {
        param: "number_to",
        on: "set_number",
        cond: EnabledCond::BoolIs(true),
    },
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
    enabled_when = VARY_ENABLED_WHEN,
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

    /// How fast the noise evolves, and how many waves pass a point, per
    /// second. 0 holds them still.
    #[slider(
        label = "Noise speed",
        min = 0.0,
        max = 5.0,
        default = 0.0,
        hard_min = 0.0,
        unit = Raw
    )]
    pub noise_speed: f32,

    /// Where Distance, Across and Down are measured from, and where the
    /// stripes, squares and waves start, px@comp. Keyframe it to slide them.
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

    /// The number that reads as 1 under the Number pattern. A point's number
    /// is divided by it, so with word numbers 0 to 4 a range of 4 spreads the
    /// words from 0 to 1.
    #[slider(
        label = "Number range",
        min = 1.0,
        max = 100.0,
        default = 1.0,
        hard_min = 0.001,
        unit = Raw
    )]
    pub number_range: f32,

    /// How far it is from one stripe, square or wave to the next, px@comp.
    #[slider(
        label = "Spacing",
        min = 1.0,
        max = 1000.0,
        default = 100.0,
        hard_min = 0.0,
        unit = Px
    )]
    pub spacing: f32,

    /// How far the stripes and squares are turned about Centre, and the way
    /// the waves travel, degrees. 0 lays the stripes across and sends the
    /// waves to the right. Rings take no notice of it.
    #[dial(label = "Angle", default = 0.0)]
    pub angle: f32,

    /// How much of each Spacing a stripe fills, per cent. The rest is the gap.
    #[slider(
        label = "Width",
        min = 0.0,
        max = 100.0,
        default = 50.0,
        hard_min = 0.0,
        hard_max = 100.0,
        unit = Percent
    )]
    pub band_width: f32,

    /// The shape of each wave.
    #[choice(
        label = "Wave",
        options = ["Sine", "Triangle", "Square", "Sawtooth"],
        default = 0
    )]
    pub wave: u32,

    /// The waves run out from Centre as rings, not across as bands.
    #[toggle(label = "Rings", default = false)]
    pub rings: bool,

    /// The layer whose picture the Image pattern and Colour from image read.
    /// Unset reads this effect's own input picture.
    #[layer(label = "Image layer")]
    pub image_layer: bool,

    /// The point takes the picture's colour under it. Its own alpha still
    /// multiplies it.
    #[toggle(label = "Colour from image", default = false)]
    pub image_colour: bool,

    /// Which points are changed. The rest pass through as they came.
    #[choice(
        label = "Apply to",
        options = ["All points", "Picked", "Not picked"],
        default = 0
    )]
    pub apply_to: u32,

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

    /// How far a point is stretched across where the pattern reads 1, in its
    /// own turned frame, per cent. 100 leaves it alone.
    #[slider(
        label = "Stretch x",
        min = 0.0,
        max = 400.0,
        default = 100.0,
        hard_min = 0.0,
        unit = Percent
    )]
    pub stretch_x: f32,

    /// Per cent, down. See [`stretch_x`](Self::stretch_x).
    #[slider(
        label = "Stretch y",
        min = 0.0,
        max = 400.0,
        default = 100.0,
        hard_min = 0.0,
        unit = Percent
    )]
    pub stretch_y: f32,

    /// The colour a point is multiplied by where the pattern reads 0. White
    /// changes nothing.
    #[colour(label = "Start colour", default = [1.0, 1.0, 1.0, 1.0], max = 4.0)]
    pub colour_start: [f32; 4],

    /// The colours run from Start colour through Middle colour to Colour,
    /// not straight from one to the other.
    #[toggle(label = "Use middle colour", default = false)]
    pub use_colour_mid: bool,

    /// The colour a point is multiplied by where the pattern reads a half.
    #[colour(label = "Middle colour", default = [1.0, 1.0, 1.0, 1.0], max = 4.0)]
    pub colour_mid: [f32; 4],

    /// The colour a point is multiplied by where the pattern reads 1. In
    /// between it is a mix of the two it lies between.
    #[colour(default = [1.0, 1.0, 1.0, 1.0], max = 4.0)]
    pub colour: [f32; 4],

    /// Rotation and the Offsets read the pattern as -1 to 1 instead of 0 to
    /// 1, and a Random or Noise pattern rolls again for each Offset, so one
    /// effect scatters points every way at once.
    #[toggle(label = "Both ways", default = false)]
    pub both_ways: bool,

    /// The turn added where the pattern reads 1, degrees.
    #[dial(label = "Rotation", default = 0.0)]
    pub rotation: f32,

    /// How far a point is moved across where the pattern reads 1, px@comp.
    #[slider(label = "Offset x", min = -1000.0, max = 1000.0, default = 0.0, unit = Px)]
    pub offset_x: f32,

    /// px@comp, down. See [`offset_x`](Self::offset_x).
    #[slider(label = "Offset y", min = -1000.0, max = 1000.0, default = 0.0, unit = Px)]
    pub offset_y: f32,

    /// px@comp, through the layer's plane. See [`offset_x`](Self::offset_x).
    #[slider(label = "Offset z", min = -1000.0, max = 1000.0, default = 0.0, unit = Px)]
    pub offset_z: f32,

    /// How far a point is moved along the way it faces where the pattern
    /// reads 1, px@comp. The way it faces is its rotation, after the turn
    /// above.
    #[slider(
        label = "Offset forward",
        min = -1000.0,
        max = 1000.0,
        default = 0.0,
        unit = Px
    )]
    pub offset_forward: f32,

    /// px@comp, at right angles to the way it faces. See
    /// [`offset_forward`](Self::offset_forward).
    #[slider(
        label = "Offset sideways",
        min = -1000.0,
        max = 1000.0,
        default = 0.0,
        unit = Px
    )]
    pub offset_side: f32,

    /// Write the number each point carries, which effects below read to
    /// choose a variant, a time offset or an order.
    #[toggle(label = "Set number", default = false)]
    pub set_number: bool,

    /// The number a point gets where the pattern reads 0.
    #[slider(label = "Number from", min = 0.0, max = 100.0, default = 0.0, unit = Raw)]
    pub number_from: f32,

    /// The number a point gets where the pattern reads 1.
    #[slider(label = "Number to", min = 0.0, max = 100.0, default = 1.0, unit = Raw)]
    pub number_to: f32,

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
            number_range: self.number_range,
            spacing: self.spacing,
            angle: self.angle,
            width: self.band_width / 100.0,
            wave: self.wave,
            rings: self.rings,
        }
    }

    /// Whether these settings read the picture under the points.
    #[must_use]
    pub fn needs_picture(self) -> bool {
        self.image_colour || PatternKind::from_code(self.pattern) == PatternKind::Image
    }

    /// The wired stream with the changes made. The draw and every reader
    /// downstream go through this one function. `in_stream` and the bag must
    /// be in the same units, and the answer is in those units too. `sampled`
    /// is the picture's colour under each point, where there is a picture.
    #[must_use]
    pub fn apply(
        self,
        in_stream: &PointsStream,
        t: f64,
        sampled: Option<&[[f32; 4]]>,
    ) -> PointsStream {
        let mut out = in_stream.clone();
        let pattern = self.pattern();
        let table = cpu::curve_table(&self.curve);
        let size = (self.size / 100.0).max(0.0);
        let opacity = (self.opacity / 100.0).clamp(0.0, 1.0);
        let turn = self.rotation.to_radians();
        let stretch = [
            (self.stretch_x / 100.0).max(0.0),
            (self.stretch_y / 100.0).max(0.0),
        ];
        // Premultiplied, as every colour in the working space is.
        let premultiplied = |c: [f32; 4]| [c[0] * c[3], c[1] * c[3], c[2] * c[3], c[3]];
        let tint_from = premultiplied(self.colour_start);
        let tint_mid = premultiplied(self.colour_mid);
        let tint_to = premultiplied(self.colour);
        // An optional column is only filled in when it is written, so a
        // stream this effect leaves alone stays as it came.
        if stretch != [1.0; 2] {
            out.stretch_mut();
        }
        if self.set_number {
            out.index_mut();
        }
        // Under Both ways a Random or Noise pattern rolls again for each axis.
        let own_rolls =
            self.both_ways && matches!(pattern.kind, PatternKind::Random | PatternKind::Noise);
        for i in 0..out.len() {
            let picked = in_stream.picked(i);
            if (self.apply_to == 1 && !picked) || (self.apply_to == 2 && picked) {
                continue;
            }
            let shaped = |axis: u32| {
                let v = pattern.value(in_stream, i, t, sampled, axis);
                cpu::curve_at(v, &table).clamp(0.0, 1.0)
            };
            let v = shaped(0);
            let towards = |to: f32| 1.0 + (to - 1.0) * v;
            // How much of `amount` Rotation or an Offset takes: the pattern
            // as it is, or under Both ways from -1 to 1.
            let signed = |amount: f32, axis: u32| {
                if !self.both_ways {
                    return amount * v;
                }
                let own = own_rolls && axis > 0 && amount != 0.0;
                let v = if own { shaped(axis) } else { v };
                amount * (v * 2.0 - 1.0)
            };
            if let Some(s) = out.size.get_mut(i) {
                *s *= towards(size);
            }
            if let Some(s) = out.stretch.get_mut(i) {
                s[0] *= towards(stretch[0]);
                s[1] *= towards(stretch[1]);
            }
            if let Some(r) = out.rotation.get_mut(i) {
                *r += signed(turn, 0);
            }
            if let Some(c) = out.colour.get_mut(i) {
                let under = sampled.filter(|_| self.image_colour);
                if let Some(under) = under.and_then(|s| s.get(i)) {
                    let alpha = c[3];
                    *c = under.map(|ch| ch * alpha);
                }
                let fade = towards(opacity);
                // Which two colours it lies between, and how far from the
                // first to the second.
                let (from, to, v) = if !self.use_colour_mid {
                    (tint_from, tint_to, v)
                } else if v < 0.5 {
                    (tint_from, tint_mid, v * 2.0)
                } else {
                    (tint_mid, tint_to, v * 2.0 - 1.0)
                };
                for ((ch, from), to) in c.iter_mut().zip(from).zip(to) {
                    *ch *= (from + (to - from) * v) * fade;
                }
            }
            if self.set_number {
                if let Some(n) = out.index.get_mut(i) {
                    *n = self.number_from + (self.number_to - self.number_from) * v;
                }
            }
            let facing = out.rotation.get(i).copied().unwrap_or(0.0);
            if let Some(p) = out.position.get_mut(i) {
                p[0] += signed(self.offset_x, 0);
                p[1] += signed(self.offset_y, 1);
                p[2] += signed(self.offset_z, 2);
                let forward = signed(self.offset_forward, 3);
                let side = signed(self.offset_side, 4);
                if forward != 0.0 || side != 0.0 {
                    let (sin, cos) = facing.sin_cos();
                    p[0] += forward * cos - side * sin;
                    p[1] += forward * sin + side * cos;
                }
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

    /// `None` while it needs a picture and has none, which leaves the stream
    /// to be made during the render.
    fn modify_points(&self, p: Params<'_>, cx: &crate::fx::ModifyCx<'_>) -> Option<PointsStream> {
        let vary = VaryPoints::read(p);
        if vary.needs_picture() && cx.sampled.is_none() {
            return None;
        }
        Some(vary.apply(cx.input()?, cx.t, cx.sampled))
    }

    fn points_need_picture(&self, p: Params<'_>) -> bool {
        VaryPoints::read(p).needs_picture()
    }
}

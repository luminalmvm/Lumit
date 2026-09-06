//! Grid: a points stream on a regular lattice, and the discs that make
//! it visible.
//!
//! **In plain terms.** Particulate makes points that are *born* and drift;
//! Grid makes points that simply *are*. Rows, columns and — since the stream
//! has three axes — planes receding into depth, spaced by a distance
//! you type, with a jitter dial per axis so a lattice can be nudged off its
//! own perfection. There is no time in it at all: frame 500's grid and frame
//! 3's grid are the same arithmetic, and neither reads the other.
//!
//! It exists for the family the points stream is for — Clone to points, Connect
//! points, and the Points sample driver that is already wired — where "one
//! copy of this layer at every cell" and "a mesh over a lattice" are the two
//! things everybody asks for first. Beside the stream it draws its own points
//! as feathered discs, because a generator you cannot see is a generator you
//! cannot aim; **Mix at nought emits the stream and draws nothing**, which is
//! the emit-only mode without a row of its own.

use crate::fx::points::{self, DrawStyle, PointsStream, Projection, RenderMode};
use crate::fx::{
    EffectDef, EffectMetadata, EffectSchema, ParamGroup, ParamId, Params, Port, PortType,
    ResolveCx, Signature, Value,
};
use lumit_fx_macros::Effect;

/// Grid's declared **data** output — the same port Particulate declares, so a
/// wire does not know which generator it came from.
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

/// Two kickers: what the lattice is, and what a cell looks like.
pub const GRID_GROUPS: &[ParamGroup] = &[
    group(
        "Grid",
        &[
            "columns",
            "rows",
            "planes",
            "spacing_x",
            "spacing_y",
            "spacing_z",
            "position_x",
            "position_y",
            "position_z",
            "jitter_x",
            "jitter_y",
            "jitter_z",
            "seed",
        ],
    ),
    group("Point", &["size", "feather", "colour", "max_points"]),
];

/// Which per-point draw is being made. A cell's jitter is a pure function of
/// its index, exactly as a particle's dice are of its birth index.
mod attr {
    pub const JITTER_X: u32 = 0;
    pub const JITTER_Y: u32 = 1;
    pub const JITTER_Z: u32 = 2;
}

/// Grid's controls.
#[derive(Debug, Clone, Copy, PartialEq, Effect)]
#[effect(
    match_name = "grid",
    label = "Grid",
    version = 1,
    category = Generate,
    cost = Moderate,
    // A cell may be jittered anywhere, and the discs are drawn over the whole
    // picture, so no padding covers it.
    roi = FullFrame,
    premultiplied = true,
    // **Not seeded**, deliberately, and unlike Particulate: `seeded` means the
    // pixels are a function of *time* under constant parameters, which is why
    // it folds the layer's clock into the cache key (docs/08 §1.3). A lattice
    // has no clock in it. Folding one in would retire every cached frame on
    // every scrub for nothing.
    seeded = false,
    groups = GRID_GROUPS,
)]
pub struct Grid {
    /// Cells across.
    #[counter(
        label = "Columns",
        min = 1,
        max = 100,
        default = 10,
        hard_min = 1,
        hard_max = 1000,
        unit = Raw
    )]
    pub columns: i32,

    /// Cells down.
    #[counter(
        label = "Rows",
        min = 1,
        max = 100,
        default = 6,
        hard_min = 1,
        hard_max = 1000,
        unit = Raw
    )]
    pub rows: i32,

    /// Cells **through** the layer's plane: copies of the lattice
    /// receding from the camera. One is flat, which is what a 2D layer draws.
    #[counter(
        label = "Planes",
        min = 1,
        max = 100,
        default = 1,
        hard_min = 1,
        hard_max = 1000,
        unit = Raw
    )]
    pub planes: i32,

    /// The gap between columns, px@comp.
    #[slider(
        label = "Spacing x",
        min = 0.0,
        max = 1000.0,
        default = 120.0,
        hard_min = 0.0,
        unit = Px
    )]
    pub spacing_x: f32,

    /// The gap between rows, px@comp.
    #[slider(
        label = "Spacing y",
        min = 0.0,
        max = 1000.0,
        default = 120.0,
        hard_min = 0.0,
        unit = Px
    )]
    pub spacing_y: f32,

    /// The gap between planes, px@comp.
    #[slider(
        label = "Spacing z",
        min = 0.0,
        max = 1000.0,
        default = 120.0,
        hard_min = 0.0,
        unit = Px
    )]
    pub spacing_z: f32,

    /// The lattice's centre, px@comp (point parameters are PIXELS).
    #[slider(label = "Position x", min = 0.0, max = 3840.0, default = 960.0, unit = Px)]
    pub position_x: f32,

    /// px@comp; see [`position_x`](Self::position_x).
    #[slider(label = "Position y", min = 0.0, max = 2160.0, default = 540.0, unit = Px)]
    pub position_y: f32,

    /// How far in front of or behind the layer's own plane the lattice sits,
    /// px@comp. Nought is the plane.
    #[slider(label = "Position z", min = -2000.0, max = 2000.0, default = 0.0, unit = Px)]
    pub position_z: f32,

    /// How far a cell may wander from its place across, px@comp — a uniform
    /// draw of ±half of this, from the seed.
    #[slider(
        label = "Jitter x",
        min = 0.0,
        max = 500.0,
        default = 0.0,
        hard_min = 0.0,
        unit = Px
    )]
    pub jitter_x: f32,

    /// px@comp; see [`jitter_x`](Self::jitter_x).
    #[slider(
        label = "Jitter y",
        min = 0.0,
        max = 500.0,
        default = 0.0,
        hard_min = 0.0,
        unit = Px
    )]
    pub jitter_y: f32,

    /// px@comp, through the plane; see [`jitter_x`](Self::jitter_x).
    #[slider(
        label = "Jitter z",
        min = 0.0,
        max = 500.0,
        default = 0.0,
        hard_min = 0.0,
        unit = Px
    )]
    pub jitter_z: f32,

    /// Which jitter (§2.4). The reseed button rolls it.
    #[seed]
    pub seed: u32,

    /// The diameter of the disc a point is drawn as, px@comp.
    #[slider(min = 0.0, max = 200.0, default = 8.0, hard_min = 0.0, unit = Px)]
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

    /// **The budget dial**, the same rule Particulate's Max particles
    /// carries: the most points that may exist at once, and the peak scratch
    /// the governor grants against. A lattice past it is trimmed from the
    /// **end** of the walk — see [`Grid::stream`]. Deliberately not animatable:
    /// it is a capacity declaration.
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
    /// **At nought the stream is still emitted and nothing is drawn**, which is
    /// this effect's emit-only mode.
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

impl Grid {
    /// The raster factor, for the one input the declaration cannot scale: the
    /// composition's camera arrives in px@comp, like a mask path, and has to be
    /// rearranged into the pixels the frame is drawn at. Particulate's row, for
    /// its reason.
    pub const DERIVED_PX_SCALE: ParamId = ParamId::new("derived.px_scale");

    /// This instance's raster factor, read back out of a resolved bag.
    #[must_use]
    pub fn px_scale_of(p: Params<'_>) -> f32 {
        p.float(Self::DERIVED_PX_SCALE, 1.0)
    }

    /// The lattice at this frame, as a points stream.
    ///
    /// **Closed form, no walk.** Point *i* is
    /// `((plane · rows) + row) · columns + column`, and its place is that
    /// index taken apart again — so the thousandth cell costs what the first
    /// one costs, and `id` is the index for ever, which is what lets a
    /// consumer follow one cell while the lattice grows around it.
    ///
    /// **Units are the caller's**, as [`points::evaluate`]'s are: hand it a
    /// px@comp bag and the stream is px@comp, which is what a wire reads; hand
    /// it the raster-scaled bag and the stream is in the pixels being drawn.
    ///
    /// **The cap rule** keeps the **first** `cap` by index. Particulate
    /// keeps the newest, because a particle set has a birth order and the
    /// newest are the ones the eye is following; a lattice has no birth order
    /// at all, so the rule that is the same *shape* — deterministic, a prefix
    /// of one fixed ordering, identical from any scrub direction — takes the
    /// cells the walk reaches first. It stops rather than trims, so an
    /// over-large lattice costs the cap and not the lattice.
    #[must_use]
    pub fn stream(self, projection: Projection) -> PointsStream {
        let mut out = PointsStream {
            projection,
            ..PointsStream::default()
        };
        let cols = self.columns.clamp(1, 100_000);
        let rows = self.rows.clamp(1, 100_000);
        let planes = self.planes.clamp(1, 100_000);
        let cap = self.max_points.clamp(0, points::CAP_HARD as i32) as usize;
        let size = self.size.max(0.0);
        // Premultiplied, as every colour in the working space is.
        let a = self.colour[3];
        let colour = [
            self.colour[0] * a,
            self.colour[1] * a,
            self.colour[2] * a,
            a,
        ];
        let half = |n: i32| (n - 1) as f32 * 0.5;
        let mut i: u64 = 0;
        for k in 0..planes {
            for j in 0..rows {
                for c in 0..cols {
                    if out.len() >= cap {
                        return out;
                    }
                    let die = |attr| points::draw(self.seed, i, attr) - 0.5;
                    out.position.push([
                        self.position_x
                            + (c as f32 - half(cols)) * self.spacing_x
                            + die(attr::JITTER_X) * self.jitter_x,
                        self.position_y
                            + (j as f32 - half(rows)) * self.spacing_y
                            + die(attr::JITTER_Y) * self.jitter_y,
                        self.position_z
                            + (k as f32 - half(planes)) * self.spacing_z
                            + die(attr::JITTER_Z) * self.jitter_z,
                    ]);
                    // A lattice does not move and does not age. `life` is one
                    // rather than nought so a consumer normalising by it reads
                    // a young point rather than dividing by nothing.
                    out.speed.push([0.0; 3]);
                    out.age.push(0.0);
                    out.life.push(1.0);
                    out.size.push(size);
                    out.rotation.push(0.0);
                    out.colour.push(colour);
                    out.id.push(i);
                    i += 1;
                }
            }
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

/// Grid's behaviour.
///
/// **No CPU reference through the trait**, the same shape Particulate,
/// Scribble and Stroke have and for the same reason: what this effect draws
/// depends on the composition's camera, which is not a parameter and so
/// is not in the bag [`apply_cpu`](EffectDef::apply_cpu) is handed. It rides
/// beside the op instead, on the carriage the birth schedule already uses. The
/// §1.6 oracle is [`Grid::stream`] with [`points::draw_stream`], exercised
/// directly from the test suite — and it is the very stream the GPU draw is
/// handed, so the two paths cannot describe different lattices.
pub struct GridDef;

impl EffectDef for GridDef {
    fn schema(&self) -> &'static EffectSchema {
        &<Grid as EffectMetadata>::SCHEMA
    }

    /// The picture *and* the data, exactly as Particulate declares it.
    fn signature(&self) -> Signature {
        Signature::Image {
            inputs: &[],
            extra: POINTS_OUT,
        }
    }

    /// The raster factor, so the composition's camera reaches the pixels this
    /// frame is drawn at.
    fn resolve_derived(&self, cx: &ResolveCx<'_>, push: &mut dyn FnMut(ParamId, Value)) {
        push(Grid::DERIVED_PX_SCALE, Value::Float(cx.px_scale));
    }
}

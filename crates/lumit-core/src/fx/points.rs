//! The points stream, and the closed forms that put a particle where it is
//! ([impl/particulate.md](../../../../docs/impl/particulate.md)
//! §3–§4, [impl/points-stream.md](../../../../docs/impl/points-stream.md) §3).
//!
//! # In plain terms
//!
//! A particle system makes many small things — sparks, dust, snow — that are
//! born, drift about and fade away. The usual way to build one is a
//! **simulation**: each frame takes the last frame's particles and nudges them
//! along. Lumit does it the other way round. Every particle's whole life is
//! decided the moment it is born, from a seeded random number that never
//! changes, and "where is it at frame 500?" is then arithmetic rather than
//! history: put its birth attributes and its age into a formula and read the
//! answer off. No frame depends on any other frame, so scrubbing anywhere is
//! one evaluation, export matches preview, and the same project renders the
//! same pixels for ever. The price is that particles cannot react to each
//! other — no collisions, no flocking — and for the montage staples this
//! exists for, that is the right trade.
//!
//! Three pieces, in the order a frame uses them:
//!
//! 1. **The birth schedule** ([`Schedule`]). The one place a walk exists, and
//!    it walks *one scalar*: frame by frame from the layer's in point it adds
//!    `rate × Δt` to a carry and hands out a whole particle each time the carry
//!    passes one. Every birth gets a **birth index**, counting from the in
//!    point, and that index is the particle's identity for ever — it is the
//!    `id` the stream reports, so a consumer can follow one particle across
//!    frames without anything being remembered between them.
//! 2. **The closed forms** ([`evaluate`]). For each particle that could be
//!    alive at this moment: what its seeded dice said (where it started, which
//!    way it went, how long it lives), and where the four forces have carried
//!    it since. All four — gravity, wind through drag, drag, turbulence as a
//!    displacement — were chosen *because* they can be integrated on paper;
//!    that is the selection criterion, not a styling choice.
//! 3. **The reference draw** ([`draw_discs`]). The CPU oracle for the picture:
//!    a feathered disc stamped per particle, which is the shape the GPU's
//!    instanced quad has to agree with (docs/08 §1.6).
//!
//! One module, two readers: Particulate's own drawing, and — when PS4 lands —
//! the Points sample driver, which reads the very same stream so that what a
//! wire measures is what the viewer sees. A second implementation of these
//! formulas would be a drift waiting to be found.
//!
//! # The third axis
//!
//! A particle carries three coordinates, not two. Everything above is
//! unchanged by that — the same dice, the same schedule, the same algebra with
//! one more component — and what is new is one small thing at the end:
//! [`Projection`], which says where a particle at depth `z` lands on the
//! layer's own flat picture once the **composition's camera** has looked at it.
//! On a 2D layer there is no camera to look with, the projection is
//! [`Projection::FLAT`], and every number this module produces for the picture
//! is bit-for-bit what it was before the axis existed.

use crate::fx::cpu::{curve_at, CURVE_TABLE};
use crate::fx::noise::{hash01, value3};
use crate::mask::MaskPolyline;

/// Max particles' default: the cap a fresh instance declares, and the
/// budget the reference desktop must draw in a millisecond.
pub const CAP_DEFAULT: i64 = 20_000;

/// Max particles' hard ceiling: the most an instance may ever be typed
/// up to, and the peak scratch the governor grants against (docs/13 §6).
pub const CAP_HARD: i64 = 1_000_000;

/// Where a particle at depth `z` lands on the layer's own flat picture, once
/// the composition's active camera has looked at it.
///
/// # In plain terms
///
/// An effect draws into the layer's own rectangle of pixels, and *then* the
/// compositor turns that rectangle in space and photographs it with the
/// composition's camera. A particle that sits a hundred pixels in front of the
/// rectangle therefore cannot simply be drawn where its `x` and `y` say: it has
/// to be drawn where the camera would *see* it, which is somewhere else on the
/// rectangle, and smaller or larger for being further off or nearer.
///
/// This is that "somewhere else", as one 3×4 table of numbers. It is worked out
/// once a frame by the renderer, from the very matrices the compositor places
/// layers with — `lumit_gpu::camera_matrix` and `lumit_gpu::place_matrix` — so
/// there is no second camera in this engine and nothing here to re-derive. What
/// arrives is the composition of the two, restricted back onto the layer's own
/// plane: `m · (x, y, z, 1)` gives `(X, Y, W)`, the particle lands at
/// `(X/W, Y/W)`, and everything about it — its diameter, its streak — is
/// `1/W` of the size it would have had on the plane.
///
/// [`FLAT`](Self::FLAT) is the identity: `(x, y, 1)` for any `z`, which is a
/// 2D layer, a composition with no camera, and every project saved before the
/// axis existed. It is applied without a branch and lands on exactly the bits
/// it was handed, which is what makes that compatibility guarantee arithmetic
/// rather than a promise.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Projection {
    /// Row-major 3×4: `[x, y, z, 1] → (X, Y, W)`, in the units the particle
    /// positions are already in.
    pub m: [[f32; 4]; 3],
}

impl Default for Projection {
    fn default() -> Self {
        Projection::FLAT
    }
}

impl Projection {
    /// The layer's own plane, unlooked-at: `z` changes nothing.
    pub const FLAT: Projection = Projection {
        m: [
            [1.0, 0.0, 0.0, 0.0],
            [0.0, 1.0, 0.0, 0.0],
            [0.0, 0.0, 0.0, 1.0],
        ],
    };

    /// Whether this is the identity — a 2D layer, or a comp with no camera.
    #[must_use]
    pub fn is_flat(&self) -> bool {
        *self == Projection::FLAT
    }

    /// Where `p` lands, and by how much it foreshortens: `([x, y], scale)`.
    ///
    /// A particle at or behind the camera's own plane answers a **scale of
    /// nought**, which draws nothing at all: a disc of no radius covers no
    /// pixel, so the degenerate case degrades rather than dividing by a
    /// vanishing `W` and flinging a particle across the frame
    /// (14-ENGINEERING-RULES §4).
    #[must_use]
    pub fn apply(&self, p: [f32; 3]) -> ([f32; 2], f32) {
        let v = [p[0], p[1], p[2], 1.0];
        let dot = |r: &[f32; 4]| r[0] * v[0] + r[1] * v[1] + r[2] * v[2] + r[3];
        let w = dot(&self.m[2]);
        if w <= 1e-4 {
            return ([p[0], p[1]], 0.0);
        }
        let inv = 1.0 / w;
        ([dot(&self.m[0]) * inv, dot(&self.m[1]) * inv], inv)
    }

    /// The same projection, for particle coordinates measured in a raster
    /// `px_scale` times the size the matrix was built at.
    ///
    /// The matrix is built in **px@comp**, because that is what the layer's
    /// placement is in; the draw works in whatever raster the preview is
    /// running at. Scaling the input down and the answer back up is one
    /// rearrangement of the same numbers: only the translation column of the
    /// two position rows grows with the raster, and only the direction part of
    /// the depth row shrinks with it. At full scale nothing moves, so a
    /// full-resolution preview and the export are bit-identical by
    /// construction.
    #[must_use]
    pub fn rescaled(&self, px_scale: f32) -> Projection {
        let s = px_scale.max(1e-6);
        if s == 1.0 {
            return *self;
        }
        let mut m = self.m;
        m[0][3] *= s;
        m[1][3] *= s;
        for c in m[2].iter_mut().take(3) {
            *c /= s;
        }
        Projection { m }
    }
}

/// What a particle is drawn as (particulate.md §2, Render group).
///
/// All three are the same instanced quad with a different coverage inside it,
/// which is why they share one kernel and one CPU reference: a disc is a
/// feathered circle, a streak is that circle swept from `p(t − length)` to
/// `p(t)` — a capsule, and exactly a disc when the length is zero — and a
/// sprite is the referenced layer's picture in the quad instead.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum RenderMode {
    /// A feathered disc: the reference mode, and what an unset Sprite falls
    /// back to.
    #[default]
    Disc,
    /// The Sprite layer's picture, rotated and scaled per particle. **Unset
    /// draws discs** — a render mode must always draw something.
    Sprite,
    /// A capsule from the particle's position a Streak length ago to where it
    /// is now, found by the closed form again rather than by history.
    Streak,
}

impl RenderMode {
    /// The Choice option labels, in code order.
    pub const OPTIONS: &'static [&'static str] = &["Disc", "Sprite", "Streak"];

    /// The mode for a stored Choice index; anything unknown is a Disc, the
    /// declared default (a document from a newer build renders).
    #[must_use]
    pub const fn from_code(code: u32) -> Self {
        match code {
            1 => RenderMode::Sprite,
            2 => RenderMode::Streak,
            _ => RenderMode::Disc,
        }
    }
}

/// How the stream is drawn: the Render group, reduced (particulate.md §2).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DrawStyle {
    pub mode: RenderMode,
    /// Disc edge softness, `0..=1`.
    pub feather: f32,
    /// The tail's age offset in seconds — Streak length. Zero everywhere else.
    pub streak_seconds: f32,
    /// The host Mix, `0..=1`. Folded into the particle's own alpha rather than
    /// run as a second pass: for a premultiplied `over`, scaling the source's
    /// coverage by the Mix **is** the dissolve, exactly.
    pub mix: f32,
}

/// Where a particle is born.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EmitterShape {
    /// Every particle from one point.
    Point,
    /// Uniformly along a segment of `width`, centred on the position.
    Line,
    /// Uniformly over the interior of an ellipse of `width` × `height`.
    Ellipse,
    /// Uniformly over the interior of a rectangle of `width` × `height`.
    Rectangle,
    /// Uniformly along the arc length of a mask path. An **empty
    /// polyline emits nothing** — the documented no-op, degrade and never
    /// fault (14-ENGINEERING-RULES §4).
    MaskPath,
    /// Uniformly along an ellipse's **outline**, by arc length.
    EllipseOutline,
    /// Uniformly along a rectangle's **outline**, by arc length.
    RectangleOutline,
}

impl EmitterShape {
    /// The Choice option labels, in code order.
    ///
    /// The two outline shapes are **appended**, not slotted in beside the areas
    /// they hollow out: a Choice is stored as its index, so inserting
    /// one would silently turn every saved Mask path emitter into something
    /// else. The dropdown reads in code order and the panel is the poorer for
    /// it by one line, which is the price of a document that still means what
    /// it said.
    pub const OPTIONS: &'static [&'static str] = &[
        "Point",
        "Line",
        "Ellipse",
        "Rectangle",
        "Mask path",
        "Ellipse outline",
        "Rectangle outline",
    ];

    /// The shape for a stored Choice index; anything unknown is a Point, which
    /// is the declared default (a document from a newer build is rendered, not
    /// refused).
    #[must_use]
    pub const fn from_code(code: u32) -> Self {
        match code {
            1 => EmitterShape::Line,
            2 => EmitterShape::Ellipse,
            3 => EmitterShape::Rectangle,
            4 => EmitterShape::MaskPath,
            5 => EmitterShape::EllipseOutline,
            6 => EmitterShape::RectangleOutline,
            _ => EmitterShape::Point,
        }
    }

    /// Whether this shape emits along an **outline** the host flattens for it:
    /// the two shapes that walk a polyline of their own rather than filling an
    /// interior.
    #[must_use]
    pub const fn is_outline(self) -> bool {
        matches!(
            self,
            EmitterShape::EllipseOutline | EmitterShape::RectangleOutline
        )
    }
}

/// How many chords an ellipse's outline is flattened into.
///
/// The vertices sit **on** the true ellipse, so the only error is the chord
/// cutting the corner between two of them: `r · (1 − cos(π/N))`, which at 128
/// is three parts in ten thousand — a twentieth of a pixel on a four-hundred
/// pixel emitter, and well inside the 10⁻⁵-of-range agreement the two render
/// paths owe each other. Walking that polyline by arc length is
/// **exactly** what a Mask path emitter already does, which is why there is one
/// walk in this engine and not two.
const OUTLINE_SEGMENTS: usize = 128;

/// The emitter's own outline as a closed polyline in its **local** frame —
/// centred on the origin, unturned — or an empty one for every shape that fills
/// an interior instead.
///
/// Local rather than absolute, because [`birth_point`] then turns and places it
/// with the same two lines every other area shape uses: an outline is a
/// rectangle or an ellipse that has been hollowed out, not a path the user drew
/// somewhere.
///
/// Both render paths call **this** function — the CPU reference once per
/// evaluation, the GPU host once per frame on its way into the kernel's path
/// buffer — so the two cannot come to flatten the same ellipse differently.
#[must_use]
pub fn outline_polyline(e: &Emitter) -> MaskPolyline {
    let (hw, hh) = (0.5 * e.width, 0.5 * e.height);
    let mut points: Vec<[f32; 2]> = match e.shape {
        EmitterShape::EllipseOutline => (0..=OUTLINE_SEGMENTS)
            .map(|i| {
                let a = i as f32 / OUTLINE_SEGMENTS as f32 * std::f32::consts::TAU;
                let (s, c) = a.sin_cos();
                [hw * c, hh * s]
            })
            .collect(),
        EmitterShape::RectangleOutline => {
            vec![[-hw, -hh], [hw, -hh], [hw, hh], [-hw, hh], [-hw, -hh]]
        }
        _ => return MaskPolyline::default(),
    };
    // A degenerate extent is a shape with nowhere to walk; one point is not a
    // polyline, so it is given a second coincident one and every birth lands on
    // the emitter's centre — degrade, never fault (14-ENGINEERING-RULES §4).
    if points.len() < 2 {
        points = vec![[0.0, 0.0], [0.0, 0.0]];
    }
    let mut arc = Vec::with_capacity(points.len());
    let mut total = 0.0f32;
    arc.push(0.0);
    for w in points.windows(2) {
        total += (w[1][0] - w[0][0]).hypot(w[1][1] - w[0][1]);
        arc.push(total);
    }
    MaskPolyline {
        points,
        arc,
        closed: true,
        feather: 0.0,
        expansion: 0.0,
        corners: Vec::new(),
    }
}

/// How particles are born: where, how often, and which way they leave.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Emitter {
    pub shape: EmitterShape,
    /// px@comp (or raster px — the caller's own scale; see [`evaluate`]).
    /// Three coordinates: `z` is towards the camera, positive away
    /// from it, and zero is the layer's own plane.
    pub position: [f32; 3],
    /// The extents Line, Ellipse and Rectangle are drawn to.
    pub width: f32,
    pub height: f32,
    /// The extent **through** the layer's plane: Point becomes a
    /// segment along `z`, Ellipse a cylinder and Rectangle a box, each filled
    /// uniformly. Line and Mask path stay planar — a mask path is where the
    /// user drew it, and a line is one dimension by name.
    pub depth: f32,
    /// Rotation of the emitter's own shape about `position`, degrees. Turns the
    /// shape in its own plane; `depth` runs through that plane and is not
    /// turned by it.
    pub angle_deg: f32,
    /// Launch direction in the layer's plane, degrees; −90 is up.
    pub direction_deg: f32,
    /// Launch **elevation** out of that plane, degrees. Zero is the
    /// plane itself, which is what every project saved before the axis existed
    /// reads as.
    pub direction_z_deg: f32,
    /// The cone about `direction_deg`, degrees.
    pub spread_deg: f32,
    /// The cone about `direction_z_deg`, degrees — the elevation's own spread,
    /// separate so that a full 360° in-plane spread stays a disc of directions
    /// rather than quietly becoming a sphere.
    pub spread_z_deg: f32,
    /// Speed at birth, px per second.
    pub speed: f32,
    /// Per-particle speed spread, `0..=1`.
    pub speed_jitter: f32,
}

/// What a particle looks like and how long it lasts.
///
/// Not `Copy`: it carries the two baked over-life curves, which are the same
/// 257-entry tables the grade family bakes ([`curve_table`]) so both render
/// paths read one shape from one place.
#[derive(Debug, Clone, PartialEq)]
pub struct ParticleLook {
    /// Lifetime, seconds.
    pub life: f32,
    /// Per-particle lifetime spread, `0..=1`.
    pub life_jitter: f32,
    /// Diameter at birth, px.
    pub size: f32,
    /// Per-particle size spread, `0..=1`.
    pub size_jitter: f32,
    /// Multiplies `size` by normalised age.
    pub size_curve: [f32; CURVE_TABLE],
    /// Multiplies alpha by normalised age.
    pub opacity_curve: [f32; CURVE_TABLE],
    /// Scene-linear RGBA at birth; values above 1 are legal and useful.
    pub colour: [f32; 4],
    /// Scene-linear RGBA at death, blended to over normalised age.
    pub end_colour: [f32; 4],
    /// Rotation, degrees.
    pub rotation_deg: f32,
    /// The per-particle spread of `rotation_deg`, **degrees**: each particle
    /// takes a uniform draw of `±rotation_jitter_deg/2` about it, from the seed
    /// hash.
    pub rotation_jitter_deg: f32,
    /// Spin, degrees per second.
    pub spin_deg: f32,
    /// Rotation follows the direction of travel; `rotation_deg` adds on top.
    pub align_to_motion: bool,
}

/// The four v1 forces — exactly the set with closed-form integrals.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Forces {
    /// px per second², positive down. **Down stays down**: gravity is
    /// the one force with a direction of its own, and giving it a depth
    /// component would be inventing a control the note does not ask for.
    pub gravity: f32,
    /// The air's own speed, px per second, on all three axes. Wind acts
    /// **through** drag: with `drag` at 0 it does nothing at all, which is the
    /// documented behaviour.
    pub wind: [f32; 3],
    /// Exponential approach of the particle's speed towards the wind's, per
    /// second.
    pub drag: f32,
    /// Displacement magnitude, px.
    pub turbulence: f32,
    /// Spatial wavelength of the noise, px.
    pub turbulence_scale: f32,
    /// Evolution rate against age, Hz.
    pub turbulence_speed: f32,
}

/// Everything the closed forms read, already reduced to plain numbers — the
/// `packed` shape the registry's §2.4 convention asks of every effect.
#[derive(Debug, Clone, PartialEq)]
pub struct PointsParams {
    pub emitter: Emitter,
    pub particle: ParticleLook,
    pub forces: Forces,
    /// Max particles: the most that may be **live** at once. Over
    /// budget, the newest by birth index survive.
    pub cap: u32,
    pub seed: u32,
    /// Where the composition's camera puts a particle that is off the layer's
    /// plane. [`Projection::FLAT`] — the default — is a 2D layer, and
    /// is what [`crate::fx::effects::particulate::Particulate::points`] hands
    /// back, because a bag of parameters cannot know what the comp is looking
    /// with. The renderer and the driver walk fill it in with
    /// [`projected`](Self::projected).
    pub projection: Projection,
}

impl PointsParams {
    /// The same parameters, seen through `projection`.
    #[must_use]
    pub fn projected(mut self, projection: Projection) -> Self {
        self.projection = projection;
        self
    }
}

/// One frame's particles, structure-of-arrays — the CPU form of the stream
/// (particulate.md §4, `Vec3`).
///
/// The GPU form is the same attributes in buffers; `count` there is what
/// [`len`](Self::len) is here, because a length beside a `Vec` would be a
/// second truth about one number. Every array is the same length and indexed
/// alike: entry *i* of each describes one particle, ordered by **birth index
/// ascending**, which is a fact of the evaluation rather than an artefact of
/// how it was scheduled.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct PointsStream {
    /// px in the layer's own three axes, this frame (closed form plus
    /// turbulence) — **unprojected**. A consumer that has not declared
    /// 3D awareness reads [`projected`](Self::projected) instead.
    pub position: Vec<[f32; 3]>,
    /// px per second — the analytic speed plus turbulence's own rate of change.
    pub speed: Vec<[f32; 3]>,
    /// Seconds since birth.
    pub age: Vec<f32>,
    /// This particle's whole lifetime, seconds, so a consumer can normalise the
    /// age without re-deriving the per-particle jitter.
    pub life: Vec<f32>,
    /// px, after Size over life.
    pub size: Vec<f32>,
    /// Radians, after Spin and Align to motion.
    pub rotation: Vec<f32>,
    /// Premultiplied scene-linear RGBA, after the over-life blends.
    pub colour: Vec<[f32; 4]>,
    /// The **birth index** — stable across frames, and what makes trails
    /// possible. There is no separate id space (particulate.md §4).
    pub id: Vec<u64>,
    /// The camera the stream was evaluated under — carried on the
    /// stream itself so that "where does particle *i* appear?" is one call and
    /// not a second thing to thread beside it. [`Projection::FLAT`] on a 2D
    /// layer, and in the driver walk's px@comp evaluation of a 2D layer.
    pub projection: Projection,
    /// How far each point's size is stretched across and down, in its own
    /// turned frame. 1 is unstretched. **Empty means no point is**, which is
    /// what every producer hands out.
    pub stretch: Vec<[f32; 2]>,
    /// Which points are picked, 1 or 0, for the effects that act on only some
    /// of a stream. **Empty means all of them.**
    pub pick: Vec<f32>,
    /// A number each point carries for the effects below to read: which of
    /// several layers to stamp, how far to offset it in time, where it comes
    /// in an order. **Empty means its place in the stream.**
    pub index: Vec<f32>,
    /// The named columns: a value for each point under a name an effect
    /// gave it, for the effects below to read by that name. Never more than
    /// [`NAMED_MAX`] of them. Written through [`named_mut`](Self::named_mut)
    /// and read through [`column`](Self::column) and [`read`](Self::read).
    pub named: Vec<Named>,
    /// How many of this stream's pixels make one px@comp, which is what the
    /// `@x`, `@y`, `@z` and `@size` names are read in at any preview size.
    /// **Nought means 1**, which is what a stream made in px@comp carries.
    pub px_scale: f32,
}

/// One named column: a name, and one, two or four numbers for each point.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Named {
    pub name: String,
    /// How many of a point's four numbers are its value: 1 for a number, 2
    /// for an offset across and down, 4 for a colour. The rest are 0. Nought
    /// is a name two joined streams held at different widths, which reads
    /// nothing.
    pub width: u8,
    pub values: Vec<[f32; 4]>,
}

/// The most named columns one stream carries, which is far more than a
/// stack of effects writes. A new name past it is not written.
pub const NAMED_MAX: usize = 64;

/// What a name reads, found once so a walk over the points does not look
/// the name up again. It holds for the stream it came from, and for a copy
/// of that stream with points dropped or reordered.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Column {
    /// The number each point carries, which is what an empty name means.
    Number,
    /// A named column, by its place in the stream's list.
    Named(usize),
    /// The names starting with `@`, worked out from the point itself.
    Index,
    Id,
    X,
    Y,
    Z,
    Count,
    IndexNorm,
    Rand01,
    Size,
    Age,
    Picked,
    /// A name nothing wrote.
    Nothing,
}

/// The dice `@rand01` rolls, kept off the numbers every effect uses for its
/// own.
const RAND01_ATTR: u32 = 96;

impl PointsStream {
    /// The named column `name`, filled in for every point so it can be
    /// written, holding `width` numbers a point: 1, 2 or 4. A new one starts
    /// at 0, and one that was wider keeps nothing in the numbers it gives
    /// up. `None` for an empty name, for one starting with `@`, which is
    /// only ever read, and for a new name on a stream that carries
    /// [`NAMED_MAX`] already.
    pub fn named_mut(&mut self, name: &str, width: u8) -> Option<&mut Vec<[f32; 4]>> {
        if name.is_empty() || name.starts_with('@') {
            return None;
        }
        let n = self.len();
        let width = match width {
            0 | 1 => 1,
            2 => 2,
            _ => 4,
        };
        let at = match self.named.iter().position(|c| c.name == name) {
            Some(at) => at,
            None if self.named.len() < NAMED_MAX => {
                self.named.push(Named {
                    name: name.to_owned(),
                    ..Named::default()
                });
                self.named.len() - 1
            }
            None => return None,
        };
        let column = self.named.get_mut(at)?;
        column.values.resize(n, [0.0; 4]);
        if width < column.width {
            for value in &mut column.values {
                value
                    .iter_mut()
                    .skip(usize::from(width))
                    .for_each(|v| *v = 0.0);
            }
        }
        column.width = width;
        Some(&mut column.values)
    }

    /// What `name` reads, found once for a walk over the points.
    ///
    /// An empty name is the number each point carries. A name starting with
    /// `@` is worked out from the point and never stored, in any mix of
    /// capitals: `@index` its place in the stream, `@id`, `@x`, `@y` and
    /// `@z` where it is, `@n` how many points there are, `@indexnorm` its
    /// place from 0 at the first to 1 at the last, `@rand01` a roll of its
    /// own dice that is the same every frame, `@size`, `@age` and `@picked`.
    /// Anything else is a named column.
    #[must_use]
    pub fn column(&self, name: &str) -> Column {
        if name.is_empty() {
            return Column::Number;
        }
        let Some(built_in) = name.strip_prefix('@') else {
            let carried = |c: &Named| c.name == name && c.width > 0;
            let at = self.named.iter().position(carried);
            return at.map_or(Column::Nothing, Column::Named);
        };
        [
            ("index", Column::Index),
            ("id", Column::Id),
            ("x", Column::X),
            ("y", Column::Y),
            ("z", Column::Z),
            ("n", Column::Count),
            ("indexnorm", Column::IndexNorm),
            ("rand01", Column::Rand01),
            ("size", Column::Size),
            ("age", Column::Age),
            ("picked", Column::Picked),
        ]
        .into_iter()
        .find(|(want, _)| built_in.eq_ignore_ascii_case(want))
        .map_or(Column::Nothing, |(_, column)| column)
    }

    /// The group a name means: the picked points for an empty name, and
    /// [`column`](Self::column) for any other.
    #[must_use]
    pub fn group(&self, name: &str) -> Column {
        if name.is_empty() {
            Column::Picked
        } else {
            self.column(name)
        }
    }

    /// How many of this stream's pixels make one px@comp.
    #[must_use]
    pub fn units(&self) -> f32 {
        if self.px_scale > 0.0 {
            self.px_scale
        } else {
            1.0
        }
    }

    /// What `column` reads at point `i` as one number, or `None` for a name
    /// nothing wrote. The one reader every effect goes through. An offset
    /// reads as its length, and a colour as how bright its first three
    /// numbers are.
    #[must_use]
    pub fn read(&self, column: Column, i: usize) -> Option<f32> {
        let place = |axis: usize| self.position.get(i).map_or(0.0, |p| p[axis]) / self.units();
        let of = |column: &[f32]| column.get(i).copied().unwrap_or(0.0);
        let id = self.id.get(i).copied().unwrap_or(0);
        Some(match column {
            Column::Number => self.index_of(i),
            Column::Named(at) => {
                let ([a, b, c, _], width) = self.whole(Column::Named(at), i)?;
                let luma = crate::fx::cpu::LUMA;
                match width {
                    2 => a.hypot(b),
                    4 => a * luma[0] + b * luma[1] + c * luma[2],
                    _ => a,
                }
            }
            Column::Index => i as f32,
            Column::Id => id as f32,
            Column::X => place(0),
            Column::Y => place(1),
            Column::Z => place(2),
            Column::Count => self.len() as f32,
            // A single point reads as the first.
            Column::IndexNorm => i as f32 / self.len().saturating_sub(1).max(1) as f32,
            Column::Rand01 => draw(0, id, RAND01_ATTR),
            Column::Size => of(&self.size) / self.units(),
            Column::Age => of(&self.age),
            Column::Picked => f32::from(u8::from(self.picked(i))),
            Column::Nothing => return None,
        })
    }

    /// Every number `column` holds at point `i`, and how many of the four
    /// are its value. Anything but a named column is one number.
    #[must_use]
    pub fn whole(&self, column: Column, i: usize) -> Option<([f32; 4], u8)> {
        let Column::Named(at) = column else {
            return Some(([self.read(column, i)?, 0.0, 0.0, 0.0], 1));
        };
        let named = self.named.get(at)?;
        let value = named.values.get(i).copied().unwrap_or([0.0; 4]);
        Some((value, named.width))
    }

    /// [`read`](Self::read) for one point by name. A walk over the points
    /// finds the [`column`](Self::column) once instead.
    #[must_use]
    pub fn value(&self, name: &str, i: usize) -> Option<f32> {
        self.read(self.column(name), i)
    }

    /// [`value`](Self::value), with 0 for a name that reads nothing. So a
    /// name nobody wrote puts no point in a group.
    #[must_use]
    pub fn value_of(&self, name: &str, i: usize) -> f32 {
        self.value(name, i).unwrap_or(0.0)
    }

    /// Whether point `i` is in `group`: picked or not for the picked points,
    /// and for any other column a value above `threshold`.
    #[must_use]
    pub fn in_group(&self, group: Column, threshold: f32, i: usize) -> bool {
        match group {
            Column::Picked => self.picked(i),
            _ => self.read(group, i).unwrap_or(0.0) > threshold,
        }
    }

    /// Whether an effect acts on point `i`, by its Apply to, Group and
    /// Threshold rows: every point, the ones in the group, or the ones
    /// not in it. `group` is what [`group`](Self::group) made of the row.
    #[must_use]
    pub fn applies(&self, apply_to: u32, group: Column, threshold: f32, i: usize) -> bool {
        match apply_to {
            1 => self.in_group(group, threshold, i),
            2 => !self.in_group(group, threshold, i),
            _ => true,
        }
    }

    /// Every named column made as long as the stream, so the helpers that
    /// drop or reorder points can treat it like any other.
    fn fill_named(&mut self) {
        let n = self.len();
        for column in &mut self.named {
            column.values.resize(n, [0.0; 4]);
        }
    }

    /// Point `i`'s stretch, `[1, 1]` where none was set.
    #[must_use]
    pub fn stretch_of(&self, i: usize) -> [f32; 2] {
        self.stretch.get(i).copied().unwrap_or([1.0; 2])
    }

    /// Whether point `i` is picked. Every point is until something says not.
    #[must_use]
    pub fn picked(&self, i: usize) -> bool {
        self.pick.get(i).is_none_or(|p| *p > 0.5)
    }

    /// The number point `i` carries, its place in the stream where none was
    /// set.
    #[must_use]
    pub fn index_of(&self, i: usize) -> f32 {
        self.index.get(i).copied().unwrap_or(i as f32)
    }

    /// The stretch column, filled in for every point so it can be written.
    pub fn stretch_mut(&mut self) -> &mut Vec<[f32; 2]> {
        let n = self.len();
        self.stretch.resize(n, [1.0; 2]);
        &mut self.stretch
    }

    /// The pick column, filled in for every point so it can be written.
    pub fn pick_mut(&mut self) -> &mut Vec<f32> {
        let n = self.len();
        self.pick.resize(n, 1.0);
        &mut self.pick
    }

    /// The index column, filled in for every point so it can be written.
    pub fn index_mut(&mut self) -> &mut Vec<f32> {
        let n = self.len();
        for i in self.index.len()..n {
            self.index.push(i as f32);
        }
        self.index.truncate(n);
        &mut self.index
    }

    /// `other`'s points added after this stream's own. The camera stays this
    /// stream's. The result is no longer in `id` order.
    pub fn append(&mut self, other: &PointsStream) {
        // Fill the optional columns in first if either side carries one.
        // Decided here and not from the column in the loop, since a stream
        // with no points yet has every column empty.
        let stretch = !self.stretch.is_empty() || !other.stretch.is_empty();
        let pick = !self.pick.is_empty() || !other.pick.is_empty();
        let index = !self.index.is_empty() || !other.index.is_empty();
        if stretch {
            self.stretch_mut();
        }
        if pick {
            self.pick_mut();
        }
        if index {
            self.index_mut();
        }
        for i in 0..other.len() {
            if stretch {
                self.stretch.push(other.stretch_of(i));
            }
            if pick {
                self.pick.push(if other.picked(i) { 1.0 } else { 0.0 });
            }
            if index {
                self.index.push(other.index_of(i));
            }
        }
        // A named column either side carries goes across, and the side
        // without it reads 0. A name the two hold at different widths reads
        // nothing from here on, since one value cannot be made of both.
        self.fill_named();
        let n = self.len();
        for theirs in &other.named {
            if let Some(ours) = self.named.iter_mut().find(|c| c.name == theirs.name) {
                if ours.width != theirs.width {
                    ours.width = 0;
                }
            } else if self.named.len() < NAMED_MAX {
                self.named.push(Named {
                    name: theirs.name.clone(),
                    width: theirs.width,
                    values: vec![[0.0; 4]; n],
                });
            }
        }
        for ours in &mut self.named {
            let theirs = other.named.iter().find(|c| c.name == ours.name);
            let value = |i: usize| theirs.and_then(|c| c.values.get(i)).copied();
            ours.values
                .extend((0..other.len()).map(|i| value(i).unwrap_or([0.0; 4])));
            if ours.width == 0 {
                ours.values.fill([0.0; 4]);
            }
        }
        self.position.extend_from_slice(&other.position);
        self.speed.extend_from_slice(&other.speed);
        self.age.extend_from_slice(&other.age);
        self.life.extend_from_slice(&other.life);
        self.size.extend_from_slice(&other.size);
        self.rotation.extend_from_slice(&other.rotation);
        self.colour.extend_from_slice(&other.colour);
        self.id.extend_from_slice(&other.id);
    }

    /// How many particles are live this frame.
    #[must_use]
    pub fn len(&self) -> usize {
        self.id.len()
    }

    /// **What a 2D consumer reads**: particle `i` where the camera puts
    /// it on the layer's own plane.
    ///
    /// The wire carries one type. A consumer that declares 3D awareness on its
    /// port ([`crate::fx::Port::three_d`]) reads [`position`](Self::position)
    /// and does its own geometry; everything else — the Points sample driver,
    /// and the whole 2D half of the family — reads this, and on a 2D layer the
    /// two are the same numbers.
    #[must_use]
    pub fn projected(&self, i: usize) -> [f32; 2] {
        self.position
            .get(i)
            .map_or([0.0; 2], |p| self.projection.apply(*p).0)
    }

    /// How much particle `i` foreshortens: 1 on the plane, less further off,
    /// more nearer, and **nought** for a particle at or behind the camera.
    #[must_use]
    pub fn depth_scale(&self, i: usize) -> f32 {
        self.position
            .get(i)
            .map_or(0.0, |p| self.projection.apply(*p).1)
    }

    /// Whether nothing is alive — an unwired or unborn stream.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.id.is_empty()
    }

    /// The same stream in a raster `px_scale` times the size it was evaluated
    /// at.
    ///
    /// **Why a consumer needs this and a producer does not.** A producer's
    /// distances arrive already rescaled, because they are rows in its bag and
    /// the resolve step scales a `Px` row generically (docs/08 §2.3). A stream
    /// read off a *wire* is not in anybody's bag: it is evaluated once, in
    /// px@comp, because that is the unit a consumer reads it as data in.
    /// Turning it into the pixels a frame is being drawn at is therefore one
    /// multiplication per length — the three position axes, the three speed
    /// axes, the diameter — and the camera taking the same factor it takes
    /// for Particulate ([`Projection::rescaled`]). At full resolution
    /// `px_scale` is 1 and every number is the bits it already was, which is
    /// what makes preview and export identical by construction.
    #[must_use]
    pub fn rescaled(&self, px_scale: f32) -> PointsStream {
        let s = px_scale.max(1e-6);
        if s == 1.0 {
            return self.clone();
        }
        let scale3 = |v: &Vec<[f32; 3]>| -> Vec<[f32; 3]> {
            v.iter().map(|p| [p[0] * s, p[1] * s, p[2] * s]).collect()
        };
        PointsStream {
            position: scale3(&self.position),
            speed: scale3(&self.speed),
            age: self.age.clone(),
            life: self.life.clone(),
            size: self.size.iter().map(|d| d * s).collect(),
            rotation: self.rotation.clone(),
            colour: self.colour.clone(),
            id: self.id.clone(),
            projection: self.projection.rescaled(s),
            stretch: self.stretch.clone(),
            pick: self.pick.clone(),
            index: self.index.clone(),
            named: self.named.clone(),
            px_scale: s * self.units(),
        }
    }

    /// Where particle `id` was in `past`, or `None` if it was not alive then.
    ///
    /// **A merge, not a search**: both streams are ordered by birth
    /// index ascending — a fact of the evaluation rather than a scheduling
    /// artefact (particulate.md §5) — so a walk that only ever moves forwards
    /// answers every particle of one stream against another in one pass.
    /// `cursor` is that walk's place in `past`, carried by the caller from one
    /// particle to the next.
    #[must_use]
    pub fn seek_id(past: &PointsStream, id: u64, cursor: &mut usize) -> Option<usize> {
        while past.id.get(*cursor).is_some_and(|got| *got < id) {
            *cursor += 1;
        }
        (past.id.get(*cursor) == Some(&id)).then_some(*cursor)
    }

    /// The points furthest from the camera first, so a draw in that order
    /// lays the near ones on top. Points the same distance off keep the order
    /// they were in, and on a 2D layer nothing moves.
    pub fn sort_far_to_near(&mut self) {
        if self.projection.is_flat() {
            return;
        }
        let near: Vec<f32> = (0..self.len()).map(|i| self.depth_scale(i)).collect();
        let mut order: Vec<usize> = (0..self.len()).collect();
        order.sort_by(|a, b| near[*a].total_cmp(&near[*b]));
        self.reorder(&order);
    }

    /// The points in ascending `id` order, which is the order Trail and the
    /// other readers walk a stream in. Points with the same `id` keep the
    /// order they were in.
    pub fn sort_by_id(&mut self) {
        let mut order: Vec<usize> = (0..self.len()).collect();
        order.sort_by_key(|i| self.id.get(*i).copied().unwrap_or(0));
        self.reorder(&order);
    }

    /// The same points in another order: place `k` takes the point that was
    /// at `order[k]`.
    fn reorder(&mut self, order: &[usize]) {
        fn pick<T: Copy>(v: &mut Vec<T>, order: &[usize]) {
            *v = order.iter().filter_map(|i| v.get(*i).copied()).collect();
        }
        self.fill_named();
        pick(&mut self.position, order);
        pick(&mut self.speed, order);
        pick(&mut self.age, order);
        pick(&mut self.life, order);
        pick(&mut self.size, order);
        pick(&mut self.rotation, order);
        pick(&mut self.colour, order);
        pick(&mut self.id, order);
        pick(&mut self.stretch, order);
        pick(&mut self.pick, order);
        pick(&mut self.index, order);
        for column in &mut self.named {
            pick(&mut column.values, order);
        }
    }

    /// Keep the points `keep` answers true for, by index, in the order they
    /// are in.
    pub fn retain(&mut self, keep: impl FnMut(usize) -> bool) {
        let flags: Vec<bool> = (0..self.len()).map(keep).collect();
        fn sift<T>(v: &mut Vec<T>, flags: &[bool]) {
            let mut i = 0;
            v.retain(|_| {
                i += 1;
                flags.get(i - 1).copied().unwrap_or(false)
            });
        }
        sift(&mut self.position, &flags);
        sift(&mut self.speed, &flags);
        sift(&mut self.age, &flags);
        sift(&mut self.life, &flags);
        sift(&mut self.size, &flags);
        sift(&mut self.rotation, &flags);
        sift(&mut self.colour, &flags);
        sift(&mut self.id, &flags);
        if !self.stretch.is_empty() {
            sift(&mut self.stretch, &flags);
        }
        if !self.pick.is_empty() {
            sift(&mut self.pick, &flags);
        }
        if !self.index.is_empty() {
            sift(&mut self.index, &flags);
        }
        for column in &mut self.named {
            sift(&mut column.values, &flags);
        }
    }

    /// Keep the **newest `n`** particles by birth index, dropping the rest.
    ///
    /// The degradation rung: under governor pressure the effect draws
    /// the newest half, halving again as pressure demands. It is the cap rule
    /// applied a second time, so what vanishes under pressure is what would
    /// have vanished under a smaller cap — deterministic, and identical from
    /// any scrub direction. Interaction only; never on export (docs/06 §6.2).
    pub fn keep_newest(&mut self, n: usize) {
        let len = self.len();
        if n >= len {
            return;
        }
        let cut = len - n;
        self.fill_named();
        for column in &mut self.named {
            column.values.drain(..cut);
        }
        self.position.drain(..cut);
        self.speed.drain(..cut);
        self.age.drain(..cut);
        self.life.drain(..cut);
        self.size.drain(..cut);
        self.rotation.drain(..cut);
        self.colour.drain(..cut);
        self.id.drain(..cut);
        for column in [&mut self.pick, &mut self.index] {
            if column.len() == len {
                column.drain(..cut);
            }
        }
        if self.stretch.len() == len {
            self.stretch.drain(..cut);
        }
    }
}

/// The most frames [`Schedule::scan`] records births for.
///
/// The scan itself is scalar work over every frame since the layer's in point;
/// what is *recorded* is only the window a frame can still see particles from,
/// and that record is an allocation, so it is budgeted (14-ENGINEERING-RULES
/// §6). A hundred thousand frames is nearly half an hour at 60 fps — well past
/// any lifetime a particle is given — and a Life typed past it simply stops
/// producing older particles rather than growing the allocation without bound.
pub const MAX_WINDOW_FRAMES: i64 = 100_000;

/// The most frames [`Schedule::scan`] will **walk**, as against record.
///
/// # In plain terms
///
/// The scan starts at the layer's in point and steps one frame at a time to the
/// frame being drawn, because the birth count carries a fraction forward and a
/// fraction cannot be jumped over. Each step also asks the Emit rate what it is
/// — through keyframes, expressions and driver wires — so a step is not free.
///
/// How many steps that is comes from the layer's own time, and a layer's time is
/// a *retime curve*: a keyframe saying "at frame 10, this layer is at ten
/// million seconds" is a thing a project file can say, and it used to mean a
/// loop of six hundred million expression evaluations on the render thread. Not
/// slow — stopped. That is the whole of this finding: a project file must not be
/// able to hang the application, however unlikely the project.
///
/// A million frames is four and a half hours at 60 fps, which no layer's time
/// legitimately reaches; below it nothing changes at all. Above it the walk
/// starts late and [`Schedule::is_exact`] says so, so the effect can be badged
/// rather than quietly drawing a different picture.
///
/// **On determinism** (docs/14 §3). A clamped scan gives a different answer from
/// an unclamped one, because the births before the clamp would have shifted
/// every later particle's birth *index*, and the index is what seeds its
/// randomness. What §3 requires is that the same project gives the same pixels
/// on every machine and every run, and a clamp at a fixed frame count does
/// exactly that. What it cannot give is the answer an infinitely patient
/// renderer would reach — and that renderer would take an hour a frame, so it is
/// not an answer anybody was going to see either.
pub const MAX_SCAN_FRAMES: i64 = 1_000_000;

/// The birth schedule: which particles exist, and when each was born
/// (particulate.md §3.1).
///
/// # In plain terms
///
/// Emit rate is "particles per second", and a frame is a sixtieth of a second,
/// so most frames are owed a *fraction* of a particle. The carry is the change
/// left over: add `rate × Δt` each frame, hand out whole particles, keep the
/// remainder. That makes the schedule a pure function of the rate curve, the
/// in point and the comp rate — no state, no history, the same answer from any
/// scrub direction — and it is the reason frame 500's particles can be
/// enumerated without rendering frame 499.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Schedule {
    /// The comp frame's length in seconds of layer time.
    dt: f64,
    /// The first frame `counts` describes, counted from the layer's in point.
    first_frame: i64,
    /// The birth index of the first particle born in that frame.
    first_birth: u64,
    /// Births per frame, oldest first.
    counts: Vec<u32>,
    /// Births in total, from the in point to the end of the last frame here.
    total: u64,
    /// Whether the walk reached the layer's in point, or started late because
    /// the layer's time was carried past [`MAX_SCAN_FRAMES`].
    exact: bool,
}

impl Schedule {
    /// Walk the rate curve from the layer's in point to `upto_frame`
    /// inclusive, recording the births of the last `window_frames` frames.
    ///
    /// `rate_at` is handed each frame's **start** in layer-time seconds and
    /// answers the Emit rate there — keyframes, expressions and driver wires
    /// already applied, which is what makes "sparks burst on the beat" one
    /// wire and no new machinery. A negative rate reads as none.
    ///
    /// O(frames since the in point) of scalar arithmetic — a 60 s comp at
    /// 60 fps is 3 600 iterations, microseconds — and a pure function of its
    /// inputs, which is what lets a caller cache it ([`ScheduleCache`]).
    pub fn scan(
        dt: f64,
        upto_frame: i64,
        window_frames: i64,
        rate_at: &dyn Fn(f64) -> f64,
    ) -> Self {
        let dt = if dt.is_finite() && dt > 0.0 { dt } else { 1.0 };
        let window = window_frames.clamp(1, MAX_WINDOW_FRAMES);
        let first_frame = (upto_frame - window + 1).max(0);
        let mut sched = Schedule {
            dt,
            first_frame,
            first_birth: 0,
            counts: Vec::new(),
            total: 0,
            exact: true,
        };
        if upto_frame < 0 {
            return sched;
        }
        // Where the walk starts. Normally the in point; on a layer whose time
        // has been carried somewhere absurd, [`MAX_SCAN_FRAMES`] back from the
        // frame being drawn — which is still far enough back to cover the whole
        // recorded window, so every particle that can be *seen* is computed
        // from a walk that reached it.
        let from = upto_frame.saturating_sub(MAX_SCAN_FRAMES).max(0);
        sched.exact = from == 0;
        sched
            .counts
            .reserve((upto_frame - first_frame + 1).clamp(0, MAX_WINDOW_FRAMES) as usize);
        let mut carry = 0.0f64;
        for f in from..=upto_frame {
            let rate = rate_at(f as f64 * dt);
            // A rate that is not a number is no rate at all; an engine crate
            // renders such a document rather than reporting it (docs/14 §4).
            let rate = if rate.is_finite() { rate.max(0.0) } else { 0.0 };
            carry += rate * dt;
            // `floor` of a carry that can never be negative, saturating so a
            // nonsense rate cannot wrap the count.
            let n = carry.floor().clamp(0.0, u32::MAX as f64) as u32;
            carry -= f64::from(n);
            if f == first_frame {
                sched.first_birth = sched.total;
            }
            if f >= first_frame {
                sched.counts.push(n);
            }
            sched.total = sched.total.saturating_add(u64::from(n));
        }
        sched
    }

    /// Births from the layer's in point to the end of the scanned range — the
    /// next birth index that would be handed out.
    #[must_use]
    pub fn total(&self) -> u64 {
        self.total
    }

    /// The comp frame's length in seconds, as the scan was given it.
    #[must_use]
    pub fn dt(&self) -> f64 {
        self.dt
    }

    /// Whether the walk reached the layer's in point.
    ///
    /// False only when the layer's own time is more than [`MAX_SCAN_FRAMES`]
    /// past its in point — four and a half hours at 60 fps, which a retime
    /// curve can reach and a timeline cannot. The particles that can be seen
    /// are still computed properly; what differs from an unclamped walk is
    /// their birth *indices*, and so their seeds.
    ///
    /// Read by the renderer so this can be said rather than left to be noticed:
    /// silent degradation is a bug (docs/14 §8).
    #[must_use]
    pub fn is_exact(&self) -> bool {
        self.exact
    }

    /// Let go of the oldest recorded frames until at most `max` births remain.
    ///
    /// **Why a scan needs a second ceiling.** The window is Life plus its
    /// jitter, and how many births fall inside it is Emit rate times that —
    /// both open-ended controls, so a rate of a million and a life of an hour
    /// is a document somebody can type and a buffer sized off it is an
    /// allocation with no ceiling (14-ENGINEERING-RULES §6). What this drops is
    /// what the **cap rule** drops first anyway: the oldest candidates, when
    /// there are already many times the cap of newer ones in play. Both render
    /// paths read the trimmed schedule, so they see one candidate set and
    /// agree.
    /// **Worked out in one pass, not one frame at a time.** The loop this
    /// replaced summed the whole `counts` vector to ask "am I under yet?" and
    /// then used `Vec::remove(0)`, which shifts everything left — so both
    /// halves were linear inside a loop that could run once per frame. With the
    /// hundred thousand frames [`MAX_WINDOW_FRAMES`] permits and a Life typed
    /// long enough to fill them, that is 10¹⁰ operations for a trim, from a
    /// number somebody can type into a control. It is a quadratic hiding behind
    /// two innocent-looking lines, which is where they usually are.
    pub fn trim_to_newest(&mut self, max: u64) {
        // Walk forward once, adding up what would be dropped, and stop at the
        // first frame that leaves the rest under the ceiling.
        let mut live = self.candidates();
        if live <= max {
            return;
        }
        let mut drop_frames = 0usize;
        let mut dropped_births = 0u64;
        // Never the last frame: a schedule with no recorded frame at all has
        // nothing to hand the renderer, and the old loop guarded that too.
        let keepable = self.counts.len().saturating_sub(1);
        for n in self.counts.iter().take(keepable) {
            if live <= max {
                break;
            }
            live = live.saturating_sub(u64::from(*n));
            dropped_births = dropped_births.saturating_add(u64::from(*n));
            drop_frames = drop_frames.saturating_add(1);
        }
        if drop_frames == 0 {
            return;
        }
        // One shift for all of them rather than one per frame.
        self.counts.drain(..drop_frames);
        self.first_frame = self
            .first_frame
            .saturating_add(i64::try_from(drop_frames).unwrap_or(i64::MAX));
        self.first_birth = self.first_birth.saturating_add(dropped_births);
    }

    /// The first frame [`counts`](Self::counts) describes, from the in point.
    #[must_use]
    pub fn first_frame(&self) -> i64 {
        self.first_frame
    }

    /// The birth index of the first particle born in that frame — candidate
    /// zero of this window.
    #[must_use]
    pub fn first_birth(&self) -> u64 {
        self.first_birth
    }

    /// Births per recorded frame, oldest first.
    ///
    /// The GPU twin walks candidates rather than births: candidate *c* is birth
    /// [`first_birth`](Self::first_birth)` + c`, and which frame owed it — and
    /// so its birth time — is a search over the running sum of this. Handing
    /// the counts over rather than a birth time per candidate is what keeps the
    /// per-frame upload the size of the *window* instead of the size of the
    /// particle set.
    #[must_use]
    pub fn counts(&self) -> &[u32] {
        &self.counts
    }

    /// How many births this window records: the candidate set's size.
    #[must_use]
    pub fn candidates(&self) -> u64 {
        self.counts.iter().map(|n| u64::from(*n)).sum()
    }

    /// Every birth the schedule recorded, **newest first**: its birth index and
    /// its birth time in layer-time seconds.
    ///
    /// Births are spread evenly inside the frame that owed them —
    /// `t_b = frame start + (j + ½)·Δt/n` — so a rate of one per frame does not
    /// stack every particle on the frame boundary.
    fn newest_first(&self) -> impl Iterator<Item = (u64, f64)> + '_ {
        // The running birth index of each recorded frame, so the reverse walk
        // does not have to sum the prefix per frame.
        let mut starts: Vec<u64> = Vec::with_capacity(self.counts.len());
        let mut b = self.first_birth;
        for n in &self.counts {
            starts.push(b);
            b = b.saturating_add(u64::from(*n));
        }
        let dt = self.dt;
        let first_frame = self.first_frame;
        (0..self.counts.len()).rev().flat_map(move |i| {
            let n = self.counts[i];
            let start = starts[i];
            let frame_start = (first_frame + i as i64) as f64 * dt;
            (0..n).rev().map(move |j| {
                let t_b = frame_start + (f64::from(j) + 0.5) * dt / f64::from(n);
                (start + u64::from(j), t_b)
            })
        })
    }
}

/// **The schedule, threaded beside the op** (points-stream.md §3.3): one
/// producer's birth scan and the layer time it was scanned for.
///
/// **In plain terms.** Everything else a kernel needs arrives in the resolved
/// parameter bag, because everything else is a number somebody typed. These two
/// are not: the layer's own clock is not a control, and the birth schedule is
/// the *whole history* of the Emit rate track rather than its value now. So
/// they ride beside the op the way a mask's flattened polyline does —
/// built once by the draw builder, which is the only place that holds a layer's
/// timing and its stored tracks, and handed to whichever render path runs.
///
/// A default one — no births, time zero — is the documented no-op: the effect
/// passes its picture through, exactly as an empty polyline does.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct PointsSchedule {
    /// The births this frame can still see.
    pub schedule: Schedule,
    /// Layer time in seconds, the moment the stream is evaluated at. The
    /// *sample* time for a sub-frame re-render, so accumulation motion blur
    /// gets true particle motion for free.
    pub t: f64,
    /// Where the composition's camera puts a particle off the layer's plane,
    /// in **px@comp** — the third thing the bag cannot carry, and for
    /// the same reason as the other two: the comp's camera and the layer's own
    /// placement are not controls on this effect. `None` is a 2D layer, and is
    /// what every project saved before the axis existed answers.
    pub projection: Option<Projection>,
    /// **The stream a wire brings in** (points-stream.md §3.3), in
    /// px@comp, newest first: entry 0 is this frame, and entry *k* is the same
    /// producer evaluated `k` steps into the past — which is how Trail looks
    /// backwards without remembering anything. Empty for a producer,
    /// and empty for a consumer with **nothing wired**, which is the documented
    /// calm: the effect draws nothing and passes its picture on.
    ///
    /// It is filled on the host rather than on the card because that is the
    /// shape the generators already established: the points a consumer
    /// stamps are bit for bit the ones the closed forms evaluated, and the
    /// driver walk reads the very same function. (ponytail: one host evaluation
    /// per sample per frame, memoised per producer. points-stream.md §3.3
    /// designs a GPU arena carriage for when a profile shows a real comp
    /// spending it.) A producer whose points depend on a picture leaves this
    /// empty and the walk fills it, see [`late`](Self::late).
    pub input: Vec<PointsStream>,
    /// Which effect in the layer's stack the wire came from, by index — folded
    /// into the frame key, and read for nothing else.
    ///
    /// The stream is a pure function of the producer's parameters, the time and
    /// the camera; the producer sits strictly earlier in the stack (the
    /// downstream-only rule), so its bag is already inside this op's
    /// cumulative key. What is *not* in that key is which of two producers the
    /// wire names, or whether it is drawn at all. An index rather than an id,
    /// so a duplicated layer still hits the per-effect cache: a key
    /// names content, never which row it came from.
    pub input_from: Option<u32>,
    /// The entries of `input` that could not be made before the render,
    /// because their points depend on a picture: which entry, and the slot in
    /// this list of the op that makes it. The walk asks that op for it on the
    /// card and fills the entry in.
    pub late: Vec<(u32, u32)>,
    /// For an effect with several Points inputs: which effect in the stack
    /// each input after the first is wired from, by index, and `u32::MAX` for
    /// none. Folded into the frame key as `input_from` is. `input` then holds
    /// one stream per input instead of a history.
    pub ports_from: Vec<u32>,
    /// For Text to points: the letters of the Text layer its row names, laid
    /// out at this frame's time. The text engine sits above this crate, so
    /// the draw builder lays them out and the effect reads them here.
    pub text: Vec<TextGlyph>,
    /// For Label points: the Text layer its row names, with `text` already
    /// the words it reads at this frame's time. Folded into the frame key.
    pub label: Option<Box<crate::model::TextDocument>>,
    /// How many texels of empty room this op's picture has on each side of
    /// the layer's art, across and down. A Shape or Text layer's picture is
    /// only as big as its art, and a point may fall anywhere, so the walk adds
    /// the room before the first op that asks and cuts it away again before
    /// one that does not. `projection` already carries the points the same
    /// distance in. Only the points effects a stack ends on are given any,
    /// and it is nought on every other layer.
    pub grow: [u32; 2],
}

/// One letter of a Text layer as it is laid out, in the layer's own px. A
/// line break is not one, and a space only when the effect asks for spaces.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TextGlyph {
    /// Where its pen starts, on the baseline.
    pub origin: [f32; 2],
    /// Where its pen ends, on the baseline.
    pub end: [f32; 2],
    /// From the baseline to the middle of the line's height. Straight up for
    /// a straight line, and turned with the curve for text on a path.
    pub up: [f32; 2],
    /// Which word it is in and which line it is on, counted from 0.
    pub word: u32,
    pub line: u32,
}

/// One producer's cached birth scan (particulate.md §3.1,
/// points-stream.md §3.3).
///
/// A single slot, because that is the shape both callers want: Particulate
/// re-scans when its own rate curve or timing changes, and the driver walk
/// memoises one stream per producer within one frame's walk. The key is the
/// caller's — a hash of exactly the scan's inputs (the rate track, the in
/// point, the comp rate) — and a key that changes is a scan that runs again.
#[derive(Debug, Default)]
pub struct ScheduleCache {
    key: Option<u64>,
    schedule: Schedule,
}

impl ScheduleCache {
    /// The schedule for `key`, scanning only when the key has changed.
    pub fn get_or_scan(&mut self, key: u64, scan: impl FnOnce() -> Schedule) -> &Schedule {
        if self.key != Some(key) {
            self.schedule = scan();
            self.key = Some(key);
        }
        &self.schedule
    }
}

/// Which per-particle draw is being made. A particle is a pure function of its
/// birth index, and these separate the dice it rolls (particulate.md §3.1).
mod attr {
    pub const EMIT_U: u32 = 0;
    pub const EMIT_V: u32 = 1;
    pub const DIRECTION: u32 = 2;
    pub const SPEED: u32 = 3;
    pub const LIFE: u32 = 4;
    pub const SIZE: u32 = 5;
    pub const TURB_PHASE: u32 = 6;
    pub const ROTATION: u32 = 7;
    /// Where in the emitter's **depth** this particle starts. A new id
    /// rather than a reuse: every earlier draw keeps the number it always drew,
    /// so a project made before the axis existed is untouched.
    pub const EMIT_W: u32 = 8;
    /// The elevation draw, out of the layer's plane.
    pub const DIRECTION_Z: u32 = 9;
}

/// The noise lattices turbulence displaces along, one per axis. Channels of the
/// **shared** value-noise core ([`value3`]) — Wiggle's and Fractal noise's own
/// lattice, not a second one (particulate.md §3.2).
const TURB_CHANNEL_X: u32 = 64;
const TURB_CHANNEL_Y: u32 = 65;
/// Turbulence's third lattice — a jitter that had x and y gains z.
const TURB_CHANNEL_Z: u32 = 66;

/// One per-particle draw in `[0, 1)`: `hash(seed, birth index, attribute)`.
///
/// Stateless and exact — the top 24 bits of an integer fold, which WGSL
/// performs identically (docs/08 §2.4).
#[must_use]
pub fn draw(seed: u32, birth: u64, attribute: u32) -> f32 {
    hash01(
        seed,
        attribute,
        birth as u32 as i32,
        (birth >> 32) as u32 as i32,
        0,
    )
}

/// A per-particle spread about `base`: `±amount` of it, never below zero.
fn jitter(base: f32, amount: f32, u: f32) -> f32 {
    (base * (1.0 + amount * (2.0 * u - 1.0))).max(0.0)
}

/// `(1 − e^(−x)) / x`, and `(1 − that) / x` — the two shapes the closed forms
/// need, written so **neither divides by the drag** (particulate.md §3.2).
///
/// The published formulas carry `g/k`, which is infinite at zero drag and
/// enormous just above it; the same algebra rearranged puts `age·r(x)` where
/// `(1 − e^(−k·age))/k` was and `age²·s(x)` where the gravity term was, and
/// both are finite everywhere.
///
/// **The guard sits at `x = 0.1`, not at particulate.md's `1e−4`.** The note
/// put it where `1 − e^(−x)` starts to cancel at all; in `f32` it has lost
/// three of its seven digits by then, so a threshold that low leaves the two
/// branches disagreeing by parts in a thousand — a visible seam in a
/// trajectory, and far past the ≤ 2 ULP the GPU twin owes this (particulate.md
/// §5). One more term of the series moves the crossing to where the two are
/// each accurate to about a part in a million, which is where they genuinely
/// meet. The formulas are the note's; only the number changed, and
/// `the_closed_forms_match_the_analytic_solutions` pins it.
fn drag_terms(x: f32) -> (f32, f32) {
    if x < 0.1 {
        // r = 1 − x/2 + x²/6 − x³/24, and s = (1 − r)/x term by term.
        let r = 1.0 - x * 0.5 + x * x / 6.0 - x * x * x / 24.0;
        let s = 0.5 - x / 6.0 + x * x / 24.0 - x * x * x / 120.0;
        (r, s)
    } else {
        let r = (1.0 - (-x).exp()) / x;
        (r, (1.0 - r) / x)
    }
}

/// Where a particle born at `p0` with speed `v0` is, `age` seconds later
/// (particulate.md §3.2) — position and speed, before turbulence.
///
/// The forces are constants over the particle's life, **sampled at the current
/// frame**: keyframe gravity and every live trajectory re-solves under the new
/// value, so the whole system leans when the keyframe lands. That is
/// physically wrong and visually right, and integrating the changing force
/// instead *is* the simulation this design excludes.
///
/// **Three axes, one algebra**: the depth component integrates under
/// exactly the same drag and wind terms as the other two. Gravity does not,
/// because gravity is `[0, g, 0]` — down is down, and a depth component would
/// be a control nobody asked for.
#[must_use]
pub fn integrate(p0: [f32; 3], v0: [f32; 3], f: &Forces, age: f32) -> ([f32; 3], [f32; 3]) {
    let k = f.drag.max(0.0);
    let x = k * age;
    let (r, s) = drag_terms(x);
    // No cancellation in this one, at any x, so it needs no branch.
    let decay = (-x).exp();
    let g = [0.0, f.gravity, 0.0];
    let mut pos = [0.0f32; 3];
    let mut vel = [0.0f32; 3];
    for i in 0..3 {
        let w = f.wind[i];
        pos[i] = p0[i] + w * age + (v0[i] - w) * age * r + g[i] * age * age * s;
        vel[i] = w + (v0[i] - w) * decay + g[i] * age * r;
    }
    (pos, vel)
}

/// Turbulence's displacement at `age` — a displacement, not an integrated
/// force, which is the standard trick that keeps it closed form
/// (particulate.md §3.2).
fn turbulence(p0: [f32; 3], phase: f32, f: &Forces, seed: u32, age: f32) -> [f32; 3] {
    if f.turbulence == 0.0 {
        return [0.0; 3];
    }
    let scale = f.turbulence_scale.max(1e-3);
    let (qx, qy) = (p0[0] / scale + phase, p0[1] / scale + phase);
    let z = age * f.turbulence_speed;
    // The lattice is sampled at the birth point's own x and y, as it always
    // was: a third *input* coordinate would move every existing sample and
    // repaint every project. The third *output* is a third channel of the same
    // lattice, which is what "a jitter gains z where x and y have one" means.
    [
        f.turbulence * value3(seed, TURB_CHANNEL_X, qx, qy, z, 0),
        f.turbulence * value3(seed, TURB_CHANNEL_Y, qx, qy, z, 0),
        f.turbulence * value3(seed, TURB_CHANNEL_Z, qx, qy, z, 0),
    ]
}

/// Where in the emitter this particle starts.
///
/// `w` is the draw through the emitter's [`depth`](Emitter::depth),
/// filled uniformly: a Point becomes a segment, an Ellipse a cylinder and a
/// Rectangle a box. Line and Mask path ignore it and stay on the plane.
///
/// `path` is **the polyline this shape walks**: the layer's flattened mask for
/// a Mask path emitter, and the emitter's own local outline for the two outline
/// shapes. One argument, because both are the same question — where is
/// `u` of the way along? — answered by the same arc-length walk.
fn birth_point(e: &Emitter, path: &MaskPolyline, u: f32, v: f32, w: f32) -> Option<[f32; 3]> {
    let (s, c) = e.angle_deg.to_radians().sin_cos();
    let (local, depth) = match e.shape {
        EmitterShape::Point => ([0.0, 0.0], (w - 0.5) * e.depth),
        // The outline walks its own local polyline and is then turned and
        // placed exactly as the interior it hollows out would have been, so an
        // Ellipse and an Ellipse outline sit in the same place at the same
        // angle. Depth fills as before: the cylinder becomes a tube.
        EmitterShape::EllipseOutline | EmitterShape::RectangleOutline => {
            if path.is_empty() {
                return None;
            }
            (path.point_at(u * path.length()), (w - 0.5) * e.depth)
        }
        EmitterShape::Line => ([(u - 0.5) * e.width, 0.0], 0.0),
        EmitterShape::Ellipse => {
            // √u for the radius, or the middle of the disc would be crowded:
            // area grows as r², so a uniform radius is not a uniform fill.
            let r = u.max(0.0).sqrt();
            let (sa, ca) = (v * std::f32::consts::TAU).sin_cos();
            (
                [0.5 * e.width * r * ca, 0.5 * e.height * r * sa],
                (w - 0.5) * e.depth,
            )
        }
        EmitterShape::Rectangle => (
            [(u - 0.5) * e.width, (v - 0.5) * e.height],
            (w - 0.5) * e.depth,
        ),
        EmitterShape::MaskPath => {
            // The mask's own line, by arc length. Nothing to walk is
            // the documented no-op: no path, no particles.
            if path.is_empty() {
                return None;
            }
            let p = path.point_at(u * path.length());
            // Already an absolute position, so the emitter's own rotation and
            // position do not move it — the path is where the user drew it —
            // and it stays on the layer's plane, at the depth the emitter sits
            // at.
            return Some([p[0], p[1], e.position[2]]);
        }
    };
    Some([
        e.position[0] + local[0] * c - local[1] * s,
        e.position[1] + local[0] * s + local[1] * c,
        e.position[2] + depth,
    ])
}

/// The stream at layer time `t`: every live particle, in birth-index order
/// (particulate.md §3.2–§3.3).
///
/// `path` is the flattened mask path for a Mask path emitter, and
/// [`MaskPolyline::default`] — an empty one — for every other shape.
///
/// **Units are the caller's.** Hand it px@comp parameters and the stream is
/// px@comp, which is what a consumer reading it as data wants; hand it the
/// raster-scaled bag an effect resolves to and the stream is in raster pixels,
/// which is what the draw wants. The formulas are the same either way, which
/// is why there is one of them.
#[must_use]
pub fn evaluate(p: &PointsParams, sched: &Schedule, t: f64, path: &MaskPolyline) -> PointsStream {
    evaluate_with_tail(p, sched, t, path, 0.0).0
}

/// [`evaluate`], and beside it **where each particle was `tail` seconds ago** —
/// what Streak mode draws its capsule back to (particulate.md §2, Render).
///
/// The tail is deliberately *not* a field of [`PointsStream`]: the stream's
/// layout is the finalised one consumers read as data (particulate.md §4), and
/// a tail is a fact about one render mode, not about a particle. It rides
/// beside, in the same order, exactly as the GPU's own tail buffer does.
///
/// A tail of zero — every mode but Streak — is the head, so the capsule
/// degenerates to the disc and the three modes really are one kernel.
#[must_use]
pub fn evaluate_with_tail(
    p: &PointsParams,
    sched: &Schedule,
    t: f64,
    path: &MaskPolyline,
    tail: f32,
) -> (PointsStream, Vec<[f32; 3]>) {
    let mut out = PointsStream {
        projection: p.projection,
        ..PointsStream::default()
    };
    let mut tails: Vec<[f32; 3]> = Vec::new();
    let cap = p.cap.min(CAP_HARD as u32) as usize;
    if cap == 0 {
        return (out, tails);
    }
    let e = &p.emitter;
    let look = &p.particle;
    // The polyline this emitter walks: the layer's mask for a Mask path
    // shape, the emitter's own outline for the two outline shapes, and nothing
    // at all for the interiors — one flattening, done once for the whole
    // evaluation rather than per particle.
    let outline = outline_polyline(e);
    let walk = if e.shape.is_outline() { &outline } else { path };
    // The turbulence rate of change, by central difference at a **fixed** ε —
    // fixed, so one frame key names one picture whatever raster or refresh the
    // preview happens to be running at (particulate.md §3.2).
    let eps = (sched.dt() as f32 * 0.5).max(1e-6);
    let spread = e.spread_deg.to_radians();
    let base_dir = e.direction_deg.to_radians();
    let spread_z = e.spread_z_deg.to_radians();
    let base_dir_z = e.direction_z_deg.to_radians();

    for (b, t_b) in sched.newest_first() {
        let age = (t - t_b) as f32;
        // Not yet born. The walk is newest first, so its neighbours are no
        // more born than it is — but a frame's births are spread inside the
        // frame, so this is a skip and not a stop.
        if age < 0.0 {
            continue;
        }
        let life = jitter(look.life, look.life_jitter, draw(p.seed, b, attr::LIFE));
        if life <= 0.0 || age >= life {
            continue;
        }
        let Some(p0) = birth_point(
            e,
            walk,
            draw(p.seed, b, attr::EMIT_U),
            draw(p.seed, b, attr::EMIT_V),
            draw(p.seed, b, attr::EMIT_W),
        ) else {
            // A Mask path emitter with nothing to walk emits nothing at all,
            // and no later birth will find a path either.
            break;
        };
        let dir = base_dir + (draw(p.seed, b, attr::DIRECTION) - 0.5) * spread;
        let dir_z = base_dir_z + (draw(p.seed, b, attr::DIRECTION_Z) - 0.5) * spread_z;
        let speed = jitter(e.speed, e.speed_jitter, draw(p.seed, b, attr::SPEED));
        let (sd, cd) = dir.sin_cos();
        // The elevation tilts the launch out of the plane. At nought — every
        // project saved before the axis existed — `cos` is exactly 1 and `sin`
        // exactly 0, so the two in-plane components are the bits they always
        // were.
        let (sz, cz) = dir_z.sin_cos();
        let v0 = [speed * cd * cz, speed * sd * cz, speed * sz];
        let (pos, vel) = integrate(p0, v0, &p.forces, age);
        let phase = draw(p.seed, b, attr::TURB_PHASE) * 1000.0;
        let d = turbulence(p0, phase, &p.forces, p.seed, age);
        let d_next = turbulence(p0, phase, &p.forces, p.seed, age + eps);
        let d_prev = turbulence(p0, phase, &p.forces, p.seed, age - eps);
        let u = (age / life).clamp(0.0, 1.0);
        let speed_out = [
            vel[0] + (d_next[0] - d_prev[0]) / (2.0 * eps),
            vel[1] + (d_next[1] - d_prev[1]) / (2.0 * eps),
            vel[2] + (d_next[2] - d_prev[2]) / (2.0 * eps),
        ];
        let size = jitter(look.size, look.size_jitter, draw(p.seed, b, attr::SIZE))
            * curve_at(u, &look.size_curve);
        let alpha = look.colour[3] * curve_at(u, &look.opacity_curve);
        // Premultiplied, and the blend to End colour is in working space —
        // which is what "in working space" costs: one lerp, no encode.
        let mut colour = [alpha; 4];
        for (c, (from, to)) in colour
            .iter_mut()
            .zip(look.colour.iter().zip(look.end_colour.iter()))
            .take(3)
        {
            *c = (from + (to - from) * u) * alpha;
        }
        // The per-particle rotation spread: a uniform draw of ±half the
        // dial about Rotation, from the seed hash like every other die, so two
        // sprites born together do not point the same way.
        let spread_rot =
            (draw(p.seed, b, attr::ROTATION) - 0.5) * look.rotation_jitter_deg.to_radians();
        // Align to motion follows the motion **in the layer's plane**: a
        // rotation is one angle in the picture the sprite is stamped into, and
        // the depth component of the speed is not an angle that picture has.
        let rotation = if look.align_to_motion {
            speed_out[1].atan2(speed_out[0]) + look.rotation_deg.to_radians()
        } else {
            look.rotation_deg.to_radians()
        } + spread_rot
            + look.spin_deg.to_radians() * age;

        // Where it was a Streak length ago — the same closed form at an earlier
        // age, never a remembered position. Clamped at birth: a particle
        // younger than the tail streaks from where it was born.
        if tail > 0.0 {
            let back = (age - tail).max(0.0);
            let (bp, _) = integrate(p0, v0, &p.forces, back);
            let bd = turbulence(p0, phase, &p.forces, p.seed, back);
            tails.push([bp[0] + bd[0], bp[1] + bd[1], bp[2] + bd[2]]);
        }
        out.position
            .push([pos[0] + d[0], pos[1] + d[1], pos[2] + d[2]]);
        out.speed.push(speed_out);
        out.age.push(age);
        out.life.push(life);
        out.size.push(size);
        out.rotation.push(rotation);
        out.colour.push(colour);
        out.id.push(b);
        // **The cap rule**: over budget, the newest `cap` by birth
        // index survive. Walking newest first makes that a stop rather than a
        // sort, and old particles vanishing early under overload is visible,
        // deterministic and the same from any scrub direction.
        if out.len() >= cap {
            break;
        }
    }

    // Birth-index ascending, which is the order the GPU's prefix-sum
    // compaction produces and the order `id` is promised in (particulate.md
    // §5). Reversing a walk that ran newest-first is the whole of it.
    out.position.reverse();
    out.speed.reverse();
    out.age.reverse();
    out.life.reverse();
    out.size.reverse();
    out.rotation.reverse();
    out.colour.reverse();
    out.id.reverse();
    tails.reverse();
    (out, tails)
}

/// The CPU reference draw: a feathered disc per particle, over the picture
/// (docs/08 §1.6).
///
/// **In plain terms.** Each particle is stamped as a soft round dab into the
/// buffer, oldest first so newer particles land on top — the same order, and
/// so the same picture, as the instanced quads the GPU draws. `feather` is
/// `0..=1`: at 0 the disc has a hard edge (with the half-pixel ramp any
/// rasteriser needs so it is not jagged), at 1 it fades out from its own
/// centre.
///
/// A dab and not a full-frame pass: a particle covers a few dozen pixels, and
/// visiting two million of them per particle to find that out is the shape a
/// reference implementation cannot afford even as an oracle.
pub fn draw_discs(rgba: &mut [f32], w: u32, h: u32, s: &PointsStream, feather: f32) {
    draw_stream(
        rgba,
        w,
        h,
        s,
        &[],
        &DrawStyle {
            mode: RenderMode::Disc,
            feather,
            streak_seconds: 0.0,
            mix: 1.0,
        },
        None,
    );
}

/// The picture the Sprite mode stamps: the referenced layer's frame, linear
/// premultiplied RGBA, `w × h` (layer-input.md).
#[derive(Debug, Clone, Copy)]
pub struct Sprite<'a> {
    pub rgba: &'a [f32],
    pub w: u32,
    pub h: u32,
}

/// How a sprite sits on its point. The default is what Particulate stamps: a
/// square of the point's size with the whole picture across it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SpriteFit {
    /// The stamp's width and height, as multiples of the point's size.
    pub unit: [f32; 2],
    /// The place on the stamp that sits on the point, 0 to 1 from its top
    /// left corner. The stamp turns about it.
    pub anchor: [f32; 2],
    /// How many times the picture's width and height the stamp is. Above 1
    /// leaves the stamp empty round the picture, below 1 cuts the picture off.
    pub uv: [f32; 2],
    /// Corner radius, px on the stamp of a point whose size is 100.
    pub corner: f32,
}

impl Default for SpriteFit {
    fn default() -> Self {
        Self {
            unit: [1.0; 2],
            anchor: [0.5; 2],
            uv: [1.0; 2],
            corner: 0.0,
        }
    }
}

/// The shortest distance from `p` to the segment `a`–`b`.
///
/// The one shape all three modes share: with `a == b` it is the distance to a
/// point, which is why a disc is a streak of no length and the kernel does not
/// branch between them.
fn seg_distance(p: [f32; 2], a: [f32; 2], b: [f32; 2]) -> f32 {
    let (ex, ey) = (b[0] - a[0], b[1] - a[1]);
    let len2 = ex * ex + ey * ey;
    let t = if len2 > 0.0 {
        (((p[0] - a[0]) * ex + (p[1] - a[1]) * ey) / len2).clamp(0.0, 1.0)
    } else {
        0.0
    };
    (p[0] - a[0] - ex * t).hypot(p[1] - a[1] - ey * t)
}

/// How much of a pixel a capsule covers, `far` from its spine, before the
/// clamp to 0..=1.
///
/// From a radius of 1 px up this is the soft edge every dab has. Under that
/// the edge's half pixel of ramp eats the line: one 1 px wide lying between
/// two pixel rows is half a pixel from both, and drew nothing. So a thin
/// capsule is measured as the share of the pixel its width fills. That puts
/// half its light in each row there and all of it in one row when it lies on
/// a pixel centre, and a line thinner than a pixel carries its share. The two
/// are mixed between 1 px and 2 px wide, so a width can animate across
/// without a jump.
fn capsule_cover(radius: f32, edge: f32, far: f32) -> f32 {
    let soft = (radius - far) / edge;
    if radius >= 1.0 {
        return soft;
    }
    let boxed = (radius + 0.5 - far).clamp(0.0, 1.0).min(2.0 * radius);
    let blend = (2.0 * radius - 1.0).clamp(0.0, 1.0);
    boxed + (soft.clamp(0.0, 1.0) - boxed) * blend
}

/// Whether an entry produces a points stream, and so wants the birth schedule
/// threaded beside its op (points-stream.md §3.3).
///
/// **One predicate, so one order.** The draw builder fills a schedule per op
/// that answers yes and the render walk consumes one per op that answers yes;
/// two rules spelled two ways would hand a schedule to whichever effect
/// happened to sit above. It lives here rather than on `Signature` because the
/// rule belongs to this programme, not to the shape of a registry entry.
#[must_use]
pub fn wants_schedule(sig: crate::fx::Signature) -> bool {
    sig.outputs()
        .iter()
        .any(|p| p.ty == crate::fx::PortType::Points)
}

/// Whether an entry **reads** a points stream off a wire: the other
/// half of the family, and the other reason to want a carriage.
#[must_use]
pub fn consumes_points(sig: crate::fx::Signature) -> bool {
    sig.inputs()
        .iter()
        .any(|p| p.ty == crate::fx::PortType::Points)
}

/// Whether an entry wants a [`PointsSchedule`] threaded beside its op at all —
/// because it produces points, consumes them, or both.
///
/// **One predicate, so one order**, which is why this exists as a function
/// rather than as an `||` written out at each site: the draw builder fills a
/// carriage per op that answers yes and the render walk consumes one per op
/// that answers yes, and two rules spelled two ways would hand a carriage to
/// whichever effect happened to sit above.
#[must_use]
pub fn wants_carriage(sig: crate::fx::Signature) -> bool {
    wants_schedule(sig) || consumes_points(sig)
}

/// A mask path in the raster the frame is being drawn at.
///
/// A polyline is flattened in **px@comp** — deliberately, so the same document
/// gives the same curve at any preview divisor — and every other
/// distance the closed forms read has already been multiplied by the raster
/// factor on its way through the bag. One place does the same to the path, so
/// the two render paths cannot come to scale it differently.
#[must_use]
pub fn scale_path(poly: &MaskPolyline, px_scale: f32) -> MaskPolyline {
    let k = px_scale.max(1e-6);
    MaskPolyline {
        points: poly.points.iter().map(|p| [p[0] * k, p[1] * k]).collect(),
        arc: poly.arc.iter().map(|a| a * k).collect(),
        closed: poly.closed,
        // Both are px@comp distances like the points themselves, so they take
        // the same factor — a scaled path whose feather stayed put would soften
        // by a different amount at every preview divisor.
        feather: poly.feather * k,
        expansion: poly.expansion * k,
        corners: poly.corners.clone(),
    }
}

/// One bilinear tap into a sprite, clamped at the edges.
///
/// Written as four `textureLoad`-shaped reads and three lerps rather than as a
/// sampler call, because the WGSL twin does exactly this: a hardware sampler's
/// filtering precision is the driver's business, and a stamped sprite is
/// compared against this function.
fn sprite_tap(sp: &Sprite<'_>, u: f32, v: f32) -> [f32; 4] {
    let last_x = sp.w.saturating_sub(1) as f32;
    let last_y = sp.h.saturating_sub(1) as f32;
    let x = (u * sp.w as f32 - 0.5).clamp(0.0, last_x);
    let y = (v * sp.h as f32 - 0.5).clamp(0.0, last_y);
    let (x0, y0) = (x.floor(), y.floor());
    let (fx, fy) = (x - x0, y - y0);
    let texel = |ix: f32, iy: f32| -> [f32; 4] {
        let ix = (ix.min(last_x) as u32).min(sp.w.saturating_sub(1));
        let iy = (iy.min(last_y) as u32).min(sp.h.saturating_sub(1));
        let d = ((iy * sp.w + ix) * 4) as usize;
        sp.rgba
            .get(d..d + 4)
            .map_or([0.0; 4], |t| [t[0], t[1], t[2], t[3]])
    };
    let (a, b) = (texel(x0, y0), texel(x0 + 1.0, y0));
    let (c, d) = (texel(x0, y0 + 1.0), texel(x0 + 1.0, y0 + 1.0));
    let mut out = [0.0f32; 4];
    for k in 0..4 {
        let top = a[k] + (b[k] - a[k]) * fx;
        let bot = c[k] + (d[k] - c[k]) * fx;
        out[k] = top + (bot - top) * fy;
    }
    out
}

/// The CPU reference draw for all three render modes, over the picture
/// (docs/08 §1.6).
///
/// **In plain terms.** Each particle is stamped into the buffer, oldest first
/// so newer particles land on top — the same order, and so the same picture, as
/// the instanced quads the GPU draws. What is stamped depends on the mode:
///
/// - **Disc**: a soft round dab. `feather` is `0..=1` — at 0 the edge is hard
///   (with the half-pixel ramp any rasteriser needs so it is not jagged), at 1
///   it fades out from its own centre.
/// - **Streak**: the same dab swept along the line from `tails[i]` to the
///   particle's position — a capsule, and exactly the disc when the tail is the
///   head, which is why one distance function serves both.
/// - **Sprite**: the referenced layer's picture in a square of the particle's
///   own size, turned by its rotation and tinted by its colour. **An unset
///   sprite draws discs** — a render mode must always draw something
///   (particulate.md §2).
///
/// A dab and not a full-frame pass: a particle covers a few dozen pixels, and
/// visiting two million of them per particle to find that out is the shape a
/// reference implementation cannot afford even as an oracle.
/// - **Depth**: every particle is put through the stream's own
///   [`Projection`] first — where the composition's camera sees it, and how
///   much smaller or larger for being further off or nearer. On a 2D layer the
///   projection is flat and this is exactly the arithmetic it always was.
pub fn draw_stream(
    rgba: &mut [f32],
    w: u32,
    h: u32,
    s: &PointsStream,
    tails: &[[f32; 3]],
    style: &DrawStyle,
    sprite: Option<Sprite<'_>>,
) {
    draw_stream_fit(rgba, w, h, s, tails, style, sprite, &SpriteFit::default());
}

/// [`draw_stream`], with a say in how a sprite sits on its point. Clone to
/// points' reference draw. `fit` is unread in the other two modes.
#[allow(clippy::too_many_arguments)]
pub fn draw_stream_fit(
    rgba: &mut [f32],
    w: u32,
    h: u32,
    s: &PointsStream,
    tails: &[[f32; 3]],
    style: &DrawStyle,
    sprite: Option<Sprite<'_>>,
    fit: &SpriteFit,
) {
    let feather = style.feather.clamp(0.0, 1.0);
    let mix = style.mix.clamp(0.0, 1.0);
    // Sprite with nothing to stamp falls back to the disc, here and in the
    // kernel, rather than the effect going quietly no-op.
    let sprite = sprite.filter(|sp| {
        style.mode == RenderMode::Sprite && sp.w > 0 && sp.h > 0 && !sp.rgba.is_empty()
    });
    for i in 0..s.len() {
        let (Some(&head3), Some(&size), Some(&colour)) =
            (s.position.get(i), s.size.get(i), s.colour.get(i))
        else {
            continue;
        };
        // Through the camera, if there is one. `depth` is nought for a particle
        // at or behind the camera's plane, which makes the radius nought and
        // draws nothing — the degenerate case degrades (docs/14 §4).
        let (head, depth) = s.projection.apply(head3);
        let size = size * depth;
        let radius = size * 0.5;
        if radius <= 0.0 || colour[3] <= 0.0 || mix <= 0.0 {
            continue;
        }
        // The host Mix, folded into the source's coverage. For a premultiplied
        // `over` that is the dissolve exactly, so no second pass runs.
        let src = [
            colour[0] * mix,
            colour[1] * mix,
            colour[2] * mix,
            colour[3] * mix,
        ];
        // The tail is a position like any other, so it goes through the same
        // camera: a streak that runs towards the lens really does run towards
        // the lens.
        let tail = tails.get(i).map_or(head, |t| s.projection.apply(*t).0);
        let (rot_s, rot_c) = s.rotation.get(i).copied().unwrap_or(0.0).sin_cos();
        let stretch = s.stretch_of(i);
        // A stretched dot is an ellipse in the point's own turned frame. A
        // capsule keeps its round ends.
        let oval = (stretch != [1.0; 2] && tail == head).then_some(stretch);
        let widest = stretch[0].max(stretch[1]);
        // A turned stamp reaches as far as its furthest corner from the
        // point, a capsule its radius past either end.
        let far = |k: usize| size * stretch[k] * fit.unit[k] * (0.5 + (fit.anchor[k] - 0.5).abs());
        let reach = if sprite.is_some() {
            far(0).hypot(far(1))
        } else if oval.is_some() {
            radius * widest
        } else {
            radius
        };
        let lo_x = head[0].min(tail[0]) - reach;
        let hi_x = head[0].max(tail[0]) + reach;
        let lo_y = head[1].min(tail[1]) - reach;
        let hi_y = head[1].max(tail[1]) + reach;
        let x0 = (lo_x.floor().max(0.0) as u32).min(w);
        let x1 = ((hi_x.ceil().max(0.0) as u32).saturating_add(1)).min(w);
        let y0 = (lo_y.floor().max(0.0) as u32).min(h);
        let y1 = ((hi_y.ceil().max(0.0) as u32).saturating_add(1)).min(h);
        // Half a pixel of ramp even at no feather: an edge that lands between
        // two pixel centres has to be shared between them or the disc crawls.
        let edge = (feather * radius).max(0.5);
        for y in y0..y1 {
            for x in x0..x1 {
                let p = [x as f32 + 0.5, y as f32 + 0.5];
                let contrib = match &sprite {
                    Some(sp) => {
                        let (dx, dy) = (p[0] - head[0], p[1] - head[1]);
                        // Into the sprite's own frame: undo the particle's turn,
                        // then measure across a square of its size.
                        let lx = dx * rot_c + dy * rot_s;
                        let ly = -dx * rot_s + dy * rot_c;
                        let (u, v) = (
                            lx / (size * stretch[0] * fit.unit[0]) + 0.5 + (fit.anchor[0] - 0.5),
                            ly / (size * stretch[1] * fit.unit[1]) + 0.5 + (fit.anchor[1] - 0.5),
                        );
                        if !(0.0..=1.0).contains(&u) || !(0.0..=1.0).contains(&v) {
                            continue;
                        }
                        // Rounded corners, measured on the stamp of a point
                        // whose size is 100 so they turn and scale with it.
                        let mut cover = 1.0;
                        if fit.corner > 0.0 {
                            let b = [50.0 * fit.unit[0], 50.0 * fit.unit[1]];
                            let r = fit.corner.min(b[0]).min(b[1]);
                            let d = [
                                (u - 0.5).abs() * 100.0 * fit.unit[0] - (b[0] - r),
                                (v - 0.5).abs() * 100.0 * fit.unit[1] - (b[1] - r),
                            ];
                            if d[0] > 0.0 && d[1] > 0.0 {
                                // Half a pixel of ramp, as a disc's edge has.
                                let px = size * stretch[0].min(stretch[1]) / 100.0;
                                cover = ((r - d[0].hypot(d[1])) * px + 0.5).clamp(0.0, 1.0);
                            }
                        }
                        // More or less of the picture across the stamp. Past
                        // the picture's own edge the stamp is empty.
                        let (tu, tv) = (
                            u + (u - 0.5) * (fit.uv[0] - 1.0),
                            v + (v - 0.5) * (fit.uv[1] - 1.0),
                        );
                        if !(0.0..=1.0).contains(&tu) || !(0.0..=1.0).contains(&tv) {
                            continue;
                        }
                        let t = sprite_tap(sp, tu, tv);
                        // Both are premultiplied, so the tint is the plain
                        // product: the sprite's own colour times the
                        // particle's, its alpha times the particle's.
                        [
                            t[0] * src[0] * cover,
                            t[1] * src[1] * cover,
                            t[2] * src[2] * cover,
                            t[3] * src[3] * cover,
                        ]
                    }
                    None => {
                        let cov = match oval {
                            Some(st) => {
                                let (dx, dy) = (p[0] - head[0], p[1] - head[1]);
                                let lx = (dx * rot_c + dy * rot_s) / st[0];
                                let ly = (-dx * rot_s + dy * rot_c) / st[1];
                                (radius - lx.hypot(ly)) * st[0].min(st[1]) / edge
                            }
                            // A dot keeps its soft edge whatever its size.
                            None if tail == head => (radius - seg_distance(p, tail, head)) / edge,
                            None => capsule_cover(radius, edge, seg_distance(p, tail, head)),
                        }
                        .clamp(0.0, 1.0);
                        if cov <= 0.0 {
                            continue;
                        }
                        [src[0] * cov, src[1] * cov, src[2] * cov, src[3] * cov]
                    }
                };
                if contrib[3] <= 0.0 && contrib[0] <= 0.0 && contrib[1] <= 0.0 && contrib[2] <= 0.0
                {
                    continue;
                }
                let d = ((y * w + x) * 4) as usize;
                let Some(dst) = rgba.get_mut(d..d + 4) else {
                    continue;
                };
                // Premultiplied `over`: the particle's own colour, and what was
                // there kept by however much of the pixel it did not cover.
                let keep = 1.0 - contrib[3];
                for c in 0..4 {
                    dst[c] = contrib[c] + dst[c] * keep;
                }
            }
        }
    }
}

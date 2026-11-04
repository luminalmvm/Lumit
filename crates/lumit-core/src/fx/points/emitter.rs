//! Where Particulate's particles are born: the emitter's shapes and the
//! point in one a particle starts from.

use crate::mask::MaskPolyline;

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
    /// Uniformly through a box of `width` × `height` × `depth`.
    Box,
    /// Uniformly through a ball as wide as `width`.
    Sphere,
    /// Over the whole frame, where another layer's picture is bright, and on
    /// that layer's plane when the particles are in the scene.
    Layer,
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
        "Box",
        "Sphere",
        "Layer",
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
            7 => EmitterShape::Box,
            8 => EmitterShape::Sphere,
            9 => EmitterShape::Layer,
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
    /// px@comp (or raster px — the caller's own scale; see [`super::evaluate`]).
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
///
/// `frame` is the picture's width and height, which a Layer emitter fills.
pub fn birth_point(
    e: &Emitter,
    path: &MaskPolyline,
    frame: [f32; 2],
    [u, v, w]: [f32; 3],
) -> Option<[f32; 3]> {
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
        EmitterShape::Rectangle | EmitterShape::Box => (
            [(u - 0.5) * e.width, (v - 0.5) * e.height],
            (w - 0.5) * e.depth,
        ),
        EmitterShape::Sphere => {
            // The cube root for the radius, as volume grows as r³, and an even
            // height up the ball for an even spread over its surface.
            let r = 0.5 * e.width * u.max(0.0).cbrt();
            let up = 1.0 - 2.0 * v;
            let round = (1.0 - up * up).max(0.0).sqrt();
            let (sa, ca) = (w * std::f32::consts::TAU).sin_cos();
            ([r * round * ca, r * round * sa], r * up)
        }
        // With no picture to fill, nothing is born.
        EmitterShape::Layer if frame[0] <= 0.0 || frame[1] <= 0.0 => return None,
        EmitterShape::Layer => return Some([u * frame[0], v * frame[1], e.position[2]]),
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

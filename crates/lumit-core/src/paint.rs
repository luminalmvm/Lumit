//! Paint strokes: brush, eraser and clone stamp marks on a layer
//! (docs/03-DATA-MODEL.md §7.1, docs/impl/paint.md), plus the rasteriser that
//! stamps them into the layer's pixels.
//!
//! # In plain terms
//!
//! A paint stroke is a line you drew on a layer with a round brush. The document
//! keeps *what you drew* — the path your pointer took, in the layer's own
//! coordinates, with the colour, the width, how soft the edge is and how opaque
//! the mark is — and never the pixels. That is the whole point: a stroke is
//! re-stamped at whatever resolution the frame is being rendered at, so painting
//! at a quarter preview and exporting at full size gives a full-size stroke
//! rather than a blurry quarter-size one, and every setting stays changeable
//! afterwards.
//!
//! Three kinds of mark, all the same shape of thing:
//!
//! * **Paint** lays the colour down.
//! * **Erase** takes alpha away, so the layer becomes see-through where you
//!   brushed.
//! * **Clone** copies from somewhere else on the *same* layer, offset by a fixed
//!   distance you set before painting — the stamp everybody uses to paint out a
//!   boom or a blemish.
//!
//! **Where it happens in the picture.** Strokes are stamped into the layer's own
//! raster, before its masks gate it and before its effects run, which is what
//! makes "mask off the part I painted" and "blur what I painted" both mean the
//! obvious thing.
//!
//! **A stroke blends.** `blend` is the layer blend list — the same
//! words on the same maths, through the one shared kernel — deciding what
//! colour the mark lays down. It never changes how the mark *covers*, and an
//! eraser ignores it, having no colour to combine.
//!
//! **A stroke can be pressed.** `pressures` is how hard the stylus was
//! pressed at each point, and the only thing it changes is the *width* of the
//! dab stamped there. An empty list is the constant 1.0 — every mouse-drawn
//! stroke, and everything painted before there was a stylus to read — so it
//! costs an old project neither a byte in the file nor a pixel on the screen.
//!
//! **What is deliberately not here.** Tilt, spacing curves and any GPU path.
//! Tilt needs a brush tip that can *turn*, and there is deliberately no angle
//! anywhere in [`BrushShape`]; angling a round dab would be a lie and angling a
//! square one is the brush-tip system this module refused. Each is a real
//! feature; none of them changes the shape of what is stored, which is what
//! this first cut is for.
//!
//! **A stroke can draw itself on.** `start` and `end` are a per cent of
//! the stroke's own length, animatable like any other property: hold Start at 0,
//! key End from 0 to 100, and the mark appears as if it were being made. The
//! trim is a walk along the polyline by arc length — per cent of *length*, not
//! of the samples, so a write-on runs at an even speed whatever the hand that
//! drew it was doing.
//!
//! The brush tip itself is round or square ([`BrushShape`]) and softens
//! by one hardness ramp either way. That is deliberately not a brush-*tip*
//! system: no bitmap, no angle, no roundness, and no room made for one.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::anim::{Animation, Property};
use crate::model::{BlendMode, LinearColour};
use crate::pixels::over;

/// What a stroke does to the pixels under it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum PaintMode {
    /// Lay the stroke's colour down.
    #[default]
    Paint,
    /// Take alpha away.
    Erase,
    /// Copy from elsewhere on the same layer, by [`PaintStroke::clone_offset`].
    Clone,
}

/// The shape one dab of the brush leaves.
///
/// Not a brush-tip system — there is no bitmap, no angle and no roundness, and
/// there is deliberately no room for one here. Two shapes, both measured from
/// the dab's centre out to [`PaintStroke::width`] ÷ 2, and both softened by the
/// same [`PaintStroke::hardness`] ramp: what changes is only *how the distance
/// to the centre is measured*.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum BrushShape {
    /// The usual brush: distance is the straight line to the centre, so the
    /// mark is a circle.
    #[default]
    Round,
    /// Distance is the greater of the two axes, so the mark is a square with
    /// flat sides and square corners — the chisel that draws a clean rectangular
    /// patch, and the tip a clone stamp wants when it is copying architecture.
    Square,
}

impl BrushShape {
    /// True for the shape every stroke had before there was a choice — the one
    /// left out of the file entirely, so a project nobody has re-shaped writes
    /// exactly the bytes it wrote yesterday.
    fn is_round(&self) -> bool {
        matches!(self, BrushShape::Round)
    }
}

/// True for the blend every stroke had before there was a choice — left out of
/// the file entirely, so an unblended stroke writes the bytes it always wrote.
fn is_normal(mode: &BlendMode) -> bool {
    matches!(mode, BlendMode::Normal)
}

/// serde default for [`PaintStroke::end`] and for a shape item's Trim end
/// ([`crate::shape::ShapeItem`]): the whole path.
pub(crate) fn full() -> Property {
    Property::fixed(100.0)
}

/// True for a still 0 — [`PaintStroke::start`]'s default, and so the thing left
/// out of the file entirely. Shape items' trims default the same way.
pub(crate) fn is_static_zero(p: &Property) -> bool {
    matches!(p.animation, Animation::Static(v) if v == 0.0) && p.extra.is_empty()
}

/// True for a still 100 — [`PaintStroke::end`]'s default, left out for the same
/// reason: a stroke nobody has trimmed writes the bytes it always wrote.
pub(crate) fn is_static_full(p: &Property) -> bool {
    matches!(p.animation, Animation::Static(v) if v == 100.0) && p.extra.is_empty()
}

/// One stroke: the path the pointer took and how it was painted.
///
/// The path is a **polyline** rather than a bezier: it is a record of a gesture,
/// sampled as it happened, not a shape anyone will edit vertex by vertex. Masks
/// and shape layers are the bezier things; a stroke is a stroke.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PaintStroke {
    pub id: Uuid,
    pub name: String,
    /// The pointer's path in **layer** coordinates, in the order it was drawn.
    /// One point is a dab; two or more are joined by round-capped segments.
    pub points: Vec<(f64, f64)>,
    /// How hard the stylus was pressed at each point, 0..1, parallel to
    /// [`points`](Self::points).
    ///
    /// **Empty is the whole of the compatibility story**: an empty list is the
    /// constant 1.0 everybody painted with, so a mouse-drawn stroke — and every
    /// stroke drawn before there was a stylus to read — writes exactly the
    /// bytes it wrote yesterday and stamps exactly the pixels it stamped. A
    /// short or overlong list is read the same way point by point: a missing
    /// entry is 1.0, rather than an error nobody could act on.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub pressures: Vec<f64>,
    pub colour: LinearColour,
    /// The brush's **diameter** in layer pixels.
    pub width: f64,
    /// 0 = fully soft (fades from the centre out), 1 = a hard edge with only
    /// enough falloff left to keep it from stair-stepping. The same ramp
    /// whatever the [`shape`](Self::shape) is.
    pub hardness: f64,
    /// Round (the default, and what every stroke was before brush shapes
    /// existed) or square.
    #[serde(default, skip_serializing_if = "BrushShape::is_round")]
    pub shape: BrushShape,
    /// 0..100, like every other opacity in the document.
    pub opacity: f64,
    /// Where along the path the mark **begins**, as a per cent of the stroke's
    /// own length, and where it **ends**. Animatable, and the pair is
    /// the whole of write-on: hold `start` at 0, key `end` from 0 to 100, and
    /// the stroke draws itself on. `end` at or below `start` is a stroke that
    /// is not there yet, which is what a write-on's first frame looks like.
    #[serde(
        default = "Property::zero",
        with = "crate::mask::still_or_keyed",
        skip_serializing_if = "is_static_zero"
    )]
    pub start: Property,
    #[serde(
        default = "full",
        with = "crate::mask::still_or_keyed",
        skip_serializing_if = "is_static_full"
    )]
    pub end: Property,
    pub mode: PaintMode,
    /// How the mark combines with what is already on the layer — the
    /// layer blend list, the same words on the same maths. `Normal` is
    /// source-over, which is what every stroke did before there was a choice
    /// and what the rasteriser still runs byte for byte.
    ///
    /// Meaningless on [`PaintMode::Erase`], which takes alpha away and never
    /// touches colour; it is ignored there rather than being a second way to
    /// say nothing.
    #[serde(default, skip_serializing_if = "is_normal")]
    pub blend: BlendMode,
    /// For [`PaintMode::Clone`]: where the copied pixels come from, as an offset
    /// in layer pixels from the point being painted.
    #[serde(default)]
    pub clone_offset: (f64, f64),
    /// Unknown fields from newer Lumit versions, preserved on load/save
    /// (docs/10-FILE-FORMAT.md §1.1).
    #[serde(flatten, default, skip_serializing_if = "serde_json::Map::is_empty")]
    pub extra: serde_json::Map<String, serde_json::Value>,
}

impl PaintStroke {
    /// A stroke with the usual defaults, for tests and for callers that only
    /// care about the path.
    pub fn new(name: impl Into<String>, points: Vec<(f64, f64)>) -> Self {
        Self {
            id: Uuid::now_v7(),
            name: name.into(),
            points,
            pressures: Vec::new(),
            colour: LinearColour([1.0, 1.0, 1.0, 1.0]),
            width: 20.0,
            hardness: 0.8,
            shape: BrushShape::default(),
            opacity: 100.0,
            start: Property::zero(),
            end: full(),
            mode: PaintMode::Paint,
            blend: BlendMode::Normal,
            clone_offset: (0.0, 0.0),
            extra: serde_json::Map::new(),
        }
    }

    /// The stroke's bounding box in layer coordinates, brush width included, or
    /// `None` when it has no points. Used to skip strokes that cannot touch the
    /// raster being drawn.
    ///
    /// The **whole** path, untrimmed: this answers "where could this stroke
    /// ever be", which is what a caller asking about a stroke wants. The
    /// rasteriser measures the trimmed piece itself.
    pub fn bounds(&self) -> Option<(f64, f64, f64, f64)> {
        bounds_of(&self.points, self.width)
    }

    /// The piece of the path drawn at `t`, in the order it was painted — the
    /// whole path when Start is 0 and End is 100, which is nearly always.
    pub fn drawn_at(&self, t: f64) -> Vec<(f64, f64)> {
        self.drawn_pressed_at(t).0
    }

    /// [`drawn_at`](Self::drawn_at) with the pressure at each surviving point
    /// beside it — empty when the stroke has no pressures at all, which is the
    /// constant 1.0.
    pub fn drawn_pressed_at(&self, t: f64) -> (Vec<(f64, f64)>, Vec<f64>) {
        trimmed_with(
            &self.points,
            &self.pressures,
            self.start.value_at(t),
            self.end.value_at(t),
        )
    }
}

/// The pressure at point `i` of a path: 1.0 wherever there is none, which is
/// the whole of a mouse-drawn stroke and of every stroke drawn before pressure
/// existed.
fn pressure_at(pressures: &[f64], i: usize) -> f64 {
    match pressures.get(i) {
        // A number nobody could act on reads as a full press rather than as an
        // error: this is a file the engine did not necessarily write, and a
        // stroke that draws is a better answer than one that vanishes.
        Some(p) if p.is_finite() => p.clamp(0.0, 1.0),
        _ => 1.0,
    }
}

/// The box `points` occupy with a brush of `width` run along them, or `None`
/// for no points.
fn bounds_of(points: &[(f64, f64)], width: f64) -> Option<(f64, f64, f64, f64)> {
    let first = *points.first()?;
    let r = width.max(0.0) / 2.0;
    let mut b = (first.0, first.1, first.0, first.1);
    for &(x, y) in points {
        b.0 = b.0.min(x);
        b.1 = b.1.min(y);
        b.2 = b.2.max(x);
        b.3 = b.3.max(y);
    }
    Some((b.0 - r, b.1 - r, b.2 + r, b.3 + r))
}

/// The piece of `points` between `start` and `end` per cent of the path's own
/// **arc length** — a straight walk along the polyline, cutting the two
/// segments the ends land in at exactly the right fraction.
///
/// Per cent of *length*, not of the point count: the samples of a gesture are
/// as far apart as the pointer was moving fast, so counting them would make a
/// write-on speed up and slow down with the hand that drew it. Length is what
/// the eye is watching.
///
/// An `end` at or below `start` draws nothing at all — that is the first frame
/// of a write-on, not an error. A path with no length (a single dab, or one
/// sample repeated) has nothing to cut, so it is drawn whole whenever anything
/// of it is asked for.
pub(crate) fn trimmed(points: &[(f64, f64)], start: f64, end: f64) -> Vec<(f64, f64)> {
    trimmed_with(points, &[], start, end).0
}

/// [`trimmed`] carrying a per-point scalar through the same walk — the stylus
/// pressures, lerped at the two segments the ends cut into, so a
/// write-on of a pressed stroke thins and thickens exactly where the untrimmed
/// one does.
///
/// One walk rather than two: a second copy of this arithmetic is a second place
/// for a dash and a trim to disagree about where "ten along" is. An empty
/// `pressures` stays empty on the way out, which is how "the constant 1.0"
/// survives a trim.
fn trimmed_with(
    points: &[(f64, f64)],
    pressures: &[f64],
    start: f64,
    end: f64,
) -> (Vec<(f64, f64)>, Vec<f64>) {
    let from_pct = start.clamp(0.0, 100.0);
    let to_pct = end.clamp(0.0, 100.0);
    if to_pct <= from_pct {
        return (Vec::new(), Vec::new());
    }
    if from_pct <= 0.0 && to_pct >= 100.0 {
        return (points.to_vec(), pressures.to_vec());
    }
    let seg_len = |a: (f64, f64), b: (f64, f64)| ((b.0 - a.0).powi(2) + (b.1 - a.1).powi(2)).sqrt();
    let total: f64 = points.windows(2).map(|p| seg_len(p[0], p[1])).sum();
    if total <= 0.0 {
        return (points.to_vec(), pressures.to_vec());
    }
    let (from, to) = (total * from_pct / 100.0, total * to_pct / 100.0);
    let at = |a: (f64, f64), b: (f64, f64), t: f64| (a.0 + (b.0 - a.0) * t, a.1 + (b.1 - a.1) * t);
    let pressed = !pressures.is_empty();

    let mut out: Vec<(f64, f64)> = Vec::new();
    let mut out_p: Vec<f64> = Vec::new();
    let mut walked = 0.0;
    for (i, pair) in points.windows(2).enumerate() {
        let (p0, p1) = (pair[0], pair[1]);
        let length = seg_len(p0, p1);
        if length <= 0.0 {
            continue;
        }
        let (s0, s1) = (walked, walked + length);
        walked = s1;
        // Wholly before the cut, or wholly after it.
        if s1 < from || s0 > to {
            continue;
        }
        let t0 = ((from - s0) / length).clamp(0.0, 1.0);
        let t1 = ((to - s0) / length).clamp(0.0, 1.0);
        let (a, b) = (pressure_at(pressures, i), pressure_at(pressures, i + 1));
        if out.is_empty() {
            out.push(at(p0, p1, t0));
            if pressed {
                out_p.push(a + (b - a) * t0);
            }
        }
        out.push(at(p0, p1, t1));
        if pressed {
            out_p.push(a + (b - a) * t1);
        }
    }
    (out, out_p)
}

/// How far apart, as a fraction of the brush's radius, the dabs along a segment
/// are stamped.
///
/// Small enough that a fast drag leaves a line rather than a dotted one, large
/// enough that a long stroke does not cost more than it needs to. A quarter of
/// the radius is the usual figure, and it is what the tests are written against.
const DAB_SPACING: f64 = 0.25;

/// The least falloff any brush gets, in raster pixels: a perfectly hard edge
/// would stair-step, and half a pixel is what turns that into an edge.
const MIN_FEATHER: f64 = 0.5;

/// Stamp `strokes` into `rgba` (`w`×`h`), whose full extent is `natural_w` ×
/// `natural_h` in the layer's own coordinates.
///
/// The same shape of call as [`crate::mask::apply_masks`], and for the same
/// reason: the layer's pixels are already in hand, in a buffer whose size is
/// whatever this frame is being rendered at, and a stroke is written in layer
/// coordinates that have to be scaled onto it.
///
/// Clone strokes read from a copy of the raster taken **before any stroke is
/// stamped**, so a clone never picks up paint laid down in the same pass — the
/// alternative smears its own output across the picture as it goes.
pub fn apply_strokes(
    rgba: &mut [u8],
    w: u32,
    h: u32,
    natural_w: f64,
    natural_h: f64,
    strokes: &[PaintStroke],
    t: f64,
) {
    if strokes.is_empty() || w == 0 || h == 0 {
        return;
    }
    // The one length check every stroke rides on: a raster that does not match
    // its stated size paints nothing, calmly, rather than slicing past the end
    // (docs/14: engine crates do not panic). Everything below may then index
    // by row arithmetic without further guards.
    if rgba.len() != (w as usize) * (h as usize) * 4 {
        return;
    }
    let sx = f64::from(w) / natural_w.max(1.0);
    let sy = f64::from(h) / natural_h.max(1.0);
    // Only taken when something actually clones: a copy of the layer is not
    // cheap, and most strokes are not clones.
    let source = strokes
        .iter()
        .any(|s| s.mode == PaintMode::Clone)
        .then(|| rgba.to_vec());

    // One coverage buffer for the whole pass: allocating a full raster per
    // stroke was most of a paint frame's memory traffic. Each stroke clears
    // and writes only its own bounds rectangle.
    let mut coverage = vec![0u8; (w as usize) * (h as usize)];
    for stroke in strokes {
        let Some(rect) = fill_coverage(&mut coverage, stroke, w, h, sx, sy, t) else {
            continue;
        };
        composite(rgba, source.as_deref(), w, sx, sy, stroke, &coverage, rect);
    }
}

/// The 0..255 coverage a stroke's brush leaves on a `w`×`h` raster, or `None`
/// when it cannot touch it at all.
///
/// Public because the same numbers answer "did this stroke mark this pixel",
/// which is what the tests ask and what a GPU path would upload.
pub fn coverage_of(
    stroke: &PaintStroke,
    w: u32,
    h: u32,
    sx: f64,
    sy: f64,
    t: f64,
) -> Option<Vec<u8>> {
    let mut coverage = vec![0u8; (w as usize) * (h as usize)];
    fill_coverage(&mut coverage, stroke, w, h, sx, sy, t)?;
    Some(coverage)
}

/// [`coverage_of`] into a reused buffer: clears the stroke's own bounds
/// rectangle and stamps into it, returning that rectangle as inclusive raster
/// coordinates `(x0, y0, x1, y1)` so the composite can visit only the pixels
/// the stroke can have touched. `coverage` must be `w × h`.
#[allow(clippy::too_many_arguments)]
fn fill_coverage(
    coverage: &mut [u8],
    stroke: &PaintStroke,
    w: u32,
    h: u32,
    sx: f64,
    sy: f64,
    t: f64,
) -> Option<(u32, u32, u32, u32)> {
    if stroke.points.is_empty() || stroke.width <= 0.0 || stroke.opacity <= 0.0 {
        return None;
    }
    // The piece drawn at this frame. Measured and dabbed from the
    // trimmed path, so a write-on's box grows with it rather than reserving
    // the whole stroke from the first frame.
    let (points, pressures) = stroke.drawn_pressed_at(t);
    let (min_x, min_y, max_x, max_y) = bounds_of(&points, stroke.width)?;
    if max_x * sx < 0.0
        || max_y * sy < 0.0
        || min_x * sx > f64::from(w)
        || min_y * sy > f64::from(h)
    {
        return None;
    }

    // The brush is round in *layer* space; a layer raster is scaled the same
    // way in both axes in every path that exists today, so one radius is
    // enough. Taking the smaller of the two keeps a stroke from bleeding if
    // that ever stops being true.
    let scale = sx.min(sy);
    // The brush at full pressure. Every dab is this scaled by how hard the
    // stylus was pressed there; with no pressures that factor is 1.0
    // at every dab and the numbers below are the ones the rasteriser has always
    // used, to the bit.
    let radius = (stroke.width / 2.0 * scale).max(0.5);
    let hardness = stroke.hardness.clamp(0.0, 1.0);

    // The raster rectangle this stroke can reach: its bounds scaled onto the
    // raster, grown by the brush radius — the same floor/ceil `stamp` uses per
    // dab, so every dab lands inside the cleared area.
    let x0 = (min_x * sx - radius).floor().max(0.0) as u32;
    let y0 = (min_y * sy - radius).floor().max(0.0) as u32;
    let x1 = (max_x * sx + radius).ceil().clamp(0.0, f64::from(w) - 1.0) as u32;
    let y1 = (max_y * sy + radius).ceil().clamp(0.0, f64::from(h) - 1.0) as u32;
    for y in y0..=y1 {
        let row = (y as usize) * (w as usize);
        coverage[row + x0 as usize..=row + x1 as usize].fill(0);
    }
    for (cx, cy, pressure) in dabs(&points, &pressures, radius) {
        // Nothing pressed is nothing drawn — the stylus lifted, or a tablet
        // reporting a zero on the way down.
        if pressure <= 0.0 {
            continue;
        }
        let r = (radius * pressure).max(0.5);
        let feather = (r * (1.0 - hardness)).max(MIN_FEATHER);
        stamp(
            coverage,
            w,
            h,
            cx * sx,
            cy * sy,
            Dab {
                radius: r,
                solid: (r - feather).max(0.0),
                feather,
                shape: stroke.shape,
            },
        );
    }
    Some((x0, y0, x1, y1))
}

/// Where the dabs along a stroke's polyline go, in layer coordinates, and how
/// hard each one presses.
///
/// One point gives one dab. Each segment is walked at [`DAB_SPACING`] of the
/// radius so the marks overlap into a line; the segment's far end is always
/// stamped, so a stroke never falls short of where the pointer stopped.
///
/// The step shrinks with the pressure along the segment, because a light dab is
/// a *smaller* dab: spacing worked out from the full radius would leave a dotted
/// line wherever the hand went light. The lighter of the segment's two ends is
/// what it is measured from, and never below a tenth, so a pressure sliding
/// towards nothing cannot ask for an unbounded number of invisible dabs.
fn dabs(points: &[(f64, f64)], pressures: &[f64], radius_px: f64) -> Vec<(f64, f64, f64)> {
    let mut out = Vec::new();
    let Some(&first) = points.first() else {
        return out;
    };
    out.push((first.0, first.1, pressure_at(pressures, 0)));
    if points.len() == 1 {
        return out;
    }
    for (i, pair) in points.windows(2).enumerate() {
        let (x0, y0) = pair[0];
        let (x1, y1) = pair[1];
        let dx = x1 - x0;
        let dy = y1 - y0;
        let length = (dx * dx + dy * dy).sqrt();
        if length <= f64::EPSILON {
            continue;
        }
        let (p0, p1) = (pressure_at(pressures, i), pressure_at(pressures, i + 1));
        // The step is measured in layer units: the radius is in raster pixels,
        // so a stroke on a layer being drawn small still gets dabs close enough
        // together to join up.
        let step = (radius_px * p0.min(p1).max(0.1) * DAB_SPACING).max(0.25);
        let count = (length / step).ceil().min(4096.0) as usize;
        for j in 1..=count {
            let t = j as f64 / count as f64;
            out.push((x0 + dx * t, y0 + dy * t, p0 + (p1 - p0) * t));
        }
    }
    out
}

/// One dab's measurements: everything about the brush that does not move as it
/// is walked along the stroke. Worked out once per stroke rather than passed as
/// four more arguments per dab.
#[derive(Debug, Clone, Copy)]
struct Dab {
    /// Half the brush's width on the raster being drawn, in pixels.
    radius: f64,
    /// Inside this distance the coverage is full.
    solid: f64,
    /// The width of the ramp from full to nothing: `radius - solid`, never less
    /// than [`MIN_FEATHER`].
    feather: f64,
    shape: BrushShape,
}

impl Dab {
    /// How far a pixel at `(dx, dy)` from the dab's centre counts as being,
    /// **in the brush's own idea of distance** — a straight line for a round
    /// brush, the greater of the two axes for a square one.
    ///
    /// That one substitution is the whole of the shape: everything downstream —
    /// the radius, the hardness ramp, the bounding box, the dab spacing — is
    /// written in terms of this number and needs no case of its own. A square
    /// brush therefore softens exactly as a round one does, outwards from a
    /// flat-sided core rather than a circular one.
    fn distance(&self, dx: f64, dy: f64) -> f64 {
        match self.shape {
            BrushShape::Round => (dx * dx + dy * dy).sqrt(),
            BrushShape::Square => dx.abs().max(dy.abs()),
        }
    }
}

/// One dab into the coverage buffer, taking the greatest coverage at each
/// pixel rather than adding.
///
/// Greatest, not sum: the dabs along a stroke overlap heavily by design, and
/// adding them would make the middle of a slow stroke opaque and its ends thin.
/// A stroke's *own* opacity is applied once, when it is composited.
fn stamp(coverage: &mut [u8], w: u32, h: u32, cx: f64, cy: f64, dab: Dab) {
    let Dab {
        radius,
        solid,
        feather,
        ..
    } = dab;
    let x0 = ((cx - radius).floor()).max(0.0) as u32;
    let y0 = ((cy - radius).floor()).max(0.0) as u32;
    let x1 = ((cx + radius).ceil()).min(f64::from(w) - 1.0);
    let y1 = ((cy + radius).ceil()).min(f64::from(h) - 1.0);
    if x1 < 0.0 || y1 < 0.0 {
        return;
    }
    let x1 = x1 as u32;
    let y1 = y1 as u32;
    for y in y0..=y1 {
        for x in x0..=x1 {
            // The pixel's centre, which is what a round brush is measured to.
            let dx = f64::from(x) + 0.5 - cx;
            let dy = f64::from(y) + 0.5 - cy;
            let d = dab.distance(dx, dy);
            let a = if d <= solid {
                1.0
            } else if d >= radius {
                0.0
            } else {
                (radius - d) / feather
            };
            if a <= 0.0 {
                continue;
            }
            let value = (a.clamp(0.0, 1.0) * 255.0).round() as u8;
            let slot = &mut coverage[(y as usize) * (w as usize) + (x as usize)];
            *slot = (*slot).max(value);
        }
    }
}

/// Put one stroke's coverage into the pixels, by its mode. Visits only the
/// stroke's own `(x0, y0, x1, y1)` rectangle — the rest of the raster is
/// pixels this stroke cannot have marked. `apply_strokes` has already checked
/// `rgba` is `w × h × 4`, and the rows are walked as `chunks_exact_mut(4)`, so
/// nothing here can slice out of bounds.
#[allow(clippy::too_many_arguments)]
fn composite(
    rgba: &mut [u8],
    source: Option<&[u8]>,
    w: u32,
    sx: f64,
    sy: f64,
    stroke: &PaintStroke,
    coverage: &[u8],
    (x0, y0, x1, y1): (u32, u32, u32, u32),
) {
    let opacity = (stroke.opacity.clamp(0.0, 100.0) / 100.0) as f32;
    let colour = crate::pixels::solid_rgba(stroke.colour);
    // The stroke's own alpha multiplies its opacity: a stroke of a
    // half-transparent colour is half transparent.
    let colour_alpha = f32::from(colour[3]) / 255.0;
    // The clone source is the same raster, so its height is implied by its
    // length and the shared width.
    let h = rgba.len() / 4 / (w as usize).max(1);
    // The blend, as the index into `BlendMode::ALL` the shared kernel takes.
    // Worked out once per stroke; `None` for Normal, which is the early exit
    // that keeps every unblended stroke byte for byte what it was.
    let blend = (stroke.blend != BlendMode::Normal).then(|| {
        BlendMode::ALL
            .iter()
            .position(|m| *m == stroke.blend)
            .unwrap_or(0) as u32
    });
    // Lay `rgb` down on `px` at coverage-alpha `a`, by the stroke's blend.
    //
    // The blend runs in the domains the compositor and the effect Blend run in
    // (docs/06 §blend domains), through the one shared kernel — decode both
    // sides to linear light, `blend_pixel`, encode the answer back. What comes
    // out is a *colour*, which is then laid down by exactly the source-over the
    // rasteriser has always used, so a mode changes what the mark is, never how
    // it covers.
    let marked = |px: &[u8], rgb: [u8; 3]| -> [u8; 3] {
        let Some(mode) = blend else { return rgb };
        let d = [
            crate::pixels::srgb_decode(px[0]),
            crate::pixels::srgb_decode(px[1]),
            crate::pixels::srgb_decode(px[2]),
            1.0,
        ];
        let src = [
            crate::pixels::srgb_decode(rgb[0]),
            crate::pixels::srgb_decode(rgb[1]),
            crate::pixels::srgb_decode(rgb[2]),
            1.0,
        ];
        let b = crate::fx::cpu::blend_pixel(mode, d, src);
        [
            crate::pixels::srgb_encode(b[0]),
            crate::pixels::srgb_encode(b[1]),
            crate::pixels::srgb_encode(b[2]),
        ]
    };

    for y in y0..=y1 {
        let row = (y as usize) * (w as usize);
        let px_row = &mut rgba[(row + x0 as usize) * 4..(row + x1 as usize + 1) * 4];
        let cov_row = &coverage[row + x0 as usize..=row + x1 as usize];
        for ((x, px), &cov) in (x0..=x1).zip(px_row.chunks_exact_mut(4)).zip(cov_row) {
            let c = f32::from(cov) / 255.0;
            if c <= 0.0 {
                continue;
            }
            match stroke.mode {
                PaintMode::Paint => {
                    let a = c * opacity * colour_alpha;
                    let rgb = marked(px, [colour[0], colour[1], colour[2]]);
                    over(px, rgb, a);
                }
                PaintMode::Erase => {
                    // No colour to blend: an erase takes alpha away, which is
                    // what makes it reversible by lowering its opacity later.
                    let keep = 1.0 - c * opacity;
                    px[3] = (f32::from(px[3]) * keep).round().clamp(0.0, 255.0) as u8;
                }
                PaintMode::Clone => {
                    let Some(source) = source else { continue };
                    let ox = f64::from(x) + stroke.clone_offset.0 * sx;
                    let oy = f64::from(y) + stroke.clone_offset.1 * sy;
                    // Off the layer there is nothing to copy; a clone that ran
                    // off the edge used to wrap, which reads as a bug.
                    if ox < 0.0 || oy < 0.0 || ox >= f64::from(w) || oy >= h as f64 {
                        continue;
                    }
                    let j = (oy as usize) * (w as usize) + (ox as usize);
                    let src = &source[j * 4..j * 4 + 4];
                    let a = c * opacity * (f32::from(src[3]) / 255.0);
                    let rgb = marked(px, [src[0], src[1], src[2]]);
                    over(px, rgb, a);
                }
            }
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    /// A transparent raster to paint into, and an opaque one to erase from.
    fn raster(w: u32, h: u32, px: [u8; 4]) -> Vec<u8> {
        crate::pixels::px_tile(&px, w, h)
    }

    fn alpha_at(rgba: &[u8], w: u32, x: u32, y: u32) -> u8 {
        rgba[((y * w + x) as usize) * 4 + 3]
    }

    fn rgb_at(rgba: &[u8], w: u32, x: u32, y: u32) -> [u8; 3] {
        let i = ((y * w + x) as usize) * 4;
        [rgba[i], rgba[i + 1], rgba[i + 2]]
    }

    #[test]
    fn a_dab_marks_where_it_was_put_and_nowhere_else() {
        let mut rgba = raster(40, 40, [0, 0, 0, 0]);
        let mut stroke = PaintStroke::new("Dab", vec![(20.0, 20.0)]);
        stroke.width = 10.0;
        apply_strokes(&mut rgba, 40, 40, 40.0, 40.0, &[stroke], 0.0);

        assert_eq!(alpha_at(&rgba, 40, 20, 20), 255, "the middle of the dab");
        assert_eq!(alpha_at(&rgba, 40, 20, 30), 0, "well outside its radius");
        assert_eq!(alpha_at(&rgba, 40, 0, 0), 0, "and the far corner");
    }

    #[test]
    fn a_stroke_joins_its_points_up() {
        let mut rgba = raster(80, 20, [0, 0, 0, 0]);
        let mut stroke = PaintStroke::new("Line", vec![(5.0, 10.0), (75.0, 10.0)]);
        stroke.width = 6.0;
        apply_strokes(&mut rgba, 80, 20, 80.0, 20.0, &[stroke], 0.0);

        // Every pixel along the line is marked: a gap here is the dab spacing
        // being wrong, which is what a dotted stroke looks like.
        for x in 6..75 {
            assert!(
                alpha_at(&rgba, 80, x, 10) > 200,
                "the line broke at x={x} ({})",
                alpha_at(&rgba, 80, x, 10)
            );
        }
        assert_eq!(alpha_at(&rgba, 80, 40, 0), 0, "and it did not spread");
    }

    #[test]
    fn a_soft_brush_fades_and_a_hard_one_does_not() {
        let dab = |hardness: f64| {
            let mut rgba = raster(40, 40, [0, 0, 0, 0]);
            let mut stroke = PaintStroke::new("Dab", vec![(20.0, 20.0)]);
            stroke.width = 20.0;
            stroke.hardness = hardness;
            apply_strokes(&mut rgba, 40, 40, 40.0, 40.0, &[stroke], 0.0);
            // Halfway out from the middle.
            alpha_at(&rgba, 40, 25, 20)
        };
        assert_eq!(dab(1.0), 255, "a hard brush is solid to its edge");
        assert!(
            dab(0.0) < 200,
            "a soft one has faded by halfway: {}",
            dab(0.0)
        );
        assert!(dab(0.0) > 0, "but has not vanished");
    }

    /// A square brush fills its corners; a round one does not. Same width, same
    /// hardness, same place — the only difference is how the distance to the
    /// centre is measured.
    #[test]
    fn a_square_brush_marks_its_corners_and_a_round_one_does_not() {
        let dab = |shape: BrushShape| {
            let mut rgba = raster(40, 40, [0, 0, 0, 0]);
            let mut stroke = PaintStroke::new("Dab", vec![(20.0, 20.0)]);
            stroke.width = 20.0;
            stroke.hardness = 1.0;
            stroke.shape = shape;
            apply_strokes(&mut rgba, 40, 40, 40.0, 40.0, &[stroke], 0.0);
            rgba
        };
        let round = dab(BrushShape::Round);
        let square = dab(BrushShape::Square);
        // Just inside the corner of the 20×20 box, which is 14 px from the
        // centre in a straight line — outside a radius-10 circle, inside a
        // half-width-10 square.
        assert_eq!(
            alpha_at(&round, 40, 28, 28),
            0,
            "a round brush leaves its corners alone"
        );
        assert_eq!(
            alpha_at(&square, 40, 28, 28),
            255,
            "a square one fills them"
        );
        // And the two agree straight out from the middle, where the two ways
        // of measuring distance give the same answer.
        assert_eq!(alpha_at(&round, 40, 25, 20), alpha_at(&square, 40, 25, 20));
        assert_eq!(
            alpha_at(&square, 40, 20, 32),
            0,
            "the square stops at its own edge rather than growing"
        );
    }

    /// Hardness is one ramp, and it softens a square exactly as it softens a
    /// round: the shape decides what "distance from the centre" means
    /// and nothing else.
    #[test]
    fn a_soft_square_fades_at_its_flat_edge() {
        let dab = |hardness: f64| {
            let mut rgba = raster(40, 40, [0, 0, 0, 0]);
            let mut stroke = PaintStroke::new("Dab", vec![(20.0, 20.0)]);
            stroke.width = 20.0;
            stroke.hardness = hardness;
            stroke.shape = BrushShape::Square;
            apply_strokes(&mut rgba, 40, 40, 40.0, 40.0, &[stroke], 0.0);
            alpha_at(&rgba, 40, 25, 20)
        };
        assert_eq!(dab(1.0), 255, "a hard square is solid to its edge");
        assert!(
            dab(0.0) < 200,
            "a soft one has faded by halfway: {}",
            dab(0.0)
        );
        assert!(dab(0.0) > 0, "but has not vanished");
    }

    /// The shape is left out of the file until somebody picks the other one, so
    /// every project ever saved writes exactly the bytes it wrote before — and
    /// one written before there was a choice reads back as Round.
    #[test]
    fn a_round_brush_is_absent_from_the_file() {
        let stroke = PaintStroke::new("Dab", vec![(1.0, 2.0)]);
        let json = serde_json::to_string(&stroke).expect("serialise");
        assert!(
            !json.contains("shape"),
            "a round brush writes nothing: {json}"
        );

        let mut square = stroke.clone();
        square.shape = BrushShape::Square;
        let json = serde_json::to_string(&square).expect("serialise");
        assert!(json.contains("Square"), "a square one does: {json}");
        let back: PaintStroke = serde_json::from_str(&json).expect("read back");
        assert_eq!(back.shape, BrushShape::Square);
    }

    /// Start and End trim the path by **arc length**: the write-on
    /// that makes a stroke draw itself on.
    #[test]
    fn start_and_end_trim_the_stroke_by_its_length() {
        // A straight 70-pixel line, painted left to right.
        let line = |start: f64, end: f64| {
            let mut rgba = raster(80, 20, [0, 0, 0, 0]);
            let mut stroke = PaintStroke::new("Line", vec![(5.0, 10.0), (75.0, 10.0)]);
            stroke.width = 6.0;
            stroke.start = Property::fixed(start);
            stroke.end = Property::fixed(end);
            apply_strokes(&mut rgba, 80, 20, 80.0, 20.0, &[stroke], 0.0);
            rgba
        };
        // Half drawn: the left half is marked and the right half is not. The
        // cut is at 5 + 35 = 40, and the brush is 6 wide, so 36 is inside and
        // 46 is clear of it.
        let half = line(0.0, 50.0);
        assert!(alpha_at(&half, 80, 36, 10) > 200, "the drawn half");
        assert_eq!(alpha_at(&half, 80, 46, 10), 0, "and nothing past the cut");

        // The other end, by the same measure.
        let tail = line(50.0, 100.0);
        assert_eq!(alpha_at(&tail, 80, 34, 10), 0, "nothing before the cut");
        assert!(alpha_at(&tail, 80, 44, 10) > 200, "and the tail is drawn");

        // Nothing yet: the first frame of a write-on.
        let none = line(0.0, 0.0);
        assert!(none.iter().all(|&b| b == 0), "End at 0 draws nothing");
        // And End below Start is the same answer rather than a stroke drawn
        // backwards.
        let crossed = line(80.0, 20.0);
        assert!(crossed.iter().all(|&b| b == 0));

        // Untouched is the whole thing, byte for byte what it always was.
        let all = line(0.0, 100.0);
        let mut plain = raster(80, 20, [0, 0, 0, 0]);
        let mut stroke = PaintStroke::new("Line", vec![(5.0, 10.0), (75.0, 10.0)]);
        stroke.width = 6.0;
        apply_strokes(&mut plain, 80, 20, 80.0, 20.0, &[stroke], 0.0);
        assert_eq!(all, plain);
    }

    /// Length, not point count: a gesture's samples bunch up where the hand
    /// slowed down, so counting them would make a write-on speed up and slow
    /// down with the drawing.
    #[test]
    fn the_trim_measures_length_and_not_samples() {
        // Two arms of 40 pixels each. The first is sampled once, the second
        // ten times — the same shape, drawn at two speeds.
        let mut points = vec![(0.0, 0.0), (40.0, 0.0)];
        for i in 1..=10 {
            points.push((40.0, 40.0 * f64::from(i) / 10.0));
        }
        let drawn = trimmed(&points, 0.0, 50.0);
        let walked: f64 = drawn
            .windows(2)
            .map(|p| ((p[1].0 - p[0].0).powi(2) + (p[1].1 - p[0].1).powi(2)).sqrt())
            .sum();
        assert!(
            (walked - 40.0).abs() < 1e-9,
            "half of 80 pixels is 40, whatever the samples do: {walked}"
        );
        assert_eq!(
            *drawn.last().expect("a piece"),
            (40.0, 0.0),
            "and half lands exactly at the corner"
        );
    }

    /// A single dab has no length to cut, so it is drawn whole as soon as
    /// anything of it is asked for — and not at all before that.
    #[test]
    fn a_dab_has_no_length_to_trim() {
        assert_eq!(trimmed(&[(3.0, 4.0)], 0.0, 1.0), vec![(3.0, 4.0)]);
        assert_eq!(trimmed(&[(3.0, 4.0)], 25.0, 75.0), vec![(3.0, 4.0)]);
        assert!(trimmed(&[(3.0, 4.0)], 40.0, 40.0).is_empty());
    }

    /// An untrimmed stroke says nothing about Start or End in the file, so
    /// every project ever saved writes the bytes it wrote before.
    #[test]
    fn an_untrimmed_stroke_is_absent_from_the_file() {
        let stroke = PaintStroke::new("Line", vec![(1.0, 2.0), (3.0, 4.0)]);
        let json = serde_json::to_string(&stroke).expect("serialise");
        assert!(!json.contains("start"), "nothing about start: {json}");
        assert!(!json.contains("\"end\""), "nor end: {json}");

        let mut written = stroke.clone();
        written.end = Property::fixed(40.0);
        let json = serde_json::to_string(&written).expect("serialise");
        let back: PaintStroke = serde_json::from_str(&json).expect("read back");
        assert_eq!(back.end.value_at(0.0), 40.0);
        assert_eq!(back.start.value_at(0.0), 0.0, "the default comes back");
    }

    /// A stroke's blend is the layer blend list on the layer blend maths, run
    /// through the one shared kernel.
    #[test]
    fn a_strokes_blend_combines_it_with_what_is_under_it() {
        let dab = |blend: BlendMode, colour: LinearColour| {
            // A mid-grey layer to mark. Opaque, so the source-over that lays
            // the blended colour down is the blended colour.
            let mut rgba = raster(20, 20, [128, 128, 128, 255]);
            let mut stroke = PaintStroke::new("Dab", vec![(10.0, 10.0)]);
            stroke.width = 8.0;
            stroke.hardness = 1.0;
            stroke.colour = colour;
            stroke.blend = blend;
            apply_strokes(&mut rgba, 20, 20, 20.0, 20.0, &[stroke], 0.0);
            rgb_at(&rgba, 20, 10, 10)
        };
        let white = LinearColour([1.0, 1.0, 1.0, 1.0]);
        let black = LinearColour([0.0, 0.0, 0.0, 1.0]);

        assert_eq!(dab(BlendMode::Normal, white), [255, 255, 255], "over");
        assert_eq!(
            dab(BlendMode::Multiply, white),
            [128, 128, 128],
            "white multiplied into grey leaves the grey"
        );
        assert_eq!(
            dab(BlendMode::Multiply, black),
            [0, 0, 0],
            "and black takes it to black"
        );
        assert_eq!(
            dab(BlendMode::Lighten, black),
            [128, 128, 128],
            "the lighter of black and grey is the grey"
        );
        assert_eq!(
            dab(BlendMode::Darken, white),
            [128, 128, 128],
            "and the darker of white and grey is the grey"
        );
        // Difference against itself is black, whatever the grey happens to be.
        let grey = LinearColour([
            crate::pixels::srgb_decode(128),
            crate::pixels::srgb_decode(128),
            crate::pixels::srgb_decode(128),
            1.0,
        ]);
        let diff = dab(BlendMode::Difference, grey);
        assert!(
            diff.iter().all(|&c| c <= 1),
            "a colour differenced with itself is black, got {diff:?}"
        );
    }

    /// A blend does not change how the mark *covers*, only what colour it
    /// lays down — so half opacity is still half the way there.
    #[test]
    fn a_blend_changes_the_colour_and_not_the_coverage() {
        let mut rgba = raster(20, 20, [128, 128, 128, 255]);
        let mut stroke = PaintStroke::new("Dab", vec![(10.0, 10.0)]);
        stroke.width = 8.0;
        stroke.hardness = 1.0;
        stroke.opacity = 50.0;
        stroke.colour = LinearColour([0.0, 0.0, 0.0, 1.0]);
        stroke.blend = BlendMode::Multiply;
        apply_strokes(&mut rgba, 20, 20, 20.0, 20.0, &[stroke], 0.0);
        // Black multiplied into grey is black; laid down at half coverage that
        // is halfway between the grey and black, in the encoded domain the
        // rasteriser has always composited in.
        let got = rgb_at(&rgba, 20, 10, 10);
        assert!(
            (62..=66).contains(&got[0]),
            "half of the way to black, got {got:?}"
        );
        assert_eq!(alpha_at(&rgba, 20, 10, 10), 255, "and the layer is opaque");
    }

    /// An erase has no colour to blend, so a mode on one is ignored rather
    /// than being a second way of saying nothing.
    #[test]
    fn a_blend_on_an_erase_changes_nothing() {
        let rub = |blend: BlendMode| {
            let mut rgba = raster(20, 20, [200, 100, 50, 255]);
            let mut stroke = PaintStroke::new("Rub", vec![(10.0, 10.0)]);
            stroke.width = 8.0;
            stroke.mode = PaintMode::Erase;
            stroke.blend = blend;
            apply_strokes(&mut rgba, 20, 20, 20.0, 20.0, &[stroke], 0.0);
            rgba
        };
        assert_eq!(rub(BlendMode::Normal), rub(BlendMode::Difference));
    }

    /// Normal is left out of the file, so an unblended stroke writes exactly
    /// the bytes it wrote before there was a choice.
    #[test]
    fn an_unblended_stroke_is_absent_from_the_file() {
        let stroke = PaintStroke::new("Dab", vec![(1.0, 2.0)]);
        let json = serde_json::to_string(&stroke).expect("serialise");
        assert!(!json.contains("blend"), "nothing about blend: {json}");

        let mut screened = stroke.clone();
        screened.blend = BlendMode::Screen;
        let json = serde_json::to_string(&screened).expect("serialise");
        let back: PaintStroke = serde_json::from_str(&json).expect("read back");
        assert_eq!(back.blend, BlendMode::Screen);
    }

    #[test]
    fn opacity_scales_the_mark() {
        let mut rgba = raster(20, 20, [0, 0, 0, 0]);
        let mut stroke = PaintStroke::new("Dab", vec![(10.0, 10.0)]);
        stroke.width = 8.0;
        stroke.opacity = 50.0;
        apply_strokes(&mut rgba, 20, 20, 20.0, 20.0, &[stroke], 0.0);
        let a = alpha_at(&rgba, 20, 10, 10);
        assert!((120..=136).contains(&a), "half-opaque, got {a}");
    }

    /// A stroke is written in layer coordinates and stamped at whatever size the
    /// frame is being rendered at — the whole reason the document keeps the
    /// gesture rather than the pixels.
    #[test]
    fn a_stroke_follows_the_resolution_it_is_drawn_at() {
        let mut half = raster(20, 20, [0, 0, 0, 0]);
        let mut stroke = PaintStroke::new("Dab", vec![(20.0, 20.0)]);
        stroke.width = 20.0;
        // The layer is 40×40 in its own coordinates, rendered into 20×20.
        apply_strokes(&mut half, 20, 20, 40.0, 40.0, &[stroke.clone()], 0.0);
        assert_eq!(
            alpha_at(&half, 20, 10, 10),
            255,
            "the dab is at the middle of the half-size raster"
        );
        assert_eq!(
            alpha_at(&half, 20, 10, 18),
            0,
            "and it is half the size, not the same number of pixels"
        );
    }

    #[test]
    fn erasing_takes_alpha_away_and_leaves_the_colour() {
        let mut rgba = raster(20, 20, [200, 100, 50, 255]);
        let mut stroke = PaintStroke::new("Rub", vec![(10.0, 10.0)]);
        stroke.width = 8.0;
        stroke.mode = PaintMode::Erase;
        apply_strokes(&mut rgba, 20, 20, 20.0, 20.0, &[stroke], 0.0);

        assert_eq!(alpha_at(&rgba, 20, 10, 10), 0, "rubbed through");
        assert_eq!(
            rgb_at(&rgba, 20, 10, 10),
            [200, 100, 50],
            "colour untouched"
        );
        assert_eq!(alpha_at(&rgba, 20, 0, 0), 255, "and only where it brushed");
    }

    #[test]
    fn cloning_copies_from_the_offset_it_was_given() {
        // The left half is red, the right half is transparent.
        let mut rgba = raster(20, 20, [0, 0, 0, 0]);
        for y in 0..20u32 {
            for x in 0..10u32 {
                let i = ((y * 20 + x) as usize) * 4;
                rgba[i..i + 4].copy_from_slice(&[255, 0, 0, 255]);
            }
        }
        let mut stroke = PaintStroke::new("Stamp", vec![(15.0, 10.0)]);
        stroke.width = 6.0;
        stroke.mode = PaintMode::Clone;
        // Take from ten pixels to the left, which is the red half.
        stroke.clone_offset = (-10.0, 0.0);
        apply_strokes(&mut rgba, 20, 20, 20.0, 20.0, &[stroke], 0.0);

        assert_eq!(
            rgb_at(&rgba, 20, 15, 10),
            [255, 0, 0],
            "red was copied over"
        );
        assert_eq!(alpha_at(&rgba, 20, 15, 10), 255);
        assert_eq!(alpha_at(&rgba, 20, 19, 19), 0, "and only under the brush");
    }

    /// A clone must read the layer as it was, not as it is being painted:
    /// sampling its own output smears the copy across the picture.
    #[test]
    fn cloning_reads_the_layer_as_it_was() {
        let mut rgba = raster(40, 10, [0, 0, 0, 0]);
        // Paint blue at x=5 in the same pass the clone runs in...
        let mut paint = PaintStroke::new("Paint", vec![(5.0, 5.0)]);
        paint.width = 6.0;
        paint.colour = LinearColour([0.0, 0.0, 1.0, 1.0]);
        // ...and clone from x=5 onto x=25.
        let mut clone = PaintStroke::new("Stamp", vec![(25.0, 5.0)]);
        clone.width = 6.0;
        clone.mode = PaintMode::Clone;
        clone.clone_offset = (-20.0, 0.0);

        apply_strokes(&mut rgba, 40, 10, 40.0, 10.0, &[paint, clone], 0.0);

        assert!(alpha_at(&rgba, 40, 5, 5) > 0, "the paint landed");
        assert_eq!(
            alpha_at(&rgba, 40, 25, 5),
            0,
            "the clone read the layer as it was — transparent — rather than \
             the blue laid down beside it in the same pass"
        );
    }

    #[test]
    fn a_stroke_with_nothing_in_it_does_nothing() {
        let mut rgba = raster(8, 8, [1, 2, 3, 4]);
        let before = rgba.clone();
        let empty = PaintStroke::new("Nothing", vec![]);
        let mut zero = PaintStroke::new("Zero", vec![(4.0, 4.0)]);
        zero.width = 0.0;
        let mut clear = PaintStroke::new("Clear", vec![(4.0, 4.0)]);
        clear.opacity = 0.0;
        apply_strokes(&mut rgba, 8, 8, 8.0, 8.0, &[empty, zero, clear], 0.0);
        assert_eq!(rgba, before);
    }

    #[test]
    fn a_stroke_off_the_layer_is_skipped_rather_than_drawn() {
        let mut rgba = raster(8, 8, [0, 0, 0, 0]);
        let mut stroke = PaintStroke::new("Away", vec![(500.0, 500.0)]);
        stroke.width = 4.0;
        apply_strokes(&mut rgba, 8, 8, 8.0, 8.0, &[stroke], 0.0);
        assert!(rgba.iter().all(|&b| b == 0));
    }

    #[test]
    fn bounds_include_the_brush_width() {
        let mut stroke = PaintStroke::new("Line", vec![(10.0, 10.0), (30.0, 20.0)]);
        stroke.width = 10.0;
        let (x0, y0, x1, y1) = stroke.bounds().expect("points");
        assert_eq!((x0, y0, x1, y1), (5.0, 5.0, 35.0, 25.0));
        assert!(PaintStroke::new("None", vec![]).bounds().is_none());
    }

    /// A raster that does not match its stated size paints nothing rather
    /// than slicing past the end (docs/14: engine crates do not panic).
    #[test]
    fn a_short_raster_paints_nothing_rather_than_panicking() {
        let mut short = vec![0u8; 10];
        let mut stroke = PaintStroke::new("Dab", vec![(4.0, 4.0)]);
        stroke.width = 4.0;
        apply_strokes(&mut short, 8, 8, 8.0, 8.0, &[stroke], 0.0);
        assert!(short.iter().all(|&b| b == 0));
    }

    /// The coverage buffer is reused across the strokes of a pass, so one
    /// stroke's marks must never leak into the next stroke's composite. The
    /// second stroke's bounds rectangle here contains a pixel the first
    /// stroke painted red — stale coverage there would turn it blue.
    #[test]
    fn a_reused_coverage_buffer_carries_nothing_between_strokes() {
        let mut rgba = raster(40, 40, [0, 0, 0, 0]);
        let mut red = PaintStroke::new("Red", vec![(10.0, 10.0)]);
        red.width = 4.0;
        red.colour = crate::model::LinearColour([1.0, 0.0, 0.0, 1.0]);
        // A wide dab lower down: its rectangle reaches (10, 10), but its round
        // brush does not.
        let mut blue = PaintStroke::new("Blue", vec![(24.0, 24.0)]);
        blue.width = 24.0;
        blue.colour = crate::model::LinearColour([0.0, 0.0, 1.0, 1.0]);
        apply_strokes(&mut rgba, 40, 40, 40.0, 40.0, &[red, blue], 0.0);

        assert_eq!(rgb_at(&rgba, 40, 10, 10), [255, 0, 0], "red stays red");
        assert_eq!(rgb_at(&rgba, 40, 24, 24), [0, 0, 255], "blue landed");
    }

    /// The same document must give the same pixels on every machine and every
    /// run (docs/14 §determinism), so the same strokes twice are the same
    /// bytes twice.
    #[test]
    fn painting_is_deterministic() {
        let strokes = vec![
            PaintStroke::new("A", vec![(2.0, 2.0), (18.0, 9.0)]),
            PaintStroke::new("B", vec![(4.0, 14.0)]),
        ];
        let mut once = raster(20, 20, [0, 0, 0, 0]);
        let mut twice = raster(20, 20, [0, 0, 0, 0]);
        apply_strokes(&mut once, 20, 20, 20.0, 20.0, &strokes, 0.0);
        apply_strokes(&mut twice, 20, 20, 20.0, 20.0, &strokes, 0.0);
        assert_eq!(once, twice);
    }

    // --- Pressure ---------------------------------------------------------

    /// How wide the mark is across the middle of a dab: the numeric row a
    /// pressure has to move, and the whole of what pressure does.
    fn marked_width(rgba: &[u8], w: u32, y: u32) -> u32 {
        (0..w).filter(|&x| alpha_at(rgba, w, x, y) > 0).count() as u32
    }

    /// Width scales by pressure, dab by dab. A golden-ish row rather than an
    /// exact byte: what is being pinned is that half the pressure is half the
    /// brush, not the anti-aliasing at its edge.
    #[test]
    fn pressure_scales_the_width_of_the_mark() {
        let dab = |pressure: Option<f64>| {
            let mut rgba = raster(60, 60, [0, 0, 0, 0]);
            let mut stroke = PaintStroke::new("Dab", vec![(30.0, 30.0)]);
            stroke.width = 40.0;
            stroke.hardness = 1.0;
            if let Some(p) = pressure {
                stroke.pressures = vec![p];
            }
            apply_strokes(&mut rgba, 60, 60, 60.0, 60.0, &[stroke], 0.0);
            marked_width(&rgba, 60, 30)
        };

        let full = dab(None);
        assert!((39..=42).contains(&full), "a 40px brush marks 40px: {full}");
        assert_eq!(dab(Some(1.0)), full, "a full press is the plain brush");
        let half = dab(Some(0.5));
        assert!(
            (19..=22).contains(&half),
            "half the press, half of it: {half}"
        );
        let light = dab(Some(0.25));
        assert!((9..=12).contains(&light), "and a quarter: {light}");
        assert_eq!(dab(Some(0.0)), 0, "nothing pressed is nothing drawn");
    }

    /// The pressure moves *along* the stroke, so a gesture that pressed harder
    /// as it went leaves a mark that widens — and no gap where it was light,
    /// which is the dab spacing following the pressure rather than the width.
    #[test]
    fn a_stroke_thickens_where_it_was_pressed_harder() {
        let mut rgba = raster(120, 40, [0, 0, 0, 0]);
        let mut stroke = PaintStroke::new("Line", vec![(10.0, 20.0), (110.0, 20.0)]);
        stroke.width = 24.0;
        stroke.hardness = 1.0;
        stroke.pressures = vec![0.2, 1.0];
        apply_strokes(&mut rgba, 120, 40, 120.0, 40.0, &[stroke], 0.0);

        let height = |x: u32| (0..40).filter(|&y| alpha_at(&rgba, 120, x, y) > 0).count();
        assert!(
            height(100) > height(20) * 2,
            "the pressed end is much the wider: {} against {}",
            height(100),
            height(20)
        );
        for x in 11..110 {
            assert!(alpha_at(&rgba, 120, x, 20) > 0, "the line broke at x={x}");
        }
    }

    /// A trim cuts the pressures with the points, so a write-on of a pressed
    /// stroke thins where the whole stroke thins.
    #[test]
    fn a_trim_carries_the_pressure_with_it() {
        let mut stroke = PaintStroke::new("Line", vec![(0.0, 0.0), (100.0, 0.0)]);
        stroke.pressures = vec![0.0, 1.0];
        stroke.start = Property::fixed(50.0);
        let (points, pressures) = stroke.drawn_pressed_at(0.0);
        assert_eq!(points, vec![(50.0, 0.0), (100.0, 0.0)]);
        assert_eq!(pressures, vec![0.5, 1.0], "lerped at the cut");

        // A stroke with no pressures keeps none: empty is the constant 1.0 and
        // has to stay empty, or the trim invents a list the file never had.
        let plain = PaintStroke::new("Line", vec![(0.0, 0.0), (100.0, 0.0)]);
        assert!(plain.drawn_pressed_at(0.0).1.is_empty());
    }

    /// The compatibility promise in one test: a stroke nobody pressed writes
    /// the bytes it always wrote, and reads back the same either way.
    #[test]
    fn an_unpressed_stroke_is_absent_from_the_file() {
        let stroke = PaintStroke::new("Line", vec![(1.0, 2.0), (3.0, 4.0)]);
        let json = serde_json::to_string(&stroke).expect("serialise");
        assert!(!json.contains("pressure"), "nothing about pressure: {json}");
        let back: PaintStroke = serde_json::from_str(&json).expect("read back");
        assert!(back.pressures.is_empty(), "and none comes back");

        let mut pressed = stroke.clone();
        pressed.pressures = vec![0.25, 0.75];
        let json = serde_json::to_string(&pressed).expect("serialise");
        let back: PaintStroke = serde_json::from_str(&json).expect("read back");
        assert_eq!(back.pressures, vec![0.25, 0.75]);
    }

    /// No pressures at all and a full press everywhere are the same pixels, to
    /// the byte — the thing that keeps every banked frame of every old project
    /// valid.
    #[test]
    fn a_full_press_paints_exactly_what_no_pressure_does() {
        let plain = PaintStroke::new("Line", vec![(3.0, 3.0), (30.0, 24.0), (50.0, 8.0)]);
        let mut pressed = plain.clone();
        pressed.pressures = vec![1.0, 1.0, 1.0];

        let mut a = raster(60, 30, [0, 0, 0, 0]);
        let mut b = raster(60, 30, [0, 0, 0, 0]);
        apply_strokes(&mut a, 60, 30, 60.0, 30.0, &[plain], 0.0);
        apply_strokes(&mut b, 60, 30, 60.0, 30.0, &[pressed], 0.0);
        assert_eq!(a, b);
    }

    /// A pressure list that stops short — or runs long, or carries nonsense —
    /// is read point by point rather than refused: a missing entry is a full
    /// press, and the engine does not panic on a file it did not write.
    #[test]
    fn a_short_or_absurd_pressure_list_is_read_calmly() {
        let mut stroke = PaintStroke::new("Line", vec![(5.0, 15.0), (55.0, 15.0)]);
        stroke.width = 10.0;
        stroke.pressures = vec![f64::NAN, 9.0, -4.0, 0.5];
        let mut rgba = raster(60, 30, [0, 0, 0, 0]);
        apply_strokes(&mut rgba, 60, 30, 60.0, 30.0, &[stroke], 0.0);
        assert!(alpha_at(&rgba, 60, 55, 15) > 0, "the far end is drawn");
    }
}

//! Shape layers: vector art as a layer's own picture (docs/03-DATA-MODEL.md
//! §7.2, docs/impl/shape-layers.md), and the rasteriser that draws one.
//!
//! # In plain terms
//!
//! A **mask** is a path drawn *on* another layer, deciding which of its pixels
//! show. A **shape layer** is a path that **is** the picture: a rectangle, an
//! ellipse, a drawn path — filled, outlined, and made of numbers rather than
//! pixels, so it stays crisp at any size. After Effects makes one whenever you
//! drag a shape tool with nothing selected, and this is the layer kind that lets
//! Lumit do the same.
//!
//! **The path type is the mask's.** One `BezierPath` in the document, one set of
//! maths, one vertex type crossing the bridge. A shape's path and a mask's path
//! differ in what they *do*, not in what they are — which is why the shape tools
//! could draw both from the same geometry from the day they landed (K-222).
//!
//! **The layer's own size is the art's bounding box**, and it changes as the art
//! is edited. Every other layer kind has a size fixed by its source; this is the
//! first that does not, and anything caching "how big is this layer" has to
//! follow the document's revision rather than assume (docs/impl/shape-layers.md).
//!
//! **The modifiers are fields on the item, not a tree** (K-451). After Effects
//! carries Trim Paths, the Repeater and the rest as entries in a nested group,
//! where their position decides what they act on. Lumit's list is flat, so each
//! modifier is a property of the item it modifies and the order they apply in is
//! fixed and written down here rather than dragged: **trim, then repeat**. The
//! nested tree is still the long-term shape (docs/03 §9.2); nothing stored here
//! stands in its way, because every modifier is absent from the file until it is
//! used.
//!
//! **What is deliberately not here.** Nested groups, wiggle paths, line joins
//! other than round, and animated paths.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::anim::Property;
use crate::mask::{BezierPath, Vertex};
use crate::model::LinearColour;
use crate::pixels::over;

/// One piece of vector art: a path, and how it is painted.
///
/// A flat list of these per layer rather than After Effects' nested groups —
/// groups are a modifier feature and are later work (docs/impl/shape-layers.md).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ShapeItem {
    pub id: Uuid,
    pub name: String,
    pub path: BezierPath,
    /// The colour inside the path. `None` draws no fill, which is how an
    /// outline-only shape is made.
    #[serde(default)]
    pub fill: Option<LinearColour>,
    /// The colour of the outline, and how wide it is in layer pixels. `None`
    /// draws no stroke.
    #[serde(default)]
    pub stroke: Option<LinearColour>,
    #[serde(default)]
    pub stroke_width: f64,
    /// 0..100, like every other opacity in the document.
    #[serde(default = "full_opacity")]
    pub opacity: f64,
    /// **Trim paths** (K-451): where along the path the art begins and ends, as
    /// a per cent of the path's own **arc length**, and how far the pair is slid
    /// along it in degrees (360 is once round).
    ///
    /// Per cent of length rather than of vertex count, for the reason a paint
    /// stroke's write-on gives (K-449): the eye watches length. The trim cuts
    /// the fill as well as the outline — a half-trimmed circle is a half circle,
    /// filled by closing the piece that is left.
    ///
    /// `end` at or below `start` draws nothing at all, which is what the first
    /// frame of a write-on looks like rather than an error.
    #[serde(
        default = "Property::zero",
        with = "crate::mask::still_or_keyed",
        skip_serializing_if = "crate::paint::is_static_zero"
    )]
    pub trim_start: Property,
    #[serde(
        default = "crate::paint::full",
        with = "crate::mask::still_or_keyed",
        skip_serializing_if = "crate::paint::is_static_full"
    )]
    pub trim_end: Property,
    /// Degrees; 360 slides the trimmed piece exactly once round a closed path.
    /// A closed path **wraps** — the piece runs through the seam and comes back
    /// — and an open one does not, because it has no seam to run through.
    #[serde(
        default = "Property::zero",
        with = "crate::mask::still_or_keyed",
        skip_serializing_if = "crate::paint::is_static_zero"
    )]
    pub trim_offset: Property,
    /// Unknown fields from newer Lumit versions, preserved on load/save
    /// (docs/10-FILE-FORMAT.md §1.1).
    #[serde(flatten, default, skip_serializing_if = "serde_json::Map::is_empty")]
    pub extra: serde_json::Map<String, serde_json::Value>,
}

fn full_opacity() -> f64 {
    100.0
}

impl ShapeItem {
    /// A filled item with no outline — what a shape tool makes.
    pub fn filled(name: impl Into<String>, path: BezierPath, fill: LinearColour) -> Self {
        Self {
            id: Uuid::now_v7(),
            name: name.into(),
            path,
            fill: Some(fill),
            stroke: None,
            stroke_width: 0.0,
            opacity: 100.0,
            trim_start: Property::zero(),
            trim_end: crate::paint::full(),
            trim_offset: Property::zero(),
            extra: serde_json::Map::new(),
        }
    }

    /// True when the trim at `t` would change the art — false for the whole
    /// path, which is the state every shape is in until somebody trims it.
    ///
    /// The check exists so an untrimmed shape is rasterised from its **bezier**
    /// rather than from a polyline of it: flattening is exact enough to stroke
    /// with and not exact enough to make the identity case draw different
    /// pixels than it did before there were modifiers.
    fn trims_at(&self, t: f64) -> bool {
        self.trim_start.value_at(t) > 0.0
            || self.trim_end.value_at(t) < 100.0
            || self.trim_offset.value_at(t) != 0.0
    }

    /// The item's art at `t` as a polyline, once the trim has had it — `None`
    /// when the trim leaves the whole path alone (draw the bezier instead), and
    /// `Some(empty)` when it leaves nothing at all.
    fn trimmed_at(&self, t: f64) -> Option<Vec<(f64, f64)>> {
        if !self.trims_at(t) {
            return None;
        }
        let points = flatten_path(&self.path);
        if points.len() < 2 {
            return None;
        }
        let (start, end) = (
            self.trim_start.value_at(t).clamp(0.0, 100.0),
            self.trim_end.value_at(t).clamp(0.0, 100.0),
        );
        // Degrees to per cent of the path: 360 is once round.
        let shift = self.trim_offset.value_at(t) / 3.6;
        Some(if self.path.closed {
            // A closed path has a seam to run through, so the offset moves the
            // seam rather than the window: re-start the polyline `shift` per
            // cent along, and the ordinary trim then cuts one contiguous piece
            // even when it straddles where the first vertex used to be.
            crate::paint::trimmed(&rotated(&points, shift), start, end)
        } else {
            // An open path has two ends. Sliding the window off either of them
            // is the window running out of path, which is what clamping says.
            crate::paint::trimmed(&points, start + shift, end + shift)
        })
    }

    /// The item's bounding box in layer coordinates, its outline included, or
    /// `None` when its path has no vertices.
    ///
    /// The **control points** bound the curve rather than the curve itself: a
    /// cubic never leaves its own control hull, so this is correct and never
    /// too small. It can be a little generous on a strongly curved path, which
    /// costs a few transparent pixels and no correctness.
    pub fn bounds(&self) -> Option<(f64, f64, f64, f64)> {
        // The **whole** path, untrimmed, and for the reason a paint stroke's
        // bounds give (K-449): this answers "where could this art ever be",
        // which is what a layer's natural size has to be if it is not to
        // breathe as a write-on plays.
        let mut out: Option<(f64, f64, f64, f64)> = None;
        for v in &self.path.vertices {
            for (x, y) in [
                v.pos,
                (v.pos.0 + v.tan_in.0, v.pos.1 + v.tan_in.1),
                (v.pos.0 + v.tan_out.0, v.pos.1 + v.tan_out.1),
            ] {
                out = Some(match out {
                    None => (x, y, x, y),
                    Some(b) => (b.0.min(x), b.1.min(y), b.2.max(x), b.3.max(y)),
                });
            }
        }
        let (x0, y0, x1, y1) = out?;
        // Half the stroke sits outside the path, so the box has to hold it.
        let half = if self.stroke.is_some() {
            self.stroke_width.max(0.0) / 2.0
        } else {
            0.0
        };
        Some((x0 - half, y0 - half, x1 + half, y1 + half))
    }
}

/// The bounding box of a whole layer's worth of art, or `None` when there is no
/// art in it — the layer's **natural size** (docs/06 §1.2 step 1).
///
/// This is the number the wireframe, hit-testing and the transform all read, and
/// it moves when the art is edited: a shape layer is the first kind whose size
/// is not fixed by its source.
pub fn contents_bounds(contents: &[ShapeItem]) -> Option<(f64, f64, f64, f64)> {
    let mut out: Option<(f64, f64, f64, f64)> = None;
    for item in contents {
        let Some(b) = item.bounds() else { continue };
        out = Some(match out {
            None => b,
            Some(a) => (a.0.min(b.0), a.1.min(b.1), a.2.max(b.2), a.3.max(b.3)),
        });
    }
    out
}

/// Draw `contents` into a fresh `w`×`h` RGBA buffer whose extent is the box
/// `(min_x, min_y)`–`(max_x, max_y)` in layer coordinates.
///
/// Items are drawn in order, each filled and then stroked, so an item's own
/// outline sits over its own fill. The buffer starts transparent: a shape layer
/// is its art and nothing else.
///
/// The fill's coverage comes from the **mask rasteriser** — the same scanline
/// walk, with the same two vertical subsamples, that decides which pixels a mask
/// gates. One path type, one rasteriser.
///
/// `t` is the **layer's** own clock (K-213), the one its masks and its paint are
/// read on: it is what a keyframed trim is sampled at.
// The box is four numbers and the raster is two; bundling them into a struct
// nobody else holds would be a name for an argument list, not a type.
#[allow(clippy::too_many_arguments)]
pub fn rasterise_contents(
    contents: &[ShapeItem],
    w: u32,
    h: u32,
    min_x: f64,
    min_y: f64,
    max_x: f64,
    max_y: f64,
    t: f64,
) -> Vec<u8> {
    let mut rgba = vec![0u8; (w as usize) * (h as usize) * 4];
    if w == 0 || h == 0 {
        return rgba;
    }
    let sx = f64::from(w) / (max_x - min_x).max(1e-9);
    let sy = f64::from(h) / (max_y - min_y).max(1e-9);

    for item in contents {
        let opacity = (item.opacity.clamp(0.0, 100.0) / 100.0) as f32;
        if opacity <= 0.0 {
            continue;
        }
        // The art at this instant, once the modifiers have had it. `None` is
        // "nothing has changed it", which draws the bezier itself.
        let trimmed = item.trimmed_at(t);
        if trimmed.as_ref().is_some_and(|p| p.len() < 2) {
            continue; // trimmed away to nothing: the first frame of a write-on
        }
        // The path in the buffer's own coordinates: the rasteriser works from
        // the origin, so the art is shifted to the box's top-left first.
        let shifted = match &trimmed {
            // A trimmed piece is a polyline, and a polyline is a bezier whose
            // handles are all zero — one path type, still (K-237). Closed so
            // the fill has something to fill: a half-trimmed circle fills as a
            // half circle, exactly as After Effects draws it.
            Some(points) => polyline_path(points, item.path.closed),
            None => item.path.clone(),
        };
        let shifted = shift_path(&shifted, -min_x, -min_y);

        if let Some(fill) = item.fill {
            let coverage = crate::mask::rasterise(&shifted, w, h, sx, sy);
            let colour = crate::pixels::solid_rgba(fill);
            let alpha = f32::from(colour[3]) / 255.0;
            for (px, c) in rgba.chunks_exact_mut(4).zip(coverage) {
                over(
                    px,
                    [colour[0], colour[1], colour[2]],
                    (f32::from(c) / 255.0) * opacity * alpha,
                );
            }
        }

        if let (Some(stroke), true) = (item.stroke, item.stroke_width > 0.0) {
            // A stroke is a brush run along the path, which is exactly what the
            // paint rasteriser already does — one widened-path implementation
            // for both, rather than two that can disagree (K-237).
            // The outline follows the trimmed piece, and it is drawn **open**
            // whatever the path was: a trim is what turns a closed ring into a
            // stroke with two ends.
            let points = match &trimmed {
                Some(points) => points
                    .iter()
                    .map(|&(x, y)| (x - min_x, y - min_y))
                    .collect(),
                None => flatten_path(&shifted),
            };
            if points.len() >= 2 {
                let brush = crate::paint::PaintStroke {
                    id: item.id,
                    name: item.name.clone(),
                    points,
                    colour: stroke,
                    width: item.stroke_width,
                    // A vector outline has a hard edge; the rasteriser keeps
                    // half a pixel of falloff whatever this says, which is the
                    // anti-aliasing rather than a soft brush.
                    hardness: 1.0,
                    // A vector outline is round-capped and round-joined, which
                    // is what the round brush already draws.
                    shape: crate::paint::BrushShape::Round,
                    opacity: item.opacity,
                    // A vector outline is drawn whole; a shape item's own trim
                    // paths are a shape-layer feature, not the brush's (K-449),
                    // so the whole path every time and no clock to read.
                    start: crate::anim::Property::zero(),
                    end: crate::anim::Property::fixed(100.0),
                    mode: crate::paint::PaintMode::Paint,
                    // A shape item's outline lays its colour down; a blend of
                    // its own would be a shape-layer feature (K-450).
                    blend: crate::model::BlendMode::Normal,
                    clone_offset: (0.0, 0.0),
                    extra: serde_json::Map::new(),
                };
                crate::paint::apply_strokes(
                    &mut rgba,
                    w,
                    h,
                    max_x - min_x,
                    max_y - min_y,
                    std::slice::from_ref(&brush),
                    0.0,
                );
            }
        }
    }
    rgba
}

/// How many straight steps each cubic segment becomes when a path is walked for
/// stroking.
///
/// Sixteen is smooth to well past the sizes a stroke is drawn at, and a fixed
/// count keeps the result identical on every machine (docs/14 §determinism).
const FLATTEN_STEPS: usize = 16;

/// The path as a polyline: each cubic segment walked in [`FLATTEN_STEPS`] steps.
/// A closed path returns to its first point, so its outline closes.
pub fn flatten_path(path: &BezierPath) -> Vec<(f64, f64)> {
    let n = path.vertices.len();
    if n == 0 {
        return Vec::new();
    }
    if n == 1 {
        return vec![path.vertices[0].pos];
    }
    let mut out = vec![path.vertices[0].pos];
    let last = if path.closed { n } else { n - 1 };
    for i in 0..last {
        let a = &path.vertices[i];
        let b = &path.vertices[(i + 1) % n];
        let p0 = a.pos;
        let p1 = (a.pos.0 + a.tan_out.0, a.pos.1 + a.tan_out.1);
        let p2 = (b.pos.0 + b.tan_in.0, b.pos.1 + b.tan_in.1);
        let p3 = b.pos;
        for step in 1..=FLATTEN_STEPS {
            let t = step as f64 / FLATTEN_STEPS as f64;
            out.push(cubic_at(p0, p1, p2, p3, t));
        }
    }
    out
}

fn cubic_at(p0: (f64, f64), p1: (f64, f64), p2: (f64, f64), p3: (f64, f64), t: f64) -> (f64, f64) {
    let u = 1.0 - t;
    let (a, b, c, d) = (u * u * u, 3.0 * u * u * t, 3.0 * u * t * t, t * t * t);
    (
        a * p0.0 + b * p1.0 + c * p2.0 + d * p3.0,
        a * p0.1 + b * p1.1 + c * p2.1 + d * p3.1,
    )
}

/// A polyline as a [`BezierPath`]: every handle zero, so the segments stay
/// straight and the one rasteriser in the crate can draw it.
fn polyline_path(points: &[(f64, f64)], closed: bool) -> BezierPath {
    BezierPath {
        vertices: points
            .iter()
            .map(|&pos| Vertex {
                pos,
                tan_in: (0.0, 0.0),
                tan_out: (0.0, 0.0),
            })
            .collect(),
        closed,
    }
}

/// `points` re-started `shift` per cent of its own length along, wrapping — the
/// same ring of points with the seam moved.
///
/// Only meaningful for a closed polyline (one whose last point is its first),
/// which is what a closed path flattens to. A shift of zero, or a path with no
/// length to measure, hands the points straight back.
fn rotated(points: &[(f64, f64)], shift: f64) -> Vec<(f64, f64)> {
    let shift = shift.rem_euclid(100.0);
    if shift == 0.0 || points.len() < 2 {
        return points.to_vec();
    }
    // Walk to the point `shift` per cent along, cut there, and hand back the
    // two pieces the other way round. `trimmed` already knows how to cut a
    // polyline by length, so the two cuts are the two halves of the ring.
    let head = crate::paint::trimmed(points, 0.0, shift);
    let tail = crate::paint::trimmed(points, shift, 100.0);
    if head.len() < 2 || tail.len() < 2 {
        return points.to_vec();
    }
    let mut out = tail;
    // The join is one shared point, not two: the tail ends where the head
    // begins.
    out.extend_from_slice(&head[1..]);
    out
}

fn shift_path(path: &BezierPath, dx: f64, dy: f64) -> BezierPath {
    BezierPath {
        vertices: path
            .vertices
            .iter()
            .map(|v| crate::mask::Vertex {
                pos: (v.pos.0 + dx, v.pos.1 + dy),
                tan_in: v.tan_in,
                tan_out: v.tan_out,
            })
            .collect(),
        closed: path.closed,
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use crate::mask::Mask;

    fn square(x: f64, y: f64, side: f64) -> BezierPath {
        Mask::rectangle(x, y, side, side).path
    }

    fn item(path: BezierPath) -> ShapeItem {
        ShapeItem::filled("Rectangle", path, LinearColour([1.0, 0.0, 0.0, 1.0]))
    }

    fn alpha_at(rgba: &[u8], w: u32, x: u32, y: u32) -> u8 {
        rgba[((y * w + x) as usize) * 4 + 3]
    }

    fn rgb_at(rgba: &[u8], w: u32, x: u32, y: u32) -> [u8; 3] {
        let i = ((y * w + x) as usize) * 4;
        [rgba[i], rgba[i + 1], rgba[i + 2]]
    }

    #[test]
    fn a_layers_size_is_the_box_its_art_fills() {
        let contents = vec![item(square(10.0, 20.0, 30.0))];
        assert_eq!(contents_bounds(&contents), Some((10.0, 20.0, 40.0, 50.0)));

        // Two items: the box holds both.
        let contents = vec![item(square(10.0, 20.0, 30.0)), item(square(-5.0, 0.0, 8.0))];
        assert_eq!(contents_bounds(&contents), Some((-5.0, 0.0, 40.0, 50.0)));

        assert!(contents_bounds(&[]).is_none());
    }

    #[test]
    fn a_stroke_widens_the_box_by_half_its_width() {
        let mut it = item(square(0.0, 0.0, 10.0));
        it.stroke = Some(LinearColour::BLACK);
        it.stroke_width = 4.0;
        assert_eq!(it.bounds(), Some((-2.0, -2.0, 12.0, 12.0)));
    }

    #[test]
    fn the_box_holds_the_whole_curve_not_just_its_ends() {
        // One vertex with a long handle: the control point is outside the
        // straight line between the vertices, and the box has to hold it.
        let mut path = square(0.0, 0.0, 10.0);
        path.vertices[0].tan_out = (0.0, -20.0);
        let bounds = item(path).bounds().expect("bounds");
        assert!(
            bounds.1 <= -20.0,
            "the handle reaches above the box: {bounds:?}"
        );
    }

    #[test]
    fn a_filled_shape_draws_its_colour_inside_and_nothing_outside() {
        let contents = vec![item(square(0.0, 0.0, 20.0))];
        // Rasterised into its own bounding box at 1:1.
        let rgba = rasterise_contents(&contents, 20, 20, 0.0, 0.0, 20.0, 20.0, 0.0);
        assert_eq!(alpha_at(&rgba, 20, 10, 10), 255, "inside the square");
        assert_eq!(rgb_at(&rgba, 20, 10, 10), [255, 0, 0]);

        // A smaller square in a bigger box leaves the rest transparent.
        let contents = vec![item(square(5.0, 5.0, 5.0))];
        let rgba = rasterise_contents(&contents, 20, 20, 0.0, 0.0, 20.0, 20.0, 0.0);
        assert_eq!(alpha_at(&rgba, 20, 7, 7), 255);
        assert_eq!(alpha_at(&rgba, 20, 18, 18), 0, "outside the art");
    }

    /// The whole reason a shape layer is vector: the same art at twice the
    /// resolution is the same picture, twice as big.
    #[test]
    fn a_shape_is_drawn_at_whatever_resolution_it_is_asked_for() {
        let contents = vec![item(square(0.0, 0.0, 10.0))];
        let small = rasterise_contents(&contents, 10, 10, 0.0, 0.0, 10.0, 10.0, 0.0);
        let big = rasterise_contents(&contents, 40, 40, 0.0, 0.0, 10.0, 10.0, 0.0);
        assert_eq!(alpha_at(&small, 10, 5, 5), 255);
        assert_eq!(alpha_at(&big, 40, 20, 20), 255);
        assert_eq!(big.len(), small.len() * 16);
    }

    #[test]
    fn an_outline_is_drawn_round_the_path() {
        let mut it = item(square(4.0, 4.0, 12.0));
        it.fill = None;
        it.stroke = Some(LinearColour([0.0, 1.0, 0.0, 1.0]));
        it.stroke_width = 3.0;
        let rgba = rasterise_contents(&[it], 20, 20, 0.0, 0.0, 20.0, 20.0, 0.0);

        assert!(alpha_at(&rgba, 20, 4, 10) > 200, "on the left edge");
        assert_eq!(
            alpha_at(&rgba, 20, 10, 10),
            0,
            "and nothing in the middle: this item has no fill"
        );
    }

    #[test]
    fn a_fill_and_a_stroke_are_both_drawn_and_the_stroke_is_on_top() {
        let mut it = item(square(4.0, 4.0, 12.0));
        it.stroke = Some(LinearColour([0.0, 1.0, 0.0, 1.0]));
        it.stroke_width = 3.0;
        let rgba = rasterise_contents(&[it], 20, 20, 0.0, 0.0, 20.0, 20.0, 0.0);

        assert_eq!(rgb_at(&rgba, 20, 10, 10), [255, 0, 0], "the fill inside");
        assert_eq!(
            rgb_at(&rgba, 20, 4, 10),
            [0, 255, 0],
            "the stroke on the edge"
        );
    }

    #[test]
    fn opacity_fades_the_item() {
        let mut it = item(square(0.0, 0.0, 20.0));
        it.opacity = 50.0;
        let rgba = rasterise_contents(&[it], 20, 20, 0.0, 0.0, 20.0, 20.0, 0.0);
        let a = alpha_at(&rgba, 20, 10, 10);
        assert!((120..=136).contains(&a), "half opaque, got {a}");
    }

    #[test]
    fn items_are_drawn_in_order() {
        let under = item(square(0.0, 0.0, 20.0));
        let mut over = item(square(0.0, 0.0, 20.0));
        over.fill = Some(LinearColour([0.0, 0.0, 1.0, 1.0]));
        let rgba = rasterise_contents(&[under, over], 20, 20, 0.0, 0.0, 20.0, 20.0, 0.0);
        assert_eq!(
            rgb_at(&rgba, 20, 10, 10),
            [0, 0, 255],
            "the later item wins"
        );
    }

    #[test]
    fn nothing_in_it_draws_nothing_and_never_panics() {
        assert!(rasterise_contents(&[], 4, 4, 0.0, 0.0, 4.0, 4.0, 0.0)
            .iter()
            .all(|&b| b == 0));
        assert!(rasterise_contents(&[], 0, 0, 0.0, 0.0, 1.0, 1.0, 0.0).is_empty());

        // A path with no vertices, and one with a single vertex.
        let empty = item(BezierPath {
            vertices: Vec::new(),
            closed: true,
        });
        assert!(empty.bounds().is_none());
        assert!(rasterise_contents(&[empty], 4, 4, 0.0, 0.0, 4.0, 4.0, 0.0)
            .iter()
            .all(|&b| b == 0));
    }

    #[test]
    fn flattening_walks_the_curve_and_closes_a_closed_path() {
        let path = square(0.0, 0.0, 10.0);
        let points = flatten_path(&path);
        // Four segments, sixteen steps each, plus the first point.
        assert_eq!(points.len(), 4 * FLATTEN_STEPS + 1);
        assert_eq!(points.first(), points.last(), "a closed path comes home");

        let open = BezierPath {
            vertices: path.vertices.clone(),
            closed: false,
        };
        assert_eq!(flatten_path(&open).len(), 3 * FLATTEN_STEPS + 1);
    }

    /// Total length of a polyline, for the trim tests below.
    fn length(points: &[(f64, f64)]) -> f64 {
        points
            .windows(2)
            .map(|p| ((p[1].0 - p[0].0).powi(2) + (p[1].1 - p[0].1).powi(2)).sqrt())
            .sum()
    }

    fn ink(rgba: &[u8]) -> u32 {
        rgba.chunks_exact(4).map(|p| u32::from(p[3])).sum()
    }

    #[test]
    fn an_untrimmed_shape_is_drawn_from_its_curve_and_not_a_polyline_of_it() {
        let it = item(square(0.0, 0.0, 20.0));
        assert!(!it.trims_at(0.0));
        assert!(it.trimmed_at(0.0).is_none(), "nothing to cut");

        // The whole path asked for explicitly is still the whole path.
        let mut whole = item(square(0.0, 0.0, 20.0));
        whole.trim_start = Property::fixed(0.0);
        whole.trim_end = Property::fixed(100.0);
        assert!(whole.trimmed_at(0.0).is_none());
    }

    #[test]
    fn a_trim_cuts_the_path_by_its_own_length() {
        let mut it = item(square(0.0, 0.0, 20.0));
        it.trim_end = Property::fixed(50.0);
        let half = it.trimmed_at(0.0).expect("a trimmed piece");
        let whole = length(&flatten_path(&it.path));
        assert!(
            (length(&half) - whole / 2.0).abs() < 1e-6,
            "half the perimeter, got {} of {whole}",
            length(&half)
        );
        // It starts where the path starts.
        assert_eq!(half.first().copied(), Some(it.path.vertices[0].pos));
    }

    #[test]
    fn a_trimmed_fill_closes_the_piece_that_is_left() {
        let mut it = item(square(0.0, 0.0, 20.0));
        it.trim_end = Property::fixed(50.0);
        let rgba = rasterise_contents(&[it], 20, 20, 0.0, 0.0, 20.0, 20.0, 0.0);
        // Half a square's perimeter, closed, is a triangle over two corners —
        // it covers less than the whole square and more than nothing.
        let full = rasterise_contents(
            &[item(square(0.0, 0.0, 20.0))],
            20,
            20,
            0.0,
            0.0,
            20.0,
            20.0,
            0.0,
        );
        assert!(ink(&rgba) > 0, "something is left");
        assert!(ink(&rgba) < ink(&full), "and less than the whole square");
    }

    #[test]
    fn an_end_at_or_below_the_start_draws_nothing() {
        let mut it = item(square(0.0, 0.0, 20.0));
        it.trim_start = Property::fixed(60.0);
        it.trim_end = Property::fixed(40.0);
        let rgba = rasterise_contents(&[it], 20, 20, 0.0, 0.0, 20.0, 20.0, 0.0);
        assert_eq!(ink(&rgba), 0, "the first frame of a write-on");
    }

    #[test]
    fn the_offset_slides_the_piece_round_a_closed_path_without_shortening_it() {
        let quarter = |offset: f64| {
            let mut it = item(square(0.0, 0.0, 20.0));
            it.trim_end = Property::fixed(25.0);
            it.trim_offset = Property::fixed(offset);
            it.trimmed_at(0.0).expect("a piece")
        };
        let (a, b) = (quarter(0.0), quarter(90.0));
        assert!(
            (length(&a) - length(&b)).abs() < 1e-6,
            "the same length, slid: {} vs {}",
            length(&a),
            length(&b)
        );
        assert_ne!(a.first(), b.first(), "and it starts somewhere else");
        // 360 degrees is once round: back where it began.
        let round = quarter(360.0);
        assert!((round[0].0 - a[0].0).abs() < 1e-6 && (round[0].1 - a[0].1).abs() < 1e-6);
    }

    #[test]
    fn an_open_paths_offset_runs_the_window_off_the_end_rather_than_wrapping() {
        let mut path = square(0.0, 0.0, 20.0);
        path.closed = false;
        let mut it = item(path);
        it.trim_start = Property::fixed(0.0);
        it.trim_end = Property::fixed(50.0);
        it.trim_offset = Property::fixed(360.0); // a whole path's worth
        let piece = it.trimmed_at(0.0).expect("a piece");
        assert!(piece.len() < 2, "slid clean off: {piece:?}");
    }

    #[test]
    fn a_keyed_trim_is_read_on_the_layers_clock() {
        let mut it = item(square(0.0, 0.0, 20.0));
        let key = |secs: i64, value: f64| crate::anim::Keyframe {
            time: crate::time::Rational::new(secs, 1).expect("a whole second"),
            value,
            interp_in: crate::anim::SideInterp::Linear,
            interp_out: crate::anim::SideInterp::Linear,
        };
        it.trim_end.animation = crate::anim::Animation::Keyframed(vec![key(0, 0.0), key(1, 100.0)]);
        let at = |t: f64| {
            ink(&rasterise_contents(
                &[it.clone()],
                20,
                20,
                0.0,
                0.0,
                20.0,
                20.0,
                t,
            ))
        };
        assert_eq!(at(0.0), 0, "nothing drawn yet");
        assert!(at(1.0) > at(0.5), "and it fills in as it plays");
    }

    #[test]
    fn an_untrimmed_item_is_absent_from_the_file() {
        let json = serde_json::to_string(&item(square(0.0, 0.0, 4.0))).expect("json");
        assert!(!json.contains("trim"), "nothing about a trim: {json}");
        let back: ShapeItem = serde_json::from_str(&json).expect("round trip");
        assert_eq!(back.trim_end.value_at(0.0), 100.0, "the default comes back");
    }

    #[test]
    fn drawing_a_shape_is_deterministic() {
        let contents = vec![item(square(2.0, 3.0, 9.0))];
        let once = rasterise_contents(&contents, 16, 16, 0.0, 0.0, 16.0, 16.0, 0.0);
        let twice = rasterise_contents(&contents, 16, 16, 0.0, 0.0, 16.0, 16.0, 0.0);
        assert_eq!(once, twice);
    }
}

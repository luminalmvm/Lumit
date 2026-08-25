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
//! **What is deliberately not here.** Nested groups, the shape *modifiers*
//! (repeater, trim paths, wiggle, offset), gradient fills, dashed strokes, line
//! joins other than round, and animated paths. Each is a real feature; none of
//! them changes the shape of what is stored, which is what this first cut is
//! for.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::mask::BezierPath;
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
            extra: serde_json::Map::new(),
        }
    }

    /// The item's bounding box in layer coordinates, its outline included, or
    /// `None` when its path has no vertices.
    ///
    /// The **control points** bound the curve rather than the curve itself: a
    /// cubic never leaves its own control hull, so this is correct and never
    /// too small. It can be a little generous on a strongly curved path, which
    /// costs a few transparent pixels and no correctness.
    pub fn bounds(&self) -> Option<(f64, f64, f64, f64)> {
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
pub fn rasterise_contents(
    contents: &[ShapeItem],
    w: u32,
    h: u32,
    min_x: f64,
    min_y: f64,
    max_x: f64,
    max_y: f64,
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
        // The path in the buffer's own coordinates: the rasteriser works from
        // the origin, so the art is shifted to the box's top-left first.
        let shifted = shift_path(&item.path, -min_x, -min_y);

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
            let points = flatten_path(&shifted);
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
                    opacity: item.opacity,
                    mode: crate::paint::PaintMode::Paint,
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
        let rgba = rasterise_contents(&contents, 20, 20, 0.0, 0.0, 20.0, 20.0);
        assert_eq!(alpha_at(&rgba, 20, 10, 10), 255, "inside the square");
        assert_eq!(rgb_at(&rgba, 20, 10, 10), [255, 0, 0]);

        // A smaller square in a bigger box leaves the rest transparent.
        let contents = vec![item(square(5.0, 5.0, 5.0))];
        let rgba = rasterise_contents(&contents, 20, 20, 0.0, 0.0, 20.0, 20.0);
        assert_eq!(alpha_at(&rgba, 20, 7, 7), 255);
        assert_eq!(alpha_at(&rgba, 20, 18, 18), 0, "outside the art");
    }

    /// The whole reason a shape layer is vector: the same art at twice the
    /// resolution is the same picture, twice as big.
    #[test]
    fn a_shape_is_drawn_at_whatever_resolution_it_is_asked_for() {
        let contents = vec![item(square(0.0, 0.0, 10.0))];
        let small = rasterise_contents(&contents, 10, 10, 0.0, 0.0, 10.0, 10.0);
        let big = rasterise_contents(&contents, 40, 40, 0.0, 0.0, 10.0, 10.0);
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
        let rgba = rasterise_contents(&[it], 20, 20, 0.0, 0.0, 20.0, 20.0);

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
        let rgba = rasterise_contents(&[it], 20, 20, 0.0, 0.0, 20.0, 20.0);

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
        let rgba = rasterise_contents(&[it], 20, 20, 0.0, 0.0, 20.0, 20.0);
        let a = alpha_at(&rgba, 20, 10, 10);
        assert!((120..=136).contains(&a), "half opaque, got {a}");
    }

    #[test]
    fn items_are_drawn_in_order() {
        let under = item(square(0.0, 0.0, 20.0));
        let mut over = item(square(0.0, 0.0, 20.0));
        over.fill = Some(LinearColour([0.0, 0.0, 1.0, 1.0]));
        let rgba = rasterise_contents(&[under, over], 20, 20, 0.0, 0.0, 20.0, 20.0);
        assert_eq!(
            rgb_at(&rgba, 20, 10, 10),
            [0, 0, 255],
            "the later item wins"
        );
    }

    #[test]
    fn nothing_in_it_draws_nothing_and_never_panics() {
        assert!(rasterise_contents(&[], 4, 4, 0.0, 0.0, 4.0, 4.0)
            .iter()
            .all(|&b| b == 0));
        assert!(rasterise_contents(&[], 0, 0, 0.0, 0.0, 1.0, 1.0).is_empty());

        // A path with no vertices, and one with a single vertex.
        let empty = item(BezierPath {
            vertices: Vec::new(),
            closed: true,
        });
        assert!(empty.bounds().is_none());
        assert!(rasterise_contents(&[empty], 4, 4, 0.0, 0.0, 4.0, 4.0)
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

    #[test]
    fn drawing_a_shape_is_deterministic() {
        let contents = vec![item(square(2.0, 3.0, 9.0))];
        let once = rasterise_contents(&contents, 16, 16, 0.0, 0.0, 16.0, 16.0);
        let twice = rasterise_contents(&contents, 16, 16, 0.0, 0.0, 16.0, 16.0);
        assert_eq!(once, twice);
    }
}

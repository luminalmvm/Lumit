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
//! **What is deliberately not here.** Pressure, tilt, spacing curves, brush
//! shapes other than round, stroke start/end times (After Effects' write-on) and
//! any GPU path. Each is a real feature; none of them changes the shape of what
//! is stored, which is what this first cut is for.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::model::LinearColour;
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

/// One stroke: the path the pointer took and how it was painted.
///
/// The path is a **polyline** rather than a bezier: it is a record of a gesture,
/// sampled as it happened, not a shape anyone will edit vertex by vertex. Masks
/// and shape layers are the bezier things (K-222); a stroke is a stroke.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PaintStroke {
    pub id: Uuid,
    pub name: String,
    /// The pointer's path in **layer** coordinates, in the order it was drawn.
    /// One point is a dab; two or more are joined by round-capped segments.
    pub points: Vec<(f64, f64)>,
    pub colour: LinearColour,
    /// The brush's **diameter** in layer pixels.
    pub width: f64,
    /// 0 = fully soft (fades from the centre out), 1 = a hard edge with only
    /// enough falloff left to keep it from stair-stepping.
    pub hardness: f64,
    /// 0..100, like every other opacity in the document.
    pub opacity: f64,
    pub mode: PaintMode,
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
            colour: LinearColour([1.0, 1.0, 1.0, 1.0]),
            width: 20.0,
            hardness: 0.8,
            opacity: 100.0,
            mode: PaintMode::Paint,
            clone_offset: (0.0, 0.0),
            extra: serde_json::Map::new(),
        }
    }

    /// The stroke's bounding box in layer coordinates, brush width included, or
    /// `None` when it has no points. Used to skip strokes that cannot touch the
    /// raster being drawn.
    pub fn bounds(&self) -> Option<(f64, f64, f64, f64)> {
        let first = *self.points.first()?;
        let r = self.width.max(0.0) / 2.0;
        let mut b = (first.0, first.1, first.0, first.1);
        for &(x, y) in &self.points {
            b.0 = b.0.min(x);
            b.1 = b.1.min(y);
            b.2 = b.2.max(x);
            b.3 = b.3.max(y);
        }
        Some((b.0 - r, b.1 - r, b.2 + r, b.3 + r))
    }
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
        let Some(rect) = fill_coverage(&mut coverage, stroke, w, h, sx, sy) else {
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
pub fn coverage_of(stroke: &PaintStroke, w: u32, h: u32, sx: f64, sy: f64) -> Option<Vec<u8>> {
    let mut coverage = vec![0u8; (w as usize) * (h as usize)];
    fill_coverage(&mut coverage, stroke, w, h, sx, sy)?;
    Some(coverage)
}

/// [`coverage_of`] into a reused buffer: clears the stroke's own bounds
/// rectangle and stamps into it, returning that rectangle as inclusive raster
/// coordinates `(x0, y0, x1, y1)` so the composite can visit only the pixels
/// the stroke can have touched. `coverage` must be `w × h`.
fn fill_coverage(
    coverage: &mut [u8],
    stroke: &PaintStroke,
    w: u32,
    h: u32,
    sx: f64,
    sy: f64,
) -> Option<(u32, u32, u32, u32)> {
    if stroke.points.is_empty() || stroke.width <= 0.0 || stroke.opacity <= 0.0 {
        return None;
    }
    let (min_x, min_y, max_x, max_y) = stroke.bounds()?;
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
    let radius = (stroke.width / 2.0 * scale).max(0.5);
    let feather = (radius * (1.0 - stroke.hardness.clamp(0.0, 1.0))).max(MIN_FEATHER);
    let solid = (radius - feather).max(0.0);

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
    for (cx, cy) in dabs(&stroke.points, radius) {
        let px = cx * sx;
        let py = cy * sy;
        stamp(coverage, w, h, px, py, radius, solid, feather);
    }
    Some((x0, y0, x1, y1))
}

/// Where the dabs along a stroke's polyline go, in layer coordinates.
///
/// One point gives one dab. Each segment is walked at [`DAB_SPACING`] of the
/// radius so the marks overlap into a line; the segment's far end is always
/// stamped, so a stroke never falls short of where the pointer stopped.
fn dabs(points: &[(f64, f64)], radius_px: f64) -> Vec<(f64, f64)> {
    let mut out = Vec::new();
    let Some(&first) = points.first() else {
        return out;
    };
    out.push(first);
    if points.len() == 1 {
        return out;
    }
    // The step is measured in layer units: the radius is in raster pixels, so a
    // stroke on a layer being drawn small still gets dabs close enough together
    // to join up.
    let step = (radius_px * DAB_SPACING).max(0.25);
    for pair in points.windows(2) {
        let (x0, y0) = pair[0];
        let (x1, y1) = pair[1];
        let dx = x1 - x0;
        let dy = y1 - y0;
        let length = (dx * dx + dy * dy).sqrt();
        if length <= f64::EPSILON {
            continue;
        }
        let count = (length / step).ceil().min(4096.0) as usize;
        for i in 1..=count {
            let t = i as f64 / count as f64;
            out.push((x0 + dx * t, y0 + dy * t));
        }
    }
    out
}

/// One round dab into the coverage buffer, taking the greatest coverage at each
/// pixel rather than adding.
///
/// Greatest, not sum: the dabs along a stroke overlap heavily by design, and
/// adding them would make the middle of a slow stroke opaque and its ends thin.
/// A stroke's *own* opacity is applied once, when it is composited.
#[allow(clippy::too_many_arguments)]
fn stamp(
    coverage: &mut [u8],
    w: u32,
    h: u32,
    cx: f64,
    cy: f64,
    radius: f64,
    solid: f64,
    feather: f64,
) {
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
            let d = (dx * dx + dy * dy).sqrt();
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
                    over(px, [colour[0], colour[1], colour[2]], a);
                }
                PaintMode::Erase => {
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
                    over(px, [src[0], src[1], src[2]], a);
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
        apply_strokes(&mut rgba, 40, 40, 40.0, 40.0, &[stroke]);

        assert_eq!(alpha_at(&rgba, 40, 20, 20), 255, "the middle of the dab");
        assert_eq!(alpha_at(&rgba, 40, 20, 30), 0, "well outside its radius");
        assert_eq!(alpha_at(&rgba, 40, 0, 0), 0, "and the far corner");
    }

    #[test]
    fn a_stroke_joins_its_points_up() {
        let mut rgba = raster(80, 20, [0, 0, 0, 0]);
        let mut stroke = PaintStroke::new("Line", vec![(5.0, 10.0), (75.0, 10.0)]);
        stroke.width = 6.0;
        apply_strokes(&mut rgba, 80, 20, 80.0, 20.0, &[stroke]);

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
            apply_strokes(&mut rgba, 40, 40, 40.0, 40.0, &[stroke]);
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

    #[test]
    fn opacity_scales_the_mark() {
        let mut rgba = raster(20, 20, [0, 0, 0, 0]);
        let mut stroke = PaintStroke::new("Dab", vec![(10.0, 10.0)]);
        stroke.width = 8.0;
        stroke.opacity = 50.0;
        apply_strokes(&mut rgba, 20, 20, 20.0, 20.0, &[stroke]);
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
        apply_strokes(&mut half, 20, 20, 40.0, 40.0, &[stroke.clone()]);
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
        apply_strokes(&mut rgba, 20, 20, 20.0, 20.0, &[stroke]);

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
        apply_strokes(&mut rgba, 20, 20, 20.0, 20.0, &[stroke]);

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

        apply_strokes(&mut rgba, 40, 10, 40.0, 10.0, &[paint, clone]);

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
        apply_strokes(&mut rgba, 8, 8, 8.0, 8.0, &[empty, zero, clear]);
        assert_eq!(rgba, before);
    }

    #[test]
    fn a_stroke_off_the_layer_is_skipped_rather_than_drawn() {
        let mut rgba = raster(8, 8, [0, 0, 0, 0]);
        let mut stroke = PaintStroke::new("Away", vec![(500.0, 500.0)]);
        stroke.width = 4.0;
        apply_strokes(&mut rgba, 8, 8, 8.0, 8.0, &[stroke]);
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
        apply_strokes(&mut short, 8, 8, 8.0, 8.0, &[stroke]);
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
        apply_strokes(&mut rgba, 40, 40, 40.0, 40.0, &[red, blue]);

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
        apply_strokes(&mut once, 20, 20, 20.0, 20.0, &strokes);
        apply_strokes(&mut twice, 20, 20, 20.0, 20.0, &strokes);
        assert_eq!(once, twice);
    }
}

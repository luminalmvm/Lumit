//! Gradients on a shape's outline: what the file keeps, and the painter that
//! colours an outline by a ramp.
//!
//! An outline is marked by the same brush runs a flat one is, in white, and
//! the ramp colours what they marked. So a gradient outline keeps its width,
//! taper, dashes and round ends, and only its colour changes.
//!
//! **Along the line** is the one reading only an outline has: each line runs
//! from the start colour where it begins to the end colour where it ends. A
//! trimmed line is measured as it is drawn, as a taper is, so the colours
//! follow the part that shows. A pixel's place along the line is that of the
//! nearest point on it.

use serde::{Deserialize, Serialize};

use super::geometry::dist;
use super::raster::Ramp;
use super::GradientStop;
use crate::anim::Property;
use crate::paint::PaintStroke;
use crate::pixels::over;

/// The ramp runs across the art, projected onto the line between its points.
pub const LINEAR: u32 = 1;
/// The ramp runs out from its start point, the end point on its outer edge.
pub const RADIAL: u32 = 2;
/// The ramp runs along the outline's own line, start to end. An outline
/// only: a fill has no length to run along, and reads it as linear.
pub const ALONG: u32 = 3;

/// **A gradient outline**: the stroke's colour at the start of the ramp,
/// running through the stops to [`colour`](Self::colour) at its end. The
/// numbers mean what a gradient fill's do.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StrokeGradient {
    /// [`LINEAR`], [`RADIAL`] or [`ALONG`]. A choice rather than a
    /// `Property`, for the fill's reason: what a ramp is does not tween. 0
    /// draws the flat stroke colour.
    pub kind: u32,
    /// The colour at the far end, each channel keyable. `None` is black.
    #[serde(
        default,
        with = "crate::shape::colour::keyed",
        skip_serializing_if = "Option::is_none"
    )]
    pub colour: Option<[Property; 4]>,
    /// Where the ramp starts and ends, in layer pixels. Along the line has
    /// no use for them.
    #[serde(
        default = "Property::zero",
        with = "crate::mask::still_or_keyed",
        skip_serializing_if = "crate::paint::is_static_zero"
    )]
    pub start_x: Property,
    #[serde(
        default = "Property::zero",
        with = "crate::mask::still_or_keyed",
        skip_serializing_if = "crate::paint::is_static_zero"
    )]
    pub start_y: Property,
    #[serde(
        default = "Property::zero",
        with = "crate::mask::still_or_keyed",
        skip_serializing_if = "crate::paint::is_static_zero"
    )]
    pub end_x: Property,
    #[serde(
        default = "Property::zero",
        with = "crate::mask::still_or_keyed",
        skip_serializing_if = "crate::paint::is_static_zero"
    )]
    pub end_y: Property,
    /// How opaque each end is, per cent.
    #[serde(
        default = "crate::paint::full",
        with = "crate::mask::still_or_keyed",
        skip_serializing_if = "crate::paint::is_static_full"
    )]
    pub start_opacity: Property,
    #[serde(
        default = "crate::paint::full",
        with = "crate::mask::still_or_keyed",
        skip_serializing_if = "crate::paint::is_static_full"
    )]
    pub end_opacity: Property,
    /// The colours between the two ends. Empty is a two-colour ramp.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub stops: Vec<GradientStop>,
    /// Unknown fields from newer Lumit versions, preserved on load and save.
    #[serde(flatten, default, skip_serializing_if = "serde_json::Map::is_empty")]
    pub extra: serde_json::Map<String, serde_json::Value>,
}

/// Lay a gradient outline into `rgba`, a `size` raster whose extent is
/// `natural` in layer pixels. `brushes` are the outline's runs in white;
/// `lines` are its lines before the dashes cut them, which along the line
/// is measured on. `scale` is the raster's pixels per layer pixel in x and
/// y, and how far from its line the outline reaches, in raster pixels.
pub(super) fn paint_ramped(
    rgba: &mut [u8],
    (w, h): (u32, u32),
    natural: (f64, f64),
    brushes: &[PaintStroke],
    ramp: &Ramp,
    lines: &[Vec<(f64, f64)>],
    (sx, sy, reach): (f64, f64, f64),
) {
    if rgba.len() != (w as usize) * (h as usize) * 4 {
        return;
    }
    let mut marked = vec![0u8; rgba.len()];
    crate::paint::apply_strokes(&mut marked, w, h, natural.0, natural.1, brushes, 0.0);
    let field = ramp
        .along()
        .then(|| Field::measure(lines, (w, h), (sx, sy), reach + 1.5));
    let w = w as usize;
    for (i, (px, m)) in rgba
        .chunks_exact_mut(4)
        .zip(marked.chunks_exact(4))
        .enumerate()
    {
        if m[3] == 0 {
            continue;
        }
        let (x, y) = (i % w, i / w);
        let (colour, alpha) = match &field {
            Some(field) => ramp.at_place(field.place(x, y)),
            None => ramp.at(x as f64 + 0.5, y as f64 + 0.5),
        };
        over(px, colour, f32::from(m[3]) / 255.0 * alpha);
    }
}

/// How far along its line each pixel near an outline is, 0..1, over the box
/// the lines and their reach cover.
struct Field {
    x0: usize,
    y0: usize,
    w: usize,
    h: usize,
    /// The squared distance to the nearest point found so far, and its place.
    nearest: Vec<(f32, f32)>,
}

impl Field {
    /// Walk every line, each piece of it visiting the pixels within `reach`
    /// of it and leaving its place wherever it is the nearest yet.
    fn measure(
        lines: &[Vec<(f64, f64)>],
        (w, h): (u32, u32),
        (sx, sy): (f64, f64),
        reach: f64,
    ) -> Self {
        let lines: Vec<Vec<(f64, f64)>> = lines
            .iter()
            .map(|l| l.iter().map(|&(x, y)| (x * sx, y * sy)).collect())
            .collect();
        let (mut lo, mut hi) = ((f64::MAX, f64::MAX), (f64::MIN, f64::MIN));
        for &(x, y) in lines.iter().flatten() {
            lo = (lo.0.min(x), lo.1.min(y));
            hi = (hi.0.max(x), hi.1.max(y));
        }
        let clamp = |v: f64, n: u32| v.clamp(0.0, f64::from(n)) as usize;
        let (x0, y0) = (clamp(lo.0 - reach, w), clamp(lo.1 - reach, h));
        let (x1, y1) = (clamp(hi.0 + reach + 1.0, w), clamp(hi.1 + reach + 1.0, h));
        let mut field = Self {
            x0,
            y0,
            w: x1.saturating_sub(x0),
            h: y1.saturating_sub(y0),
            nearest: Vec::new(),
        };
        field.nearest = vec![(f32::MAX, 0.0); field.w * field.h];
        // A long piece is walked in short ones, so the box each visits stays
        // close to the line rather than a square on its diagonal.
        let step = (reach * 2.0).max(8.0);
        for line in &lines {
            let length: f64 = line.windows(2).map(|p| dist(p[0], p[1])).sum();
            let mut along = 0.0;
            for pair in line.windows(2) {
                let (a, b) = (pair[0], pair[1]);
                let d = dist(a, b);
                let pieces = (d / step).ceil().clamp(1.0, 4096.0) as usize;
                for k in 0..pieces {
                    let (f0, f1) = (k as f64 / pieces as f64, (k + 1) as f64 / pieces as f64);
                    let p = (a.0 + (b.0 - a.0) * f0, a.1 + (b.1 - a.1) * f0);
                    let q = (a.0 + (b.0 - a.0) * f1, a.1 + (b.1 - a.1) * f1);
                    let ends = (along + d * f0, along + d * f1);
                    field.visit(p, q, ends, length, reach);
                }
                along += d;
            }
        }
        field
    }

    /// One straight piece from `p` to `q`, `ends` along a line `length`
    /// long.
    fn visit(&mut self, p: (f64, f64), q: (f64, f64), ends: (f64, f64), length: f64, reach: f64) {
        let local = |v: f64, from: usize, n: usize| (v - from as f64).clamp(0.0, n as f64) as usize;
        let (bx0, bx1) = (
            local(p.0.min(q.0) - reach, self.x0, self.w),
            local(p.0.max(q.0) + reach + 1.0, self.x0, self.w),
        );
        let (by0, by1) = (
            local(p.1.min(q.1) - reach, self.y0, self.h),
            local(p.1.max(q.1) + reach + 1.0, self.y0, self.h),
        );
        let axis = (q.0 - p.0, q.1 - p.1);
        let len2 = axis.0 * axis.0 + axis.1 * axis.1;
        for y in by0..by1 {
            let cy = (self.y0 + y) as f64 + 0.5;
            for x in bx0..bx1 {
                let cx = (self.x0 + x) as f64 + 0.5;
                let (dx, dy) = (cx - p.0, cy - p.1);
                let f = if len2 > 0.0 {
                    ((dx * axis.0 + dy * axis.1) / len2).clamp(0.0, 1.0)
                } else {
                    0.0
                };
                let (ex, ey) = (dx - axis.0 * f, dy - axis.1 * f);
                let d2 = (ex * ex + ey * ey) as f32;
                let slot = &mut self.nearest[y * self.w + x];
                if d2 < slot.0 {
                    let place = if length > 0.0 {
                        (ends.0 + (ends.1 - ends.0) * f) / length
                    } else {
                        0.0
                    };
                    *slot = (d2, place as f32);
                }
            }
        }
    }

    /// The place of the pixel at `(x, y)` in the raster: 0 where no line
    /// came near it.
    fn place(&self, x: usize, y: usize) -> f64 {
        let (Some(x), Some(y)) = (x.checked_sub(self.x0), y.checked_sub(self.y0)) else {
            return 0.0;
        };
        if x >= self.w || y >= self.h {
            return 0.0;
        }
        self.nearest
            .get(y * self.w + x)
            .map_or(0.0, |s| f64::from(s.1))
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use serde_json::json;

    /// Along the line runs from the stroke colour where the line starts to
    /// the end colour where it ends, and a trim measures the part showing.
    #[test]
    fn along_the_line_follows_the_part_that_shows() {
        let line = |x: f64| json!({"pos": [x, 10.0], "tan_in": [0.0, 0.0], "tan_out": [0.0, 0.0]});
        let art = |end: f64| {
            let contents = json!([
                {"id": uuid::Uuid::now_v7(), "name": "Line", "fill": null,
                 "path": {"vertices": [line(10.0), line(190.0)], "closed": false},
                 "stroke": [1.0, 0.0, 0.0, 1.0], "stroke_width": 10.0, "opacity": 100.0,
                 "stroke_gradient": {"kind": super::ALONG, "colour": [0.0, 0.0, 1.0, 1.0]}},
                {"id": uuid::Uuid::now_v7(), "name": "Trim", "modifier": "trim",
                 "start": 0.0, "end": end, "offset": 0.0}
            ]);
            let contents = super::super::modifier::read_entries(contents).unwrap();
            super::super::rasterise_contents(&contents, 200, 20, 0.0, 0.0, 200.0, 20.0, 0.0)
        };
        let rgb = |rgba: &[u8], x: usize| {
            let i = (10 * 200 + x) * 4;
            [rgba[i], rgba[i + 1], rgba[i + 2]]
        };
        let whole = art(100.0);
        let (start, end) = (rgb(&whole, 12), rgb(&whole, 188));
        assert!(
            start[0] > 200 && start[2] < 60,
            "the stroke colour at the start: {start:?}"
        );
        assert!(
            end[2] > 200 && end[0] < 60,
            "the end colour at the end: {end:?}"
        );
        // Trimmed to half, the end colour moves to where the line now ends.
        let half = art(50.0);
        let end = rgb(&half, 98);
        assert!(
            end[2] > 200 && end[0] < 60,
            "the end colour at the trimmed end: {end:?}"
        );
    }
}

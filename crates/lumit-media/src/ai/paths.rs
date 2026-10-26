//! A layer's artwork as paths rather than pixels, for making shape layers
//! from it.
//!
//! The page is walked by the same PDF reader that draws it, with a device
//! that keeps each path it is asked to paint instead of painting it. Clipping
//! masks, placed pictures and live text are left out.

use hayro::hayro_interpret::font::GlyphRun;
use hayro::hayro_interpret::util::TransformExt;
use hayro::hayro_interpret::{
    interpret_page, BlendMode, ClipPath, Context, Device, DrawMode, DrawProps, Image,
    ImageDrawProps, InterpreterCache, InterpreterSettings, Paint, SoftMask,
};
use hayro::kurbo::{Affine, BezPath, PathEl, Point, Rect};

use super::{bad, picked};
use crate::MediaError;

/// The most points a layer's paths may hold. A detailed logo is a few
/// thousand.
pub const MAX_POINTS: usize = 1 << 20;

/// One painted path: its runs, and how it is painted.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct AiPath {
    pub contours: Vec<AiContour>,
    /// sRGB and alpha, from 0 to 255. `None` paints nothing.
    pub fill: Option<[u8; 4]>,
    pub stroke: Option<[u8; 4]>,
    /// In pixels.
    pub stroke_width: f64,
}

/// One run of a path, in the artboard's pixels from its top left. Each point
/// is its position, then its handles in and out, measured from it.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct AiContour {
    pub closed: bool,
    pub points: Vec<[f64; 6]>,
}

pub(super) fn paths(bytes: Vec<u8>, index: Option<u32>) -> Result<Vec<AiPath>, MediaError> {
    let (pdf, doc) = picked(bytes, index)?;
    let page = pdf
        .pages()
        .iter()
        .next()
        .ok_or_else(|| bad("no artboard"))?;
    // The same fit the picture is drawn with, so the paths land on it.
    let (wide, high) = page.render_dimensions();
    let fit = Affine::scale_non_uniform(
        f64::from(doc.width) / f64::from(wide).max(1.0),
        f64::from(doc.height) / f64::from(high).max(1.0),
    );
    let cache = InterpreterCache::new();
    let mut context = Context::new(
        fit * page.initial_transform(true).to_kurbo(),
        Rect::new(0.0, 0.0, f64::from(doc.width), f64::from(doc.height)),
        &cache,
        page.xref(),
        InterpreterSettings::default(),
    );
    let mut kept = Kept::default();
    interpret_page(page, &mut context, &mut kept);
    if kept.points > MAX_POINTS {
        return Err(bad("the layer holds too many points"));
    }
    Ok(kept.paths)
}

#[derive(Default)]
struct Kept {
    paths: Vec<AiPath>,
    /// The last path kept, as it was placed.
    last: Option<BezPath>,
    points: usize,
}

impl<'a> Device<'a> for Kept {
    fn draw_path(&mut self, path: &BezPath, props: DrawProps<'a>, mode: &DrawMode) {
        if self.points > MAX_POINTS {
            return;
        }
        let placed = props.transform * path.clone();
        let colour = match &props.paint {
            Paint::Color(colour) => colour.to_rgba().to_rgba8(),
            // ponytail: a gradient or pattern comes across as flat grey. The
            // upgrade is the shading's own stops as a gradient fill.
            Paint::Pattern(_) => [128, 128, 128, 255],
        };
        let width = |w: f32| f64::from(w) * props.transform.determinant().abs().sqrt();
        // A path both filled and outlined arrives twice, the fill first, so
        // the outline joins the fill it follows.
        if let DrawMode::Stroke(stroke) = mode {
            let last = self.paths.last_mut().filter(|p| p.stroke.is_none());
            if let (Some(last), true) = (last, self.last.as_ref() == Some(&placed)) {
                last.stroke = Some(colour);
                last.stroke_width = width(stroke.line_width);
                return;
            }
        }
        let (fill, stroke, stroke_width) = match mode {
            DrawMode::Fill(_) => (Some(colour), None, 0.0),
            DrawMode::Stroke(s) => (None, Some(colour), width(s.line_width)),
            DrawMode::FillAndStroke(_, s) => (Some(colour), Some(colour), width(s.line_width)),
            DrawMode::Invisible => return,
        };
        let contours = contours(&placed, fill.is_some());
        self.points += contours.iter().map(|c| c.points.len()).sum::<usize>();
        self.paths.push(AiPath {
            contours,
            fill,
            stroke,
            stroke_width,
        });
        self.last = Some(placed);
    }
    fn push_clip_path(&mut self, _: &ClipPath) {}
    fn push_transparency_group(&mut self, _: f32, _: Option<SoftMask<'a>>, _: BlendMode) {}
    fn draw_glyph_run(&mut self, _: &GlyphRun<'_, 'a>, _: DrawProps<'a>, _: &DrawMode) {}
    fn draw_image(&mut self, _: Image<'a, '_>, _: ImageDrawProps<'a>) {}
    fn pop_clip(&mut self) {}
    fn pop_transparency_group(&mut self) {}
}

/// `path`'s runs as points with handles. A filled run is closed whether or
/// not it says so, as a fill closes it.
fn contours(path: &BezPath, filled: bool) -> Vec<AiContour> {
    let mut out = Vec::new();
    let mut points: Vec<[f64; 6]> = Vec::new();
    let (mut start, mut last) = (Point::ZERO, Point::ZERO);
    let curve = |points: &mut Vec<[f64; 6]>, from: Point, c1: Point, c2: Point, to: Point| {
        if let Some(previous) = points.last_mut() {
            previous[4] = c1.x - from.x;
            previous[5] = c1.y - from.y;
        }
        points.push([to.x, to.y, c2.x - to.x, c2.y - to.y, 0.0, 0.0]);
    };
    for element in path.elements() {
        match *element {
            PathEl::MoveTo(p) => {
                end(&mut points, filled, &mut out);
                points.push([p.x, p.y, 0.0, 0.0, 0.0, 0.0]);
                (start, last) = (p, p);
            }
            PathEl::LineTo(p) => {
                points.push([p.x, p.y, 0.0, 0.0, 0.0, 0.0]);
                last = p;
            }
            // A quadratic is the cubic with its handles two thirds of the way
            // to the one control point.
            PathEl::QuadTo(c, p) => {
                let (c1, c2) = (last + (c - last) * (2.0 / 3.0), p + (c - p) * (2.0 / 3.0));
                curve(&mut points, last, c1, c2, p);
                last = p;
            }
            PathEl::CurveTo(c1, c2, p) => {
                curve(&mut points, last, c1, c2, p);
                last = p;
            }
            PathEl::ClosePath => {
                end(&mut points, true, &mut out);
                last = start;
            }
        }
    }
    end(&mut points, filled, &mut out);
    out
}

/// Finish the run in `points`. A closed run that ends back on its first point
/// loses that last point, and its handle in moves onto the first.
fn end(points: &mut Vec<[f64; 6]>, closed: bool, out: &mut Vec<AiContour>) {
    let mut points = std::mem::take(points);
    if closed && points.len() >= 2 {
        let (first, last) = (points[0], points[points.len() - 1]);
        if (first[0] - last[0]).abs() < 1e-6 && (first[1] - last[1]).abs() < 1e-6 {
            points.pop();
            points[0][2] = last[2];
            points[0][3] = last[3];
        }
    }
    if points.len() >= 2 {
        out.push(AiContour { closed, points });
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::super::fixture::{document, Layer};
    use super::*;

    /// One layer's square comes back as one closed path where it sat, in its
    /// own colour, and the other layer's does not come with it.
    #[test]
    fn a_layer_reads_as_its_own_paths() {
        let layers = [
            Layer::solid("Back", [0, 0, 6, 8], [255, 0, 0]),
            Layer::solid("Square", [1, 2, 3, 6], [0, 0, 255]),
        ];
        let paths = paths(document(8, 6, &layers), Some(1)).unwrap();
        assert_eq!(paths.len(), 1);
        assert_eq!(paths[0].fill, Some([0, 0, 255, 255]));
        let run = &paths[0].contours[0];
        assert!(run.closed);
        let mut corners: Vec<(i64, i64)> = run
            .points
            .iter()
            .map(|p| (p[0].round() as i64, p[1].round() as i64))
            .collect();
        corners.sort_unstable();
        assert_eq!(corners, [(2, 1), (2, 3), (6, 1), (6, 3)]);
    }
}

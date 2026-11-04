//! A Shape layer's art as the outlines Geometry options make solid: what the
//! picture would paint, with the same combines, trims and repeater copies, in
//! the pixels of the layer's own picture.

use i_overlay::mesh::stroke::offset::StrokeOffset;
use i_overlay::mesh::style::StrokeStyle;

use super::raster::{runs, Art};
use super::{colour, contents_bounds, modifier, Affine, ShapeEntry};

/// One run of a shape's art at one of its copies.
#[derive(Debug, Clone, PartialEq)]
pub struct Solid {
    pub contours: Vec<Vec<[f64; 2]>>,
    /// Linear, its fill or, with none, its outline.
    pub fill: [f32; 4],
    /// Linear, its outline where it has one as well as a fill.
    pub outline: Option<[f32; 4]>,
}

/// The shape a brush `width` wide paints along `line`.
#[must_use]
pub fn outline_of(line: &[[f64; 2]], width: f64) -> Vec<Vec<[f64; 2]>> {
    let round = line.len() > 2 && line.first() == line.last();
    let style = StrokeStyle::new(width.max(0.0));
    let shapes = line.stroke(style, round);
    shapes.into_iter().flatten().collect()
}

/// `contours` with the part of a brush `width` wide that falls outside them, so
/// a stroked fill keeps the silhouette it paints. Cleaned first, so each ring
/// turns the way a brush's own shape does and the two add up rather than cancel.
#[must_use]
pub fn with_stroke(contours: &[Vec<[f64; 2]>], width: f64) -> Vec<Vec<[f64; 2]>> {
    use i_overlay::core::fill_rule::FillRule;
    use i_overlay::core::overlay_rule::OverlayRule;
    use i_overlay::float::single::SingleFloatOverlay;
    if width <= 0.0 {
        return contours.to_vec();
    }
    let none: Vec<Vec<[f64; 2]>> = Vec::new();
    let clean = contours
        .to_vec()
        .overlay(&none, OverlayRule::Subject, FillRule::NonZero);
    let mut out: Vec<Vec<[f64; 2]>> = clean.into_iter().flatten().collect();
    for c in contours {
        let mut ring = c.clone();
        if let (Some(first), true) = (c.first().copied(), c.first() != c.last()) {
            ring.push(first);
        }
        out.extend(outline_of(&ring, width));
    }
    out
}

/// The outlines of `contents` at `t`, in the order they're painted. A run with a fill gives its fill's
/// outline, and one with only an outline gives the shape that outline paints.
/// A run with neither paints nothing, so it gives nothing.
#[must_use]
pub fn solids(contents: &[ShapeEntry], t: f64) -> Vec<Solid> {
    let mut out = Vec::new();
    // The picture's corner is the corner of the art's box.
    let Some((x0, y0, ..)) = contents_bounds(contents, t) else {
        return out;
    };
    let to_box = Affine::translation(-x0, -y0);
    let items = modifier::lowered(contents);
    for run in runs(&items) {
        let Some(item) = items.get(run.start).map(|art| art.item) else {
            continue;
        };
        let opacity = (item.opacity.value_at(t).clamp(0.0, 100.0) / 100.0) as f32;
        let Some(art) = Art::at(&items, run, t) else {
            continue;
        };
        let width = item.stroke_width.value_at(t);
        let stroke = item.stroke.as_ref().map(|c| colour::at(c, t).0);
        let stroke = stroke.filter(|_| width > 0.0);
        let fill = item.fill.as_ref().map(|c| colour::at(c, t).0);
        // ponytail: a gradient fill takes its flat colour, as a vertex colour
        // has no ramp. Read the ramp at each point if a logo needs it.
        let Some(paint) = fill.or(stroke) else {
            continue;
        };
        // Last copy first, so the original paints on top of what it made.
        for (copy, ramp) in modifier::copies_at(art.mods, t).into_iter().rev() {
            let share = opacity * ramp as f32;
            if share <= 0.0 {
                continue;
            }
            let place = copy.then(&to_box);
            let placed = art.placed(&place);
            let contours: Vec<Vec<[f64; 2]>> = match fill {
                Some(_) => {
                    let flat: Vec<_> = placed.iter().map(crate::extrude::flattened).collect();
                    with_stroke(&flat, stroke.map_or(0.0, |_| width * copy.scale()))
                }
                None => art
                    .stroked(&placed, copy.scale(), t)
                    .into_iter()
                    .flat_map(|(line, _)| {
                        let points: Vec<[f64; 2]> = line.iter().map(|&(x, y)| [x, y]).collect();
                        outline_of(&points, width * copy.scale())
                    })
                    .collect(),
            };
            let mut fill = paint;
            fill[3] *= share;
            out.push(Solid {
                contours,
                fill,
                outline: stroke.filter(|_| item.fill.is_some()),
            });
        }
    }
    out
}

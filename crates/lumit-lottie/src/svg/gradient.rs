//! A gradient fill: the art's own pixels, with stops in between Lumit's own
//! so the browser's mix in display colour follows Lumit's in linear light.
//!
//! Runs on the export's thread.

use std::fmt::Write;

use lumit_core::anim::Property;
use lumit_core::model::Layer;
use lumit_core::shape::{GradientStop, ShapeItem, MAX_GRADIENT_STOPS};

use super::anim::{self, Value};
use super::shapes::{pick, still};
use super::{Scope, Svg};
use crate::fit::tolerance;

impl Svg<'_> {
    /// A gradient fill in the art's own pixels, and its id.
    pub(super) fn gradient(
        &mut self,
        scope: &Scope<'_>,
        layer: &Layer,
        item: &ShapeItem,
    ) -> String {
        let id = self.id("g");
        let radial = item.gradient == 2;
        let geometry = self.value(
            scope,
            layer,
            &[
                &item.gradient_start_x,
                &item.gradient_start_y,
                &item.gradient_end_x,
                &item.gradient_end_y,
            ],
            &[tolerance::PIXELS; 4],
        );
        let mut tag = String::new();
        let mut animations = String::new();
        if radial {
            let _ = write!(
                tag,
                "<radialGradient id=\"{id}\" gradientUnits=\"userSpaceOnUse\""
            );
            let xy = pick(&geometry, 0..2);
            let r = match &geometry {
                Value::Still(v) => Value::Still(vec![radius(v)]),
                moving => {
                    let samples = self
                        .span
                        .grid
                        .iter()
                        .map(|t| vec![radius(&moving.at(*t))])
                        .collect();
                    Value::sampled(&self.span.grid, samples, &[tolerance::PIXELS])
                }
            };
            for (attr, value, d) in [("cx", &xy, 0usize), ("cy", &xy, 1), ("r", &r, 0)] {
                let f = |v: &[f64]| self.px(v.get(d).copied().unwrap_or(0.0));
                self.attr(&mut tag, attr, value, &f);
                let n = if attr == "r" { 1 } else { 2 };
                self.animate(
                    &mut animations,
                    attr,
                    value,
                    &vec![tolerance::PIXELS; n],
                    &f,
                );
            }
        } else {
            let _ = write!(
                tag,
                "<linearGradient id=\"{id}\" gradientUnits=\"userSpaceOnUse\""
            );
            for (attr, d) in [("x1", 0usize), ("y1", 1), ("x2", 2), ("y2", 3)] {
                let f = |v: &[f64]| self.px(v.get(d).copied().unwrap_or(0.0));
                self.attr(&mut tag, attr, &geometry, &f);
                self.animate(
                    &mut animations,
                    attr,
                    &geometry,
                    &[tolerance::PIXELS; 4],
                    &f,
                );
            }
        }
        tag.push('>');
        tag.push_str(&animations);

        // The stops: every one of them, at every frame when any moves, so
        // stops that pass each other stay in order.
        let moves = item
            .fill_shown()
            .is_some_and(|c| c.iter().any(|p| !still(p)))
            || !still(&item.gradient_start_opacity)
            || !still(&item.gradient_end_opacity)
            || item
                .gradient_stops
                .iter()
                .any(|s| !still(&s.position) || !still(&s.opacity));
        let clock = Svg::layer_clock(scope, layer);
        let display = |c: [f64; 3]| self.host.display(c);
        if !moves {
            let points = points(item, 0.0);
            let splits = splits(std::slice::from_ref(&points), &display);
            for s in stops(&points, &splits, &display) {
                write_stop(self, &mut tag, &s, "");
            }
        } else {
            // One set of stops for every frame, placed for all of them.
            let moments: Vec<Vec<Point>> = self
                .span
                .grid
                .iter()
                .enumerate()
                .map(|(i, t)| points(item, clock.at(i, *t)))
                .collect();
            let splits = splits(&moments, &display);
            let frames: Vec<Vec<[f64; 5]>> = moments
                .iter()
                .map(|p| stops(p, &splits, &display))
                .collect();
            let count = frames.iter().map(Vec::len).min().unwrap_or(0);
            for k in 0..count {
                let samples: Vec<Vec<f64>> = frames
                    .iter()
                    .map(|f| f.get(k).map(|s| s.to_vec()).unwrap_or_default())
                    .collect();
                let value = Value::sampled(
                    &self.span.grid,
                    samples,
                    &[
                        1e-4,
                        tolerance::CHANNEL,
                        tolerance::CHANNEL,
                        tolerance::CHANNEL,
                        1e-4,
                    ],
                );
                let mut animations = String::new();
                let tol = [
                    1e-4,
                    tolerance::CHANNEL,
                    tolerance::CHANNEL,
                    tolerance::CHANNEL,
                    1e-4,
                ];
                let offset = |v: &[f64]| anim::num(v.first().copied().unwrap_or(0.0), 4);
                let colour = |v: &[f64]| {
                    crate::colour::hex([
                        v.get(1).copied().unwrap_or(0.0),
                        v.get(2).copied().unwrap_or(0.0),
                        v.get(3).copied().unwrap_or(0.0),
                    ])
                };
                let alpha = |v: &[f64]| anim::num(v.get(4).copied().unwrap_or(1.0), 4);
                self.animate(&mut animations, "offset", &value, &tol, &offset);
                self.animate(&mut animations, "stop-color", &value, &tol, &colour);
                self.animate(&mut animations, "stop-opacity", &value, &tol, &alpha);
                let first: [f64; 5] = frames
                    .first()
                    .and_then(|f| f.get(k))
                    .copied()
                    .unwrap_or([0.0, 0.0, 0.0, 0.0, 1.0]);
                write_stop(self, &mut tag, &first, &animations);
            }
        }
        tag.push_str(if radial {
            "</radialGradient>"
        } else {
            "</linearGradient>"
        });
        self.defs.push_str(&tag);
        id
    }
}

/// A gradient stop: `[offset, r, g, b, alpha]`, with its animations inside.
fn write_stop(svg: &Svg<'_>, out: &mut String, s: &[f64; 5], animations: &str) {
    let _ = write!(
        out,
        "<stop offset=\"{}\" stop-color=\"{}\"",
        anim::num(s[0], 4),
        crate::colour::hex([s[1], s[2], s[3]])
    );
    if s[4] < 1.0 || !animations.is_empty() {
        let _ = write!(out, " stop-opacity=\"{}\"", svg.unit(s[4].clamp(0.0, 1.0)));
    }
    if animations.is_empty() {
        out.push_str("/>");
    } else {
        out.push('>');
        out.push_str(animations);
        out.push_str("</stop>");
    }
}

/// One of a gradient's own stops: where it is, 0 to 1, and its linear colour
/// and alpha.
type Point = (f64, [f64; 4]);

/// Where stops go between each two of a gradient's own, placed for every
/// moment in `moments`.
fn splits(moments: &[Vec<Point>], display: &dyn Fn([f64; 3]) -> [f64; 3]) -> Vec<Vec<f64>> {
    let colours: Vec<Vec<[f64; 3]>> = moments
        .iter()
        .map(|m| m.iter().map(|(_, c)| [c[0], c[1], c[2]]).collect())
        .collect();
    crate::colour::splits(&colours, display)
}

/// `points` as `[offset, r, g, b, alpha]` in display colour, with stops
/// mixed in linear light at `splits` between each two, as Lumit's ramp
/// mixes them.
fn stops(
    points: &[Point],
    splits: &[Vec<f64>],
    display: &dyn Fn([f64; 3]) -> [f64; 3],
) -> Vec<[f64; 5]> {
    let shown = |at: f64, c: [f64; 4]| -> [f64; 5] {
        let [r, g, b] = display([c[0], c[1], c[2]]);
        [at, r, g, b, c[3].clamp(0.0, 1.0)]
    };
    let mut out = Vec::new();
    for (i, pair) in points.windows(2).enumerate() {
        let [(a_at, a), (b_at, b)] = pair else {
            continue;
        };
        if i == 0 {
            out.push(shown(*a_at, *a));
        }
        for u in splits.get(i).into_iter().flatten() {
            let mix: [f64; 4] = std::array::from_fn(|j| a[j] + (b[j] - a[j]) * u);
            out.push(shown(a_at + (b_at - a_at) * u, mix));
        }
        out.push(shown(*b_at, *b));
    }
    out
}

/// The stops of `item`'s gradient at `lt`, in linear light.
fn points(item: &ShapeItem, lt: f64) -> Vec<Point> {
    let faded = |p: &Property| p.value_at(lt).clamp(0.0, 100.0) / 100.0;
    let from = item
        .fill_shown()
        .map(|c| lumit_core::shape::colour::at(c, lt))
        .unwrap_or(lumit_core::model::LinearColour::BLACK)
        .0
        .map(f64::from);
    let to = item
        .gradient_colour
        .unwrap_or(lumit_core::model::LinearColour::BLACK)
        .0
        .map(f64::from);
    let mut between: Vec<(f64, &GradientStop)> = item
        .gradient_stops
        .iter()
        .take(MAX_GRADIENT_STOPS)
        .map(|s| (s.position.value_at(lt), s))
        .filter(|(at, _)| at.is_finite())
        .collect();
    between.sort_by(|a, b| a.0.total_cmp(&b.0));
    let mut points: Vec<(f64, [f64; 4])> = Vec::with_capacity(between.len() + 2);
    points.push((
        0.0,
        [
            from[0],
            from[1],
            from[2],
            from[3] * faded(&item.gradient_start_opacity),
        ],
    ));
    for (at, stop) in between {
        let c = stop.colour.0.map(f64::from);
        points.push((
            (at.clamp(0.0, 100.0)) / 100.0,
            [c[0], c[1], c[2], c[3] * faded(&stop.opacity)],
        ));
    }
    points.push((
        1.0,
        [
            to[0],
            to[1],
            to[2],
            to[3] * faded(&item.gradient_end_opacity),
        ],
    ));
    points
}

/// A radial gradient's radius from its start and end points.
fn radius(v: &[f64]) -> f64 {
    let g = |i: usize| v.get(i).copied().unwrap_or(0.0);
    (g(2) - g(0)).hypot(g(3) - g(1))
}

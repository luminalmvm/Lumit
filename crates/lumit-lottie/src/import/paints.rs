//! A Lottie's fills, strokes and gradients as Lumit's shape items.
//!
//! # In plain terms
//!
//! A paint in a Lottie paints every path before it in its group. Here each
//! path it reaches becomes an item with that paint. A fill over several paths
//! is one shape with holes, so its items are a combine run (see
//! [`super::shapes`]). Colours arrive in display sRGB and are kept in Lumit's
//! linear light.
//!
//! A gradient's stops become Lumit's ramp: the colour at its start, the colour
//! at its end, and a stop wherever a colour or an opacity stop sits between.
//! A gradient on an outline has no equal, so the outline takes the gradient's
//! first colour and the report says so.
//!
//! Runs on the import's thread.

use lumit_core::anim::Property;
use lumit_core::model::LinearColour;
use lumit_core::shape::geometry::Affine;
use lumit_core::shape::{GradientStop, ShapeItem, MAX_GRADIENT_STOPS};
use lumit_import::report::{ItemPath, Outcome, Reason};
use serde_json::Value;

use super::layers::linear;
use super::read::{self, num, Read};
use super::shapes::{affine, Art, Drawn};
use super::{keys, Cx};
use crate::colour::srgb_decode;
use crate::LottieError;

/// The items one paint makes from the paths before it, bottom first.
pub(super) fn paint(
    cx: &mut Cx<'_>,
    here: &ItemPath,
    name: &str,
    node: &Value,
    ty: &str,
    drawn: &[Drawn],
    chain: &Affine,
) -> Result<Vec<Art>, LottieError> {
    if drawn.is_empty() {
        return Ok(Vec::new());
    }
    cx.budget.take_items(drawn.len() as u64)?;
    let stroke = matches!(ty, "st" | "gs");
    let clock = cx.clock;
    let mut base = ShapeItem::filled(
        name,
        drawn.first().map(|d| d.path.clone()).unwrap_or_else(empty),
        LinearColour::BLACK,
    );
    base.opacity = cx.one(here, "Opacity", node.get("o"), 100.0)?;

    if ty == "fl" || ty == "st" {
        let old = cx.version.before([4, 1, 9]);
        let decode = move |v: f64| srgb_decode(if old { v / 255.0 } else { v });
        let colour = cx.read_mapped(here, "Colour", node.get("c"), &decode)?;
        base.fill = Some(channels(colour.as_ref(), &clock));
    } else {
        gradient(cx, here, node, chain, stroke, &mut base)?;
    }
    if stroke {
        base.stroke = base.fill.take();
        let scale = chain.scale();
        base.stroke_width = cx.one(here, "Stroke width", node.get("w"), 2.0)?;
        affine(&mut base.stroke_width, scale, 0.0);
        for dash in read::list(node, "d") {
            let mut length = cx.one(here, "Dashes", dash.get("v"), 0.0)?;
            affine(&mut length, scale, 0.0);
            match read::text(dash, "n") {
                Some("o") => base.dash_offset = length,
                _ => base.dashes.push(length),
            }
        }
    }

    let make = |d: &Drawn| {
        let mut item = base.clone();
        item.id = uuid::Uuid::now_v7();
        item.name = d.name.clone();
        item.path = d.path.clone();
        item.path_keys = d.keys.clone();
        item.primitive = d.primitive.clone();
        Art {
            item,
            mods: d.mods.clone(),
        }
    };
    if stroke {
        // Each path is outlined on its own; the first listed draws on top.
        return Ok(drawn.iter().rev().map(make).collect());
    }
    // One shape with holes: a combine run. Unless a merge paths set the
    // order, the largest path goes first, so a hole listed before its outline
    // is cut from it rather than the other way round.
    let even_odd = num(node.get("r")) == Some(2.0);
    let mut order: Vec<&Drawn> = drawn.iter().collect();
    if drawn.iter().all(|d| d.merge.is_none()) {
        order.sort_by(|a, b| winding(&b.path).abs().total_cmp(&winding(&a.path).abs()));
    }
    let first = order.first().map_or(0.0, |d| winding(&d.path));
    Ok(order
        .into_iter()
        .enumerate()
        .map(|(n, d)| {
            let mut art = make(d);
            art.item.combine = match (n, d.merge) {
                (0, _) => 0,
                (_, Some(code)) => code,
                _ if even_odd => 4,
                _ if winding(&d.path) * first >= 0.0 => 1,
                _ => 2,
            };
            art
        })
        .collect())
}

fn empty() -> lumit_core::mask::BezierPath {
    lumit_core::mask::BezierPath {
        vertices: Vec::new(),
        closed: true,
    }
}

/// A read colour as four channels, opaque: players draw a colour's own alpha
/// nowhere.
fn channels(read: Option<&Read>, clock: &read::Clock) -> [Property; 4] {
    [
        keys::one(read, 0, clock, 1.0),
        keys::one(read, 1, clock, 1.0),
        keys::one(read, 2, clock, 1.0),
        Property::fixed(1.0),
    ]
}

/// The area a path's points and handles enclose, signed by which way it
/// winds.
fn winding(path: &lumit_core::mask::BezierPath) -> f64 {
    let n = path.vertices.len();
    let mut points = Vec::with_capacity(n * 3);
    for (i, v) in path.vertices.iter().enumerate() {
        points.push(v.pos);
        if let Some(next) = path.vertices.get((i + 1) % n.max(1)) {
            points.push((v.pos.0 + v.tan_out.0, v.pos.1 + v.tan_out.1));
            points.push((next.pos.0 + next.tan_in.0, next.pos.1 + next.tan_in.1));
        }
    }
    let m = points.len();
    (0..m)
        .filter_map(|i| {
            let (a, b) = (points.get(i)?, points.get((i + 1) % m)?);
            Some(a.0 * b.1 - b.0 * a.1)
        })
        .sum()
}

/// A gradient fill's ramp, or a gradient stroke's first colour.
fn gradient(
    cx: &mut Cx<'_>,
    here: &ItemPath,
    node: &Value,
    chain: &Affine,
    stroke: bool,
    item: &mut ShapeItem,
) -> Result<(), LottieError> {
    let g = node.get("g");
    let count = num(g.and_then(|g| g.get("p"))).unwrap_or(0.0).max(0.0) as usize;
    let stops = cx.read(here, "Colours", g.and_then(|g| g.get("k")))?;
    if stops.as_ref().is_some_and(Read::is_animated) {
        cx.row(here.clone(), Outcome::Adjusted, Reason::ShapePaintKeptStill);
    }
    let ramp = Ramp::of(&stops.map(|s| s.first()).unwrap_or_default(), count);
    item.fill = Some(lumit_core::shape::colour::still(linear(
        ramp.colour_at(0.0),
    )));
    if stroke {
        cx.approximated(here.clone(), "Gradient stroke");
        return Ok(());
    }
    item.gradient = if num(node.get("t")) == Some(2.0) {
        2
    } else {
        1
    };
    if item.gradient == 2 {
        let highlight = cx.read(here, "Highlight length", node.get("h"))?;
        if highlight
            .is_some_and(|h| h.is_animated() || h.first().first().is_some_and(|v| v.abs() > 1e-9))
        {
            cx.approximated(here.property("Highlight"), "Highlight");
        }
    }
    item.gradient_colour = Some(linear(ramp.colour_at(1.0)));
    item.gradient_start_opacity = Property::fixed(ramp.alpha_at(0.0) * 100.0);
    item.gradient_end_opacity = Property::fixed(ramp.alpha_at(1.0) * 100.0);
    let mut inside = ramp.offsets();
    if inside.len() > MAX_GRADIENT_STOPS {
        cx.approximated(here.clone(), "Gradient stops");
        inside.truncate(MAX_GRADIENT_STOPS);
    }
    item.gradient_stops = inside
        .into_iter()
        .map(|at| GradientStop {
            position: Property::fixed(at * 100.0),
            colour: linear(ramp.colour_at(at)),
            opacity: Property::fixed(ramp.alpha_at(at) * 100.0),
        })
        .collect();
    let start = cx.read(here, "Start point", node.get("s"))?;
    let end = cx.read(here, "End point", node.get("e"))?;
    [item.gradient_start_x, item.gradient_start_y] = point(start.as_ref(), chain, &cx.clock);
    [item.gradient_end_x, item.gradient_end_y] = point(end.as_ref(), chain, &cx.clock);
    Ok(())
}

/// A point property carried through a still transform. Each key is carried
/// whole, so the easing stays exact wherever both numbers ease alike.
fn point(read: Option<&Read>, chain: &Affine, clock: &read::Clock) -> [Property; 2] {
    let Some(read) = read else {
        return [Property::zero(), Property::zero()];
    };
    let mut track = if read.spatial.is_empty() {
        read.track.clone()
    } else {
        keys::along_path(read, clock)
    };
    for key in &mut track.keys {
        let p = (
            key.value.first().copied().unwrap_or(0.0),
            key.value.get(1).copied().unwrap_or(0.0),
        );
        let (x, y) = chain.apply(p);
        key.value = vec![x, y];
    }
    [
        keys::property(&track, 0, clock, 0.0),
        keys::property(&track, 1, clock, 0.0),
    ]
}

/// A Lottie gradient's stops: colours (offset, red, green, blue) and
/// opacities (offset, alpha), in display numbers.
struct Ramp {
    colours: Vec<(f64, [f64; 3])>,
    alphas: Vec<(f64, f64)>,
}

impl Ramp {
    /// The stops in `flat`, `count` of them colours and the rest opacities.
    fn of(flat: &[f64], count: usize) -> Ramp {
        let split = (count * 4).min(flat.len());
        let (c, a) = flat.split_at(split);
        let mut colours: Vec<(f64, [f64; 3])> = c
            .chunks_exact(4)
            .filter_map(|s| match s {
                [o, r, g, b] => Some((*o, [*r, *g, *b])),
                _ => None,
            })
            .collect();
        let mut alphas: Vec<(f64, f64)> = a
            .chunks_exact(2)
            .filter_map(|s| match s {
                [o, v] => Some((*o, *v)),
                _ => None,
            })
            .collect();
        colours.sort_by(|x, y| x.0.total_cmp(&y.0));
        alphas.sort_by(|x, y| x.0.total_cmp(&y.0));
        if colours.is_empty() {
            colours = vec![(0.0, [1.0; 3]), (1.0, [0.0; 3])];
        }
        Ramp { colours, alphas }
    }

    fn colour_at(&self, at: f64) -> [f64; 3] {
        let mut out = [0.0; 3];
        for (c, slot) in out.iter_mut().enumerate() {
            let stops: Vec<(f64, f64)> = self.colours.iter().map(|(o, rgb)| (*o, rgb[c])).collect();
            *slot = along(&stops, at, 1.0);
        }
        out
    }

    fn alpha_at(&self, at: f64) -> f64 {
        along(&self.alphas, at, 1.0)
    }

    /// Every offset strictly inside the ramp where a stop sits.
    fn offsets(&self) -> Vec<f64> {
        let mut all: Vec<f64> = self
            .colours
            .iter()
            .map(|s| s.0)
            .chain(self.alphas.iter().map(|s| s.0))
            .filter(|o| *o > 1e-6 && *o < 1.0 - 1e-6)
            .collect();
        all.sort_by(f64::total_cmp);
        all.dedup_by(|a, b| (*a - *b).abs() < 1e-6);
        all
    }
}

/// The value at `at` along stops sorted by offset, held past either end.
fn along(stops: &[(f64, f64)], at: f64, none: f64) -> f64 {
    let (Some(first), Some(last)) = (stops.first(), stops.last()) else {
        return none;
    };
    if at <= first.0 {
        return first.1;
    }
    if at >= last.0 {
        return last.1;
    }
    for w in stops.windows(2) {
        if let [a, b] = w {
            if at <= b.0 {
                let f = if b.0 > a.0 {
                    (at - a.0) / (b.0 - a.0)
                } else {
                    0.0
                };
                return a.1 + (b.1 - a.1) * f;
            }
        }
    }
    last.1
}

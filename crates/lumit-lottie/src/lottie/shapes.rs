//! A Shape layer's contents as Lottie shape items.
//!
//! Lumit draws its contents in list order, each piece of art over the one
//! before, and a modifier reaches every piece above it, groups included. A
//! Lottie draws the first item on top and its modifiers reach only inside
//! their own group. So each piece of art (or each combined run) is written
//! as a group of its own carrying the modifiers that reach it, and the groups
//! go out last first. Nothing about the picture changes on the way.
//!
//! Inside each group: the paths, the modifiers a player knows (trim, offset),
//! the stroke, then the fill or gradient, the repeaters, and a transform with
//! the item's opacity. What a player has no modifier for (twist, deform,
//! smooth, a subtract or an intersect) is baked into the path when nothing
//! about it moves, and left out with a word in the check when it does.
//!
//! Runs on the export's thread.

use lumit_core::anim::{Animation, Property};
use lumit_core::mask::{BezierPath, Vertex};
use lumit_core::model::LinearColour;
use lumit_core::shape::modifier::{
    lowered, Lowered, ModifierKind, ShapeEntry, ShapeModifier, Trim, MAX_COPIES,
};
use lumit_core::shape::primitive::{ShapePrimitive, MAX_POINTS};
use lumit_core::shape::{contents_bounds, solid, ShapeItem};
use serde_json::{json, Map, Value};

use super::value::{Class, Clock};
use crate::check::{arg, Feature, Place, Report};
use crate::fit::tolerance;
use crate::keys::{Track, TrackKey};

/// `entries` as Lottie shape items, the top of the picture first.
pub fn contents(c: &Clock, entries: &[ShapeEntry], report: &mut Report, at: &Place) -> Vec<Value> {
    let items = lowered(entries);
    let mut runs: Vec<std::ops::Range<usize>> = Vec::new();
    for (i, art) in items.iter().enumerate() {
        match runs.last_mut() {
            Some(run) if art.joins => run.end = i + 1,
            _ => runs.push(i..i + 1),
        }
    }
    let mut out: Vec<Value> = runs
        .into_iter()
        .filter_map(|run| {
            items
                .get(run)
                .and_then(|members| group(c, members, report, at))
        })
        .collect();
    out.reverse();
    out
}

/// What a combine is called, for the check.
fn combine_label(kind: u32) -> &'static str {
    match kind {
        1 => "Union",
        2 => "Subtract",
        3 => "Intersect",
        _ => "Exclude",
    }
}

/// One run of art, drawn as one, as a Lottie group.
fn group(c: &Clock, members: &[Lowered], report: &mut Report, at: &Place) -> Option<Value> {
    let first = members.first()?;
    let item = first.item;
    let here = at.part(&item.name);
    let mods = &first.mods;

    // The modifiers a player has none of, and the trims and offsets that
    // come before them, go into the path; the rest are written as modifiers.
    let bends = |m: &ShapeModifier| {
        matches!(
            m.kind,
            ModifierKind::Twist(_) | ModifierKind::Deform(_) | ModifierKind::Smooth(_)
        )
    };
    let repeats = |m: &ShapeModifier| matches!(m.kind, ModifierKind::Repeat(_));
    let last_bend = mods.iter().rposition(|m| bends(m));
    let (mut bake_mods, mut kept): (Vec<&ShapeModifier>, Vec<&ShapeModifier>) = match last_bend {
        Some(k) => {
            let (before, after) = mods.split_at(k + 1);
            let (repeat, bake): (Vec<_>, Vec<_>) = before.iter().partition(|m| repeats(m));
            (
                bake,
                repeat.into_iter().chain(after.iter().copied()).collect(),
            )
        }
        None => (Vec::new(), mods.clone()),
    };

    let joins: Vec<u32> = members.iter().skip(1).map(|m| m.item.combine).collect();
    let stroked = item.stroke_shown().is_some();
    // Union and exclude of plain paths are a group of paths with the fill
    // rule that makes them; every other combine has no Lottie equal.
    let kind = joins.first().copied();
    let mut multi = match kind {
        None => false,
        Some(k) => joins.iter().all(|j| *j == k) && (k == 4 || (k == 1 && !stroked)),
    };
    // Lumit trims and offsets the combined outline, a player each path in
    // the group. The outline is baked when nothing moves.
    let cuts =
        |m: &ShapeModifier| matches!(m.kind, ModifierKind::Trim(_) | ModifierKind::Offset(_));
    if multi && kept.iter().any(|m| cuts(m)) {
        if moves(members, mods) {
            report.changed(
                &here,
                "combined_trim",
                "Trims each path, not the combined shape".into(),
                Vec::new(),
            );
        } else {
            multi = false;
            (kept, bake_mods) = mods.iter().partition(|m| repeats(m));
        }
    }
    let mut rule = if kind == Some(4) && multi { 2 } else { 1 };
    let needs_bake = !bake_mods.is_empty() || (members.len() > 1 && !multi);

    let mut shapes: Vec<Value> = Vec::new();
    let mut baked = false;
    if needs_bake {
        let label = match last_bend.and_then(|k| mods.get(k)) {
            Some(m) => m.kind.label(),
            None => combine_label(kind.unwrap_or(2)),
        };
        if moves(members, &bake_mods) {
            // Only what bends is left out: a trim or an offset before it is
            // still written, as is a combine a player can draw.
            for m in bake_mods.iter().filter(|m| bends(m)) {
                let label = m.kind.label();
                report.left_out(
                    &here,
                    "modifier_animated",
                    format!("{label} is left out"),
                    vec![arg("label", label)],
                );
            }
            kept = mods.iter().filter(|m| !bends(m)).copied().collect();
            bake_mods.clear();
        } else {
            let closed = members.len() > 1 || item.path_at(0.0).closed;
            for contour in bake(members, &bake_mods) {
                shapes.push(
                    json!({"ty": "sh", "ks": c.path(&polyline(&contour, closed), &[], (0.0, 0.0))}),
                );
            }
            report.changed(
                &here,
                "baked_outline",
                format!("{label} is baked into the path"),
                vec![arg("label", label)],
            );
            baked = true;
        }
    }
    if !baked {
        if members.len() > 1 && !multi {
            // An animated subtract or intersect: the first path alone.
            let label = combine_label(kind.unwrap_or(2));
            report.left_out(
                &here,
                "merge",
                format!("{label} needs merge paths"),
                vec![arg("label", label)],
            );
        }
        let trimmed = kept.iter().any(|m| matches!(m.kind, ModifierKind::Trim(_)));
        let reference = winding(&item.path_at(0.0));
        let paths = if multi { members.len() } else { 1 };
        for member in members.iter().take(paths) {
            let flip = kind == Some(1) && winding(&member.item.path_at(0.0)) * reference < 0.0;
            shapes.push(path_item(c, member.item, trimmed, flip));
        }
    } else {
        rule = 1;
    }

    // The modifiers a player has, in their order, then the paint.
    let mut repeaters = Vec::new();
    let closed = baked || multi || item.path_at(0.0).closed;
    for m in kept.drain(..) {
        match &m.kind {
            ModifierKind::Trim(t) => shapes.push(trim(c, t, closed)),
            ModifierKind::Offset(o) => {
                report.players(
                    &here,
                    Feature::OffsetPaths,
                    "offset_paths",
                    "Offset paths".into(),
                    Vec::new(),
                );
                shapes.push(json!({
                    "ty": "op",
                    "a": c.prop(&[&o.amount], Class::Px, tolerance::PIXELS),
                    "lj": 2,
                    "ml": {"a": 0, "k": 4},
                }));
            }
            ModifierKind::Repeat(r) => repeaters.push(json!({
                "ty": "rp",
                "c": stepped(c, &r.copies, 1.0, MAX_COPIES as f64),
                "o": stepped(c, &r.offset, -(MAX_COPIES as f64), MAX_COPIES as f64),
                "m": 1,
                "tr": {
                    "ty": "tr",
                    "a": c.prop(&[&r.anchor_x, &r.anchor_y], Class::Px, tolerance::PIXELS),
                    "p": c.prop(&[&r.position_x, &r.position_y], Class::Px, tolerance::PIXELS),
                    "r": c.prop(&[&r.rotation], Class::Deg, tolerance::DEGREES),
                    "s": c.prop(&[&r.scale, &r.scale], Class::Px, tolerance::PERCENT),
                    "so": c.prop(&[&r.start_opacity], Class::Px, tolerance::PERCENT),
                    "eo": c.prop(&[&r.end_opacity], Class::Px, tolerance::PERCENT),
                },
            })),
            // Baked above, or left out with a word.
            _ => {}
        }
    }
    if let Some(stroke) = item.stroke_shown() {
        shapes.push(stroke_item(c, item, stroke, report, &here));
    }
    if let Some(fill) = item.fill_shown() {
        shapes.push(if item.gradient == 0 {
            json!({
                "ty": "fl",
                "c": c.colour([&fill[0], &fill[1], &fill[2]]),
                "o": c.prop_mapped(&fill[3], Class::Px, tolerance::PERCENT, |a| a * 100.0),
                "r": rule,
            })
        } else {
            gradient(c, item, fill, rule)
        });
    }
    shapes.extend(repeaters);
    shapes.push(json!({
        "ty": "tr",
        "a": {"a": 0, "k": [0, 0]},
        "p": {"a": 0, "k": [0, 0]},
        "s": {"a": 0, "k": [100, 100]},
        "r": {"a": 0, "k": 0},
        "o": c.prop(&[&item.opacity], Class::Px, tolerance::PERCENT),
    }));
    let mut g = Map::new();
    g.insert("ty".into(), Value::from("gr"));
    c.fmt.name(&mut g, &item.name);
    g.insert("it".into(), Value::Array(shapes));
    Some(Value::Object(g))
}

/// A trim as a player reads it. Lumit draws nothing once the end is at or
/// before the start, and slides an open path's window off its ends rather
/// than round them; a player swaps the two ends and wraps. Where either
/// would happen, the ends Lumit draws are written frame by frame, and an
/// empty trim as two equal ends.
fn trim(c: &Clock, t: &Trim, closed: bool) -> Value {
    let pct = |p: &Property, lt: f64| c.value(p, lt).clamp(0.0, 100.0);
    let ends = |lt: f64| {
        let (s, e) = (pct(&t.start, lt), pct(&t.end, lt));
        let (s, e) = if closed {
            (s, e)
        } else {
            let slide = c.value(&t.offset, lt) / 3.6;
            ((s + slide).clamp(0.0, 100.0), (e + slide).clamp(0.0, 100.0))
        };
        if e > s {
            [s, e]
        } else {
            [0.0, 0.0]
        }
    };
    let slides = !matches!(t.offset.animation, Animation::Static(v) if v == 0.0);
    let exact = (closed || !slides)
        && c.times()
            .into_iter()
            .all(|lt| pct(&t.end, lt) >= pct(&t.start, lt));
    if exact {
        return json!({
            "ty": "tm",
            "s": c.prop(&[&t.start], Class::Px, tolerance::PERCENT),
            "e": c.prop(&[&t.end], Class::Px, tolerance::PERCENT),
            "o": c.prop(&[&t.offset], Class::Deg, tolerance::DEGREES),
            "m": 1,
        });
    }
    if [&t.start, &t.end, &t.offset]
        .iter()
        .any(|p| matches!(p.animation, Animation::Expression(_)))
    {
        c.baked.set(true);
    }
    let end = |i: usize| {
        let track = c.sampled(|lt| vec![ends(lt)[i]], &[tolerance::PERCENT]);
        c.write(&track, Class::Px, false, &[])
    };
    json!({
        "ty": "tm",
        "s": end(0),
        "e": end(1),
        "o": if closed {
            c.prop(&[&t.offset], Class::Deg, tolerance::DEGREES)
        } else {
            json!({"a": 0, "k": 0})
        },
        "m": 1,
    })
}

/// A count Lumit rounds and holds to `lo..=hi`, as hold keys where it
/// steps, since a player rounds it differently or not at all.
fn stepped(c: &Clock, p: &Property, lo: f64, hi: f64) -> Value {
    let step = |lt: f64| c.value(p, lt).round().clamp(lo, hi);
    if !Clock::moves(p) {
        return json!({"a": 0, "k": c.fmt.num(step(0.0), Class::Px)});
    }
    if matches!(p.animation, Animation::Expression(_)) {
        c.baked.set(true);
    }
    let mut keys: Vec<TrackKey> = Vec::new();
    for lt in c.times() {
        let v = step(lt);
        if keys.last().is_none_or(|k| k.value.first() != Some(&v)) {
            keys.push(TrackKey {
                seconds: lt,
                value: vec![v],
                hold: true,
                out: Vec::new(),
                inn: Vec::new(),
            });
        }
    }
    c.write(&Track { keys }, Class::Px, false, &[])
}

/// True when anything the bake would read moves: a member's path or its
/// numbers, or a modifier's.
fn moves(members: &[Lowered], mods: &[&ShapeModifier]) -> bool {
    fn animated(v: &Value) -> bool {
        match v {
            Value::Object(map) => {
                let keyed = map
                    .get("animation")
                    .and_then(Value::as_object)
                    .is_some_and(|a| !a.contains_key("Static"));
                keyed || map.values().any(animated)
            }
            Value::Array(list) => list.iter().any(animated),
            _ => false,
        }
    }
    members.iter().any(|m| m.item.path_is_animated())
        || mods
            .iter()
            .any(|m| serde_json::to_value(&m.kind).map_or(true, |v| animated(&v)))
}

/// The outline Lumit draws for `members` through `mods`, which have nothing
/// moving, as polylines in the art's own coordinates. Lumit's own shape code
/// draws it, so the outline is the one the picture has.
fn bake(members: &[Lowered], mods: &[&ShapeModifier]) -> Vec<Vec<(f64, f64)>> {
    let white = LinearColour([1.0, 1.0, 1.0, 1.0]);
    let mut entries: Vec<ShapeEntry> = members
        .iter()
        .enumerate()
        .map(|(i, m)| {
            let mut item = ShapeItem::filled(m.item.name.clone(), m.item.path.clone(), white);
            item.primitive = m.item.primitive.clone();
            item.path_keys = m.item.path_keys.clone();
            item.combine = if i == 0 { 0 } else { m.item.combine };
            ShapeEntry::Item(item)
        })
        .collect();
    entries.extend(mods.iter().map(|m| {
        let mut m = (*m).clone();
        m.own = false;
        ShapeEntry::Modifier(m)
    }));
    let Some((x0, y0, ..)) = contents_bounds(&entries, 0.0) else {
        return Vec::new();
    };
    solid::solids(&entries, 0.0)
        .into_iter()
        .flat_map(|s| s.contours)
        .map(|contour| {
            let mut points: Vec<(f64, f64)> =
                contour.iter().map(|p| (p[0] + x0, p[1] + y0)).collect();
            if points.len() > 1 && points.first() == points.last() {
                points.pop();
            }
            points
        })
        .filter(|p| p.len() >= 2)
        .collect()
}

/// A polyline as a path with straight edges.
fn polyline(points: &[(f64, f64)], closed: bool) -> BezierPath {
    BezierPath {
        vertices: points
            .iter()
            .map(|p| Vertex {
                pos: *p,
                tan_in: (0.0, 0.0),
                tan_out: (0.0, 0.0),
            })
            .collect(),
        closed,
    }
}

/// Twice the signed area a path encloses, by its flattened outline: which
/// way round it runs.
fn winding(path: &BezierPath) -> f64 {
    let points = lumit_core::shape::geometry::flatten_path(path);
    points
        .iter()
        .zip(points.iter().cycle().skip(1))
        .map(|(a, b)| a.0 * b.1 - b.0 * a.1)
        .sum()
}

/// `path` run the other way round.
fn reversed(path: &BezierPath) -> BezierPath {
    BezierPath {
        vertices: path
            .vertices
            .iter()
            .rev()
            .map(|v| Vertex {
                pos: v.pos,
                tan_in: v.tan_out,
                tan_out: v.tan_in,
            })
            .collect(),
        closed: path.closed,
    }
}

/// An item's path: a rectangle, ellipse or polystar a player draws from its
/// numbers when no trim reaches it (a trim starts where the player's shape
/// starts, which is not where Lumit's does), else the path itself.
fn path_item(c: &Clock, item: &ShapeItem, trimmed: bool, flip: bool) -> Value {
    // Dashes start where the path does too: a player's rectangle starts on
    // its right edge, Lumit's at its top left. Its ellipse and polystar
    // start at the top and run the same way round as Lumit's.
    let dashed = !item.dashes.is_empty() && item.stroke_shown().is_some();
    if let (Some(p), false, false) = (&item.primitive, trimmed, flip) {
        if let Some(v) = (!dashed || p.kind != 0).then(|| primitive(c, p)).flatten() {
            return v;
        }
    }
    let ks = match &item.primitive {
        Some(p) if p.is_animated() => c.path_sampled(
            |lt| {
                let path = p.path_at(lt);
                if flip {
                    reversed(&path)
                } else {
                    path
                }
            },
            (0.0, 0.0),
        ),
        Some(p) => {
            let path = p.path_at(0.0);
            c.path(&if flip { reversed(&path) } else { path }, &[], (0.0, 0.0))
        }
        None if flip => {
            let keys: Vec<_> = item
                .path_keys
                .iter()
                .map(|k| lumit_core::mask::PathKeyframe {
                    path: reversed(&k.path),
                    ..k.clone()
                })
                .collect();
            c.path(&reversed(&item.path), &keys, (0.0, 0.0))
        }
        None => c.path(&item.path, &item.path_keys, (0.0, 0.0)),
    };
    json!({"ty": "sh", "ks": ks})
}

/// A parametric shape as a player's own, or `None` where the player would
/// round its corners differently from Lumit (a rounded polygon or star).
fn primitive(c: &Clock, p: &ShapePrimitive) -> Option<Value> {
    let centre = c.prop(
        &[&p.position_x, &p.position_y],
        Class::Px,
        tolerance::PIXELS,
    );
    let size = || c.prop(&[&p.width, &p.height], Class::Px, tolerance::PIXELS);
    let px = |q: &Property| c.prop(&[q], Class::Px, tolerance::PIXELS);
    match p.kind {
        0 => Some(json!({"ty": "rc", "d": 1, "p": centre, "s": size(), "r": px(&p.roundness)})),
        1 => Some(json!({"ty": "el", "d": 1, "p": centre, "s": size()})),
        kind => {
            let still_zero = |q: &Property| matches!(q.animation, Animation::Static(v) if v == 0.0);
            if !(still_zero(&p.outer_roundness) && still_zero(&p.inner_roundness)) {
                return None;
            }
            let star = kind == 3;
            Some(json!({
                "ty": "sr",
                "d": 1,
                "sy": if star { 1 } else { 2 },
                "pt": stepped(c, &p.points, 3.0, MAX_POINTS),
                "p": centre,
                "r": c.prop(&[&p.rotation], Class::Deg, tolerance::DEGREES),
                "or": px(&p.outer_radius),
                "os": {"a": 0, "k": 0},
                "ir": px(&p.inner_radius),
                "is": {"a": 0, "k": 0},
            }))
        }
    }
}

/// An item's outline: colour, opacity, width and dashes, with Lumit's round
/// caps and joins.
fn stroke_item(
    c: &Clock,
    item: &ShapeItem,
    colour: &[Property; 4],
    report: &mut Report,
    at: &Place,
) -> Value {
    if item.taper.is_some() {
        report.left_out(
            at,
            "taper",
            "Stroke taper is left out".into(),
            vec![arg("label", "Taper")],
        );
    }
    let mut st = Map::new();
    st.insert("ty".into(), Value::from("st"));
    st.insert("c".into(), c.colour([&colour[0], &colour[1], &colour[2]]));
    st.insert(
        "o".into(),
        c.prop_mapped(&colour[3], Class::Px, tolerance::PERCENT, |a| a * 100.0),
    );
    st.insert(
        "w".into(),
        c.prop(&[&item.stroke_width], Class::Px, tolerance::PIXELS),
    );
    st.insert("lc".into(), Value::from(2));
    st.insert("lj".into(), Value::from(2));
    st.insert("ml".into(), Value::from(4));
    if !item.dashes.is_empty() {
        let mut pattern: Vec<&Property> = item.dashes.iter().collect();
        if pattern.len() % 2 == 1 {
            pattern.extend(item.dashes.iter());
        }
        let mut d: Vec<Value> = pattern
            .iter()
            .enumerate()
            .map(|(i, p)| {
                let (n, nm) = if i % 2 == 0 {
                    ("d", "dash")
                } else {
                    ("g", "gap")
                };
                json!({"n": n, "nm": nm, "v": c.prop(&[p], Class::Px, tolerance::PIXELS)})
            })
            .collect();
        // Named even when minified: lottie-web doesn't load dashes without.
        d.push(json!({
            "n": "o",
            "nm": "offset",
            "v": c.prop(&[&item.dash_offset], Class::Px, tolerance::PIXELS),
        }));
        st.insert("d".into(), Value::Array(d));
    }
    Value::Object(st)
}

/// A gradient fill: its two points, and its colour and opacity stops.
fn gradient(c: &Clock, item: &ShapeItem, fill: &[Property; 4], rule: u32) -> Value {
    let props = gradient_props(item, fill);
    let still = props
        .iter()
        .all(|p| matches!(p.animation, Animation::Static(_)));
    // Colours between Lumit's own, so the ramp a player mixes in display
    // colour passes through the colours Lumit mixes in linear light: one
    // set for every frame, placed for all of them.
    let times = if still { vec![0.0] } else { c.times() };
    let moments: Vec<Vec<[f64; 3]>> = times
        .iter()
        .map(|lt| points(c, item, fill, *lt).iter().map(|p| p.1).collect())
        .collect();
    let splits = crate::colour::splits(&moments, &|rgb| c.host.display(rgb));
    let stops = |lt: f64| stops(c, &points(c, item, fill, lt), &splits);
    let (count, first) = stops(0.0);
    let k = if still {
        json!({"a": 0, "k": c.fmt.nums(&first, Class::Unit)})
    } else {
        if props
            .iter()
            .any(|p| matches!(p.animation, Animation::Expression(_)))
        {
            c.baked.set(true);
        }
        let track = c.sampled(|lt| stops(lt).1, &vec![tolerance::CHANNEL; first.len()]);
        c.write(&track, Class::Unit, true, &[])
    };
    json!({
        "ty": "gf",
        "o": {"a": 0, "k": 100},
        "r": rule,
        "t": if item.gradient == 2 { 2 } else { 1 },
        "s": c.prop(&[&item.gradient_start_x, &item.gradient_start_y], Class::Px, tolerance::PIXELS),
        "e": c.prop(&[&item.gradient_end_x, &item.gradient_end_y], Class::Px, tolerance::PIXELS),
        "h": {"a": 0, "k": 0},
        "a": {"a": 0, "k": 0},
        "g": {"p": count, "k": k},
    })
}

/// Every number a gradient's stops are read from.
fn gradient_props<'a>(item: &'a ShapeItem, fill: &'a [Property; 4]) -> Vec<&'a Property> {
    let mut out: Vec<&Property> = fill.iter().collect();
    out.push(&item.gradient_start_opacity);
    out.push(&item.gradient_end_opacity);
    for s in &item.gradient_stops {
        out.push(&s.position);
        out.push(&s.opacity);
    }
    out
}

/// A gradient's own stops at layer time `lt`: where, linear colour, alpha.
fn points(c: &Clock, item: &ShapeItem, fill: &[Property; 4], lt: f64) -> Vec<(f64, [f64; 3], f64)> {
    let v = |p: &Property| c.value(p, lt);
    let faded = |p: &Property| v(p).clamp(0.0, 100.0) / 100.0;
    let to = item
        .gradient_colour
        .unwrap_or(LinearColour::BLACK)
        .0
        .map(f64::from);
    let from = [v(&fill[0]), v(&fill[1]), v(&fill[2]), v(&fill[3])];
    let mut between: Vec<(f64, [f64; 4], f64)> = item
        .gradient_stops
        .iter()
        .take(lumit_core::shape::MAX_GRADIENT_STOPS)
        .map(|s| {
            let colour = s.colour.0.map(f64::from);
            (
                (v(&s.position).clamp(0.0, 100.0)) / 100.0,
                colour,
                colour[3] * faded(&s.opacity),
            )
        })
        .collect();
    between.sort_by(|a, b| a.0.total_cmp(&b.0));
    let mut points: Vec<(f64, [f64; 3], f64)> = vec![(
        0.0,
        [from[0], from[1], from[2]],
        from[3] * faded(&item.gradient_start_opacity),
    )];
    points.extend(
        between
            .iter()
            .map(|(at, c, a)| (*at, [c[0], c[1], c[2]], *a)),
    );
    points.push((
        1.0,
        [to[0], to[1], to[2]],
        to[3] * faded(&item.gradient_end_opacity),
    ));
    points
}

/// `points` as Lottie's flat list: the colour stops (offset, r, g, b), with
/// colours mixed in linear light at `splits` between each two, then the
/// opacity stops (offset, a); with how many colour stops there are.
fn stops(c: &Clock, points: &[(f64, [f64; 3], f64)], splits: &[Vec<f64>]) -> (usize, Vec<f64>) {
    let mut colours = Vec::new();
    let mut count = 0;
    for (i, (at, rgb, _)) in points.iter().enumerate() {
        let shown = c.host.display(*rgb);
        colours.extend([*at, shown[0], shown[1], shown[2]]);
        count += 1;
        if let Some((next_at, next, _)) = points.get(i + 1) {
            for u in splits.get(i).into_iter().flatten() {
                let mix = [0, 1, 2].map(|ch| rgb[ch] + (next[ch] - rgb[ch]) * u);
                let shown = c.host.display(mix);
                colours.extend([at + (next_at - at) * u, shown[0], shown[1], shown[2]]);
                count += 1;
            }
        }
    }
    for (at, _, alpha) in points {
        colours.extend([*at, alpha.clamp(0.0, 1.0)]);
    }
    (count, colours)
}

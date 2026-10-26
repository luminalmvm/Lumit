//! Shape layers: After Effects' contents tree as Lumit's flat list of items.
//!
//! In After Effects a shape layer holds groups, each with its own transform,
//! and inside them paths and the operators that act on the paths above them:
//! a fill or a stroke paints them, Trim Paths cuts them and a repeater copies
//! everything above it. Lumit keeps one item per painted path, each followed by
//! the modifiers that act on it. So the tree is walked once, every path is
//! carried to layer space through the transforms of the groups it sits in, and
//! each paint makes one item for each path above it. An item higher in After
//! Effects' list draws on top, so the list is turned round at the end.
//!
//! A path whose size, place or group transform moves is sampled a frame at a
//! time across the keys that move it, so it lands where After Effects draws it
//! on every frame.

use lumit_core::anim::{Animation, Property as LumProperty};
use lumit_core::mask::{BezierPath, PathKeyframe, Vertex};
use lumit_core::model::LinearColour;
use lumit_core::shape::colour;
use lumit_core::shape::modifier::{
    Deform, ModifierKind, Offset, Repeater, ShapeEntry, ShapeGroup, ShapeModifier, Trim,
};
use lumit_core::shape::ShapeItem;
use lumit_core::time::Rational;

use crate::capture::Property;
use crate::report::{ItemPath, Outcome, Reason};

use super::props::{child, display_name, from_node, match_name_of, starting_value};
use super::shape_paths::{ellipse, rect, star, Bake, Op};
use super::{linear_colour, Conv};

/// A shape layer's contents, ready to draw.
pub(super) fn contents(
    conv: &mut Conv<'_>,
    path: &ItemPath,
    props: &[Property],
) -> Vec<ShapeEntry> {
    let root = child(props, "ADBE Root Vectors Group").map_or(&[][..], Property::children);
    let mut items = walk(conv, path, root, &[]).items;
    items.reverse();
    entries(items)
}

/// An item and the modifiers that act on it, in the order they apply.
struct Art {
    item: ShapeItem,
    mods: Vec<ModifierKind>,
}

/// The items as entries. A run combined into one shape takes its first item's
/// modifiers, after the run so they act on all of it. A run with modifiers
/// below other art goes in a group of its own, so they reach nothing else.
fn entries(items: Vec<Art>) -> Vec<ShapeEntry> {
    let mut runs: Vec<Vec<Art>> = Vec::new();
    for art in items {
        match runs.last_mut() {
            Some(run) if art.item.combine != 0 => run.push(art),
            _ => runs.push(vec![art]),
        }
    }
    let mut out = Vec::new();
    for mut run in runs {
        let mods = std::mem::take(&mut run[0].mods);
        let name = run[0].item.name.clone();
        let mut entries: Vec<ShapeEntry> = run
            .into_iter()
            .map(|art| ShapeEntry::Item(art.item))
            .collect();
        if mods.is_empty() {
            out.append(&mut entries);
            continue;
        }
        entries.extend(mods.into_iter().map(|kind| {
            ShapeEntry::Modifier(ShapeModifier {
                id: uuid::Uuid::now_v7(),
                name: kind.label().into(),
                kind,
            })
        }));
        if out.is_empty() {
            out.append(&mut entries);
        } else {
            out.push(ShapeEntry::Group(ShapeGroup::new(name, entries)));
        }
    }
    out
}

/// How far the layer's origin moves: After Effects measures a shape layer
/// from its own zero and Lumit from its art's top-left corner.
pub(super) fn origin(contents: &[ShapeEntry]) -> Option<(f64, f64)> {
    let (x, y, ..) = lumit_core::shape::contents_bounds(contents, 0.0)?;
    Some((-x, -y))
}

/// Art that moves takes its top-left corner with it, so the anchor follows the
/// corner on every frame the art is keyed on, and the art stays where After
/// Effects draws it.
pub(super) fn follow(contents: &[ShapeEntry], transform: &mut lumit_core::model::TransformGroup) {
    let items = lumit_core::shape::modifier::items(contents);
    let span = items
        .iter()
        .filter_map(|item| Some((item.path_keys.first()?.time, item.path_keys.last()?.time)))
        .reduce(|a, b| (a.0.min(b.0), a.1.max(b.1)));
    let (Some((first, last)), Some((x0, y0, ..))) =
        (span, lumit_core::shape::contents_bounds(contents, 0.0))
    else {
        return;
    };
    let mut times: Vec<Rational> = items
        .iter()
        .flat_map(|item| item.path_keys.iter().map(|k| k.time))
        .collect();
    times.sort();
    times.dedup();
    let times: Vec<Rational> = times
        .into_iter()
        .filter(|t| *t >= first && *t <= last)
        .collect();
    for (axis, anchor) in [(0, &mut transform.anchor_x), (1, &mut transform.anchor_y)] {
        let keys = times
            .iter()
            .filter_map(|t| {
                let (x, y, ..) = lumit_core::shape::contents_bounds(contents, t.to_f64())?;
                let moved = [x0 - x, y0 - y][axis];
                Some(lumit_core::anim::Keyframe {
                    time: *t,
                    value: anchor.value_at(t.to_f64()) + moved,
                    interp_in: lumit_core::anim::SideInterp::Linear,
                    interp_out: lumit_core::anim::SideInterp::Linear,
                })
            })
            .collect();
        anchor.animation = Animation::Keyframed(keys);
    }
}

/// One group's transform, as its five properties.
#[derive(Clone)]
struct Xf {
    anchor: [LumProperty; 2],
    position: [LumProperty; 2],
    scale: [LumProperty; 2],
    rotation: LumProperty,
}

/// A path as it was made, with the transforms of the groups it sits in, the
/// innermost first, and the trim that cuts it.
#[derive(Clone)]
struct Drawn {
    name: String,
    make: Make,
    chain: Vec<Xf>,
    trim: Option<[LumProperty; 3]>,
    /// Operators worked into the points, each after this many of the
    /// innermost group transforms, which is the space it was in.
    bakes: Vec<(usize, Bake)>,
    merge: Option<u32>,
    offset: Option<LumProperty>,
    wiggle: Option<Deform>,
}

/// What makes a path.
#[derive(Clone)]
enum Make {
    Rect {
        size: [LumProperty; 2],
        at: [LumProperty; 2],
        round: LumProperty,
    },
    Ellipse {
        size: [LumProperty; 2],
        at: [LumProperty; 2],
    },
    Star(Box<Star>),
    Path {
        still: BezierPath,
        keys: Vec<PathKeyframe>,
    },
}

#[derive(Clone)]
pub(super) struct Star {
    pub(super) polygon: bool,
    pub(super) points: LumProperty,
    pub(super) at: [LumProperty; 2],
    pub(super) rotation: LumProperty,
    pub(super) inner: LumProperty,
    pub(super) outer: LumProperty,
    pub(super) inner_round: LumProperty,
    pub(super) outer_round: LumProperty,
}

/// What one group hands its parent: the paths it made, for a paint further
/// down the parent to draw too, and the items its own paints made, in After
/// Effects' order.
struct Walked {
    drawn: Vec<(usize, Drawn)>,
    items: Vec<Art>,
}

fn walk(conv: &mut Conv<'_>, path: &ItemPath, nodes: &[Property], chain: &[Xf]) -> Walked {
    // Each entry's place in the list, which is what decides what an operator
    // reaches: everything above it.
    let mut drawn: Vec<(usize, Drawn)> = Vec::new();
    let mut items: Vec<(usize, Art)> = Vec::new();
    let mut paints: Vec<(usize, &Property)> = Vec::new();
    let mut repeaters: Vec<(usize, &Property)> = Vec::new();
    for (at, node) in nodes.iter().enumerate() {
        if node.enabled == Some(false) {
            continue;
        }
        let name = display_name(node, match_name_of(node)).to_string();
        let here = path.property(&name);
        let props = node.children();
        let make = match match_name_of(node) {
            "ADBE Vector Group" => {
                let mut inner = chain.to_vec();
                let transform =
                    child(props, "ADBE Vector Transform Group").map_or(&[][..], Property::children);
                inner.insert(0, xf(conv, &here, transform));
                let contents =
                    child(props, "ADBE Vectors Group").map_or(&[][..], Property::children);
                let mut walked = walk(conv, &here, contents, &inner);
                let opacity = still_value(transform, "ADBE Vector Group Opacity", 100.0);
                if !keyed(transform, "ADBE Vector Group Opacity") {
                    for art in &mut walked.items {
                        let own = art.item.opacity.value_at(0.0);
                        art.item.opacity = LumProperty::fixed(own * opacity / 100.0);
                    }
                } else {
                    kept_still(conv, &here);
                }
                drawn.extend(walked.drawn.into_iter().map(|(_, d)| (at, d)));
                items.extend(walked.items.into_iter().map(|item| (at, item)));
                if still_value(transform, "ADBE Vector Skew", 0.0) != 0.0 {
                    not_carried(conv, &here, "Skew");
                }
                None
            }
            "ADBE Vector Shape - Rect" => Some(Make::Rect {
                size: pair(conv, &here, props, "ADBE Vector Rect Size", [100.0, 100.0]),
                at: pair(conv, &here, props, "ADBE Vector Rect Position", [0.0, 0.0]),
                round: one(conv, &here, props, "ADBE Vector Rect Roundness", 0.0),
            }),
            "ADBE Vector Shape - Ellipse" => Some(Make::Ellipse {
                size: pair(
                    conv,
                    &here,
                    props,
                    "ADBE Vector Ellipse Size",
                    [100.0, 100.0],
                ),
                at: pair(
                    conv,
                    &here,
                    props,
                    "ADBE Vector Ellipse Position",
                    [0.0, 0.0],
                ),
            }),
            "ADBE Vector Shape - Star" => Some(Make::Star(Box::new(Star {
                polygon: still_value(props, "ADBE Vector Star Type", 1.0) == 2.0,
                points: one(conv, &here, props, "ADBE Vector Star Points", 5.0),
                at: pair(conv, &here, props, "ADBE Vector Star Position", [0.0, 0.0]),
                rotation: one(conv, &here, props, "ADBE Vector Star Rotation", 0.0),
                inner: one(conv, &here, props, "ADBE Vector Star Inner Radius", 50.0),
                outer: one(conv, &here, props, "ADBE Vector Star Outer Radius", 100.0),
                // After Effects spells these two without the second n.
                inner_round: one(conv, &here, props, "ADBE Vector Star Inner Roundess", 0.0),
                outer_round: one(conv, &here, props, "ADBE Vector Star Outer Roundess", 0.0),
            }))),
            "ADBE Vector Shape - Group" => child(props, "ADBE Vector Shape").map(|shape| {
                let keys: Vec<PathKeyframe> = shape
                    .keyframes
                    .as_deref()
                    .unwrap_or_default()
                    .iter()
                    .filter_map(|key| {
                        let ae: crate::capture::Shape =
                            serde_json::from_value(key.v.clone()?).ok()?;
                        Some(PathKeyframe {
                            time: conv.layer_time(key.t?),
                            path: super::masks::bezier(&ae),
                            interp_in: super::props::path_side(key.in_interp.as_deref()),
                            interp_out: super::props::path_side(key.out_interp.as_deref()),
                        })
                    })
                    .collect();
                let still = shape
                    .shape()
                    .as_ref()
                    .map(super::masks::bezier)
                    .or_else(|| keys.first().map(|k| k.path.clone()))
                    .unwrap_or(BezierPath {
                        vertices: Vec::new(),
                        closed: true,
                    });
                Make::Path { still, keys }
            }),
            "ADBE Vector Graphic - Fill"
            | "ADBE Vector Graphic - Stroke"
            | "ADBE Vector Graphic - G-Fill"
            | "ADBE Vector Graphic - G-Stroke" => {
                paints.push((at, node));
                None
            }
            "ADBE Vector Filter - Trim" => {
                let trim = [
                    one(conv, &here, props, "ADBE Vector Trim Start", 0.0),
                    one(conv, &here, props, "ADBE Vector Trim End", 100.0),
                    one(conv, &here, props, "ADBE Vector Trim Offset", 0.0),
                ];
                // Trim Paths cuts every path above it, painted before it or
                // after.
                for (_, d) in drawn.iter_mut() {
                    d.trim = Some(trim.clone());
                }
                None
            }
            "ADBE Vector Filter - Repeater" => {
                repeaters.push((at, node));
                None
            }
            "ADBE Vector Blend Mode"
            | "ADBE Vector Materials Group"
            | "ADBE Vector Transform Group" => None,
            _ => {
                match super::shape_paths::read(conv, &here, node) {
                    Some(op) => {
                        if matches!(op, Op::Wiggle(_) | Op::Offset(_, false)) {
                            conv.report.row(
                                here.clone(),
                                Outcome::Adjusted,
                                Reason::ShapeOperatorApproximated {
                                    name: kind_name(node, &name),
                                },
                            );
                        }
                        for (_, d) in drawn.iter_mut() {
                            let depth = d.chain.len().saturating_sub(chain.len());
                            match &op {
                                Op::Bake(bake) => d.bakes.push((depth, bake.clone())),
                                Op::Merge(mode) => d.merge = Some(*mode),
                                Op::Offset(amount, _) => d.offset = Some(amount.clone()),
                                Op::Wiggle(deform) => d.wiggle = Some(deform.clone()),
                            }
                        }
                    }
                    None => not_carried(conv, &here, &kind_name(node, &name)),
                }
                None
            }
        };
        if let Some(make) = make {
            drawn.push((
                at,
                Drawn {
                    name,
                    make,
                    chain: chain.to_vec(),
                    trim: None,
                    bakes: Vec::new(),
                    merge: None,
                    offset: None,
                    wiggle: None,
                },
            ));
        }
    }

    for (at, node) in paints {
        let name = display_name(node, match_name_of(node)).to_string();
        let here = path.property(&name);
        let props = node.children();
        if match_name_of(node) == "ADBE Vector Graphic - G-Stroke" {
            not_carried(conv, &here, "Gradient stroke");
        }
        // The script route cannot read a gradient's colours at all, so they
        // arrive in After Effects' own starting colours.
        if child(props, "ADBE Vector Grad Colors").is_some_and(|node| node.unreadable.is_some()) {
            conv.report.row(
                here.clone(),
                Outcome::Adjusted,
                Reason::ShapeGradientColoursDefault,
            );
        }
        let moving = [
            "ADBE Vector Fill Color",
            "ADBE Vector Fill Opacity",
            "ADBE Vector Stroke Color",
            "ADBE Vector Stroke Opacity",
            "ADBE Vector Stroke Width",
        ];
        if moving.iter().any(|name| keyed(props, name)) {
            kept_still(conv, &here);
        }
        let mut merged = Vec::new();
        for (_, d) in drawn.iter().filter(|(above, _)| *above < at) {
            match (paint(conv, node, d), d.merge) {
                (Some(item), None) => items.push((at, item)),
                (Some(mut art), Some(mode)) => {
                    art.item.combine = if merged.is_empty() { 0 } else { mode };
                    merged.push(art);
                }
                (None, _) => {}
            }
        }
        // Pushed backwards, as the whole list is turned round at the end and a
        // run of combined items has to start with the topmost.
        items.extend(merged.into_iter().rev().map(|item| (at, item)));
    }
    for (at, node) in repeaters {
        let here = path.property(display_name(node, match_name_of(node)));
        for (_, art) in items.iter_mut().filter(|(above, _)| *above < at) {
            let copies = repeat(conv, &here, node.children());
            art.mods.push(ModifierKind::Repeat(copies));
        }
    }
    // After Effects' order, top of the list first. A stable sort keeps a
    // group's own items in the order its paints made them.
    items.sort_by_key(|(at, _)| *at);
    Walked {
        drawn,
        items: items.into_iter().map(|(_, item)| item).collect(),
    }
}

/// One item: the path under one fill or stroke.
fn paint(conv: &mut Conv<'_>, node: &Property, drawn: &Drawn) -> Option<Art> {
    let props = node.children();
    let kind = match_name_of(node);
    let stroke = kind.ends_with("Stroke");
    let gradient = kind.contains("G-");
    let (colour_name, opacity_name) = if stroke {
        ("ADBE Vector Stroke Color", "ADBE Vector Stroke Opacity")
    } else {
        ("ADBE Vector Fill Color", "ADBE Vector Fill Opacity")
    };
    // After Effects' own new fill is red and its new stroke white, and a file
    // stores only what is not at its default.
    let default = if stroke {
        [1.0, 1.0, 1.0]
    } else {
        [1.0, 0.0, 0.0]
    };
    let colour = colour_of(props, colour_name).unwrap_or(default);

    let (still, keys) = geometry(conv, drawn);
    let mut item = ShapeItem::filled(drawn.name.clone(), still, linear_colour(Some(&colour)));
    item.name = drawn.name.clone();
    item.path_keys = keys;
    item.opacity = LumProperty::fixed(still_value(props, opacity_name, 100.0));
    if stroke {
        item.stroke = item.fill.take();
        // A group's scale thickens the outline in After Effects too.
        item.stroke_width = LumProperty::fixed(
            still_value(props, "ADBE Vector Stroke Width", 2.0) * chain_scale(&drawn.chain),
        );
    }
    // A file leaves out a gradient at After Effects' own white to black.
    let ramp = child(props, "ADBE Vector Grad Colors")
        .and_then(|node| node.value.as_ref())
        .and_then(Ramp::of)
        .unwrap_or_else(Ramp::white_to_black);
    if gradient && stroke {
        // Lumit's outlines are one colour, so a gradient one takes its first.
        item.stroke = Some(colour::still(ramp.colour_at(0.0)));
    }
    if gradient && !stroke {
        item.fill = Some(colour::still(ramp.colour_at(0.0)));
        item.gradient_colour = Some(ramp.colour_at(1.0));
        item.gradient_start_opacity = LumProperty::fixed(ramp.alpha_at(0.0) * 100.0);
        item.gradient_end_opacity = LumProperty::fixed(ramp.alpha_at(1.0) * 100.0);
        item.gradient_stops = ramp
            .inner()
            .into_iter()
            .map(|at| lumit_core::shape::GradientStop {
                position: LumProperty::fixed(at * 100.0),
                colour: ramp.colour_at(at),
                opacity: LumProperty::fixed(ramp.alpha_at(at) * 100.0),
            })
            .collect();
        item.gradient = if still_value(props, "ADBE Vector Grad Type", 1.0) == 2.0 {
            2
        } else {
            1
        };
        let start = place(
            &drawn.chain,
            point_of(props, "ADBE Vector Grad Start Pt").unwrap_or([0.0, 0.0]),
            0.0,
        );
        let end = place(
            &drawn.chain,
            point_of(props, "ADBE Vector Grad End Pt").unwrap_or([100.0, 0.0]),
            0.0,
        );
        item.gradient_start_x = LumProperty::fixed(start.0);
        item.gradient_start_y = LumProperty::fixed(start.1);
        item.gradient_end_x = LumProperty::fixed(end.0);
        item.gradient_end_y = LumProperty::fixed(end.1);
    }
    // In the order Lumit's shape items always took them.
    let mut mods = Vec::new();
    if let Some(deform) = drawn.wiggle.clone() {
        mods.push(ModifierKind::Deform(deform));
    }
    if let Some(amount) = drawn.offset.clone() {
        mods.push(ModifierKind::Offset(Offset {
            amount,
            extra: serde_json::Map::new(),
        }));
    }
    if let Some([start, end, offset]) = drawn.trim.clone() {
        mods.push(ModifierKind::Trim(Trim {
            start,
            end,
            offset,
            extra: serde_json::Map::new(),
        }));
    }
    (!item.path.vertices.is_empty()).then_some(Art { item, mods })
}

/// The repeater, for an item above it.
fn repeat(conv: &mut Conv<'_>, path: &ItemPath, props: &[Property]) -> Repeater {
    let transform =
        child(props, "ADBE Vector Repeater Transform").map_or(&[][..], Property::children);
    let copies = one(conv, path, props, "ADBE Vector Repeater Copies", 3.0);
    let offset = one(conv, path, props, "ADBE Vector Repeater Offset", 0.0);
    let [ax, ay] = pair(
        conv,
        path,
        transform,
        "ADBE Vector Repeater Anchor",
        [0.0, 0.0],
    );
    let [px, py] = pair(
        conv,
        path,
        transform,
        "ADBE Vector Repeater Position",
        // The file's own default. The 100 After Effects' menu puts in is
        // written into the file, so it arrives anyway.
        [0.0, 0.0],
    );
    Repeater {
        copies,
        offset,
        anchor_x: ax,
        anchor_y: ay,
        position_x: px,
        position_y: py,
        rotation: one(conv, path, transform, "ADBE Vector Repeater Rotation", 0.0),
        scale: one(conv, path, transform, "ADBE Vector Repeater Scale", 100.0),
        start_opacity: one(
            conv,
            path,
            transform,
            "ADBE Vector Repeater Opacity 1",
            100.0,
        ),
        end_opacity: one(
            conv,
            path,
            transform,
            "ADBE Vector Repeater Opacity 2",
            100.0,
        ),
        extra: serde_json::Map::new(),
    }
}

/// The path at the start, and, where anything that shapes or places it moves,
/// its shape on every frame across the keys that move it.
///
/// ponytail: a frame at a time rather than the keys' own curves, which is exact
/// on every frame and a few hundred keys for a long move. Map the keys straight
/// across if a file ever grows too big from it.
fn geometry(conv: &Conv<'_>, drawn: &Drawn) -> (BezierPath, Vec<PathKeyframe>) {
    let at = |t: f64| shape_at(drawn, t);
    let Some((first, last)) = moving(drawn) else {
        return (at(0.0), Vec::new());
    };
    let step = conv
        .tb
        .rate()
        .frame_duration()
        .map_or(Rational::ONE, |d| d.0);
    let mut keys = Vec::new();
    let mut t = first;
    while t <= last {
        keys.push(PathKeyframe {
            time: t,
            path: at(t.to_f64()),
            interp_in: lumit_core::anim::SideInterp::Linear,
            interp_out: lumit_core::anim::SideInterp::Linear,
        });
        let Ok(next) = t.checked_add(step) else { break };
        t = next;
    }
    (at(first.to_f64()), keys)
}

/// The first and last key time of everything that shapes or places the path,
/// or `None` when none of it moves.
fn moving(drawn: &Drawn) -> Option<(Rational, Rational)> {
    let mut props: Vec<&LumProperty> = Vec::new();
    match &drawn.make {
        Make::Rect { size, at, round } => props.extend(size.iter().chain(at).chain([round])),
        Make::Ellipse { size, at } => props.extend(size.iter().chain(at)),
        Make::Star(star) => props.extend(star.at.iter().chain([
            &star.points,
            &star.rotation,
            &star.inner,
            &star.outer,
            &star.inner_round,
            &star.outer_round,
        ])),
        Make::Path { .. } => {}
    }
    let mut span = match &drawn.make {
        Make::Path { keys, .. } => keys.first().zip(keys.last()).map(|(a, b)| (a.time, b.time)),
        _ => None,
    };
    props.extend(drawn.bakes.iter().flat_map(|(_, bake)| bake.props()));
    for p in props {
        span = join(span, key_span(p));
    }
    widen(span, &drawn.chain)
}

fn widen(mut span: Option<(Rational, Rational)>, chain: &[Xf]) -> Option<(Rational, Rational)> {
    for x in chain {
        for p in x
            .anchor
            .iter()
            .chain(&x.position)
            .chain(&x.scale)
            .chain([&x.rotation])
        {
            span = join(span, key_span(p));
        }
    }
    span
}

fn join(
    a: Option<(Rational, Rational)>,
    b: Option<(Rational, Rational)>,
) -> Option<(Rational, Rational)> {
    match (a, b) {
        (Some(a), Some(b)) => Some((a.0.min(b.0), a.1.max(b.1))),
        (a, None) => a,
        (None, b) => b,
    }
}

fn key_span(p: &LumProperty) -> Option<(Rational, Rational)> {
    match &p.animation {
        Animation::Keyframed(keys) => Some((keys.first()?.time, keys.last()?.time)),
        _ => None,
    }
}

/// The path at layer time `t`, in layer space.
fn shape_at(drawn: &Drawn, t: f64) -> BezierPath {
    let v = |p: &LumProperty| p.value_at(t);
    let local = match &drawn.make {
        Make::Rect { size, at, round } => {
            rect(v(&size[0]), v(&size[1]), v(&at[0]), v(&at[1]), v(round))
        }
        Make::Ellipse { size, at } => ellipse(v(&size[0]), v(&size[1]), v(&at[0]), v(&at[1])),
        Make::Star(s) => star(s, t),
        Make::Path { still, keys } => {
            if keys.is_empty() {
                still.clone()
            } else {
                lumit_core::mask::path_at(keys, still, t).into_owned()
            }
        }
    };
    // Out through each group in turn, innermost first, with the operators
    // that sit in a group worked in before its transform carries the path on.
    let mut path = local;
    for depth in 0..=drawn.chain.len() {
        for (at, bake) in &drawn.bakes {
            if *at == depth {
                path = bake.apply(path, t);
            }
        }
        if let Some(group) = drawn.chain.get(depth) {
            path = carry(std::slice::from_ref(group), &path, t);
        }
    }
    path
}

/// A path carried out through the groups in `chain`.
fn carry(chain: &[Xf], path: &BezierPath, t: f64) -> BezierPath {
    BezierPath {
        vertices: path
            .vertices
            .iter()
            .map(|vertex| {
                let pos = place(chain, [vertex.pos.0, vertex.pos.1], t);
                let turn = |d: (f64, f64)| {
                    let to = place(chain, [vertex.pos.0 + d.0, vertex.pos.1 + d.1], t);
                    (to.0 - pos.0, to.1 - pos.1)
                };
                Vertex {
                    pos,
                    tan_in: turn(vertex.tan_in),
                    tan_out: turn(vertex.tan_out),
                }
            })
            .collect(),
        closed: path.closed,
    }
}

/// A point carried out through every group it sits in, innermost first.
fn place(chain: &[Xf], p: [f64; 2], t: f64) -> (f64, f64) {
    let (mut x, mut y) = (p[0], p[1]);
    for g in chain {
        let (ax, ay) = (g.anchor[0].value_at(t), g.anchor[1].value_at(t));
        let (sx, sy) = (
            g.scale[0].value_at(t) / 100.0,
            g.scale[1].value_at(t) / 100.0,
        );
        let (sin, cos) = g.rotation.value_at(t).to_radians().sin_cos();
        let (dx, dy) = ((x - ax) * sx, (y - ay) * sy);
        x = g.position[0].value_at(t) + dx * cos - dy * sin;
        y = g.position[1].value_at(t) + dx * sin + dy * cos;
    }
    (x, y)
}

/// How much the groups scale an outline, as the mean of their two axes.
fn chain_scale(chain: &[Xf]) -> f64 {
    chain
        .iter()
        .map(|g| (g.scale[0].value_at(0.0) + g.scale[1].value_at(0.0)).abs() / 200.0)
        .product()
}

fn xf(conv: &mut Conv<'_>, path: &ItemPath, props: &[Property]) -> Xf {
    Xf {
        anchor: pair(conv, path, props, "ADBE Vector Anchor", [0.0, 0.0]),
        position: pair(conv, path, props, "ADBE Vector Position", [0.0, 0.0]),
        scale: pair(conv, path, props, "ADBE Vector Scale", [100.0, 100.0]),
        rotation: one(conv, path, props, "ADBE Vector Rotation", 0.0),
    }
}

/// One number, at After Effects' own default where the file leaves it out.
fn one(
    conv: &mut Conv<'_>,
    path: &ItemPath,
    props: &[Property],
    name: &str,
    default: f64,
) -> LumProperty {
    child(props, name).map_or(LumProperty::fixed(default), |node| {
        from_node(conv, path, node, 0, default)
    })
}

fn pair(
    conv: &mut Conv<'_>,
    path: &ItemPath,
    props: &[Property],
    name: &str,
    default: [f64; 2],
) -> [LumProperty; 2] {
    [0, 1].map(|axis| {
        child(props, name).map_or(LumProperty::fixed(default[axis]), |node| {
            from_node(conv, path, node, axis, default[axis])
        })
    })
}

/// A number Lumit keeps still, as it starts.
fn still_value(props: &[Property], name: &str, default: f64) -> f64 {
    child(props, name)
        .and_then(|node| starting_value(node).0)
        .unwrap_or(default)
}

fn keyed(props: &[Property], name: &str) -> bool {
    child(props, name).is_some_and(|node| node.keyframes.as_deref().is_some_and(|k| !k.is_empty()))
}

fn colour_of(props: &[Property], name: &str) -> Option<[f64; 3]> {
    let node = child(props, name)?;
    let value = match node.keyframes.as_deref() {
        Some([first, ..]) => first.v.clone()?,
        _ => node.value.clone()?,
    };
    let c: Vec<f64> = (0..3)
        .filter_map(|i| super::props::axis_of(&value, i))
        .collect();
    (c.len() == 3).then(|| [c[0], c[1], c[2]])
}

fn point_of(props: &[Property], name: &str) -> Option<[f64; 2]> {
    let value = child(props, name)?.value.as_ref()?;
    Some([
        super::props::axis_of(value, 0)?,
        super::props::axis_of(value, 1)?,
    ])
}

/// What After Effects calls an operator, whatever this one was renamed, so
/// the rows about one kind read alike.
fn kind_name(node: &Property, fallback: &str) -> String {
    match match_name_of(node) {
        "ADBE Vector Filter - Twist" => "Twist",
        "ADBE Vector Filter - Wiggler" => "Wiggle Transform",
        "ADBE Vector Filter - Roughen" => "Wiggle Paths",
        "ADBE Vector Filter - Offset" => "Offset Paths",
        _ => fallback,
    }
    .to_string()
}

fn not_carried(conv: &mut Conv<'_>, path: &ItemPath, name: &str) {
    conv.report.row(
        path.clone(),
        Outcome::Adjusted,
        Reason::ShapeOperatorNotCarried {
            name: name.to_string(),
        },
    );
}

fn kept_still(conv: &mut Conv<'_>, path: &ItemPath) {
    conv.report
        .row(path.clone(), Outcome::Adjusted, Reason::ShapePaintKeptStill);
}

/// A gradient as the reader hands it over: its colour stops and its opacity
/// stops, each at positions of their own from 0 to 1.
struct Ramp {
    colours: Vec<(f64, Vec<f64>)>,
    alphas: Vec<(f64, Vec<f64>)>,
}

impl Ramp {
    fn of(value: &serde_json::Value) -> Option<Self> {
        let list = |key: &str| -> Vec<(f64, Vec<f64>)> {
            value
                .get(key)
                .and_then(serde_json::Value::as_array)
                .map(|stops| {
                    stops
                        .iter()
                        .filter_map(|stop| {
                            let numbers: Vec<f64> = stop
                                .as_array()?
                                .iter()
                                .filter_map(serde_json::Value::as_f64)
                                .collect();
                            let (at, rest) = numbers.split_first()?;
                            Some((*at, rest.to_vec()))
                        })
                        .collect()
                })
                .unwrap_or_default()
        };
        let mut ramp = Ramp {
            colours: list("colours"),
            alphas: list("alphas"),
        };
        ramp.colours.sort_by(|a, b| a.0.total_cmp(&b.0));
        ramp.alphas.sort_by(|a, b| a.0.total_cmp(&b.0));
        (!ramp.colours.is_empty()).then_some(ramp)
    }

    fn white_to_black() -> Self {
        Ramp {
            colours: vec![(0.0, vec![1.0, 1.0, 1.0]), (1.0, vec![0.0, 0.0, 0.0])],
            alphas: Vec::new(),
        }
    }

    fn colour_at(&self, at: f64) -> LinearColour {
        let c = between(&self.colours, at, &[1.0, 1.0, 1.0]);
        linear_colour(Some(&c))
    }

    fn alpha_at(&self, at: f64) -> f64 {
        between(&self.alphas, at, &[1.0])
            .first()
            .copied()
            .unwrap_or(1.0)
    }

    /// Every place strictly between the two ends where either list has a
    /// stop, as many as Lumit's gradient holds.
    fn inner(&self) -> Vec<f64> {
        let mut at: Vec<f64> = self
            .colours
            .iter()
            .chain(&self.alphas)
            .map(|(at, _)| *at)
            .filter(|at| *at > 0.0 && *at < 1.0)
            .collect();
        at.sort_by(f64::total_cmp);
        at.dedup_by(|a, b| (*a - *b).abs() < 1e-6);
        at.truncate(lumit_core::shape::MAX_GRADIENT_STOPS);
        at
    }
}

/// The stops' values at `at`, straight between the two either side and held
/// past the first and the last.
fn between(stops: &[(f64, Vec<f64>)], at: f64, none: &[f64]) -> Vec<f64> {
    let Some(first) = stops.first() else {
        return none.to_vec();
    };
    if at <= first.0 {
        return first.1.clone();
    }
    for pair in stops.windows(2) {
        let [(a, from), (b, to)] = pair else { continue };
        if at <= *b {
            let u = if b > a { (at - a) / (b - a) } else { 1.0 };
            return from.iter().zip(to).map(|(x, y)| x + (y - x) * u).collect();
        }
    }
    stops
        .last()
        .map_or_else(|| none.to_vec(), |last| last.1.clone())
}

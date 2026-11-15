//! A Lottie shape layer's contents as Lumit's items.
//!
//! # In plain terms
//!
//! In a Lottie a group lists paths, then paints and modifiers, and each paint
//! or modifier acts on every path listed before it in its group, the paths in
//! the groups before it included. Lumit keeps one item per painted path, so
//! the list is walked once: every path is carried into the layer's space
//! through the transforms of the groups it sits in, and each paint makes one
//! item from each path before it. Items higher in a Lottie's list draw on top,
//! where Lumit's list draws its first entry at the bottom, so each group's
//! items are laid down last paint first.
//!
//! - **Several paths under one fill** are one shape with holes in a Lottie.
//!   In Lumit they are a combine run: the even-odd rule is an exclusive
//!   combine, and the non-zero rule a union of the paths wound like the
//!   largest and a subtraction of the ones wound the other way.
//! - **A group that stands still** is worked into the points of its paths.
//! - **A group that moves** can't be, as Lumit's groups have no transform. It
//!   becomes a Shape layer of its own, parented to a Null that carries the
//!   group's transform (and one for each group round it), placed directly
//!   above its layer.
//! - Trim paths, Repeater and Offset paths are Lumit's own modifiers. Round
//!   corners, pucker and bloat, zig zag, twist and anything newer are left
//!   out and named in the report.
//!
//! Runs on the import's thread.

use lumit_core::anim::{Animation, Property, SideInterp};
use lumit_core::mask::{BezierPath, PathKeyframe};
use lumit_core::model::TransformGroup;
use lumit_core::shape::geometry::Affine;
use lumit_core::shape::modifier::{
    ModifierKind, Offset, Repeater, ShapeEntry, ShapeGroup, ShapeModifier, Trim,
};
use lumit_core::shape::primitive::ShapePrimitive;
use lumit_core::shape::ShapeItem;
use lumit_import::report::{ItemPath, Outcome, Reason};
use serde_json::Value;

use super::read::{num, on, text};
use super::{keys, layers, paints, Cx, MAX_DEPTH};
use crate::LottieError;

/// A group that moves, as the layers it becomes.
pub(super) struct Split {
    pub name: String,
    /// A transform for each group it sits in, outermost first, its own last.
    pub nulls: Vec<TransformGroup>,
    pub contents: Vec<ShapeEntry>,
    pub opacity: Property,
    /// The groups inside it that move of their own.
    pub inner: Vec<Split>,
}

/// A path, in the layer's space, and what acts on it.
#[derive(Clone)]
pub(super) struct Drawn {
    pub name: String,
    pub path: BezierPath,
    pub keys: Vec<PathKeyframe>,
    pub primitive: Option<ShapePrimitive>,
    pub mods: Vec<ModifierKind>,
    /// The combine a merge paths after it asked for.
    pub merge: Option<u32>,
}

/// An item and the modifiers that act on it, in the order they apply.
pub(super) struct Art {
    pub item: ShapeItem,
    pub mods: Vec<ModifierKind>,
}

/// Where the walk is: the transform to the layer's space, the groups round
/// it, the opacity they add up to, and how deep it is.
#[derive(Clone)]
pub(super) struct Frame<'v> {
    pub chain: Affine,
    pub outer: Vec<&'v Value>,
    pub fade: f64,
    pub depth: u32,
}

/// What one list hands the list it sits in: the paths it made, for a paint
/// later in that list, and its items, bottom first.
struct Level {
    drawn: Vec<Drawn>,
    blocks: Vec<(usize, Vec<Art>)>,
}

impl Level {
    /// The items, bottom first: a paint later in the list draws under one
    /// before it.
    fn arts(mut self) -> Vec<Art> {
        self.blocks.sort_by_key(|b| std::cmp::Reverse(b.0));
        self.blocks.into_iter().flat_map(|(_, arts)| arts).collect()
    }
}

/// A shape layer's contents, and the groups that moved and became layers.
pub(super) fn contents(
    cx: &mut Cx<'_>,
    path: &ItemPath,
    shapes: &[Value],
) -> Result<(Vec<ShapeEntry>, Vec<Split>), LottieError> {
    let mut splits = Vec::new();
    let nodes: Vec<&Value> = shapes.iter().collect();
    let frame = Frame {
        chain: Affine::IDENTITY,
        outer: Vec::new(),
        fade: 1.0,
        depth: 0,
    };
    let level = walk(cx, path, &nodes, &frame, &mut splits)?;
    Ok((entries(level.arts()), splits))
}

fn walk<'v>(
    cx: &mut Cx<'_>,
    path: &ItemPath,
    nodes: &[&'v Value],
    frame: &Frame<'v>,
    splits: &mut Vec<Split>,
) -> Result<Level, LottieError> {
    if frame.depth > MAX_DEPTH {
        return Err(lumit_ingress::IngressError::Depth { limit: MAX_DEPTH }.into());
    }
    cx.budget.take_items(nodes.len() as u64)?;
    let mut level = Level {
        drawn: Vec::new(),
        blocks: Vec::new(),
    };
    for (at, node) in nodes.iter().copied().enumerate() {
        if on(node.get("hd")) {
            continue;
        }
        let ty = text(node, "ty").unwrap_or_default();
        let name = text(node, "nm")
            .filter(|n| !n.trim().is_empty())
            .unwrap_or_else(|| label(ty))
            .to_owned();
        let here = path.property(&name);
        match ty {
            "gr" => group(cx, &here, &name, node, at, nodes, frame, splits, &mut level)?,
            "sh" | "rc" | "el" | "sr" => {
                if let Some(drawn) = shape(cx, &here, &name, node, ty, frame)? {
                    level.drawn.push(drawn);
                }
            }
            "fl" | "st" | "gf" | "gs" => {
                let arts = paints::paint(cx, &here, &name, node, ty, &level.drawn, &frame.chain)?;
                level.blocks.push((at, arts));
            }
            "tm" | "rp" | "op" => {
                if let Some(kind) = modifier(cx, &here, node, ty, &frame.chain, level.drawn.len())?
                {
                    let arts: usize = level.blocks.iter().map(|(_, a)| a.len()).sum();
                    cx.budget.take_items((level.drawn.len() + arts) as u64)?;
                    for d in &mut level.drawn {
                        d.mods.push(kind.clone());
                    }
                    for (_, arts) in &mut level.blocks {
                        for art in arts.iter_mut() {
                            art.mods.push(kind.clone());
                        }
                    }
                }
            }
            "mm" => {
                // Merge, add, subtract, intersect, exclude: the last four are
                // Lumit's combines; a plain merge is the paths as one shape,
                // which the fill after it makes anyway.
                let code = match num(node.get("mm")).map(|m| m as i64) {
                    Some(2) => Some(1),
                    Some(3) => Some(2),
                    Some(4) => Some(3),
                    Some(5) => Some(4),
                    _ => None,
                };
                for d in &mut level.drawn {
                    d.merge = code;
                }
            }
            "tr" | "no" => {}
            _ => cx.left_out(here, &name),
        }
    }
    Ok(level)
}

/// What a shape item is called when nobody named it.
fn label(ty: &str) -> &'static str {
    match ty {
        "gr" => "Group",
        "sh" => "Path",
        "rc" => "Rectangle",
        "el" => "Ellipse",
        "sr" => "Polystar",
        "fl" => "Fill",
        "st" => "Stroke",
        "gf" => "Gradient fill",
        "gs" => "Gradient stroke",
        "tm" => "Trim paths",
        "rp" => "Repeater",
        "op" => "Offset paths",
        "mm" => "Merge paths",
        "rd" => "Round corners",
        "pb" => "Pucker and bloat",
        "zz" => "Zig zag",
        "tw" => "Twist",
        _ => "Shape item",
    }
}

/// A group: worked into its paths when it stands still, a layer of its own
/// when it moves.
#[allow(clippy::too_many_arguments)]
fn group<'v>(
    cx: &mut Cx<'_>,
    here: &ItemPath,
    name: &str,
    node: &'v Value,
    at: usize,
    siblings: &[&'v Value],
    frame: &Frame<'v>,
    splits: &mut Vec<Split>,
    level: &mut Level,
) -> Result<(), LottieError> {
    let items: Vec<&'v Value> = super::read::list(node, "it").iter().collect();
    let tr = items.iter().copied().find(|i| text(i, "ty") == Some("tr"));
    let t = layers::transform(cx, here, tr)?;
    let keyed = |p: &Property| !matches!(p.animation, Animation::Static(_));
    let moves = [
        &t.anchor_x,
        &t.anchor_y,
        &t.position_x,
        &t.position_y,
        &t.scale_x,
        &t.scale_y,
        &t.rotation,
    ]
    .into_iter()
    .any(keyed);

    if moves {
        // Paints and modifiers after the group in its list reach into it, so
        // they go with it.
        let mut nodes = items.clone();
        nodes.extend(
            siblings
                .iter()
                .skip(at + 1)
                .copied()
                .filter(|s| !matches!(text(s, "ty"), Some("gr" | "sh" | "rc" | "el" | "sr"))),
        );
        let mut inner = Vec::new();
        let sub = Frame {
            chain: Affine::IDENTITY,
            outer: Vec::new(),
            fade: 1.0,
            depth: frame.depth + 1,
        };
        let walked = walk(cx, here, &nodes, &sub, &mut inner)?;
        let mut nulls = Vec::with_capacity(frame.outer.len() + 1);
        for o in &frame.outer {
            nulls.push(layers::transform(cx, here, Some(o))?);
        }
        nulls.push(t.clone());
        let mut opacity = t.opacity;
        affine(&mut opacity, frame.fade, 0.0);
        splits.push(Split {
            name: name.to_owned(),
            nulls,
            contents: entries(walked.arts()),
            opacity,
            inner,
        });
        return Ok(());
    }

    let v = |p: &Property| p.value_at(0.0);
    let (sx, sy) = (v(&t.scale_x) / 100.0, v(&t.scale_y) / 100.0);
    let (sin, cos) = v(&t.rotation).to_radians().sin_cos();
    let own = Affine::translation(-v(&t.anchor_x), -v(&t.anchor_y))
        .then(&Affine([sx, 0.0, 0.0, sy, 0.0, 0.0]))
        .then(&Affine([cos, sin, -sin, cos, 0.0, 0.0]))
        .then(&Affine::translation(v(&t.position_x), v(&t.position_y)));
    let mut outer = frame.outer.clone();
    if let Some(tr) = tr {
        outer.push(tr);
    }
    let still_fade = match t.opacity.animation {
        Animation::Static(o) => o / 100.0,
        _ => 1.0,
    };
    let sub = Frame {
        chain: own.then(&frame.chain),
        outer,
        fade: frame.fade * still_fade,
        depth: frame.depth + 1,
    };
    let walked = walk(cx, here, &items, &sub, splits)?;
    let drawn = walked.drawn.clone();
    let mut arts = walked.arts();
    for art in &mut arts {
        match &t.opacity.animation {
            Animation::Static(o) => affine(&mut art.item.opacity, o / 100.0, 0.0),
            _ => match art.item.opacity.animation {
                Animation::Static(own) => {
                    let mut faded = t.opacity.clone();
                    affine(&mut faded, own / 100.0, 0.0);
                    art.item.opacity = faded;
                }
                _ => cx.row(here.clone(), Outcome::Adjusted, Reason::ShapePaintKeptStill),
            },
        }
    }
    level.drawn.extend(drawn);
    level.blocks.push((at, arts));
    Ok(())
}

/// A path, rectangle, ellipse or polystar, carried into the layer's space.
fn shape(
    cx: &mut Cx<'_>,
    here: &ItemPath,
    name: &str,
    node: &Value,
    ty: &str,
    frame: &Frame<'_>,
) -> Result<Option<Drawn>, LottieError> {
    let chain = frame.chain;
    let drawn = |path, keys, primitive| Drawn {
        name: name.to_owned(),
        path,
        keys,
        primitive,
        mods: Vec::new(),
        merge: None,
    };
    if ty == "sh" {
        let closed = node.get("closed").and_then(Value::as_bool);
        let (still, mut keys) = cx.path(node.get("ks"), closed)?;
        for key in &mut keys {
            carry(&mut key.path, &chain);
        }
        let mut still = still;
        carry(&mut still, &chain);
        return Ok(Some(drawn(still, keys, None)));
    }
    let clock = cx.clock;
    let p = cx.read(here, "Position", node.get("p"))?;
    let [px, py] = keys::pair(p.as_ref(), &clock, [0.0, 0.0]);
    let mut prim = ShapePrimitive {
        kind: 0,
        position_x: px,
        position_y: py,
        width: Property::zero(),
        height: Property::zero(),
        roundness: Property::zero(),
        points: Property::fixed(5.0),
        rotation: Property::zero(),
        outer_radius: Property::zero(),
        inner_radius: Property::zero(),
        outer_roundness: Property::zero(),
        inner_roundness: Property::zero(),
        extra: serde_json::Map::new(),
    };
    match ty {
        "rc" | "el" => {
            let s = cx.read(here, "Size", node.get("s"))?;
            prim.kind = u32::from(ty == "el");
            prim.width = keys::one(s.as_ref(), 0, &clock, 100.0);
            prim.height = keys::one(s.as_ref(), 1, &clock, 100.0);
            if ty == "rc" {
                prim.roundness = cx.one(here, "Roundness", node.get("r"), 0.0)?;
            }
        }
        _ => {
            prim.kind = if num(node.get("sy")) == Some(2.0) {
                2
            } else {
                3
            };
            prim.points = cx.one(here, "Points", node.get("pt"), 5.0)?;
            prim.rotation = cx.one(here, "Rotation", node.get("r"), 0.0)?;
            prim.outer_radius = cx.one(here, "Outer radius", node.get("or"), 100.0)?;
            prim.outer_roundness = cx.one(here, "Outer roundness", node.get("os"), 0.0)?;
            prim.inner_radius = cx.one(here, "Inner radius", node.get("ir"), 50.0)?;
            prim.inner_roundness = cx.one(here, "Inner roundness", node.get("is"), 0.0)?;
        }
    }
    let m = chain.0;
    let fits =
        m[1].abs() < 1e-12 && m[2].abs() < 1e-12 && (m[0] - m[3]).abs() < 1e-12 && m[0] > 0.0;
    if fits {
        // A move and an even scale keep it the shape it is, with its numbers.
        let s = m[0];
        affine(&mut prim.position_x, s, m[4]);
        affine(&mut prim.position_y, s, m[5]);
        for p in [
            &mut prim.width,
            &mut prim.height,
            &mut prim.roundness,
            &mut prim.outer_radius,
            &mut prim.inner_radius,
        ] {
            affine(p, s, 0.0);
        }
        let still = prim.path_at(0.0);
        return Ok(Some(drawn(still, Vec::new(), Some(prim))));
    }
    // Turned or stretched, it is a path: still, or keyed a frame at a time
    // across the keys that move it.
    let mut still = prim.path_at(0.0);
    carry(&mut still, &chain);
    let times: Vec<f64> = [
        &prim.position_x,
        &prim.position_y,
        &prim.width,
        &prim.height,
        &prim.roundness,
        &prim.points,
        &prim.rotation,
        &prim.outer_radius,
        &prim.inner_radius,
        &prim.outer_roundness,
        &prim.inner_roundness,
    ]
    .into_iter()
    .filter_map(|p| match &p.animation {
        Animation::Keyframed(k) => Some(k.iter().map(|k| k.time.to_f64()).collect::<Vec<_>>()),
        _ => None,
    })
    .flatten()
    .collect();
    let (Some(first), Some(last)) = (
        times.iter().copied().reduce(f64::min),
        times.iter().copied().reduce(f64::max),
    ) else {
        return Ok(Some(drawn(still, Vec::new(), None)));
    };
    let (a, b) = ((first * clock.fps).floor(), (last * clock.fps).ceil());
    let count = (b - a).max(0.0) as usize + 1;
    cx.budget.take_items(count as u64)?;
    let keys = (0..count.min(crate::time::MAX_SAMPLES))
        .map(|n| {
            let frame = a + n as f64;
            let mut path = prim.path_at(clock.seconds(frame));
            carry(&mut path, &chain);
            PathKeyframe {
                time: clock.time(frame),
                path,
                interp_in: SideInterp::Linear,
                interp_out: SideInterp::Linear,
            }
        })
        .collect();
    Ok(Some(drawn(still, keys, None)))
}

/// Trim paths, Repeater or Offset paths as Lumit's.
fn modifier(
    cx: &mut Cx<'_>,
    here: &ItemPath,
    node: &Value,
    ty: &str,
    chain: &Affine,
    paths: usize,
) -> Result<Option<ModifierKind>, LottieError> {
    let clock = cx.clock;
    Ok(Some(match ty {
        "tm" => {
            if num(node.get("m")) == Some(2.0) && paths > 1 {
                cx.approximated(here.clone(), "Trim paths");
            }
            ModifierKind::Trim(Trim {
                start: cx.one(here, "Start", node.get("s"), 0.0)?,
                end: cx.one(here, "End", node.get("e"), 100.0)?,
                offset: cx.one(here, "Offset", node.get("o"), 0.0)?,
                extra: serde_json::Map::new(),
            })
        }
        "op" => {
            let mut amount = cx.one(here, "Amount", node.get("a"), 0.0)?;
            affine(&mut amount, chain.scale(), 0.0);
            ModifierKind::Offset(Offset {
                amount,
                extra: serde_json::Map::new(),
            })
        }
        _ => {
            let tr = node.get("tr");
            let field = |key: &str| tr.and_then(|t| t.get(key));
            let anchor = cx.read(here, "Anchor point", field("a"))?;
            let position = cx.read(here, "Position", field("p"))?;
            let scale = cx.read(here, "Scale", field("s"))?;
            if scale.as_ref().is_some_and(|s| {
                s.track.keys.iter().any(|k| {
                    (k.value.first().copied().unwrap_or(100.0)
                        - k.value.get(1).copied().unwrap_or(100.0))
                    .abs()
                        > 1e-9
                })
            }) {
                cx.approximated(here.property("Scale"), "Repeater scale");
            }
            let [anchor_x, anchor_y] = keys::pair(anchor.as_ref(), &clock, [0.0, 0.0]);
            let [position_x, position_y] = keys::pair(position.as_ref(), &clock, [100.0, 0.0]);
            ModifierKind::Repeat(Repeater {
                copies: cx.one(here, "Copies", node.get("c"), 3.0)?,
                offset: cx.one(here, "Offset", node.get("o"), 0.0)?,
                anchor_x,
                anchor_y,
                position_x,
                position_y,
                rotation: cx.one(here, "Rotation", field("r"), 0.0)?,
                scale: keys::one(scale.as_ref(), 0, &clock, 100.0),
                start_opacity: cx.one(here, "Start opacity", field("so"), 100.0)?,
                end_opacity: cx.one(here, "End opacity", field("eo"), 100.0)?,
                extra: serde_json::Map::new(),
            })
        }
    }))
}

/// A path carried through a still transform: points through the whole of it,
/// handles through its turn and scale.
pub(super) fn carry(path: &mut BezierPath, m: &Affine) {
    if *m == Affine::IDENTITY {
        return;
    }
    let linear = Affine([m.0[0], m.0[1], m.0[2], m.0[3], 0.0, 0.0]);
    for v in &mut path.vertices {
        v.pos = m.apply(v.pos);
        v.tan_in = linear.apply(v.tan_in);
        v.tan_out = linear.apply(v.tan_out);
    }
}

/// Every value of a property as `a·v + b`, keys and their speeds with it.
pub(super) fn affine(p: &mut Property, a: f64, b: f64) {
    match &mut p.animation {
        Animation::Static(v) => *v = a * *v + b,
        Animation::Keyframed(keys) => {
            for k in keys {
                k.value = a * k.value + b;
                for side in [&mut k.interp_in, &mut k.interp_out] {
                    if let SideInterp::Bezier { speed, .. } = side {
                        *speed *= a;
                    }
                }
            }
        }
        Animation::Expression(_) => {}
    }
}

/// The items as entries. A run combined into one shape takes its first item's
/// modifiers, after the run so they act on all of it; a run with modifiers
/// above other art goes in a group of its own, so they reach nothing else.
fn entries(arts: Vec<Art>) -> Vec<ShapeEntry> {
    let mut runs: Vec<Vec<Art>> = Vec::new();
    for art in arts {
        match runs.last_mut() {
            Some(run) if art.item.combine != 0 => run.push(art),
            _ => runs.push(vec![art]),
        }
    }
    let mut out = Vec::new();
    for mut run in runs {
        let Some(first) = run.first_mut() else {
            continue;
        };
        let mods = std::mem::take(&mut first.mods);
        let name = first.item.name.clone();
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
                hidden: false,
                own: false,
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

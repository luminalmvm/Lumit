//! Essential Graphics: a composition's controllers become a graphics
//! template's controls, and what a layer placing it set for itself becomes
//! that copy's values.
//!
//! # In plain terms
//!
//! After Effects names a controller's source by where it sits in a layer's
//! property tree. Lumit names a row by the ids of the layer, effect, shape or
//! mask it belongs to, which the import has only just made. Three kinds of
//! source are found by looking: a transform row, a mask row and a Text
//! layer's words are where their names say.
//!
//! An effect's row and a shape's paint are found by **asking the mapping
//! itself**. The source is given another value, the layer's effect or shapes
//! are mapped again, and whichever Lumit row moved is the row the controller
//! sets. A copy's value takes the same road: it is put where the source was,
//! mapped, and read off that row. So a number that changes its unit on the
//! way across changes it for the copy too, and nothing here knows any one
//! effect.
//!
//! A controller whose source Lumit has no row for is a row in the report.
//! Comments are left behind, as Lumit has none.

use std::collections::{BTreeMap, HashMap};

use lumit_core::anim::Property as LumProperty;
use lumit_core::fx::effects::template::{instance_for, is_bare, row_ids, set_row, set_value};
use lumit_core::fx::params::Unit;
use lumit_core::model::{
    Composition, Document, EffectInstance, EffectValue, Layer, LayerKind, TransformProp,
};
use lumit_core::shape::modifier::items as shape_items;
use lumit_core::shape::ShapeItem;
use lumit_core::template::{
    Control, ControlKind, Fit, MaskRow, Picture, ShapeRow, Target, Template, Value,
};
use lumit_core::time::Rational;
use uuid::Uuid;

use crate::capture::{Capture, Comp, Controller, Layer as AeLayer, Property, Step};
use crate::report::{ImportReport, ItemPath, Outcome, Reason};

use super::effects::map_effect;
use super::props::{from_node, group};
use super::time::TimeBase;
use super::{Conv, ItemKind, Items};

/// The match name a layer's Essential Properties sit under.
const OVERRIDES: &str = "ADBE Layer Overrides";

/// How a copy's value for one control is turned into Lumit's.
enum How {
    /// Transform rows. `shift` is how far each Lumit row sits from After
    /// Effects' number: a Shape or Text layer's anchor moves on import.
    Transform { shift: Vec<f64> },
    /// Rows of the effect at `at` in the layer's stack.
    Effect { at: usize, rows: Vec<String> },
    /// A paint row of the shape at `item` among the layer's shapes.
    Shape { item: usize, row: ShapeRow },
    /// A mask's number, as it is.
    Mask,
    /// A Text layer's words.
    Text,
    /// What a layer shows.
    Media,
}

/// One composition's template, with how each of After Effects' controllers
/// reaches it: the control's place in the template and the road its values
/// take. `None` for a controller that did not come across.
struct Made<'a> {
    comp: &'a Comp,
    id: Uuid,
    template: Template,
    roads: Vec<Option<(usize, How)>>,
}

/// Give every composition with Essential Graphics its template, and every
/// layer placing one the values it set.
pub(super) fn apply(
    doc: &mut Document,
    report: &mut ImportReport,
    capture: &Capture,
    items: &Items,
) {
    let mut made: HashMap<i64, Made<'_>> = HashMap::new();
    for comp in &capture.comps {
        let Some((ae_id, (id, ItemKind::Comp))) = comp.id.and_then(|i| Some((i, items.get(i)?)))
        else {
            continue;
        };
        let (Some(essential), Some(built)) = (comp.essential.as_ref(), doc.comp(id)) else {
            continue;
        };
        let one = template(report, comp, &essential.controllers, built, id, items);
        if !one.template.controls.is_empty() {
            made.insert(ae_id, one);
        }
    }
    for one in made.values() {
        if let Some(comp) = doc.comp_mut(one.id) {
            comp.set_template(Some(&one.template));
        }
    }

    for comp in &capture.comps {
        let Some((outer, ItemKind::Comp)) = comp.id.and_then(|i| items.get(i)) else {
            continue;
        };
        for layer in &comp.layers {
            let Some(one) = layer.source_id.and_then(|id| made.get(&id)) else {
                continue;
            };
            let Some(values) = copy(doc, report, comp, layer, one, items) else {
                continue;
            };
            let placed = doc
                .comp_mut(outer)
                .and_then(|c| c.layers.iter_mut().find(|l| ae_index(l) == layer.index));
            if let Some(placed) = placed {
                placed.graph_inputs = Some(values);
            }
        }
    }
}

/// The After Effects stacking index an imported layer was given.
fn ae_index(layer: &Layer) -> Option<u32> {
    let index = layer.extra.get("ae")?.get("index")?.as_u64()?;
    u32::try_from(index).ok()
}

/// What a report row calls a controller's source: its last match name.
fn source_name(controller: &Controller) -> String {
    let last = controller.path.last().and_then(|s| s.match_name.as_deref());
    match (last, controller.kind.as_deref()) {
        (Some(name), _) => name.strip_prefix("ADBE ").unwrap_or(name).to_string(),
        (None, Some(kind)) => kind.to_string(),
        (None, None) => String::new(),
    }
}

/// One composition's controllers as a template.
fn template<'a>(
    report: &mut ImportReport,
    comp: &'a Comp,
    controllers: &[Controller],
    built: &Composition,
    id: Uuid,
    items: &Items,
) -> Made<'a> {
    let mut template = Template::default();
    let mut roads = Vec::with_capacity(controllers.len());
    for controller in controllers {
        // A comment sets nothing, and Lumit has nowhere to keep one.
        if controller.kind.as_deref() == Some("comment") {
            roads.push(None);
            continue;
        }
        let label = controller.name.clone().unwrap_or_default();
        // ponytail: a controller whose source is in a composition nested
        // inside this one is reported. Scripting refuses to make one, so no
        // file was to hand. When one is, it is a `Target::Copy` on the layer
        // placing the nested composition.
        let found = (controller.comp_id.is_none() || controller.comp_id == comp.id)
            .then(|| control(comp, controller, built, items))
            .flatten();
        let Some((mut made, how)) = found else {
            report.row(
                ItemPath::item(&built.name),
                Outcome::Skipped,
                Reason::ControlNotCarried {
                    control: label,
                    row: source_name(controller),
                },
            );
            roads.push(None);
            continue;
        };
        made.id = template.free_id(&label);
        made.label = label;
        made.group = controller.group.clone().filter(|g| !g.is_empty());
        roads.push(Some((template.controls.len(), how)));
        template.controls.push(made);
        report.imported();
    }
    Made {
        comp,
        id,
        template,
        roads,
    }
}

/// The conversion's state standing on one layer of `comp`, as
/// [`super::layers::map_layer`] leaves it for that layer's effects.
fn conv_on<'r>(
    report: &'r mut ImportReport,
    comp: &Comp,
    layer: &AeLayer,
    built: &Composition,
    items: &Items,
) -> Conv<'r> {
    let tb = TimeBase::of_fps(comp.fps).unwrap_or_else(TimeBase::fallback);
    let whole = (
        f64::from(comp.width.unwrap_or(1920).max(1)),
        f64::from(comp.height.unwrap_or(1080).max(1)),
    );
    let layer_ids: BTreeMap<u32, Uuid> = built
        .layers
        .iter()
        .filter_map(|l| Some((ae_index(l)?, l.id)))
        .collect();
    let masks = built
        .layers
        .iter()
        .find(|l| ae_index(l) == layer.index)
        .map(|l| super::fx_colour::mask_refs(&l.masks))
        .unwrap_or_default();
    let mut conv = Conv {
        report,
        tb,
        offset: tb.seconds(layer.start_time.unwrap_or(0.0)),
        size: layer
            .source_id
            .and_then(|id| items.size(id))
            .unwrap_or(whole),
        solids: comp
            .renderer
            .as_deref()
            .is_some_and(|r| !matches!(r, "ADBE Advanced 3d" | "ADBE Standard 3d")),
        span: (Rational::ZERO, Rational::ZERO),
        layer_ids,
        masks,
        self_index: layer.index.unwrap_or(0),
    };
    conv.span = (
        conv.offset_from(tb.seconds(layer.in_point.unwrap_or(0.0))),
        conv.offset_from(tb.seconds(layer.out_point.unwrap_or(0.0))),
    );
    conv
}

/// The node at `path` under `props`, where the capture has one.
fn node_at<'a>(props: &'a [Property], path: &[Step]) -> Option<&'a Property> {
    let (step, rest) = path.split_first()?;
    let node = props.get(place(props, step)?)?;
    if rest.is_empty() {
        Some(node)
    } else {
        node_at(node.children(), rest)
    }
}

/// Where `step` is among `props`: at its index when the node there has its
/// match name, else the first node that has.
fn place(props: &[Property], step: &Step) -> Option<usize> {
    let name = step.match_name.as_deref()?;
    let named = |p: &Property| p.match_name.as_deref() == Some(name);
    step.index
        .map(|i| i as usize)
        .filter(|i| props.get(*i).is_some_and(named))
        .or_else(|| props.iter().position(named))
}

/// `props` with the node at `path` swapped for `leaf`, or added where the
/// capture left it out. `None` when a group on the way is not there.
fn with_leaf(props: &[Property], path: &[Step], leaf: &Property) -> Option<Vec<Property>> {
    let (step, rest) = path.split_first()?;
    let mut out = props.to_vec();
    let at = place(&out, step);
    if rest.is_empty() {
        match at.and_then(|at| out.get_mut(at)) {
            Some(slot) => *slot = leaf.clone(),
            None => out.push(leaf.clone()),
        }
        return Some(out);
    }
    let node = out.get_mut(at?)?;
    node.group = Some(with_leaf(node.children(), rest, leaf)?);
    Some(out)
}

/// A property's still value: its own, or its first key's.
fn still_of(leaf: &Property) -> Option<&serde_json::Value> {
    leaf.value
        .as_ref()
        .or_else(|| leaf.keyframes.as_ref()?.first()?.v.as_ref())
}

/// The source as the capture has it, or a still one of the controller's kind
/// where the project file left a property at its default out.
fn source_leaf(props: &[Property], controller: &Controller) -> Property {
    if let Some(found) = node_at(props, &controller.path) {
        return found.clone();
    }
    let (value_type, value) = match controller.kind.as_deref() {
        Some("colour") => ("colour", serde_json::json!([1.0, 1.0, 1.0, 1.0])),
        Some("point" | "scale") => ("point", serde_json::json!([0.0, 0.0])),
        Some("menu") => ("float", serde_json::json!(1.0)),
        _ => ("float", serde_json::json!(0.0)),
    };
    Property {
        match_name: controller.path.last().and_then(|s| s.match_name.clone()),
        value_type: Some(value_type.to_string()),
        value: Some(value),
        ..Property::default()
    }
}

/// `leaf` holding other values, still: each is one try at moving the row it
/// feeds. A point gets one try an axis, so its two rows can be told apart.
fn nudged(leaf: &Property) -> Vec<Property> {
    let still = |value: serde_json::Value| Property {
        value: Some(value),
        keyframes: None,
        expression: None,
        expression_enabled: None,
        separated: None,
        unreadable: None,
        ..leaf.clone()
    };
    let now = still_of(leaf).cloned().unwrap_or(serde_json::Value::Null);
    let axes: Vec<f64> = match &now {
        serde_json::Value::Array(items) => items.iter().filter_map(|v| v.as_f64()).collect(),
        one => one.as_f64().into_iter().collect(),
    };
    match (leaf.value_type.as_deref(), axes.as_slice()) {
        // Half way round, so no channel lands where it started.
        (Some("colour"), [r, g, b, rest @ ..]) => {
            let mut next = vec![(r + 0.5) % 1.0, (g + 0.5) % 1.0, (b + 0.5) % 1.0];
            next.extend(rest);
            vec![still(serde_json::json!(next))]
        }
        (_, [x, y, rest @ ..]) => {
            let mut across = vec![x + 16.0, *y];
            let mut down = vec![*x, y + 16.0];
            across.extend(rest);
            down.extend(rest);
            vec![
                still(serde_json::json!(across)),
                still(serde_json::json!(down)),
            ]
        }
        // A step up, then a tick flipped: a checkbox reads any number but 0
        // as on.
        (_, [v]) => vec![
            still(serde_json::json!(v + 1.0)),
            still(serde_json::json!(if *v == 0.0 { 1.0 } else { 0.0 })),
        ],
        _ => Vec::new(),
    }
}

/// The effect at `at` in a layer's stack, mapped with nothing reported.
fn effect_at(conv: &mut Conv<'_>, props: &[Property], at: usize) -> Option<EffectInstance> {
    let node = group(props, "ADBE Effect Parade").get(at)?;
    Some(map_effect(conv, &ItemPath::default(), node).instance())
}

/// The rows of `was` that hold something else in `now`.
fn moved(was: &EffectInstance, now: &EffectInstance) -> Vec<String> {
    was.params
        .iter()
        .filter(|p| now.param(&p.id).is_some_and(|value| *value != p.value))
        .map(|p| p.id.clone())
        .collect()
}

/// A shape's paint rows, each as numbers: what tells two mappings apart.
fn paint(item: &ShapeItem) -> [(ShapeRow, Vec<f64>); 4] {
    let colour = |c: &Option<[LumProperty; 4]>| -> Vec<f64> {
        c.iter().flatten().map(|p| p.value_at(0.0)).collect()
    };
    [
        (ShapeRow::Fill, colour(&item.fill)),
        (ShapeRow::Stroke, colour(&item.stroke)),
        (ShapeRow::StrokeWidth, vec![item.stroke_width.value_at(0.0)]),
        (ShapeRow::Opacity, vec![item.opacity.value_at(0.0)]),
    ]
}

/// The control one controller makes, without its id, label and group, and
/// the road a copy's values take. `None` when Lumit has no row for it.
fn control(
    comp: &Comp,
    controller: &Controller,
    built: &Composition,
    items: &Items,
) -> Option<(Control, How)> {
    let ae = comp
        .layers
        .iter()
        .find(|l| l.index.is_some() && l.index == controller.layer_index)?;
    let layer = built.layers.iter().find(|l| ae_index(l) == ae.index)?;
    let props = &ae.properties;
    let make = |kind, target| Control::new(String::new(), String::new(), kind, target);
    let travel = controller
        .range
        .filter(|(min, max)| min.is_finite() && max.is_finite() && max > min);
    let names: Vec<&str> = controller
        .path
        .iter()
        .filter_map(|s| s.match_name.as_deref())
        .collect();
    let mut scratch = ImportReport::default();
    let mut conv = conv_on(&mut scratch, comp, ae, built, items);
    let here = ItemPath::default();

    // Replaceable media names the layer itself, whatever path the file gives.
    if controller.kind.as_deref() == Some("media") {
        if !matches!(
            layer.kind,
            LayerKind::Footage { .. } | LayerKind::Precomp { .. }
        ) {
            return None;
        }
        let (w, h) = ae.source_id.and_then(|id| items.size(id))?;
        let target = Target::Source {
            layer: layer.id,
            frame: (w.round().max(1.0) as u32, h.round().max(1.0) as u32),
        };
        return Some((make(ControlKind::Footage, target), How::Media));
    }

    match names.as_slice() {
        ["ADBE Transform Group", name] => {
            use TransformProp as P;
            let deep = layer.is_3d();
            let (rows, kind, unit) = match *name {
                "ADBE Position" if deep => (
                    vec![P::PositionX, P::PositionY, P::PositionZ],
                    ControlKind::Point,
                    Unit::Px,
                ),
                "ADBE Position" => (
                    vec![P::PositionX, P::PositionY],
                    ControlKind::Point,
                    Unit::Px,
                ),
                "ADBE Position_0" => (vec![P::PositionX], ControlKind::Number, Unit::Px),
                "ADBE Position_1" => (vec![P::PositionY], ControlKind::Number, Unit::Px),
                "ADBE Position_2" => (vec![P::PositionZ], ControlKind::Number, Unit::Px),
                "ADBE Anchor Point" if deep => (
                    vec![P::AnchorX, P::AnchorY, P::AnchorZ],
                    ControlKind::Point,
                    Unit::Px,
                ),
                "ADBE Anchor Point" => (vec![P::AnchorX, P::AnchorY], ControlKind::Point, Unit::Px),
                "ADBE Scale" if deep => (
                    vec![P::ScaleX, P::ScaleY, P::ScaleZ],
                    ControlKind::Point,
                    Unit::Percent,
                ),
                "ADBE Scale" => (
                    vec![P::ScaleX, P::ScaleY],
                    ControlKind::Point,
                    Unit::Percent,
                ),
                "ADBE Rotate Z" => (vec![P::Rotation], ControlKind::Angle, Unit::Raw),
                "ADBE Rotate X" => (vec![P::RotationX], ControlKind::Angle, Unit::Raw),
                "ADBE Rotate Y" => (vec![P::RotationY], ControlKind::Angle, Unit::Raw),
                "ADBE Opacity" => (vec![P::Opacity], ControlKind::Number, Unit::Percent),
                _ => return None,
            };
            // Only a place moves on the way in, and by the same amount at
            // every moment, so the first moment says how far.
            let source = node_at(props, &controller.path)
                .filter(|_| matches!(*name, "ADBE Position" | "ADBE Anchor Point"));
            let mut shift = Vec::with_capacity(rows.len());
            for (axis, row) in rows.iter().enumerate() {
                let theirs = source.map(|node| from_node(&mut conv, &here, node, axis, 0.0));
                let ours = layer.prop(*row)?;
                shift.push(theirs.map_or(0.0, |p| ours.value_at(0.0) - p.value_at(0.0)));
            }
            let mut made = make(
                kind,
                Target::Transform {
                    layer: layer.id,
                    props: rows.clone(),
                },
            );
            made.unit = unit;
            if kind == ControlKind::Point {
                made.axes = rows.len() as u8;
            }
            made.slider = travel.or((*name == "ADBE Opacity").then_some((0.0, 100.0)));
            Some((made, How::Transform { shift }))
        }
        ["ADBE Text Properties", "ADBE Text Document"] => {
            if !matches!(layer.kind, LayerKind::Text { .. }) {
                return None;
            }
            let mut made = make(ControlKind::Text, Target::Text { layer: layer.id });
            // Only the project file says these. A bundle leaves both off, as
            // scripting cannot read them.
            made.font = controller.font == Some(true);
            made.size = controller.size == Some(true);
            Some((made, How::Text))
        }
        ["ADBE Mask Parade", _, name] => {
            let value = match *name {
                "ADBE Mask Opacity" => MaskRow::Opacity,
                "ADBE Mask Feather" => MaskRow::Feather,
                "ADBE Mask Offset" => MaskRow::Expansion,
                _ => return None,
            };
            let all = group(props, "ADBE Mask Parade");
            let at = place(all, controller.path.get(1)?)?;
            // A mask that did not come across moves the ones after it up.
            let mask = layer
                .masks
                .get(at)
                .filter(|_| layer.masks.len() == all.len())?;
            let target = Target::Mask {
                layer: layer.id,
                mask: mask.id,
                value,
            };
            let mut made = make(ControlKind::Number, target);
            made.slider = travel;
            Some((made, How::Mask))
        }
        ["ADBE Effect Parade", ..] => {
            let all = group(props, "ADBE Effect Parade");
            let at = place(all, controller.path.get(1)?)?;
            let effect = layer
                .effects
                .get(at)
                .filter(|_| layer.effects.len() == all.len())?;
            // The source is written in first, so a property the file left at
            // its default starts from the value the tries are measured from.
            let source = source_leaf(props, controller);
            let whole = with_leaf(props, &controller.path, &source)?;
            let was = effect_at(&mut conv, &whole, at)?;
            let mut tries: Vec<Vec<String>> = Vec::new();
            for other in nudged(&source) {
                let changed = with_leaf(props, &controller.path, &other)?;
                tries.push(moved(&was, &effect_at(&mut conv, &changed, at)?));
            }
            let point = matches!(source.value_type.as_deref(), Some("point" | "point3"));
            let rows: Vec<String> = match tries.as_slice() {
                // A point Lumit keeps as two rows: each axis moved its own.
                [x, y] if point && x.len() == 1 && y.len() == 1 && x != y => {
                    vec![x.first()?.clone(), y.first()?.clone()]
                }
                [x, y] if point && x.len() == 1 && x == y => x.clone(),
                _ if point => return None,
                // The first try that moved one row and no other.
                tries => tries.iter().find(|rows| rows.len() == 1)?.clone(),
            };
            let param = |id: &String| Target::Param {
                layer: layer.id,
                effect: effect.id,
                param: id.clone(),
            };
            let mut made = match (rows.as_slice(), was.param(rows.first()?)?) {
                ([x, y], EffectValue::Float(_)) => {
                    let target = Target::ParamPoint {
                        layer: layer.id,
                        effect: effect.id,
                        x: x.clone(),
                        y: y.clone(),
                    };
                    make(ControlKind::Point, target)
                }
                ([id], EffectValue::Point(..)) => make(ControlKind::Point, param(id)),
                ([id], EffectValue::Float(_)) => {
                    let angle = controller.kind.as_deref() == Some("angle")
                        || source.control.as_deref() == Some("angle");
                    let kind = if angle {
                        ControlKind::Angle
                    } else {
                        ControlKind::Number
                    };
                    make(kind, param(id))
                }
                ([id], EffectValue::Colour(_)) => make(ControlKind::Colour, param(id)),
                ([id], EffectValue::Bool(_)) => make(ControlKind::Checkbox, param(id)),
                ([id], EffectValue::Choice(_)) => {
                    let mut menu = make(ControlKind::Menu, param(id));
                    menu.options = if controller.options.is_empty() {
                        source.options.clone().unwrap_or_default()
                    } else {
                        controller.options.clone()
                    };
                    menu
                }
                _ => return None,
            };
            if made.kind == ControlKind::Point {
                made.axes = 2;
            }
            made.slider = travel;
            Some((made, How::Effect { at, rows }))
        }
        ["ADBE Root Vectors Group", ..] => {
            let LayerKind::Shape { contents } = &layer.kind else {
                return None;
            };
            let source = source_leaf(props, controller);
            let whole = with_leaf(props, &controller.path, &source)?;
            let was = super::shapes::contents(&mut conv, &here, &whole);
            let other = nudged(&source).into_iter().next()?;
            let changed = with_leaf(props, &controller.path, &other)?;
            let now = super::shapes::contents(&mut conv, &here, &changed);
            let (was, now) = (shape_items(&was), shape_items(&now));
            let ours = shape_items(contents);
            if was.len() != now.len() || was.len() != ours.len() {
                return None;
            }
            // Every paint row that moved. A fill shared by two paths moves
            // two, and one control names one row.
            // ponytail: paint only. A rectangle's size and corners are not
            // kept as rows by the shape import, so those controllers are
            // reported. Carry them once it keeps a shape's own numbers.
            let mut hits = Vec::new();
            for (item, (a, b)) in was.iter().zip(&now).enumerate() {
                for ((row, before), (_, after)) in paint(a).into_iter().zip(paint(b)) {
                    if before != after {
                        hits.push((item, row));
                    }
                }
            }
            let [(item, row)] = hits.as_slice() else {
                return None;
            };
            let target = Target::Shape {
                layer: layer.id,
                item: ours.get(*item)?.id,
                value: *row,
            };
            let kind = match row {
                ShapeRow::Fill | ShapeRow::Stroke => ControlKind::Colour,
                _ => ControlKind::Number,
            };
            let mut made = make(kind, target);
            made.slider = travel;
            Some((
                made,
                How::Shape {
                    item: *item,
                    row: *row,
                },
            ))
        }
        // ponytail: Time Remap, audio levels and anything else After Effects
        // offers are reported. Each needs an arm here and a road in `set_one`.
        _ => None,
    }
}

/// The leaves under a layer's Essential Properties, groups opened, in order.
fn leaves<'a>(nodes: &'a [Property], out: &mut Vec<&'a Property>) {
    for node in nodes {
        if node.group.is_some() {
            leaves(node.children(), out);
        } else {
            out.push(node);
        }
    }
}

/// What one layer placing a template set for itself, or `None` for a layer
/// that set nothing.
fn copy(
    doc: &Document,
    report: &mut ImportReport,
    comp: &Comp,
    layer: &AeLayer,
    made: &Made<'_>,
    items: &Items,
) -> Option<EffectInstance> {
    let built = doc.comp(made.id)?;
    let controllers = &made.comp.essential.as_ref()?.controllers;
    let mut all = Vec::new();
    leaves(group(&layer.properties, OVERRIDES), &mut all);
    let mut values = instance_for(made.id, &made.template);
    // The copy's own clock: its keys are timed on the composition it sits in.
    let tb = TimeBase::of_fps(comp.fps).unwrap_or_else(TimeBase::fallback);
    let start = tb.seconds(layer.start_time.unwrap_or(0.0));
    let here = ItemPath::item(&layer_comp_name(doc, comp, items))
        .layer(layer.name.as_deref().unwrap_or("(unnamed)"));

    for (place, leaf) in all.iter().enumerate() {
        if leaf.overridden != Some(true) {
            continue;
        }
        // By After Effects' own id where the file has one, else by its place.
        let at = match leaf.controller_id.as_deref() {
            Some(id) => controllers.iter().position(|c| c.id.as_deref() == Some(id)),
            None => Some(place),
        };
        let Some((controller, Some((index, how)))) =
            at.and_then(|at| Some((controllers.get(at)?, made.roads.get(at)?)))
        else {
            continue;
        };
        let Some(control) = made.template.controls.get(*index) else {
            continue;
        };
        let set = set_one(
            &mut values,
            control,
            how,
            controller,
            leaf,
            made,
            built,
            items,
            (tb, start),
        );
        if set.is_none() {
            report.row(
                here.clone(),
                Outcome::Skipped,
                Reason::ControlValueNotCarried {
                    control: control.label.clone(),
                },
            );
        }
    }
    (!is_bare(&values)).then_some(values)
}

/// The name of the composition a capture composition imported as.
fn layer_comp_name(doc: &Document, comp: &Comp, items: &Items) -> String {
    comp.id
        .and_then(|id| items.get(id))
        .and_then(|(id, _)| doc.comp(id))
        .map(|c| c.name.clone())
        .unwrap_or_default()
}

/// Keep `rows` on a copy as its own: the control's value at the copy's first
/// moment, then each row as it moves.
fn keep(values: &mut EffectInstance, control: &Control, rows: Vec<LumProperty>) -> Option<()> {
    let ids = row_ids(control);
    let first: Vec<f64> = rows.iter().map(|p| p.value_at(0.0)).collect();
    let value = match (control.kind, first.as_slice()) {
        (ControlKind::Number | ControlKind::Angle, [one]) => Value::Number(*one),
        (ControlKind::Point, many) if many.len() == ids.len() => Value::Point(many.to_vec()),
        (ControlKind::Colour, [r, g, b, a]) => Value::Colour([*r, *g, *b, *a]),
        _ => return None,
    };
    if !set_value(values, control, &value) {
        return None;
    }
    if control.kind == ControlKind::Colour {
        let channels: [LumProperty; 4] = rows.try_into().ok()?;
        set_row(values, &control.id, EffectValue::Colour(channels));
    } else {
        for (id, row) in ids.iter().zip(rows) {
            set_row(values, id, EffectValue::Float(row));
        }
    }
    Some(())
}

/// One Essential Property a layer set, written into the copy's values.
/// `None` when it could not be read as the control's kind.
#[allow(clippy::too_many_arguments)]
fn set_one(
    values: &mut EffectInstance,
    control: &Control,
    how: &How,
    controller: &Controller,
    leaf: &Property,
    made: &Made<'_>,
    built: &Composition,
    items: &Items,
    (tb, start): (TimeBase, Rational),
) -> Option<()> {
    let ae = made
        .comp
        .layers
        .iter()
        .find(|l| l.index.is_some() && l.index == controller.layer_index)?;
    let props = &ae.properties;
    let mut scratch = ImportReport::default();
    let mut conv = conv_on(&mut scratch, made.comp, ae, built, items);
    conv.tb = tb;
    conv.offset = start;
    let here = ItemPath::default();
    // The copy's value standing where the source stands, so the mapping
    // reads it as that property.
    let stand_in = || {
        let source = source_leaf(props, controller);
        Property {
            value: leaf.value.clone(),
            keyframes: leaf.keyframes.clone(),
            expression: leaf.expression.clone(),
            expression_enabled: leaf.expression_enabled,
            separated: None,
            unreadable: None,
            ..source
        }
    };

    match how {
        How::Transform { shift } => {
            let rows = shift
                .iter()
                .enumerate()
                .map(|(axis, by)| {
                    let mut row = from_node(&mut conv, &here, leaf, axis, 0.0);
                    if *by != 0.0 {
                        super::layers::shift(&mut row, *by);
                    }
                    row
                })
                .collect();
            keep(values, control, rows)
        }
        How::Mask => keep(
            values,
            control,
            vec![from_node(&mut conv, &here, leaf, 0, 0.0)],
        ),
        How::Effect { at, rows } => {
            let changed = with_leaf(props, &controller.path, &stand_in())?;
            let now = effect_at(&mut conv, &changed, *at)?;
            let held: Vec<&EffectValue> = rows.iter().filter_map(|id| now.param(id)).collect();
            match held.as_slice() {
                [EffectValue::Float(p)] => keep(values, control, vec![p.clone()]),
                [EffectValue::Float(x), EffectValue::Float(y)] | [EffectValue::Point(x, y)] => {
                    keep(values, control, vec![x.clone(), y.clone()])
                }
                [EffectValue::Colour(c)] => keep(values, control, c.to_vec()),
                [EffectValue::Bool(on)] => {
                    set_value(values, control, &Value::Tick(*on)).then_some(())
                }
                [EffectValue::Choice(option)] => {
                    set_value(values, control, &Value::Choice(*option)).then_some(())
                }
                _ => None,
            }
        }
        How::Shape { item, row } => {
            let changed = with_leaf(props, &controller.path, &stand_in())?;
            let now = super::shapes::contents(&mut conv, &here, &changed);
            let shape = *shape_items(&now).get(*item)?;
            let rows = match row {
                ShapeRow::Fill => shape.fill.clone()?.to_vec(),
                ShapeRow::Stroke => shape.stroke.clone()?.to_vec(),
                ShapeRow::StrokeWidth => vec![shape.stroke_width.clone()],
                ShapeRow::Opacity => vec![shape.opacity.clone()],
                _ => return None,
            };
            keep(values, control, rows)
        }
        How::Text => {
            let theirs = still_of(leaf)?;
            let words = theirs.get("text")?.as_str()?;
            if !set_value(values, control, &Value::Text(words.to_string())) {
                return None;
            }
            let layer = built.layers.iter().find(|l| ae_index(l) == ae.index)?;
            let LayerKind::Text { document } = &layer.kind else {
                return Some(());
            };
            let word = |key: &str| theirs.get(key).and_then(|v| v.as_str()).unwrap_or_default();
            // A project file names a font by its PostScript name alone.
            let (family, face) = match word("fontFamily") {
                "" => lumit_text::fonts::by_postscript_name(word("font")).unwrap_or_default(),
                family => (family.to_string(), word("fontStyle").to_string()),
            };
            let changed = !family.is_empty()
                && (family != document.style.family || face != document.style.face);
            if control.font && changed {
                // One row keeps a font as its family, a line break, its face.
                let font = EffectValue::Text(format!("{family}\n{face}"));
                set_row(values, &format!("{}_font", control.id), font);
            }
            let size = theirs.get("fontSize").and_then(serde_json::Value::as_f64);
            if let Some(size) = size.filter(|s| control.size && *s != document.size) {
                let size = EffectValue::Float(LumProperty::fixed(size));
                set_row(values, &format!("{}_size", control.id), size);
            }
            Some(())
        }
        How::Media => {
            let theirs = leaf.alternate_id?;
            let (item, kind) = items.get(theirs)?;
            if !matches!(kind, ItemKind::Footage | ItemKind::Comp) {
                return None;
            }
            let Target::Source { frame, .. } = &control.target else {
                return None;
            };
            let size = items.size(theirs).map_or(*frame, |(w, h)| {
                (w.round().max(1.0) as u32, h.round().max(1.0) as u32)
            });
            let picture = Picture {
                item,
                size,
                fit: Fit::default(),
            };
            set_value(values, control, &Value::Picture(picture)).then_some(())
        }
    }
}

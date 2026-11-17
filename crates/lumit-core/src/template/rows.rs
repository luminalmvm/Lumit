//! Reading and writing the row a control names.
//!
//! [`read`] answers what a row holds as the template has it, and [`write`]
//! puts a value there, fixed, in place of whatever animation or expression
//! the row had. A copy's document is made by writing its values
//! ([`super::placed`]); setting a control in the template, recording a menu's
//! options and saving a starting data file all read through here too, so a
//! row means one thing everywhere.
//!
//! A row that has gone (its layer, effect, shape or mask deleted) reads as
//! `None` and takes no value. Nothing here fails.

use uuid::Uuid;

use super::{MaskRow, ShapeRow, Target, TextRow, Value};
use crate::anim::Property;
use crate::model::{
    Composition, Document, EffectInstance, EffectParam, EffectValue, Layer, LayerKind,
    LinearColour, TransformProp,
};
use crate::shape::modifier::ShapeEntry;
use crate::shape::ShapeItem;

/// Whether `props` are the three rows of a colour kept as transform rows: a
/// light's, or the haze's.
fn is_colour(props: &[TransformProp]) -> bool {
    matches!(
        props,
        [
            TransformProp::LightColourR,
            TransformProp::LightColourG,
            TransformProp::LightColourB
        ] | [
            TransformProp::HazeColourR,
            TransformProp::HazeColourG,
            TransformProp::HazeColourB
        ]
    )
}

/// The effect or style `effect` on `layer`.
fn instance(layer: &Layer, effect: Uuid) -> Option<&EffectInstance> {
    layer
        .effects
        .iter()
        .chain(&layer.styles)
        .find(|e| e.id == effect)
}

fn instance_mut(layer: &mut Layer, effect: Uuid) -> Option<&mut EffectInstance> {
    layer
        .effects
        .iter_mut()
        .chain(&mut layer.styles)
        .find(|e| e.id == effect)
}

/// What an effect row holds: the stored value, or the one its declaration
/// starts it at when nothing has been stored yet.
fn param_value(inst: &EffectInstance, param: &str) -> Option<EffectValue> {
    if let Some(stored) = inst.param(param) {
        return Some(stored.clone());
    }
    let def = crate::fx::builtins::def(&inst.effect.match_name)?;
    let declared = def
        .schema()
        .params
        .iter()
        .chain(def.derived(inst))
        .find(|p| p.id == param)?;
    crate::fx::builtins::default_param_value(&declared.kind)
}

fn set_param(inst: &mut EffectInstance, param: &str, value: EffectValue) {
    match inst.params.iter_mut().find(|p| p.id == param) {
        Some(p) => p.value = value,
        None => inst.params.push(EffectParam {
            id: param.to_owned(),
            value,
            extra: serde_json::Map::new(),
        }),
    }
}

/// The shape `item` among `entries`, inside any group.
fn shape(entries: &[ShapeEntry], item: Uuid) -> Option<&ShapeItem> {
    entries.iter().find_map(|entry| match entry {
        ShapeEntry::Item(s) if s.id == item => Some(s),
        ShapeEntry::Group(g) => shape(&g.entries, item),
        _ => None,
    })
}

fn shape_mut(entries: &mut [ShapeEntry], item: Uuid) -> Option<&mut ShapeItem> {
    entries.iter_mut().find_map(|entry| match entry {
        ShapeEntry::Item(s) if s.id == item => Some(s),
        ShapeEntry::Group(g) => shape_mut(&mut g.entries, item),
        _ => None,
    })
}

fn colour_of(c: LinearColour) -> Value {
    Value::Colour(c.0.map(f64::from))
}

fn linear(c: [f64; 4]) -> LinearColour {
    LinearColour(c.map(|v| v as f32))
}

fn keyed_colour(c: &[Property; 4]) -> Value {
    Value::Colour([
        c[0].value_at(0.0),
        c[1].value_at(0.0),
        c[2].value_at(0.0),
        c[3].value_at(0.0),
    ])
}

/// What the row `target` names holds, as the template has it at its first
/// moment. `None` when the row has gone, or holds something no control sets.
#[must_use]
pub fn read(doc: &Document, comp: &Composition, target: &Target) -> Option<Value> {
    let layer = comp.layers.iter().find(|l| l.id == target.layer())?;
    Some(match target {
        Target::Transform { props, .. } => {
            let values: Option<Vec<f64>> = props
                .iter()
                .map(|p| layer.prop(*p).map(|row| row.value_at(0.0)))
                .collect();
            let values = values?;
            match values.as_slice() {
                [one] => Value::Number(*one),
                [r, g, b] if is_colour(props) => Value::Colour([*r, *g, *b, 1.0]),
                _ => Value::Point(values),
            }
        }
        Target::Param { effect, param, .. } => {
            match param_value(instance(layer, *effect)?, param)? {
                EffectValue::Float(p) => Value::Number(p.value_at(0.0)),
                EffectValue::Point(x, y) => Value::Point(vec![x.value_at(0.0), y.value_at(0.0)]),
                EffectValue::Colour(c) => keyed_colour(&c),
                EffectValue::Bool(b) => Value::Tick(b),
                EffectValue::Choice(c) => Value::Choice(c),
                EffectValue::Text(t) => Value::Text(t),
                _ => return None,
            }
        }
        Target::ParamPoint { effect, x, y, .. } => {
            let inst = instance(layer, *effect)?;
            let axis = |id: &str| match param_value(inst, id)? {
                EffectValue::Float(p) => Some(p.value_at(0.0)),
                _ => None,
            };
            Value::Point(vec![axis(x)?, axis(y)?])
        }
        Target::EffectOn { effect, .. } => Value::Tick(instance(layer, *effect)?.enabled),
        Target::Text { .. } => match &layer.kind {
            LayerKind::Text { document } => Value::Text(document.text.clone()),
            _ => return None,
        },
        Target::Font { .. } => match &layer.kind {
            LayerKind::Text { document } => Value::Font {
                family: document.style.family.clone(),
                face: document.style.face.clone(),
            },
            _ => return None,
        },
        Target::TextRow { value, .. } => {
            let LayerKind::Text { document } = &layer.kind else {
                return None;
            };
            match value {
                TextRow::Size => Value::Number(document.size),
                TextRow::Fill => colour_of(document.fill),
                TextRow::Stroke => colour_of(document.style.stroke),
                TextRow::StrokeWidth => Value::Number(document.style.stroke_width),
                TextRow::Tracking => Value::Number(document.style.tracking),
                TextRow::Background => colour_of(document.style.plate),
            }
        }
        Target::Shape { item, value, .. } => {
            let LayerKind::Shape { contents } = &layer.kind else {
                return None;
            };
            let s = shape(contents, *item)?;
            let primitive = s.primitive.as_ref();
            match value {
                ShapeRow::Fill => keyed_colour(s.fill.as_ref()?),
                ShapeRow::Stroke => keyed_colour(s.stroke.as_ref()?),
                ShapeRow::StrokeWidth => Value::Number(s.stroke_width.value_at(0.0)),
                ShapeRow::Opacity => Value::Number(s.opacity.value_at(0.0)),
                ShapeRow::Width => Value::Number(primitive?.width.value_at(0.0)),
                ShapeRow::Height => Value::Number(primitive?.height.value_at(0.0)),
                ShapeRow::Roundness => Value::Number(primitive?.roundness.value_at(0.0)),
                ShapeRow::Rotation => Value::Number(primitive?.rotation.value_at(0.0)),
            }
        }
        Target::Mask { mask, value, .. } => {
            let m = layer.masks.iter().find(|m| m.id == *mask)?;
            Value::Number(match value {
                MaskRow::Opacity => m.opacity.value_at(0.0),
                MaskRow::Feather => m.feather.value_at(0.0),
                MaskRow::Expansion => m.expansion.value_at(0.0),
            })
        }
        Target::Visible { .. } => Value::Tick(layer.switches.visible),
        Target::Volume { .. } => Value::Number(layer.volume_db.value_at(0.0)),
        Target::Source { frame, .. } => {
            let item = match &layer.kind {
                LayerKind::Footage { item } => *item,
                LayerKind::Precomp { comp } => *comp,
                _ => return None,
            };
            Value::Picture(super::Picture {
                item,
                size: *frame,
                fit: super::Fit::default(),
            })
        }
        Target::Copy { control, .. } => {
            let LayerKind::Precomp { comp: nested } = &layer.kind else {
                return None;
            };
            let nested = doc.comp(*nested)?;
            let template = nested.template()?;
            let inner = template.control(control)?;
            if let Some(own) = layer
                .graph_inputs
                .as_ref()
                .and_then(|inst| crate::fx::effects::template::value_of(inst, inner, 0.0, None))
            {
                return Some(own);
            }
            return read(doc, nested, &inner.target);
        }
    })
}

/// Write `value` into the row `target` names in composition `comp`, fixed, in
/// place of any animation or expression the row had. Answers whether the row
/// was there and the value was its kind.
///
/// A picture is the one value that reaches past its layer: it makes a
/// composition of its own for the copy's picture to be fitted in
/// ([`super::placed::fitted`]).
pub fn write(doc: &mut Document, comp: Uuid, target: &Target, value: &Value) -> bool {
    match (target, value) {
        (Target::Source { layer, frame }, Value::Picture(picture)) => {
            return super::placed::fitted(doc, comp, *layer, *frame, picture);
        }
        (Target::Copy { layer, control }, value) => {
            return write_copy(doc, comp, *layer, control, value);
        }
        _ => {}
    }
    let Some(layer) = doc
        .comp_mut(comp)
        .and_then(|c| c.layers.iter_mut().find(|l| l.id == target.layer()))
    else {
        return false;
    };
    write_layer(layer, target, value).is_some()
}

/// A nested copy's control: the value goes into that copy's own values.
fn write_copy(doc: &mut Document, comp: Uuid, layer: Uuid, control: &str, value: &Value) -> bool {
    let nested = doc
        .comp(comp)
        .and_then(|c| c.layers.iter().find(|l| l.id == layer))
        .and_then(|l| match &l.kind {
            LayerKind::Precomp { comp } => Some(*comp),
            _ => None,
        });
    let Some(template) = nested.and_then(|id| doc.comp(id)?.template()) else {
        return false;
    };
    let (Some(nested), Some(inner)) = (nested, template.control(control)) else {
        return false;
    };
    let Some(layer) = doc
        .comp_mut(comp)
        .and_then(|c| c.layers.iter_mut().find(|l| l.id == layer))
    else {
        return false;
    };
    let inst = layer
        .graph_inputs
        .get_or_insert_with(|| crate::fx::effects::template::instance_for(nested, &template));
    crate::fx::effects::template::set_value(inst, inner, value)
}

/// The rows of one layer. `None` when the row is not there or the value is
/// not its kind.
fn write_layer(layer: &mut Layer, target: &Target, value: &Value) -> Option<()> {
    match (target, value) {
        (Target::Transform { props, .. }, value) => {
            let values: Vec<f64> = match value {
                Value::Number(v) => vec![*v],
                Value::Point(p) => p.clone(),
                Value::Colour(c) if is_colour(props) => c[..3].to_vec(),
                _ => return None,
            };
            for (prop, v) in props.iter().zip(values) {
                *layer.prop_mut(*prop)? = Property::fixed(v);
            }
        }
        (Target::Param { effect, param, .. }, value) => {
            let inst = instance_mut(layer, *effect)?;
            let held = param_value(inst, param)?;
            let next = match (held, value) {
                (EffectValue::Float(_), Value::Number(v)) => {
                    EffectValue::Float(Property::fixed(*v))
                }
                (EffectValue::Point(..), Value::Point(p)) => {
                    EffectValue::Point(Property::fixed(*p.first()?), Property::fixed(*p.get(1)?))
                }
                (EffectValue::Colour(_), Value::Colour(c)) => {
                    EffectValue::Colour(c.map(Property::fixed))
                }
                (EffectValue::Bool(_), Value::Tick(b)) => EffectValue::Bool(*b),
                (EffectValue::Choice(_), Value::Choice(c)) => EffectValue::Choice(*c),
                (EffectValue::Text(_), Value::Text(t)) => EffectValue::Text(t.clone()),
                _ => return None,
            };
            set_param(inst, param, next);
        }
        (Target::ParamPoint { effect, x, y, .. }, Value::Point(p)) => {
            let inst = instance_mut(layer, *effect)?;
            set_param(inst, x, EffectValue::Float(Property::fixed(*p.first()?)));
            set_param(inst, y, EffectValue::Float(Property::fixed(*p.get(1)?)));
        }
        (Target::EffectOn { effect, .. }, Value::Tick(on)) => {
            instance_mut(layer, *effect)?.enabled = *on;
        }
        (Target::Text { .. }, Value::Text(words)) => {
            let LayerKind::Text { document } = &mut layer.kind else {
                return None;
            };
            document.text = words.clone();
            document.expression = None;
        }
        (Target::Font { .. }, Value::Font { family, face }) => {
            let LayerKind::Text { document } = &mut layer.kind else {
                return None;
            };
            document.style.family = family.clone();
            document.style.face = face.clone();
        }
        (Target::TextRow { value: row, .. }, value) => {
            let LayerKind::Text { document } = &mut layer.kind else {
                return None;
            };
            match (row, value) {
                (TextRow::Size, Value::Number(v)) => document.size = *v,
                (TextRow::Fill, Value::Colour(c)) => document.fill = linear(*c),
                (TextRow::Stroke, Value::Colour(c)) => document.style.stroke = linear(*c),
                (TextRow::StrokeWidth, Value::Number(v)) => document.style.stroke_width = *v,
                (TextRow::Tracking, Value::Number(v)) => document.style.tracking = *v,
                (TextRow::Background, Value::Colour(c)) => document.style.plate = linear(*c),
                _ => return None,
            }
        }
        (
            Target::Shape {
                item, value: row, ..
            },
            value,
        ) => {
            let LayerKind::Shape { contents } = &mut layer.kind else {
                return None;
            };
            let s = shape_mut(contents, *item)?;
            match (row, value) {
                (ShapeRow::Fill, Value::Colour(c)) => s.fill = Some(c.map(Property::fixed)),
                (ShapeRow::Stroke, Value::Colour(c)) => s.stroke = Some(c.map(Property::fixed)),
                (ShapeRow::StrokeWidth, Value::Number(v)) => s.stroke_width = Property::fixed(*v),
                (ShapeRow::Opacity, Value::Number(v)) => s.opacity = Property::fixed(*v),
                (ShapeRow::Width, Value::Number(v)) => {
                    s.primitive.as_mut()?.width = Property::fixed(*v);
                }
                (ShapeRow::Height, Value::Number(v)) => {
                    s.primitive.as_mut()?.height = Property::fixed(*v);
                }
                (ShapeRow::Roundness, Value::Number(v)) => {
                    s.primitive.as_mut()?.roundness = Property::fixed(*v);
                }
                (ShapeRow::Rotation, Value::Number(v)) => {
                    s.primitive.as_mut()?.rotation = Property::fixed(*v);
                }
                _ => return None,
            }
        }
        (
            Target::Mask {
                mask, value: row, ..
            },
            Value::Number(v),
        ) => {
            let m = layer.masks.iter_mut().find(|m| m.id == *mask)?;
            let fixed = Property::fixed(*v);
            match row {
                MaskRow::Opacity => m.opacity = fixed,
                MaskRow::Feather => m.feather = fixed,
                MaskRow::Expansion => m.expansion = fixed,
            }
        }
        (Target::Visible { .. }, Value::Tick(on)) => layer.switches.visible = *on,
        (Target::Volume { .. }, Value::Number(v)) => layer.volume_db = Property::fixed(*v),
        _ => return None,
    }
    Some(())
}

/// Every row of `layer` a control could name, with what it holds. What
/// recording a menu option compares before and after, so a change to any row
/// is found without knowing which rows to look at.
#[must_use]
pub fn of_layer(doc: &Document, comp: &Composition, layer: &Layer) -> Vec<(Target, Value)> {
    let id = layer.id;
    let mut targets: Vec<Target> = Vec::new();
    // Transform rows, a point's rows together as the Timeline shows them.
    let mut seen: Vec<TransformProp> = Vec::new();
    for prop in TRANSFORM_ROWS {
        if seen.contains(prop) || layer.prop(*prop).is_none() {
            continue;
        }
        let (rows, _) = layer.point_rows(*prop);
        let rows: Vec<TransformProp> = rows
            .into_iter()
            .filter(|p| layer.prop(*p).is_some())
            .collect();
        seen.extend(&rows);
        targets.push(Target::Transform {
            layer: id,
            props: rows,
        });
    }
    for inst in layer.effects.iter().chain(&layer.styles) {
        targets.push(Target::EffectOn {
            layer: id,
            effect: inst.id,
        });
        let declared = crate::fx::builtins::def(&inst.effect.match_name)
            .map(|def| {
                def.schema()
                    .params
                    .iter()
                    .chain(def.derived(inst))
                    .map(|p| p.id)
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        for param in declared {
            targets.push(Target::Param {
                layer: id,
                effect: inst.id,
                param: param.to_owned(),
            });
        }
    }
    match &layer.kind {
        LayerKind::Text { .. } => {
            targets.push(Target::Text { layer: id });
            targets.push(Target::Font { layer: id });
            for value in [
                TextRow::Size,
                TextRow::Fill,
                TextRow::Stroke,
                TextRow::StrokeWidth,
                TextRow::Tracking,
                TextRow::Background,
            ] {
                targets.push(Target::TextRow { layer: id, value });
            }
        }
        LayerKind::Shape { contents } => {
            fn items(entries: &[ShapeEntry], found: &mut Vec<Uuid>) {
                for entry in entries {
                    match entry {
                        ShapeEntry::Item(s) => found.push(s.id),
                        ShapeEntry::Group(g) => items(&g.entries, found),
                        ShapeEntry::Modifier(_) => {}
                    }
                }
            }
            let mut found = Vec::new();
            items(contents, &mut found);
            for item in found {
                for value in [
                    ShapeRow::Fill,
                    ShapeRow::Stroke,
                    ShapeRow::StrokeWidth,
                    ShapeRow::Opacity,
                    ShapeRow::Width,
                    ShapeRow::Height,
                    ShapeRow::Roundness,
                    ShapeRow::Rotation,
                ] {
                    targets.push(Target::Shape {
                        layer: id,
                        item,
                        value,
                    });
                }
            }
        }
        _ => {}
    }
    for mask in &layer.masks {
        for value in [MaskRow::Opacity, MaskRow::Feather, MaskRow::Expansion] {
            targets.push(Target::Mask {
                layer: id,
                mask: mask.id,
                value,
            });
        }
    }
    targets.push(Target::Visible { layer: id });
    targets.push(Target::Volume { layer: id });
    targets
        .into_iter()
        .filter_map(|t| read(doc, comp, &t).map(|v| (t, v)))
        .collect()
}

/// The transform rows a control can name, a point's first row standing for
/// the point.
const TRANSFORM_ROWS: &[TransformProp] = &[
    TransformProp::AnchorX,
    TransformProp::PositionX,
    TransformProp::ScaleX,
    TransformProp::Rotation,
    TransformProp::RotationX,
    TransformProp::RotationY,
    TransformProp::OrientationX,
    TransformProp::Opacity,
    TransformProp::PoiX,
    TransformProp::Zoom,
    TransformProp::FocusDistance,
    TransformProp::Aperture,
    TransformProp::BlurLevel,
    TransformProp::LightIntensity,
    TransformProp::ConeAngle,
    TransformProp::ConeFeather,
];

/// What changed between a composition as it `was` and as it is `now`: every
/// row that holds another value, with the value it holds now. A layer in only
/// one of the two is left out.
#[must_use]
pub fn changed(doc: &Document, was: &Composition, now: &Composition) -> Vec<super::Setting> {
    let mut settings = Vec::new();
    for layer in &now.layers {
        let Some(before) = was.layers.iter().find(|l| l.id == layer.id) else {
            continue;
        };
        if before == layer {
            continue;
        }
        let old = of_layer(doc, was, before);
        for (target, value) in of_layer(doc, now, layer) {
            let same = old.iter().any(|(t, v)| *t == target && *v == value);
            if !same {
                settings.push(super::Setting { target, value });
            }
        }
    }
    settings
}

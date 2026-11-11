use lumit_core::anim::Property;
use lumit_core::edits;
use lumit_core::model::{Composition, Document, LayerKind, ProjectItem};
use lumit_core::ops::Op;
use lumit_core::time::{CompTime, Rational};
use serde::Deserialize;
use serde_json::{json, Value};
use uuid::Uuid;

use super::keys::{animation_json, markers_json, PROPERTIES};
use super::write;
use super::{
    bad, commit, find_comp, find_footage, find_layer, layer_json, layer_now, present, Time,
};
use crate::{Host, ScriptError};

#[derive(Deserialize, Clone, Copy)]
#[serde(rename_all = "lowercase")]
pub(super) enum AddKind {
    Null,
    Camera,
    Solid,
    Footage,
    Precomp,
    Text,
    Shape,
    Adjustment,
    Light,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct AddArgs {
    comp: String,
    kind: AddKind,
    item: Option<String>,
    name: Option<String>,
    index: Option<usize>,
    colour: Option<[f64; 3]>,
    width: Option<u32>,
    height: Option<u32>,
    fields: Option<serde_json::Map<String, Value>>,
}

/// Whether `comp`, or a comp anywhere inside it, holds comp `id`.
pub(super) fn holds(doc: &Document, comp: &Composition, id: Uuid, seen: &mut Vec<Uuid>) -> bool {
    edits::nested_comps(doc, comp).into_iter().any(|inner| {
        if inner == id {
            return true;
        }
        if seen.contains(&inner) {
            return false;
        }
        seen.push(inner);
        doc.comp(inner)
            .is_some_and(|inner| holds(doc, inner, id, seen))
    })
}

pub(super) fn layer_add(host: &dyn Host, a: AddArgs) -> Result<Value, ScriptError> {
    let store = host.store();
    let doc = store.snapshot();
    let comp = find_comp(&doc, &a.comp)?;
    if !matches!(a.kind, AddKind::Solid)
        && (a.colour.is_some() || a.width.is_some() || a.height.is_some())
    {
        return Err(bad("Only a solid takes a colour and a size."));
    }
    let (mut layers, mut ops) = match a.kind {
        AddKind::Null => (vec![edits::null_layer(comp)], Vec::new()),
        AddKind::Camera => (
            vec![edits::fresh_camera(comp, "Camera".into(), None)],
            Vec::new(),
        ),
        AddKind::Solid => {
            let width = a.width.unwrap_or(comp.width);
            let height = a.height.unwrap_or(comp.height);
            if !(1..=16384).contains(&width) || !(1..=16384).contains(&height) {
                return Err(bad("A solid is 1 to 16384 pixels each way."));
            }
            // Lumit's new solid, as white_solid_layer makes one, in the size
            // and colour asked for. Its item takes the layer's name, as in
            // After Effects.
            let (def, name, mut ops) = edits::white_solid_ops(&doc, width, height);
            for op in &mut ops {
                if let Op::AddItem { item, .. } = op {
                    if let ProjectItem::Solid(solid) = item.as_mut() {
                        if let Some(colour) = a.colour {
                            solid.colour = lumit_import::map::linear_colour(Some(&colour));
                        }
                        if let Some(name) = &a.name {
                            solid.name.clone_from(name);
                        }
                    }
                }
            }
            let layer = edits::base_layer(
                name,
                LayerKind::Solid { def },
                comp.duration.0,
                edits::centred_transform(
                    f64::from(width),
                    f64::from(height),
                    comp.width,
                    comp.height,
                ),
            );
            (vec![layer], ops)
        }
        AddKind::Precomp => {
            let key = a
                .item
                .as_deref()
                .ok_or_else(|| bad("A precomp layer needs the comp it places."))?;
            let inner = find_comp(&doc, key)?;
            if inner.id == comp.id {
                return Err(bad("A comp cannot go inside itself."));
            }
            if holds(&doc, inner, comp.id, &mut Vec::new()) {
                return Err(bad(format!(
                    "{} is inside {} already, so {} cannot go in it.",
                    comp.name, inner.name, inner.name
                )));
            }
            (vec![edits::precomp_layer(&doc, comp, inner)], Vec::new())
        }
        AddKind::Footage => {
            let key = a
                .item
                .as_deref()
                .ok_or_else(|| bad("A footage layer needs the footage item."))?;
            let item = find_footage(&doc, key)?;
            (host.footage_layers(comp, item)?, Vec::new())
        }
        AddKind::Text | AddKind::Shape | AddKind::Adjustment | AddKind::Light => {
            (write::fresh(comp, a.kind).into_iter().collect(), Vec::new())
        }
    };
    let first = layers
        .first_mut()
        .ok_or_else(|| ScriptError::Failed("That footage makes no layer.".into()))?;
    if let Some(name) = a.name {
        first.name = name;
    }
    *first = write::with_fields(first.clone(), a.fields)?;
    let id = first.id;
    let index = a.index.unwrap_or(0).min(comp.layers.len());
    for (n, mut layer) in layers.into_iter().enumerate() {
        edits::solo_on_arrival(&mut layer, comp.layers.iter());
        ops.push(Op::AddLayer {
            comp: comp.id,
            index: index + n,
            layer: Box::new(layer),
        });
    }
    commit(store, ops)?;
    layer_now(store, comp.id, id)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct CompositeArgs {
    comp: String,
    layer: String,
    blend: Option<lumit_core::model::BlendMode>,
    #[serde(default, deserialize_with = "present")]
    matte: Option<Option<String>>,
    #[serde(default)]
    luma: bool,
    #[serde(default)]
    inverted: bool,
}

pub(super) fn layer_composite(host: &dyn Host, a: CompositeArgs) -> Result<Value, ScriptError> {
    use lumit_core::model::{LayerInputSource, MatteChannel, MatteRef};

    let store = host.store();
    let doc = store.snapshot();
    let comp = find_comp(&doc, &a.comp)?;
    let layer = find_layer(comp, &a.layer)?;
    let mut ops = Vec::new();
    if let Some(blend) = a.blend {
        ops.push(Op::SetLayerBlend {
            comp: comp.id,
            layer: layer.id,
            blend,
        });
    }
    if let Some(matte) = a.matte {
        let matte = match matte {
            Some(key) => {
                let source = find_layer(comp, &key)?;
                if source.id == layer.id {
                    return Err(bad("A layer cannot be its own matte."));
                }
                Some(MatteRef {
                    layer: source.id,
                    channel: if a.luma {
                        MatteChannel::Luma
                    } else {
                        MatteChannel::Alpha
                    },
                    inverted: a.inverted,
                    source: LayerInputSource::default(),
                })
            }
            None => None,
        };
        ops.push(Op::SetLayerMatte {
            comp: comp.id,
            layer: layer.id,
            matte,
        });
    }
    commit(store, ops)?;
    layer_now(store, comp.id, layer.id)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ShapeArgs {
    comp: String,
    name: Option<String>,
    index: Option<usize>,
    items: Vec<Value>,
}

pub(super) fn shape_add(host: &dyn Host, a: ShapeArgs) -> Result<Value, ScriptError> {
    use lumit_core::model::TransformGroup;
    let store = host.store();
    let doc = store.snapshot();
    let comp = find_comp(&doc, &a.comp)?;
    let mut items = a.items;
    for (n, item) in items.iter_mut().enumerate() {
        let Some(object) = item.as_object_mut() else {
            return Err(bad("Each item is an object."));
        };
        object.entry("id").or_insert_with(|| json!(Uuid::now_v7()));
        object
            .entry("name")
            .or_insert_with(|| json!(format!("Path {}", n + 1)));
    }
    // Read as a file's contents are, so an item written the old way, with
    // its trim or its copies as fields, still lands as entries.
    let contents = lumit_core::shape::modifier::read_entries(Value::Array(items))
        .map_err(|e| bad(format!("Items: {e}.")))?;
    for (n, item) in lumit_core::shape::modifier::items(&contents)
        .iter()
        .enumerate()
    {
        if item.path.vertices.len() < 2 {
            return Err(bad(format!("Item {} needs two vertices or more.", n + 1)));
        }
    }
    // The art is drawn in comp pixels, so the layer sits on the art's own box
    // with its anchor at the corner, as the pen tool's layers do.
    let (x0, y0, _, _) = lumit_core::shape::contents_bounds(&contents, 0.0)
        .ok_or_else(|| bad("The paths draw nothing."))?;
    let layer = edits::base_layer(
        a.name.unwrap_or_else(|| "Shape".into()),
        LayerKind::Shape { contents },
        comp.duration.0,
        TransformGroup {
            anchor_x: Property::fixed(0.0),
            anchor_y: Property::fixed(0.0),
            position_x: Property::fixed(x0),
            position_y: Property::fixed(y0),
            ..TransformGroup::default()
        },
    );
    let id = layer.id;
    let mut layer = layer;
    edits::solo_on_arrival(&mut layer, comp.layers.iter());
    commit(
        store,
        vec![Op::AddLayer {
            comp: comp.id,
            index: a.index.unwrap_or(0).min(comp.layers.len()),
            layer: Box::new(layer),
        }],
    )?;
    layer_now(store, comp.id, id)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct LayerArgs {
    pub(super) comp: String,
    pub(super) layer: String,
}

pub(super) fn layer_get(host: &dyn Host, a: LayerArgs) -> Result<Value, ScriptError> {
    let doc = host.store().snapshot();
    let comp = find_comp(&doc, &a.comp)?;
    let layer = find_layer(comp, &a.layer)?;
    let offset = layer.start_offset.0;
    let mut properties = serde_json::Map::new();
    for (name, channels) in PROPERTIES {
        let values: Option<Vec<Value>> = channels
            .iter()
            .map(|c| layer.prop(*c).map(|p| animation_json(p, offset)))
            .collect();
        let Some(mut values) = values else {
            continue;
        };
        let value = match values.len() {
            1 => values.remove(0),
            _ => Value::Array(values),
        };
        properties.insert((*name).into(), value);
    }
    let mut answer = layer_json(comp, layer);
    answer["properties"] = Value::Object(properties);
    answer["markers"] = markers_json(&layer.markers, comp.frame_rate, offset);
    Ok(answer)
}

#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields)]
struct SwitchArgs {
    visible: Option<bool>,
    audible: Option<bool>,
    solo: Option<bool>,
    locked: Option<bool>,
    shy: Option<bool>,
    guide: Option<bool>,
    motion_blur: Option<bool>,
    fx: Option<bool>,
    accepts_lights: Option<bool>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct SetArgs {
    comp: String,
    layer: String,
    name: Option<String>,
    #[serde(default, deserialize_with = "present")]
    parent: Option<Option<String>>,
    #[serde(default)]
    jump: bool,
    #[serde(rename = "in")]
    in_point: Option<Time>,
    out: Option<Time>,
    start_offset: Option<Time>,
    three_d: Option<bool>,
    index: Option<usize>,
    two_node: Option<bool>,
    after_effects_order: Option<bool>,
    #[serde(default)]
    switches: SwitchArgs,
}

pub(super) fn layer_set(host: &dyn Host, a: SetArgs) -> Result<Value, ScriptError> {
    let store = host.store();
    let doc = store.snapshot();
    let comp = find_comp(&doc, &a.comp)?;
    let found = find_layer(comp, &a.layer)?;
    let (c, layer) = (comp.id, found.id);
    let s = a.switches;
    let mut ops = Vec::new();
    // A locked layer refuses edits, so unlocking comes first and locking last.
    if s.locked == Some(false) {
        ops.push(Op::SetLayerLocked {
            comp: c,
            layer,
            locked: false,
        });
    }
    if let Some(name) = a.name {
        ops.push(Op::RenameLayer {
            comp: c,
            layer,
            name,
        });
    }
    if let Some(parent) = a.parent {
        let parent = match parent {
            Some(key) => Some(find_layer(comp, &key)?.id),
            None => None,
        };
        // Kept where it is at the comp's start, unless asked to jump.
        if a.jump {
            ops.push(Op::SetLayerParent {
                comp: c,
                layer,
                parent,
            });
        } else {
            ops.extend(host.parent_ops(c, layer, parent, CompTime(Rational::ZERO)));
        }
    } else if a.jump {
        return Err(bad("jump goes with a parent."));
    }
    if a.in_point.is_some() || a.out.is_some() || a.start_offset.is_some() {
        let rate = comp.frame_rate;
        let at = |t: Option<Time>, now: CompTime| -> Result<CompTime, ScriptError> {
            t.map_or(Ok(now), |t| t.at(rate).map(CompTime))
        };
        ops.push(Op::SetLayerSpan {
            comp: c,
            layer,
            in_point: at(a.in_point, found.in_point)?,
            out_point: at(a.out, found.out_point)?,
            start_offset: at(a.start_offset, found.start_offset)?,
        });
    }
    if let Some(on) = a
        .after_effects_order
        .filter(|on| *on != found.transform.after_effects_order)
    {
        // Rewritten so the layer keeps its pose, as the Timeline's switch does.
        let rows = found
            .transform
            .rotations_in_order(on)
            .ok_or_else(|| bad("An expression drives a rotation, so the order stays as it is."))?;
        ops.push(Op::SetRotationOrder {
            comp: c,
            layer,
            after_effects: on,
        });
        ops.extend(
            rows.into_iter()
                .map(|(prop, animation)| Op::SetTransformProperty {
                    comp: c,
                    layer,
                    prop,
                    animation,
                }),
        );
    }
    if let Some(three_d) = a.three_d {
        ops.push(Op::SetLayerThreeD {
            comp: c,
            layer,
            three_d,
        });
    }
    if let Some(index) = a.index {
        ops.push(Op::ReorderLayer {
            comp: c,
            layer,
            new_index: index.min(comp.layers.len().saturating_sub(1)),
        });
    }
    if let Some(two_node) = a.two_node {
        let LayerKind::Camera { options, .. } = &found.kind else {
            return Err(bad("Only a camera has a point of interest to aim at."));
        };
        ops.push(Op::SetCameraSettings {
            comp: c,
            layer,
            settings: lumit_core::model::CameraSettings {
                two_node,
                ..options.settings()
            },
        });
    }
    type Switch = fn(Uuid, Uuid, bool) -> Op;
    let switches: [(Option<bool>, Switch); 8] = [
        (s.visible, |comp, layer, visible| Op::SetLayerVisible {
            comp,
            layer,
            visible,
        }),
        (s.audible, |comp, layer, audible| Op::SetLayerAudible {
            comp,
            layer,
            audible,
        }),
        (s.solo, |comp, layer, solo| Op::SetLayerSolo {
            comp,
            layer,
            solo,
        }),
        (s.shy, |comp, layer, shy| Op::SetLayerShy {
            comp,
            layer,
            shy,
        }),
        (s.guide, |comp, layer, guide| Op::SetLayerGuide {
            comp,
            layer,
            guide,
        }),
        (s.motion_blur, |comp, layer, motion_blur| {
            Op::SetLayerMotionBlur {
                comp,
                layer,
                motion_blur,
            }
        }),
        (s.fx, |comp, layer, fx| Op::SetLayerFx { comp, layer, fx }),
        (s.accepts_lights, |comp, layer, accepts_lights| {
            Op::SetLayerAcceptsLights {
                comp,
                layer,
                accepts_lights,
            }
        }),
    ];
    for (on, op) in switches {
        if let Some(on) = on {
            ops.push(op(c, layer, on));
        }
    }
    if s.locked == Some(true) {
        ops.push(Op::SetLayerLocked {
            comp: c,
            layer,
            locked: true,
        });
    }
    commit(store, ops)?;
    layer_now(store, c, layer)
}

pub(super) fn layer_delete(host: &dyn Host, a: LayerArgs) -> Result<Value, ScriptError> {
    let store = host.store();
    let doc = store.snapshot();
    let comp = find_comp(&doc, &a.comp)?;
    let layer = find_layer(comp, &a.layer)?.id;
    commit(
        store,
        vec![Op::RemoveLayer {
            comp: comp.id,
            layer,
        }],
    )?;
    Ok(json!({ "deleted": layer }))
}

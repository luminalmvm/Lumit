//! Layers and comps read and written in the form a .lum file keeps them, so a
//! script reaches everything a project can hold, not only what the other
//! commands name.

use lumit_core::anim::Property;
use lumit_core::edits;
use lumit_core::model::{
    Composition, Layer, LayerKind, LightDef, LinearColour, TextDocument, TransformGroup,
};
use lumit_core::ops::Op;
use serde::Deserialize;
use serde_json::{json, Map, Value};

use super::layers::AddKind;
use super::{bad, commit, find_comp, find_layer, layer_now};
use crate::{Host, ScriptError};

/// Lay `patch` over `onto` as JSON Merge Patch (RFC 7386) does: objects merge
/// key by key, null takes a key away, and anything else replaces. One more
/// rule for a .lum file's choices, the only keys that start with a capital:
/// `{"Keyframed": [...]}` over `{"Static": 1}` replaces it, where merging
/// would leave both.
fn merge(onto: &mut Value, patch: Value) {
    let Value::Object(patch) = patch else {
        *onto = patch;
        return;
    };
    let other_choice = patch.len() == 1
        && patch.keys().all(|k| k.starts_with(char::is_uppercase))
        && onto
            .as_object()
            .is_some_and(|o| patch.keys().all(|k| !o.contains_key(k)));
    if other_choice || !onto.is_object() {
        *onto = json!({});
    }
    if let Value::Object(onto) = onto {
        for (key, value) in patch {
            if value.is_null() {
                onto.remove(&key);
            } else {
                merge(onto.entry(key).or_insert(Value::Null), value);
            }
        }
    }
}

/// `value` with `patch` laid over it, read back as its own type.
fn patched<T: serde::Serialize + serde::de::DeserializeOwned>(
    value: &T,
    patch: Value,
    what: &str,
) -> Result<T, ScriptError> {
    let mut json = serde_json::to_value(value).map_err(|e| ScriptError::Failed(e.to_string()))?;
    merge(&mut json, patch);
    serde_json::from_value(json).map_err(|e| bad(format!("{what}: {e}.")))
}

/// A new text, shape, adjustment or light layer, made as the app's own menu
/// makes it. The other kinds are made where `layer.add` makes them.
pub(super) fn fresh(comp: &Composition, kind: AddKind) -> Option<Layer> {
    let (w, h) = (f64::from(comp.width), f64::from(comp.height));
    let (name, kind, transform) = match kind {
        AddKind::Text => (
            "Text",
            LayerKind::Text {
                document: TextDocument {
                    text: "Text".into(),
                    expression: None,
                    size: 72.0,
                    fill: LinearColour([1.0, 1.0, 1.0, 1.0]),
                    path: None,
                    path_offset: Property::zero(),
                    animators: Vec::new(),
                    style: Box::new(lumit_core::text::TextStyle {
                        kerning: lumit_core::text::Kerning::Metrics,
                        ..Default::default()
                    }),
                    paragraph: Default::default(),
                    extra: Map::new(),
                },
            },
            TransformGroup {
                position_x: Property::fixed(w * 0.5),
                position_y: Property::fixed(h * 0.5),
                ..TransformGroup::default()
            },
        ),
        AddKind::Adjustment => (
            "Adjustment",
            LayerKind::Adjustment,
            edits::centred_transform(w, h, comp.width, comp.height),
        ),
        // Drawn in comp pixels: its fields give the contents.
        AddKind::Shape => (
            "Shape",
            LayerKind::Shape {
                contents: Vec::new(),
            },
            TransformGroup::default(),
        ),
        AddKind::Light => (
            "Point light",
            LayerKind::Light {
                light: Box::new(LightDef {
                    size: [Property::fixed(w * 0.2), Property::fixed(h * 0.2)],
                    two_node: true,
                    point_of_interest: [
                        Property::fixed(w * 0.5),
                        Property::fixed(h * 0.5),
                        Property::zero(),
                    ],
                    ..LightDef::default()
                }),
            },
            TransformGroup {
                position_x: Property::fixed(w * 0.4),
                position_y: Property::fixed(h * 0.35),
                position_z: Property::fixed(-w * 0.35),
                ..TransformGroup::default()
            },
        ),
        _ => return None,
    };
    Some(edits::base_layer(
        name.into(),
        kind,
        comp.duration.0,
        transform,
    ))
}

/// `layer` with `fields` laid over it. A text layer whose fields leave its
/// anchor alone turns about the middle of its words, as the app's do, in the
/// larger box it draws in once it has animators.
pub(super) fn with_fields(
    layer: Layer,
    fields: Option<Map<String, Value>>,
) -> Result<Layer, ScriptError> {
    let Some(fields) = fields else {
        return Ok(centred(layer, false));
    };
    if fields.contains_key("id") {
        return Err(bad("A layer keeps its id."));
    }
    let anchored = fields
        .get("transform")
        .and_then(Value::as_object)
        .is_some_and(|t| t.contains_key("anchor_x") || t.contains_key("anchor_y"));
    let layer = patched(&layer, Value::Object(fields), "Fields")?;
    Ok(centred(layer, anchored))
}

fn centred(mut layer: Layer, anchored: bool) -> Layer {
    if let (false, LayerKind::Text { document }) = (anchored, &layer.kind) {
        let line = lumit_text::layout(
            &lumit_text::TextBlock::of(document, &document.text),
            !document.animators.is_empty(),
        );
        layer.transform.anchor_x = Property::fixed(f64::from(line.width) * 0.5);
        layer.transform.anchor_y = Property::fixed(f64::from(line.height) * 0.5);
    }
    layer
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ReadArgs {
    comp: String,
    layer: String,
}

pub(super) fn layer_read(host: &dyn Host, a: ReadArgs) -> Result<Value, ScriptError> {
    let doc = host.store().snapshot();
    let layer = find_layer(find_comp(&doc, &a.comp)?, &a.layer)?;
    serde_json::to_value(layer).map_err(|e| ScriptError::Failed(e.to_string()))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct LayerWriteArgs {
    comp: String,
    layer: String,
    fields: Map<String, Value>,
}

/// The layer taken out and put back changed, in its own place and with its
/// own id, so mattes, parents and links to it still find it. One undo step.
pub(super) fn layer_write(host: &dyn Host, a: LayerWriteArgs) -> Result<Value, ScriptError> {
    let store = host.store();
    let doc = store.snapshot();
    let comp = find_comp(&doc, &a.comp)?;
    let old = find_layer(comp, &a.layer)?;
    if a.fields.contains_key("id") {
        return Err(bad("A layer keeps its id."));
    }
    let layer = patched(old, Value::Object(a.fields), "Fields")?;
    let index = comp.layers.iter().position(|l| l.id == old.id).unwrap_or(0);
    commit(
        store,
        vec![
            Op::RemoveLayer {
                comp: comp.id,
                layer: old.id,
            },
            Op::AddLayer {
                comp: comp.id,
                index,
                layer: Box::new(layer),
            },
        ],
    )?;
    layer_now(store, comp.id, old.id)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct CompWriteArgs {
    comp: String,
    fields: Map<String, Value>,
}

pub(super) fn comp_write(host: &dyn Host, a: CompWriteArgs) -> Result<Value, ScriptError> {
    let store = host.store();
    let doc = store.snapshot();
    let comp = find_comp(&doc, &a.comp)?;
    let mut ops = Vec::new();
    for (key, patch) in a.fields {
        ops.push(match key.as_str() {
            "motion_blur" => Op::SetCompMotionBlur {
                comp: comp.id,
                motion_blur: patched(&comp.motion_blur, patch, "motion_blur")?,
            },
            "lighting" => Op::SetCompLighting {
                comp: comp.id,
                lighting: patched(&comp.lighting, patch, "lighting")?,
            },
            "graph" => Op::SetCompGraph {
                comp: comp.id,
                graph: Box::new(
                    patched(&comp.graph, patch, "graph")?
                        .ok_or_else(|| bad("A comp's graph cannot be taken away here."))?,
                ),
            },
            _ => {
                return Err(bad(format!(
                    "comp.write sets motion_blur, lighting and graph, not {key}."
                )))
            }
        });
    }
    commit(store, ops)?;
    let doc = store.snapshot();
    let comp = find_comp(&doc, &comp.id.to_string())?;
    Ok(json!({
        "motion_blur": comp.motion_blur,
        "lighting": comp.lighting,
        "graph": comp.graph,
    }))
}

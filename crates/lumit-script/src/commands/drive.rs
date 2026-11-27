//! Driving an effect's control from a driver on the layer's node graph: the
//! chains the app's Drive with menu builds, for a script or MCP.
//!
//! Audio level goes through Remap and Smooth, Beat trigger through Remap,
//! Wiggle through Math so the control wobbles about the number it had, and
//! any other driver straight in. The drivers, their wires and the layer's
//! graph are written as one undo step, and a control that already had a wire
//! is re-routed rather than given a second.

use lumit_core::fx::{self, schema::ParamKind};
use lumit_core::graph::{Edge, InputRef, NodeRef, OutputRef};
use lumit_core::model::{EffectInstance, EffectValue};
use serde::Deserialize;
use serde_json::{json, Map, Value};

use super::effects::{find_effect, set_param, Change, Which};
use super::{bad, commit, find_comp, find_layer};
use crate::{Host, ScriptError};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct DriveArgs {
    comp: String,
    layer: String,
    effect: Which,
    param: String,
    driver: String,
    listen: Option<String>,
    output: Option<String>,
    low: Option<f64>,
    high: Option<f64>,
    in_high: Option<f64>,
    #[serde(default)]
    settings: Map<String, Value>,
    expression: Option<String>,
}

pub(super) fn drive(host: &dyn Host, a: DriveArgs) -> Result<Value, ScriptError> {
    let store = host.store();
    let doc = store.snapshot();
    let comp = find_comp(&doc, &a.comp)?;
    let layer = find_layer(comp, &a.layer)?;
    let target_effect = &layer.effects[find_effect(layer, &a.effect)?];
    let row = fx::builtins::def(&target_effect.effect.match_name)
        .and_then(|def| {
            def.schema()
                .params
                .iter()
                .chain(def.derived(target_effect))
                .find(|p| p.id == a.param || p.label.eq_ignore_ascii_case(&a.param))
        })
        .ok_or_else(|| bad(format!("The effect has no control {}.", a.param)))?;
    let port = row.id.to_owned();
    // The number the control shows now, which Wiggle wobbles about and a
    // sound lifts from.
    let current = target_effect
        .params
        .iter()
        .find(|p| p.id == port)
        .and_then(|p| match &p.value {
            EffectValue::Float(p) => Some(p.value_at(0.0)),
            _ => None,
        })
        .unwrap_or(0.0);
    let listen = match &a.listen {
        Some(key) => Some(find_layer(comp, key)?.id),
        None => None,
    };
    let (w, h) = (f64::from(comp.width), f64::from(comp.height));
    let mint = |name: &str| -> Result<EffectInstance, ScriptError> {
        let mut d = fx::builtins::instantiate_for_raster(name, w, h)
            .ok_or_else(|| bad(format!("Lumit has no driver called {name}.")))?;
        fx::builtins::point_self_layer_params_at(&mut d, layer.id);
        Ok(d)
    };
    let set = |d: &mut EffectInstance, key: &str, v: Value| -> Result<(), ScriptError> {
        set_param(comp, layer, d, key, Change::Value(v))
    };
    let out = |d: &EffectInstance, port: &str| OutputRef::Driver {
        node: d.id,
        port: port.to_owned(),
    };
    let into = |d: &EffectInstance, port: &str| InputRef::Param {
        node: NodeRef::Driver(d.id),
        port: port.to_owned(),
    };
    let target = InputRef::Param {
        node: NodeRef::Effect(target_effect.id),
        port: port.clone(),
    };

    let mut first = mint(&a.driver)?;
    if !fx::builtins::schema(&first.effect.match_name).is_some_and(|s| s.category.is_driver()) {
        return Err(bad(format!("{} is not a driver.", a.driver)));
    }
    let name = first.effect.match_name.clone();
    if let Some(id) = listen {
        set(&mut first, "audio", json!(id.to_string()))?;
        set(&mut first, "source", json!(1))?;
    }
    if let Some(text) = &a.expression {
        fx::drivers::expression::set_source(&mut first, text);
    }
    for (key, v) in &a.settings {
        set(&mut first, key, v.clone())?;
    }
    let mut nodes = Vec::new();
    let mut edges = Vec::new();
    match name.as_str() {
        "audio_level" | "beat_trigger" => {
            let ears = a.output.clone().unwrap_or_else(|| {
                if name == "audio_level" {
                    "amplitude"
                } else {
                    "envelope"
                }
                .to_owned()
            });
            let mut remap = mint("remap")?;
            let in_high = a
                .in_high
                .unwrap_or(if name == "audio_level" { 0.3 } else { 1.0 });
            set(&mut remap, "in_high", json!(in_high))?;
            set(&mut remap, "out_low", json!(a.low.unwrap_or(current)))?;
            set(
                &mut remap,
                "out_high",
                json!(a.high.unwrap_or_else(|| top_of(&row.kind))),
            )?;
            edges.push(Edge {
                from: out(&first, &ears),
                to: into(&remap, "value"),
            });
            if name == "audio_level" {
                let smooth = mint("smooth")?;
                edges.push(Edge {
                    from: out(&remap, "value"),
                    to: into(&smooth, "value"),
                });
                edges.push(Edge {
                    from: out(&smooth, "value"),
                    to: target.clone(),
                });
                nodes.extend([first, remap, smooth]);
            } else {
                edges.push(Edge {
                    from: out(&remap, "value"),
                    to: target.clone(),
                });
                nodes.extend([first, remap]);
            }
        }
        "wiggle" => {
            let mut add = mint("math")?;
            set(&mut add, "operation", json!(0))?;
            set(&mut add, "b", json!(current))?;
            edges.push(Edge {
                from: out(&first, "value"),
                to: into(&add, "a"),
            });
            edges.push(Edge {
                from: out(&add, "value"),
                to: target.clone(),
            });
            nodes.extend([first, add]);
        }
        _ => {
            let port = a.output.clone().unwrap_or_else(|| first_output(&first));
            edges.push(Edge {
                from: out(&first, &port),
                to: target.clone(),
            });
            nodes.push(first);
        }
    }
    let made: Vec<String> = nodes.iter().map(|n| n.id.to_string()).collect();
    let mut graph = layer.graph.clone();
    graph.edges.retain(|e| e.to != target);
    graph.nodes.extend(nodes);
    graph.edges.extend(edges);
    commit(
        store,
        vec![lumit_core::ops::Op::SetLayerGraph {
            comp: comp.id,
            layer: layer.id,
            graph: Box::new(graph),
        }],
    )?;
    Ok(json!({ "drivers": made, "effect": target_effect.id.to_string(), "param": port }))
}

/// The driver's first declared output, which is what a plain wire takes.
fn first_output(d: &EffectInstance) -> String {
    fx::builtins::def(&d.effect.match_name)
        .and_then(|def| def.signature().outputs().first().map(|p| p.id.to_owned()))
        .unwrap_or_else(|| "value".to_owned())
}

/// The top of a control's range, where a sound lifts it to by default.
fn top_of(kind: &ParamKind) -> f64 {
    match kind {
        ParamKind::Float { slider, .. } => slider.1,
        ParamKind::Slider { range, .. } => range.1,
        _ => 100.0,
    }
}

//! The effects on a layer: listing them, adding one by Lumit's name or After
//! Effects', and setting what they do.
//!
//! An After Effects effect, and a control named the way After Effects names
//! it, go through the importer's own table and conversions, so a script's
//! Gaussian Blur lands as the same effect does in an imported project.

use lumit_core::anim::{Animation, Property};
use lumit_core::fx::{self, schema::FxCategory, schema::ParamSchema};
use lumit_core::model::{
    Composition, EffectInstance, EffectNamespace, EffectParam, EffectValue, Layer,
};
use lumit_core::ops::Op;
use lumit_core::store::DocumentStore;
use lumit_core::time::Rational;
use serde::Deserialize;
use serde_json::{json, Value};
use uuid::Uuid;

use super::{
    animation_json, bad, commit, expressed, find_comp, find_layer, keyed, present, underneath,
    Ease, Interp, KeyArgs, LayerArgs, Named, Side, Values,
};
use crate::{Host, ScriptError};

/// The name an effect shows: its own, or its effect's.
fn name_of(e: &EffectInstance) -> &str {
    e.custom_name
        .as_deref()
        .or_else(|| fx::builtins::schema(&e.effect.match_name).map(|s| s.label))
        .unwrap_or(&e.effect.match_name)
}

/// Every control of `e`: its effect's own, then the ones its source declares,
/// such as an LFX effect's shader rows.
fn rows_of(e: &EffectInstance) -> Vec<&'static ParamSchema> {
    fx::builtins::def(&e.effect.match_name).map_or_else(Vec::new, |def| {
        def.schema().params.iter().chain(def.derived(e)).collect()
    })
}

/// The label of control `id` of `e`, or the id when it has none.
fn label_of<'a>(e: &EffectInstance, id: &'a str) -> &'a str {
    rows_of(e)
        .into_iter()
        .find(|p| p.id == id)
        .map_or(id, |p| p.label)
}

/// Store `row` on `e` at its default, as the panel does when a declared row
/// is first written. A button has nothing to store.
fn fill(e: &mut EffectInstance, row: &ParamSchema) {
    if let Some(value) = fx::builtins::default_param_value(&row.kind) {
        e.params.push(EffectParam {
            id: row.id.to_owned(),
            value,
            extra: serde_json::Map::new(),
        });
    }
}

/// One effect as `effect.list` answers it. Key times are on the comp's
/// clock, `offset` ahead of the layer's.
fn effect_json(index: usize, e: &EffectInstance, offset: Rational) -> Value {
    // Rows the source declares and nobody has written yet, at their defaults.
    let mut e = e.clone();
    for row in rows_of(&e) {
        if !e.params.iter().any(|p| p.id == row.id) {
            fill(&mut e, row);
        }
    }
    let e = &e;
    let params: Vec<Value> = e
        .params
        .iter()
        .map(|p| {
            let (kind, value) = match &p.value {
                EffectValue::Float(v) => ("float", animation_json(v, offset)),
                EffectValue::Point(x, y) => (
                    "point",
                    json!([animation_json(x, offset), animation_json(y, offset)]),
                ),
                EffectValue::Colour(c) => (
                    "colour",
                    c.iter().map(|v| animation_json(v, offset)).collect(),
                ),
                EffectValue::Bool(on) => ("bool", json!(on)),
                EffectValue::Choice(n) => ("choice", json!(n)),
                EffectValue::Seed(n) => ("seed", json!(n)),
                _ => ("other", Value::Null),
            };
            json!({ "id": p.id, "label": label_of(e, &p.id), "kind": kind, "value": value })
        })
        .collect();
    json!({
        "id": e.id,
        "index": index,
        "effect": e.effect.match_name,
        "name": name_of(e),
        "enabled": e.enabled,
        // The After Effects effect it stands for, which names its controls.
        "after_effects": lumit_import::map::table::ae_match_name_of(&e.effect.match_name),
        "params": params,
    })
}

/// Effect `id` on layer `layer` of comp `comp`, as the store now holds it.
fn now(store: &DocumentStore, comp: Uuid, layer: Uuid, id: Uuid) -> Result<Value, ScriptError> {
    let doc = store.snapshot();
    let layer = doc
        .comp(comp)
        .and_then(|c| c.layers.iter().find(|l| l.id == layer))
        .ok_or_else(|| ScriptError::Failed("The layer went away.".into()))?;
    let index = layer
        .effects
        .iter()
        .position(|e| e.id == id)
        .ok_or_else(|| ScriptError::Failed("The effect went away.".into()))?;
    Ok(effect_json(
        index,
        &layer.effects[index],
        layer.start_offset.0,
    ))
}

pub(super) fn list(host: &dyn Host, a: LayerArgs) -> Result<Value, ScriptError> {
    let doc = host.store().snapshot();
    let comp = find_comp(&doc, &a.comp)?;
    let layer = find_layer(comp, &a.layer)?;
    let effects: Vec<Value> = layer
        .effects
        .iter()
        .enumerate()
        .map(|(n, e)| effect_json(n, e, layer.start_offset.0))
        .collect();
    Ok(json!({ "effects": effects }))
}

/// The Lumit effect After Effects' effect `ae` imports as, with `controls`
/// set, each in the form an After Effects project is read in. `None` when
/// Lumit has nothing for it.
fn converted(ae: &str, controls: Vec<Value>, size: (f64, f64)) -> Option<EffectInstance> {
    let node: lumit_import::capture::Property =
        serde_json::from_value(json!({ "match_name": ae, "enabled": true, "group": controls }))
            .ok()?;
    let (mut made, _) = lumit_import::map::map_preset("Script", std::slice::from_ref(&node), size);
    made.pop()
        .filter(|e| e.effect.namespace != EffectNamespace::Placeholder)
}

/// A fresh effect called `name` in `comp`'s frame: Lumit's own by its id,
/// After Effects' by its match name or name, or Lumit's by its name. With it
/// comes the After Effects match name it was asked for by, if it was.
fn made(comp: &Composition, name: &str) -> Result<(EffectInstance, Option<String>), ScriptError> {
    let (w, h) = (f64::from(comp.width), f64::from(comp.height));
    let ae =
        lumit_import::map::table::ae_match_name(name).filter(|_| fx::builtins::def(name).is_none());
    let found = if fx::builtins::def(name).is_some() {
        fx::builtins::instantiate_for_raster(name, w, h)
    } else if let Some(ae) = &ae {
        converted(ae, Vec::new(), (w, h))
    } else {
        fx::catalogue::BUILTIN_DEFS
            .iter()
            .find(|d| d.schema().label.eq_ignore_ascii_case(name))
            .and_then(|d| fx::builtins::instantiate_for_raster(d.schema().match_name, w, h))
    };
    let made = found
        .ok_or_else(|| ScriptError::NotFound(format!("Lumit has no effect called {name}.")))?;
    let joins = fx::builtins::schema(&made.effect.match_name)
        .is_some_and(|s| matches!(s.category, FxCategory::Compositing | FxCategory::Drivers));
    if joins || made.effect.match_name == lumit_core::comp_graph::NODE_GRAPH {
        return Err(bad(format!("{name} goes in a node graph, not on a layer.")));
    }
    Ok((made, ae))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct AddArgs {
    comp: String,
    layer: String,
    effect: String,
    name: Option<String>,
}

pub(super) fn add(host: &dyn Host, a: AddArgs) -> Result<Value, ScriptError> {
    let store = host.store();
    let doc = store.snapshot();
    let comp = find_comp(&doc, &a.comp)?;
    let layer = find_layer(comp, &a.layer)?;
    let (mut instance, ae) = made(comp, &a.effect)?;
    // What Lumit's own Add effect does on landing.
    fx::builtins::point_self_layer_params_at(&mut instance, layer.id);
    fx::builtins::spread_on_art(&mut instance, &layer.kind);
    instance.custom_name = a.name;
    let id = instance.id;
    let mut effects = layer.effects.clone();
    effects.push(instance);
    commit(
        store,
        vec![Op::SetLayerEffects {
            comp: comp.id,
            layer: layer.id,
            effects,
        }],
    )?;
    let mut answer = now(store, comp.id, layer.id, id)?;
    if let Some(ae) = ae {
        answer["after_effects"] = json!(ae);
    }
    Ok(answer)
}

/// An effect by its place in the stack, from 0 at the top, or by its id or
/// name.
#[derive(Deserialize)]
#[serde(untagged)]
pub(super) enum Which {
    Place(usize),
    Key(String),
}

fn find_effect(layer: &Layer, which: &Which) -> Result<usize, ScriptError> {
    let effects = &layer.effects;
    let found = match which {
        Which::Place(n) => (*n < effects.len()).then_some(*n),
        Which::Key(key) => {
            let id = Uuid::try_parse(key).ok();
            effects
                .iter()
                .position(|e| Some(e.id) == id)
                .or_else(|| effects.iter().position(|e| name_of(e) == key))
                .or_else(|| {
                    effects
                        .iter()
                        .position(|e| name_of(e).eq_ignore_ascii_case(key))
                })
        }
    };
    found.ok_or_else(|| {
        let which = match which {
            Which::Place(n) => format!("number {n}"),
            Which::Key(key) => format!("called {key}"),
        };
        ScriptError::NotFound(format!("{} has no effect {which}.", layer.name))
    })
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct SetArgs {
    comp: String,
    layer: String,
    effect: Which,
    enabled: Option<bool>,
    #[serde(default, deserialize_with = "present")]
    name: Option<Option<String>>,
    param: Option<String>,
    value: Option<Value>,
    keys: Option<Vec<KeyArgs>>,
    expression: Option<String>,
}

/// What a control is set to.
enum Change {
    Value(Value),
    Keys(Vec<KeyArgs>),
    Expression(String),
}

pub(super) fn set(host: &dyn Host, a: SetArgs) -> Result<Value, ScriptError> {
    let store = host.store();
    let doc = store.snapshot();
    let comp = find_comp(&doc, &a.comp)?;
    let layer = find_layer(comp, &a.layer)?;
    let index = find_effect(layer, &a.effect)?;
    let mut effects = layer.effects.clone();
    let e = effects
        .get_mut(index)
        .ok_or_else(|| ScriptError::Failed("The effect went away.".into()))?;
    if let Some(on) = a.enabled {
        e.enabled = on;
    }
    if let Some(name) = a.name {
        e.custom_name = name;
    }
    let change = match (a.value, a.keys, a.expression) {
        (Some(value), None, None) => Some(Change::Value(value)),
        (None, Some(keys), None) => Some(Change::Keys(keys)),
        (None, None, Some(text)) => Some(Change::Expression(text)),
        (None, None, None) => None,
        _ => return Err(bad("Give a value, keys or an expression, one at a time.")),
    };
    match (a.param, change) {
        (Some(key), Some(change)) => set_param(comp, layer, e, &key, change)?,
        (Some(_), None) => return Err(bad("Give the control a value, keys or an expression.")),
        (None, Some(_)) => return Err(bad("Name the control with param.")),
        (None, None) => {}
    }
    let id = e.id;
    commit(
        store,
        vec![Op::SetLayerEffects {
            comp: comp.id,
            layer: layer.id,
            effects,
        }],
    )?;
    now(store, comp.id, layer.id, id)
}

fn set_param(
    comp: &Composition,
    layer: &Layer,
    e: &mut EffectInstance,
    key: &str,
    change: Change,
) -> Result<(), ScriptError> {
    let rows = rows_of(e);
    let declared = rows
        .iter()
        .find(|p| p.id == key)
        .or_else(|| rows.iter().find(|p| p.label.eq_ignore_ascii_case(key)));
    if let Some(row) = declared.filter(|row| !e.params.iter().any(|p| p.id == row.id)) {
        fill(e, row);
    }
    let own = e.params.iter().position(|p| p.id == key).or_else(|| {
        e.params
            .iter()
            .position(|p| label_of(e, &p.id).eq_ignore_ascii_case(key))
    });
    match own {
        Some(n) => {
            let what = label_of(e, &e.params[n].id).to_owned();
            set_own(comp, layer, &mut e.params[n].value, &what, change)
        }
        None => set_after_effects(comp, layer, e, key, change),
    }
}

/// Set one of Lumit's controls, in Lumit's own units.
fn set_own(
    comp: &Composition,
    layer: &Layer,
    value: &mut EffectValue,
    what: &str,
    change: Change,
) -> Result<(), ScriptError> {
    let channels: Vec<&mut Property> = match value {
        EffectValue::Float(p) => vec![p],
        EffectValue::Point(x, y) => vec![x, y],
        EffectValue::Colour(c) => c.iter_mut().collect(),
        EffectValue::Bool(on) => {
            *on = match change {
                Change::Value(Value::Bool(b)) => b,
                Change::Value(Value::Number(n)) => n.as_f64() != Some(0.0),
                _ => return Err(bad(format!("{what} is a switch, set with true or false."))),
            };
            return Ok(());
        }
        EffectValue::Choice(n) | EffectValue::Seed(n) => {
            *n = match change {
                Change::Value(v) => v.as_u64().and_then(|n| u32::try_from(n).ok()),
                _ => None,
            }
            .ok_or_else(|| bad(format!("{what} takes a whole number.")))?;
            return Ok(());
        }
        _ => return Err(bad(format!("{what} cannot be set from a script."))),
    };
    // A colour may leave its alpha alone.
    let len = channels.len();
    let axes = |values: &Values| -> Result<usize, ScriptError> {
        let n = match values {
            Values::One(_) => 1,
            Values::Many(v) => v.len(),
        };
        if n == len || (len == 4 && n == 3) {
            Ok(n)
        } else if len == 1 {
            Err(bad(format!("{what} takes one number.")))
        } else {
            Err(bad(format!("{what} takes {len} values.")))
        }
    };
    match change {
        Change::Value(v) => {
            let values: Values =
                serde_json::from_value(v).map_err(|_| bad(format!("{what} takes numbers.")))?;
            axes(&values)?;
            for (p, v) in channels.into_iter().zip(values.list()) {
                p.set_animation(Animation::Static(v));
            }
        }
        Change::Keys(keys) => {
            let animations = keyed(keys, comp.frame_rate, layer.start_offset.0, axes)?;
            for (p, animation) in channels.into_iter().zip(animations) {
                p.set_animation(animation);
            }
        }
        Change::Expression(text) => {
            let values: Vec<f64> = channels.iter().map(|p| underneath(p)).collect();
            for (axis, p) in channels.into_iter().enumerate() {
                *p = expressed(p, &text, &values, axis);
            }
        }
    }
    Ok(())
}

/// A side of a key as an After Effects project gives it: its interpolation,
/// and its ease when it has one.
fn ae_side(interp: Interp, side: Option<&Side>) -> (&'static str, Value) {
    let ease = |e: &Ease| json!({ "speed": e.speed, "influence": e.influence });
    match side {
        None => match interp {
            Interp::Hold => ("HOLD", Value::Null),
            Interp::Linear => ("LINEAR", Value::Null),
        },
        Some(Side::Named(Named::Hold)) => ("HOLD", Value::Null),
        Some(Side::Named(Named::Linear)) => ("LINEAR", Value::Null),
        Some(Side::Named(_)) => ("BEZIER", Value::Null),
        Some(Side::Ease(e)) => ("BEZIER", json!([ease(e)])),
        Some(Side::Eases(list)) => ("BEZIER", list.iter().map(ease).collect()),
    }
}

/// `value` a step away, every number one more and a switch the other way.
fn nudged(value: &Value) -> Value {
    match value {
        Value::Number(n) => json!(n.as_f64().unwrap_or(0.0) + 1.0),
        Value::Bool(on) => json!(!on),
        Value::Array(items) => items.iter().map(nudged).collect(),
        other => other.clone(),
    }
}

/// The controls `a` and `b` set differently.
fn differing(a: &EffectInstance, b: &EffectInstance) -> Vec<String> {
    a.params
        .iter()
        .filter(|p| {
            b.params
                .iter()
                .find(|q| q.id == p.id)
                .is_none_or(|q| q.value != p.value)
        })
        .map(|p| p.id.clone())
        .collect()
}

/// Set an After Effects control by its match name, such as
/// `ADBE Gaussian Blur 2-0001`, through the importer's conversion of it. The
/// controls it drives are found by converting the effect with and without
/// it.
fn set_after_effects(
    comp: &Composition,
    layer: &Layer,
    e: &mut EffectInstance,
    key: &str,
    change: Change,
) -> Result<(), ScriptError> {
    let missing = || ScriptError::NotFound(format!("{} has no control called {key}.", name_of(e)));
    // After Effects names an effect's controls after the effect, numbered.
    let ae = key
        .rsplit_once('-')
        .filter(|(_, n)| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()))
        .map(|(ae, _)| ae)
        .ok_or_else(missing)?;
    let size = (f64::from(comp.width), f64::from(comp.height));
    let bare = converted(ae, Vec::new(), size)
        .filter(|bare| bare.effect.match_name == e.effect.match_name)
        .ok_or_else(missing)?;

    let (rate, offset) = (comp.frame_rate, layer.start_offset.0);
    // The keys' own times and sides, which importing rounds or eases, to put
    // back afterwards.
    let mut times = Vec::new();
    let mut sides = Vec::new();
    let mut control = json!({ "match_name": key });
    match change {
        Change::Value(v) => control["value"] = v,
        Change::Keys(keys) => {
            let mut list = Vec::new();
            for k in keys {
                let time = k
                    .time
                    .at(rate)?
                    .checked_sub(offset)
                    .map_err(|_| bad("That time is out of range."))?;
                if times.last().is_some_and(|last| time <= *last) {
                    return Err(bad("Each key needs a time after the one before it."));
                }
                let (in_interp, in_ease) = ae_side(k.interp, k.into.as_ref());
                let (out_interp, out_ease) = ae_side(k.interp, k.out.as_ref());
                list.push(json!({
                    "t": time.to_f64(),
                    "v": match &k.value {
                        Values::One(v) => json!(v),
                        Values::Many(v) => json!(v),
                    },
                    "in_interp": in_interp,
                    "out_interp": out_interp,
                    "in_ease": in_ease,
                    "out_ease": out_ease,
                }));
                times.push(time);
                sides.push((k.into, k.out));
            }
            control["keyframes"] = Value::Array(list);
        }
        Change::Expression(text) => {
            control["expression"] = json!(text);
            control["expression_enabled"] = json!(true);
        }
    }
    let set = converted(ae, vec![control.clone()], size).ok_or_else(missing)?;
    let mut driven = differing(&set, &bare);
    if driven.is_empty() {
        // Set to where After Effects starts it, so nothing moved. A step
        // away shows which controls it drives.
        if let Some(v) = control.get_mut("value") {
            *v = nudged(v);
        }
        if let Some(Value::Array(keys)) = control.get_mut("keyframes") {
            for k in keys {
                k["v"] = nudged(&k["v"]);
            }
        }
        let away = converted(ae, vec![control], size).ok_or_else(missing)?;
        driven = differing(&away, &bare);
    }
    if driven.is_empty() {
        return Err(missing());
    }
    for id in driven {
        let (Some(from), Some(to)) = (
            set.params.iter().find(|p| p.id == id),
            e.params.iter_mut().find(|p| p.id == id),
        ) else {
            continue;
        };
        to.value = from.value.clone();
        let channels: Vec<&mut Property> = match &mut to.value {
            EffectValue::Float(p) => vec![p],
            EffectValue::Point(x, y) => vec![x, y],
            EffectValue::Colour(c) => c.iter_mut().collect(),
            _ => Vec::new(),
        };
        for (axis, p) in channels.into_iter().enumerate() {
            let Animation::Keyframed(keys) = &mut p.animation else {
                continue;
            };
            if keys.len() != times.len() {
                continue;
            }
            for (key, (time, (into, out))) in keys.iter_mut().zip(times.iter().zip(&sides)) {
                key.time = *time;
                // Importing reads an automatic side as an ease, which a
                // script asked for by name.
                if let Some(side @ Side::Named(Named::Auto | Named::Clamped)) = into {
                    key.interp_in = side.on(axis);
                }
                if let Some(side @ Side::Named(Named::Auto | Named::Clamped)) = out {
                    key.interp_out = side.on(axis);
                }
            }
        }
    }
    Ok(())
}

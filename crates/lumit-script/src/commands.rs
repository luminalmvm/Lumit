//! The commands themselves: the table, and one function each.

mod effects;

use std::path::{Path, PathBuf};

use lumit_core::anim::{Animation, Keyframe, Property, SideInterp, TangentMode};
use lumit_core::edits;
use lumit_core::expression::{Language, Slot};
use lumit_core::markers::{Marker, MarkerEdit, MarkerKind};
use lumit_core::model::{Composition, Document, Layer, LayerKind, ProjectItem, TransformProp};
use lumit_core::ops::Op;
use lumit_core::time::{CompTime, FrameRate, Rational};
use lumit_core::DocumentStore;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Deserializer};
use serde_json::{json, Value};
use uuid::Uuid;

use crate::reach::{self, Reach};
use crate::{Command, Host, ScriptError};

// Pieces of schema the commands share. Macros rather than consts so
// `concat!` can splice them into each command's schema text.
macro_rules! none {
    () => {
        r#"{"type":"object","properties":{},"additionalProperties":false}"#
    };
}
macro_rules! comp {
    () => {
        r#"{"type":"string","description":"The comp's id or name"}"#
    };
}
macro_rules! layer {
    () => {
        r#"{"type":"string","description":"The layer's id, or its name in the comp"}"#
    };
}
macro_rules! time {
    () => {
        r#"{"oneOf":[{"type":"integer","description":"A frame on the comp's rate"},{"type":"object","properties":{"num":{"type":"integer"},"den":{"type":"integer","minimum":1}},"required":["num","den"],"additionalProperties":false,"description":"Exact seconds"}]}"#
    };
}
// How a key meets the curve on one side, and an ease as After Effects gives
// one.
macro_rules! ease {
    () => {
        r#"{"type":"object","properties":{"speed":{"type":"number","description":"Units a second"},"influence":{"type":"number","minimum":0.1,"maximum":100,"description":"Per cent of the span"}},"additionalProperties":false}"#
    };
}
macro_rules! side {
    () => {
        concat!(
            r#"{"description":"Linear, hold, auto or clamped, or an ease, or a list with an ease for each value","oneOf":[{"enum":["linear","hold","auto","clamped"]},"#,
            ease!(),
            r#",{"type":"array","minItems":1,"items":"#,
            ease!(),
            r#"}]}"#
        )
    };
}
// One key whose value is `$value`.
macro_rules! key {
    ($value:literal) => {
        concat!(
            r#"{"type":"object","properties":{"time":"#,
            time!(),
            r#","value":"#,
            $value,
            r#","interp":{"enum":["linear","hold"]},"in":"#,
            side!(),
            r#","out":"#,
            side!(),
            r#"},"required":["time","value"],"additionalProperties":false}"#
        )
    };
}
macro_rules! expression {
    () => {
        r#"{"type":"string","description":"A JavaScript expression, as After Effects writes them"}"#
    };
}
// One `layer.keys` form: the properties named, and the value each takes.
macro_rules! keys_form {
    ($names:literal, $value:literal) => {
        concat!(
            r#"{"properties":{"comp":"#,
            comp!(),
            r#","layer":"#,
            layer!(),
            r#","property":{"enum":"#,
            $names,
            r#"},"value":"#,
            $value,
            r#","keys":{"type":"array","minItems":1,"items":"#,
            key!($value),
            r#"},"expression":"#,
            expression!(),
            r#"},"additionalProperties":false,"oneOf":[{"required":["value"]},{"required":["keys"]},{"required":["expression"]}]}"#
        )
    };
}
macro_rules! rate {
    () => {
        r#"{"description":"1 to 999 frames a second","oneOf":[{"type":"integer","minimum":1,"maximum":999},{"type":"object","properties":{"num":{"type":"integer","minimum":1},"den":{"type":"integer","minimum":1}},"required":["num","den"],"additionalProperties":false}]}"#
    };
}
macro_rules! comp_settings {
    () => {
        concat!(
            r#""name":{"type":"string"},"width":{"type":"integer","minimum":16,"maximum":16384},"height":{"type":"integer","minimum":16,"maximum":16384},"frame_rate":"#,
            rate!(),
            r#","duration":{"description":"At most 3 hours","allOf":["#,
            time!(),
            r#"]},"background":{"type":"array","items":{"type":"number"},"minItems":4,"maxItems":4,"description":"Scene-linear RGBA"}"#
        )
    };
}

/// Every command, in the order a list of them reads best.
pub const COMMANDS: &[Command] = &[
    Command {
        name: "project.info",
        summary: "Where the project is saved, how much it holds, and whether undo and redo have anything to do",
        schema: none!(),
    },
    Command {
        name: "project.save",
        summary: "Save the project, where it was last saved or to a new path",
        schema: r#"{"type":"object","properties":{"path":{"type":"string"}},"additionalProperties":false}"#,
    },
    Command {
        name: "undo",
        summary: "Undo the last step",
        schema: none!(),
    },
    Command {
        name: "redo",
        summary: "Redo the last undone step",
        schema: none!(),
    },
    Command {
        name: "comp.list",
        summary: "Every comp in the project, with its settings",
        schema: none!(),
    },
    Command {
        name: "comp.get",
        summary: "One comp's settings and its layers",
        schema: concat!(
            r#"{"type":"object","properties":{"comp":"#,
            comp!(),
            r#"},"required":["comp"],"additionalProperties":false}"#
        ),
    },
    Command {
        name: "comp.new",
        summary: "Make a comp, 1920x1080 at 60 fps for 30 seconds unless told otherwise",
        schema: concat!(
            r#"{"type":"object","properties":{"#,
            comp_settings!(),
            r#","folder":{"type":["string","null"],"description":"The folder to file it in, by id or name, or null for the top of the panel. Left out, it goes in the Compositions folder"}},"additionalProperties":false}"#
        ),
    },
    Command {
        name: "comp.set",
        summary: "Change a comp's name, size, frame rate, duration or background",
        schema: concat!(
            r#"{"type":"object","properties":{"comp":"#,
            comp!(),
            ",",
            comp_settings!(),
            r#"},"required":["comp"],"additionalProperties":false}"#
        ),
    },
    Command {
        name: "comp.precompose",
        summary: "Move layers into a new comp, and put that comp in their place as one layer",
        schema: concat!(
            r#"{"type":"object","properties":{"comp":"#,
            comp!(),
            r#","layers":{"type":"array","items":{"type":"string"},"minItems":1,"description":"The layers' ids, or their names in the comp"},"name":{"type":"string","description":"The new comp's name. Pre-comp and a number if left out"},"leave_attributes":{"type":"boolean","description":"For one layer: leave its transform, effects and masks on the new layer, and move only its source"}},"required":["comp","layers"],"additionalProperties":false}"#
        ),
    },
    Command {
        name: "comp.open",
        summary: "Show a comp in the Viewer",
        schema: concat!(
            r#"{"type":"object","properties":{"comp":"#,
            comp!(),
            r#"},"required":["comp"],"additionalProperties":false}"#
        ),
    },
    Command {
        name: "comp.active",
        summary: "The comp open in the Viewer, if there is one",
        schema: none!(),
    },
    Command {
        name: "comp.frame",
        summary: "Render a frame of a comp to a PNG, and answer its path, width and height",
        schema: concat!(
            r#"{"type":"object","properties":{"comp":"#,
            comp!(),
            r#","time":{"description":"The frame, 0 when left out","allOf":["#,
            time!(),
            r#"]},"scale":{"type":"number","exclusiveMinimum":0,"maximum":1,"description":"Of the comp's size, 1 when left out"},"out":{"type":"string","description":"The .png file to write. Without it, a numbered file beside the project"}},"required":["comp"],"additionalProperties":false}"#
        ),
    },
    Command {
        name: "item.list",
        summary: "Every item in the project panel: comps, footage, folders and solids, with the folder each is in",
        schema: none!(),
    },
    Command {
        name: "item.set",
        summary: "Rename an item, file it in a folder, or set the rate a footage item plays at",
        schema: concat!(
            r#"{"type":"object","properties":{"item":{"type":"string","description":"The item's id or name"},"name":{"type":"string"},"folder":{"type":["string","null"],"description":"The folder's id or name, or null for none"},"frame_rate":{"anyOf":["#,
            rate!(),
            r#",{"type":"null","description":"A video's own rate"}]}},"required":["item"],"additionalProperties":false}"#
        ),
    },
    Command {
        name: "folder.new",
        summary: "Make a folder in the project panel",
        schema: r#"{"type":"object","properties":{"name":{"type":"string"},"parent":{"type":"string","description":"The folder to make it in, by id or name"}},"additionalProperties":false}"#,
    },
    Command {
        name: "layer.add",
        summary: "Add a null, camera, solid, footage or precomp layer to a comp",
        schema: concat!(
            r#"{"type":"object","properties":{"comp":"#,
            comp!(),
            r#","kind":{"enum":["null","camera","solid","footage","precomp"]},"item":{"type":"string","description":"For footage: the footage item's id or name. For a precomp: the comp's"},"name":{"type":"string"},"index":{"type":"integer","minimum":0,"description":"Where in the stack, 0 is the top"},"colour":{"type":"array","items":{"type":"number","minimum":0,"maximum":1},"minItems":3,"maxItems":3,"description":"For a solid: its colour as a colour picker shows it, 0 to 1. White if left out"},"width":{"type":"integer","minimum":1,"maximum":16384,"description":"For a solid, the comp's if left out"},"height":{"type":"integer","minimum":1,"maximum":16384}},"required":["comp","kind"],"additionalProperties":false}"#
        ),
    },
    Command {
        name: "shape.add",
        summary: "Add a shape layer drawn from paths, each item in the form a .lum file keeps it: path, fill, stroke, stroke_width, opacity, gradient and the rest",
        schema: concat!(
            r#"{"type":"object","properties":{"comp":"#,
            comp!(),
            r#","name":{"type":"string"},"index":{"type":"integer","minimum":0,"description":"Where in the stack, 0 is the top"},"items":{"type":"array","minItems":1,"description":"The shape items as a .lum file keeps them. Each path is {vertices:[{pos:[x,y],tan_in:[x,y],tan_out:[x,y]}],closed}, in comp pixels with tangents relative to their vertex. Colours are linear-light RGBA. An id and a name are made for any item without one","items":{"type":"object"}}},"required":["comp","items"],"additionalProperties":false}"#
        ),
    },
    Command {
        name: "layer.composite",
        summary: "Set how a layer meets the layers below: its blend mode, and a track matte from another layer of its comp",
        schema: concat!(
            r#"{"type":"object","properties":{"comp":"#,
            comp!(),
            r#","layer":"#,
            layer!(),
            r#","blend":{"type":"string","description":"A blend mode as a .lum file names it: Normal, Add, Screen, Multiply, Overlay, SoftLight and the rest"},"matte":{"type":["string","null"],"description":"The matte layer's id or name, or null for none"},"luma":{"type":"boolean","description":"Matte by brightness rather than alpha"},"inverted":{"type":"boolean"}},"required":["comp","layer"],"additionalProperties":false}"#
        ),
    },
    Command {
        name: "layer.get",
        summary: "One layer: its span, parent, switches and transform",
        schema: concat!(
            r#"{"type":"object","properties":{"comp":"#,
            comp!(),
            r#","layer":"#,
            layer!(),
            r#"},"required":["comp","layer"],"additionalProperties":false}"#
        ),
    },
    Command {
        name: "layer.set",
        summary: "Change a layer's name, parent, span, switches, 3D switch or place in the stack",
        schema: concat!(
            r#"{"type":"object","properties":{"comp":"#,
            comp!(),
            r#","layer":"#,
            layer!(),
            r#","name":{"type":"string"},"parent":{"type":["string","null"],"description":"The parent's id or name, or null for none. The layer stays where it is at the comp's start"},"jump":{"type":"boolean","description":"With parent: leave the transform as it is, so the layer moves with its new parent"},"in":"#,
            time!(),
            r#","out":"#,
            time!(),
            r#","start_offset":"#,
            time!(),
            r#","three_d":{"type":"boolean"},"index":{"type":"integer","minimum":0,"description":"Where in the stack, 0 is the top"},"two_node":{"type":"boolean","description":"For a camera, whether it aims at its point of interest"},"switches":{"type":"object","properties":{"visible":{"type":"boolean"},"audible":{"type":"boolean"},"solo":{"type":"boolean"},"locked":{"type":"boolean"},"shy":{"type":"boolean"},"guide":{"type":"boolean"},"motion_blur":{"type":"boolean"},"fx":{"type":"boolean"},"accepts_lights":{"type":"boolean"}},"additionalProperties":false}},"required":["comp","layer"],"additionalProperties":false}"#
        ),
    },
    Command {
        name: "layer.delete",
        summary: "Delete a layer",
        schema: concat!(
            r#"{"type":"object","properties":{"comp":"#,
            comp!(),
            r#","layer":"#,
            layer!(),
            r#"},"required":["comp","layer"],"additionalProperties":false}"#
        ),
    },
    Command {
        name: "layer.keys",
        summary: "Set one transform or camera property's whole animation: a value, a list of keys, or an expression",
        schema: concat!(
            r#"{"type":"object","required":["comp","layer","property"],"oneOf":["#,
            keys_form!(
                r#"["rotation","rotation_x","rotation_y","opacity","zoom","focus_distance","aperture"]"#,
                r#"{"type":"number"}"#
            ),
            ",",
            keys_form!(
                r#"["anchor","scale"]"#,
                r#"{"type":"array","items":{"type":"number"},"minItems":2,"maxItems":2}"#
            ),
            ",",
            keys_form!(
                r#"["position","point_of_interest"]"#,
                r#"{"type":"array","items":{"type":"number"},"minItems":2,"maxItems":3,"description":"Z is left as it is when there are two"}"#
            ),
            r#"],"description":"rotation turns about Z"}"#
        ),
    },
    Command {
        name: "markers.set",
        summary: "Replace a comp's markers, or a layer's when a layer is named",
        schema: concat!(
            r#"{"type":"object","properties":{"comp":"#,
            comp!(),
            r#","layer":"#,
            layer!(),
            r#","markers":{"type":"array","items":{"type":"object","properties":{"id":{"type":"string","description":"An existing marker's id, to keep what it was"},"time":"#,
            time!(),
            r#","label":{"type":"string"},"colour":{"type":"integer","minimum":0,"maximum":255},"duration":{"type":"integer","description":"Frames it spans"},"chapter":{"type":"boolean","description":"Whether it starts a chapter"}},"required":["time"],"additionalProperties":false}}},"required":["comp","markers"],"additionalProperties":false}"#
        ),
    },
    Command {
        name: "effect.list",
        summary: "The effects on a layer, top first, with their controls",
        schema: concat!(
            r#"{"type":"object","properties":{"comp":"#,
            comp!(),
            r#","layer":"#,
            layer!(),
            r#"},"required":["comp","layer"],"additionalProperties":false}"#
        ),
    },
    Command {
        name: "effect.add",
        summary: "Add an effect to the bottom of a layer's stack, by Lumit's name for it or After Effects'",
        schema: concat!(
            r#"{"type":"object","properties":{"comp":"#,
            comp!(),
            r#","layer":"#,
            layer!(),
            r#","effect":{"type":"string","description":"Lumit's id or name for the effect, or After Effects' match name or name, such as ADBE Gaussian Blur 2"},"name":{"type":"string","description":"The name it shows"}},"required":["comp","layer","effect"],"additionalProperties":false}"#
        ),
    },
    Command {
        name: "effect.set",
        summary: "Switch an effect on or off, rename it, or set one of its controls: a value, a list of keys, or an expression",
        schema: concat!(
            r#"{"type":"object","properties":{"comp":"#,
            comp!(),
            r#","layer":"#,
            layer!(),
            r#","effect":{"description":"The effect's id or name, or its place in the stack, 0 the top","oneOf":[{"type":"string"},{"type":"integer","minimum":0}]},"enabled":{"type":"boolean"},"name":{"type":["string","null"],"description":"The name it shows, or null for the effect's own"},"param":{"type":"string","description":"A control's id or name. After Effects' match name for one, such as ADBE Gaussian Blur 2-0001, takes After Effects' values and converts them as importing does"},"value":{"description":"A number, a list for a point or a colour, true or false for a switch, or an option's number"},"keys":{"type":"array","minItems":1,"items":"#,
            key!(r#"{"oneOf":[{"type":"number"},{"type":"array","items":{"type":"number"},"minItems":1,"maxItems":4}]}"#),
            r#"},"expression":"#,
            expression!(),
            r#"},"required":["comp","layer","effect"],"additionalProperties":false}"#
        ),
    },
    Command {
        name: "footage.import",
        summary: "Import files as footage items",
        schema: r#"{"type":"object","properties":{"paths":{"type":"array","items":{"type":"string"},"minItems":1}},"required":["paths"],"additionalProperties":false}"#,
    },
    Command {
        name: "export.start",
        summary: "Start exporting a comp",
        schema: concat!(
            r#"{"type":"object","properties":{"comp":"#,
            comp!(),
            r#","out":{"type":"string","description":"The file to write"}},"required":["comp"],"additionalProperties":false}"#
        ),
    },
    Command {
        name: "export.status",
        summary: "How an export is going",
        schema: r#"{"type":"object","properties":{"job":{"type":"integer","minimum":0}},"required":["job"],"additionalProperties":false}"#,
    },
    Command {
        name: "export.cancel",
        summary: "Stop an export",
        schema: r#"{"type":"object","properties":{"job":{"type":"integer","minimum":0}},"required":["job"],"additionalProperties":false}"#,
    },
];

/// `reach`, when there is one, is where the command's paths may go.
pub(crate) fn run(
    host: &dyn Host,
    reach: Option<&Reach>,
    name: &str,
    args: Value,
) -> Result<Value, ScriptError> {
    match name {
        "project.info" => project_info(host, parse(args)?),
        "project.save" => project_save(host, reach, parse(args)?),
        "undo" => undo(host, parse(args)?, true),
        "redo" => undo(host, parse(args)?, false),
        "comp.list" => comp_list(host, parse(args)?),
        "comp.get" => comp_get(host, parse(args)?),
        "comp.new" => comp_new(host, parse(args)?),
        "comp.set" => comp_set(host, parse(args)?),
        "comp.precompose" => comp_precompose(host, parse(args)?),
        "comp.open" => comp_open(host, parse(args)?),
        "comp.active" => comp_active(host, parse(args)?),
        "comp.frame" => comp_frame(host, reach, parse(args)?),
        "item.list" => item_list(host, parse(args)?),
        "item.set" => item_set(host, parse(args)?),
        "folder.new" => folder_new(host, parse(args)?),
        "layer.add" => layer_add(host, parse(args)?),
        "shape.add" => shape_add(host, parse(args)?),
        "layer.composite" => layer_composite(host, parse(args)?),
        "layer.get" => layer_get(host, parse(args)?),
        "layer.set" => layer_set(host, parse(args)?),
        "layer.delete" => layer_delete(host, parse(args)?),
        "layer.keys" => layer_keys(host, parse(args)?),
        "markers.set" => markers_set(host, parse(args)?),
        "effect.list" => effects::list(host, parse(args)?),
        "effect.add" => effects::add(host, parse(args)?),
        "effect.set" => effects::set(host, parse(args)?),
        "footage.import" => footage_import(host, reach, parse(args)?),
        "export.start" => export_start(host, reach, parse(args)?),
        "export.status" => export_status(host, parse(args)?),
        "export.cancel" => export_cancel(host, parse(args)?),
        _ => Err(ScriptError::UnknownCommand(name.to_owned())),
    }
}

/// `args` as the command's own argument type. Null is no arguments.
fn parse<T: DeserializeOwned>(args: Value) -> Result<T, ScriptError> {
    let args = match args {
        Value::Null => json!({}),
        args => args,
    };
    serde_json::from_value(args).map_err(|e| ScriptError::BadArgs(e.to_string()))
}

/// For a field that can be left out or set to null, and the two mean
/// different things.
fn present<'de, D: Deserializer<'de>, T: Deserialize<'de>>(d: D) -> Result<Option<T>, D::Error> {
    T::deserialize(d).map(Some)
}

fn bad(message: impl Into<String>) -> ScriptError {
    ScriptError::BadArgs(message.into())
}

/// Commit `ops` as one undo step: one op as itself, more as a batch.
fn commit(store: &DocumentStore, ops: Vec<Op>) -> Result<(), ScriptError> {
    let op = match <[Op; 1]>::try_from(ops) {
        Ok([op]) => op,
        Err(ops) if ops.is_empty() => return Ok(()),
        Err(ops) => Op::Batch { ops },
    };
    store.commit(op)?;
    Ok(())
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct NoArgs {}

/// A time on the wire: a frame on the comp's rate, or exact seconds.
#[derive(Deserialize, Clone, Copy)]
#[serde(untagged)]
enum Time {
    Frame(i64),
    Exact { num: i64, den: i64 },
}

impl Time {
    fn at(self, rate: FrameRate) -> Result<Rational, ScriptError> {
        match self {
            Time::Frame(n) => rate.time_of_frame(n).map(|t| t.0),
            Time::Exact { num, den } => Rational::new(num, den),
        }
        .map_err(|_| bad("That time is out of range."))
    }
}

fn exact(t: Rational) -> Value {
    json!({ "num": t.num(), "den": t.den() })
}

fn find_comp<'a>(doc: &'a Document, key: &str) -> Result<&'a Composition, ScriptError> {
    Uuid::try_parse(key)
        .ok()
        .and_then(|id| doc.comp(id))
        .or_else(|| {
            doc.items.iter().find_map(|item| match item {
                ProjectItem::Composition(c) if c.name == key => Some(c),
                _ => None,
            })
        })
        .ok_or_else(|| ScriptError::NotFound(format!("There is no comp called \"{key}\".")))
}

fn find_layer<'a>(comp: &'a Composition, key: &str) -> Result<&'a Layer, ScriptError> {
    let id = Uuid::try_parse(key).ok();
    comp.layers
        .iter()
        .find(|l| Some(l.id) == id)
        .or_else(|| comp.layers.iter().find(|l| l.name == key))
        .ok_or_else(|| {
            ScriptError::NotFound(format!(
                "There is no layer called \"{key}\" in {}.",
                comp.name
            ))
        })
}

fn find_footage(doc: &Document, key: &str) -> Result<Uuid, ScriptError> {
    let id = Uuid::try_parse(key).ok();
    doc.items
        .iter()
        .find_map(|item| match item {
            ProjectItem::Footage(f) if Some(f.id) == id || f.name == key => Some(f.id),
            _ => None,
        })
        .ok_or_else(|| ScriptError::NotFound(format!("There is no footage called \"{key}\".")))
}

fn comp_json(c: &Composition) -> Value {
    json!({
        "id": c.id,
        "name": c.name,
        "width": c.width,
        "height": c.height,
        "frame_rate": { "num": c.frame_rate.num(), "den": c.frame_rate.den() },
        "duration": exact(c.duration.0),
        "frames": c.frame_rate.frame_at(CompTime(c.duration.0)),
        "background": c.background.0,
        "node_graph": c.graph.is_some(),
        "layers": c.layers.len(),
    })
}

fn kind_name(kind: &LayerKind) -> &'static str {
    match kind {
        LayerKind::Footage { .. } => "footage",
        LayerKind::Solid { .. } => "solid",
        LayerKind::Precomp { .. } => "precomp",
        LayerKind::Text { .. } => "text",
        LayerKind::Camera { .. } => "camera",
        LayerKind::Sequence { .. } => "sequence",
        LayerKind::Adjustment => "adjustment",
        LayerKind::Null => "null",
        LayerKind::Shape { .. } => "shape",
        LayerKind::Light { .. } => "light",
    }
}

fn layer_json(comp: &Composition, layer: &Layer) -> Value {
    let s = &layer.switches;
    let two_node = match &layer.kind {
        LayerKind::Camera { options, .. } => Some(options.two_node),
        _ => None,
    };
    json!({
        "id": layer.id,
        "name": layer.name,
        "kind": kind_name(&layer.kind),
        "index": comp.layers.iter().position(|l| l.id == layer.id),
        "parent": layer.parent,
        "in": exact(layer.in_point.0),
        "out": exact(layer.out_point.0),
        "start_offset": exact(layer.start_offset.0),
        "three_d": s.three_d,
        "switches": {
            "visible": s.visible,
            "audible": s.audible,
            "solo": s.solo,
            "locked": s.locked,
            "shy": s.shy,
            "guide": s.guide,
            "motion_blur": s.motion_blur,
            "fx": s.fx,
            "accepts_lights": s.accepts_lights,
        },
        "two_node": two_node,
    })
}

/// The layer `id` of comp `comp` as the store now holds it.
fn layer_now(store: &DocumentStore, comp: Uuid, id: Uuid) -> Result<Value, ScriptError> {
    let doc = store.snapshot();
    let comp = doc
        .comp(comp)
        .ok_or_else(|| ScriptError::Failed("The comp went away.".into()))?;
    let layer = find_layer(comp, &id.to_string())?;
    Ok(layer_json(comp, layer))
}

fn project_info(host: &dyn Host, _: NoArgs) -> Result<Value, ScriptError> {
    let store = host.store();
    let doc = store.snapshot();
    let comps = doc
        .items
        .iter()
        .filter(|i| matches!(i, ProjectItem::Composition(_)))
        .count();
    Ok(json!({
        "path": host.path(),
        "items": doc.items.len(),
        "comps": comps,
        "can_undo": store.can_undo(),
        "can_redo": store.can_redo(),
    }))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SaveArgs {
    path: Option<PathBuf>,
}

fn project_save(host: &dyn Host, reach: Option<&Reach>, a: SaveArgs) -> Result<Value, ScriptError> {
    let path = a.path.as_deref().map(reach::plain).transpose()?;
    let path = match reach {
        Some(reach) => reach.save(host, path)?,
        None => path.map(Path::to_path_buf),
    };
    let path = host.save(path.as_deref())?;
    Ok(json!({ "path": path }))
}

fn undo(host: &dyn Host, _: NoArgs, back: bool) -> Result<Value, ScriptError> {
    let store = host.store();
    let moved = if back { store.undo()? } else { store.redo()? };
    Ok(json!({ "done": moved.is_some() }))
}

fn comp_list(host: &dyn Host, _: NoArgs) -> Result<Value, ScriptError> {
    let doc = host.store().snapshot();
    let comps: Vec<Value> = doc
        .items
        .iter()
        .filter_map(|item| match item {
            ProjectItem::Composition(c) => Some(comp_json(c)),
            _ => None,
        })
        .collect();
    Ok(json!({ "comps": comps }))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CompArgs {
    comp: String,
}

fn comp_get(host: &dyn Host, a: CompArgs) -> Result<Value, ScriptError> {
    let doc = host.store().snapshot();
    let comp = find_comp(&doc, &a.comp)?;
    let mut answer = comp_json(comp);
    answer["layers"] = comp.layers.iter().map(|l| layer_json(comp, l)).collect();
    answer["markers"] = markers_json(&comp.markers, comp.frame_rate, Rational::ZERO);
    Ok(answer)
}

/// Markers as `comp.get` and `layer.get` answer them, on the comp's clock as
/// `markers.set` takes them. A layer keeps its own on its clock, `offset`
/// behind the comp's.
fn markers_json(markers: &[Marker], rate: FrameRate, offset: Rational) -> Value {
    markers
        .iter()
        .map(|m| {
            json!({
                "id": m.id,
                "time": exact(m.time.0.checked_add(offset).unwrap_or(m.time.0)),
                "label": m.label,
                "colour": m.colour,
                "duration": m.duration.map(|d| rate.frame_at(CompTime(d))),
                "chapter": matches!(m.kind, MarkerKind::Chapter),
            })
        })
        .collect()
}

/// A frame rate on the wire: whole frames a second, or an exact fraction.
#[derive(Deserialize, Clone, Copy)]
#[serde(untagged)]
enum Rate {
    Whole(u32),
    Exact { num: u32, den: u32 },
}

impl Rate {
    fn checked(self) -> Result<FrameRate, ScriptError> {
        let (num, den) = match self {
            Rate::Whole(fps) => (fps, 1),
            Rate::Exact { num, den } => (num, den),
        };
        // After Effects' range, which is plenty for anything a comp shows.
        let fps = (u64::from(num), u64::from(den));
        if fps.0 < fps.1 || fps.0 > 999 * fps.1 {
            return Err(bad("A frame rate is 1 to 999 frames a second."));
        }
        FrameRate::new(num, den).map_err(|_| bad("That frame rate is nought."))
    }
}

/// What `comp.new` and `comp.set` can change. `comp` names the comp to change
/// and is left out for a new one.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CompSettingsArgs {
    comp: Option<String>,
    name: Option<String>,
    width: Option<u32>,
    height: Option<u32>,
    frame_rate: Option<Rate>,
    duration: Option<Time>,
    background: Option<[f32; 4]>,
    #[serde(default, deserialize_with = "present")]
    folder: Option<Option<String>>,
}

impl CompSettingsArgs {
    /// `base` with what these arguments change. A duration given in frames is
    /// counted on the rate the comp will have.
    fn over(self, base: edits::CompSettings) -> Result<edits::CompSettings, ScriptError> {
        let frame_rate = match self.frame_rate {
            None => base.frame_rate,
            Some(rate) => rate.checked()?,
        };
        let seconds = match self.duration {
            Some(t) => t.at(frame_rate)?,
            None => base.duration.0,
        };
        // The longest comp a command makes is 3 hours, as After Effects allows.
        if i128::from(seconds.num()) > 3 * 60 * 60 * i128::from(seconds.den()) {
            return Err(bad("A comp is at most 3 hours long."));
        }
        Ok(edits::CompSettings {
            name: self.name.unwrap_or(base.name),
            width: self.width.unwrap_or(base.width),
            height: self.height.unwrap_or(base.height),
            frame_rate,
            duration: edits::comp_duration(frame_rate, seconds)
                .ok_or_else(|| bad("That duration is out of range."))?,
            background: self.background.unwrap_or(base.background),
            ..base
        })
    }
}

fn comp_new(host: &dyn Host, mut a: CompSettingsArgs) -> Result<Value, ScriptError> {
    if a.comp.is_some() {
        return Err(bad("comp.new makes a comp, so it takes no comp to change."));
    }
    let store = host.store();
    let doc = store.snapshot();
    let folder = a.folder.take();
    let settings = a.over(edits::CompSettings::default())?;
    let (id, mut ops) = edits::new_comp_ops(&doc, settings, None);
    if let Some(folder) = folder {
        // Filed where asked, so of the ops only the comp itself is kept and
        // the Compositions folder is neither made nor used.
        let folder = folder.map(|key| find_folder(&doc, &key)).transpose()?;
        ops = ops
            .into_iter()
            .filter_map(|op| match op {
                Op::AddItem { item, .. } if matches!(*item, ProjectItem::Composition(_)) => {
                    Some(Op::AddItem {
                        index: doc.items.len(),
                        item,
                    })
                }
                _ => None,
            })
            .collect();
        if let Some(f) = folder.and_then(|f| doc.folder(f)) {
            let mut children = f.children.clone();
            children.push(id);
            ops.push(Op::SetFolderChildren {
                folder: f.id,
                children,
            });
        }
    }
    commit(store, ops)?;
    let doc = store.snapshot();
    let comp = doc
        .comp(id)
        .ok_or_else(|| ScriptError::Failed("The comp was not made.".into()))?;
    Ok(comp_json(comp))
}

fn comp_set(host: &dyn Host, a: CompSettingsArgs) -> Result<Value, ScriptError> {
    let store = host.store();
    let doc = store.snapshot();
    let key = a
        .comp
        .clone()
        .ok_or_else(|| bad("comp.set needs the comp to change."))?;
    if a.folder.is_some() {
        return Err(bad("comp.set does not file a comp. item.set does."));
    }
    let comp = find_comp(&doc, &key)?;
    let base = edits::CompSettings {
        name: comp.name.clone(),
        width: comp.width,
        height: comp.height,
        frame_rate: comp.frame_rate,
        duration: comp.duration,
        background: comp.background.0,
        shutter_angle: comp.motion_blur.shutter_angle,
        motion_blur_samples: comp.motion_blur.samples,
    };
    commit(store, edits::comp_settings_ops(comp, a.over(base)?))?;
    let doc = store.snapshot();
    let comp = doc
        .comp(comp.id)
        .ok_or_else(|| ScriptError::Failed("The comp went away.".into()))?;
    Ok(comp_json(comp))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PrecomposeArgs {
    comp: String,
    layers: Vec<String>,
    #[serde(default)]
    name: String,
    #[serde(default)]
    leave_attributes: bool,
}

/// Pre-compose as Layer > Pre-compose does, with the new comp as long as this
/// one, as After Effects' own `precompose` makes it.
fn comp_precompose(host: &dyn Host, a: PrecomposeArgs) -> Result<Value, ScriptError> {
    let store = host.store();
    let doc = store.snapshot();
    let comp = find_comp(&doc, &a.comp)?;
    let layers = a
        .layers
        .iter()
        .map(|key| find_layer(comp, key).map(|l| l.id))
        .collect::<Result<Vec<_>, _>>()?;
    if layers.is_empty() {
        return Err(bad("comp.precompose needs a layer to move."));
    }
    if a.leave_attributes && layers.len() > 1 {
        return Err(bad("Only a single layer can leave its attributes behind."));
    }
    let length = |layer: &Layer| {
        host.source_length(comp.id, layer)
            .or_else(|| match layer.kind {
                LayerKind::Precomp { comp } => doc.comp(comp).map(|c| c.duration),
                _ => None,
            })
    };
    let (ops, id) = edits::precompose_ops(
        &doc,
        comp,
        edits::Precompose {
            layers: &layers,
            name: a.name,
            leave_attributes: a.leave_attributes,
            adjust_duration: false,
            group: None,
            source_length: &length,
        },
    )?;
    commit(store, ops)?;
    let doc = store.snapshot();
    let comp = doc
        .comp(comp.id)
        .ok_or_else(|| ScriptError::Failed("The comp went away.".into()))?;
    let layer = find_layer(comp, &id.to_string())?;
    let LayerKind::Precomp { comp: made } = layer.kind else {
        return Err(ScriptError::Failed("The new comp was not made.".into()));
    };
    let made = doc
        .comp(made)
        .ok_or_else(|| ScriptError::Failed("The new comp was not made.".into()))?;
    Ok(json!({ "comp": comp_json(made), "layer": layer_json(comp, layer) }))
}

fn comp_open(host: &dyn Host, a: CompArgs) -> Result<Value, ScriptError> {
    let comp = find_comp(&host.store().snapshot(), &a.comp)?.id;
    host.open_comp(comp)?;
    Ok(json!({}))
}

fn comp_active(host: &dyn Host, _: NoArgs) -> Result<Value, ScriptError> {
    let doc = host.store().snapshot();
    let comp = host.active_comp().and_then(|id| doc.comp(id));
    Ok(json!({ "comp": comp.map(comp_json) }))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct FrameArgs {
    comp: String,
    time: Option<Time>,
    scale: Option<f32>,
    out: Option<PathBuf>,
}

fn comp_frame(host: &dyn Host, reach: Option<&Reach>, a: FrameArgs) -> Result<Value, ScriptError> {
    let doc = host.store().snapshot();
    let comp = find_comp(&doc, &a.comp)?;
    let rate = comp.frame_rate;
    let at = a.time.unwrap_or(Time::Frame(0)).at(rate)?;
    let frames = rate.frame_at(CompTime(comp.duration.0)).max(1);
    let frame = u64::try_from(rate.frame_at(CompTime(at)))
        .ok()
        .filter(|f| i64::try_from(*f).is_ok_and(|f| f < frames))
        .ok_or_else(|| bad(format!("{} has frames 0 to {}.", comp.name, frames - 1)))?;
    let scale = a.scale.unwrap_or(1.0);
    if !(scale > 0.0 && scale <= 1.0) {
        return Err(bad("The scale is above 0 and at most 1."));
    }
    let out = a.out.as_deref().map(reach::plain).transpose()?;
    let out = match reach {
        Some(reach) => reach.frame(host, out)?,
        None => out.map(|out| {
            let mut out = out.to_path_buf();
            if out.extension().is_none() {
                out.set_extension("png");
            }
            out
        }),
    };
    let out = match out {
        Some(out) => out,
        None => frame_path(host, reach, &comp.name, frame)?,
    };
    let (width, height) = host.render_frame(comp.id, frame, scale, &out)?;
    Ok(json!({ "path": out, "width": width, "height": height }))
}

/// A new file for frame `frame` of comp `name`, numbered, beside the project
/// or else in the script's folder.
fn frame_path(
    host: &dyn Host,
    reach: Option<&Reach>,
    name: &str,
    frame: u64,
) -> Result<PathBuf, ScriptError> {
    let dir = host
        .path()
        .and_then(|p| p.parent().map(Path::to_path_buf))
        .or_else(|| reach.and_then(|r| r.folder.clone()))
        .ok_or_else(|| bad("The project has not been saved yet, so the frame needs a path."))?;
    // A comp's name may hold anything, a separator included, and the file
    // must stay in `dir`.
    let safe: String = name
        .chars()
        .map(|c| match c {
            '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*' => '_',
            c if c.is_control() => '_',
            c => c,
        })
        .collect();
    let safe = safe.trim().trim_end_matches('.');
    let safe = if safe.is_empty() { "Frame" } else { safe };
    let stem = format!("{safe}_{frame:05}");
    let mut path = dir.join(format!("{stem}.png"));
    let mut n = 2;
    while path.symlink_metadata().is_ok() {
        path = dir.join(format!("{stem}_{n}.png"));
        n += 1;
    }
    Ok(path)
}

fn find_item<'a>(doc: &'a Document, key: &str) -> Result<&'a ProjectItem, ScriptError> {
    let id = Uuid::try_parse(key).ok();
    doc.items
        .iter()
        .find(|i| Some(i.id()) == id)
        .or_else(|| doc.items.iter().find(|i| i.name() == key))
        .ok_or_else(|| {
            ScriptError::NotFound(format!("There is nothing called \"{key}\" in the project."))
        })
}

fn find_folder(doc: &Document, key: &str) -> Result<Uuid, ScriptError> {
    match find_item(doc, key)? {
        ProjectItem::Folder(f) => Ok(f.id),
        _ => Err(bad(format!("\"{key}\" is not a folder."))),
    }
}

/// The folder `id` is filed in, if any.
fn folder_of(doc: &Document, id: Uuid) -> Option<Uuid> {
    doc.items.iter().find_map(|i| match i {
        ProjectItem::Folder(f) if f.children.contains(&id) => Some(f.id),
        _ => None,
    })
}

fn item_json(doc: &Document, item: &ProjectItem) -> Value {
    let kind = match item {
        ProjectItem::Footage(_) => "footage",
        ProjectItem::Folder(_) => "folder",
        ProjectItem::Composition(_) => "comp",
        ProjectItem::Solid(_) => "solid",
    };
    let mut answer = json!({
        "id": item.id(),
        "name": item.name(),
        "kind": kind,
        "folder": folder_of(doc, item.id()),
    });
    if let ProjectItem::Footage(f) = item {
        let rate = f
            .sequence
            .as_ref()
            .map(|s| s.frame_rate)
            .or(f.interpretation.frame_rate);
        answer["sequence"] = json!(f.sequence.is_some());
        answer["frame_rate"] = json!(rate.map(|r| json!({ "num": r.num(), "den": r.den() })));
        answer["path"] = json!(f.media.absolute_path);
    }
    answer
}

fn item_list(host: &dyn Host, _: NoArgs) -> Result<Value, ScriptError> {
    let doc = host.store().snapshot();
    let items: Vec<Value> = doc.items.iter().map(|i| item_json(&doc, i)).collect();
    Ok(json!({ "items": items }))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ItemSetArgs {
    item: String,
    name: Option<String>,
    #[serde(default, deserialize_with = "present")]
    folder: Option<Option<String>>,
    #[serde(default, deserialize_with = "present")]
    frame_rate: Option<Option<Rate>>,
}

fn item_set(host: &dyn Host, a: ItemSetArgs) -> Result<Value, ScriptError> {
    let store = host.store();
    let doc = store.snapshot();
    let item = find_item(&doc, &a.item)?;
    let id = item.id();
    let mut ops = Vec::new();
    if let Some(name) = a.name {
        ops.push(Op::RenameItem { id, name });
    }
    if let Some(rate) = a.frame_rate {
        let ProjectItem::Footage(f) = item else {
            return Err(bad("Only footage has a frame rate of its own."));
        };
        let rate = rate.map(Rate::checked).transpose()?;
        // A run of stills keeps its rate on the run, a video on how it is read.
        if f.sequence.is_some() {
            ops.push(Op::SetSequenceRate {
                id,
                frame_rate: rate
                    .ok_or_else(|| bad("A run of stills has no rate of its own to go back to."))?,
            });
        } else {
            let mut interpretation = f.interpretation.clone();
            interpretation.frame_rate = rate;
            ops.push(Op::SetFootageInterpretation {
                id,
                interpretation: Box::new(interpretation),
            });
        }
    }
    match a.folder {
        Some(Some(key)) => {
            let folder = find_folder(&doc, &key)?;
            ops.extend(
                lumit_core::ops::move_to_folder_ops(&doc, id, folder)
                    .ok_or_else(|| bad("A folder cannot go inside itself."))?,
            );
        }
        Some(None) => {
            // Out of whatever folder holds it, to the top of the panel.
            if let Some(ProjectItem::Folder(f)) = folder_of(&doc, id).and_then(|f| doc.item(f)) {
                ops.push(Op::SetFolderChildren {
                    folder: f.id,
                    children: f.children.iter().copied().filter(|c| *c != id).collect(),
                });
            }
        }
        None => {}
    }
    commit(store, ops)?;
    let doc = store.snapshot();
    let item = doc
        .item(id)
        .ok_or_else(|| ScriptError::Failed("The item went away.".into()))?;
    Ok(item_json(&doc, item))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct FolderArgs {
    #[serde(default)]
    name: String,
    parent: Option<String>,
}

fn folder_new(host: &dyn Host, a: FolderArgs) -> Result<Value, ScriptError> {
    let store = host.store();
    let doc = store.snapshot();
    let parent = a.parent.map(|key| find_folder(&doc, &key)).transpose()?;
    let (id, ops) = lumit_core::ops::new_folder_ops(&doc, &a.name, parent);
    commit(store, ops)?;
    let doc = store.snapshot();
    let item = doc
        .item(id)
        .ok_or_else(|| ScriptError::Failed("The folder was not made.".into()))?;
    Ok(item_json(&doc, item))
}

#[derive(Deserialize, Clone, Copy)]
#[serde(rename_all = "lowercase")]
enum AddKind {
    Null,
    Camera,
    Solid,
    Footage,
    Precomp,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AddArgs {
    comp: String,
    kind: AddKind,
    item: Option<String>,
    name: Option<String>,
    index: Option<usize>,
    colour: Option<[f64; 3]>,
    width: Option<u32>,
    height: Option<u32>,
}

/// Whether `comp`, or a comp anywhere inside it, holds comp `id`.
fn holds(doc: &Document, comp: &Composition, id: Uuid, seen: &mut Vec<Uuid>) -> bool {
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

fn layer_add(host: &dyn Host, a: AddArgs) -> Result<Value, ScriptError> {
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
                            solid.colour = lumit_import::linear_colour(Some(&colour));
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
    };
    let first = layers
        .first_mut()
        .ok_or_else(|| ScriptError::Failed("That footage makes no layer.".into()))?;
    if let Some(name) = a.name {
        first.name = name;
    }
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
struct CompositeArgs {
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

fn layer_composite(host: &dyn Host, a: CompositeArgs) -> Result<Value, ScriptError> {
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
struct ShapeArgs {
    comp: String,
    name: Option<String>,
    index: Option<usize>,
    items: Vec<Value>,
}

fn shape_add(host: &dyn Host, a: ShapeArgs) -> Result<Value, ScriptError> {
    use lumit_core::model::TransformGroup;
    use lumit_core::shape::ShapeItem;

    let store = host.store();
    let doc = store.snapshot();
    let comp = find_comp(&doc, &a.comp)?;
    let mut contents = Vec::new();
    for (n, mut item) in a.items.into_iter().enumerate() {
        let Some(object) = item.as_object_mut() else {
            return Err(bad("Each item is an object."));
        };
        object.entry("id").or_insert_with(|| json!(Uuid::now_v7()));
        object
            .entry("name")
            .or_insert_with(|| json!(format!("Path {}", n + 1)));
        let item: ShapeItem =
            serde_json::from_value(item).map_err(|e| bad(format!("Item {}: {e}.", n + 1)))?;
        if item.path.vertices.len() < 2 {
            return Err(bad(format!("Item {} needs two vertices or more.", n + 1)));
        }
        contents.push(item);
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
struct LayerArgs {
    comp: String,
    layer: String,
}

/// The transform and camera properties `layer.keys` and `layer.get` know, and
/// the channels each is stored as.
const PROPERTIES: &[(&str, &[TransformProp])] = &[
    ("anchor", &[TransformProp::AnchorX, TransformProp::AnchorY]),
    (
        "position",
        &[
            TransformProp::PositionX,
            TransformProp::PositionY,
            TransformProp::PositionZ,
        ],
    ),
    ("scale", &[TransformProp::ScaleX, TransformProp::ScaleY]),
    ("rotation", &[TransformProp::Rotation]),
    ("rotation_x", &[TransformProp::RotationX]),
    ("rotation_y", &[TransformProp::RotationY]),
    ("opacity", &[TransformProp::Opacity]),
    (
        "point_of_interest",
        &[
            TransformProp::PoiX,
            TransformProp::PoiY,
            TransformProp::PoiZ,
        ],
    ),
    ("zoom", &[TransformProp::Zoom]),
    ("focus_distance", &[TransformProp::FocusDistance]),
    ("aperture", &[TransformProp::Aperture]),
];

/// One channel's animation as `layer.get` answers it: a number, keys on the
/// comp's clock, or an expression.
fn animation_json(property: &Property, offset: Rational) -> Value {
    match &property.animation {
        Animation::Static(value) => json!(value),
        Animation::Keyframed(keys) => keys
            .iter()
            .map(|k| {
                let time = k.time.checked_add(offset).unwrap_or(k.time);
                json!({
                    "time": exact(time),
                    "value": k.value,
                    "hold": k.is_hold(),
                    "in": side_json(k.interp_in),
                    "out": side_json(k.interp_out),
                })
            })
            .collect(),
        Animation::Expression(text) => json!({ "expression": text }),
    }
}

/// One side of a key as `layer.keys` takes it, an ease in After Effects'
/// terms.
fn side_json(side: SideInterp) -> Value {
    match side {
        SideInterp::Hold => json!("hold"),
        SideInterp::Linear => json!("linear"),
        SideInterp::Auto { clamped: false, .. } => json!("auto"),
        SideInterp::Auto { clamped: true, .. } => json!("clamped"),
        SideInterp::Bezier { speed, influence } => {
            json!({ "speed": speed, "influence": influence * 100.0 })
        }
    }
}

fn layer_get(host: &dyn Host, a: LayerArgs) -> Result<Value, ScriptError> {
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
struct SetArgs {
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
    #[serde(default)]
    switches: SwitchArgs,
}

fn layer_set(host: &dyn Host, a: SetArgs) -> Result<Value, ScriptError> {
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

fn layer_delete(host: &dyn Host, a: LayerArgs) -> Result<Value, ScriptError> {
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

/// A property's value on the wire: one number, or one per axis.
#[derive(Deserialize, Clone)]
#[serde(untagged)]
enum Values {
    One(f64),
    Many(Vec<f64>),
}

impl Values {
    fn list(self) -> Vec<f64> {
        match self {
            Values::One(v) => vec![v],
            Values::Many(v) => v,
        }
    }
}

#[derive(Deserialize, Clone, Copy, Default)]
#[serde(rename_all = "lowercase")]
enum Interp {
    #[default]
    Linear,
    Hold,
}

/// How a key meets the curve on one side: by name, or eased as After
/// Effects eases it, with one ease for every value or one for each.
#[derive(Deserialize, Clone)]
#[serde(untagged)]
enum Side {
    Named(Named),
    Ease(Ease),
    Eases(Vec<Ease>),
}

#[derive(Deserialize, Clone, Copy)]
#[serde(rename_all = "lowercase")]
enum Named {
    Linear,
    Hold,
    Auto,
    Clamped,
}

/// A speed in units a second and an influence in per cent.
#[derive(Deserialize, Clone, Copy)]
#[serde(deny_unknown_fields)]
struct Ease {
    speed: Option<f64>,
    influence: Option<f64>,
}

impl Side {
    /// This side on value `axis` of a key, through the mapping importing an
    /// After Effects project uses.
    fn on(&self, axis: usize) -> SideInterp {
        let eased = |e: Option<&Ease>| {
            lumit_core::anim::ae_ease(e.and_then(|e| e.speed), e.and_then(|e| e.influence))
        };
        match self {
            Side::Named(Named::Linear) => SideInterp::Linear,
            Side::Named(Named::Hold) => SideInterp::Hold,
            Side::Named(Named::Auto) => SideInterp::Linear.with_tangent_mode(TangentMode::Auto),
            Side::Named(Named::Clamped) => SideInterp::Linear.with_tangent_mode(TangentMode::Clamp),
            Side::Ease(e) => eased(Some(e)),
            Side::Eases(list) => eased(list.get(axis).or_else(|| list.first())),
        }
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct KeyArgs {
    time: Time,
    value: Values,
    #[serde(default)]
    interp: Interp,
    #[serde(rename = "in")]
    into: Option<Side>,
    out: Option<Side>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct KeysArgs {
    comp: String,
    layer: String,
    property: String,
    value: Option<Values>,
    keys: Option<Vec<KeyArgs>>,
    expression: Option<String>,
}

/// `keys` as one keyframed animation per value, on the layer's clock,
/// `offset` behind the comp's. `axes` says how many values a key's value
/// is, or why it is wrong.
fn keyed(
    keys: Vec<KeyArgs>,
    rate: FrameRate,
    offset: Rational,
    axes: impl Fn(&Values) -> Result<usize, ScriptError>,
) -> Result<Vec<Animation>, ScriptError> {
    // One list of keys per axis, all at the same times.
    let mut keyed: Vec<Vec<Keyframe>> = Vec::new();
    for key in keys {
        let time = key.time.at(rate)?;
        let n = axes(&key.value)?;
        let values = key.value.list();
        if keyed.is_empty() {
            keyed.resize_with(n, Vec::new);
        } else if keyed.len() != n {
            return Err(bad("Every key needs the same number of values."));
        }
        for (n, (axis, value)) in keyed.iter_mut().zip(values).enumerate() {
            let made = Keyframe {
                time,
                value,
                interp_in: SideInterp::Linear,
                interp_out: SideInterp::Linear,
            };
            let made = match key.interp {
                Interp::Linear => made,
                Interp::Hold => made.to_hold(),
            };
            axis.push(Keyframe {
                interp_in: key.into.as_ref().map_or(made.interp_in, |s| s.on(n)),
                interp_out: key.out.as_ref().map_or(made.interp_out, |s| s.on(n)),
                ..made
            });
        }
    }
    keyed
        .into_iter()
        .map(|keys| {
            keys.into_iter()
                .map(|k| edits::key_on_layer_clock(k, offset))
                .collect::<Option<Vec<_>>>()
                .and_then(edits::keyframed)
                .ok_or_else(|| bad("Each key needs a time after the one before it."))
        })
        .collect()
}

/// What `property` holds before an expression takes it over.
fn underneath(property: &Property) -> f64 {
    match &property.animation {
        Animation::Expression(_) => Slot::read(&property.extra).own().unwrap_or(0.0),
        _ => property.value_at(0.0),
    }
}

/// `held` driven by JavaScript expression `text`. A property with several
/// values is a channel each in Lumit, so each is told all of `values` and
/// which of them it is, as importing an After Effects expression does.
fn expressed(held: &Property, text: &str, values: &[f64], axis: usize) -> Property {
    let mut next = held.clone();
    next.set_animation(Animation::Expression(text.to_owned()));
    let seed = Slot::read(&next.extra).seed;
    Slot::new(values, axis, seed)
        .in_language(Language::JavaScript)
        .write(&mut next.extra);
    next
}

fn layer_keys(host: &dyn Host, a: KeysArgs) -> Result<Value, ScriptError> {
    let store = host.store();
    let doc = store.snapshot();
    let comp = find_comp(&doc, &a.comp)?;
    let layer = find_layer(comp, &a.layer)?;
    let channels = PROPERTIES
        .iter()
        .find(|(name, _)| *name == a.property)
        .map(|(_, channels)| *channels)
        .ok_or_else(|| bad(format!("Lumit has no property called {}.", a.property)))?;
    if channels.iter().any(|c| layer.prop(*c).is_none()) {
        return Err(bad(format!("{} is not on this layer.", a.property)));
    }
    // One number for a one-axis property, a list with one for each axis
    // otherwise, though position can leave Z alone.
    let axes = |values: &Values| -> Result<usize, ScriptError> {
        match (channels.len(), values) {
            (1, Values::One(_)) => Ok(1),
            (n, Values::Many(v)) if n > 1 && (2..=n).contains(&v.len()) => Ok(v.len()),
            (1, _) => Err(bad(format!("{} takes one number.", a.property))),
            (2, _) => Err(bad(format!("{} takes 2 values.", a.property))),
            _ => Err(bad(format!("{} takes 2 or 3 values.", a.property))),
        }
    };

    let animations: Vec<Animation> = match (a.value, a.keys, a.expression) {
        (Some(value), None, None) => {
            axes(&value)?;
            value.list().into_iter().map(Animation::Static).collect()
        }
        (None, Some(keys), None) => keyed(keys, comp.frame_rate, layer.start_offset.0, axes)?,
        (None, None, Some(text)) => {
            // A 2D layer's point has no Z in After Effects, so its expression
            // answers two values and Z is left as it is.
            let flat = !layer.switches.three_d && !matches!(layer.kind, LayerKind::Camera { .. });
            let used = if flat {
                channels.len().min(2)
            } else {
                channels.len()
            };
            let mut ops = Vec::new();
            for channel in channels.iter().take(used) {
                ops.extend(edits::transform_ops(
                    comp.id,
                    layer,
                    *channel,
                    Animation::Expression(text.clone()),
                    Some(Language::JavaScript),
                ));
            }
            let id = layer.id;
            commit(store, ops)?;
            return layer_now(store, comp.id, id);
        }
        _ => return Err(bad("Give a value, keys or an expression.")),
    };

    let mut ops = Vec::new();
    for (channel, animation) in channels.iter().zip(animations) {
        ops.extend(edits::transform_ops(
            comp.id, layer, *channel, animation, None,
        ));
    }
    let id = layer.id;
    commit(store, ops)?;
    layer_now(store, comp.id, id)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct MarkerArgs {
    id: Option<Uuid>,
    time: Time,
    #[serde(default)]
    label: String,
    colour: Option<u8>,
    duration: Option<i64>,
    chapter: Option<bool>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct MarkersArgs {
    comp: String,
    layer: Option<String>,
    markers: Vec<MarkerArgs>,
}

fn markers_set(host: &dyn Host, a: MarkersArgs) -> Result<Value, ScriptError> {
    let store = host.store();
    let doc = store.snapshot();
    let comp = find_comp(&doc, &a.comp)?;
    let rate = comp.frame_rate;
    let layer = a.layer.map(|key| find_layer(comp, &key)).transpose()?;
    // Times come in on the comp's clock, and a layer keeps its markers on
    // its own.
    let offset = layer.map_or(Rational::ZERO, |l| l.start_offset.0);
    let chapters: Vec<Option<bool>> = a.markers.iter().map(|m| m.chapter).collect();
    let edits = a
        .markers
        .into_iter()
        .map(|m| {
            let time = m.time.at(rate)?.checked_sub(offset);
            Ok(MarkerEdit {
                id: m.id.unwrap_or_else(Uuid::now_v7),
                time: CompTime(time.map_err(|_| bad("That time is out of range."))?),
                label: m.label,
                colour: m.colour,
                duration_frames: m.duration,
            })
        })
        .collect::<Result<Vec<_>, ScriptError>>()?;
    let count = edits.len();
    // Merged onto what is there, so a marker named by id keeps what a script
    // cannot say, such as being a beat.
    let held = layer.map_or(&comp.markers, |l| &l.markers);
    let mut markers = lumit_core::markers::merge_edits(edits, held, rate);
    for (marker, chapter) in markers.iter_mut().zip(chapters) {
        match chapter {
            Some(true) => marker.kind = MarkerKind::Chapter,
            Some(false) if matches!(marker.kind, MarkerKind::Chapter) => {
                marker.kind = MarkerKind::User;
            }
            _ => {}
        }
    }
    let op = match layer {
        Some(layer) => Op::SetLayerMarkers {
            comp: comp.id,
            layer: layer.id,
            markers,
        },
        None => Op::SetCompMarkers {
            comp: comp.id,
            markers,
        },
    };
    commit(store, vec![op])?;
    Ok(json!({ "markers": count }))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ImportArgs {
    paths: Vec<PathBuf>,
}

fn footage_import(
    host: &dyn Host,
    reach: Option<&Reach>,
    a: ImportArgs,
) -> Result<Value, ScriptError> {
    let paths = a
        .paths
        .iter()
        .map(|path| match (reach::plain(path)?, reach) {
            (path, Some(reach)) => reach.footage(host, path),
            (path, None) => Ok(path.to_path_buf()),
        })
        .collect::<Result<Vec<_>, _>>()?;
    let ids = host.import_footage(&paths)?;
    let doc = host.store().snapshot();
    let items: Vec<Value> = ids
        .iter()
        .map(|id| {
            let name = match doc.item(*id) {
                Some(ProjectItem::Footage(f)) => Some(f.name.clone()),
                _ => None,
            };
            json!({ "id": id, "name": name })
        })
        .collect();
    Ok(json!({ "items": items }))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ExportArgs {
    comp: String,
    out: Option<PathBuf>,
}

fn export_start(
    host: &dyn Host,
    reach: Option<&Reach>,
    a: ExportArgs,
) -> Result<Value, ScriptError> {
    let comp = find_comp(&host.store().snapshot(), &a.comp)?.id;
    let out = a.out.as_deref().map(reach::plain).transpose()?;
    let out = match reach {
        Some(reach) => reach.export(host, out)?,
        None => out.map(Path::to_path_buf),
    };
    let job = host.export_start(comp, out.as_deref())?;
    Ok(json!({ "job": job }))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct JobArgs {
    job: u64,
}

fn export_status(host: &dyn Host, a: JobArgs) -> Result<Value, ScriptError> {
    serde_json::to_value(host.export_status(a.job)?).map_err(|e| ScriptError::Failed(e.to_string()))
}

fn export_cancel(host: &dyn Host, a: JobArgs) -> Result<Value, ScriptError> {
    host.export_cancel(a.job)?;
    Ok(json!({}))
}

//! The commands themselves: the table, and one function each.

mod comps;
mod effects;
mod items;
mod keys;
mod layers;
mod write;

use std::path::{Path, PathBuf};

use lumit_core::model::{Composition, Document, Layer, LayerKind, ProjectItem};
use lumit_core::ops::Op;
use lumit_core::store::DocumentStore;
use lumit_core::time::{CompTime, FrameRate, Rational};
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
        summary: "Every item in the project panel: comps, footage, folders and solids, with the folder each is in, and the size, rate and length of footage and solids",
        schema: none!(),
    },
    Command {
        name: "item.set",
        summary: "Rename an item, file it in a folder, set the rate a footage item plays at, or set its comment",
        schema: concat!(
            r#"{"type":"object","properties":{"item":{"type":"string","description":"The item's id or name"},"name":{"type":"string"},"folder":{"type":["string","null"],"description":"The folder's id or name, or null for none"},"frame_rate":{"anyOf":["#,
            rate!(),
            r#",{"type":"null","description":"A video's own rate"}]},"comment":{"type":"string","description":"Empty for none"}},"required":["item"],"additionalProperties":false}"#
        ),
    },
    Command {
        name: "folder.new",
        summary: "Make a folder in the project panel",
        schema: r#"{"type":"object","properties":{"name":{"type":"string"},"parent":{"type":"string","description":"The folder to make it in, by id or name"}},"additionalProperties":false}"#,
    },
    Command {
        name: "layer.add",
        summary: "Add a null, camera, solid, footage, precomp, text, shape, adjustment or light layer to a comp",
        schema: concat!(
            r#"{"type":"object","properties":{"comp":"#,
            comp!(),
            r#","kind":{"enum":["null","camera","solid","footage","precomp","text","shape","adjustment","light"]},"item":{"type":"string","description":"For footage: the footage item's id or name. For a precomp: the comp's"},"name":{"type":"string"},"index":{"type":"integer","minimum":0,"description":"Where in the stack, 0 is the top"},"colour":{"type":"array","items":{"type":"number","minimum":0,"maximum":1},"minItems":3,"maxItems":3,"description":"For a solid: its colour as a colour picker shows it, 0 to 1. White if left out"},"width":{"type":"integer","minimum":1,"maximum":16384,"description":"For a solid, the comp's if left out"},"height":{"type":"integer","minimum":1,"maximum":16384},"fields":{"type":"object","description":"Any of the layer's fields as a .lum file keeps them, laid over the new layer as layer.write lays them. A text layer turns about the middle of its words unless these set its anchor"}},"required":["comp","kind"],"additionalProperties":false}"#
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
        name: "layer.read",
        summary: "One layer whole, in the form a .lum file keeps it, to change and give back to layer.write",
        schema: concat!(
            r#"{"type":"object","properties":{"comp":"#,
            comp!(),
            r#","layer":"#,
            layer!(),
            r#"},"required":["comp","layer"],"additionalProperties":false}"#
        ),
    },
    Command {
        name: "layer.write",
        summary: "Change any part of a layer, in the form a .lum file keeps it: its kind and contents, text, transform, masks, effects, styles, paint, switches and the rest",
        schema: concat!(
            r#"{"type":"object","properties":{"comp":"#,
            comp!(),
            r#","layer":"#,
            layer!(),
            r#","fields":{"type":"object","description":"Laid over the layer as JSON Merge Patch does: objects merge key by key, null takes a field away, anything else replaces it, and a choice such as {\"Keyframed\": [...]} replaces another such as {\"Static\": 1}. Times are exact seconds as [num, den], colours linear-light RGBA. The layer keeps its id and its place"}},"required":["comp","layer","fields"],"additionalProperties":false}"#
        ),
    },
    Command {
        name: "comp.write",
        summary: "Change a comp's motion blur, lighting or node graph, in the form a .lum file keeps them",
        schema: concat!(
            r#"{"type":"object","properties":{"comp":"#,
            comp!(),
            r#","fields":{"type":"object","properties":{"motion_blur":{"type":"object"},"lighting":{"type":"object"},"graph":{"type":"object"}},"additionalProperties":false,"description":"Each laid over the comp's own as JSON Merge Patch does"}},"required":["comp","fields"],"additionalProperties":false}"#
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
        summary: "Change a layer's name, parent, span, switches, 3D switch, rotation order or place in the stack",
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
            r#","three_d":{"type":"boolean"},"index":{"type":"integer","minimum":0,"description":"Where in the stack, 0 is the top"},"two_node":{"type":"boolean","description":"For a camera, whether it aims at its point of interest"},"after_effects_order":{"type":"boolean","description":"Turn X, Y and Z Rotation in After Effects' order rather than Lumit's. The rotations are rewritten so the layer keeps its pose"},"switches":{"type":"object","properties":{"visible":{"type":"boolean"},"audible":{"type":"boolean"},"solo":{"type":"boolean"},"locked":{"type":"boolean"},"shy":{"type":"boolean"},"guide":{"type":"boolean"},"motion_blur":{"type":"boolean"},"fx":{"type":"boolean"},"accepts_lights":{"type":"boolean"}},"additionalProperties":false}},"required":["comp","layer"],"additionalProperties":false}"#
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
            ",",
            keys_form!(
                r#"["orientation"]"#,
                r#"{"type":"array","items":{"type":"number"},"minItems":3,"maxItems":3,"description":"X, Y and Z, in After Effects' order"}"#
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
        name: "effect.get",
        summary: "One effect on a layer, with every control it has",
        schema: concat!(
            r#"{"type":"object","properties":{"comp":"#,
            comp!(),
            r#","layer":"#,
            layer!(),
            r#","effect":{"description":"The effect's id or name, or its place in the stack, 0 the top","oneOf":[{"type":"string"},{"type":"integer","minimum":0}]}},"required":["comp","layer","effect"],"additionalProperties":false}"#
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
        "comp.list" => comps::comp_list(host, parse(args)?),
        "comp.get" => comps::comp_get(host, parse(args)?),
        "comp.new" => comps::comp_new(host, parse(args)?),
        "comp.set" => comps::comp_set(host, parse(args)?),
        "comp.precompose" => comps::comp_precompose(host, parse(args)?),
        "comp.open" => comps::comp_open(host, parse(args)?),
        "comp.active" => comps::comp_active(host, parse(args)?),
        "comp.frame" => comps::comp_frame(host, reach, parse(args)?),
        "item.list" => items::item_list(host, parse(args)?),
        "item.set" => items::item_set(host, parse(args)?),
        "folder.new" => items::folder_new(host, parse(args)?),
        "layer.add" => layers::layer_add(host, parse(args)?),
        "shape.add" => layers::shape_add(host, parse(args)?),
        "layer.read" => write::layer_read(host, parse(args)?),
        "layer.write" => write::layer_write(host, parse(args)?),
        "comp.write" => write::comp_write(host, parse(args)?),
        "layer.composite" => layers::layer_composite(host, parse(args)?),
        "layer.get" => layers::layer_get(host, parse(args)?),
        "layer.set" => layers::layer_set(host, parse(args)?),
        "layer.delete" => layers::layer_delete(host, parse(args)?),
        "layer.keys" => keys::layer_keys(host, parse(args)?),
        "markers.set" => keys::markers_set(host, parse(args)?),
        "effect.list" => effects::list(host, parse(args)?),
        "effect.get" => effects::get(host, parse(args)?),
        "effect.add" => effects::add(host, parse(args)?),
        "effect.set" => effects::set(host, parse(args)?),
        "footage.import" => items::footage_import(host, reach, parse(args)?),
        "export.start" => items::export_start(host, reach, parse(args)?),
        "export.status" => items::export_status(host, parse(args)?),
        "export.cancel" => items::export_cancel(host, parse(args)?),
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
        LayerKind::Model { .. } => "model",
        LayerKind::Mesh { .. } => "mesh",
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
        "after_effects_order": layer.transform.after_effects_order,
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

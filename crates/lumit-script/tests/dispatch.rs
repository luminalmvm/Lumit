//! A run of commands through `dispatch` is one undo step: a camera with
//! keys, a null it is parented to and the comp's markers, all taken back by
//! one undo.

// A test asserts by panicking; the engine's no-panic rule is about the engine
// (the same allow every other integration test in the workspace carries).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::{Path, PathBuf};

use lumit_core::model::{Composition, Document, Layer};
use lumit_core::DocumentStore;
use lumit_script::{dispatch, one_step, ExportStatus, Host, ScriptError, COMMANDS};
use serde_json::{json, Value};
use uuid::Uuid;

/// A host with a document and nothing else.
struct Bare(DocumentStore);

impl Host for Bare {
    fn store(&self) -> &DocumentStore {
        &self.0
    }
    fn path(&self) -> Option<PathBuf> {
        None
    }
    fn save(&self, _: Option<&Path>) -> Result<PathBuf, ScriptError> {
        Err(ScriptError::Failed("no saving here".into()))
    }
    fn import_footage(&self, _: &[PathBuf]) -> Result<Vec<Uuid>, ScriptError> {
        Err(ScriptError::Failed("no footage here".into()))
    }
    fn footage_layers(&self, _: &Composition, _: Uuid) -> Result<Vec<Layer>, ScriptError> {
        Err(ScriptError::Failed("no footage here".into()))
    }
    fn export_start(&self, _: Uuid, _: Option<&Path>) -> Result<u64, ScriptError> {
        Err(ScriptError::Failed("no export here".into()))
    }
    fn export_status(&self, _: u64) -> Result<ExportStatus, ScriptError> {
        Err(ScriptError::Failed("no export here".into()))
    }
    fn export_cancel(&self, _: u64) -> Result<(), ScriptError> {
        Err(ScriptError::Failed("no export here".into()))
    }
}

fn call(host: &Bare, name: &str, args: Value) -> Value {
    dispatch(host, name, args).unwrap_or_else(|e| panic!("{name}: {e}"))
}

#[test]
fn a_run_of_commands_is_one_undo_step() {
    for command in COMMANDS {
        serde_json::from_str::<Value>(command.schema)
            .unwrap_or_else(|e| panic!("{}'s schema: {e}", command.name));
    }

    let host = Bare(DocumentStore::new(Document::new()));
    call(&host, "comp.new", json!({ "name": "Main" }));

    one_step(host.store(), || {
        dispatch(
            &host,
            "layer.add",
            json!({ "comp": "Main", "kind": "camera" }),
        )?;
        dispatch(
            &host,
            "layer.keys",
            json!({ "comp": "Main", "layer": "Camera", "property": "position", "keys": [
                { "time": 0, "value": [960, 540, -1500] },
                { "time": { "num": 1, "den": 2 }, "value": [1200, 540, -1200], "interp": "hold" },
            ]}),
        )?;
        dispatch(
            &host,
            "layer.keys",
            json!({ "comp": "Main", "layer": "Camera", "property": "zoom", "value": 1800 }),
        )?;
        dispatch(
            &host,
            "markers.set",
            json!({ "comp": "Main", "markers": [{ "time": 30, "label": "Cut", "duration": 6 }] }),
        )?;
        dispatch(
            &host,
            "layer.add",
            json!({ "comp": "Main", "kind": "null", "name": "Rig" }),
        )?;
        dispatch(
            &host,
            "layer.set",
            json!({ "comp": "Main", "layer": "Camera", "parent": "Rig" }),
        )
    })
    .unwrap();

    let camera = call(
        &host,
        "layer.get",
        json!({ "comp": "Main", "layer": "Camera" }),
    );
    let position = &camera["properties"]["position"];
    assert_eq!(position[0][1]["value"], json!(1200.0));
    assert_eq!(position[0][1]["time"], json!({ "num": 1, "den": 2 }));
    assert_eq!(position[2][0]["value"], json!(-1500.0));
    assert_eq!(camera["properties"]["zoom"], json!(1800.0));
    let rig = call(
        &host,
        "layer.get",
        json!({ "comp": "Main", "layer": "Rig" }),
    );
    assert_eq!(camera["parent"], rig["id"]);

    let doc = host.store().snapshot();
    let comp = doc.items.iter().find_map(|i| match i {
        lumit_core::model::ProjectItem::Composition(c) => Some(c),
        _ => None,
    });
    let comp = comp.unwrap();
    assert_eq!(comp.markers.len(), 1);
    assert_eq!(comp.layers.len(), 2);

    assert_eq!(call(&host, "undo", Value::Null), json!({ "done": true }));
    let main = call(&host, "comp.get", json!({ "comp": "Main" }));
    assert_eq!(main["layers"], json!([]));
    let doc = host.store().snapshot();
    assert!(doc.comp(comp.id).unwrap().markers.is_empty());

    // A run that fails takes back what it did before failing, and leaves
    // the history as it was: the step undone above can still be redone.
    let failed = one_step(host.store(), || {
        dispatch(
            &host,
            "layer.add",
            json!({ "comp": "Main", "kind": "null" }),
        )?;
        dispatch(
            &host,
            "layer.add",
            json!({ "comp": "Nowhere", "kind": "null" }),
        )
    });
    assert_eq!(failed.unwrap_err().code(), "not-found");
    assert_eq!(
        call(&host, "comp.get", json!({ "comp": "Main" }))["layers"],
        json!([])
    );
    assert!(host.store().can_redo());
    assert_eq!(host.store().history().len(), 2);

    // A run inside a group someone else opened is refused, so it can never
    // take back their edits.
    host.store().begin_undo_group();
    let nested = one_step(host.store(), || Ok(()));
    host.store().end_undo_group();
    assert_eq!(nested.unwrap_err().code(), "failed");
}

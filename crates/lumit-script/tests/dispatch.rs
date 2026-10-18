//! A run of commands through `dispatch` is one undo step: a camera with
//! keys, a null it is parented to and the comp's markers, all taken back by
//! one undo.

// A test asserts by panicking; the engine's no-panic rule is about the engine
// (the same allow every other integration test in the workspace carries).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use lumit_core::model::{Composition, Document, Layer};
use lumit_core::DocumentStore;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use lumit_script::{
    dispatch, one_step, run, ExportStatus, Host, RunOptions, ScriptError, COMMANDS,
};
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

#[test]
fn a_script_that_runs_away_stops_on_the_flag_and_leaves_nothing() {
    let host = Bare(DocumentStore::new(Document::new()));
    call(&host, "comp.new", json!({ "name": "Main" }));

    let stop = Arc::new(AtomicBool::new(false));
    let flag = stop.clone();
    let stopper = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(200));
        flag.store(true, Ordering::Relaxed);
    });
    let report = run(
        &host,
        "lumit.layer.add({ comp: \"Main\", kind: \"null\" });
print(\"added\");
while (true) {}
",
        RunOptions {
            name: "Runaway.js".into(),
            stop,
            ..RunOptions::default()
        },
    );
    stopper.join().unwrap();

    let failed = report.result.unwrap_err();
    assert_eq!(failed.error, ScriptError::Stopped);
    assert_eq!(report.log, ["added"]);
    assert_eq!(
        call(&host, "comp.get", json!({ "comp": "Main" }))["layers"],
        json!([])
    );
    // Nothing of the script's is left to redo either.
    assert!(!host.store().can_redo());
}

/// An After Effects script keys a camera, and the camera in Lumit faces the
/// way it does in After Effects, which turns by Rx · Ry · Rz where Lumit turns
/// by Ry · Rx · Rz.
#[test]
fn an_after_effects_script_keys_a_camera() {
    let host = Bare(DocumentStore::new(Document::new()));
    let script = r#"#target aftereffects
app.beginUndoGroup("Camera");
var comp = app.project.items.addComp("Shot", 1920, 1080, 1, 2, 29.97);
var cam = comp.layers.addCamera("Cam", [960, 540]);
cam.autoOrient = AutoOrientType.NO_AUTO_ORIENT;
var t = cam.property("ADBE Transform Group");
var times = [0, 1 / 29.97, 2 / 29.97];
t.property("ADBE Position").setValuesAtTimes(times, [[0, 0, -100], [10, 20, -90], [20, 40, -80]]);
t.property("ADBE Rotate X").setValuesAtTimes(times, [10, 20, 30]);
t.property("Y Rotation").setValuesAtTimes(times, [40, 50, 60]);
t.property("ADBE Rotate Z").setValue(5);
var zoom = cam.property("ADBE Camera Options Group").property("ADBE Camera Zoom");
zoom.setValuesAtTimes(times, [1000, 1100, 1200]);
zoom.setInterpolationTypeAtKey(2, KeyframeInterpolationType.HOLD);
app.endUndoGroup();
"#;
    let report = run(
        &host,
        script,
        RunOptions {
            name: "Camera.jsx".into(),
            ..RunOptions::default()
        },
    );
    report.result.unwrap();

    let comp = call(&host, "comp.get", json!({ "comp": "Shot" }));
    assert_eq!(comp["frame_rate"], json!({ "num": 30000, "den": 1001 }));
    assert_eq!(comp["frames"], json!(60));
    let cam = call(
        &host,
        "layer.get",
        json!({ "comp": "Shot", "layer": "Cam" }),
    );
    assert_eq!(cam["two_node"], json!(false));
    let p = &cam["properties"];
    let at = |key: &Value, n: usize| key[n]["value"].as_f64().unwrap();
    assert_eq!(p["position"][2].as_array().unwrap().len(), 3);
    assert_eq!(at(&p["position"][1], 2), 40.0);
    assert_eq!(p["zoom"][1]["hold"], json!(true));
    assert_eq!(at(&p["zoom"], 2), 1200.0);
    let faces = |p: &Value, ae: [(f64, f64, f64); 3]| {
        for (n, ae) in ae.iter().enumerate() {
            let turned = (
                at(&p["rotation_x"], n),
                at(&p["rotation_y"], n),
                at(&p["rotation"], n),
            );
            let want = Mat::rx(ae.0).mul(Mat::ry(ae.1)).mul(Mat::rz(ae.2));
            let got = lumit_core::camera::rotation(turned);
            for (r, row) in got.iter().enumerate() {
                for (c, v) in row.iter().enumerate() {
                    assert!((v - want.0[r][c]).abs() < 1e-9, "key {n}: {turned:?}");
                }
            }
        }
    };
    faces(p, [(10.0, 40.0, 5.0), (20.0, 50.0, 5.0), (30.0, 60.0, 5.0)]);
    // The whole run is one step, named after the script.
    let history = host.store().history();
    assert_eq!(history.len(), 1);
    assert_eq!(history[0].name, "Camera.jsx");

    // X and Y keyed apart from Z, as Lumit can leave them. A second run
    // reads every axis at every key, and reads the rotation back in After
    // Effects' order, so turning Z alone leaves X and Y as they were.
    call(
        &host,
        "layer.keys",
        json!({ "comp": "Shot", "layer": "Cam", "property": "position", "keys": [
            { "time": 0, "value": [0, 0] },
            { "time": 3, "value": [30, 60] },
        ]}),
    );
    let again = r#"var comp;
for (var i = 1; i <= app.project.numItems; i++) {
    if (app.project.item(i).name === "Shot") comp = app.project.item(i);
}
var t = comp.layer("Cam").transform;
var read = [t.position.numKeys].concat(t.position.keyValue(2), t.xRotation.keyValue(1), t.yRotation.keyValue(1));
t.zRotation.setValueAtTime(0, 15);
read"#;
    let report = run(&host, again, RunOptions::default());
    let read: Vec<f64> = report
        .result
        .unwrap()
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_f64().unwrap())
        .collect();
    for (got, want) in read.iter().zip([4.0, 10.0, 20.0, -90.0, 10.0, 40.0]) {
        assert!((got - want).abs() < 1e-9, "{read:?}");
    }
    let cam = call(
        &host,
        "layer.get",
        json!({ "comp": "Shot", "layer": "Cam" }),
    );
    faces(
        &cam["properties"],
        [(10.0, 40.0, 15.0), (20.0, 50.0, 5.0), (30.0, 60.0, 5.0)],
    );
}

struct Mat([[f64; 3]; 3]);

impl Mat {
    fn turn(deg: f64, axis: usize) -> Mat {
        let (s, c) = deg.to_radians().sin_cos();
        let (a, b) = ((axis + 1) % 3, (axis + 2) % 3);
        let mut m = [[0.0; 3]; 3];
        m[axis][axis] = 1.0;
        m[a][a] = c;
        m[b][b] = c;
        m[a][b] = -s;
        m[b][a] = s;
        Mat(m)
    }
    fn rx(deg: f64) -> Mat {
        Mat::turn(deg, 0)
    }
    fn ry(deg: f64) -> Mat {
        Mat::turn(deg, 1)
    }
    fn rz(deg: f64) -> Mat {
        Mat::turn(deg, 2)
    }
    fn mul(self, o: Mat) -> Mat {
        let mut m = [[0.0; 3]; 3];
        for (r, row) in m.iter_mut().enumerate() {
            for (c, v) in row.iter_mut().enumerate() {
                *v = (0..3).map(|k| self.0[r][k] * o.0[k][c]).sum();
            }
        }
        Mat(m)
    }
}

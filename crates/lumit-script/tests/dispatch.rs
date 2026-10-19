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

/// The After Effects pieces past keys and cameras: eased keys, a solid, a
/// precomp, an effect with a keyed control, markers, an expression and a file
/// written in the script's folder, with one outside it refused.
#[test]
fn an_after_effects_script_eases_adds_effects_and_writes_a_file() {
    use lumit_core::anim::{Animation, SideInterp};
    use lumit_core::markers::MarkerKind;
    use lumit_core::model::{EffectValue, LayerKind, ProjectItem, TransformProp};

    let dir = std::env::var_os("LUMIT_USER_DIRS")
        .map_or_else(std::env::temp_dir, PathBuf::from)
        .join(format!("ae-{}", Uuid::now_v7()));
    let folder = dir.join("script");
    std::fs::create_dir_all(&folder).unwrap();
    let path = folder.join("edit.jsx");
    let script = r#"var comp = app.project.items.addComp("Edit", 1920, 1080, 1, 4, 30);
var inner = app.project.items.addComp("Inner", 640, 360, 1, 2, 30);
var solid = comp.layers.addSolid([1, 0, 0], "Red", 800, 600, 1);
var nest = comp.layers.add(inner);
var fade = solid.property("ADBE Transform Group").property("ADBE Opacity");
fade.setValuesAtTimes([0, 1], [0, 100]);
fade.setTemporalEaseAtKey(2, [new KeyframeEase(0, 75)]);
var turn = solid.transform.rotation;
turn.setValuesAtTimes([0, 1], [0, 90]);
turn.setInterpolationTypeAtKey(1, KeyframeInterpolationType.BEZIER);
var blur = solid.property("ADBE Effect Parade").addProperty("ADBE Gaussian Blur 2");
blur.property(1).setValueAtTime(0, 0);
blur.property(1).setValueAtTime(2, 40);
blur.enabled = false;
comp.markerProperty.setValueAtTime(1, new MarkerValue("Hit"));
nest.property("Marker").setValueAtTime(0.5, new MarkerValue("In", "In"));
nest.transform.position.expression = "wiggle(2, 10)";
var here = new File($.fileName).parent;
new Folder(here.fsName + "/out").create();
var log = new File(here.fsName + "/out/log.txt");
log.open("w");
log.writeln("layers " + comp.numLayers);
log.close();
var refused = "";
try {
    new File(here.parent.fsName + "/outside.txt").open("w");
} catch (e) {
    refused = e.message;
}
[app.project.activeItem, refused, fade.keyInTemporalEase(2)[0].influence]
"#;
    std::fs::write(&path, script).unwrap();
    let host = Bare(DocumentStore::new(Document::new()));
    let report = run(
        &host,
        script,
        RunOptions {
            name: "edit.jsx".into(),
            path: Some(path),
            ..RunOptions::default()
        },
    );
    let read = report.result.unwrap();
    assert_eq!(read[0], Value::Null);
    assert!(read[1].as_str().unwrap().contains("own folder"), "{read}");
    assert_eq!(read[2].as_f64(), Some(75.0));

    let doc = host.store().snapshot();
    let comp = |name: &str| {
        doc.items
            .iter()
            .find_map(|i| match i {
                ProjectItem::Composition(c) if c.name == name => Some(c),
                _ => None,
            })
            .unwrap()
    };
    let (edit, inner) = (comp("Edit"), comp("Inner"));
    let layer = |name: &str| edit.layers.iter().find(|l| l.name == name).unwrap();
    let keys = |l: &Layer, p: TransformProp| match &l.prop(p).unwrap().animation {
        Animation::Keyframed(keys) => keys.clone(),
        other => panic!("{other:?}"),
    };

    // A red solid 800 by 600, with its fade eased in and its turn automatic.
    let red = layer("Red");
    let LayerKind::Solid { def } = red.kind else {
        panic!("{:?}", red.kind)
    };
    let Some(ProjectItem::Solid(solid)) = doc.item(def) else {
        panic!("no solid")
    };
    assert_eq!((solid.width, solid.height), (800, 600));
    assert_eq!(solid.colour.0, [1.0, 0.0, 0.0, 1.0]);
    assert_eq!(
        keys(red, TransformProp::Opacity)[1].interp_in,
        SideInterp::Bezier {
            speed: 0.0,
            influence: 0.75
        }
    );
    assert!(matches!(
        keys(red, TransformProp::Rotation)[0].interp_out,
        SideInterp::Auto { clamped: false, .. }
    ));

    // The blur came across as importing brings it, its radius keyed and the
    // effect off.
    let blur = &red.effects[0];
    assert_eq!(blur.effect.match_name, "blur");
    assert!(!blur.enabled);
    let radius = blur.params.iter().find(|p| p.id == "radius").unwrap();
    let EffectValue::Float(radius) = &radius.value else {
        panic!("{radius:?}")
    };
    let Animation::Keyframed(radius) = &radius.animation else {
        panic!("{radius:?}")
    };
    assert_eq!(radius.len(), 2);
    assert_eq!(radius[1].value, 40.0);
    assert_eq!(
        radius[1].time,
        lumit_core::time::Rational::new(2, 1).unwrap()
    );

    // The precomp, its chapter marker, and the comp's own marker.
    let nest = layer("Inner");
    assert_eq!(nest.kind, LayerKind::Precomp { comp: inner.id });
    assert_eq!(nest.markers[0].label, "In");
    assert!(matches!(nest.markers[0].kind, MarkerKind::Chapter));
    assert_eq!(edit.markers[0].label, "Hit");

    // One expression on each axis of the position, each told which it is.
    for (axis, prop) in [TransformProp::PositionX, TransformProp::PositionY]
        .into_iter()
        .enumerate()
    {
        let p = nest.prop(prop).unwrap();
        assert_eq!(p.animation, Animation::Expression("wiggle(2, 10)".into()));
        let slot = lumit_core::expression::Slot::read(&p.extra);
        assert_eq!(usize::from(slot.axis), axis);
        assert_eq!(slot.language, lumit_core::expression::Language::JavaScript);
    }

    // The file inside the folder, and nothing outside it.
    let written = std::fs::read_to_string(folder.join("out").join("log.txt")).unwrap();
    assert_eq!(written.trim_end(), "layers 2");
    assert!(!dir.join("outside.txt").exists());
    let _ = std::fs::remove_dir_all(&dir);
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

/// MCP clients stop at the first field out of place, so the handshake, the
/// tool list and a call are pinned here.
#[test]
fn mcp_answers_in_the_shape_clients_expect() {
    let host = Bare(DocumentStore::new(Document::new()));
    call(&host, "comp.new", json!({ "name": "Main" }));
    let ask = |line: Value| -> Value {
        let reply = lumit_script::mcp::answer(&line.to_string(), &host).unwrap();
        assert!(!reply.contains('\n'));
        serde_json::from_str(&reply).unwrap()
    };

    let hello = ask(
        json!({ "jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {
        "protocolVersion": "2025-06-18", "capabilities": {},
        "clientInfo": { "name": "test", "version": "1" } } }),
    );
    assert_eq!(hello["id"], json!(1));
    assert_eq!(hello["result"]["protocolVersion"], json!("2025-06-18"));
    assert_eq!(hello["result"]["capabilities"], json!({ "tools": {} }));
    assert_eq!(hello["result"]["serverInfo"]["name"], json!("lumit"));
    let initialized = json!({ "jsonrpc": "2.0", "method": "notifications/initialized" });
    assert_eq!(
        lumit_script::mcp::answer(&initialized.to_string(), &host),
        None
    );

    let list = ask(json!({ "jsonrpc": "2.0", "id": 2, "method": "tools/list" }));
    let tools = list["result"]["tools"].as_array().unwrap();
    assert_eq!(tools.len(), COMMANDS.len() + 1);
    for tool in tools {
        let name = tool["name"].as_str().unwrap();
        assert!(name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-'));
        assert_eq!(tool["inputSchema"]["type"], json!("object"), "{name}");
        assert!(tool["description"].is_string());
    }

    let added = ask(
        json!({ "jsonrpc": "2.0", "id": "a", "method": "tools/call", "params": {
        "name": "layer_add", "arguments": { "comp": "Main", "kind": "camera" } } }),
    );
    assert_eq!(added["result"]["isError"], json!(false));
    let text = added["result"]["content"][0]["text"].as_str().unwrap();
    let layer: Value = serde_json::from_str(text).unwrap();
    assert!(layer["id"].is_string(), "{text}");

    // A command that fails is the tool's answer, an unknown method the
    // protocol's.
    let missing = ask(
        json!({ "jsonrpc": "2.0", "id": 3, "method": "tools/call", "params": {
        "name": "comp_get", "arguments": { "comp": "Nowhere" } } }),
    );
    assert_eq!(missing["result"]["isError"], json!(true));
    let unknown = ask(json!({ "jsonrpc": "2.0", "id": 4, "method": "resources/list" }));
    assert_eq!(unknown["error"]["code"], json!(-32601));
}

// The live endpoint is the one way in from another program, so a program
// without the secret must get nothing, and one with it a working MCP.
#[test]
fn the_live_endpoint_answers_only_a_program_holding_the_secret() {
    use lumit_script::live;
    use std::io::{BufRead, Write};

    let dir = std::env::var_os("LUMIT_USER_DIRS")
        .map_or_else(std::env::temp_dir, PathBuf::from)
        .join(format!("live-{}", Uuid::now_v7()));
    let file = dir.join("endpoint");
    let (listener, secret) = live::open(&file).unwrap();
    let host = Bare(DocumentStore::new(Document::new()));
    let (stop, leave) = (AtomicBool::new(false), AtomicBool::new(false));

    // Stops the endpoint however the checks end, or a failed one would
    // leave the scope waiting on it for ever.
    struct Stop<'a>(&'a AtomicBool);
    impl Drop for Stop<'_> {
        fn drop(&mut self) {
            self.0.store(true, Ordering::Relaxed);
        }
    }

    std::thread::scope(|s| {
        s.spawn(|| {
            live::serve(
                &listener,
                &secret,
                &stop,
                &leave,
                &mut |_| {},
                &mut |line| lumit_script::mcp::answer(line, &host),
            );
        });
        let _stop = Stop(&stop);

        // The right name with a secret of its own is let go after its proof.
        let text = std::fs::read_to_string(&file).unwrap();
        let name = text.lines().next().unwrap();
        let wrong = dir.join("wrong");
        let other = lumit_peer::Secret::generate().unwrap().to_handover();
        std::fs::write(&wrong, format!("{name}\n{other}")).unwrap();
        assert_eq!(live::connect(&wrong).err(), Some(live::Unreached::Denied));

        let mut link = live::connect(&file).unwrap();
        writeln!(
            link.to,
            r#"{{"jsonrpc":"2.0","id":1,"method":"initialize","params":{{"protocolVersion":"2025-06-18","capabilities":{{}},"clientInfo":{{"name":"test","version":"1"}}}}}}"#
        )
        .unwrap();
        let mut reply = String::new();
        link.from.read_line(&mut reply).unwrap();
        let reply: Value = serde_json::from_str(&reply).unwrap();
        assert_eq!(reply["result"]["serverInfo"]["name"], json!("lumit"));
    });
    let _ = std::fs::remove_dir_all(&dir);
}

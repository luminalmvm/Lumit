//! The bridge v0.4 timeline-column ops: blend mode, matte, parent, the comp
//! motion-blur master, and adding a mask.
//!
//! # In plain terms
//!
//! These are the dropdowns and toggles that live in the Timeline's layer
//! columns and the comp header — the blend mode a layer composites with, the
//! layer it borrows as a matte, the layer it is parented to, the comp-wide
//! motion-blur shutter, and dropping a starter mask shape on a layer. Each one
//! routes through the same [`lumit_core::ops::Op`] the egui frontend commits
//! (`SetLayerBlend`, `SetLayerMatte`, `SetLayerParent`, `SetCompMotionBlur`,
//! `SetLayerMasks`), so undo is one clean step and the two frontends cannot
//! drift, and every success returns the full refreshed snapshot.

use crate::err_json;
use crate::state::{commit, parse_comp_layer, Bridge};
use lumit_core::model::{BlendMode, LayerInputSource, MatteChannel, MatteRef, MotionBlur};
use lumit_core::ops::Op;
use serde_json::{json, Value};
use uuid::Uuid;

/// The blend-mode registry as `[{name, label}]` — the layer dropdown's source of
/// truth ([`BlendMode::ALL`]). `name` is the serde variant name the ops take
/// (round-trip stable); `label` is the sentence-case display name. Stateless.
pub(crate) fn list_blend_modes() -> String {
    let modes: Vec<Value> = BlendMode::ALL
        .iter()
        .map(|m| {
            json!({
                "name": serde_json::to_value(m).unwrap_or(json!("Normal")),
                "label": m.name(),
            })
        })
        .collect();
    json!({ "ok": true, "blend_modes": modes }).to_string()
}

/// Parse a blend-mode name (the serde variant name, e.g. `Normal`, `ColourBurn`)
/// into a [`BlendMode`], routing through serde so the accepted set is exactly
/// what the read-back emits.
fn parse_blend_mode(name: &str) -> Option<BlendMode> {
    serde_json::from_value(Value::String(name.to_owned())).ok()
}

/// Set a layer's blend mode — the timeline Mode dropdown (`SetLayerBlend`).
pub(crate) fn set_blend_mode(
    bridge: &mut Bridge,
    comp_id: &str,
    layer_id: &str,
    mode: &str,
) -> String {
    let (comp, layer) = match parse_comp_layer(comp_id, layer_id) {
        Ok(pair) => pair,
        Err(e) => return err_json(format!("set blend mode: {e}")),
    };
    let Some(blend) = parse_blend_mode(mode) else {
        return err_json(format!("set blend mode: unknown blend mode '{mode}'"));
    };
    commit(
        bridge,
        Op::SetLayerBlend { comp, layer, blend },
        "set blend mode",
    )
}

/// Point a layer at another as its matte, or clear it when `source` is empty —
/// the timeline TrkMat dropdown (`SetLayerMatte`, K-142). `channel` is
/// `alpha`/`luma`; `inverted` flips the gate; the source mode defaults to
/// `EffectsAndMasks`, exactly as a fresh matte does in the egui inspector.
pub(crate) fn set_matte(
    bridge: &mut Bridge,
    comp_id: &str,
    layer_id: &str,
    source: &str,
    channel: &str,
    inverted: bool,
) -> String {
    let (comp, layer) = match parse_comp_layer(comp_id, layer_id) {
        Ok(pair) => pair,
        Err(e) => return err_json(format!("set matte: {e}")),
    };
    let matte = if source.trim().is_empty() {
        None
    } else {
        let src = match Uuid::parse_str(source) {
            Ok(id) => id,
            Err(_) => return err_json("set matte: source id is not a valid UUID"),
        };
        let channel = match channel {
            "alpha" => MatteChannel::Alpha,
            "luma" => MatteChannel::Luma,
            other => return err_json(format!("set matte: unknown channel '{other}'")),
        };
        Some(MatteRef {
            layer: src,
            channel,
            inverted,
            source: LayerInputSource::default(),
        })
    };
    commit(
        bridge,
        Op::SetLayerMatte { comp, layer, matte },
        "set matte",
    )
}

/// Point a layer at another as its transform parent, or clear it when `parent`
/// is empty — the timeline Parent dropdown (`SetLayerParent`). A self-parent or
/// a cycle is rejected by the op (`OpError::InvalidParent`), surfaced as a calm
/// error reply.
pub(crate) fn set_parent(
    bridge: &mut Bridge,
    comp_id: &str,
    layer_id: &str,
    parent: &str,
) -> String {
    let (comp, layer) = match parse_comp_layer(comp_id, layer_id) {
        Ok(pair) => pair,
        Err(e) => return err_json(format!("set parent: {e}")),
    };
    let parent = if parent.trim().is_empty() {
        None
    } else {
        match Uuid::parse_str(parent) {
            Ok(id) => Some(id),
            Err(_) => return err_json("set parent: parent id is not a valid UUID"),
        }
    };
    commit(
        bridge,
        Op::SetLayerParent {
            comp,
            layer,
            parent,
        },
        "set parent",
    )
}

/// Set the comp's motion-blur master — the comp header's shutter controls
/// (`SetCompMotionBlur`, K-120). `enabled` is the master; the shutter angle and
/// phase are degrees; `samples` is clamped to the model's 2..=`MAX_SAMPLES`
/// working range (the UI's own control clamps 2..64, but the op accepts any and
/// the offset budget enforces the hard ceiling).
pub(crate) fn set_motion_blur(
    bridge: &mut Bridge,
    comp_id: &str,
    enabled: bool,
    shutter_angle: f64,
    shutter_phase: f64,
    samples: u32,
) -> String {
    let comp = match Uuid::parse_str(comp_id) {
        Ok(id) => id,
        Err(_) => return err_json("set motion blur: composition id is not a valid UUID"),
    };
    let motion_blur = MotionBlur {
        enabled,
        shutter_angle,
        shutter_phase,
        samples: samples.clamp(2, MotionBlur::MAX_SAMPLES),
    };
    commit(
        bridge,
        Op::SetCompMotionBlur { comp, motion_blur },
        "set motion blur",
    )
}

/// Add a starter mask shape to a layer — the timeline "Add mask" menu
/// (`add_mask_to_selected`): a rectangle, ellipse or star centred in the layer's
/// mask space, appended to the stack as one `SetLayerMasks`. The mask space here
/// is the comp's own dimensions (the same fractions the egui menu uses); the
/// per-layer natural-size refinement is a later phase.
pub(crate) fn add_mask(bridge: &mut Bridge, comp_id: &str, layer_id: &str, kind: &str) -> String {
    let (comp, layer) = match parse_comp_layer(comp_id, layer_id) {
        Ok(pair) => pair,
        Err(e) => return err_json(format!("add mask: {e}")),
    };
    let doc = bridge.store.snapshot();
    let Some(c) = doc.comp(comp) else {
        return err_json("add mask: unknown composition");
    };
    let Some(l) = c.layers.iter().find(|l| l.id == layer) else {
        return err_json("add mask: unknown layer");
    };
    let (w, h) = (f64::from(c.width), f64::from(c.height));
    let mask = match kind {
        "rectangle" => lumit_core::mask::Mask::rectangle(w * 0.25, h * 0.25, w * 0.5, h * 0.5),
        "ellipse" => lumit_core::mask::Mask::ellipse(w * 0.5, h * 0.5, w * 0.3, h * 0.3),
        "star" => lumit_core::mask::Mask::star(w * 0.5, h * 0.5, w * 0.32, w * 0.14, 5),
        other => return err_json(format!("add mask: unknown mask kind '{other}'")),
    };
    let mut masks = l.masks.clone();
    masks.push(mask);
    commit(bridge, Op::SetLayerMasks { comp, layer, masks }, "add mask")
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use crate::edits::{add_camera_layer, add_solid_layer};
    use crate::state::{new_composition, snapshot};
    use lumit_core::model::ProjectItem;
    use serde_json::Value;

    fn parse(s: &str) -> Value {
        serde_json::from_str(s).expect("reply is valid JSON")
    }

    /// A comp with one camera layer, returning bridge, comp id, layer id.
    fn comp_with_layer() -> (Bridge, String, String) {
        let mut b = Bridge::new();
        new_composition(&mut b, "Scene");
        let comp_id = b
            .store
            .snapshot()
            .items
            .iter()
            .find_map(|i| match i {
                ProjectItem::Composition(c) => Some(c.id),
                _ => None,
            })
            .expect("a comp exists")
            .to_string();
        add_camera_layer(&mut b, &comp_id);
        let snap = parse(&snapshot(&b));
        let layer_id = first_layer(&snap)["id"].as_str().unwrap().to_owned();
        (b, comp_id, layer_id)
    }

    fn find_comp(snap: &Value) -> Value {
        for item in snap["items"].as_array().unwrap() {
            if item["kind"] == json!("composition") {
                return item.clone();
            }
            for child in item["children"].as_array().unwrap() {
                if child["kind"] == json!("composition") {
                    return child.clone();
                }
            }
        }
        panic!("no composition in snapshot");
    }

    fn first_layer(snap: &Value) -> Value {
        find_comp(snap)["comp"]["layers"][0].clone()
    }

    #[test]
    fn list_blend_modes_names_every_mode() {
        let reply = parse(&list_blend_modes());
        assert_eq!(reply["ok"], json!(true));
        let modes = reply["blend_modes"].as_array().unwrap();
        assert_eq!(modes.len(), BlendMode::ALL.len());
        assert_eq!(modes[0]["name"], json!("Normal"));
        assert!(modes
            .iter()
            .all(|m| m["name"].is_string() && m["label"].is_string()));
    }

    #[test]
    fn set_blend_mode_round_trips_and_undoes() {
        let (mut b, comp, layer) = comp_with_layer();
        let snap = parse(&set_blend_mode(&mut b, &comp, &layer, "Multiply"));
        assert_eq!(snap["ok"], json!(true));
        assert_eq!(first_layer(&snap)["blend_mode"], json!("Multiply"));
        let after = parse(&crate::state::undo(&mut b));
        assert_eq!(first_layer(&after)["blend_mode"], json!("Normal"));
    }

    #[test]
    fn set_blend_mode_rejects_an_unknown_mode() {
        let (mut b, comp, layer) = comp_with_layer();
        let reply = parse(&set_blend_mode(&mut b, &comp, &layer, "Nebula"));
        assert_eq!(reply["ok"], json!(false));
        assert!(reply["error"]
            .as_str()
            .unwrap()
            .contains("unknown blend mode"));
    }

    #[test]
    fn set_matte_points_and_clears() {
        // Two layers: the top one gets the bottom as its luma matte.
        let (mut b, comp, top) = comp_with_layer();
        add_solid_layer(&mut b, &comp);
        let snap = parse(&snapshot(&b));
        // The solid is now index 0; the camera moved to index 1. Use the solid as
        // the matte source for the camera.
        let layers = find_comp(&snap)["comp"]["layers"]
            .as_array()
            .unwrap()
            .clone();
        let source = layers[0]["id"].as_str().unwrap().to_owned();
        let _ = top; // the camera id changes index but keeps its uuid
        let camera = layers
            .iter()
            .find(|l| l["kind"] == json!("camera"))
            .unwrap()["id"]
            .as_str()
            .unwrap()
            .to_owned();
        let snap = parse(&set_matte(&mut b, &comp, &camera, &source, "luma", true));
        assert_eq!(snap["ok"], json!(true));
        let cam = find_comp(&snap)["comp"]["layers"]
            .as_array()
            .unwrap()
            .iter()
            .find(|l| l["id"] == json!(camera))
            .unwrap()
            .clone();
        assert_eq!(cam["matte"]["source"], json!(source));
        assert_eq!(cam["matte"]["channel"], json!("luma"));
        assert_eq!(cam["matte"]["inverted"], json!(true));
        // Clearing removes it.
        let snap = parse(&set_matte(&mut b, &comp, &camera, "", "alpha", false));
        let cam = find_comp(&snap)["comp"]["layers"]
            .as_array()
            .unwrap()
            .iter()
            .find(|l| l["id"] == json!(camera))
            .unwrap()
            .clone();
        assert_eq!(cam["matte"], Value::Null);
    }

    #[test]
    fn set_parent_points_and_clears() {
        let (mut b, comp, child) = comp_with_layer();
        add_solid_layer(&mut b, &comp);
        let snap = parse(&snapshot(&b));
        let layers = find_comp(&snap)["comp"]["layers"]
            .as_array()
            .unwrap()
            .clone();
        let parent = layers[0]["id"].as_str().unwrap().to_owned();
        let snap = parse(&set_parent(&mut b, &comp, &child, &parent));
        assert_eq!(snap["ok"], json!(true));
        let c = find_comp(&snap)["comp"]["layers"]
            .as_array()
            .unwrap()
            .iter()
            .find(|l| l["id"] == json!(child))
            .unwrap()
            .clone();
        assert_eq!(c["parent"], json!(parent));
        let snap = parse(&set_parent(&mut b, &comp, &child, ""));
        let c = find_comp(&snap)["comp"]["layers"]
            .as_array()
            .unwrap()
            .iter()
            .find(|l| l["id"] == json!(child))
            .unwrap()
            .clone();
        assert_eq!(c["parent"], Value::Null);
    }

    #[test]
    fn set_motion_blur_reads_back_and_clamps_samples() {
        let (mut b, comp, _layer) = comp_with_layer();
        let snap = parse(&set_motion_blur(&mut b, &comp, true, 200.0, -80.0, 1));
        assert_eq!(snap["ok"], json!(true));
        let mb = find_comp(&snap)["comp"]["motion_blur"].clone();
        assert_eq!(mb["enabled"], json!(true));
        assert_eq!(mb["shutter_angle"], json!(200.0));
        assert_eq!(mb["shutter_phase"], json!(-80.0));
        // 1 clamps up to the working minimum of 2.
        assert_eq!(mb["samples"], json!(2));
    }

    #[test]
    fn add_mask_appends_each_kind() {
        for (kind, expect) in [("rectangle", 4), ("ellipse", 4), ("star", 10)] {
            let (mut b, comp, layer) = comp_with_layer();
            let snap = parse(&add_mask(&mut b, &comp, &layer, kind));
            assert_eq!(snap["ok"], json!(true), "kind {kind}");
            // The mask landed on the layer in the model (the snapshot does not
            // surface mask geometry yet, so read the store).
            let doc = b.store.snapshot();
            let cid = Uuid::parse_str(&comp).unwrap();
            let lid = Uuid::parse_str(&layer).unwrap();
            let l = doc
                .comp(cid)
                .unwrap()
                .layers
                .iter()
                .find(|l| l.id == lid)
                .unwrap();
            assert_eq!(l.masks.len(), 1, "kind {kind} added one mask");
            assert_eq!(
                l.masks[0].path.vertices.len(),
                expect,
                "kind {kind} vertex count"
            );
        }
    }

    #[test]
    fn add_mask_rejects_an_unknown_kind() {
        let (mut b, comp, layer) = comp_with_layer();
        let reply = parse(&add_mask(&mut b, &comp, &layer, "hexagon"));
        assert_eq!(reply["ok"], json!(false));
        assert!(reply["error"]
            .as_str()
            .unwrap()
            .contains("unknown mask kind"));
    }
}

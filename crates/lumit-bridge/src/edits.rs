//! The bridge v0.3 edit operations: layer lifecycle, comp settings, keyframes,
//! the work area, and effects.
//!
//! # In plain terms
//!
//! These are the actions the Flutter panels need beyond v0.2's switches and
//! single transform sets: adding and removing layers of every kind, editing a
//! composition's settings, keyframing a property (the stopwatch and the
//! add/remove/shift a Timeline needs), moving the work area, and applying
//! effects. Every one routes through the same [`lumit_core::ops::Op`] path the
//! egui frontend commits, so undo/redo is one clean step and the two frontends
//! can never drift — and every success returns the full refreshed snapshot, the
//! same wholesale-re-read contract v0.2 established.

use crate::err_json;
use crate::state::{commit, parse_comp_layer, parse_transform_prop, Bridge};
use lumit_core::anim::{Animation, Keyframe, SideInterp};
use lumit_core::model::{
    Composition, EffectInstance, EffectValue, Folder, Layer, LayerKind, LinearColour, ProjectItem,
    SolidDef, Switches, TextDocument, TransformGroup, TransformProp,
};
use lumit_core::ops::{AutoFolderKind, Op};
use lumit_core::time::{CompTime, Duration, FrameRate, Rational};
use serde_json::{json, Value};
use uuid::Uuid;

// ---------------------------------------------------------------------------
// Shared helpers.
// ---------------------------------------------------------------------------

/// A transform anchored on the content's own centre and placed at the comp
/// centre — `lumit-ui`'s `centred_transform`, the seeding every add-layer path
/// uses so a fresh layer appears centred and pivots about its middle (K-150).
fn centred_transform(nat_w: f64, nat_h: f64, comp_w: u32, comp_h: u32) -> TransformGroup {
    use lumit_core::anim::Property;
    TransformGroup {
        anchor_x: Property::fixed(nat_w * 0.5),
        anchor_y: Property::fixed(nat_h * 0.5),
        position_x: Property::fixed(f64::from(comp_w) * 0.5),
        position_y: Property::fixed(f64::from(comp_h) * 0.5),
        ..TransformGroup::default()
    }
}

/// A layer with the house defaults every add path shares, given the parts that
/// differ (name, kind, span end, transform). The span starts at comp 0 and the
/// switches are the model defaults — exactly as the egui add-layer paths build.
fn base_layer(name: String, kind: LayerKind, out: Rational, transform: TransformGroup) -> Layer {
    Layer {
        id: Uuid::now_v7(),
        name,
        kind,
        in_point: CompTime(Rational::ZERO),
        out_point: CompTime(out),
        start_offset: CompTime(Rational::ZERO),
        transform,
        matte: None,
        parent: None,
        label: 0,
        volume_db: lumit_core::anim::Property::zero(),
        blend: lumit_core::model::BlendMode::Normal,
        masks: Vec::new(),
        effects: Vec::new(),
        switches: Switches::default(),
        extra: serde_json::Map::new(),
    }
}

/// Parse a composition id, returning it and a clone of the composition, or a
/// calm error prefixed with `ctx`.
fn resolve_comp(bridge: &Bridge, comp_id: &str, ctx: &str) -> Result<(Uuid, Composition), String> {
    let id = Uuid::parse_str(comp_id)
        .map_err(|_| format!("{ctx}: composition id is not a valid UUID"))?;
    let doc = bridge.store.snapshot();
    match doc.comp(id) {
        Some(c) => Ok((id, c.clone())),
        None => Err(format!("{ctx}: unknown composition")),
    }
}

/// Add `layer` at the top of `comp` (index 0), committing one undoable
/// [`Op::AddLayer`]. The shared tail of every simple add-layer path.
fn add_top_layer(bridge: &mut Bridge, comp: Uuid, layer: Layer, ctx: &str) -> String {
    commit(
        bridge,
        Op::AddLayer {
            comp,
            index: 0,
            layer: Box::new(layer),
        },
        ctx,
    )
}

/// The ops that guarantee `kind`'s auto-filing folder exists, plus its id —
/// `lumit-ui`'s `ensure_auto_folder_ops`, tracked by id so renaming or nesting
/// the folder keeps the habit.
fn ensure_auto_folder_ops(
    doc: &lumit_core::model::Document,
    kind: AutoFolderKind,
) -> (Uuid, Vec<Op>) {
    let slot = match kind {
        AutoFolderKind::Solids => doc.auto_folders.solids,
        AutoFolderKind::Compositions => doc.auto_folders.compositions,
    };
    if let Some(id) = slot {
        if doc.folder(id).is_some() {
            return (id, Vec::new());
        }
    }
    let id = Uuid::now_v7();
    let name = match kind {
        AutoFolderKind::Solids => "Solids",
        AutoFolderKind::Compositions => "Compositions",
    };
    (
        id,
        vec![
            Op::AddItem {
                index: doc.items.len(),
                item: Box::new(ProjectItem::Folder(Folder {
                    id,
                    name: name.into(),
                    children: Vec::new(),
                    extra: serde_json::Map::new(),
                })),
            },
            Op::SetAutoFolder {
                kind,
                folder: Some(id),
            },
        ],
    )
}

/// The op that files `item` into `folder` (appended). The folder may have been
/// created earlier in the same batch, so its children start empty then.
fn file_into_folder_op(doc: &lumit_core::model::Document, folder: Uuid, item: Uuid) -> Op {
    let mut children = doc
        .folder(folder)
        .map(|f| f.children.clone())
        .unwrap_or_default();
    children.push(item);
    Op::SetFolderChildren { folder, children }
}

// ---------------------------------------------------------------------------
// Item 3: layer lifecycle.
// ---------------------------------------------------------------------------

/// Add a Solid layer backed by a fresh `SolidDef` asset filed in the Solids
/// auto-folder — `lumit-ui`'s `add_solid_layer`, one batch / one undo step. The
/// solid is comp-sized and white, named "White solid N".
pub(crate) fn add_solid_layer(bridge: &mut Bridge, comp_id: &str) -> String {
    let (comp, c) = match resolve_comp(bridge, comp_id, "add solid layer") {
        Ok(pair) => pair,
        Err(e) => return err_json(e),
    };
    let doc = bridge.store.snapshot();
    let (folder_id, mut ops) = ensure_auto_folder_ops(&doc, AutoFolderKind::Solids);
    let def_id = Uuid::now_v7();
    let n_solids = doc
        .items
        .iter()
        .filter(|i| matches!(i, ProjectItem::Solid(_)))
        .count();
    let name = format!("White solid {}", n_solids + 1);
    let added = ops
        .iter()
        .filter(|o| matches!(o, Op::AddItem { .. }))
        .count();
    ops.push(Op::AddItem {
        index: doc.items.len() + added,
        item: Box::new(ProjectItem::Solid(SolidDef {
            id: def_id,
            name: name.clone(),
            colour: LinearColour([1.0, 1.0, 1.0, 1.0]),
            width: c.width,
            height: c.height,
            extra: serde_json::Map::new(),
        })),
    });
    ops.push(file_into_folder_op(&doc, folder_id, def_id));
    let layer = base_layer(
        name,
        LayerKind::Solid { def: def_id },
        c.duration.0,
        centred_transform(f64::from(c.width), f64::from(c.height), c.width, c.height),
    );
    ops.push(Op::AddLayer {
        comp,
        index: 0,
        layer: Box::new(layer),
    });
    commit(bridge, Op::Batch { ops }, "add solid layer")
}

/// Add a Text layer with the "Text" starter document — `lumit-ui`'s
/// `add_text_layer` (size 72, white fill, anchor on the estimated glyph bounds,
/// placed at the comp centre).
pub(crate) fn add_text_layer(bridge: &mut Bridge, comp_id: &str) -> String {
    use lumit_core::anim::Property;
    let (comp, c) = match resolve_comp(bridge, comp_id, "add text layer") {
        Ok(pair) => pair,
        Err(e) => return err_json(e),
    };
    let text = "Text";
    let size = 72.0_f64;
    let est_w = text.chars().count() as f64 * size * 0.5;
    let transform = TransformGroup {
        anchor_x: Property::fixed(est_w * 0.5),
        anchor_y: Property::fixed(size * 0.5),
        position_x: Property::fixed(f64::from(c.width) * 0.5),
        position_y: Property::fixed(f64::from(c.height) * 0.5),
        ..TransformGroup::default()
    };
    let layer = base_layer(
        "Text".into(),
        LayerKind::Text {
            document: TextDocument {
                text: text.into(),
                size,
                fill: LinearColour([1.0, 1.0, 1.0, 1.0]),
                extra: serde_json::Map::new(),
            },
        },
        c.duration.0,
        transform,
    );
    add_top_layer(bridge, comp, layer, "add text layer")
}

/// Add a Camera layer at the comp centre — `lumit-ui`'s `add_camera_layer`. The
/// default zoom is the AE 50 mm model, `comp width × 50/36`.
pub(crate) fn add_camera_layer(bridge: &mut Bridge, comp_id: &str) -> String {
    use lumit_core::anim::Property;
    let (comp, c) = match resolve_comp(bridge, comp_id, "add camera layer") {
        Ok(pair) => pair,
        Err(e) => return err_json(e),
    };
    let transform = TransformGroup {
        position_x: Property::fixed(f64::from(c.width) * 0.5),
        position_y: Property::fixed(f64::from(c.height) * 0.5),
        ..TransformGroup::default()
    };
    let layer = base_layer(
        "Camera".into(),
        LayerKind::Camera {
            zoom: Property::fixed(f64::from(c.width) * 50.0 / 36.0),
        },
        c.duration.0,
        transform,
    );
    add_top_layer(bridge, comp, layer, "add camera layer")
}

/// Add an Adjustment layer at the top — `lumit-ui`'s `add_adjustment_layer`. A
/// comp-sized effect container, centred so scale/rotation pivot about the middle.
pub(crate) fn add_adjustment_layer(bridge: &mut Bridge, comp_id: &str) -> String {
    let (comp, c) = match resolve_comp(bridge, comp_id, "add adjustment layer") {
        Ok(pair) => pair,
        Err(e) => return err_json(e),
    };
    let layer = base_layer(
        "Adjustment".into(),
        LayerKind::Adjustment,
        c.duration.0,
        centred_transform(f64::from(c.width), f64::from(c.height), c.width, c.height),
    );
    add_top_layer(bridge, comp, layer, "add adjustment layer")
}

/// Add an (empty) Sequence layer — `lumit-ui`'s `add_sequence_layer` with no
/// footage selected: a "Sequence" clip row spanning the comp, centred.
pub(crate) fn add_sequence_layer(bridge: &mut Bridge, comp_id: &str) -> String {
    let (comp, c) = match resolve_comp(bridge, comp_id, "add sequence layer") {
        Ok(pair) => pair,
        Err(e) => return err_json(e),
    };
    let layer = base_layer(
        "Sequence".into(),
        LayerKind::Sequence { clips: Vec::new() },
        c.duration.0,
        centred_transform(f64::from(c.width), f64::from(c.height), c.width, c.height),
    );
    add_top_layer(bridge, comp, layer, "add sequence layer")
}

/// Delete a layer from its composition (one [`Op::RemoveLayer`], undoable).
pub(crate) fn delete_layer(bridge: &mut Bridge, comp_id: &str, layer_id: &str) -> String {
    let (comp, layer) = match parse_comp_layer(comp_id, layer_id) {
        Ok(pair) => pair,
        Err(e) => return err_json(format!("delete layer: {e}")),
    };
    commit(bridge, Op::RemoveLayer { comp, layer }, "delete layer")
}

/// Duplicate a layer — `lumit-ui`'s `duplicate_layer`: an exact copy with a
/// fresh id, a "… copy" name and fresh effect-instance ids, inserted directly
/// above the original (one [`Op::AddLayer`]).
pub(crate) fn duplicate_layer(bridge: &mut Bridge, comp_id: &str, layer_id: &str) -> String {
    let (comp, layer) = match parse_comp_layer(comp_id, layer_id) {
        Ok(pair) => pair,
        Err(e) => return err_json(format!("duplicate layer: {e}")),
    };
    let doc = bridge.store.snapshot();
    let Some(c) = doc.comp(comp) else {
        return err_json("duplicate layer: unknown composition");
    };
    let Some(pos) = c.layers.iter().position(|l| l.id == layer) else {
        return err_json("duplicate layer: unknown layer");
    };
    let mut copy = c.layers[pos].clone();
    copy.id = Uuid::now_v7();
    copy.name = format!("{} copy", copy.name);
    for e in &mut copy.effects {
        e.id = Uuid::now_v7();
    }
    commit(
        bridge,
        Op::AddLayer {
            comp,
            index: pos,
            layer: Box::new(copy),
        },
        "duplicate layer",
    )
}

// ---------------------------------------------------------------------------
// Item 4: comp settings.
// ---------------------------------------------------------------------------

/// Edit a composition's settings — the AE Composition Settings dialogue that
/// `lumit-ui`'s `confirm_comp_dialog` commits. One [`Op::SetCompSettings`] (so
/// undo is one step); the background is preserved. `duration_frames` is the comp
/// length in whole frames at the given rate; width/height clamp to 16..16384.
#[allow(clippy::too_many_arguments)]
pub(crate) fn set_comp_settings(
    bridge: &mut Bridge,
    comp_id: &str,
    name: &str,
    width: u32,
    height: u32,
    fps_num: i64,
    fps_den: i64,
    duration_frames: i64,
) -> String {
    let (comp, c) = match resolve_comp(bridge, comp_id, "set comp settings") {
        Ok(pair) => pair,
        Err(e) => return err_json(e),
    };
    let (Ok(fps_num), Ok(fps_den)) = (u32::try_from(fps_num), u32::try_from(fps_den)) else {
        return err_json("set comp settings: frame rate must be positive");
    };
    let Ok(frame_rate) = FrameRate::new(fps_num, fps_den) else {
        return err_json("set comp settings: invalid frame rate");
    };
    let duration = match frame_rate.time_of_frame(duration_frames.max(1)) {
        Ok(t) => Duration(t.0),
        Err(e) => return err_json(format!("set comp settings: {e}")),
    };
    commit(
        bridge,
        Op::SetCompSettings {
            comp,
            name: name.to_owned(),
            width: width.clamp(16, 16384),
            height: height.clamp(16, 16384),
            frame_rate,
            duration,
            background: c.background,
        },
        "set comp settings",
    )
}

// ---------------------------------------------------------------------------
// Item 5: keyframes.
// ---------------------------------------------------------------------------

/// The layer-local time (seconds) a comp `frame` maps to on `layer`, using the
/// comp's own rate: `frame / fps − start_offset`. Layer-local time is where
/// transform keyframes and Retime boundaries live (the egui frontend's
/// convention); shared with [`crate::retime`] so both speak one clock.
pub(crate) fn layer_local_seconds(c: &Composition, layer: &Layer, frame: i64) -> f64 {
    let fps = c.frame_rate.fps().max(1.0);
    frame as f64 / fps - layer.start_offset.0.to_f64()
}

/// A layer-local time (seconds, clamped ≥ 0) as a rational on the flick grid —
/// `lumit-ui`'s `rational_at`, so bridge-made keyframes land on the same grid.
pub(crate) fn rational_at(seconds: f64) -> Rational {
    Rational::from_f64_on_grid(seconds.max(0.0), Rational::FLICK_DEN).unwrap_or(Rational::ZERO)
}

/// Resolve a comp, layer and transform property together for a keyframe op,
/// returning the comp (cloned), the layer's index, and the property.
fn resolve_prop(
    bridge: &Bridge,
    comp_id: &str,
    layer_id: &str,
    property: &str,
    ctx: &str,
) -> Result<(Uuid, Composition, Layer, TransformProp), String> {
    let (comp, layer) = parse_comp_layer(comp_id, layer_id).map_err(|e| format!("{ctx}: {e}"))?;
    let prop = parse_transform_prop(property)
        .ok_or_else(|| format!("{ctx}: unknown property '{property}'"))?;
    let doc = bridge.store.snapshot();
    let c = doc
        .comp(comp)
        .cloned()
        .ok_or_else(|| format!("{ctx}: unknown composition"))?;
    let l = c
        .layers
        .iter()
        .find(|l| l.id == layer)
        .cloned()
        .ok_or_else(|| format!("{ctx}: unknown layer"))?;
    Ok((comp, c, l, prop))
}

/// Commit a transform property's new animation as one [`Op::SetTransformProperty`].
fn commit_property(
    bridge: &mut Bridge,
    comp: Uuid,
    layer: Uuid,
    prop: TransformProp,
    animation: Animation,
    ctx: &str,
) -> String {
    commit(
        bridge,
        Op::SetTransformProperty {
            comp,
            layer,
            prop,
            animation,
        },
        ctx,
    )
}

/// The stopwatch: on enable seed a key at the playhead holding the current
/// value; on disable collapse to a static at the current evaluated value —
/// `lumit-ui`'s `stopwatch`. Toggles by reading whether the property is already
/// animated.
pub(crate) fn toggle_property_animated(
    bridge: &mut Bridge,
    comp_id: &str,
    layer_id: &str,
    property: &str,
    frame: i64,
) -> String {
    let ctx = "toggle property animated";
    let (comp, c, l, prop) = match resolve_prop(bridge, comp_id, layer_id, property, ctx) {
        Ok(t) => t,
        Err(e) => return err_json(e),
    };
    let lt = layer_local_seconds(&c, &l, frame);
    let slot = l.transform.get(prop);
    let animation = if slot.is_animated() {
        Animation::Static(slot.value_at(lt))
    } else {
        Animation::Keyframed(vec![Keyframe {
            time: rational_at(lt),
            value: slot.value_at(lt),
            interp_in: SideInterp::Linear,
            interp_out: SideInterp::Linear,
        }])
    };
    commit_property(bridge, comp, l.id, prop, animation, ctx)
}

/// Insert or replace a keyframe at the playhead `frame` with `value` —
/// `lumit-ui`'s `upsert_key` (half-frame tolerance, sorted, Linear sides for a
/// fresh key). A static property becomes keyframed with this one key.
pub(crate) fn add_keyframe(
    bridge: &mut Bridge,
    comp_id: &str,
    layer_id: &str,
    property: &str,
    frame: i64,
    value: f64,
) -> String {
    let ctx = "add keyframe";
    let (comp, c, l, prop) = match resolve_prop(bridge, comp_id, layer_id, property, ctx) {
        Ok(t) => t,
        Err(e) => return err_json(e),
    };
    let lt = layer_local_seconds(&c, &l, frame);
    let slot = l.transform.get(prop);
    let mut keys = match &slot.animation {
        Animation::Keyframed(k) => k.clone(),
        Animation::Static(v) => vec![Keyframe {
            time: rational_at(0.0),
            value: *v,
            interp_in: SideInterp::Linear,
            interp_out: SideInterp::Linear,
        }],
    };
    const EPS: f64 = 1.0 / 240.0;
    if let Some(existing) = keys.iter_mut().find(|k| (k.time.to_f64() - lt).abs() < EPS) {
        existing.value = value;
    } else {
        keys.push(Keyframe {
            time: rational_at(lt),
            value,
            interp_in: SideInterp::Linear,
            interp_out: SideInterp::Linear,
        });
        keys.sort_by_key(|k| k.time);
    }
    commit_property(bridge, comp, l.id, prop, Animation::Keyframed(keys), ctx)
}

/// Remove the keyframe at the playhead `frame`. When it was the last key the
/// property collapses to a static at the value there (mirrors the egui delete);
/// otherwise the remaining keys stay. A no-op (no key there) still refreshes.
pub(crate) fn remove_keyframe(
    bridge: &mut Bridge,
    comp_id: &str,
    layer_id: &str,
    property: &str,
    frame: i64,
) -> String {
    let ctx = "remove keyframe";
    let (comp, c, l, prop) = match resolve_prop(bridge, comp_id, layer_id, property, ctx) {
        Ok(t) => t,
        Err(e) => return err_json(e),
    };
    let lt = layer_local_seconds(&c, &l, frame);
    let slot = l.transform.get(prop);
    let Animation::Keyframed(keys) = &slot.animation else {
        return err_json("remove keyframe: property is not animated");
    };
    let fps = c.frame_rate.fps().max(1.0);
    let tol = 0.5 / fps;
    let kept: Vec<Keyframe> = keys
        .iter()
        .copied()
        .filter(|k| (k.time.to_f64() - lt).abs() >= tol)
        .collect();
    let animation = if kept.is_empty() {
        Animation::Static(slot.value_at(lt))
    } else {
        Animation::Keyframed(kept)
    };
    commit_property(bridge, comp, l.id, prop, animation, ctx)
}

/// Slide the keyframes at comp `frames` by `delta` frames — the Timeline lane's
/// batched shift (`lumit-ui`'s `shift_keys_time`): matched keys move by
/// `delta / fps` seconds (interp preserved), the rest stay, sorted and
/// deduped. `frames_json` is a JSON array of comp frame indices.
pub(crate) fn shift_keyframes(
    bridge: &mut Bridge,
    comp_id: &str,
    layer_id: &str,
    property: &str,
    frames_json: &str,
    delta: i64,
) -> String {
    let ctx = "shift keyframes";
    let frames: Vec<i64> = match serde_json::from_str(frames_json) {
        Ok(v) => v,
        Err(_) => return err_json("shift keyframes: frames must be a JSON array of integers"),
    };
    let (comp, c, l, prop) = match resolve_prop(bridge, comp_id, layer_id, property, ctx) {
        Ok(t) => t,
        Err(e) => return err_json(e),
    };
    let slot = l.transform.get(prop);
    let Animation::Keyframed(keys) = &slot.animation else {
        return err_json("shift keyframes: property is not animated");
    };
    let fps = c.frame_rate.fps().max(1.0);
    let tol = 0.5 / fps;
    let delta_secs = delta as f64 / fps;
    // The layer-local times to move (each requested comp frame, mapped back).
    let move_times: Vec<f64> = frames
        .iter()
        .map(|f| layer_local_seconds(&c, &l, *f))
        .collect();
    let mut out: Vec<Keyframe> = keys
        .iter()
        .map(|k| {
            let moved = move_times.iter().any(|t| (t - k.time.to_f64()).abs() < tol);
            if moved {
                Keyframe {
                    time: rational_at(k.time.to_f64() + delta_secs),
                    ..*k
                }
            } else {
                *k
            }
        })
        .collect();
    out.sort_by_key(|k| k.time);
    out.dedup_by(|a, b| a.time == b.time);
    commit_property(bridge, comp, l.id, prop, Animation::Keyframed(out), ctx)
}

/// Parse a [`SideInterp`] variant name (`Hold`/`Linear`/`Bezier`), building a
/// `Bezier` from the supplied `speed`/`influence` when named. Any other name is
/// an error. Shared with the retime ease parser only in spirit — this is the
/// transform-graph interpolation vocabulary the snapshot read-back speaks.
pub(crate) fn parse_side_interp(
    name: &str,
    speed: f64,
    influence: f64,
) -> Result<SideInterp, String> {
    match name {
        "Hold" => Ok(SideInterp::Hold),
        "Linear" => Ok(SideInterp::Linear),
        "Bezier" => Ok(SideInterp::Bezier { speed, influence }),
        other => Err(format!("unknown interpolation '{other}'")),
    }
}

/// Set the interpolation of the keyframe nearest the playhead `frame` on a
/// transform property — the graph/inspector's interp edit, committed as one
/// [`Op::SetTransformProperty`] replacing the whole animation (the same
/// coarse-grained, exactly-invertible shape every keyframe edit uses). Each
/// side takes `interp_in`/`interp_out` (`Hold`/`Linear`/`Bezier`); when a side
/// is `Bezier` its `(speed, influence)` come from the matching pair. A no-op
/// (property not animated, or no key at the playhead) is a calm error.
#[allow(clippy::too_many_arguments)]
pub(crate) fn set_keyframe_interp(
    bridge: &mut Bridge,
    comp_id: &str,
    layer_id: &str,
    property: &str,
    frame: i64,
    interp_in: &str,
    interp_out: &str,
    speed_in: f64,
    influence_in: f64,
    speed_out: f64,
    influence_out: f64,
) -> String {
    let ctx = "set keyframe interp";
    let (comp, c, l, prop) = match resolve_prop(bridge, comp_id, layer_id, property, ctx) {
        Ok(t) => t,
        Err(e) => return err_json(e),
    };
    let side_in = match parse_side_interp(interp_in, speed_in, influence_in) {
        Ok(s) => s,
        Err(e) => return err_json(format!("{ctx}: {e}")),
    };
    let side_out = match parse_side_interp(interp_out, speed_out, influence_out) {
        Ok(s) => s,
        Err(e) => return err_json(format!("{ctx}: {e}")),
    };
    let lt = layer_local_seconds(&c, &l, frame);
    let slot = l.transform.get(prop);
    let Animation::Keyframed(keys) = &slot.animation else {
        return err_json("set keyframe interp: property is not animated");
    };
    let fps = c.frame_rate.fps().max(1.0);
    let tol = 0.5 / fps;
    let mut keys = keys.clone();
    let Some(k) = keys.iter_mut().find(|k| (k.time.to_f64() - lt).abs() < tol) else {
        return err_json("set keyframe interp: no keyframe at the playhead");
    };
    k.interp_in = side_in;
    k.interp_out = side_out;
    commit_property(bridge, comp, l.id, prop, Animation::Keyframed(keys), ctx)
}

// ---------------------------------------------------------------------------
// Item 6: work area.
// ---------------------------------------------------------------------------

/// Set one work-area edge to the playhead `frame` — `lumit-ui`'s
/// `set_work_area_edge` (the B / N keys). `is_out` chooses the out edge; the
/// other edge is preserved (or snaps to the comp bound if the span would
/// invert). A span covering the whole comp clears the work area (None).
pub(crate) fn set_work_area_edge(
    bridge: &mut Bridge,
    comp_id: &str,
    frame: i64,
    is_out: bool,
) -> String {
    let ctx = "set work area edge";
    let (comp, c) = match resolve_comp(bridge, comp_id, ctx) {
        Ok(pair) => pair,
        Err(e) => return err_json(e),
    };
    let fps = c.frame_rate.fps().max(1.0);
    let t = frame as f64 / fps;
    let dur = c.duration.0.to_f64();
    let (mut a, mut b) = c
        .work_area
        .map(|(a, b)| (a.0.to_f64(), b.0.to_f64()))
        .unwrap_or((0.0, dur));
    if is_out {
        b = (t + 1.0 / fps).min(dur);
        if a >= b {
            a = 0.0;
        }
    } else {
        a = t.min(dur - 1.0 / fps);
        if b <= a {
            b = dur;
        }
    }
    let work_area = if a <= 0.0 && (b - dur).abs() < 1e-9 {
        None
    } else {
        Some((
            CompTime(rational_at(a)),
            CompTime(Rational::from_f64_on_grid(b, Rational::FLICK_DEN).unwrap_or(c.duration.0)),
        ))
    };
    commit(bridge, Op::SetWorkArea { comp, work_area }, ctx)
}

// ---------------------------------------------------------------------------
// Item 7: effects.
// ---------------------------------------------------------------------------

/// The built-in effect registry as `[{name, label}]` — the Add-effect menu's
/// source of truth ([`lumit_core::fx::BUILTINS`]). Stateless.
pub(crate) fn list_effects() -> String {
    let effects: Vec<Value> = lumit_core::fx::BUILTINS
        .iter()
        .map(|s| json!({ "name": s.match_name, "label": s.label }))
        .collect();
    json!({ "ok": true, "effects": effects }).to_string()
}

/// Read `layer`'s effect stack, let `f` edit a clone, and commit it as one
/// [`Op::SetLayerEffects`]. The shared tail of every effect op.
fn with_effects(
    bridge: &mut Bridge,
    comp_id: &str,
    layer_id: &str,
    ctx: &str,
    f: impl FnOnce(&mut Vec<EffectInstance>) -> Result<(), String>,
) -> String {
    let (comp, layer) = match parse_comp_layer(comp_id, layer_id) {
        Ok(pair) => pair,
        Err(e) => return err_json(format!("{ctx}: {e}")),
    };
    let doc = bridge.store.snapshot();
    let Some(c) = doc.comp(comp) else {
        return err_json(format!("{ctx}: unknown composition"));
    };
    let Some(l) = c.layers.iter().find(|l| l.id == layer) else {
        return err_json(format!("{ctx}: unknown layer"));
    };
    let mut effects = l.effects.clone();
    if let Err(e) = f(&mut effects) {
        return err_json(e);
    }
    commit(
        bridge,
        Op::SetLayerEffects {
            comp,
            layer,
            effects,
        },
        ctx,
    )
}

/// Apply a built-in effect to a layer — `lumit-ui`'s add-effect path
/// (`instantiate_for_raster` at comp size, appended to the stack).
pub(crate) fn add_effect(
    bridge: &mut Bridge,
    comp_id: &str,
    layer_id: &str,
    effect_name: &str,
) -> String {
    let ctx = "add effect";
    let (_comp, c) = match resolve_comp(bridge, comp_id, ctx) {
        Ok(pair) => pair,
        Err(e) => return err_json(e),
    };
    let (w, h) = (f64::from(c.width), f64::from(c.height));
    let Some(inst) = lumit_core::fx::instantiate_for_raster(effect_name, w, h) else {
        return err_json(format!("add effect: unknown effect '{effect_name}'"));
    };
    with_effects(bridge, comp_id, layer_id, ctx, |effects| {
        effects.push(inst);
        Ok(())
    })
}

/// Remove an effect instance from a layer by its `effect_id`.
pub(crate) fn remove_effect(
    bridge: &mut Bridge,
    comp_id: &str,
    layer_id: &str,
    effect_id: &str,
) -> String {
    let ctx = "remove effect";
    let id = match Uuid::parse_str(effect_id) {
        Ok(id) => id,
        Err(_) => return err_json("remove effect: effect id is not a valid UUID"),
    };
    with_effects(bridge, comp_id, layer_id, ctx, |effects| {
        let before = effects.len();
        effects.retain(|e| e.id != id);
        if effects.len() == before {
            return Err(err_json("remove effect: unknown effect"));
        }
        Ok(())
    })
}

/// Enable or bypass an effect instance (docs/08 §1.5).
pub(crate) fn set_effect_enabled(
    bridge: &mut Bridge,
    comp_id: &str,
    layer_id: &str,
    effect_id: &str,
    enabled: bool,
) -> String {
    let ctx = "set effect enabled";
    let id = match Uuid::parse_str(effect_id) {
        Ok(id) => id,
        Err(_) => return err_json("set effect enabled: effect id is not a valid UUID"),
    };
    with_effects(bridge, comp_id, layer_id, ctx, |effects| {
        let Some(inst) = effects.iter_mut().find(|e| e.id == id) else {
            return Err(err_json("set effect enabled: unknown effect"));
        };
        inst.enabled = enabled;
        Ok(())
    })
}

/// Set a scalar (Float) effect parameter to a static `value`.
pub(crate) fn set_effect_param_scalar(
    bridge: &mut Bridge,
    comp_id: &str,
    layer_id: &str,
    effect_id: &str,
    param_name: &str,
    value: f64,
) -> String {
    use lumit_core::anim::Property;
    let ctx = "set effect param";
    let id = match Uuid::parse_str(effect_id) {
        Ok(id) => id,
        Err(_) => return err_json("set effect param: effect id is not a valid UUID"),
    };
    let param_name = param_name.to_owned();
    with_effects(bridge, comp_id, layer_id, ctx, move |effects| {
        let Some(inst) = effects.iter_mut().find(|e| e.id == id) else {
            return Err(err_json("set effect param: unknown effect"));
        };
        let Some(param) = inst.params.iter_mut().find(|p| p.id == param_name) else {
            return Err(err_json("set effect param: unknown parameter"));
        };
        match &mut param.value {
            EffectValue::Float(p) => {
                *p = Property::fixed(value);
                Ok(())
            }
            _ => Err(err_json("set effect param: parameter is not a scalar")),
        }
    })
}

/// Set a Colour effect parameter to a static scene-linear RGBA.
#[allow(clippy::too_many_arguments)]
pub(crate) fn set_effect_param_colour(
    bridge: &mut Bridge,
    comp_id: &str,
    layer_id: &str,
    effect_id: &str,
    param_name: &str,
    r: f64,
    g: f64,
    b: f64,
    a: f64,
) -> String {
    use lumit_core::anim::Property;
    let ctx = "set effect param";
    let id = match Uuid::parse_str(effect_id) {
        Ok(id) => id,
        Err(_) => return err_json("set effect param: effect id is not a valid UUID"),
    };
    let param_name = param_name.to_owned();
    with_effects(bridge, comp_id, layer_id, ctx, move |effects| {
        let Some(inst) = effects.iter_mut().find(|e| e.id == id) else {
            return Err(err_json("set effect param: unknown effect"));
        };
        let Some(param) = inst.params.iter_mut().find(|p| p.id == param_name) else {
            return Err(err_json("set effect param: unknown parameter"));
        };
        match &mut param.value {
            EffectValue::Colour(ch) => {
                *ch = [
                    Property::fixed(r),
                    Property::fixed(g),
                    Property::fixed(b),
                    Property::fixed(a),
                ];
                Ok(())
            }
            _ => Err(err_json("set effect param: parameter is not a colour")),
        }
    })
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use crate::state::{new_composition, undo};
    use serde_json::Value;

    fn parse(s: &str) -> Value {
        serde_json::from_str(s).expect("reply is valid JSON")
    }

    /// A fresh bridge with one comp; returns the bridge and the comp id string.
    fn bridge_with_comp() -> (Bridge, String) {
        let mut b = Bridge::new();
        new_composition(&mut b, "Scene");
        let doc = b.store.snapshot();
        let comp_id = doc
            .items
            .iter()
            .find_map(|i| match i {
                ProjectItem::Composition(c) => Some(c.id),
                _ => None,
            })
            .expect("a composition exists");
        (b, comp_id.to_string())
    }

    /// The single composition item in a snapshot (nested one level under the
    /// auto-folder).
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
    fn add_solid_layer_is_white_comp_sized_and_undoes() {
        let (mut b, comp) = bridge_with_comp();
        let snap = parse(&add_solid_layer(&mut b, &comp));
        let layer = first_layer(&snap);
        assert_eq!(layer["kind"], json!("solid"));
        assert_eq!(layer["name"], json!("White solid 1"));
        assert_eq!(layer["colour"], json!([1.0, 1.0, 1.0, 1.0]));
        // The solid asset is filed too — the item tree gained a Solids folder.
        assert!(snap["items"]
            .as_array()
            .unwrap()
            .iter()
            .any(|i| i["name"] == json!("Solids")));
        // One undo step removes the whole batch (layer + asset + folder).
        let after = parse(&undo(&mut b));
        assert!(find_comp(&after)["comp"]["layers"]
            .as_array()
            .unwrap()
            .is_empty());
    }

    #[test]
    #[allow(clippy::type_complexity)]
    fn add_each_layer_kind_reports_its_kind() {
        let cases: [(fn(&mut Bridge, &str) -> String, &str); 4] = [
            (add_text_layer, "text"),
            (add_camera_layer, "camera"),
            (add_adjustment_layer, "adjustment"),
            (add_sequence_layer, "sequence"),
        ];
        for (add, kind) in cases {
            let (mut b, comp) = bridge_with_comp();
            let snap = parse(&add(&mut b, &comp));
            assert_eq!(first_layer(&snap)["kind"], json!(kind), "kind {kind}");
        }
    }

    #[test]
    fn add_layer_on_a_bad_comp_is_a_calm_error() {
        let mut b = Bridge::new();
        let reply = parse(&add_text_layer(&mut b, "not-a-uuid"));
        assert_eq!(reply["ok"], json!(false));
        assert!(reply["error"].as_str().unwrap().contains("UUID"));
    }

    #[test]
    fn duplicate_layer_copies_above_with_a_copy_name() {
        let (mut b, comp) = bridge_with_comp();
        add_camera_layer(&mut b, &comp);
        let snap = parse(&crate::state::snapshot(&b));
        let layer_id = first_layer(&snap)["id"].as_str().unwrap().to_owned();
        let dup = parse(&duplicate_layer(&mut b, &comp, &layer_id));
        let layers = find_comp(&dup)["comp"]["layers"]
            .as_array()
            .unwrap()
            .clone();
        assert_eq!(layers.len(), 2);
        // The copy sits at index 0 (directly above the original) with a fresh id.
        assert_eq!(layers[0]["name"], json!("Camera copy"));
        assert_ne!(layers[0]["id"], json!(layer_id));
    }

    #[test]
    fn delete_layer_removes_it_and_undoes() {
        let (mut b, comp) = bridge_with_comp();
        add_camera_layer(&mut b, &comp);
        let snap = parse(&crate::state::snapshot(&b));
        let layer_id = first_layer(&snap)["id"].as_str().unwrap().to_owned();
        let after = parse(&delete_layer(&mut b, &comp, &layer_id));
        assert!(find_comp(&after)["comp"]["layers"]
            .as_array()
            .unwrap()
            .is_empty());
        let restored = parse(&undo(&mut b));
        assert_eq!(
            find_comp(&restored)["comp"]["layers"]
                .as_array()
                .unwrap()
                .len(),
            1
        );
    }

    #[test]
    fn set_comp_settings_applies_and_undoes_in_one_step() {
        let (mut b, comp) = bridge_with_comp();
        // Default comp is 60 fps, 30 s. Change to 1280x720 at 24 fps, 48 frames.
        let snap = parse(&set_comp_settings(
            &mut b, &comp, "Retitled", 1280, 720, 24, 1, 48,
        ));
        let cb = find_comp(&snap)["comp"].clone();
        assert_eq!(cb["width"], json!(1280));
        assert_eq!(cb["height"], json!(720));
        assert_eq!(cb["fps"], json!({ "num": 24, "den": 1 }));
        assert_eq!(cb["frame_count"], json!(48));
        assert_eq!(find_comp(&snap)["name"], json!("Retitled"));
        // A single op: one undo restores the original settings.
        let after = parse(&undo(&mut b));
        assert_eq!(
            find_comp(&after)["comp"]["fps"],
            json!({ "num": 60, "den": 1 })
        );
    }

    /// Build a comp with one camera layer, returning bridge, comp id, layer id.
    fn comp_with_camera() -> (Bridge, String, String) {
        let (mut b, comp) = bridge_with_comp();
        add_camera_layer(&mut b, &comp);
        let snap = parse(&crate::state::snapshot(&b));
        let layer_id = first_layer(&snap)["id"].as_str().unwrap().to_owned();
        (b, comp, layer_id)
    }

    #[test]
    fn stopwatch_enables_then_disables_animation() {
        let (mut b, comp, layer) = comp_with_camera();
        // Enable at frame 0: opacity gains one key holding its current value.
        let snap = parse(&toggle_property_animated(
            &mut b, &comp, &layer, "opacity", 0,
        ));
        let tr = &first_layer(&snap)["transform"]["opacity"];
        assert_eq!(tr["animated"], json!(true));
        assert_eq!(tr["value"], json!(100.0));
        assert_eq!(tr["keys"].as_array().unwrap().len(), 1);
        // Disable: collapses back to static (no keys).
        let snap = parse(&toggle_property_animated(
            &mut b, &comp, &layer, "opacity", 0,
        ));
        assert_eq!(
            first_layer(&snap)["transform"]["opacity"]["animated"],
            json!(false)
        );
    }

    #[test]
    fn add_remove_and_read_back_keyframes() {
        let (mut b, comp, layer) = comp_with_camera();
        // Two keys on rotation: frame 0 = 0°, frame 60 = 90° (60 fps default).
        add_keyframe(&mut b, &comp, &layer, "rotation", 0, 0.0);
        let snap = parse(&add_keyframe(&mut b, &comp, &layer, "rotation", 60, 90.0));
        let rot = first_layer(&snap)["transform"]["rotation"].clone();
        assert_eq!(rot["animated"], json!(true));
        let keys = rot["keys"].as_array().unwrap();
        assert_eq!(keys.len(), 2);
        assert_eq!(keys[0]["frame"], json!(0));
        assert_eq!(keys[0]["interp_in"], json!("Linear"));
        assert_eq!(keys[1]["frame"], json!(60));
        assert_eq!(keys[1]["value"], json!(90.0));
        // Shift the second key by +30 frames → frame 90.
        let snap = parse(&shift_keyframes(
            &mut b, &comp, &layer, "rotation", "[60]", 30,
        ));
        let keys = first_layer(&snap)["transform"]["rotation"]["keys"]
            .as_array()
            .unwrap()
            .clone();
        assert_eq!(keys[1]["frame"], json!(90));
        // Remove the key at frame 0.
        let snap = parse(&remove_keyframe(&mut b, &comp, &layer, "rotation", 0));
        let keys = first_layer(&snap)["transform"]["rotation"]["keys"]
            .as_array()
            .unwrap()
            .clone();
        assert_eq!(keys.len(), 1);
        assert_eq!(keys[0]["frame"], json!(90));
    }

    #[test]
    fn removing_the_last_keyframe_collapses_to_static() {
        let (mut b, comp, layer) = comp_with_camera();
        // The stopwatch seeds exactly one key at the playhead (value 100 there).
        toggle_property_animated(&mut b, &comp, &layer, "opacity", 30);
        let snap = parse(&remove_keyframe(&mut b, &comp, &layer, "opacity", 30));
        let op = first_layer(&snap)["transform"]["opacity"].clone();
        assert_eq!(op["animated"], json!(false));
        assert_eq!(op["value"], json!(100.0));
    }

    #[test]
    fn work_area_sets_edges_and_reads_back_frames() {
        let (mut b, comp) = bridge_with_comp();
        // Out edge at frame 120 → work_area out is 121 (edge is exclusive + 1).
        let snap = parse(&set_work_area_edge(&mut b, &comp, 120, true));
        let wa = find_comp(&snap)["comp"]["work_area"].clone();
        assert!(wa.is_array(), "work area is set");
        assert_eq!(wa[0], json!(0));
        assert_eq!(wa[1], json!(121));
        // In edge at frame 60.
        let snap = parse(&set_work_area_edge(&mut b, &comp, 60, false));
        assert_eq!(find_comp(&snap)["comp"]["work_area"][0], json!(60));
    }

    #[test]
    fn list_effects_names_the_builtins() {
        let reply = parse(&list_effects());
        assert_eq!(reply["ok"], json!(true));
        let effects = reply["effects"].as_array().unwrap();
        assert!(!effects.is_empty());
        assert!(effects
            .iter()
            .all(|e| e["name"].is_string() && e["label"].is_string()));
    }

    #[test]
    fn add_effect_then_edit_and_remove() {
        let (mut b, comp, layer) = comp_with_camera();
        // Add the first builtin by its match name.
        let name = lumit_core::fx::BUILTINS[0].match_name;
        let snap = parse(&add_effect(&mut b, &comp, &layer, name));
        let effects = first_layer(&snap)["effects"].as_array().unwrap().clone();
        assert_eq!(effects.len(), 1);
        assert_eq!(effects[0]["name"], json!(name));
        assert_eq!(effects[0]["enabled"], json!(true));
        let effect_id = effects[0]["id"].as_str().unwrap().to_owned();
        // Bypass it.
        let snap = parse(&set_effect_enabled(
            &mut b, &comp, &layer, &effect_id, false,
        ));
        assert_eq!(first_layer(&snap)["effects"][0]["enabled"], json!(false));
        // Remove it.
        let snap = parse(&remove_effect(&mut b, &comp, &layer, &effect_id));
        assert!(first_layer(&snap)["effects"].as_array().unwrap().is_empty());
    }

    #[test]
    fn set_scalar_effect_param_round_trips() {
        let (mut b, comp, layer) = comp_with_camera();
        // Find a builtin that has a scalar (Float) parameter.
        let (name, param) = lumit_core::fx::BUILTINS
            .iter()
            .find_map(|s| {
                s.params.iter().find_map(|p| match p.kind {
                    lumit_core::fx::ParamKind::Float { .. } => Some((s.match_name, p.id)),
                    _ => None,
                })
            })
            .expect("a builtin has a scalar param");
        add_effect(&mut b, &comp, &layer, name);
        let snap = parse(&crate::state::snapshot(&b));
        let effect_id = first_layer(&snap)["effects"][0]["id"]
            .as_str()
            .unwrap()
            .to_owned();
        let snap = parse(&set_effect_param_scalar(
            &mut b, &comp, &layer, &effect_id, param, 12.5,
        ));
        let params = first_layer(&snap)["effects"][0]["params"]
            .as_array()
            .unwrap()
            .clone();
        let got = params
            .iter()
            .find(|p| p["name"] == json!(param))
            .expect("param present");
        assert_eq!(got["kind"], json!("scalar"));
        assert_eq!(got["value"], json!(12.5));
    }
}

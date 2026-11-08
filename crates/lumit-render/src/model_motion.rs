//! A Model layer's rows on the way to a pose: which take at what moment, under
//! its Retime, Loop and Offset, and what its parts' rows do. A model with no
//! take and no part changed has no pose at all, and draws as its file has it.

use std::sync::Arc;

use glam::{EulerRot, Mat4, Quat, Vec3};
use lumit_core::expression::ExpressionContext;
use lumit_core::fx::builtins::instantiate;
use lumit_core::model::{Composition, Document, EffectInstance, EffectValue, Layer, LayerKind};
use lumit_core::model_layer::{ModelSettings, Take};
use lumit_core::model_part::{
    choice_of, number, part_id, playback_id, take_of, untouched, PART_NAME, PLAYBACK_NAME,
};
use lumit_scene::pose::{pose, take_time, Looping, PartEdit, Pose};
use lumit_scene::{Clip, ModelScene};

/// The take `settings` names in `scene`, `None` for none or one the file no
/// longer has.
#[must_use]
pub fn clip_of<'a>(settings: &ModelSettings, scene: &'a ModelScene) -> Option<&'a Clip> {
    match &settings.take {
        Take::Rest => None,
        Take::First => scene.clips.first(),
        Take::Named(name) => scene.clips.iter().find(|c| &c.name == name),
    }
}

/// The node a part's rows are for: by the number in their id when that node
/// is still the one they were kept for, else by its path, so a file whose
/// nodes move keeps each one's edits.
fn node_of(
    layer: &Layer,
    rows: &lumit_core::model::EffectInstance,
    scene: &ModelScene,
) -> Option<usize> {
    let path = rows.custom_name.as_deref()?;
    let by_id = lumit_core::model_part::node_of(layer.id, rows.id)
        .filter(|&k| scene.nodes.get(k).is_some_and(|n| n.path == path));
    by_id.or_else(|| scene.nodes.iter().position(|n| n.path == path))
}

/// `layer`'s model at comp time `t`, `None` when it draws as its file has it.
/// Nothing is carried from the frame before.
#[must_use]
pub fn pose_at(
    comp: &Composition,
    layer: &Layer,
    settings: &ModelSettings,
    scene: &ModelScene,
    t: f64,
    context: &Arc<ExpressionContext>,
) -> Option<Pose> {
    let clip = clip_of(settings, scene);
    if clip.is_none() && settings.parts.is_empty() {
        return None;
    }
    let lt = lumit_core::time::layer_time(t, layer.start_offset.0);
    let context = Arc::new(ExpressionContext {
        comp_time: t,
        layer: Some(layer.id),
        ..(**context).clone()
    });
    let take =
        clip.and_then(|clip| Some((clip, take_moment(layer, settings, clip, lt, &context)?)));
    let comp_size = (comp.width as f32, comp.height as f32);
    let edits = edits_at(layer, settings, scene, comp_size, lt, &context);
    Some(pose(scene, take, &edits))
}

/// Where in `clip` `layer` is at layer time `lt`, under its Retime, Offset and
/// Loop. `None` when it rests outside the take.
pub(crate) fn take_moment(
    layer: &Layer,
    settings: &ModelSettings,
    clip: &Clip,
    lt: f64,
    context: &Arc<ExpressionContext>,
) -> Option<f64> {
    let rows = settings.playback.as_ref();
    let looping = match rows.and_then(|r| r.param("looping")) {
        Some(EffectValue::Choice(0)) => Looping::Off,
        Some(EffectValue::Choice(2)) => Looping::Mirror,
        Some(EffectValue::Choice(3)) => Looping::Freeze,
        _ => Looping::Loop,
    };
    let offset = rows.map_or(0.0, |r| number(r, "offset", lt, context, 0.0));
    let at = layer.source_time_at(lt) + offset;
    take_time(at, f64::from(clip.duration), looping)
}

/// A part's pivot, and the corners of the box round what it moves.
pub type PartPlace = ([f32; 3], Option<[[f32; 3]; 8]>);

/// Where the part with rows `id` on Model layer `layer` is at comp time `t`, in
/// the layer's own pixels: its pivot, and the eight corners of the box round
/// what it moves, by the bits of x, y and z. No box on a part that moves
/// nothing of its own. `None` before the file has been read.
#[must_use]
pub fn part_place(
    doc: &Arc<Document>,
    comp: &Composition,
    layer: &Layer,
    id: uuid::Uuid,
    t: f64,
) -> Option<PartPlace> {
    let LayerKind::Model { item, settings } = &layer.kind else {
        return None;
    };
    let k = lumit_core::model_part::node_of(layer.id, id)?;
    let scene = crate::model::scene_if_read(doc, *item)??;
    let posed = layer_pose(doc, comp, layer, t).unwrap_or_else(|| pose(&scene, None, &[]));
    let size = (comp.width as f32, comp.height as f32);
    let to_layer =
        Mat4::from_cols_array_2d(&crate::model::model_to_comp(settings, &scene.bounds, size));
    let world = *posed.worlds.get(k)?;
    let node = scene.nodes.get(k)?;
    let own = lumit_scene::pose::joint_box(&scene, &posed, k).or_else(|| {
        let mesh = scene.meshes.get(node.mesh?)?;
        Some(match node.skin {
            Some(_) => lumit_scene::pose::corners(&Mat4::IDENTITY, &posed.bounds),
            None => lumit_scene::pose::corners(&world, &mesh.bounds),
        })
    });
    let in_layer = |p: Vec3| to_layer.transform_point3(p).to_array();
    Some((
        in_layer(world.w_axis.truncate()),
        own.map(|c| c.map(in_layer)),
    ))
}

/// `layer`'s pose at comp time `t` for a panel: `None` when it has none, when
/// `layer` isn't a Model layer and before its file has been read, which this
/// never starts.
#[must_use]
pub fn layer_pose(doc: &Arc<Document>, comp: &Composition, layer: &Layer, t: f64) -> Option<Pose> {
    let LayerKind::Model { item, settings } = &layer.kind else {
        return None;
    };
    let scene = crate::model::scene_if_read(doc, *item)??;
    let context = lumit_core::model::comp_context(doc, comp, t);
    pose_at(comp, layer, settings, &scene, t, &context)
}

/// Every row Model layer `layer` showing `scene` has: its playback rows first,
/// the file's takes read into them, then a part's for each node. Those it keeps
/// are as it keeps them, the rest as nobody has changed them.
#[must_use]
pub fn all_rows(
    layer: &Layer,
    settings: &ModelSettings,
    scene: &ModelScene,
) -> Vec<EffectInstance> {
    let mut list = Vec::with_capacity(scene.nodes.len() + 1);
    let mut playback = settings
        .playback
        .clone()
        .or_else(|| instantiate(PLAYBACK_NAME));
    if let Some(rows) = &mut playback {
        rows.id = playback_id(layer.id);
        if let Some(p) = rows.params.iter_mut().find(|p| p.id == "take") {
            p.value = EffectValue::Choice(choice_of(&settings.take, &take_names(scene)));
        }
    }
    list.extend(playback);
    for (k, node) in scene.nodes.iter().enumerate() {
        let kept = settings
            .parts
            .iter()
            .find(|r| r.custom_name.as_deref() == Some(node.path.as_str()));
        let Some(mut rows) = kept.cloned().or_else(|| instantiate(PART_NAME)) else {
            continue;
        };
        rows.id = part_id(layer.id, k);
        rows.custom_name = Some(node.path.clone());
        list.push(rows);
    }
    list
}

/// The file's takes by name, as the Animation row lists them after its first.
#[must_use]
pub fn take_names(scene: &ModelScene) -> Vec<String> {
    scene.clips.iter().map(|c| c.name.clone()).collect()
}

/// `settings` once its rows read as `list` does: only what differs from the
/// file is kept, along with rows of a part this file doesn't have.
#[must_use]
pub fn with_rows(
    settings: &ModelSettings,
    scene: &ModelScene,
    list: &[EffectInstance],
) -> ModelSettings {
    let mut out = settings.clone();
    let (mut playback, mut parts) = (None, Vec::new());
    for rows in list {
        if rows.effect.match_name == PLAYBACK_NAME {
            let choice = match rows.param("take") {
                Some(EffectValue::Choice(c)) => *c,
                _ => 0,
            };
            // Anything but a change of take leaves a layer on its first take
            // on it, so a file later given another first take still plays.
            let names = take_names(scene);
            if choice != choice_of(&settings.take, &names) {
                out.take = take_of(choice, &names);
            }
            playback = (!untouched(rows)).then(|| rows.clone());
        } else if !untouched(rows) {
            parts.push(rows.clone());
        }
    }
    // The rows of parts that aren't in this file wait for the file to come back.
    let here = |r: &EffectInstance| {
        let path = r.custom_name.as_deref();
        scene.nodes.iter().any(|n| Some(n.path.as_str()) == path)
    };
    parts.extend(settings.parts.iter().filter(|r| !here(r)).cloned());
    out.playback = playback;
    out.parts = parts;
    out
}

/// What `settings`' part rows come to at `lt`, in model space.
fn edits_at(
    layer: &Layer,
    settings: &ModelSettings,
    scene: &ModelScene,
    comp: (f32, f32),
    lt: f64,
    context: &Arc<ExpressionContext>,
) -> Vec<PartEdit> {
    if settings.parts.is_empty() {
        return Vec::new();
    }
    let to_layer =
        Mat4::from_cols_array_2d(&crate::model::model_to_comp(settings, &scene.bounds, comp));
    let back = to_layer.inverse();
    let turn = crate::model::up_right(settings.up_axis);
    settings
        .parts
        .iter()
        .filter_map(|rows| {
            let node = node_of(layer, rows, scene)?;
            let n = |id, default| number(rows, id, lt, context, default) as f32;
            let d = |id| n(id, 0.0).to_radians();
            let moved = Vec3::new(
                n("position_x", 0.0),
                n("position_y", 0.0),
                n("position_z", 0.0),
            );
            // In the order the layer's own rows turn it, Y, then X, then Z.
            let turned = Quat::from_euler(
                EulerRot::YXZ,
                d("rotation_y"),
                d("rotation_x"),
                d("rotation_z"),
            );
            Some(PartEdit {
                node,
                moved: back.transform_vector3(moved),
                // The layer's axes seen from the model's.
                turned: turn.inverse() * turned * turn,
                scale: n("scale", 100.0) / 100.0,
                hidden: n("visible", 1.0) < 0.5,
            })
        })
        .collect()
}

/// Whether a model under `settings` changes shape with the playhead: it plays
/// a take, or a part's row is keyed or driven. Over-answering only costs asking.
#[must_use]
pub fn moves(settings: &ModelSettings, scene: &ModelScene) -> bool {
    clip_of(settings, scene).is_some()
        || settings.parts.iter().any(|rows| {
            rows.params.iter().any(|p| match &p.value {
                EffectValue::Float(f) => f.is_animated(),
                _ => false,
            })
        })
}

/// Whether a staged list is this layer's playback or part rows.
pub fn owns(layer: &Layer, effects: &[EffectInstance]) -> bool {
    effects
        .first()
        .is_some_and(|f| lumit_core::model_part::owns(layer.id, f.id))
}

/// Lay rows `list`, staged for a drag, on Model layer `layer` for one preview
/// frame. Nothing while its file isn't read.
pub fn stage(doc: &Document, layer: &mut Layer, list: &[EffectInstance]) {
    let LayerKind::Model { item, settings } = &mut layer.kind else {
        return;
    };
    if let Some(Some(scene)) = crate::model::scene_if_read(doc, *item) {
        **settings = with_rows(settings, &scene, list);
    }
}

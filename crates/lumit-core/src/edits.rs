//! The layer and asset defaults both frontends build from.
//!
//! # In plain terms
//!
//! When you add a solid, or place footage, something has to decide how big it
//! is, where its anchor sits and how long it lasts. These are those decisions,
//! in one place — because the egui frontend and the Flutter one must produce
//! *identical* layers, and two copies of "a new solid is comp-sized and white"
//! would drift the first time one was changed.
//!
//! Everything here is pure: it takes a document or a few numbers and returns
//! ops or values. Nothing commits, and nothing reaches for global state.

use lumit_core::model::{
    Folder, Layer, LayerKind, LinearColour, ProjectItem, SolidDef, Switches, TransformGroup,
};
use lumit_core::ops::{AutoFolderKind, Op};
use lumit_core::time::{CompTime, Rational};
use uuid::Uuid;

/// A transform anchored on the content's own centre and placed at the comp
/// centre — `lumit-ui`'s `centred_transform`, the seeding every add-layer path
/// uses so a fresh layer appears centred and pivots about its middle.
///
/// Shared with the frb API rather than copied: a second set of layer-seeding
/// defaults would drift, and a footage layer added through the new bridge would
/// then land in a different place from one added through the old.
pub fn centred_transform(nat_w: f64, nat_h: f64, comp_w: u32, comp_h: u32) -> TransformGroup {
    use lumit_core::anim::Property;
    TransformGroup {
        anchor_x: Property::fixed(nat_w * 0.5),
        anchor_y: Property::fixed(nat_h * 0.5),
        position_x: Property::fixed(f64::from(comp_w) * 0.5),
        position_y: Property::fixed(f64::from(comp_h) * 0.5),
        ..TransformGroup::default()
    }
}

/// A fresh Camera layer for `comp` (docs/impl/camera.md §1): the eye sits
/// the After Effects 50 mm zoom behind the comp centre looking down +z, the
/// point of interest is the centre, and the focus is on that plane. A camera
/// made this way changes no picture. `solve_link` makes it a solved camera.
pub fn fresh_camera(
    comp: &lumit_core::model::Composition,
    name: String,
    solve_link: Option<Uuid>,
) -> Layer {
    use lumit_core::anim::Property;
    let (w, h) = (f64::from(comp.width), f64::from(comp.height));
    let zoom = lumit_core::camera::default_zoom(w);
    let at = (w * 0.5, h * 0.5, 0.0);
    base_layer(
        name,
        LayerKind::Camera {
            zoom: Property::fixed(zoom),
            solve_link,
            correction_base: None,
            options: Box::new(lumit_core::model::CameraOptions::fresh(zoom, at)),
        },
        comp.duration.0,
        TransformGroup {
            position_x: Property::fixed(at.0),
            position_y: Property::fixed(at.1),
            position_z: Property::fixed(-zoom),
            ..TransformGroup::default()
        },
    )
}

/// A layer with the house defaults every add path shares, given the parts that
/// differ (name, kind, span end, transform). The span starts at comp 0 and the
/// switches are the model defaults — exactly as the egui add-layer paths build.
///
/// Shared with the frb API for the same reason as [`centred_transform`].
pub fn base_layer(
    name: String,
    kind: LayerKind,
    out: Rational,
    transform: TransformGroup,
) -> Layer {
    // Each kind starts on its own label colour: the label drives both
    // the outline's swatch and the bar's fill, so a fresh stack is tellable
    // apart at a glance. The user's own pick simply overwrites it.
    let label = match &kind {
        LayerKind::Footage { .. } => 1,
        LayerKind::Solid { .. } => 2,
        LayerKind::Precomp { .. } => 3,
        LayerKind::Text { .. } => 4,
        LayerKind::Camera { .. } => 5,
        LayerKind::Sequence { .. } => 6,
        LayerKind::Adjustment => 7,
        LayerKind::Null => 8,
        LayerKind::Shape { .. } => 9,
        LayerKind::Light { .. } => 10,
    };
    Layer {
        markers: Vec::new(),
        graph: Default::default(),
        id: Uuid::now_v7(),
        name,
        kind,
        in_point: CompTime(Rational::ZERO),
        out_point: CompTime(out),
        start_offset: CompTime(Rational::ZERO),
        transform,
        matte: None,
        parent: None,
        label,
        volume_db: lumit_core::anim::Property::zero(),
        pan: lumit_core::anim::Property::zero(),
        audio_only: false,
        audio_streams: Default::default(),
        adjustment: false,
        retime: None,
        interpolation: Default::default(),
        parked_flow: None,
        graph_inputs: None,
        blend: lumit_core::model::BlendMode::Normal,
        masks: Vec::new(),
        paint: Vec::new(),
        puppet: None,
        effects: Vec::new(),
        styles: Vec::new(),
        switches: Switches::default(),
        extra: serde_json::Map::new(),
    }
}

/// A layer born into a composition that already has a solo up wears solo too.
///
/// **In plain terms.** While something is soloed the comp shows only that
/// layer, so anything added lands invisible and inaudible — you add a solid and
/// nothing happens. Adding it soloed puts it where it was just asked for.
///
/// Counted the way the two halves of solo are counted: a layer with a
/// picture asks whether any layer that *draws* is soloed, an Audio layer asks
/// the mixer's question — every soloed layer. A Camera, a Light and a Null are
/// left out of it in both directions ([`Layer::can_adjust`] is the same "shows
/// something in the Viewer" question): none of them draws, so soloing one would
/// blank the comp, and they are chosen by their visible switch and never by
/// solo, so they work in a soloed comp untouched.
///
/// `existing` is what the comp holds around the newcomer, as an iterator
/// because Pre-compose asks with the layers it is packing away left out. The
/// switch is only ever set, never cleared: a pasted layer that was soloed where
/// it was copied from stays soloed.
pub fn solo_on_arrival<'a>(layer: &mut Layer, mut existing: impl Iterator<Item = &'a Layer>) {
    let audio_only = layer.audio_only;
    if !audio_only && !layer.can_adjust() {
        return;
    }
    layer.switches.solo |= existing.any(|l| l.switches.solo && (audio_only || !l.audio_only));
}

/// The ops that guarantee `kind`'s auto-filing folder exists, plus its id —
/// `lumit-ui`'s `ensure_auto_folder_ops`, tracked by id so renaming or nesting
/// the folder keeps the habit.
pub fn ensure_auto_folder_ops(
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
pub fn file_into_folder_op(doc: &lumit_core::model::Document, folder: Uuid, item: Uuid) -> Op {
    let mut children = doc
        .folder(folder)
        .map(|f| f.children.clone())
        .unwrap_or_default();
    children.push(item);
    Op::SetFolderChildren { folder, children }
}

/// The ops that add one fresh comp-sized white solid asset, filed in the Solids
/// auto-folder, plus its id and the name it was given ("White solid N").
///
/// Two callers ask for exactly this: **New solid**, which then adds a layer
/// pointing at it, and the Modes column's adjustment toggle turned back off,
/// which has to hand an adjustment layer a picture of its own again.
pub fn white_solid_ops(
    doc: &lumit_core::model::Document,
    width: u32,
    height: u32,
) -> (Uuid, String, Vec<Op>) {
    let (folder, mut ops) = ensure_auto_folder_ops(doc, AutoFolderKind::Solids);

    let def = Uuid::now_v7();
    let solids = doc
        .items
        .iter()
        .filter(|i| matches!(i, ProjectItem::Solid(_)))
        .count();
    let name = format!("White solid {}", solids + 1);

    // The folder op may itself be an AddItem, so the index has to account for
    // what this batch has already inserted.
    let added = ops
        .iter()
        .filter(|o| matches!(o, Op::AddItem { .. }))
        .count();
    ops.push(Op::AddItem {
        index: doc.items.len() + added,
        item: Box::new(ProjectItem::Solid(SolidDef {
            id: def,
            name: name.clone(),
            colour: LinearColour([1.0, 1.0, 1.0, 1.0]),
            width,
            height,
            extra: serde_json::Map::new(),
        })),
    });
    ops.push(file_into_folder_op(doc, folder, def));

    (def, name, ops)
}

/// A stable machine key for a [`FxCategory`] (the variant in snake_case) — the
/// grouping key the Dart Effects browser sorts and headers by.
///
/// Shared with the frb surface (`api::effect::list_effects`) rather than
/// restated there, so the two frontends cannot disagree about a category key.
/// **It therefore has to move rather than die when v0 is deleted.**
pub fn fx_category_key(cat: lumit_core::fx::FxCategory) -> &'static str {
    use lumit_core::fx::FxCategory;
    match cat {
        FxCategory::BlurSharpen => "blur_sharpen",
        FxCategory::Colour => "colour",
        FxCategory::Distortion => "distortion",
        FxCategory::Generate => "generate",
        FxCategory::Stylise => "stylise",
        FxCategory::Temporal => "temporal",
        FxCategory::Transition => "transition",
        FxCategory::Utility => "utility",
        // The same key a hosted audio plugin is filed under
        // (`api::effect::NAMESPACE_AUDIO`), on purpose: a built-in and a
        // plugin belong under one Audio heading, and two keys would draw two.
        FxCategory::Audio => "audio",
        FxCategory::Compositing => "compositing",
        FxCategory::Controls => "controls",
        FxCategory::Drivers => "drivers",
    }
}

/// Hidden copies that keep the parent and matte links a Pre-compose would cut.
///
/// `inside` are copies of layers left outside that a moving layer leans on,
/// and `outside` are copies of moving layers that a layer left outside leans
/// on. Each map takes the original's id to its copy's on that side.
#[derive(Default)]
pub struct LinkCopies {
    pub inside: Vec<Layer>,
    pub inside_ids: std::collections::HashMap<Uuid, Uuid>,
    pub outside: Vec<Layer>,
    pub outside_ids: std::collections::HashMap<Uuid, Uuid>,
}

impl LinkCopies {
    pub fn is_empty(&self) -> bool {
        self.inside.is_empty() && self.outside.is_empty()
    }
}

/// The copies [`LinkCopies`] describes, for `moving` leaving `layers`.
///
/// A copy is hidden and silent, so the picture keeps one of each layer, and
/// it takes its parent chain with it: a parent that is not on the copy's side
/// is copied too.
pub fn link_copies(layers: &[Layer], moving: &[Uuid]) -> LinkCopies {
    use std::collections::HashMap;

    fn links(layer: &Layer) -> impl Iterator<Item = Uuid> + '_ {
        layer
            .parent
            .into_iter()
            .chain(layer.matte.iter().map(|m| m.layer))
    }

    /// The copy of `id` on the side `here` says yes to, made once.
    fn copy_of(
        layers: &[Layer],
        id: Uuid,
        here: &dyn Fn(Uuid) -> bool,
        copies: &mut Vec<Layer>,
        ids: &mut HashMap<Uuid, Uuid>,
    ) -> Option<Uuid> {
        if let Some(made) = ids.get(&id) {
            return Some(*made);
        }
        let mut copy = layers.iter().find(|l| l.id == id)?.clone();
        copy.id = Uuid::now_v7();
        ids.insert(id, copy.id);
        copy.switches.visible = false;
        copy.switches.audible = false;
        copy.switches.solo = false;
        copy.matte = None;
        for effect in &mut copy.effects {
            effect.id = Uuid::now_v7();
        }
        copy.parent = match copy.parent {
            Some(p) if !here(p) => copy_of(layers, p, here, copies, ids),
            kept => kept,
        };
        copies.push(copy);
        Some(ids[&id])
    }

    let known = |id: Uuid| layers.iter().any(|l| l.id == id);
    let inside = |id: Uuid| moving.contains(&id);
    let outside = |id: Uuid| !moving.contains(&id);
    let mut out = LinkCopies::default();
    for layer in layers {
        let mover = inside(layer.id);
        for target in links(layer).filter(|t| known(*t) && inside(*t) != mover) {
            if mover {
                copy_of(
                    layers,
                    target,
                    &inside,
                    &mut out.inside,
                    &mut out.inside_ids,
                );
            } else {
                copy_of(
                    layers,
                    target,
                    &outside,
                    &mut out.outside,
                    &mut out.outside_ids,
                );
            }
        }
    }
    out
}

/// Hand `f` every composition id `comp` names: a Precomp layer's, a Sequence
/// clip's, a node graph Read box's (which may be any project item) and a Node
/// graph effect's. What `f` writes back is what the comp then names.
pub fn visit_comp_refs(comp: &mut lumit_core::model::Composition, f: &mut dyn FnMut(&mut Uuid)) {
    use lumit_core::comp_graph::GraphNode;
    use lumit_core::fx::effects::node_graph;

    fn effects(list: &mut [lumit_core::model::EffectInstance], f: &mut dyn FnMut(&mut Uuid)) {
        for inst in list {
            if inst.effect.match_name != lumit_core::comp_graph::NODE_GRAPH {
                continue;
            }
            let Some(was) = node_graph::comp_of(inst) else {
                continue;
            };
            let mut id = was;
            f(&mut id);
            if id != was {
                if let Some(serde_json::Value::Object(bound)) =
                    inst.extra.get_mut(node_graph::EXTRA_KEY)
                {
                    bound.insert("comp".into(), id.to_string().into());
                }
            }
        }
    }

    for layer in &mut comp.layers {
        match &mut layer.kind {
            LayerKind::Precomp { comp } => f(comp),
            LayerKind::Sequence { clips } => {
                for clip in clips {
                    if let lumit_core::sequence::ClipSource::Comp(id) = &mut clip.source {
                        f(id);
                    }
                }
            }
            _ => {}
        }
        effects(&mut layer.effects, f);
    }
    for group in &mut comp.groups {
        effects(&mut group.effects, f);
    }
    if let Some(graph) = &mut comp.graph {
        for node in &mut graph.nodes {
            match node {
                GraphNode::Read { item, .. } => f(item),
                GraphNode::Fx(inst) => effects(std::slice::from_mut(inst), f),
                _ => {}
            }
        }
    }
}

/// The compositions `comp` nests directly, each once, in the order met.
pub fn nested_comps(
    doc: &lumit_core::model::Document,
    comp: &lumit_core::model::Composition,
) -> Vec<Uuid> {
    let mut found = Vec::new();
    visit_comp_refs(&mut comp.clone(), &mut |id| {
        if doc.comp(*id).is_some() && !found.contains(id) {
            found.push(*id);
        }
    });
    found
}

/// Give everything `comps` hold fresh ids: layers, effects, graph boxes,
/// masks, clips, markers, anything stored with an `id` of its own. Whatever
/// names one of them inside `comps` (a parent, a matte, a layer parameter, a
/// wire, a box's place) follows it to the new id. What they name outside
/// keeps pointing where it did. The comps' own ids are left alone.
pub fn fresh_ids(comps: &mut [lumit_core::model::Composition]) -> Option<()> {
    use serde_json::Value;
    use std::collections::HashMap;

    fn collect(value: &Value, fresh: &mut HashMap<String, String>) {
        match value {
            Value::Object(fields) => {
                if let Some(Value::String(id)) = fields.get("id") {
                    if Uuid::try_parse(id).is_ok() {
                        fresh
                            .entry(id.clone())
                            .or_insert_with(|| Uuid::now_v7().to_string());
                    }
                }
                fields.values().for_each(|v| collect(v, fresh));
            }
            Value::Array(list) => list.iter().for_each(|v| collect(v, fresh)),
            _ => {}
        }
    }

    fn repoint(value: &mut Value, fresh: &HashMap<String, String>) {
        match value {
            Value::String(s) => {
                if let Some(id) = fresh.get(s.as_str()) {
                    s.clone_from(id);
                }
            }
            Value::Object(fields) => {
                *fields = std::mem::take(fields)
                    .into_iter()
                    .map(|(k, mut v)| {
                        repoint(&mut v, fresh);
                        (fresh.get(&k).cloned().unwrap_or(k), v)
                    })
                    .collect();
            }
            Value::Array(list) => list.iter_mut().for_each(|v| repoint(v, fresh)),
            _ => {}
        }
    }

    let values: Vec<Value> = comps
        .iter()
        .map(serde_json::to_value)
        .collect::<Result<_, _>>()
        .ok()?;
    let mut fresh = HashMap::new();
    values.iter().for_each(|v| collect(v, &mut fresh));
    for comp in comps.iter() {
        fresh.remove(&comp.id.to_string());
    }
    for (comp, mut value) in comps.iter_mut().zip(values) {
        repoint(&mut value, &fresh);
        *comp = serde_json::from_value(value).ok()?;
    }
    Some(())
}

/// What a composition is made with, or changed to: the engine's half of the
/// New composition and Composition settings dialogues.
#[derive(Debug, Clone, PartialEq)]
pub struct CompSettings {
    /// A blank name on a new comp gets the next "Comp N".
    pub name: String,
    pub width: u32,
    pub height: u32,
    pub frame_rate: lumit_core::time::FrameRate,
    pub duration: lumit_core::time::Duration,
    /// Scene-linear RGBA.
    pub background: [f32; 4],
    /// The master shutter's angle in degrees, and how many sub-frame samples a
    /// blurred layer is drawn at. The shutter's phase is left as it stands.
    pub shutter_angle: f64,
    pub motion_blur_samples: u32,
}

impl Default for CompSettings {
    /// What a comp gets when nobody chose: 1920×1080, 60 fps, 30 seconds,
    /// black, and the model's own shutter. One place decides what a comp is.
    fn default() -> Self {
        use lumit_core::time::{Duration, FrameRate};
        let blur = lumit_core::model::MotionBlur::default();
        CompSettings {
            name: String::new(),
            width: 1920,
            height: 1080,
            frame_rate: FrameRate::new(60, 1).unwrap_or(FrameRate::FPS_25),
            duration: Duration(Rational::new(30, 1).unwrap_or(Rational::ONE)),
            background: [0.0, 0.0, 0.0, 1.0],
            shutter_angle: blur.shutter_angle,
            motion_blur_samples: blur.samples,
        }
    }
}

/// A comp's length for `seconds` at `rate`. A comp shorter than one frame has
/// nothing to show, so the floor is one frame at that rate.
pub fn comp_duration(
    rate: lumit_core::time::FrameRate,
    seconds: Rational,
) -> Option<lumit_core::time::Duration> {
    let floor = rate.frame_duration().ok()?;
    Some(lumit_core::time::Duration(seconds.max(floor.0)))
}

/// The name a comp made now would get if nobody typed one. `node_graph`
/// counts and names the node graphs; otherwise the comps. The two are counted
/// apart, so making a graph does not push the next comp's number along and
/// neither reads as the other's.
pub fn next_comp_name(doc: &lumit_core::model::Document, node_graph: bool) -> String {
    let existing = doc
        .items
        .iter()
        .filter(|i| match i {
            ProjectItem::Composition(c) => c.graph.is_some() == node_graph,
            _ => false,
        })
        .count();
    if node_graph {
        format!("Node graph {}", existing + 1)
    } else {
        format!("Comp {}", existing + 1)
    }
}

/// The ops that add a composition and file it, with the new comp's id.
/// Nothing is committed, so a caller can add to the batch.
pub fn new_comp_ops(
    doc: &lumit_core::model::Document,
    settings: CompSettings,
    graph: Option<lumit_core::comp_graph::CompGraph>,
) -> (Uuid, Vec<Op>) {
    use lumit_core::model::{Composition, MotionBlur};

    let name = if settings.name.trim().is_empty() {
        next_comp_name(doc, graph.is_some())
    } else {
        settings.name
    };

    let mut ops: Vec<Op> = Vec::new();
    let folder_id = match doc
        .auto_folders
        .compositions
        .filter(|id| doc.folder(*id).is_some())
    {
        Some(id) => id,
        None => {
            let id = Uuid::now_v7();
            ops.push(Op::AddItem {
                index: doc.items.len(),
                item: Box::new(ProjectItem::Folder(Folder {
                    id,
                    name: "Compositions".into(),
                    children: Vec::new(),
                    extra: serde_json::Map::new(),
                })),
            });
            ops.push(Op::SetAutoFolder {
                kind: AutoFolderKind::Compositions,
                folder: Some(id),
            });
            id
        }
    };

    let comp = Composition {
        graph,
        master_volume_db: 0.0,
        sound_mix: false,
        groups: Vec::new(),
        beat_grid: None,
        id: Uuid::now_v7(),
        name,
        width: settings.width.clamp(16, 16384),
        height: settings.height.clamp(16, 16384),
        frame_rate: settings.frame_rate,
        duration: settings.duration,
        // The dialog's own answers: the Background row and the
        // Motion blur section decide these at creation, and a settings
        // block that was never filled in carries the defaults anyway.
        background: LinearColour(settings.background),
        work_area: None,
        layers: Vec::new(),
        markers: Vec::new(),
        motion_blur: MotionBlur {
            shutter_angle: settings.shutter_angle,
            samples: settings.motion_blur_samples.max(1),
            ..MotionBlur::default()
        },
        extra: serde_json::Map::new(),
    };
    let comp_id = comp.id;

    // The comp's index has to account for any AddItem already queued ahead of
    // it in this same batch.
    let queued = ops
        .iter()
        .filter(|o| matches!(o, Op::AddItem { .. }))
        .count();
    ops.push(Op::AddItem {
        index: doc.items.len() + queued,
        item: Box::new(ProjectItem::Composition(comp)),
    });

    // The folder may have been created earlier in this same batch, so it is
    // absent from `doc` and its children start empty.
    let mut children = doc
        .folder(folder_id)
        .map(|f| f.children.clone())
        .unwrap_or_default();
    children.push(comp_id);
    ops.push(Op::SetFolderChildren {
        folder: folder_id,
        children,
    });

    (comp_id, ops)
}

/// The ops that give `comp` new settings, to be committed in order as one undo
/// step.
///
/// Dimensions are clamped to 16..=16384. The other settings go first, at the
/// size the frame has, so the step is named for the settings in the undo list.
/// Then a new frame size, which keeps the picture's middle in the middle: the
/// frame is cropped (or grown) evenly about its centre, which moves the layers
/// with it. Without this every layer stays where it was against the top-left
/// corner and a narrower comp shows them pushed to one side.
pub fn comp_settings_ops(comp: &lumit_core::model::Composition, settings: CompSettings) -> Vec<Op> {
    let motion_blur = lumit_core::model::MotionBlur {
        // The dialog does not ask whether the master switch is on, or what
        // the shutter's phase is; both are left exactly as they stand.
        shutter_angle: settings.shutter_angle,
        samples: settings.motion_blur_samples.max(1),
        ..comp.motion_blur
    };
    let (width, height) = (
        settings.width.clamp(16, 16384),
        settings.height.clamp(16, 16384),
    );
    let mut ops = vec![Op::SetCompSettings {
        comp: comp.id,
        name: settings.name,
        width: comp.width,
        height: comp.height,
        frame_rate: settings.frame_rate,
        duration: settings.duration,
        background: LinearColour(settings.background),
    }];
    if (width, height) != (comp.width, comp.height) {
        ops.push(Op::CropCompToRegion {
            comp: comp.id,
            x: (f64::from(comp.width) - f64::from(width)) / 2.0,
            y: (f64::from(comp.height) - f64::from(height)) / 2.0,
            width,
            height,
        });
    }
    if motion_blur != comp.motion_blur {
        ops.push(Op::SetCompMotionBlur {
            comp: comp.id,
            motion_blur,
        });
    }
    ops
}

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
    mut transform: TransformGroup,
) -> Layer {
    let label = kind.default_label();
    // Nothing lights a camera, a light or a null, so it keeps the material
    // a file leaves out.
    if matches!(
        kind,
        LayerKind::Camera { .. } | LayerKind::Light { .. } | LayerKind::Null
    ) {
        transform.material = Box::new(lumit_core::model::Material::legacy());
    }
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
        pitch_follows_speed: false,
        audio_routes: Default::default(),
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
/// picture asks whether any layer that *draws* is soloed, an Audio layer
/// whether another Audio layer is. The mixer also counts a soloed footage
/// layer with sound, but that takes a probe of the file, which this cannot
/// make. A Camera, a Light and a Null are
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
    layer.switches.solo |= existing.any(|l| l.switches.solo && l.audio_only == audio_only);
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

/// A fresh Null layer for `comp`: an invisible layer with no source of its
/// own, carrying only a transform, for parenting rigs. It has no size, so only
/// its position is centred and the anchor stays at the origin.
pub fn null_layer(comp: &lumit_core::model::Composition) -> Layer {
    use lumit_core::anim::Property;
    base_layer(
        "Null".into(),
        LayerKind::Null,
        comp.duration.0,
        TransformGroup {
            position_x: Property::fixed(f64::from(comp.width) * 0.5),
            position_y: Property::fixed(f64::from(comp.height) * 0.5),
            ..TransformGroup::default()
        },
    )
}

/// A fresh Solid layer for `comp`, comp-sized, white and centred, and the ops
/// that add the solid asset it shows ([`white_solid_ops`]). The layer itself
/// is left for the caller to add.
pub fn white_solid_layer(
    doc: &lumit_core::model::Document,
    comp: &lumit_core::model::Composition,
) -> (Layer, Vec<Op>) {
    let (def, name, ops) = white_solid_ops(doc, comp.width, comp.height);
    let layer = base_layer(
        name,
        LayerKind::Solid { def },
        comp.duration.0,
        centred_transform(
            f64::from(comp.width),
            f64::from(comp.height),
            comp.width,
            comp.height,
        ),
    );
    (layer, ops)
}

/// A fresh Precomp layer placing comp `inner` into comp `outer`, centred and
/// as long as `outer`. The layer is left for the caller to add.
pub fn precomp_layer(
    doc: &lumit_core::model::Document,
    outer: &lumit_core::model::Composition,
    inner: &lumit_core::model::Composition,
) -> Layer {
    let mut layer = base_layer(
        inner.name.clone(),
        LayerKind::Precomp { comp: inner.id },
        outer.duration.0,
        centred_transform(
            f64::from(inner.width),
            f64::from(inner.height),
            outer.width,
            outer.height,
        ),
    );
    // The comp's own markers come with it as the layer's: a
    // composition dropped into another is a piece of material, and its
    // beats are part of what you are placing. Copied, not referenced —
    // from here they are this layer's, and editing them never reaches back
    // into the composition they came from. New ids for the same reason.
    layer.markers = inner
        .markers
        .iter()
        .map(|m| lumit_core::markers::Marker {
            id: Uuid::now_v7(),
            ..m.clone()
        })
        .collect();
    // Placing a node graph places its Inputs with it
    // (docs/impl/node-graph-comp.md §5.3), so the layer's rows are there
    // to be edited from the moment it lands.
    layer.graph_inputs = graph_inputs_for(doc, inner.id);
    layer
}

/// A fresh `node_graph` instance bound to the node graph `comp`, the Inputs a
/// Precomp layer of it carries (docs/impl/node-graph-comp.md §5.3).
///
/// Seeded at the **graph's own** frame size rather than the placing comp's: the
/// picture the Inputs feed is made in the graph's frame, which is where a
/// positional default belongs. `None` when `comp` is not a node graph.
pub fn graph_inputs_for(
    doc: &lumit_core::model::Document,
    comp: Uuid,
) -> Option<lumit_core::model::EffectInstance> {
    let placed = doc.comp(comp)?;
    let graph = placed.graph.as_ref()?;
    let mut instance = lumit_core::fx::builtins::instantiate_for_raster(
        lumit_core::comp_graph::NODE_GRAPH,
        f64::from(placed.width),
        f64::from(placed.height),
    )?;
    lumit_core::fx::effects::node_graph::bind(&mut instance, comp, graph);
    Some(instance)
}

/// A stable machine key for a [`FxCategory`] (the variant in snake_case) — the
/// grouping key the Dart Effects browser sorts and headers by.
///
/// Shared with the frb surface (`api::effect::list_effects`) rather than
/// restated there, so the two frontends cannot disagree about a category key.
/// **It therefore has to move rather than die when v0 is deleted.**
pub fn fx_category_key(cat: lumit_core::fx::schema::FxCategory) -> &'static str {
    use lumit_core::fx::schema::FxCategory;
    match cat {
        FxCategory::BlurSharpen => "blur_sharpen",
        FxCategory::Colour => "colour",
        FxCategory::Distortion => "distortion",
        FxCategory::Generate => "generate",
        FxCategory::Points => "points",
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
        FxCategory::SiceliotDrivers => "siceliot_drivers",
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

/// What a Pre-compose packs, and how. See [`precompose_ops`].
pub struct Precompose<'a> {
    /// The layers to pack, in any order.
    pub layers: &'a [Uuid],
    /// The new comp's name. Blank names it "Pre-comp" and a number.
    pub name: String,
    pub leave_attributes: bool,
    pub adjust_duration: bool,
    /// The layer group being packed, when there is one.
    pub group: Option<Uuid>,
    /// How long a layer's source runs, where it can be known: a nested comp's
    /// length, or a footage file's once probed.
    pub source_length: &'a dyn Fn(&Layer) -> Option<lumit_core::time::Duration>,
}

/// The ops that pack layers of `comp` into a new comp and put a Precomp layer
/// in their place, and that layer's id. Committed as one [`Op::Batch`], one
/// undo step puts the layers back.
///
/// `leave_attributes` is the dialogue's first choice, and only ever offered
/// for a single layer: the layer moves into the new comp stripped back to
/// its source, and its transform, effects, masks, retime, blend and
/// switches stay behind on the Precomp layer, so the picture is unchanged
/// but the attributes now act on the nested comp. Asking for it with more
/// than one layer is refused rather than half-applied. Without it every
/// layer moves whole, and the Precomp layer is a plain centred one.
///
/// `adjust_duration` trims the new comp to the selection's own span: its
/// duration becomes `max(out) - min(in)`, every packed layer shifts back by
/// `min(in)`, and the Precomp layer spans that same stretch with a start
/// offset that lines inner time zero up with it. Without it the new comp is
/// as long as this one and nothing moves in time at all: every packed layer
/// keeps the times it had, and the Precomp layer spans the whole comp.
///
/// The layers go in at the depth of the topmost one, so a precompose in
/// the middle of a stack does not send it to the front. The comp auto-files
/// into the Compositions folder however it was made.
///
/// A parent or matte link that would cross the new comp's edge is kept by
/// a hidden copy of the layer at the far end, made on the near side with
/// its parent chain ([`link_copies`]), so the picture does not change.
/// `group` names the layer group being pre-composed, when the command came
/// from a group header's *Pre-compose group*: the header's
/// effect stack moves onto the new Precomp layer — where it means exactly
/// what it meant, because a live group renders as that precompose already —
/// and the emptied band is ungrouped in the same batch, so one undo puts
/// the band, its wardrobe and the layers all back.
///
/// Refused with `UnknownLayer` when none of the layers is in the comp or
/// more than one leaves its attributes, `UnknownGroup` for a group the comp
/// does not have, and `InvalidSpan` when a time falls out of range.
pub fn precompose_ops(
    doc: &lumit_core::model::Document,
    comp: &lumit_core::model::Composition,
    asked: Precompose<'_>,
) -> Result<(Vec<Op>, Uuid), lumit_core::ops::OpError> {
    use lumit_core::model::{Composition, MotionBlur};
    use lumit_core::ops::OpError;

    let Precompose {
        layers: layer_ids,
        name,
        leave_attributes,
        adjust_duration,
        group,
        source_length,
    } = asked;

    // The group whose header stack rides across. Resolved up front so a
    // dangling id refuses before anything is committed.
    let carried = match group {
        Some(id) => Some(
            comp.groups
                .iter()
                .find(|g| g.id == id)
                .cloned()
                .ok_or(OpError::UnknownGroup)?,
        ),
        None => None,
    };

    // Read in stack order, not selection order, so the packed comp holds
    // the layers the way the timeline showed them. What is actually packed
    // is what was actually found: an id belonging to some other comp would
    // otherwise fail the batch on its way through `RemoveLayer` and lose
    // the whole precompose with it.
    let packed: Vec<Layer> = comp
        .layers
        .iter()
        .filter(|l| layer_ids.contains(&l.id))
        .cloned()
        .collect();
    if packed.is_empty() {
        return Err(OpError::UnknownLayer);
    }
    // Leaving the attributes behind means there is one layer for them to
    // act on. Asked for a stack, the dialogue offers Move instead, and the
    // engine refuses rather than picking a layer for the user.
    if leave_attributes && packed.len() > 1 {
        return Err(OpError::UnknownLayer);
    }

    // Every packed layer sits at or below this index, so the slot is still
    // a valid one once the batch's removals have run.
    let index = comp
        .layers
        .iter()
        .position(|l| packed.iter().any(|p| p.id == l.id))
        .unwrap_or(0);

    // The selection's own span, and the shift that brings it back to zero
    // inside the new comp. Without `adjust_duration` the shift is nothing
    // and the span is the whole comp, which is what leaves timing alone.
    let min_in = packed
        .iter()
        .map(|l| l.in_point)
        .min()
        .unwrap_or(CompTime::ZERO);
    let max_out = packed
        .iter()
        .map(|l| l.out_point)
        .max()
        .unwrap_or(CompTime(comp.duration.0));
    let span = max_out.delta(min_in).map_err(|_| OpError::InvalidSpan)?;
    let (duration, shift, precomp_in, precomp_out) =
        if adjust_duration && !span.0.is_zero() && !span.0.is_negative() {
            (span, min_in, min_in, max_out)
        } else {
            (
                comp.duration,
                CompTime::ZERO,
                CompTime::ZERO,
                CompTime(comp.duration.0),
            )
        };
    // Time moves as a whole: a layer's in point, out point and start
    // offset all step back by the same amount, so a packed layer plays the
    // same footage at the same moment of the Precomp layer as before.
    let shift_back = |t: CompTime| -> Result<CompTime, OpError> {
        t.sub_dur(lumit_core::time::Duration(shift.0))
            .map_err(|_| OpError::InvalidSpan)
    };

    let name = if name.trim().is_empty() {
        let existing = doc
            .items
            .iter()
            .filter(|i| matches!(i, ProjectItem::Composition(c) if c.name.starts_with("Pre-comp ")))
            .count();
        format!("Pre-comp {}", existing + 1)
    } else {
        name.trim().to_string()
    };

    // A retimed layer that leaves its attributes behind leaves its Retime
    // too, and its place in time goes with that: the Retime reads the
    // layer's own clock, so the Precomp layer has to keep that clock. The
    // layer inside then starts at nought and runs the whole of the new
    // comp, which is made long enough to hold all of its source.
    let retimed = leave_attributes && packed.iter().any(|l| l.retime.is_some());
    let duration = if retimed {
        let source = packed.first().and_then(source_length);
        duration.max(source.unwrap_or(duration))
    } else {
        duration
    };

    let mut inner_layers = Vec::with_capacity(packed.len());
    for src in &packed {
        let mut layer = src.clone();
        if leave_attributes {
            // An id of its own. The Precomp layer takes this one's, and
            // with it everything in the comp that points here: a child, a
            // matte, an effect's layer row.
            layer.id = Uuid::now_v7();
            // Stripped back to its source: the attributes are staying
            // behind on the Precomp layer, and a copy on both would apply
            // each of them twice.
            layer.transform = centred_transform(
                f64::from(comp.width),
                f64::from(comp.height),
                comp.width,
                comp.height,
            );
            layer.effects.clear();
            layer.masks.clear();
            layer.retime = None;
            layer.blend = Default::default();
            layer.switches = Default::default();
            layer.parent = None;
            layer.matte = None;
        }
        if retimed {
            layer.in_point = CompTime::ZERO;
            layer.out_point = CompTime(duration.0);
            layer.start_offset = CompTime::ZERO;
        } else {
            layer.in_point = shift_back(src.in_point)?;
            layer.out_point = shift_back(src.out_point)?;
            layer.start_offset = shift_back(src.start_offset)?;
        }
        inner_layers.push(layer);
    }

    // A parent or matte link that would cross the new comp's edge is kept
    // by a hidden copy of the layer at the far end, on the near side.
    let ids: Vec<Uuid> = packed.iter().map(|l| l.id).collect();
    let copies = if leave_attributes {
        LinkCopies::default()
    } else {
        link_copies(&comp.layers, &ids)
    };
    for layer in &mut inner_layers {
        if let Some(copy) = layer.parent.and_then(|p| copies.inside_ids.get(&p)) {
            layer.parent = Some(*copy);
        }
        if let Some(matte) = &mut layer.matte {
            if let Some(copy) = copies.inside_ids.get(&matte.layer) {
                matte.layer = *copy;
            }
        }
    }
    for copy in &copies.inside {
        let mut copy = copy.clone();
        copy.in_point = shift_back(copy.in_point)?;
        copy.out_point = shift_back(copy.out_point)?;
        copy.start_offset = shift_back(copy.start_offset)?;
        inner_layers.push(copy);
    }

    let inner = Composition {
        graph: None,
        master_volume_db: 0.0,
        lighting: Default::default(),
        sound_mix: false,
        groups: Vec::new(),
        beat_grid: None,
        id: Uuid::now_v7(),
        name: name.clone(),
        width: comp.width,
        height: comp.height,
        frame_rate: comp.frame_rate,
        duration,
        background: comp.background,
        work_area: None,
        layers: inner_layers,
        // The comp's markers go in with the layers. They are part
        // of how the work is laid out, and a packed section that loses its
        // cues has lost the map to itself. Shifted with everything else
        // when `adjust_duration` moves time back to zero, and any that fall
        // outside the new comp's span are left behind rather than parked
        // where nothing can reach them.
        markers: comp
            .markers
            .iter()
            .filter_map(|m| {
                let time = shift_back(m.time).ok()?;
                (!time.0.is_negative() && time.0 <= duration.0).then(|| {
                    lumit_core::markers::Marker {
                        id: Uuid::now_v7(),
                        time,
                        ..m.clone()
                    }
                })
            })
            .collect(),
        motion_blur: MotionBlur::default(),
        extra: serde_json::Map::new(),
    };
    let inner_id = inner.id;

    // Comps auto-file into the Compositions folder, however they are made —
    // a precomp that landed at the project root would be the one comp the
    // habit missed.
    let (folder, mut ops) = ensure_auto_folder_ops(doc, AutoFolderKind::Compositions);
    let queued = ops
        .iter()
        .filter(|o| matches!(o, Op::AddItem { .. }))
        .count();
    ops.push(Op::AddItem {
        index: doc.items.len() + queued,
        item: Box::new(ProjectItem::Composition(inner)),
    });
    ops.push(file_into_folder_op(doc, folder, inner_id));

    // The band goes before its members do: a batch inverts in
    // reverse, so ungrouping first is what lets the undo put the layers
    // back before `GroupLayers` checks that they are a contiguous run.
    if let Some(g) = &carried {
        ops.push(Op::UngroupLayers {
            comp: comp.id,
            group: g.id,
        });
    }
    for layer in &packed {
        ops.push(Op::RemoveLayer {
            comp: comp.id,
            layer: layer.id,
        });
    }

    let mut layer = base_layer(
        name,
        LayerKind::Precomp { comp: inner_id },
        precomp_out.0,
        if leave_attributes {
            packed[0].transform.clone()
        } else {
            centred_transform(
                f64::from(comp.width),
                f64::from(comp.height),
                comp.width,
                comp.height,
            )
        },
    );
    layer.in_point = precomp_in;
    layer.out_point = precomp_out;
    // Inner time zero is the moment the new comp starts in this one, so a
    // trimmed comp needs the offset to line the two up; an untrimmed one
    // starts at zero and needs none.
    layer.start_offset = shift;
    if leave_attributes {
        let src = &packed[0];
        layer.effects = src.effects.clone();
        layer.masks = src.masks.clone();
        layer.retime = src.retime.clone();
        layer.blend = src.blend;
        layer.switches = src.switches;
        // It stands where the layer stood: under the same id, with the
        // same parent and the same matte.
        layer.id = src.id;
        layer.parent = src.parent;
        layer.matte = src.matte;
        if retimed {
            layer.in_point = src.in_point;
            layer.out_point = src.out_point;
            layer.start_offset = src.start_offset;
        }
    }
    if let Some(g) = &carried {
        // The header's stack lands after any attributes the layer kept —
        // the order the live group already ran them in: each member's own
        // effects first, the header's on the composite. The picture does
        // not change across the conversion, which is §2's whole argument
        // and test 8's assertion. The band itself was ungrouped above,
        // before the removals.
        layer.effects.extend(g.effects.iter().cloned());
    }
    // Asked of the layers that are staying: the soloed ones being packed
    // away are about to leave, and a Precomp layer soloed on their account
    // would isolate itself against a comp that has no solo left.
    solo_on_arrival(
        &mut layer,
        comp.layers
            .iter()
            .filter(|l| !packed.iter().any(|p| p.id == l.id)),
    );
    let layer_id = layer.id;
    ops.push(Op::AddLayer {
        comp: comp.id,
        index,
        layer: Box::new(layer),
    });

    // The copies left behind go to the bottom of the stack, and what
    // pointed at a packed layer points at its copy.
    let bottom = comp.layers.len() - packed.len() + 1;
    for (n, copy) in copies.outside.into_iter().enumerate() {
        ops.push(Op::AddLayer {
            comp: comp.id,
            index: bottom + n,
            layer: Box::new(copy),
        });
    }
    for layer in comp.layers.iter().filter(|l| !ids.contains(&l.id)) {
        if let Some(copy) = layer.parent.and_then(|p| copies.outside_ids.get(&p)) {
            ops.push(Op::SetLayerParent {
                comp: comp.id,
                layer: layer.id,
                parent: Some(*copy),
            });
        }
        if let Some(matte) = &layer.matte {
            if let Some(copy) = copies.outside_ids.get(&matte.layer) {
                ops.push(Op::SetLayerMatte {
                    comp: comp.id,
                    layer: layer.id,
                    matte: Some(lumit_core::model::MatteRef {
                        layer: *copy,
                        ..*matte
                    }),
                });
            }
        }
        // Captions made from a packed layer now hear it through the Precomp.
        ops.extend(lumit_core::captions::repointed(
            comp.id, layer, &ids, layer_id,
        ));
    }
    Ok((ops, layer_id))
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
            LayerKind::Precomp { comp } => {
                f(comp);
                // A copy of a graphics template keeps its values for the
                // composition it places, by id, so they follow it.
                if let Some(inputs) = &mut layer.graph_inputs {
                    lumit_core::fx::effects::template::rebind(inputs, *comp);
                }
            }
            LayerKind::Sequence { clips } => {
                for clip in clips {
                    if let lumit_core::sequence::ClipSource::Comp(id) = &mut clip.source {
                        f(id);
                        if let Some(inputs) = &mut clip.inputs {
                            lumit_core::fx::effects::template::rebind(inputs, *id);
                        }
                    }
                    for take in &mut clip.takes {
                        if let lumit_core::sequence::ClipSource::Comp(id) = &mut take.source {
                            f(id);
                        }
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
                // A speaker is the same person in the copy, so a rename
                // reaches both.
                let kept = |(k, _): &(&String, &Value)| k.as_str() != "speakers";
                fields
                    .iter()
                    .filter(kept)
                    .for_each(|(_, v)| collect(v, fresh));
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
                } else if let Some((name, socket)) = s.split_once('/') {
                    // A tie names a wire `kind:id/socket`, by the box it ends in.
                    let (kind, id) = name.split_once(':').unwrap_or(("", name));
                    if let Some(new) = fresh.get(id) {
                        *s = if kind.is_empty() {
                            format!("{new}/{socket}")
                        } else {
                            format!("{kind}:{new}/{socket}")
                        };
                    }
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
    /// The master shutter's angle in degrees, and the fewest and most
    /// sub-frame samples a blurred layer is drawn at. The shutter's phase is
    /// left as it stands.
    pub shutter_angle: f64,
    pub motion_blur_samples: u32,
    pub motion_blur_limit: u32,
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
            motion_blur_limit: blur.limit,
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
        lighting: Default::default(),
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
            limit: settings.motion_blur_limit.max(1),
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
        limit: settings.motion_blur_limit.max(1),
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

/// One key given on the composition's clock, moved onto the layer's own by
/// `offset`, the layer's `start_offset`. The engine keys every property in
/// layer time so a layer's animation travels with it, while people draw, edit
/// and write keys in comp time. `None` when the time overflows.
pub fn key_on_layer_clock(
    key: lumit_core::anim::Keyframe,
    offset: Rational,
) -> Option<lumit_core::anim::Keyframe> {
    Some(lumit_core::anim::Keyframe {
        time: key.time.checked_sub(offset).ok()?,
        ..key
    })
}

/// `keys` as a keyframed animation, or `None` when there are none or their
/// times are not strictly ascending. `anim::evaluate` walks the list assuming
/// it is sorted, so an unsorted one does not error, it silently evaluates
/// wrongly.
pub fn keyframed(keys: Vec<lumit_core::anim::Keyframe>) -> Option<lumit_core::anim::Animation> {
    if keys.is_empty() || keys.windows(2).any(|pair| pair[1].time <= pair[0].time) {
        return None;
    }
    Some(lumit_core::anim::Animation::Keyframed(keys))
}

/// The ops that put `animation` on one transform or camera property of
/// `layer` in `comp`: its animation, and beside it what an expression knows
/// about the property (its language, the value underneath, its seed) whenever
/// that changes. `language` is the expression's, when `animation` is one.
///
/// The note is worked out here and carried whole in the op, so undo puts
/// back the one that was there and a replayed journal writes the same seed.
pub fn transform_ops(
    comp: Uuid,
    layer: &Layer,
    prop: lumit_core::model::TransformProp,
    animation: lumit_core::anim::Animation,
    language: Option<lumit_core::expression::script::Language>,
) -> Vec<Op> {
    use lumit_core::expression::script::SLOT_KEY;

    let mut ops = vec![Op::SetTransformProperty {
        comp,
        layer: layer.id,
        prop,
        animation: animation.clone(),
    }];
    if let Some(held) = layer.prop(prop) {
        let mut next = held.clone();
        next.set_animation(animation);
        if let Some(language) = language {
            next.set_expression_language(language);
        }
        // A new expression on half of a point reads the whole point, as one
        // imported from After Effects does.
        if !held.extra.contains_key(SLOT_KEY) {
            lumit_core::expression::note_transform_point(layer, prop, &mut next);
        }
        let slot = next.extra.get(SLOT_KEY).cloned();
        if slot.as_ref() != held.extra.get(SLOT_KEY) {
            ops.push(Op::SetTransformExpressionSlot {
                comp,
                layer: layer.id,
                prop,
                slot,
            });
        }
    }
    // An old light's edge and falloff numbers mean the old curves. Edit one
    // and the light moves onto After Effects' curves in the same step, so
    // undo puts the old ones back.
    if let LayerKind::Light { light } = &layer.kind {
        use lumit_core::model::TransformProp as P;
        let shape = matches!(prop, P::ConeFeather | P::LightRadius | P::FalloffDistance);
        if light.legacy_shape && shape {
            ops.push(Op::SetLightSettings {
                comp,
                layer: layer.id,
                settings: lumit_core::model::LightSettings {
                    legacy_shape: false,
                    ..light.settings()
                },
            });
        }
    }
    ops
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests;

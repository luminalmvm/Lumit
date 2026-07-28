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

use lumit_core::model::{Folder, Layer, LayerKind, ProjectItem, Switches, TransformGroup};
use lumit_core::ops::{AutoFolderKind, Op};
use lumit_core::time::{CompTime, Rational};
use uuid::Uuid;

/// A transform anchored on the content's own centre and placed at the comp
/// centre — `lumit-ui`'s `centred_transform`, the seeding every add-layer path
/// uses so a fresh layer appears centred and pivots about its middle (K-150).
///
/// Shared with the frb API rather than copied: a second set of layer-seeding
/// defaults would drift, and a footage layer added through the new bridge would
/// then land in a different place from one added through the old.
pub(crate) fn centred_transform(
    nat_w: f64,
    nat_h: f64,
    comp_w: u32,
    comp_h: u32,
) -> TransformGroup {
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
///
/// Shared with the frb API for the same reason as [`centred_transform`].
pub(crate) fn base_layer(
    name: String,
    kind: LayerKind,
    out: Rational,
    transform: TransformGroup,
) -> Layer {
    // Each kind starts on its own label colour (K-188): the label drives both
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
        LayerKind::NullObject => 0,
    };
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
        label,
        volume_db: lumit_core::anim::Property::zero(),
        retime: None,
        blend: lumit_core::model::BlendMode::Normal,
        masks: Vec::new(),
        effects: Vec::new(),
        switches: Switches::default(),
        extra: serde_json::Map::new(),
    }
}

/// The ops that guarantee `kind`'s auto-filing folder exists, plus its id —
/// `lumit-ui`'s `ensure_auto_folder_ops`, tracked by id so renaming or nesting
/// the folder keeps the habit.
pub(crate) fn ensure_auto_folder_ops(
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
pub(crate) fn file_into_folder_op(
    doc: &lumit_core::model::Document,
    folder: Uuid,
    item: Uuid,
) -> Op {
    let mut children = doc
        .folder(folder)
        .map(|f| f.children.clone())
        .unwrap_or_default();
    children.push(item);
    Op::SetFolderChildren { folder, children }
}

/// A stable machine key for a [`FxCategory`] (the variant in snake_case) — the
/// grouping key the Dart Effects browser sorts and headers by.
///
/// Shared with the frb surface (`api::effect::list_effects`) rather than
/// restated there, so the two frontends cannot disagree about a category key.
/// **It therefore has to move rather than die when v0 is deleted.**
pub(crate) fn fx_category_key(cat: lumit_core::fx::FxCategory) -> &'static str {
    use lumit_core::fx::FxCategory;
    match cat {
        FxCategory::BlurSharpen => "blur_sharpen",
        FxCategory::Colour => "colour",
        FxCategory::Distortion => "distortion",
        FxCategory::Stylise => "stylise",
        FxCategory::Temporal => "temporal",
        FxCategory::Utility => "utility",
    }
}
